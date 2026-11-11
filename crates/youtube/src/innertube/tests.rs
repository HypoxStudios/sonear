use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::models::{HomeItem, HomeShelf, ShelfKind};

const DAFT_PUNK: &str = "UCRr1xG_2WIDs18a6cIiCxeA";
const RAM: &str = "MPREb_K8qWMWVqXGi";

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.json"));

    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn artist_ids(song: &Song) -> Vec<&str> {
    song.artists
        .iter()
        .filter_map(|artist| artist.id.as_deref())
        .collect()
}

#[test]
fn durations_parse_with_and_without_hours() {
    assert_eq!(parse::parse_duration("4:36"), Some(276_000));
    assert_eq!(parse::parse_duration("1:02:03"), Some(3_723_000));
    assert_eq!(parse::parse_duration("0:05"), Some(5_000));

    for text in ["", "2013", "5", "1:2:3:4", "a:bc", "12 views", ":30", "1:"] {
        assert_eq!(parse::parse_duration(text), None, "{text}");
    }
}

#[test]
fn searches_songs() {
    let results = parse::search(&fixture("search_songs")).unwrap();
    let first = &results.songs[0];

    assert!(results.albums.is_empty() && results.artists.is_empty());
    assert_eq!(first.video_id, "khnokW3Mw24");
    assert_eq!(first.title, "Instant Crush (feat. Julian Casablancas)");
    assert_eq!(artist_ids(first), [DAFT_PUNK, "UCWhpbdGLnR8XtXLr0yl2nYA"]);
    assert_eq!(first.album.as_ref().unwrap().id, RAM);
    assert_eq!(first.album.as_ref().unwrap().name, "Random Access Memories");
    assert_eq!(first.duration_ms, Some(338_000));
    assert_eq!(first.thumbs[0].width, Some(60));
}

#[test]
fn searches_albums() {
    let results = parse::search(&fixture("search_albums")).unwrap();
    let first = &results.albums[0];

    assert_eq!(first.browse_id, RAM);
    assert_eq!(first.title, "Random Access Memories");
    assert_eq!(first.kind, "Album");
    assert_eq!(first.year.as_deref(), Some("2013"));
    assert_eq!(first.artists[0].name, "Daft Punk");
    assert_eq!(first.artists[0].id.as_deref(), Some(DAFT_PUNK));
    assert!(results.albums.iter().any(|album| album.kind == "EP"));
}

#[test]
fn searches_artists_and_playlists() {
    let artists = parse::search(&fixture("search_artists")).unwrap().artists;
    let playlists = parse::search(&fixture("search_playlists"))
        .unwrap()
        .playlists;

    assert_eq!(artists[0].channel_id, DAFT_PUNK);
    assert_eq!(artists[0].name, "Daft Punk");
    assert!(!artists[0].thumbs.is_empty());
    assert_eq!(playlists[0].playlist_id, "PLGfGJFH5ltY8");
    assert_eq!(playlists[0].title, "daft punk");
    assert_eq!(playlists[0].author.as_deref(), Some("Rośliniarz"));
}

#[test]
fn unfiltered_search_returns_every_kind_and_skips_the_rest() {
    let results = parse::search(&fixture("search_all")).unwrap();

    assert!(!results.songs.is_empty());
    assert_eq!(results.albums[0].title, "Random Access Memories");
    assert_eq!(results.artists[0].channel_id, DAFT_PUNK);
    assert!(!results.playlists.is_empty());

    let titles: Vec<_> = results
        .songs
        .iter()
        .map(|song| song.title.as_str())
        .collect();

    assert!(
        !titles.iter().any(|title| title.contains("Swan Song")),
        "podcast episodes are not songs"
    );
    assert!(
        results
            .artists
            .iter()
            .all(|artist| !artist.name.starts_with('@')),
        "profiles are not artists"
    );

    let mut ids: Vec<_> = results.songs.iter().map(|song| &song.video_id).collect();
    let count = ids.len();

    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), count);
}

#[test]
fn the_top_result_card_is_included_and_gives_its_songs_the_artist() {
    let results = parse::search(&fixture("search_all")).unwrap();
    let card_song = &results.songs[0];

    assert_eq!(card_song.video_id, "khnokW3Mw24");
    assert_eq!(artist_ids(card_song), [DAFT_PUNK]);
    assert_eq!(card_song.duration_ms, Some(338_000));
    assert_eq!(results.artists[0].name, "Daft Punk");
}

