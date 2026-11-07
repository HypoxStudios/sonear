//! Listening history: a plain list of plays in SQLite, kept current from
//! Spotify's recently-played list. Display metadata lives in `MetaCache`.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::{Connection, params};
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::runtime::Handle;
use tokio::sync::{Notify, broadcast};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::config::ConfigHandle;
use crate::events::AppEvent;
use crate::library::{LibraryBackend, LibraryError, LibraryResult};
use crate::meta::TrackResolver;
use crate::model::{Artwork, split_uri};
use crate::playback::now_ms;

pub const SYNC_EVERY: Duration = Duration::from_secs(15 * 60);
pub const SEED_LIMIT: u32 = 50;
/// Spotify lists a play a little after it ends, so a sync triggered by a track
/// change waits. Repeated skips within this time share one sync.
const AFTER_TRACK_DELAY: Duration = if cfg!(test) {
    Duration::from_millis(10)
} else {
    Duration::from_secs(30)
};
/// Distinct contexts the hero tries before giving up.
const HERO_CANDIDATES: usize = 8;
const MONTH_MS: u64 = 30 * 24 * 60 * 60 * 1000;

#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("history database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("history task failed: {0}")]
    Task(String),
    #[error("history file error: {0}")]
    Io(#[from] std::io::Error),
}

impl HistoryError {
    /// The error in the interface language. `Display` stays English for logs.
    pub fn user_message(&self) -> String {
        match self {
            Self::Database(error) => t!("history.database", detail = error.to_string()),
            Self::Task(detail) => t!("history.task", detail = detail),
            Self::Io(error) => t!("history.io", detail = error.to_string()),
        }
        .into_owned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryRow {
    pub played_at_ms: u64,
    pub track_uri: String,
    pub context_uri: Option<String>,
}

/// Blocking SQLite behind `spawn_blocking`, so the UI thread never waits on it.
pub struct HistoryStore {
    path: Option<PathBuf>,
    conn: Arc<Mutex<Option<Connection>>>,
}

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS plays (
    played_at INTEGER NOT NULL,
    track_uri TEXT NOT NULL,
    context_uri TEXT,
    PRIMARY KEY (played_at, track_uri)
) WITHOUT ROWID;";

fn open_connection(path: Option<&PathBuf>) -> Result<Connection, HistoryError> {
    let connection = match path {
        Some(path) => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }

            Connection::open(path)?
        }
        None => Connection::open_in_memory()?,
    };

    connection.execute_batch(SCHEMA)?;

    Ok(connection)
}

impl HistoryStore {
    /// Nothing is created until the first call.
    pub fn open(path: impl Into<PathBuf>) -> Arc<Self> {
        Arc::new(Self {
            path: Some(path.into()),
            conn: Arc::new(Mutex::new(None)),
        })
    }

    pub fn in_memory() -> Arc<Self> {
        Arc::new(Self {
            path: None,
            conn: Arc::new(Mutex::new(None)),
        })
    }

