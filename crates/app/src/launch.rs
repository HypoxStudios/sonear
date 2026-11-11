//! What this process was started to do. The setup file is the Sonear program
//! itself, so the same binary installs, uninstalls, moves data and runs the app.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sonear_core::SpotifyLink;
use tracing::warn;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};

const AFTER: &str = "--after";
const WAIT_FOR_EXIT: Duration = Duration::from_secs(15);
/// Longest argument the "ignored argument" log shows.
const SHOWN_ARGUMENT: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// The player. `ignored` is an argument that is not a Spotify link, for the log.
    App {
        link: Option<SpotifyLink>,
        ignored: Option<String>,
    },
    Setup,
    Uninstall,
    UninstallFrom(PathBuf),
    MoveData(PathBuf),
    /// Reads the YT Music sign-in cookies out of the WebView2 profile and prints
    /// them on stdout. With `show`, opens the sign-in window first.
    YtSession {
        show: bool,
    },
    /// Swaps this downloaded update into the given install folder.
    ApplyUpdate(PathBuf),
    #[cfg(feature = "demo")]
    DataMissing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub launch: Launch,
    /// A process to wait for before doing anything.
    pub after_pid: Option<u32>,
}

/// Reads the command line without the program name. `spotify:` launches pass a
/// single quoted link, so they can never produce one of the flags.
pub fn parse(mut args: impl Iterator<Item = OsString>, exe: &Path) -> Parsed {
    let mut first = args.next();
    let mut after_pid = None;

    if first.as_deref().is_some_and(|arg| arg == AFTER) {
        after_pid = args
            .next()
            .and_then(|pid| pid.to_str().and_then(|pid| pid.parse().ok()));
        first = args.next();
    }

    let launch = match first.as_ref().and_then(|arg| arg.to_str()) {
        Some("--setup") => Launch::Setup,
        Some("--uninstall") => Launch::Uninstall,
        Some("--uninstall-from") => args.next().map_or_else(
            || app(first.as_deref()),
            |dir| Launch::UninstallFrom(PathBuf::from(dir)),
        ),
        Some("--yt-session") => Launch::YtSession {
            show: args.next().is_some_and(|arg| arg == "--show"),
        },
        Some("--move-data") => args.next().map_or_else(
            || app(first.as_deref()),
            |dir| Launch::MoveData(PathBuf::from(dir)),
        ),
        Some("--apply-update") => args.next().map_or_else(
            || app(first.as_deref()),
            |dir| Launch::ApplyUpdate(PathBuf::from(dir)),
        ),
        #[cfg(feature = "demo")]
        Some("--data-missing") => Launch::DataMissing,
        _ if first.is_none() && is_setup_file(exe) => Launch::Setup,
        _ => app(first.as_deref()),
    };

    Parsed { launch, after_pid }
}

fn app(arg: Option<&std::ffi::OsStr>) -> Launch {
    let Some(arg) = arg else {
        return Launch::App {
            link: None,
            ignored: None,
        };
    };
    let link = arg.to_str().and_then(SpotifyLink::parse);
    let ignored = link
        .is_none()
        .then(|| arg.to_string_lossy().chars().take(SHOWN_ARGUMENT).collect());

    Launch::App { link, ignored }
}

/// A download renamed by the browser (`sonear-0.1.0-setup (1).exe`) still counts.
/// The demo build never turns itself into a setup file.
fn is_setup_file(exe: &Path) -> bool {
    !cfg!(feature = "demo")
        && exe
            .file_name()
            .is_some_and(|name| name.to_string_lossy().to_lowercase().contains("setup"))
}