#[test]
fn a_song_top_result_card_becomes_a_song() {
    let root = json!({"contents": {"x": [{"musicCardShelfRenderer": {
        "title": {"runs": [{
            "text": "Get Lucky",
            "navigationEndpoint": {"watchEndpoint": {"videoId": "5NV6Rdv1a3I"}}
        }]},
        "subtitle": {"runs": [
            {"text": "Song"}, {"text": " • "},
            {"text": "Daft Punk", "navigationEndpoint": {"browseEndpoint": {
                "browseId": "UCRr1xG_2WIDs18a6cIiCxeA",
                "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ARTIST"}}
            }}},
            {"text": " • "}, {"text": "6:10"}
        ]},
        "contents": []
    }}]}});
    let results = parse::search(&root).unwrap();

    assert_eq!(results.songs.len(), 1);
    assert_eq!(results.songs[0].video_id, "5NV6Rdv1a3I");
    assert_eq!(results.songs[0].duration_ms, Some(370_000));
    assert_eq!(artist_ids(&results.songs[0]), [DAFT_PUNK]);
}

#[test]
fn explicit_badges_are_read() {
    let row = |badges: Value| {
        json!({"contents": [{"musicResponsiveListItemRenderer": {
            "playlistItemData": {"videoId": "IluRBvnYMoY"},
            "badges": badges,
            "flexColumns": [
                {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "Song"}]}}}
            ]
        }}]})
    };
    let explicit =
        row(json!([{"musicInlineBadgeRenderer": {"icon": {"iconType": "MUSIC_EXPLICIT_BADGE"}}}]));

    assert!(parse::search(&explicit).unwrap().songs[0].explicit);
    assert!(!parse::search(&row(json!([]))).unwrap().songs[0].explicit);
}

#[test]
fn a_bad_row_is_skipped_but_the_rest_survive() {
    let root = json!({"contents": [
        {"musicResponsiveListItemRenderer": {"flexColumns": "nope"}},
        {"musicResponsiveListItemRenderer": {"playlistItemData": {"videoId": 5}}},
        {"musicResponsiveListItemRenderer": {
            "playlistItemData": {"videoId": "IluRBvnYMoY"},
            "flexColumns": [
                {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "Fine"}]}}}
            ]
        }},
    ]});
    let results = parse::search(&root).unwrap();

    assert_eq!(results.songs.len(), 1);
    assert_eq!(results.songs[0].title, "Fine");
}

#[test]
fn an_answer_without_structure_is_a_decode_error() {
    for result in [
        parse::search(&json!({})).map(|_| ()),
        parse::album(&json!({"contents": {}}), RAM).map(|_| ()),
        parse::artist(&json!({"contents": {}}), DAFT_PUNK).map(|_| ()),
        parse::playlist(&json!({"contents": {}}), "PLx").map(|_| ()),
        parse::track_page(&json!({})).map(|_| ()),
        parse::home(&json!({"contents": {}})).map(|_| ()),
        parse::up_next(&json!({"contents": {}})).map(|_| ()),
        parse::artist_releases(&json!([])).map(|_| ()),
    ] {
        assert!(matches!(result, Err(YoutubeError::Decode(_))));
    }
}

#[test]
fn parses_an_album() {
    let page = parse::album(&fixture("album"), RAM).unwrap();
    let first = &page.tracks[0];

    assert_eq!(page.album.title, "Random Access Memories");
    assert_eq!(page.album.browse_id, RAM);
    assert_eq!(page.album.kind, "Album");
    assert_eq!(page.album.year.as_deref(), Some("2013"));
    assert_eq!(page.album.artists[0].id.as_deref(), Some(DAFT_PUNK));
    assert_eq!(page.track_count, Some(13));
    assert_eq!(page.tracks.len(), 6);
    assert_eq!(
        page.audio_playlist_id.as_deref(),
        Some("OLAK5uy_kNhM2yaBTOVwrcZJepB1C9P3-n5_Sfy5c")
    );
    assert_eq!(first.title, "Give Life Back to Music");
    assert_eq!(first.video_id, "IluRBvnYMoY");
    assert_eq!(first.duration_ms, Some(276_000));
    assert_eq!(first.album.as_ref().unwrap().id, RAM);
    assert_eq!(first.thumbs, page.album.thumbs);
    assert_eq!(page.album.thumbs.last().unwrap().width, Some(544));
    assert_eq!(
        page.tracks[4]
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>(),
        ["Daft Punk", "Julian Casablancas"]
    );
}

