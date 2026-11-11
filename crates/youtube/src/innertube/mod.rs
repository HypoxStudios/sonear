//! The anonymous YouTube Music InnerTube client and its parsers.

pub mod parse;
pub mod thumbs;

use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::{Method, StatusCode, header};
use serde_json::{Map, Value, json};
use tokio::sync::Semaphore;
use tracing::debug;
use url::Url;

use crate::auth::Session;
use crate::error::YoutubeError;
use crate::models::{
    AccountInfo, AlbumCard, AlbumPage, ArtistCard, ArtistPage, BrowseRef, HomeItem, HomeShelf,
    PlaylistCard, PlaylistPage, SearchResults, Song, TrackPage,
};

pub const BASE_URL: &str = "https://music.youtube.com/youtubei/v1/";
pub const CLIENT_VERSION: &str = "1.20261101.01.00";
pub const MAX_CONCURRENT: usize = 4;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/130.0 Safari/537.36";
const ORIGIN: &str = "https://music.youtube.com";
const MAX_ID_LEN: usize = 128;
const MAX_TOKEN_LEN: usize = 4096;
/// Pages of a library list followed before it is cut off.
const MAX_LIBRARY_PAGES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchFilter {
    Songs,
    Albums,
    Artists,
    Playlists,
}

impl SearchFilter {
    fn params(self) -> &'static str {
        match self {
            Self::Songs => "EgWKAQIIAWoMEA4QChADEAQQCRAF",
            Self::Albums => "EgWKAQIYAWoMEA4QChADEAQQCRAF",
            Self::Artists => "EgWKAQIgAWoMEA4QChADEAQQCRAF",
            Self::Playlists => "EgWKAQIoAWoMEA4QChADEAQQCRAF",
        }
    }
}

struct Inner {
    http: reqwest::Client,
    base: String,
    language: String,
    permits: Semaphore,
    session: RwLock<Option<Session>>,
}

#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

fn network(error: reqwest::Error) -> YoutubeError {
    YoutubeError::Network(error.without_url().to_string())
}

fn valid_video_id(id: &str) -> bool {
    id.len() == 11 && id.bytes().all(is_id_byte)
}

fn is_id_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_LEN && id.bytes().all(is_id_byte)
}

fn valid_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= MAX_TOKEN_LEN
        && token
            .bytes()
            .all(|byte| is_id_byte(byte) || matches!(byte, b'%' | b'=' | b'.' | b'+' | b'/' | b'~'))
}

fn check(valid: bool) -> Result<(), YoutubeError> {
    valid.then_some(()).ok_or(YoutubeError::NotFound)
}

impl Client {
    pub fn new(language: &str) -> Self {
        Self::with_base(language, BASE_URL)
    }

    pub fn with_base(language: &str, base: &str) -> Self {
        let mut base = base.to_owned();

        if !base.ends_with('/') {
            base.push('/');
        }

        let mut headers = header::HeaderMap::new();

        headers.insert(header::ORIGIN, header::HeaderValue::from_static(ORIGIN));

        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .default_headers(headers)
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        Self {
            inner: Arc::new(Inner {
                http,
                base,
                language: language.to_owned(),
                permits: Semaphore::new(MAX_CONCURRENT),
                session: RwLock::new(None),
            }),
        }
    }

    /// Signs every request from now on, or none with `None`.
    pub fn set_session(&self, session: Option<Session>) {
        *self
            .inner
            .session
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = session;
    }

    pub fn has_session(&self) -> bool {
        self.session().is_some()
    }

