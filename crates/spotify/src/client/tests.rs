use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use serde_json::Value;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::*;
use crate::auth::tests::{MemoryTokens, auth_for, token_json};

pub(crate) async fn client_for(server: &MockServer) -> SpotifyClient {
    Mock::given(method("POST"))
        .and(path("/api/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_json("tok", None, 3600)))
        .mount(server)
        .await;

    let auth = auth_for(server, MemoryTokens::with("refresh"));

    SpotifyClient::with_base(auth, &format!("{}/v1", server.uri()))
}

fn fixture(name: &str) -> (u16, Value) {
    let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let status = value["status"].as_u64().unwrap() as u16;

    (status, value["body"].take())
}

#[tokio::test]
async fn success_and_empty_bodies() {
    let server = MockServer::start().await;
    let client = client_for(&server).await;

    Mock::given(path("/v1/me/player"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    assert_eq!(client.player_state().await.unwrap(), None);
}

#[tokio::test]
async fn an_unauthorised_request_renews_the_token_and_retries_once() {
    let server = MockServer::start().await;
    let issued = Arc::new(AtomicUsize::new(0));
    let counter = issued.clone();

    Mock::given(method("POST"))
        .and(path("/api/token"))
        .respond_with(move |_: &Request| {
            let n = counter.fetch_add(1, Ordering::SeqCst);

            ResponseTemplate::new(200).set_body_json(token_json(&format!("t{n}"), None, 3600))
        })
        .mount(&server)
        .await;
    Mock::given(path("/v1/me"))
        .and(header("authorization", "Bearer t0"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/v1/me"))
        .and(header("authorization", "Bearer t1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "u"})))
        .expect(1)
        .mount(&server)
        .await;

    let auth = auth_for(&server, MemoryTokens::with("refresh"));
    let client = SpotifyClient::with_base(auth, &format!("{}/v1", server.uri()));

    assert_eq!(client.me().await.unwrap().id.as_deref(), Some("u"));
}

#[tokio::test]
async fn a_second_401_needs_login() {
    let server = MockServer::start().await;
    let client = client_for(&server).await;

    Mock::given(path("/v1/me"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    assert_eq!(client.me().await.unwrap_err(), SpotifyError::NeedsLogin);
}

#[tokio::test]
async fn a_429_pauses_everything_then_retries_once() {
    let server = MockServer::start().await;
    let client = client_for(&server).await;
    let limit = client.rate_limit();

    Mock::given(path("/v1/me"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(path("/v1/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "u"})))
        .mount(&server)
        .await;

    let started = Instant::now();
    let pending = tokio::spawn({
        let client = client.clone();

        async move { client.me().await }
    });

    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(limit.borrow().paused_until.is_some());

    assert!(pending.await.unwrap().is_ok());
    assert!(started.elapsed() >= Duration::from_millis(900));

    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(limit.borrow().paused_until.is_none());
}

#[tokio::test]
async fn the_pause_is_shared_by_other_requests() {
    let server = MockServer::start().await;
    let client = client_for(&server).await;

    Mock::given(path("/v1/me"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(path("/v1/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "u"})))
        .mount(&server)
        .await;
    Mock::given(path("/v1/me/player/devices"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"devices": []})))
        .mount(&server)
        .await;

    let first = tokio::spawn({
        let client = client.clone();

        async move { client.me().await }
    });

    tokio::time::sleep(Duration::from_millis(250)).await;

    let started = Instant::now();

    client.devices().await.unwrap();

    assert!(started.elapsed() >= Duration::from_millis(500));
    assert!(first.await.unwrap().is_ok());
}

#[tokio::test]
async fn a_repeated_429_is_rate_limited() {
    let server = MockServer::start().await;
    let client = client_for(&server).await;

    Mock::given(path("/v1/me"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .expect(2)
        .mount(&server)
        .await;

    assert_eq!(
        client.me().await.unwrap_err(),
        SpotifyError::RateLimited {
            retry_after: Duration::ZERO
        }
    );
}

#[tokio::test]
async fn quota_exhaustion_is_its_own_error_and_is_not_retried() {
    let server = MockServer::start().await;
    let client = client_for(&server).await;

    Mock::given(path("/v1/me"))
        .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
            "error": {"status": 429, "message": "quota", "reason": "QUOTA_EXCEEDED"}
        })))
        .expect(1)
        .mount(&server)
        .await;

    assert_eq!(client.me().await.unwrap_err(), SpotifyError::QuotaExceeded);
    assert!(client.rate_limit().borrow().quota_exceeded);
}

#[tokio::test]
async fn error_fixtures_map_to_distinct_errors() {
    let server = MockServer::start().await;
    let client = client_for(&server).await;
    let cases = [
        ("followed_playlist_items_forbidden", "playlists/p/items"),
        ("editorial_playlist_404", "playlists/e"),
        ("batch_tracks_forbidden", "tracks"),
        ("search_limit_11", "search"),
    ];

    for (name, route) in cases {
        let (status, body) = fixture(name);

        Mock::given(path(format!("/v1/{route}")))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&server)
            .await;
    }

    assert_eq!(
        client.playlist_items("p", 50, 0).await.unwrap_err(),
        SpotifyError::Forbidden
    );
    assert_eq!(
        client.playlist("e").await.unwrap_err(),
        SpotifyError::NotFound
    );
    assert_eq!(
        client.get::<Value>("tracks", &[]).await.unwrap_err(),
        SpotifyError::Forbidden
    );
    assert_eq!(
        client.get::<Value>("search", &[]).await.unwrap_err(),
        SpotifyError::Api {
            status: 400,
            message: "Invalid limit".into()
        }
    );
}

#[tokio::test]
async fn undecodable_answers_are_decode_errors() {
    let server = MockServer::start().await;
    let client = client_for(&server).await;

    Mock::given(path("/v1/me"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&server)
        .await;

    assert!(matches!(
        client.me().await.unwrap_err(),
        SpotifyError::Decode(_)
    ));
}

#[tokio::test]
async fn at_most_four_requests_run_at_once() {
    let server = MockServer::start().await;
    let client = client_for(&server).await;

    Mock::given(path("/v1/me"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(300))
                .set_body_json(serde_json::json!({"id": "u"})),
        )
        .mount(&server)
        .await;

    let started = Instant::now();
    let calls: Vec<_> = (0..8)
        .map(|_| {
            let client = client.clone();

            tokio::spawn(async move { client.me().await })
        })
        .collect();

    for call in calls {
        call.await.unwrap().unwrap();
    }

    assert!(started.elapsed() >= Duration::from_millis(580));
}
