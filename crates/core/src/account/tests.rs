use super::*;
use crate::history::{HistoryRow, HistoryStore};
use crate::library::LibraryBackend;
use crate::meta::TrackResolver;
use crate::secrets::MemoryStore;

struct Fixture {
    account: Arc<Account>,
    secrets: Arc<MemoryStore>,
    paths: AppPaths,
    meta: Arc<MetaCache>,
    history: Arc<History>,
    queue: Arc<QueueManager>,
    config: ConfigHandle,
    _dir: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths::new(dir.path().join("config"), dir.path().join("local"));
    let config = ConfigHandle::load(paths.config_file());
    let secrets = Arc::new(MemoryStore::default());
    let library = Arc::new(SpotifyLibrary::default());
    let backend: Arc<dyn LibraryBackend> = library.clone();
    let meta = MetaCache::open(paths.meta_cache_dir());
    let (events, _) = broadcast::channel(16);
    let history = History::new(
        HistoryStore::open(paths.history_db()),
        backend.clone(),
        TrackResolver::new(backend.clone(), meta.clone()),
        config.clone(),
        events.clone(),
    );
    let likes = LikeStore::new(backend, events.clone());
    let player = crate::Player::new(
        crate::queue::tests::FakeBackend::new(),
        Handle::current(),
        config.clone(),
        events.clone(),
    );
    let queue = QueueManager::new(
        player,
        events.clone(),
        Some(paths.queue_file()),
        Handle::current(),
    );
    let account = Account::new(
        Handle::current(),
        paths.clone(),
        config.clone(),
        secrets.clone(),
        Some(library),
        meta.clone(),
        history.clone(),
        likes,
        queue.clone(),
        crate::LyricsStore::new(std::sync::Arc::new(crate::LrcLibSource::new())),
        events,
    );

    Fixture {
        account,
        secrets,
        paths,
        meta,
        history,
        queue,
        config,
        _dir: dir,
    }
}

#[test]
fn client_ids_are_checked_loosely() {
    assert!(looks_like_client_id("0123456789abcdef0123456789abcdef"));
    assert!(looks_like_client_id("  0123456789abcdef  "));
    assert!(!looks_like_client_id("short"));
    assert!(!looks_like_client_id("has spaces in the middle of it all"));
    assert!(!looks_like_client_id(
        "https://developer.spotify.com/dashboard"
    ));
}

#[tokio::test]
async fn a_bad_client_id_fails_without_touching_the_network() {
    let fx = fixture();

    fx.account.begin_login("nope");

    assert!(matches!(fx.account.state(), AccountState::LoginFailed(_)));
    assert_eq!(fx.config.get().spotify_client_id, "");
}

#[tokio::test]
async fn disconnect_deletes_the_login_cache_covers_history_and_client_id() {
    let fx = fixture();
    let covers = fx.paths.cover_cache_dir();

    fx.secrets
        .set(SPOTIFY_REFRESH_TOKEN, &Secret::new("refresh"))
        .unwrap();
    fx.secrets
        .set(crate::secrets::SOLOIST_API_KEY, &Secret::new("keep-me"))
        .unwrap();
    fx.config.update(|config| {
        config.spotify_client_id = "0123456789abcdef0123456789abcdef".into();
        config.history_cleared_at_ms = 5;
    });
    fx.meta.put("playlist:x", &"cached");
    fs::create_dir_all(&covers).unwrap();
    fs::write(covers.join("cover.jpg"), b"image").unwrap();
    fx.history
        .store()
        .insert(vec![HistoryRow {
            played_at_ms: 1,
            track_uri: "spotify:track:a".into(),
            context_uri: None,
        }])
        .await
        .unwrap();

    assert!(fx.paths.history_db().exists());
    fx.queue.add("spotify:track:q", crate::QueueSource::User);
    fs::write(fx.paths.queue_file(), b"{}").unwrap();

    let problems = fx.account.disconnect().await;

    assert!(problems.is_empty(), "{problems:?}");
    assert!(fx.secrets.get(SPOTIFY_REFRESH_TOKEN).unwrap().is_none());
    assert_eq!(
        fx.secrets
            .get(crate::secrets::SOLOIST_API_KEY)
            .unwrap()
            .unwrap()
            .expose(),
        "keep-me"
    );
    assert!(fx.meta.get::<String>("playlist:x").is_none());
    assert!(!covers.exists());
    assert!(!fx.paths.history_db().exists());
    assert!(!fx.paths.queue_file().exists());
    assert_eq!(fx.config.get().spotify_client_id, "");
    assert_eq!(fx.config.get().history_cleared_at_ms, 0);
    assert!(fx.queue.snapshot().entries.is_empty());
    assert_eq!(fx.account.state(), AccountState::Disconnected);
}

