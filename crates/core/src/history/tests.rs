use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;

use super::*;
use crate::meta::MetaCache;
use crate::model::{Album, AlbumRef, Artist, PlayRecord, Playlist, Track};

fn row(at: u64, uri: &str, context: Option<&str>) -> HistoryRow {
    HistoryRow {
        played_at_ms: at,
        track_uri: uri.to_owned(),
        context_uri: context.map(str::to_owned),
    }
}

#[tokio::test]
async fn insert_ignores_duplicates_and_lists_newest_first() {
    let store = HistoryStore::in_memory();

    assert_eq!(
        store
            .insert(vec![
                row(10, "a", None),
                row(30, "b", Some("c")),
                row(20, "a", None)
            ])
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        store
            .insert(vec![row(10, "a", None), row(40, "d", None)])
            .await
            .unwrap(),
        1
    );
    assert_eq!(store.count().await.unwrap(), 4);
    assert_eq!(store.latest().await.unwrap(), Some(40));

    let recent = store.recent(2, 0).await.unwrap();

    assert_eq!(recent[0].track_uri, "d");
    assert_eq!(recent[1].context_uri.as_deref(), Some("c"));
    assert_eq!(store.recent(10, 3).await.unwrap()[0].played_at_ms, 10);
}

#[tokio::test]
async fn the_same_track_at_different_times_is_two_plays() {
    let store = HistoryStore::in_memory();

    store
        .insert(vec![row(1, "a", None), row(2, "a", None)])
        .await
        .unwrap();

    assert_eq!(store.count().await.unwrap(), 2);
}

#[tokio::test]
async fn prune_and_clear() {
    let store = HistoryStore::in_memory();

    store
        .insert(vec![row(5, "a", None), row(50, "b", None)])
        .await
        .unwrap();

    assert_eq!(store.prune(10).await.unwrap(), 1);
    assert_eq!(store.count().await.unwrap(), 1);

    store.clear().await.unwrap();

    assert_eq!(store.latest().await.unwrap(), None);
}

#[tokio::test]
async fn wipe_deletes_the_file_and_the_store_reopens_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sub").join("history.db");
    let store = HistoryStore::open(&path);

    assert!(!path.exists());

    store.insert(vec![row(1, "a", None)]).await.unwrap();

    assert!(path.exists());

    store.wipe().await.unwrap();

    assert!(!path.exists());
    assert_eq!(store.count().await.unwrap(), 0);

    drop(store);
}

#[tokio::test]
async fn data_persists_across_opens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");

    HistoryStore::open(&path)
        .insert(vec![row(7, "a", Some("ctx"))])
        .await
        .unwrap();

    let reopened = HistoryStore::open(&path);

    assert_eq!(reopened.recent(5, 0).await.unwrap().len(), 1);
}

fn track(id: &str, album: Option<&str>) -> Track {
    Track {
        uri: format!("spotify:track:{id}"),
        id: Some(id.to_owned()),
        name: id.to_owned(),
        album: album.map(|album| AlbumRef {
            id: Some(album.to_owned()),
            uri: format!("spotify:album:{album}"),
            name: album.to_owned(),
        }),
        ..Track::default()
    }
}

#[derive(Default)]
struct Fake {
    plays: Mutex<Vec<PlayRecord>>,
    fetches: Mutex<usize>,
    contexts: HashMap<String, LibraryError>,
    tracks: HashMap<String, Track>,
}

#[async_trait]
impl LibraryBackend for Fake {
    async fn recently_played(&self, _limit: u32) -> LibraryResult<Vec<PlayRecord>> {
        *self.fetches.lock().unwrap() += 1;

        Ok(self.plays.lock().unwrap().clone())
    }

    async fn track(&self, id: &str) -> LibraryResult<Track> {
        self.tracks.get(id).cloned().ok_or(LibraryError::NotFound)
    }

    async fn playlist(&self, id: &str) -> LibraryResult<Playlist> {
        if let Some(error) = self.contexts.get(&format!("playlist:{id}")) {
            return Err(error.clone());
        }

        Ok(Playlist {
            id: id.to_owned(),
            uri: format!("spotify:playlist:{id}"),
            name: format!("Playlist {id}"),
            track_count: Some(3),
            ..Playlist::default()
        })
    }

