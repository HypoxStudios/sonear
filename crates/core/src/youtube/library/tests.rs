use async_trait::async_trait;
use serde_json::json;
use sonear_soloist::client::ClientEvent;
use sonear_soloist::protocol::Event;
use sonear_youtube::ToolError;
use tokio::runtime::Handle;
use tokio::sync::broadcast;
use wiremock::MockServer;

use super::*;
use crate::config::ConfigHandle;
use crate::model::{ItemKind, liked_context_uri};
use crate::player::PlayerBackend as _;
use crate::youtube::catalog::tests::{
    ALBUM, EXPLORE, HOME, LONG_PLAYLIST, NEXT_RADIO, RAM, catalog, mock, mock_playlist_pages,
};
use crate::youtube::engine::{AudioEngine, EngineLauncher, StreamSource};

struct NoTools;

#[async_trait]
impl EngineLauncher for NoTools {
    async fn launch(&self, _volume: u8) -> Result<Arc<dyn AudioEngine>, ToolError> {
        Err(ToolError::NotInstalled)
    }
}

#[async_trait]
impl StreamSource for NoTools {
    async fn resolve(&self, _video_id: &str) -> Result<String, ToolError> {
        Err(ToolError::NotInstalled)
    }

    fn prefetch(&self, _video_id: &str) {}

    fn forget(&self, _video_id: &str) {}
}

struct SignedInProvider;

#[async_trait]
impl crate::youtube::SessionProvider for SignedInProvider {
    async fn fetch(&self, _interactive: bool) -> crate::youtube::SessionOutcome {
        crate::youtube::SessionOutcome::SignedIn(sonear_youtube::Session::from_header(
            "SAPISID=synthetic; HSID=other",
        ))
    }
}

const ACCOUNT_MENU: &str = include_str!("../../../../youtube/tests/fixtures/account_menu.json");
const LIBRARY_PLAYLISTS: &str =
    include_str!("../../../../youtube/tests/fixtures/library_playlists.json");
const LIBRARY_PLAYLISTS_MORE: &str =
    include_str!("../../../../youtube/tests/fixtures/library_playlists_more.json");
const LIBRARY_ALBUMS: &str = include_str!("../../../../youtube/tests/fixtures/library_albums.json");
const LIBRARY_ARTISTS: &str =
    include_str!("../../../../youtube/tests/fixtures/library_subscriptions.json");

struct Rig {
    account: Arc<YoutubeAccount>,
    library: Arc<YoutubeLibrary>,
    catalog: Arc<YoutubeCatalog>,
    feed: broadcast::Receiver<ClientEvent>,
    _dir: tempfile::TempDir,
}

impl Rig {
    fn new(server: &MockServer) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = ConfigHandle::load(dir.path().join("config.toml"));
        let catalog = catalog(server);
        let (events, _) = broadcast::channel(16);
        let backend = YoutubeBackend::spawn(
            &Handle::current(),
            Arc::new(NoTools),
            Arc::new(NoTools),
            catalog.clone(),
            config.clone(),
            events,
        );
        let feed = backend.subscribe();
        let (account_events, _) = broadcast::channel(16);
        let account = YoutubeAccount::new(
            catalog.clone(),
            Arc::new(SignedInProvider),
            config.clone(),
            dir.path().join("webview"),
            account_events,
        );

        Self {
            account: account.clone(),
            library: YoutubeLibrary::new(catalog.clone(), backend, config, account),
            catalog,
            feed,
            _dir: dir,
        }
    }

    /// The track and context the player last showed.
    async fn shown(&mut self) -> (Option<String>, Option<String>) {
        for _ in 0..30 {
            tokio::task::yield_now().await;
        }

        let mut shown = (None, None);

        while let Ok(event) = self.feed.try_recv() {
            if let ClientEvent::Event(Event::PlaybackState(state)) = event {
                shown = (
                    state.item.map(|item| item.uri),
                    state.context.map(|context| context.uri),
                );
            }
        }

        shown
    }
}

fn track(id: &str) -> Track {
    Track {
        uri: format!("ytm:track:{id}"),
        id: Some(id.to_owned()),
        name: id.to_owned(),
        playable: true,
        kind: ItemKind::Track,
        ..Track::default()
    }
}

#[tokio::test]
async fn the_profile_and_empty_sections_match_a_local_account() {
    let server = MockServer::start().await;
    let rig = Rig::new(&server);
    let library = &rig.library;

    assert_eq!(library.profile().await.unwrap().id, "local");
    assert_eq!(library.profile().await.unwrap().display_name, "YT Music");
    assert_eq!(library.my_playlists(0).await.unwrap().total, Some(0));
    assert_eq!(library.followed_artists(None).await.unwrap().total, Some(0));
    assert_eq!(library.saved_shows(0).await.unwrap().total, Some(0));
    assert!(library.top_tracks(10).await.unwrap().is_empty());
    assert_eq!(library.show("x").await, Err(LibraryError::NotFound));
    assert_eq!(
        library.show_episodes("x", 0).await,
        Err(LibraryError::NotFound)
    );
    assert!(library.rate_limit().is_none());

    let devices = library.devices().await.unwrap();

    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].name, "Sonear");
    assert!(devices[0].active);
}

