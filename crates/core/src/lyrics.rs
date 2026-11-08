//! Song lyrics from LRCLIB. Held in memory only, never written to disk.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::StatusCode;
use reqwest::header::USER_AGENT;
use serde::Deserialize;
use thiserror::Error;
use tracing::debug;
use url::Url;

const DEFAULT_BASE_URL: &str = "https://lrclib.net";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// How far a result's length may be from the playing track's.
const DURATION_TOLERANCE_SECS: f64 = 3.;
const CACHE_TRACKS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lyrics {
    Synced(Vec<LyricLine>),
    Plain(String),
    Instrumental,
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricLine {
    pub at_ms: u64,
    /// Empty for an instrumental gap.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricsQuery {
    pub uri: String,
    pub track: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Error)]
pub enum LyricsError {
    #[error("the lyrics request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("the lyrics service answered with status {0}")]
    Status(u16),
    #[error("the lyrics service sent something unreadable: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("the lyrics address is invalid: {0}")]
    Url(#[from] url::ParseError),
}

#[async_trait]
pub trait LyricsSource: Send + Sync {
    async fn fetch(&self, query: &LyricsQuery) -> Result<Lyrics, LyricsError>;
}

/// Parses LRC text. Lines without a timestamp, and metadata other than
/// `offset`, are dropped. Lines come back sorted by time.
pub fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    let mut offset_ms: i64 = 0;

    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut times = Vec::new();

        while let Some(tag) = rest.strip_prefix('[') {
            let Some((inner, after)) = tag.split_once(']') else {
                break;
            };

            if let Some(at) = parse_timestamp(inner) {
                times.push(at);
            } else if let Some(offset) = parse_offset(inner) {
                offset_ms = offset;
            }

            rest = after.trim_start();
        }

        if times.is_empty() {
            continue;
        }

        let content = strip_word_tags(rest);
        let content = content.trim();

        for at_ms in times {
            lines.push(LyricLine {
                at_ms,
                text: content.to_owned(),
            });
        }
    }

    if offset_ms != 0 {
        for line in &mut lines {
            line.at_ms = line.at_ms.saturating_add_signed(-offset_ms);
        }
    }

    lines.sort_by_key(|line| line.at_ms);

    lines
}

fn parse_timestamp(tag: &str) -> Option<u64> {
    let (minutes, seconds) = tag.split_once(':')?;

    if minutes.is_empty() || minutes.len() > 4 || !minutes.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }

    let (whole, fraction) = match seconds.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (seconds, None),
    };

    if whole.is_empty() || whole.len() > 2 || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }

    let millis = match fraction {
        None => 0,
        Some(fraction) => {
            if fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }

            let digits: String = fraction.chars().take(3).collect();
            let value: u64 = digits.parse().ok()?;

            value * 10_u64.pow(3 - digits.len() as u32)
        }
    };

    let minutes: u64 = minutes.parse().ok()?;
    let whole: u64 = whole.parse().ok()?;

    Some(minutes * 60_000 + whole * 1000 + millis)
}

fn parse_offset(tag: &str) -> Option<i64> {
    let (key, value) = tag.split_once(':')?;

    if !key.trim().eq_ignore_ascii_case("offset") {
        return None;
    }

    value.trim().parse().ok()
}

/// Drops enhanced-LRC word timestamps such as `<00:12.34>`.
fn strip_word_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start) = rest.find('<') {
        let Some(end) = rest[start..].find('>') else {
            break;
        };

        if parse_timestamp(&rest[start + 1..start + end]).is_none() {
            out.push_str(&rest[..start + end + 1]);
        } else {
            out.push_str(&rest[..start]);
        }

        rest = &rest[start + end + 1..];
    }

    out.push_str(rest);

    out
}

/// The line being sung at `position_ms`, or `None` before the first line.
pub fn active_line(lines: &[LyricLine], position_ms: u64) -> Option<usize> {
    lines
        .partition_point(|line| line.at_ms <= position_ms)
        .checked_sub(1)
}

