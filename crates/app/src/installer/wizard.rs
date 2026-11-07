//! The setup and uninstall wizard.

use std::cmp::Ordering;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::time::Duration;

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::{
    AnyElement, AppContext as _, Context, Div, Entity, IntoElement, ParentElement as _,
    PathPromptOptions, Render, StatefulInteractiveElement as _, Styled as _, Task, Window, div, px,
};
use sonear_core::AppPaths;
use sonear_core::data_dir::resolve_target;
use sonear_core::update::compare_versions;

use super::job::{Job, Reporter, Snapshot, watch};
use super::ops::{self, InstallOptions, Installed, Target};
use super::{
    CLOSE_AND_RETRY, chrome, default_install_dir, open_and_quit, sonear_is_open, take_over,
};
use crate::instance::InstanceGuard;
use crate::theme;
use crate::ui::widgets::{accent_button, danger_button, ghost_button, icon};
use gpui_kit::assets::IconName;

const DEMO: bool = cfg!(feature = "demo");
/// A copy takes about a tenth of a second, which would flash past.
const MIN_WORK: Duration = Duration::from_millis(600);
const DEMO_STEP: Duration = Duration::from_millis(500);

const INSTALL_STEPS: [&str; 4] = ["Welcome", "Location", "Install", "Finish"];
const UPDATE_STEPS: [&str; 3] = ["Welcome", "Install", "Finish"];
const UNINSTALL_STEPS: [&str; 3] = ["Confirm", "Remove", "Finish"];

pub enum Flow {
    Install {
        installed: Option<Installed>,
        source: PathBuf,
    },
    Uninstall {
        dir: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Page {
    Welcome,
    Location,
    Confirm,
    Working,
    Finish,
    Failed(String),
}

/// What a finished job hands back. The guard keeps Sonear from starting
/// until the wizard is done.
#[derive(Default)]
struct Done {
    guard: Option<InstanceGuard>,
    problems: Vec<String>,
}

/// The first page of an install that already exists.
#[derive(Debug, PartialEq, Eq)]
struct Welcome {
    title: String,
    detail: Option<String>,
    button: &'static str,
}

fn welcome_text(installed: &str, new: &str) -> Welcome {
    let known = !installed.is_empty();

    match compare_versions(installed, new) {
        Ordering::Less => Welcome {
            title: format!("Update Sonear to {new}"),
            detail: known.then(|| format!("You have {installed}.")),
            button: "Update",
        },
        Ordering::Equal => Welcome {
            title: format!("Reinstall Sonear {new}"),
            detail: Some("This version is already installed.".to_owned()),
            button: "Reinstall",
        },
        Ordering::Greater => Welcome {
            title: format!("Replace Sonear {installed} with {new}"),
            detail: Some("The installed version is newer.".to_owned()),
            button: "Replace",
        },
    }
}

pub struct Wizard {
    flow: Flow,
    page: Page,
    dir_input: Entity<InputState>,
    desktop_shortcut: bool,
    open_now: bool,
    delete_data: bool,
    location_error: Option<String>,
    running: bool,
    job: Option<Job<Result<Done, String>>>,
    progress: Snapshot,
    guard: Option<InstanceGuard>,
    problems: Vec<String>,
    install_dir: PathBuf,
    data_root: PathBuf,
    config_dir: PathBuf,
    busy: Arc<AtomicBool>,
    _poll: Option<Task<()>>,
}

impl Wizard {
    pub fn new(flow: Flow, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let paths = AppPaths::discover();
        let shown = match &flow {
            Flow::Install {
                installed: Some(installed),
                ..
            } => installed.dir.clone(),
            Flow::Install { .. } => default_install_dir().unwrap_or_default(),
            Flow::Uninstall { dir } => dir.clone(),
        };
        let page = match &flow {
            Flow::Install { .. } => Page::Welcome,
            Flow::Uninstall { .. } => Page::Confirm,
        };
        let dir_input = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder("Install folder");

            input.set_value(shown.display().to_string(), window, cx);

            input
        });
        let busy = Arc::new(AtomicBool::new(false));
        let closing = busy.clone();

        window.on_window_should_close(cx, move |_, _| !closing.load(AtomicOrdering::Relaxed));

        Self {
            flow,
            page,
            dir_input,
            desktop_shortcut: false,
            open_now: true,
            delete_data: false,
            location_error: None,
            running: sonear_is_open(),
            job: None,
            progress: Snapshot::default(),
            guard: None,
            problems: Vec::new(),
            install_dir: shown,
            data_root: paths.as_ref().map(AppPaths::local_root).unwrap_or_default(),
            config_dir: paths
                .as_ref()
                .map(|paths| paths.config_dir().to_owned())
                .unwrap_or_default(),
            busy,
            _poll: None,
        }
    }

