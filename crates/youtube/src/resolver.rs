//! Turns a video id into a direct audio URL by running yt-dlp.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};
use tokio::process::Command;
use tracing::warn;
use url::Url;

use crate::error::ToolError;

const RUN_TIMEOUT: Duration = Duration::from_secs(30);
const EXPIRY_MARGIN: Duration = Duration::from_secs(5 * 60);
const DEFAULT_LIFETIME: Duration = Duration::from_secs(60 * 60);
const STDERR_TAIL: usize = 400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamUrl {
    pub url: String,
    pub expires_at: Option<SystemTime>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs a program to completion. Swapped for a fake in tests.
#[async_trait]
pub trait Runner: Send + Sync {
    async fn run(&self, exe: &Path, args: &[String]) -> Result<RunOutput, ToolError>;
}

pub struct ProcessRunner;

#[async_trait]
impl Runner for ProcessRunner {
    async fn run(&self, exe: &Path, args: &[String]) -> Result<RunOutput, ToolError> {
        let mut command = Command::new(exe);

        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        #[cfg(windows)]
        command.creation_flags(crate::CREATE_NO_WINDOW);

        let child = command.spawn().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ToolError::NotInstalled
            } else {
                ToolError::Spawn(error.to_string())
            }
        })?;
        let output = tokio::time::timeout(RUN_TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| ToolError::Timeout)??;

        Ok(RunOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// A JavaScript runtime yt-dlp can use to solve YouTube's challenges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsRuntime {
    pub name: String,
    pub path: PathBuf,
}

impl JsRuntime {
    /// Looks for `deno`, then `node`, on PATH.
    pub fn find() -> Option<Self> {
        let paths: Vec<PathBuf> = std::env::split_paths(&std::env::var_os("PATH")?).collect();

        ["deno", "node"].into_iter().find_map(|name| {
            let file = if cfg!(windows) {
                format!("{name}.exe")
            } else {
                name.to_owned()
            };

            paths
                .iter()
                .map(|dir| dir.join(&file))
                .find(|candidate| candidate.is_file())
                .map(|path| Self {
                    name: name.to_owned(),
                    path,
                })
        })
    }
}

#[derive(Clone)]
struct Cached {
    stream: StreamUrl,
    fresh_until: SystemTime,
}

type Pending = Shared<BoxFuture<'static, Result<StreamUrl, ToolError>>>;

struct Inner {
    ytdlp: PathBuf,
    runner: Arc<dyn Runner>,
    js_runtime: Option<JsRuntime>,
    cache: Mutex<HashMap<String, Cached>>,
    pending: Mutex<HashMap<String, Pending>>,
}

#[derive(Clone)]
pub struct Resolver {
    inner: Arc<Inner>,
}

pub fn valid_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn expiry_of(url: &str) -> Option<SystemTime> {
    let url = Url::parse(url).ok()?;
    let seconds: u64 = url
        .query_pairs()
        .find(|(key, _)| key == "expire")?
        .1
        .parse()
        .ok()?;

    Some(UNIX_EPOCH + Duration::from_secs(seconds))
}

fn tail(text: &str) -> String {
    let text = text.trim();
    let start = text
        .char_indices()
        .rev()
        .nth(STDERR_TAIL)
        .map_or(0, |(index, _)| index);

    text[start..].trim().to_owned()
}

impl Inner {
    fn arguments(&self, video_id: &str) -> Vec<String> {
        let mut args: Vec<String> = ["-f", "bestaudio", "-g", "--no-playlist", "--no-warnings"]
            .into_iter()
            .map(str::to_owned)
            .collect();

        if let Some(runtime) = &self.js_runtime {
            args.push("--js-runtimes".into());
            args.push(format!("{}:{}", runtime.name, runtime.path.display()));
        }

        args.push(format!("https://music.youtube.com/watch?v={video_id}"));

        args
    }

    async fn run(&self, video_id: &str) -> Result<StreamUrl, ToolError> {
        let output = self
            .runner
            .run(&self.ytdlp, &self.arguments(video_id))
            .await?;

        if !output.success {
            return Err(ToolError::Resolve(tail(&output.stderr)));
        }

        let url = output
            .stdout
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .filter(|line| line.starts_with("https://"))
            .ok_or_else(|| ToolError::Resolve(tail(&output.stderr)))?;

        Ok(StreamUrl {
            url: url.to_owned(),
            expires_at: expiry_of(url),
        })
    }

    fn remember(&self, video_id: &str, stream: &StreamUrl) {
        let fresh_until = match stream.expires_at {
            Some(expires) => expires.checked_sub(EXPIRY_MARGIN).unwrap_or(UNIX_EPOCH),
            None => SystemTime::now() + DEFAULT_LIFETIME,
        };

        lock(&self.cache).insert(
            video_id.to_owned(),
            Cached {
                stream: stream.clone(),
                fresh_until,
            },
        );
    }

    fn cached(&self, video_id: &str) -> Option<StreamUrl> {
        let mut cache = lock(&self.cache);
        let entry = cache.get(video_id)?;

        if entry.fresh_until > SystemTime::now() {
            return Some(entry.stream.clone());
        }

        cache.remove(video_id);

        None
    }
}

impl Resolver {
    /// Uses the real yt-dlp with no JavaScript runtime. With one, yt-dlp
    /// hands out web-client stream URLs that YouTube answers with 403 when
    /// mpv fetches them, while the URLs it picks without one play fine. Pass
    /// `JsRuntime::find()` to `with_runner` to try it anyway.
    pub fn new(ytdlp: impl Into<PathBuf>) -> Self {
        Self::with_runner(ytdlp, Arc::new(ProcessRunner), None)
    }

    pub fn with_runner(
        ytdlp: impl Into<PathBuf>,
        runner: Arc<dyn Runner>,
        js_runtime: Option<JsRuntime>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                ytdlp: ytdlp.into(),
                runner,
                js_runtime,
                cache: Mutex::new(HashMap::new()),
                pending: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Concurrent calls for one id share a single yt-dlp run.
    pub async fn resolve(&self, video_id: &str) -> Result<StreamUrl, ToolError> {
        if !valid_video_id(video_id) {
            return Err(ToolError::Resolve("not a video id".into()));
        }

        if let Some(hit) = self.inner.cached(video_id) {
            return Ok(hit);
        }

        let shared = {
            let mut pending = lock(&self.inner.pending);

            match pending.get(video_id) {
                Some(running) => running.clone(),
                None => {
                    let inner = self.inner.clone();
                    let id = video_id.to_owned();
                    let running = async move {
                        let result = inner.run(&id).await;

                        if let Ok(stream) = &result {
                            inner.remember(&id, stream);
                        }

                        lock(&inner.pending).remove(&id);

                        result
                    }
                    .boxed()
                    .shared();

                    pending.insert(video_id.to_owned(), running.clone());

                    running
                }
            }
        };

        shared.await
    }

    /// Starts resolving in the background so a later `resolve` is instant.
    pub fn prefetch(&self, video_id: &str) {
        if !valid_video_id(video_id) {
            return;
        }

        let resolver = self.clone();
        let id = video_id.to_owned();

        tokio::spawn(async move {
            if let Err(error) = resolver.resolve(&id).await {
                warn!(%error, "prefetch failed");
            }
        });
    }

    /// Drops a cached URL, for when mpv could not load it.
    pub fn forget(&self, video_id: &str) {
        lock(&self.inner.cache).remove(video_id);
    }
}

#[cfg(test)]
mod tests;