#[tokio::test]
async fn saving_and_removing_use_the_local_library() {
    let server = MockServer::start().await;
    let rig = Rig::new(&server);
    let library = &rig.library;

    mock(&server, "browse", json!({"browseId": RAM}), ALBUM).await;
    rig.catalog.remember(&[track("abc"), track("def")]);

    let uris = [
        "ytm:track:abc".to_owned(),
        format!("ytm:album:{RAM}"),
        "ytm:track:def".to_owned(),
    ];

    library.save(&uris).await.unwrap();

    assert_eq!(
        library
            .contains(&[uris[0].clone(), "ytm:track:zzz".into(), uris[1].clone()])
            .await
            .unwrap(),
        [true, false, true]
    );

    let saved = library.saved_tracks(0).await.unwrap();

    assert_eq!(
        saved
            .items
            .iter()
            .map(|t| t.uri.as_str())
            .collect::<Vec<_>>(),
        ["ytm:track:def", "ytm:track:abc"]
    );
    assert_eq!(saved.total, Some(2));
    assert!(!saved.has_more);
    assert_eq!(
        library.saved_albums(0).await.unwrap().items[0].name,
        "Random Access Memories"
    );

    library
        .remove(&[uris[0].clone(), uris[1].clone()])
        .await
        .unwrap();

    assert_eq!(library.saved_tracks(0).await.unwrap().items.len(), 1);
    assert!(library.saved_albums(0).await.unwrap().items.is_empty());
    assert_eq!(
        library.save(&["ytm:artist:UCx".into()]).await,
        Err(LibraryError::Unsupported)
    );
}

#[tokio::test]
async fn saved_pages_are_cut_at_the_page_size() {
    let server = MockServer::start().await;
    let rig = Rig::new(&server);

    for n in 0..(PAGE_SIZE + 5) {
        rig.catalog.local().save_track(track(&format!("t{n}")));
    }

    let first = rig.library.saved_tracks(0).await.unwrap();
    let second = rig.library.saved_tracks(PAGE_SIZE).await.unwrap();

    assert_eq!(first.items.len(), PAGE_SIZE as usize);
    assert!(first.has_more);
    assert_eq!(second.items.len(), 5);
    assert!(!second.has_more);
}

#[tokio::test]
async fn recently_played_comes_from_the_local_plays() {
    let server = MockServer::start().await;
    let rig = Rig::new(&server);

    rig.catalog
        .local()
        .record_play(track("a"), Some("ytm:album:x".into()), 5);
    rig.catalog.local().record_play(track("b"), None, 9);

    let plays = rig.library.recently_played(10).await.unwrap();

    assert_eq!(plays[0].track.uri, "ytm:track:b");
    assert_eq!(plays[0].played_at_ms, 9);
    assert_eq!(plays[1].context_uri.as_deref(), Some("ytm:album:x"));
    assert_eq!(rig.library.recently_played(1).await.unwrap().len(), 1);
}

