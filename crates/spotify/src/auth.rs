//! PKCE login, the one-shot loopback listener, and access token refresh.

use std::future::Future;
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng as _;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tracing::{debug, warn};
use url::Url;

use crate::error::{AuthError, SpotifyError};
use crate::secret::Secret;
use crate::{ACCOUNTS_BASE, REDIRECT_ADDR, REDIRECT_URI, SCOPES};

/// Access tokens are renewed this long before they expire.
pub const EXPIRY_MARGIN: Duration = Duration::from_secs(60);
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

const VERIFIER_LEN: usize = 64;
const UNRESERVED: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
const CALLBACK_PATH: &str = "/callback";
const CALLBACK_PAGE: &str = "Sonear is connected to Spotify. You can close this tab.";

/// Where the refresh token lives between runs.
pub trait TokenStore: Send + Sync {
    fn load(&self) -> Option<Secret>;

    fn save(&self, token: &Secret);

    fn clear(&self);
}

#[derive(Debug, Clone)]
pub struct LoginRequest {
    /// The page to open in the browser.
    pub url: String,
    pub state: String,
    verifier: Secret,
}

impl LoginRequest {
    pub fn verifier_len(&self) -> usize {
        self.verifier.expose().len()
    }
}

fn random_string(len: usize) -> String {
    let mut rng = rand::rng();

    (0..len)
        .map(|_| char::from(UNRESERVED[rng.random_range(0..UNRESERVED.len())]))
        .collect()
}

pub fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Builds the authorize URL with a fresh verifier and state.
pub fn begin(client_id: &str) -> LoginRequest {
    begin_with(client_id, random_string(VERIFIER_LEN), random_string(32))
}

pub(crate) fn begin_with(client_id: &str, verifier: String, state: String) -> LoginRequest {
    let mut url = Url::parse(&format!("{ACCOUNTS_BASE}/authorize")).expect("static URL");

    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("scope", SCOPES)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("state", &state)
        .append_pair("code_challenge_method", "S256")
        .append_pair("code_challenge", &challenge(&verifier));

    LoginRequest {
        url: url.into(),
        state,
        verifier: Secret::new(verifier),
    }
}

/// Binds the loopback port. Held only for the length of one login.
pub async fn bind_listener(addr: &str) -> Result<TcpListener, AuthError> {
    TcpListener::bind(addr).await.map_err(|error| {
        if error.kind() == io::ErrorKind::AddrInUse {
            AuthError::PortBusy
        } else {
            AuthError::Listen(error.to_string())
        }
    })
}

pub async fn bind_redirect_listener() -> Result<TcpListener, AuthError> {
    bind_listener(REDIRECT_ADDR).await
}

enum Callback {
    Ignore,
    Rejected,
    Code(String),
    Denied(String),
}

fn parse_callback(request_line: &str, expected_state: &str) -> (Callback, &'static str) {
    let mut parts = request_line.split_whitespace();
    let (Some("GET"), Some(target)) = (parts.next(), parts.next()) else {
        return (Callback::Ignore, "404 Not Found");
    };
    let Ok(url) = Url::parse(&format!("http://127.0.0.1{target}")) else {
        return (Callback::Ignore, "404 Not Found");
    };

    if url.path() != CALLBACK_PATH {
        return (Callback::Ignore, "404 Not Found");
    }

    let find = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };

    if find("state").as_deref() != Some(expected_state) {
        return (Callback::Rejected, "400 Bad Request");
    }

    if let Some(error) = find("error") {
        return (Callback::Denied(error), "200 OK");
    }

    match find("code") {
        Some(code) if !code.is_empty() => (Callback::Code(code), "200 OK"),
        _ => (Callback::Rejected, "400 Bad Request"),
    }
}

async fn read_request_line(stream: &mut TcpStream) -> Option<String> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];

    while buffer.len() < 8192 && !buffer.windows(2).any(|pair| pair == b"\r\n") {
        let read = stream.read(&mut chunk).await.ok()?;

        if read == 0 {
            break;
        }

        buffer.extend_from_slice(&chunk[..read]);
    }

    let text = String::from_utf8_lossy(&buffer);

    text.lines().next().map(str::to_owned)
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let text = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );

    let _ = stream.write_all(text.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Waits for the single redirect from Spotify and returns the code. Requests
/// for other paths or with a wrong `state` are answered and ignored.
pub async fn wait_for_callback(
    listener: TcpListener,
    expected_state: &str,
    timeout: Duration,
    cancel: impl Future<Output = ()>,
) -> Result<String, AuthError> {
    let serve = async {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                continue;
            };
            let line = tokio::time::timeout(Duration::from_secs(2), read_request_line(&mut stream))
                .await
                .ok()
                .flatten()
                .unwrap_or_default();
            let (outcome, status) = parse_callback(&line, expected_state);

            match outcome {
                Callback::Ignore => respond(&mut stream, status, "Not found").await,
                Callback::Rejected => {
                    respond(&mut stream, status, "This login link is not valid.").await;
                }
                Callback::Code(code) => {
                    respond(&mut stream, status, CALLBACK_PAGE).await;

                    return Ok(code);
                }
                Callback::Denied(reason) => {
                    respond(&mut stream, status, "Spotify did not allow the login.").await;

                    return Err(AuthError::Denied(reason));
                }
            }
        }
    };

    tokio::select! {
        result = serve => result,
        () = cancel => Err(AuthError::Cancelled),
        () = tokio::time::sleep(timeout) => Err(AuthError::TimedOut),
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
    refresh_token: Option<String>,
}

