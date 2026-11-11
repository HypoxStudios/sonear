//! The Spotify login: connect, session state, and disconnect with data deletion.

use std::fs;
use std::sync::{Arc, Mutex};

use rust_i18n::t;
use sonear_spotify::auth::{self, LOGIN_TIMEOUT};
use sonear_spotify::{Auth, Secret as SpotifySecret, SpotifyClient, TokenStore};
use tokio::runtime::Handle;
use tokio::sync::{Notify, broadcast};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::config::{ConfigHandle, Source, active_source};
use crate::events::AppEvent;
use crate::history::History;
use crate::library::{LibraryBackend, SpotifyLibrary};
use crate::likes::LikeStore;
use crate::lyrics::LyricsStore;
use crate::meta::MetaCache;
use crate::paths::AppPaths;
use crate::queue::QueueManager;
use crate::secrets::{SPOTIFY_REFRESH_TOKEN, Secret, SecretStore};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountState {
    Disconnected,
    /// The browser should be open on `url`.
    LoggingIn {
        url: String,
    },
    Connected,
    LoginFailed(String),
    /// Spotify rejected the saved login.
    SessionExpired,
}

/// Keeps the refresh token in the credential store.
pub struct SecretTokenStore(pub Arc<dyn SecretStore>);

impl TokenStore for SecretTokenStore {
    fn load(&self) -> Option<SpotifySecret> {
        match self.0.get(SPOTIFY_REFRESH_TOKEN) {
            Ok(secret) => secret.map(|secret| SpotifySecret::new(secret.expose())),
            Err(error) => {
                warn!(%error, "could not read the Spotify login");

                None
            }
        }
    }

    fn save(&self, token: &SpotifySecret) {
        if let Err(error) = self
            .0
            .set(SPOTIFY_REFRESH_TOKEN, &Secret::new(token.expose()))
        {
            warn!(%error, "could not save the Spotify login");
        }
    }

