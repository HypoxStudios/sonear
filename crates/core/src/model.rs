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

/// Folds `observed`, a contiguous run of a playlist seen in the queue, into
/// the `stored` list. The run is authoritative for the span of `stored` it
/// covers, so reorders show up and any other stored copy of an observed track
/// is dropped. Stored tracks inside that span but missing from the run were
/// either removed or the run's tracks moved past them, which can't be told
/// apart, so they are kept right after the run instead of being lost; a
/// removed track drifts towards the end. With no overlap the run goes in at
/// the front. Returns whether `stored` changed.
pub fn merge_window(stored: &mut Vec<Track>, observed: &[Track]) -> bool {
    let mut window: Vec<Track> = Vec::new();

    for track in observed {
        if !window.iter().any(|known| known.uri == track.uri) {
            window.push(track.clone());
        }
    }

    if window.is_empty() {
        return false;
    }

    let in_window = |track: &Track| window.iter().any(|known| known.uri == track.uri);
    let mut overlap = stored
        .iter()
        .enumerate()
        .filter(|(_, track)| in_window(track))
        .map(|(index, _)| index);
    let first = overlap.next();
    let last = overlap.next_back().or(first);
    let (before, span, after) = match (first, last) {
        (Some(first), Some(last)) => (&stored[..first], &stored[first..=last], &stored[last + 1..]),
        _ => (&stored[..0], &stored[..0], &stored[..]),
    };
    let merged: Vec<Track> = before
        .iter()
        .filter(|track| !in_window(track))
        .chain(&window)
        .chain(span.iter().filter(|track| !in_window(track)))
        .chain(after.iter().filter(|track| !in_window(track)))
        .cloned()
        .collect();

    if merged == *stored {
        return false;
    }

    *stored = merged;

    true
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

    fn window(stored: &[&str], observed: &[&str]) -> (Vec<String>, bool) {
        let mut stored = stubs(stored);
        let changed = merge_window(&mut stored, &stubs(observed));

        (
            uris(&stored).into_iter().map(str::to_owned).collect(),
            changed,
        )
    }

    #[test]
    fn an_all_new_window_goes_in_at_the_front() {
        assert_eq!(
            window(&[], &["a", "b", "c"]),
            (["a", "b", "c"].map(String::from).to_vec(), true)
        );
        assert_eq!(window(&["a", "b"], &["x", "y"]).0, ["x", "y", "a", "b"]);
    }

    #[test]
    fn overlap_appends_the_new_tail() {
        assert_eq!(
            window(&["a", "b", "c"], &["b", "c", "d", "e"]),
            (["a", "b", "c", "d", "e"].map(String::from).to_vec(), true)
        );
    }

    #[test]
    fn a_reorder_inside_the_window_is_taken() {
        assert_eq!(
            window(&["a", "b", "c", "d"], &["a", "c", "b", "d"]),
            (["a", "c", "b", "d"].map(String::from).to_vec(), true)
        );
    }

    #[test]
    fn tracks_left_out_of_the_window_move_after_it() {
        assert_eq!(
            window(&["a", "b", "c", "d", "e"], &["a", "c", "e"]),
            (["a", "c", "e", "b", "d"].map(String::from).to_vec(), true)
        );
    }

    #[test]
    fn one_track_moved_from_far_away_loses_nothing() {
        assert_eq!(
            window(&["a", "b", "c", "d", "e", "f"], &["b", "f"]),
            (
                ["a", "b", "f", "c", "d", "e"].map(String::from).to_vec(),
                true
            )
        );
        assert!(!window(&["a", "b", "f", "c", "d", "e"], &["b", "f"]).1);
    }

    #[test]
    fn a_track_moved_into_the_window_is_not_duplicated() {
        assert_eq!(
            window(&["a", "b", "c", "z"], &["b", "z", "c"]),
            (["a", "b", "z", "c"].map(String::from).to_vec(), true)
        );
        assert_eq!(
            window(&["z", "a", "b", "c"], &["a", "z", "b"]),
            (["a", "z", "b", "c"].map(String::from).to_vec(), true)
        );
    }

    #[test]
    fn tracks_outside_the_window_keep_their_order() {
        assert_eq!(
            window(&["a", "b", "c", "d", "e"], &["c", "d"]).0,
            ["a", "b", "c", "d", "e"]
        );
        assert_eq!(
            window(&["a", "b", "c", "d", "e"], &["c", "x", "d"]).0,
            ["a", "b", "c", "x", "d", "e"]
        );
    }

    #[test]
    fn an_identical_window_reports_no_change() {
        assert!(!window(&["a", "b", "c"], &["a", "b", "c"]).1);
        assert!(!window(&["a", "b", "c"], &["b", "c"]).1);
        assert!(!window(&["a"], &[]).1);
    }

    #[test]
    fn a_repeated_track_in_the_window_keeps_the_first() {
        assert_eq!(
            window(&["a"], &["a", "b", "a", "b", "c"]),
            (["a", "b", "c"].map(String::from).to_vec(), true)
        );
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
