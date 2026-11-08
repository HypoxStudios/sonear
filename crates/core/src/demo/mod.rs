//! Fake data and a simulated player, so the UI runs without Soloist, keys or network.

pub mod catalog;
pub mod library;
pub mod lyrics;

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use sonear_soloist::client::ClientEvent;
use sonear_soloist::protocol::{
    ActionInfo, Cover, Creator, Decorations, Entity, Event, Identity, Options, Parent, PlayStatus,
    PlaybackInfo, PlaybackSnapshot, Position, QueueEntry, RepeatMode, VisualIdentity,
};
use tokio::runtime::Handle;
use tokio::sync::{broadcast, mpsc};
use tokio::time::{Instant, sleep_until};

use crate::model::{PlayOffset, WebPlayRequest};
use crate::playback::now_ms;
use crate::player::{PlayerBackend, PlayerError};
use catalog::{DemoTrack, PLAYLIST_URI, catalog, cover_path};

pub use library::DemoLibrary;
pub use lyrics::DemoLyrics;

pub const DEVICE_NAME: &str = "Sonear (demo)";

fn covers(slot: usize) -> VisualIdentity {
    VisualIdentity {
        cover: ["small", "default", "large"]
            .into_iter()
            .map(|size| Cover {
                url: cover_path(slot),
                size: size.into(),
            })
            .collect(),
    }
}

fn named(uri: &str, kind: &str, name: &str, slot: Option<usize>) -> Entity {
    Entity {
        uri: uri.into(),
        entity_type: kind.into(),
        decorations: Decorations {
            identity: Identity { name: name.into() },
            visual_identity: slot.map(covers).unwrap_or_default(),
            ..Decorations::default()
        },
    }
}

fn track_entity(track: &DemoTrack) -> Entity {
    let slot = catalog::album_slot(track.album_index);

    Entity {
        uri: track.uri.clone(),
        entity_type: "track".into(),
        decorations: Decorations {
            identity: Identity {
                name: track.title.clone(),
            },
            visual_identity: covers(slot),
            creators: vec![Creator {
                entity: Some(named(
                    &track.artist_uri(),
                    "artist",
                    track.artist,
                    Some(catalog::artist_slot(track.artist_index)),
                )),
            }],
            parent: Some(Parent {
                entity: Some(Box::new(named(
                    &track.album_uri(),
                    "album",
                    &catalog().album_model(track.album_index).name,
                    Some(slot),
                ))),
            }),
            playback: Some(PlaybackInfo {
                duration_ms: Some(track.duration_ms),
            }),
        },
    }
}

fn context_entity(uri: &str) -> Entity {
    let slot = crate::model::split_uri(uri).and_then(|(kind, id)| match kind {
        "playlist" => catalog().playlist(id).map(catalog::DemoPlaylist::slot),
        "album" => id
            .strip_prefix("demo-album-")
            .and_then(|index| index.parse().ok())
            .map(catalog::album_slot),
        "artist" => id
            .strip_prefix("demo-artist-")
            .and_then(|index| index.parse().ok())
            .map(catalog::artist_slot),
        _ => None,
    });
    let name = crate::model::split_uri(uri)
        .filter(|_| !crate::model::is_liked_context(uri))
        .and_then(|(kind, id)| match kind {
            "playlist" => catalog()
                .playlist(id)
                .map(|playlist| playlist.name.to_owned()),
            "album" => id
                .strip_prefix("demo-album-")
                .and_then(|index| index.parse().ok())
                .map(|index| catalog().album_model(index).name),
            "artist" => id
                .strip_prefix("demo-artist-")
                .and_then(|index| index.parse::<usize>().ok())
                .map(|index| catalog::ARTISTS[index % catalog::ARTISTS.len()].to_owned()),
            _ => None,
        })
        .unwrap_or_else(|| {
            if crate::model::is_liked_context(uri) {
                "Liked Songs".to_owned()
            } else {
                String::new()
            }
        });

    let kind = if crate::radio::is_station(uri) {
        "unknown"
    } else {
        uri.split(':').nth(1).unwrap_or("playlist")
    };

    named(uri, kind, &name, slot)
}

