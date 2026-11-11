//! The YouTube side has no account, so saved music and recent plays are
//! plain files on this computer.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::model::{Album, PlayRecord, Track};
use crate::playback::now_ms;

pub const MAX_PLAYS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SavedTrack {
    track: Track,
    saved_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SavedAlbum {
    album: Album,
    saved_at_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    tracks: Vec<SavedTrack>,
    albums: Vec<SavedAlbum>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Play {
    played_at_ms: u64,
    track: Track,
    context_uri: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Plays {
    plays: Vec<Play>,
}

enum Job {
    Write(PathBuf, String),
    Remove(PathBuf),
    Flush(mpsc::Sender<()>),
}

#[derive(Default)]
struct State {
    saved: Saved,
    plays: Plays,
}

pub struct LocalLibrary {
    state: Mutex<State>,
    files: Option<(PathBuf, PathBuf)>,
    writer: Mutex<Option<mpsc::Sender<Job>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Reads `path`. A file that cannot be parsed is renamed to `<name>.bak` so
/// the user's data is not lost, and the result is empty.
fn load<T: Default + for<'de> Deserialize<'de>>(path: &Path) -> T {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return T::default(),
        Err(error) => {
            warn!(%error, "could not read a YouTube library file");

            return T::default();
        }
    };

    match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(error) => {
            warn!(%error, "a YouTube library file is corrupt, keeping a backup");

            let mut backup = path.as_os_str().to_owned();

            backup.push(".bak");

            if let Err(error) = fs::rename(path, PathBuf::from(backup)) {
                warn!(%error, "could not back up the corrupt file");
            }

            T::default()
        }
    }
}

fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let temp = path.with_extension("json.tmp");
    let mut file = fs::File::create(&temp)?;

    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    drop(file);

    fs::rename(&temp, path)
}

fn run_writer(jobs: mpsc::Receiver<Job>) {
    for job in jobs {
        match job {
            Job::Write(path, text) => {
                if let Err(error) = write_atomically(&path, &text) {
                    warn!(%error, "could not save a YouTube library file");
                }
            }
            Job::Remove(path) => {
                if let Err(error) = fs::remove_file(&path)
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    warn!(%error, "could not delete a YouTube library file");
                }
            }
            Job::Flush(done) => {
                let _ = done.send(());
            }
        }
    }
}

impl LocalLibrary {
    pub fn new(library_file: PathBuf, plays_file: PathBuf) -> Arc<Self> {
        let state = State {
            saved: load(&library_file),
            plays: load(&plays_file),
        };
        let (sender, receiver) = mpsc::channel();

        std::thread::spawn(move || run_writer(receiver));

        Arc::new(Self {
            state: Mutex::new(state),
            files: Some((library_file, plays_file)),
            writer: Mutex::new(Some(sender)),
        })
    }

