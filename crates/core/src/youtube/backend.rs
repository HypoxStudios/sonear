//! The YouTube player: an actor that owns the queue rules and the mpv
//! engine and publishes the same Soloist-shaped events the other backends
//! do, so the playback model, the player handle and the UI stay as they are.
//!
//! Stream URLs carry session tokens and are never logged; video ids are.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rust_i18n::t;
use sonear_soloist::client::ClientEvent;
use sonear_soloist::protocol::{
    ActionInfo, Event, Options, PlayStatus, PlaybackSnapshot, Position,
};
use sonear_youtube::ToolError;
use sonear_youtube::mpv::{EndReason, MpvEvent};
use tokio::runtime::Handle;
use tokio::sync::{broadcast, mpsc};
use tokio::time::Instant;
use tracing::{debug, warn};

use super::catalog::YoutubeCatalog;
use super::convert::track_entity;
use super::deck::{ContextInfo, Deck, Previous};
use super::engine::{AudioEngine, EngineLauncher, StreamSource};
use crate::config::ConfigHandle;
use crate::events::AppEvent;
use crate::model::{Track, split_uri};
use crate::playback::now_ms;
use crate::player::{PlayerBackend, PlayerError};

/// How many upcoming tracks the queue event lists.
const UPCOMING: usize = 50;
/// Tracks that may fail in a row before the player stops skipping.
const MAX_FAILURES: u32 = 3;

enum Cmd {
    Play(Option<String>),
    PlayList {
        tracks: Vec<Track>,
        start: usize,
        context: Option<ContextInfo>,
    },
    Pause,
    Next,
    Previous,
    Seek(u64),
    Volume(u8),
    Shuffle(bool),
    RepeatContext(bool),
    RepeatTrack(bool),
    AddToQueue(String),
    Enqueue(Track),
    Refresh,
    Prepared {
        generation: u64,
        engine: Option<(u64, Arc<dyn AudioEngine>)>,
        url: Result<String, ToolError>,
    },
    LoadFailed {
        generation: u64,
        error: ToolError,
    },
    Position {
        generation: u64,
        position: Option<u64>,
    },
    Engine {
        id: u64,
        event: MpvEvent,
    },
}

enum EngineCmd {
    Load { generation: u64, url: String },
    SetPause(bool),
    Seek(u64),
    Volume(u8),
    Stop,
    Query { generation: u64 },
}

pub struct YoutubeBackend {
    commands: mpsc::UnboundedSender<Cmd>,
    events: broadcast::Sender<ClientEvent>,
}

impl YoutubeBackend {
    pub fn spawn(
        runtime: &Handle,
        launcher: Arc<dyn EngineLauncher>,
        streams: Arc<dyn StreamSource>,
        catalog: Arc<YoutubeCatalog>,
        config: ConfigHandle,
        app_events: broadcast::Sender<AppEvent>,
    ) -> Arc<Self> {
        let (commands, receiver) = mpsc::unbounded_channel();
        let (events, _) = broadcast::channel(256);
        let settings = config.get();
        let actor = Actor {
            deck: Deck::new(),
            catalog,
            streams,
            launcher,
            slot: Arc::new(EngineSlot::default()),
            app_events,
            out: events.clone(),
            tx: commands.clone(),
            engine: None,
            status: PlayStatus::Paused,
            want_playing: false,
            generation: 0,
            loading: false,
            loaded: false,
            file_loaded: false,
            pending_seek: None,
            base_ms: 0,
            anchor: Instant::now(),
            volume: settings.last_volume,
            duration_ms: None,
            retry_used: false,
            failures: 0,
            device_name: settings.device_name,
        };

        runtime.spawn(run(actor, receiver));

        Arc::new(Self { commands, events })
    }

    fn send(&self, command: Cmd) -> Result<(), PlayerError> {
        self.commands
            .send(command)
            .map_err(|_| PlayerError::NotConnected)
    }

    /// Starts `tracks` as the playing list, at `start`.
    pub fn play_list(
        &self,
        tracks: Vec<Track>,
        start: usize,
        context: Option<ContextInfo>,
    ) -> Result<(), PlayerError> {
        self.send(Cmd::PlayList {
            tracks,
            start,
            context,
        })
    }
}

