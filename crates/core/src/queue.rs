//! The user's play queue. Soloist's own queue is append-only, so Sonear keeps
//! the real one and hands Soloist a single track at a time, late enough that
//! the rest stay editable.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sonear_soloist::protocol::PlayStatus;
use tokio::runtime::Handle;
use tokio::sync::{Notify, broadcast};
use tracing::{debug, warn};

use crate::events::AppEvent;
use crate::playback::{PlaybackModel, now_ms};
use crate::player::Player;

/// How long before the end of a track the next queued one is handed to Soloist.
pub const HANDOFF_MARGIN_MS: u64 = 25_000;
/// How long Soloist gets to show a handed-off track in its queue.
pub const CONFIRM_TIMEOUT_MS: u64 = 10_000;
/// How long a confirmed track may be missing from Soloist's queue before it counts as lost.
pub const LOST_GRACE_MS: u64 = 3_000;
pub const QUEUE_LIMIT: usize = 500;

const MIN_WAKE_MS: u64 = 50;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueSource {
    #[default]
    User,
    Dj,
    Assistant,
}

/// A queued track. The id only identifies it inside this process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedTrack {
    pub id: u64,
    pub uri: String,
    pub source: QueueSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueCommand {
    AddToQueue(String),
    SkipNext,
    PlayNow(String),
}

#[derive(Debug, Clone)]
struct Handoff {
    entry: QueuedTrack,
    sent_at_ms: u64,
    item_at_send: Option<String>,
    confirmed: bool,
    skip_on_confirm: bool,
    missing_since_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueueSnapshot {
    /// The track already handed to Soloist, which can't be pulled back.
    pub locked: Option<QueuedTrack>,
    /// Whether Soloist's queue has shown the locked track yet.
    pub locked_confirmed: bool,
    pub entries: Vec<QueuedTrack>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedEntry {
    pub uri: String,
    #[serde(default)]
    pub source: QueueSource,
}

/// What is written to disk: URIs only, never metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedQueue {
    #[serde(default)]
    pub entries: Vec<SavedEntry>,
}

/// The queue rules, with no IO and the clock passed in.
#[derive(Debug, Default)]
pub struct QueueState {
    entries: Vec<QueuedTrack>,
    handoff: Option<Handoff>,
    next_id: u64,
    dirty: bool,
}

fn queued_uri(model: &PlaybackModel) -> impl Iterator<Item = &str> {
    model
        .upcoming
        .iter()
        .filter(|entry| entry.source == "queue")
        .map(|entry| entry.item.uri.as_str())
}

/// A disconnect reset leaves no item and no upcoming entries, and a device
/// move is not a loss, so neither says anything about the hand-off.
fn trustworthy(model: &PlaybackModel) -> bool {
    model.is_active && (model.item.is_some() || !model.upcoming.is_empty())
}

impl QueueState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_saved(saved: SavedQueue) -> Self {
        let mut state = Self::new();

        for entry in saved.entries.into_iter().take(QUEUE_LIMIT) {
            state.push(entry.uri, entry.source);
        }

        state.dirty = false;

        state
    }

    /// The handed-off track first, then the editable entries.
    pub fn to_saved(&self) -> SavedQueue {
        let entries = self
            .handoff
            .iter()
            .map(|handoff| &handoff.entry)
            .chain(&self.entries)
            .map(|entry| SavedEntry {
                uri: entry.uri.clone(),
                source: entry.source,
            })
            .collect();

        SavedQueue { entries }
    }

    pub fn snapshot(&self) -> QueueSnapshot {
        QueueSnapshot {
            locked: self.handoff.as_ref().map(|handoff| handoff.entry.clone()),
            locked_confirmed: self
                .handoff
                .as_ref()
                .is_some_and(|handoff| handoff.confirmed),
            entries: self.entries.clone(),
        }
    }

    /// Whether anything changed since the last call.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    fn push(&mut self, uri: String, source: QueueSource) {
        let id = self.next_id;

