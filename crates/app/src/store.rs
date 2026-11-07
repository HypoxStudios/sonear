use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::{AppContext as _, Context, Entity, SharedString, Task};
use sonear_core::model::ItemKind;
use sonear_core::model::{PlayOffset, PlaybackRequest, Playlist, Profile, Track};
use sonear_core::queue::QueueSource;
use sonear_core::{AccountState, AppEvent, Core, PlaybackModel, QueueSnapshot};
use sonear_soloist::Status;
use sonear_soloist::install::InstallProgress;
use sonear_spotify::RateLimit;
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use crate::data::{self, Env, Load, Slot};

const REPAINT_EVERY: Duration = Duration::from_millis(250);
const TOAST_FOR: Duration = Duration::from_secs(6);
const PENDING_PLAY_FOR: Duration = Duration::from_secs(10);
const SIDEBAR_PLAYLISTS: usize = 8;
const BACK_STACK_LIMIT: usize = 64;
/// Rows the Up next panel shows.
pub const UP_NEXT_ROWS: usize = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryTab {
    Playlists,
    Albums,
    Artists,
    Shows,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Home,
    Search,
    Library(LibraryTab),
    LikedSongs,
    History,
    Album(String),
    Artist(String),
    Playlist(String),
    Show(String),
}

impl Route {
    /// What the breadcrumb calls the page you came from.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Search => "Search",
            Self::Library(_) => "Your Library",
            Self::LikedSongs => "Liked Songs",
            Self::History => "History",
            Self::Album(_) => "Album",
            Self::Artist(_) => "Artist",
            Self::Playlist(_) => "Playlist",
            Self::Show(_) => "Show",
        }
    }

    /// Whether the sidebar should mark `item` as the current section.
    pub fn is_in_section(&self, item: &Route) -> bool {
        matches!(
            (self, item),
            (Self::Library(_), Self::Library(_))
                | (Self::Home, Self::Home)
                | (Self::Search, Self::Search)
                | (Self::LikedSongs, Self::LikedSongs)
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum InstallState {
    Idle,
    Downloading { received: u64, total: Option<u64> },
    Verifying,
    Finalizing,
    Failed(String),
}

impl InstallState {
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Self::Downloading { .. } | Self::Verifying | Self::Finalizing
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    Logout,
    ClearHistory,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub text: SharedString,
    serial: u64,
}

/// `SONEAR_DEMO_ROUTE` opens the demo on a page, for screenshots.
#[cfg(feature = "demo")]
fn initial_route() -> Route {
    let route = std::env::var("SONEAR_DEMO_ROUTE").unwrap_or_default();
    let (kind, id) = route.split_once(':').unwrap_or((route.as_str(), ""));

    match kind {
        "search" => Route::Search,
        "library" => Route::Library(match id {
            "albums" => LibraryTab::Albums,
            "artists" => LibraryTab::Artists,
            "shows" => LibraryTab::Shows,
            _ => LibraryTab::Playlists,
        }),
        "liked" => Route::LikedSongs,
        "history" => Route::History,
        "album" => Route::Album(id.to_owned()),
        "artist" => Route::Artist(id.to_owned()),
        "playlist" => Route::Playlist(id.to_owned()),
        "show" => Route::Show(id.to_owned()),
        _ => Route::Home,
    }
}

#[cfg(not(feature = "demo"))]
fn initial_route() -> Route {
    Route::Home
}

enum InstallMessage {
    Progress(InstallProgress),
    Finished(Result<(), String>),
}

/// UI-facing state: everything the views read, fed by `AppEvent`s.
pub struct AppStore {
    pub core: Arc<Core>,
    runtime: Handle,
    pub playback: PlaybackModel,
    /// The user's own queue, which Sonear holds and hands to Soloist one track at a time.
    pub queue: QueueSnapshot,
    pub soloist: Status,
    pub expiry_days: Option<i64>,
    pub route: Route,
    back_stack: Vec<Route>,
    pub install: InstallState,
    pub key_error: Option<String>,
    pub editing_key: bool,
    pub sidebar_collapsed: bool,
    pub right_panel_open: bool,
    pub ai_hint_dismissed: bool,
    pub account: AccountState,
    pub profile: Slot<Profile>,
    pub sidebar_playlists: Vec<Playlist>,
    /// Details of queue entries Soloist sent without any. `None` means the lookup failed.
    pub queue_tracks: HashMap<String, Option<Track>>,
    queue_requested: HashSet<String>,
    pub rate: RateLimit,
    pub toast: Option<Toast>,
    pub pending_play: bool,
    pub avatar_menu_open: bool,
    pub confirm: Option<Confirm>,
    /// Bumped whenever the history changes, so history views reload.
    pub history_version: u64,
    /// Width of the main panel, written by the shell and read by grid pages.
    pub main_width: Rc<Cell<f32>>,
    serial: u64,
    ticker: Option<Task<()>>,
    loads: Vec<Task<()>>,
}

impl AppStore {
    pub fn new(core: Arc<Core>, runtime: Handle, cx: &mut Context<Self>) -> Self {
        let config = core.config.get();
        let soloist = core
            .soloist
            .as_ref()
            .map_or(Status::Stopped, |manager| manager.status());
        let mut store = Self {
            playback: core.player.snapshot(),
            queue: core.queue.snapshot(),
            account: core.account.state(),
            core: core.clone(),
            runtime,
            soloist,
            expiry_days: None,
            route: initial_route(),
            back_stack: Vec::new(),
            install: InstallState::Idle,
            key_error: None,
            editing_key: false,
            sidebar_collapsed: config.sidebar_collapsed,
            right_panel_open: config.right_panel.open,
            ai_hint_dismissed: config.ui.ai_hint_dismissed,
            profile: Slot::default(),
            sidebar_playlists: Vec::new(),
            queue_tracks: HashMap::new(),
            queue_requested: HashSet::new(),
            rate: RateLimit::default(),
            toast: None,
            pending_play: false,
            avatar_menu_open: false,
            confirm: None,
            history_version: 0,
            main_width: Rc::new(Cell::new(900.)),
            serial: 0,
            ticker: None,
            loads: Vec::new(),
        };
        let mut events = core.subscribe();

        cx.spawn(async move |this, cx| {
            loop {
                let event = match events.recv().await {
                    Ok(event) => event,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };

                if this.update(cx, |store, cx| store.apply(event, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        if store.account == AccountState::Connected {
            store.load_session(cx);
            store.ensure_current_like();
            store.resolve_queue(cx);
        }

        store
    }

    pub fn env(&self) -> Env {
        Env {
            core: self.core.clone(),
            runtime: self.runtime.clone(),
        }
    }

    pub fn is_demo(&self) -> bool {
        self.core.is_demo()
    }

    pub fn connected(&self) -> bool {
        self.account == AccountState::Connected
    }

    pub fn user_id(&self) -> Option<&str> {
        self.profile.value().map(|profile| profile.id.as_str())
    }

    /// Loads the profile and the playlists the sidebar lists.
    fn load_session(&mut self, cx: &mut Context<Self>) {
        let env = self.env();
        let profile = data::load(
            &env,
            Some("profile".to_owned()),
            data::fetch(|core| async move { core.library.profile().await }),
            cx,
            |store: &mut Self, message, _| store.profile.apply(message),
        );
        let playlists = data::load(
            &env,
            Some("sidebar-playlists".to_owned()),
            data::fetch(|core| async move { core.library.my_playlists(0).await }),
            cx,
            |store: &mut Self, message, _| {
                if let Load::Ready { value, .. } = message {
                    store.sidebar_playlists =
                        value.items.into_iter().take(SIDEBAR_PLAYLISTS).collect();
                }
            },
        );

        self.loads = vec![profile, playlists];
    }

    fn apply(&mut self, event: AppEvent, cx: &mut Context<Self>) {
        match event {
            AppEvent::Playback(model) => {
                let started = model.is_playing()
                    || model.is_buffering()
                    || model.item.as_ref().map(|item| &item.uri)
                        != self.playback.item.as_ref().map(|item| &item.uri);

                let track_changed = model.item.as_ref().map(|item| &item.uri)
                    != self.playback.item.as_ref().map(|item| &item.uri);

                self.playback = *model;
                self.resolve_queue(cx);

                if started {
                    self.pending_play = false;
                }

                if track_changed {
                    self.ensure_current_like();
                    self.core.history.sync_soon();
                }

                self.sync_ticker(cx);
            }
            AppEvent::Soloist(status) => self.soloist = status,
            AppEvent::SoloistExpiry { days_left } => self.expiry_days = days_left,
            AppEvent::Account(state) => self.apply_account(state, cx),
            AppEvent::Likes => {}
            AppEvent::HistoryChanged => self.history_version += 1,
            AppEvent::Queue => self.queue = self.core.queue.snapshot(),
            AppEvent::RateLimit(rate) => self.apply_rate(rate, cx),
            AppEvent::Notice(text) => {
                self.pending_play = false;
                self.show_toast(text, cx);
            }
        }

        cx.notify();
    }

    fn apply_account(&mut self, state: AccountState, cx: &mut Context<Self>) {
        let previous = std::mem::replace(&mut self.account, state.clone());

        match state {
            AccountState::LoggingIn { url } => {
                if previous != (AccountState::LoggingIn { url: url.clone() }) {
                    cx.open_url(&url);
                }
            }
            AccountState::Connected => {
                if previous != AccountState::Connected {
                    self.load_session(cx);
                    self.ensure_current_like();
                    self.resolve_queue(cx);
                }
            }
            AccountState::Disconnected
            | AccountState::SessionExpired
            | AccountState::LoginFailed(_) => {
                self.profile = Slot::default();
                self.sidebar_playlists.clear();
                self.queue = QueueSnapshot::default();
                self.queue_tracks.clear();
                self.queue_requested.clear();
                self.loads.clear();
            }
        }
    }

    /// Soloist can send queue entries with only a URI. Looks up the ones on
    /// screen, each once.
    fn resolve_queue(&mut self, cx: &mut Context<Self>) {
        let missing: Vec<String> = self
            .playback
            .upcoming
            .iter()
            .take(UP_NEXT_ROWS)
            .filter(|entry| entry.item.name().is_empty())
            .map(|entry| entry.item.uri.clone())
            .collect();

        self.ensure_queue_rows(missing, cx);
    }

    /// Looks up the details of `uris` that have none yet, each URI once. Asks
    /// only for what the panel shows: every lookup is a request against the
    /// developer account's quota.
    pub fn ensure_queue_rows(&mut self, uris: Vec<String>, cx: &mut Context<Self>) {
        if !self.connected() {
            return;
        }

        let missing: Vec<String> = uris
            .into_iter()
            .filter(|uri| !self.queue_tracks.contains_key(uri))
            .filter(|uri| self.queue_requested.insert(uri.clone()))
            .collect();

        if missing.is_empty() {
            return;
        }

        let (sender, mut inbox) = mpsc::unbounded_channel();

        for uri in missing {
            let sender = sender.clone();
            let history = self.core.history.clone();

            self.runtime.spawn(async move {
                let track = history.resolver().resolve(&uri).await;
                let _ = sender.send((uri, track));
            });
        }

        drop(sender);
        cx.spawn(async move |this, cx| {
            while let Some((uri, track)) = inbox.recv().await {
                if this
                    .update(cx, |store, cx| {
                        store.queue_tracks.insert(uri, track);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// Adds a track to the user's queue. Episodes and local files can't be queued.
    pub fn add_to_queue(&mut self, track: &Track, cx: &mut Context<Self>) {
        if track.kind != ItemKind::Track || track.is_local || !track.playable {
            return;
        }

        self.core.history.resolver().remember(track);
        self.queue_tracks
            .insert(track.uri.clone(), Some(track.clone()));
        self.queue_requested.insert(track.uri.clone());

        if self.core.queue.add(&track.uri, QueueSource::User) {
            self.show_toast("Added to queue", cx);
        } else {
            self.show_toast("Your queue is full.", cx);
        }
    }

    fn apply_rate(&mut self, rate: RateLimit, cx: &mut Context<Self>) {
        self.rate = rate;

        let Some(until) = rate.paused_until else {
            return;
        };
        let wait = until.saturating_duration_since(std::time::Instant::now());

        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(wait + Duration::from_millis(150))
                .await;

            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    pub fn show_toast(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.serial += 1;

        let serial = self.serial;

        self.toast = Some(Toast {
            text: text.into(),
            serial,
        });
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_FOR).await;

            let _ = this.update(cx, |store, cx| {
                if store
                    .toast
                    .as_ref()
                    .is_some_and(|toast| toast.serial == serial)
                {
                    store.toast = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub fn dismiss_toast(&mut self, cx: &mut Context<Self>) {
        self.toast = None;
        cx.notify();
    }

    /// The repaint timer only runs while playing, so an idle app uses no CPU.
    fn sync_ticker(&mut self, cx: &mut Context<Self>) {
        if !self.playback.is_playing() {
            self.ticker = None;
            return;
        }

        if self.ticker.is_some() {
            return;
        }

        self.ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(REPAINT_EVERY).await;

                let keep_going = this
                    .update(cx, |store, cx| {
                        if !store.playback.is_playing() {
                            store.ticker = None;
                            return false;
                        }

                        cx.notify();

                        true
                    })
                    .unwrap_or(false);

                if !keep_going {
                    break;
                }
            }
        }));
    }

    pub fn navigate(&mut self, route: Route, cx: &mut Context<Self>) {
        self.avatar_menu_open = false;

        if self.route == route {
            return;
        }

        let from = std::mem::replace(&mut self.route, route);

        self.back_stack.push(from);

        if self.back_stack.len() > BACK_STACK_LIMIT {
            self.back_stack.remove(0);
        }

        cx.notify();
    }

    pub fn can_go_back(&self) -> bool {
        !self.back_stack.is_empty()
    }

    pub fn previous_route(&self) -> Option<&Route> {
        self.back_stack.last()
    }

    pub fn back(&mut self, cx: &mut Context<Self>) {
        if let Some(route) = self.back_stack.pop() {
            self.route = route;
            self.avatar_menu_open = false;
            cx.notify();
        }
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;

        let collapsed = self.sidebar_collapsed;

        self.core
            .config
            .update(|config| config.sidebar_collapsed = collapsed);
        cx.notify();
    }

    pub fn toggle_right_panel(&mut self, cx: &mut Context<Self>) {
        self.right_panel_open = !self.right_panel_open;

        let open = self.right_panel_open;

        self.core
            .config
            .update(|config| config.right_panel.open = open);
        cx.notify();
    }

    pub fn dismiss_ai_hint(&mut self, cx: &mut Context<Self>) {
        self.ai_hint_dismissed = true;
        self.core
            .config
            .update(|config| config.ui.ai_hint_dismissed = true);
        cx.notify();
    }

    fn begin_play(&mut self, cx: &mut Context<Self>) {
        self.pending_play = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PENDING_PLAY_FOR).await;

            let _ = this.update(cx, |store, cx| {
                if store.pending_play {
                    store.pending_play = false;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// Starts a whole album, playlist or artist from its first track, or
    /// continues it if it is what's paused right now.
    pub fn play_context(&mut self, uri: &str, cx: &mut Context<Self>) {
        let paused_here = !self.playback.is_playing()
            && self.playback.item.is_some()
            && self
                .playback
                .context
                .as_ref()
                .is_some_and(|context| context.uri == uri);

        self.begin_play(cx);

        if paused_here {
            self.core.player.play(None);
        } else {
            self.core.player.play_context(uri);
        }
    }

    /// Starts a context at a row.
    pub fn play_in_context(
        &mut self,
        context_uri: &str,
        offset: PlayOffset,
        cx: &mut Context<Self>,
    ) {
        self.begin_play(cx);
        self.core.player.play_in_context(context_uri, offset);
    }

    /// Plays a list of tracks from `start`, for lists with no context URI.
    pub fn play_tracks(&mut self, uris: Vec<String>, start: usize, cx: &mut Context<Self>) {
        self.begin_play(cx);
        self.core
            .player
            .play_request(PlaybackRequest::Tracks { uris, start });
    }

    pub fn toggle_like(&mut self, uri: &str, cx: &mut Context<Self>) {
        let likes = self.core.likes.clone();
        let uri = uri.to_owned();

        self.runtime.spawn(async move {
            let _ = likes.toggle(&uri).await;
        });
        cx.notify();
    }

    /// Checks the heart of the track that is playing now.
    fn ensure_current_like(&self) {
        if let Some(item) = &self.playback.item
            && item.uri.starts_with("spotify:track:")
        {
            self.ensure_likes(vec![item.uri.clone()]);
        }
    }

    pub fn ensure_likes(&self, uris: Vec<String>) {
        let likes = self.core.likes.clone();

        self.runtime.spawn(async move {
            likes.ensure(&uris).await;
        });
    }

    /// Starts the Spotify login for a Client ID.
    pub fn connect(&mut self, client_id: &str, cx: &mut Context<Self>) {
        self.core.account.begin_login(client_id);
        cx.notify();
    }

    pub fn cancel_connect(&mut self, cx: &mut Context<Self>) {
        self.core.account.cancel_login();
        cx.notify();
    }

    pub fn toggle_avatar_menu(&mut self, cx: &mut Context<Self>) {
        self.avatar_menu_open = !self.avatar_menu_open;
        cx.notify();
    }

    pub fn ask(&mut self, confirm: Confirm, cx: &mut Context<Self>) {
        self.avatar_menu_open = false;
        self.confirm = Some(confirm);
        cx.notify();
    }

    pub fn dismiss_confirm(&mut self, cx: &mut Context<Self>) {
        self.confirm = None;
        cx.notify();
    }

    pub fn accept_confirm(&mut self, cx: &mut Context<Self>) {
        let Some(confirm) = self.confirm.take() else {
            return;
        };

        match confirm {
            Confirm::Logout => self.disconnect(cx),
            Confirm::ClearHistory => {
                let history = self.core.history.clone();

                self.runtime.spawn(async move {
                    let _ = history.clear().await;
                });
            }
        }

        cx.notify();
    }

    fn disconnect(&mut self, cx: &mut Context<Self>) {
        let account = self.core.account.clone();
        let (done, mut inbox) = mpsc::unbounded_channel();

        self.route = Route::Home;
        self.back_stack.clear();
        self.runtime.spawn(async move {
            let problems = account.disconnect().await;

            if let Some(covers) = crate::http::cover_cache() {
                covers.reset();
            }

            let _ = done.send(problems);
        });

        cx.spawn(async move |this, cx| {
            if let Some(problems) = inbox.recv().await {
                let _ = this.update(cx, |store, cx| {
                    let text = if problems.is_empty() {
                        "Disconnected. The login, cached data, history and queue were removed from this computer."
                            .to_owned()
                    } else {
                        format!("Disconnected, but some data could not be removed: {}", problems.join("; "))
                    };

                    store.show_toast(text, cx);
                });
            }
        })
        .detach();
    }

    pub fn start_install(&mut self, cx: &mut Context<Self>) {
        let Some(manager) = self.core.soloist.clone() else {
            return;
        };

        if self.install.is_busy() {
            return;
        }

        self.install = InstallState::Downloading {
            received: 0,
            total: None,
        };
        self.key_error = None;
        cx.notify();

        let (messages, mut inbox) = mpsc::unbounded_channel();
        let progress = messages.clone();

        self.runtime.spawn(async move {
            let result = manager
                .install(&move |step| {
                    let _ = progress.send(InstallMessage::Progress(step));
                })
                .await
                .map(|_| ())
                .map_err(|error| error.to_string());

            let _ = messages.send(InstallMessage::Finished(result));
        });

        cx.spawn(async move |this, cx| {
            while let Some(message) = inbox.recv().await {
                let finished = matches!(message, InstallMessage::Finished(_));

                if this
                    .update(cx, |store, cx| {
                        store.install = match message {
                            InstallMessage::Progress(InstallProgress::Downloading {
                                received,
                                total,
                            }) => InstallState::Downloading { received, total },
                            InstallMessage::Progress(InstallProgress::Verifying) => {
                                InstallState::Verifying
                            }
                            InstallMessage::Progress(InstallProgress::Finalizing) => {
                                InstallState::Finalizing
                            }
                            InstallMessage::Progress(InstallProgress::Done)
                            | InstallMessage::Finished(Ok(())) => InstallState::Idle,
                            InstallMessage::Finished(Err(reason)) => InstallState::Failed(reason),
                        };
                        cx.notify();
                    })
                    .is_err()
                    || finished
                {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn save_key(&mut self, key: String, cx: &mut Context<Self>) {
        let Some(manager) = self.core.soloist.clone() else {
            return;
        };

        if key.trim().is_empty() {
            self.key_error = Some("Paste your Soloist key first.".to_owned());
            cx.notify();
            return;
        }

        self.key_error = None;
        self.editing_key = false;
        cx.notify();

        let (done, mut result) = mpsc::unbounded_channel();

        self.runtime.spawn(async move {
            let outcome = manager
                .save_key(&key)
                .await
                .map_err(|error| error.to_string());

            let _ = done.send(outcome);
        });

        cx.spawn(async move |this, cx| {
            if let Some(Err(reason)) = result.recv().await {
                let _ = this.update(cx, |store, cx| {
                    store.key_error = Some(format!("Could not save the key: {reason}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub fn edit_key(&mut self, cx: &mut Context<Self>) {
        self.editing_key = true;
        self.key_error = None;
        cx.notify();
    }

    pub fn retry(&mut self, cx: &mut Context<Self>) {
        let Some(manager) = self.core.soloist.clone() else {
            return;
        };

        self.runtime.spawn(async move {
            manager.start().await;
        });
        cx.notify();
    }
}

pub fn new_entity(core: Arc<Core>, runtime: Handle, cx: &mut gpui_kit::App) -> Entity<AppStore> {
    cx.new(|cx| AppStore::new(core, runtime, cx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_routes_share_a_sidebar_section() {
        assert!(
            Route::Library(LibraryTab::Albums)
                .is_in_section(&Route::Library(LibraryTab::Playlists))
        );
        assert!(Route::Home.is_in_section(&Route::Home));
        assert!(!Route::Search.is_in_section(&Route::Home));
        assert!(!Route::Album("x".into()).is_in_section(&Route::Home));
    }

    #[test]
    fn routes_have_breadcrumb_labels() {
        assert_eq!(Route::Library(LibraryTab::Shows).label(), "Your Library");
        assert_eq!(Route::Playlist("p".into()).label(), "Playlist");
    }
}
