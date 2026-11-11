//! Player model, backends, config and secrets. Has no UI dependencies.

pub mod account;
pub mod config;
pub mod convert;
pub mod data_dir;
#[cfg(feature = "demo")]
pub mod demo;
pub mod events;
pub mod history;
pub mod library;
pub mod likes;
pub mod link;
pub mod lyrics;
pub mod manager;
pub mod meta;
pub mod model;
pub mod paths;
pub mod playback;
pub mod player;
pub mod queue;
pub mod radio;
pub mod secrets;
pub mod seen;
pub mod soloist_backend;
pub mod update;
pub mod youtube;

rust_i18n::i18n!("locales", fallback = "en");

use std::sync::Arc;

use rust_i18n::t;
use tokio::runtime::Handle;
use tokio::sync::broadcast;

pub use account::{Account, AccountState};
pub use config::{Config, ConfigHandle, Language, Source, active_source};
pub use data_dir::{MoveError, delete_local_data};
pub use events::AppEvent;
pub use history::{History, HistoryRow, HistoryStore};
pub use library::{LibraryBackend, LibraryError, SpotifyLibrary};
pub use likes::LikeStore;
pub use link::SpotifyLink;
pub use lyrics::{LrcLibSource, LyricLine, Lyrics, LyricsQuery, LyricsSource, LyricsStore};
pub use manager::SoloistManager;
pub use meta::{Freshness, MetaCache, TrackResolver};
pub use paths::AppPaths;
pub use playback::PlaybackModel;
pub use player::{Player, PlayerBackend, PlayerError};
pub use queue::{QueueManager, QueueSnapshot, QueueSource, QueuedTrack};
pub use secrets::{KeyringStore, Secret, SecretStore};
pub use seen::SeenCollector;
pub use youtube::{
    SessionOutcome, SessionProvider, ToolsStatus, YoutubeAccount, YoutubeTools, YtAccountState,
};

const EVENT_BUFFER: usize = 256;

/// Everything the UI holds on to: the player, config, the music library
/// and, outside demo mode, the Soloist manager.
pub struct Core {
    pub paths: AppPaths,
    pub player: Player,
    pub config: ConfigHandle,
    pub soloist: Option<Arc<SoloistManager>>,
    /// Which music service this process runs.
    pub source: Source,
    /// The YouTube Music playback tools; only with the YouTube source.
    pub tools: Option<Arc<YoutubeTools>>,
    /// The optional YT Music sign-in; only with the YouTube source.
    pub youtube_account: Option<Arc<YoutubeAccount>>,
    pub library: Arc<dyn LibraryBackend>,
    pub meta: Arc<MetaCache>,
    pub history: Arc<History>,
    pub likes: Arc<LikeStore>,
    pub account: Arc<Account>,
    pub queue: Arc<QueueManager>,
    pub lyrics: Arc<LyricsStore>,
    secrets: Arc<dyn SecretStore>,
    events: broadcast::Sender<AppEvent>,
    demo: bool,
    youtube: Option<Arc<youtube::YoutubeCatalog>>,
}

