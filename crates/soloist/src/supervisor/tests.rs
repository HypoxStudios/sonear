use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures_util::SinkExt;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::Message;

use super::*;

const FAKE_PID: u32 = 4242;
const KEY: &str = "sk-very-secret-key-123";

#[derive(Clone)]
struct Script {
    exit_after: Option<Duration>,
    exit_code: i32,
    port: Option<u16>,
    lines: Vec<String>,
}

impl Script {
    fn exits(code: i32) -> Self {
        Self {
            exit_after: Some(Duration::from_millis(10)),
            exit_code: code,
            port: None,
            lines: Vec::new(),
        }
    }

    fn hangs() -> Self {
        Self {
            exit_after: None,
            exit_code: 0,
            port: None,
            lines: Vec::new(),
        }
    }
}

struct FakeSpawner {
    dirs: SoloistDirs,
    scripts: Mutex<VecDeque<Script>>,
    spawned: AtomicUsize,
    stale_seen: Mutex<Vec<bool>>,
    args: Mutex<Vec<Vec<String>>>,
    version: Mutex<Option<SoloistVersion>>,
}

impl FakeSpawner {
    fn new(dirs: &SoloistDirs, scripts: Vec<Script>) -> Arc<Self> {
        Arc::new(Self {
            dirs: dirs.clone(),
            scripts: Mutex::new(scripts.into()),
            spawned: AtomicUsize::new(0),
            stale_seen: Mutex::new(Vec::new()),
            args: Mutex::new(Vec::new()),
            version: Mutex::new(None),
        })
    }

    fn spawn_count(&self) -> usize {
        self.spawned.load(Ordering::SeqCst)
    }
}

struct FakeProcess {
    script: Script,
    killed: Arc<Notify>,
}

#[async_trait]
impl ProcessHandle for FakeProcess {
    async fn wait(&mut self) -> io::Result<Option<i32>> {
        match self.script.exit_after {
            Some(after) => tokio::select! {
                _ = sleep(after) => Ok(Some(self.script.exit_code)),
                _ = self.killed.notified() => Ok(None),
            },
            None => {
                self.killed.notified().await;
                Ok(None)
            }
        }
    }

    async fn kill(&mut self) {
        self.killed.notify_one();
    }
}

#[async_trait]
impl Spawner for FakeSpawner {
    fn spawn(&self, _: &Path, args: &[String]) -> io::Result<SpawnedProcess> {
        self.spawned.fetch_add(1, Ordering::SeqCst);
        lock(&self.stale_seen).push(self.dirs.port_file().exists());
        lock(&self.args).push(args.to_vec());

        let script = lock(&self.scripts)
            .pop_front()
            .unwrap_or_else(Script::hangs);
        let (lines, output) = mpsc::unbounded_channel();

        for line in &script.lines {
            let _ = lines.send(line.clone());
        }

        if let Some(port) = script.port {
            std::fs::write(self.dirs.port_file(), port.to_string())?;
        }

        Ok(SpawnedProcess {
            handle: Box::new(FakeProcess {
                script,
                killed: Arc::new(Notify::new()),
            }),
            pid: Some(FAKE_PID),
            output,
        })
    }

    async fn version(&self, _: &Path) -> Result<SoloistVersion, VersionError> {
        lock(&self.version)
            .clone()
            .ok_or_else(|| VersionError::Run("no fake version".into()))
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    dirs: SoloistDirs,
    spawner: Arc<FakeSpawner>,
    supervisor: Supervisor,
    status: broadcast::Receiver<Status>,
}

fn fixture(scripts: Vec<Script>, installed: bool) -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let dirs = SoloistDirs::new(tmp.path().join("soloist"));

    std::fs::create_dir_all(dirs.bin()).unwrap();
    std::fs::create_dir_all(dirs.data()).unwrap();

    if installed {
        std::fs::write(dirs.exe(), b"fake").unwrap();
    }

    let spawner = FakeSpawner::new(&dirs, scripts);
    let mut config = SupervisorConfig::new(dirs.clone());

    config.port_timeout = Duration::from_secs(10);

    let supervisor = Supervisor::new(config, spawner.clone());
    let status = supervisor.subscribe();

    Fixture {
        _tmp: tmp,
        dirs,
        spawner,
        supervisor,
        status,
    }
}

fn params() -> LaunchParams {
    LaunchParams {
        device_name: "Sonear".into(),
        key: Some(KEY.into()),
        volume: 40,
    }
}

