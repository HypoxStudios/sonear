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
    PathPromptOptions, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Task,
    Window, div, px,
};
use rust_i18n::t;
use sonear_core::AppPaths;
use sonear_core::data_dir::resolve_target;
use sonear_core::update::compare_versions;

use super::job::{Job, Reporter, Snapshot, watch};
use super::ops::{self, InstallOptions, Installed, Target};
use super::{
    chrome, close_and_retry, default_install_dir, open_and_quit, sonear_is_open, take_over,
};
use crate::instance::InstanceGuard;
use crate::theme;
use crate::ui::widgets::{accent_button, danger_button, ghost_button, icon};
use gpui_kit::assets::IconName;

const DEMO: bool = cfg!(feature = "demo");
/// A copy takes about a tenth of a second, which would flash past.
const MIN_WORK: Duration = Duration::from_millis(600);
const DEMO_STEP: Duration = Duration::from_millis(500);

const INSTALL_STEPS: [&str; 4] = [
    "installer.step.welcome",
    "installer.step.location",
    "installer.step.install",
    "installer.step.finish",
];
const UPDATE_STEPS: [&str; 3] = [
    "installer.step.welcome",
    "installer.step.install",
    "installer.step.finish",
];
const UNINSTALL_STEPS: [&str; 3] = [
    "installer.step.confirm",
    "installer.step.remove",
    "installer.step.finish",
];

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
    button: String,
}

