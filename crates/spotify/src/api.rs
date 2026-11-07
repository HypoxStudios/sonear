//! One method per endpoint Sonear uses. Only endpoints that work on a
//! brand-new Client ID are here, plus artist top tracks as an optional extra.

use reqwest::Method;
use serde::Serialize;
use serde_json::{Value, json};

use crate::client::SpotifyClient;
use crate::error::SpotifyError;
use crate::models::{
    Album, Artist, Devices, Episode, FollowedArtists, MediaItem, Page, PlayerState, Playlist,
    PlaylistRow, Queue, RecentlyPlayed, SavedAlbum, SavedShow, SavedTrack, SearchResults, Show,
    Track, User,
};

/// Spotify rejects a search `limit` above 10 on new apps.
pub const SEARCH_LIMIT_MAX: u32 = 10;
/// Spotify answers 400 to a higher `limit` on an artist's albums.
pub const ARTIST_ALBUMS_LIMIT_MAX: u32 = 10;
/// Most URIs `/me/library` accepts per call.
pub const LIBRARY_CHUNK: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeRange {
    Short,
    Medium,
    Long,
}

impl TimeRange {
    fn as_str(self) -> &'static str {
        match self {
            Self::Short => "short_term",
            Self::Medium => "medium_term",
            Self::Long => "long_term",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchType {
    Track,
    Artist,
    Album,
    Playlist,
    Show,
}

impl SearchType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Track => "track",
            Self::Artist => "artist",
            Self::Album => "album",
            Self::Playlist => "playlist",
            Self::Show => "show",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Offset {
    Position(u32),
    Uri(String),
}

/// Body of `PUT /me/player/play`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayParams {
    pub context_uri: Option<String>,
    pub uris: Option<Vec<String>>,
    pub offset: Option<Offset>,
}

impl PlayParams {
    fn to_body(&self) -> Value {
        let mut body = serde_json::Map::new();

        if let Some(context) = &self.context_uri {
            body.insert("context_uri".into(), json!(context));
        }

        if let Some(uris) = &self.uris {
            body.insert("uris".into(), json!(uris));
        }

        if let Some(offset) = &self.offset {
            body.insert(
                "offset".into(),
                serde_json::to_value(offset).unwrap_or_default(),
            );
        }

        Value::Object(body)
    }
}

/// Spotify IDs are 22 base62 characters. Anything else could carry `../` or
/// `?` into the joined URL and reach another endpoint.
fn resource(kind: &str, id: &str, rest: &str) -> Result<String, SpotifyError> {
    if id.len() != 22 || !id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(SpotifyError::InvalidId(id.chars().take(40).collect()));
    }

    Ok(format!("{kind}/{id}{rest}"))
}

fn paging(limit: u32, offset: u32) -> Vec<(&'static str, String)> {
    vec![("limit", limit.to_string()), ("offset", offset.to_string())]
}

impl SpotifyClient {
    pub async fn me(&self) -> Result<User, SpotifyError> {
        self.get("me", &[]).await
    }

    /// `None` when nothing is active (HTTP 204).
    pub async fn player_state(&self) -> Result<Option<PlayerState>, SpotifyError> {
        self.get_optional("me/player", &[]).await
    }

    pub async fn devices(&self) -> Result<Devices, SpotifyError> {
        self.get("me/player/devices", &[]).await
    }

    pub async fn queue(&self) -> Result<Queue, SpotifyError> {
        self.get("me/player/queue", &[]).await
    }

    pub async fn play(&self, params: &PlayParams, device_id: &str) -> Result<(), SpotifyError> {
        let query = [("device_id", device_id.to_owned())];

        self.send(
            Method::PUT,
            "me/player/play",
            &query,
            Some(&params.to_body()),
        )
        .await
        .map(|_| ())
    }

    /// The last plays. `after` is a Unix time in milliseconds. Only the
    /// latest 50 plays exist, so paging backwards is pointless.
    pub async fn recently_played(
        &self,
        limit: u32,
        after: Option<u64>,
    ) -> Result<RecentlyPlayed, SpotifyError> {
        let mut query = vec![("limit", limit.min(50).to_string())];

        if let Some(after) = after {
            query.push(("after", after.to_string()));
        }

        self.get("me/player/recently-played", &query).await
    }