#[derive(Debug)]
enum Cmd {
    Play(Option<String>),
    PlayWeb(WebPlayRequest),
    PlayCollection {
        indices: Vec<usize>,
        start: usize,
        context_uri: String,
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
    Refresh,
    Activate,
}

pub struct DemoBackend {
    commands: mpsc::UnboundedSender<Cmd>,
    events: broadcast::Sender<ClientEvent>,
}

impl DemoBackend {
    pub fn spawn(runtime: &Handle, volume: u8) -> Arc<Self> {
        let (commands, rx) = mpsc::unbounded_channel();
        let (events, _) = broadcast::channel(256);
        let sim = Sim::new(volume);

        runtime.spawn(run(sim, rx, events.clone()));

        Arc::new(Self { commands, events })
    }

    fn send(&self, command: Cmd) -> Result<(), PlayerError> {
        self.commands
            .send(command)
            .map_err(|_| PlayerError::NotConnected)
    }

    /// Starts something the way a Web API play call would.
    pub fn play_web(&self, request: WebPlayRequest) -> Result<(), PlayerError> {
        self.send(Cmd::PlayWeb(request))
    }

    /// Starts the saved tracks, given as catalogue indices, as the user's collection context.
    pub fn play_collection(
        &self,
        indices: Vec<usize>,
        start: usize,
        context_uri: String,
    ) -> Result<(), PlayerError> {
        self.send(Cmd::PlayCollection {
            indices,
            start,
            context_uri,
        })
    }
}

impl PlayerBackend for DemoBackend {
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
        self.send(Cmd::Activate)
    }
}

struct Sim {
    /// The playing context in its natural order, as indices into the catalogue.
    base: Vec<usize>,
    /// The order being played, which differs from `base` while shuffled.
    order: Vec<usize>,
    cursor: usize,
    context: Option<Entity>,
    status: PlayStatus,
    base_ms: u64,
    anchor: Instant,
    volume: u8,
    shuffle: bool,
    repeat_context: bool,
    repeat_track: bool,
    shuffle_seed: u64,
    /// Catalogue indices the user queued, played before the context resumes.
    queued: VecDeque<usize>,
    /// A queued track that is playing now, on top of the context cursor.
    interrupt: Option<usize>,
}

impl Sim {
    fn new(volume: u8) -> Self {
        let base: Vec<usize> = catalog()
            .playlist("demo-night-drive")
            .map(catalog::DemoPlaylist::indices)
            .unwrap_or_default();

        Self {
            order: base.clone(),
            base,
            cursor: 0,
            context: Some(context_entity(PLAYLIST_URI)),
            status: PlayStatus::Paused,
            base_ms: 79_000,
            anchor: Instant::now(),
            volume,
            shuffle: false,
            repeat_context: false,
            repeat_track: false,
            shuffle_seed: 0x9E37_79B9,
            queued: VecDeque::new(),
            interrupt: None,
        }
    }

    fn current(&self) -> &DemoTrack {
        &catalog().tracks[self.current_index()]
    }

    fn current_index(&self) -> usize {
        self.interrupt.unwrap_or(self.order[self.cursor])
    }

    fn position_ms(&self) -> u64 {
        let elapsed = if self.status == PlayStatus::Playing {
            self.anchor.elapsed().as_millis() as u64
        } else {
            0
        };

        (self.base_ms + elapsed).min(self.current().duration_ms)
    }

    fn set_position(&mut self, position_ms: u64) {
        self.base_ms = position_ms;
        self.anchor = Instant::now();
    }

    fn start_track(&mut self, cursor: usize) {
        self.cursor = cursor;
        self.interrupt = None;
        self.set_position(0);
    }

    /// Plays the next queued track, if any. The context cursor stays put.
    fn start_queued(&mut self) -> bool {
        let Some(index) = self.queued.pop_front() else {
            return false;
        };

        self.interrupt = Some(index);
        self.set_position(0);

        true
    }

    /// Moves on to the following context track, resuming after a queued one.
    fn advance_context(&mut self) {
        if self.cursor + 1 < self.order.len() {
            self.start_track(self.cursor + 1);
        } else if self.repeat_context {
            self.start_track(0);
        } else {
            self.status = PlayStatus::Paused;
            self.start_track(0);
        }
    }

