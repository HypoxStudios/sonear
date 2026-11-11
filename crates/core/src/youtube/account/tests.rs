use std::collections::VecDeque;

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::library::LibraryBackend;
use crate::youtube::local::LocalLibrary;

const ACCOUNT_MENU: &str = include_str!("../../../../youtube/tests/fixtures/account_menu.json");

struct FakeProvider {
    calls: Mutex<Vec<bool>>,
    outcomes: Mutex<VecDeque<SessionOutcome>>,
}

impl FakeProvider {
    fn new(outcomes: Vec<SessionOutcome>) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            outcomes: Mutex::new(outcomes.into()),
        })
    }

    fn calls(&self) -> Vec<bool> {
        lock(&self.calls).clone()
    }
}

#[async_trait]
impl SessionProvider for FakeProvider {
    async fn fetch(&self, interactive: bool) -> SessionOutcome {
        lock(&self.calls).push(interactive);

        lock(&self.outcomes)
            .pop_front()
            .unwrap_or(SessionOutcome::SignedOut)
    }
}

struct NoLibrary;

impl LibraryBackend for NoLibrary {}

fn session() -> Session {
    Session::from_header("SAPISID=synthetic; HSID=other")
}

struct Rig {
    account: Arc<YoutubeAccount>,
    provider: Arc<FakeProvider>,
    config: ConfigHandle,
    likes: Arc<LikeStore>,
    events: broadcast::Receiver<AppEvent>,
    webview: PathBuf,
    _server: MockServer,
    dir: tempfile::TempDir,
}

impl Rig {
    async fn new(signed_in_before: bool, outcomes: Vec<SessionOutcome>) -> Self {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/youtubei/v1/account/account_menu"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(ACCOUNT_MENU.as_bytes().to_vec(), "application/json"),
            )
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let config = ConfigHandle::load(dir.path().join("config.toml"));

        config.update(|config| config.youtube_signed_in = signed_in_before);

        let webview = dir.path().join("webview");

        std::fs::create_dir_all(&webview).unwrap();
        std::fs::write(webview.join("Cookies"), b"encrypted").unwrap();

        let provider = FakeProvider::new(outcomes);
        let catalog = YoutubeCatalog::new(
            Client::with_base("en", &format!("{}/youtubei/v1/", server.uri())),
            LocalLibrary::in_memory(),
        );
        let (events, receiver) = broadcast::channel(64);
        let account = YoutubeAccount::new(
            catalog,
            provider.clone(),
            config.clone(),
            webview.clone(),
            events.clone(),
        );
        let likes = LikeStore::new(Arc::new(NoLibrary), events);

        account.attach_likes(likes.clone());

        Self {
            account,
            provider,
            config,
            likes,
            events: receiver,
            webview,
            _server: server,
            dir,
        }
    }

    fn events(&mut self) -> Vec<AppEvent> {
        let mut found = Vec::new();

        while let Ok(event) = self.events.try_recv() {
            found.push(event);
        }

        found
    }

    fn states(events: &[AppEvent]) -> Vec<YtAccountState> {
        events
            .iter()
            .filter_map(|event| match event {
                AppEvent::YoutubeAccount(state) => Some(state.clone()),
                _ => None,
            })
            .collect()
    }

    async fn settle(&self) {
        for _ in 0..30 {
            tokio::task::yield_now().await;
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn signed_in_name(state: &YtAccountState) -> Option<&str> {
    match state {
        YtAccountState::SignedIn { name, .. } => Some(name),
        _ => None,
    }
}

#[tokio::test]
async fn without_a_remembered_sign_in_nothing_is_fetched() {
    let rig = Rig::new(false, vec![]).await;

    rig.account.start(&Handle::current());
    rig.settle().await;

    assert!(rig.provider.calls().is_empty());
    assert_eq!(rig.account.state(), YtAccountState::SignedOut);
    assert!(!rig.account.is_signed_in());
}

#[tokio::test]
async fn a_remembered_sign_in_is_restored_in_the_background() {
    let mut rig = Rig::new(true, vec![SessionOutcome::SignedIn(session())]).await;

    rig.likes.mark(&["ytm:track:a".to_owned()], true);
    rig.account.start(&Handle::current());
    rig.settle().await;

    let events = rig.events();
    let states = Rig::states(&events);

    assert_eq!(rig.provider.calls(), [false]);
    assert_eq!(states[0], YtAccountState::SigningIn);
    assert_eq!(signed_in_name(&states[1]), Some("Sample User"));
    assert!(rig.account.is_signed_in());
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AppEvent::AccountDataWiped))
    );
    assert_eq!(
        rig.likes.is_liked("ytm:track:a"),
        None,
        "the hearts were dropped"
    );
    assert!(rig.config.get().youtube_signed_in);
}