impl PlayerBackend for YoutubeBackend {
    fn subscribe(&self) -> broadcast::Receiver<ClientEvent> {
        self.events.subscribe()
    }

    fn request_state(&self) -> Result<(), PlayerError> {
        self.send(Cmd::Refresh)
    }

    fn play(&self, uri: Option<&str>) -> Result<(), PlayerError> {
        self.send(Cmd::Play(uri.map(str::to_owned)))
    }

    fn pause(&self) -> Result<(), PlayerError> {
        self.send(Cmd::Pause)
    }

    fn next(&self) -> Result<(), PlayerError> {
        self.send(Cmd::Next)
    }

    fn previous(&self) -> Result<(), PlayerError> {
        self.send(Cmd::Previous)
    }

    fn seek(&self, position_ms: u64) -> Result<(), PlayerError> {
        self.send(Cmd::Seek(position_ms))
    }

    fn set_volume(&self, volume: u8) -> Result<(), PlayerError> {
        self.send(Cmd::Volume(volume.min(100)))
    }

    fn set_shuffle(&self, enabled: bool) -> Result<(), PlayerError> {
        self.send(Cmd::Shuffle(enabled))
    }

    fn set_repeat_context(&self, enabled: bool) -> Result<(), PlayerError> {
        self.send(Cmd::RepeatContext(enabled))
    }

    fn set_repeat_track(&self, enabled: bool) -> Result<(), PlayerError> {
        self.send(Cmd::RepeatTrack(enabled))
    }

    fn add_to_queue(&self, uri: &str) -> Result<(), PlayerError> {
        self.send(Cmd::AddToQueue(uri.to_owned()))
    }

    fn activate(&self) -> Result<(), PlayerError> {
        self.send(Cmd::Refresh)
    }
}

/// The one mpv process, shared by the loading tasks so a quick skip never
/// launches a second one.
#[derive(Default)]
struct EngineSlot {
    current: tokio::sync::Mutex<Option<(u64, Arc<dyn AudioEngine>)>>,
    next_id: AtomicU64,
}

impl EngineSlot {
    async fn get_or_launch(
        self: &Arc<Self>,
        launcher: &Arc<dyn EngineLauncher>,
        volume: u8,
        commands: &mpsc::UnboundedSender<Cmd>,
    ) -> Result<(u64, Arc<dyn AudioEngine>), ToolError> {
        let mut current = self.current.lock().await;

        if let Some(running) = current.as_ref() {
            return Ok(running.clone());
        }

        let engine = launcher.launch(volume).await?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let mut events = engine.events();
        let slot = self.clone();
        let commands = commands.clone();

        tokio::spawn(async move {
            loop {
                let event = match events.recv().await {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => MpvEvent::Exited,
                };
                let exited = event == MpvEvent::Exited;

                if commands.send(Cmd::Engine { id, event }).is_err() || exited {
                    slot.forget(id).await;

                    break;
                }
            }
        });

        *current = Some((id, engine.clone()));

        Ok((id, engine))
    }

    async fn forget(&self, id: u64) {
        let mut current = self.current.lock().await;

        if current.as_ref().is_some_and(|(running, _)| *running == id) {
            *current = None;
        }
    }
}

struct EngineHandle {
    id: u64,
    commands: mpsc::UnboundedSender<EngineCmd>,
}

impl EngineHandle {
    fn send(&self, command: EngineCmd) {
        let _ = self.commands.send(command);
    }
}

