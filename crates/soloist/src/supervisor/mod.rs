//! Owns the Soloist child process: spawn, connect, restart with backoff.

mod process;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, timeout};
use tracing::{debug, info, warn};

use crate::client::{ClientEvent, SoloistClient};
use crate::protocol::Event;
use crate::version::{SoloistVersion, VersionError};

pub use process::ProcessSpawner;

pub const LOG_LINES: usize = 200;
pub const PORT_TIMEOUT: Duration = Duration::from_secs(10);
pub const STABLE_AFTER: Duration = Duration::from_secs(60);
pub const BACKOFF: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(30),
];

/// Exit code Soloist uses once its build has expired.
const EXIT_EXPIRED: i32 = 10;
const CONNECT_ATTEMPTS: u32 = 10;
const REDACTED: &str = "[redacted]";

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    NotInstalled,
    NeedsKey,
    Expired,
    Starting,
    NeedsPairing {
        device_name: String,
    },
    Ready {
        device_name: String,
    },
    Failed {
        reason: String,
        recent_log: Vec<String>,
    },
    Stopped,
}

/// `%LOCALAPPDATA%\Sonear\soloist` and its subdirectories.
#[derive(Debug, Clone)]
pub struct SoloistDirs {
    root: PathBuf,
}

impl SoloistDirs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn bin(&self) -> PathBuf {
        self.root.join("bin")
    }

    pub fn data(&self) -> PathBuf {
        self.root.join("data")
    }

    pub fn cache(&self) -> PathBuf {
        self.root.join("cache")
    }

    pub fn exe(&self) -> PathBuf {
        self.bin().join(crate::install::EXE_NAME)
    }

    pub fn port_file(&self) -> PathBuf {
        self.data().join("ws.port")
    }

    pub fn addr_file(&self) -> PathBuf {
        self.data().join("ws.addr")
    }
}

#[derive(Clone)]
pub struct SupervisorConfig {
    pub dirs: SoloistDirs,
    pub cache_mb: u32,
    pub port_timeout: Duration,
}

impl SupervisorConfig {
    pub fn new(dirs: SoloistDirs) -> Self {
        Self {
            dirs,
            cache_mb: 1024,
            port_timeout: PORT_TIMEOUT,
        }
    }
}

/// Everything needed for one launch. The key never appears in `Debug`.
#[derive(Clone)]
pub struct LaunchParams {
    pub device_name: String,
    pub key: Option<String>,
    pub volume: u8,
}

impl fmt::Debug for LaunchParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LaunchParams")
            .field("device_name", &self.device_name)
            .field("key", &self.key.as_ref().map(|_| REDACTED))
            .field("volume", &self.volume)
            .finish()
    }
}

pub struct SpawnedProcess {
    pub handle: Box<dyn ProcessHandle>,
    /// Lines from stdout and stderr, closed when the process ends.
    pub output: mpsc::UnboundedReceiver<String>,
}

#[async_trait]
pub trait ProcessHandle: Send {
    /// Waits for exit and returns the exit code. Must be cancel-safe.
    async fn wait(&mut self) -> io::Result<Option<i32>>;

    async fn kill(&mut self);
}

#[async_trait]
pub trait Spawner: Send + Sync {
    fn spawn(&self, exe: &Path, args: &[String]) -> io::Result<SpawnedProcess>;

    async fn version(&self, exe: &Path) -> Result<SoloistVersion, VersionError>;
}

fn build_args(params: &LaunchParams, key: &str, config: &SupervisorConfig) -> Vec<String> {
    vec![
        "-n".into(),
        params.device_name.clone(),
        "-k".into(),
        key.to_owned(),
        "-D".into(),
        config.dirs.data().to_string_lossy().into_owned(),
        "-C".into(),
        config.dirs.cache().to_string_lossy().into_owned(),
        "-z".into(),
        config.cache_mb.to_string(),
        "-i".into(),
        params.volume.min(100).to_string(),
        "-w".into(),
        "127.0.0.1:0".into(),
    ]
}

fn redact(line: &str, key: &str) -> String {
    if key.is_empty() || !line.contains(key) {
        return line.to_owned();
    }

    line.replace(key, REDACTED)
}

/// Restart policy: delays of 1, 2, 5, 10 and 30 seconds, then give up.
/// Running stably for a minute resets the count.
#[derive(Debug, Default)]
struct Backoff {
    failures: usize,
}

impl Backoff {
    fn next_delay(&mut self, ran_for: Duration) -> Option<Duration> {
        if ran_for >= STABLE_AFTER {
            self.failures = 0;
        }

        self.failures += 1;

        BACKOFF.get(self.failures - 1).copied()
    }
}

struct Shared {
    spawner: Arc<dyn Spawner>,
    config: SupervisorConfig,
    status: Mutex<Status>,
    status_tx: broadcast::Sender<Status>,
    client_tx: watch::Sender<Option<SoloistClient>>,
    log: Mutex<VecDeque<String>>,
    version: Mutex<Option<SoloistVersion>>,
}

