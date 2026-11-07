//! Wire types, written against real responses. Every field has a default,
//! unknown fields are ignored and unreadable list entries are skipped.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

fn lenient_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let values = Option::<Vec<Value>>::deserialize(deserializer)?.unwrap_or_default();

    Ok(values
        .into_iter()
        .filter(|value| !value.is_null())
        .filter_map(|value| serde_json::from_value(value).ok())
        .collect())
}

fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Image {
    pub url: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ExternalUrls {
    pub spotify: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Page<T: DeserializeOwned> {
    #[serde(deserialize_with = "lenient_vec")]
    pub items: Vec<T>,
    pub total: Option<u32>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub next: Option<String>,
}

impl<T: DeserializeOwned> Default for Page<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            total: None,
            limit: None,
            offset: None,
            next: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Artist {
    pub id: Option<String>,
    pub uri: String,
    pub name: String,
    #[serde(deserialize_with = "lenient_vec")]
    pub images: Vec<Image>,
    pub external_urls: ExternalUrls,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Album {
    pub id: Option<String>,
    pub uri: String,
    pub name: String,
    pub album_type: Option<String>,
    #[serde(deserialize_with = "lenient_vec")]
    pub images: Vec<Image>,
    #[serde(deserialize_with = "lenient_vec")]
    pub artists: Vec<Artist>,
    pub release_date: Option<String>,
    pub total_tracks: Option<u32>,
    pub external_urls: ExternalUrls,
    /// Only on the full album object.
    pub tracks: Option<Page<Track>>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Track {
    pub id: Option<String>,
    pub uri: String,
    pub name: String,
    pub duration_ms: u64,
    #[serde(deserialize_with = "lenient_vec")]
    pub artists: Vec<Artist>,
    pub album: Option<Album>,
    pub is_local: bool,
    pub is_playable: Option<bool>,
    pub explicit: bool,
    pub track_number: Option<u32>,
    pub external_urls: ExternalUrls,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Show {
    pub id: Option<String>,
    pub uri: String,
    pub name: String,
    pub publisher: Option<String>,
    pub description: Option<String>,
    #[serde(deserialize_with = "lenient_vec")]
    pub images: Vec<Image>,
    pub total_episodes: Option<u32>,
    pub external_urls: ExternalUrls,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Episode {
    pub id: Option<String>,
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
    pub duration_ms: u64,
    #[serde(deserialize_with = "lenient_vec")]
    pub images: Vec<Image>,
    pub release_date: Option<String>,
    pub show: Option<Show>,
    pub external_urls: ExternalUrls,
}

/// A track or an episode, told apart by `type`.
#[derive(Debug, Clone, PartialEq)]
pub enum MediaItem {
    Track(Box<Track>),
    Episode(Box<Episode>),
}

impl<'de> Deserialize<'de> for MediaItem {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let is_episode = value.get("type").and_then(Value::as_str) == Some("episode");

        if is_episode {
            serde_json::from_value(value)
                .map(|episode| Self::Episode(Box::new(episode)))
                .map_err(serde::de::Error::custom)
        } else {
            serde_json::from_value(value)
                .map(|track| Self::Track(Box::new(track)))
                .map_err(serde::de::Error::custom)
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct User {
    pub id: Option<String>,
    pub display_name: Option<String>,
    #[serde(deserialize_with = "lenient_vec")]
    pub images: Vec<Image>,
    pub uri: Option<String>,
    pub external_urls: ExternalUrls,
}

/// The `items` (older apps: `tracks`) block of a playlist. In a listing it is
/// only a total; in the full object it also carries the first rows.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct PlaylistItems {
    pub total: Option<u32>,
    #[serde(deserialize_with = "lenient_vec")]
    pub items: Vec<PlaylistRow>,
    pub next: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Playlist {
    pub id: Option<String>,
    pub uri: String,
    pub name: String,
    #[serde(deserialize_with = "null_as_default")]
    pub description: String,
    #[serde(deserialize_with = "lenient_vec")]
    pub images: Vec<Image>,
    pub owner: Option<User>,
    pub collaborative: bool,
    pub public: Option<bool>,
    pub snapshot_id: Option<String>,
    pub items: Option<PlaylistItems>,
    pub tracks: Option<PlaylistItems>,
    pub external_urls: ExternalUrls,
}

impl Playlist {
    pub fn total(&self) -> Option<u32> {
        self.items
            .as_ref()
            .and_then(|items| items.total)
            .or_else(|| self.tracks.as_ref().and_then(|tracks| tracks.total))
    }
}

/// A playlist row. The track or episode is `item`, or `track` in older shapes.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct PlaylistRow {
    pub added_at: Option<String>,
    pub is_local: bool,
    #[serde(alias = "track")]
    pub item: Option<MediaItem>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct SavedTrack {
    pub added_at: Option<String>,
    pub track: Option<Track>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct SavedAlbum {
    pub added_at: Option<String>,
    pub album: Option<Album>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct SavedShow {
    pub added_at: Option<String>,
    pub show: Option<Show>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ContextRef {
    pub uri: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct PlayHistory {
    pub track: Option<Track>,
    pub played_at: String,
    pub context: Option<ContextRef>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Cursors {
    pub after: Option<String>,
    pub before: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct RecentlyPlayed {
    #[serde(deserialize_with = "lenient_vec")]
    pub items: Vec<PlayHistory>,
    pub cursors: Option<Cursors>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Device {
    pub id: Option<String>,
    pub name: String,
    pub is_active: bool,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Devices {
    #[serde(deserialize_with = "lenient_vec")]
    pub devices: Vec<Device>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Queue {
    pub currently_playing: Option<MediaItem>,
    #[serde(deserialize_with = "lenient_vec")]
    pub queue: Vec<MediaItem>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct PlayerState {
    pub is_playing: bool,
    pub device: Option<Device>,
    pub item: Option<MediaItem>,
    pub context: Option<ContextRef>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct SearchResults {
    pub tracks: Option<Page<Track>>,
    pub artists: Option<Page<Artist>>,
    pub albums: Option<Page<Album>>,
    pub playlists: Option<Page<Playlist>>,
    pub shows: Option<Page<Show>>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct FollowedArtists {
    pub artists: ArtistCursorPage,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ArtistCursorPage {
    #[serde(deserialize_with = "lenient_vec")]
    pub items: Vec<Artist>,
    pub total: Option<u32>,
    pub next: Option<String>,
    pub cursors: Option<Cursors>,
}

#[cfg(test)]
mod tests;
