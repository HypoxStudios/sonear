use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not write the config file: {0}")]
    Write(#[from] io::Error),
    #[error("could not encode the config: {0}")]
    Encode(#[from] toml::ser::Error),
}

/// Version of the end-user agreement and privacy policy. Bump it when either
/// changes, so everyone is asked to accept again.
pub const LEGAL_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub device_name: String,
    /// Where the data folder lives, when the user moved it from the default.
    pub data_dir: Option<PathBuf>,
    pub last_volume: u8,
    pub sidebar_collapsed: bool,
    pub spotify_client_id: String,
    /// Version of the agreement and privacy policy the user accepted; 0 if none.
    pub legal_accepted: u32,
    /// Spotify user ID of the account whose data is on disk.
    pub account_user_id: String,
    pub history_retention_months: u32,
    /// Plays up to this time were cleared by the user, so syncs skip them.
    pub history_cleared_at_ms: u64,
    /// Whether lyrics are looked up. Looking them up sends the song's title,
    /// artist, album and length to LRCLIB.
    pub lyrics_enabled: bool,
    /// Whether buttons and sliders take a colour from the playing song's cover.
    pub cover_accent: bool,
    /// Whether closing the window keeps Sonear running in the tray.
    pub close_to_tray: bool,
    /// Whether the one-time note about the tray icon was shown.
    pub tray_hint_shown: bool,
    /// Top-left corner of the mini player in logical pixels, once the user has placed it.
    pub mini_player_position: Option<[f32; 2]>,
    pub right_panel: RightPanelConfig,
    pub ui: UiConfig,
    /// Keystrokes the user changed, by shortcut id. An empty string means unbound.
    pub shortcuts: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RightPanelConfig {
    pub open: bool,
    /// Whether the panel shows lyrics instead of the queue.
    pub lyrics: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub ai_hint_dismissed: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            device_name: "Sonear".to_owned(),
            data_dir: None,
            last_volume: 40,
            sidebar_collapsed: false,
            spotify_client_id: String::new(),
            legal_accepted: 0,
            account_user_id: String::new(),
            history_retention_months: 12,
            history_cleared_at_ms: 0,
            lyrics_enabled: true,
            cover_accent: true,
            close_to_tray: true,
            tray_hint_shown: false,
            mini_player_position: None,
            right_panel: RightPanelConfig::default(),
            ui: UiConfig::default(),
            shortcuts: BTreeMap::new(),
        }
    }
}

impl Default for RightPanelConfig {
    fn default() -> Self {
        Self {
            open: true,
            lyrics: false,
        }
    }
}

impl Config {
    fn sanitised(mut self) -> Self {
        if self.device_name.trim().is_empty() {
            self.device_name = Self::default().device_name;
        }

        if self
            .data_dir
            .as_deref()
            .is_some_and(|dir| !dir.is_absolute())
        {
            self.data_dir = None;
        }

        self.last_volume = self.last_volume.min(100);
        self.history_retention_months = self.history_retention_months.clamp(1, 120);
        self.spotify_client_id = self.spotify_client_id.trim().to_owned();

        if self
            .mini_player_position
            .is_some_and(|position| !position.iter().all(|axis| axis.is_finite()))
        {
            self.mini_player_position = None;
        }

        self
    }

    /// Defaults for everything except where the data folder is, because the
    /// Soloist program stays there.
    pub fn reset_keeping_data_dir(&mut self) {
        let data_dir = self.data_dir.take();

        *self = Self::default();
        self.data_dir = data_dir;
    }

    /// Reads the file, falling back to defaults. A file that cannot be parsed
    /// is renamed to `<name>.bak` so the user's text is not lost.
    pub fn load(path: &Path) -> Self {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Self::default(),
            Err(error) => {
                warn!(%error, "could not read the config file, using defaults");
                return Self::default();
            }
        };

        match toml::from_str::<Config>(&text) {
            Ok(config) => config.sanitised(),
            Err(error) => {
                warn!(%error, "config file is corrupt, keeping a backup and using defaults");

                let backup = backup_path(path);

                if let Err(error) = fs::rename(path, &backup) {
                    warn!(%error, "could not back up the corrupt config file");
                }

                Self::default()
            }
        }
    }

    /// Writes to a temp file, syncs it, then renames over the target.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let text = toml::to_string_pretty(self)?;
        let temp = path.with_extension("toml.tmp");

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = File::create(&temp)?;

        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);

        fs::rename(&temp, path)?;

        Ok(())
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();

    name.push(".bak");

    path.with_file_name(name)
}

/// Shared, saved-on-change view of the config file.
#[derive(Clone)]
pub struct ConfigHandle {
    path: Arc<PathBuf>,
    value: Arc<Mutex<Config>>,
}

impl ConfigHandle {
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let value = Config::load(&path);

