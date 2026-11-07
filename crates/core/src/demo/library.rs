//! Answers the library questions from the fake catalogue.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::DemoBackend;
use super::catalog::{ARTISTS, DEMO_USER, NOT_OWNED_ID, catalog, episodes, shows};
use crate::library::{
    ARTIST_ALBUMS_PAGE, LibraryBackend, LibraryError, LibraryResult, PAGE_SIZE, SEARCH_PAGE,
};
use crate::model::{
    Album, Artist, Artwork, Device, Episode, Page, PlayRecord, Playlist, Profile, SearchKind,
    SearchResults, Show, Track, WebPlayRequest,
};
use crate::playback::now_ms;

const INITIAL_LIKED: usize = 24;
const DEVICE_ID: &str = "demo-device";

pub struct DemoLibrary {
    backend: Arc<DemoBackend>,
    device_name: String,
    /// Saved URIs, newest first.
    liked: Mutex<Vec<String>>,
}

impl DemoLibrary {
    pub fn new(backend: Arc<DemoBackend>, device_name: String) -> Arc<Self> {
        let liked = catalog()
            .tracks
            .iter()
            .take(INITIAL_LIKED)
            .filter(|track| track.index % 3 != 1)
            .rev()
            .map(|track| track.uri.clone())
            .chain((0..4).map(super::catalog::album_uri))
            .collect();

        Arc::new(Self {
            backend,
            device_name,
            liked: Mutex::new(liked),
        })
    }