impl Shared {
    fn set_status(&self, status: Status) {
        {
            let mut current = lock(&self.status);

            if *current == status {
                return;
            }

            *current = status.clone();
        }

        debug!(?status, "soloist status");
        let _ = self.status_tx.send(status);
    }

    fn push_log(&self, line: String) {
        let mut log = lock(&self.log);

        if log.len() == LOG_LINES {
            log.pop_front();
        }

        log.push_back(line);
    }

    fn recent_log(&self) -> Vec<String> {
        lock(&self.log).iter().cloned().collect()
    }

    fn fail(&self, reason: String) {
        self.set_status(Status::Failed {
            reason,
            recent_log: self.recent_log(),
        });
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct RunHandle {
    cancel: watch::Sender<bool>,
    join: JoinHandle<()>,
}

#[derive(Clone)]
pub struct Supervisor {
    shared: Arc<Shared>,
    run: Arc<Mutex<Option<RunHandle>>>,
}

impl Supervisor {
    pub fn new(config: SupervisorConfig, spawner: Arc<dyn Spawner>) -> Self {
        let (status_tx, _) = broadcast::channel(32);
        let (client_tx, _) = watch::channel(None);
        let initial = if config.dirs.exe().exists() {
            Status::Stopped
        } else {
            Status::NotInstalled
        };

        Self {
            shared: Arc::new(Shared {
                spawner,
                config,
                status: Mutex::new(initial),
                status_tx,
                client_tx,
                log: Mutex::new(VecDeque::new()),
                version: Mutex::new(None),
            }),
            run: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_process_spawner(config: SupervisorConfig) -> Self {
        Self::new(config, Arc::new(ProcessSpawner::new()))
    }

    pub fn dirs(&self) -> &SoloistDirs {
        &self.shared.config.dirs
    }

    pub fn status(&self) -> Status {
        lock(&self.shared.status).clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Status> {
        self.shared.status_tx.subscribe()
    }

    /// The live client, or `None` while Soloist is not connected.
    pub fn client(&self) -> watch::Receiver<Option<SoloistClient>> {
        self.shared.client_tx.subscribe()
    }

    /// Version of the installed build, known after the last launch or `refresh_version`.
    pub fn version(&self) -> Option<SoloistVersion> {
        lock(&self.shared.version).clone()
    }

    pub async fn refresh_version(&self) -> Option<SoloistVersion> {
        let exe = self.shared.config.dirs.exe();

        if !exe.exists() {
            return None;
        }

        let version = self.shared.spawner.version(&exe).await.ok()?;

        *lock(&self.shared.version) = Some(version.clone());

        Some(version)
    }

    pub fn recent_log(&self) -> Vec<String> {
        self.shared.recent_log()
    }

    /// Launches Soloist, replacing a previous run. Missing install or key
    /// are reported as status instead of an error.
    pub async fn start(&self, params: LaunchParams) {
        self.halt().await;

        let shared = self.shared.clone();

        if !shared.config.dirs.exe().exists() {
            shared.set_status(Status::NotInstalled);
            return;
        }

        let Some(key) = params.key.clone().filter(|key| !key.trim().is_empty()) else {
            shared.set_status(Status::NeedsKey);
            return;
        };

        shared.set_status(Status::Starting);

        let (cancel, cancel_rx) = watch::channel(false);
        let join = tokio::spawn(run(shared, params, key, cancel_rx));

        *lock(&self.run) = Some(RunHandle { cancel, join });
    }

    pub async fn restart(&self, params: LaunchParams) {
        self.start(params).await;
    }

    pub async fn stop(&self) {
        self.halt().await;

        if !matches!(self.status(), Status::NotInstalled) {
            self.shared.set_status(Status::Stopped);
        }
    }

    async fn halt(&self) {
        let handle = lock(&self.run).take();

        if let Some(handle) = handle {
            let _ = handle.cancel.send(true);
            let _ = handle.join.await;
        }

        self.shared.client_tx.send_replace(None);
    }
}

enum Outcome {
    Cancelled,
    Expired,
    Failed { reason: String, ran_for: Duration },
}

async fn run(
    shared: Arc<Shared>,
    params: LaunchParams,
    key: String,
    mut cancel: watch::Receiver<bool>,
) {
    let exe = shared.config.dirs.exe();

    match shared.spawner.version(&exe).await {
        Ok(version) => {
            let today = chrono::Utc::now().date_naive();
            let expired = version.is_expired(today);

            *lock(&shared.version) = Some(version);

            if expired {
                shared.set_status(Status::Expired);
                return;
            }
        }
        Err(error) => warn!(%error, "could not read the Soloist version"),
    }

    let mut backoff = Backoff::default();

    loop {
        let outcome = run_once(&shared, &params, &key, &mut cancel).await;

        shared.client_tx.send_replace(None);

        let (reason, ran_for) = match outcome {
            Outcome::Cancelled => return,
            Outcome::Expired => {
                shared.set_status(Status::Expired);
                return;
            }
            Outcome::Failed { reason, ran_for } => (reason, ran_for),
        };

        warn!(%reason, "Soloist stopped");

        let Some(delay) = backoff.next_delay(ran_for) else {
            shared.fail(reason);
            return;
        };

        shared.set_status(Status::Starting);

        tokio::select! {
            _ = sleep(delay) => {}
            _ = cancel.changed() => return,
        }
    }
}

async fn run_once(
    shared: &Arc<Shared>,
    params: &LaunchParams,
    key: &str,
    cancel: &mut watch::Receiver<bool>,
) -> Outcome {
    let config = &shared.config;
    let dirs = &config.dirs;

    for dir in [dirs.data(), dirs.cache()] {
        if let Err(error) = std::fs::create_dir_all(&dir) {
            return failed(format!("could not create {}: {error}", dir.display()));
        }
    }

    let _ = std::fs::remove_file(dirs.port_file());
    let _ = std::fs::remove_file(dirs.addr_file());

    let args = build_args(params, key, config);
    let SpawnedProcess { mut handle, output } = match shared.spawner.spawn(&dirs.exe(), &args) {
        Ok(process) => process,
        Err(error) => return failed(format!("could not start Soloist: {error}")),
    };
    let started = Instant::now();
    let pump = tokio::spawn(pump_log(shared.clone(), output, key.to_owned()));

    info!("started Soloist");

    let outcome = supervise(shared, params, &mut *handle, started, cancel).await;

    handle.kill().await;

    let _ = timeout(Duration::from_millis(500), pump).await;

    outcome
}

fn failed(reason: String) -> Outcome {
    Outcome::Failed {
        reason,
        ran_for: Duration::ZERO,
    }
}

async fn supervise(
    shared: &Arc<Shared>,
    params: &LaunchParams,
    handle: &mut dyn ProcessHandle,
    started: Instant,
    cancel: &mut watch::Receiver<bool>,
) -> Outcome {
    let established = tokio::select! {
        result = establish(shared) => result,
        code = handle.wait() => return exited(code, started),
        _ = cancel.changed() => {
            handle.kill().await;
            shared.set_status(Status::Stopped);
            return Outcome::Cancelled;
        }
    };
    let (client, mut events) = match established {
        Ok(connection) => connection,
        Err(reason) => {
            return Outcome::Failed {
                reason,
                ran_for: started.elapsed(),
            };
        }
    };

    shared.client_tx.send_replace(Some(client));

    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(ClientEvent::Event(Event::AuthState { logged_in, device_name, .. })) => {
                    let device_name = if device_name.is_empty() {
                        params.device_name.clone()
                    } else {
                        device_name
                    };

                    shared.set_status(if logged_in {
                        Status::Ready { device_name }
                    } else {
                        Status::NeedsPairing { device_name }
                    });
                }
                Ok(ClientEvent::Event(_)) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Ok(ClientEvent::Disconnected) | Err(broadcast::error::RecvError::Closed) => {
                    return Outcome::Failed {
                        reason: "lost the connection to Soloist".to_owned(),
                        ran_for: started.elapsed(),
                    };
                }
            },
            code = handle.wait() => return exited(code, started),
            _ = cancel.changed() => {
                handle.kill().await;
                shared.set_status(Status::Stopped);
                return Outcome::Cancelled;
            }
        }
    }
}

fn exited(code: io::Result<Option<i32>>, started: Instant) -> Outcome {
    let ran_for = started.elapsed();

    match code {
        Ok(Some(EXIT_EXPIRED)) => Outcome::Expired,
        Ok(Some(code)) => Outcome::Failed {
            reason: format!("Soloist exited with code {code}"),
            ran_for,
        },
        Ok(None) => Outcome::Failed {
            reason: "Soloist was terminated".to_owned(),
            ran_for,
        },
        Err(error) => Outcome::Failed {
            reason: format!("could not wait for Soloist: {error}"),
            ran_for,
        },
    }
}

type Connection = (SoloistClient, broadcast::Receiver<ClientEvent>);

async fn establish(shared: &Shared) -> Result<Connection, String> {
    let dirs = &shared.config.dirs;
    let port_file = dirs.port_file();
    let deadline = Instant::now() + shared.config.port_timeout;

    let port = loop {
        if let Some(port) = read_port(&port_file) {
            break port;
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "Soloist did not report its port within {} seconds",
                shared.config.port_timeout.as_secs()
            ));
        }

        sleep(Duration::from_millis(100)).await;
    };
    let addr = format!("127.0.0.1:{port}");
    let mut last_error = String::new();

    for _ in 0..CONNECT_ATTEMPTS {
        match SoloistClient::connect(&addr).await {
            Ok(connection) => return Ok(connection),
            Err(error) => last_error = error.to_string(),
        }

        sleep(Duration::from_millis(250)).await;
    }

    Err(last_error)
}

fn read_port(path: &Path) -> Option<u16> {
    std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse()
        .ok()
        .filter(|port| *port != 0)
}

async fn pump_log(shared: Arc<Shared>, mut output: mpsc::UnboundedReceiver<String>, key: String) {
    while let Some(line) = output.recv().await {
        shared.push_log(redact(&line, &key));
    }
}
