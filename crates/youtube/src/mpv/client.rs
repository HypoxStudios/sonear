use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, oneshot};
use tracing::{debug, warn};

use super::protocol::{self, Incoming, MpvEvent};
use crate::error::ToolError;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const EVENT_BUFFER: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDevice {
    pub name: String,
    pub description: String,
}

type Reply = Result<Value, ToolError>;

#[derive(Default)]
struct State {
    pending: HashMap<u64, oneshot::Sender<Reply>>,
    closed: bool,
}

struct Inner {
    writer: tokio::sync::Mutex<Box<dyn AsyncWrite + Send + Unpin>>,
    state: Mutex<State>,
    next_id: AtomicU64,
    events: broadcast::Sender<MpvEvent>,
    timeout: Duration,
}

/// Talks to one mpv over any duplex transport (a named pipe in production).
#[derive(Clone)]
pub struct MpvClient {
    inner: Arc<Inner>,
}

fn lock(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl MpvClient {
    pub async fn connect<T>(transport: T) -> Self
    where
        T: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        Self::connect_with_timeout(transport, COMMAND_TIMEOUT).await
    }

    pub async fn connect_with_timeout<T>(transport: T, timeout: Duration) -> Self
    where
        T: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        let (reader, writer) = tokio::io::split(transport);
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        let client = Self {
            inner: Arc::new(Inner {
                writer: tokio::sync::Mutex::new(Box::new(writer)),
                state: Mutex::new(State::default()),
                next_id: AtomicU64::new(1),
                events,
                timeout,
            }),
        };

        tokio::spawn(read_loop(client.inner.clone(), reader));

        for (id, name) in [(1, "pause"), (2, "duration"), (3, "paused-for-cache")] {
            let line = protocol::encode(None, &protocol::observe_property(id, name));

            if let Err(error) = client.write(&line).await {
                warn!(%error, name, "could not observe an mpv property");
            }
        }

        client
    }

    pub fn events(&self) -> broadcast::Receiver<MpvEvent> {
        self.inner.events.subscribe()
    }

    async fn write(&self, line: &str) -> Result<(), ToolError> {
        let mut writer = self.inner.writer.lock().await;

        writer
            .write_all(line.as_bytes())
            .await
            .map_err(|_| ToolError::Disconnected)?;
        writer.flush().await.map_err(|_| ToolError::Disconnected)
    }

    async fn request(&self, command: Vec<Value>) -> Result<Value, ToolError> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed) + 1000;
        let (sender, receiver) = oneshot::channel();

        {
            let mut state = lock(&self.inner.state);

            if state.closed {
                return Err(ToolError::Disconnected);
            }

            state.pending.insert(id, sender);
        }

        if let Err(error) = self.write(&protocol::encode(Some(id), &command)).await {
            lock(&self.inner.state).pending.remove(&id);

            return Err(error);
        }

        match tokio::time::timeout(self.inner.timeout, receiver).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => Err(ToolError::Disconnected),
            Err(_) => {
                lock(&self.inner.state).pending.remove(&id);

                Err(ToolError::Timeout)
            }
        }
    }

    async fn command(&self, command: Vec<Value>) -> Result<(), ToolError> {
        self.request(command).await.map(|_| ())
    }

    pub async fn load(&self, url: &str) -> Result<(), ToolError> {
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err(ToolError::Mpv("refusing to load a non-HTTP address".into()));
        }

        self.command(protocol::loadfile(url)).await
    }

    pub async fn stop(&self) -> Result<(), ToolError> {
        self.command(protocol::stop()).await
    }

    pub async fn set_pause(&self, paused: bool) -> Result<(), ToolError> {
        self.command(protocol::set_property("pause", paused)).await
    }

    pub async fn seek(&self, position_ms: u64) -> Result<(), ToolError> {
        self.command(protocol::seek(position_ms)).await
    }

    pub async fn set_volume(&self, volume: u8) -> Result<(), ToolError> {
        self.command(protocol::set_property("volume", volume)).await
    }

    /// `None` when nothing is loaded or mpv does not answer.
    pub async fn position_ms(&self) -> Option<u64> {
        let seconds = self
            .request(protocol::get_property("time-pos"))
            .await
            .ok()?
            .as_f64()?;

        (seconds >= 0.0).then(|| (seconds * 1000.0).round() as u64)
    }

    pub async fn audio_devices(&self) -> Result<Vec<AudioDevice>, ToolError> {
        let list = self
            .request(protocol::get_property("audio-device-list"))
            .await?;

        Ok(list
            .as_array()
            .map(|devices| {
                devices
                    .iter()
                    .filter_map(|device| {
                        Some(AudioDevice {
                            name: device.get("name")?.as_str()?.to_owned(),
                            description: device
                                .get("description")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    pub async fn set_audio_device(&self, name: &str) -> Result<(), ToolError> {
        self.command(protocol::set_property("audio-device", name))
            .await
    }
}

async fn read_loop<R: AsyncRead + Unpin>(inner: Arc<Inner>, reader: R) {
    let mut lines = BufReader::new(reader).lines();

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => match protocol::parse_line(&line) {
                Incoming::Reply { request_id, result } => {
                    let waiting = lock(&inner.state).pending.remove(&request_id);

                    if let Some(waiting) = waiting {
                        let _ = waiting.send(result.map_err(ToolError::Mpv));
                    }
                }
                Incoming::Event(event) => {
                    let _ = inner.events.send(event);
                }
                Incoming::Ignored => {}
            },
            Ok(None) => break,
            Err(error) => {
                debug!(%error, "the mpv connection broke");

                break;
            }
        }
    }

    let pending = {
        let mut state = lock(&inner.state);

        state.closed = true;

        std::mem::take(&mut state.pending)
    };

    for (_, waiting) in pending {
        let _ = waiting.send(Err(ToolError::Disconnected));
    }

    let _ = inner.events.send(MpvEvent::Exited);
}

#[cfg(test)]
mod tests;