    fn deadline(&self) -> Option<Instant> {
        if self.status != PlayStatus::Playing {
            return None;
        }

        let remaining = self.current().duration_ms.saturating_sub(self.base_ms);

        Some(self.anchor + Duration::from_millis(remaining))
    }

    fn rebuild_order(&mut self, shuffle: bool) {
        let current = self.order[self.cursor];

        self.shuffle = shuffle;

        if !shuffle {
            self.order = self.base.clone();
            self.cursor = self.base.iter().position(|i| *i == current).unwrap_or(0);

            return;
        }

        let mut rest: Vec<usize> = self
            .base
            .iter()
            .copied()
            .filter(|i| *i != current)
            .collect();

        for index in (1..rest.len()).rev() {
            self.shuffle_seed = self
                .shuffle_seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);

            rest.swap(index, (self.shuffle_seed >> 33) as usize % (index + 1));
        }

        self.order = std::iter::once(current).chain(rest).collect();
        self.cursor = 0;
    }

    fn advance(&mut self) {
        if self.repeat_track {
            self.set_position(0);
        } else if !self.start_queued() {
            self.advance_context();
        }
    }

    fn handle(&mut self, command: Cmd) {
        match command {
            Cmd::Play(None) => {
                if self.status != PlayStatus::Playing {
                    self.set_position(self.position_ms());
                    self.status = PlayStatus::Playing;
                }
            }
            Cmd::Play(Some(uri)) => self.play_uri(&uri),
            Cmd::PlayWeb(request) => self.play_web(&request),
            Cmd::PlayCollection {
                indices,
                start,
                context_uri,
            } => self.load(indices, start, Some(context_entity(&context_uri))),
            Cmd::Pause => {
                self.set_position(self.position_ms());
                self.status = PlayStatus::Paused;
            }
            Cmd::Next => {
                if !self.start_queued()
                    && (self.cursor + 1 < self.order.len() || self.repeat_context)
                {
                    self.advance_context();
                }
            }
            Cmd::AddToQueue(uri) => {
                if let Some(track) = catalog().track(&uri) {
                    self.queued.push_back(track.index);
                }
            }
            Cmd::Previous => {
                if self.position_ms() > 3_000 || self.cursor == 0 || self.interrupt.is_some() {
                    self.set_position(0);
                } else {
                    self.start_track(self.cursor - 1);
                }
            }
            Cmd::Seek(position) => self.set_position(position.min(self.current().duration_ms)),
            Cmd::Volume(volume) => self.volume = volume,
            Cmd::Shuffle(enabled) => self.rebuild_order(enabled),
            Cmd::RepeatContext(enabled) => self.repeat_context = enabled,
            Cmd::RepeatTrack(enabled) => self.repeat_track = enabled,
            Cmd::Activate | Cmd::Refresh => {}
        }
    }

    /// Starts `indices` as the context and begins at `start`.
    fn load(&mut self, indices: Vec<usize>, start: usize, context: Option<Entity>) {
        if indices.is_empty() {
            return;
        }

        self.base = indices;
        self.order = self.base.clone();
        self.cursor = start.min(self.base.len() - 1);
        self.context = context;

        if self.shuffle {
            self.rebuild_order(true);
        }

        self.status = PlayStatus::Playing;
        self.interrupt = None;
        self.set_position(0);
    }

    fn play_uri(&mut self, uri: &str) {
        let catalog = catalog();

        if let Some(indices) = catalog.context_indices(uri) {
            self.load(indices, 0, Some(context_entity(uri)));

            return;
        }

        let Some(track) = catalog.track(uri) else {
            return;
        };

        let context_uri = if track.index < catalog::NIGHT_DRIVE_TRACKS {
            PLAYLIST_URI.to_owned()
        } else {
            track.album_uri()
        };
        let indices = catalog.context_indices(&context_uri).unwrap_or_default();
        let start = indices.iter().position(|i| *i == track.index).unwrap_or(0);

        self.load(indices, start, Some(context_entity(&context_uri)));
    }

    fn play_web(&mut self, request: &WebPlayRequest) {
        let catalog = catalog();

        if let Some(context) = &request.context_uri {
            let Some(indices) = catalog.context_indices(context) else {
                return;
            };
            let start = match &request.offset {
                Some(PlayOffset::Position(position)) => *position as usize,
                Some(PlayOffset::Uri(uri)) => indices
                    .iter()
                    .position(|i| catalog.tracks[*i].uri == *uri)
                    .unwrap_or(0),
                None => 0,
            };

            self.load(indices, start, Some(context_entity(context)));

            return;
        }

        if let Some(uris) = &request.uris {
            let indices: Vec<usize> = uris
                .iter()
                .filter_map(|uri| catalog.track(uri).map(|track| track.index))
                .collect();
            let start = match &request.offset {
                Some(PlayOffset::Position(position)) => *position as usize,
                Some(PlayOffset::Uri(uri)) => {
                    uris.iter().position(|other| other == uri).unwrap_or(0)
                }
                None => 0,
            };

            self.load(indices, start, None);
        }
    }

    fn repeat_mode(&self) -> RepeatMode {
        if self.repeat_track {
            RepeatMode::Track
        } else if self.repeat_context {
            RepeatMode::Context
        } else {
            RepeatMode::Off
        }
    }

    fn snapshot(&self) -> PlaybackSnapshot {
        let playing = self.status == PlayStatus::Playing;
        let mut actions = std::collections::BTreeMap::new();

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

        PlaybackSnapshot {
            status: self.status,
            item: Some(track_entity(self.current())),
            context: self.context.clone(),
            position: Some(Position {
                position_ms: self.position_ms(),
                timestamp_ms: now_ms(),
                speed: if playing { 1.0 } else { 0.0 },
            }),
            volume: Some(self.volume),
            is_active: true,
            options: Some(Options {
                shuffle: self.shuffle,
                repeat: self.repeat_mode(),
                playback_speed: 1.0,
            }),
            available_actions: actions,
        }
    }

    fn upcoming(&self) -> Vec<QueueEntry> {
        let queued = self.queued.iter().map(|index| QueueEntry {
            uid: format!("demo-queue-{index}"),
            source: "queue".into(),
            item: track_entity(&catalog().tracks[*index]),
        });
        let context = self
            .order
            .iter()
            .skip(self.cursor + 1)
            .take(10)
            .map(|index| QueueEntry {
                uid: format!("demo-{index}"),
                source: "context".into(),
                item: track_entity(&catalog().tracks[*index]),
            });

        queued.chain(context).collect()
    }

    fn emit(&self, events: &broadcast::Sender<ClientEvent>) {
        let _ = events.send(ClientEvent::Event(Event::PlaybackState(Box::new(
            self.snapshot(),
        ))));
        let _ = events.send(ClientEvent::Event(Event::QueueChanged {
            previous: Vec::new(),
            upcoming: self.upcoming(),
        }));
    }
}