async fn wait_for(rx: &mut broadcast::Receiver<Status>, pred: impl Fn(&Status) -> bool) -> Status {
    loop {
        let status = timeout(Duration::from_secs(600), rx.recv())
            .await
            .expect("status in time")
            .expect("channel open");

        if pred(&status) {
            return status;
        }
    }
}

fn expired_version() -> SoloistVersion {
    SoloistVersion::parse("soloist 1.0.0.1 build 1 (20200101) (gabc) (windows/x86_64)").unwrap()
}

#[tokio::test]
async fn not_installed_and_needs_key() {
    let mut fx = fixture(vec![], false);

    fx.supervisor.start(params()).await;

    assert_eq!(fx.supervisor.status(), Status::NotInstalled);

    std::fs::write(fx.dirs.exe(), b"fake").unwrap();
    fx.supervisor
        .start(LaunchParams {
            key: None,
            ..params()
        })
        .await;

    assert_eq!(
        wait_for(&mut fx.status, |s| *s == Status::NeedsKey).await,
        Status::NeedsKey
    );
    assert_eq!(fx.spawner.spawn_count(), 0);

    fx.supervisor
        .start(LaunchParams {
            key: Some("   ".into()),
            ..params()
        })
        .await;

    assert_eq!(fx.supervisor.status(), Status::NeedsKey);
}

#[tokio::test(start_paused = true)]
async fn exit_code_ten_is_expired_without_retry() {
    let mut fx = fixture(vec![Script::exits(10)], true);

    fx.supervisor.start(params()).await;
    wait_for(&mut fx.status, |s| *s == Status::Expired).await;
    sleep(Duration::from_secs(120)).await;

    assert_eq!(fx.supervisor.status(), Status::Expired);
    assert_eq!(fx.spawner.spawn_count(), 1);
}

#[tokio::test(start_paused = true)]
async fn expired_build_is_not_launched() {
    let mut fx = fixture(vec![], true);

    *lock(&fx.spawner.version) = Some(expired_version());
    fx.supervisor.start(params()).await;
    wait_for(&mut fx.status, |s| *s == Status::Expired).await;

    assert_eq!(fx.spawner.spawn_count(), 0);
}

#[tokio::test(start_paused = true)]
async fn retries_with_backoff_then_fails() {
    let mut script = Script::exits(1);

    script.lines = vec!["boom".into()];

    let mut fx = fixture(vec![script; 10], true);
    let begin = Instant::now();

    fx.supervisor.start(params()).await;

    let status = wait_for(&mut fx.status, |s| matches!(s, Status::Failed { .. })).await;
    let Status::Failed { reason, recent_log } = status else {
        unreachable!()
    };

    assert_eq!(fx.spawner.spawn_count(), 6);
    assert!(reason.to_string().contains("code 1"));
    assert!(recent_log.iter().any(|line| line == "boom"));

    let waited: Duration = BACKOFF.iter().sum();

    assert!(begin.elapsed() >= waited);
    assert!(begin.elapsed() < waited + Duration::from_secs(5));
}

#[tokio::test(start_paused = true)]
async fn missing_port_file_times_out() {
    let mut fx = fixture(vec![Script::hangs(); 10], true);
    let begin = Instant::now();

    fx.supervisor.start(params()).await;

    let status = wait_for(&mut fx.status, |s| matches!(s, Status::Failed { .. })).await;
    let Status::Failed { reason, .. } = status else {
        unreachable!()
    };

    assert!(
        reason.to_string().contains("did not report its port"),
        "{reason}"
    );
    assert_eq!(fx.spawner.spawn_count(), 6);
    assert!(begin.elapsed() >= Duration::from_secs(10 * 6));
}

#[tokio::test(start_paused = true)]
async fn stale_port_files_are_removed_before_spawn() {
    let mut fx = fixture(vec![Script::exits(10)], true);

    std::fs::write(fx.dirs.port_file(), "4242").unwrap();
    std::fs::write(fx.dirs.addr_file(), "127.0.0.1:4242").unwrap();
    fx.supervisor.start(params()).await;
    wait_for(&mut fx.status, |s| *s == Status::Expired).await;

    assert_eq!(*lock(&fx.spawner.stale_seen), vec![false]);
    assert!(!fx.dirs.addr_file().exists());
}