        Self {
            path: Arc::new(path),
            value: Arc::new(Mutex::new(value)),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self) -> Config {
        self.value
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Applies `change` and saves. A failed save is logged and the in-memory
    /// value is still updated.
    pub fn update(&self, change: impl FnOnce(&mut Config)) {
        let snapshot = {
            let mut value = self
                .value
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());

            change(&mut value);

            value.clone().sanitised()
        };

        if let Err(error) = snapshot.save(&self.path) {
            warn!(%error, "could not save the config file");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load(&dir.path().join("config.toml"));

        assert_eq!(config, Config::default());
        assert_eq!(config.device_name, "Sonear");
        assert_eq!(config.last_volume, 40);
        assert!(!config.sidebar_collapsed);
        assert!(!config.ui.ai_hint_dismissed);
        assert_eq!(config.spotify_client_id, "");
        assert_eq!(config.history_retention_months, 12);
        assert_eq!(config.legal_accepted, 0);
        assert_eq!(config.account_user_id, "");
        assert!(config.lyrics_enabled);
        assert!(config.cover_accent);
        assert!(config.close_to_tray);
        assert!(!config.tray_hint_shown);
        assert_eq!(config.mini_player_position, None);
        assert!(!config.right_panel.lyrics);
    }

    #[test]
    fn a_relative_data_dir_is_dropped() {
        let relative = Config {
            data_dir: Some(PathBuf::from("data")),
            ..Config::default()
        };
        let absolute = Config {
            data_dir: Some(PathBuf::from(r"D:\Sonear")),
            ..Config::default()
        };

        assert_eq!(relative.sanitised().data_dir, None);
        assert_eq!(
            absolute.sanitised().data_dir,
            Some(PathBuf::from(r"D:\Sonear"))
        );
    }

    #[test]
    fn resetting_keeps_the_data_dir() {
        let mut config = Config {
            data_dir: Some(PathBuf::from(r"D:\Sonear")),
            last_volume: 80,
            legal_accepted: 2,
            ..Config::default()
        };

        config.reset_keeping_data_dir();

        assert_eq!(config.data_dir, Some(PathBuf::from(r"D:\Sonear")));
        assert_eq!(config.last_volume, 40);
        assert_eq!(config.legal_accepted, 0);
    }

    #[test]
    fn files_from_before_lyrics_still_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        fs::write(
            &path,
            "[right_panel]
open = false
",
        )
        .unwrap();

        let config = Config::load(&path);

        assert!(!config.right_panel.open);
        assert!(!config.right_panel.lyrics);
        assert!(config.lyrics_enabled);
        assert!(config.cover_accent);
        assert!(config.close_to_tray);
        assert!(!config.tray_hint_shown);
        assert_eq!(config.mini_player_position, None);
    }

    #[test]
    fn tray_and_mini_player_settings_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = Config {
            close_to_tray: false,
            tray_hint_shown: true,
            mini_player_position: Some([120.5, 48.]),
            ..Config::default()
        };

        config.save(&path).unwrap();

        assert_eq!(Config::load(&path), config);
    }

    #[test]
    fn legal_and_account_fields_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = Config {
            legal_accepted: LEGAL_VERSION,
            account_user_id: "someone".into(),
            ..Config::default()
        };

        config.save(&path).unwrap();

        let loaded = Config::load(&path);

        assert_eq!(loaded, config);
        assert_eq!(loaded.legal_accepted, LEGAL_VERSION);
        assert_eq!(loaded.account_user_id, "someone");
    }

    #[test]
    fn partial_files_fill_in_defaults_and_unknown_fields_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        fs::write(
            &path,
            "device_name = \"Desk\"\nfuture_setting = 7\n[ui]\nsomething_new = true\n",
        )
        .unwrap();

        let config = Config::load(&path);

        assert_eq!(config.device_name, "Desk");
        assert_eq!(config.last_volume, 40);
        assert!(config.right_panel.open);
        assert!(path.exists());
    }

    #[test]
    fn corrupt_file_is_renamed_and_defaults_used() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        fs::write(&path, "this is = = not toml [").unwrap();

        let config = Config::load(&path);

        assert_eq!(config, Config::default());
        assert!(!path.exists());
        assert_eq!(
            fs::read_to_string(dir.path().join("config.toml.bak")).unwrap(),
            "this is = = not toml ["
        );
    }

    #[test]
    fn wrong_types_count_as_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        fs::write(&path, "last_volume = \"loud\"").unwrap();

        assert_eq!(Config::load(&path), Config::default());
        assert!(dir.path().join("config.toml.bak").exists());
    }

    #[test]
    fn save_is_atomic_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        let config = Config {
            device_name: "Kitchen".into(),
            last_volume: 77,
            sidebar_collapsed: true,
            ..Config::default()
        };

        config.save(&path).unwrap();

        assert_eq!(Config::load(&path), config);
        assert!(!path.with_extension("toml.tmp").exists());

        config.save(&path).unwrap();

        assert_eq!(Config::load(&path), config);
    }

    #[test]
    fn shortcut_overrides_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config::default();

        config.shortcuts.insert("play_pause".into(), "p".into());
        config.shortcuts.insert("like".into(), String::new());
        config.save(&path).unwrap();

        let loaded = Config::load(&path);

        assert_eq!(loaded, config);
        assert_eq!(
            loaded.shortcuts.get("play_pause").map(String::as_str),
            Some("p")
        );
        assert_eq!(loaded.shortcuts.get("like").map(String::as_str), Some(""));
        assert!(Config::default().shortcuts.is_empty());
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        fs::write(&path, "last_volume = 250\ndevice_name = \"  \"").unwrap();

        let config = Config::load(&path);

        assert_eq!(config.last_volume, 100);
        assert_eq!(config.device_name, "Sonear");
    }

    #[test]
    fn handle_saves_updates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let handle = ConfigHandle::load(&path);

        handle.update(|config| config.ui.ai_hint_dismissed = true);

        assert!(handle.get().ui.ai_hint_dismissed);
        assert!(Config::load(&path).ui.ai_hint_dismissed);
    }
}
