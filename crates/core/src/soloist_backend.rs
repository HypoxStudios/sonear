use std::sync::Arc;

use sonear_soloist::SoloistClient;
use sonear_soloist::client::ClientEvent;
use sonear_soloist::protocol::Command;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, watch};
use tracing::{debug, warn};

use crate::player::{PlayerBackend, PlayerError};

/// Sends commands to whichever client the supervisor currently publishes and
/// forwards that client's events.
pub struct SoloistBackend {
    client: watch::Receiver<Option<SoloistClient>>,
    events: broadcast::Sender<ClientEvent>,
}

impl SoloistBackend {
    pub fn spawn(client: watch::Receiver<Option<SoloistClient>>, runtime: &Handle) -> Arc<Self> {
        let backend = Arc::new(Self {
            client: client.clone(),
            events: broadcast::channel(256).0,
        });

        runtime.spawn(forward(client, backend.events.clone()));

        backend
    }

    fn send(&self, command: Command) -> Result<(), PlayerError> {
        let client = self.client.borrow().clone();

        client
            .ok_or(PlayerError::NotConnected)?
            .send(command)
            .map_err(|_| PlayerError::NotConnected)
    }
}

async fn forward(
    mut client: watch::Receiver<Option<SoloistClient>>,
    events: broadcast::Sender<ClientEvent>,
) {
    loop {
        let current = client.borrow_and_update().clone();
        let mut replaced = false;

        if let Some(current) = current {
            let mut feed = current.subscribe();

            // The supervisor already consumed the greeting, so ask again.
            let _ = current.send(Command::GetAuthState);
            let _ = current.send(Command::GetState);

            loop {
                tokio::select! {
                    event = feed.recv() => match event {
                        Ok(event) => {
                            let _ = events.send(event);
                        }
                        Err(broadcast::error::RecvError::Lagged(missed)) => {
                            warn!(missed, "Soloist events lagged, requesting state");
                            let _ = current.send(Command::GetState);
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    },
                    changed = client.changed() => {
                        if changed.is_err() {
                            return;
                        }

                        replaced = true;
                        break;
                    }
                }
            }
        }

        if replaced {
            continue;
        }

        debug!("Soloist client gone");
        let _ = events.send(ClientEvent::Disconnected);

        if client.changed().await.is_err() {
            return;
        }
    }
}

impl PlayerBackend for SoloistBackend {
    fn subscribe(&self) -> broadcast::Receiver<ClientEvent> {
        self.events.subscribe()
    }

    fn request_state(&self) -> Result<(), PlayerError> {
        self.send(Command::GetAuthState)?;
        self.send(Command::GetState)
    }

    fn play(&self, uri: Option<&str>) -> Result<(), PlayerError> {
        self.send(Command::Play {
            uri: uri.map(str::to_owned),
        })
    }

    fn pause(&self) -> Result<(), PlayerError> {
        self.send(Command::Pause)
    }

    fn next(&self) -> Result<(), PlayerError> {
        self.send(Command::SkipNext)
    }

    fn previous(&self) -> Result<(), PlayerError> {
        self.send(Command::SkipPrev)
    }

    fn seek(&self, position_ms: u64) -> Result<(), PlayerError> {
        self.send(Command::Seek { position_ms })
    }

    fn set_volume(&self, volume: u8) -> Result<(), PlayerError> {
        self.send(Command::SetVolume {
            volume: volume.min(100),
        })
    }

    fn set_shuffle(&self, enabled: bool) -> Result<(), PlayerError> {
        self.send(Command::SetShuffle { enabled })
    }

    fn set_repeat_context(&self, enabled: bool) -> Result<(), PlayerError> {
        self.send(Command::SetRepeatContext { enabled })
    }

    fn set_repeat_track(&self, enabled: bool) -> Result<(), PlayerError> {
        self.send(Command::SetRepeatTrack { enabled })
    }

    fn add_to_queue(&self, uri: &str) -> Result<(), PlayerError> {
        self.send(Command::AddToQueue {
            uri: uri.to_owned(),
        })
    }

    fn activate(&self) -> Result<(), PlayerError> {
        self.send(Command::Activate)
    }
}
