//! The library questions answered from YouTube Music and the local files.

use std::sync::Arc;

use async_trait::async_trait;
use futures_util::StreamExt;
use futures_util::stream;

use super::account::{YoutubeAccount, YtAccountState};
use super::backend::YoutubeBackend;
use super::catalog::{LIKED_PLAYLIST, YoutubeCatalog};
use crate::config::ConfigHandle;
use crate::library::{LibraryBackend, LibraryError, LibraryResult, PAGE_SIZE};
use crate::model::{
    Album, Artist, Artwork, Device, Page, PlayOffset, PlayRecord, Playlist, Profile, SearchKind,
    SearchResults, Shelf, Show, Track, WebPlayRequest, split_uri,
};
use crate::player::PlayerError;

pub const DEVICE_ID: &str = "sonear-youtube";
const PROFILE_ID: &str = "local";
const LOOKUP_CONCURRENCY: usize = 4;

pub struct YoutubeLibrary {
    catalog: Arc<YoutubeCatalog>,
    backend: Arc<YoutubeBackend>,
    config: ConfigHandle,
    account: Arc<YoutubeAccount>,
}

impl YoutubeLibrary {
    async fn rate(&self, uris: &[String], like: bool) -> LibraryResult<()> {
        let catalog = self.catalog.clone();

        for uri in uris {
            self.account
                .retrying(|| {
                    let catalog = catalog.clone();
                    let uri = uri.clone();

                    async move { catalog.rate(&uri, like).await }
                })
                .await?;
        }

        Ok(())
    }

    pub fn new(
        catalog: Arc<YoutubeCatalog>,
        backend: Arc<YoutubeBackend>,
        config: ConfigHandle,
        account: Arc<YoutubeAccount>,
    ) -> Arc<Self> {
        Arc::new(Self {
            catalog,
            backend,
            config,
            account,
        })
    }
}

fn page_of<T: Clone>(items: &[T], offset: u32, has_more_beyond: bool) -> Page<T> {
    let start = (offset as usize).min(items.len());
    let end = (start + PAGE_SIZE as usize).min(items.len());

    Page {
        items: items[start..end].to_vec(),
        total: (!has_more_beyond).then_some(items.len() as u32),
        offset,
        has_more: end < items.len() || has_more_beyond,
        cursor: None,
    }
}

fn empty_page<T>() -> Page<T> {
    Page {
        total: Some(0),
        ..Page::default()
    }
}

fn start_index(offset: Option<&PlayOffset>, tracks: &[Track]) -> usize {
    match offset {
        Some(PlayOffset::Position(position)) => *position as usize,
        Some(PlayOffset::Uri(uri)) => tracks
            .iter()
            .position(|track| track.uri == *uri)
            .unwrap_or(0),
        None => 0,
    }
}

#[async_trait]
impl LibraryBackend for YoutubeLibrary {
    async fn profile(&self) -> LibraryResult<Profile> {
        let (display_name, art) = match self.account.state() {
            YtAccountState::SignedIn { name, photo } => (name, photo),
            _ => ("YT Music".to_owned(), Artwork::default()),
        };

        Ok(Profile {
            id: PROFILE_ID.to_owned(),
            display_name,
            art,
        })
    }

    async fn my_playlists(&self, offset: u32) -> LibraryResult<Page<Playlist>> {
        if !self.catalog.signed_in() {
            return Ok(empty_page());
        }

        let catalog = self.catalog.clone();
        let playlists = self
            .account
            .retrying(|| {
                let catalog = catalog.clone();

                async move { catalog.account_playlists().await }
            })
            .await?;

        Ok(page_of(&playlists, offset, false))
    }

    async fn saved_albums(&self, offset: u32) -> LibraryResult<Page<Album>> {
        if !self.catalog.signed_in() {
            return Ok(page_of(&self.catalog.local().albums(), offset, false));
        }

        let catalog = self.catalog.clone();
        let albums = self
            .account
            .retrying(|| {
                let catalog = catalog.clone();

                async move { catalog.account_albums().await }
            })
            .await?;

        Ok(page_of(&albums, offset, false))
    }

    async fn followed_artists(&self, _after: Option<String>) -> LibraryResult<Page<Artist>> {
        if !self.catalog.signed_in() {
            return Ok(empty_page());
        }

        let catalog = self.catalog.clone();
        let artists = self
            .account
            .retrying(|| {
                let catalog = catalog.clone();

                async move { catalog.account_artists().await }
            })
            .await?;

        Ok(Page {
            total: Some(artists.len() as u32),
            items: artists,
            ..Page::default()
        })
    }

    async fn saved_shows(&self, _offset: u32) -> LibraryResult<Page<Show>> {
        Ok(empty_page())
    }

    async fn saved_tracks(&self, offset: u32) -> LibraryResult<Page<Track>> {
        if !self.catalog.signed_in() {
            return Ok(page_of(&self.catalog.local().tracks(), offset, false));
        }

        let upto = offset as usize + PAGE_SIZE as usize;
        let catalog = self.catalog.clone();
        let loaded = self
            .account
            .retrying(|| {
                let catalog = catalog.clone();

                async move { catalog.playlist_tracks(LIKED_PLAYLIST, upto).await }
            })
            .await?;

        Ok(page_of(&loaded.tracks, offset, loaded.has_more))
    }

    async fn top_tracks(&self, _limit: u32) -> LibraryResult<Vec<Track>> {
        Ok(Vec::new())
    }

    async fn recently_played(&self, limit: u32) -> LibraryResult<Vec<PlayRecord>> {
        Ok(self.catalog.local().plays(limit))
    }

