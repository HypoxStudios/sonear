//! Finds, downloads and verifies new versions of Sonear. Nothing here applies an
//! update: the app does that when the user asks, and only with a file this module
//! has checked against the maker's public key.

use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt as _;
use minisign_verify::{PublicKey, Signature};
use reqwest::redirect::Policy;
use reqwest::{Response, StatusCode, Url, header};
use serde::Deserialize;
use thiserror::Error;
use tokio::io::AsyncWriteExt as _;
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, warn};

use crate::paths::AppPaths;

/// The public half of the key that signs releases, as the base64 line of its
/// `.pub` file. While it is `None` the updater never runs.
pub const PUBLIC_KEY: Option<&str> =
    Some("RWRrqngyBCUpPeWlRBJiGer5ERmesAHQE6Qq2Dtssvr5xU3QFgDuuTiy");
pub const RELEASES_URL: &str = "https://api.github.com/repos/HypoxStudios/sonear/releases/latest";

const RELEASE_PAGE_PREFIX: &str = "https://github.com/HypoxStudios/sonear/";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_RELEASE_BYTES: u64 = 1024 * 1024;
const MAX_SIGNATURE_BYTES: u64 = 16 * 1024;
const MAX_EXE_BYTES: u64 = 256 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const STALL_TIMEOUT: Duration = Duration::from_secs(60);
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(10);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const PROGRESS_EVERY: Duration = Duration::from_millis(500);
const VERIFY_CHUNK: usize = 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("could not reach GitHub: {0}")]
    Network(String),
    #[error("GitHub answered {0}")]
    Status(u16),
    #[error("the release data could not be read: {0}")]
    Release(String),
    #[error("a download is larger than allowed")]
    TooLarge,
    #[error("the signature check failed: {0}")]
    Signature(String),
    #[error("the file is signed as `{found}`, not as `sonear {expected}`")]
    WrongVersion { expected: String, found: String },
    #[error("file error: {0}")]
    Io(#[from] io::Error),
}

impl UpdateError {
    /// Whether the file itself is to blame, not the connection.
    fn is_untrusted(&self) -> bool {
        matches!(self, Self::Signature(_) | Self::WrongVersion { .. })
    }
}

fn network(error: reqwest::Error) -> UpdateError {
    UpdateError::Network(error.to_string())
}

/// A newer version that can be downloaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub exe_url: String,
    pub signature_url: String,
    /// The release page, when it is on Sonear's own repository.
    pub notes_url: Option<String>,
}

/// A downloaded update whose signature has been checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub version: String,
    pub file: PathBuf,
    pub notes_url: Option<String>,
}

/// What the background task tells whoever started it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateEvent {
    Downloading {
        received: u64,
        total: Option<u64>,
    },
    Ready(Prepared),
    /// A download ended without a usable update.
    Idle,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
}

/// Compares dotted versions as numbers, so `0.10.0` is newer than `0.9.0`.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let numbers = |version: &str| -> Vec<u64> {
        version
            .split('.')
            .map(|part| part.trim().parse().unwrap_or(0))
            .collect()
    };
    let (mut a, mut b) = (numbers(a), numbers(b));
    let length = a.len().max(b.len());

    a.resize(length, 0);
    b.resize(length, 0);

    a.cmp(&b)
}

/// `X.Y.Z` with plain digits only, because the version ends up in file names.
fn parse_version(text: &str) -> Option<String> {
    let mut parts = text.split('.');
    let valid = |part: Option<&str>| {
        part.is_some_and(|part| {
            !part.is_empty() && part.len() <= 9 && part.bytes().all(|byte| byte.is_ascii_digit())
        })
    };

    (valid(parts.next()) && valid(parts.next()) && valid(parts.next()) && parts.next().is_none())
        .then(|| text.to_owned())
}

fn exe_name(version: &str) -> String {
    format!("sonear-{version}-update.exe")
}

fn signature_name(version: &str) -> String {
    format!("{}.minisig", exe_name(version))
}

fn part_name(version: &str) -> String {
    format!("{}.part", exe_name(version))
}

fn setup_name(version: &str) -> String {
    format!("sonear-{version}-setup.exe")
}

/// The version in a downloaded update's file name, for the `.exe` or the `.minisig`.
fn version_in_name(name: &str) -> Option<String> {
    let name = name.strip_suffix(".minisig").unwrap_or(name);

    parse_version(name.strip_prefix("sonear-")?.strip_suffix("-update.exe")?)
}

fn release_page(version: &str) -> String {
    format!("{RELEASE_PAGE_PREFIX}releases/tag/v{version}")
}

