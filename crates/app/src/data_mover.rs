//! `--move-data <folder>`: moves Sonear's data folder while no Sonear runs,
//! and the window shown when the configured data folder is missing.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::{
    AppContext as _, Context, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Task, Window, div,
};
use sonear_core::data_dir::{self, Moved};
use sonear_core::{AppPaths, Config};
use tracing::warn;

use crate::format::megabytes;
use crate::installer::{self, Job, Reporter, chrome, watch};
use crate::instance::InstanceGuard;
use crate::launch;
use crate::theme;
use crate::ui::widgets::{accent_button, ghost_button};

const DEMO: bool = cfg!(feature = "demo");
const MIN_WORK: Duration = Duration::from_millis(600);
const DEMO_STEP: Duration = Duration::from_millis(1000);
const DEMO_TOTAL: u64 = 240 * 1024 * 1024;

/// What a finished move hands back.
struct MoveDone {
    guard: Option<InstanceGuard>,
    /// Set when the old folder could not be removed.
    leftover: Option<String>,
}

enum Stage {
    Moving,
    Moved(String),
    Failed(String),
}

struct DataMover {
    stage: Stage,
    job: Option<Job<Result<MoveDone, String>>>,
    done: u64,
    total: u64,
    fraction: f32,
    guard: Option<InstanceGuard>,
    busy: Arc<AtomicBool>,
    _poll: Option<Task<()>>,
}

/// Saves where the data folder is. `None` means the default one.
fn save_data_dir(config_file: &Path, dir: Option<PathBuf>) -> Result<(), String> {
    let mut config = Config::load(config_file);

    config.data_dir = dir;
    config.save(config_file).map_err(|error| error.to_string())
}

fn is_default(target: &Path) -> bool {
    AppPaths::default_local_root().is_some_and(|default| data_dir::same_path(&default, target))
}

fn run_move(target: &Path, reporter: &Reporter) -> Result<MoveDone, String> {
    let guard = installer::take_over()?;
    let paths = AppPaths::discover().ok_or("Could not find your data folder.")?;
    let from = paths.local_root();
    let config_file = paths.config_file();
    let moved = data_dir::move_data(&from, target, &|done, total| reporter.bytes(done, total))
        .map_err(|error| error.to_string())?;
    let saved = save_data_dir(
        &config_file,
        (!is_default(target)).then(|| target.to_owned()),
    );

    if let Err(error) = saved {
        if let Err(undo) = data_dir::undo_move(&from, target, moved) {
            warn!(%undo, "could not undo the move");
        }

        return Err(format!("Could not save the new location: {error}"));
    }

    reporter.step(1., "Done");

    let leftover = match moved {
        Moved::Copied => data_dir::finish_move(&from)
            .err()
            .map(|error| format!("{}: {error}", from.display())),
        Moved::Renamed => None,
    };

    Ok(MoveDone {
        guard: Some(guard),
        leftover,
    })
}

fn simulate_move(reporter: &Reporter) {
    for step in 1..=5u64 {
        reporter.bytes(DEMO_TOTAL * step / 5, DEMO_TOTAL);
        std::thread::sleep(DEMO_STEP);
    }
}

impl DataMover {
    fn new(
        target: PathBuf,
        after_pid: Option<u32>,
        busy: Arc<AtomicBool>,
        cx: &mut Context<Self>,
    ) -> Self {
        let job = Job::spawn(MIN_WORK, move |reporter| {
            if let Some(pid) = after_pid {
                launch::wait_for_exit(pid);
            }

            if DEMO {
                simulate_move(reporter);

                return Ok(MoveDone {
                    guard: None,
                    leftover: None,
                });
            }

            run_move(&target, reporter)
        });

        Self {
            stage: Stage::Moving,
            job: Some(job),
            done: 0,
            total: 0,
            fraction: 0.,
            guard: None,
            busy,
            _poll: Some(watch(cx, Self::poll)),
        }
    }

    fn poll(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(job) = &self.job else {
            return true;
        };
        let snapshot = job.snapshot();

        self.fraction = snapshot.fraction;
        (self.done, self.total) = snapshot.bytes.unwrap_or_default();

        let Some(result) = job.take() else {
            cx.notify();

            return false;
        };

        self.job = None;
        self.busy.store(false, Ordering::Relaxed);

        match result {
            Ok(done) => {
                self.guard = done.guard;

                match done.leftover {
                    Some(leftover) => self.stage = Stage::Moved(leftover),
                    None => self.back_to_sonear(cx),
                }
            }
            Err(error) => self.stage = Stage::Failed(error),
        }

        cx.notify();

        true
    }