    fn liked(&self) -> Vec<String> {
        self.liked
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

fn page_of<T: Clone>(items: &[T], offset: u32, size: u32) -> Page<T> {
    let start = (offset as usize).min(items.len());
    let end = (start + size as usize).min(items.len());

    Page {
        items: items[start..end].to_vec(),
        total: Some(items.len() as u32),
        offset,
        has_more: end < items.len(),
        cursor: None,
    }
}

fn track_models(indices: &[usize]) -> Vec<Track> {
    indices
        .iter()
        .map(|index| catalog().tracks[*index].model())
        .collect()
}

fn matches(text: &str, query: &str) -> bool {
    text.to_lowercase().contains(&query.to_lowercase())
}

#[async_trait]
impl LibraryBackend for DemoLibrary {
    async fn profile(&self) -> LibraryResult<Profile> {
        Ok(Profile {
            id: DEMO_USER.to_owned(),
            display_name: "Hypox".to_owned(),
            art: Artwork::default(),
        })
    }

    async fn my_playlists(&self, offset: u32) -> LibraryResult<Page<Playlist>> {
        let all: Vec<Playlist> = catalog()
            .playlists
            .iter()
            .map(|playlist| playlist.model())
            .collect();

        Ok(page_of(&all, offset, PAGE_SIZE))
    }

    async fn saved_albums(&self, offset: u32) -> LibraryResult<Page<Album>> {
        let liked = self.liked();
        let albums: Vec<Album> = (0..catalog().album_count())
            .filter(|index| liked.contains(&super::catalog::album_uri(*index)))
            .map(|index| catalog().album_model(index))
            .collect();

        Ok(page_of(&albums, offset, PAGE_SIZE))
    }

    async fn followed_artists(&self, _after: Option<String>) -> LibraryResult<Page<Artist>> {
        let artists: Vec<Artist> = (0..ARTISTS.len())
            .map(|index| catalog().artist_model(index))
            .collect();

        Ok(Page {
            total: Some(artists.len() as u32),
            items: artists,
            ..Page::default()
        })
    }

    async fn saved_shows(&self, offset: u32) -> LibraryResult<Page<Show>> {
        Ok(page_of(&shows(), offset, PAGE_SIZE))
    }

    async fn saved_tracks(&self, offset: u32) -> LibraryResult<Page<Track>> {
        let tracks: Vec<Track> = self
            .liked()
            .iter()
            .filter_map(|uri| catalog().track(uri))
            .map(|track| track.model())
            .collect();

        Ok(page_of(&tracks, offset, PAGE_SIZE))
    }

    async fn top_tracks(&self, limit: u32) -> LibraryResult<Vec<Track>> {
        Ok(track_models(
            &(0..limit.min(5) as usize).collect::<Vec<_>>(),
        ))
    }

    async fn recently_played(&self, limit: u32) -> LibraryResult<Vec<PlayRecord>> {
        let now = now_ms();
        let night_drive = "spotify:playlist:demo-night-drive";
        let weekend = format!("spotify:playlist:{NOT_OWNED_ID}");
        let plays = (0..40usize)
            .map(|n| {
                let (index, context) = match n {
                    0..=11 => (4 + n, Some(night_drive.to_owned())),
                    12..=17 => (30 + n, Some(super::catalog::album_uri(3))),
                    18..=21 => (50 + n, Some(weekend.clone())),
                    22..=29 => (60 + n, None),
                    _ => (n * 2, Some("spotify:playlist:demo-favorites".to_owned())),
                };

                PlayRecord {
                    played_at_ms: now - (n as u64 + 1) * 7 * 60_000,
                    track: catalog().tracks[index % catalog().tracks.len()].model(),
                    context_uri: context,
                }
            })
            .take(limit as usize)
            .collect();

        Ok(plays)
    }

    async fn track(&self, id: &str) -> LibraryResult<Track> {
        catalog()
            .track(&format!("spotify:track:{id}"))
            .map(|track| track.model())
            .ok_or(LibraryError::NotFound)
    }

    async fn album(&self, id: &str) -> LibraryResult<Album> {
        let index: usize = id
            .strip_prefix("demo-album-")
            .and_then(|index| index.parse().ok())
            .filter(|index| *index < catalog().album_count())
            .ok_or(LibraryError::NotFound)?;

        Ok(catalog().album_model(index))
    }

    async fn album_tracks(&self, id: &str, offset: u32) -> LibraryResult<Page<Track>> {
        let indices = catalog()
            .context_indices(&format!("spotify:album:{id}"))
            .ok_or(LibraryError::NotFound)?;

        Ok(page_of(&track_models(&indices), offset, PAGE_SIZE))
    }

    async fn artist(&self, id: &str) -> LibraryResult<Artist> {
        let index: usize = id
            .strip_prefix("demo-artist-")
            .and_then(|index| index.parse().ok())
            .filter(|index| *index < ARTISTS.len())
            .ok_or(LibraryError::NotFound)?;

        Ok(catalog().artist_model(index))
    }

    async fn artist_albums(&self, id: &str, offset: u32) -> LibraryResult<Page<Album>> {
        let index: usize = id
            .strip_prefix("demo-artist-")
            .and_then(|index| index.parse().ok())
            .ok_or(LibraryError::NotFound)?;
        let mut albums: Vec<usize> = catalog()
            .artist_indices(index)
            .iter()
            .map(|track| catalog().tracks[*track].album_index)
            .collect();

        albums.dedup();
        albums.sort_unstable();
        albums.dedup();

        let models: Vec<Album> = albums
            .into_iter()
            .map(|album| catalog().album_model(album))
            .collect();

        Ok(page_of(&models, offset, ARTIST_ALBUMS_PAGE))
    }

    async fn artist_top_tracks(&self, id: &str) -> LibraryResult<Vec<Track>> {
        let index: usize = id
            .strip_prefix("demo-artist-")
            .and_then(|index| index.parse().ok())
            .ok_or(LibraryError::NotFound)?;
        let indices: Vec<usize> = catalog()
            .artist_indices(index)
            .into_iter()
            .take(5)
            .collect();

        Ok(track_models(&indices))
    }

    async fn playlist(&self, id: &str) -> LibraryResult<Playlist> {
        catalog()
            .playlist(id)
            .map(|playlist| playlist.model())
            .ok_or(LibraryError::NotFound)
    }

    async fn playlist_tracks(&self, id: &str, offset: u32) -> LibraryResult<Page<Track>> {
        let playlist = catalog().playlist(id).ok_or(LibraryError::NotFound)?;

        if !playlist.owned() {
            return Err(LibraryError::Forbidden);
        }

        Ok(page_of(
            &track_models(&playlist.indices()),
            offset,
            PAGE_SIZE,
        ))
    }

    async fn show(&self, id: &str) -> LibraryResult<Show> {
        shows()
            .into_iter()
            .find(|show| show.id == id)
            .ok_or(LibraryError::NotFound)
    }

    async fn show_episodes(&self, id: &str, offset: u32) -> LibraryResult<Page<Episode>> {
        let show = shows()
            .into_iter()
            .find(|show| show.id == id)
            .ok_or(LibraryError::NotFound)?;

        Ok(page_of(&episodes(&show), offset, PAGE_SIZE))
    }

    async fn search(
        &self,
        query: &str,
        kinds: &[SearchKind],
        offset: u32,
    ) -> LibraryResult<SearchResults> {
        let wants = |kind| kinds.contains(&kind);
        let mut results = SearchResults::default();

        if wants(SearchKind::Track) {
            let tracks: Vec<Track> = catalog()
                .tracks
                .iter()
                .filter(|track| matches(&track.title, query) || matches(track.artist, query))
                .map(|track| track.model())
                .collect();

            results.tracks = page_of(&tracks, offset, SEARCH_PAGE);
        }

        if wants(SearchKind::Artist) {
            let artists: Vec<Artist> = (0..ARTISTS.len())
                .filter(|index| matches(ARTISTS[*index], query))
                .map(|index| catalog().artist_model(index))
                .collect();

            results.artists = page_of(&artists, offset, SEARCH_PAGE);
        }

        if wants(SearchKind::Album) {
            let albums: Vec<Album> = (0..catalog().album_count())
                .map(|index| catalog().album_model(index))
                .filter(|album| matches(&album.name, query))
                .collect();

            results.albums = page_of(&albums, offset, SEARCH_PAGE);
        }

        if wants(SearchKind::Playlist) {
            let playlists: Vec<Playlist> = catalog()
                .playlists
                .iter()
                .filter(|playlist| matches(playlist.name, query))
                .map(|playlist| playlist.model())
                .collect();

            results.playlists = page_of(&playlists, offset, SEARCH_PAGE);
        }

        if wants(SearchKind::Show) {
            let found: Vec<Show> = shows()
                .into_iter()
                .filter(|show| matches(&show.name, query))
                .collect();

            results.shows = page_of(&found, offset, SEARCH_PAGE);
        }

        Ok(results)
    }

    async fn contains(&self, uris: &[String]) -> LibraryResult<Vec<bool>> {
        let liked = self.liked();

        Ok(uris.iter().map(|uri| liked.contains(uri)).collect())
    }

    async fn save(&self, uris: &[String]) -> LibraryResult<()> {
        let mut liked = self
            .liked
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        for uri in uris {
            if !liked.contains(uri) {
                liked.insert(0, uri.clone());
            }
        }

        Ok(())
    }

    async fn remove(&self, uris: &[String]) -> LibraryResult<()> {
        self.liked
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|uri| !uris.contains(uri));

        Ok(())
    }

    async fn devices(&self) -> LibraryResult<Vec<Device>> {
        Ok(vec![Device {
            id: DEVICE_ID.to_owned(),
            name: self.device_name.clone(),
            active: true,
        }])
    }

    async fn start_playback(
        &self,
        request: &WebPlayRequest,
        _device_id: &str,
    ) -> LibraryResult<()> {
        self.backend
            .play_web(request.clone())
            .map_err(|_| LibraryError::Network("the demo player stopped".into()))
    }
}
