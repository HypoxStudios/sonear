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

    pub fn logs_dir(&self) -> PathBuf {
        self.local_root.join("logs")
    }

    pub fn queue_file(&self) -> PathBuf {
        self.local_root.join("queue.json")
    }

    /// Everything the YouTube Music source keeps.
    pub fn youtube_root(&self) -> PathBuf {
        self.local_root.join("youtube")
    }

    /// Where mpv and yt-dlp are downloaded.
    pub fn youtube_tools_dir(&self) -> PathBuf {
        self.youtube_root().join("tools")
    }

    /// The WebView2 profile of the YT Music sign-in. It holds the only copy of the cookies.
    pub fn youtube_webview_dir(&self) -> PathBuf {
        self.youtube_root().join("webview")
    }

    pub fn youtube_meta_cache_dir(&self) -> PathBuf {
        self.youtube_root().join("cache").join("meta")
    }

    pub fn youtube_history_db(&self) -> PathBuf {
        self.youtube_root().join("history.db")
    }

    pub fn youtube_queue_file(&self) -> PathBuf {
        self.youtube_root().join("queue.json")
    }

    pub fn youtube_library_file(&self) -> PathBuf {
        self.youtube_root().join("library.json")
    }

    pub fn youtube_plays_file(&self) -> PathBuf {
        self.youtube_root().join("plays.json")
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
    fn youtube_files_live_under_their_own_folder() {
        let paths = AppPaths::new("cfg", "root");
        let root = Path::new("root").join("youtube");

        assert_eq!(paths.youtube_root(), root);
        assert_eq!(paths.youtube_tools_dir(), root.join("tools"));
        assert_eq!(paths.youtube_webview_dir(), root.join("webview"));
        assert_eq!(
            paths.youtube_meta_cache_dir(),
            root.join("cache").join("meta")
        );
        assert_eq!(paths.youtube_history_db(), root.join("history.db"));
        assert_eq!(paths.youtube_queue_file(), root.join("queue.json"));
        assert_eq!(paths.youtube_library_file(), root.join("library.json"));
        assert_eq!(paths.youtube_plays_file(), root.join("plays.json"));
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
