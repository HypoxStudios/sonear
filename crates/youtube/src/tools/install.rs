use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::Url;
use reqwest::header;
use reqwest::redirect::Policy;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

use super::{ToolPaths, ToolVersions};
use crate::error::ToolError;

pub const GITHUB_API: &str = "https://api.github.com/repos/";

const MPV_REPO: &str = "mpv-player/mpv";
const YTDLP_REPO: &str = "yt-dlp/yt-dlp";
const MPV_ASSET_SUFFIX: &str = "-x86_64-pc-windows-msvc.zip";
const YTDLP_ASSET: &str = "yt-dlp.exe";
const SUMS_ASSET: &str = "SHA2-256SUMS";
const MAX_EXTRACTED_BYTES: u64 = 512 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const STALL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Mpv,
    YtDlp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolProgress {
    Downloading {
        tool: Tool,
        received: u64,
        total: Option<u64>,
    },
    Verifying {
        tool: Tool,
    },
    Finalizing {
        tool: Tool,
    },
    Done {
        tool: Tool,
    },
}

type Progress<'a> = &'a (dyn Fn(ToolProgress) + Send + Sync);

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

impl Release {
    fn asset(&self, wanted: impl Fn(&str) -> bool) -> Option<&Asset> {
        self.assets.iter().find(|asset| wanted(&asset.name))
    }
}

/// Removes the download directory when it goes out of scope.
struct WorkDir(PathBuf);

impl Drop for WorkDir {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            warn!(%error, "could not remove the tool install work directory");
        }
    }
}

pub struct Installer {
    paths: ToolPaths,
    http: reqwest::Client,
    api_base: String,
}

fn allowed_host(url: &Url, strict: bool) -> bool {
    if !strict {
        return true;
    }

    url.scheme() == "https"
        && url
            .host_str()
            .is_some_and(|host| host == "github.com" || host.ends_with(".githubusercontent.com"))
}

fn http_client(strict: bool) -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("Sonear/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(STALL_TIMEOUT)
        .redirect(Policy::custom(move |attempt| {
            if allowed_host(attempt.url(), strict) && attempt.previous().len() < 5 {
                attempt.follow()
            } else {
                attempt.error("redirected away from GitHub")
            }
        }))
        .build()
        .unwrap_or_default()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn digest_of(asset: &Asset) -> Option<String> {
    asset
        .digest
        .as_deref()?
        .strip_prefix("sha256:")
        .map(str::to_lowercase)
}

/// The checksum for `name` from a `SHA2-256SUMS` file.
fn checksum_in(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;

        (file.trim().trim_start_matches('*') == name).then(|| hash.to_lowercase())
    })
}

impl Installer {
    pub fn new(paths: ToolPaths) -> Self {
        Self {
            paths,
            http: http_client(true),
            api_base: GITHUB_API.to_owned(),
        }
    }

    /// Points at another API and allows plain HTTP, for tests.
    pub fn with_api_base(paths: ToolPaths, api_base: &str) -> Self {
        let mut api_base = api_base.to_owned();

        if !api_base.ends_with('/') {
            api_base.push('/');
        }

        Self {
            paths,
            http: http_client(false),
            api_base,
        }
    }

    fn strict(&self) -> bool {
        self.api_base == GITHUB_API
    }

    pub async fn install_mpv(&self, progress: Progress<'_>) -> Result<String, ToolError> {
        let release = self.release(MPV_REPO).await?;
        let asset = release
            .asset(|name| {
                name.starts_with("mpv-v")
                    && name.ends_with(MPV_ASSET_SUFFIX)
                    && !name.contains("pdb")
            })
            .ok_or_else(|| ToolError::Download("the mpv release has no Windows build".into()))?;
        let expected = digest_of(asset).ok_or(ToolError::Checksum)?;
        let work = self.work_dir()?;
        let archive = work.0.join("mpv.zip");

        self.download(
            Tool::Mpv,
            &asset.browser_download_url,
            &archive,
            &expected,
            progress,
        )
        .await?;
        progress(ToolProgress::Verifying { tool: Tool::Mpv });

        let staging = work.0.join("staging");
        let source = archive.clone();
        let target = staging.clone();

        tokio::task::spawn_blocking(move || extract_mpv(&source, &target))
            .await
            .map_err(|error| ToolError::Io(error.to_string()))??;

        self.finish(Tool::Mpv, &staging, &release.tag_name, progress)
            .await?;

        Ok(release.tag_name)
    }