    pub async fn my_playlists(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Page<Playlist>, SpotifyError> {
        self.get("me/playlists", &paging(limit, offset)).await
    }

    pub async fn track(&self, id: &str) -> Result<Track, SpotifyError> {
        self.get(&resource("tracks", id, "")?, &[]).await
    }

    pub async fn album(&self, id: &str) -> Result<Album, SpotifyError> {
        self.get(&resource("albums", id, "")?, &[]).await
    }

    pub async fn album_tracks(
        &self,
        id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Page<Track>, SpotifyError> {
        self.get(&resource("albums", id, "/tracks")?, &paging(limit, offset))
            .await
    }

    pub async fn artist(&self, id: &str) -> Result<Artist, SpotifyError> {
        self.get(&resource("artists", id, "")?, &[]).await
    }

    pub async fn artist_albums(
        &self,
        id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Page<Album>, SpotifyError> {
        let mut query = paging(limit.clamp(1, ARTIST_ALBUMS_LIMIT_MAX), offset);

        query.push(("include_groups", "album,single".to_owned()));

        self.get(&resource("artists", id, "/albums")?, &query).await
    }

    /// Forbidden on new Client IDs. Callers try it once and remember the answer.
    pub async fn artist_top_tracks(&self, id: &str) -> Result<Vec<Track>, SpotifyError> {
        #[derive(serde::Deserialize, Default)]
        #[serde(default)]
        struct Tracks {
            tracks: Vec<Track>,
        }

        let tracks: Tracks = self
            .get(&resource("artists", id, "/top-tracks")?, &[])
            .await?;

        Ok(tracks.tracks)
    }

    /// One call for several types. `limit` is clamped to 10.
    pub async fn search(
        &self,
        query: &str,
        types: &[SearchType],
        limit: u32,
        offset: u32,
    ) -> Result<SearchResults, SpotifyError> {
        let kinds = types
            .iter()
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let mut params = paging(limit.clamp(1, SEARCH_LIMIT_MAX), offset);

        params.push(("q", query.to_owned()));
        params.push(("type", kinds));

        self.get("search", &params).await
    }

    pub async fn saved_tracks(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Page<SavedTrack>, SpotifyError> {
        self.get("me/tracks", &paging(limit, offset)).await
    }

    pub async fn saved_albums(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Page<SavedAlbum>, SpotifyError> {
        self.get("me/albums", &paging(limit, offset)).await
    }

    pub async fn saved_shows(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Page<SavedShow>, SpotifyError> {
        self.get("me/shows", &paging(limit, offset)).await
    }

    /// Cursor paged: pass the `after` of the previous answer.
    pub async fn followed_artists(
        &self,
        limit: u32,
        after: Option<&str>,
    ) -> Result<FollowedArtists, SpotifyError> {
        let mut query = vec![("type", "artist".to_owned()), ("limit", limit.to_string())];

        if let Some(after) = after {
            query.push(("after", after.to_owned()));
        }

        self.get("me/following", &query).await
    }

    pub async fn top_tracks(
        &self,
        range: TimeRange,
        limit: u32,
        offset: u32,
    ) -> Result<Page<Track>, SpotifyError> {
        let mut query = paging(limit, offset);

        query.push(("time_range", range.as_str().to_owned()));

        self.get("me/top/tracks", &query).await
    }

    pub async fn top_artists(
        &self,
        range: TimeRange,
        limit: u32,
        offset: u32,
    ) -> Result<Page<Artist>, SpotifyError> {
        let mut query = paging(limit, offset);

        query.push(("time_range", range.as_str().to_owned()));

        self.get("me/top/artists", &query).await
    }

    /// Metadata plus the first rows. The rows are missing, or the call fails
    /// with `Forbidden`, for playlists the user does not own.
    pub async fn playlist(&self, id: &str) -> Result<Playlist, SpotifyError> {
        self.get(&resource("playlists", id, "")?, &[]).await
    }

    /// `Forbidden` or `NotFound` for playlists the user does not own.
    pub async fn playlist_items(
        &self,
        id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Page<PlaylistRow>, SpotifyError> {
        self.get(
            &resource("playlists", id, "/items")?,
            &paging(limit, offset),
        )
        .await
    }

    pub async fn show(&self, id: &str) -> Result<Show, SpotifyError> {
        self.get(&resource("shows", id, "")?, &[]).await
    }

    pub async fn show_episodes(
        &self,
        id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Page<Episode>, SpotifyError> {
        self.get(&resource("shows", id, "/episodes")?, &paging(limit, offset))
            .await
    }

    /// Which of `uris` are saved, in order. Chunked to `LIBRARY_CHUNK`.
    pub async fn library_contains(&self, uris: &[String]) -> Result<Vec<bool>, SpotifyError> {
        let mut saved = Vec::with_capacity(uris.len());

        for chunk in uris.chunks(LIBRARY_CHUNK) {
            let query = [("uris", chunk.join(","))];
            let answer: Vec<bool> = self.get("me/library/contains", &query).await?;

            saved.extend(answer);
        }

        Ok(saved)
    }

    pub async fn library_save(&self, uris: &[String]) -> Result<(), SpotifyError> {
        self.library_write(Method::PUT, uris).await
    }

    pub async fn library_remove(&self, uris: &[String]) -> Result<(), SpotifyError> {
        self.library_write(Method::DELETE, uris).await
    }

    async fn library_write(&self, method: Method, uris: &[String]) -> Result<(), SpotifyError> {
        for chunk in uris.chunks(LIBRARY_CHUNK) {
            let query = [("uris", chunk.join(","))];

            self.send(method.clone(), "me/library", &query, None)
                .await?;
        }

        Ok(())
    }
}

impl MediaItem {
    pub fn uri(&self) -> &str {
        match self {
            Self::Track(track) => &track.uri,
            Self::Episode(episode) => &episode.uri,
        }
    }
}

#[cfg(test)]
mod tests;