#[tokio::test]
async fn starting_without_a_saved_login_stays_disconnected() {
    let fx = fixture();

    fx.account.start();

    assert_eq!(fx.account.state(), AccountState::Disconnected);

    fx.config
        .update(|config| config.spotify_client_id = "0123456789abcdef0123456789abcdef".into());
    fx.account.start();

    assert_eq!(fx.account.state(), AccountState::Disconnected);
}

async fn seed_account_data(fx: &Fixture) {
    fx.meta.put("playlist:x", &"cached");
    fs::create_dir_all(fx.paths.cover_cache_dir()).unwrap();
    fs::write(fx.paths.cover_cache_dir().join("cover.jpg"), b"image").unwrap();
    fx.history
        .store()
        .insert(vec![HistoryRow {
            played_at_ms: 1,
            track_uri: "spotify:track:a".into(),
            context_uri: None,
        }])
        .await
        .unwrap();
    fx.queue.add("spotify:track:q", crate::QueueSource::User);
    fs::write(fx.paths.queue_file(), b"{}").unwrap();
}

#[tokio::test]
async fn adopting_the_first_user_keeps_existing_data() {
    let fx = fixture();

    seed_account_data(&fx).await;

    assert!(!fx.account.adopt_user("alice").await);
    assert_eq!(fx.config.get().account_user_id, "alice");
    assert!(fx.meta.get::<String>("playlist:x").is_some());
    assert!(fx.paths.history_db().exists());
    assert!(fx.paths.cover_cache_dir().exists());
}

#[tokio::test]
async fn adopting_the_same_user_keeps_data() {
    let fx = fixture();

    fx.config
        .update(|config| config.account_user_id = "alice".into());
    seed_account_data(&fx).await;

    assert!(!fx.account.adopt_user("alice").await);
    assert!(fx.meta.get::<String>("playlist:x").is_some());
    assert!(fx.paths.history_db().exists());
}

#[tokio::test]
async fn adopting_a_different_user_wipes_the_previous_data() {
    let fx = fixture();
    let mut events = fx.account.events.subscribe();

    fx.config.update(|config| {
        config.account_user_id = "alice".into();
        config.history_cleared_at_ms = 9;
    });
    seed_account_data(&fx).await;

    assert!(fx.account.adopt_user("bob").await);
    assert_eq!(fx.config.get().account_user_id, "bob");
    assert_eq!(fx.config.get().history_cleared_at_ms, 0);
    assert!(fx.meta.get::<String>("playlist:x").is_none());
    assert!(!fx.paths.history_db().exists());
    assert!(!fx.paths.queue_file().exists());
    assert!(!fx.paths.cover_cache_dir().exists());
    assert!(fx.queue.snapshot().entries.is_empty());

    let mut wiped = false;

    while let Ok(event) = events.try_recv() {
        wiped |= matches!(event, AppEvent::AccountDataWiped);
    }

    assert!(wiped);
}

#[tokio::test]
async fn disconnect_clears_the_account_user_and_announces_the_wipe() {
    let fx = fixture();
    let mut events = fx.account.events.subscribe();

    fx.config
        .update(|config| config.account_user_id = "alice".into());

    fx.account.disconnect().await;

    assert_eq!(fx.config.get().account_user_id, "");

    let mut wiped = false;

    while let Ok(event) = events.try_recv() {
        wiped |= matches!(event, AppEvent::AccountDataWiped);
    }

    assert!(wiped);
}
