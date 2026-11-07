use std::sync::Mutex;
use std::time::Duration;

use sonear_soloist::client::ClientEvent;
use sonear_soloist::protocol::{
    Decorations, Entity, Event, PlaybackInfo, PlaybackSnapshot, Position, QueueEntry,
};

use super::*;
use crate::config::ConfigHandle;
use crate::player::{PlayerBackend, PlayerError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Call {
    Play(Option<String>),
    Next,
    AddToQueue(String),
}

/// A backend that only records what the queue sends.
pub(crate) struct FakeBackend {
    calls: Mutex<Vec<Call>>,
    pub(crate) events: broadcast::Sender<ClientEvent>,
}

impl FakeBackend {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            events: broadcast::channel(64).0,
        })
    }

    fn calls(&self) -> Vec<Call> {
        lock(&self.calls).clone()
    }

    fn record(&self, call: Call) -> Result<(), PlayerError> {
        lock(&self.calls).push(call);

        Ok(())
    }

    fn feed(&self, event: Event) {
        let _ = self.events.send(ClientEvent::Event(event));
    }
}

impl PlayerBackend for FakeBackend {
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
        Ok(())
    }

    fn next(&self) -> Result<(), PlayerError> {
        self.record(Call::Next)
    }

    fn previous(&self) -> Result<(), PlayerError> {
        Ok(())
    }

    fn seek(&self, _: u64) -> Result<(), PlayerError> {
        Ok(())
    }

    fn set_volume(&self, _: u8) -> Result<(), PlayerError> {
        Ok(())
    }

    fn set_shuffle(&self, _: bool) -> Result<(), PlayerError> {
        Ok(())
    }

    fn set_repeat_context(&self, _: bool) -> Result<(), PlayerError> {
        Ok(())
    }

    fn set_repeat_track(&self, _: bool) -> Result<(), PlayerError> {
        Ok(())
    }

    fn add_to_queue(&self, uri: &str) -> Result<(), PlayerError> {
        self.record(Call::AddToQueue(uri.to_owned()))
    }

    fn activate(&self) -> Result<(), PlayerError> {
        Ok(())
    }
}

const NOW: u64 = 1_000_000;
const DURATION: u64 = 200_000;

fn entity(uri: &str, duration_ms: u64) -> Entity {
    Entity {
        uri: uri.to_owned(),
        entity_type: "track".to_owned(),
        decorations: Decorations {
            playback: Some(PlaybackInfo {
                duration_ms: Some(duration_ms),
            }),
            ..Decorations::default()
        },
    }
}

fn queue_entry(uri: &str, source: &str) -> QueueEntry {
    QueueEntry {
        uid: uri.to_owned(),
        source: source.to_owned(),
        item: entity(uri, DURATION),
    }
}

fn snapshot(item: &str, duration_ms: u64, left_ms: u64, status: PlayStatus) -> PlaybackSnapshot {
    PlaybackSnapshot {
        status,
        item: Some(entity(item, duration_ms)),
        position: Some(Position {
            position_ms: duration_ms - left_ms,
            timestamp_ms: NOW,
            speed: 1.0,
        }),
        is_active: true,
        ..PlaybackSnapshot::default()
    }
}

/// `item` is playing with `left_ms` remaining at `NOW`, and `upcoming` holds
/// `(uri, source)` pairs.
fn model(item: &str, left_ms: u64, upcoming: &[(&str, &str)]) -> PlaybackModel {
    let mut model = PlaybackModel::default();

    model.apply_at(
        &Event::PlaybackState(Box::new(snapshot(
            item,
            DURATION,
            left_ms,
            PlayStatus::Playing,
        ))),
        NOW,
    );
    model.apply_at(
        &Event::QueueChanged {
            previous: Vec::new(),
            upcoming: upcoming
                .iter()
                .map(|(uri, source)| queue_entry(uri, source))
                .collect(),
        },
        NOW,
    );

    model
}

