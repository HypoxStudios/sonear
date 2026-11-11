//! The optional YT Music sign-in: keeps the session in memory, restores it
//! from the sign-in helper, refreshes it when a request says it ended, and
//! tells the rest of the app which library is showing.
//!
//! The cookies are secrets. They live only in the client's memory and in the
//! helper's WebView2 profile; nothing here logs or prints them.

use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use rust_i18n::t;
use sonear_youtube::{Client, Session, YoutubeError};
use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tokio::time::Instant;
use tracing::{debug, warn};

use super::catalog::YoutubeCatalog;
use super::convert;
use crate::config::ConfigHandle;
use crate::events::AppEvent;
use crate::library::{LibraryError, LibraryResult};
use crate::likes::LikeStore;
use crate::model::Artwork;

/// The shortest time between two background session refreshes.
pub const REFRESH_EVERY: Duration = Duration::from_secs(60);

/// What the sign-in helper found.
pub enum SessionOutcome {
    SignedIn(Session),
    SignedOut,
    Cancelled,
    Failed(String),
}

/// Reads the session out of the WebView2 profile, in a helper process.
/// `interactive` opens the sign-in window; otherwise nothing is shown.
#[async_trait]
pub trait SessionProvider: Send + Sync {
    async fn fetch(&self, interactive: bool) -> SessionOutcome;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YtAccountState {
    SignedOut,
    SigningIn,
    SignedIn {
        name: String,
        photo: Artwork,
    },
    /// The session ended and the user has to sign in again.
    Expired,
    Failed(String),
}

pub struct YoutubeAccount {
    client: Client,
    catalog: Arc<YoutubeCatalog>,
    provider: Arc<dyn SessionProvider>,
    config: ConfigHandle,
    webview_dir: PathBuf,
    events: broadcast::Sender<AppEvent>,
    likes: OnceLock<Arc<LikeStore>>,
    state: Mutex<YtAccountState>,
    /// When the last background refresh ran. Holding the lock makes refreshes one at a time.
    refreshed: tokio::sync::Mutex<Option<Instant>>,
    refresh_every: Mutex<Duration>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl YoutubeAccount {
    pub fn new(
        catalog: Arc<YoutubeCatalog>,
        provider: Arc<dyn SessionProvider>,
        config: ConfigHandle,
        webview_dir: PathBuf,
        events: broadcast::Sender<AppEvent>,
    ) -> Arc<Self> {
        Arc::new(Self {
            client: catalog.client().clone(),
            catalog,
            provider,
            config,
            webview_dir,
            events,
            likes: OnceLock::new(),
            state: Mutex::new(YtAccountState::SignedOut),
            refreshed: tokio::sync::Mutex::new(None),
            refresh_every: Mutex::new(REFRESH_EVERY),
        })
    }

    /// Gives the account the like cache it clears whenever the library changes.
    pub fn attach_likes(&self, likes: Arc<LikeStore>) {
        let _ = self.likes.set(likes);
    }

    #[cfg(test)]
    fn set_refresh_every(&self, every: Duration) {
        *lock(&self.refresh_every) = every;
    }

    pub fn state(&self) -> YtAccountState {
        lock(&self.state).clone()
    }

    pub fn is_signed_in(&self) -> bool {
        self.client.has_session()
    }

    fn set_state(&self, state: YtAccountState) {
        *lock(&self.state) = state.clone();
        let _ = self.events.send(AppEvent::YoutubeAccount(state));
    }

    fn notice(&self, text: String) {
        let _ = self.events.send(AppEvent::Notice(text));
    }

    /// Another library is showing: signed in, signed out or expired. History stays.
    fn library_changed(&self) {
        self.catalog.invalidate_account();

        if let Some(likes) = self.likes.get() {
            likes.clear();
        }

        let _ = self.events.send(AppEvent::AccountDataWiped);
    }

    /// Installs `session` and reads the account it belongs to.
    async fn adopt(&self, session: Session) -> Result<(String, Artwork), YoutubeError> {
        self.client.set_session(Some(session));

        match self.client.account().await {
            Ok(account) => Ok((account.name, convert::artwork(&account.photo))),
            Err(error) => {
                self.client.set_session(None);

                Err(error)
            }
        }
    }

    async fn finish_sign_in(&self, session: Session) -> bool {
        match self.adopt(session).await {
            Ok((name, photo)) => {
                self.config.update(|config| config.youtube_signed_in = true);
                self.set_state(YtAccountState::SignedIn { name, photo });
                self.library_changed();
                self.catalog.start_account_load();

                true
            }
            Err(YoutubeError::NeedsLogin) => {
                self.expired();

                false
            }
            Err(error) => {
                warn!(%error, "could not read the YT Music account");
                self.set_state(YtAccountState::Failed(
                    LibraryError::from(error).user_message(),
                ));

                false
            }
        }
    }

    fn expired(&self) {
        self.client.set_session(None);
        self.set_state(YtAccountState::Expired);
        self.library_changed();
    }

    /// Restores the sign-in of the last run, if there was one.
    pub fn start(self: &Arc<Self>, runtime: &Handle) {
        if !self.config.get().youtube_signed_in {
            return;
        }

        let account = self.clone();

        self.set_state(YtAccountState::SigningIn);
        runtime.spawn(async move {
            match account.provider.fetch(false).await {
                SessionOutcome::SignedIn(session) => {
                    account.finish_sign_in(session).await;
                }
                SessionOutcome::SignedOut => account.expired(),
                SessionOutcome::Cancelled => account.set_state(YtAccountState::SignedOut),
                SessionOutcome::Failed(reason) => {
                    warn!(%reason, "could not restore the YT Music sign-in");
                    account.set_state(YtAccountState::Failed(reason));
                }
            }
        });
    }

    /// Opens the sign-in window and waits for the result.
    pub async fn sign_in(self: &Arc<Self>) {
        let before = self.state();

        if before == YtAccountState::SigningIn {
            return;
        }

        self.set_state(YtAccountState::SigningIn);

        match self.provider.fetch(true).await {
            SessionOutcome::SignedIn(session) => {
                self.finish_sign_in(session).await;
            }
            SessionOutcome::SignedOut | SessionOutcome::Cancelled => self.set_state(before),
            SessionOutcome::Failed(reason) => self.set_state(YtAccountState::Failed(reason)),
        }
    }

    /// Forgets the session and deletes the WebView2 profile.
    pub async fn sign_out(self: &Arc<Self>) -> Vec<String> {
        let mut problems = Vec::new();

        self.client.set_session(None);
        self.config
            .update(|config| config.youtube_signed_in = false);

        match std::fs::remove_dir_all(&self.webview_dir) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                let text = t!("youtube.signout_profile", error = error.to_string()).into_owned();

                self.notice(text.clone());
                problems.push(text);
            }
        }