/// Runs engine commands in the order they were given.
fn spawn_worker(
    engine: Arc<dyn AudioEngine>,
    results: mpsc::UnboundedSender<Cmd>,
) -> mpsc::UnboundedSender<EngineCmd> {
    let (sender, mut receiver) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        while let Some(command) = receiver.recv().await {
            match command {
                EngineCmd::Load { generation, url } => {
                    if let Err(error) = engine.load(&url).await {
                        let _ = results.send(Cmd::LoadFailed { generation, error });
                    }
                }
                EngineCmd::SetPause(paused) => {
                    let _ = engine.set_pause(paused).await;
                }
                EngineCmd::Seek(position_ms) => {
                    let _ = engine.seek(position_ms).await;
                }
                EngineCmd::Volume(volume) => {
                    let _ = engine.set_volume(volume).await;
                }
                EngineCmd::Stop => {
                    let _ = engine.stop().await;
                }
                EngineCmd::Query { generation } => {
                    let position = engine.position_ms().await;
                    let _ = results.send(Cmd::Position {
                        generation,
                        position,
                    });
                }
            }
        }
    });

    sender
}

struct Actor {
    deck: Deck,
    catalog: Arc<YoutubeCatalog>,
    streams: Arc<dyn StreamSource>,
    launcher: Arc<dyn EngineLauncher>,
    slot: Arc<EngineSlot>,
    app_events: broadcast::Sender<AppEvent>,
    out: broadcast::Sender<ClientEvent>,
    tx: mpsc::UnboundedSender<Cmd>,
    engine: Option<EngineHandle>,
    status: PlayStatus,
    /// What the user last asked for, which a track that is still loading
    /// honours once it can.
    want_playing: bool,
    /// Bumped whenever the track to play changes, so late results are dropped.
    generation: u64,
    /// From starting a track until mpv reports it playing.
    loading: bool,
    /// The engine was told to load the current track.
    loaded: bool,
    file_loaded: bool,
    pending_seek: Option<u64>,
    base_ms: u64,
    anchor: Instant,
    volume: u8,
    /// The length mpv reports, for tracks that did not come with one.
    duration_ms: Option<u64>,
    retry_used: bool,
    failures: u32,
    device_name: String,
}

fn video_id(track: &Track) -> Option<String> {
    track.id.clone().or_else(|| {
        split_uri(&track.uri)
            .filter(|(kind, _)| *kind == "track")
            .map(|(_, id)| id.to_owned())
    })
}

impl Actor {
    fn notice(&self, text: String) {
        let _ = self.app_events.send(AppEvent::Notice(text));
    }

    fn set_position(&mut self, position_ms: u64) {
        self.base_ms = position_ms;
        self.anchor = Instant::now();
    }

    fn track_length(&self) -> Option<u64> {
        self.deck
            .current()
            .map(|track| track.duration_ms)
            .filter(|duration| *duration > 0)
            .or(self.duration_ms)
    }

    fn position_ms(&self) -> u64 {
        let elapsed = if self.status == PlayStatus::Playing {
            self.anchor.elapsed().as_millis() as u64
        } else {
            0
        };
        let position = self.base_ms + elapsed;

        self.track_length()
            .map_or(position, |length| position.min(length))
    }

    fn snapshot(&self) -> PlaybackSnapshot {
        let playing = self.status == PlayStatus::Playing;
        let mut actions = BTreeMap::new();

        for name in [
            if playing { "pause" } else { "play" },
            "seek",
            "skip_next",
            "skip_prev",
            "shuffle",
            "set_repeat",
            "add_to_queue",
        ] {
            actions.insert(name.to_owned(), ActionInfo::default());
        }

        let current = self.deck.current();

        PlaybackSnapshot {
            status: if current.is_some() {
                self.status
            } else {
                PlayStatus::Stopped
            },
            item: current.map(|track| track_entity(track, self.duration_ms)),
            context: self.deck.context().map(ContextInfo::entity),
            position: current.map(|_| Position {
                position_ms: self.position_ms(),
                timestamp_ms: now_ms(),
                speed: if playing { 1.0 } else { 0.0 },
            }),
            volume: Some(self.volume),
            is_active: true,
            options: Some(Options {
                shuffle: self.deck.shuffle(),
                repeat: self.deck.repeat(),
                playback_speed: 1.0,
            }),
            available_actions: actions,
        }
    }

    fn emit(&self) {
        let _ = self
            .out
            .send(ClientEvent::Event(Event::PlaybackState(Box::new(
                self.snapshot(),
            ))));
        let _ = self.out.send(ClientEvent::Event(Event::QueueChanged {
            previous: Vec::new(),
            upcoming: self.deck.upcoming(UPCOMING),
        }));
    }

