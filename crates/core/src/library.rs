//! What the UI may ask of the music catalogue. Real mode talks to Spotify,
//! demo mode answers from fake data through the same trait.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use async_trait::async_trait;
use sonear_spotify::api::{Offset, PlayParams, SearchType, TimeRange};
use sonear_spotify::{RateLimit, SpotifyClient, SpotifyError};
use thiserror::Error;
use tokio::sync::{Notify, watch};

use crate::convert;
use crate::model::{
    Album, Artist, Device, Episode, Page, PlayOffset, PlayRecord, Playlist, Profile, SearchKind,
    SearchResults, Show, Track, WebPlayRequest,
};

pub const PAGE_SIZE: u32 = 50;
/// Spotify's limit for search on a new Client ID.
pub const SEARCH_PAGE: u32 = 10;
/// Spotify's limit for an artist's albums.
pub const ARTIST_ALBUMS_PAGE: u32 = 10;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LibraryError {
    #[error("Spotify needs you to log in again.")]
    NeedsLogin,
    #[error("Spotify doesn't allow this for your account.")]
    Forbidden,
    #[error("Spotify couldn't find that.")]
    NotFound,
    #[error("Spotify asked Sonear to slow down. Try again in a moment.")]
    RateLimited,
    #[error(
        "The Spotify request quota for this developer account is used up. It resets later and is shared by everything using your Client ID."
    )]
    QuotaExceeded,
    #[error("Couldn't reach Spotify. Check your connection.")]
    Network(String),
    #[error("Spotify answered with an error ({0}).")]
    Api(String),
    #[error("Spotify sent something Sonear couldn't read.")]
    Decode(String),
    #[error("This isn't available in the current mode.")]
    Unsupported,
}

impl From<SpotifyError> for LibraryError {
    fn from(error: SpotifyError) -> Self {
        match error {
            SpotifyError::NeedsLogin => Self::NeedsLogin,
            SpotifyError::Forbidden => Self::Forbidden,
            SpotifyError::NotFound => Self::NotFound,
            SpotifyError::RateLimited { .. } => Self::RateLimited,
            SpotifyError::QuotaExceeded => Self::QuotaExceeded,
            SpotifyError::Network(reason) => Self::Network(reason),
            SpotifyError::Api { status, message } => Self::Api(format!("{status}: {message}")),
            SpotifyError::Decode(reason) => Self::Decode(reason),
        }
    }
}

pub type LibraryResult<T> = Result<T, LibraryError>;

/// Every method has an "unsupported" default so tests and demo mode only
/// implement what they need.
#[async_trait]
pub trait LibraryBackend: Send + Sync {
    async fn profile(&self) -> LibraryResult<Profile> {
        Err(LibraryError::Unsupported)
    }

    async fn my_playlists(&self, _offset: u32) -> LibraryResult<Page<Playlist>> {
        Err(LibraryError::Unsupported)
    }

    async fn saved_albums(&self, _offset: u32) -> LibraryResult<Page<Album>> {
        Err(LibraryError::Unsupported)
    }

    /// Cursor paged: pass the previous page's `cursor`.
    async fn followed_artists(&self, _after: Option<String>) -> LibraryResult<Page<Artist>> {
        Err(LibraryError::Unsupported)
    }

    async fn saved_shows(&self, _offset: u32) -> LibraryResult<Page<Show>> {
        Err(LibraryError::Unsupported)
    }

    async fn saved_tracks(&self, _offset: u32) -> LibraryResult<Page<Track>> {
        Err(LibraryError::Unsupported)
    }

    async fn top_tracks(&self, _limit: u32) -> LibraryResult<Vec<Track>> {
        Err(LibraryError::Unsupported)
    }

    async fn recently_played(&self, _limit: u32) -> LibraryResult<Vec<PlayRecord>> {
        Err(LibraryError::Unsupported)
    }

    async fn track(&self, _id: &str) -> LibraryResult<Track> {
        Err(LibraryError::Unsupported)
    }