fn state(uris: &[&str]) -> QueueState {
    let mut state = QueueState::new();

    for uri in uris {
        assert!(state.add((*uri).to_owned(), QueueSource::User));
    }

    state
}

fn uris(state: &QueueState) -> Vec<String> {
    state
        .snapshot()
        .entries
        .into_iter()
        .map(|entry| entry.uri)
        .collect()
}

fn add(uri: &str) -> Vec<QueueCommand> {
    vec![QueueCommand::AddToQueue(uri.to_owned())]
}

const SKIP: QueueCommand = QueueCommand::SkipNext;

#[test]
fn handoff_fires_at_exactly_the_margin() {
    for (left, expected) in [
        (25_001, Vec::new()),
        (25_000, add("b")),
        (24_999, add("b")),
        (100_000, Vec::new()),
    ] {
        let mut queue = state(&["b"]);

        assert_eq!(
            queue.observe(&model("a", left, &[]), NOW),
            expected,
            "{left}"
        );
    }
}

#[test]
fn handoff_follows_the_clock() {
    let mut queue = state(&["b"]);
    let playing = model("a", 30_000, &[("c", "context")]);

    assert!(queue.observe(&playing, NOW + 4_999).is_empty());
    assert_eq!(
        queue.next_wake_ms(&playing, NOW + 4_999),
        Some(1),
        "wakes one millisecond before the margin"
    );
    assert_eq!(queue.observe(&playing, NOW + 5_000), add("b"));
}

#[test]
fn short_tracks_hand_off_straight_away() {
    let mut queue = state(&["b"]);
    let mut short = PlaybackModel::default();

    short.apply_at(
        &Event::PlaybackState(Box::new(snapshot("a", 10_000, 10_000, PlayStatus::Playing))),
        NOW,
    );

    assert_eq!(queue.observe(&short, NOW), add("b"));
}

#[test]
fn nothing_fires_while_paused_or_empty() {
    let mut queue = state(&["b"]);
    let mut paused = model("a", 10_000, &[]);

    paused.status = PlayStatus::Paused;

    assert!(queue.observe(&paused, NOW).is_empty());
    assert_eq!(queue.next_wake_ms(&paused, NOW), None);

    let mut empty = QueueState::new();
    let playing = model("a", 10_000, &[]);

    assert!(empty.observe(&playing, NOW).is_empty());
    assert_eq!(empty.next_wake_ms(&playing, NOW), None);
}

#[test]
fn buffering_still_hands_off() {
    let mut queue = state(&["b"]);
    let mut buffering = model("a", 10_000, &[]);

    buffering.status = PlayStatus::Buffering;

    assert_eq!(queue.observe(&buffering, NOW), add("b"));
}

#[test]
fn waiting_time_is_the_distance_to_the_margin() {
    let queue = state(&["b"]);

    assert_eq!(
        queue.next_wake_ms(&model("a", 60_000, &[]), NOW),
        Some(35_000)
    );
    assert_eq!(queue.next_wake_ms(&model("a", 20_000, &[]), NOW), Some(0));
}

#[test]
fn a_device_move_is_not_a_hand_off_or_a_loss() {
    let mut queue = state(&["b", "c"]);
    let mut away = model("a", 10_000, &[]);

    away.is_active = false;

    assert!(queue.observe(&away, NOW).is_empty());
    assert_eq!(queue.next_wake_ms(&away, NOW), None);

    assert_eq!(queue.observe(&model("a", 10_000, &[]), NOW), add("b"));
    assert!(
        queue
            .observe(&model("a", 10_000, &[("b", "queue")]), NOW)
            .is_empty()
    );

    let mut away = model("a", 10_000, &[]);

    away.is_active = false;

    assert!(queue.observe(&away, NOW + 60_000).is_empty());
    assert_eq!(queue.snapshot().locked.map(|t| t.uri), Some("b".into()));
    assert_eq!(queue.next_wake_ms(&away, NOW + 60_000), None);

    assert!(
        queue
            .observe(&model("a", 10_000, &[("b", "queue")]), NOW + 61_000)
            .is_empty()
    );
    assert!(queue.snapshot().locked_confirmed);
}