    fn engine_send(&self, command: EngineCmd) {
        if let Some(engine) = &self.engine {
            engine.send(command);
        }
    }

    fn install_engine(&mut self, id: u64, engine: Arc<dyn AudioEngine>) {
        if self.engine.as_ref().is_some_and(|known| known.id >= id) {
            return;
        }

        self.engine = Some(EngineHandle {
            id,
            commands: spawn_worker(engine, self.tx.clone()),
        });
    }

    /// Shows the current track at once and loads it in the background.
    fn start_current(&mut self, seek: Option<u64>, retry: bool) {
        let Some(track) = self.deck.current().cloned() else {
            return;
        };
        let Some(id) = video_id(&track) else {
            return;
        };

        if !retry {
            self.retry_used = false;
        }

        self.generation += 1;
        self.loading = true;
        self.loaded = false;
        self.file_loaded = false;
        self.duration_ms = None;
        self.pending_seek = seek.filter(|position| *position > 0);
        self.set_position(self.pending_seek.unwrap_or(0));
        self.status = if self.want_playing {
            PlayStatus::Buffering
        } else {
            PlayStatus::Paused
        };

        let generation = self.generation;
        let slot = self.slot.clone();
        let launcher = self.launcher.clone();
        let streams = self.streams.clone();
        let commands = self.tx.clone();
        let volume = self.volume;

        debug!(video = %id, generation, "loading a track");

        tokio::spawn(async move {
            let (engine, url) = tokio::join!(
                slot.get_or_launch(&launcher, volume, &commands),
                streams.resolve(&id)
            );
            let (engine, url) = match engine {
                Ok(engine) => (Some(engine), url),
                Err(error) => (None, Err(error)),
            };

            let _ = commands.send(Cmd::Prepared {
                generation,
                engine,
                url,
            });
        });
    }

    fn stop_playback(&mut self) {
        self.engine_send(EngineCmd::Stop);
        self.generation += 1;
        self.loading = false;
        self.loaded = false;
        self.file_loaded = false;
        self.want_playing = false;
        self.failures = 0;
        self.set_position(0);
        self.status = PlayStatus::Paused;
    }

    fn pause_in_place(&mut self) {
        let position = self.position_ms();

        self.want_playing = false;
        self.set_position(position);
        self.status = PlayStatus::Paused;
        self.engine_send(EngineCmd::Stop);
        self.generation += 1;
        self.loading = false;
        self.loaded = false;
        self.file_loaded = false;
        self.failures = 0;
    }

    fn fail_track(&mut self, error: ToolError) {
        let Some(id) = self.deck.current().and_then(video_id) else {
            return;
        };

        self.streams.forget(&id);

        if error == ToolError::NotInstalled {
            self.pause_in_place();
            self.notice(error.user_message());

            return;
        }

        if !self.retry_used {
            warn!(video = %id, %error, "a track failed to load, trying once more");
            self.retry_used = true;
            self.start_current(None, true);

            return;
        }

        warn!(video = %id, %error, "a track failed to load again");
        self.notice(error.user_message());
        self.failures += 1;

        if self.failures >= MAX_FAILURES {
            self.pause_in_place();

            return;
        }

        if self.deck.advance() {
            self.start_current(None, false);
        } else {
            self.stop_playback();
        }
    }

    fn on_track_end(&mut self) {
        if self.deck.advance() {
            self.start_current(None, false);
        } else {
            self.stop_playback();
        }
    }

    fn seek_to(&mut self, position_ms: u64) {
        let position = self
            .track_length()
            .map_or(position_ms, |length| position_ms.min(length));

        if self.file_loaded {
            self.engine_send(EngineCmd::Seek(position));
        } else {
            self.pending_seek = Some(position).filter(|position| *position > 0);
        }

        self.set_position(position);
    }

