use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

pub const ALBUM: &str = include_str!("../../../../youtube/tests/fixtures/album.json");
pub const ARTIST: &str = include_str!("../../../../youtube/tests/fixtures/artist.json");
pub const RELEASES: &str = include_str!("../../../../youtube/tests/fixtures/artist_releases.json");
pub const PLAYLIST_LONG: &str =
    include_str!("../../../../youtube/tests/fixtures/playlist_long.json");
pub const PLAYLIST_MORE: &str =
    include_str!("../../../../youtube/tests/fixtures/playlist_more.json");
pub const SEARCH_ALL: &str = include_str!("../../../../youtube/tests/fixtures/search_all.json");
pub const SEARCH_SONGS: &str = include_str!("../../../../youtube/tests/fixtures/search_songs.json");
pub const SEARCH_ALBUMS: &str =
    include_str!("../../../../youtube/tests/fixtures/search_albums.json");
pub const EXPLORE: &str = include_str!("../../../../youtube/tests/fixtures/explore.json");
pub const HOME: &str = include_str!("../../../../youtube/tests/fixtures/home.json");
pub const NEXT_RADIO: &str = include_str!("../../../../youtube/tests/fixtures/next_radio.json");
pub const NEXT_SINGLE: &str = include_str!("../../../../youtube/tests/fixtures/next_single.json");

pub const RAM: &str = "MPREb_K8qWMWVqXGi";
pub const LONG_PLAYLIST: &str = "PLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI";

pub fn catalog(server: &MockServer) -> Arc<YoutubeCatalog> {
    YoutubeCatalog::new(
        Client::with_base("en", &format!("{}/youtubei/v1/", server.uri())),
        LocalLibrary::in_memory(),
    )
}

pub async fn mock(server: &MockServer, endpoint: &str, matcher: Value, fixture: &str) {
    Mock::given(method("POST"))
        .and(path(format!("/youtubei/v1/{endpoint}")))
        .and(body_partial_json(matcher))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(fixture.as_bytes().to_vec(), "application/json"),
        )
        .mount(server)
        .await;
}

pub async fn mock_playlist_pages(server: &MockServer) {
    mock(
        server,
        "browse",
        json!({"browseId": format!("VL{LONG_PLAYLIST}")}),
        PLAYLIST_LONG,
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/youtubei/v1/browse"))
        .and(body_string_contains("\"continuation\""))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(PLAYLIST_MORE.as_bytes().to_vec(), "application/json"),
        )
        .mount(server)
        .await;
}

async fn bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| serde_json::from_slice(&request.body).unwrap())
        .collect()
}

#[tokio::test]
async fn an_album_gives_its_tracks_and_a_total() {
    let server = MockServer::start().await;
    let catalog = catalog(&server);

    mock(&server, "browse", json!({"browseId": RAM}), ALBUM).await;

    let album = catalog.album(RAM).await.unwrap();
    let tracks = catalog.album_tracks(RAM).await.unwrap();

    assert_eq!(album.name, "Random Access Memories");
    assert_eq!(album.uri, format!("ytm:album:{RAM}"));
    assert_eq!(album.total_tracks, 13);
    assert_eq!(tracks[0].uri, "ytm:track:IluRBvnYMoY");
    assert_eq!(tracks[0].name, "Give Life Back to Music");
    assert_eq!(bodies(&server).await.len(), 1, "the page is cached");
}

#[tokio::test]
async fn a_track_is_found_in_the_index_before_the_network() {
    let server = MockServer::start().await;
    let catalog = catalog(&server);

    mock(&server, "browse", json!({"browseId": RAM}), ALBUM).await;
    catalog.album_tracks(RAM).await.unwrap();

    let track = catalog.track("ytm:track:IluRBvnYMoY").await.unwrap();

    assert_eq!(track.name, "Give Life Back to Music");
    assert_eq!(bodies(&server).await.len(), 1);

    mock(
        &server,
        "next",
        json!({"videoId": "IluRBvnYMoY"}),
        NEXT_SINGLE,
    )
    .await;

    let fresh = self::catalog(&server);

    assert_eq!(
        fresh.track("IluRBvnYMoY").await.unwrap().name,
        "Give Life Back to Music (Official Audio)"
    );
    assert_eq!(
        fresh.track("nonexistent1").await,
        Err(LibraryError::NotFound)
    );
}