#[test]
fn a_disconnect_reset_does_not_lose_the_locked_entry() {
    let mut queue = state(&["b"]);

    queue.observe(&model("a", 10_000, &[]), NOW);

    let mut reset = PlaybackModel::default();

    reset.is_active = true;

    assert!(queue.observe(&reset, NOW + 60_000).is_empty());
    assert!(queue.snapshot().locked.is_some());
}

#[test]
fn foreign_queue_entries_block_the_hand_off() {
    let mut queue = state(&["b"]);

    assert!(
        queue
            .observe(&model("a", 10_000, &[("x", "queue")]), NOW)
            .is_empty()
    );
    assert_eq!(
        queue.next_wake_ms(&model("a", 10_000, &[("x", "queue")]), NOW),
        None
    );
    assert_eq!(
        queue.observe(&model("a", 10_000, &[("y", "context")]), NOW),
        add("b")
    );
}

#[test]
fn confirmed_then_started_drops_the_lock() {
    let mut queue = state(&["b", "c"]);

    queue.observe(&model("a", 20_000, &[]), NOW);

    assert!(!queue.snapshot().locked_confirmed);

    queue.observe(&model("a", 20_000, &[("b", "queue")]), NOW + 10);

    let snapshot = queue.snapshot();

    assert!(snapshot.locked_confirmed);
    assert_eq!(snapshot.locked.map(|t| t.uri), Some("b".into()));
    assert_eq!(uris(&queue), ["c"]);

    let commands = queue.observe(&model("b", 190_000, &[("x", "context")]), NOW + 20_000);

    assert!(commands.is_empty());
    assert_eq!(queue.snapshot().locked, None);
    assert_eq!(uris(&queue), ["c"]);
}

#[test]
fn a_queue_event_before_the_track_event_is_not_a_loss() {
    let mut queue = state(&["b", "c"]);

    queue.observe(&model("a", 20_000, &[]), NOW);
    queue.observe(&model("a", 20_000, &[("b", "queue")]), NOW + 10);

    let gap = model("a", 20_000, &[("x", "context")]);

    assert!(queue.observe(&gap, NOW + 5_000).is_empty());
    assert!(queue.observe(&gap, NOW + 7_000).is_empty());
    assert_eq!(queue.next_wake_ms(&gap, NOW + 7_000), Some(1_000));
    assert!(queue.snapshot().locked.is_some());

    assert!(
        queue
            .observe(&model("b", 190_000, &[]), NOW + 7_500)
            .is_empty()
    );
    assert_eq!(queue.snapshot().locked, None);
    assert_eq!(uris(&queue), ["c"]);
}

#[test]
fn a_track_that_stays_missing_goes_back_to_the_front() {
    let mut queue = state(&["b", "c"]);

    queue.observe(&model("a", 20_000, &[]), NOW);
    queue.observe(&model("a", 20_000, &[("b", "queue")]), NOW + 10);

    let gone = model("a", 20_000, &[]);

    assert!(queue.observe(&gone, NOW + 5_000).is_empty());
    assert!(queue.snapshot().locked.is_some());

    let commands = queue.observe(&gone, NOW + 8_000);

    assert_eq!(commands, add("b"), "lost track is handed over again");
    assert_eq!(uris(&queue), ["c"]);

    let mut queue = state(&["b", "c"]);
    let far = model("a", 100_000, &[]);

    queue.observe(&model("a", 20_000, &[]), NOW);
    queue.observe(&model("a", 20_000, &[("b", "queue")]), NOW + 10);
    queue.observe(&far, NOW + 5_000);
    queue.observe(&far, NOW + 8_000);

    assert_eq!(queue.snapshot().locked, None);
    assert_eq!(uris(&queue), ["b", "c"]);
}