    fn play(&mut self) {
        self.want_playing = true;

        if self.deck.is_empty() {
            return;
        }

        if self.loading {
            if self.loaded {
                self.engine_send(EngineCmd::SetPause(false));
            }

            self.status = PlayStatus::Buffering;
        } else if self.loaded {
            if self.status != PlayStatus::Playing {
                let position = self.position_ms();

                self.set_position(position);
                self.engine_send(EngineCmd::SetPause(false));
                self.status = PlayStatus::Playing;
            }
        } else {
            let position = self.position_ms();

            self.start_current(Some(position), false);
        }
    }

    fn pause(&mut self) {
        self.want_playing = false;

        if self.deck.is_empty() {
            return;
        }

        let position = self.position_ms();

        self.set_position(position);
        self.status = PlayStatus::Paused;

        if self.loaded {
            self.engine_send(EngineCmd::SetPause(true));
        }
    }

    fn record_play(&self) {
        if let Some(track) = self.deck.current() {
            self.catalog.local().record_play(
                track.clone(),
                self.deck.context().map(|context| context.uri.clone()),
                now_ms(),
            );
        }
    }

    fn prefetch_next(&self) {
        if let Some(id) = self.deck.peek_next().and_then(video_id) {
            self.streams.prefetch(&id);
        }
    }

    /// What plays next just changed, so its stream is looked up now: a queued
    /// track arrives only seconds before it is due.
    fn prefetch_changed_next(&self) {
        if self.loaded {
            self.prefetch_next();
        }
    }

    fn handle_engine_event(&mut self, event: MpvEvent) {
        match event {
            MpvEvent::FileLoaded => {
                if self.loaded {
                    self.file_loaded = true;

                    if let Some(position) = self.pending_seek.take() {
                        self.engine_send(EngineCmd::Seek(position));
                    }
                }
            }
            MpvEvent::PlaybackRestart => {
                if !self.loaded {
                    return;
                }

                if self.loading {
                    self.loading = false;
                    self.failures = 0;
                    self.retry_used = false;
                    self.status = if self.want_playing {
                        PlayStatus::Playing
                    } else {
                        PlayStatus::Paused
                    };
                    self.set_position(self.base_ms);
                    self.record_play();
                    self.prefetch_next();
                }

                self.engine_send(EngineCmd::Query {
                    generation: self.generation,
                });
            }
            MpvEvent::EndFile { reason, error } => match reason {
                EndReason::Eof => {
                    if self.loaded && !self.loading {
                        self.on_track_end();
                    }
                }
                EndReason::Error => {
                    if self.loaded {
                        self.fail_track(ToolError::Mpv(error.unwrap_or_default()));
                    }
                }
                EndReason::Stop | EndReason::Other => {}
            },
            MpvEvent::Pause(paused) => {
                if !self.loaded || self.loading {
                    return;
                }

                if paused {
                    let position = self.position_ms();

                    self.set_position(position);
                    self.status = PlayStatus::Paused;
                } else if self.want_playing {
                    let position = self.position_ms();

                    self.set_position(position);
                    self.status = PlayStatus::Playing;
                }

                self.engine_send(EngineCmd::Query {
                    generation: self.generation,
                });
            }
            MpvEvent::Buffering(buffering) => {
                if !self.loaded || self.loading {
                    return;
                }

                if buffering && self.status == PlayStatus::Playing {
                    let position = self.position_ms();

                    self.set_position(position);
                    self.status = PlayStatus::Buffering;
                } else if !buffering && self.status == PlayStatus::Buffering && self.want_playing {
                    let position = self.position_ms();

                    self.set_position(position);
                    self.status = PlayStatus::Playing;
                }
            }
            MpvEvent::Duration(seconds) => {
                self.duration_ms = seconds
                    .filter(|seconds| *seconds > 0.0)
                    .map(|seconds| (seconds * 1000.0) as u64);
            }
            MpvEvent::Exited => {
                let position = self.position_ms();

                self.engine = None;
                self.generation += 1;
                self.loading = false;
                self.loaded = false;
                self.file_loaded = false;
                self.want_playing = false;
                self.set_position(position);
                self.status = PlayStatus::Paused;
            }
        }
    }

