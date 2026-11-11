use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

const ID: &str = "IluRBvnYMoY";

struct FakeRunner {
    calls: AtomicUsize,
    args: Mutex<Vec<Vec<String>>>,
    delay: Duration,
    output: Mutex<RunOutput>,
}

impl FakeRunner {
    fn new(stdout: &str) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            args: Mutex::new(Vec::new()),
            delay: Duration::ZERO,
            output: Mutex::new(RunOutput {
                success: true,
                stdout: stdout.to_owned(),
                stderr: String::new(),
            }),
        })
    }

    fn slow(stdout: &str) -> Arc<Self> {
        let runner = Self::new(stdout);

        Arc::new(Self {
            calls: AtomicUsize::new(0),
            args: Mutex::new(Vec::new()),
            delay: Duration::from_secs(5),
            output: Mutex::new(runner.output.lock().unwrap().clone()),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Runner for FakeRunner {
    async fn run(&self, _: &Path, args: &[String]) -> Result<RunOutput, ToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.args.lock().unwrap().push(args.to_vec());
        tokio::time::sleep(self.delay).await;

        Ok(self.output.lock().unwrap().clone())
    }
}

fn resolver(runner: &Arc<FakeRunner>) -> Resolver {
    Resolver::with_runner("yt-dlp.exe", runner.clone(), None)
}

fn url_expiring_in(seconds: u64) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();

    format!(
        "https://rr1.googlevideo.com/videoplayback?id=1&expire={}&mime=audio",
        now.as_secs() + seconds
    )
}

#[tokio::test]
async fn runs_yt_dlp_with_the_verified_arguments() {
    let runner = FakeRunner::new("https://rr1.googlevideo.com/videoplayback?id=1\n");
    let resolver = Resolver::with_runner(
        "yt-dlp.exe",
        runner.clone(),
        Some(JsRuntime {
            name: "node".into(),
            path: PathBuf::from("C:/node/node.exe"),
        }),
    );

    resolver.resolve(ID).await.unwrap();

    let args = runner.args.lock().unwrap()[0].clone();

    assert_eq!(
        args,
        [
            "-f",
            "bestaudio",
            "-g",
            "--no-playlist",
            "--no-warnings",
            "--js-runtimes",
            "node:C:/node/node.exe",
            "https://music.youtube.com/watch?v=IluRBvnYMoY",
        ]
    );
}

#[tokio::test]
async fn reads_the_expiry_from_the_url() {
    let url = "https://rr1.googlevideo.com/videoplayback?id=1&expire=1900000000&mime=audio";
    let runner = FakeRunner::new(&format!("\n{url}\n"));
    let stream = resolver(&runner).resolve(ID).await.unwrap();

    assert_eq!(stream.url, url);
    assert_eq!(
        stream.expires_at,
        Some(UNIX_EPOCH + Duration::from_secs(1_900_000_000))
    );
}

#[tokio::test]
async fn caches_until_shortly_before_expiry() {
    let runner = FakeRunner::new(&url_expiring_in(3600));
    let resolver = resolver(&runner);

    resolver.resolve(ID).await.unwrap();
    resolver.resolve(ID).await.unwrap();

    assert_eq!(runner.calls(), 1);

    let soon = FakeRunner::new(&url_expiring_in(120));
    let resolver = self::resolver(&soon);

    resolver.resolve(ID).await.unwrap();
    resolver.resolve(ID).await.unwrap();

    assert_eq!(soon.calls(), 2);
}

#[tokio::test]
async fn a_url_without_expiry_is_cached_for_an_hour() {
    let runner = FakeRunner::new("https://rr1.googlevideo.com/videoplayback?id=1");
    let resolver = resolver(&runner);

    resolver.resolve(ID).await.unwrap();
    resolver.resolve(ID).await.unwrap();

    assert_eq!(runner.calls(), 1);
    assert_eq!(resolver.resolve(ID).await.unwrap().expires_at, None);
}

#[tokio::test(start_paused = true)]
async fn concurrent_resolves_share_one_run() {
    let runner = FakeRunner::slow(&url_expiring_in(3600));
    let resolver = resolver(&runner);
    let (first, second, third) = tokio::join!(
        resolver.resolve(ID),
        resolver.resolve(ID),
        resolver.resolve(ID)
    );

    assert_eq!(runner.calls(), 1);
    assert_eq!(first, second);
    assert_eq!(second, third);
    assert!(first.is_ok());
}

#[tokio::test]
async fn forget_makes_the_next_resolve_run_again() {
    let runner = FakeRunner::new(&url_expiring_in(3600));
    let resolver = resolver(&runner);

    resolver.resolve(ID).await.unwrap();
    resolver.forget(ID);
    resolver.resolve(ID).await.unwrap();

    assert_eq!(runner.calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn prefetch_fills_the_cache() {
    let runner = FakeRunner::new(&url_expiring_in(3600));
    let resolver = resolver(&runner);

    resolver.prefetch(ID);
    tokio::time::sleep(Duration::from_millis(10)).await;
    resolver.resolve(ID).await.unwrap();

    assert_eq!(runner.calls(), 1);
}

#[tokio::test]
async fn failures_carry_the_end_of_stderr_and_are_not_cached() {
    let runner = FakeRunner::new("");

    *runner.output.lock().unwrap() = RunOutput {
        success: false,
        stdout: String::new(),
        stderr: format!("{}\nERROR: Sign in to confirm", "noise ".repeat(500)),
    };

    let resolver = resolver(&runner);
    let Err(ToolError::Resolve(detail)) = resolver.resolve(ID).await else {
        panic!("expected a resolve error");
    };

    assert!(detail.ends_with("ERROR: Sign in to confirm"));
    assert!(detail.len() <= STDERR_TAIL + 8);
    assert!(resolver.resolve(ID).await.is_err());
    assert_eq!(runner.calls(), 2);
}

#[tokio::test]
async fn output_that_is_not_a_url_is_an_error() {
    for stdout in ["", "WARNING: something\n", "http://insecure.example/a\n"] {
        let runner = FakeRunner::new(stdout);

        assert!(matches!(
            resolver(&runner).resolve(ID).await,
            Err(ToolError::Resolve(_))
        ));
    }
}

#[tokio::test]
async fn bad_ids_never_reach_the_runner() {
    let runner = FakeRunner::new("https://rr1.googlevideo.com/x");
    let resolver = resolver(&runner);

    for id in [
        "",
        "short",
        "IluRBvnYMoYY",
        "IluRBv nYMo",
        "a;calc.exe;b",
        "--version  ",
        "https://x.y",
    ] {
        assert!(resolver.resolve(id).await.is_err(), "{id}");
    }

    resolver.prefetch("not an id");
    assert_eq!(runner.calls(), 0);
}

#[test]
fn video_ids_are_eleven_url_safe_characters() {
    assert!(valid_video_id("IluRBvnYMoY"));
    assert!(valid_video_id("a_b-c_d-e_f"));
    assert!(!valid_video_id("a_b-c_d-e_"));
    assert!(!valid_video_id("a_b-c_d-e!f"));
}