#[test]
fn an_unconfirmed_hand_off_times_out_and_is_sent_again_later() {
    let mut queue = state(&["b"]);
    let far = model("a", 100_000, &[]);

    assert_eq!(queue.observe(&model("a", 20_000, &[]), NOW), add("b"));
    assert_eq!(queue.next_wake_ms(&far, NOW), Some(10_000));
    assert!(queue.observe(&far, NOW + 9_999).is_empty());
    assert!(queue.observe(&far, NOW + 10_000).is_empty());
    assert_eq!(queue.snapshot().locked, None);
    assert_eq!(uris(&queue), ["b"]);

    assert_eq!(
        queue.observe(&model("a", 20_000, &[]), NOW + 90_000),
        add("b")
    );
}

#[test]
fn a_late_confirmation_is_adopted_not_sent_twice() {
    let mut queue = state(&["b"]);
    let far = model("a", 100_000, &[]);

    queue.observe(&model("a", 20_000, &[]), NOW);
    queue.observe(&far, NOW + 10_000);

    assert_eq!(queue.snapshot().locked, None);

    let late = model("a", 100_000, &[("b", "queue")]);

    assert!(queue.observe(&late, NOW + 11_000).is_empty());
    assert!(queue.snapshot().locked_confirmed);
    assert!(queue.snapshot().entries.is_empty());
}

#[test]
fn next_with_nothing_handed_off_adds_then_skips_on_confirmation() {
    let mut queue = state(&["b"]);
    let waiting = model("a", 100_000, &[("x", "context")]);

    assert_eq!(queue.next_pressed(&waiting, NOW), add("b"));
    assert!(queue.observe(&waiting, NOW + 100).is_empty());

    let confirmed = model("a", 100_000, &[("b", "queue"), ("x", "context")]);

    assert_eq!(queue.observe(&confirmed, NOW + 200), vec![SKIP]);
    assert!(queue.observe(&confirmed, NOW + 300).is_empty());
    assert!(
        queue
            .observe(&model("b", 190_000, &[]), NOW + 400)
            .is_empty()
    );
    assert_eq!(queue.snapshot().locked, None);
}

#[test]
fn next_while_confirmed_only_skips() {
    let mut queue = state(&["b", "c"]);

    queue.observe(&model("a", 20_000, &[]), NOW);

    let confirmed = model("a", 20_000, &[("b", "queue")]);

    queue.observe(&confirmed, NOW + 10);

    assert_eq!(queue.next_pressed(&confirmed, NOW + 20), vec![SKIP]);
    assert_eq!(uris(&queue), ["c"]);
}

#[test]
fn next_while_unconfirmed_skips_on_confirmation() {
    let mut queue = state(&["b"]);
    let unconfirmed = model("a", 20_000, &[]);

    queue.observe(&unconfirmed, NOW);

    assert!(queue.next_pressed(&unconfirmed, NOW + 5).is_empty());
    assert_eq!(
        queue.observe(&model("a", 20_000, &[("b", "queue")]), NOW + 10),
        vec![SKIP]
    );
}

#[test]
fn next_without_entries_or_with_foreign_tracks_skips() {
    let mut empty = QueueState::new();

    assert_eq!(
        empty.next_pressed(&model("a", 100_000, &[]), NOW),
        vec![SKIP]
    );

    let mut queue = state(&["b"]);
    let foreign = model("a", 100_000, &[("x", "queue")]);

    assert_eq!(queue.next_pressed(&foreign, NOW), vec![SKIP]);
    assert_eq!(uris(&queue), ["b"]);

    let mut away = model("a", 100_000, &[]);

    away.is_active = false;

    assert_eq!(queue.next_pressed(&away, NOW), vec![SKIP]);
    assert_eq!(uris(&queue), ["b"]);
}