    async fn album(&self, _id: &str) -> LibraryResult<Album> {
        Err(LibraryError::Unsupported)
    }

    async fn album_tracks(&self, _id: &str, _offset: u32) -> LibraryResult<Page<Track>> {
        Err(LibraryError::Unsupported)
    }

    async fn artist(&self, _id: &str) -> LibraryResult<Artist> {
        Err(LibraryError::Unsupported)
    }

    async fn artist_albums(&self, _id: &str, _offset: u32) -> LibraryResult<Page<Album>> {
        Err(LibraryError::Unsupported)
    }

    /// Forbidden on new Client IDs. Callers try once and remember the answer.
    async fn artist_top_tracks(&self, _id: &str) -> LibraryResult<Vec<Track>> {
        Err(LibraryError::Forbidden)
    }

    async fn playlist(&self, _id: &str) -> LibraryResult<Playlist> {
        Err(LibraryError::Unsupported)
    }

    /// `Forbidden` or `NotFound` when Spotify withholds the rows of a playlist the user does not own.
    async fn playlist_tracks(&self, _id: &str, _offset: u32) -> LibraryResult<Page<Track>> {
        Err(LibraryError::Unsupported)
    }

    async fn show(&self, _id: &str) -> LibraryResult<Show> {
        Err(LibraryError::Unsupported)
    }

    async fn show_episodes(&self, _id: &str, _offset: u32) -> LibraryResult<Page<Episode>> {
        Err(LibraryError::Unsupported)
    }

    async fn search(
        &self,
        _query: &str,
        _kinds: &[SearchKind],
        _offset: u32,
    ) -> LibraryResult<SearchResults> {
        Err(LibraryError::Unsupported)
    }

    /// Which of `uris` are saved, in order.
    async fn contains(&self, _uris: &[String]) -> LibraryResult<Vec<bool>> {
        Err(LibraryError::Unsupported)
    }

    async fn save(&self, _uris: &[String]) -> LibraryResult<()> {
        Err(LibraryError::Unsupported)
    }

    async fn remove(&self, _uris: &[String]) -> LibraryResult<()> {
        Err(LibraryError::Unsupported)
    }

    async fn devices(&self) -> LibraryResult<Vec<Device>> {
        Err(LibraryError::Unsupported)
    }

    async fn start_playback(
        &self,
        _request: &WebPlayRequest,
        _device_id: &str,
    ) -> LibraryResult<()> {
        Err(LibraryError::Unsupported)
    }

    /// Spotify's current pause state, if this backend has one.
    fn rate_limit(&self) -> Option<watch::Receiver<RateLimit>> {
        None
    }
}

const TOP_TRACKS_UNKNOWN: u8 = 0;
const TOP_TRACKS_YES: u8 = 1;
const TOP_TRACKS_NO: u8 = 2;
/// How many pages of `/me/playlists` are scanned to find a followed playlist's total.
const TOTAL_SCAN_PAGES: u32 = 8;

/// Real-mode backend. The client appears after login and goes away on disconnect.
pub struct SpotifyLibrary {
    client: RwLock<Option<SpotifyClient>>,
    totals: Mutex<HashMap<String, u32>>,
    top_tracks_support: AtomicU8,
    session_lost: Arc<Notify>,
}

impl Default for SpotifyLibrary {
    fn default() -> Self {
        Self {
            client: RwLock::new(None),
            totals: Mutex::new(HashMap::new()),
            top_tracks_support: AtomicU8::new(TOP_TRACKS_UNKNOWN),
            session_lost: Arc::new(Notify::new()),
        }
    }
}

impl SpotifyLibrary {
    /// Swaps the client and returns the one it replaced.
    pub fn set_client(&self, client: Option<SpotifyClient>) -> Option<SpotifyClient> {
        let previous = std::mem::replace(
            &mut *self
                .client
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            client,
        );

        self.top_tracks_support
            .store(TOP_TRACKS_UNKNOWN, Ordering::Relaxed);
        self.totals
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();

        previous
    }