async fn run(
    mut sim: Sim,
    mut commands: mpsc::UnboundedReceiver<Cmd>,
    events: broadcast::Sender<ClientEvent>,
) {
    let _ = events.send(ClientEvent::Event(Event::AuthState {
        logged_in: true,
        is_active: true,
        device_name: DEVICE_NAME.into(),
    }));

    loop {
        let deadline = sim.deadline();

        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else {
                    return;
                };

                sim.handle(command);
            }
            _ = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                sim.advance();
            }
        }

        sim.emit(&events);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn night_drive() -> &'static catalog::DemoPlaylist {
        catalog().playlist("demo-night-drive").unwrap()
    }

    #[test]
    fn stations_carry_their_seed_later_and_have_no_name() {
        let track = &catalog().tracks[5];
        let station = crate::radio::station_uri(&track.uri).unwrap();
        let indices = catalog().context_indices(&station).unwrap();

        assert_ne!(indices[0], 5);
        assert_eq!(indices.iter().filter(|index| **index == 5).count(), 1);
        assert!(indices.len() > 20 && indices.len() <= 51);
        assert!(
            context_entity(&station)
                .decorations
                .identity
                .name
                .is_empty()
        );

        let artist = crate::radio::station_uri(&track.artist_uri()).unwrap();

        assert!(catalog().context_indices(&artist).is_some());
    }

    #[test]
    fn playlist_matches_the_mockups() {
        let playlist = night_drive();
        let tracks = &catalog().tracks[playlist.start..playlist.start + playlist.count];
        let total: u64 = tracks.iter().map(|track| track.duration_ms).sum();

        assert_eq!(playlist.name, "Night Drive");
        assert_eq!(tracks.len(), 32);
        assert_eq!(tracks[0].title, "Midnight City");
        assert_eq!(tracks[2].artist, "Soliste");
        assert_eq!(total / 60_000, 137);
    }

    #[test]
    fn sim_honours_transport_commands() {
        let mut sim = Sim::new(40);

        assert_eq!(sim.status, PlayStatus::Paused);

        sim.handle(Cmd::Play(None));

        assert_eq!(sim.status, PlayStatus::Playing);

        sim.handle(Cmd::Next);

        assert_eq!(sim.current().title, "Afterglow");

        sim.handle(Cmd::Seek(10_000));
        sim.handle(Cmd::Previous);

        assert_eq!(sim.position_ms(), 0);
        assert_eq!(sim.current().title, "Afterglow");

        sim.handle(Cmd::Previous);

        assert_eq!(sim.current().title, "Midnight City");

        sim.handle(Cmd::Volume(80));

        assert_eq!(sim.snapshot().volume, Some(80));

        sim.handle(Cmd::RepeatContext(true));

        assert_eq!(sim.snapshot().options.unwrap().repeat, RepeatMode::Context);

        sim.handle(Cmd::RepeatTrack(true));

        assert_eq!(sim.snapshot().options.unwrap().repeat, RepeatMode::Track);

        sim.advance();

        assert_eq!(sim.current().title, "Midnight City");
    }

    #[test]
    fn shuffle_keeps_the_current_track_and_covers_every_track() {
        let mut sim = Sim::new(40);

        sim.handle(Cmd::Play(Some(catalog().tracks[5].uri.clone())));
        sim.handle(Cmd::Shuffle(true));

        let mut seen = sim.order.clone();

        seen.sort_unstable();

        assert_eq!(sim.current().title, "Solstice");
        assert_eq!(seen, (0..32).collect::<Vec<_>>());
        assert_ne!(sim.order, (0..32).collect::<Vec<_>>());

        sim.handle(Cmd::Shuffle(false));

        assert_eq!(sim.current().title, "Solstice");
        assert_eq!(sim.cursor, 5);
    }

    #[test]
    fn end_of_context_stops_unless_repeating() {
        let mut sim = Sim::new(40);

        sim.handle(Cmd::Play(Some(catalog().tracks[31].uri.clone())));
        sim.advance();

        assert_eq!(sim.status, PlayStatus::Paused);
        assert_eq!(sim.current().title, "Midnight City");
    }

    #[test]
    fn playing_a_playlist_you_do_not_own_exposes_a_context_sequence() {
        let mut sim = Sim::new(40);
        let uri = format!("spotify:playlist:{}", catalog::NOT_OWNED_ID);

        sim.handle(Cmd::PlayWeb(WebPlayRequest {
            context_uri: Some(uri.clone()),
            ..WebPlayRequest::default()
        }));

        let mut model = crate::PlaybackModel::default();

        model.apply(&Event::PlaybackState(Box::new(sim.snapshot())));
        model.apply(&Event::QueueChanged {
            previous: Vec::new(),
            upcoming: sim.upcoming(),
        });

        let sequence = model.context_sequence(&uri).unwrap();

        assert_eq!(sequence.len(), 11);
        assert_eq!(sequence[0].uri, sim.current().uri);
    }

    #[test]
    fn web_play_starts_a_context_at_an_offset_or_a_list() {
        let mut sim = Sim::new(40);
        let album = catalog::album_uri(2);

        sim.handle(Cmd::PlayWeb(WebPlayRequest {
            context_uri: Some(album.clone()),
            offset: Some(PlayOffset::Position(3)),
            ..WebPlayRequest::default()
        }));

        assert_eq!(sim.current().index, catalog().album_indices(2)[3]);
        assert_eq!(sim.context.as_ref().unwrap().uri, album);
        assert_eq!(sim.status, PlayStatus::Playing);

        sim.handle(Cmd::PlayWeb(WebPlayRequest {
            uris: Some(vec![
                catalog().tracks[40].uri.clone(),
                catalog().tracks[41].uri.clone(),
            ]),
            ..WebPlayRequest::default()
        }));

        assert_eq!(sim.current().index, 40);
        assert_eq!(sim.order.len(), 2);
        assert!(sim.context.is_none());
    }

    #[test]
    fn the_collection_context_plays_the_saved_tracks_in_order() {
        let mut sim = Sim::new(40);
        let uri = crate::model::liked_context_uri(catalog::DEMO_USER);

        sim.handle(Cmd::PlayCollection {
            indices: vec![9, 4, 7, 2],
            start: 2,
            context_uri: uri.clone(),
        });

        assert_eq!(sim.current().index, 7);
        assert_eq!(sim.status, PlayStatus::Playing);

        let snapshot = sim.snapshot();
        let context = snapshot.context.unwrap();

        assert_eq!(context.uri, uri);
        assert_eq!(context.name(), "Liked Songs");
        assert_eq!(sim.upcoming()[0].item.uri, catalog().tracks[2].uri);

        sim.advance();
        sim.advance();

        assert_eq!(sim.status, PlayStatus::Paused);
        assert_eq!(sim.current().index, 9);
    }

    #[tokio::test(start_paused = true)]
    async fn the_demo_library_starts_the_collection_from_the_offset_track() {
        use crate::library::LibraryBackend as _;

        let backend = DemoBackend::spawn(&Handle::current(), 40);
        let library = DemoLibrary::new(backend.clone(), DEVICE_NAME.into());
        let mut events = backend.subscribe();
        let liked = library.saved_tracks(0).await.unwrap().items;
        let uri = crate::model::liked_context_uri(catalog::DEMO_USER);

        library
            .start_playback(
                &WebPlayRequest {
                    context_uri: Some(uri.clone()),
                    offset: Some(PlayOffset::Uri(liked[2].uri.clone())),
                    ..WebPlayRequest::default()
                },
                "demo-device",
            )
            .await
            .unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(10)).await;
        tokio::task::yield_now().await;

        let mut last = None;

        while let Ok(event) = events.try_recv() {
            if let ClientEvent::Event(Event::PlaybackState(state)) = event {
                last = Some(state);
            }
        }

        let state = last.unwrap();

        assert_eq!(state.context.unwrap().uri, uri);
        assert_eq!(state.item.unwrap().uri, liked[2].uri);
    }

    #[test]
    fn queued_tracks_play_next_and_the_context_resumes_after_them() {
        let mut sim = Sim::new(40);
        let queued = catalog().tracks[40].uri.clone();

        sim.handle(Cmd::Play(None));
        sim.handle(Cmd::AddToQueue(queued.clone()));
        sim.handle(Cmd::AddToQueue("spotify:track:unknown".into()));
        sim.handle(Cmd::AddToQueue(catalog().tracks[41].uri.clone()));

        let upcoming = sim.upcoming();

        assert_eq!(upcoming[0].source, "queue");
        assert_eq!(upcoming[0].item.uri, queued);
        assert_eq!(upcoming[1].source, "queue");
        assert_eq!(upcoming[2].source, "context");
        assert_eq!(upcoming[2].item.uri, catalog().tracks[1].uri);

        sim.handle(Cmd::Next);

        assert_eq!(sim.current().uri, queued);
        assert_eq!(sim.cursor, 0);
        assert_eq!(sim.upcoming()[0].item.uri, catalog().tracks[41].uri);

        sim.advance();

        assert_eq!(sim.current().index, 41);
        assert!(sim.queued.is_empty());
        assert_eq!(sim.upcoming()[0].source, "context");
        assert_eq!(sim.upcoming()[0].item.uri, catalog().tracks[1].uri);

        sim.advance();

        assert_eq!(sim.current().index, 1);
        assert_eq!(sim.cursor, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn actor_emits_events_and_ends_tracks_on_time() {
        let backend = DemoBackend::spawn(&Handle::current(), 40);
        let mut events = backend.subscribe();

        backend.request_state().unwrap();
        backend.play(None).unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(200)).await;
        tokio::task::yield_now().await;

        let mut titles = Vec::new();

        while let Ok(ClientEvent::Event(event)) = events.try_recv() {
            if let Event::PlaybackState(state) = event {
                titles.push(state.item.unwrap().name().to_owned());
            }
        }

        assert_eq!(titles.first().map(String::as_str), Some("Midnight City"));
        assert_eq!(titles.last().map(String::as_str), Some("Afterglow"));
    }
}