    async fn album(&self, id: &str) -> LibraryResult<Album> {
        if let Some(error) = self.contexts.get(&format!("album:{id}")) {
            return Err(error.clone());
        }

        Ok(Album {
            id: id.to_owned(),
            uri: format!("spotify:album:{id}"),
            name: format!("Album {id}"),
            total_tracks: 9,
            ..Album::default()
        })
    }

    async fn artist(&self, id: &str) -> LibraryResult<Artist> {
        Ok(Artist {
            id: id.to_owned(),
            uri: format!("spotify:artist:{id}"),
            name: id.to_owned(),
            ..Artist::default()
        })
    }
}

fn play(at: u64, id: &str, context: Option<&str>) -> PlayRecord {
    PlayRecord {
        played_at_ms: at,
        track: track(id, Some("alb")),
        context_uri: context.map(str::to_owned),
    }
}

fn history_with(
    fake: Fake,
    retention: u32,
) -> (Arc<History>, Arc<Fake>, broadcast::Receiver<AppEvent>) {
    let dir = tempfile::tempdir().unwrap();
    let config = ConfigHandle::load(dir.path().join("config.toml"));

    config.update(|config| config.history_retention_months = retention);

    let fake = Arc::new(fake);
    let library: Arc<dyn LibraryBackend> = fake.clone();
    let resolver = TrackResolver::new(library.clone(), MetaCache::in_memory());
    let (events, receiver) = broadcast::channel(16);
    let history = History::new(HistoryStore::in_memory(), library, resolver, config, events);

    (history, fake, receiver)
}

#[tokio::test]
async fn syncs_add_only_new_plays_and_never_duplicate() {
    let fake = Fake::default();

    fake.plays.lock().unwrap().extend([
        play(300, "b", Some("spotify:playlist:p")),
        play(200, "a", None),
    ]);

    let (history, fake, mut events) = history_with(fake, 12);

    assert_eq!(history.sync().await.unwrap(), 2);
    assert!(matches!(events.try_recv(), Ok(AppEvent::HistoryChanged)));
    assert_eq!(history.sync().await.unwrap(), 0);
    assert!(events.try_recv().is_err());

    fake.plays.lock().unwrap().push(play(400, "c", None));

    assert_eq!(history.sync().await.unwrap(), 1);
    assert_eq!(history.recent(10, 0).await.len(), 3);
    assert!(history.resolver().cached("spotify:track:b").is_some());
}

#[tokio::test]
async fn a_play_listed_late_is_still_added() {
    let fake = Fake::default();

    fake.plays.lock().unwrap().push(play(300, "newer", None));

    let (history, fake, _) = history_with(fake, 12);

    assert_eq!(history.sync().await.unwrap(), 1);

    fake.plays.lock().unwrap().push(play(200, "late", None));

    assert_eq!(history.sync().await.unwrap(), 1);
    assert_eq!(history.recent(10, 0).await.len(), 2);
}

#[tokio::test]
async fn a_finished_track_wakes_the_sync_loop() {
    let (history, fake, _) = history_with(Fake::default(), 12);
    let task = history.spawn_sync_loop(&Handle::current());
    let fetches = |count: usize| {
        let fake = fake.clone();

        async move {
            for _ in 0..200 {
                if *fake.fetches.lock().unwrap() >= count {
                    return true;
                }

                tokio::time::sleep(Duration::from_millis(5)).await;
            }

            false
        }
    };

    assert!(fetches(1).await);

    history.sync_soon();

    assert!(fetches(2).await);

    task.abort();
}

#[tokio::test]
async fn old_plays_are_pruned_by_the_retention_setting() {
    let (history, _, _) = history_with(Fake::default(), 1);
    let day = 24 * 60 * 60 * 1000;
    let now = now_ms();

    history
        .store()
        .insert(vec![
            row(now - 45 * day, "old", None),
            row(now - 2 * day, "recent", None),
        ])
        .await
        .unwrap();

    assert_eq!(history.prune_old().await.unwrap(), 1);
    assert_eq!(history.recent(10, 0).await[0].track_uri, "recent");
    assert_eq!(history.recent(10, 0).await.len(), 1);
}