#[test]
fn a_timeout_while_waiting_on_next_still_skips() {
    let mut queue = state(&["b"]);
    let waiting = model("a", 100_000, &[("x", "context")]);

    assert_eq!(queue.next_pressed(&waiting, NOW), add("b"));
    assert_eq!(queue.observe(&waiting, NOW + 10_000), vec![SKIP]);
    assert_eq!(queue.snapshot().locked, None);
    assert_eq!(uris(&queue), ["b"]);
}

#[test]
fn play_now_starts_directly_when_something_is_in_the_way() {
    let mut queue = state(&["b", "c"]);
    let id = queue.snapshot().entries[1].id;

    assert_eq!(
        queue.play_now(id, &model("a", 100_000, &[("x", "queue")]), NOW),
        vec![QueueCommand::PlayNow("c".into())]
    );
    assert_eq!(uris(&queue), ["b"]);

    let id = queue.snapshot().entries[0].id;

    queue.observe(&model("a", 10_000, &[]), NOW);

    let mut queue = state(&["b", "c"]);

    queue.observe(&model("a", 10_000, &[]), NOW);

    let id_c = queue.snapshot().entries[0].id;

    assert_eq!(
        queue.play_now(id_c, &model("a", 10_000, &[("b", "queue")]), NOW),
        vec![QueueCommand::PlayNow("c".into())]
    );
    assert_eq!(queue.snapshot().locked.map(|t| t.uri), Some("b".into()));
    assert_ne!(id, id_c);
}

#[test]
fn play_now_otherwise_behaves_like_next() {
    let mut queue = state(&["b", "c"]);
    let id = queue.snapshot().entries[1].id;

    assert_eq!(queue.play_now(id, &model("a", 100_000, &[]), NOW), add("c"));
    assert_eq!(queue.snapshot().locked.map(|t| t.uri), Some("c".into()));
    assert_eq!(uris(&queue), ["b"]);
    assert!(
        queue
            .play_now(9_999, &model("a", 100_000, &[]), NOW)
            .is_empty()
    );
}

#[test]
fn queueing_the_playing_track_is_not_started_until_replayed() {
    let mut queue = state(&["b"]);

    assert_eq!(queue.observe(&model("b", 20_000, &[]), NOW), add("b"));
    assert!(
        queue
            .observe(&model("b", 19_000, &[]), NOW + 1_000)
            .is_empty()
    );
    assert!(queue.snapshot().locked.is_some());

    queue.observe(&model("b", 19_000, &[("b", "queue")]), NOW + 2_000);

    assert!(queue.snapshot().locked_confirmed);

    queue.observe(&model("c", 190_000, &[("b", "queue")]), NOW + 3_000);

    assert!(queue.snapshot().locked.is_some());

    queue.observe(&model("b", 190_000, &[]), NOW + 4_000);

    assert_eq!(queue.snapshot().locked, None);
}

#[test]
fn a_saved_entry_already_in_soloist_is_adopted_without_sending() {
    let mut queue = state(&["b", "c"]);
    let saved = queue.to_saved();
    let mut restarted = QueueState::from_saved(saved);

    assert!(
        restarted
            .observe(&model("a", 10_000, &[("b", "queue")]), NOW)
            .is_empty()
    );

    let snapshot = restarted.snapshot();

    assert_eq!(snapshot.locked.map(|t| t.uri), Some("b".into()));
    assert!(snapshot.locked_confirmed);
    assert_eq!(uris(&restarted), ["c"]);

    assert!(queue.observe(&model("a", 10_000, &[]), NOW).len() == 1);
}