/// Checks `file` against `signature` and `public_key`, and that what the maker
/// signed alongside it says this is `version`. The second check is what stops an
/// old signed build from being passed off as a new one.
fn verify_file(
    public_key: &str,
    version: &str,
    file: &Path,
    signature: &str,
) -> Result<(), UpdateError> {
    let failed = |error: minisign_verify::Error| UpdateError::Signature(error.to_string());
    let key = PublicKey::from_base64(public_key).map_err(failed)?;
    let signature = Signature::decode(signature).map_err(failed)?;
    let mut verifier = key.verify_stream(&signature).map_err(failed)?;
    let mut input = File::open(file)?;
    let mut buffer = vec![0u8; VERIFY_CHUNK];

    loop {
        let read = input.read(&mut buffer)?;

        if read == 0 {
            break;
        }

        verifier.update(&buffer[..read]);
    }

    verifier.finalize().map_err(failed)?;

    let expected = format!("sonear {version}");
    let found = signature.trusted_comment();

    if found != expected {
        return Err(UpdateError::WrongVersion {
            expected: version.to_owned(),
            found: found.to_owned(),
        });
    }

    Ok(())
}

fn remove_quietly(path: &Path) {
    if let Err(error) = fs::remove_file(path)
        && error.kind() != io::ErrorKind::NotFound
    {
        debug!(%error, path = %path.display(), "could not remove an update file");
    }
}

pub struct Updater {
    client: reqwest::Client,
    releases_url: String,
    public_key: String,
    current: String,
    dir: PathBuf,
    require_https: bool,
    max_exe_bytes: u64,
    max_signature_bytes: u64,
}

impl Updater {
    /// The updater for this build, or `None` while no public key is compiled in.
    pub fn for_build(paths: &AppPaths) -> Option<Self> {
        let key = PUBLIC_KEY?;

        match Self::new(RELEASES_URL, key, VERSION, paths.update_dir(), true) {
            Ok(updater) => Some(updater),
            Err(error) => {
                warn!(%error, "could not set up the updater");

                None
            }
        }
    }

