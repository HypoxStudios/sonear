use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use sonear_soloist::client::ClientEvent;

use super::*;
use crate::library::LibraryResult;
use crate::model::Device;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ws {
    Play(Option<String>),
    Activate,
}

struct Socket {
    calls: Mutex<Vec<Ws>>,
    events: broadcast::Sender<ClientEvent>,
}

impl Socket {
    fn record(&self, call: Ws) -> Result<(), PlayerError> {
        lock(&self.calls).push(call);

        Ok(())
    }
}

impl PlayerBackend for Socket {
    fn subscribe(&self) -> broadcast::Receiver<ClientEvent> {
        self.events.subscribe()
    }

    fn request_state(&self) -> Result<(), PlayerError> {
        Ok(())
    }

    fn play(&self, uri: Option<&str>) -> Result<(), PlayerError> {
        self.record(Ws::Play(uri.map(str::to_owned)))
    }

    fn pause(&self) -> Result<(), PlayerError> {
        Ok(())
    }

    fn next(&self) -> Result<(), PlayerError> {
        Ok(())
    }

    fn previous(&self) -> Result<(), PlayerError> {
        Ok(())
    }

    fn seek(&self, _: u64) -> Result<(), PlayerError> {
        Ok(())
    }

    fn set_volume(&self, _: u8) -> Result<(), PlayerError> {
        Ok(())
    }

    fn set_shuffle(&self, _: bool) -> Result<(), PlayerError> {
        Ok(())
    }

    fn set_repeat_context(&self, _: bool) -> Result<(), PlayerError> {
        Ok(())
    }

    fn set_repeat_track(&self, _: bool) -> Result<(), PlayerError> {
        Ok(())
    }

    fn add_to_queue(&self, _: &str) -> Result<(), PlayerError> {
        Ok(())
    }

    fn activate(&self) -> Result<(), PlayerError> {
        self.record(Ws::Activate)
    }
}

#[derive(Default)]
struct Web {
    /// One device list per `devices()` call; the last repeats.
    device_lists: Mutex<Vec<Vec<Device>>>,
    device_calls: AtomicUsize,
    plays: Mutex<Vec<(WebPlayRequest, String)>>,
    /// Device ids whose play call answers 404.
    dead_devices: Vec<String>,
    /// What a play call with a context answers instead of succeeding.
    context_error: Option<LibraryError>,
}

fn soloist(id: &str) -> Device {
    Device {
        id: id.to_owned(),
        name: "Sonear".to_owned(),
        active: false,
    }
}

fn other(id: &str) -> Device {
    Device {
        id: id.to_owned(),
        name: "Kitchen speaker".to_owned(),
        active: true,
    }
}

#[async_trait]
impl LibraryBackend for Web {
    async fn devices(&self) -> LibraryResult<Vec<Device>> {
        let call = self.device_calls.fetch_add(1, Ordering::SeqCst);
        let lists = lock(&self.device_lists);
        let list = lists.get(call).or_else(|| lists.last()).cloned();

        Ok(list.unwrap_or_default())
    }

    async fn start_playback(&self, request: &WebPlayRequest, device_id: &str) -> LibraryResult<()> {
        if self.dead_devices.iter().any(|dead| dead == device_id) {
            return Err(LibraryError::NotFound);
        }

        if let (Some(_), Some(error)) = (&request.context_uri, &self.context_error) {
            return Err(error.clone());
        }

        lock(&self.plays).push((request.clone(), device_id.to_owned()));

        Ok(())
    }
}

struct Fixture {
    player: Player,
    socket: Arc<Socket>,
    web: Arc<Web>,
    events: broadcast::Receiver<AppEvent>,
    _dir: tempfile::TempDir,
}

fn fixture(web: Web) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let config = ConfigHandle::load(dir.path().join("config.toml"));
    let socket = Arc::new(Socket {
        calls: Mutex::new(Vec::new()),
        events: broadcast::channel(16).0,
    });
    let (events, receiver) = broadcast::channel(16);
    let player = Player::new(socket.clone(), Handle::current(), config, events);
    let web = Arc::new(web);

    player.attach_library(web.clone());

    Fixture {
        player,
        socket,
        web,
        events: receiver,
        _dir: dir,
    }
}

impl Fixture {
    fn inner_support(&self) -> u8 {
        self.player.inner.liked_support.load(Ordering::Relaxed)
    }
}

async fn settle() {
    tokio::time::sleep(Duration::from_secs(10)).await;
}