    /// Notified when Spotify rejects the login for good.
    pub fn session_lost(&self) -> Arc<Notify> {
        self.session_lost.clone()
    }

    fn client(&self) -> LibraryResult<SpotifyClient> {
        self.client
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .ok_or(LibraryError::NeedsLogin)
    }

    fn lift<T>(&self, result: Result<T, SpotifyError>) -> LibraryResult<T> {
        result.map_err(|error| {
            if error == SpotifyError::NeedsLogin {
                self.session_lost.notify_one();
            }

            error.into()
        })
    }

    fn remember_totals(&self, playlists: &[Playlist]) {
        let mut totals = self
            .totals
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        for playlist in playlists {
            if let Some(total) = playlist.track_count {
                totals.insert(playlist.id.clone(), total);
            }
        }
    }

    fn known_total(&self, id: &str) -> Option<u32> {
        self.totals
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(id)
            .copied()
    }
}

fn to_params(request: &WebPlayRequest) -> PlayParams {
    PlayParams {
        context_uri: request.context_uri.clone(),
        uris: request.uris.clone(),
        offset: request.offset.as_ref().map(|offset| match offset {
            PlayOffset::Position(position) => Offset::Position(*position),
            PlayOffset::Uri(uri) => Offset::Uri(uri.clone()),
        }),
    }
}

fn search_type(kind: SearchKind) -> SearchType {
    match kind {
        SearchKind::Track => SearchType::Track,
        SearchKind::Artist => SearchType::Artist,
        SearchKind::Album => SearchType::Album,
        SearchKind::Playlist => SearchType::Playlist,
        SearchKind::Show => SearchType::Show,
    }
}

#[async_trait]
impl LibraryBackend for SpotifyLibrary {
    async fn profile(&self) -> LibraryResult<Profile> {
        let user = self.lift(self.client()?.me().await)?;

        Ok(convert::profile(&user))
    }

    async fn my_playlists(&self, offset: u32) -> LibraryResult<Page<Playlist>> {
        let page = self.lift(self.client()?.my_playlists(PAGE_SIZE, offset).await)?;
        let page = convert::page(page, offset, |playlist| convert::playlist(&playlist));

        self.remember_totals(&page.items);

        Ok(page)
    }

    async fn saved_albums(&self, offset: u32) -> LibraryResult<Page<Album>> {
        let page = self.lift(self.client()?.saved_albums(PAGE_SIZE, offset).await)?;

        Ok(convert::page(page, offset, |saved| {
            saved.album.as_ref().and_then(convert::album)
        }))
    }

    async fn followed_artists(&self, after: Option<String>) -> LibraryResult<Page<Artist>> {
        let answer = self.lift(
            self.client()?
                .followed_artists(PAGE_SIZE, after.as_deref())
                .await,
        )?;
        let cursor = answer
            .artists
            .cursors
            .and_then(|cursors| cursors.after)
            .filter(|_| answer.artists.next.is_some());

        Ok(Page {
            items: answer
                .artists
                .items
                .iter()
                .filter_map(convert::artist)
                .collect(),
            total: answer.artists.total,
            offset: 0,
            has_more: cursor.is_some(),
            cursor,
        })
    }

    async fn saved_shows(&self, offset: u32) -> LibraryResult<Page<Show>> {
        let page = self.lift(self.client()?.saved_shows(PAGE_SIZE, offset).await)?;

        Ok(convert::page(page, offset, |saved| {
            saved.show.as_ref().and_then(convert::show)
        }))
    }

    async fn saved_tracks(&self, offset: u32) -> LibraryResult<Page<Track>> {
        let page = self.lift(self.client()?.saved_tracks(PAGE_SIZE, offset).await)?;

        Ok(convert::page(page, offset, |saved| {
            saved
                .track
                .as_ref()
                .map(|track| convert::track(track, None))
        }))
    }