#[test]
fn parses_an_artist() {
    let page = parse::artist(&fixture("artist"), DAFT_PUNK).unwrap();

    assert_eq!(page.artist.channel_id, DAFT_PUNK);
    assert_eq!(page.artist.name, "Daft Punk");
    assert_eq!(page.artist.thumbs.last().unwrap().width, Some(1080));
    assert_eq!(page.top_songs.len(), 5);
    assert_eq!(page.top_songs[0].video_id, "khnokW3Mw24");
    assert_eq!(page.top_songs[0].album.as_ref().unwrap().id, RAM);
    assert_eq!(
        page.top_songs_playlist.as_deref(),
        Some("OLAK5uy_lNVBcjNtiCwq-n95uoxJ-Gd4vsfMkyZNs")
    );
    assert_eq!(
        page.albums[0].title,
        "Random Access Memories (Drumless Edition)"
    );
    assert_eq!(page.albums[0].year.as_deref(), Some("2023"));
    assert_eq!(page.albums[0].artists[0].name, "Daft Punk");
    assert_eq!(page.singles[0].title, "GLBTM (Studio Outtakes)");
    assert_eq!(page.singles[0].kind, "Single");

    let more = page.albums_more.unwrap();

    assert_eq!(more.browse_id, format!("MPAD{DAFT_PUNK}"));
    assert_eq!(more.params.as_deref(), Some("ggMIegYIARoCAQI%3D"));
    assert_eq!(
        page.singles_more.unwrap().params.as_deref(),
        Some("ggMIegYIAhoCAQI%3D")
    );
}

#[test]
fn discography_grids_are_told_apart_by_their_params_in_any_language() {
    let carousel = |title: &str, params: &str| {
        json!({"musicCarouselShelfRenderer": {
            "header": {"musicCarouselShelfBasicHeaderRenderer": {
                "title": {"runs": [{"text": title}]},
                "moreContentButton": {"buttonRenderer": {"navigationEndpoint": {"browseEndpoint": {
                    "browseId": "MPADUCx", "params": params
                }}}}
            }},
            "contents": [{"musicTwoRowItemRenderer": {
                "title": {"runs": [{"text": title}]},
                "navigationEndpoint": {"browseEndpoint": {"browseId": "MPREb_x"}}
            }}]
        }})
    };
    let root = json!({
        "header": {"musicImmersiveHeaderRenderer": {"title": {"runs": [{"text": "A"}]}}},
        "contents": {"sectionListRenderer": {"contents": [
            carousel("Sencillos y EP", "ggMIegYIAhoCAQI%3D"),
            carousel("Discos", "ggMIegYIARoCAQI%3D"),
        ]}}
    });
    let page = parse::artist(&root, "UCx").unwrap();

    assert_eq!(page.albums[0].title, "Discos");
    assert_eq!(page.singles[0].title, "Sencillos y EP");
}

#[test]
fn parses_the_releases_grid() {
    let albums = parse::artist_releases(&fixture("artist_releases")).unwrap();

    assert_eq!(albums.len(), 6);
    assert_eq!(albums[1].browse_id, RAM);
    assert_eq!(albums[1].title, "Random Access Memories");
    assert_eq!(albums[1].year.as_deref(), Some("2013"));
    assert_eq!(albums[1].kind, "Album");
}

#[test]
fn parses_a_playlist() {
    let page = parse::playlist(&fixture("playlist"), "VLPLGfGJFH5ltY8").unwrap();
    let first = &page.tracks[0];

    assert_eq!(page.playlist.playlist_id, "PLGfGJFH5ltY8");
    assert_eq!(page.playlist.title, "daft punk");
    assert_eq!(page.playlist.author.as_deref(), Some("Rośliniarz"));
    assert_eq!(page.playlist.track_count, Some(10));
    assert_eq!(page.tracks.len(), 6);
    assert_eq!(page.continuation, None);
    assert_eq!(first.video_id, "khnokW3Mw24");
    assert_eq!(first.duration_ms, Some(338_000));
    assert_eq!(first.album.as_ref().unwrap().name, "Random Access Memories");
    assert_eq!(artist_ids(first), [DAFT_PUNK, "UCWhpbdGLnR8XtXLr0yl2nYA"]);
}

