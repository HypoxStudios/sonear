use std::time::{SystemTime, UNIX_EPOCH};

use sonear_soloist::protocol::{
    AvailableActions, Entity, Event, Options, PlayStatus, Position, QueueEntry, RepeatMode,
};

/// Everything the UI needs to know about playback, built only from Soloist events.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlaybackModel {
    pub status: PlayStatus,
    pub item: Option<Entity>,
    /// May be a track URI that differs from `item`, so never assume they match.
    pub context: Option<Entity>,
    position: Option<Position>,
    pub volume: u8,
    pub options: Options,
    pub available_actions: AvailableActions,
    pub is_active: bool,
    pub device_name: String,
    pub logged_in: bool,
    pub upcoming: Vec<QueueEntry>,
    pub previous: Vec<QueueEntry>,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

impl PlaybackModel {
    pub fn with_volume(volume: u8) -> Self {
        Self {
            volume,
            ..Self::default()
        }
    }

    /// Applies an event using the system clock. Returns whether anything changed.
    pub fn apply(&mut self, event: &Event) -> bool {
        self.apply_at(event, now_ms())
    }

    /// Like `apply`, with the clock supplied for deterministic tests.
    pub fn apply_at(&mut self, event: &Event, now_ms: u64) -> bool {
        let before = self.clone();

        match event {
            Event::AuthState {
                logged_in,
                is_active,
                device_name,
            } => {
                self.logged_in = *logged_in;
                self.is_active = *is_active;
                self.device_name.clone_from(device_name);
            }
            Event::PlaybackState(snapshot) => {
                self.status = snapshot.status;
                self.item.clone_from(&snapshot.item);
                self.context.clone_from(&snapshot.context);
                self.position = snapshot.position;
                self.is_active = snapshot.is_active;
                self.available_actions
                    .clone_from(&snapshot.available_actions);

                if let Some(volume) = snapshot.volume {
                    self.volume = volume.min(100);
                }

                if let Some(options) = &snapshot.options {
                    self.options = options.clone();
                }
            }
            Event::PlaybackChanged { status } => self.status = *status,
            Event::PositionSync { position } => self.position = Some(*position),
            Event::TrackChanged { item } => {
                self.item.clone_from(item);
                self.position = Some(Position {
                    position_ms: 0,
                    timestamp_ms: now_ms,
                    speed: self.position.map_or(1.0, |position| position.speed),
                });
            }
            Event::DeviceChanged {
                is_active,
                device_name,
            } => {
                self.is_active = *is_active;
                self.device_name.clone_from(device_name);
            }
            Event::QueueChanged { previous, upcoming } => {
                self.previous.clone_from(previous);
                self.upcoming.clone_from(upcoming);
            }
            Event::VolumeChanged { volume } => self.volume = (*volume).min(100),
            Event::ContextChanged { context } => self.context.clone_from(context),
            Event::OptionsChanged { options } => self.options = options.clone(),
            Event::CommandResult { .. } | Event::Error { .. } | Event::Unknown => {}
        }

        *self != before
    }

    pub fn duration_ms(&self) -> Option<u64> {
        self.item.as_ref().and_then(Entity::duration_ms)
    }

    /// What the queue shows of `context_uri`: the current item, then upcoming
    /// entries from that context. `None` when something else is playing. Entries
    /// without details, or from the user's queue or autoplay, are left out, and
    /// so is the current item when the queue shows it under another source.
    /// `previous` isn't used, because nothing says it belongs to this context.
    /// Soloist sends the context and the queue separately and in either order,
    /// so right after a switch this can mix the two: wait for it to settle.
    pub fn context_sequence(&self, context_uri: &str) -> Option<Vec<&Entity>> {
        if self.context.as_ref()?.uri != context_uri {
            return None;
        }

        let foreign = |uri: &str| {
            self.previous
                .iter()
                .chain(&self.upcoming)
                .any(|entry| entry.source != "context" && entry.item.uri == uri)
        };
        let current = self
            .item
            .as_ref()
            .filter(|item| !item.name().is_empty() && !foreign(&item.uri));
        let upcoming = self
            .upcoming
            .iter()
            .filter(|entry| entry.source == "context" && !entry.item.name().is_empty())
            .map(|entry| &entry.item);

        Some(current.into_iter().chain(upcoming).collect())
    }

    pub fn is_playing(&self) -> bool {
        self.status == PlayStatus::Playing
    }

    pub fn is_buffering(&self) -> bool {
        self.status == PlayStatus::Buffering
    }

    pub fn can(&self, action: &str) -> bool {
        self.available_actions.contains_key(action)
    }

    pub fn repeat(&self) -> RepeatMode {
        self.options.repeat
    }

    /// The last position Soloist reported, which `position_ms_at` extrapolates from.
    pub fn position_anchor(&self) -> Option<Position> {
        self.position
    }

