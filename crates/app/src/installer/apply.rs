//! `--apply-update <folder>`: the downloaded update swaps itself into the install
//! folder while no Sonear runs, then opens the installed one. It runs from the
//! updates folder, so the installed program is free to be replaced.

use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::{
    AppContext as _, Context, IntoElement, ParentElement as _, Render,
    StatefulInteractiveElement as _, Styled as _, Task, Window, div,
};
use sonear_core::data_dir::same_place;
use tracing::{error, info};

use super::ops::{self, InstallOptions, Target};
use super::{
    CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS, Job, Reporter, chrome, open_and_quit, restart_app,
    take_over, watch,
};
use crate::instance::InstanceGuard;
use crate::launch;
use crate::theme;
use crate::ui::widgets::accent_button;

const DEMO: bool = cfg!(feature = "demo");
const TITLE: &str = "Updating Sonear";
const MIN_WORK: Duration = Duration::from_millis(1200);
const DEMO_STEP: Duration = Duration::from_millis(700);
const DEMO_STEPS: [(f32, &str); 5] = [
    (0.1, "Preparing"),
    (0.4, "Copying Sonear"),
    (0.7, "Replacing the program"),
    (0.9, "Adding shortcuts"),
    (1., "Done"),
];

/// Starts the downloaded update `file` to replace the Sonear in `install_dir`,
/// once this process has exited. Returns its process ID.
pub fn launch(file: &Path, install_dir: &Path) -> Result<u32, String> {
    let started = Command::new(file)
        .arg("--after")
        .arg(std::process::id().to_string())
        .arg("--apply-update")
        .arg(install_dir)
        .current_dir(file.parent().unwrap_or(install_dir))
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    started
        .map(|child| child.id())
        .map_err(|error| format!("Could not start the update: {error}"))
}

/// What a finished update hands back.
struct Applied {
    guard: Option<InstanceGuard>,
    exe: PathBuf,
}

enum Stage {
    Working,
    Failed(String),
}

struct Applier {
    stage: Stage,
    job: Option<Job<Result<Applied, String>>>,
    fraction: f32,
    label: String,
    dir: PathBuf,
    guard: Option<InstanceGuard>,
    busy: Arc<AtomicBool>,
    _poll: Option<Task<()>>,
}

fn run_apply(dir: &Path, reporter: &Reporter) -> Result<Applied, String> {
    let guard = take_over()?;
    let installed_here =
        super::installed().is_some_and(|installed| same_place(&installed.dir, dir));

    if !installed_here {
        return Err("Sonear is not installed in that folder.".to_owned());
    }

    let source =
        std::env::current_exe().map_err(|error| format!("Could not find the update: {error}"))?;
    let target = Target::real(dir.to_owned()).map_err(|error| error.to_string())?;
    let options = InstallOptions {
        desktop_shortcut: false,
        is_update: true,
    };

    ops::install(&target, &source, options, &|fraction, label| {
        reporter.step(fraction, label);
    })
    .map_err(|error| error.to_string())?;

    Ok(Applied {
        guard: Some(guard),
        exe: target.exe(),
    })
}

fn simulate_apply(reporter: &Reporter) {
    for (fraction, label) in DEMO_STEPS {
        reporter.step(fraction, label);
        std::thread::sleep(DEMO_STEP);
    }
}

impl Applier {
    fn new(
        dir: PathBuf,
        after_pid: Option<u32>,
        busy: Arc<AtomicBool>,
        cx: &mut Context<Self>,
    ) -> Self {
        let target = dir.clone();
        let job = Job::spawn(MIN_WORK, move |reporter| {
            if let Some(pid) = after_pid {
                launch::wait_for_exit(pid);
            }

            if DEMO {
                simulate_apply(reporter);

                return Ok(Applied {
                    guard: None,
                    exe: PathBuf::new(),
                });
            }

            run_apply(&target, reporter)
        });

        Self {
            stage: Stage::Working,
            job: Some(job),
            fraction: 0.,
            label: "Waiting for Sonear to close".to_owned(),
            dir,
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

        if !snapshot.label.is_empty() {
            self.label = snapshot.label;
        }

        self.fraction = snapshot.fraction;

        let Some(result) = job.take() else {
            cx.notify();

            return false;
        };

        self.job = None;
        self.busy.store(false, Ordering::Relaxed);

        match result {
            Ok(applied) => {
                info!("updated Sonear");
                self.guard = applied.guard;
                self.open_sonear(&applied.exe, cx);
            }
            Err(error) => {
                error!(%error, "could not apply the update");
                self.stage = Stage::Failed(error);
            }
        }

        cx.notify();

        true
    }

    fn open_sonear(&mut self, exe: &Path, cx: &mut Context<Self>) {
        self.guard = None;

        if DEMO {
            restart_app(cx);
        } else {
            open_and_quit(exe, &self.dir, cx);
        }
    }

    fn back_to_sonear(&mut self, cx: &mut Context<Self>) {
        let exe = self.dir.join(ops::EXE_NAME);

        self.open_sonear(&exe, cx);
    }
}

impl Render for Applier {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = match &self.stage {
            Stage::Working => chrome::page(
                TITLE,
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(chrome::progress_bar(self.fraction))
                    .child(chrome::note(self.label.clone()))
                    .child(chrome::note("Sonear opens again when it is done.")),
                div(),
            ),
            Stage::Failed(error) => chrome::page(
                "Sonear could not be updated",
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(chrome::message(error.clone(), theme::danger()))
                    .child(chrome::text("Your current version still works.")),
                accent_button("apply-back", "Back to Sonear", None, true)
                    .on_click(cx.listener(|applier, _, _, cx| applier.back_to_sonear(cx))),
            ),
        };

        chrome::frame(TITLE, &[], 0, page)
    }
}

/// Runs the update window. `after_pid` is the Sonear that started it: its window
/// stays up until this one has the focus, so the wait for it to exit happens
/// behind the window.
pub fn run(dir: PathBuf, after_pid: Option<u32>) {
    super::run_window(TITLE, move |window, cx| {
        let busy = Arc::new(AtomicBool::new(true));
        let closing = busy.clone();

        window.on_window_should_close(cx, move |_, _| !closing.load(Ordering::Relaxed));

        cx.new(|cx| Applier::new(dir, after_pid, busy, cx))
    });
}