    fn clear(&self) {
        if let Err(error) = self.0.delete(SPOTIFY_REFRESH_TOKEN) {
            warn!(%error, "could not delete the Spotify login");
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Spotify Client IDs are 32 hex characters; accept anything plausible.
pub fn looks_like_client_id(text: &str) -> bool {
    let text = text.trim();

    (16..=64).contains(&text.len()) && text.chars().all(|c| c.is_ascii_alphanumeric())
}

pub struct Account {
    runtime: Handle,
    paths: AppPaths,
    config: ConfigHandle,
    secrets: Arc<dyn SecretStore>,
    library: Option<Arc<SpotifyLibrary>>,
    meta: Arc<MetaCache>,
    history: Arc<History>,
    likes: Arc<LikeStore>,
    queue: Arc<QueueManager>,
    lyrics: Arc<LyricsStore>,
    events: broadcast::Sender<AppEvent>,
    state: Mutex<AccountState>,
    cancel: Mutex<Option<Arc<Notify>>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl Account {
    /// `library` is `None` in demo mode, where the account is always connected.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        runtime: Handle,
        paths: AppPaths,
        config: ConfigHandle,
        secrets: Arc<dyn SecretStore>,
        library: Option<Arc<SpotifyLibrary>>,
        meta: Arc<MetaCache>,
        history: Arc<History>,
        likes: Arc<LikeStore>,
        queue: Arc<QueueManager>,
        lyrics: Arc<LyricsStore>,
        events: broadcast::Sender<AppEvent>,
    ) -> Arc<Self> {
        let state = if library.is_some() {
            AccountState::Disconnected
        } else {
            AccountState::Connected
        };

        Arc::new(Self {
            runtime,
            paths,
            config,
            secrets,
            library,
            meta,
            history,
            likes,
            queue,
            lyrics,
            events,
            state: Mutex::new(state),
            cancel: Mutex::new(None),
            tasks: Mutex::new(Vec::new()),
        })
    }

    pub fn state(&self) -> AccountState {
        lock(&self.state).clone()
    }

    fn set_state(&self, state: AccountState) {
        *lock(&self.state) = state.clone();
        let _ = self.events.send(AppEvent::Account(state));
    }

    fn token_store(&self) -> Arc<dyn TokenStore> {
        Arc::new(SecretTokenStore(self.secrets.clone()))
    }

    /// Restores the saved login, if there is one.
    pub fn start(self: &Arc<Self>) {
        let Some(library) = self.library.clone() else {
            return;
        };
        let client_id = self.config.get().spotify_client_id;

        let lost = library.session_lost();
        let account = self.clone();

        self.runtime.spawn(async move {
            loop {
                lost.notified().await;
                account.session_lost();
            }
        });

        if client_id.is_empty() {
            return;
        }

        let auth = Arc::new(Auth::new(client_id, self.token_store()));

        if !auth.has_refresh_token() {
            return;
        }

        let client = SpotifyClient::new(auth);

        let _ = library.set_client(Some(client.clone()));

        self.finish_connect(&client);

        // Logins from before the user ID was recorded: note who it is now, so
        // a later login as someone else is caught.
        if self.config.get().account_user_id.is_empty() {
            let account = self.clone();

            self.track(self.runtime.spawn(async move {
                if let Ok(profile) = library.profile().await {
                    account.adopt_user(&profile.id).await;
                }
            }));
        }
    }

    fn track(&self, task: JoinHandle<()>) {
        lock(&self.tasks).push(task);
    }

    fn session_lost(self: &Arc<Self>) {
        if self.state() != AccountState::Connected {
            return;
        }

        info!("the Spotify session is gone");
        self.stop_background();
        self.set_state(AccountState::SessionExpired);
    }

    fn stop_background(&self) {
        let tasks: Vec<_> = lock(&self.tasks).drain(..).collect();

        for task in tasks {
            task.abort();
        }
    }

    fn finish_connect(self: &Arc<Self>, client: &SpotifyClient) {
        let mut limit = client.rate_limit();
        let events = self.events.clone();

        self.set_state(AccountState::Connected);
        self.track(self.history.spawn_sync_loop(&self.runtime));
        self.track(self.runtime.spawn(async move {
            while limit.changed().await.is_ok() {
                let _ = events.send(AppEvent::RateLimit(*limit.borrow()));
            }
        }));
    }

    /// Checks who just logged in before the account counts as connected.
    async fn verify_login(self: &Arc<Self>, auth: Arc<Auth>) {
        let Some(library) = &self.library else {
            return;
        };
        let client = SpotifyClient::new(auth);

        let _ = library.set_client(Some(client.clone()));

        match library.profile().await {
            Ok(profile) => {
                self.adopt_user(&profile.id).await;
                self.finish_connect(&client);
            }
            Err(error) => {
                warn!(%error, "could not read the Spotify profile after login");

                if let Some(client) = library.set_client(None) {
                    client.auth().sign_out().await;
                }

                self.set_state(AccountState::LoginFailed(
                    t!("account.profile_failed").into_owned(),
                ));
            }
        }
    }

    /// Records which Spotify user the data on disk belongs to, deleting what
    /// is there if it belonged to someone else: logging in again after an
    /// expired session can be a different person. Returns whether it wiped.
    pub async fn adopt_user(&self, user_id: &str) -> bool {
        let previous = self.config.get().account_user_id;
        let switched = !previous.is_empty() && previous != user_id;

        if switched {
            info!("a different Spotify user logged in, deleting the previous account's data");

            for problem in self.wipe_account_data().await {
                warn!(%problem, "could not delete the previous account's data");
            }
        }

        self.config
            .update(|config| config.account_user_id = user_id.to_owned());

        switched
    }

    /// Starts a login: binds the loopback port, then reports the URL to open
    /// through `AccountState::LoggingIn`. Everything after happens in the background.
    pub fn begin_login(self: &Arc<Self>, client_id: &str) {
        if self.library.is_none() {
            return;
        }

        let client_id = client_id.trim().to_owned();

        if !looks_like_client_id(&client_id) {
            self.set_state(AccountState::LoginFailed(
                t!("account.bad_client_id").into_owned(),
            ));

            return;
        }

        self.cancel_login();

        let cancel = Arc::new(Notify::new());

        *lock(&self.cancel) = Some(cancel.clone());
        self.config
            .update(|config| config.spotify_client_id.clone_from(&client_id));

        let account = self.clone();

        self.runtime.spawn(async move {
            account.run_login(client_id, cancel).await;
        });
    }

    async fn run_login(self: &Arc<Self>, client_id: String, cancel: Arc<Notify>) {
        let listener = match auth::bind_redirect_listener().await {
            Ok(listener) => listener,
            Err(error) => {
                self.set_state(AccountState::LoginFailed(error.user_message()));

                return;
            }
        };
        let auth = Arc::new(Auth::new(client_id.clone(), self.token_store()));
        let request = auth::begin(&client_id);

        self.set_state(AccountState::LoggingIn {
            url: request.url.clone(),
        });

        let outcome = auth::wait_for_callback(listener, &request.state, LOGIN_TIMEOUT, async {
            cancel.notified().await;
        })
        .await;

        match outcome {
            Ok(code) => match auth.complete_login(&request, &code).await {
                Ok(()) => self.verify_login(auth).await,
                Err(error) => self.set_state(AccountState::LoginFailed(error.user_message())),
            },
            Err(sonear_spotify::error::AuthError::Cancelled) => {
                self.set_state(AccountState::Disconnected);
            }
            Err(error) => self.set_state(AccountState::LoginFailed(error.user_message())),
        }
    }

    pub fn cancel_login(&self) {
        if let Some(cancel) = lock(&self.cancel).take() {
            cancel.notify_one();
        }
    }

    /// Forgets the Spotify login and deletes everything stored for it: the
    /// refresh token, cached metadata, covers, history, queue and the Client ID.
    /// The Soloist key and install are left alone.
    pub async fn disconnect(self: &Arc<Self>) -> Vec<String> {
        let mut problems = Vec::new();

        if self.library.is_none() && active_source() == Source::Youtube {
            return problems;
        }

        if self.library.is_none() {
            let _ = self
                .events
                .send(AppEvent::Notice(t!("account.demo_disconnect").into_owned()));

            return problems;
        }

        self.cancel_login();
        self.stop_background();

        if let Some(library) = &self.library
            && let Some(client) = library.set_client(None)
        {
            client.auth().sign_out().await;
        }

        if let Err(error) = self.secrets.delete(SPOTIFY_REFRESH_TOKEN) {
            problems.push(format!(
                "{}: {}",
                t!("problem.login_token"),
                error.user_message()
            ));
        }

        problems.extend(self.wipe_account_data().await);
        self.config
            .update(|config| config.spotify_client_id.clear());
        self.set_state(if self.library.is_some() {
            AccountState::Disconnected
        } else {
            AccountState::Connected
        });

        problems
    }

    /// Deletes everything cached for the account and tells the UI to drop its copies.
    pub(crate) async fn wipe_account_data(&self) -> Vec<String> {
        let mut problems = Vec::new();

        self.meta.clear();
        self.likes.clear();
        self.lyrics.clear();

        if let Err(error) = self.history.store().wipe().await {
            problems.push(format!(
                "{}: {}",
                t!("problem.history"),
                error.user_message()
            ));
        }

        if let Err(error) = self.queue.wipe() {
            problems.push(format!("{}: {error}", t!("problem.queue")));
        }

        let covers = self.paths.cover_cache_dir();

        if let Err(error) = fs::remove_dir_all(&covers)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            problems.push(format!("{}: {error}", t!("problem.cover_cache")));
        }

        self.config.update(|config| {
            config.history_cleared_at_ms = 0;
            config.account_user_id.clear();
        });

        let _ = self.events.send(AppEvent::AccountDataWiped);

        problems
    }
}

#[cfg(test)]
mod tests;
