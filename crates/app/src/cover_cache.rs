//! Bounded caches for cover art: files on disk and decoded images in memory.
//! Covers are the only Spotify content kept besides metadata (Terms IV.3.2).

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::{App, ImageCache, ImageCacheError, ImageCacheItem, RenderImage, Resource, Window};
use tracing::{debug, warn};

pub const MAX_BYTES: u64 = 200 * 1024 * 1024;
pub const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

struct Entry {
    size: u64,
    written_ms: u64,
}

#[derive(Default)]
struct Index {
    entries: HashMap<String, Entry>,
    total: u64,
}

type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

fn system_clock() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

fn file_name(url: &str) -> String {
    let hash = url.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    });

    format!("{hash:016x}.img")
}

/// Whether `host` serves Spotify cover art.
pub fn is_cover_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();

    ["scdn.co", "spotifycdn.com"]
        .iter()
        .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
}

/// Cover files on disk. Holds at most `max_bytes`, evicting the oldest
/// first, and drops anything older than `max_age`.
pub struct DiskCache {
    dir: PathBuf,
    max_bytes: u64,
    max_age: Duration,
    clock: Clock,
    index: Mutex<Index>,
}

fn lock(index: &Mutex<Index>) -> std::sync::MutexGuard<'_, Index> {
    index
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl DiskCache {
    pub fn open(dir: impl Into<PathBuf>, max_bytes: u64, max_age: Duration) -> Arc<Self> {
        Self::open_with_clock(dir, max_bytes, max_age, Arc::new(system_clock))
    }

    pub fn open_with_clock(
        dir: impl Into<PathBuf>,
        max_bytes: u64,
        max_age: Duration,
        clock: Clock,
    ) -> Arc<Self> {
        let cache = Arc::new(Self {
            dir: dir.into(),
            max_bytes,
            max_age,
            clock,
            index: Mutex::new(Index::default()),
        });

        cache.rescan();

        cache
    }

    /// Rebuilds the index from the files, deleting stale ones and leftovers.
    pub fn rescan(&self) {
        let mut index = Index::default();

        if let Ok(entries) = fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                let written_ms = meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |elapsed| elapsed.as_millis() as u64);
                let too_old =
                    (self.clock)().saturating_sub(written_ms) > self.max_age.as_millis() as u64;

                if !name.ends_with(".img") || too_old {
                    let _ = fs::remove_file(&path);

                    continue;
                }

                index.total += meta.len();
                index.entries.insert(
                    name,
                    Entry {
                        size: meta.len(),
                        written_ms,
                    },
                );
            }
        }

        *lock(&self.index) = index;
        self.evict();
    }

    #[cfg(test)]
    pub fn total_bytes(&self) -> u64 {
        lock(&self.index).total
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        lock(&self.index).entries.len()
    }

    pub fn get(&self, url: &str) -> Option<Vec<u8>> {
        let name = file_name(url);
        let age_ok = {
            let index = lock(&self.index);
            let entry = index.entries.get(&name)?;

            (self.clock)().saturating_sub(entry.written_ms) <= self.max_age.as_millis() as u64
        };

        if !age_ok {
            self.remove(&name);

            return None;
        }

        match fs::read(self.dir.join(&name)) {
            Ok(bytes) => Some(bytes),
            Err(_) => {
                self.remove(&name);

                None
            }
        }
    }

    pub fn put(&self, url: &str, bytes: &[u8]) {
        if bytes.is_empty() || bytes.len() as u64 > self.max_bytes {
            return;
        }

        let name = file_name(url);

        if let Err(error) = self.write(&name, bytes) {
            warn!(%error, "could not cache a cover");

            return;
        }

        {
            let mut index = lock(&self.index);

            if let Some(old) = index.entries.remove(&name) {
                index.total -= old.size;
            }

            index.total += bytes.len() as u64;
            index.entries.insert(
                name,
                Entry {
                    size: bytes.len() as u64,
                    written_ms: (self.clock)(),
                },
            );
        }

        self.evict();
    }

    fn write(&self, name: &str, bytes: &[u8]) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;

        let target = self.dir.join(name);
        let temp = self.dir.join(format!("{name}.tmp"));
        let mut file = fs::File::create(&temp)?;

        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);

        fs::rename(&temp, target)
    }

    fn remove(&self, name: &str) {
        let _ = fs::remove_file(self.dir.join(name));

        let mut index = lock(&self.index);

        if let Some(entry) = index.entries.remove(name) {
            index.total -= entry.size;
        }
    }

    fn evict(&self) {
        loop {
            let oldest = {
                let index = lock(&self.index);

                if index.total <= self.max_bytes {
                    return;
                }

                index
                    .entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.written_ms)
                    .map(|(name, _)| name.clone())
            };

            let Some(name) = oldest else {
                return;
            };

            debug!(%name, "evicting a cover");
            self.remove(&name);
        }
    }

    /// Forgets the index. Call after the directory was deleted.
    pub fn reset(&self) {
        *lock(&self.index) = Index::default();
    }
}