        self.set_state(YtAccountState::SignedOut);
        self.library_changed();

        problems
    }

    /// Reads a fresh session from the profile after a request said the
    /// session ended. At most one runs at a time and one a minute. Returns
    /// whether the client has a session to retry with.
    pub async fn refresh(self: &Arc<Self>) -> bool {
        if !self.is_signed_in() {
            return false;
        }

        let mut last = self.refreshed.lock().await;

        let every = *lock(&self.refresh_every);

        if last.is_some_and(|at| at.elapsed() < every) {
            return self.is_signed_in();
        }

        *last = Some(Instant::now());
        debug!("refreshing the YT Music session");

        match self.provider.fetch(false).await {
            SessionOutcome::SignedIn(session) => {
                self.client.set_session(Some(session));

                true
            }
            SessionOutcome::SignedOut => {
                drop(last);
                self.expired();
                self.notice(t!("youtube.session_ended").into_owned());

                false
            }
            SessionOutcome::Cancelled => false,
            SessionOutcome::Failed(reason) => {
                warn!(%reason, "could not refresh the YT Music session");

                false
            }
        }
    }

    /// Runs `operation`; when the session turns out to have ended, refreshes
    /// it and runs the operation once more.
    pub async fn retrying<T, F, Fut>(self: &Arc<Self>, operation: F) -> LibraryResult<T>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = LibraryResult<T>>,
    {
        match operation().await {
            Err(LibraryError::NeedsLogin) if self.is_signed_in() => {
                if self.refresh().await {
                    operation().await
                } else {
                    Err(LibraryError::NeedsLogin)
                }
            }
            result => result,
        }
    }
}

#[cfg(test)]
mod tests;