#[tokio::test]
async fn a_signed_out_profile_means_the_session_expired_and_the_flag_stays() {
    let mut rig = Rig::new(true, vec![SessionOutcome::SignedOut]).await;

    rig.account.start(&Handle::current());
    rig.settle().await;

    assert_eq!(rig.account.state(), YtAccountState::Expired);
    assert!(!rig.account.is_signed_in());
    assert!(rig.config.get().youtube_signed_in);
    assert!(
        rig.webview.exists(),
        "the profile is kept so they can sign in again"
    );
    assert!(
        rig.events()
            .iter()
            .any(|event| matches!(event, AppEvent::AccountDataWiped))
    );
}

#[tokio::test]
async fn a_failed_restore_stays_signed_out_for_this_run() {
    let rig = Rig::new(
        true,
        vec![SessionOutcome::Failed("WebView2 is missing".into())],
    )
    .await;

    rig.account.start(&Handle::current());
    rig.settle().await;

    assert_eq!(
        rig.account.state(),
        YtAccountState::Failed("WebView2 is missing".into())
    );
    assert!(!rig.account.is_signed_in());
    assert!(rig.config.get().youtube_signed_in);
}

#[tokio::test]
async fn signing_in_opens_the_window_and_remembers_it() {
    let mut rig = Rig::new(false, vec![SessionOutcome::SignedIn(session())]).await;

    rig.account.sign_in().await;

    let states = Rig::states(&rig.events());

    assert_eq!(rig.provider.calls(), [true]);
    assert_eq!(states[0], YtAccountState::SigningIn);
    assert_eq!(signed_in_name(&states[1]), Some("Sample User"));
    assert!(rig.config.get().youtube_signed_in);
}

#[tokio::test]
async fn cancelling_the_window_goes_back_to_where_it_was() {
    for (before, expected) in [
        (false, YtAccountState::SignedOut),
        (true, YtAccountState::Expired),
    ] {
        let rig = Rig::new(
            before,
            vec![SessionOutcome::SignedOut, SessionOutcome::Cancelled],
        )
        .await;

        if before {
            rig.account.start(&Handle::current());
            rig.settle().await;
        }

        rig.account.sign_in().await;

        assert_eq!(rig.account.state(), expected);
        assert!(!rig.account.is_signed_in());
    }
}

#[tokio::test]
async fn a_failed_sign_in_reports_why() {
    let rig = Rig::new(false, vec![SessionOutcome::Failed("no WebView2".into())]).await;

    rig.account.sign_in().await;

    assert_eq!(
        rig.account.state(),
        YtAccountState::Failed("no WebView2".into())
    );
    assert!(!rig.config.get().youtube_signed_in);
}

