//! HTTP plumbing: auth header, 401 renewal, the shared rate-limit pause and
//! the concurrency cap. Endpoints live in `api`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::Method;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::{Semaphore, watch};
use tracing::{debug, warn};
use url::Url;

use crate::API_BASE;
use crate::auth::Auth;
use crate::error::SpotifyError;

pub const MAX_CONCURRENT: usize = 4;
/// Longest pause a request will sit out. Longer ones fail with `RateLimited`.
pub const MAX_WAIT: Duration = Duration::from_secs(30);
const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(5);

/// What the title bar shows about Spotify's limits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RateLimit {
    pub paused_until: Option<Instant>,
    pub quota_exceeded: bool,
}

struct Inner {
    http: reqwest::Client,
    base: Url,
    auth: Arc<Auth>,
    permits: Semaphore,
    pause: Mutex<Option<Instant>>,
    rate: watch::Sender<RateLimit>,
}

#[derive(Clone)]
pub struct SpotifyClient {
    inner: Arc<Inner>,
}

fn network(error: reqwest::Error) -> SpotifyError {
    SpotifyError::Network(error.without_url().to_string())
}

impl SpotifyClient {
    pub fn new(auth: Arc<Auth>) -> Self {
        Self::with_base(auth, API_BASE)
    }

    pub fn with_base(auth: Arc<Auth>, base: &str) -> Self {
        let mut base = base.to_owned();

        if !base.ends_with('/') {
            base.push('/');
        }

        Self {
            inner: Arc::new(Inner {
                http: crate::http_client(),
                base: Url::parse(&base).expect("valid API base URL"),
                auth,
                permits: Semaphore::new(MAX_CONCURRENT),
                pause: Mutex::new(None),
                rate: watch::channel(RateLimit::default()).0,
            }),
        }
    }

    pub fn auth(&self) -> &Arc<Auth> {
        &self.inner.auth
    }

    /// Changes whenever a pause starts or ends or the quota flag flips.
    pub fn rate_limit(&self) -> watch::Receiver<RateLimit> {
        self.inner.rate.subscribe()
    }

    fn pause_remaining(&self) -> Option<Duration> {
        let until = (*self
            .inner
            .pause
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()))?;

        until.checked_duration_since(Instant::now())
    }

    fn start_pause(&self, length: Duration) {
        let until = Instant::now() + length;

        {
            let mut pause = self
                .inner
                .pause
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());

            if pause.is_some_and(|existing| existing >= until) {
                return;
            }

            *pause = Some(until);
        }

        self.inner
            .rate
            .send_modify(|state| state.paused_until = Some(until));

        let client = self.clone();

        tokio::spawn(async move {
            tokio::time::sleep(length).await;

            client.inner.rate.send_modify(|state| {
                if state.paused_until == Some(until) {
                    state.paused_until = None;
                }
            });
        });
    }

    fn set_quota(&self, exceeded: bool) {
        self.inner.rate.send_if_modified(|state| {
            let changed = state.quota_exceeded != exceeded;

            state.quota_exceeded = exceeded;

            changed
        });
    }

    fn url(&self, path: &str, query: &[(&str, String)]) -> Result<Url, SpotifyError> {
        let mut url = self
            .inner
            .base
            .join(path.trim_start_matches('/'))
            .map_err(|error| SpotifyError::Decode(error.to_string()))?;

        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(key, value)| (*key, value.as_str())));
        }

        Ok(url)
    }

    /// Sends one request and returns the body of a success. A 204 is an empty body.
    pub async fn send(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<Vec<u8>, SpotifyError> {
        let url = self.url(path, query)?;
        let _permit = self
            .inner
            .permits
            .acquire()
            .await
            .map_err(|_| SpotifyError::Network("client closed".into()))?;
        let mut renewed = false;
        let mut waited = false;

        loop {
            if let Some(remaining) = self.pause_remaining() {
                if remaining > MAX_WAIT {
                    return Err(SpotifyError::RateLimited {
                        retry_after: remaining,
                    });
                }

                tokio::time::sleep(remaining).await;
            }

            let token = self.inner.auth.access_token().await?;
            let mut request = self
                .inner
                .http
                .request(method.clone(), url.clone())
                .bearer_auth(token.expose());

            if let Some(body) = body {
                request = request.json_body(body);
            } else if method != Method::GET {
                // Spotify answers 411 to a bodyless PUT that doesn't say so.
                request = request.header(reqwest::header::CONTENT_LENGTH, "0");
            }

            let response = request.send().await.map_err(network)?;
            let status = response.status().as_u16();
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse::<u64>().ok())
                .map_or(DEFAULT_RETRY_AFTER, Duration::from_secs);
            let bytes = response.bytes().await.map_err(network)?.to_vec();

            match status {
                200..=299 => {
                    self.set_quota(false);

                    return Ok(bytes);
                }
                401 if !renewed => {
                    renewed = true;
                    debug!("access token rejected, renewing");
                    self.inner.auth.renew_after_rejection(&token).await?;
                }
                401 => return Err(SpotifyError::NeedsLogin),
                403 => return Err(SpotifyError::Forbidden),
                404 => return Err(SpotifyError::NotFound),
                429 => {
                    if error_reason(&bytes).as_deref() == Some("QUOTA_EXCEEDED") {
                        warn!("Spotify quota exceeded");
                        self.set_quota(true);

                        return Err(SpotifyError::QuotaExceeded);
                    }

                    self.start_pause(retry_after);

                    if waited || retry_after > MAX_WAIT {
                        return Err(SpotifyError::RateLimited { retry_after });
                    }

                    waited = true;
                }
                status => {
                    return Err(SpotifyError::Api {
                        status,
                        message: error_message(&bytes),
                    });
                }
            }
        }
    }

    pub async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, SpotifyError> {
        let bytes = self.send(Method::GET, path, query, None).await?;

        decode(&bytes)
    }

    /// Like `get`, but an empty answer (HTTP 204) is `None`.
    pub async fn get_optional<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Option<T>, SpotifyError> {
        let bytes = self.send(Method::GET, path, query, None).await?;

        if bytes.is_empty() {
            return Ok(None);
        }

        decode(&bytes).map(Some)
    }
}

trait JsonBody {
    fn json_body(self, body: &Value) -> Self;
}

impl JsonBody for reqwest::RequestBuilder {
    fn json_body(self, body: &Value) -> Self {
        self.header("Content-Type", "application/json")
            .body(body.to_string())
    }
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, SpotifyError> {
    serde_json::from_slice(bytes).map_err(|error| SpotifyError::Decode(error.to_string()))
}

fn error_reason(bytes: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(bytes).ok()?;

    value
        .pointer("/error/reason")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn error_message(bytes: &[u8]) -> String {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return String::from_utf8_lossy(bytes).chars().take(200).collect();
    };

    value
        .pointer("/error/message")
        .or_else(|| value.pointer("/error_description"))
        .or_else(|| value.get("error"))
        .and_then(Value::as_str)
        .unwrap_or("unknown error")
        .to_owned()
}

#[cfg(test)]
pub(crate) mod tests;
