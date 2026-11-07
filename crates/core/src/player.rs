use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use rust_i18n::t;
use sonear_soloist::client::ClientEvent;
use sonear_soloist::protocol::RepeatMode;
use thiserror::Error;
use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tracing::{debug, warn};

use crate::config::ConfigHandle;
use crate::events::AppEvent;
use crate::library::{LibraryBackend, LibraryError};
use crate::model::{MAX_PLAY_URIS, PlayOffset, PlaybackRequest, WebPlayRequest, liked_context_uri};
use crate::playback::PlaybackModel;

/// Time between the steps of a volume ramp.
pub const VOLUME_TICK: Duration = Duration::from_millis(40);
/// Most percentage points one ramp step moves.
pub const VOLUME_MAX_STEP: u8 = 8;
/// How long an `activate` gets to make Soloist show up in the device list.
pub const ACTIVATE_WAIT: Duration = Duration::from_millis(1500);

const LIKED_UNKNOWN: u8 = 0;
const LIKED_YES: u8 = 1;
const LIKED_NO: u8 = 2;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PlayerError {
    #[error("Soloist is not connected")]
    NotConnected,
}

impl PlayerError {
    /// The error in the interface language. `Display` stays English for logs.
    pub fn user_message(&self) -> String {
        match self {
            Self::NotConnected => t!("player.not_connected"),
        }
        .into_owned()
    }
}

/// Transport operations. Implementations are fire-and-forget: the result of
/// a command only shows up as an event on `subscribe`.
pub trait PlayerBackend: Send + Sync + 'static {
    fn subscribe(&self) -> broadcast::Receiver<ClientEvent>;

    /// Asks for a fresh state snapshot as events.
    fn request_state(&self) -> Result<(), PlayerError>;

    fn play(&self, uri: Option<&str>) -> Result<(), PlayerError>;

    fn pause(&self) -> Result<(), PlayerError>;

    fn next(&self) -> Result<(), PlayerError>;

    fn previous(&self) -> Result<(), PlayerError>;

    fn seek(&self, position_ms: u64) -> Result<(), PlayerError>;

    fn set_volume(&self, volume: u8) -> Result<(), PlayerError>;

    fn set_shuffle(&self, enabled: bool) -> Result<(), PlayerError>;

    fn set_repeat_context(&self, enabled: bool) -> Result<(), PlayerError>;

    fn set_repeat_track(&self, enabled: bool) -> Result<(), PlayerError>;

    fn add_to_queue(&self, uri: &str) -> Result<(), PlayerError>;

    fn activate(&self) -> Result<(), PlayerError>;
}

#[derive(Default)]
struct VolumeState {
    /// Where the running ramp is heading; `None` while no ramp runs.
    target: Option<u8>,
}

struct Inner {
    backend: Arc<dyn PlayerBackend>,
    state: Mutex<PlaybackModel>,
    runtime: Handle,
    config: ConfigHandle,
    volume: Mutex<VolumeState>,
    events: broadcast::Sender<AppEvent>,
    web: OnceLock<Arc<dyn LibraryBackend>>,
    device: Mutex<Option<String>>,
    /// Whether Spotify accepts the saved-tracks context. Lives as long as the
    /// process, so it isn't reset when the account changes.
    liked_support: AtomicU8,
}

