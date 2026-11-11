//! Plain output types. No InnerTube wire shapes leak out of this crate.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thumb {
    pub url: String,
    pub width: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistRef {
    pub id: Option<String>,
    pub name: String,
}

/// The signed-in account.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountInfo {
    pub name: String,
    pub photo: Vec<Thumb>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Song {
    pub video_id: String,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub album: Option<AlbumRef>,
    pub duration_ms: Option<u64>,
    pub thumbs: Vec<Thumb>,
    pub explicit: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumCard {
    pub browse_id: String,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub year: Option<String>,
    /// "Album", "Single" or "EP", worded in the request language.
    pub kind: String,
    pub thumbs: Vec<Thumb>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistCard {
    pub channel_id: String,
    pub name: String,
    pub thumbs: Vec<Thumb>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistCard {
    /// Without the `VL` prefix that browse ids carry.
    pub playlist_id: String,
    pub title: String,
    pub author: Option<String>,
    pub track_count: Option<u32>,
    pub thumbs: Vec<Thumb>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumPage {
    pub album: AlbumCard,
    /// The `OLAK5uy_…` playlist that plays the whole album.
    pub audio_playlist_id: Option<String>,
    pub tracks: Vec<Song>,
    pub track_count: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowseRef {
    pub browse_id: String,
    pub params: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistPage {
    pub artist: ArtistCard,
    pub top_songs: Vec<Song>,
    pub top_songs_playlist: Option<String>,
    pub albums: Vec<AlbumCard>,
    pub singles: Vec<AlbumCard>,
    pub albums_more: Option<BrowseRef>,
    pub singles_more: Option<BrowseRef>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistPage {
    pub playlist: PlaylistCard,
    pub description: String,
    pub tracks: Vec<Song>,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackPage {
    pub tracks: Vec<Song>,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResults {
    pub songs: Vec<Song>,
    pub albums: Vec<AlbumCard>,
    pub artists: Vec<ArtistCard>,
    pub playlists: Vec<PlaylistCard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HomeItem {
    Song(Song),
    Album(AlbumCard),
    Artist(ArtistCard),
    Playlist(PlaylistCard),
}

/// What a shelf is made of, so a caller can pick the shelf it wants.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShelfKind {
    Songs,
    Albums,
    Playlists,
    Artists,
    #[default]
    Mixed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeShelf {
    pub title: String,
    pub kind: ShelfKind,
    pub items: Vec<HomeItem>,
}

impl HomeShelf {
    pub fn new(title: String, items: Vec<HomeItem>) -> Self {
        let kind_of = |item: &HomeItem| match item {
            HomeItem::Song(_) => ShelfKind::Songs,
            HomeItem::Album(_) => ShelfKind::Albums,
            HomeItem::Artist(_) => ShelfKind::Artists,
            HomeItem::Playlist(_) => ShelfKind::Playlists,
        };
        let kind = match items.first().map(kind_of) {
            Some(first) if items.iter().all(|item| kind_of(item) == first) => first,
            _ => ShelfKind::Mixed,
        };

        Self { title, kind, items }
    }
}