#[derive(Deserialize, Default)]
struct TokenError {
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
}

struct Access {
    token: Secret,
    expires_at: Instant,
}

#[derive(Default)]
struct State {
    access: Option<Access>,
    refresh: Option<Secret>,
}

/// Holds the access token in memory and renews it. Concurrent callers share
/// one refresh because the state lock is held across the request.
pub struct Auth {
    client_id: String,
    token_url: String,
    http: reqwest::Client,
    store: Arc<dyn TokenStore>,
    state: Mutex<State>,
}

impl Auth {
    pub fn new(client_id: impl Into<String>, store: Arc<dyn TokenStore>) -> Self {
        Self::with_token_url(client_id, store, format!("{ACCOUNTS_BASE}/api/token"))
    }

    pub fn with_token_url(
        client_id: impl Into<String>,
        store: Arc<dyn TokenStore>,
        token_url: impl Into<String>,
    ) -> Self {
        Self {
            client_id: client_id.into(),
            token_url: token_url.into(),
            http: crate::http_client(),
            store,
            state: Mutex::new(State::default()),
        }
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn has_refresh_token(&self) -> bool {
        self.store.load().is_some()
    }

    async fn request_token(&self, form: &[(&str, &str)]) -> Result<TokenResponse, SpotifyError> {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form)
            .finish();
        let response = self
            .http
            .post(&self.token_url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|error| SpotifyError::Network(error.without_url().to_string()))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|error| SpotifyError::Network(error.without_url().to_string()))?;

        if status.is_success() {
            return serde_json::from_slice(&bytes)
                .map_err(|error| SpotifyError::Decode(error.to_string()));
        }

        let error: TokenError = serde_json::from_slice(&bytes).unwrap_or_default();

        if error.error == "invalid_grant" {
            return Err(SpotifyError::NeedsLogin);
        }

        Err(SpotifyError::Api {
            status: status.as_u16(),
            message: if error.error_description.is_empty() {
                error.error
            } else {
                error.error_description
            },
        })
    }

    fn adopt(&self, state: &mut State, response: TokenResponse) -> Secret {
        let token = Secret::new(response.access_token);

        state.access = Some(Access {
            token: token.clone(),
            expires_at: Instant::now() + Duration::from_secs(response.expires_in),
        });

        if let Some(refresh) = response.refresh_token {
            let refresh = Secret::new(refresh);

            self.store.save(&refresh);
            state.refresh = Some(refresh);
        }

        token
    }

    async fn refresh_locked(&self, state: &mut State) -> Result<Secret, SpotifyError> {
        let Some(refresh) = state.refresh.clone().or_else(|| self.store.load()) else {
            return Err(SpotifyError::NeedsLogin);
        };
        let result = self
            .request_token(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh.expose()),
                ("client_id", &self.client_id),
            ])
            .await;

        match result {
            Ok(response) => {
                state.refresh = Some(refresh);

                Ok(self.adopt(state, response))
            }
            Err(SpotifyError::NeedsLogin) => {
                warn!("Spotify rejected the refresh token, login is needed");
                *state = State::default();
                self.store.clear();

                Err(SpotifyError::NeedsLogin)
            }
            Err(error) => Err(error),
        }
    }

    /// A valid access token, refreshed first if it is about to expire.
    pub async fn access_token(&self) -> Result<Secret, SpotifyError> {
        let mut state = self.state.lock().await;

        if let Some(access) = &state.access
            && access.expires_at > Instant::now() + EXPIRY_MARGIN
        {
            return Ok(access.token.clone());
        }

        self.refresh_locked(&mut state).await
    }

    /// Called after a 401 for `rejected`. Refreshes unless another caller
    /// already replaced that token.
    pub async fn renew_after_rejection(&self, rejected: &Secret) -> Result<Secret, SpotifyError> {
        let mut state = self.state.lock().await;

        if let Some(access) = &state.access
            && access.token != *rejected
            && access.expires_at > Instant::now() + EXPIRY_MARGIN
        {
            return Ok(access.token.clone());
        }

        self.refresh_locked(&mut state).await
    }

    pub async fn complete_login(
        &self,
        request: &LoginRequest,
        code: &str,
    ) -> Result<(), SpotifyError> {
        let mut state = self.state.lock().await;
        let response = self
            .request_token(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", REDIRECT_URI),
                ("client_id", &self.client_id),
                ("code_verifier", request.verifier.expose()),
            ])
            .await?;

        self.adopt(&mut state, response);
        debug!("Spotify login completed");

        Ok(())
    }

    /// Forgets the tokens in memory and in the store. Waits for a refresh in
    /// flight, whose new token would otherwise be saved after the delete.
    pub async fn sign_out(&self) {
        *self.state.lock().await = State::default();
        self.store.clear();
    }
}

#[cfg(test)]
pub(crate) mod tests;
