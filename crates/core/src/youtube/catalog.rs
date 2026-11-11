//! One shared view of YouTube Music for the library and the player: the
//! catalogue client, the local library, and small caches so the pages the
//! UI asks for in quick succession cost one request.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use futures_util::future::join_all;
use sonear_youtube::Client;
use sonear_youtube::SearchFilter;
use sonear_youtube::models as yt;
use tracing::debug;

use super::convert;
use super::deck::ContextInfo;
use super::local::LocalLibrary;
use crate::library::{LibraryError, LibraryResult};
use crate::model::{
    Album, Artist, Artwork, Page, Playlist, SearchKind, SearchResults, Shelf, ShelfItems, Track,
    is_liked_context,
};
use crate::radio;

/// The most tracks a playlist context loads.
pub const MAX_CONTEXT_TRACKS: usize = 1000;
/// Tracks remembered by video id before the index starts over.
const INDEX_LIMIT: usize = 5_000;
const PAGE_CACHE: usize = 32;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct Bounded<V> {
    items: HashMap<String, V>,
    order: VecDeque<String>,
}

impl<V: Clone> Bounded<V> {
    fn new() -> Self {
        Self {
            items: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(&self, key: &str) -> Option<V> {
        self.items.get(key).cloned()
    }

    fn put(&mut self, key: &str, value: V) {
        if self.items.insert(key.to_owned(), value).is_none() {
            self.order.push_back(key.to_owned());

            if self.order.len() > PAGE_CACHE
                && let Some(oldest) = self.order.pop_front()
            {
                self.items.remove(&oldest);
            }
        }
    }

    fn clear(&mut self) {
        self.items.clear();
        self.order.clear();
    }

    fn remove(&mut self, key: &str) {
        if self.items.remove(key).is_some() {
            self.order.retain(|other| other != key);
        }
    }
}

#[derive(Clone)]
struct AlbumData {
    album: Album,
    tracks: Vec<Track>,
    /// The `OLAK5uy_` playlist that stands for the album when it is saved.
    audio_playlist: Option<String>,
}

/// The account's lists, kept until the library changes.
#[derive(Default)]
struct AccountLists {
    playlists: Option<Vec<Playlist>>,
    albums: Option<Vec<Album>>,
    artists: Option<Vec<Artist>>,
}

/// What the account has liked, to answer hearts without a request each.
#[derive(Default)]
struct Liked {
    songs: HashSet<String>,
    albums: HashSet<String>,
}

/// The id of the Liked Music playlist.
pub const LIKED_PLAYLIST: &str = "LM";
/// The most liked songs walked to fill the hearts.
const LIKED_LIMIT: usize = 5_000;

#[derive(Clone)]
struct ArtistData {
    artist: Artist,
    top: Vec<Track>,
    albums: Vec<Album>,
    singles: Vec<Album>,
    albums_more: Option<yt::BrowseRef>,
    singles_more: Option<yt::BrowseRef>,
}

#[derive(Clone)]
struct PlaylistData {
    playlist: Playlist,
    tracks: Vec<Track>,
    continuation: Option<String>,
}

/// Tracks of a playlist loaded so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistTracks {
    pub tracks: Vec<Track>,
    pub has_more: bool,
}

/// Something ready to play. `context` is `None` for a lone track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedContext {
    pub context: Option<ContextInfo>,
    pub tracks: Vec<Track>,
}

struct State {
    index: HashMap<String, Track>,
    albums: Bounded<AlbumData>,
    artists: Bounded<ArtistData>,
    releases: Bounded<Vec<Album>>,
    playlists: Bounded<PlaylistData>,
    lists: AccountLists,
    liked: Liked,
}

pub struct YoutubeCatalog {
    client: Client,
    local: Arc<LocalLibrary>,
    state: Mutex<State>,
    /// Bumped when the library that shows changes, so a load that started
    /// before does not fill the new one.
    generation: AtomicU64,
    liked_ready: AtomicBool,
    liked_gate: tokio::sync::Mutex<()>,
    /// Playlist continuations are followed one request at a time.
    paging: tokio::sync::Mutex<()>,
}

fn video_id(uri_or_id: &str) -> &str {
    uri_or_id.strip_prefix("ytm:track:").unwrap_or(uri_or_id)
}

fn kind_of(uri: &str) -> String {
    if radio::is_station(uri) {
        return "unknown".to_owned();
    }

    uri.split(':').nth(1).unwrap_or("playlist").to_owned()
}

