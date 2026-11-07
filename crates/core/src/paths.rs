use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Where Sonear keeps its files.
#[derive(Debug, Clone)]
pub struct AppPaths {
    config_dir: PathBuf,
    local_root: PathBuf,
}

impl AppPaths {
    pub fn new(config_dir: impl Into<PathBuf>, local_root: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
            local_root: local_root.into(),
        }
    }

    /// `%APPDATA%\Sonear` for config, and for everything else the folder the
    /// config's `data_dir` names or `%LOCALAPPDATA%\Sonear`. Only reads the
    /// config file: loading it later is what backs up a corrupt one.
    pub fn discover() -> Option<Self> {
        let config = dirs::config_dir()?.join("Sonear");
        let local = peek_data_dir(&config.join("config.toml"))
            .map_or_else(Self::default_local_root, Some)?;

        Some(Self::new(config, local))
    }

    /// `%LOCALAPPDATA%\Sonear`, where the data lives unless it was moved.
    pub fn default_local_root() -> Option<PathBuf> {
        Some(dirs::data_local_dir()?.join("Sonear"))
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn local_root(&self) -> PathBuf {
        self.local_root.clone()
    }

    pub fn soloist_root(&self) -> PathBuf {
        self.local_root.join("soloist")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.local_root.join("cache")
    }

    pub fn meta_cache_dir(&self) -> PathBuf {
        self.cache_dir().join("meta")
    }

    pub fn cover_cache_dir(&self) -> PathBuf {
        self.cache_dir().join("covers")
    }

    pub fn history_db(&self) -> PathBuf {
        self.local_root.join("history.db")
    }

    /// Where downloaded updates wait until the user restarts into them.
    pub fn update_dir(&self) -> PathBuf {
        self.local_root.join("updates")
    }

    pub fn queue_file(&self) -> PathBuf {
        self.local_root.join("queue.json")
    }
}

#[derive(Deserialize, Default)]
struct DataDirOnly {
    data_dir: Option<PathBuf>,
}

/// The `data_dir` in a config file, if it can be read and is absolute.
fn peek_data_dir(path: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(path).ok()?;
    let peek: DataDirOnly = toml::from_str(&text).ok()?;

    peek.data_dir.filter(|dir| dir.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        std::fs::write(&path, text).unwrap();

        (dir, path)
    }

    #[test]
    fn the_configured_folder_is_used() {
        let (_dir, path) = write("data_dir = 'D:/Data/Sonear'\nlast_volume = 3\n");

        assert_eq!(peek_data_dir(&path), Some(PathBuf::from(r"D:\Data\Sonear")));
    }

    #[test]
    fn a_missing_relative_or_broken_setting_means_the_default() {
        let (_dir, relative) = write("data_dir = 'data'\n");
        let (_dir2, broken) = write("data_dir = [[[\n");
        let (_dir3, wrong_type) = write("data_dir = 4\n");
        let (_dir4, none) = write("last_volume = 3\n");

        assert_eq!(peek_data_dir(&relative), None);
        assert_eq!(peek_data_dir(&broken), None);
        assert_eq!(peek_data_dir(&wrong_type), None);
        assert_eq!(peek_data_dir(&none), None);
        assert_eq!(peek_data_dir(Path::new("missing.toml")), None);
    }

    #[test]
    fn peeking_leaves_a_corrupt_file_alone() {
        let (dir, path) = write("data_dir = [[[\n");

        let _ = peek_data_dir(&path);

        assert!(path.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
