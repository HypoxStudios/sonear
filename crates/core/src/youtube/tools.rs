//! Keeps mpv and yt-dlp installed and fresh, the way `SoloistManager` does
//! for Soloist.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sonear_youtube::ToolError;
use sonear_youtube::tools::{Installer, Tool, ToolPaths, ToolProgress, ToolVersions};
use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tracing::{info, warn};

use crate::events::AppEvent;

/// How often a running download publishes its progress.
const PROGRESS_EVERY: Duration = Duration::from_millis(150);
const UPDATE_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, PartialEq)]
pub enum ToolsStatus {
    Missing,
    Installing {
        tool: Tool,
        received: u64,
        total: Option<u64>,
    },
    Ready,
    Failed(String),
}

pub struct YoutubeTools {
    paths: ToolPaths,
    installer: Installer,
    status: Mutex<ToolsStatus>,
    events: broadcast::Sender<AppEvent>,
    installing: tokio::sync::Mutex<()>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl YoutubeTools {
    pub fn new(
        paths: ToolPaths,
        installer: Installer,
        events: broadcast::Sender<AppEvent>,
    ) -> Arc<Self> {
        let status = if ToolVersions::installed(&paths).is_complete() {
            ToolsStatus::Ready
        } else {
            ToolsStatus::Missing
        };

        Arc::new(Self {
            paths,
            installer,
            status: Mutex::new(status),
            events,
            installing: tokio::sync::Mutex::new(()),
        })
    }

    pub fn status(&self) -> ToolsStatus {
        lock(&self.status).clone()
    }

    pub fn versions(&self) -> ToolVersions {
        ToolVersions::installed(&self.paths)
    }

    pub fn paths(&self) -> &ToolPaths {
        &self.paths
    }

    fn set_status(&self, status: ToolsStatus) {
        *lock(&self.status) = status.clone();
        let _ = self.events.send(AppEvent::Tools(status));
    }

    /// Downloads whichever tool is missing. Does nothing when both are there
    /// or another install is running.
    pub async fn install(&self) -> Result<(), ToolError> {
        let Ok(_running) = self.installing.try_lock() else {
            return Ok(());
        };

        if ToolVersions::installed(&self.paths).is_complete() {
            self.set_status(ToolsStatus::Ready);

            return Ok(());
        }

        self.set_status(ToolsStatus::Installing {
            tool: Tool::Mpv,
            received: 0,
            total: None,
        });

        let last = Mutex::new(None::<(Tool, Instant)>);
        let progress = |event: ToolProgress| {
            let ToolProgress::Downloading {
                tool,
                received,
                total,
            } = event
            else {
                return;
            };
            let mut last = lock(&last);
            let due = match *last {
                Some((previous, at)) => previous != tool || at.elapsed() >= PROGRESS_EVERY,
                None => true,
            };

            if !due && total.is_none_or(|total| received < total) {
                return;
            }

            *last = Some((tool, Instant::now()));
            self.set_status(ToolsStatus::Installing {
                tool,
                received,
                total,
            });
        };

        match self.installer.install_missing(&progress).await {
            Ok(()) => {
                self.set_status(ToolsStatus::Ready);

                Ok(())
            }
            Err(error) => {
                warn!(%error, "could not install the playback tools");
                self.set_status(ToolsStatus::Failed(error.user_message()));

                Err(error)
            }
        }
    }

    /// Installs a newer yt-dlp when there is one. Returns whether it changed.
    pub async fn update_ytdlp(&self) -> Result<bool, ToolError> {
        self.installer.update_ytdlp(&|_| {}).await
    }

    /// Publishes the starting status and keeps yt-dlp current while the
    /// tools are installed: it breaks often, so it is checked at start and
    /// then once a day.
    pub fn run(self: &Arc<Self>, runtime: &Handle) {
        let tools = self.clone();

        runtime.spawn(async move {
            let _ = tools.events.send(AppEvent::Tools(tools.status()));

            loop {
                if tools.status() == ToolsStatus::Ready {
                    match tools.update_ytdlp().await {
                        Ok(true) => info!("updated yt-dlp"),
                        Ok(false) => {}
                        Err(error) => warn!(%error, "could not check for a yt-dlp update"),
                    }
                }

                tokio::time::sleep(UPDATE_EVERY).await;
            }
        });
    }
}

#[cfg(test)]
mod tests;