    pub async fn install_ytdlp(&self, progress: Progress<'_>) -> Result<String, ToolError> {
        let release = self.release(YTDLP_REPO).await?;

        self.install_ytdlp_release(release, progress).await
    }

    async fn install_ytdlp_release(
        &self,
        release: Release,
        progress: Progress<'_>,
    ) -> Result<String, ToolError> {
        let asset = release
            .asset(|name| name == YTDLP_ASSET)
            .ok_or_else(|| ToolError::Download("the yt-dlp release has no exe".into()))?;
        let listed = digest_of(asset);
        let sums = match release.asset(|name| name == SUMS_ASSET) {
            Some(sums) => self
                .fetch_text(&sums.browser_download_url)
                .await
                .inspect_err(|error| warn!(%error, "could not fetch the yt-dlp checksums"))
                .ok()
                .and_then(|text| checksum_in(&text, YTDLP_ASSET)),
            None => None,
        };
        let expected = match (listed, sums) {
            (Some(listed), Some(sums)) if listed != sums => return Err(ToolError::Checksum),
            (Some(hash), _) | (None, Some(hash)) => hash,
            (None, None) => return Err(ToolError::Checksum),
        };
        let work = self.work_dir()?;
        let staging = work.0.join("staging");

        fs::create_dir_all(&staging)?;
        self.download(
            Tool::YtDlp,
            &asset.browser_download_url,
            &staging.join(YTDLP_ASSET),
            &expected,
            progress,
        )
        .await?;
        progress(ToolProgress::Verifying { tool: Tool::YtDlp });
        self.finish(Tool::YtDlp, &staging, &release.tag_name, progress)
            .await?;

        Ok(release.tag_name)
    }

    /// Installs whichever of the two tools is missing.
    pub async fn install_missing(&self, progress: Progress<'_>) -> Result<(), ToolError> {
        let installed = ToolVersions::installed(&self.paths);

        if installed.mpv.is_none() {
            self.install_mpv(progress).await?;
        }

        if installed.ytdlp.is_none() {
            self.install_ytdlp(progress).await?;
        }

        Ok(())
    }

    pub async fn latest_ytdlp_tag(&self) -> Result<String, ToolError> {
        Ok(self.release(YTDLP_REPO).await?.tag_name)
    }

    /// Installs the newest yt-dlp when it differs from the installed one.
    /// Returns whether anything changed. mpv is not touched.
    pub async fn update_ytdlp(&self, progress: Progress<'_>) -> Result<bool, ToolError> {
        let release = self.release(YTDLP_REPO).await?;
        let installed = ToolVersions::installed(&self.paths);

        if installed.ytdlp.as_deref() == Some(release.tag_name.as_str()) {
            return Ok(false);
        }

        self.install_ytdlp_release(release, progress).await?;

        Ok(true)
    }

    async fn release(&self, repo: &str) -> Result<Release, ToolError> {
        let url = format!("{}{repo}/releases/latest", self.api_base);
        let response = self
            .http
            .get(url)
            .header(header::ACCEPT, "application/vnd.github+json")
            .send()
            .await
            .map_err(|error| ToolError::Download(error.without_url().to_string()))?;

        if !response.status().is_success() {
            return Err(ToolError::Download(format!(
                "GitHub answered {}",
                response.status()
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|error| ToolError::Download(error.without_url().to_string()))?;

        serde_json::from_slice(&bytes)
            .map_err(|error| ToolError::Download(format!("unreadable release: {error}")))
    }

    fn checked_url(&self, url: &str) -> Result<Url, ToolError> {
        let url = Url::parse(url).map_err(|_| ToolError::Download("bad download URL".into()))?;

        if !allowed_host(&url, self.strict()) {
            return Err(ToolError::Download(format!(
                "refused to download from {}",
                url.host_str().unwrap_or("an unknown host")
            )));
        }

        Ok(url)
    }

    async fn fetch_text(&self, url: &str) -> Result<String, ToolError> {
        let response = self
            .http
            .get(self.checked_url(url)?)
            .send()
            .await
            .map_err(|error| ToolError::Download(error.without_url().to_string()))?;

        if !response.status().is_success() {
            return Err(ToolError::Download(format!(
                "server answered {}",
                response.status()
            )));
        }

        response
            .text()
            .await
            .map_err(|error| ToolError::Download(error.without_url().to_string()))
    }

    async fn download(
        &self,
        tool: Tool,
        url: &str,
        destination: &Path,
        expected: &str,
        progress: Progress<'_>,
    ) -> Result<(), ToolError> {
        let response = self
            .http
            .get(self.checked_url(url)?)
            .send()
            .await
            .map_err(|error| ToolError::Download(error.without_url().to_string()))?;

        if !response.status().is_success() {
            return Err(ToolError::Download(format!(
                "server answered {}",
                response.status()
            )));
        }

        let total = response.content_length();
        let mut file = tokio::fs::File::create(destination).await?;
        let mut stream = response.bytes_stream();
        let mut hasher = Sha256::new();
        let mut received = 0u64;

        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|error| ToolError::Download(error.without_url().to_string()))?;

            hasher.update(&chunk);
            file.write_all(&chunk).await?;
            received += chunk.len() as u64;
            progress(ToolProgress::Downloading {
                tool,
                received,
                total,
            });
        }

        file.flush().await?;
        file.sync_all().await?;

        if hex(&hasher.finalize()) != expected.to_lowercase() {
            return Err(ToolError::Checksum);
        }

        Ok(())
    }

    fn work_dir(&self) -> Result<WorkDir, ToolError> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or_default();
        let dir = self
            .paths
            .root()
            .join(format!("install-{}-{stamp}", std::process::id()));

        fs::create_dir_all(&dir)?;

        Ok(WorkDir(dir))
    }