fn page_of<T>(items: Vec<T>) -> Page<T> {
    Page {
        total: Some(items.len() as u32),
        items,
        ..Page::default()
    }
}

impl YoutubeCatalog {
    pub fn new(client: Client, local: Arc<LocalLibrary>) -> Arc<Self> {
        Arc::new(Self {
            client,
            local,
            state: Mutex::new(State {
                index: HashMap::new(),
                albums: Bounded::new(),
                artists: Bounded::new(),
                releases: Bounded::new(),
                playlists: Bounded::new(),
                lists: AccountLists::default(),
                liked: Liked::default(),
            }),
            generation: AtomicU64::new(0),
            liked_ready: AtomicBool::new(false),
            liked_gate: tokio::sync::Mutex::new(()),
            paging: tokio::sync::Mutex::new(()),
        })
    }

    pub fn local(&self) -> &Arc<LocalLibrary> {
        &self.local
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn signed_in(&self) -> bool {
        self.client.has_session()
    }

    /// Another library is showing, so what was cached for the account goes.
    pub fn invalidate_account(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.liked_ready.store(false, Ordering::SeqCst);

        let mut state = lock(&self.state);

        state.lists = AccountLists::default();
        state.liked = Liked::default();
        state.playlists.remove(LIKED_PLAYLIST);
    }

    /// Starts reading what the account has liked, in the background.
    pub fn start_account_load(self: &Arc<Self>) {
        let catalog = self.clone();

        tokio::spawn(async move {
            if let Err(error) = catalog.ensure_liked().await {
                debug!(%error, "could not read the liked songs");
            }
        });
    }

    pub async fn account_playlists(&self) -> LibraryResult<Vec<Playlist>> {
        if let Some(lists) = lock(&self.state).lists.playlists.clone() {
            return Ok(lists);
        }

        let cards = self.client.library_playlists().await?;
        let playlists: Vec<Playlist> = cards
            .iter()
            .map(|card| convert::playlist(card, ""))
            .collect();

        lock(&self.state).lists.playlists = Some(playlists.clone());

        Ok(playlists)
    }

    pub async fn account_albums(&self) -> LibraryResult<Vec<Album>> {
        if let Some(lists) = lock(&self.state).lists.albums.clone() {
            return Ok(lists);
        }

        let cards = self.client.library_albums().await?;
        let albums: Vec<Album> = cards.iter().map(|card| convert::album(card, 0)).collect();

        lock(&self.state).lists.albums = Some(albums.clone());

        Ok(albums)
    }

    pub async fn account_artists(&self) -> LibraryResult<Vec<Artist>> {
        if let Some(lists) = lock(&self.state).lists.artists.clone() {
            return Ok(lists);
        }

        let cards = self.client.library_artists().await?;
        let artists: Vec<Artist> = cards.iter().map(convert::artist).collect();

        lock(&self.state).lists.artists = Some(artists.clone());

        Ok(artists)
    }

    /// Fills the sets of liked songs and saved albums. One load at a time;
    /// callers that arrive meanwhile wait for it.
    async fn ensure_liked(&self) -> LibraryResult<()> {
        if self.liked_ready.load(Ordering::SeqCst) {
            return Ok(());
        }

        let _gate = self.liked_gate.lock().await;

        if self.liked_ready.load(Ordering::SeqCst) {
            return Ok(());
        }

        let generation = self.generation.load(Ordering::SeqCst);
        let songs = self.playlist_tracks(LIKED_PLAYLIST, LIKED_LIMIT).await?;
        let albums = self.account_albums().await?;

        if generation != self.generation.load(Ordering::SeqCst) {
            return Ok(());
        }

        let mut state = lock(&self.state);

        state.liked.songs = songs
            .tracks
            .iter()
            .filter_map(|track| track.id.clone())
            .collect();
        state.liked.albums = albums.iter().map(|album| album.id.clone()).collect();
        drop(state);
        self.liked_ready.store(true, Ordering::SeqCst);

        Ok(())
    }

    /// Whether the account has liked the song or saved the album `uri`
    /// names. Waits for the first load.
    pub async fn is_liked(&self, uri: &str) -> LibraryResult<bool> {
        self.ensure_liked().await?;

        let state = lock(&self.state);

        Ok(match uri.split_once(':') {
            Some(("ytm", rest)) => match rest.split_once(':') {
                Some(("track", id)) => state.liked.songs.contains(id),
                Some(("album", id)) => state.liked.albums.contains(id),
                _ => false,
            },
            _ => false,
        })
    }

    /// Likes or unlikes a song, or saves or removes an album, on the account.
    pub async fn rate(&self, uri: &str, like: bool) -> LibraryResult<()> {
        let Some(("ytm", rest)) = uri.split_once(':') else {
            return Err(LibraryError::NotFound);
        };
        let (kind, id) = rest.split_once(':').ok_or(LibraryError::NotFound)?;

        self.ensure_liked().await?;

        match kind {
            "track" => {
                if like {
                    self.client.like_song(id).await?;
                } else {
                    self.client.unlike_song(id).await?;
                }

                let mut state = lock(&self.state);

                if like {
                    state.liked.songs.insert(id.to_owned());
                } else {
                    state.liked.songs.remove(id);
                }

                state.playlists.remove(LIKED_PLAYLIST);
            }
            "album" => {
                let playlist = self
                    .album_data(id)
                    .await?
                    .audio_playlist
                    .ok_or(LibraryError::Unsupported)?;

                if like {
                    self.client.like_playlist(&playlist).await?;
                } else {
                    self.client.unlike_playlist(&playlist).await?;
                }

                let mut state = lock(&self.state);

                if like {
                    state.liked.albums.insert(id.to_owned());
                } else {
                    state.liked.albums.remove(id);
                }

                state.lists.albums = None;
            }
            _ => return Err(LibraryError::Unsupported),
        }

        Ok(())
    }

    /// Forgets everything cached.
    pub fn clear(&self) {
        let mut state = lock(&self.state);

        state.index.clear();
        state.albums.clear();
        state.artists.clear();
        state.releases.clear();
        state.playlists.clear();
        state.lists = AccountLists::default();
        state.liked = Liked::default();
        self.liked_ready.store(false, Ordering::SeqCst);
    }

    pub(crate) fn remember(&self, tracks: &[Track]) {
        let mut state = lock(&self.state);

        if state.index.len() + tracks.len() > INDEX_LIMIT {
            state.index.clear();
        }

        for track in tracks {
            if let Some(id) = &track.id {
                state.index.insert(id.clone(), track.clone());
            }
        }
    }

    fn songs(&self, songs: &[yt::Song]) -> Vec<Track> {
        let tracks: Vec<Track> = songs.iter().map(convert::track).collect();

        self.remember(&tracks);

        tracks
    }

    /// `id` is a video id or a `ytm:track:` URI.
    pub async fn track(&self, id: &str) -> LibraryResult<Track> {
        let id = video_id(id);

        if let Some(track) = lock(&self.state).index.get(id).cloned() {
            return Ok(track);
        }

        let song = self.client.song(id).await?;
        let track = convert::track(&song);

        self.remember(std::slice::from_ref(&track));

        Ok(track)
    }

    async fn album_data(&self, id: &str) -> LibraryResult<AlbumData> {
        if let Some(data) = lock(&self.state).albums.get(id) {
            return Ok(data);
        }

        let page = self.client.album(id).await?;
        let tracks = self.songs(&page.tracks);
        let total = page.track_count.unwrap_or(tracks.len() as u32);
        let data = AlbumData {
            album: convert::album(&page.album, total),
            tracks,
            audio_playlist: page.audio_playlist_id.clone(),
        };

        lock(&self.state).albums.put(id, data.clone());

        Ok(data)
    }

    pub async fn album(&self, id: &str) -> LibraryResult<Album> {
        Ok(self.album_data(id).await?.album)
    }

    pub async fn album_tracks(&self, id: &str) -> LibraryResult<Vec<Track>> {
        Ok(self.album_data(id).await?.tracks)
    }

    async fn artist_data(&self, id: &str) -> LibraryResult<ArtistData> {
        if let Some(data) = lock(&self.state).artists.get(id) {
            return Ok(data);
        }

        let page = self.client.artist(id).await?;
        let albums = |cards: &[yt::AlbumCard]| -> Vec<Album> {
            cards.iter().map(|card| convert::album(card, 0)).collect()
        };
        let data = ArtistData {
            artist: convert::artist(&page.artist),
            top: self.songs(&page.top_songs),
            albums: albums(&page.albums),
            singles: albums(&page.singles),
            albums_more: page.albums_more.clone(),
            singles_more: page.singles_more.clone(),
        };

        lock(&self.state).artists.put(id, data.clone());

        Ok(data)
    }

    pub async fn artist(&self, id: &str) -> LibraryResult<Artist> {
        Ok(self.artist_data(id).await?.artist)
    }

    pub async fn artist_top_tracks(&self, id: &str) -> LibraryResult<Vec<Track>> {
        Ok(self.artist_data(id).await?.top)
    }

    /// Albums, then singles and EPs. When the page only shows a few, the full
    /// grid it links to is fetched.
    pub async fn artist_releases(&self, id: &str) -> LibraryResult<Vec<Album>> {
        if let Some(releases) = lock(&self.state).releases.get(id) {
            return Ok(releases);
        }

        let data = self.artist_data(id).await?;
        let full = |page: Vec<Album>, more: Option<yt::BrowseRef>| {
            let client = self.client.clone();
            let artist = data.artist.clone();

            async move {
                let Some(more) = more else {
                    return Ok::<_, LibraryError>(page);
                };
                let cards = client.artist_releases(&more).await?;

                Ok(cards
                    .iter()
                    .map(|card| {
                        let mut album = convert::album(card, 0);

                        if album.artists.is_empty() {
                            album.artists = vec![crate::model::ArtistRef {
                                id: Some(artist.id.clone()),
                                name: artist.name.clone(),
                            }];
                        }

                        album
                    })
                    .collect())
            }
        };
        let (albums, singles) = tokio::join!(
            full(data.albums.clone(), data.albums_more.clone()),
            full(data.singles.clone(), data.singles_more.clone())
        );
        let mut releases = albums.unwrap_or(data.albums);

        releases.extend(singles.unwrap_or(data.singles));
        lock(&self.state).releases.put(id, releases.clone());

        Ok(releases)
    }

    async fn playlist_data(&self, id: &str) -> LibraryResult<PlaylistData> {
        if let Some(data) = lock(&self.state).playlists.get(id) {
            return Ok(data);
        }

        let page = self.client.playlist(id).await?;
        let tracks = self.songs(&page.tracks);
        let data = PlaylistData {
            playlist: convert::playlist(&page.playlist, &page.description),
            tracks,
            continuation: page.continuation,
        };

        lock(&self.state).playlists.put(id, data.clone());

        Ok(data)
    }

    pub async fn playlist(&self, id: &str) -> LibraryResult<Playlist> {
        Ok(self.playlist_data(id).await?.playlist)
    }

    /// Follows continuations until `upto` tracks are loaded or the list ends.
    pub async fn playlist_tracks(&self, id: &str, upto: usize) -> LibraryResult<PlaylistTracks> {
        let _paging = self.paging.lock().await;
        let mut data = self.playlist_data(id).await?;

        while data.tracks.len() < upto
            && let Some(token) = data.continuation.clone()
        {
            let page = self.client.playlist_more(&token).await?;

            data.tracks.extend(self.songs(&page.tracks));
            data.continuation = page.continuation;
            lock(&self.state).playlists.put(id, data.clone());
        }

        Ok(PlaylistTracks {
            has_more: data.continuation.is_some(),
            tracks: data.tracks,
        })
    }

    pub async fn search(&self, query: &str, kinds: &[SearchKind]) -> LibraryResult<SearchResults> {
        let wants = |kind| kinds.contains(&kind);
        let (tracks, artists, albums, playlists) = (
            wants(SearchKind::Track),
            wants(SearchKind::Artist),
            wants(SearchKind::Album),
            wants(SearchKind::Playlist),
        );
        let mut found = yt::SearchResults::default();

        if tracks && artists && albums && playlists {
            found = self.client.search(query, None).await?;
        } else {
            let filters = [
                (tracks, SearchFilter::Songs),
                (artists, SearchFilter::Artists),
                (albums, SearchFilter::Albums),
                (playlists, SearchFilter::Playlists),
            ];
            let requests = filters
                .into_iter()
                .filter(|(wanted, _)| *wanted)
                .map(|(_, filter)| self.client.search(query, Some(filter)));

            for result in join_all(requests).await {
                let part = result?;

                found.songs.extend(part.songs);
                found.albums.extend(part.albums);
                found.artists.extend(part.artists);
                found.playlists.extend(part.playlists);
            }
        }

        let mut results = SearchResults::default();

        if tracks {
            results.tracks = page_of(self.songs(&found.songs));
        }

        if artists {
            results.artists = page_of(found.artists.iter().map(convert::artist).collect());
        }

        if albums {
            results.albums = page_of(
                found
                    .albums
                    .iter()
                    .map(|card| convert::album(card, 0))
                    .collect(),
            );
        }

        if playlists {
            results.playlists = page_of(
                found
                    .playlists
                    .iter()
                    .map(|card| convert::playlist(card, ""))
                    .collect(),
            );
        }

        Ok(results)
    }

    /// Trending and new releases from Explore, quick picks from Home. A call
    /// that fails only leaves its shelves out.
    pub async fn discover(&self) -> Vec<Shelf> {
        let (explore, home) = tokio::join!(self.client.explore(), self.client.home());
        let mut shelves = Vec::new();
        let first = |shelves: &[yt::HomeShelf], kind| {
            shelves.iter().find(|shelf| shelf.kind == kind).cloned()
        };

        if let Ok(explore) = explore {
            if let Some(shelf) = first(&explore, yt::ShelfKind::Songs) {
                shelves.push(self.shelf("trending", &shelf));
            }

            if let Some(shelf) = first(&explore, yt::ShelfKind::Albums) {
                shelves.push(self.shelf("new_releases", &shelf));
            }
        }

        if let Ok(home) = home
            && let Some(shelf) = first(&home, yt::ShelfKind::Songs)
        {
            shelves.push(self.shelf("quick_picks", &shelf));
        }

        shelves
    }

    fn shelf(&self, id: &str, shelf: &yt::HomeShelf) -> Shelf {
        let items = match shelf.kind {
            yt::ShelfKind::Albums => ShelfItems::Albums(
                shelf
                    .items
                    .iter()
                    .filter_map(|item| match item {
                        yt::HomeItem::Album(card) => Some(convert::album(card, 0)),
                        _ => None,
                    })
                    .collect(),
            ),
            _ => {
                let songs: Vec<yt::Song> = shelf
                    .items
                    .iter()
                    .filter_map(|item| match item {
                        yt::HomeItem::Song(song) => Some(song.clone()),
                        _ => None,
                    })
                    .collect();

                ShelfItems::Tracks(self.songs(&songs))
            }
        };

        Shelf {
            id: id.to_owned(),
            title: shelf.title.clone(),
            items,
        }
    }

    /// The tracks behind a URI the UI or the queue asks to play.
    pub async fn load_context(&self, uri: &str) -> LibraryResult<LoadedContext> {
        let info = |name: String, art: Artwork| {
            Some(ContextInfo {
                uri: uri.to_owned(),
                kind: kind_of(uri),
                name,
                art,
            })
        };

        if is_liked_context(uri) {
            let tracks = if self.signed_in() {
                let mut tracks = self
                    .playlist_tracks(LIKED_PLAYLIST, MAX_CONTEXT_TRACKS)
                    .await?
                    .tracks;

                tracks.truncate(MAX_CONTEXT_TRACKS);

                tracks
            } else {
                self.local.tracks()
            };

            return Self::non_empty(LoadedContext {
                context: info("Liked Songs".to_owned(), Artwork::default()),
                tracks,
            });
        }

        let Some(("ytm", rest)) = uri.split_once(':') else {
            return Err(LibraryError::NotFound);
        };

        if let Some(id) = rest.strip_prefix("station:track:") {
            let songs = self.client.radio(id).await?;

            return Self::non_empty(LoadedContext {
                context: info(String::new(), Artwork::default()),
                tracks: self.songs(&songs),
            });
        }

        let (kind, id) = rest.split_once(':').ok_or(LibraryError::NotFound)?;

        match kind {
            "track" => Ok(LoadedContext {
                context: None,
                tracks: vec![self.track(id).await?],
            }),
            "album" => {
                let data = self.album_data(id).await?;

                Self::non_empty(LoadedContext {
                    context: info(data.album.name, data.album.art),
                    tracks: data.tracks,
                })
            }
            "playlist" => {
                let playlist = self.playlist(id).await?;
                let loaded = self.playlist_tracks(id, MAX_CONTEXT_TRACKS).await?;
                let mut tracks = loaded.tracks;

                tracks.truncate(MAX_CONTEXT_TRACKS);

                Self::non_empty(LoadedContext {
                    context: info(playlist.name, playlist.art),
                    tracks,
                })
            }
            "artist" => {
                let data = self.artist_data(id).await?;

                Self::non_empty(LoadedContext {
                    context: info(data.artist.name, data.artist.art),
                    tracks: data.top,
                })
            }
            _ => Err(LibraryError::NotFound),
        }
    }

    fn non_empty(loaded: LoadedContext) -> LibraryResult<LoadedContext> {
        if loaded.tracks.is_empty() {
            return Err(LibraryError::NotFound);
        }

        Ok(loaded)
    }
}

#[cfg(test)]
pub(crate) mod tests;