    async fn top_tracks(&self, limit: u32) -> LibraryResult<Vec<Track>> {
        let page = self.lift(self.client()?.top_tracks(TimeRange::Short, limit, 0).await)?;

        Ok(page
            .items
            .iter()
            .map(|track| convert::track(track, None))
            .collect())
    }

    async fn recently_played(&self, limit: u32) -> LibraryResult<Vec<PlayRecord>> {
        let recent = self.lift(self.client()?.recently_played(limit, None).await)?;

        Ok(recent
            .items
            .iter()
            .filter_map(|play| {
                let track = play.track.as_ref()?;
                let played_at = chrono_ms(&play.played_at)?;

                Some(PlayRecord {
                    played_at_ms: played_at,
                    track: convert::track(track, None),
                    context_uri: play
                        .context
                        .as_ref()
                        .map(|context| context.uri.clone())
                        .filter(|uri| !uri.is_empty()),
                })
            })
            .collect())
    }

    async fn track(&self, id: &str) -> LibraryResult<Track> {
        let track = self.lift(self.client()?.track(id).await)?;

        Ok(convert::track(&track, None))
    }

    async fn album(&self, id: &str) -> LibraryResult<Album> {
        let album = self.lift(self.client()?.album(id).await)?;

        convert::album(&album).ok_or(LibraryError::NotFound)
    }

    async fn album_tracks(&self, id: &str, offset: u32) -> LibraryResult<Page<Track>> {
        let tracks = self.lift(self.client()?.album_tracks(id, PAGE_SIZE, offset).await)?;

        Ok(convert::page(tracks, offset, |track| {
            Some(convert::track(&track, None))
        }))
    }

    async fn artist(&self, id: &str) -> LibraryResult<Artist> {
        let artist = self.lift(self.client()?.artist(id).await)?;

        convert::artist(&artist).ok_or(LibraryError::NotFound)
    }

    async fn artist_albums(&self, id: &str, offset: u32) -> LibraryResult<Page<Album>> {
        let page = self.lift(
            self.client()?
                .artist_albums(id, ARTIST_ALBUMS_PAGE, offset)
                .await,
        )?;

        Ok(convert::page(page, offset, |album| convert::album(&album)))
    }

    async fn artist_top_tracks(&self, id: &str) -> LibraryResult<Vec<Track>> {
        if self.top_tracks_support.load(Ordering::Relaxed) == TOP_TRACKS_NO {
            return Err(LibraryError::Forbidden);
        }

        let result = self.client()?.artist_top_tracks(id).await;

        match &result {
            Ok(_) => self
                .top_tracks_support
                .store(TOP_TRACKS_YES, Ordering::Relaxed),
            Err(SpotifyError::Forbidden | SpotifyError::NotFound) => self
                .top_tracks_support
                .store(TOP_TRACKS_NO, Ordering::Relaxed),
            Err(_) => {}
        }

        Ok(self
            .lift(result)?
            .iter()
            .map(|track| convert::track(track, None))
            .collect())
    }

    async fn playlist(&self, id: &str) -> LibraryResult<Playlist> {
        let wire_playlist = self.lift(self.client()?.playlist(id).await)?;
        let mut playlist = convert::playlist(&wire_playlist).ok_or(LibraryError::NotFound)?;

        if playlist.track_count.is_none() {
            playlist.track_count = self.known_total(id);
        }

        if playlist.track_count.is_none() {
            for page in 0..TOTAL_SCAN_PAGES {
                // The total is a nicety, so a failed scan must not fail the page.
                let Ok(listing) = self.my_playlists(page * PAGE_SIZE).await else {
                    break;
                };

                if let Some(total) = self.known_total(id) {
                    playlist.track_count = Some(total);
                    break;
                }

                if !listing.has_more {
                    break;
                }
            }
        }

        Ok(playlist)
    }

