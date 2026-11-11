//! Where mpv and yt-dlp live, which releases are installed, and how they are
//! downloaded. Both are fetched on first use and never bundled.

mod install;

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::ToolError;

pub use install::{Installer, Tool, ToolProgress};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPaths {
    root: PathBuf,
}

impl ToolPaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn mpv_dir(&self) -> PathBuf {
        self.root.join("mpv")
    }

    pub fn ytdlp_dir(&self) -> PathBuf {
        self.root.join("yt-dlp")
    }

    pub fn mpv_exe(&self) -> PathBuf {
        self.mpv_dir().join("mpv.exe")
    }

    pub fn ytdlp_exe(&self) -> PathBuf {
        self.ytdlp_dir().join("yt-dlp.exe")
    }

    pub fn versions_file(&self) -> PathBuf {
        self.root.join("versions.json")
    }
}

/// Release tags of the installed tools.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolVersions {
    pub mpv: Option<String>,
    pub ytdlp: Option<String>,
}

impl ToolVersions {
    /// What is recorded on disk, minus any tool whose exe is gone.
    pub fn installed(paths: &ToolPaths) -> Self {
        let mut versions = Self::load(paths);

        if !paths.mpv_exe().is_file() {
            versions.mpv = None;
        }

        if !paths.ytdlp_exe().is_file() {
            versions.ytdlp = None;
        }

        versions
    }

    fn load(paths: &ToolPaths) -> Self {
        fs::read(paths.versions_file())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, paths: &ToolPaths) -> Result<(), ToolError> {
        fs::create_dir_all(paths.root())?;

        let text =
            serde_json::to_vec_pretty(self).map_err(|error| ToolError::Io(error.to_string()))?;
        let target = paths.versions_file();
        let temp = target.with_extension("json.tmp");

        fs::write(&temp, text)?;
        fs::rename(&temp, &target)?;

        Ok(())
    }

    pub fn is_complete(&self) -> bool {
        self.mpv.is_some() && self.ytdlp.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_follow_the_layout() {
        let paths = ToolPaths::new("root");

        assert_eq!(paths.mpv_exe(), Path::new("root/mpv/mpv.exe"));
        assert_eq!(paths.ytdlp_exe(), Path::new("root/yt-dlp/yt-dlp.exe"));
        assert_eq!(paths.versions_file(), Path::new("root/versions.json"));
    }

    #[test]
    fn versions_need_the_exes_to_exist() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ToolPaths::new(dir.path());
        let versions = ToolVersions {
            mpv: Some("v0.40.0".into()),
            ytdlp: Some("2026.11.01".into()),
        };

        versions.save(&paths).unwrap();
        assert_eq!(ToolVersions::installed(&paths), ToolVersions::default());

        fs::create_dir_all(paths.mpv_dir()).unwrap();
        fs::write(paths.mpv_exe(), b"x").unwrap();

        let installed = ToolVersions::installed(&paths);

        assert_eq!(installed.mpv.as_deref(), Some("v0.40.0"));
        assert_eq!(installed.ytdlp, None);
        assert!(!installed.is_complete());
    }

    #[test]
    fn a_corrupt_versions_file_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ToolPaths::new(dir.path());

        fs::write(paths.versions_file(), b"{ not json").unwrap();

        assert_eq!(ToolVersions::installed(&paths), ToolVersions::default());
    }
}