#[tokio::test]
async fn a_request_that_says_signed_out_refreshes_once_and_retries_once() {
    let rig = Rig::new(
        false,
        vec![
            SessionOutcome::SignedIn(session()),
            SessionOutcome::SignedIn(session()),
        ],
    )
    .await;

    rig.account.sign_in().await;

    let attempts = Mutex::new(0);
    let result = rig
        .account
        .retrying(|| async {
            let mut attempts = lock(&attempts);

            *attempts += 1;

            if *attempts == 1 {
                Err(LibraryError::NeedsLogin)
            } else {
                Ok("answer")
            }
        })
        .await;

    assert_eq!(result, Ok("answer"));
    assert_eq!(*lock(&attempts), 2);
    assert_eq!(rig.provider.calls(), [true, false]);
}

#[tokio::test]
async fn a_second_refresh_within_a_minute_does_not_run() {
    let rig = Rig::new(
        false,
        vec![
            SessionOutcome::SignedIn(session()),
            SessionOutcome::SignedIn(session()),
            SessionOutcome::SignedIn(session()),
        ],
    )
    .await;

    rig.account.set_refresh_every(Duration::from_millis(300));
    rig.account.sign_in().await;

    assert!(rig.account.refresh().await);
    assert!(rig.account.refresh().await);
    assert_eq!(rig.provider.calls(), [true, false]);

    tokio::time::sleep(Duration::from_millis(350)).await;

    assert!(rig.account.refresh().await);
    assert_eq!(rig.provider.calls(), [true, false, false]);

    let still_failing = rig
        .account
        .retrying(|| async { Err::<(), _>(LibraryError::NeedsLogin) })
        .await;

    assert_eq!(still_failing, Err(LibraryError::NeedsLogin));
    assert_eq!(
        rig.provider.calls().len(),
        3,
        "the retry did not refresh again"
    );
}

#[tokio::test]
async fn a_session_that_really_ended_expires_with_a_notice() {
    let mut rig = Rig::new(
        false,
        vec![
            SessionOutcome::SignedIn(session()),
            SessionOutcome::SignedOut,
        ],
    )
    .await;

    rig.account.sign_in().await;
    rig.events();

    let result: LibraryResult<()> = rig
        .account
        .retrying(|| async { Err(LibraryError::NeedsLogin) })
        .await;
    let events = rig.events();

    assert_eq!(result, Err(LibraryError::NeedsLogin));
    assert_eq!(rig.account.state(), YtAccountState::Expired);
    assert!(!rig.account.is_signed_in());
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AppEvent::Notice(text) if text.contains("session ended")))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AppEvent::AccountDataWiped))
    );
}

#[tokio::test]
async fn needs_login_passes_through_when_nobody_is_signed_in() {
    let rig = Rig::new(false, vec![]).await;
    let result: LibraryResult<()> = rig
        .account
        .retrying(|| async { Err(LibraryError::NeedsLogin) })
        .await;

    assert_eq!(result, Err(LibraryError::NeedsLogin));
    assert!(rig.provider.calls().is_empty());
}

#[tokio::test]
async fn signing_out_deletes_the_profile_and_forgets_everything() {
    let mut rig = Rig::new(false, vec![SessionOutcome::SignedIn(session())]).await;

    rig.account.sign_in().await;
    rig.likes.mark(&["ytm:track:a".to_owned()], true);
    rig.events();

    let problems = rig.account.sign_out().await;
    let events = rig.events();

    assert!(problems.is_empty());
    assert!(!rig.webview.exists());
    assert!(!rig.config.get().youtube_signed_in);
    assert!(!rig.account.is_signed_in());
    assert_eq!(rig.account.state(), YtAccountState::SignedOut);
    assert_eq!(rig.likes.is_liked("ytm:track:a"), None);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AppEvent::AccountDataWiped))
    );
    assert!(rig.dir.path().exists());

    let again = rig.account.sign_out().await;

    assert!(again.is_empty(), "a missing profile is fine");
}

#[tokio::test]
async fn the_debug_output_of_an_outcome_holds_no_cookies() {
    let _ = json!({});
    let shown = format!("{:?}", session());

    assert!(!shown.contains("synthetic"));
}
