//! Collects the tracks of playlists Spotify won't list. Soloist's queue shows
//! what is coming up in the playing playlist, so this saves it to the meta
//! cache, where the playlist page reads it back.

use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tokio::time::{Instant, sleep_until};
use tracing::warn;

use crate::convert::entity_track;
use crate::events::AppEvent;
use crate::meta::MetaCache;
use crate::model::{Track, merge_window};
use crate::player::Player;

/// How long the context and queue must stay put before they're trusted.
/// Soloist sends them separately and in either order, so during a switch the
/// new playlist can appear next to the old one's queue. Saving that would put
/// the wrong tracks in the list for days.
pub const SEEN_SETTLE: Duration = Duration::from_secs(2);

const PLAYLIST_PREFIX: &str = "spotify:playlist:";

/// The meta cache key of a playlist's collected tracks.
pub fn seen_key(playlist_id: &str) -> String {
    format!("seen:playlist:{playlist_id}")
}

struct Watch {
    context: String,
    last: Vec<String>,
    deadline: Option<Instant>,
}

pub struct SeenCollector {
    player: Player,
    meta: Arc<MetaCache>,
    events: broadcast::Sender<AppEvent>,
}

impl SeenCollector {
    pub fn spawn(
        player: Player,
        meta: Arc<MetaCache>,
        events: broadcast::Sender<AppEvent>,
        runtime: &Handle,
    ) {
        let feed = events.subscribe();
        let collector = Self {
            player,
            meta,
            events,
        };

        runtime.spawn(collector.run(feed));
    }

    async fn run(self, mut feed: broadcast::Receiver<AppEvent>) {
        let mut watch: Option<Watch> = None;

        loop {
            let deadline = watch.as_ref().and_then(|watch| watch.deadline);

            tokio::select! {
                event = feed.recv() => match event {
                    Ok(AppEvent::Playback(_)) => self.observe(&mut watch),
                    Err(broadcast::error::RecvError::Lagged(_)) => self.observe(&mut watch),
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                () = sleep_until(deadline.unwrap_or_else(Instant::now)), if deadline.is_some() => {
                    self.settled(&mut watch).await;
                }
            }
        }
    }

    /// Notices a change in what the queue shows of the playing playlist and
    /// restarts the wait for it to settle. Shuffled playback is skipped,
    /// because the queue is then in shuffled order, not the playlist's.
    fn observe(&self, watch: &mut Option<Watch>) {
        let model = self.player.snapshot();
        let Some(context) = model
            .context
            .as_ref()
            .map(|context| context.uri.as_str())
            .filter(|uri| uri.starts_with(PLAYLIST_PREFIX))
            .filter(|_| !model.options.shuffle)
        else {
            *watch = None;

            return;
        };
        let Some(sequence) = model.context_sequence(context) else {
            *watch = None;

            return;
        };
        let last: Vec<String> = sequence.iter().map(|entity| entity.uri.clone()).collect();

        if let Some(known) = watch
            && known.context == context
            && known.last == last
        {
            return;
        }

        *watch = Some(Watch {
            context: context.to_owned(),
            last,
            deadline: Some(Instant::now() + SEEN_SETTLE),
        });
    }

    async fn settled(&self, watch: &mut Option<Watch>) {
        let Some(current) = watch.as_mut() else {
            return;
        };

        current.deadline = None;

        let model = self.player.snapshot();

        if model.options.shuffle {
            *watch = None;

            return;
        }

        let Some(sequence) = model.context_sequence(&current.context) else {
            *watch = None;

            return;
        };
        let tracks: Vec<Track> = sequence.into_iter().map(entity_track).collect();
        let Some(id) = current.context.strip_prefix(PLAYLIST_PREFIX) else {
            return;
        };
        let key = seen_key(id);
        let id = id.to_owned();
        let meta = self.meta.clone();
        let changed = tokio::task::spawn_blocking(move || {
            let mut seen = meta
                .get::<Vec<Track>>(&key)
                .map(|cached| cached.value)
                .unwrap_or_default();
            let changed = merge_window(&mut seen, &tracks);

            if changed {
                meta.put(&key, &seen);
            }

            changed
        })
        .await;

        match changed {
            Ok(true) => {
                let _ = self.events.send(AppEvent::SeenTracks { playlist_id: id });
            }
            Ok(false) => {}
            Err(error) => warn!(%error, "could not save the collected tracks"),
        }
    }
}

#[cfg(test)]
mod tests;