    async fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Connection) -> Result<T, rusqlite::Error> + Send + 'static,
    ) -> Result<T, HistoryError> {
        let conn = self.conn.clone();
        let path = self.path.clone();

        tokio::task::spawn_blocking(move || {
            let mut guard = conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

            if guard.is_none() {
                *guard = Some(open_connection(path.as_ref())?);
            }

            let connection = guard
                .as_ref()
                .ok_or_else(|| HistoryError::Task("the history database is not open".into()))?;

            Ok(work(connection)?)
        })
        .await
        .map_err(|error| HistoryError::Task(error.to_string()))?
    }

    /// Adds plays, ignoring ones already stored. Returns how many were new.
    pub async fn insert(&self, rows: Vec<HistoryRow>) -> Result<usize, HistoryError> {
        self.run(move |connection| {
            let mut added = 0;
            let transaction = connection.unchecked_transaction()?;

            {
                let mut statement = transaction.prepare(
                    "INSERT OR IGNORE INTO plays (played_at, track_uri, context_uri) VALUES (?1, ?2, ?3)",
                )?;

                for row in &rows {
                    added += statement.execute(params![
                        row.played_at_ms as i64,
                        row.track_uri,
                        row.context_uri
                    ])?;
                }
            }

            transaction.commit()?;

            Ok(added)
        })
        .await
    }

    /// Time of the newest stored play, used as the `after` cursor.
    pub async fn latest(&self) -> Result<Option<u64>, HistoryError> {
        self.run(|connection| {
            connection.query_row("SELECT MAX(played_at) FROM plays", [], |row| {
                row.get::<_, Option<i64>>(0)
            })
        })
        .await
        .map(|latest| latest.map(|value| value as u64))
    }

    pub async fn count(&self) -> Result<u64, HistoryError> {
        self.run(|connection| {
            connection.query_row("SELECT COUNT(*) FROM plays", [], |row| row.get::<_, i64>(0))
        })
        .await
        .map(|count| count as u64)
    }

    /// Newest first.
    pub async fn recent(&self, limit: u32, offset: u32) -> Result<Vec<HistoryRow>, HistoryError> {
        self.run(move |connection| {
            let mut statement = connection.prepare(
                "SELECT played_at, track_uri, context_uri FROM plays
                 ORDER BY played_at DESC, track_uri LIMIT ?1 OFFSET ?2",
            )?;
            let rows = statement.query_map(params![limit, offset], |row| {
                Ok(HistoryRow {
                    played_at_ms: row.get::<_, i64>(0)? as u64,
                    track_uri: row.get(1)?,
                    context_uri: row.get(2)?,
                })
            })?;

            rows.collect()
        })
        .await
    }

    /// Deletes plays before `cutoff_ms`. Returns how many went.
    pub async fn prune(&self, cutoff_ms: u64) -> Result<usize, HistoryError> {
        self.run(move |connection| {
            connection.execute("DELETE FROM plays WHERE played_at < ?1", [cutoff_ms as i64])
        })
        .await
    }

    pub async fn clear(&self) -> Result<(), HistoryError> {
        self.run(|connection| connection.execute("DELETE FROM plays", []).map(|_| ()))
            .await
    }

    /// Closes the database and deletes its file. The store reopens empty if used again.
    pub async fn wipe(&self) -> Result<(), HistoryError> {
        let conn = self.conn.clone();
        let path = self.path.clone();

        tokio::task::spawn_blocking(move || {
            *conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;

            if let Some(path) = path {
                for suffix in ["", "-journal", "-wal", "-shm"] {
                    let mut name = path.clone().into_os_string();

                    name.push(suffix);

                    match fs::remove_file(PathBuf::from(name)) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(HistoryError::Io(error)),
                    }
                }
            }

            Ok(())
        })
        .await
        .map_err(|error| HistoryError::Task(error.to_string()))?
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeroKind {
    Playlist,
    Album,
    Artist,
    LikedSongs,
}

/// The "pick up where you left off" card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hero {
    pub kind: HeroKind,
    pub uri: String,
    pub id: String,
    pub name: String,
    pub art: Artwork,
    pub track_count: Option<u32>,
}

pub struct History {
    store: Arc<HistoryStore>,
    library: Arc<dyn LibraryBackend>,
    resolver: Arc<TrackResolver>,
    config: ConfigHandle,
    events: broadcast::Sender<AppEvent>,
    wake: Notify,
}