    /// Playback position at `now_ms`, extrapolated from the last anchor and
    /// clamped to the track length. Frozen unless the status is playing.
    pub fn position_ms_at(&self, now_ms: u64) -> u64 {
        let Some(anchor) = self.position else {
            return 0;
        };
        let speed = if self.is_playing() { anchor.speed } else { 0.0 };
        let elapsed = now_ms.saturating_sub(anchor.timestamp_ms) as f64 * speed;
        let position = (anchor.position_ms as f64 + elapsed).max(0.0) as u64;

        match self.duration_ms() {
            Some(duration) => position.min(duration),
            None => position,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(text: &str) -> Event {
        Event::parse(text).unwrap()
    }

    fn playing_state() -> Event {
        event(include_str!(
            "../../soloist/tests/fixtures/playback_state.json"
        ))
    }

    const SYNC_TS: u64 = 1_793_994_587_635;

    #[test]
    fn auth_state_sets_account_fields() {
        let mut model = PlaybackModel::default();
        let changed = model.apply_at(
            &event(include_str!("../../soloist/tests/fixtures/auth_state.json")),
            0,
        );

        assert!(changed);
        assert!(model.logged_in);
        assert!(!model.is_active);
        assert_eq!(model.device_name, "Sonear Test");
    }

    #[test]
    fn playback_state_fills_the_model() {
        let mut model = PlaybackModel::default();

        model.apply_at(&playing_state(), 0);

        assert!(model.is_playing());
        assert_eq!(
            model.item.as_ref().unwrap().artist_line(),
            "Daft Punk, Julian Casablancas"
        );
        assert_eq!(
            model.context.as_ref().unwrap().name(),
            "Random Access Memories"
        );
        assert_eq!(model.volume, 30);
        assert_eq!(model.repeat(), RepeatMode::Off);
        assert!(model.can("skip_next"));
        assert!(!model.can("play"));
        assert_eq!(model.duration_ms(), Some(337_560));
    }

    #[test]
    fn identical_events_report_no_change() {
        let mut model = PlaybackModel::default();

        assert!(model.apply_at(&playing_state(), 0));
        assert!(!model.apply_at(&playing_state(), 0));
    }

    #[test]
    fn playback_changed_and_device_changed() {
        let mut model = PlaybackModel::default();

        model.apply_at(&playing_state(), 0);
        model.apply_at(
            &event(include_str!(
                "../../soloist/tests/fixtures/playback_changed.json"
            )),
            0,
        );
        model.apply_at(
            &event(include_str!(
                "../../soloist/tests/fixtures/device_changed.json"
            )),
            0,
        );

        assert_eq!(model.status, PlayStatus::Paused);
        assert!(model.is_active);
        assert_eq!(model.device_name, "Sonear Test");
    }

    #[test]
    fn queue_changed_stores_both_lists() {
        let mut model = PlaybackModel::default();

        model.apply_at(
            &event(include_str!(
                "../../soloist/tests/fixtures/queue_changed.json"
            )),
            0,
        );

        assert_eq!(model.previous.len(), 3);
        assert_eq!(model.upcoming.len(), 3);
        assert_eq!(model.upcoming[0].source, "context");
    }

    fn item(uri: &str, name: &str) -> Entity {
        Entity {
            uri: uri.to_owned(),
            decorations: sonear_soloist::protocol::Decorations {
                identity: sonear_soloist::protocol::Identity {
                    name: name.to_owned(),
                },
                ..Default::default()
            },
            ..Entity::default()
        }
    }

    fn entry(uri: &str, source: &str) -> QueueEntry {
        QueueEntry {
            uid: uri.to_owned(),
            source: source.to_owned(),
            item: item(uri, uri),
        }
    }

    fn sequence_uris(model: &PlaybackModel, context: &str) -> Option<Vec<String>> {
        model.context_sequence(context).map(|sequence| {
            sequence
                .into_iter()
                .map(|entity| entity.uri.clone())
                .collect()
        })
    }

    #[test]
    fn context_sequence_orders_and_filters_by_source() {
        let mut model = PlaybackModel {
            context: Some(Entity {
                uri: "spotify:playlist:p".into(),
                ..Entity::default()
            }),
            item: Some(item("t3", "t3")),
            previous: vec![entry("t1", "context"), entry("q0", "queue")],
            upcoming: vec![
                entry("q1", "queue"),
                entry("t4", "context"),
                entry("a1", "autoplay"),
                entry("t5", "context"),
            ],
            ..PlaybackModel::default()
        };

        assert_eq!(sequence_uris(&model, "spotify:playlist:other"), None);
        assert_eq!(
            sequence_uris(&model, "spotify:playlist:p").unwrap(),
            ["t3", "t4", "t5"]
        );

        model.item = Some(item("q1", "q1"));

        assert_eq!(
            sequence_uris(&model, "spotify:playlist:p").unwrap(),
            ["t4", "t5"]
        );

        model.context = None;

        assert_eq!(sequence_uris(&model, "spotify:playlist:p"), None);
    }

    #[test]
    fn nameless_entries_wait_for_their_details() {
        let mut model = PlaybackModel {
            context: Some(item("spotify:playlist:p", "P")),
            item: Some(item("t1", "t1")),
            upcoming: vec![
                QueueEntry {
                    uid: "t2".into(),
                    source: "context".into(),
                    item: item("t2", ""),
                },
                entry("t3", "context"),
            ],
            ..PlaybackModel::default()
        };

        assert_eq!(
            sequence_uris(&model, "spotify:playlist:p").unwrap(),
            ["t1", "t3"]
        );

        model.upcoming[0].item = item("t2", "t2");
        model.item = Some(item("t0", ""));

        assert_eq!(
            sequence_uris(&model, "spotify:playlist:p").unwrap(),
            ["t2", "t3"]
        );
    }

    #[test]
    fn volume_options_and_context_events() {
        let mut model = PlaybackModel::default();

        model.apply_at(&event(r#"{"type":"volume_changed","volume":64}"#), 0);
        model.apply_at(
            &event(r#"{"type":"options_changed","options":{"shuffle":true,"repeat":"context"}}"#),
            0,
        );
        model.apply_at(
            &event(r#"{"type":"context_changed","context":{"uri":"spotify:playlist:p"}}"#),
            0,
        );

        assert_eq!(model.volume, 64);
        assert!(model.options.shuffle);
        assert_eq!(model.repeat(), RepeatMode::Context);
        assert_eq!(model.context.as_ref().unwrap().uri, "spotify:playlist:p");
    }

    #[test]
    fn noise_events_change_nothing() {
        let mut model = PlaybackModel::default();

        model.apply_at(&playing_state(), 0);

        let before = model.clone();

        assert!(!model.apply_at(
            &event(include_str!(
                "../../soloist/tests/fixtures/command_result.json"
            )),
            1
        ));
        assert!(!model.apply_at(
            &event(include_str!("../../soloist/tests/fixtures/error.json")),
            1
        ));
        assert!(!model.apply_at(&event(r#"{"type":"from_the_future"}"#), 1));
        assert_eq!(model, before);
    }

    #[test]
    fn position_interpolates_at_speed_one() {
        let mut model = PlaybackModel::default();

        model.apply_at(&playing_state(), SYNC_TS);

        assert_eq!(model.position_ms_at(SYNC_TS), 94_053);
        assert_eq!(model.position_ms_at(SYNC_TS + 2_500), 96_553);
    }

    #[test]
    fn position_is_frozen_at_speed_zero_and_when_paused() {
        let mut model = PlaybackModel::default();

        model.apply_at(&playing_state(), SYNC_TS);
        model.apply_at(
            &event(include_str!(
                "../../soloist/tests/fixtures/position_sync.json"
            )),
            SYNC_TS,
        );

        assert_eq!(model.position_ms_at(1_793_994_596_527 + 5_000), 102_892);

        model.apply_at(&playing_state(), SYNC_TS);
        model.apply_at(
            &event(r#"{"type":"playback_changed","status":"paused"}"#),
            SYNC_TS,
        );

        assert_eq!(model.position_ms_at(SYNC_TS + 9_000), 94_053);
    }

    #[test]
    fn position_is_clamped_to_the_track_length() {
        let mut model = PlaybackModel::default();

        model.apply_at(&playing_state(), SYNC_TS);

        assert_eq!(model.position_ms_at(SYNC_TS + 10_000_000), 337_560);
        assert_eq!(model.position_ms_at(SYNC_TS - 50_000), 94_053);
    }

    #[test]
    fn track_changed_resets_position_even_mid_crossfade() {
        let mut model = PlaybackModel::default();

        model.apply_at(&playing_state(), SYNC_TS);

        let late = SYNC_TS + 320_000;

        assert_eq!(model.position_ms_at(late), 337_560);

        model.apply_at(
            &event(include_str!(
                "../../soloist/tests/fixtures/track_changed.json"
            )),
            late,
        );

        assert_eq!(
            model.item.as_ref().unwrap().name(),
            "Lose Yourself to Dance (feat. Pharrell Williams)"
        );
        assert_eq!(model.position_ms_at(late), 0);
        assert_eq!(model.position_ms_at(late + 1_000), 1_000);
        assert_eq!(model.duration_ms(), Some(353_893));
    }

    #[test]
    fn position_without_anchor_is_zero() {
        assert_eq!(PlaybackModel::default().position_ms_at(123), 0);
    }
}