    async fn finish(
        &self,
        tool: Tool,
        staging: &Path,
        tag: &str,
        progress: Progress<'_>,
    ) -> Result<(), ToolError> {
        progress(ToolProgress::Finalizing { tool });

        let target = match tool {
            Tool::Mpv => self.paths.mpv_dir(),
            Tool::YtDlp => self.paths.ytdlp_dir(),
        };
        let staging = staging.to_owned();

        tokio::task::spawn_blocking(move || swap_dir(&staging, &target))
            .await
            .map_err(|error| ToolError::Io(error.to_string()))??;

        let mut versions = ToolVersions::load(&self.paths);

        match tool {
            Tool::Mpv => versions.mpv = Some(tag.to_owned()),
            Tool::YtDlp => versions.ytdlp = Some(tag.to_owned()),
        }

        versions.save(&self.paths)?;
        info!(?tool, tag, "installed a playback tool");
        progress(ToolProgress::Done { tool });

        Ok(())
    }
}

fn extract_mpv(archive: &Path, target: &Path) -> Result<(), ToolError> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)
        .map_err(|error| ToolError::Io(format!("unreadable archive: {error}")))?;
    let mut found_exe = false;
    let mut total = 0u64;

    fs::create_dir_all(target)?;

    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| ToolError::Io(format!("unreadable archive: {error}")))?;

        if entry.is_dir() {
            continue;
        }

        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        let mut parts = path.components();
        let (Some(std::path::Component::Normal(name)), None) = (parts.next(), parts.next()) else {
            continue;
        };
        let Some(name) = name.to_str().map(str::to_owned) else {
            continue;
        };
        let extension = Path::new(&name)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_lowercase);

        if !matches!(extension.as_deref(), Some("exe" | "dll")) {
            continue;
        }

        total += entry.size();

        if total > MAX_EXTRACTED_BYTES {
            return Err(ToolError::Io("the archive is unexpectedly large".into()));
        }

        let mut out = File::create(target.join(&name))?;

        io::copy(&mut entry, &mut out)?;
        out.sync_all()?;
        found_exe |= name.eq_ignore_ascii_case("mpv.exe");
    }

    if !found_exe {
        return Err(ToolError::Io("the archive does not contain mpv.exe".into()));
    }

    Ok(())
}

/// Replaces `target` with `staging` by renaming, keeping the old directory
/// as `<name>.old` until the new one is in place.
fn swap_dir(staging: &Path, target: &Path) -> Result<(), ToolError> {
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ToolError::Io("bad install directory".into()))?;
    let backup = target.with_file_name(format!("{name}.old"));
    let had_previous = target.exists();

    if backup.exists() {
        fs::remove_dir_all(&backup)?;
    }

    if had_previous {
        fs::rename(target, &backup)?;
    }

    if let Err(error) = fs::rename(staging, target) {
        if had_previous {
            let _ = fs::rename(&backup, target);
        }

        return Err(error.into());
    }

    if had_previous && let Err(error) = fs::remove_dir_all(&backup) {
        warn!(%error, "could not remove the previous tool install");
    }

    Ok(())
}

#[cfg(test)]
mod tests;
