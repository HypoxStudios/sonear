//! The store's side of auto-updates: the background task's results, and the
//! restart into a downloaded update, which only the user starts.

use rust_i18n::t;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::Context;
use sonear_core::data_dir::same_place;
use sonear_core::update::{Prepared, UpdateEvent, Updater};
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

use super::AppStore;
use crate::{installer, platform_window};

#[derive(Debug, Clone, PartialEq)]
pub enum UpdateState {
    Idle,
    Downloading {
        received: u64,
        total: Option<u64>,
    },
    /// A verified download is waiting for the user to restart into it.
    Ready {
        version: String,
        file: PathBuf,
    },
}

impl UpdateState {
    pub fn ready_version(&self) -> Option<&str> {
        match self {
            Self::Ready { version, .. } => Some(version),
            _ => None,
        }
    }
}

/// The running background task.
pub(super) struct UpdateTask {
    updater: Arc<Updater>,
    enabled: watch::Sender<bool>,
}

impl From<UpdateEvent> for UpdateState {
    fn from(event: UpdateEvent) -> Self {
        match event {
            UpdateEvent::Downloading { received, total } => Self::Downloading { received, total },
            UpdateEvent::Ready(prepared) => Self::Ready {
                version: prepared.version,
                file: prepared.file,
            },
            UpdateEvent::Idle => Self::Idle,
        }
    }
}

/// `SONEAR_DEMO_UPDATE=0.2.0` starts the demo with that update ready.
#[cfg(feature = "demo")]
fn demo_update() -> Option<UpdateState> {
    let version = std::env::var("SONEAR_DEMO_UPDATE").ok()?;

    Some(UpdateState::Ready {
        version,
        file: std::env::current_exe().unwrap_or_default(),
    })
}

#[cfg(not(feature = "demo"))]
fn demo_update() -> Option<UpdateState> {
    None
}

/// Only the installed program updates itself; a build run from `target` never does.
fn is_the_installed_program() -> bool {
    let Some(installed) = installer::installed() else {
        return false;
    };

    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| same_place(dir, &installed.dir)))
        .unwrap_or(false)
}

impl AppStore {
    /// Starts the background checks, if this build is allowed to run them.
    /// The demo build never touches the network.
    pub(super) fn start_updates(&mut self, cx: &mut Context<Self>) {
        if self.is_demo() {
            if let Some(state) = demo_update() {
                self.update = state;
            }

            return;
        }

        if !is_the_installed_program() {
            return;
        }

        let Some(updater) = Updater::for_build(&self.core.paths) else {
            return;
        };
        let updater = Arc::new(updater);
        let (enabled, watcher) = watch::channel(self.check_for_updates);
        let (events, mut inbox) = mpsc::unbounded_channel();

        self.updates = Some(UpdateTask {
            updater: updater.clone(),
            enabled,
        });
        self.runtime.spawn(updater.run(watcher, events));

        cx.spawn(async move |this, cx| {
            while let Some(event) = inbox.recv().await {
                let next = UpdateState::from(event);
                let applied = this.update(cx, |store, cx| {
                    let keep_ready = store.update.ready_version().is_some()
                        && !matches!(next, UpdateState::Ready { .. });

                    if keep_ready || store.update == next {
                        return;
                    }

                    store.update = next;
                    cx.notify();
                });

                if applied.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Turns the checks on or off. Takes effect at once.
    pub fn set_check_for_updates(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.check_for_updates = enabled;
        self.core
            .config
            .update(|config| config.check_for_updates = enabled);

        if let Some(task) = &self.updates {
            task.enabled.send_replace(enabled);
        }

        cx.notify();
    }

    /// Verifies the downloaded update once more, then starts it and quits. The
    /// update replaces the installed program once this process has gone.
    pub fn restart_to_update(&mut self, cx: &mut Context<Self>) {
        let UpdateState::Ready { version, file } = self.update.clone() else {
            return;
        };

        if self.update_restarting {
            return;
        }

        self.update_restarting = true;

        if self.is_demo() {
            let file = std::env::current_exe().unwrap_or(file);

            self.launch_update(&file, PathBuf::from(r"C:\Demo\Sonear"), cx);

            return;
        }

        let (Some(task), Some(installed)) = (&self.updates, installer::installed()) else {
            self.update_failed(t!("store.update.not_installed"), cx);

            return;
        };
        let updater = task.updater.clone();
        let prepared = Prepared {
            version,
            file: file.clone(),
            notes_url: None,
        };
        let (done, mut inbox) = mpsc::unbounded_channel();

        self.runtime.spawn(async move {
            let outcome = tokio::task::spawn_blocking(move || updater.verify_prepared(&prepared))
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));

            let _ = done.send(outcome);
        });

        cx.spawn(async move |this, cx| {
            let Some(outcome) = inbox.recv().await else {
                return;
            };

            let _ = this.update(cx, |store, cx| match outcome {
                Ok(()) => store.launch_update(&file, installed.dir, cx),
                Err(reason) => {
                    warn!(%reason, "the downloaded update did not pass its last check");
                    store.update = UpdateState::Idle;
                    store.update_failed(t!("store.update.check_failed"), cx);
                }
            });
        })
        .detach();
    }

    fn launch_update(&mut self, file: &std::path::Path, dir: PathBuf, cx: &mut Context<Self>) {
        match installer::apply::launch(file, &dir) {
            Ok(pid) => {
                info!("restarting to update");
                platform_window::quit_after_handing_focus_to(pid, cx);
            }
            Err(error) => self.update_failed(error, cx),
        }
    }

    fn update_failed(&mut self, text: impl Into<gpui_kit::SharedString>, cx: &mut Context<Self>) {
        self.update_restarting = false;
        self.show_toast(text, cx);
    }
}