    fn handle(&mut self, command: Cmd) {
        match command {
            Cmd::Play(None) => self.play(),
            Cmd::Play(Some(uri)) => self.spawn_play_uri(uri),
            Cmd::PlayList {
                tracks,
                start,
                context,
            } => {
                if self.deck.load(tracks, start, context) {
                    self.want_playing = true;
                    self.start_current(None, false);
                }
            }
            Cmd::Pause => self.pause(),
            Cmd::Next => {
                if self.deck.next() {
                    self.start_current(None, false);
                }
            }
            Cmd::Previous => match self.deck.previous(self.position_ms()) {
                Previous::Restart => self.seek_to(0),
                Previous::Track => self.start_current(None, false),
            },
            Cmd::Seek(position_ms) => self.seek_to(position_ms),
            Cmd::Volume(volume) => {
                self.volume = volume;
                self.engine_send(EngineCmd::Volume(volume));
            }
            Cmd::Shuffle(enabled) => {
                self.deck.set_shuffle(enabled);
                self.prefetch_changed_next();
            }
            Cmd::RepeatContext(enabled) => {
                self.deck.set_repeat_context(enabled);
                self.prefetch_changed_next();
            }
            Cmd::RepeatTrack(enabled) => {
                self.deck.set_repeat_track(enabled);
                self.prefetch_changed_next();
            }
            Cmd::AddToQueue(uri) => self.spawn_enqueue(uri),
            Cmd::Enqueue(track) => {
                self.deck.enqueue(track);
                self.prefetch_changed_next();
            }
            Cmd::Refresh => {}
            Cmd::Prepared {
                generation,
                engine,
                url,
            } => {
                if let Some((id, engine)) = engine {
                    self.install_engine(id, engine);
                }

                if generation != self.generation {
                    return;
                }

                match url {
                    Ok(url) => {
                        self.loaded = true;
                        self.engine_send(EngineCmd::Load { generation, url });
                        self.engine_send(EngineCmd::SetPause(!self.want_playing));
                    }
                    Err(error) => self.fail_track(error),
                }
            }
            Cmd::LoadFailed { generation, error } => {
                if generation == self.generation {
                    self.fail_track(error);
                }
            }
            Cmd::Position {
                generation,
                position,
            } => {
                if generation == self.generation
                    && self.loaded
                    && !self.loading
                    && let Some(position) = position
                {
                    self.set_position(position);
                }
            }
            Cmd::Engine { id, event } => {
                if self.engine.as_ref().is_some_and(|engine| engine.id == id) {
                    self.handle_engine_event(event);
                }
            }
        }
    }

    fn spawn_play_uri(&self, uri: String) {
        let catalog = self.catalog.clone();
        let commands = self.tx.clone();
        let events = self.app_events.clone();

        tokio::spawn(async move {
            match catalog.load_context(&uri).await {
                Ok(loaded) => {
                    let _ = commands.send(Cmd::PlayList {
                        tracks: loaded.tracks,
                        start: 0,
                        context: loaded.context,
                    });
                }
                Err(error) => {
                    let _ = events.send(AppEvent::Notice(
                        t!("player.start_failed", error = error.user_message()).into_owned(),
                    ));
                }
            }
        });
    }

    fn spawn_enqueue(&self, uri: String) {
        let catalog = self.catalog.clone();
        let commands = self.tx.clone();

        tokio::spawn(async move {
            if !uri.starts_with("ytm:track:") {
                return;
            }

            match catalog.track(&uri).await {
                Ok(track) => {
                    let _ = commands.send(Cmd::Enqueue(track));
                }
                Err(error) => debug!(%error, "could not queue a track"),
            }
        });
    }
}

async fn run(mut actor: Actor, mut commands: mpsc::UnboundedReceiver<Cmd>) {
    let _ = actor.out.send(ClientEvent::Event(Event::AuthState {
        logged_in: true,
        is_active: true,
        device_name: actor.device_name.clone(),
    }));
    actor.emit();

    while let Some(command) = commands.recv().await {
        actor.handle(command);
        actor.emit();
    }
}

#[cfg(test)]
mod tests;