    /// `require_https` is only ever off in tests, which talk to a local server.
    pub fn new(
        releases_url: impl Into<String>,
        public_key: impl Into<String>,
        current_version: impl Into<String>,
        dir: impl Into<PathBuf>,
        require_https: bool,
    ) -> Result<Self, UpdateError> {
        let current = current_version.into();
        let client = reqwest::Client::builder()
            .user_agent(format!("Sonear/{current}"))
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(STALL_TIMEOUT)
            .redirect(Policy::custom(move |attempt| {
                if attempt.previous().len() >= MAX_REDIRECTS {
                    attempt.error("too many redirects")
                } else if require_https && attempt.url().scheme() != "https" {
                    attempt.error("redirected away from https")
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(network)?;

        Ok(Self {
            client,
            releases_url: releases_url.into(),
            public_key: public_key.into(),
            current,
            dir: dir.into(),
            require_https,
            max_exe_bytes: MAX_EXE_BYTES,
            max_signature_bytes: MAX_SIGNATURE_BYTES,
        })
    }

    /// Asks GitHub for the latest release. `None` when it is not newer, is
    /// missing a file, or GitHub would not say (not found, forbidden, rate limited).
    pub async fn check(&self) -> Result<Option<Release>, UpdateError> {
        let response = self
            .client
            .get(&self.releases_url)
            .header(header::ACCEPT, "application/vnd.github+json")
            .timeout(CHECK_TIMEOUT)
            .send()
            .await
            .map_err(network)?;
        let status = response.status();

        if matches!(
            status,
            StatusCode::NOT_FOUND | StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
        ) {
            info!(%status, "GitHub has no release information to give right now");

            return Ok(None);
        }

        if !status.is_success() {
            return Err(UpdateError::Status(status.as_u16()));
        }

        let body = read_capped(response, MAX_RELEASE_BYTES).await?;
        let release: ApiRelease = serde_json::from_slice(&body)
            .map_err(|error| UpdateError::Release(error.to_string()))?;

        Ok(self.newer(release))
    }

    fn newer(&self, release: ApiRelease) -> Option<Release> {
        let Some(version) = release.tag_name.strip_prefix('v').and_then(parse_version) else {
            debug!(tag = %release.tag_name, "the latest release has no usable version");

            return None;
        };

        if compare_versions(&version, &self.current) != Ordering::Greater {
            return None;
        }

        let asset = |name: String| {
            release
                .assets
                .iter()
                .find(|asset| asset.name == name)
                .map(|asset| asset.browser_download_url.clone())
                .filter(|url| self.allowed_url(url))
        };
        let (Some(exe_url), Some(signature_url)) = (
            asset(setup_name(&version)),
            asset(format!("{}.minisig", setup_name(&version))),
        ) else {
            debug!(%version, "the newer release is missing its file or signature");

            return None;
        };
        let notes_url = Some(release.html_url)
            .filter(|url| url.starts_with(RELEASE_PAGE_PREFIX))
            .or_else(|| Some(release_page(&version)));

        Some(Release {
            version,
            exe_url,
            signature_url,
            notes_url,
        })
    }

    fn allowed_url(&self, url: &str) -> bool {
        Url::parse(url).is_ok_and(|url| {
            url.host_str().is_some()
                && (url.scheme() == "https" || !self.require_https && url.scheme() == "http")
        })
    }

    /// Downloads `release` into the updates folder and verifies it. Whatever
    /// went wrong, nothing of it is left behind.
    pub async fn download(
        &self,
        release: &Release,
        progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> Result<Prepared, UpdateError> {
        let result = self.fetch_and_verify(release, progress).await;

        if result.is_err() {
            self.remove_version(&release.version);
        }

        result
    }

    async fn fetch_and_verify(
        &self,
        release: &Release,
        progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> Result<Prepared, UpdateError> {
        let version = release.version.as_str();

        if !self.allowed_url(&release.exe_url) || !self.allowed_url(&release.signature_url) {
            return Err(UpdateError::Release(
                "a download link is not https".to_owned(),
            ));
        }

        fs::create_dir_all(&self.dir)?;

        let response = self.get(&release.signature_url).await?;
        let signature = read_capped(response, self.max_signature_bytes).await?;
        let signature = String::from_utf8(signature)
            .map_err(|_| UpdateError::Signature("the signature is not text".to_owned()))?;
        let part = self.dir.join(part_name(version));

        self.save(&release.exe_url, &part, progress).await?;

        let key = self.public_key.clone();
        let checked = part.clone();
        let wanted = version.to_owned();
        let text = signature.clone();

        tokio::task::spawn_blocking(move || verify_file(&key, &wanted, &checked, &text))
            .await
            .map_err(|error| UpdateError::Signature(error.to_string()))??;

        let file = self.dir.join(exe_name(version));

        fs::write(self.dir.join(signature_name(version)), signature)?;
        fs::rename(&part, &file)?;

        Ok(Prepared {
            version: release.version.clone(),
            file,
            notes_url: release.notes_url.clone(),
        })
    }

    async fn get(&self, url: &str) -> Result<Response, UpdateError> {
        let response = self.client.get(url).send().await.map_err(network)?;

        if !response.status().is_success() {
            return Err(UpdateError::Status(response.status().as_u16()));
        }

        Ok(response)
    }

    async fn save(
        &self,
        url: &str,
        destination: &Path,
        progress: &(dyn Fn(u64, Option<u64>) + Send + Sync),
    ) -> Result<(), UpdateError> {
        let response = self.get(url).await?;
        let total = response.content_length();

        if total.is_some_and(|total| total > self.max_exe_bytes) {
            return Err(UpdateError::TooLarge);
        }

        let mut file = tokio::fs::File::create(destination).await?;
        let mut stream = response.bytes_stream();
        let mut received = 0u64;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(network)?;

            received += chunk.len() as u64;

            if received > self.max_exe_bytes {
                return Err(UpdateError::TooLarge);
            }

            file.write_all(&chunk).await?;
            progress(received, total);
        }

        file.flush().await?;
        file.sync_all().await?;

        Ok(())
    }

    /// Checks a download that is already on disk, with the signature beside it.
    pub fn verify_prepared(&self, prepared: &Prepared) -> Result<(), UpdateError> {
        let signature = fs::read_to_string(self.dir.join(signature_name(&prepared.version)))?;

        verify_file(
            &self.public_key,
            &prepared.version,
            &prepared.file,
            &signature,
        )
    }

    /// The newest verified download on disk that is newer than this version.
    /// Anything that no longer verifies is deleted. Reads whole files, so call
    /// it off the UI thread.
    pub fn prepared(&self) -> Option<Prepared> {
        let mut found = self.downloaded();

        found.sort_by(|a, b| compare_versions(&b.0, &a.0));

        for (version, file) in found {
            let prepared = Prepared {
                notes_url: Some(release_page(&version)),
                version,
                file,
            };

            match self.verify_prepared(&prepared) {
                Ok(()) => return Some(prepared),
                Err(error) => {
                    warn!(%error, version = %prepared.version, "a downloaded update is not valid, deleting it");
                    self.remove_version(&prepared.version);
                }
            }
        }

        None
    }

    /// Downloaded update files that are newer than this version, as version and path.
    fn downloaded(&self) -> Vec<(String, PathBuf)> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };

        entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;

                (!name.ends_with(".minisig")).then_some(())?;

                let version = version_in_name(&name)?;

                (compare_versions(&version, &self.current) == Ordering::Greater)
                    .then(|| (version, entry.path()))
            })
            .collect()
    }

    /// Removes what is left of old updates and interrupted downloads. Files
    /// that are still in use stay.
    pub fn cleanup(&self) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };

        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let keep = version_in_name(&name).is_some_and(|version| {
                compare_versions(&version, &self.current) == Ordering::Greater
            });

