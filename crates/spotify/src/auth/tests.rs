use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::net::TcpStream;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::*;

#[derive(Default)]
pub(crate) struct MemoryTokens {
    token: Mutex<Option<Secret>>,
    pub(crate) clears: AtomicUsize,
}

impl MemoryTokens {
    pub(crate) fn with(token: &str) -> Arc<Self> {
        let store = Self::default();

        *store.token.lock().unwrap() = Some(Secret::new(token));

        Arc::new(store)
    }

    pub(crate) fn current(&self) -> Option<String> {
        self.token
            .lock()
            .unwrap()
            .as_ref()
            .map(|token| token.expose().to_owned())
    }
}

impl TokenStore for MemoryTokens {
    fn load(&self) -> Option<Secret> {
        self.token.lock().unwrap().clone()
    }

    fn save(&self, token: &Secret) {
        *self.token.lock().unwrap() = Some(token.clone());
    }

    fn clear(&self) {
        self.clears.fetch_add(1, Ordering::SeqCst);
        *self.token.lock().unwrap() = None;
    }
}

pub(crate) fn token_json(
    access: &str,
    refresh: Option<&str>,
    expires_in: u64,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": expires_in,
        "scope": "x",
    });

    if let Some(refresh) = refresh {
        body["refresh_token"] = refresh.into();
    }

    body
}

pub(crate) fn auth_for(server: &MockServer, store: Arc<MemoryTokens>) -> Arc<Auth> {
    Arc::new(Auth::with_token_url(
        "client-1",
        store,
        format!("{}/api/token", server.uri()),
    ))
}

#[test]
fn verifier_is_long_enough_and_unreserved() {
    let request = begin("id");

    assert!((43..=128).contains(&request.verifier_len()));
    assert!(request.url.contains("code_challenge_method=S256"));
    assert!(request.url.contains("client_id=id"));
    assert!(
        request
            .url
            .contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A27196%2Fcallback")
    );
    assert!(request.url.contains("user-follow-read"));
}

#[test]
fn challenge_matches_the_rfc_7636_example() {
    assert_eq!(
        challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}

async fn hit(port: u16, target: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();

    stream
        .write_all(format!("GET {target} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
        .await
        .unwrap();

    let mut answer = String::new();

    stream.read_to_string(&mut answer).await.unwrap();

    answer
}

#[tokio::test]
async fn callback_returns_the_code_and_shows_a_plain_page() {
    let listener = bind_listener("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let waiting = tokio::spawn(wait_for_callback(
        listener,
        "s1",
        Duration::from_secs(5),
        std::future::pending(),
    ));
    let answer = hit(port, "/callback?code=abc&state=s1").await;

    assert!(answer.starts_with("HTTP/1.1 200"));
    assert!(answer.contains("close this tab"));
    assert_eq!(waiting.await.unwrap().unwrap(), "abc");
}

#[tokio::test]
async fn wrong_state_and_wrong_path_are_ignored_until_a_valid_request() {
    let listener = bind_listener("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let waiting = tokio::spawn(wait_for_callback(
        listener,
        "good",
        Duration::from_secs(5),
        std::future::pending(),
    ));

    assert!(
        hit(port, "/callback?code=evil&state=bad")
            .await
            .starts_with("HTTP/1.1 400")
    );
    assert!(
        hit(port, "/other?code=x&state=good")
            .await
            .starts_with("HTTP/1.1 404")
    );
    assert!(!waiting.is_finished());

    hit(port, "/callback?code=real&state=good").await;

    assert_eq!(waiting.await.unwrap().unwrap(), "real");
}

#[tokio::test]
async fn denied_login_reports_the_reason() {
    let listener = bind_listener("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let waiting = tokio::spawn(wait_for_callback(
        listener,
        "s",
        Duration::from_secs(5),
        std::future::pending(),
    ));

    hit(port, "/callback?error=access_denied&state=s").await;

    assert_eq!(
        waiting.await.unwrap(),
        Err(AuthError::Denied("access_denied".into()))
    );
}

#[tokio::test]
async fn login_can_be_cancelled_and_times_out() {
    let listener = bind_listener("127.0.0.1:0").await.unwrap();
    let cancelled = wait_for_callback(listener, "s", Duration::from_secs(5), async {}).await;

    assert_eq!(cancelled, Err(AuthError::Cancelled));

    let listener = bind_listener("127.0.0.1:0").await.unwrap();
    let timed_out = wait_for_callback(
        listener,
        "s",
        Duration::from_millis(50),
        std::future::pending(),
    )
    .await;

    assert_eq!(timed_out, Err(AuthError::TimedOut));
}

