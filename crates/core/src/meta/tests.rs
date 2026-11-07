use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;

use super::*;
use crate::library::{LibraryError, LibraryResult};

const MINUTE: u64 = 60_000;
const DAY: u64 = 24 * 60 * MINUTE;

fn cache_at(dir: &Path, now: &Arc<AtomicU64>) -> Arc<MetaCache> {
    let now = now.clone();

    MetaCache::open_with_clock(dir, Arc::new(move || now.load(Ordering::SeqCst)))
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

#[test]
fn values_are_fresh_then_stale_then_expired() {
    let dir = tempfile::tempdir().unwrap();
    let now = Arc::new(AtomicU64::new(1_000_000));
    let cache = cache_at(dir.path(), &now);

    assert!(cache.get::<String>("k").is_none());

    cache.put("k", &"value".to_owned());

    let fresh = cache.get::<String>("k").unwrap();

    assert_eq!(fresh.value, "value");
    assert_eq!(fresh.freshness, Freshness::Fresh);

    now.fetch_add(6 * MINUTE, Ordering::SeqCst);

    assert_eq!(
        cache.get::<String>("k").unwrap().freshness,
        Freshness::Stale
    );

    now.fetch_add(7 * DAY, Ordering::SeqCst);

    assert!(cache.get::<String>("k").is_none());
    assert!(json_files(dir.path()).is_empty());
}

#[test]
fn data_survives_a_restart_and_expires_at_startup() {
    let dir = tempfile::tempdir().unwrap();
    let now = Arc::new(AtomicU64::new(5_000_000));

    cache_at(dir.path(), &now).put("a", &vec![1, 2, 3]);
    now.fetch_add(10 * MINUTE, Ordering::SeqCst);

    let reopened = cache_at(dir.path(), &now);

    assert_eq!(reopened.get::<Vec<i32>>("a").unwrap().value, [1, 2, 3]);

    now.fetch_add(8 * DAY, Ordering::SeqCst);
    cache_at(dir.path(), &now);

    assert!(json_files(dir.path()).is_empty());
}

#[test]
fn corrupt_files_are_dropped_not_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let now = Arc::new(AtomicU64::new(1));
    let cache = cache_at(dir.path(), &now);

    cache.put("k", &1_u32);

    let file = json_files(dir.path()).remove(0);
    let fresh_process = cache_at(dir.path(), &now);

    fs::write(&file, "{ not json").unwrap();

    assert!(fresh_process.get::<u32>("k").is_none());
    assert!(!file.exists());
}

#[test]
fn wrong_shape_is_a_miss() {
    let cache = MetaCache::in_memory();

    cache.put("k", &"text".to_owned());

    assert!(cache.get::<Vec<u32>>("k").is_none());
    assert!(cache.get::<String>("k").is_none());
}

#[test]
fn writes_leave_no_temp_file_and_leftovers_are_purged() {
    let dir = tempfile::tempdir().unwrap();
    let now = Arc::new(AtomicU64::new(1));
    let cache = cache_at(dir.path(), &now);

    cache.put("k", &1_u32);
    cache.put("k", &2_u32);

    let files = json_files(dir.path());

    assert_eq!(files.len(), 1);
    assert!(files.iter().all(|file| file.extension().unwrap() == "json"));

    fs::write(dir.path().join("half-written.tmp"), "{").unwrap();
    cache.purge_expired();

    assert_eq!(json_files(dir.path()).len(), 1);
    assert_eq!(cache.get::<u32>("k").unwrap().value, 2);
}

#[test]
fn clear_removes_memory_and_files() {
    let dir = tempfile::tempdir().unwrap();
    let cache = MetaCache::open(dir.path());

    cache.put("a", &1_u32);
    cache.put("b", &2_u32);
    cache.clear();

    assert!(cache.get::<u32>("a").is_none());
    assert!(json_files(dir.path()).is_empty());
}

struct CountingLibrary {
    calls: AtomicU64,
}

#[async_trait]
impl LibraryBackend for CountingLibrary {
    async fn track(&self, id: &str) -> LibraryResult<Track> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(50)).await;

        if id == "missing" {
            return Err(LibraryError::NotFound);
        }

        Ok(Track {
            uri: format!("spotify:track:{id}"),
            id: Some(id.to_owned()),
            name: id.to_owned(),
            ..Track::default()
        })
    }
}

#[tokio::test]
async fn resolver_fetches_once_then_serves_the_cache() {
    let library = Arc::new(CountingLibrary {
        calls: AtomicU64::new(0),
    });
    let resolver = TrackResolver::new(library.clone(), MetaCache::in_memory());
    let uri = "spotify:track:abc";
    let (a, b) = tokio::join!(resolver.resolve(uri), resolver.resolve(uri));

    assert_eq!(a.unwrap().name, "abc");
    assert_eq!(b.unwrap().name, "abc");
    assert_eq!(library.calls.load(Ordering::SeqCst), 1);
    assert!(resolver.cached(uri).is_some());
    assert!(resolver.resolve(uri).await.is_some());
    assert_eq!(library.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn resolver_skips_non_tracks_and_failures() {
    let library = Arc::new(CountingLibrary {
        calls: AtomicU64::new(0),
    });
    let resolver = TrackResolver::new(library.clone(), MetaCache::in_memory());

    assert!(resolver.resolve("spotify:local:a:b:c:1").await.is_none());
    assert!(resolver.resolve("spotify:episode:e").await.is_none());
    assert!(resolver.resolve("spotify:track:missing").await.is_none());
    assert_eq!(library.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn resolver_runs_at_most_two_lookups_at_once() {
    struct Gauge {
        now: AtomicU64,
        peak: AtomicU64,
    }

    #[async_trait]
    impl LibraryBackend for Gauge {
        async fn track(&self, id: &str) -> LibraryResult<Track> {
            let running = self.now.fetch_add(1, Ordering::SeqCst) + 1;

            self.peak.fetch_max(running, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(30)).await;
            self.now.fetch_sub(1, Ordering::SeqCst);

            Ok(Track {
                id: Some(id.to_owned()),
                ..Track::default()
            })
        }
    }

    let gauge = Arc::new(Gauge {
        now: AtomicU64::new(0),
        peak: AtomicU64::new(0),
    });
    let resolver = TrackResolver::new(gauge.clone(), MetaCache::in_memory());
    let lookups = (0..6).map(|n| {
        let resolver = resolver.clone();

        async move { resolver.resolve(&format!("spotify:track:t{n}")).await }
    });

    futures_util::future::join_all(lookups).await;

    assert_eq!(gauge.peak.load(Ordering::SeqCst), 2);
}