            if !keep && entry.path().is_file() {
                remove_quietly(&entry.path());
            }
        }
    }

    fn remove_version(&self, version: &str) {
        for name in [
            exe_name(version),
            signature_name(version),
            part_name(version),
        ] {
            remove_quietly(&self.dir.join(name));
        }
    }

    /// Checks for updates about ten seconds from now and then every six hours,
    /// while `enabled` is true, and reports through `events`. Uses timers on the
    /// runtime only. Ends when `enabled` or `events` is dropped.
    pub async fn run(
        self: Arc<Self>,
        mut enabled: watch::Receiver<bool>,
        events: mpsc::UnboundedSender<UpdateEvent>,
    ) {
        let housekeeper = self.clone();

        let _ = tokio::task::spawn_blocking(move || housekeeper.cleanup()).await;

        let mut ready: Option<String> = None;
        let mut first = true;

        loop {
            while !*enabled.borrow_and_update() {
                if enabled.changed().await.is_err() {
                    return;
                }

                first = true;
            }

            if first && ready.is_none() {
                let finder = self.clone();

                if let Ok(Some(prepared)) =
                    tokio::task::spawn_blocking(move || finder.prepared()).await
                {
                    ready = Some(prepared.version.clone());

                    if events.send(UpdateEvent::Ready(prepared)).is_err() {
                        return;
                    }
                }
            }

            let wait = if first {
                FIRST_CHECK_AFTER
            } else {
                CHECK_EVERY
            };

            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                changed = enabled.changed() => {
                    if changed.is_err() {
                        return;
                    }

                    first = true;

                    continue;
                }
            }

            first = false;

            if !*enabled.borrow() {
                continue;
            }

            if self.check_once(&mut ready, &events).await.is_err() {
                return;
            }
        }
    }

    /// One check and, if there is something new, its download. Fails only when
    /// nobody listens any more.
    async fn check_once(
        &self,
        ready: &mut Option<String>,
        events: &mpsc::UnboundedSender<UpdateEvent>,
    ) -> Result<(), ()> {
        let release = match self.check().await {
            Ok(Some(release)) => release,
            Ok(None) => return Ok(()),
            Err(error) => {
                warn!(%error, "could not check for updates");

                return Ok(());
            }
        };

        if ready
            .as_deref()
            .is_some_and(|version| compare_versions(version, &release.version) != Ordering::Less)
        {
            return Ok(());
        }

        info!(version = %release.version, "downloading an update");

        let last = Mutex::new(Instant::now() - PROGRESS_EVERY);
        let report = |received, total| {
            let mut last = last.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

            if last.elapsed() >= PROGRESS_EVERY {
                *last = Instant::now();
                let _ = events.send(UpdateEvent::Downloading { received, total });
            }
        };

        match self.download(&release, &report).await {
            Ok(prepared) => {
                info!(version = %prepared.version, "an update is ready");
                *ready = Some(prepared.version.clone());
                events.send(UpdateEvent::Ready(prepared)).map_err(|_| ())
            }
            Err(error) => {
                if error.is_untrusted() {
                    error!(%error, version = %release.version, "a downloaded update failed its signature check");
                } else {
                    warn!(%error, "could not download the update");
                }

                events.send(UpdateEvent::Idle).map_err(|_| ())
            }
        }
    }
}