#[tokio::test]
async fn playlist_paging_follows_the_continuation_only_as_far_as_asked() {
    let server = MockServer::start().await;
    let catalog = catalog(&server);

    mock_playlist_pages(&server).await;

    let first = catalog.playlist_tracks(LONG_PLAYLIST, 2).await.unwrap();

    assert_eq!(first.tracks.len(), 3);
    assert!(first.has_more);
    assert_eq!(bodies(&server).await.len(), 1);

    let all = catalog.playlist_tracks(LONG_PLAYLIST, 50).await.unwrap();

    assert_eq!(all.tracks.len(), 6);
    assert!(!all.has_more);
    assert_eq!(all.tracks[4].uri, "ytm:track:Ke9A6XV2H38");
    assert_eq!(bodies(&server).await.len(), 2);

    let again = catalog.playlist_tracks(LONG_PLAYLIST, 50).await.unwrap();

    assert_eq!(again.tracks.len(), 6);
    assert_eq!(bodies(&server).await.len(), 2);

    let playlist = catalog.playlist(LONG_PLAYLIST).await.unwrap();

    assert_eq!(playlist.name, "Popular Music Videos");
    assert_eq!(playlist.track_count, Some(182));
    assert_eq!(playlist.owner_name.as_deref(), Some("Music"));
}

#[tokio::test]
async fn an_artist_gets_top_songs_and_the_full_release_grids() {
    let server = MockServer::start().await;
    let catalog = catalog(&server);
    let id = "UCRr1xG_2WIDs18a6cIiCxeA";

    mock(&server, "browse", json!({"browseId": id}), ARTIST).await;
    mock(
        &server,
        "browse",
        json!({"browseId": format!("MPAD{id}")}),
        RELEASES,
    )
    .await;

    assert_eq!(catalog.artist(id).await.unwrap().name, "Daft Punk");
    assert_eq!(catalog.artist_top_tracks(id).await.unwrap().len(), 5);

    let releases = catalog.artist_releases(id).await.unwrap();

    assert_eq!(releases.len(), 12, "the albums grid twice, once per kind");
    assert!(
        releases
            .iter()
            .all(|album| album.artists[0].name == "Daft Punk")
    );

    let grids: Vec<_> = bodies(&server)
        .await
        .into_iter()
        .filter_map(|body| body["params"].as_str().map(str::to_owned))
        .collect();

    assert_eq!(grids.len(), 2);
}

#[tokio::test]
async fn an_unfiltered_search_is_one_request_and_a_filtered_one_asks_per_kind() {
    let server = MockServer::start().await;
    let catalog = catalog(&server);

    mock(
        &server,
        "search",
        json!({"params": "EgWKAQIIAWoMEA4QChADEAQQCRAF"}),
        SEARCH_SONGS,
    )
    .await;
    mock(
        &server,
        "search",
        json!({"params": "EgWKAQIYAWoMEA4QChADEAQQCRAF"}),
        SEARCH_ALBUMS,
    )
    .await;
    mock(&server, "search", json!({"query": "daft punk"}), SEARCH_ALL).await;

    let everything = [
        SearchKind::Track,
        SearchKind::Artist,
        SearchKind::Album,
        SearchKind::Playlist,
        SearchKind::Show,
    ];
    let all = catalog.search("daft punk", &everything).await.unwrap();

    assert!(!all.tracks.items.is_empty() && !all.artists.items.is_empty());
    assert!(!all.albums.items.is_empty() && !all.playlists.items.is_empty());
    assert!(all.shows.items.is_empty());
    assert!(all.tracks.items[0].uri.starts_with("ytm:track:"));

    let sent = bodies(&server).await;

    assert_eq!(sent.len(), 1);
    assert!(sent[0].get("params").is_none());

    let some = catalog
        .search("daft punk", &[SearchKind::Track, SearchKind::Album])
        .await
        .unwrap();
    let sent = bodies(&server).await;

    assert_eq!(sent.len(), 3);
    assert!(!some.tracks.items.is_empty() && !some.albums.items.is_empty());
    assert!(some.artists.items.is_empty() && some.playlists.items.is_empty());
    assert_eq!(some.albums.items[0].name, "Random Access Memories");

    let none = catalog
        .search("daft punk", &[SearchKind::Show])
        .await
        .unwrap();

    assert_eq!(none, SearchResults::default());
    assert_eq!(bodies(&server).await.len(), 3);
}