    fn installed(&self) -> Option<&Installed> {
        match &self.flow {
            Flow::Install { installed, .. } => installed.as_ref(),
            Flow::Uninstall { .. } => None,
        }
    }

    fn is_uninstall(&self) -> bool {
        matches!(self.flow, Flow::Uninstall { .. })
    }

    fn steps(&self) -> &'static [&'static str] {
        if self.is_uninstall() {
            &UNINSTALL_STEPS
        } else if self.installed().is_some() {
            &UPDATE_STEPS
        } else {
            &INSTALL_STEPS
        }
    }

    fn step_index(&self) -> usize {
        let last = self.steps().len() - 1;
        let has_location = self.steps().len() == INSTALL_STEPS.len();

        match &self.page {
            Page::Welcome | Page::Confirm => 0,
            Page::Location => 1,
            Page::Working | Page::Failed(_) if has_location => 2,
            Page::Working | Page::Failed(_) => 1,
            Page::Finish => last + 1,
        }
    }

    fn go(&mut self, page: Page, cx: &mut Context<Self>) {
        self.running = sonear_is_open();
        self.page = page;
        cx.notify();
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });

        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(folder) = paths.into_iter().next() else {
                return;
            };
            let target = resolve_target(&folder);

            let _ = this.update_in(cx, |wizard, window, cx| {
                wizard.dir_input.update(cx, |input, cx| {
                    input.set_value(target.display().to_string(), window, cx);
                });
                wizard.location_error = None;
                cx.notify();
            });
        })
        .detach();
    }

    fn typed_dir(&self, cx: &Context<Self>) -> PathBuf {
        PathBuf::from(self.dir_input.read(cx).value().trim())
    }

    /// Checks the typed folder, which touches the disk, and starts installing.
    fn install_from_location(&mut self, cx: &mut Context<Self>) {
        let dir = self.typed_dir(cx);

        if !DEMO
            && let Err(message) = ops::check_install_dir(&dir, &self.data_root, &self.config_dir)
        {
            self.location_error = Some(message);
            cx.notify();

            return;
        }

        self.location_error = None;
        self.install_dir = dir;
        self.start_install(cx);
    }

    fn start_install(&mut self, cx: &mut Context<Self>) {
        let Flow::Install { installed, source } = &self.flow else {
            return;
        };
        let source = source.clone();
        let dir = self.install_dir.clone();
        let options = InstallOptions {
            desktop_shortcut: self.desktop_shortcut,
            is_update: installed.is_some(),
        };

        self.begin(cx, move |reporter| {
            if DEMO {
                simulate(
                    reporter,
                    &[
                        (0.3, "Copying Sonear"),
                        (0.7, "Adding shortcuts"),
                        (0.95, "Registering with Windows"),
                        (1., "Done"),
                    ],
                );

                return Ok(Done::default());
            }

            let guard = take_over()?;
            let target = Target::real(dir).map_err(|error| error.to_string())?;

            ops::install(&target, &source, options, &|fraction, label| {
                reporter.step(fraction, label);
            })
            .map_err(|error| error.to_string())?;

            Ok(Done {
                guard: Some(guard),
                problems: Vec::new(),
            })
        });
    }

    fn start_uninstall(&mut self, cx: &mut Context<Self>) {
        let Flow::Uninstall { dir } = &self.flow else {
            return;
        };
        let dir = dir.clone();
        let delete_data = self.delete_data;

        self.begin(cx, move |reporter| {
            if DEMO {
                simulate(
                    reporter,
                    &[
                        (0.3, "Removing the program"),
                        (0.6, "Removing shortcuts"),
                        (0.9, "Removing it from installed apps"),
                        (1., "Done"),
                    ],
                );

                return Ok(Done::default());
            }

            let guard = take_over()?;
            let problems = ops::uninstall_real(dir, delete_data, &|fraction, label| {
                reporter.step(fraction, label);
            });

            Ok(Done {
                guard: Some(guard),
                problems,
            })
        });
    }

    fn begin(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(&Reporter) -> Result<Done, String> + Send + 'static,
    ) {
        self.busy.store(true, AtomicOrdering::Relaxed);
        self.progress = Snapshot::default();
        self.job = Some(Job::spawn(MIN_WORK, work));
        self.page = Page::Working;
        self._poll = Some(watch(cx, Self::poll));
        cx.notify();
    }

    /// Returns whether the job is over, which ends the polling.
    fn poll(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(job) = &self.job else {
            return true;
        };

        self.progress = job.snapshot();

        let Some(result) = job.take() else {
            cx.notify();

            return false;
        };

        self.job = None;
        self.busy.store(false, AtomicOrdering::Relaxed);

        match result {
            Ok(done) => {
                self.guard = done.guard;
                self.problems = done.problems;
                self.page = Page::Finish;
            }
            Err(message) => self.page = Page::Failed(message),
        }

        cx.notify();

        true
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        self.guard = None;

        if self.open_now && !self.is_uninstall() && !DEMO {
            open_and_quit(&self.install_dir.join(ops::EXE_NAME), &self.install_dir, cx);

            return;
        }

        cx.quit();
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        let page = if self.is_uninstall() {
            Page::Confirm
        } else if self.installed().is_some() {
            Page::Welcome
        } else {
            Page::Location
        };

        self.go(page, cx);
    }

    fn switch_row(
        &self,
        id: &'static str,
        label: &'static str,
        checked: bool,
        change: impl Fn(&mut Wizard, bool) + 'static,
        cx: &mut Context<Self>,
    ) -> Div {
        let wizard = cx.entity();

        div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                Switch::new(id)
                    .checked(checked)
                    .color(theme::accent())
                    .accessibility_label(label)
                    .on_change(move |on, _, cx| {
                        wizard.update(cx, |wizard, cx| {
                            change(wizard, *on);
                            cx.notify();
                        });
                    }),
            )
            .child(
                div()
                    .text_size(px(14.))
                    .text_color(theme::text_primary())
                    .child(label),
            )
    }

    fn warning(&self) -> Option<AnyElement> {
        self.running.then(|| {
            chrome::message(
                "Sonear is open. It will close when you continue.",
                theme::warning(),
            )
        })
    }

    fn welcome_page(&self, cx: &mut Context<Self>) -> Div {
        let Some(installed) = self.installed() else {
            return chrome::page(
                "Welcome to Sonear",
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(chrome::text(
                        "A desktop player for Spotify. It needs Spotify Premium.",
                    ))
                    .child(chrome::note(
                        "Not affiliated with Spotify. Open source under Apache-2.0.",
                    )),
                accent_button("welcome-next", "Next", None, true)
                    .on_click(cx.listener(|wizard, _, _, cx| wizard.go(Page::Location, cx))),
            );
        };
        let welcome = welcome_text(&installed.version, env!("CARGO_PKG_VERSION"));

        chrome::page(
            welcome.title,
            div()
                .flex()
                .flex_col()
                .gap_3()
                .children(welcome.detail.map(chrome::text))
                .child(chrome::note("Your data is not touched."))
                .children(self.warning()),
            accent_button("welcome-install", welcome.button, None, true)
                .on_click(cx.listener(|wizard, _, _, cx| wizard.start_install(cx))),
        )
    }

    fn location_page(&self, cx: &mut Context<Self>) -> Div {
        let body =
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().flex_1().child(Input::new(&self.dir_input)))
                        .child(ghost_button("location-browse", "Browse", true).on_click(
                            cx.listener(|wizard, _, window, cx| wizard.browse(window, cx)),
                        )),
                )
                .children(
                    self.location_error
                        .clone()
                        .map(|error| chrome::message(error, theme::danger())),
                )
                .child(self.switch_row(
                    "location-desktop",
                    "Add a shortcut to the desktop",
                    self.desktop_shortcut,
                    |wizard, on| wizard.desktop_shortcut = on,
                    cx,
                ))
                .child(chrome::note("Sonear is also added to the Start menu."))
                .children(self.warning());

        chrome::page(
            "Where should Sonear go?",
            body,
            div()
                .flex()
                .gap_2()
                .child(
                    ghost_button("location-back", "Back", true)
                        .on_click(cx.listener(|wizard, _, _, cx| wizard.go(Page::Welcome, cx))),
                )
                .child(
                    accent_button("location-install", "Install", None, true)
                        .on_click(cx.listener(|wizard, _, _, cx| wizard.install_from_location(cx))),
                ),
        )
    }

    fn confirm_page(&self, cx: &mut Context<Self>) -> Div {
        let body = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(chrome::text(
                "This removes the program and its shortcuts from this computer.",
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(self.switch_row(
                        "confirm-delete-data",
                        "Also delete my data",
                        self.delete_data,
                        |wizard, on| wizard.delete_data = on,
                        cx,
                    ))
                    .child(chrome::note(
                        "History, queue, cache, settings, your login and Soloist.",
                    )),
            )
            .children(self.warning());

        chrome::page(
            "Uninstall Sonear?",
            body,
            div()
                .flex()
                .gap_2()
                .child(
                    ghost_button("confirm-cancel", "Cancel", true)
                        .on_click(cx.listener(|_, _, _, cx| cx.quit())),
                )
                .child(
                    danger_button("confirm-uninstall", "Uninstall", true)
                        .on_click(cx.listener(|wizard, _, _, cx| wizard.start_uninstall(cx))),
                ),
        )
    }

    fn working_page(&self) -> Div {
        let title = if self.is_uninstall() {
            "Removing Sonear"
        } else if self.installed().is_some() {
            "Updating Sonear"
        } else {
            "Installing Sonear"
        };

        chrome::page(
            title,
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(chrome::progress_bar(self.progress.fraction))
                .child(chrome::note(self.progress.label.clone())),
            div(),
        )
    }

    fn finish_page(&self, cx: &mut Context<Self>) -> Div {
        if self.is_uninstall() {
            let problems = self
                .problems
                .iter()
                .map(|problem| chrome::message(problem.clone(), theme::warning()));

            return chrome::done_page(
                "Sonear is uninstalled",
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(
                        (!self.problems.is_empty())
                            .then(|| chrome::text("Some things could not be removed:")),
                    )
                    .children(problems),
                accent_button("finish-close", "Close", None, true)
                    .on_click(cx.listener(|wizard, _, _, cx| wizard.finish(cx))),
            );
        }

        chrome::done_page(
            "Sonear is installed",
            self.switch_row(
                "finish-open",
                "Open Sonear now",
                self.open_now,
                |wizard, on| wizard.open_now = on,
                cx,
            ),
            accent_button("finish-done", "Finish", None, true)
                .on_click(cx.listener(|wizard, _, _, cx| wizard.finish(cx))),
        )
    }

    fn failed_page(&self, error: &str, cx: &mut Context<Self>) -> Div {
        let title = if self.is_uninstall() {
            "Sonear could not be uninstalled"
        } else {
            "Sonear could not be installed"
        };

        chrome::page(
            title,
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(icon(IconName::TriangleAlert, 16., theme::danger()))
                        .child(chrome::message(error.to_owned(), theme::danger())),
                )
                .children(
                    (error == CLOSE_AND_RETRY)
                        .then(|| chrome::note("Quit it from the tray icon, then try again.")),
                ),
            div()
                .flex()
                .gap_2()
                .child(
                    ghost_button("failed-close", "Close", true)
                        .on_click(cx.listener(|_, _, _, cx| cx.quit())),
                )
                .child(
                    accent_button("failed-retry", "Try again", None, true)
                        .on_click(cx.listener(|wizard, _, _, cx| wizard.retry(cx))),
                ),
        )
    }
}