fn welcome_text(installed: &str, new: &str) -> Welcome {
    let known = !installed.is_empty();

    match compare_versions(installed, new) {
        Ordering::Less => Welcome {
            title: t!("installer.update.title", new = new).into_owned(),
            detail: known
                .then(|| t!("installer.update.detail", installed = installed).into_owned()),
            button: t!("installer.update.button").into_owned(),
        },
        Ordering::Equal => Welcome {
            title: t!("installer.reinstall.title", new = new).into_owned(),
            detail: Some(t!("installer.reinstall.detail").into_owned()),
            button: t!("installer.reinstall.button").into_owned(),
        },
        Ordering::Greater => Welcome {
            title: t!("installer.replace.title", installed = installed, new = new).into_owned(),
            detail: Some(t!("installer.replace.detail").into_owned()),
            button: t!("installer.replace.button").into_owned(),
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
            let mut input = InputState::new(window, cx)
                .placeholder(t!("installer.folder_placeholder").into_owned());

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

    fn step_keys(&self) -> &'static [&'static str] {
        if self.is_uninstall() {
            &UNINSTALL_STEPS
        } else if self.installed().is_some() {
            &UPDATE_STEPS
        } else {
            &INSTALL_STEPS
        }
    }

    fn steps(&self) -> Vec<SharedString> {
        self.step_keys()
            .iter()
            .map(|key| SharedString::from(t!(*key).into_owned()))
            .collect()
    }

    fn step_index(&self) -> usize {
        let last = self.step_keys().len() - 1;
        let has_location = self.step_keys().len() == INSTALL_STEPS.len();

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
                        (0.3, t!("installer.progress.copying")),
                        (0.7, t!("installer.progress.shortcuts")),
                        (0.95, t!("installer.progress.registering")),
                        (1., t!("installer.progress.done")),
                    ],
                );

                return Ok(Done::default());
            }

            let guard = take_over()?;
            let target = Target::real(dir).map_err(|error| error.user_message())?;

            ops::install(&target, &source, options, &|fraction, label| {
                reporter.step(fraction, label);
            })
            .map_err(|error| error.user_message())?;

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
                        (0.3, t!("installer.progress.removing_program")),
                        (0.6, t!("installer.progress.removing_shortcuts")),
                        (0.9, t!("installer.progress.removing_apps")),
                        (1., t!("installer.progress.done")),
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
        label: String,
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
                    .accessibility_label(label.clone())
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
        self.running
            .then(|| chrome::message(t!("installer.running"), theme::warning()))
    }

    fn welcome_page(&self, cx: &mut Context<Self>) -> Div {
        let Some(installed) = self.installed() else {
            return chrome::page(
                t!("installer.welcome.title"),
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(chrome::text(t!("installer.welcome.body")))
                    .child(chrome::note(t!("installer.welcome.note"))),
                accent_button("welcome-next", t!("installer.welcome.next"), None, true)
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
                .child(chrome::note(t!("installer.data_untouched")))
                .children(self.warning()),
            accent_button("welcome-install", welcome.button, None, true)
                .on_click(cx.listener(|wizard, _, _, cx| wizard.start_install(cx))),
        )
    }

    fn location_page(&self, cx: &mut Context<Self>) -> Div {
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.dir_input)))
                    .child(
                        ghost_button("location-browse", t!("installer.location.browse"), true)
                            .on_click(
                                cx.listener(|wizard, _, window, cx| wizard.browse(window, cx)),
                            ),
                    ),
            )
            .children(
                self.location_error
                    .clone()
                    .map(|error| chrome::message(error, theme::danger())),
            )
            .child(self.switch_row(
                "location-desktop",
                t!("installer.location.desktop").into_owned(),
                self.desktop_shortcut,
                |wizard, on| wizard.desktop_shortcut = on,
                cx,
            ))
            .child(chrome::note(t!("installer.location.start_menu")))
            .children(self.warning());

        chrome::page(
            t!("installer.location.title"),
            body,
            div()
                .flex()
                .gap_2()
                .child(
                    ghost_button("location-back", t!("installer.back"), true)
                        .on_click(cx.listener(|wizard, _, _, cx| wizard.go(Page::Welcome, cx))),
                )
                .child(
                    accent_button(
                        "location-install",
                        t!("installer.location.install"),
                        None,
                        true,
                    )
                    .on_click(cx.listener(|wizard, _, _, cx| wizard.install_from_location(cx))),
                ),
        )
    }

    fn confirm_page(&self, cx: &mut Context<Self>) -> Div {
        let body = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(chrome::text(t!("installer.confirm.body")))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(self.switch_row(
                        "confirm-delete-data",
                        t!("installer.confirm.delete_data").into_owned(),
                        self.delete_data,
                        |wizard, on| wizard.delete_data = on,
                        cx,
                    ))
                    .child(chrome::note(t!("installer.confirm.delete_data_note"))),
            )
            .children(self.warning());

        chrome::page(
            t!("installer.confirm.title"),
            body,
            div()
                .flex()
                .gap_2()
                .child(
                    ghost_button("confirm-cancel", t!("common.cancel"), true)
                        .on_click(cx.listener(|_, _, _, cx| cx.quit())),
                )
                .child(
                    danger_button("confirm-uninstall", t!("installer.confirm.uninstall"), true)
                        .on_click(cx.listener(|wizard, _, _, cx| wizard.start_uninstall(cx))),
                ),
        )
    }

    fn working_page(&self) -> Div {
        let title = if self.is_uninstall() {
            t!("installer.working.remove")
        } else if self.installed().is_some() {
            t!("installer.working.update")
        } else {
            t!("installer.working.install")
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
                t!("installer.finish.uninstalled"),
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(
                        (!self.problems.is_empty())
                            .then(|| chrome::text(t!("installer.finish.problems"))),
                    )
                    .children(problems),
                accent_button("finish-close", t!("common.close"), None, true)
                    .on_click(cx.listener(|wizard, _, _, cx| wizard.finish(cx))),
            );
        }

        chrome::done_page(
            t!("installer.finish.installed"),
            self.switch_row(
                "finish-open",
                t!("installer.finish.open_now").into_owned(),
                self.open_now,
                |wizard, on| wizard.open_now = on,
                cx,
            ),
            accent_button("finish-done", t!("installer.finish.finish"), None, true)
                .on_click(cx.listener(|wizard, _, _, cx| wizard.finish(cx))),
        )
    }

    fn failed_page(&self, error: &str, cx: &mut Context<Self>) -> Div {
        let title = if self.is_uninstall() {
            t!("installer.failed.uninstall")
        } else {
            t!("installer.failed.install")
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
                    (error == close_and_retry())
                        .then(|| chrome::note(t!("installer.failed.quit_hint"))),
                ),
            div()
                .flex()
                .gap_2()
                .child(
                    ghost_button("failed-close", t!("common.close"), true)
                        .on_click(cx.listener(|_, _, _, cx| cx.quit())),
                )
                .child(
                    accent_button("failed-retry", t!("common.try_again"), None, true)
                        .on_click(cx.listener(|wizard, _, _, cx| wizard.retry(cx))),
                ),
        )
    }
}

/// Pretends to work for the demo build, which touches nothing.
fn simulate(reporter: &Reporter, steps: &[(f32, std::borrow::Cow<'static, str>)]) {
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
            t!("installer.title.uninstall")
        } else {
            t!("installer.title.setup")
        };

        chrome::frame(title.into_owned(), &self.steps(), self.step_index(), page)
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
                button: "Update".to_owned(),
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
