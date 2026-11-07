use sonear_soloist::client::ClientEvent;
use sonear_soloist::protocol::{
    Decorations, Entity, Event, Identity, Options, PlaybackSnapshot, QueueEntry,
};
use tempfile::TempDir;

use super::*;
use crate::config::ConfigHandle;
use crate::queue::tests::FakeBackend;

const PLAYLIST: &str = "spotify:playlist:p1";

struct Fixture {
    backend: Arc<FakeBackend>,
    meta: Arc<MetaCache>,
    events: broadcast::Receiver<AppEvent>,
    _dir: TempDir,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let config = ConfigHandle::load(dir.path().join("config.toml"));
    let backend = FakeBackend::new();
    let (events, receiver) = broadcast::channel(64);
    let player = Player::new(backend.clone(), Handle::current(), config, events.clone());
    let meta = MetaCache::in_memory();

    SeenCollector::spawn(player, meta.clone(), events, &Handle::current());

    Fixture {
        backend,
        meta,
        events: receiver,
        _dir: dir,
    }
}

fn track(uri: &str) -> Entity {
    Entity {
        uri: uri.to_owned(),
        entity_type: "track".to_owned(),
        decorations: Decorations {
            identity: Identity {
                name: uri.to_owned(),
            },
            ..Decorations::default()
        },
    }
}

fn context(uri: &str) -> Entity {
    Entity {
        uri: uri.to_owned(),
        entity_type: "playlist".to_owned(),
        decorations: Decorations::default(),
    }
}

fn play_with(fx: &Fixture, context_uri: &str, tracks: &[&str], shuffle: bool) {
    let _ = fx
        .backend
        .events
        .send(ClientEvent::Event(Event::PlaybackState(Box::new(
            PlaybackSnapshot {
                item: Some(track(tracks[0])),
                context: Some(context(context_uri)),
                options: Some(Options {
                    shuffle,
                    ..Options::default()
                }),
                ..PlaybackSnapshot::default()
            },
        ))));
    let _ = fx
        .backend
        .events
        .send(ClientEvent::Event(Event::QueueChanged {
            previous: Vec::new(),
            upcoming: tracks[1..]
                .iter()
                .map(|uri| QueueEntry {
                    uid: (*uri).to_owned(),
                    source: "context".to_owned(),
                    item: track(uri),
                })
                .collect(),
        }));
}

/// Plays `tracks[0]` in `context_uri` with the rest coming up.
fn play(fx: &Fixture, context_uri: &str, tracks: &[&str]) {
    play_with(fx, context_uri, tracks, false);
}

async fn pass(duration: Duration) {
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }

    tokio::time::sleep(duration).await;

    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
}

fn saved(fx: &Fixture, id: &str) -> Option<Vec<String>> {
    fx.meta
        .get::<Vec<Track>>(&seen_key(id))
        .map(|cached| cached.value.into_iter().map(|track| track.uri).collect())
}

fn seen_events(fx: &mut Fixture) -> Vec<String> {
    let mut ids = Vec::new();

    while let Ok(event) = fx.events.try_recv() {
        if let AppEvent::SeenTracks { playlist_id } = event {
            ids.push(playlist_id);
        }
    }

    ids
}

fn strings(uris: &[&str]) -> Option<Vec<String>> {
    Some(uris.iter().map(|uri| (*uri).to_owned()).collect())
}