/// Cloneable handle the UI uses to control playback and read the model.
#[derive(Clone)]
pub struct Player {
    inner: Arc<Inner>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Player {
    /// Spawns the task that folds backend events into the model and
    /// publishes each change as `AppEvent::Playback`.
    pub fn new(
        backend: Arc<dyn PlayerBackend>,
        runtime: Handle,
        config: ConfigHandle,
        events: broadcast::Sender<AppEvent>,
    ) -> Self {
        let initial = PlaybackModel::with_volume(config.get().last_volume);
        let inner = Arc::new(Inner {
            backend,
            state: Mutex::new(initial),
            runtime,
            config,
            volume: Mutex::new(VolumeState::default()),
            events: events.clone(),
            web: OnceLock::new(),
            device: Mutex::new(None),
            liked_support: AtomicU8::new(LIKED_UNKNOWN),
        });
        let mut feed = inner.backend.subscribe();
        let pump = inner.clone();

        inner.runtime.spawn(async move {
            loop {
                let event = match feed.recv().await {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(missed)) => {
                        warn!(missed, "playback events lagged");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                let snapshot = {
                    let mut state = lock(&pump.state);
                    let context_before = state.context.as_ref().map(|context| context.uri.clone());

                    let changed = match &event {
                        ClientEvent::Event(event) => state.apply(event),
                        ClientEvent::Disconnected => {
                            let reset = PlaybackModel::with_volume(state.volume);
                            let changed = *state != reset;

                            *state = reset;

                            changed
                        }
                    };
                    let context_after = state.context.as_ref().map(|context| context.uri.clone());

                    if context_after != context_before {
                        debug!(
                            context = context_after.as_deref().unwrap_or("none"),
                            upcoming = state.upcoming.len(),
                            "playback context changed"
                        );
                    }

                    changed.then(|| state.clone())
                };

                if let Some(snapshot) = snapshot {
                    let _ = events.send(AppEvent::Playback(Box::new(snapshot)));
                }
            }
        });

        let _ = inner.backend.request_state();

        Self { inner }
    }

    /// Gives the player the Web API, which "play at row N" needs. Without it
    /// every request falls back to the WebSocket.
    pub fn attach_library(&self, library: Arc<dyn LibraryBackend>) {
        let _ = self.inner.web.set(library);
    }

    pub fn snapshot(&self) -> PlaybackModel {
        lock(&self.inner.state).clone()
    }

    fn run(&self, name: &str, result: Result<(), PlayerError>) {
        if let Err(error) = result {
            debug!(%error, command = name, "command not sent");
        }
    }

    pub fn play(&self, uri: Option<&str>) {
        self.run("play", self.inner.backend.play(uri));
    }

    /// Starts a whole context over the WebSocket.
    pub fn play_context(&self, uri: &str) {
        self.play(Some(uri));
    }

    /// Starts `context_uri` at a row. Goes through the Web API to the Soloist device.
    pub fn play_in_context(&self, context_uri: &str, offset: PlayOffset) {
        self.play_request(PlaybackRequest::InContext {
            context_uri: context_uri.to_owned(),
            offset,
        });
    }

    /// Plays tracks from `start` onward. At most `MAX_PLAY_URIS` are sent.
    pub fn play_tracks(&self, uris: Vec<String>, start: usize) {
        self.play_request(PlaybackRequest::Tracks { uris, start });
    }

    pub fn play_request(&self, request: PlaybackRequest) {
        let inner = self.inner.clone();

        self.inner.runtime.spawn(route(inner, request));
    }

    pub fn pause(&self) {
        self.run("pause", self.inner.backend.pause());
    }

    pub fn toggle_play_pause(&self) {
        if self.snapshot().is_playing() {
            self.pause();
        } else {
            self.play(None);
        }
    }

    pub fn next(&self) {
        self.run("skip_next", self.inner.backend.next());
    }

    pub fn previous(&self) {
        self.run("skip_prev", self.inner.backend.previous());
    }

    pub fn seek(&self, position_ms: u64) {
        self.run("seek", self.inner.backend.seek(position_ms));
    }

    pub fn add_to_queue(&self, uri: &str) {
        self.run("add_to_queue", self.inner.backend.add_to_queue(uri));
    }

    pub fn toggle_shuffle(&self) {
        let enabled = !self.snapshot().options.shuffle;

        self.run("set_shuffle", self.inner.backend.set_shuffle(enabled));
    }

    /// Off, then repeat context, then repeat track, then off again. Soloist
    /// has two independent switches, so each step sends two commands in an
    /// order that never leaves both on.
    pub fn cycle_repeat(&self) {
        let next = match self.snapshot().repeat() {
            RepeatMode::Off | RepeatMode::Unknown => RepeatMode::Context,
            RepeatMode::Context => RepeatMode::Track,
            RepeatMode::Track => RepeatMode::Off,
        };

        self.set_repeat(next);
    }

    pub fn set_repeat(&self, mode: RepeatMode) {
        let backend = &self.inner.backend;
        let result = match mode {
            RepeatMode::Off | RepeatMode::Unknown => backend
                .set_repeat_track(false)
                .and_then(|()| backend.set_repeat_context(false)),
            RepeatMode::Context => backend
                .set_repeat_track(false)
                .and_then(|()| backend.set_repeat_context(true)),
            RepeatMode::Track => backend
                .set_repeat_context(false)
                .and_then(|()| backend.set_repeat_track(true)),
        };

        self.run("set_repeat", result);
    }

    /// Ramps the volume to `volume` in steps of at most `VOLUME_MAX_STEP`, and
    /// remembers it as `last_volume` once the ramp ends. A new call redirects
    /// a running ramp from where it is.
    pub fn set_volume(&self, volume: u8) {
        let volume = volume.min(100);
        let start = {
            let mut state = lock(&self.inner.volume);

            state.target.replace(volume).is_none()
        };

        if start {
            self.inner.runtime.spawn(volume_task(self.inner.clone()));
        }
    }

    /// Where the volume is heading: the target of a running ramp, else the model's volume.
    pub fn volume_target(&self) -> u8 {
        lock(&self.inner.volume)
            .target
            .unwrap_or_else(|| self.snapshot().volume)
    }
}

fn web_request(request: &PlaybackRequest) -> Option<WebPlayRequest> {
    match request {
        PlaybackRequest::Context { uri } => Some(WebPlayRequest {
            context_uri: Some(uri.clone()),
            ..WebPlayRequest::default()
        }),
        PlaybackRequest::InContext {
            context_uri,
            offset,
        } => Some(WebPlayRequest {
            context_uri: Some(context_uri.clone()),
            uris: None,
            offset: Some(offset.clone()),
        }),
        PlaybackRequest::Tracks { uris, start } => {
            let slice: Vec<String> = uris
                .iter()
                .skip(*start)
                .take(MAX_PLAY_URIS)
                .cloned()
                .collect();

            (!slice.is_empty()).then_some(WebPlayRequest {
                context_uri: None,
                uris: Some(slice),
                offset: None,
            })
        }
        PlaybackRequest::LikedSongs {
            user_id,
            offset_uri,
            ..
        } => Some(WebPlayRequest {
            context_uri: Some(liked_context_uri(user_id)),
            uris: None,
            offset: offset_uri.clone().map(PlayOffset::Uri),
        }),
    }
}

/// What the WebSocket can do when the Web API route is unavailable: start one URI.
fn fallback_uri(request: &PlaybackRequest) -> Option<String> {
    match request {
        PlaybackRequest::Context { uri } => Some(uri.clone()),
        PlaybackRequest::InContext {
            context_uri,
            offset,
        } => match offset {
            PlayOffset::Uri(uri) => Some(uri.clone()),
            PlayOffset::Position(_) => Some(context_uri.clone()),
        },
        PlaybackRequest::Tracks { uris, start } => uris.get(*start).cloned(),
        PlaybackRequest::LikedSongs {
            user_id,
            uris,
            start,
            ..
        } => uris
            .get(*start)
            .cloned()
            .or_else(|| Some(liked_context_uri(user_id))),
    }
}

fn start_failed(error: &LibraryError) -> String {
    t!("player.start_failed", error = error.user_message()).into_owned()
}

fn notice(inner: &Inner, text: impl Into<String>) {
    let _ = inner.events.send(AppEvent::Notice(text.into()));
}

async fn find_device(
    inner: &Inner,
    web: &dyn LibraryBackend,
) -> Result<Option<String>, LibraryError> {
    let name = inner.config.get().device_name;
    let found = web
        .devices()
        .await?
        .into_iter()
        .find(|device| device.name.eq_ignore_ascii_case(&name))
        .map(|device| device.id);

    *lock(&inner.device) = found.clone();

    Ok(found)
}

enum Attempt {
    Started,
    NoDevice,
    Failed(LibraryError),
}

/// With `probe` set, an error other than a 404 from a cached device id means
/// Spotify rejected the request itself, so the device lookup isn't retried.
async fn attempt(
    inner: &Inner,
    web: &dyn LibraryBackend,
    request: &WebPlayRequest,
    probe: bool,
) -> Attempt {
    let mut activated = false;
    let mut retried = false;

    loop {
        let cached = lock(&inner.device).clone();
        let fresh = cached.is_none();
        let id = match cached {
            Some(id) => id,
            None => match find_device(inner, web).await {
                Ok(Some(id)) => id,
                Ok(None) if !activated => {
                    activated = true;
                    let _ = inner.backend.activate();
                    tokio::time::sleep(ACTIVATE_WAIT).await;

                    continue;
                }
                Ok(None) => return Attempt::NoDevice,
                Err(error) => return Attempt::Failed(error),
            },
        };

        match web.start_playback(request, &id).await {
            Ok(()) => return Attempt::Started,
            Err(
                error @ (LibraryError::NeedsLogin
                | LibraryError::RateLimited
                | LibraryError::QuotaExceeded
                | LibraryError::Network(_)),
            ) => return Attempt::Failed(error),
            Err(error) => {
                let device_suspect = !probe || (error == LibraryError::NotFound && !fresh);

                if !device_suspect {
                    return Attempt::Failed(error);
                }

                *lock(&inner.device) = None;

                if retried {
                    return Attempt::Failed(error);
                }

                retried = true;
            }
        }
    }
}

fn socket_fallback(inner: &Inner, request: &PlaybackRequest) {
    if let Some(uri) = fallback_uri(request) {
        let _ = inner.backend.play(Some(&uri));
    }
}

async fn route(inner: Arc<Inner>, request: PlaybackRequest) {
    match request {
        PlaybackRequest::LikedSongs { .. } => route_liked(&inner, request).await,
        request => route_plain(&inner, &request).await,
    }
}

async fn route_plain(inner: &Inner, request: &PlaybackRequest) {
    let (Some(web), Some(wire)) = (inner.web.get(), web_request(request)) else {
        socket_fallback(inner, request);

        return;
    };

    match attempt(inner, web.as_ref(), &wire, false).await {
        Attempt::Started => {}
        Attempt::NoDevice => {
            socket_fallback(inner, request);
            notice(inner, t!("player.no_device"));
        }
        Attempt::Failed(error) => notice(inner, start_failed(&error)),
    }
}

/// Tries the saved-tracks context first. When Spotify rejects it, plays the
/// loaded tracks instead and skips the context from then on.
async fn route_liked(inner: &Inner, request: PlaybackRequest) {
    let (Some(web), Some(wire)) = (inner.web.get(), web_request(&request)) else {
        socket_fallback(inner, &request);

        return;
    };
    let PlaybackRequest::LikedSongs { uris, start, .. } = &request else {
        return;
    };
    let tracks = PlaybackRequest::Tracks {
        uris: uris.clone(),
        start: *start,
    };

    if inner.liked_support.load(Ordering::Relaxed) == LIKED_NO {
        route_plain(inner, &tracks).await;

        return;
    }

    match attempt(inner, web.as_ref(), &wire, true).await {
        Attempt::Started => inner.liked_support.store(LIKED_YES, Ordering::Relaxed),
        Attempt::NoDevice => {
            socket_fallback(inner, &request);
            notice(inner, t!("player.no_device"));
        }
        Attempt::Failed(
            error @ (LibraryError::Forbidden | LibraryError::NotFound | LibraryError::Api(_)),
        ) => {
            debug!(%error, "Spotify rejected the saved-tracks context");
            inner.liked_support.store(LIKED_NO, Ordering::Relaxed);
            route_plain(inner, &tracks).await;
        }
        Attempt::Failed(error) => notice(inner, start_failed(&error)),
    }
}

async fn volume_task(inner: Arc<Inner>) {
    let mut current = lock(&inner.state).volume;

    let target = loop {
        let Some(target) = lock(&inner.volume).target else {
            return;
        };

        if current != target {
            let step = target.abs_diff(current).min(VOLUME_MAX_STEP);

            current = if target > current {
                current + step
            } else {
                current - step
            };

            if let Err(error) = inner.backend.set_volume(current) {
                debug!(%error, "volume not sent");
            }
        }

        {
            let mut state = lock(&inner.volume);

            if current == target && state.target == Some(target) {
                state.target = None;

                break target;
            }
        }

        tokio::time::sleep(VOLUME_TICK).await;
    };

    inner.config.update(|config| config.last_volume = target);
}

#[cfg(test)]
mod routing_tests;

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use sonear_soloist::protocol::Event;

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Play(Option<String>),
        Pause,
        Volume(u8),
        Shuffle(bool),
        RepeatContext(bool),
        RepeatTrack(bool),
    }