#[test]
fn playlist_paging_takes_the_token_from_the_first_page_and_stops_on_the_last() {
    let first = parse::playlist(
        &fixture("playlist_long"),
        "PLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI",
    )
    .unwrap();

    assert_eq!(first.playlist.track_count, Some(182));
    assert_eq!(first.tracks.len(), 3);
    assert!(
        first
            .continuation
            .as_deref()
            .is_some_and(|token| token.starts_with("4qmFsgKH"))
    );

    let more = parse::track_page(&fixture("playlist_more")).unwrap();

    assert_eq!(more.tracks.len(), 3);
    assert_eq!(more.tracks[1].video_id, "Ke9A6XV2H38");
    assert_eq!(
        more.tracks[1].album.as_ref().unwrap().id,
        "MPREb_eMVLDe27KCd"
    );
    assert_eq!(more.continuation, None);
}

#[test]
fn the_older_continuation_shape_is_understood_too() {
    let root = json!({"continuationContents": {"musicPlaylistShelfContinuation": {
        "contents": [],
        "continuations": [{"nextContinuationData": {"continuation": "next-token"}}]
    }}});

    assert_eq!(
        parse::track_page(&root).unwrap().continuation.as_deref(),
        Some("next-token")
    );
}

#[test]
fn parses_home_shelves() {
    let shelves = parse::home(&fixture("home")).unwrap();

    assert_eq!(shelves[0].title, "Quick picks");
    assert!(matches!(&shelves[0].items[0], HomeItem::Song(song) if song.video_id == "xukbqwRuN5w"));
    assert_eq!(shelves[1].title, "Throwback jams");
    assert!(matches!(
        &shelves[1].items[0],
        HomeItem::Playlist(playlist)
            if playlist.title == "Clásicos del Vallenato"
                && playlist.playlist_id.starts_with("RDCLAK5uy_")
    ));
}

#[test]
fn parses_the_up_next_queue() {
    let songs = parse::up_next(&fixture("next_radio")).unwrap();

    assert_eq!(songs.len(), 6);
    assert_eq!(songs[0].video_id, "5NV6Rdv1a3I");
    assert_eq!(
        songs[0].title,
        "Get Lucky (Official Audio) (feat. Pharrell Williams & Nile Rodgers)"
    );
    assert_eq!(songs[0].artists[0].name, "Daft Punk");
    assert_eq!(songs[0].duration_ms, Some(249_000));
    assert_eq!(songs[0].thumbs[0].width, Some(400));
}

fn client(server: &MockServer) -> Client {
    Client::with_base("es", &format!("{}/youtubei/v1/", server.uri()))
}

