//! Player model, backends, config and secrets. Has no UI dependencies.

pub mod account;
pub mod config;
pub mod convert;
#[cfg(feature = "demo")]
pub mod demo;
pub mod events;
pub mod history;
pub mod library;
pub mod likes;
pub mod manager;
pub mod meta;
pub mod model;
pub mod paths;
pub mod playback;
pub mod player;
pub mod queue;
pub mod secrets;
pub mod seen;
pub mod soloist_backend;

use std::sync::Arc;

use tokio::runtime::Handle;
use tokio::sync::broadcast;

pub use account::{Account, AccountState};
pub use config::{Config, ConfigHandle};
pub use events::AppEvent;
pub use history::{History, HistoryRow, HistoryStore};
pub use library::{LibraryBackend, LibraryError, SpotifyLibrary};
pub use likes::LikeStore;
pub use manager::SoloistManager;
pub use meta::{Freshness, MetaCache, TrackResolver};
pub use paths::AppPaths;
pub use playback::PlaybackModel;
pub use player::{Player, PlayerBackend, PlayerError};
pub use queue::{QueueManager, QueueSnapshot, QueueSource, QueuedTrack};
pub use secrets::{KeyringStore, Secret, SecretStore};
pub use seen::SeenCollector;

const EVENT_BUFFER: usize = 256;

/// Everything the UI holds on to: the player, config, the music library
/// and, outside demo mode, the Soloist manager.
pub struct Core {
    pub player: Player,
    pub config: ConfigHandle,
    pub soloist: Option<Arc<SoloistManager>>,
    pub library: Arc<dyn LibraryBackend>,
    pub meta: Arc<MetaCache>,
    pub history: Arc<History>,
    pub likes: Arc<LikeStore>,
    pub account: Arc<Account>,
    pub queue: Arc<QueueManager>,
    secrets: Arc<dyn SecretStore>,
    events: broadcast::Sender<AppEvent>,
}

impl Core {
    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.events.subscribe()
    }

    pub fn is_demo(&self) -> bool {
        self.soloist.is_none()
    }

    /// Deletes everything Sonear stored: the Spotify login and its cached data,
    /// the Soloist key, pairing and cache, and the settings. The Soloist
    /// program stays so it isn't downloaded again. Returns what could not be deleted.
    pub async fn delete_all_data(&self) -> Vec<String> {
        let mut problems = Vec::new();

        let Some(soloist) = &self.soloist else {
            let _ = self.events.send(AppEvent::Notice(
                "Deleting data isn't available in demo mode.".to_owned(),
            ));

            return problems;
        };

        problems.extend(self.account.disconnect().await);
        problems.extend(soloist.forget().await);

        for name in secrets::ALL_KEYS {
            if let Err(error) = self.secrets.delete(name) {
                problems.push(format!("{name}: {error}"));
            }
        }

        let backup = self.config.path().with_file_name("config.toml.bak");

        if let Err(error) = std::fs::remove_file(&backup)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            problems.push(format!("config backup: {error}"));
        }

        self.config.update(|config| *config = Config::default());
        soloist.start().await;

        problems
    }

    /// Starts the real stack: config, key store, supervisor, player and Spotify.
    pub fn start(runtime: &Handle, paths: &AppPaths) -> Self {
        let config = ConfigHandle::load(paths.config_file());
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        let secrets: Arc<dyn SecretStore> = Arc::new(KeyringStore);
        let manager = SoloistManager::new(
            paths,
            config.clone(),
            secrets.clone(),
            default_verifier(),
            events.clone(),
        );
        let backend =
            soloist_backend::SoloistBackend::spawn(manager.supervisor().client(), runtime);
        let player = Player::new(backend, runtime.clone(), config.clone(), events.clone());
        let spotify = Arc::new(SpotifyLibrary::default());
        let library: Arc<dyn LibraryBackend> = spotify.clone();
        let meta = MetaCache::open(paths.meta_cache_dir());
        let history = History::new(
            HistoryStore::open(paths.history_db()),
            library.clone(),
            TrackResolver::new(library.clone(), meta.clone()),
            config.clone(),
            events.clone(),
        );
        let likes = LikeStore::new(library.clone(), events.clone());
        let queue = QueueManager::new(
            player.clone(),
            events.clone(),
            Some(paths.queue_file()),
            runtime.clone(),
        );
        let account = Account::new(
            runtime.clone(),
            paths.clone(),
            config.clone(),
            secrets.clone(),
            Some(spotify),
            meta.clone(),
            history.clone(),
            likes.clone(),
            queue.clone(),
            events.clone(),
        );

        player.attach_library(library.clone());
        SeenCollector::spawn(player.clone(), meta.clone(), events.clone(), runtime);
        manager.run(runtime);
        account.start();

        Self {
            player,
            config,
            soloist: Some(manager),
            library,
            meta,
            history,
            likes,
            account,
            queue,
            secrets,
            events,
        }
    }

    /// Starts a simulated player with fake data. Needs no Soloist, key or network.
    #[cfg(feature = "demo")]
    pub fn start_demo(runtime: &Handle, config: ConfigHandle) -> Self {
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        let volume = config.get().last_volume;
        let backend = demo::DemoBackend::spawn(runtime, volume);
        let player = Player::new(
            backend.clone(),
            runtime.clone(),
            config.clone(),
            events.clone(),
        );
        let library: Arc<dyn LibraryBackend> =
            demo::DemoLibrary::new(backend, config.get().device_name);
        let meta = MetaCache::in_memory();
        let history = History::new(
            HistoryStore::in_memory(),
            library.clone(),
            TrackResolver::new(library.clone(), meta.clone()),
            config.clone(),
            events.clone(),
        );
        let likes = LikeStore::new(library.clone(), events.clone());
        let queue = QueueManager::new(player.clone(), events.clone(), None, runtime.clone());
        let secrets: Arc<dyn SecretStore> = Arc::new(secrets::MemoryStore::default());
        let account = Account::new(
            runtime.clone(),
            AppPaths::new("demo", "demo"),
            config.clone(),
            secrets.clone(),
            None,
            meta.clone(),
            history.clone(),
            likes.clone(),
            queue.clone(),
            events.clone(),
        );

        player.attach_library(library.clone());
        SeenCollector::spawn(player.clone(), meta.clone(), events.clone(), runtime);

        let seed = history.clone();

        runtime.spawn(async move {
            let _ = seed.sync().await;
        });

        Self {
            player,
            config,
            soloist: None,
            library,
            meta,
            history,
            likes,
            account,
            queue,
            secrets,
            events,
        }
    }
}

#[cfg(windows)]
fn default_verifier() -> Arc<dyn sonear_soloist::install::SignatureVerifier> {
    Arc::new(sonear_soloist::install::WinTrustVerifier)
}

#[cfg(not(windows))]
fn default_verifier() -> Arc<dyn sonear_soloist::install::SignatureVerifier> {
    struct Unsupported;

    impl sonear_soloist::install::SignatureVerifier for Unsupported {
        fn verify(
            &self,
            _: &std::path::Path,
        ) -> Result<String, sonear_soloist::install::InstallError> {
            Err(sonear_soloist::install::InstallError::SignatureInvalid(
                "signature checks are only available on Windows".into(),
            ))
        }
    }

    Arc::new(Unsupported)
}