/// Keeps at most `capacity` decoded images, dropping the least recently used.
pub struct LruImageCache {
    items: HashMap<u64, ImageCacheItem>,
    order: VecDeque<u64>,
    capacity: usize,
}

impl LruImageCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            items: HashMap::new(),
            order: VecDeque::new(),
            capacity,
        }
    }
}

fn key_of(resource: &Resource) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();

    resource.hash(&mut hasher);

    hasher.finish()
}

impl ImageCache for LruImageCache {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let key = key_of(resource);

        self.items
            .entry(key)
            .or_insert_with(|| ImageCacheItem::new(resource, cx));

        self.order.retain(|existing| *existing != key);
        self.order.push_back(key);

        while self.items.len() > self.capacity {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };

            if let Some(item) = self.items.remove(&oldest)
                && let Some(Ok(image)) = item.get()
            {
                cx.drop_image(image, Some(window));
            }
        }

        self.items.get(&key)?.use_image(window)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    fn cache(dir: &Path, max_bytes: u64, now: &Arc<AtomicU64>) -> Arc<DiskCache> {
        let now = now.clone();

        DiskCache::open_with_clock(
            dir,
            max_bytes,
            Duration::from_secs(30 * 24 * 60 * 60),
            Arc::new(move || now.load(Ordering::SeqCst)),
        )
    }

    #[test]
    fn only_spotify_cover_hosts_are_cached() {
        assert!(is_cover_host("i.scdn.co"));
        assert!(is_cover_host("mosaic.scdn.co"));
        assert!(is_cover_host("image-cdn-ak.spotifycdn.com"));
        assert!(!is_cover_host("scdn.co.evil.com"));
        assert!(!is_cover_host("example.com"));
        assert!(!is_cover_host("notscdn.co"));
    }

    #[test]
    fn stores_and_returns_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let now = Arc::new(AtomicU64::new(1_000));
        let cache = cache(dir.path(), 1000, &now);

        assert!(cache.get("https://i.scdn.co/a").is_none());

        cache.put("https://i.scdn.co/a", b"hello");

        assert_eq!(cache.get("https://i.scdn.co/a").unwrap(), b"hello");
        assert_eq!(cache.total_bytes(), 5);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn the_oldest_files_go_first_when_over_the_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let now = Arc::new(AtomicU64::new(1_000));
        let cache = cache(dir.path(), 100, &now);

        for (n, name) in ["a", "b", "c", "d"].iter().enumerate() {
            now.store(1_000 + n as u64, Ordering::SeqCst);
            cache.put(name, &[7u8; 40]);
        }

        assert!(cache.total_bytes() <= 100);
        assert_eq!(cache.len(), 2);
        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_none());
        assert!(cache.get("c").is_some());
        assert!(cache.get("d").is_some());
    }

    #[test]
    fn files_older_than_thirty_days_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let now = Arc::new(AtomicU64::new(1_000));
        let cache = cache(dir.path(), 1000, &now);

        cache.put("a", b"x");
        now.fetch_add(31 * 24 * 60 * 60 * 1000, Ordering::SeqCst);

        assert!(cache.get("a").is_none());
        assert_eq!(cache.len(), 0);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn startup_scan_removes_old_and_foreign_files_and_trims_to_size() {
        let dir = tempfile::tempdir().unwrap();
        let day = 24 * 60 * 60;
        let write = |name: &str, bytes: usize, age_days: u64| {
            let path = dir.path().join(name);

            fs::write(&path, vec![1u8; bytes]).unwrap();

            let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
            let modified = SystemTime::now() - Duration::from_secs(age_days * day);

            file.set_modified(modified).unwrap();
        };

        write("fresh1.img", 60, 1);
        write("fresh2.img", 60, 2);
        write("ancient.img", 10, 40);
        write("half.img.tmp", 10, 0);

        let now = Arc::new(AtomicU64::new(system_clock()));
        let cache = cache(dir.path(), 100, &now);

        assert_eq!(cache.len(), 1);
        assert!(cache.total_bytes() <= 100);
        assert!(!dir.path().join("ancient.img").exists());
        assert!(!dir.path().join("half.img.tmp").exists());
        assert!(!dir.path().join("fresh2.img").exists());
        assert!(dir.path().join("fresh1.img").exists());
    }

    #[test]
    fn a_deleted_file_is_a_miss_and_leaves_the_index_clean() {
        let dir = tempfile::tempdir().unwrap();
        let now = Arc::new(AtomicU64::new(1));
        let cache = cache(dir.path(), 1000, &now);

        cache.put("a", b"abc");
        fs::remove_dir_all(dir.path()).unwrap();

        assert!(cache.get("a").is_none());
        assert_eq!(cache.total_bytes(), 0);

        cache.put("b", b"def");

        assert_eq!(cache.get("b").unwrap(), b"def");
    }

    #[test]
    fn oversized_and_empty_covers_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let now = Arc::new(AtomicU64::new(1));
        let cache = cache(dir.path(), 10, &now);

        cache.put("a", &[0u8; 11]);
        cache.put("b", b"");

        assert_eq!(cache.len(), 0);
    }
}
