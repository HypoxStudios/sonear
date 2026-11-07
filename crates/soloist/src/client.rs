use futures_util::{SinkExt, StreamExt};
use thiserror::Error;
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};

use crate::protocol::{Command, Event};

const EVENT_BUFFER: usize = 256;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("could not connect to Soloist: {0}")]
    Connect(String),
    #[error("the connection to Soloist is closed")]
    Closed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientEvent {
    Event(Event),
    Disconnected,
}

/// Cloneable handle to one WebSocket connection. The connection closes once
/// every clone is dropped or `close` is called.
#[derive(Debug, Clone)]
pub struct SoloistClient {
    commands: mpsc::UnboundedSender<Command>,
    events: broadcast::Sender<ClientEvent>,
}

impl SoloistClient {
    /// Connects to `addr` (`host:port`). The returned receiver is subscribed
    /// before the first frame is read, so the initial `auth_state` is not lost.
    pub async fn connect(
        addr: &str,
    ) -> Result<(Self, broadcast::Receiver<ClientEvent>), ClientError> {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
            .await
            .map_err(|error| ClientError::Connect(error.to_string()))?;
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (events, first) = broadcast::channel(EVENT_BUFFER);

        tokio::spawn(run(socket, command_rx, events.clone()));

        Ok((Self { commands, events }, first))
    }

    pub fn send(&self, command: Command) -> Result<(), ClientError> {
        self.commands.send(command).map_err(|_| ClientError::Closed)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ClientEvent> {
        self.events.subscribe()
    }

    pub fn is_closed(&self) -> bool {
        self.commands.is_closed()
    }
}

async fn run<S>(
    socket: tokio_tungstenite::WebSocketStream<S>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: broadcast::Sender<ClientEvent>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut sink, mut stream) = socket.split();

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else {
                    let _ = sink.close().await;
                    break;
                };

                if let Err(error) = sink.send(Message::text(command.to_json())).await {
                    warn!(%error, "failed to send command to Soloist");
                    break;
                }
            }
            frame = stream.next() => {
                match frame {
                    Some(Ok(Message::Text(text))) => forward(text.as_str(), &events),
                    Some(Ok(Message::Binary(bytes))) => match std::str::from_utf8(&bytes) {
                        Ok(text) => forward(text, &events),
                        Err(_) => warn!("skipped non-UTF-8 frame from Soloist"),
                    },
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        debug!(%error, "Soloist connection error");
                        break;
                    }
                }
            }
        }
    }

    commands.close();
    let _ = events.send(ClientEvent::Disconnected);
}

fn forward(text: &str, events: &broadcast::Sender<ClientEvent>) {
    match Event::parse(text) {
        Ok(event) => {
            let _ = events.send(ClientEvent::Event(event));
        }
        Err(error) => warn!(%error, "skipped malformed frame from Soloist"),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::net::TcpListener;
    use tokio::time::timeout;

    use super::*;

    async fn fake_server() -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();

        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut received = Vec::new();

            socket
                .send(Message::text(
                    r#"{"type":"auth_state","logged_in":true,"is_active":false,"device_name":"X"}"#,
                ))
                .await
                .unwrap();
            socket.send(Message::text("not json at all")).await.unwrap();
            socket
                .send(Message::text(r#"{"type":"mystery"}"#))
                .await
                .unwrap();

            while let Some(Ok(message)) = socket.next().await {
                if let Message::Text(text) = message {
                    received.push(text.as_str().to_owned());

                    if received.len() == 2 {
                        break;
                    }
                }
            }

            received
        });

        (addr, handle)
    }

    #[tokio::test]
    async fn receives_events_sends_commands_and_reports_disconnect() {
        let (addr, server) = fake_server().await;
        let (client, mut events) = SoloistClient::connect(&addr).await.unwrap();
        let wait = Duration::from_secs(5);

        let first = timeout(wait, events.recv()).await.unwrap().unwrap();

        assert!(matches!(
            first,
            ClientEvent::Event(Event::AuthState {
                logged_in: true,
                ..
            })
        ));

        let second = timeout(wait, events.recv()).await.unwrap().unwrap();

        assert_eq!(second, ClientEvent::Event(Event::Unknown));

        client.send(Command::Pause).unwrap();
        client.send(Command::SetVolume { volume: 42 }).unwrap();

        let received = timeout(wait, server).await.unwrap().unwrap();
        let parsed: Vec<serde_json::Value> = received
            .iter()
            .map(|text| serde_json::from_str(text).unwrap())
            .collect();

        assert_eq!(
            parsed[0],
            serde_json::json!({"type": "command", "command": "pause"})
        );
        assert_eq!(
            parsed[1],
            serde_json::json!({"type": "command", "command": "set_volume", "volume": 42})
        );

        let last = timeout(wait, events.recv()).await.unwrap().unwrap();

        assert_eq!(last, ClientEvent::Disconnected);
    }

    #[tokio::test]
    async fn connect_failure_is_an_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();

        drop(listener);

        assert!(matches!(
            SoloistClient::connect(&addr).await,
            Err(ClientError::Connect(_))
        ));
    }
}
