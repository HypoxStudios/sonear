use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::{AnyWindowHandle, AppContext as _, Context, Entity, SharedString, Task, Window};
use sonear_core::config::LEGAL_VERSION;
use sonear_core::data_dir::check_target;
use sonear_core::model::ItemKind;
use sonear_core::model::{PlayOffset, PlaybackRequest, Playlist, Profile, Track};
use sonear_core::queue::QueueSource;
use sonear_core::{
    AccountState, AppEvent, AppPaths, Core, PlaybackModel, QueueSnapshot, SpotifyLink,
};
use sonear_soloist::Status;
use sonear_soloist::install::InstallProgress;
use sonear_spotify::RateLimit;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tracing::warn;

use crate::cover_accent::{self, Accent};
use crate::data::{self, Env, Load, Slot};
use crate::keymap;
use crate::legal::LegalDoc;
use crate::theme;
use crate::ui::mini_player;
use crate::{platform_window, tray};

mod updates;

pub use updates::UpdateState;
use updates::UpdateTask;

const TOAST_FOR: Duration = Duration::from_secs(6);
const PENDING_PLAY_FOR: Duration = Duration::from_secs(10);
const SIDEBAR_PLAYLISTS: usize = 8;
/// Longest device name the settings page accepts.
pub const DEVICE_NAME_MAX: usize = 64;
const BACK_STACK_LIMIT: usize = 64;
const ACCENT_MEMORY: usize = 32;
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
    Settings,
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
            Self::Settings => "Settings",
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
    DeleteAll,
    /// Moves the data folder to `AppStore::pending_data_move`.
    MoveData,
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
        "settings" => Route::Settings,
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
    pub update: UpdateState,
    pub check_for_updates: bool,
    updates: Option<UpdateTask>,
    /// Set once the user has started the restart, so a second click does nothing.
    update_restarting: bool,
    pub key_error: Option<String>,
    pub editing_key: bool,
    pub sidebar_collapsed: bool,
    pub right_panel_open: bool,
    /// Whether the right panel shows lyrics instead of the queue.
    pub right_panel_lyrics: bool,
    pub lyrics_enabled: bool,
    pub cover_accent: bool,
    pub close_to_tray: bool,
    main_window_hidden: bool,
    mini_player: Option<AnyWindowHandle>,
    main_window: Option<AnyWindowHandle>,
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
    /// Where the data folder moves to once the user confirms.
    pub pending_data_move: Option<PathBuf>,
    pub shortcuts_open: bool,
    pub legal_open: Option<LegalDoc>,
    /// A query the Search page should fill in and run, taken when it reads it.
    pub pending_search: Option<String>,
    /// A link opened before the app could act on it, opened once it can.
    pending_link: Option<SpotifyLink>,
    /// Bumped whenever the history changes, so history views reload.
    pub history_version: u64,
    /// Bumped whenever collected playlist tracks change, so playlist pages reload them.
    pub seen_version: u64,
    /// The playlist the last bump was for.
    pub seen_changed: Option<String>,
    /// Width of the main panel, written by the shell and read by grid pages.
    pub main_width: Rc<Cell<f32>>,
    serial: u64,
    loads: Vec<Task<()>>,
    /// What the accent was last worked out for: a cover URL, or a track URI without one.
    accent_source: Option<String>,
    /// Colours found for recent covers; `None` means the cover has no usable colour.
    accent_memory: Vec<(String, Option<Accent>)>,
    accent_serial: u64,
    accent_load: Option<Task<()>>,
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
            update: UpdateState::Idle,
            check_for_updates: config.check_for_updates,
            updates: None,
            update_restarting: false,
            key_error: None,
            editing_key: false,
            sidebar_collapsed: config.sidebar_collapsed,
            right_panel_open: config.right_panel.open,
            right_panel_lyrics: config.right_panel.lyrics,
            lyrics_enabled: config.lyrics_enabled,
            cover_accent: config.cover_accent,
            close_to_tray: config.close_to_tray,
            main_window_hidden: false,
            mini_player: None,
            main_window: None,
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
            pending_data_move: None,
            shortcuts_open: false,
            legal_open: None,
            pending_search: None,
            pending_link: None,
            history_version: 0,
            seen_version: 0,
            seen_changed: None,
            main_width: Rc::new(Cell::new(900.)),
            serial: 0,
            loads: Vec::new(),
            accent_source: None,
            accent_memory: Vec::new(),
            accent_serial: 0,
            accent_load: None,
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

        store.sync_accent(cx);
        store.sync_tray();
        store.start_updates(cx);

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

    /// Whether the setup screen replaces the pages.
    pub fn needs_setup(&self) -> bool {
        (!self.is_demo()
            && (!self.legal_accepted()
                || !matches!(self.soloist, Status::Ready { .. })
                || !self.connected()))
            || (self.is_demo() && std::env::var("SONEAR_DEMO_SETUP").is_ok())
    }

    /// Whether views that follow the playing position should repaint on a timer.
    pub fn ticking(&self) -> bool {
        self.playback.is_playing() && !self.main_window_hidden
    }

    /// Views that repaint on a timer stop while the window is hidden and pick
    /// up again from the notification this sends when it comes back.
    pub fn set_main_window_hidden(&mut self, hidden: bool, cx: &mut Context<Self>) {
        if self.main_window_hidden == hidden {
            return;
        }

        self.main_window_hidden = hidden;
        cx.notify();
    }

    pub fn set_main_window(&mut self, handle: AnyWindowHandle) {
        self.main_window = Some(handle);
    }

    /// Brings the main window back from the tray, the taskbar or behind other windows.
    pub fn show_main_window(&mut self, cx: &mut Context<Self>) {
        self.set_main_window_hidden(false, cx);

        let Some(handle) = self.main_window else {
            return;
        };

        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, _| {
                if let Some(hwnd) = platform_window::hwnd(window) {
                    platform_window::show(hwnd);
                }

                window.activate_window();
            });
        });
    }

    pub fn set_close_to_tray(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.close_to_tray = enabled;
        self.core
            .config
            .update(|config| config.close_to_tray = enabled);
        cx.notify();
    }

    /// Handles a request to close the main window. Hides it into the tray when
    /// that is on and the tray icon exists, and quits the app otherwise. The
    /// return value says whether the window may close.
    pub fn request_close(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        if self.close_to_tray
            && !self.needs_setup()
            && tray::installed()
            && let Some(hwnd) = platform_window::hwnd(window)
        {
            platform_window::hide(hwnd);
            self.set_main_window_hidden(true, cx);
            self.show_tray_hint();

            return false;
        }

        cx.quit();

        true
    }

    fn show_tray_hint(&self) {
        if self.core.config.get().tray_hint_shown {
            return;
        }

        tray::show_hint();
        self.core
            .config
            .update(|config| config.tray_hint_shown = true);
    }

    pub fn mini_player_open(&self) -> bool {
        self.mini_player.is_some()
    }

    /// Opens the mini player, or closes it and remembers where it was.
    pub fn toggle_mini_player(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.mini_player.take() {
            let core = self.core.clone();

            self.sync_tray();
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| {
                    mini_player::remember_position(window, &core);
                    window.remove_window();
                });
            });
            cx.notify();

            return;
        }

        let store = cx.entity();
        let saved = self.core.config.get().mini_player_position;
        let core = self.core.clone();

        // The new window draws straight away and reads this store, which can't
        // happen while the store is being updated.
        cx.defer(move |cx| {
            let opened = mini_player::open(store.clone(), saved, core, cx);

            store.update(cx, |state, cx| match opened {
                Ok(handle) => {
                    state.mini_player = Some(handle);
                    state.sync_tray();
                    cx.notify();
                }
                Err(error) => {
                    warn!(%error, "could not open the mini player");
                    state.show_toast("Could not open the mini player.", cx);
                }
            });
        });
    }

    /// The mini player's window is gone, however it was closed.
    pub fn mini_player_closed(&mut self, cx: &mut Context<Self>) {
        if self.mini_player.take().is_some() {
            self.sync_tray();
            cx.notify();
        }
    }

    fn sync_tray(&self) {
        tray::set_state(tray::MenuState {
            playing: self.playback.is_playing(),
            has_item: self.playback.item.is_some(),
            mini_open: self.mini_player.is_some(),
        });
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

                if self.accent_source != self.accent_key() {
                    self.sync_accent(cx);
                }

                self.sync_tray();
            }
            AppEvent::Soloist(status) => {
                self.soloist = status;
                self.open_pending_link(cx);
            }
            AppEvent::SoloistExpiry { days_left } => self.expiry_days = days_left,
            AppEvent::Account(state) => self.apply_account(state, cx),
            AppEvent::AccountDataWiped => self.drop_account_caches(),
            AppEvent::Likes => {}
            AppEvent::HistoryChanged => self.history_version += 1,
            AppEvent::SeenTracks { playlist_id } => {
                self.seen_version += 1;
                self.seen_changed = Some(playlist_id);
            }
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

                self.open_pending_link(cx);
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

    fn drop_account_caches(&mut self) {
        if let Some(covers) = crate::http::cover_cache() {
            covers.reset();
        }

        self.queue_tracks.clear();
        self.queue_requested.clear();
        self.history_version += 1;
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

    pub fn navigate(&mut self, route: Route, cx: &mut Context<Self>) {
        self.avatar_menu_open = false;
        cx.notify();

        if self.route == route {
            return;
        }

        let from = std::mem::replace(&mut self.route, route);

        self.back_stack.push(from);

        if self.back_stack.len() > BACK_STACK_LIMIT {
            self.back_stack.remove(0);
        }
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

    /// Whether the right slot is open on the lyrics.
    pub fn lyrics_showing(&self) -> bool {
        self.right_panel_open && self.lyrics_enabled && self.right_panel_lyrics
    }

    /// Whether the lyrics are on screen: showing, and in a window that is not hidden.
    pub fn lyrics_visible(&self) -> bool {
        self.lyrics_showing() && !self.main_window_hidden
    }

    /// Whether the right slot is open on the queue.
    pub fn queue_showing(&self) -> bool {
        self.right_panel_open && !(self.lyrics_enabled && self.right_panel_lyrics)
    }

    /// Opens the lyrics, or closes the panel if the lyrics are already showing.
    pub fn show_lyrics_panel(&mut self, cx: &mut Context<Self>) {
        if !self.lyrics_enabled {
            return;
        }

        if self.lyrics_showing() {
            self.right_panel_open = false;
        } else {
            self.right_panel_open = true;
            self.right_panel_lyrics = true;
        }

        self.save_right_panel();
        cx.notify();
    }

    /// Opens the queue, or closes the panel if the queue is already showing.
    pub fn show_queue_panel(&mut self, cx: &mut Context<Self>) {
        if self.queue_showing() {
            self.right_panel_open = false;
        } else {
            self.right_panel_open = true;
            self.right_panel_lyrics = false;
        }

        self.save_right_panel();
        cx.notify();
    }

    fn save_right_panel(&self) {
        let (open, lyrics) = (self.right_panel_open, self.right_panel_lyrics);

        self.core.config.update(|config| {
            config.right_panel.open = open;
            config.right_panel.lyrics = lyrics;
        });
    }

    pub fn set_lyrics_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.lyrics_enabled = enabled;
        self.core
            .config
            .update(|config| config.lyrics_enabled = enabled);
        cx.notify();
    }

    pub fn set_cover_accent(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.cover_accent = enabled;
        self.core
            .config
            .update(|config| config.cover_accent = enabled);
        self.sync_accent(cx);
        cx.notify();
    }

    fn accent_key(&self) -> Option<String> {
        let item = self.playback.item.as_ref()?;

        Some(
            item.cover_url("small")
                .map_or_else(|| item.uri.clone(), str::to_owned),
        )
    }

    fn apply_accent(&mut self, accent: Option<Accent>, cx: &mut Context<Self>) {
        if theme::set_accent(accent.map(Accent::hsla)) {
            theme::refresh_accent(cx);
        }
    }

    /// Works out the accent for the playing track's cover. A slow answer for
    /// an earlier track is dropped.
    fn sync_accent(&mut self, cx: &mut Context<Self>) {
        self.accent_serial += 1;
        self.accent_load = None;
        self.accent_source = self.accent_key();

        let serial = self.accent_serial;
        let item = self.playback.item.as_ref().filter(|_| self.cover_accent);
        let Some(item) = item else {
            self.apply_accent(None, cx);

            return;
        };
        let Some(url) = item.cover_url("small").map(str::to_owned) else {
            self.apply_accent(None, cx);

            return;
        };

        if let Some((_, known)) = self.accent_memory.iter().find(|(key, _)| *key == url) {
            let known = *known;

            self.apply_accent(known, cx);

            return;
        }

        let work = self.runtime.spawn({
            let url = url.clone();

            async move {
                let bytes = cover_bytes(&url).await?;

                tokio::task::spawn_blocking(move || cover_accent::accent_from_image(&bytes))
                    .await
                    .ok()
                    .flatten()
            }
        });

        self.accent_load = Some(cx.spawn(async move |this, cx| {
            let found = work.await.ok().flatten();

            let _ = this.update(cx, |store, cx| {
                if store.accent_serial != serial {
                    return;
                }

                if store.accent_memory.len() >= ACCENT_MEMORY {
                    store.accent_memory.remove(0);
                }

                store.accent_memory.push((url, found));
                store.apply_accent(found, cx);
            });
        }));
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

    /// Plays Liked Songs as one context when the user is known, so it keeps
    /// going past the loaded rows. `uris` and `start` are what plays when
    /// Spotify refuses that.
    pub fn play_liked(
        &mut self,
        offset_uri: Option<String>,
        uris: Vec<String>,
        start: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = self.user_id().map(str::to_owned) else {
            self.play_tracks(uris, start, cx);

            return;
        };

        self.begin_play(cx);
        self.core.player.play_request(PlaybackRequest::LikedSongs {
            user_id,
            offset_uri,
            uris,
            start,
        });
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

    pub fn close_avatar_menu(&mut self, cx: &mut Context<Self>) {
        self.avatar_menu_open = false;
        cx.notify();
    }

    pub fn ask(&mut self, confirm: Confirm, cx: &mut Context<Self>) {
        self.avatar_menu_open = false;
        self.confirm = Some(confirm);
        cx.notify();
    }

    pub fn open_shortcuts(&mut self, cx: &mut Context<Self>) {
        self.avatar_menu_open = false;
        self.shortcuts_open = true;
        cx.notify();
    }

    pub fn close_shortcuts(&mut self, cx: &mut Context<Self>) {
        self.shortcuts_open = false;
        cx.notify();
    }

    pub fn open_legal(&mut self, doc: LegalDoc, cx: &mut Context<Self>) {
        self.legal_open = Some(doc);
        cx.notify();
    }

    pub fn close_legal(&mut self, cx: &mut Context<Self>) {
        self.legal_open = None;
        cx.notify();
    }

    pub fn legal_accepted(&self) -> bool {
        self.core.config.get().legal_accepted >= LEGAL_VERSION
    }

    pub fn accept_legal(&mut self, cx: &mut Context<Self>) {
        self.core
            .config
            .update(|config| config.legal_accepted = LEGAL_VERSION);
        cx.notify();
    }

    /// Opens an album, playlist, artist or show, or plays a track or episode.
    /// Waits until the app is logged in and Soloist is ready.
    pub fn open_link(&mut self, link: SpotifyLink, cx: &mut Context<Self>) {
        if !self.can_open_links() {
            self.pending_link = Some(link);

            return;
        }

        match link {
            SpotifyLink::Album(id) => self.navigate(Route::Album(id), cx),
            SpotifyLink::Playlist(id) => self.navigate(Route::Playlist(id), cx),
            SpotifyLink::Artist(id) => self.navigate(Route::Artist(id), cx),
            SpotifyLink::Show(id) => self.navigate(Route::Show(id), cx),
            SpotifyLink::Track(_) => self.open_track(link.uri(), cx),
            SpotifyLink::Episode(_) => self.play_tracks(vec![link.uri()], 0, cx),
        }

        cx.notify();
    }

    /// Plays a track inside its album and shows the album, so the queue goes
    /// on from there. Plays it alone when the lookup finds no album.
    fn open_track(&mut self, uri: String, cx: &mut Context<Self>) {
        let history = self.core.history.clone();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let lookup = uri.clone();

        self.begin_play(cx);
        self.runtime.spawn(async move {
            let track = history.resolver().resolve(&lookup).await;
            let _ = sender.send(track);
        });
        cx.spawn(async move |this, cx| {
            let track = receiver.await.ok().flatten();
            let album = track
                .and_then(|track| track.album)
                .and_then(|album| album.id.clone().map(|id| (id, album.uri)));

            let _ = this.update(cx, |store, cx| {
                match album {
                    Some((id, album_uri)) => {
                        store.navigate(Route::Album(id), cx);
                        store.play_in_context(&album_uri, PlayOffset::Uri(uri), cx);
                    }
                    None => store.play_tracks(vec![uri], 0, cx),
                }

                cx.notify();
            });
        })
        .detach();
    }

    fn can_open_links(&self) -> bool {
        self.connected() && (self.is_demo() || matches!(self.soloist, Status::Ready { .. }))
    }

    fn open_pending_link(&mut self, cx: &mut Context<Self>) {
        if !self.can_open_links() {
            return;
        }

        if let Some(link) = self.pending_link.take() {
            self.open_link(link, cx);
        }
    }

    /// Opens the Search page with `query` filled in, also when it is already open.
    pub fn search_for(&mut self, query: &str, cx: &mut Context<Self>) {
        self.pending_search = Some(query.to_owned());
        self.navigate(Route::Search, cx);
        cx.notify();
    }

    /// Checks that the data can move to `target` and asks the user to confirm.
    pub fn propose_data_move(&mut self, target: PathBuf, cx: &mut Context<Self>) {
        if !self.is_demo()
            && let Err(error) = check_target(&self.core.paths.local_root(), &target)
        {
            self.show_toast(error.to_string(), cx);

            return;
        }

        self.ask_move_data(target, cx);
    }

    /// Like `propose_data_move`, to the folder Sonear uses unless told otherwise.
    pub fn propose_default_data_move(&mut self, cx: &mut Context<Self>) {
        match AppPaths::default_local_root() {
            Some(target) => self.propose_data_move(target, cx),
            None => self.show_toast("Could not find the default folder.", cx),
        }
    }

    pub fn ask_move_data(&mut self, target: PathBuf, cx: &mut Context<Self>) {
        self.pending_data_move = Some(target);
        self.ask(Confirm::MoveData, cx);
    }

    pub fn dismiss_confirm(&mut self, cx: &mut Context<Self>) {
        self.confirm = None;
        self.pending_data_move = None;
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
            Confirm::DeleteAll => self.delete_all_data(cx),
            Confirm::MoveData => self.move_data(cx),
        }

        cx.notify();
    }

    /// Starts a copy of this program that moves the data while no Sonear runs,
    /// then quits so it can.
    fn move_data(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.pending_data_move.take() else {
            return;
        };

        if self.is_demo() {
            self.show_toast("Moving data isn't available in demo mode.", cx);

            return;
        }

        let started = std::env::current_exe().and_then(|exe| {
            std::process::Command::new(exe)
                .arg("--after")
                .arg(std::process::id().to_string())
                .arg("--move-data")
                .arg(&target)
                .spawn()
        });

        match started {
            Ok(child) => platform_window::quit_after_handing_focus_to(child.id(), cx),
            Err(error) => {
                warn!(%error, "could not start the data move");
                self.show_toast(format!("Could not start moving your data: {error}"), cx);
            }
        }
    }

    fn disconnect(&mut self, cx: &mut Context<Self>) {
        let account = self.core.account.clone();
        let (done, mut inbox) = mpsc::unbounded_channel();

        self.route = Route::Home;
        self.back_stack.clear();
        self.runtime.spawn(async move {
            let problems = account.disconnect().await;

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

    fn delete_all_data(&mut self, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let previous = keymap::effective(&core.config.get().shortcuts);
        let (done, mut inbox) = mpsc::unbounded_channel();

        self.route = Route::Home;
        self.back_stack.clear();
        self.runtime.spawn(async move {
            let problems = core.delete_all_data().await;

            let _ = done.send(problems);
        });

        cx.spawn(async move |this, cx| {
            let Some(mut problems) = inbox.recv().await else {
                return;
            };

            let _ = this.update(cx, |store, cx| {
                let config = store.core.config.get();

                if !store.is_demo()
                    && let Err(error) = crate::protocol::unregister()
                {
                    problems.push(format!("Spotify link registration: {error}"));
                }

                if !store.is_demo()
                    && let Err(error) = crate::app_identity::remove()
                {
                    problems.push(format!("Start Menu shortcut: {error}"));
                }

                store.sidebar_collapsed = config.sidebar_collapsed;
                store.right_panel_open = config.right_panel.open;
                store.right_panel_lyrics = config.right_panel.lyrics;
                store.lyrics_enabled = config.lyrics_enabled;
                store.cover_accent = config.cover_accent;
                store.close_to_tray = config.close_to_tray;
                store.sync_accent(cx);
                store.ai_hint_dismissed = config.ui.ai_hint_dismissed;
                store.editing_key = false;
                store.key_error = None;
                keymap::apply(cx, &previous, &keymap::effective(&config.shortcuts));

                let text = if problems.is_empty() {
                    "Everything Sonear stored was deleted.".to_owned()
                } else {
                    format!(
                        "Deleted, but some data could not be removed: {}",
                        problems.join("; ")
                    )
                };

                store.show_toast(text, cx);
            });
        })
        .detach();
    }

    /// Stores the retention, drops plays older than it, then reloads the history views.
    pub fn set_history_retention(&mut self, months: u32, cx: &mut Context<Self>) {
        self.core
            .config
            .update(|config| config.history_retention_months = months);

        let history = self.core.history.clone();
        let (done, mut inbox) = mpsc::unbounded_channel();

        self.runtime.spawn(async move {
            let _ = history.prune_old().await;
            let _ = done.send(());
        });
        cx.spawn(async move |this, cx| {
            if inbox.recv().await.is_some() {
                let _ = this.update(cx, |store, cx| {
                    store.history_version += 1;
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }

    /// Saves the name Spotify apps show for this computer and, outside demo
    /// mode, restarts Soloist so it takes effect.
    pub fn save_device_name(&mut self, name: &str, cx: &mut Context<Self>) {
        let name: String = name.trim().chars().take(DEVICE_NAME_MAX).collect();

        if name.is_empty() {
            return;
        }

        self.core
            .config
            .update(|config| config.device_name.clone_from(&name));

        let Some(manager) = self.core.soloist.clone() else {
            self.show_toast("Device name saved.", cx);

            return;
        };
        let (done, mut inbox) = mpsc::unbounded_channel();

        self.runtime.spawn(async move {
            manager.restart().await;

            let _ = done.send(());
        });
        cx.spawn(async move |this, cx| {
            if inbox.recv().await.is_some() {
                let _ = this.update(cx, |store, cx| {
                    store.show_toast(format!("Soloist restarted as \u{201c}{name}\u{201d}."), cx);
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

async fn cover_bytes(url: &str) -> Option<Vec<u8>> {
    #[cfg(feature = "demo")]
    if let Ok(Some(bytes)) = gpui_kit::AssetSource::load(&crate::assets::AppAssets, url) {
        return Some(bytes.into_owned());
    }

    crate::http::fetch_cover(url).await
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