#[test]
fn move_before_works_on_ids() {
    let mut queue = state(&["a", "b", "c", "d"]);
    let ids: Vec<u64> = queue.snapshot().entries.iter().map(|e| e.id).collect();

    queue.move_before(ids[3], Some(ids[1]));

    assert_eq!(uris(&queue), ["a", "d", "b", "c"]);

    queue.move_before(ids[0], None);

    assert_eq!(uris(&queue), ["d", "b", "c", "a"]);

    queue.move_before(ids[1], Some(ids[1]));

    assert_eq!(uris(&queue), ["d", "b", "c", "a"]);

    queue.move_before(ids[2], Some(9_999));

    assert_eq!(uris(&queue), ["d", "b", "a", "c"]);

    queue.move_before(9_999, Some(ids[0]));

    assert_eq!(uris(&queue), ["d", "b", "a", "c"]);
}

#[test]
fn a_hand_off_during_a_drag_does_not_break_the_drop() {
    let mut queue = state(&["a", "b", "c"]);
    let ids: Vec<u64> = queue.snapshot().entries.iter().map(|e| e.id).collect();

    queue.observe(&model("x", 10_000, &[]), NOW);
    queue.move_before(ids[2], Some(ids[0]));

    assert_eq!(uris(&queue), ["b", "c"]);
}

#[test]
fn clear_keeps_the_locked_entry() {
    let mut queue = state(&["b", "c", "d"]);

    queue.observe(&model("a", 10_000, &[]), NOW);
    queue.clear();

    assert!(queue.snapshot().entries.is_empty());
    assert_eq!(queue.snapshot().locked.map(|t| t.uri), Some("b".into()));
    assert_eq!(queue.to_saved().entries.len(), 1);

    queue.reset();

    assert_eq!(queue.snapshot(), QueueSnapshot::default());
}

#[test]
fn remove_and_duplicates() {
    let mut queue = state(&["a", "a", "b"]);
    let id = queue.snapshot().entries[0].id;

    queue.remove(id);
    queue.remove(id);
    queue.remove(12_345);

    assert_eq!(uris(&queue), ["a", "b"]);
}

