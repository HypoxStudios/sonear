use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::client::tests::client_for;

const ID: &str = "4uLU6hMCjMI75M1A2tKUQC";

#[tokio::test]
async fn search_limit_is_never_above_ten() {
    let server = MockServer::start().await;
    let client = client_for(&server).await;

    Mock::given(path("/v1/search"))
        .and(query_param("limit", "10"))
        .and(query_param("type", "track,artist,album,playlist,show"))
        .and(query_param("q", "daft punk"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&server)
        .await;

    client
        .search(
            "daft punk",
            &[
                SearchType::Track,
                SearchType::Artist,
                SearchType::Album,
                SearchType::Playlist,
                SearchType::Show,
            ],
            50,
            0,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn artist_albums_limit_is_never_above_ten() {
    let server = MockServer::start().await;
    let client = client_for(&server).await;

    Mock::given(path("/v1/artists/4uLU6hMCjMI75M1A2tKUQC/albums"))
        .and(query_param("limit", "10"))
        .and(query_param("offset", "20"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&server)
        .await;

    client.artist_albums(ID, 50, 20).await.unwrap();
}

#[tokio::test]
async fn saving_says_its_empty_body_has_no_length() {
    let server = MockServer::start().await;
    let client = client_for(&server).await;

    Mock::given(method("PUT"))
        .and(path("/v1/me/library"))
        .and(header("content-length", "0"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    client
        .library_save(&["spotify:track:a".to_owned()])
        .await
        .unwrap();
}

#[tokio::test]
async fn contains_and_save_are_chunked_at_forty() {
    let server = MockServer::start().await;
    let client = client_for(&server).await;
    let uris: Vec<String> = (0..95).map(|n| format!("spotify:track:{n}")).collect();

    Mock::given(method("GET"))
        .and(path("/v1/me/library/contains"))
        .respond_with(|request: &wiremock::Request| {
            let uris = request
                .url
                .query_pairs()
                .find(|(key, _)| key == "uris")
                .map(|(_, value)| value.split(',').count())
                .unwrap();

            assert!(uris <= 40);

            ResponseTemplate::new(200).set_body_json(vec![true; uris])
        })
        .expect(3)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/v1/me/library"))
        .respond_with(ResponseTemplate::new(200))
        .expect(3)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/me/library"))
        .respond_with(ResponseTemplate::new(200))
        .expect(3)
        .mount(&server)
        .await;

    assert_eq!(client.library_contains(&uris).await.unwrap().len(), 95);

    client.library_save(&uris).await.unwrap();
    client.library_remove(&uris).await.unwrap();
}

#[tokio::test]
async fn play_sends_the_offset_to_the_chosen_device() {
    let server = MockServer::start().await;
    let client = client_for(&server).await;

    Mock::given(method("PUT"))
        .and(path("/v1/me/player/play"))
        .and(query_param("device_id", "dev1"))
        .and(body_json(serde_json::json!({
            "context_uri": "spotify:album:a",
            "offset": {"position": 4}
        })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/v1/me/player/play"))
        .and(query_param("device_id", "dev1"))
        .and(body_json(serde_json::json!({
            "uris": ["spotify:track:b", "spotify:track:c"],
            "offset": {"uri": "spotify:track:c"}
        })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    client
        .play(
            &PlayParams {
                context_uri: Some("spotify:album:a".into()),
                offset: Some(Offset::Position(4)),
                ..PlayParams::default()
            },
            "dev1",
        )
        .await
        .unwrap();
    client
        .play(
            &PlayParams {
                uris: Some(vec!["spotify:track:b".into(), "spotify:track:c".into()]),
                offset: Some(Offset::Uri("spotify:track:c".into())),
                ..PlayParams::default()
            },
            "dev1",
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn recently_played_passes_the_after_cursor() {
    let server = MockServer::start().await;
    let client = client_for(&server).await;

    Mock::given(path("/v1/me/player/recently-played"))
        .and(query_param("limit", "50"))
        .and(query_param("after", "1793997716424"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})))
        .expect(1)
        .mount(&server)
        .await;

    client
        .recently_played(80, Some(1_793_997_716_424))
        .await
        .unwrap();
}

#[tokio::test]
async fn playlist_items_not_exposed_is_forbidden() {
    let server = MockServer::start().await;
    let client = client_for(&server).await;

    Mock::given(path("/v1/playlists/4uLU6hMCjMI75M1A2tKUQC/items"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "error": {"status": 403, "message": "Forbidden"}
        })))
        .expect(1)
        .mount(&server)
        .await;

    assert_eq!(
        client.playlist_items(ID, 50, 0).await.unwrap_err(),
        SpotifyError::Forbidden
    );
}

#[tokio::test]
async fn a_bad_id_fails_before_any_request() {
    let server = MockServer::start().await;
    let client = client_for(&server).await;
    let bad = [
        "",
        "../me",
        "4uLU6hMCjMI75M1A2tKUQ?",
        "4uLU6hMCjMI75M1A2tKUQ",
        "4uLU6hMCjMI75M1A2tKUQCC",
        "4uLU6hMCjMI75M1A2tKU/C",
    ];

    for id in bad {
        let expected = SpotifyError::InvalidId(id.to_owned());

        assert_eq!(client.track(id).await.unwrap_err(), expected);
        assert_eq!(client.album(id).await.unwrap_err(), expected);
        assert_eq!(client.album_tracks(id, 1, 0).await.unwrap_err(), expected);
        assert_eq!(client.artist(id).await.unwrap_err(), expected);
        assert_eq!(client.artist_albums(id, 1, 0).await.unwrap_err(), expected);
        assert_eq!(client.artist_top_tracks(id).await.unwrap_err(), expected);
        assert_eq!(client.playlist(id).await.unwrap_err(), expected);
        assert_eq!(client.playlist_items(id, 1, 0).await.unwrap_err(), expected);
        assert_eq!(client.show(id).await.unwrap_err(), expected);
        assert_eq!(client.show_episodes(id, 1, 0).await.unwrap_err(), expected);
    }

    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
}