    struct Recorder {
        calls: Mutex<Vec<Call>>,
        events: broadcast::Sender<ClientEvent>,
        fail_volume: AtomicBool,
    }

    impl Recorder {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
                events: broadcast::channel(16).0,
                fail_volume: AtomicBool::new(false),
            })
        }

        fn calls(&self) -> Vec<Call> {
            lock(&self.calls).clone()
        }

        fn record(&self, call: Call) -> Result<(), PlayerError> {
            lock(&self.calls).push(call);

            Ok(())
        }
    }

    impl PlayerBackend for Recorder {
        fn subscribe(&self) -> broadcast::Receiver<ClientEvent> {
            self.events.subscribe()
        }

        fn request_state(&self) -> Result<(), PlayerError> {
            Ok(())
        }

        fn play(&self, uri: Option<&str>) -> Result<(), PlayerError> {
            self.record(Call::Play(uri.map(str::to_owned)))
        }

        fn pause(&self) -> Result<(), PlayerError> {
            self.record(Call::Pause)
        }

        fn next(&self) -> Result<(), PlayerError> {
            Ok(())
        }

        fn previous(&self) -> Result<(), PlayerError> {
            Ok(())
        }

        fn seek(&self, _: u64) -> Result<(), PlayerError> {
            Ok(())
        }

        fn set_volume(&self, volume: u8) -> Result<(), PlayerError> {
            self.record(Call::Volume(volume))?;

            if self.fail_volume.load(Ordering::Relaxed) {
                return Err(PlayerError::NotConnected);
            }

            Ok(())
        }

        fn set_shuffle(&self, enabled: bool) -> Result<(), PlayerError> {
            self.record(Call::Shuffle(enabled))
        }

        fn set_repeat_context(&self, enabled: bool) -> Result<(), PlayerError> {
            self.record(Call::RepeatContext(enabled))
        }

        fn set_repeat_track(&self, enabled: bool) -> Result<(), PlayerError> {
            self.record(Call::RepeatTrack(enabled))
        }

        fn add_to_queue(&self, _: &str) -> Result<(), PlayerError> {
            Ok(())
        }

        fn activate(&self) -> Result<(), PlayerError> {
            Ok(())
        }
    }

    struct Fixture {
        recorder: Arc<Recorder>,
        player: Player,
        config: ConfigHandle,
        app_events: broadcast::Receiver<AppEvent>,
        _dir: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let config = ConfigHandle::load(dir.path().join("config.toml"));
        let recorder = Recorder::new();
        let (events, app_events) = broadcast::channel(16);
        let player = Player::new(recorder.clone(), Handle::current(), config.clone(), events);

        Fixture {
            recorder,
            player,
            config,
            app_events,
            _dir: dir,
        }
    }

    async fn feed(fx: &mut Fixture, text: &str) {
        let _ = fx
            .recorder
            .events
            .send(ClientEvent::Event(Event::parse(text).unwrap()));
        let _ = fx.app_events.recv().await;
    }

    #[tokio::test]
    async fn repeat_cycle_sends_the_two_command_sequences() {
        let mut fx = fixture();

        fx.player.cycle_repeat();

        assert_eq!(
            fx.recorder.calls(),
            [Call::RepeatTrack(false), Call::RepeatContext(true)]
        );

        feed(
            &mut fx,
            r#"{"type":"options_changed","options":{"repeat":"context"}}"#,
        )
        .await;
        lock(&fx.recorder.calls).clear();
        fx.player.cycle_repeat();

        assert_eq!(
            fx.recorder.calls(),
            [Call::RepeatContext(false), Call::RepeatTrack(true)]
        );

        feed(
            &mut fx,
            r#"{"type":"options_changed","options":{"repeat":"track"}}"#,
        )
        .await;
        lock(&fx.recorder.calls).clear();
        fx.player.cycle_repeat();

        assert_eq!(
            fx.recorder.calls(),
            [Call::RepeatTrack(false), Call::RepeatContext(false)]
        );
    }

    #[tokio::test]
    async fn shuffle_and_play_pause_follow_the_model() {
        let mut fx = fixture();

        fx.player.toggle_shuffle();
        fx.player.toggle_play_pause();
        feed(&mut fx, r#"{"type":"playback_changed","status":"playing"}"#).await;
        fx.player.toggle_play_pause();

        assert_eq!(
            fx.recorder.calls(),
            [Call::Shuffle(true), Call::Play(None), Call::Pause]
        );
    }

    async fn ticks(count: u32) {
        for _ in 0..count {
            tokio::time::advance(VOLUME_TICK).await;

            for _ in 0..3 {
                tokio::task::yield_now().await;
            }
        }
    }

    fn volumes(fx: &Fixture) -> Vec<u8> {
        fx.recorder
            .calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Volume(volume) => Some(volume),
                _ => None,
            })
            .collect()
    }

    async fn start_at(volume: u8) -> Fixture {
        let mut fx = fixture();

        feed(
            &mut fx,
            &format!(r#"{{"type":"volume_changed","volume":{volume}}}"#),
        )
        .await;

        fx
    }

    #[tokio::test(start_paused = true)]
    async fn volume_ramps_in_bounded_steps_and_ends_on_the_target() {
        let fx = start_at(20).await;

        fx.player.set_volume(80);
        tokio::task::yield_now().await;

        assert_eq!(volumes(&fx), [28]);
        assert_eq!(fx.player.volume_target(), 80);

        ticks(20).await;

        let sent = volumes(&fx);
        let mut previous = 20;

        for volume in &sent {
            assert!(*volume > previous);
            assert!(volume - previous <= VOLUME_MAX_STEP);

            previous = *volume;
        }

        assert_eq!(sent.len(), 8);
        assert_eq!(sent.last(), Some(&80));
        assert_eq!(fx.player.volume_target(), 20);
    }

    #[tokio::test(start_paused = true)]
    async fn volume_redirects_a_running_ramp_from_where_it_is() {
        let fx = start_at(20).await;

        fx.player.set_volume(80);
        tokio::task::yield_now().await;
        ticks(2).await;

        assert_eq!(volumes(&fx), [28, 36, 44]);

        fx.player.set_volume(30);
        ticks(5).await;

        assert_eq!(volumes(&fx), [28, 36, 44, 36, 30]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_small_volume_change_is_one_command() {
        let fx = start_at(20).await;

        fx.player.set_volume(23);
        tokio::task::yield_now().await;
        ticks(5).await;

        assert_eq!(volumes(&fx), [23]);
    }

    #[tokio::test(start_paused = true)]
    async fn no_volume_command_when_already_at_the_target() {
        let fx = start_at(20).await;

        fx.player.set_volume(20);
        tokio::task::yield_now().await;
        ticks(3).await;

        assert!(volumes(&fx).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn volume_above_100_is_clamped() {
        let fx = start_at(90).await;

        fx.player.set_volume(200);
        tokio::task::yield_now().await;
        ticks(5).await;

        assert_eq!(volumes(&fx), [98, 100]);
        assert_eq!(fx.config.get().last_volume, 100);
    }

    #[tokio::test(start_paused = true)]
    async fn volume_is_persisted_once_with_the_final_target() {
        let fx = start_at(20).await;
        let before = fx.config.get().last_volume;

        fx.player.set_volume(80);
        tokio::task::yield_now().await;
        ticks(3).await;

        assert_eq!(fx.config.get().last_volume, before);

        fx.player.set_volume(60);
        ticks(20).await;

        assert_eq!(fx.config.get().last_volume, 60);
        assert_eq!(volumes(&fx).last(), Some(&60));
    }

    #[tokio::test(start_paused = true)]
    async fn volume_ramp_ends_and_persists_when_sends_fail() {
        let fx = start_at(20).await;

        fx.recorder.fail_volume.store(true, Ordering::Relaxed);
        fx.player.set_volume(80);
        tokio::task::yield_now().await;
        ticks(30).await;

        assert_eq!(volumes(&fx).len(), 8);
        assert_eq!(fx.config.get().last_volume, 80);
        assert_eq!(fx.player.volume_target(), 20);
    }

    #[tokio::test]
    async fn disconnect_resets_the_model_but_keeps_volume() {
        let mut fx = fixture();

        feed(&mut fx, r#"{"type":"volume_changed","volume":55}"#).await;
        feed(&mut fx, r#"{"type":"playback_changed","status":"playing"}"#).await;

        let _ = fx.recorder.events.send(ClientEvent::Disconnected);
        let _ = fx.app_events.recv().await;

        let snapshot = fx.player.snapshot();

        assert_eq!(snapshot.volume, 55);
        assert!(!snapshot.is_playing());
    }
}
