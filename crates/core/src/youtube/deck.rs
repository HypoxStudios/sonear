//! The queue rules of the YouTube player: what plays now, next and after,
//! with shuffle, repeat and the user's queue. Pure and synchronous.

use std::collections::VecDeque;

use sonear_soloist::protocol::{Entity, QueueEntry, RepeatMode};

use super::convert::{context_entity, track_entity};
use crate::model::{Artwork, Track};

const SHUFFLE_SEED: u64 = 0x9E37_79B9;
/// A track restarts instead of going back once it is past this point.
const RESTART_AFTER_MS: u64 = 3_000;

/// What the tracks being played belong to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContextInfo {
    pub uri: String,
    pub kind: String,
    pub name: String,
    pub art: Artwork,
}

impl ContextInfo {
    pub fn entity(&self) -> Entity {
        context_entity(&self.uri, &self.kind, &self.name, &self.art)
    }
}

/// What `previous` should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Previous {
    /// Start the same track again.
    Restart,
    /// A different track is current now.
    Track,
}

#[derive(Debug, Clone)]
struct Queued {
    uid: u64,
    track: Track,
}

#[derive(Debug, Clone)]
pub struct Deck {
    base: Vec<Track>,
    /// Indices into `base` in the order they play.
    order: Vec<usize>,
    cursor: usize,
    context: Option<ContextInfo>,
    queued: VecDeque<Queued>,
    /// A queued track that is playing now, on top of the context cursor.
    interrupt: Option<Queued>,
    shuffle: bool,
    repeat_context: bool,
    repeat_track: bool,
    shuffle_seed: u64,
    next_uid: u64,
}

impl Default for Deck {
    fn default() -> Self {
        Self::new()
    }
}

impl Deck {
    pub fn new() -> Self {
        Self {
            base: Vec::new(),
            order: Vec::new(),
            cursor: 0,
            context: None,
            queued: VecDeque::new(),
            interrupt: None,
            shuffle: false,
            repeat_context: false,
            repeat_track: false,
            shuffle_seed: SHUFFLE_SEED,
            next_uid: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.current().is_none()
    }

    pub fn current(&self) -> Option<&Track> {
        match &self.interrupt {
            Some(queued) => Some(&queued.track),
            None => self.order.get(self.cursor).map(|index| &self.base[*index]),
        }
    }

    pub fn current_is_queued(&self) -> bool {
        self.interrupt.is_some()
    }

    pub fn context(&self) -> Option<&ContextInfo> {
        self.context.as_ref()
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn repeat(&self) -> RepeatMode {
        if self.repeat_track {
            RepeatMode::Track
        } else if self.repeat_context {
            RepeatMode::Context
        } else {
            RepeatMode::Off
        }
    }

    pub fn set_repeat_context(&mut self, enabled: bool) {
        self.repeat_context = enabled;
    }

    pub fn set_repeat_track(&mut self, enabled: bool) {
        self.repeat_track = enabled;
    }

    /// Starts `tracks` as the context at `start`. Returns false, changing
    /// nothing, when there are no tracks.
    pub fn load(&mut self, tracks: Vec<Track>, start: usize, context: Option<ContextInfo>) -> bool {
        if tracks.is_empty() {
            return false;
        }

        self.order = (0..tracks.len()).collect();
        self.cursor = start.min(tracks.len() - 1);
        self.base = tracks;
        self.context = context;
        self.interrupt = None;

        if self.shuffle {
            self.rebuild_order(true);
        }

        true
    }

    pub fn set_shuffle(&mut self, enabled: bool) {
        if self.order.is_empty() {
            self.shuffle = enabled;

            return;
        }

        self.rebuild_order(enabled);
    }

    fn rebuild_order(&mut self, shuffle: bool) {
        let current = self.order[self.cursor];

        self.shuffle = shuffle;

        if !shuffle {
            self.order = (0..self.base.len()).collect();
            self.cursor = current;

            return;
        }

        let mut rest: Vec<usize> = (0..self.base.len()).filter(|i| *i != current).collect();

        for index in (1..rest.len()).rev() {
            self.shuffle_seed = self
                .shuffle_seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);

            rest.swap(index, (self.shuffle_seed >> 33) as usize % (index + 1));
        }

        self.order = std::iter::once(current).chain(rest).collect();
        self.cursor = 0;
    }

    pub fn enqueue(&mut self, track: Track) {
        let uid = self.next_uid;

        self.next_uid += 1;
        self.queued.push_back(Queued { uid, track });
    }

    pub fn queued_len(&self) -> usize {
        self.queued.len()
    }

    fn start_queued(&mut self) -> bool {
        let Some(queued) = self.queued.pop_front() else {
            return false;
        };

        self.interrupt = Some(queued);

        true
    }

    fn start_cursor(&mut self, cursor: usize) {
        self.cursor = cursor;
        self.interrupt = None;
    }

    /// The Next button. Returns whether another track is current now.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> bool {
        if self.order.is_empty() {
            return false;
        }

        if self.start_queued() {
            return true;
        }

        if self.cursor + 1 < self.order.len() {
            self.start_cursor(self.cursor + 1);

            return true;
        }

        if self.repeat_context {
            self.start_cursor(0);

            return true;
        }

        false
    }

