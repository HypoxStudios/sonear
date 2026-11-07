//! Downloads, verifies and installs Soloist. Only runs when the user asks.

#[cfg(windows)]
mod wintrust;

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::Url;
use reqwest::redirect::Policy;
use rust_i18n::t;
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

use crate::version::{SoloistVersion, VersionError};

#[cfg(windows)]
pub use wintrust::WinTrustVerifier;

pub const DOWNLOAD_URL: &str =
    "https://soloist-builds.spotifycdn.com/soloist_release_windows_x86_64.zip";
pub const ALLOWED_HOST: &str = "soloist-builds.spotifycdn.com";
pub const EXE_NAME: &str = "soloist.exe";
pub const REQUIRED_SIGNER: &str = "Spotify AB";

const EXTRACTED_FILES: [&str; 3] = [EXE_NAME, "CHANGELOG.md", "THIRD_PARTY_LICENSES.txt"];
const MAX_EXTRACTED_BYTES: u64 = 512 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const STALL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Error)]
pub enum InstallError {
    #[error(
        "refused to download from `{0}`: only soloist-builds.spotifycdn.com over HTTPS is allowed"
    )]
    HostNotAllowed(String),
    #[error("the download failed: {0}")]
    Download(String),
    #[error("the downloaded archive could not be unpacked: {0}")]
    Extract(String),
    #[error("the archive does not contain soloist.exe")]
    MissingExe,
    #[error("the signature check failed: {0}")]
    SignatureInvalid(String),
    #[error("soloist.exe is signed by `{0}`, not by Spotify AB")]
    WrongSigner(String),
    #[error("the downloaded soloist.exe does not run: {0}")]
    Probe(#[from] VersionError),
    #[error("could not replace the installed Soloist: {0}")]
    Swap(String),
    #[error("file error: {0}")]
    Io(#[from] io::Error),
}

impl InstallError {
    /// The error in the interface language. `Display` stays English for logs.
    pub fn user_message(&self) -> String {
        match self {
            Self::HostNotAllowed(url) => t!("install.host_not_allowed", url = url),
            Self::Download(detail) => t!("install.download", detail = detail),
            Self::Extract(detail) => t!("install.extract", detail = detail),
            Self::MissingExe => t!("install.missing_exe"),
            Self::SignatureInvalid(detail) => t!("install.signature_invalid", detail = detail),
            Self::WrongSigner(signer) => t!("install.wrong_signer", signer = signer),
            Self::Probe(error) => t!("install.probe", detail = error.user_message()),
            Self::Swap(detail) => t!("install.swap", detail = detail),
            Self::Io(error) => t!("install.io", detail = error.to_string()),
        }
        .into_owned()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallProgress {
    Downloading { received: u64, total: Option<u64> },
    Verifying,
    Finalizing,
    Done,
}

/// Checks the Authenticode signature of a file.
pub trait SignatureVerifier: Send + Sync {
    /// Returns the signer's subject name when the signature is valid.
    fn verify(&self, exe: &Path) -> Result<String, InstallError>;
}

#[async_trait]
pub trait VersionProbe: Send + Sync {
    async fn probe(&self, exe: &Path) -> Result<SoloistVersion, VersionError>;
}

pub struct ProcessProbe;

#[async_trait]
impl VersionProbe for ProcessProbe {
    async fn probe(&self, exe: &Path) -> Result<SoloistVersion, VersionError> {
        crate::version::probe(exe).await
    }
}

pub fn is_allowed_url(url: &Url) -> bool {
    url.scheme() == "https" && url.host_str() == Some(ALLOWED_HOST)
}

/// A verified build waiting to be swapped in. Dropping it deletes the download.
pub struct PreparedInstall {
    work: PathBuf,
    staging: PathBuf,
    version: SoloistVersion,
}

impl Drop for PreparedInstall {
    fn drop(&mut self) {
        remove_work_dir(&self.work);
    }
}

fn remove_work_dir(work: &Path) {
    if let Err(error) = fs::remove_dir_all(work) {
        warn!(%error, "could not remove the install work directory");
    }
}

pub struct Installer {
    root: PathBuf,
    verifier: Arc<dyn SignatureVerifier>,
    probe: Arc<dyn VersionProbe>,
}

impl Installer {
    /// `root` is the Soloist directory that holds `bin/`.
    pub fn new(
        root: impl Into<PathBuf>,
        verifier: Arc<dyn SignatureVerifier>,
        probe: Arc<dyn VersionProbe>,
    ) -> Self {
        Self {
            root: root.into(),
            verifier,
            probe,
        }
    }

    pub fn bin_dir(&self) -> PathBuf {
        self.root.join("bin")
    }

    pub fn exe_path(&self) -> PathBuf {
        self.bin_dir().join(EXE_NAME)
    }

    /// Downloads and verifies a build without touching the installed one, so
    /// a running Soloist can keep playing until `commit`.
    pub async fn prepare(
        &self,
        url: &str,
        progress: &(dyn Fn(InstallProgress) + Send + Sync),
    ) -> Result<PreparedInstall, InstallError> {
        let url = Url::parse(url).map_err(|_| InstallError::HostNotAllowed(url.to_owned()))?;

        if !is_allowed_url(&url) {
            return Err(InstallError::HostNotAllowed(
                url.host_str().unwrap_or("unknown host").to_owned(),
            ));
        }

        let work = self.work_dir();

        fs::create_dir_all(&work)?;

        match self.download_and_verify(&work, url, progress).await {
            Ok(version) => Ok(PreparedInstall {
                staging: work.join("staging"),
                work,
                version,
            }),
            Err(error) => {
                remove_work_dir(&work);

                Err(error)
            }
        }
    }

    async fn download_and_verify(
        &self,
        work: &Path,
        url: Url,
        progress: &(dyn Fn(InstallProgress) + Send + Sync),
    ) -> Result<SoloistVersion, InstallError> {
        let archive = work.join("soloist.zip");

        download(url, &archive, progress).await?;

        self.verify_archive(&archive, work, progress).await
    }

    async fn verify_archive(
        &self,
        archive: &Path,
        work: &Path,
        progress: &(dyn Fn(InstallProgress) + Send + Sync),
    ) -> Result<SoloistVersion, InstallError> {
        progress(InstallProgress::Verifying);

        let staging = work.join("staging");
        let archive = archive.to_owned();
        let target = staging.clone();

        tokio::task::spawn_blocking(move || extract(&archive, &target))
            .await
            .map_err(|error| InstallError::Extract(error.to_string()))??;

        let exe = staging.join(EXE_NAME);
        let verifier = self.verifier.clone();
        let checked = exe.clone();
        let signer = tokio::task::spawn_blocking(move || verifier.verify(&checked))
            .await
            .map_err(|error| InstallError::SignatureInvalid(error.to_string()))??;

        if signer != REQUIRED_SIGNER {
            return Err(InstallError::WrongSigner(signer));
        }

        Ok(self.probe.probe(&exe).await?)
    }

    /// Swaps a prepared build in. Soloist must be stopped first.
    pub async fn commit(
        &self,
        prepared: PreparedInstall,
        progress: &(dyn Fn(InstallProgress) + Send + Sync),
    ) -> Result<SoloistVersion, InstallError> {
        progress(InstallProgress::Finalizing);

        let staging = prepared.staging.clone();
        let bin = self.bin_dir();

        tokio::task::spawn_blocking(move || swap_in(&staging, &bin))
            .await
            .map_err(|error| InstallError::Swap(error.to_string()))??;

        info!(version = %prepared.version.version, "installed Soloist");
        progress(InstallProgress::Done);

        Ok(prepared.version.clone())
    }

    fn work_dir(&self) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or_default();

        self.root
            .join(format!("install-{}-{stamp}", std::process::id()))
    }
}

async fn download(
    url: Url,
    destination: &Path,
    progress: &(dyn Fn(InstallProgress) + Send + Sync),
) -> Result<(), InstallError> {
    let client = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(STALL_TIMEOUT)
        .redirect(Policy::custom(|attempt| {
            if is_allowed_url(attempt.url()) && attempt.previous().len() < 5 {
                attempt.follow()
            } else {
                attempt.error("redirected away from soloist-builds.spotifycdn.com")
            }
        }))
        .build()
        .map_err(|error| InstallError::Download(error.to_string()))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| InstallError::Download(error.to_string()))?;

    if !response.status().is_success() {
        return Err(InstallError::Download(format!(
            "server answered {}",
            response.status()
        )));
    }

    if !is_allowed_url(response.url()) {
        return Err(InstallError::HostNotAllowed(
            response
                .url()
                .host_str()
                .unwrap_or("unknown host")
                .to_owned(),
        ));
    }

    let total = response.content_length();
    let mut file = tokio::fs::File::create(destination).await?;
    let mut stream = response.bytes_stream();
    let mut received = 0u64;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| InstallError::Download(error.to_string()))?;

        file.write_all(&chunk).await?;
        received += chunk.len() as u64;
        progress(InstallProgress::Downloading { received, total });
    }

    file.flush().await?;
    file.sync_all().await?;

    Ok(())
}