    async fn playlist_tracks(&self, id: &str, offset: u32) -> LibraryResult<Page<Track>> {
        let page = self.lift(self.client()?.playlist_items(id, PAGE_SIZE, offset).await)?;

        Ok(convert::page(page, offset, |row| {
            row.item.as_ref().and_then(convert::media_item)
        }))
    }

    async fn show(&self, id: &str) -> LibraryResult<Show> {
        let show = self.lift(self.client()?.show(id).await)?;

        convert::show(&show).ok_or(LibraryError::NotFound)
    }

    async fn show_episodes(&self, id: &str, offset: u32) -> LibraryResult<Page<Episode>> {
        let client = self.client()?;
        let episodes = self.lift(client.show_episodes(id, PAGE_SIZE, offset).await)?;
        let show = self.lift(client.show(id).await).ok();

        Ok(convert::page(episodes, offset, |episode| {
            convert::episode(&episode, show.as_ref())
        }))
    }

    async fn search(
        &self,
        query: &str,
        kinds: &[SearchKind],
        offset: u32,
    ) -> LibraryResult<SearchResults> {
        let types: Vec<SearchType> = kinds.iter().copied().map(search_type).collect();
        let found = self.lift(
            self.client()?
                .search(query, &types, SEARCH_PAGE, offset)
                .await,
        )?;

        Ok(SearchResults {
            tracks: found
                .tracks
                .map(|page| convert::page(page, offset, |t| Some(convert::track(&t, None))))
                .unwrap_or_default(),
            artists: found
                .artists
                .map(|page| convert::page(page, offset, |a| convert::artist(&a)))
                .unwrap_or_default(),
            albums: found
                .albums
                .map(|page| convert::page(page, offset, |a| convert::album(&a)))
                .unwrap_or_default(),
            playlists: found
                .playlists
                .map(|page| convert::page(page, offset, |p| convert::playlist(&p)))
                .unwrap_or_default(),
            shows: found
                .shows
                .map(|page| convert::page(page, offset, |s| convert::show(&s)))
                .unwrap_or_default(),
        })
    }

    async fn contains(&self, uris: &[String]) -> LibraryResult<Vec<bool>> {
        self.lift(self.client()?.library_contains(uris).await)
    }

    async fn save(&self, uris: &[String]) -> LibraryResult<()> {
        self.lift(self.client()?.library_save(uris).await)
    }

    async fn remove(&self, uris: &[String]) -> LibraryResult<()> {
        self.lift(self.client()?.library_remove(uris).await)
    }

    async fn devices(&self) -> LibraryResult<Vec<Device>> {
        let devices = self.lift(self.client()?.devices().await)?;

        Ok(devices.devices.iter().filter_map(convert::device).collect())
    }

    async fn start_playback(&self, request: &WebPlayRequest, device_id: &str) -> LibraryResult<()> {
        self.lift(self.client()?.play(&to_params(request), device_id).await)
    }

    fn rate_limit(&self) -> Option<watch::Receiver<RateLimit>> {
        self.client().ok().map(|client| client.rate_limit())
    }
}

/// `2026-11-06T20:41:56.424Z` as Unix milliseconds.
pub fn chrono_ms(text: &str) -> Option<u64> {
    let time = chrono::DateTime::parse_from_rfc3339(text).ok()?;

    u64::try_from(time.timestamp_millis()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn played_at_parses_to_milliseconds() {
        assert_eq!(
            chrono_ms("2026-11-06T20:41:56.424Z"),
            Some(1_793_997_716_424)
        );
        assert_eq!(chrono_ms("garbage"), None);
    }

    #[test]
    fn spotify_errors_map_to_library_errors() {
        assert_eq!(
            LibraryError::from(SpotifyError::Forbidden),
            LibraryError::Forbidden
        );
        assert_eq!(
            LibraryError::from(SpotifyError::QuotaExceeded),
            LibraryError::QuotaExceeded
        );
        assert_eq!(
            LibraryError::from(SpotifyError::NeedsLogin),
            LibraryError::NeedsLogin
        );
    }
}