#[tokio::test]
async fn album_tracks_come_in_pages_and_artist_albums_all_at_once() {
    let server = MockServer::start().await;
    let rig = Rig::new(&server);

    mock(&server, "browse", json!({"browseId": RAM}), ALBUM).await;

    let page = rig.library.album_tracks(RAM, 0).await.unwrap();

    assert_eq!(page.items.len(), 6);
    assert_eq!(page.items[0].name, "Give Life Back to Music");
    assert!(!page.has_more);
    assert!(
        rig.library
            .album_tracks(RAM, 50)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(rig.library.album(RAM).await.unwrap().total_tracks, 13);
}

#[tokio::test]
async fn playlist_tracks_page_across_the_continuation() {
    let server = MockServer::start().await;
    let rig = Rig::new(&server);

    mock_playlist_pages(&server).await;

    let page = rig.library.playlist_tracks(LONG_PLAYLIST, 0).await.unwrap();

    assert_eq!(page.items.len(), 6);
    assert!(!page.has_more);
    assert_eq!(page.total, Some(6));
    assert_eq!(
        rig.library
            .playlist(LONG_PLAYLIST)
            .await
            .unwrap()
            .track_count,
        Some(182)
    );
}

#[tokio::test]
async fn search_only_answers_the_first_page() {
    let server = MockServer::start().await;
    let rig = Rig::new(&server);
    let results = rig
        .library
        .search("daft punk", &[SearchKind::Track], 10)
        .await
        .unwrap();

    assert_eq!(results, SearchResults::default());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn discover_lists_the_three_shelves() {
    let server = MockServer::start().await;
    let rig = Rig::new(&server);

    mock(
        &server,
        "browse",
        json!({"browseId": "FEmusic_explore"}),
        EXPLORE,
    )
    .await;
    mock(&server, "browse", json!({"browseId": "FEmusic_home"}), HOME).await;

    let ids: Vec<_> = rig
        .library
        .discover()
        .await
        .unwrap()
        .into_iter()
        .map(|shelf| shelf.id)
        .collect();

    assert_eq!(ids, ["trending", "new_releases", "quick_picks"]);
}

#[tokio::test]
async fn starting_a_context_hands_its_tracks_to_the_player() {
    let server = MockServer::start().await;
    let mut rig = Rig::new(&server);

    mock(&server, "browse", json!({"browseId": RAM}), ALBUM).await;

    rig.library
        .start_playback(
            &WebPlayRequest {
                context_uri: Some(format!("ytm:album:{RAM}")),
                uris: None,
                offset: Some(PlayOffset::Uri("ytm:track:ajGKWk0auOc".into())),
            },
            DEVICE_ID,
        )
        .await
        .unwrap();

    let (item, context) = rig.shown().await;

    assert_eq!(item.as_deref(), Some("ytm:track:ajGKWk0auOc"));
    assert_eq!(context, Some(format!("ytm:album:{RAM}")));
}

#[tokio::test]
async fn a_station_starts_from_its_first_track() {
    let server = MockServer::start().await;
    let mut rig = Rig::new(&server);

    mock(
        &server,
        "next",
        json!({"playlistId": "RDAMVMIluRBvnYMoY"}),
        NEXT_RADIO,
    )
    .await;
    rig.library
        .start_playback(
            &WebPlayRequest {
                context_uri: Some("ytm:station:track:IluRBvnYMoY".into()),
                ..WebPlayRequest::default()
            },
            DEVICE_ID,
        )
        .await
        .unwrap();

    let (item, context) = rig.shown().await;

    assert_eq!(item.as_deref(), Some("ytm:track:5NV6Rdv1a3I"));
    assert_eq!(context.as_deref(), Some("ytm:station:track:IluRBvnYMoY"));
}

#[tokio::test]
async fn the_liked_context_starts_at_the_chosen_saved_track() {
    let server = MockServer::start().await;
    let mut rig = Rig::new(&server);

    for id in ["a", "b", "c"] {
        rig.catalog.local().save_track(track(id));
    }

    let uri = liked_context_uri("local");

    rig.library
        .start_playback(
            &WebPlayRequest {
                context_uri: Some(uri.clone()),
                uris: None,
                offset: Some(PlayOffset::Uri("ytm:track:a".into())),
            },
            DEVICE_ID,
        )
        .await
        .unwrap();

    let (item, context) = rig.shown().await;

    assert_eq!(item.as_deref(), Some("ytm:track:a"));
    assert_eq!(context, Some(uri));
}

#[tokio::test]
async fn a_track_list_drops_unknown_tracks_and_adjusts_the_start() {
    let server = MockServer::start().await;
    let mut rig = Rig::new(&server);

    rig.catalog
        .remember(&[track("aaaaaaaaaaa"), track("ccccccccccc")]);

    rig.library
        .start_playback(
            &WebPlayRequest {
                context_uri: None,
                uris: Some(vec![
                    "ytm:track:aaaaaaaaaaa".into(),
                    "ytm:track:not a track".into(),
                    "ytm:track:ccccccccccc".into(),
                ]),
                offset: Some(PlayOffset::Position(2)),
            },
            DEVICE_ID,
        )
        .await
        .unwrap();

    let (item, context) = rig.shown().await;

    assert_eq!(item.as_deref(), Some("ytm:track:ccccccccccc"));
    assert_eq!(context, None);

    assert_eq!(
        rig.library
            .start_playback(
                &WebPlayRequest {
                    context_uri: None,
                    uris: Some(vec!["ytm:track:not a track".into()]),
                    offset: None,
                },
                DEVICE_ID,
            )
            .await,
        Err(LibraryError::NotFound)
    );
}

#[test]
fn catalogue_errors_map_to_library_errors() {
    use sonear_youtube::YoutubeError;

    assert_eq!(
        LibraryError::from(YoutubeError::Network("offline".into())),
        LibraryError::Network("offline".into())
    );
    assert_eq!(
        LibraryError::from(YoutubeError::NotFound),
        LibraryError::NotFound
    );
    assert_eq!(
        LibraryError::from(YoutubeError::Api {
            status: 400,
            message: "bad".into()
        }),
        LibraryError::Api("400: bad".into())
    );
    assert_eq!(
        LibraryError::from(YoutubeError::Decode("x".into())),
        LibraryError::Decode("x".into())
    );
}

/// Mocks everything a signed-in account reads. `LM` is a long playlist with one continuation.
async fn mock_account(server: &MockServer) {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};

    Mock::given(method("POST"))
        .and(path("/youtubei/v1/account/account_menu"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(ACCOUNT_MENU.as_bytes().to_vec(), "application/json"),
        )
        .mount(server)
        .await;

    for (id, body) in [
        ("FEmusic_liked_playlists", LIBRARY_PLAYLISTS),
        ("FEmusic_liked_albums", LIBRARY_ALBUMS),
        ("FEmusic_library_corpus_artists", LIBRARY_ARTISTS),
        ("VLLM", crate::youtube::catalog::tests::PLAYLIST_LONG),
        (RAM, ALBUM),
    ] {
        mock(server, "browse", json!({ "browseId": id }), body).await;
    }

    mock(
        server,
        "browse",
        json!({ "continuation": "synthetic-library-token-1" }),
        LIBRARY_PLAYLISTS_MORE,
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/youtubei/v1/browse"))
        .and(wiremock::matchers::body_string_contains("\"continuation\""))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                crate::youtube::catalog::tests::PLAYLIST_MORE
                    .as_bytes()
                    .to_vec(),
                "application/json",
            ),
        )
        .with_priority(5)
        .mount(server)
        .await;

    for rate in ["like/like", "like/removelike"] {
        mock(server, rate, json!({}), "{}").await;
    }
}

#[tokio::test]
async fn signed_in_the_library_comes_from_the_account() {
    let server = MockServer::start().await;
    let rig = Rig::new(&server);

    mock_account(&server).await;
    rig.catalog.local().save_track(track("local-only"));
    rig.account.sign_in().await;

    assert_eq!(
        rig.library.profile().await.unwrap().display_name,
        "Sample User"
    );

    let playlists = rig.library.my_playlists(0).await.unwrap().items;

    assert_eq!(
        playlists
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["Synthetic Road Trip", "Synthetic Study Mix"]
    );
    assert_eq!(rig.library.saved_albums(0).await.unwrap().items.len(), 2);
    assert_eq!(
        rig.library
            .followed_artists(None)
            .await
            .unwrap()
            .items
            .len(),
        2
    );

    let liked = rig.library.saved_tracks(0).await.unwrap();

    assert!(
        liked
            .items
            .iter()
            .all(|t| t.id.as_deref() != Some("local-only"))
    );
    assert!(!liked.items.is_empty());

    rig.account.sign_out().await;

    assert_eq!(
        rig.library.saved_tracks(0).await.unwrap().items[0].name,
        "local-only"
    );
    assert_eq!(rig.library.my_playlists(0).await.unwrap().total, Some(0));
}

#[tokio::test]
async fn contains_waits_for_the_first_load_and_hearts_update_the_sets() {
    let server = MockServer::start().await;
    let rig = Rig::new(&server);

    mock_account(&server).await;
    rig.account.sign_in().await;

    let first = rig.library.saved_tracks(0).await.unwrap().items[0].clone();
    let other = "ytm:track:zzzzzzzzzzz".to_owned();
    let uris = [
        first.uri.clone(),
        other.clone(),
        "ytm:album:MPREbSyntheticAlbumOne".to_owned(),
    ];

    assert_eq!(
        rig.library.contains(&uris).await.unwrap(),
        [true, false, true]
    );

    rig.catalog.remember(&[track("zzzzzzzzzzz")]);
    rig.library
        .save(std::slice::from_ref(&other))
        .await
        .unwrap();

    assert_eq!(
        rig.library
            .contains(std::slice::from_ref(&other))
            .await
            .unwrap(),
        [true]
    );

    rig.library
        .remove(&[other.clone(), first.uri.clone()])
        .await
        .unwrap();

    assert_eq!(
        rig.library.contains(&[other, first.uri]).await.unwrap(),
        [false, false]
    );
}

#[tokio::test]
async fn saving_an_album_likes_its_audio_playlist() {
    let server = MockServer::start().await;
    let rig = Rig::new(&server);

    mock_account(&server).await;
    rig.account.sign_in().await;
    rig.library
        .save(&[format!("ytm:album:{RAM}")])
        .await
        .unwrap();

    let sent: Vec<serde_json::Value> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.url.path().ends_with("/like/like"))
        .map(|request| serde_json::from_slice(&request.body).unwrap())
        .collect();

    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0]["target"]["playlistId"],
        "OLAK5uy_kNhM2yaBTOVwrcZJepB1C9P3-n5_Sfy5c"
    );
    assert_eq!(
        rig.library
            .contains(&[format!("ytm:album:{RAM}")])
            .await
            .unwrap(),
        [true]
    );
}