impl History {
    pub fn new(
        store: Arc<HistoryStore>,
        library: Arc<dyn LibraryBackend>,
        resolver: Arc<TrackResolver>,
        config: ConfigHandle,
        events: broadcast::Sender<AppEvent>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            library,
            resolver,
            config,
            events,
            wake: Notify::new(),
        })
    }

    pub fn store(&self) -> &Arc<HistoryStore> {
        &self.store
    }

    pub fn resolver(&self) -> &Arc<TrackResolver> {
        &self.resolver
    }

    /// Fetches the plays Spotify lists and stores the new ones. Returns how
    /// many were new. Always asks for the whole list: with `after=<newest>`, a
    /// play Spotify lists late, older than the newest stored, would be lost.
    pub async fn sync(&self) -> LibraryResult<usize> {
        let newest = self
            .store
            .latest()
            .await
            .map_err(storage_error)?
            .unwrap_or(0);
        let floor = self.config.get().history_cleared_at_ms;
        let records = self.library.recently_played(SEED_LIMIT).await?;
        let rows = records
            .iter()
            .filter(|record| record.played_at_ms > floor)
            .map(|record| {
                if record.played_at_ms > newest {
                    self.resolver.remember(&record.track);
                }

                HistoryRow {
                    played_at_ms: record.played_at_ms,
                    track_uri: record.track.uri.clone(),
                    context_uri: record.context_uri.clone(),
                }
            })
            .collect();
        let added = self.store.insert(rows).await.map_err(storage_error)?;

        if added > 0 {
            let _ = self.events.send(AppEvent::HistoryChanged);
        }

        Ok(added)
    }

    /// Asks the sync loop to run shortly, because a track just ended.
    pub fn sync_soon(&self) {
        self.wake.notify_one();
    }

    /// Syncs now, then every 15 minutes or shortly after `sync_soon`.
    pub fn spawn_sync_loop(self: &Arc<Self>, runtime: &Handle) -> JoinHandle<()> {
        let history = self.clone();

        runtime.spawn(async move {
            if let Err(error) = history.prune_old().await {
                warn!(%error, "could not prune the history");
            }

            loop {
                match history.sync().await {
                    Ok(added) => debug!(added, "history synced"),
                    Err(error) => warn!(%error, "history sync failed"),
                }

                tokio::select! {
                    () = tokio::time::sleep(SYNC_EVERY) => {}
                    () = history.wake.notified() => tokio::time::sleep(AFTER_TRACK_DELAY).await,
                }
            }
        })
    }

    /// Drops plays older than `history_retention_months`.
    pub async fn prune_old(&self) -> Result<usize, HistoryError> {
        let months = u64::from(self.config.get().history_retention_months);
        let cutoff = now_ms().saturating_sub(months * MONTH_MS);

        self.store.prune(cutoff).await
    }

    pub async fn recent(&self, limit: u32, offset: u32) -> Vec<HistoryRow> {
        self.store
            .recent(limit, offset)
            .await
            .unwrap_or_else(|error| {
                warn!(%error, "could not read the history");

                Vec::new()
            })
    }

    /// Empties the list and keeps it empty: Spotify still has these plays,
    /// and without the floor the next sync would seed them all back.
    pub async fn clear(&self) -> Result<(), HistoryError> {
        self.config
            .update(|config| config.history_cleared_at_ms = now_ms());
        self.store.clear().await?;
        let _ = self.events.send(AppEvent::HistoryChanged);

        Ok(())
    }

    /// The most recent context in the history that still resolves. A 403 or
    /// 404 moves on to the next one; a play without a context uses the track's album.
    pub async fn hero(&self) -> LibraryResult<Option<Hero>> {
        let rows = self.recent(SEED_LIMIT, 0).await;
        let mut tried: Vec<String> = Vec::new();

        for row in rows {
            if tried.len() >= HERO_CANDIDATES {
                break;
            }

            let uri = match row.context_uri {
                Some(uri) => uri,
                None => match self
                    .resolver
                    .resolve(&row.track_uri)
                    .await
                    .and_then(|track| track.album)
                {
                    Some(album) => album.uri,
                    None => continue,
                },
            };

            if tried.contains(&uri) {
                continue;
            }

            tried.push(uri.clone());

            match self.resolve_context(&uri).await {
                Ok(hero) => return Ok(Some(hero)),
                Err(
                    LibraryError::Forbidden | LibraryError::NotFound | LibraryError::Unsupported,
                ) => {}
                Err(error) => return Err(error),
            }
        }

        Ok(None)
    }

    async fn resolve_context(&self, uri: &str) -> LibraryResult<Hero> {
        if uri.ends_with(":collection") {
            let liked = self.library.saved_tracks(0).await?;

            return Ok(Hero {
                kind: HeroKind::LikedSongs,
                uri: uri.to_owned(),
                id: String::new(),
                name: "Liked Songs".to_owned(),
                art: Artwork::default(),
                track_count: liked.total,
            });
        }

        let (kind, id) = split_uri(uri).ok_or(LibraryError::NotFound)?;

        match kind {
            "playlist" => {
                let playlist = self.library.playlist(id).await?;

                Ok(Hero {
                    kind: HeroKind::Playlist,
                    uri: playlist.uri,
                    id: playlist.id,
                    name: playlist.name,
                    art: playlist.art,
                    track_count: playlist.track_count,
                })
            }
            "album" => {
                let album = self.library.album(id).await?;

                Ok(Hero {
                    kind: HeroKind::Album,
                    uri: album.uri,
                    id: album.id,
                    name: album.name,
                    art: album.art,
                    track_count: Some(album.total_tracks),
                })
            }
            "artist" => {
                let artist = self.library.artist(id).await?;

                Ok(Hero {
                    kind: HeroKind::Artist,
                    uri: artist.uri,
                    id: artist.id,
                    name: artist.name,
                    art: artist.art,
                    track_count: None,
                })
            }
            _ => Err(LibraryError::NotFound),
        }
    }
}

fn storage_error(error: HistoryError) -> LibraryError {
    LibraryError::Api(error.to_string())
}

#[cfg(test)]
mod tests;