#[tokio::test]
async fn a_busy_port_is_a_clear_error() {
    let first = bind_listener("127.0.0.1:0").await.unwrap();
    let addr = first.local_addr().unwrap().to_string();

    assert_eq!(bind_listener(&addr).await.unwrap_err(), AuthError::PortBusy);
}

#[tokio::test]
async fn code_exchange_sends_the_verifier_and_stores_the_refresh_token() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code_verifier=ver"))
        .and(body_string_contains("client_id=client-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_json("a1", Some("r1"), 3600)))
        .expect(1)
        .mount(&server)
        .await;

    let store = Arc::new(MemoryTokens::default());
    let auth = auth_for(&server, store.clone());
    let request = begin_with("client-1", "ver".into(), "st".into());

    auth.complete_login(&request, "the-code").await.unwrap();

    assert_eq!(store.current().as_deref(), Some("r1"));
    assert_eq!(auth.access_token().await.unwrap().expose(), "a1");
}

#[tokio::test]
async fn signing_out_during_a_refresh_leaves_no_token_behind() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(150))
                .set_body_json(token_json("fresh", Some("r2"), 3600)),
        )
        .mount(&server)
        .await;

    let store = MemoryTokens::with("r1");
    let auth = auth_for(&server, store.clone());
    let refresh = tokio::spawn({
        let auth = auth.clone();

        async move { auth.access_token().await }
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    auth.sign_out().await;

    assert!(refresh.await.unwrap().is_ok());
    assert_eq!(store.current(), None);
    assert_eq!(auth.access_token().await, Err(SpotifyError::NeedsLogin));
}

#[tokio::test]
async fn concurrent_callers_share_one_refresh() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(150))
                .set_body_json(token_json("fresh", Some("r2"), 3600)),
        )
        .expect(1)
        .mount(&server)
        .await;

    let auth = auth_for(&server, MemoryTokens::with("r1"));
    let calls = (0..8).map(|_| {
        let auth = auth.clone();

        tokio::spawn(async move { auth.access_token().await.unwrap() })
    });

    for call in calls.collect::<Vec<_>>() {
        assert_eq!(call.await.unwrap().expose(), "fresh");
    }
}

#[tokio::test]
async fn a_token_close_to_expiry_is_refreshed_early() {
    let server = MockServer::start().await;
    let counter = Arc::new(AtomicUsize::new(0));
    let seen = counter.clone();

    Mock::given(method("POST"))
        .respond_with(move |_: &Request| {
            let n = seen.fetch_add(1, Ordering::SeqCst);

            ResponseTemplate::new(200).set_body_json(token_json(&format!("t{n}"), None, 30))
        })
        .mount(&server)
        .await;

    let auth = auth_for(&server, MemoryTokens::with("r1"));

    assert_eq!(auth.access_token().await.unwrap().expose(), "t0");
    assert_eq!(auth.access_token().await.unwrap().expose(), "t1");
}

#[tokio::test]
async fn a_missing_new_refresh_token_keeps_the_old_one() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_json("a", None, 3600)))
        .mount(&server)
        .await;

    let store = MemoryTokens::with("old");
    let auth = auth_for(&server, store.clone());

    auth.access_token().await.unwrap();

    assert_eq!(store.current().as_deref(), Some("old"));
}

#[tokio::test]
async fn invalid_grant_needs_login_and_removes_the_token() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": "invalid_grant",
            "error_description": "Refresh token revoked"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let store = MemoryTokens::with("old");
    let auth = auth_for(&server, store.clone());

    assert_eq!(
        auth.access_token().await.unwrap_err(),
        SpotifyError::NeedsLogin
    );
    assert_eq!(store.current(), None);
    assert_eq!(
        auth.access_token().await.unwrap_err(),
        SpotifyError::NeedsLogin
    );
}

#[tokio::test]
async fn no_refresh_token_means_login() {
    let server = MockServer::start().await;
    let auth = auth_for(&server, Arc::new(MemoryTokens::default()));

    assert_eq!(
        auth.access_token().await.unwrap_err(),
        SpotifyError::NeedsLogin
    );
}

#[test]
fn secrets_never_print() {
    assert_eq!(format!("{:?}", Secret::new("hunter2")), "[redacted]");
}