impl Core {
    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.events.subscribe()
    }

    pub fn is_demo(&self) -> bool {
        self.demo
    }

    /// Deletes everything Sonear stored for both sources: the Spotify login and
    /// its cached data, the Soloist key, pairing and cache, the YT Music
    /// sign-in and local library, and the settings. The Soloist program and
    /// the YouTube tools stay so they aren't downloaded again. Returns what
    /// could not be deleted.
    pub async fn delete_all_data(&self) -> Vec<String> {
        let mut problems = Vec::new();

        if self.source == Source::Youtube && !self.demo {
            return self.delete_youtube_data().await;
        }

        let Some(soloist) = &self.soloist else {
            let _ = self
                .events
                .send(AppEvent::Notice(t!("account.demo_delete").into_owned()));

            return problems;
        };

        problems.extend(self.account.disconnect().await);
        problems.extend(soloist.forget().await);

        for name in secrets::ALL_KEYS {
            if let Err(error) = self.secrets.delete(name) {
                problems.push(format!("{name}: {}", error.user_message()));
            }
        }

        let backup = self.config.path().with_file_name("config.toml.bak");

        if let Err(error) = std::fs::remove_file(&backup)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            problems.push(format!("{}: {error}", t!("problem.config_backup")));
        }

        problems.extend(data_dir::delete_other_source_data(
            &self.paths,
            Source::Spotify,
            &*self.secrets,
        ));
        self.config.update(Config::reset_keeping_data_dir);
        soloist.start().await;

        problems
    }

    /// Deletes what the YouTube source stored: the local library and plays,
    /// caches, likes, lyrics, history, queue and covers, and the settings,
    /// plus what Spotify left behind. The downloaded tools stay so they are
    /// not fetched again.
    async fn delete_youtube_data(&self) -> Vec<String> {
        let mut problems = Vec::new();

        if let Some(account) = &self.youtube_account {
            problems.extend(account.sign_out().await);
        }

        if let Some(catalog) = &self.youtube {
            catalog.local().wipe();
            catalog.clear();
        }

        problems.extend(self.account.wipe_account_data().await);
        problems.extend(data_dir::delete_other_source_data(
            &self.paths,
            Source::Youtube,
            &*self.secrets,
        ));

        let backup = self.config.path().with_file_name("config.toml.bak");

        if let Err(error) = std::fs::remove_file(&backup)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            problems.push(format!("{}: {error}", t!("problem.config_backup")));
        }

        self.config.update(Config::reset_keeping_data_dir);

        problems
    }

    /// Starts the real stack for the configured source. `session_provider`
    /// reads the YT Music sign-in; the other sources ignore it.
    pub fn start(
        runtime: &Handle,
        paths: &AppPaths,
        session_provider: Arc<dyn SessionProvider>,
    ) -> Self {
        let config = ConfigHandle::load(paths.config_file());
        let source = config.get().source;

        config::set_active_source(source);

        match source {
            Source::Spotify => Self::start_spotify(runtime, paths, config),
            Source::Youtube => Self::start_youtube(runtime, paths, config, session_provider),
        }
    }

    /// Config, key store, supervisor, player and Spotify.
    fn start_spotify(runtime: &Handle, paths: &AppPaths, config: ConfigHandle) -> Self {
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
        let lyrics = LyricsStore::new(Arc::new(LrcLibSource::new()));
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
            lyrics.clone(),
            events.clone(),
        );

        player.attach_library(library.clone());
        SeenCollector::spawn(player.clone(), meta.clone(), events.clone(), runtime);
        manager.run(runtime);
        account.start();

        Self {
            paths: paths.clone(),
            player,
            config,
            soloist: Some(manager),
            source: Source::Spotify,
            tools: None,
            youtube_account: None,
            library,
            meta,
            history,
            likes,
            account,
            queue,
            lyrics,
            secrets,
            events,
            demo: false,
            youtube: None,
        }
    }

    /// The YouTube Music stack: mpv and yt-dlp instead of Soloist, and local
    /// files instead of an account.
    fn start_youtube(
        runtime: &Handle,
        paths: &AppPaths,
        config: ConfigHandle,
        session_provider: Arc<dyn SessionProvider>,
    ) -> Self {
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        let secrets: Arc<dyn SecretStore> = Arc::new(KeyringStore);
        let settings = config.get();
        let tool_paths = sonear_youtube::tools::ToolPaths::new(paths.youtube_tools_dir());
        let tools = YoutubeTools::new(
            tool_paths.clone(),
            sonear_youtube::tools::Installer::new(tool_paths.clone()),
            events.clone(),
        );
        let language = match settings.effective_language() {
            Language::Es => "es-419",
            Language::En => "en",
        };
        let local =
            youtube::LocalLibrary::new(paths.youtube_library_file(), paths.youtube_plays_file());
        let catalog = youtube::YoutubeCatalog::new(sonear_youtube::Client::new(language), local);
        let youtube_account = YoutubeAccount::new(
            catalog.clone(),
            session_provider,
            config.clone(),
            paths.youtube_webview_dir(),
            events.clone(),
        );
        let backend = youtube::YoutubeBackend::spawn(
            runtime,
            Arc::new(youtube::MpvLauncher::new(tool_paths.clone())),
            Arc::new(youtube::ResolverStreams(sonear_youtube::Resolver::new(
                tool_paths.ytdlp_exe(),
            ))),
            catalog.clone(),
            config.clone(),
            events.clone(),
        );
        let player = Player::new(
            backend.clone(),
            runtime.clone(),
            config.clone(),
            events.clone(),
        );
        let library: Arc<dyn LibraryBackend> = youtube::YoutubeLibrary::new(
            catalog.clone(),
            backend,
            config.clone(),
            youtube_account.clone(),
        );
        let meta = MetaCache::open(paths.youtube_meta_cache_dir());
        let history = History::new(
            HistoryStore::open(paths.youtube_history_db()),
            library.clone(),
            TrackResolver::new(library.clone(), meta.clone()),
            config.clone(),
            events.clone(),
        );
        let likes = LikeStore::new(library.clone(), events.clone());
        let queue = QueueManager::new(
            player.clone(),
            events.clone(),
            Some(paths.youtube_queue_file()),
            runtime.clone(),
        );
        let lyrics = LyricsStore::new(Arc::new(LrcLibSource::new()));
        let account = Account::new(
            runtime.clone(),
            paths.clone(),
            config.clone(),
            secrets.clone(),
            None,
            meta.clone(),
            history.clone(),
            likes.clone(),
            queue.clone(),
            lyrics.clone(),
            events.clone(),
        );

        player.attach_library(library.clone());
        history.spawn_sync_loop(runtime);
        tools.run(runtime);
        youtube_account.attach_likes(likes.clone());
        youtube_account.start(runtime);

        Self {
            paths: paths.clone(),
            player,
            config,
            soloist: None,
            source: Source::Youtube,
            tools: Some(tools),
            youtube_account: Some(youtube_account),
            library,
            meta,
            history,
            likes,
            account,
            queue,
            lyrics,
            secrets,
            events,
            demo: false,
            youtube: Some(catalog),
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
        let lyrics = LyricsStore::new(Arc::new(demo::DemoLyrics));
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
            lyrics.clone(),
            events.clone(),
        );

        player.attach_library(library.clone());
        SeenCollector::spawn(player.clone(), meta.clone(), events.clone(), runtime);

        let seed = history.clone();

        runtime.spawn(async move {
            let _ = seed.sync().await;
        });

        Self {
            paths: AppPaths::new("demo", "demo"),
            player,
            config,
            soloist: None,
            source: Source::Spotify,
            tools: None,
            youtube_account: None,
            library,
            meta,
            history,
            likes,
            account,
            queue,
            lyrics,
            secrets,
            events,
            demo: true,
            youtube: None,
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

#[cfg(test)]
mod i18n_tests;