    fn session(&self) -> Option<Session> {
        self.inner
            .session
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn require_session(&self) -> Result<(), YoutubeError> {
        self.has_session()
            .then_some(())
            .ok_or(YoutubeError::NeedsLogin)
    }

    async fn post(&self, endpoint: &str, fields: Value) -> Result<Value, YoutubeError> {
        let mut url = Url::parse(&format!("{}{endpoint}", self.inner.base))
            .map_err(|error| YoutubeError::Network(error.to_string()))?;

        url.query_pairs_mut().append_pair("prettyPrint", "false");

        let mut body = Map::new();

        body.insert(
            "context".into(),
            json!({
                "client": {
                    "clientName": "WEB_REMIX",
                    "clientVersion": CLIENT_VERSION,
                    "hl": self.inner.language,
                    "gl": "US",
                }
            }),
        );

        if let Value::Object(fields) = fields {
            body.extend(fields);
        }

        let body =
            serde_json::to_vec(&body).map_err(|error| YoutubeError::Decode(error.to_string()))?;
        let _permit = self
            .inner
            .permits
            .acquire()
            .await
            .map_err(|_| YoutubeError::Network("the client was closed".into()))?;

        debug!(endpoint, "InnerTube request");

        let session = self.session();
        let mut request = self
            .inner
            .http
            .request(Method::POST, url)
            .header(header::CONTENT_TYPE, "application/json");

        if let Some(session) = &session {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs());

            request = request
                .header(header::COOKIE, session.cookie_header())
                .header("X-Goog-AuthUser", "0")
                .header("X-Origin", ORIGIN);

            if let Some(authorization) = session.authorization(now) {
                request = request.header(header::AUTHORIZATION, authorization);
            }
        }

        let response = request.body(body).send().await.map_err(network)?;
        let status = response.status();

        if status == StatusCode::NOT_FOUND {
            return Err(YoutubeError::NotFound);
        }

        if status == StatusCode::UNAUTHORIZED {
            return Err(YoutubeError::NeedsLogin);
        }

        let bytes = response.bytes().await.map_err(network)?;

        if !status.is_success() {
            return Err(YoutubeError::Api {
                status: status.as_u16(),
                message: error_message(&bytes),
            });
        }

        let answer: Value = serde_json::from_slice(&bytes)
            .map_err(|error| YoutubeError::Decode(error.to_string()))?;

        if session.is_some() && signed_out(&answer) {
            return Err(YoutubeError::NeedsLogin);
        }

        Ok(answer)
    }

    /// The signed-in account's name and photo.
    pub async fn account(&self) -> Result<AccountInfo, YoutubeError> {
        self.require_session()?;

        parse::account(&self.post("account/account_menu", json!({})).await?)
    }

    /// Every item of a library list, following its pages up to a cap.
    async fn library_items(&self, browse_id: &str) -> Result<Vec<HomeItem>, YoutubeError> {
        self.require_session()?;

        let mut items = Vec::new();
        let mut fields = json!({ "browseId": browse_id });

        for _ in 0..MAX_LIBRARY_PAGES {
            let (page, next) = parse::library_page(&self.post("browse", fields).await?)?;

            items.extend(page);

            let Some(token) = next.filter(|token| valid_token(token)) else {
                break;
            };

            fields = json!({ "continuation": token });
        }

        Ok(items)
    }

    /// The account's own playlists, not Liked Music or the episodes list.
    pub async fn library_playlists(&self) -> Result<Vec<PlaylistCard>, YoutubeError> {
        let items = self.library_items("FEmusic_liked_playlists").await?;

        Ok(items
            .into_iter()
            .filter_map(|item| match item {
                HomeItem::Playlist(card) if !matches!(card.playlist_id.as_str(), "LM" | "SE") => {
                    Some(card)
                }
                _ => None,
            })
            .collect())
    }

    pub async fn library_albums(&self) -> Result<Vec<AlbumCard>, YoutubeError> {
        let items = self.library_items("FEmusic_liked_albums").await?;

        Ok(items
            .into_iter()
            .filter_map(|item| match item {
                HomeItem::Album(card) => Some(card),
                _ => None,
            })
            .collect())
    }

    /// The artists the account subscribes to.
    pub async fn library_artists(&self) -> Result<Vec<ArtistCard>, YoutubeError> {
        let items = self.library_items("FEmusic_library_corpus_artists").await?;

        Ok(items
            .into_iter()
            .filter_map(|item| match item {
                HomeItem::Artist(card) => Some(card),
                _ => None,
            })
            .collect())
    }

    async fn rate(&self, endpoint: &str, target: Value) -> Result<(), YoutubeError> {
        self.require_session()?;
        self.post(endpoint, json!({ "target": target })).await?;

        Ok(())
    }

    pub async fn like_song(&self, video_id: &str) -> Result<(), YoutubeError> {
        check(valid_video_id(video_id))?;

        self.rate("like/like", json!({ "videoId": video_id })).await
    }

    pub async fn unlike_song(&self, video_id: &str) -> Result<(), YoutubeError> {
        check(valid_video_id(video_id))?;

        self.rate("like/removelike", json!({ "videoId": video_id }))
            .await
    }

    /// Saves a playlist, or an album through its audio playlist.
    pub async fn like_playlist(&self, playlist_id: &str) -> Result<(), YoutubeError> {
        check(valid_id(playlist_id))?;

        self.rate("like/like", json!({ "playlistId": playlist_id }))
            .await
    }

    pub async fn unlike_playlist(&self, playlist_id: &str) -> Result<(), YoutubeError> {
        check(valid_id(playlist_id))?;

        self.rate("like/removelike", json!({ "playlistId": playlist_id }))
            .await
    }