async fn answer(server: &MockServer, endpoint: &str, body: Value) {
    Mock::given(method("POST"))
        .and(path(format!("/youtubei/v1/{endpoint}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn a_search_posts_the_innertube_request_shape() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/youtubei/v1/search"))
        .and(query_param("prettyPrint", "false"))
        .and(header("origin", "https://music.youtube.com"))
        .and(header("content-type", "application/json"))
        .and(body_partial_json(json!({
            "context": {"client": {
                "clientName": "WEB_REMIX",
                "clientVersion": CLIENT_VERSION,
                "hl": "es",
                "gl": "US",
            }},
            "query": "daft punk",
            "params": "EgWKAQIIAWoMEA4QChADEAQQCRAF",
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("search_songs")))
        .expect(1)
        .mount(&server)
        .await;

    let results = client(&server)
        .search("daft punk", Some(SearchFilter::Songs))
        .await
        .unwrap();

    assert_eq!(results.songs[0].video_id, "khnokW3Mw24");

    let requests = server.received_requests().await.unwrap();
    let agent = requests[0]
        .headers
        .get("user-agent")
        .unwrap()
        .to_str()
        .unwrap();

    assert!(agent.starts_with("Mozilla/5.0 (Windows NT 10.0"));
}

#[tokio::test]
async fn each_filter_has_its_params_and_no_filter_sends_none() {
    for (filter, params) in [
        (SearchFilter::Albums, "EgWKAQIYAWoMEA4QChADEAQQCRAF"),
        (SearchFilter::Artists, "EgWKAQIgAWoMEA4QChADEAQQCRAF"),
        (SearchFilter::Playlists, "EgWKAQIoAWoMEA4QChADEAQQCRAF"),
    ] {
        assert_eq!(filter.params(), params);
    }

    let server = MockServer::start().await;

    answer(&server, "search", fixture("search_all")).await;
    client(&server).search("x", None).await.unwrap();

    let body: Value =
        serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();

    assert!(body.get("params").is_none());
}

#[tokio::test]
async fn a_blank_search_makes_no_request() {
    let server = MockServer::start().await;
    let results = client(&server).search("   ", None).await.unwrap();

    assert_eq!(results, SearchResults::default());
    assert!(server.received_requests().await.unwrap().is_empty());
}

async fn first_body(server: &MockServer) -> Value {
    serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap()
}

#[tokio::test]
async fn browse_and_next_requests_carry_their_ids() {
    let server = MockServer::start().await;

    answer(&server, "browse", fixture("album")).await;
    client(&server).album(RAM).await.unwrap();

    let playlist_server = MockServer::start().await;

    answer(&playlist_server, "browse", fixture("playlist")).await;
    client(&playlist_server)
        .playlist("PLGfGJFH5ltY8")
        .await
        .unwrap();

    let more_server = MockServer::start().await;

    answer(&more_server, "browse", fixture("playlist_more")).await;
    client(&more_server)
        .playlist_more("tok%3D_1")
        .await
        .unwrap();

    let releases_server = MockServer::start().await;

    answer(&releases_server, "browse", fixture("artist_releases")).await;
    client(&releases_server)
        .artist_releases(&BrowseRef {
            browse_id: format!("MPAD{DAFT_PUNK}"),
            params: Some("ggMIegYIARoCAQI%3D".into()),
        })
        .await
        .unwrap();

    let home_server = MockServer::start().await;

    answer(&home_server, "browse", fixture("home")).await;
    client(&home_server).home().await.unwrap();

    let radio_server = MockServer::start().await;

    answer(&radio_server, "next", fixture("next_radio")).await;
    client(&radio_server).radio("IluRBvnYMoY").await.unwrap();

    let queue_server = MockServer::start().await;

    answer(&queue_server, "next", fixture("next_radio")).await;
    client(&queue_server)
        .up_next(
            "IluRBvnYMoY",
            Some("OLAK5uy_kNhM2yaBTOVwrcZJepB1C9P3-n5_Sfy5c"),
        )
        .await
        .unwrap();

    assert_eq!(first_body(&server).await["browseId"], RAM);
    assert_eq!(
        first_body(&playlist_server).await["browseId"],
        "VLPLGfGJFH5ltY8"
    );
    assert_eq!(first_body(&more_server).await["continuation"], "tok%3D_1");
    assert_eq!(
        first_body(&releases_server).await["params"],
        "ggMIegYIARoCAQI%3D"
    );
    assert_eq!(first_body(&home_server).await["browseId"], "FEmusic_home");

    let radio = first_body(&radio_server).await;

    assert_eq!(radio["videoId"], "IluRBvnYMoY");
    assert_eq!(radio["playlistId"], "RDAMVMIluRBvnYMoY");
    assert_eq!(radio["isAudioOnly"], true);

    let queue = first_body(&queue_server).await;

    assert_eq!(
        queue["playlistId"],
        "OLAK5uy_kNhM2yaBTOVwrcZJepB1C9P3-n5_Sfy5c"
    );
}

#[tokio::test]
async fn song_finds_the_matching_item_or_is_not_found() {
    let server = MockServer::start().await;

    answer(&server, "next", fixture("next_single")).await;

    let client = client(&server);
    let song = client.song("IluRBvnYMoY").await.unwrap();

    assert_eq!(song.title, "Give Life Back to Music (Official Audio)");
    assert_eq!(song.duration_ms, Some(284_000));
    assert_eq!(
        client.song("aaaaaaaaaaa").await,
        Err(YoutubeError::NotFound)
    );
}

#[tokio::test]
async fn http_errors_are_mapped() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/youtubei/v1/browse"))
        .and(body_partial_json(json!({"browseId": "MPREb_missing"})))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/youtubei/v1/browse"))
        .and(body_partial_json(json!({"browseId": "MPREb_broken"})))
        .respond_with(
            ResponseTemplate::new(500).set_body_json(json!({"error": {"message": "boom"}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/youtubei/v1/browse"))
        .and(body_partial_json(json!({"browseId": "MPREb_garbage"})))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/youtubei/v1/browse"))
        .and(body_partial_json(json!({"browseId": "MPREb_empty"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    let client = client(&server);

    assert_eq!(
        client.album("MPREb_missing").await,
        Err(YoutubeError::NotFound)
    );
    assert_eq!(
        client.album("MPREb_broken").await,
        Err(YoutubeError::Api {
            status: 500,
            message: "boom".into(),
        })
    );
    assert!(matches!(
        client.album("MPREb_garbage").await,
        Err(YoutubeError::Decode(_))
    ));
    assert!(matches!(
        client.album("MPREb_empty").await,
        Err(YoutubeError::Decode(_))
    ));
}

#[tokio::test]
async fn an_unreachable_server_is_a_network_error() {
    let client = Client::with_base("en", "http://127.0.0.1:1/");

    assert!(matches!(
        client.album(RAM).await,
        Err(YoutubeError::Network(_))
    ));
}

#[tokio::test]
async fn invalid_ids_are_rejected_before_any_request() {
    let server = MockServer::start().await;
    let client = client(&server);
    let long = "a".repeat(200);

    for id in ["", "a b", "MPRE/../x", "x\"y", "é", long.as_str()] {
        assert_eq!(client.album(id).await, Err(YoutubeError::NotFound), "{id}");
        assert_eq!(client.artist(id).await, Err(YoutubeError::NotFound), "{id}");
        assert_eq!(
            client.playlist(id).await,
            Err(YoutubeError::NotFound),
            "{id}"
        );
    }

    for id in ["", "short", "IluRBvnYMoYY", "IluRBvnYMo!", "IluRBvn YMo"] {
        assert_eq!(
            client.up_next(id, None).await,
            Err(YoutubeError::NotFound),
            "{id}"
        );
        assert_eq!(client.radio(id).await, Err(YoutubeError::NotFound), "{id}");
        assert_eq!(client.song(id).await, Err(YoutubeError::NotFound), "{id}");
    }

    assert_eq!(
        client.up_next("IluRBvnYMoY", Some("a b")).await,
        Err(YoutubeError::NotFound)
    );
    assert_eq!(client.playlist_more("").await, Err(YoutubeError::NotFound));
    assert_eq!(
        client
            .artist_releases(&BrowseRef {
                browse_id: "MPADx".into(),
                params: Some("a\"b".into()),
            })
            .await,
        Err(YoutubeError::NotFound)
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[test]
fn explore_keeps_album_and_song_shelves_in_page_order() {
    let shelves = parse::explore(&fixture("explore")).unwrap();
    let titles: Vec<_> = shelves.iter().map(|shelf| shelf.title.as_str()).collect();

    assert_eq!(titles, ["New albums & singles", "Trending"]);
    assert_eq!(shelves[0].kind, ShelfKind::Albums);
    assert_eq!(shelves[1].kind, ShelfKind::Songs);
    assert!(
        matches!(&shelves[0].items[0], HomeItem::Album(album) if album.browse_id.starts_with("MPREb_"))
    );
    assert!(matches!(&shelves[1].items[0], HomeItem::Song(song) if song.video_id.len() == 11));
}

#[test]
fn home_shelves_get_a_kind() {
    let shelves = parse::home(&fixture("home")).unwrap();

    assert_eq!(shelves[0].kind, ShelfKind::Songs);
    assert_eq!(shelves[1].kind, ShelfKind::Playlists);
    assert_eq!(HomeShelf::new("x".into(), vec![]).kind, ShelfKind::Mixed);
}

#[tokio::test]
async fn explore_browses_the_explore_page() {
    let server = MockServer::start().await;

    answer(&server, "browse", fixture("explore")).await;

    let shelves = client(&server).explore().await.unwrap();

    assert_eq!(shelves.len(), 2);
    assert_eq!(first_body(&server).await["browseId"], "FEmusic_explore");
}
