use std::path::{Path, PathBuf};

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

    /// `%APPDATA%\Sonear` for config and `%LOCALAPPDATA%\Sonear` for everything else.
    pub fn discover() -> Option<Self> {
        let config = dirs::config_dir()?.join("Sonear");
        let local = dirs::data_local_dir()?.join("Sonear");

        Some(Self::new(config, local))
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

    pub fn queue_file(&self) -> PathBuf {
        self.local_root.join("queue.json")
    }
}