fn extract(archive: &Path, target: &Path) -> Result<(), InstallError> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)
        .map_err(|error| InstallError::Extract(error.to_string()))?;
    let mut found_exe = false;
    let mut total = 0u64;

    fs::create_dir_all(target)?;

    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| InstallError::Extract(error.to_string()))?;

        if entry.is_dir() {
            continue;
        }

        let Some(name) = entry
            .enclosed_name()
            .and_then(|path| path.file_name().map(|name| name.to_owned()))
            .and_then(|name| name.to_str().map(str::to_owned))
        else {
            continue;
        };

        if !EXTRACTED_FILES.contains(&name.as_str()) {
            continue;
        }

        total += entry.size();

        if total > MAX_EXTRACTED_BYTES {
            return Err(InstallError::Extract(
                "archive is unexpectedly large".into(),
            ));
        }

        let mut out = File::create(target.join(&name))?;

        io::copy(&mut entry, &mut out)?;
        out.sync_all()?;
        found_exe |= name == EXE_NAME;
    }

    if !found_exe {
        return Err(InstallError::MissingExe);
    }

    Ok(())
}

/// Moves each staged file into `bin`, keeping the previous version as
/// `<name>.old` until every move succeeded, and restoring it if one fails.
fn swap_in(staging: &Path, bin: &Path) -> Result<(), InstallError> {
    fs::create_dir_all(bin)?;

    let mut moved: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();

    for name in EXTRACTED_FILES {
        let source = staging.join(name);

        if !source.exists() {
            continue;
        }

        let destination = bin.join(name);
        let backup = bin.join(format!("{name}.old"));
        let had_previous = destination.exists();

        if had_previous {
            let _ = fs::remove_file(&backup);

            if let Err(error) = fs::rename(&destination, &backup) {
                rollback(&moved);
                return Err(InstallError::Swap(error.to_string()));
            }
        }

        if let Err(error) = fs::rename(&source, &destination) {
            if had_previous {
                let _ = fs::rename(&backup, &destination);
            }

            rollback(&moved);

            return Err(InstallError::Swap(error.to_string()));
        }

        moved.push((destination, had_previous.then_some(backup)));
    }

    for (_, backup) in &moved {
        if let Some(backup) = backup {
            let _ = fs::remove_file(backup);
        }
    }

    Ok(())
}

