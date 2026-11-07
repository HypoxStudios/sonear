//! Which tracks and albums the user has saved, cached, with optimistic toggling.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

use crate::events::AppEvent;
use crate::library::{LibraryBackend, LibraryError};

/// URIs per `contains` call.
pub const CHUNK: usize = 40;

#[derive(Default)]
struct State {
    known: HashMap<String, bool>,
    pending: HashSet<String>,
    writing: HashSet<String>,
}

pub struct LikeStore {
    library: Arc<dyn LibraryBackend>,
    state: Mutex<State>,
    events: broadcast::Sender<AppEvent>,
}

fn lock(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn can_save(uri: &str) -> bool {
    uri.starts_with("spotify:track:") || uri.starts_with("spotify:album:")
}

impl LikeStore {
    pub fn new(library: Arc<dyn LibraryBackend>, events: broadcast::Sender<AppEvent>) -> Arc<Self> {
        Arc::new(Self {
            library,
            state: Mutex::new(State::default()),
            events,
        })
    }

    /// `None` until checked.
    pub fn is_liked(&self, uri: &str) -> Option<bool> {
        lock(&self.state).known.get(uri).copied()
    }

    /// Records answers learned elsewhere, such as every row of Liked Songs being liked.
    pub fn mark(&self, uris: &[String], liked: bool) {
        let mut state = lock(&self.state);

        for uri in uris {
            state.known.insert(uri.clone(), liked);
        }

        drop(state);
        let _ = self.events.send(AppEvent::Likes);
    }

    /// Checks the URIs that are not known yet, in chunks, once at a time per
    /// URI. Posts an event only when it learned something, so a view that
    /// calls this from its render can't loop on the repaint.
    pub async fn ensure(&self, uris: &[String]) {
        let wanted: Vec<String> = {
            let mut state = lock(&self.state);
            let wanted: Vec<String> = uris
                .iter()
                .filter(|uri| can_save(uri))
                .filter(|uri| !state.known.contains_key(*uri) && !state.pending.contains(*uri))
                .cloned()
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();

            state.pending.extend(wanted.iter().cloned());

            wanted
        };

        let mut learned = false;

        for chunk in wanted.chunks(CHUNK) {
            let answer = self.library.contains(chunk).await;
            let mut state = lock(&self.state);

            for uri in chunk {
                state.pending.remove(uri);
            }

            if let Ok(answers) = answer {
                learned = true;

                for (uri, liked) in chunk.iter().zip(answers) {
                    state.known.entry(uri.clone()).or_insert(liked);
                }
            }
        }

        if learned {
            let _ = self.events.send(AppEvent::Likes);
        }
    }

    /// Flips the heart at once, then asks Spotify. A failure puts it back
    /// and posts a notice. Returns the state after the toggle. A second click
    /// while the first is still on its way is ignored, because the two
    /// requests could land in either order and leave the heart wrong.
    pub async fn toggle(&self, uri: &str) -> Result<bool, LibraryError> {
        if !can_save(uri) {
            return Err(LibraryError::Unsupported);
        }

        let before = {
            let mut state = lock(&self.state);
            let before = state.known.get(uri).copied().unwrap_or(false);

            if !state.writing.insert(uri.to_owned()) {
                return Ok(before);
            }

            state.known.insert(uri.to_owned(), !before);

            before
        };
        let after = !before;
        let uris = [uri.to_owned()];

        let _ = self.events.send(AppEvent::Likes);

        let result = if after {
            self.library.save(&uris).await
        } else {
            self.library.remove(&uris).await
        };

        lock(&self.state).writing.remove(uri);

        match result {
            Ok(()) => Ok(after),
            Err(error) => {
                lock(&self.state).known.insert(uri.to_owned(), before);
                let _ = self.events.send(AppEvent::Likes);
                let _ = self.events.send(AppEvent::Notice(format!(
                    "Couldn't update your saved music. {error}"
                )));

                Err(error)
            }
        }
    }

    pub fn clear(&self) {
        *lock(&self.state) = State::default();
        let _ = self.events.send(AppEvent::Likes);
    }
}

#[cfg(test)]
mod tests;