    async fn track(&self, id: &str) -> LibraryResult<Track> {
        self.catalog.track(id).await
    }

    async fn album(&self, id: &str) -> LibraryResult<Album> {
        self.catalog.album(id).await
    }

    async fn album_tracks(&self, id: &str, offset: u32) -> LibraryResult<Page<Track>> {
        Ok(page_of(
            &self.catalog.album_tracks(id).await?,
            offset,
            false,
        ))
    }

    async fn artist(&self, id: &str) -> LibraryResult<Artist> {
        self.catalog.artist(id).await
    }

    async fn artist_albums(&self, id: &str, offset: u32) -> LibraryResult<Page<Album>> {
        if offset > 0 {
            return Ok(Page {
                offset,
                ..Page::default()
            });
        }

        let albums = self.catalog.artist_releases(id).await?;

        Ok(Page {
            total: Some(albums.len() as u32),
            items: albums,
            ..Page::default()
        })
    }

    async fn artist_top_tracks(&self, id: &str) -> LibraryResult<Vec<Track>> {
        self.catalog.artist_top_tracks(id).await
    }

    async fn playlist(&self, id: &str) -> LibraryResult<Playlist> {
        self.catalog.playlist(id).await
    }

    async fn playlist_tracks(&self, id: &str, offset: u32) -> LibraryResult<Page<Track>> {
        let upto = offset as usize + PAGE_SIZE as usize;
        let loaded = self.catalog.playlist_tracks(id, upto).await?;

        Ok(page_of(&loaded.tracks, offset, loaded.has_more))
    }

    async fn show(&self, _id: &str) -> LibraryResult<Show> {
        Err(LibraryError::NotFound)
    }

    async fn show_episodes(
        &self,
        _id: &str,
        _offset: u32,
    ) -> LibraryResult<Page<crate::model::Episode>> {
        Err(LibraryError::NotFound)
    }

    async fn search(
        &self,
        query: &str,
        kinds: &[SearchKind],
        offset: u32,
    ) -> LibraryResult<SearchResults> {
        if offset > 0 {
            return Ok(SearchResults::default());
        }

        self.catalog.search(query, kinds).await
    }

    async fn contains(&self, uris: &[String]) -> LibraryResult<Vec<bool>> {
        if self.catalog.signed_in() {
            let catalog = self.catalog.clone();
            let mut found = Vec::with_capacity(uris.len());

            for uri in uris {
                let liked = self
                    .account
                    .retrying(|| {
                        let catalog = catalog.clone();
                        let uri = uri.clone();

                        async move { catalog.is_liked(&uri).await }
                    })
                    .await?;

                found.push(liked);
            }

            return Ok(found);
        }

        let local = self.catalog.local();

        Ok(uris.iter().map(|uri| local.contains(uri)).collect())
    }

    async fn save(&self, uris: &[String]) -> LibraryResult<()> {
        if self.catalog.signed_in() {
            return self.rate(uris, true).await;
        }

        let local = self.catalog.local();

        for uri in uris {
            let (kind, id) = split_uri(uri).ok_or(LibraryError::NotFound)?;

            match kind {
                "track" => local.save_track(self.catalog.track(id).await?),
                "album" => local.save_album(self.catalog.album(id).await?),
                _ => return Err(LibraryError::Unsupported),
            }
        }

        Ok(())
    }

    async fn remove(&self, uris: &[String]) -> LibraryResult<()> {
        if self.catalog.signed_in() {
            return self.rate(uris, false).await;
        }

        for uri in uris {
            self.catalog.local().remove(uri);
        }

        Ok(())
    }

    async fn discover(&self) -> LibraryResult<Vec<Shelf>> {
        Ok(self.catalog.discover().await)
    }

    async fn devices(&self) -> LibraryResult<Vec<Device>> {
        Ok(vec![Device {
            id: DEVICE_ID.to_owned(),
            name: self.config.get().device_name,
            active: true,
        }])
    }

    async fn start_playback(
        &self,
        request: &WebPlayRequest,
        _device_id: &str,
    ) -> LibraryResult<()> {
        let stopped = |_: PlayerError| LibraryError::Network("the player stopped".into());

        if let Some(context) = &request.context_uri {
            let loaded = self.catalog.load_context(context).await?;
            let start = start_index(request.offset.as_ref(), &loaded.tracks);

            return self
                .backend
                .play_list(loaded.tracks, start, loaded.context)
                .map_err(stopped);
        }

        let Some(uris) = &request.uris else {
            return Ok(());
        };
        let found: Vec<Option<Track>> = stream::iter(uris.iter().cloned())
            .map(|uri| {
                let catalog = self.catalog.clone();

                async move { catalog.track(&uri).await.ok() }
            })
            .buffered(LOOKUP_CONCURRENCY)
            .collect()
            .await;
        let wanted = match &request.offset {
            Some(PlayOffset::Position(position)) => Some(*position as usize),
            _ => None,
        };
        let skipped_before = wanted.map_or(0, |wanted| {
            found
                .iter()
                .take(wanted)
                .filter(|track| track.is_none())
                .count()
        });
        let tracks: Vec<Track> = found.into_iter().flatten().collect();

        if tracks.is_empty() {
            return Err(LibraryError::NotFound);
        }

        let start = match wanted {
            Some(wanted) => wanted.saturating_sub(skipped_before),
            None => start_index(request.offset.as_ref(), &tracks),
        };

        self.backend.play_list(tracks, start, None).map_err(stopped)
    }
}

#[cfg(test)]
mod tests;