#[tokio::test]
async fn clear_empties_the_history_and_notifies() {
    let (history, _, mut events) = history_with(Fake::default(), 12);

    history
        .store()
        .insert(vec![row(1, "a", None)])
        .await
        .unwrap();
    history.clear().await.unwrap();

    assert!(history.recent(10, 0).await.is_empty());
    assert!(matches!(events.try_recv(), Ok(AppEvent::HistoryChanged)));
}

#[tokio::test]
async fn cleared_plays_do_not_come_back_on_the_next_sync() {
    let fake = Fake::default();

    fake.plays.lock().unwrap().push(play(100, "old", None));

    let (history, fake, _) = history_with(fake, 12);

    assert_eq!(history.sync().await.unwrap(), 1);

    history.clear().await.unwrap();

    assert_eq!(history.sync().await.unwrap(), 0);
    assert!(history.recent(10, 0).await.is_empty());

    fake.plays
        .lock()
        .unwrap()
        .push(play(now_ms() + 60_000, "new", None));

    assert_eq!(history.sync().await.unwrap(), 1);
    assert_eq!(
        history.recent(10, 0).await[0].track_uri,
        "spotify:track:new"
    );
}

#[tokio::test]
async fn hero_skips_a_context_that_answers_403_or_404() {
    let mut fake = Fake::default();

    fake.contexts
        .insert("playlist:forbidden".into(), LibraryError::Forbidden);
    fake.contexts
        .insert("album:gone".into(), LibraryError::NotFound);

    let (history, _, _) = history_with(fake, 12);

    history
        .store()
        .insert(vec![
            row(500, "t1", Some("spotify:playlist:forbidden")),
            row(400, "t2", Some("spotify:album:gone")),
            row(300, "t3", Some("spotify:playlist:forbidden")),
            row(200, "t4", Some("spotify:playlist:ok")),
            row(100, "t5", Some("spotify:album:later")),
        ])
        .await
        .unwrap();

    let hero = history.hero().await.unwrap().unwrap();

    assert_eq!(hero.kind, HeroKind::Playlist);
    assert_eq!(hero.id, "ok");
    assert_eq!(hero.track_count, Some(3));
}

#[tokio::test]
async fn hero_uses_the_tracks_album_for_a_play_without_context() {
    let mut fake = Fake::default();

    fake.tracks
        .insert("solo".into(), track("solo", Some("alb9")));

    let (history, _, _) = history_with(fake, 12);

    history
        .store()
        .insert(vec![row(10, "spotify:track:solo", None)])
        .await
        .unwrap();

    let hero = history.hero().await.unwrap().unwrap();

    assert_eq!(hero.kind, HeroKind::Album);
    assert_eq!(hero.uri, "spotify:album:alb9");
    assert_eq!(hero.track_count, Some(9));
}

#[tokio::test]
async fn hero_is_none_when_nothing_resolves_and_errors_stop_the_search() {
    let mut fake = Fake::default();

    fake.contexts
        .insert("playlist:a".into(), LibraryError::Forbidden);

    let (history, _, _) = history_with(fake, 12);

    assert_eq!(history.hero().await, Ok(None));

    history
        .store()
        .insert(vec![row(1, "t", Some("spotify:playlist:a"))])
        .await
        .unwrap();

    assert_eq!(history.hero().await, Ok(None));

    let mut fake = Fake::default();

    fake.contexts
        .insert("playlist:net".into(), LibraryError::Network("down".into()));

    let (history, _, _) = history_with(fake, 12);

    history
        .store()
        .insert(vec![
            row(2, "t", Some("spotify:playlist:net")),
            row(1, "u", Some("spotify:playlist:fine")),
        ])
        .await
        .unwrap();

    assert!(history.hero().await.is_err());
}