    fn back_to_sonear(&mut self, cx: &mut Context<Self>) {
        self.guard = None;
        installer::restart_app(cx);
    }
}

impl Render for DataMover {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let back = || {
            accent_button("mover-back", "Back to Sonear", None, true)
                .on_click(cx.listener(|mover, _, _, cx| mover.back_to_sonear(cx)))
        };
        let page = match &self.stage {
            Stage::Moving => {
                let counted = (self.total > 0)
                    .then(|| format!("{} of {}", megabytes(self.done), megabytes(self.total)));

                chrome::page(
                    "Moving your data",
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(chrome::progress_bar(self.fraction))
                        .child(chrome::note(counted.unwrap_or_default()))
                        .child(chrome::note("Sonear opens again when it is done.")),
                    div(),
                )
            }
            Stage::Moved(leftover) => chrome::done_page(
                "Your data is moved",
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(chrome::text("The old folder could not be removed:"))
                    .child(chrome::message(leftover.clone(), theme::warning())),
                back(),
            ),
            Stage::Failed(error) => chrome::page(
                "Could not move your data",
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(chrome::text("Your data was not moved."))
                    .child(chrome::message(error.clone(), theme::danger())),
                back(),
            ),
        };

        chrome::frame("Moving your data", &[], 0, page)
    }
}

/// Runs the mover window for `--move-data`. `after_pid` is the Sonear that started
/// it: its window stays up until this one has the focus, so the wait for it to
/// exit happens behind the window.
pub fn run(target: PathBuf, after_pid: Option<u32>) {
    installer::run_window("Moving your data", move |window, cx| {
        let busy = Arc::new(AtomicBool::new(true));
        let closing = busy.clone();

        window.on_window_should_close(cx, move |_, _| !closing.load(Ordering::Relaxed));

        cx.new(|cx| DataMover::new(target, after_pid, busy, cx))
    });
}

struct MissingFolder {
    path: PathBuf,
    error: Option<String>,
    guard: Option<InstanceGuard>,
}

impl MissingFolder {
    fn retry(&mut self, cx: &mut Context<Self>) {
        self.guard = None;

        if DEMO {
            cx.quit();
        } else {
            installer::restart_app(cx);
        }
    }

    fn use_default(&mut self, cx: &mut Context<Self>) {
        if !DEMO {
            let Some(paths) = AppPaths::discover() else {
                self.error = Some("Could not find your settings.".to_owned());
                cx.notify();

                return;
            };

            if let Err(error) = save_data_dir(&paths.config_file(), None) {
                self.error = Some(format!("Could not save the setting: {error}"));
                cx.notify();

                return;
            }
        }

        self.retry(cx);
    }
}

impl Render for MissingFolder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(chrome::text(
                "Sonear keeps its history, queue and Soloist in this folder, but it is not there. If it is on a drive that is not connected, connect it and try again.",
            ))
            .child(chrome::message(
                self.path.display().to_string(),
                theme::text_primary(),
            ))
            .children(
                self.error
                    .clone()
                    .map(|error| chrome::message(error, theme::danger())),
            );
        let footer = div()
            .flex()
            .gap_2()
            .child(
                ghost_button("missing-quit", "Quit", true)
                    .on_click(cx.listener(|_, _, _, cx| cx.quit())),
            )
            .child(
                ghost_button("missing-default", "Use the default folder", true)
                    .on_click(cx.listener(|missing, _, _, cx| missing.use_default(cx))),
            )
            .child(
                accent_button("missing-retry", "Try again", None, true)
                    .on_click(cx.listener(|missing, _, _, cx| missing.retry(cx))),
            );

        chrome::frame(
            "Sonear",
            &[],
            0,
            chrome::page("Sonear can't find its data folder", body, footer),
        )
    }
}

/// Shows the missing-folder window. Holds `guard` so no second Sonear starts meanwhile.
pub fn run_missing(path: PathBuf, guard: Option<InstanceGuard>) {
    installer::run_window("Sonear", move |_, cx| {
        cx.new(|_| MissingFolder {
            path,
            error: None,
            guard,
        })
    });
}