#[tokio::test(start_paused = true)]
async fn an_offset_goes_to_the_web_api_with_the_soloist_device_id() {
    let web = Web {
        device_lists: Mutex::new(vec![vec![other("k1"), soloist("s1")]]),
        ..Web::default()
    };
    let fx = fixture(web);

    fx.player
        .play_in_context("spotify:album:a", PlayOffset::Position(4));
    settle().await;

    let plays = lock(&fx.web.plays).clone();

    assert_eq!(plays.len(), 1);
    assert_eq!(plays[0].1, "s1");
    assert_eq!(plays[0].0.context_uri.as_deref(), Some("spotify:album:a"));
    assert_eq!(plays[0].0.offset, Some(PlayOffset::Position(4)));
    assert!(lock(&fx.socket.calls).is_empty());

    fx.player.play_in_context(
        "spotify:playlist:p",
        PlayOffset::Uri("spotify:track:t".into()),
    );
    settle().await;

    assert_eq!(fx.web.device_calls.load(Ordering::SeqCst), 1);
    assert_eq!(lock(&fx.web.plays).len(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_dead_device_id_triggers_one_refresh_and_a_retry() {
    let web = Web {
        device_lists: Mutex::new(vec![vec![soloist("old")], vec![soloist("new")]]),
        dead_devices: vec!["old".to_owned()],
        ..Web::default()
    };
    let mut fx = fixture(web);

    fx.player
        .play_in_context("spotify:album:a", PlayOffset::Position(1));
    settle().await;

    let plays = lock(&fx.web.plays).clone();

    assert_eq!(plays.len(), 1);
    assert_eq!(plays[0].1, "new");
    assert_eq!(fx.web.device_calls.load(Ordering::SeqCst), 2);
    assert!(fx.events.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn a_missing_device_is_activated_then_falls_back_to_the_websocket_with_a_notice() {
    let web = Web {
        device_lists: Mutex::new(vec![vec![other("k1")]]),
        ..Web::default()
    };
    let mut fx = fixture(web);

    fx.player.play_in_context(
        "spotify:album:a",
        PlayOffset::Uri("spotify:track:chosen".into()),
    );
    settle().await;

    assert_eq!(
        *lock(&fx.socket.calls),
        [Ws::Activate, Ws::Play(Some("spotify:track:chosen".into()))]
    );
    assert_eq!(fx.web.device_calls.load(Ordering::SeqCst), 2);
    assert!(lock(&fx.web.plays).is_empty());
    assert!(matches!(fx.events.try_recv(), Ok(AppEvent::Notice(_))));
}

#[tokio::test(start_paused = true)]
async fn the_device_appearing_after_activate_is_used() {
    let web = Web {
        device_lists: Mutex::new(vec![vec![], vec![soloist("late")]]),
        ..Web::default()
    };
    let fx = fixture(web);

    fx.player
        .play_in_context("spotify:album:a", PlayOffset::Position(0));
    settle().await;

    assert_eq!(*lock(&fx.socket.calls), [Ws::Activate]);
    assert_eq!(lock(&fx.web.plays)[0].1, "late");
}

#[tokio::test(start_paused = true)]
async fn liked_songs_play_by_uris_from_the_clicked_row_capped_at_100() {
    let web = Web {
        device_lists: Mutex::new(vec![vec![soloist("s1")]]),
        ..Web::default()
    };
    let fx = fixture(web);
    let uris: Vec<String> = (0..250).map(|n| format!("spotify:track:{n}")).collect();

    fx.player.play_tracks(uris, 20);
    settle().await;

    let plays = lock(&fx.web.plays).clone();
    let request = &plays[0].0;

    assert_eq!(request.context_uri, None);
    assert_eq!(request.offset, None);

    let sent = request.uris.as_ref().unwrap();

    assert_eq!(sent.len(), MAX_PLAY_URIS);
    assert_eq!(sent[0], "spotify:track:20");
    assert_eq!(sent[99], "spotify:track:119");
}

#[tokio::test(start_paused = true)]
async fn without_a_library_requests_go_over_the_websocket() {
    let dir = tempfile::tempdir().unwrap();
    let config = ConfigHandle::load(dir.path().join("config.toml"));
    let socket = Arc::new(Socket {
        calls: Mutex::new(Vec::new()),
        events: broadcast::channel(16).0,
    });
    let player = Player::new(
        socket.clone(),
        Handle::current(),
        config,
        broadcast::channel(16).0,
    );

    player.play_in_context("spotify:album:a", PlayOffset::Position(2));
    settle().await;

    assert_eq!(
        *lock(&socket.calls),
        [Ws::Play(Some("spotify:album:a".into()))]
    );
}

#[tokio::test(start_paused = true)]
async fn a_failed_play_posts_a_notice_and_does_not_fall_back() {
    struct Failing;

    #[async_trait]
    impl LibraryBackend for Failing {
        async fn devices(&self) -> LibraryResult<Vec<Device>> {
            Ok(vec![soloist("s1")])
        }

        async fn start_playback(&self, _: &WebPlayRequest, _: &str) -> LibraryResult<()> {
            Err(LibraryError::QuotaExceeded)
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let config = ConfigHandle::load(dir.path().join("config.toml"));
    let socket = Arc::new(Socket {
        calls: Mutex::new(Vec::new()),
        events: broadcast::channel(16).0,
    });
    let (events, mut receiver) = broadcast::channel(16);
    let player = Player::new(socket.clone(), Handle::current(), config, events);

    player.attach_library(Arc::new(Failing));
    player.play_in_context("spotify:album:a", PlayOffset::Position(1));
    settle().await;

    assert!(matches!(receiver.try_recv(), Ok(AppEvent::Notice(text)) if text.contains("quota")));
    assert!(lock(&socket.calls).is_empty());
}

const COLLECTION: &str = "spotify:user:me:collection";

fn liked_uris() -> Vec<String> {
    (0..5).map(|n| format!("spotify:track:{n}")).collect()
}

fn play_liked(fx: &Fixture, offset: Option<&str>, start: usize) {
    fx.player.play_request(PlaybackRequest::LikedSongs {
        user_id: "me".into(),
        offset_uri: offset.map(str::to_owned),
        uris: liked_uris(),
        start,
    });
}

#[tokio::test(start_paused = true)]
async fn liked_songs_start_the_collection_context_with_a_uri_offset() {
    let web = Web {
        device_lists: Mutex::new(vec![vec![soloist("s1")]]),
        ..Web::default()
    };
    let fx = fixture(web);

    play_liked(&fx, Some("spotify:track:3"), 3);
    settle().await;

    let plays = lock(&fx.web.plays).clone();

    assert_eq!(plays.len(), 1);
    assert_eq!(plays[0].0.context_uri.as_deref(), Some(COLLECTION));
    assert_eq!(plays[0].0.uris, None);
    assert_eq!(
        plays[0].0.offset,
        Some(PlayOffset::Uri("spotify:track:3".into()))
    );
    assert_eq!(fx.inner_support(), LIKED_YES);
}

#[tokio::test(start_paused = true)]
async fn a_rejected_collection_falls_back_to_the_uris_and_is_remembered() {
    let web = Web {
        device_lists: Mutex::new(vec![vec![soloist("s1")]]),
        context_error: Some(LibraryError::Forbidden),
        ..Web::default()
    };
    let mut fx = fixture(web);

    play_liked(&fx, Some("spotify:track:2"), 2);
    settle().await;

    let plays = lock(&fx.web.plays).clone();

    assert_eq!(plays.len(), 1);
    assert_eq!(plays[0].0.context_uri, None);
    assert_eq!(plays[0].0.uris.as_ref().unwrap()[0], "spotify:track:2");
    assert_eq!(fx.web.device_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fx.inner_support(), LIKED_NO);
    assert!(fx.events.try_recv().is_err());

    play_liked(&fx, None, 0);
    settle().await;

    let plays = lock(&fx.web.plays).clone();

    assert_eq!(plays.len(), 2);
    assert_eq!(plays[1].0.context_uri, None);
    assert_eq!(plays[1].0.uris.as_ref().unwrap().len(), 5);
}

#[tokio::test(start_paused = true)]
async fn a_network_error_does_not_fall_back_or_change_the_answer() {
    let web = Web {
        device_lists: Mutex::new(vec![vec![soloist("s1")]]),
        context_error: Some(LibraryError::Network("down".into())),
        ..Web::default()
    };
    let mut fx = fixture(web);

    play_liked(&fx, None, 0);
    settle().await;

    assert!(lock(&fx.web.plays).is_empty());
    assert!(lock(&fx.socket.calls).is_empty());
    assert_eq!(fx.inner_support(), LIKED_UNKNOWN);
    assert!(matches!(fx.events.try_recv(), Ok(AppEvent::Notice(_))));
}

#[tokio::test(start_paused = true)]
async fn liked_songs_without_a_library_use_the_websocket() {
    let dir = tempfile::tempdir().unwrap();
    let config = ConfigHandle::load(dir.path().join("config.toml"));
    let socket = Arc::new(Socket {
        calls: Mutex::new(Vec::new()),
        events: broadcast::channel(16).0,
    });
    let player = Player::new(
        socket.clone(),
        Handle::current(),
        config,
        broadcast::channel(16).0,
    );

    player.play_request(PlaybackRequest::LikedSongs {
        user_id: "me".into(),
        offset_uri: None,
        uris: liked_uris(),
        start: 1,
    });
    player.play_request(PlaybackRequest::LikedSongs {
        user_id: "me".into(),
        offset_uri: None,
        uris: Vec::new(),
        start: 0,
    });
    settle().await;

    assert_eq!(
        *lock(&socket.calls),
        [
            Ws::Play(Some("spotify:track:1".into())),
            Ws::Play(Some(COLLECTION.into())),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_missing_device_plays_the_clicked_liked_track_over_the_websocket() {
    let web = Web {
        device_lists: Mutex::new(vec![vec![other("k1")]]),
        ..Web::default()
    };
    let mut fx = fixture(web);

    play_liked(&fx, Some("spotify:track:4"), 4);
    settle().await;

    assert_eq!(
        *lock(&fx.socket.calls),
        [Ws::Activate, Ws::Play(Some("spotify:track:4".into()))]
    );
    assert!(matches!(fx.events.try_recv(), Ok(AppEvent::Notice(_))));
}