    pub fn previous(&mut self, position_ms: u64) -> Previous {
        if position_ms > RESTART_AFTER_MS || self.cursor == 0 || self.interrupt.is_some() {
            return Previous::Restart;
        }

        self.start_cursor(self.cursor - 1);

        Previous::Track
    }

    /// A track ended on its own. Returns whether something should play; when
    /// the context is over it is rewound to its first track and nothing does.
    pub fn advance(&mut self) -> bool {
        if self.order.is_empty() {
            return false;
        }

        if self.repeat_track || self.start_queued() {
            return true;
        }

        if self.cursor + 1 < self.order.len() {
            self.start_cursor(self.cursor + 1);

            return true;
        }

        self.start_cursor(0);

        self.repeat_context
    }

    /// What `advance` would play, for fetching it ahead of time.
    pub fn peek_next(&self) -> Option<&Track> {
        if self.repeat_track {
            return self.current();
        }

        if let Some(queued) = self.queued.front() {
            return Some(&queued.track);
        }

        let next = if self.cursor + 1 < self.order.len() {
            self.cursor + 1
        } else if self.repeat_context {
            0
        } else {
            return None;
        };

        self.order.get(next).map(|index| &self.base[*index])
    }

    /// The queue as the player shows it: queued tracks first, then what the
    /// context plays next.
    pub fn upcoming(&self, limit: usize) -> Vec<QueueEntry> {
        let queued = self.queued.iter().map(|queued| QueueEntry {
            uid: format!("ytm-queue-{}", queued.uid),
            source: "queue".into(),
            item: track_entity(&queued.track, None),
        });
        let context =
            self.order
                .iter()
                .enumerate()
                .skip(self.cursor + 1)
                .map(|(position, index)| QueueEntry {
                    uid: format!("ytm-context-{position}"),
                    source: "context".into(),
                    item: track_entity(&self.base[*index], None),
                });

        queued.chain(context).take(limit).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracks(count: usize) -> Vec<Track> {
        (0..count)
            .map(|n| Track {
                uri: format!("ytm:track:t{n}"),
                id: Some(format!("t{n}")),
                name: format!("Track {n}"),
                playable: true,
                ..Track::default()
            })
            .collect()
    }

    fn deck(count: usize) -> Deck {
        let mut deck = Deck::new();

        deck.load(tracks(count), 0, None);

        deck
    }

    fn name(deck: &Deck) -> &str {
        &deck.current().unwrap().name
    }

    #[test]
    fn an_empty_deck_has_nothing_current() {
        let mut deck = Deck::new();

        assert!(deck.is_empty());
        assert!(!deck.load(Vec::new(), 0, None));
        assert!(!deck.next() && !deck.advance());
        assert!(deck.upcoming(10).is_empty());
    }

    #[test]
    fn load_starts_at_the_requested_track_and_clamps() {
        let mut deck = Deck::new();

        deck.load(tracks(5), 3, None);
        assert_eq!(name(&deck), "Track 3");

        deck.load(tracks(5), 99, None);
        assert_eq!(name(&deck), "Track 4");
    }

    #[test]
    fn next_and_previous_follow_the_transport_rules() {
        let mut deck = deck(3);

        assert!(deck.next());
        assert_eq!(name(&deck), "Track 1");
        assert_eq!(deck.previous(10_000), Previous::Restart);
        assert_eq!(name(&deck), "Track 1");
        assert_eq!(deck.previous(500), Previous::Track);
        assert_eq!(name(&deck), "Track 0");
        assert_eq!(deck.previous(500), Previous::Restart);

        deck.next();
        deck.next();

        assert!(!deck.next());
        assert_eq!(name(&deck), "Track 2");

        deck.set_repeat_context(true);

        assert!(deck.next());
        assert_eq!(name(&deck), "Track 0");
    }

    #[test]
    fn repeat_track_replays_on_advance() {
        let mut deck = deck(3);

        deck.set_repeat_track(true);

        assert_eq!(deck.repeat(), RepeatMode::Track);
        assert!(deck.advance());
        assert_eq!(name(&deck), "Track 0");
        assert_eq!(deck.peek_next().unwrap().name, "Track 0");

        deck.set_repeat_track(false);
        deck.set_repeat_context(true);

        assert_eq!(deck.repeat(), RepeatMode::Context);
    }

    #[test]
    fn end_of_context_stops_unless_repeating() {
        let mut deck = Deck::new();

        deck.load(tracks(3), 2, None);

        assert!(deck.peek_next().is_none());
        assert!(!deck.advance());
        assert_eq!(name(&deck), "Track 0");

        deck.load(tracks(3), 2, None);
        deck.set_repeat_context(true);

        assert_eq!(deck.peek_next().unwrap().name, "Track 0");
        assert!(deck.advance());
        assert_eq!(name(&deck), "Track 0");
    }

    #[test]
    fn shuffle_keeps_the_current_track_and_covers_every_track() {
        let mut deck = Deck::new();

        deck.load(tracks(12), 5, None);
        deck.set_shuffle(true);

        assert!(deck.shuffle());
        assert_eq!(name(&deck), "Track 5");

        let mut seen = deck.order.clone();

        seen.sort_unstable();

        assert_eq!(seen, (0..12).collect::<Vec<_>>());
        assert_ne!(deck.order, (0..12).collect::<Vec<_>>());

        deck.set_shuffle(false);

        assert_eq!(name(&deck), "Track 5");
        assert_eq!(deck.cursor, 5);
    }

    #[test]
    fn shuffle_is_deterministic_and_applies_to_a_new_load() {
        let mut first = Deck::new();
        let mut second = Deck::new();

        first.set_shuffle(true);
        second.set_shuffle(true);
        first.load(tracks(10), 0, None);
        second.load(tracks(10), 0, None);

        assert_eq!(first.order, second.order);
        assert_eq!(name(&first), "Track 0");
        assert_ne!(first.order, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn queued_tracks_play_before_the_context_resumes() {
        let mut deck = deck(4);
        let queued = |n: usize| tracks(n + 1).remove(n);
        let mut first = queued(0);
        let mut second = queued(1);

        first.name = "Queued A".into();
        first.uri = "ytm:track:qa".into();
        second.name = "Queued B".into();
        second.uri = "ytm:track:qb".into();
        deck.enqueue(first);
        deck.enqueue(second);

        let upcoming = deck.upcoming(10);

        assert_eq!(upcoming[0].source, "queue");
        assert_eq!(upcoming[0].item.name(), "Queued A");
        assert_eq!(upcoming[1].source, "queue");
        assert_eq!(upcoming[2].source, "context");
        assert_eq!(upcoming[2].item.name(), "Track 1");
        assert_ne!(upcoming[0].uid, upcoming[1].uid);
        assert_eq!(deck.peek_next().unwrap().name, "Queued A");

        assert!(deck.next());
        assert_eq!(name(&deck), "Queued A");
        assert!(deck.current_is_queued());
        assert_eq!(deck.cursor, 0);
        assert_eq!(deck.upcoming(10)[0].item.name(), "Queued B");
        assert_eq!(deck.previous(500), Previous::Restart);

        assert!(deck.advance());
        assert_eq!(name(&deck), "Queued B");
        assert_eq!(deck.queued_len(), 0);
        assert_eq!(deck.upcoming(10)[0].source, "context");

        assert!(deck.advance());
        assert_eq!(name(&deck), "Track 1");
        assert_eq!(deck.cursor, 1);
        assert!(!deck.current_is_queued());
    }

    #[test]
    fn upcoming_respects_the_limit_and_a_play_never_queues_twice() {
        let mut deck = deck(100);

        assert_eq!(deck.upcoming(7).len(), 7);

        deck.enqueue(tracks(1).remove(0));
        deck.enqueue(tracks(1).remove(0));

        let uids: Vec<String> = deck
            .upcoming(50)
            .into_iter()
            .map(|entry| entry.uid)
            .collect();
        let mut unique = uids.clone();

        unique.sort();
        unique.dedup();

        assert_eq!(unique.len(), uids.len());
    }

    #[test]
    fn the_context_is_kept_for_the_snapshot() {
        let mut deck = Deck::new();

        deck.load(
            tracks(2),
            0,
            Some(ContextInfo {
                uri: "ytm:album:x".into(),
                kind: "album".into(),
                name: "Album".into(),
                art: Artwork::default(),
            }),
        );

        let entity = deck.context().unwrap().entity();

        assert_eq!(entity.uri, "ytm:album:x");
        assert_eq!(entity.name(), "Album");
    }
}