/// The body of `response`, refusing to read more than `cap` bytes.
async fn read_capped(response: Response, cap: u64) -> Result<Vec<u8>, UpdateError> {
    if response.content_length().is_some_and(|length| length > cap) {
        return Err(UpdateError::TooLarge);
    }

    let mut stream = response.bytes_stream();
    let mut body = Vec::new();

    while let Some(chunk) = stream.next().await {
        body.extend_from_slice(&chunk.map_err(network)?);

        if body.len() as u64 > cap {
            return Err(UpdateError::TooLarge);
        }
    }

    Ok(body)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::os::windows::fs::OpenOptionsExt as _;

    use minisign::KeyPair;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    const CURRENT: &str = "0.1.0";

    struct Keys(KeyPair);

    impl Keys {
        fn new() -> Self {
            Self(KeyPair::generate_unencrypted_keypair().unwrap())
        }

        fn public(&self) -> String {
            self.0.pk.to_base64()
        }

        fn sign(&self, bytes: &[u8], comment: &str) -> String {
            minisign::sign(
                Some(&self.0.pk),
                &self.0.sk,
                Cursor::new(bytes),
                Some(comment),
                Some("signature from a test key"),
            )
            .unwrap()
            .into_string()
        }
    }

    fn updater(server: &MockServer, key: &str, dir: &Path) -> Updater {
        let releases = format!("{}/releases/latest", server.uri());

        Updater::new(releases, key, CURRENT, dir, false).unwrap()
    }

    async fn serve_release(server: &MockServer, tag: &str, files: &[&str]) {
        let assets: Vec<_> = files
            .iter()
            .map(|name| {
                serde_json::json!({
                    "name": name,
                    "browser_download_url": format!("{}/files/{name}", server.uri()),
                })
            })
            .collect();

        Mock::given(method("GET"))
            .and(path("/releases/latest"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "tag_name": tag,
                "html_url": format!("{RELEASE_PAGE_PREFIX}releases/tag/{tag}"),
                "assets": assets,
            })))
            .mount(server)
            .await;
    }

    async fn serve_file(server: &MockServer, name: &str, bytes: Vec<u8>) {
        Mock::given(method("GET"))
            .and(path(format!("/files/{name}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
            .mount(server)
            .await;
    }

    /// Serves a release of `version` whose files are `exe` and `signature`.
    async fn serve_update(server: &MockServer, version: &str, exe: &[u8], signature: &str) {
        let setup = setup_name(version);
        let sig = format!("{setup}.minisig");

        serve_release(server, &format!("v{version}"), &[&setup, &sig]).await;
        serve_file(server, &setup, exe.to_vec()).await;
        serve_file(server, &sig, signature.as_bytes().to_vec()).await;
    }

    async fn latest(updater: &Updater) -> Release {
        updater.check().await.unwrap().unwrap()
    }

    fn quiet(_: u64, _: Option<u64>) {}

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();

        names.sort();

        names
    }

    #[test]
    fn the_built_in_key_can_be_read() {
        let key = PUBLIC_KEY.expect("releases are signed with this key");

        assert!(PublicKey::from_base64(key).is_ok());
    }

    #[test]
    fn versions_compare_as_numbers() {
        assert_eq!(compare_versions("0.9.0", "0.10.0"), Ordering::Less);
        assert_eq!(compare_versions("0.2.0", "0.1.0"), Ordering::Greater);
        assert_eq!(compare_versions("1.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("", "0.1.0"), Ordering::Less);
    }

    #[test]
    fn only_plain_versions_are_accepted() {
        assert_eq!(parse_version("0.2.0").as_deref(), Some("0.2.0"));
        assert_eq!(parse_version("10.20.30").as_deref(), Some("10.20.30"));

        for bad in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "1.2.x",
            "1.2.3-beta",
            "..\\1.2",
            "1.2.3/..",
            "1..3",
            "+1.2.3",
        ] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
    }

    #[test]
    fn update_file_names_carry_their_version() {
        assert_eq!(
            version_in_name("sonear-0.2.0-update.exe").as_deref(),
            Some("0.2.0")
        );
        assert_eq!(
            version_in_name("sonear-0.2.0-update.exe.minisig").as_deref(),
            Some("0.2.0")
        );
        assert_eq!(version_in_name("sonear-0.2.0-update.exe.part"), None);
        assert_eq!(version_in_name("sonear-0.2.0-setup.exe"), None);
        assert_eq!(version_in_name("notes.txt"), None);
    }

    #[tokio::test]
    async fn a_newer_release_is_found() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();

        serve_update(&server, "0.2.0", b"exe", "sig").await;

        let release = latest(&updater(&server, "key", dir.path())).await;

        assert_eq!(release.version, "0.2.0");
        assert!(release.exe_url.ends_with("/files/sonear-0.2.0-setup.exe"));
        assert!(
            release
                .signature_url
                .ends_with("/files/sonear-0.2.0-setup.exe.minisig")
        );
        assert_eq!(
            release.notes_url.as_deref(),
            Some("https://github.com/HypoxStudios/sonear/releases/tag/v0.2.0")
        );
    }

    #[tokio::test]
    async fn the_same_or_an_older_release_is_not_an_update() {
        let dir = tempfile::tempdir().unwrap();

        for version in ["0.1.0", "0.0.9"] {
            let server = MockServer::start().await;

            serve_update(&server, version, b"exe", "sig").await;

            assert_eq!(
                updater(&server, "key", dir.path()).check().await.unwrap(),
                None,
                "{version}"
            );
        }
    }

    #[tokio::test]
    async fn a_release_missing_its_file_or_signature_is_not_an_update() {
        let dir = tempfile::tempdir().unwrap();

        for files in [
            vec!["sonear-0.2.0-setup.exe"],
            vec!["sonear-0.2.0-setup.exe.minisig"],
            vec!["sonear-0.3.0-setup.exe", "sonear-0.3.0-setup.exe.minisig"],
            vec![],
        ] {
            let server = MockServer::start().await;

            serve_release(&server, "v0.2.0", &files).await;

            assert_eq!(
                updater(&server, "key", dir.path()).check().await.unwrap(),
                None,
                "{files:?}"
            );
        }
    }

    #[tokio::test]
    async fn github_saying_no_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();

        for status in [404, 403, 429] {
            let server = MockServer::start().await;

            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;

            assert_eq!(
                updater(&server, "key", dir.path()).check().await.unwrap(),
                None,
                "{status}"
            );
        }

        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        assert!(matches!(
            updater(&server, "key", dir.path()).check().await,
            Err(UpdateError::Status(500))
        ));
    }

    #[tokio::test]
    async fn a_strange_tag_or_body_is_no_update() {
        let dir = tempfile::tempdir().unwrap();

        for tag in ["nightly", "0.2.0", "v0.2", "v0.2.0-rc1", "v..\\0.2.0"] {
            let server = MockServer::start().await;

            serve_release(&server, tag, &["sonear-0.2.0-setup.exe"]).await;

            assert_eq!(
                updater(&server, "key", dir.path()).check().await.unwrap(),
                None,
                "{tag}"
            );
        }

        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>"))
            .mount(&server)
            .await;

        assert!(matches!(
            updater(&server, "key", dir.path()).check().await,
            Err(UpdateError::Release(_))
        ));
    }

    #[tokio::test]
    async fn plain_http_links_are_refused() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();

        serve_update(&server, "0.2.0", b"exe", "sig").await;

        let strict = Updater::new(
            format!("{}/releases/latest", server.uri()),
            "key",
            CURRENT,
            dir.path(),
            true,
        )
        .unwrap();

        assert_eq!(strict.check().await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_release_page_elsewhere_is_replaced_by_ours() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let setup = "sonear-0.2.0-setup.exe";

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "tag_name": "v0.2.0",
                "html_url": "https://evil.example/sonear/",
                "assets": [
                    {"name": setup, "browser_download_url": format!("{}/files/{setup}", server.uri())},
                    {"name": format!("{setup}.minisig"), "browser_download_url": format!("{}/files/s", server.uri())},
                ],
            })))
            .mount(&server)
            .await;

        let release = latest(&updater(&server, "key", dir.path())).await;

        assert_eq!(
            release.notes_url.as_deref(),
            Some("https://github.com/HypoxStudios/sonear/releases/tag/v0.2.0")
        );
    }

    #[tokio::test]
    async fn a_signed_download_is_kept_and_verified() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let keys = Keys::new();
        let exe = vec![7u8; 3_000_000];

        serve_update(&server, "0.2.0", &exe, &keys.sign(&exe, "sonear 0.2.0")).await;

        let updater = updater(&server, &keys.public(), dir.path());
        let release = latest(&updater).await;
        let seen = Mutex::new(0u64);
        let prepared = updater
            .download(&release, &|received, _| {
                *seen.lock().unwrap() = received;
            })
            .await
            .unwrap();

        assert_eq!(prepared.version, "0.2.0");
        assert_eq!(fs::read(&prepared.file).unwrap(), exe);
        assert_eq!(*seen.lock().unwrap(), 3_000_000);
        assert_eq!(
            files_in(dir.path()),
            ["sonear-0.2.0-update.exe", "sonear-0.2.0-update.exe.minisig"]
        );
        assert!(updater.verify_prepared(&prepared).is_ok());
    }

    #[tokio::test]
    async fn a_tampered_download_is_deleted() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let keys = Keys::new();
        let signature = keys.sign(b"the real file", "sonear 0.2.0");

        serve_update(&server, "0.2.0", b"a different file", &signature).await;

        let updater = updater(&server, &keys.public(), dir.path());
        let release = latest(&updater).await;
        let result = updater.download(&release, &quiet).await;

        assert!(matches!(result, Err(UpdateError::Signature(_))));
        assert!(files_in(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn a_download_signed_by_another_key_is_deleted() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let signature = Keys::new().sign(b"exe", "sonear 0.2.0");

        serve_update(&server, "0.2.0", b"exe", &signature).await;

        let updater = updater(&server, &Keys::new().public(), dir.path());
        let release = latest(&updater).await;
        let result = updater.download(&release, &quiet).await;

        assert!(matches!(result, Err(UpdateError::Signature(_))));
        assert!(files_in(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn an_old_signed_build_cannot_pass_as_a_new_version() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let keys = Keys::new();
        let signature = keys.sign(b"exe", "sonear 0.1.5");

        serve_update(&server, "0.2.0", b"exe", &signature).await;

        let updater = updater(&server, &keys.public(), dir.path());
        let release = latest(&updater).await;
        let result = updater.download(&release, &quiet).await;

        assert!(matches!(
            result,
            Err(UpdateError::WrongVersion { ref found, .. }) if found == "sonear 0.1.5"
        ));
        assert!(files_in(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn the_trusted_comment_must_match_exactly() {
        let keys = Keys::new();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("exe");

        fs::write(&file, b"exe").unwrap();

        for comment in [
            "sonear 0.2.0 ",
            "Sonear 0.2.0",
            "sonear v0.2.0",
            "sonear 0.2.0.1",
            "",
        ] {
            let result = verify_file(&keys.public(), "0.2.0", &file, &keys.sign(b"exe", comment));

            assert!(
                matches!(result, Err(UpdateError::WrongVersion { .. })),
                "{comment:?}"
            );
        }

        assert!(
            verify_file(
                &keys.public(),
                "0.2.0",
                &file,
                &keys.sign(b"exe", "sonear 0.2.0")
            )
            .is_ok()
        );
    }

    #[test]
    fn a_signature_made_by_a_minisign_cli_is_understood() {
        const KEY: &str = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
        const LEGACY: &str = "untrusted comment: signature from minisign secret key\nRWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=\ntrusted comment: timestamp:1555779966\tfile:test\nQtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==";

        let key = PublicKey::from_base64(KEY).unwrap();
        let signature = Signature::decode(LEGACY).unwrap();

        key.verify(b"test", &signature, true).unwrap();
        assert_eq!(
            signature.trusted_comment(),
            "timestamp:1555779966\tfile:test"
        );
        assert!(key.verify_stream(&signature).is_err());
    }

    #[tokio::test]
    async fn an_oversize_download_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let keys = Keys::new();
        let exe = vec![1u8; 2048];

        for (exe_cap, signature_cap) in [(1024, MAX_SIGNATURE_BYTES), (MAX_EXE_BYTES, 16)] {
            let server = MockServer::start().await;

            serve_update(&server, "0.2.0", &exe, &keys.sign(&exe, "sonear 0.2.0")).await;

            let mut updater = updater(&server, &keys.public(), dir.path());

            updater.max_exe_bytes = exe_cap;
            updater.max_signature_bytes = signature_cap;

            let release = latest(&updater).await;
            let result = updater.download(&release, &quiet).await;

            assert!(matches!(result, Err(UpdateError::TooLarge)));
            assert!(files_in(dir.path()).is_empty());
        }
    }

    #[tokio::test]
    async fn a_failed_request_leaves_nothing() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();

        serve_release(
            &server,
            "v0.2.0",
            &["sonear-0.2.0-setup.exe", "sonear-0.2.0-setup.exe.minisig"],
        )
        .await;

        let updater = updater(&server, "key", dir.path());
        let release = latest(&updater).await;

        assert!(matches!(
            updater.download(&release, &quiet).await,
            Err(UpdateError::Status(404))
        ));
        assert!(files_in(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn a_prepared_download_is_reused() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let keys = Keys::new();

        serve_update(&server, "0.2.0", b"exe", &keys.sign(b"exe", "sonear 0.2.0")).await;

        let first = updater(&server, &keys.public(), dir.path());
        let release = latest(&first).await;
        let downloaded = first.download(&release, &quiet).await.unwrap();
        let again = updater(&server, &keys.public(), dir.path());
        let reused = again.prepared().unwrap();

        assert_eq!(reused.version, "0.2.0");
        assert_eq!(reused.file, downloaded.file);
        assert_eq!(
            reused.notes_url.as_deref(),
            Some("https://github.com/HypoxStudios/sonear/releases/tag/v0.2.0")
        );
    }

    #[tokio::test]
    async fn a_prepared_download_that_no_longer_verifies_is_deleted() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let keys = Keys::new();

        serve_update(&server, "0.2.0", b"exe", &keys.sign(b"exe", "sonear 0.2.0")).await;

        let updater = updater(&server, &keys.public(), dir.path());
        let release = latest(&updater).await;
        let prepared = updater.download(&release, &quiet).await.unwrap();

        fs::write(&prepared.file, b"swapped").unwrap();

        assert!(updater.verify_prepared(&prepared).is_err());
        assert_eq!(updater.prepared(), None);
        assert!(files_in(dir.path()).is_empty());
    }

    #[test]
    fn a_prepared_download_that_is_not_newer_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let keys = Keys::new();
        let updater =
            Updater::new("http://unused", keys.public(), "0.2.0", dir.path(), true).unwrap();

        fs::write(dir.path().join("sonear-0.2.0-update.exe"), b"exe").unwrap();
        fs::write(
            dir.path().join("sonear-0.2.0-update.exe.minisig"),
            keys.sign(b"exe", "sonear 0.2.0"),
        )
        .unwrap();

        assert_eq!(updater.prepared(), None);
        assert_eq!(files_in(dir.path()).len(), 2);
    }

    #[test]
    fn cleanup_keeps_only_newer_updates() {
        let dir = tempfile::tempdir().unwrap();
        let updater = Updater::new("http://unused", "key", CURRENT, dir.path(), true).unwrap();

        for name in [
            "sonear-0.0.9-update.exe",
            "sonear-0.0.9-update.exe.minisig",
            "sonear-0.1.0-update.exe",
            "sonear-0.1.0-update.exe.minisig",
            "sonear-0.2.0-update.exe",
            "sonear-0.2.0-update.exe.minisig",
            "sonear-0.3.0-update.exe.part",
            "stray.txt",
        ] {
            fs::write(dir.path().join(name), b"x").unwrap();
        }

        fs::create_dir(dir.path().join("sub")).unwrap();
        updater.cleanup();

        assert_eq!(
            files_in(dir.path()),
            [
                "sonear-0.2.0-update.exe",
                "sonear-0.2.0-update.exe.minisig",
                "sub"
            ]
        );
    }

    #[test]
    fn cleanup_ignores_a_file_that_is_locked() {
        let dir = tempfile::tempdir().unwrap();
        let updater = Updater::new("http://unused", "key", CURRENT, dir.path(), true).unwrap();
        let old = dir.path().join("sonear-0.1.0-update.exe");

        fs::write(&old, b"x").unwrap();

        let _open = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&old)
            .unwrap();

        updater.cleanup();

        assert!(old.exists());
    }

    #[tokio::test]
    async fn a_check_downloads_and_reports_the_update_once() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let keys = Keys::new();

        serve_update(&server, "0.2.0", b"exe", &keys.sign(b"exe", "sonear 0.2.0")).await;

        let updater = updater(&server, &keys.public(), dir.path());
        let (events, mut inbox) = mpsc::unbounded_channel();
        let mut ready = None;

        updater.check_once(&mut ready, &events).await.unwrap();
        updater.check_once(&mut ready, &events).await.unwrap();
        drop(events);

        let mut received = Vec::new();

        while let Some(event) = inbox.recv().await {
            received.push(event);
        }

        let readies = received
            .iter()
            .filter(|event| matches!(event, UpdateEvent::Ready(prepared) if prepared.version == "0.2.0"))
            .count();

        assert_eq!(readies, 1);
        assert!(
            received
                .iter()
                .all(|event| !matches!(event, UpdateEvent::Idle))
        );
        assert_eq!(ready.as_deref(), Some("0.2.0"));
    }

    #[tokio::test]
    async fn a_bad_download_reports_idle() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let keys = Keys::new();

        serve_update(
            &server,
            "0.2.0",
            b"exe",
            &keys.sign(b"other", "sonear 0.2.0"),
        )
        .await;

        let updater = updater(&server, &keys.public(), dir.path());
        let (events, mut inbox) = mpsc::unbounded_channel();
        let mut ready = None;

        updater.check_once(&mut ready, &events).await.unwrap();
        drop(events);

        let mut received = Vec::new();

        while let Some(event) = inbox.recv().await {
            received.push(event);
        }

        assert_eq!(received.last(), Some(&UpdateEvent::Idle));
        assert_eq!(ready, None);
        assert!(files_in(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn the_task_stays_quiet_while_the_setting_is_off() {
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let updater = Arc::new(updater(&server, "key", dir.path()));
        let (enabled, watcher) = watch::channel(false);
        let (events, mut inbox) = mpsc::unbounded_channel();
        let task = tokio::spawn(updater.run(watcher, events));

        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(server.received_requests().await.unwrap().is_empty());
        assert!(inbox.try_recv().is_err());

        drop(enabled);
        task.await.unwrap();
    }
}