        self.next_id += 1;
        self.entries.push(QueuedTrack { id, uri, source });
    }

    /// Returns false when the queue is full.
    pub fn add(&mut self, uri: String, source: QueueSource) -> bool {
        if self.entries.len() >= QUEUE_LIMIT {
            return false;
        }

        self.push(uri, source);
        self.dirty = true;

        true
    }

    pub fn remove(&mut self, id: u64) {
        let before = self.entries.len();

        self.entries.retain(|entry| entry.id != id);
        self.dirty |= self.entries.len() != before;
    }

    /// Moves an entry in front of `before`, or to the end. Works on ids so it
    /// survives a hand-off that happened in the meantime.
    pub fn move_before(&mut self, id: u64, before: Option<u64>) {
        let Some(from) = self.entries.iter().position(|entry| entry.id == id) else {
            return;
        };

        if before == Some(id) {
            return;
        }

        let entry = self.entries.remove(from);
        let to = before
            .and_then(|before| self.entries.iter().position(|entry| entry.id == before))
            .unwrap_or(self.entries.len());

        self.entries.insert(to, entry);
        self.dirty = true;
    }

    /// Drops the editable entries. The handed-off track stays.
    pub fn clear(&mut self) {
        self.dirty |= !self.entries.is_empty();
        self.entries.clear();
    }

    /// Forgets everything, including the handed-off track.
    pub fn reset(&mut self) {
        self.entries.clear();
        self.handoff = None;
        self.dirty = true;
    }

    fn start_handoff(
        &mut self,
        entry: QueuedTrack,
        model: &PlaybackModel,
        now_ms: u64,
        skip_on_confirm: bool,
    ) -> QueueCommand {
        let command = QueueCommand::AddToQueue(entry.uri.clone());

        self.handoff = Some(Handoff {
            entry,
            sent_at_ms: now_ms,
            item_at_send: model.item.as_ref().map(|item| item.uri.clone()),
            confirmed: false,
            skip_on_confirm,
            missing_since_ms: None,
        });
        self.dirty = true;

        command
    }

    fn requeue(&mut self, handoff: Handoff) {
        self.entries.insert(0, handoff.entry);
        self.dirty = true;
    }

    /// Folds the latest playback state in and returns what to send to Soloist.
    pub fn observe(&mut self, model: &PlaybackModel, now_ms: u64) -> Vec<QueueCommand> {
        let mut commands = Vec::new();

        self.observe_handoff(model, now_ms, &mut commands);
        self.adopt(model);

        if self.handoff.is_none() && self.handoff_due(model, now_ms) {
            let entry = self.entries.remove(0);

            commands.push(self.start_handoff(entry, model, now_ms, false));
        }

        commands
    }

    fn observe_handoff(
        &mut self,
        model: &PlaybackModel,
        now_ms: u64,
        commands: &mut Vec<QueueCommand>,
    ) {
        let Some(mut handoff) = self.handoff.take() else {
            return;
        };

        if !trustworthy(model) {
            self.handoff = Some(handoff);

            return;
        }

        let uri = handoff.entry.uri.as_str();
        let in_queue = queued_uri(model).any(|queued| queued == uri);
        let playing_it = model.item.as_ref().is_some_and(|item| item.uri == uri);
        let started = playing_it
            && !in_queue
            && (handoff.confirmed || handoff.item_at_send.as_deref() != Some(uri));

        if started {
            self.dirty = true;

            return;
        }

        if in_queue {
            self.dirty |= !handoff.confirmed;
            handoff.confirmed = true;
            handoff.missing_since_ms = None;

            if std::mem::take(&mut handoff.skip_on_confirm) {
                commands.push(QueueCommand::SkipNext);
            }
        } else if handoff.confirmed {
            let since = *handoff.missing_since_ms.get_or_insert(now_ms);

            if now_ms.saturating_sub(since) >= LOST_GRACE_MS {
                self.requeue(handoff);

                return;
            }
        } else if now_ms.saturating_sub(handoff.sent_at_ms) >= CONFIRM_TIMEOUT_MS {
            if handoff.skip_on_confirm {
                commands.push(QueueCommand::SkipNext);
            }

            self.requeue(handoff);

            return;
        }

        self.handoff = Some(handoff);
    }

    /// Takes over a track Soloist already holds for us: after a restart, or
    /// when a confirmation shows up after the timeout. It is never sent twice.
    fn adopt(&mut self, model: &PlaybackModel) {
        if self.handoff.is_some() {
            return;
        }

        let Some(first) = queued_uri(model).next() else {
            return;
        };

        if self.entries.first().is_none_or(|entry| entry.uri != first) {
            return;
        }

        let entry = self.entries.remove(0);

        self.handoff = Some(Handoff {
            entry,
            sent_at_ms: 0,
            item_at_send: model.item.as_ref().map(|item| item.uri.clone()),
            confirmed: true,
            skip_on_confirm: false,
            missing_since_ms: None,
        });
        self.dirty = true;
    }

    fn remaining_ms(model: &PlaybackModel, now_ms: u64) -> Option<u64> {
        let duration = model.duration_ms()?;

        Some(duration.saturating_sub(model.position_ms_at(now_ms)))
    }

    fn ready_for_handoff(&self, model: &PlaybackModel) -> bool {
        !self.entries.is_empty()
            && model.is_active
            && matches!(model.status, PlayStatus::Playing | PlayStatus::Buffering)
            && queued_uri(model).next().is_none()
    }

    fn handoff_due(&self, model: &PlaybackModel, now_ms: u64) -> bool {
        self.ready_for_handoff(model)
            && Self::remaining_ms(model, now_ms).is_some_and(|left| left <= HANDOFF_MARGIN_MS)
    }

    fn foreign_ahead(&self, model: &PlaybackModel) -> bool {
        let locked = self
            .handoff
            .as_ref()
            .map(|handoff| handoff.entry.uri.as_str());

        queued_uri(model).any(|uri| Some(uri) != locked)
    }

    /// The Next button. Hands the first entry over and skips once Soloist has it.
    pub fn next_pressed(&mut self, model: &PlaybackModel, now_ms: u64) -> Vec<QueueCommand> {
        if !model.is_active {
            return vec![QueueCommand::SkipNext];
        }

        if let Some(handoff) = &mut self.handoff {
            if handoff.confirmed {
                return vec![QueueCommand::SkipNext];
            }

            handoff.skip_on_confirm = true;
            self.dirty = true;

            return Vec::new();
        }

        if self.entries.is_empty() || self.foreign_ahead(model) {
            return vec![QueueCommand::SkipNext];
        }

        let entry = self.entries.remove(0);

        vec![self.start_handoff(entry, model, now_ms, true)]
    }

    /// Plays an entry right away: as the next track when nothing is in the
    /// way, otherwise by starting it directly.
    pub fn play_now(&mut self, id: u64, model: &PlaybackModel, now_ms: u64) -> Vec<QueueCommand> {
        let Some(from) = self.entries.iter().position(|entry| entry.id == id) else {
            return Vec::new();
        };

        let entry = self.entries.remove(from);

        if self.handoff.is_some() || !model.is_active || self.foreign_ahead(model) {
            let command = QueueCommand::PlayNow(entry.uri.clone());

            self.dirty = true;

            return vec![command];
        }

        self.entries.insert(0, entry);

        self.next_pressed(model, now_ms)
    }

    /// Milliseconds until the next transition that depends on time, or `None`
    /// when an idle queue needs no timer.
    pub fn next_wake_ms(&self, model: &PlaybackModel, now_ms: u64) -> Option<u64> {
        if let Some(handoff) = &self.handoff {
            if !trustworthy(model) {
                return None;
            }

            let deadline = match (handoff.confirmed, handoff.missing_since_ms) {
                (false, _) => handoff.sent_at_ms + CONFIRM_TIMEOUT_MS,
                (true, Some(since)) => since + LOST_GRACE_MS,
                (true, None) => return None,
            };

            return Some(deadline.saturating_sub(now_ms));
        }

        if !model.is_playing() || !self.ready_for_handoff(model) {
            return None;
        }

        let left = Self::remaining_ms(model, now_ms)?;

        Some(left.saturating_sub(HANDOFF_MARGIN_MS))
    }
}