#[tokio::test(start_paused = true)]
async fn a_sequence_that_settles_is_saved_and_announced() {
    let mut fx = fixture();

    play(&fx, PLAYLIST, &["a", "b", "c"]);
    pass(SEEN_SETTLE - Duration::from_millis(500)).await;

    assert_eq!(saved(&fx, "p1"), None);

    pass(Duration::from_secs(1)).await;

    assert_eq!(saved(&fx, "p1"), strings(&["a", "b", "c"]));
    assert_eq!(seen_events(&mut fx), ["p1"]);

    pass(Duration::from_secs(10)).await;

    assert!(seen_events(&mut fx).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_sequence_that_keeps_changing_is_saved_once_stable() {
    let mut fx = fixture();

    play(&fx, PLAYLIST, &["a", "b"]);
    pass(Duration::from_millis(1500)).await;
    play(&fx, PLAYLIST, &["a", "b", "c"]);
    pass(Duration::from_millis(1500)).await;
    play(&fx, PLAYLIST, &["a", "b", "c", "d"]);
    pass(Duration::from_millis(1500)).await;

    assert_eq!(saved(&fx, "p1"), None);

    pass(Duration::from_secs(1)).await;

    assert_eq!(saved(&fx, "p1"), strings(&["a", "b", "c", "d"]));
    assert_eq!(seen_events(&mut fx), ["p1"]);
}

#[tokio::test(start_paused = true)]
async fn switching_context_before_it_settles_saves_nothing_for_the_old_one() {
    let mut fx = fixture();

    play(&fx, PLAYLIST, &["a", "b"]);
    pass(Duration::from_secs(1)).await;
    play(&fx, "spotify:album:x", &["x1", "x2"]);
    pass(Duration::from_secs(5)).await;

    assert_eq!(saved(&fx, "p1"), None);
    assert!(seen_events(&mut fx).is_empty());

    play(&fx, "spotify:playlist:p2", &["m", "n"]);
    pass(Duration::from_secs(3)).await;

    assert_eq!(saved(&fx, "p1"), None);
    assert_eq!(saved(&fx, "p2"), strings(&["m", "n"]));
    assert_eq!(seen_events(&mut fx), ["p2"]);
}

#[tokio::test(start_paused = true)]
async fn merging_keeps_earlier_tracks() {
    let mut fx = fixture();

    play(&fx, PLAYLIST, &["a", "b", "c"]);
    pass(Duration::from_secs(3)).await;
    play(&fx, PLAYLIST, &["c", "d", "e"]);
    pass(Duration::from_secs(3)).await;

    assert_eq!(saved(&fx, "p1"), strings(&["a", "b", "c", "d", "e"]));
    assert_eq!(seen_events(&mut fx), ["p1", "p1"]);
}

#[tokio::test(start_paused = true)]
async fn a_reorder_seen_later_corrects_the_saved_list_and_keeps_the_rest() {
    let mut fx = fixture();

    play(&fx, PLAYLIST, &["a", "b", "c", "d"]);
    pass(Duration::from_secs(3)).await;
    play(&fx, PLAYLIST, &["a", "c", "b", "d"]);
    pass(Duration::from_secs(3)).await;

    assert_eq!(saved(&fx, "p1"), strings(&["a", "c", "b", "d"]));

    play(&fx, PLAYLIST, &["a", "d"]);
    pass(Duration::from_secs(3)).await;

    assert_eq!(saved(&fx, "p1"), strings(&["a", "d", "c", "b"]));
    assert_eq!(seen_events(&mut fx), ["p1", "p1", "p1"]);
}

#[tokio::test(start_paused = true)]
async fn a_sequence_with_nothing_new_sends_no_event() {
    let mut fx = fixture();

    play(&fx, PLAYLIST, &["a", "b"]);
    pass(Duration::from_secs(3)).await;
    play(&fx, "spotify:album:x", &["x1"]);
    pass(Duration::from_secs(3)).await;
    play(&fx, PLAYLIST, &["a", "b"]);
    pass(Duration::from_secs(3)).await;

    assert_eq!(seen_events(&mut fx), ["p1"]);
}

#[tokio::test(start_paused = true)]
async fn a_non_playlist_context_is_ignored() {
    let mut fx = fixture();

    play(&fx, "spotify:album:x", &["a", "b"]);
    pass(Duration::from_secs(5)).await;
    play(&fx, "spotify:user:me:collection", &["c", "d"]);
    pass(Duration::from_secs(5)).await;

    assert!(seen_events(&mut fx).is_empty());
    assert_eq!(saved(&fx, "x"), None);
    assert_eq!(saved(&fx, "collection"), None);
}

#[tokio::test(start_paused = true)]
async fn shuffled_playback_collects_nothing_and_drops_a_pending_deadline() {
    let mut fx = fixture();

    play_with(&fx, PLAYLIST, &["c", "a", "b"], true);
    pass(Duration::from_secs(5)).await;

    assert_eq!(saved(&fx, "p1"), None);

    play(&fx, PLAYLIST, &["a", "b"]);
    pass(Duration::from_secs(1)).await;
    play_with(&fx, PLAYLIST, &["a", "b"], true);
    pass(Duration::from_secs(5)).await;

    assert_eq!(saved(&fx, "p1"), None);
    assert!(seen_events(&mut fx).is_empty());
}