/// Waits for the process that started this one to exit, so a restart does not
/// lose the single-instance race. Gives up after fifteen seconds.
pub fn wait_for_exit(pid: u32) {
    // SAFETY: a plain Win32 call; the handle is closed below.
    let Ok(process) = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }) else {
        return;
    };

    // SAFETY: `process` is a valid handle owned here.
    let waited = unsafe {
        let waited = WaitForSingleObject(process, WAIT_FOR_EXIT.as_millis() as u32);
        let _ = CloseHandle(process);

        waited
    };

    if waited.0 != 0 {
        warn!(pid, "the previous Sonear did not exit in time");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "4uLU6hMCjMI75M1A2tKUQC";

    fn parse_args(args: &[&str], exe: &str) -> Parsed {
        parse(args.iter().map(OsString::from), Path::new(exe))
    }

    fn launch(args: &[&str]) -> Launch {
        parse_args(args, r"C:\Apps\sonear.exe").launch
    }

    #[test]
    fn no_arguments_start_the_app() {
        assert_eq!(
            launch(&[]),
            Launch::App {
                link: None,
                ignored: None
            }
        );
    }

    #[test]
    fn flags_pick_the_mode() {
        assert_eq!(launch(&["--setup"]), Launch::Setup);
        assert_eq!(launch(&["--uninstall"]), Launch::Uninstall);
        assert_eq!(
            launch(&["--uninstall-from", r"C:\Apps\Sonear"]),
            Launch::UninstallFrom(PathBuf::from(r"C:\Apps\Sonear"))
        );
        assert_eq!(
            launch(&["--move-data", r"D:\Data\Sonear"]),
            Launch::MoveData(PathBuf::from(r"D:\Data\Sonear"))
        );
        assert_eq!(
            launch(&["--apply-update", r"C:\Apps\Sonear"]),
            Launch::ApplyUpdate(PathBuf::from(r"C:\Apps\Sonear"))
        );
    }

    #[test]
    fn an_update_is_applied_after_the_old_sonear_exits() {
        let parsed = parse_args(
            &["--after", "4242", "--apply-update", r"C:\Apps\Sonear"],
            r"C:\Users\me\AppData\Local\Sonear\updates\sonear-0.2.0-update.exe",
        );

        assert_eq!(parsed.after_pid, Some(4242));
        assert_eq!(
            parsed.launch,
            Launch::ApplyUpdate(PathBuf::from(r"C:\Apps\Sonear"))
        );
    }

    #[test]
    fn a_flag_without_its_folder_is_an_ignored_argument() {
        assert!(matches!(
            launch(&["--move-data"]),
            Launch::App {
                link: None,
                ignored: Some(_)
            }
        ));
        assert!(matches!(
            launch(&["--apply-update"]),
            Launch::App {
                link: None,
                ignored: Some(_)
            }
        ));
    }

    #[test]
    fn the_sign_in_helper_modes_are_parsed() {
        assert_eq!(launch(&["--yt-session"]), Launch::YtSession { show: false });
        assert_eq!(
            launch(&["--yt-session", "--show"]),
            Launch::YtSession { show: true }
        );
        assert_eq!(
            launch(&["--yt-session", "--other"]),
            Launch::YtSession { show: false }
        );
    }

    #[test]
    fn after_comes_first_and_carries_a_process_id() {
        let parsed = parse_args(&["--after", "4242", "--uninstall-from", r"C:\A"], "x.exe");

        assert_eq!(parsed.after_pid, Some(4242));
        assert_eq!(parsed.launch, Launch::UninstallFrom(PathBuf::from(r"C:\A")));

        let bad = parse_args(&["--after", "soon"], r"C:\Apps\sonear.exe");

        assert_eq!(bad.after_pid, None);
        assert!(matches!(bad.launch, Launch::App { .. }));

        let restart = parse_args(&["--after", "7"], r"C:\Apps\sonear.exe");

        assert_eq!(restart.after_pid, Some(7));
        assert!(matches!(
            restart.launch,
            Launch::App {
                link: None,
                ignored: None
            }
        ));
    }

    #[test]
    fn links_open_and_other_arguments_are_ignored() {
        let uri = format!("spotify:album:{ID}");

        assert_eq!(
            launch(&[&uri]),
            Launch::App {
                link: SpotifyLink::parse(&uri),
                ignored: None
            }
        );
        assert!(matches!(
            launch(&["--bogus"]),
            Launch::App {
                link: None,
                ignored: Some(_)
            }
        ));
    }

    #[test]
    fn a_link_cannot_inject_a_flag() {
        assert!(matches!(
            launch(&["spotify:--setup"]),
            Launch::App { link: None, .. }
        ));
    }

    #[cfg(not(feature = "demo"))]
    #[test]
    fn a_setup_named_file_runs_setup() {
        for exe in [
            r"C:\Downloads\sonear-0.1.0-setup.exe",
            r"C:\Downloads\Sonear-0.1.0-SETUP (1).exe",
        ] {
            assert_eq!(parse_args(&[], exe).launch, Launch::Setup);
        }

        assert!(matches!(
            parse_args(&[], r"C:\Apps\sonear.exe").launch,
            Launch::App { .. }
        ));
    }

    #[test]
    fn a_long_ignored_argument_is_cut() {
        let long = "x".repeat(500);
        let Launch::App { ignored, .. } = launch(&[&long]) else {
            panic!("expected the app");
        };

        assert_eq!(ignored.map(|text| text.len()), Some(SHOWN_ARGUMENT));
    }
}