type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn load(path: &Path) -> QueueState {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return QueueState::new(),
        Err(error) => {
            warn!(%error, "could not read the saved queue");

            return QueueState::new();
        }
    };

    match serde_json::from_str::<SavedQueue>(&text) {
        Ok(saved) => QueueState::from_saved(saved),
        Err(error) => {
            warn!(%error, "the saved queue is corrupt, starting empty");

            QueueState::new()
        }
    }
}

fn write(path: &Path, saved: &SavedQueue) -> std::io::Result<()> {
    if saved.entries.is_empty() {
        return match fs::remove_file(path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        };
    }

    let text = serde_json::to_string(saved).map_err(std::io::Error::other)?;
    let temp = path.with_extension("json.tmp");

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::File::create(&temp)?;

    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    drop(file);

    fs::rename(&temp, path)
}

/// Runs the queue rules against live playback and talks to Soloist.
pub struct QueueManager {
    state: Mutex<QueueState>,
    player: Player,
    events: broadcast::Sender<AppEvent>,
    path: Option<PathBuf>,
    clock: Clock,
    wake: Notify,
    /// Bumped by `wipe` so a save that was already in flight can't bring the file back.
    epoch: Arc<Mutex<u64>>,
}

impl QueueManager {
    /// `path` is `None` for an in-memory queue, which demo mode uses.
    pub fn new(
        player: Player,
        events: broadcast::Sender<AppEvent>,
        path: Option<PathBuf>,
        runtime: Handle,
    ) -> Arc<Self> {
        Self::with_clock(player, events, path, runtime, Arc::new(now_ms))
    }

