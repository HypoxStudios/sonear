//! Domain types the UI uses. They do not expose Spotify's wire types.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtImage {
    pub url: String,
    pub width: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artwork(pub Vec<ArtImage>);

impl Artwork {
    /// The smallest image at least `pixels` wide, else the largest known one.
    pub fn url_for(&self, pixels: u32) -> Option<&str> {
        let width = |image: &&ArtImage| image.width.unwrap_or(0);

        self.0
            .iter()
            .filter(|image| image.width.is_some_and(|width| width >= pixels))
            .min_by_key(width)
            .or_else(|| self.0.iter().max_by_key(width))
            .map(|image| image.url.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistRef {
    pub id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumRef {
    pub id: Option<String>,
    pub uri: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemKind {
    #[default]
    Track,
    Episode,
}

/// A playable row: a track, or an episode shown in the same lists.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub uri: String,
    pub id: Option<String>,
    pub name: String,
    pub artists: Vec<ArtistRef>,
    pub album: Option<AlbumRef>,
    pub art: Artwork,
    pub duration_ms: u64,
    pub is_local: bool,
    pub playable: bool,
    pub kind: ItemKind,
    pub open_url: Option<String>,
}

impl Track {
    pub fn artist_line(&self) -> String {
        self.artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Album {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub artists: Vec<ArtistRef>,
    pub art: Artwork,
    pub release_date: Option<String>,
    pub total_tracks: u32,
    pub album_type: String,
    pub open_url: Option<String>,
}

impl Album {
    pub fn artist_line(&self) -> String {
        self.artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn year(&self) -> Option<&str> {
        self.release_date.as_deref().and_then(|date| date.get(..4))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artist {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub art: Artwork,
    pub open_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub description: String,
    pub art: Artwork,
    pub owner_id: Option<String>,
    pub owner_name: Option<String>,
    pub collaborative: bool,
    pub track_count: Option<u32>,
    pub open_url: Option<String>,
}

impl Playlist {
    /// Whether Spotify lists this playlist's rows to the user: they own it or collaborate on it.
    pub fn is_editable_by(&self, user_id: Option<&str>) -> bool {
        self.collaborative || (user_id.is_some() && self.owner_id.as_deref() == user_id)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Show {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub publisher: String,
    pub description: String,
    pub art: Artwork,
    pub episode_count: Option<u32>,
    pub open_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Episode {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub description: String,
    pub art: Artwork,
    pub duration_ms: u64,
    pub release_date: Option<String>,
    pub show_name: Option<String>,
    pub open_url: Option<String>,
}

impl From<Episode> for Track {
    fn from(episode: Episode) -> Self {
        Self {
            id: Some(episode.id),
            uri: episode.uri,
            name: episode.name,
            artists: episode
                .show_name
                .into_iter()
                .map(|name| ArtistRef { id: None, name })
                .collect(),
            album: None,
            art: episode.art,
            duration_ms: episode.duration_ms,
            is_local: false,
            playable: true,
            kind: ItemKind::Episode,
            open_url: episode.open_url,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: Option<u32>,
    pub offset: u32,
    pub has_more: bool,
    /// For cursor-paged lists (followed artists).
    pub cursor: Option<String>,
}

impl<T> Default for Page<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            total: None,
            offset: 0,
            has_more: false,
            cursor: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub display_name: String,
    pub art: Artwork,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SearchKind {
    Track,
    Artist,
    Album,
    Playlist,
    Show,
}

pub const SEARCH_KINDS: [SearchKind; 5] = [
    SearchKind::Track,
    SearchKind::Artist,
    SearchKind::Album,
    SearchKind::Playlist,
    SearchKind::Show,
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResults {
    pub tracks: Page<Track>,
    pub artists: Page<Artist>,
    pub albums: Page<Album>,
    pub playlists: Page<Playlist>,
    pub shows: Page<Show>,
}

/// One entry of Spotify's recently-played list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayRecord {
    pub played_at_ms: u64,
    pub track: Track,
    pub context_uri: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayOffset {
    Position(u32),
    Uri(String),
}

/// What to start, described in the UI's terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackRequest {
    /// Start a whole album, playlist or artist from its first track.
    Context { uri: String },
    /// Start inside a context at a row.
    InContext {
        context_uri: String,
        offset: PlayOffset,
    },
    /// Play a list of tracks (Liked Songs, search results) from `start` on.
    Tracks { uris: Vec<String>, start: usize },
}

/// Wire form of a Web API play call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebPlayRequest {
    pub context_uri: Option<String>,
    pub uris: Option<Vec<String>>,
    pub offset: Option<PlayOffset>,
}

/// The most track URIs sent in one play call.
pub const MAX_PLAY_URIS: usize = 100;

/// `spotify:album:abc` becomes `("album", "abc")`.
pub fn split_uri(uri: &str) -> Option<(&str, &str)> {
    let mut parts = uri.split(':');

    if parts.next()? != "spotify" {
        return None;
    }

    let kind = parts.next()?;
    let id = parts.next_back()?;

    Some((kind, id))
}

/// Adds the tracks of `sequence` that `seen` lacks, each right after the
/// track before it in `sequence` (at the front when there is none). Known
/// tracks never move. Returns the URIs that were added.
pub fn merge_seen(seen: &mut Vec<Track>, sequence: &[Track]) -> Vec<String> {
    let mut added = Vec::new();
    let mut at = 0;

    for track in sequence {
        if let Some(position) = seen.iter().position(|known| known.uri == track.uri) {
            at = position + 1;

            continue;
        }

        seen.insert(at, track.clone());
        added.push(track.uri.clone());
        at += 1;
    }

    added
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stubs(uris: &[&str]) -> Vec<Track> {
        uris.iter()
            .map(|uri| Track {
                uri: (*uri).to_owned(),
                ..Track::default()
            })
            .collect()
    }

    fn uris(tracks: &[Track]) -> Vec<&str> {
        tracks.iter().map(|track| track.uri.as_str()).collect()
    }

    #[test]
    fn merging_into_nothing_takes_the_sequence() {
        let mut seen = Vec::new();
        let added = merge_seen(&mut seen, &stubs(&["a", "b", "c"]));

        assert_eq!(uris(&seen), ["a", "b", "c"]);
        assert_eq!(added, ["a", "b", "c"]);
    }

    #[test]
    fn overlap_appends_only_the_new_tail() {
        let mut seen = stubs(&["a", "b", "c"]);
        let added = merge_seen(&mut seen, &stubs(&["b", "c", "d", "e"]));

        assert_eq!(uris(&seen), ["a", "b", "c", "d", "e"]);
        assert_eq!(added, ["d", "e"]);
    }

    #[test]
    fn a_jump_forward_is_filled_in_later() {
        let mut seen = stubs(&["a", "b"]);

        merge_seen(&mut seen, &stubs(&["x", "y"]));

        assert_eq!(uris(&seen), ["x", "y", "a", "b"]);

        let mut seen = stubs(&["a", "b", "z"]);

        merge_seen(&mut seen, &stubs(&["z", "y"]));
        merge_seen(&mut seen, &stubs(&["b", "c", "z"]));

        assert_eq!(uris(&seen), ["a", "b", "c", "z", "y"]);
    }

    #[test]
    fn a_repeated_track_never_duplicates() {
        let mut seen = stubs(&["a"]);
        let added = merge_seen(&mut seen, &stubs(&["a", "b", "a", "b", "c"]));

        assert_eq!(uris(&seen), ["a", "b", "c"]);
        assert_eq!(added, ["b", "c"]);
        assert!(merge_seen(&mut seen, &stubs(&["a", "b", "c"])).is_empty());
    }

    fn art(widths: &[Option<u32>]) -> Artwork {
        Artwork(
            widths
                .iter()
                .map(|width| ArtImage {
                    url: format!("u{}", width.unwrap_or(0)),
                    width: *width,
                })
                .collect(),
        )
    }

    #[test]
    fn picks_the_smallest_image_that_is_large_enough() {
        let art = art(&[Some(640), Some(300), Some(64)]);

        assert_eq!(art.url_for(50), Some("u64"));
        assert_eq!(art.url_for(64), Some("u64"));
        assert_eq!(art.url_for(100), Some("u300"));
        assert_eq!(art.url_for(400), Some("u640"));
        assert_eq!(art.url_for(900), Some("u640"));
    }

    #[test]
    fn images_without_sizes_still_resolve() {
        let art = art(&[None]);

        assert_eq!(art.url_for(300), Some("u0"));
        assert_eq!(Artwork::default().url_for(10), None);
    }

    #[test]
    fn uris_split_into_kind_and_id() {
        assert_eq!(split_uri("spotify:album:abc"), Some(("album", "abc")));
        assert_eq!(split_uri("https://x"), None);
    }
}