#[tokio::test(start_paused = true)]
async fn recent_log_never_contains_the_key() {
    let mut script = Script::exits(1);

    script.lines = vec![
        "starting".into(),
        format!("debug: using key {KEY} for login"),
    ];

    let mut fx = fixture(vec![script; 10], true);

    fx.supervisor.start(params()).await;

    let status = wait_for(&mut fx.status, |s| matches!(s, Status::Failed { .. })).await;
    let Status::Failed { reason, recent_log } = status else {
        unreachable!()
    };

    assert!(recent_log.iter().all(|line| !line.contains(KEY)));
    assert!(recent_log.iter().any(|line| line.contains(REDACTED)));
    assert!(!reason.to_string().contains(KEY));
    assert!(
        fx.supervisor
            .recent_log()
            .iter()
            .all(|line| !line.contains(KEY))
    );
    assert!(recent_log.len() <= LOG_LINES);
    assert!(!format!("{:?}", params()).contains(KEY));
}

#[tokio::test(start_paused = true)]
async fn log_is_a_ring_buffer() {
    let mut script = Script::exits(10);

    script.lines = (0..500).map(|n| format!("line {n}")).collect();

    let mut fx = fixture(vec![script], true);

    fx.supervisor.start(params()).await;
    wait_for(&mut fx.status, |s| *s == Status::Expired).await;

    let log = fx.supervisor.recent_log();

    assert_eq!(log.len(), LOG_LINES);
    assert_eq!(log.last().unwrap(), "line 499");
}

#[tokio::test(start_paused = true)]
async fn stop_kills_the_child_and_reports_stopped() {
    let mut fx = fixture(vec![Script::hangs()], true);

    fx.supervisor.start(params()).await;
    wait_for(&mut fx.status, |s| *s == Status::Starting).await;
    fx.supervisor.stop().await;

    assert_eq!(fx.supervisor.status(), Status::Stopped);
}

#[tokio::test(start_paused = true)]
async fn process_id_is_known_only_while_the_child_runs() {
    let fx = fixture(vec![Script::hangs()], true);

    assert_eq!(fx.supervisor.process_id(), None);

    fx.supervisor.start(params()).await;
    sleep(Duration::from_secs(1)).await;

    assert_eq!(fx.supervisor.process_id(), Some(FAKE_PID));

    fx.supervisor.stop().await;

    assert_eq!(fx.supervisor.process_id(), None);
}

async fn serve_auth(logged_in: bool) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();

        socket
            .send(Message::text(format!(
                r#"{{"type":"auth_state","logged_in":{logged_in},"is_active":false,"device_name":"Sonear"}}"#
            )))
            .await
            .unwrap();

        sleep(Duration::from_secs(30)).await;
    });

    port
}

#[tokio::test]
async fn connects_and_reports_pairing_then_ready() {
    for (logged_in, expected) in [
        (
            false,
            Status::NeedsPairing {
                device_name: "Sonear".into(),
            },
        ),
        (
            true,
            Status::Ready {
                device_name: "Sonear".into(),
            },
        ),
    ] {
        let port = serve_auth(logged_in).await;
        let mut script = Script::hangs();

        script.port = Some(port);

        let mut fx = fixture(vec![script], true);

        fx.supervisor.start(params()).await;

        let status = wait_for(&mut fx.status, |s| {
            matches!(s, Status::NeedsPairing { .. } | Status::Ready { .. })
        })
        .await;

        assert_eq!(status, expected);
        assert!(fx.supervisor.client().borrow().is_some());

        fx.supervisor.stop().await;

        assert!(fx.supervisor.client().borrow().is_none());
    }
}

#[test]
fn backoff_follows_the_schedule_and_resets_after_stable_run() {
    let mut backoff = Backoff::default();
    let secs: Vec<_> = (0..5)
        .map(|_| {
            backoff
                .next_delay(Duration::from_secs(1))
                .unwrap()
                .as_secs()
        })
        .collect();

    assert_eq!(secs, [1, 2, 5, 10, 30]);
    assert_eq!(backoff.next_delay(Duration::from_secs(1)), None);

    let mut backoff = Backoff::default();

    for _ in 0..4 {
        backoff.next_delay(Duration::from_secs(1));
    }

    assert_eq!(
        backoff.next_delay(STABLE_AFTER),
        Some(Duration::from_secs(1))
    );
}

#[test]
fn args_follow_the_documented_flags() {
    let tmp = tempfile::tempdir().unwrap();
    let config = SupervisorConfig::new(SoloistDirs::new(tmp.path()));
    let args = build_args(&params(), KEY, &config);

    assert_eq!(args[..4], ["-n", "Sonear", "-k", KEY]);
    assert_eq!(args[4], "-D");
    assert_eq!(args[6], "-C");
    assert_eq!(args[8..], ["-z", "1024", "-i", "40", "-w", "127.0.0.1:0"]);
}