/// When the active line next changes, or `None` after the last line starts.
pub fn next_change_ms(lines: &[LyricLine], position_ms: u64) -> Option<u64> {
    let next = lines.partition_point(|line| line.at_ms <= position_ms);

    lines.get(next).map(|line| line.at_ms)
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Record {
    duration: Option<f64>,
    instrumental: Option<bool>,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
}

impl Record {
    fn lyrics(self) -> Option<Lyrics> {
        if self.instrumental == Some(true) {
            return Some(Lyrics::Instrumental);
        }

        if let Some(synced) = self.synced_lyrics {
            let lines = parse_lrc(&synced);

            if lines.iter().any(|line| !line.text.is_empty()) {
                return Some(Lyrics::Synced(lines));
            }
        }

        self.plain_lyrics
            .map(|plain| plain.trim().to_owned())
            .filter(|plain| !plain.is_empty())
            .map(Lyrics::Plain)
    }

    fn matches_duration(&self, duration_ms: Option<u64>) -> bool {
        let Some(duration_ms) = duration_ms else {
            return true;
        };

        self.duration.is_some_and(|secs| {
            (secs - duration_ms as f64 / 1000.).abs() <= DURATION_TOLERANCE_SECS
        })
    }
}

pub struct LrcLibSource {
    client: reqwest::Client,
    base_url: String,
    user_agent: String,
}

fn user_agent() -> String {
    format!(
        "Sonear/{} (https://sonear.hypox.studio)",
        env!("CARGO_PKG_VERSION")
    )
}

impl LrcLibSource {
    pub fn new() -> Self {
        Self::with_base_url(DEFAULT_BASE_URL)
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::builder().build().unwrap_or_default(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            user_agent: user_agent(),
        }
    }

    async fn get_json(
        &self,
        path: &str,
        params: &[(&str, String)],
    ) -> Result<Option<Vec<u8>>, LyricsError> {
        let url = Url::parse_with_params(&format!("{}{path}", self.base_url), params)?;
        let response = self
            .client
            .get(url)
            .header(USER_AGENT, &self.user_agent)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await?;
        let status = response.status();

        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !status.is_success() {
            return Err(LyricsError::Status(status.as_u16()));
        }

        Ok(Some(response.bytes().await?.to_vec()))
    }

    async fn direct(
        &self,
        query: &LyricsQuery,
        duration_ms: u64,
    ) -> Result<Option<Lyrics>, LyricsError> {
        let params = [
            ("track_name", query.track.clone()),
            ("artist_name", query.artist.clone()),
            ("album_name", query.album.clone()),
            ("duration", ((duration_ms + 500) / 1000).to_string()),
        ];
        let Some(body) = self.get_json("/api/get", &params).await? else {
            return Ok(None);
        };

        Ok(serde_json::from_slice::<Record>(&body)?.lyrics())
    }

    async fn search(&self, query: &LyricsQuery) -> Result<Option<Lyrics>, LyricsError> {
        let params = [
            ("track_name", query.track.clone()),
            ("artist_name", query.artist.clone()),
        ];
        let Some(body) = self.get_json("/api/search", &params).await? else {
            return Ok(None);
        };
        let records: Vec<Record> = serde_json::from_slice(&body)?;

        let mut first = None;

        for lyrics in records
            .into_iter()
            .filter(|record| record.matches_duration(query.duration_ms))
            .filter_map(Record::lyrics)
        {
            if matches!(lyrics, Lyrics::Synced(_)) {
                return Ok(Some(lyrics));
            }

            first.get_or_insert(lyrics);
        }

        Ok(first)
    }
}

impl Default for LrcLibSource {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LyricsSource for LrcLibSource {
    async fn fetch(&self, query: &LyricsQuery) -> Result<Lyrics, LyricsError> {
        if query.track.trim().is_empty() {
            return Ok(Lyrics::NotFound);
        }

        if let Some(duration_ms) = query.duration_ms
            && let Some(found) = self.direct(query, duration_ms).await?
        {
            return Ok(found);
        }

        debug!("no direct lyrics match, searching");

        Ok(self.search(query).await?.unwrap_or(Lyrics::NotFound))
    }
}

/// The lyrics of the last few tracks, in memory only.
pub struct LyricsStore {
    source: Arc<dyn LyricsSource>,
    cache: Mutex<VecDeque<(String, Lyrics)>>,
}

impl LyricsStore {
    pub fn new(source: Arc<dyn LyricsSource>) -> Arc<Self> {
        Arc::new(Self {
            source,
            cache: Mutex::new(VecDeque::new()),
        })
    }

    /// Cached lyrics for the track, else a fetch. Failures are not cached.
    pub async fn get(&self, query: &LyricsQuery) -> Result<Lyrics, LyricsError> {
        if let Some(found) = self.cached(&query.uri) {
            return Ok(found);
        }

        let fetched = self.source.fetch(query).await?;

        self.remember(&query.uri, fetched.clone());

        Ok(fetched)
    }

    pub fn clear(&self) {
        self.lock().clear();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<(String, Lyrics)>> {
        self.cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn cached(&self, uri: &str) -> Option<Lyrics> {
        let mut cache = self.lock();
        let index = cache.iter().position(|(key, _)| key == uri)?;
        let entry = cache.remove(index)?;
        let lyrics = entry.1.clone();

        cache.push_back(entry);

        Some(lyrics)
    }

    fn remember(&self, uri: &str, lyrics: Lyrics) {
        let mut cache = self.lock();

        cache.retain(|(key, _)| key != uri);
        cache.push_back((uri.to_owned(), lyrics));

        while cache.len() > CACHE_TRACKS {
            cache.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn line(at_ms: u64, text: &str) -> LyricLine {
        LyricLine {
            at_ms,
            text: text.to_owned(),
        }
    }

    fn query(duration_ms: Option<u64>) -> LyricsQuery {
        LyricsQuery {
            uri: "spotify:track:one".into(),
            track: "Song".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            duration_ms,
        }
    }

    #[test]
    fn parses_timestamp_precisions() {
        let lines = parse_lrc("[00:01]a\n[00:02.5]b\n[00:03.25]c\n[01:04.125]d");

        assert_eq!(
            lines,
            vec![
                line(1000, "a"),
                line(2500, "b"),
                line(3250, "c"),
                line(64_125, "d")
            ]
        );
    }

    #[test]
    fn parses_several_timestamps_on_one_line() {
        let lines = parse_lrc("[00:10.00][00:30.00] chorus ");

        assert_eq!(lines, vec![line(10_000, "chorus"), line(30_000, "chorus")]);
    }

    #[test]
    fn applies_offset_and_ignores_other_metadata() {
        let lines = parse_lrc("[ar:Someone]\n[ti:Title]\n[offset:+500]\n[00:01.00]a\n[00:00.20]b");

        assert_eq!(lines, vec![line(0, "b"), line(500, "a")]);

        let later = parse_lrc("[offset:-250]\n[00:01.00]a");

        assert_eq!(later, vec![line(1250, "a")]);
    }

    #[test]
    fn keeps_empty_lines_and_sorts() {
        let lines = parse_lrc("[00:20.00]second\n[00:05.00]\n[00:10.00]first");

        assert_eq!(
            lines,
            vec![
                line(5000, ""),
                line(10_000, "first"),
                line(20_000, "second")
            ]
        );
    }

    #[test]
    fn skips_garbage_without_panicking() {
        let lines = parse_lrc(
            "hello\n[\n[00:xx]a\n[:5]b\n[99999:00]c\n]]\n[00:01.00\n\u{1F3B5}[00:02]\n[00:03]ok",
        );

        assert_eq!(lines, vec![line(3000, "ok")]);
        assert!(parse_lrc("").is_empty());
    }

    #[test]
    fn strips_word_timestamps() {
        let lines = parse_lrc("[00:01.00]<00:01.10>hello <00:01.50>world");

        assert_eq!(lines, vec![line(1000, "hello world")]);
    }

    #[test]
    fn active_line_edges() {
        let lines = vec![
            line(1000, "a"),
            line(2000, "b"),
            line(2000, "c"),
            line(5000, "d"),
        ];

        assert_eq!(active_line(&lines, 0), None);
        assert_eq!(active_line(&lines, 999), None);
        assert_eq!(active_line(&lines, 1000), Some(0));
        assert_eq!(active_line(&lines, 1999), Some(0));
        assert_eq!(active_line(&lines, 2000), Some(2));
        assert_eq!(active_line(&lines, 9000), Some(3));
        assert_eq!(active_line(&[], 100), None);
    }

    #[test]
    fn next_change_edges() {
        let lines = vec![line(1000, "a"), line(2000, "b")];

        assert_eq!(next_change_ms(&lines, 0), Some(1000));
        assert_eq!(next_change_ms(&lines, 1000), Some(2000));
        assert_eq!(next_change_ms(&lines, 1500), Some(2000));
        assert_eq!(next_change_ms(&lines, 2000), None);
        assert_eq!(next_change_ms(&[], 0), None);
    }

    async fn source(server: &MockServer) -> LrcLibSource {
        LrcLibSource::with_base_url(server.uri())
    }

    fn synced(secs: f64) -> serde_json::Value {
        json!({
            "duration": secs,
            "instrumental": false,
            "plainLyrics": "a\nb",
            "syncedLyrics": "[00:01.00]a\n[00:02.00]b",
        })
    }

    #[tokio::test]
    async fn direct_hit_sends_query_and_user_agent() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/get"))
            .and(query_param("track_name", "Song"))
            .and(query_param("artist_name", "Artist"))
            .and(query_param("album_name", "Album"))
            .and(query_param("duration", "200"))
            .and(header("user-agent", user_agent().as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(synced(200.)))
            .expect(1)
            .mount(&server)
            .await;

        let found = source(&server)
            .await
            .fetch(&query(Some(199_600)))
            .await
            .unwrap();

        assert_eq!(
            found,
            Lyrics::Synced(vec![line(1000, "a"), line(2000, "b")])
        );
    }

    #[tokio::test]
    async fn search_prefers_synced_lyrics_over_an_earlier_plain_result() {
        let server = MockServer::start().await;

        Mock::given(path("/api/get"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(path("/api/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "duration": 200.0, "plainLyrics": "plain" },
                synced(200.),
            ])))
            .mount(&server)
            .await;

        let found = source(&server)
            .await
            .fetch(&query(Some(200_000)))
            .await
            .unwrap();

        assert_eq!(
            found,
            Lyrics::Synced(vec![line(1000, "a"), line(2000, "b")])
        );
    }

    #[tokio::test]
    async fn falls_back_to_search_after_404() {
        let server = MockServer::start().await;

        Mock::given(path("/api/get"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(path("/api/search"))
            .and(query_param("track_name", "Song"))
            .and(query_param("artist_name", "Artist"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"duration": 260.0, "plainLyrics": "wrong"},
                {"duration": 201.5, "plainLyrics": "right"},
            ])))
            .mount(&server)
            .await;

        let found = source(&server)
            .await
            .fetch(&query(Some(200_000)))
            .await
            .unwrap();

        assert_eq!(found, Lyrics::Plain("right".into()));
    }

    #[tokio::test]
    async fn unknown_duration_searches_and_takes_the_first_with_lyrics() {
        let server = MockServer::start().await;

        Mock::given(path("/api/get"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(path("/api/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"duration": 10.0, "plainLyrics": null, "syncedLyrics": null},
                {"plainLyrics": "first"},
                {"plainLyrics": "second"},
            ])))
            .mount(&server)
            .await;

        let found = source(&server).await.fetch(&query(None)).await.unwrap();

        assert_eq!(found, Lyrics::Plain("first".into()));
    }

    #[tokio::test]
    async fn search_outside_tolerance_is_not_found() {
        let server = MockServer::start().await;

        Mock::given(path("/api/get"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(path("/api/search"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([{"duration": 204.0, "plainLyrics": "far"}])),
            )
            .mount(&server)
            .await;

        let found = source(&server)
            .await
            .fetch(&query(Some(200_000)))
            .await
            .unwrap();

        assert_eq!(found, Lyrics::NotFound);
    }

    #[tokio::test]
    async fn instrumental_and_null_fields() {
        let server = MockServer::start().await;

        Mock::given(path("/api/get"))
            .and(query_param("track_name", "Song"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"instrumental": true, "plainLyrics": null, "syncedLyrics": null, "duration": null}),
            ))
            .mount(&server)
            .await;

        let found = source(&server)
            .await
            .fetch(&query(Some(100_000)))
            .await
            .unwrap();

        assert_eq!(found, Lyrics::Instrumental);

        let empty = MockServer::start().await;

        Mock::given(path("/api/get"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"instrumental": null})))
            .mount(&empty)
            .await;
        Mock::given(path("/api/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{}])))
            .mount(&empty)
            .await;

        let nothing = source(&empty)
            .await
            .fetch(&query(Some(100_000)))
            .await
            .unwrap();

        assert_eq!(nothing, Lyrics::NotFound);
    }

    #[tokio::test]
    async fn server_errors_fail_and_are_not_cached() {
        let server = MockServer::start().await;

        Mock::given(path("/api/get"))
            .respond_with(ResponseTemplate::new(500))
            .expect(2)
            .mount(&server)
            .await;

        let store = LyricsStore::new(Arc::new(source(&server).await));
        let query = query(Some(200_000));

        assert!(matches!(
            store.get(&query).await,
            Err(LyricsError::Status(500))
        ));
        assert!(store.get(&query).await.is_err());
    }

    struct Counting {
        calls: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl LyricsSource for Counting {
        async fn fetch(&self, query: &LyricsQuery) -> Result<Lyrics, LyricsError> {
            self.calls.lock().unwrap().push(query.uri.clone());

            Ok(Lyrics::Plain(query.uri.clone()))
        }
    }

    fn numbered(index: usize) -> LyricsQuery {
        LyricsQuery {
            uri: format!("spotify:track:{index}"),
            ..query(None)
        }
    }

    #[tokio::test]
    async fn cache_hit_makes_no_second_request() {
        let source = Arc::new(Counting {
            calls: Mutex::new(Vec::new()),
        });
        let store = LyricsStore::new(source.clone());

        store.get(&numbered(1)).await.unwrap();
        store.get(&numbered(1)).await.unwrap();

        assert_eq!(source.calls.lock().unwrap().len(), 1);

        store.clear();
        store.get(&numbered(1)).await.unwrap();

        assert_eq!(source.calls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn least_recently_used_track_is_evicted() {
        let source = Arc::new(Counting {
            calls: Mutex::new(Vec::new()),
        });
        let store = LyricsStore::new(source.clone());

        for index in 0..CACHE_TRACKS {
            store.get(&numbered(index)).await.unwrap();
        }

        store.get(&numbered(0)).await.unwrap();
        store.get(&numbered(CACHE_TRACKS)).await.unwrap();

        assert_eq!(source.calls.lock().unwrap().len(), CACHE_TRACKS + 1);

        store.get(&numbered(0)).await.unwrap();
        assert_eq!(source.calls.lock().unwrap().len(), CACHE_TRACKS + 1);

        store.get(&numbered(1)).await.unwrap();
        assert_eq!(source.calls.lock().unwrap().len(), CACHE_TRACKS + 2);
    }
}