fn rollback(moved: &[(PathBuf, Option<PathBuf>)]) {
    for (destination, backup) in moved.iter().rev() {
        let _ = fs::remove_file(destination);

        if let Some(backup) = backup {
            let _ = fs::rename(backup, destination);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    struct FakeVerifier(Result<String, String>);

    impl SignatureVerifier for FakeVerifier {
        fn verify(&self, _: &Path) -> Result<String, InstallError> {
            self.0.clone().map_err(InstallError::SignatureInvalid)
        }
    }

    struct FakeProbe(Result<(), String>);

    #[async_trait]
    impl VersionProbe for FakeProbe {
        async fn probe(&self, _: &Path) -> Result<SoloistVersion, VersionError> {
            self.0.clone().map_err(VersionError::Run)?;

            SoloistVersion::parse("soloist 1.4.0.1 build 1 (20270101) (gabc) (windows/x86_64)")
        }
    }

    fn installer(root: &Path, signer: Result<&str, &str>, probe: Result<(), &str>) -> Installer {
        Installer::new(
            root,
            Arc::new(FakeVerifier(
                signer.map(str::to_owned).map_err(str::to_owned),
            )),
            Arc::new(FakeProbe(probe.map_err(str::to_owned))),
        )
    }

    fn make_zip(path: &Path, files: &[(&str, &[u8])]) {
        let mut writer = zip::ZipWriter::new(File::create(path).unwrap());

        for (name, content) in files {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(content).unwrap();
        }

        writer.finish().unwrap();
    }

    fn seed_existing(installer: &Installer) {
        fs::create_dir_all(installer.bin_dir()).unwrap();
        fs::write(installer.exe_path(), b"old exe").unwrap();
    }

    async fn run(installer: &Installer, root: &Path) -> Result<SoloistVersion, InstallError> {
        let work = root.join("work");
        let archive = root.join("soloist.zip");

        fs::create_dir_all(&work).unwrap();

        match installer.verify_archive(&archive, &work, &|_| {}).await {
            Ok(version) => {
                let prepared = PreparedInstall {
                    staging: work.join("staging"),
                    work,
                    version,
                };

                installer.commit(prepared, &|_| {}).await
            }
            Err(error) => {
                let _ = fs::remove_dir_all(&work);

                Err(error)
            }
        }
    }

    #[tokio::test]
    async fn rejects_other_hosts_and_plain_http() {
        let dir = tempfile::tempdir().unwrap();
        let installer = installer(dir.path(), Ok("Spotify AB"), Ok(()));
        let noop = |_: InstallProgress| {};

        for url in [
            "https://evil.example/soloist.zip",
            "http://soloist-builds.spotifycdn.com/soloist.zip",
            "https://soloist-builds.spotifycdn.com.evil.example/x.zip",
            "not a url",
        ] {
            let result = installer.prepare(url, &noop).await;

            assert!(
                matches!(result, Err(InstallError::HostNotAllowed(_))),
                "{url}"
            );
        }

        assert!(!installer.exe_path().exists());
    }

    #[test]
    fn allows_only_the_spotify_cdn() {
        assert!(is_allowed_url(&Url::parse(DOWNLOAD_URL).unwrap()));
        assert!(!is_allowed_url(
            &Url::parse("https://cdn.example/x").unwrap()
        ));
    }

    #[tokio::test]
    async fn successful_install_swaps_the_exe() {
        let dir = tempfile::tempdir().unwrap();
        let installer = installer(dir.path(), Ok("Spotify AB"), Ok(()));

        seed_existing(&installer);
        make_zip(
            &dir.path().join("soloist.zip"),
            &[
                ("soloist.exe", b"new exe"),
                ("CHANGELOG.md", b"log"),
                ("nested/ignored.txt", b"x"),
            ],
        );

        let version = run(&installer, dir.path()).await.unwrap();

        assert_eq!(version.version, "1.4.0.1");
        assert_eq!(fs::read(installer.exe_path()).unwrap(), b"new exe");
        assert!(!installer.bin_dir().join("soloist.exe.old").exists());
        assert!(installer.bin_dir().join("CHANGELOG.md").exists());
        assert!(!installer.bin_dir().join("ignored.txt").exists());
        assert!(!dir.path().join("work").exists());
    }

    #[tokio::test]
    async fn failed_signature_leaves_existing_install_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let installer = installer(dir.path(), Err("no signature"), Ok(()));

        seed_existing(&installer);
        make_zip(&dir.path().join("soloist.zip"), &[("soloist.exe", b"bad")]);

        let error = run(&installer, dir.path()).await.unwrap_err();

        assert!(matches!(error, InstallError::SignatureInvalid(_)));
        assert!(error.to_string().contains("signature"));
        assert_eq!(fs::read(installer.exe_path()).unwrap(), b"old exe");
        assert!(!dir.path().join("work").exists());
    }

    #[tokio::test]
    async fn wrong_signer_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let installer = installer(dir.path(), Ok("Someone Else Ltd"), Ok(()));

        seed_existing(&installer);
        make_zip(&dir.path().join("soloist.zip"), &[("soloist.exe", b"bad")]);

        assert!(matches!(
            run(&installer, dir.path()).await,
            Err(InstallError::WrongSigner(_))
        ));
        assert_eq!(fs::read(installer.exe_path()).unwrap(), b"old exe");
    }

    #[tokio::test]
    async fn signer_must_match_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let installer = installer(dir.path(), Ok("Not Spotify AB Ltd"), Ok(()));

        seed_existing(&installer);
        make_zip(&dir.path().join("soloist.zip"), &[("soloist.exe", b"bad")]);

        assert!(matches!(
            run(&installer, dir.path()).await,
            Err(InstallError::WrongSigner(_))
        ));
        assert_eq!(fs::read(installer.exe_path()).unwrap(), b"old exe");
    }

    #[tokio::test]
    async fn exe_that_does_not_run_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let installer = installer(dir.path(), Ok("Spotify AB"), Err("crashed"));

        seed_existing(&installer);
        make_zip(&dir.path().join("soloist.zip"), &[("soloist.exe", b"bad")]);

        assert!(matches!(
            run(&installer, dir.path()).await,
            Err(InstallError::Probe(_))
        ));
        assert_eq!(fs::read(installer.exe_path()).unwrap(), b"old exe");
    }

    #[tokio::test]
    async fn archive_without_exe_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let installer = installer(dir.path(), Ok("Spotify AB"), Ok(()));

        make_zip(&dir.path().join("soloist.zip"), &[("CHANGELOG.md", b"x")]);

        assert!(matches!(
            run(&installer, dir.path()).await,
            Err(InstallError::MissingExe)
        ));
    }

    #[test]
    fn failed_swap_restores_previous_files() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        let bin = dir.path().join("bin");

        fs::create_dir_all(&staging).unwrap();
        fs::create_dir_all(&bin).unwrap();
        fs::write(staging.join(EXE_NAME), b"new").unwrap();
        fs::write(bin.join(EXE_NAME), b"old").unwrap();
        fs::write(staging.join("CHANGELOG.md"), b"new log").unwrap();
        fs::write(bin.join("CHANGELOG.md"), b"old log").unwrap();
        fs::create_dir_all(bin.join("CHANGELOG.md.old").join("blocker")).unwrap();

        let result = swap_in(&staging, &bin);

        assert!(result.is_err());
        assert_eq!(fs::read(bin.join(EXE_NAME)).unwrap(), b"old");
    }
}