/// Pretends to work for the demo build, which touches nothing.
fn simulate(reporter: &Reporter, steps: &[(f32, &str)]) {
    for (fraction, label) in steps {
        reporter.step(*fraction, label);
        std::thread::sleep(DEMO_STEP);
    }
}

impl Render for Wizard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = match self.page.clone() {
            Page::Welcome => self.welcome_page(cx),
            Page::Location => self.location_page(cx),
            Page::Confirm => self.confirm_page(cx),
            Page::Working => self.working_page(),
            Page::Finish => self.finish_page(cx),
            Page::Failed(error) => self.failed_page(&error, cx),
        };
        let title = if self.is_uninstall() {
            "Uninstall Sonear"
        } else {
            "Sonear Setup"
        };

        chrome::frame(title, self.steps(), self.step_index(), page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_older_install_is_updated() {
        assert_eq!(
            welcome_text("0.1.0", "0.2.0"),
            Welcome {
                title: "Update Sonear to 0.2.0".to_owned(),
                detail: Some("You have 0.1.0.".to_owned()),
                button: "Update",
            }
        );
    }

    #[test]
    fn the_same_version_is_reinstalled() {
        let welcome = welcome_text("0.1.0", "0.1.0");

        assert_eq!(welcome.title, "Reinstall Sonear 0.1.0");
        assert_eq!(welcome.button, "Reinstall");
    }

    #[test]
    fn a_newer_install_is_replaced() {
        let welcome = welcome_text("0.3.0", "0.2.0");

        assert_eq!(welcome.title, "Replace Sonear 0.3.0 with 0.2.0");
        assert_eq!(welcome.button, "Replace");
    }

    #[test]
    fn versions_compare_by_number() {
        assert_eq!(welcome_text("0.9.0", "0.10.0").button, "Update");
        assert_eq!(welcome_text("0.10.0", "0.9.0").button, "Replace");
    }

    #[test]
    fn an_unknown_installed_version_is_not_quoted() {
        assert_eq!(welcome_text("", "0.2.0").detail, None);
    }
}