#[test]
fn saved_queue_round_trips_with_the_locked_entry_first() {
    let mut queue = QueueState::new();

    queue.add("b".into(), QueueSource::Dj);
    queue.add("c".into(), QueueSource::User);
    queue.add("d".into(), QueueSource::Assistant);
    queue.observe(&model("a", 10_000, &[]), NOW);

    let saved = queue.to_saved();
    let text = serde_json::to_string(&saved).unwrap();

    assert_eq!(
        text,
        r#"{"entries":[{"uri":"b","source":"dj"},{"uri":"c","source":"user"},{"uri":"d","source":"assistant"}]}"#
    );

    let loaded = QueueState::from_saved(serde_json::from_str(&text).unwrap());

    assert_eq!(uris(&loaded), ["b", "c", "d"]);
    assert_eq!(loaded.snapshot().locked, None);

    let plain: SavedQueue = serde_json::from_str(r#"{"entries":[{"uri":"x"}]}"#).unwrap();

    assert_eq!(plain.entries[0].source, QueueSource::User);
}

#[test]
fn the_queue_has_a_limit() {
    let mut queue = QueueState::new();

    for index in 0..QUEUE_LIMIT {
        assert!(queue.add(format!("t{index}"), QueueSource::User));
    }

    assert!(!queue.add("one-too-many".into(), QueueSource::User));
    assert_eq!(queue.snapshot().entries.len(), QUEUE_LIMIT);
}

async fn settle() {
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
}

struct Engine {
    backend: Arc<FakeBackend>,
    manager: Arc<QueueManager>,
    start: tokio::time::Instant,
    _dir: tempfile::TempDir,
}

impl Engine {
    fn new(path: Option<&str>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = ConfigHandle::load(dir.path().join("config.toml"));
        let backend = FakeBackend::new();
        let (events, _) = broadcast::channel(64);
        let player = Player::new(backend.clone(), Handle::current(), config, events.clone());
        let start = tokio::time::Instant::now();
        let clock: Clock = Arc::new(move || NOW + start.elapsed().as_millis() as u64);
        let path = path.map(|name| dir.path().join(name));
        let manager = QueueManager::with_clock(player, events, path, Handle::current(), clock);

        Self {
            backend,
            manager,
            start,
            _dir: dir,
        }
    }

    fn clock(&self) -> u64 {
        NOW + self.start.elapsed().as_millis() as u64
    }

    fn play(&self, item: &str, left_ms: u64) {
        let mut playing = snapshot(item, DURATION, left_ms, PlayStatus::Playing);

        playing.position = Some(Position {
            position_ms: DURATION - left_ms,
            timestamp_ms: self.clock(),
            speed: 1.0,
        });

        self.backend.feed(Event::PlaybackState(Box::new(playing)));
        self.backend.feed(Event::QueueChanged {
            previous: Vec::new(),
            upcoming: vec![queue_entry("ctx", "context")],
        });
    }

    async fn advance(&self, ms: u64) {
        settle().await;
        tokio::time::advance(Duration::from_millis(ms)).await;
        settle().await;
    }
}

#[tokio::test(start_paused = true)]
async fn the_engine_hands_off_at_the_right_time() {
    let engine = Engine::new(None);

    assert!(engine.manager.add("b", QueueSource::User));

    engine.play("a", 60_000);
    settle().await;

    assert!(engine.backend.calls().is_empty());

    engine.advance(34_900).await;

    assert!(engine.backend.calls().is_empty());

    engine.advance(200).await;

    assert_eq!(engine.backend.calls(), [Call::AddToQueue("b".into())]);
    assert_eq!(
        engine.manager.snapshot().locked.map(|t| t.uri),
        Some("b".into())
    );

    engine.advance(30_000).await;

    assert_eq!(
        engine.backend.calls().len(),
        2,
        "no confirmation by the deadline hands the track over again"
    );
}

#[tokio::test(start_paused = true)]
async fn the_engine_runs_the_next_button_through_confirmation() {
    let engine = Engine::new(None);

    engine.manager.add("b", QueueSource::User);
    engine.play("a", 100_000);
    settle().await;
    engine.manager.next();
    settle().await;

    assert_eq!(engine.backend.calls(), [Call::AddToQueue("b".into())]);

    engine.backend.feed(Event::QueueChanged {
        previous: Vec::new(),
        upcoming: vec![queue_entry("b", "queue"), queue_entry("ctx", "context")],
    });
    settle().await;

    assert_eq!(
        engine.backend.calls(),
        [Call::AddToQueue("b".into()), Call::Next]
    );
}

#[tokio::test(start_paused = true)]
async fn the_engine_sends_nothing_for_an_empty_queue() {
    let engine = Engine::new(None);

    engine.play("a", 100_000);
    settle().await;
    engine.advance(95_000).await;
    engine.advance(10_000).await;

    assert!(engine.backend.calls().is_empty());
}

#[tokio::test]
async fn the_engine_saves_and_reloads_uris_only() {
    let engine = Engine::new(Some("queue.json"));
    let path = engine._dir.path().join("queue.json");
    let mut events = engine.manager.events.subscribe();

    engine.manager.add("spotify:track:b", QueueSource::Dj);

    tokio::time::timeout(Duration::from_secs(5), async {
        while !matches!(events.recv().await, Ok(AppEvent::Queue)) {}
    })
    .await
    .unwrap();

    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        r#"{"entries":[{"uri":"spotify:track:b","source":"dj"}]}"#
    );

    let reloaded = load(&path);

    assert_eq!(uris(&reloaded), ["spotify:track:b"]);

    fs::write(&path, "not json").unwrap();

    assert!(load(&path).snapshot().entries.is_empty());

    engine.manager.wipe().unwrap();

    assert!(!path.exists());
    assert_eq!(engine.manager.snapshot(), QueueSnapshot::default());
}