    pub async fn search(
        &self,
        query: &str,
        filter: Option<SearchFilter>,
    ) -> Result<SearchResults, YoutubeError> {
        if query.trim().is_empty() {
            return Ok(SearchResults::default());
        }

        let mut fields = json!({ "query": query });

        if let Some(filter) = filter {
            fields["params"] = filter.params().into();
        }

        parse::search(&self.post("search", fields).await?)
    }

    pub async fn album(&self, browse_id: &str) -> Result<AlbumPage, YoutubeError> {
        check(valid_id(browse_id))?;

        let root = self
            .post("browse", json!({ "browseId": browse_id }))
            .await?;

        parse::album(&root, browse_id)
    }

    pub async fn artist(&self, channel_id: &str) -> Result<ArtistPage, YoutubeError> {
        check(valid_id(channel_id))?;

        let root = self
            .post("browse", json!({ "browseId": channel_id }))
            .await?;

        parse::artist(&root, channel_id)
    }

    /// The full albums or singles grid an artist page links to.
    pub async fn artist_releases(
        &self,
        reference: &BrowseRef,
    ) -> Result<Vec<AlbumCard>, YoutubeError> {
        check(valid_id(&reference.browse_id))?;

        let mut fields = json!({ "browseId": reference.browse_id });

        if let Some(params) = &reference.params {
            check(valid_token(params))?;
            fields["params"] = params.as_str().into();
        }

        parse::artist_releases(&self.post("browse", fields).await?)
    }

    pub async fn playlist(&self, playlist_id: &str) -> Result<PlaylistPage, YoutubeError> {
        check(valid_id(playlist_id))?;

        let root = self
            .post("browse", json!({ "browseId": format!("VL{playlist_id}") }))
            .await?;

        parse::playlist(&root, playlist_id)
    }

    pub async fn playlist_more(&self, continuation: &str) -> Result<TrackPage, YoutubeError> {
        check(valid_token(continuation))?;

        let root = self
            .post("browse", json!({ "continuation": continuation }))
            .await?;

        parse::track_page(&root)
    }

    pub async fn home(&self) -> Result<Vec<HomeShelf>, YoutubeError> {
        let root = self
            .post("browse", json!({ "browseId": "FEmusic_home" }))
            .await?;

        parse::home(&root)
    }

    /// The new releases and trending shelves. `hl` must be a locale YouTube
    /// accepts here: `es-419` works where `es` is refused with HTTP 400.
    pub async fn explore(&self) -> Result<Vec<HomeShelf>, YoutubeError> {
        let root = self
            .post("browse", json!({ "browseId": "FEmusic_explore" }))
            .await?;

        parse::explore(&root)
    }

    pub async fn up_next(
        &self,
        video_id: &str,
        playlist_id: Option<&str>,
    ) -> Result<Vec<Song>, YoutubeError> {
        check(valid_video_id(video_id))?;

        let mut fields = json!({ "videoId": video_id, "isAudioOnly": true });

        if let Some(playlist_id) = playlist_id {
            check(valid_id(playlist_id))?;
            fields["playlistId"] = playlist_id.into();
        }

        parse::up_next(&self.post("next", fields).await?)
    }

    pub async fn radio(&self, video_id: &str) -> Result<Vec<Song>, YoutubeError> {
        check(valid_video_id(video_id))?;

        self.up_next(video_id, Some(&format!("RDAMVM{video_id}")))
            .await
    }

    /// Looks the song up through the queue. The title can be the video's
    /// rather than the album track's.
    pub async fn song(&self, video_id: &str) -> Result<Song, YoutubeError> {
        self.up_next(video_id, None)
            .await?
            .into_iter()
            .find(|song| song.video_id == video_id)
            .ok_or(YoutubeError::NotFound)
    }
}

/// Whether the answer says the request was not signed in.
fn signed_out(answer: &Value) -> bool {
    answer
        .pointer("/responseContext/serviceTrackingParams")
        .and_then(Value::as_array)
        .is_some_and(|services| {
            services
                .iter()
                .filter_map(|service| service.get("params")?.as_array())
                .flatten()
                .any(|param| {
                    param.get("key").and_then(Value::as_str) == Some("logged_in")
                        && param.get("value").and_then(Value::as_str) == Some("0")
                })
        })
}

fn error_message(body: &[u8]) -> String {
    let message = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| String::from_utf8_lossy(body).into_owned());

    message.chars().take(200).collect()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod account_tests;
