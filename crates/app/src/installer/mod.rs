//! Sonear's own setup and uninstall. The setup file is the Sonear program
//! itself: it copies itself into the install folder. See `launch` for how the
//! one binary decides what to be.

pub mod apply;
pub mod chrome;
mod job;
mod ops;
mod relocate;
mod wizard;

use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use gpui_kit::{App, AppContext as _, Entity, Render, SharedString, Window};
use rust_i18n::t;
use tracing::{error, warn};
use windows::Win32::UI::Shell::FOLDERID_LocalAppData;
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
use windows::core::HSTRING;

use crate::instance::{self, InstanceGuard};
use crate::{app_identity, assets, platform_window, theme};

pub use job::{Job, Reporter, watch};
use ops::Installed;

const DEMO: bool = cfg!(feature = "demo");
const CLOSE_WAIT: Duration = Duration::from_secs(10);
const CLOSE_POLL: Duration = Duration::from_millis(100);
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

pub fn close_and_retry() -> String {
    t!("installer.close_and_retry").into_owned()
}

/// Opens the one window of a flow and runs until it closes. `make` builds the
/// view; it is also the place to block closing the window.
pub fn run_window<V: Render + 'static>(
    title: impl Into<SharedString>,
    make: impl FnOnce(&mut Window, &mut App) -> Entity<V> + 'static,
) {
    let title = title.into();

    gpui_kit::application()
        .with_assets(assets::AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            theme::apply(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let options = chrome::window_options(title, cx);

            match gpui_kit::open_window(options, cx, make) {
                Ok((window, _)) => platform_window::focus(window, cx),
                Err(error) => {
                    error!(%error, "could not open the window");
                    cx.quit();
                }
            }
        });
}

pub fn run_setup() {
    let installed = installed();
    let source = match std::env::current_exe() {
        Ok(source) => source,
        Err(error) => {
            error!(%error, "could not find the setup file");

            return;
        }
    };

    run_window(
        t!("installer.title.setup").into_owned(),
        move |window, cx| {
            cx.new(|cx| {
                wizard::Wizard::new(wizard::Flow::Install { installed, source }, window, cx)
            })
        },
    );
}

/// `--uninstall`: starts a temp copy that does the work and returns at once,
/// so the caller can exit and leave the install folder free.
pub fn run_uninstall() {
    if DEMO {
        run_uninstall_from(PathBuf::from(r"C:\Demo\Sonear"));

        return;
    }

    let Some(installed) = installed() else {
        warn!("Sonear is not installed, nothing to uninstall");
        show_error(&t!("installer.not_installed"));

        return;
    };

    if let Err(error) = relocate::start_from_temp(&installed.dir) {
        error!(%error, "could not start the uninstaller");
        show_error(&error.user_message());
    }
}

/// For failures before any window exists, when nothing else would tell the user.
fn show_error(text: &str) {
    // SAFETY: both strings outlive the call, which blocks until the box closes.
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from(t!("installer.title.uninstall").into_owned()),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// `--uninstall-from <dir>`: the wizard, run from a temp copy.
pub fn run_uninstall_from(dir: PathBuf) {
    run_window(
        t!("installer.title.uninstall").into_owned(),
        move |window, cx| {
            cx.new(|cx| wizard::Wizard::new(wizard::Flow::Uninstall { dir }, window, cx))
        },
    );

    if !DEMO {
        relocate::keep_handle_alive_briefly();
    }
}

#[cfg(not(feature = "demo"))]
pub fn installed() -> Option<Installed> {
    ops::installed_at(ops::UNINSTALL_KEY)
}

/// A demo build never reads the real registry. `SONEAR_DEMO_INSTALLED` pretends
/// a version is installed, to show the update page.
#[cfg(feature = "demo")]
pub fn installed() -> Option<Installed> {
    let version = std::env::var("SONEAR_DEMO_INSTALLED").ok()?;

    Some(Installed {
        dir: PathBuf::from(r"C:\Demo\Sonear"),
        version,
    })
}

/// `%LOCALAPPDATA%\Programs\Sonear`.
pub fn default_install_dir() -> Option<PathBuf> {
    app_identity::known_folder(&FOLDERID_LocalAppData)
        .ok()
        .map(|folder| folder.join("Programs").join("Sonear"))
}

/// Whether a Sonear is open. A demo build never sees the real one;
/// `SONEAR_DEMO_RUNNING` shows the warning anyway.
pub fn sonear_is_open() -> bool {
    if DEMO {
        return std::env::var_os("SONEAR_DEMO_RUNNING").is_some();
    }

    instance::is_running(false)
}

/// Takes the single-instance lock, asking a running Sonear to quit first.
/// Holding the guard also stops Sonear from starting in the middle of the work.
pub fn take_over() -> Result<InstanceGuard, String> {
    if let Some(guard) = instance::acquire(false) {
        return Ok(guard);
    }

    instance::request_quit(false);

    let started = Instant::now();

    while started.elapsed() < CLOSE_WAIT {
        std::thread::sleep(CLOSE_POLL);

        if let Some(guard) = instance::acquire(false) {
            return Ok(guard);
        }
    }

    Err(close_and_retry())
}

/// Starts `exe` on its own, with `dir` as its working folder. Returns its process ID.
fn start_detached(exe: &Path, dir: &Path) -> Option<u32> {
    let started = Command::new(exe)
        .current_dir(dir)
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    match started {
        Ok(child) => Some(child.id()),
        Err(error) => {
            error!(%error, "could not start {}", exe.display());

            None
        }
    }
}

/// Starts `exe`, passes it the focus and quits. Any single-instance guard must
/// already be dropped, because the new process takes the lock straight away.
pub fn open_and_quit(exe: &Path, dir: &Path, cx: &mut App) {
    match start_detached(exe, dir) {
        Some(pid) => platform_window::quit_after_handing_focus_to(pid, cx),
        None => cx.quit(),
    }
}

/// Opens this program as the app and quits.
pub fn restart_app(cx: &mut App) {
    let Ok(exe) = std::env::current_exe() else {
        cx.quit();

        return;
    };
    let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();

    open_and_quit(&exe, &dir, cx);
}