    pub fn with_clock(
        player: Player,
        events: broadcast::Sender<AppEvent>,
        path: Option<PathBuf>,
        runtime: Handle,
        clock: Clock,
    ) -> Arc<Self> {
        let state = path.as_deref().map(load).unwrap_or_default();
        let feed = events.subscribe();
        let manager = Arc::new(Self {
            state: Mutex::new(state),
            player,
            events,
            path,
            clock,
            wake: Notify::new(),
            epoch: Arc::new(Mutex::new(0)),
        });

        runtime.spawn(manager.clone().run(feed));

        manager
    }

    pub fn snapshot(&self) -> QueueSnapshot {
        lock(&self.state).snapshot()
    }

    /// Returns false when the queue is full.
    pub fn add(&self, uri: &str, source: QueueSource) -> bool {
        let added = lock(&self.state).add(uri.to_owned(), source);

        self.wake.notify_one();

        added
    }

    pub fn remove(&self, id: u64) {
        lock(&self.state).remove(id);
        self.wake.notify_one();
    }

    pub fn move_before(&self, id: u64, before: Option<u64>) {
        lock(&self.state).move_before(id, before);
        self.wake.notify_one();
    }

    pub fn clear(&self) {
        lock(&self.state).clear();
        self.wake.notify_one();
    }

    /// The Next button. Use this instead of `Player::next`.
    pub fn next(&self) {
        let commands = lock(&self.state).next_pressed(&self.player.snapshot(), (self.clock)());

        self.send(commands);
        self.wake.notify_one();
    }

    pub fn play_now(&self, id: u64) {
        let commands = lock(&self.state).play_now(id, &self.player.snapshot(), (self.clock)());

        self.send(commands);
        self.wake.notify_one();
    }

    /// Empties the queue, including a track already handed to Soloist, and deletes the file.
    pub fn wipe(&self) -> std::io::Result<()> {
        lock(&self.state).reset();
        self.wake.notify_one();

        let mut epoch = lock(&self.epoch);

        *epoch += 1;

        match &self.path {
            Some(path) => match fs::remove_file(path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            },
            None => Ok(()),
        }
    }

    fn send(&self, commands: Vec<QueueCommand>) {
        for command in commands {
            match command {
                QueueCommand::AddToQueue(uri) => self.player.add_to_queue(&uri),
                QueueCommand::SkipNext => self.player.next(),
                QueueCommand::PlayNow(uri) => self.player.play(Some(&uri)),
            }
        }
    }

    async fn run(self: Arc<Self>, mut feed: broadcast::Receiver<AppEvent>) {
        loop {
            let wake = {
                let model = self.player.snapshot();

                lock(&self.state).next_wake_ms(&model, (self.clock)())
            };

            tokio::select! {
                event = feed.recv() => match event {
                    Ok(AppEvent::Playback(_)) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Ok(_) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                () = self.wake.notified() => {}
                () = tokio::time::sleep(Duration::from_millis(wake.unwrap_or_default().max(MIN_WAKE_MS))), if wake.is_some() => {}
            }

            self.step().await;
        }
    }

    async fn step(&self) {
        let model = self.player.snapshot();
        let (commands, saved, expected) = {
            let mut state = lock(&self.state);
            let commands = state.observe(&model, (self.clock)());
            let saved = state.take_dirty().then(|| state.to_saved());
            let expected = *lock(&self.epoch);

            (commands, saved, expected)
        };

        self.send(commands);

        let Some(saved) = saved else {
            return;
        };

        if let Some(path) = self.path.clone() {
            let epoch = self.epoch.clone();
            let result = tokio::task::spawn_blocking(move || {
                let epoch = lock(&epoch);

                if *epoch != expected {
                    return Ok(());
                }

                write(&path, &saved)
            })
            .await;

            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => warn!(%error, "could not save the queue"),
                Err(error) => debug!(%error, "queue save task failed"),
            }
        }

        let _ = self.events.send(AppEvent::Queue);
    }
}

#[cfg(test)]
pub(crate) mod tests;
