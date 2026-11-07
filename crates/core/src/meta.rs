//! Temporary metadata cache: in memory plus small JSON files. Nothing here
//! outlives seven days, and it holds metadata only (Terms IV.3.2).

use std::collections::HashMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Semaphore;
use tracing::{debug, warn};

use crate::library::LibraryBackend;
use crate::model::{Track, split_uri};
use crate::playback::now_ms;

pub const FRESH_FOR: Duration = Duration::from_secs(5 * 60);
pub const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// Track lookups for history rows that run at once.
pub const RESOLVE_CONCURRENCY: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Fresh,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cached<T> {
    pub value: T,
    pub freshness: Freshness,
}

#[derive(Clone, Serialize, Deserialize)]
struct Stored {
    key: String,
    stored_at_ms: u64,
    value: Value,
}

type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

pub struct MetaCache {
    dir: Option<PathBuf>,
    memory: Mutex<HashMap<String, Stored>>,
    clock: Clock,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn file_name(key: &str) -> String {
    let hash = key.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    let readable: String = key
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(40)
        .collect();

    format!("{readable}-{hash:016x}.json")
}

impl MetaCache {
    /// Memory only, for demo mode and tests.
    pub fn in_memory() -> Arc<Self> {
        Arc::new(Self {
            dir: None,
            memory: Mutex::new(HashMap::new()),
            clock: Arc::new(now_ms),
        })
    }

    pub fn open(dir: impl Into<PathBuf>) -> Arc<Self> {
        Self::open_with_clock(dir, Arc::new(now_ms))
    }

    pub fn open_with_clock(dir: impl Into<PathBuf>, clock: Clock) -> Arc<Self> {
        let cache = Arc::new(Self {
            dir: Some(dir.into()),
            memory: Mutex::new(HashMap::new()),
            clock,
        });

        cache.purge_expired();

        cache
    }

    fn path(&self, key: &str) -> Option<PathBuf> {
        self.dir.as_ref().map(|dir| dir.join(file_name(key)))
    }

    fn age(&self, stored: &Stored) -> Duration {
        Duration::from_millis((self.clock)().saturating_sub(stored.stored_at_ms))
    }

    fn read_file(&self, key: &str) -> Option<Stored> {
        let path = self.path(key)?;
        let text = fs::read_to_string(&path).ok()?;

        match serde_json::from_str::<Stored>(&text) {
            Ok(stored) if stored.key == key => Some(stored),
            _ => {
                warn!(file = %path.display(), "discarding a corrupt cache file");
                let _ = fs::remove_file(&path);

                None
            }
        }
    }

    fn forget(&self, key: &str) {
        lock(&self.memory).remove(key);

        if let Some(path) = self.path(key) {
            let _ = fs::remove_file(path);
        }
    }

    /// The stored value with its freshness, or `None` if there is none, it has
    /// expired or it cannot be read.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<Cached<T>> {
        let cached = lock(&self.memory).get(key).cloned();
        let stored = match cached {
            Some(stored) => stored,
            None => {
                let stored = self.read_file(key)?;

                lock(&self.memory).insert(key.to_owned(), stored.clone());

                stored
            }
        };
        let age = self.age(&stored);

        if age > MAX_AGE {
            self.forget(key);

            return None;
        }

        let Ok(value) = serde_json::from_value(stored.value) else {
            self.forget(key);

            return None;
        };

        Some(Cached {
            value,
            freshness: if age <= FRESH_FOR {
                Freshness::Fresh
            } else {
                Freshness::Stale
            },
        })
    }

    /// Stores `value` in memory and, atomically, on disk.
    pub fn put<T: Serialize>(&self, key: &str, value: &T) {
        let Ok(value) = serde_json::to_value(value) else {
            return;
        };
        let stored = Stored {
            key: key.to_owned(),
            stored_at_ms: (self.clock)(),
            value,
        };

        if let Some(path) = self.path(key)
            && let Err(error) = write_atomically(&path, &stored)
        {
            warn!(%error, "could not write a cache file");
        }

        lock(&self.memory).insert(key.to_owned(), stored);
    }

    /// Removes files that are past their age limit or leftovers of a crashed write.
    pub fn purge_expired(&self) {
        let Some(dir) = &self.dir else {
            return;
        };
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let keep = path.extension().is_some_and(|ext| ext == "json")
                && fs::read_to_string(&path)
                    .ok()
                    .and_then(|text| serde_json::from_str::<Stored>(&text).ok())
                    .is_some_and(|stored| self.age(&stored) <= MAX_AGE);

            if !keep {
                debug!(file = %path.display(), "removing an expired cache file");
                let _ = fs::remove_file(&path);
            }
        }
    }

    /// Forgets everything, including files.
    pub fn clear(&self) {
        lock(&self.memory).clear();

        let Some(dir) = &self.dir else {
            return;
        };

        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

fn write_atomically(path: &Path, stored: &Stored) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let temp = path.with_extension("tmp");
    let mut file = fs::File::create(&temp)?;

    file.write_all(&serde_json::to_vec(stored)?)?;
    file.sync_all()?;
    drop(file);

    fs::rename(&temp, path)
}

pub fn track_key(id: &str) -> String {
    format!("track:{id}")
}

/// Looks up track metadata for history rows, from the cache first. Fetches
/// are de-duplicated and at most `RESOLVE_CONCURRENCY` run at once.
pub struct TrackResolver {
    library: Arc<dyn LibraryBackend>,
    meta: Arc<MetaCache>,
    permits: Semaphore,
    inflight: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::OnceCell<Option<Track>>>>>,
}

impl TrackResolver {
    pub fn new(library: Arc<dyn LibraryBackend>, meta: Arc<MetaCache>) -> Arc<Self> {
        Arc::new(Self {
            library,
            meta,
            permits: Semaphore::new(RESOLVE_CONCURRENCY),
            inflight: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    /// Cache only, never touches the network.
    pub fn cached(&self, uri: &str) -> Option<Track> {
        let (kind, id) = split_uri(uri)?;

        if kind != "track" {
            return None;
        }

        self.meta.get::<Track>(&track_key(id)).map(|c| c.value)
    }

    pub fn remember(&self, track: &Track) {
        if let Some(id) = &track.id {
            self.meta.put(&track_key(id), track);
        }
    }

    pub async fn resolve(&self, uri: &str) -> Option<Track> {
        if let Some(track) = self.cached(uri) {
            return Some(track);
        }

        let (kind, id) = split_uri(uri)?;

        if kind != "track" {
            return None;
        }

        let cell = {
            let mut inflight = self.inflight.lock().await;

            inflight.entry(uri.to_owned()).or_default().clone()
        };
        let track = cell
            .get_or_init(|| async {
                let _permit = self.permits.acquire().await.ok()?;
                let track = self.library.track(id).await.ok()?;

                self.remember(&track);

                Some(track)
            })
            .await
            .clone();

        self.inflight.lock().await.remove(uri);

        track
    }
}

#[cfg(test)]
mod tests;
