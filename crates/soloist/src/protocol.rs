//! Wire types for the Soloist WebSocket API.
//!
//! The API is new, so every type tolerates missing and extra fields and the
//! event enum has an `Unknown` catch-all.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    GetAuthState,
    GetState,
    GetQueue {
        #[serde(skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
    },
    Play {
        #[serde(skip_serializing_if = "Option::is_none")]
        uri: Option<String>,
    },
    Pause,
    SkipNext,
    SkipPrev,
    Seek {
        position_ms: u64,
    },
    SetVolume {
        volume: u8,
    },
    SetShuffle {
        enabled: bool,
    },
    SetRepeatContext {
        enabled: bool,
    },
    SetRepeatTrack {
        enabled: bool,
    },
    AddToQueue {
        uri: String,
    },
    Activate,
    Deactivate,
}

impl Command {
    /// The JSON object sent over the socket.
    pub fn to_value(&self) -> Value {
        let mut value = serde_json::to_value(self).unwrap_or_else(|_| json!({}));

        if let Value::Object(map) = &mut value {
            map.insert("type".to_owned(), json!("command"));
        }

        value
    }

    pub fn to_json(&self) -> String {
        self.to_value().to_string()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cover {
    pub url: String,
    pub size: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Identity {
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VisualIdentity {
    pub cover: Vec<Cover>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Parent {
    pub entity: Option<Box<Entity>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Creator {
    pub entity: Option<Entity>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackInfo {
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Decorations {
    pub identity: Identity,
    pub visual_identity: VisualIdentity,
    pub parent: Option<Parent>,
    pub creators: Vec<Creator>,
    pub playback: Option<PlaybackInfo>,
}

/// A track, album, artist or playlist as Soloist describes it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entity {
    pub uri: String,
    pub entity_type: String,
    pub decorations: Decorations,
}

const COVER_ORDER: [&str; 4] = ["small", "default", "large", "xlarge"];

impl Entity {
    pub fn name(&self) -> &str {
        &self.decorations.identity.name
    }

    /// The cover whose size matches `prefer` (`small`, `default`, `large`,
    /// `xlarge`), or the closest available size.
    /// With no cover of its own, the album's is used.
    pub fn cover_url(&self, prefer: &str) -> Option<&str> {
        if self.decorations.visual_identity.cover.is_empty() {
            return self.album()?.cover_url(prefer);
        }

        let covers = &self.decorations.visual_identity.cover;

        if let Some(exact) = covers.iter().find(|cover| cover.size == prefer) {
            return Some(&exact.url);
        }

        let wanted = COVER_ORDER.iter().position(|size| *size == prefer);

        wanted
            .and_then(|wanted| {
                covers
                    .iter()
                    .filter_map(|cover| {
                        let rank = COVER_ORDER.iter().position(|size| *size == cover.size)?;
                        Some((rank.abs_diff(wanted), cover))
                    })
                    .min_by_key(|(distance, _)| *distance)
                    .map(|(_, cover)| cover.url.as_str())
            })
            .or_else(|| covers.first().map(|cover| cover.url.as_str()))
    }

    pub fn artist_names(&self) -> Vec<&str> {
        self.decorations
            .creators
            .iter()
            .filter_map(|creator| creator.entity.as_ref())
            .map(Entity::name)
            .filter(|name| !name.is_empty())
            .collect()
    }

    pub fn first_artist(&self) -> Option<&str> {
        self.artist_names().into_iter().next()
    }

    pub fn artist_line(&self) -> String {
        self.artist_names().join(", ")
    }

    pub fn album(&self) -> Option<&Entity> {
        self.decorations
            .parent
            .as_ref()
            .and_then(|parent| parent.entity.as_deref())
    }

    pub fn duration_ms(&self) -> Option<u64> {
        self.decorations
            .playback
            .as_ref()
            .and_then(|playback| playback.duration_ms)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Position {
    pub position_ms: u64,
    pub timestamp_ms: u64,
    pub speed: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayStatus {
    Playing,
    Paused,
    Buffering,
    #[default]
    #[serde(other)]
    Stopped,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatMode {
    #[default]
    Off,
    Context,
    Track,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub playback_speed: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ActionInfo {
    pub step_ms: Option<u64>,
}

pub type AvailableActions = BTreeMap<String, ActionInfo>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackSnapshot {
    pub status: PlayStatus,
    pub item: Option<Entity>,
    pub context: Option<Entity>,
    pub position: Option<Position>,
    pub volume: Option<u8>,
    pub is_active: bool,
    pub options: Option<Options>,
    pub available_actions: AvailableActions,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct QueueEntry {
    pub uid: String,
    pub source: String,
    pub item: Entity,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    AuthState {
        #[serde(default)]
        logged_in: bool,
        #[serde(default)]
        is_active: bool,
        #[serde(default)]
        device_name: String,
    },
    PlaybackState(Box<PlaybackSnapshot>),
    PlaybackChanged {
        #[serde(default)]
        status: PlayStatus,
    },
    PositionSync {
        #[serde(default)]
        position: Position,
    },
    TrackChanged {
        #[serde(default)]
        item: Option<Entity>,
    },
    DeviceChanged {
        #[serde(default)]
        is_active: bool,
        #[serde(default)]
        device_name: String,
    },
    QueueChanged {
        #[serde(default)]
        previous: Vec<QueueEntry>,
        #[serde(default)]
        upcoming: Vec<QueueEntry>,
    },
    VolumeChanged {
        #[serde(default)]
        volume: u8,
    },
    ContextChanged {
        #[serde(default)]
        context: Option<Entity>,
    },
    OptionsChanged {
        #[serde(default)]
        options: Options,
    },
    CommandResult {
        #[serde(default)]
        command: String,
    },
    Error {
        #[serde(default)]
        message: String,
    },
    #[serde(other)]
    Unknown,
}

impl Event {
    pub fn parse(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }
}
