use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.json"));

    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn client(server: &MockServer) -> Client {
    Client::with_base("en", &format!("{}/youtubei/v1/", server.uri()))
}

fn session() -> Session {
    Session::from_header("SAPISID=sapisidvalue; HSID=other")
}

async fn answer(server: &MockServer, endpoint: &str, body: Value) {
    Mock::given(method("POST"))
        .and(path(format!("/youtubei/v1/{endpoint}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

async fn answer_browse(server: &MockServer, matcher: Value, status: u16, body: Value) {
    Mock::given(method("POST"))
        .and(path("/youtubei/v1/browse"))
        .and(body_partial_json(matcher))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

async fn mock_library(server: &MockServer) {
    for (id, name) in [
        ("FEmusic_liked_playlists", "library_playlists"),
        ("FEmusic_liked_albums", "library_albums"),
        ("FEmusic_library_corpus_artists", "library_subscriptions"),
    ] {
        answer_browse(server, json!({ "browseId": id }), 200, fixture(name)).await;
    }

    answer_browse(
        server,
        json!({ "continuation": "synthetic-library-token-1" }),
        200,
        fixture("library_playlists_more"),
    )
    .await;
}

#[test]
fn parses_the_account_menu() {
    let account = parse::account(&fixture("account_menu")).unwrap();

    assert_eq!(account.name, "Sample User");
    assert_eq!(account.photo.len(), 2);
    assert_eq!(account.photo[1].width, Some(88));
    assert!(matches!(
        parse::account(&json!({})),
        Err(YoutubeError::Decode(_))
    ));
}

#[test]
fn a_library_page_lists_items_and_the_next_token() {
    let (items, next) = parse::library_page(&fixture("library_playlists")).unwrap();
    let playlists = items
        .iter()
        .filter(|item| matches!(item, HomeItem::Playlist(_)))
        .count();

    assert_eq!(next.as_deref(), Some("synthetic-library-token-1"));
    assert_eq!(playlists, 3, "the new playlist tile is not an item");

    let (more, last) = parse::library_page(&fixture("library_playlists_more")).unwrap();

    assert_eq!(last, None);
    assert_eq!(more.len(), 1);
}

#[tokio::test]
async fn signed_requests_carry_the_cookies_and_the_hash() {
    let server = MockServer::start().await;
    let client = client(&server);

    answer(&server, "account/account_menu", fixture("account_menu")).await;
    client.set_session(Some(session()));

    assert!(client.has_session());
    assert_eq!(client.account().await.unwrap().name, "Sample User");

    let requests = server.received_requests().await.unwrap();
    let headers = &requests[0].headers;
    let value = |name: &str| headers.get(name).unwrap().to_str().unwrap().to_owned();

    assert_eq!(value("cookie"), "SAPISID=sapisidvalue; HSID=other");
    assert!(value("authorization").starts_with("SAPISIDHASH "));
    assert_eq!(value("x-goog-authuser"), "0");
    assert_eq!(value("x-origin"), "https://music.youtube.com");

    client.set_session(None);

    assert!(!client.has_session());
}

#[tokio::test]
async fn anonymous_requests_carry_no_credentials() {
    let server = MockServer::start().await;

    answer(&server, "browse", fixture("album")).await;
    client(&server).album("MPREb_K8qWMWVqXGi").await.unwrap();

    let requests = server.received_requests().await.unwrap();

    for name in ["cookie", "authorization", "x-goog-authuser"] {
        assert!(requests[0].headers.get(name).is_none(), "{name}");
    }
}

#[tokio::test]
async fn account_calls_need_a_session() {
    let server = MockServer::start().await;
    let client = client(&server);

    assert_eq!(client.account().await, Err(YoutubeError::NeedsLogin));
    assert_eq!(
        client.library_playlists().await,
        Err(YoutubeError::NeedsLogin)
    );
    assert_eq!(client.library_albums().await, Err(YoutubeError::NeedsLogin));
    assert_eq!(
        client.library_artists().await,
        Err(YoutubeError::NeedsLogin)
    );
    assert_eq!(
        client.like_song("IluRBvnYMoY").await,
        Err(YoutubeError::NeedsLogin)
    );
    assert_eq!(
        client.unlike_playlist("OLAK5uy_x").await,
        Err(YoutubeError::NeedsLogin)
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn the_library_lists_follow_continuations_and_skip_what_is_not_a_playlist() {
    let server = MockServer::start().await;
    let client = client(&server);

    mock_library(&server).await;
    client.set_session(Some(session()));

    let playlists = client.library_playlists().await.unwrap();
    let titles: Vec<_> = playlists.iter().map(|card| card.title.as_str()).collect();

    assert_eq!(titles, ["Synthetic Road Trip", "Synthetic Study Mix"]);
    assert_eq!(
        playlists[0].playlist_id,
        "PLsyntheticroadtrip0000000000000001"
    );
    assert_eq!(playlists[0].author.as_deref(), Some("Sample User"));

    let albums = client.library_albums().await.unwrap();

    assert_eq!(albums.len(), 2);
    assert_eq!(albums[0].browse_id, "MPREbSyntheticAlbumOne");
    assert_eq!(albums[1].kind, "Single");

    let artists = client.library_artists().await.unwrap();

    assert_eq!(artists.len(), 2);
    assert_eq!(artists[0].channel_id, "UCsyntheticband00000000001");
    assert_eq!(artists[0].name, "Sample Band");
}

#[tokio::test]
async fn a_401_or_a_signed_out_answer_means_needs_login() {
    let server = MockServer::start().await;
    let client = client(&server);

    answer_browse(
        &server,
        json!({ "browseId": "FEmusic_liked_albums" }),
        401,
        json!({}),
    )
    .await;
    answer_browse(
        &server,
        json!({ "browseId": "FEmusic_liked_playlists" }),
        200,
        fixture("signed_out"),
    )
    .await;
    client.set_session(Some(session()));

    assert_eq!(client.library_albums().await, Err(YoutubeError::NeedsLogin));
    assert_eq!(
        client.library_playlists().await,
        Err(YoutubeError::NeedsLogin)
    );
}

#[tokio::test]
async fn a_signed_out_answer_is_fine_for_an_anonymous_client() {
    let server = MockServer::start().await;
    let client = client(&server);

    answer_browse(
        &server,
        json!({ "browseId": "FEmusic_home" }),
        200,
        fixture("signed_out"),
    )
    .await;

    assert!(!matches!(
        client.home().await,
        Err(YoutubeError::NeedsLogin)
    ));
}

#[tokio::test]
async fn liking_posts_the_target() {
    let server = MockServer::start().await;
    let client = client(&server);

    answer(&server, "like/like", json!({})).await;
    answer(&server, "like/removelike", json!({})).await;
    client.set_session(Some(session()));
    client.like_song("IluRBvnYMoY").await.unwrap();
    client.unlike_song("IluRBvnYMoY").await.unwrap();
    client.like_playlist("OLAK5uy_synthetic").await.unwrap();
    client.unlike_playlist("OLAK5uy_synthetic").await.unwrap();

    let requests = server.received_requests().await.unwrap();
    let bodies: Vec<Value> = requests
        .iter()
        .map(|request| serde_json::from_slice(&request.body).unwrap())
        .collect();
    let paths: Vec<_> = requests.iter().map(|request| request.url.path()).collect();

    assert_eq!(
        paths,
        [
            "/youtubei/v1/like/like",
            "/youtubei/v1/like/removelike",
            "/youtubei/v1/like/like",
            "/youtubei/v1/like/removelike",
        ]
    );
    assert_eq!(bodies[0]["target"], json!({"videoId": "IluRBvnYMoY"}));
    assert_eq!(bodies[1]["target"], json!({"videoId": "IluRBvnYMoY"}));
    assert_eq!(
        bodies[2]["target"],
        json!({"playlistId": "OLAK5uy_synthetic"})
    );
    assert_eq!(
        bodies[3]["target"],
        json!({"playlistId": "OLAK5uy_synthetic"})
    );
    assert_eq!(
        client.like_song("bad id").await,
        Err(YoutubeError::NotFound)
    );
}