    pub fn in_memory() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
            files: None,
            writer: Mutex::new(None),
        })
    }

    fn send(&self, job: Job) {
        if let Some(writer) = lock(&self.writer).as_ref() {
            let _ = writer.send(job);
        }
    }

    fn persist_saved(&self, saved: &Saved) {
        let Some((path, _)) = &self.files else {
            return;
        };

        match serde_json::to_string(saved) {
            Ok(text) => self.send(Job::Write(path.clone(), text)),
            Err(error) => warn!(%error, "could not encode the saved music"),
        }
    }

    fn persist_plays(&self, plays: &Plays) {
        let Some((_, path)) = &self.files else {
            return;
        };

        match serde_json::to_string(plays) {
            Ok(text) => self.send(Job::Write(path.clone(), text)),
            Err(error) => warn!(%error, "could not encode the play history"),
        }
    }

    /// Waits until everything changed so far is on disk.
    pub fn flush(&self) {
        let (done, wait) = mpsc::channel();

        self.send(Job::Flush(done));
        let _ = wait.recv();
    }

    pub fn contains(&self, uri: &str) -> bool {
        let state = lock(&self.state);

        state
            .saved
            .tracks
            .iter()
            .any(|saved| saved.track.uri == uri)
            || state
                .saved
                .albums
                .iter()
                .any(|saved| saved.album.uri == uri)
    }

    pub fn save_track(&self, track: Track) {
        let mut state = lock(&self.state);

        if state
            .saved
            .tracks
            .iter()
            .any(|saved| saved.track.uri == track.uri)
        {
            return;
        }

        state.saved.tracks.insert(
            0,
            SavedTrack {
                track,
                saved_at_ms: now_ms(),
            },
        );
        self.persist_saved(&state.saved);
    }

    pub fn save_album(&self, album: Album) {
        let mut state = lock(&self.state);

        if state
            .saved
            .albums
            .iter()
            .any(|saved| saved.album.uri == album.uri)
        {
            return;
        }

        state.saved.albums.insert(
            0,
            SavedAlbum {
                album,
                saved_at_ms: now_ms(),
            },
        );
        self.persist_saved(&state.saved);
    }

    pub fn remove(&self, uri: &str) {
        let mut state = lock(&self.state);
        let before = state.saved.tracks.len() + state.saved.albums.len();

        state.saved.tracks.retain(|saved| saved.track.uri != uri);
        state.saved.albums.retain(|saved| saved.album.uri != uri);

        if state.saved.tracks.len() + state.saved.albums.len() != before {
            self.persist_saved(&state.saved);
        }
    }

    /// Saved tracks, newest first.
    pub fn tracks(&self) -> Vec<Track> {
        lock(&self.state)
            .saved
            .tracks
            .iter()
            .map(|saved| saved.track.clone())
            .collect()
    }

    /// Saved albums, newest first.
    pub fn albums(&self) -> Vec<Album> {
        lock(&self.state)
            .saved
            .albums
            .iter()
            .map(|saved| saved.album.clone())
            .collect()
    }

    pub fn record_play(&self, track: Track, context_uri: Option<String>, now_ms: u64) {
        let mut state = lock(&self.state);

        state.plays.plays.insert(
            0,
            Play {
                played_at_ms: now_ms,
                track,
                context_uri,
            },
        );
        state.plays.plays.truncate(MAX_PLAYS);
        self.persist_plays(&state.plays);
    }

    /// The most recent plays, newest first.
    pub fn plays(&self, limit: u32) -> Vec<PlayRecord> {
        lock(&self.state)
            .plays
            .plays
            .iter()
            .take(limit as usize)
            .map(|play| PlayRecord {
                played_at_ms: play.played_at_ms,
                track: play.track.clone(),
                context_uri: play.context_uri.clone(),
            })
            .collect()
    }

    /// Forgets everything, on disk too.
    pub fn wipe(&self) {
        let mut state = lock(&self.state);

        *state = State::default();

        if let Some((library, plays)) = &self.files {
            self.send(Job::Remove(library.clone()));
            self.send(Job::Remove(plays.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str) -> Track {
        Track {
            uri: format!("ytm:track:{id}"),
            id: Some(id.to_owned()),
            name: id.to_owned(),
            playable: true,
            ..Track::default()
        }
    }

    fn album(id: &str) -> Album {
        Album {
            id: id.to_owned(),
            uri: format!("ytm:album:{id}"),
            name: id.to_owned(),
            ..Album::default()
        }
    }

    fn files(dir: &Path) -> (PathBuf, PathBuf) {
        (dir.join("library.json"), dir.join("plays.json"))
    }

    #[test]
    fn saved_music_is_newest_first_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let (library, plays) = files(dir.path());
        let local = LocalLibrary::new(library.clone(), plays.clone());

        local.save_track(track("a"));
        local.save_track(track("b"));
        local.save_track(track("a"));
        local.save_album(album("MPREb_1"));
        local.flush();

        assert_eq!(
            local
                .tracks()
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert!(local.contains("ytm:track:a") && local.contains("ytm:album:MPREb_1"));
        assert!(!local.contains("ytm:track:z"));

        let again = LocalLibrary::new(library, plays);

        assert_eq!(again.tracks(), local.tracks());
        assert_eq!(again.albums(), local.albums());
    }

    #[test]
    fn removing_forgets_a_track_or_an_album() {
        let local = LocalLibrary::in_memory();

        local.save_track(track("a"));
        local.save_album(album("MPREb_1"));
        local.remove("ytm:track:a");
        local.remove("ytm:album:MPREb_1");
        local.remove("ytm:track:never");

        assert!(local.tracks().is_empty() && local.albums().is_empty());
    }

    #[test]
    fn plays_are_newest_first_and_capped() {
        let dir = tempfile::tempdir().unwrap();
        let (library, plays) = files(dir.path());
        let local = LocalLibrary::new(library.clone(), plays.clone());

        for n in 0..(MAX_PLAYS as u64 + 20) {
            local.record_play(track(&format!("t{n}")), Some("ytm:album:x".into()), n);
        }

        local.flush();

        let recent = local.plays(3);

        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].played_at_ms, MAX_PLAYS as u64 + 19);
        assert_eq!(recent[0].context_uri.as_deref(), Some("ytm:album:x"));
        assert_eq!(local.plays(1000).len(), MAX_PLAYS);
        assert_eq!(
            LocalLibrary::new(library, plays).plays(1000).len(),
            MAX_PLAYS
        );
    }

    #[test]
    fn a_corrupt_file_is_backed_up_and_read_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let (library, plays) = files(dir.path());

        fs::write(&library, "{ nope").unwrap();
        fs::write(&plays, "[1,").unwrap();

        let local = LocalLibrary::new(library.clone(), plays.clone());

        assert!(local.tracks().is_empty() && local.plays(10).is_empty());
        assert!(!library.exists() && !plays.exists());
        assert_eq!(
            fs::read_to_string(dir.path().join("library.json.bak")).unwrap(),
            "{ nope"
        );
        assert!(dir.path().join("plays.json.bak").exists());

        local.save_track(track("a"));
        local.flush();

        assert!(library.exists());
    }

    #[test]
    fn a_wipe_deletes_the_files() {
        let dir = tempfile::tempdir().unwrap();
        let (library, plays) = files(dir.path());
        let local = LocalLibrary::new(library.clone(), plays.clone());

        local.save_track(track("a"));
        local.record_play(track("a"), None, 1);
        local.flush();

        assert!(library.exists() && plays.exists());

        local.wipe();
        local.flush();

        assert!(!library.exists() && !plays.exists());
        assert!(local.tracks().is_empty() && local.plays(5).is_empty());
    }
}