#[tokio::test]
async fn discover_builds_the_three_shelves_and_skips_what_fails() {
    let server = MockServer::start().await;
    let catalog = catalog(&server);

    mock(
        &server,
        "browse",
        json!({"browseId": "FEmusic_explore"}),
        EXPLORE,
    )
    .await;
    mock(&server, "browse", json!({"browseId": "FEmusic_home"}), HOME).await;

    let shelves = catalog.discover().await;
    let ids: Vec<_> = shelves.iter().map(|shelf| shelf.id.as_str()).collect();

    assert_eq!(ids, ["trending", "new_releases", "quick_picks"]);
    assert_eq!(shelves[0].title, "Trending");
    assert_eq!(shelves[1].title, "New albums & singles");
    assert!(matches!(&shelves[0].items, ShelfItems::Tracks(tracks) if !tracks.is_empty()));
    assert!(matches!(&shelves[1].items, ShelfItems::Albums(albums) if !albums.is_empty()));
    assert!(
        matches!(&shelves[2].items, ShelfItems::Tracks(tracks) if tracks[0].uri == "ytm:track:xukbqwRuN5w")
    );

    let broken = MockServer::start().await;

    mock(&broken, "browse", json!({"browseId": "FEmusic_home"}), HOME).await;

    let ids: Vec<_> = self::catalog(&broken)
        .discover()
        .await
        .into_iter()
        .map(|shelf| shelf.id)
        .collect();

    assert_eq!(ids, ["quick_picks"]);
}

#[tokio::test]
async fn contexts_load_their_tracks() {
    let server = MockServer::start().await;
    let catalog = catalog(&server);

    mock(&server, "browse", json!({"browseId": RAM}), ALBUM).await;
    mock_playlist_pages(&server).await;
    mock(
        &server,
        "next",
        json!({"playlistId": "RDAMVMIluRBvnYMoY"}),
        NEXT_RADIO,
    )
    .await;
    mock(
        &server,
        "next",
        json!({"videoId": "IluRBvnYMoY"}),
        NEXT_SINGLE,
    )
    .await;

    let album = catalog
        .load_context(&format!("ytm:album:{RAM}"))
        .await
        .unwrap();
    let info = album.context.unwrap();

    assert_eq!(info.uri, format!("ytm:album:{RAM}"));
    assert_eq!(info.kind, "album");
    assert_eq!(info.name, "Random Access Memories");
    assert!(!info.art.is_empty());
    assert_eq!(album.tracks.len(), 6);

    let playlist = catalog
        .load_context(&format!("ytm:playlist:{LONG_PLAYLIST}"))
        .await
        .unwrap();

    assert_eq!(playlist.tracks.len(), 6);
    assert_eq!(playlist.context.unwrap().kind, "playlist");

    let station = catalog
        .load_context("ytm:station:track:IluRBvnYMoY")
        .await
        .unwrap();
    let info = station.context.unwrap();

    assert_eq!(station.tracks.len(), 6);
    assert_eq!(info.kind, "unknown");
    assert!(info.name.is_empty());
    assert_eq!(station.tracks[0].uri, "ytm:track:5NV6Rdv1a3I");

    let single = catalog.load_context("ytm:track:IluRBvnYMoY").await.unwrap();

    assert!(single.context.is_none());
    assert_eq!(single.tracks.len(), 1);
}

#[tokio::test]
async fn the_liked_context_is_the_local_library() {
    let server = MockServer::start().await;
    let catalog = catalog(&server);
    let uri = crate::model::liked_context_uri("local");

    assert_eq!(
        catalog.load_context(&uri).await,
        Err(LibraryError::NotFound)
    );

    catalog.local().save_track(Track {
        uri: "ytm:track:a".into(),
        id: Some("a".into()),
        name: "A".into(),
        ..Track::default()
    });

    let liked = catalog.load_context(&uri).await.unwrap();

    assert_eq!(liked.context.unwrap().name, "Liked Songs");
    assert_eq!(liked.tracks[0].uri, "ytm:track:a");
    assert!(bodies(&server).await.is_empty());
}

#[tokio::test]
async fn contexts_that_make_no_sense_are_not_found() {
    let server = MockServer::start().await;
    let catalog = catalog(&server);

    for uri in [
        "spotify:album:abc",
        "ytm:show:abc",
        "ytm:album",
        "ytm:album:bad id",
        "nonsense",
    ] {
        assert_eq!(
            catalog.load_context(uri).await,
            Err(LibraryError::NotFound),
            "{uri}"
        );
    }

    assert!(bodies(&server).await.is_empty());
}

#[test]
fn bounded_caches_forget_the_oldest() {
    let mut cache = Bounded::new();

    for n in 0..(PAGE_CACHE + 5) {
        cache.put(&n.to_string(), n);
    }

    assert_eq!(cache.get("0"), None);
    assert_eq!(
        cache.get(&(PAGE_CACHE + 4).to_string()),
        Some(PAGE_CACHE + 4)
    );
    assert_eq!(cache.items.len(), PAGE_CACHE);
}
