#![cfg_attr(feature = "demo", allow(dead_code))]

//! Signing in to YT Music through a WebView2 window, and reading the cookies
//! back from its profile.
//!
//! The work happens in a copy of this program started with `--yt-session`
//! (hidden) or `--yt-session --show` (the sign-in window). It prints one line
//! of JSON on stdout and exits; the app never keeps a copy of the cookies on
//! disk, WebView2's own profile is the only one. The cookies are secrets:
//! nothing here logs or prints them, except that one line to the parent.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use sonear_core::{AppPaths, SessionOutcome, SessionProvider};
use sonear_youtube::Session;

const ORIGIN: &str = "https://music.youtube.com";
const SIGN_IN_URL: &str = "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F";
const COOKIE: &str = "SAPISID";
const POLL: Duration = Duration::from_millis(500);
/// How long a hidden window lets the page refresh the cookies.
const SETTLE: Duration = Duration::from_secs(5);
/// How long a hidden window waits for a signed-in profile before saying it is signed out.
const SIGNED_OUT_AFTER: Duration = Duration::from_secs(10);
const GIVE_UP: Duration = Duration::from_secs(20);
/// How long the parent lets a hidden helper run before killing it.
pub const HIDDEN_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the parent lets the sign-in window stay open.
pub const INTERACTIVE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const WINDOW: (f64, f64) = (520., 760.);

/// What the helper found.
#[derive(Clone, PartialEq, Eq)]
pub enum Outcome {
    SignedIn(Vec<(String, String)>),
    SignedOut,
    Cancelled,
    Error(String),
}

impl fmt::Debug for Outcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SignedIn(cookies) => write!(formatter, "SignedIn({} cookies)", cookies.len()),
            Self::SignedOut => formatter.write_str("SignedOut"),
            Self::Cancelled => formatter.write_str("Cancelled"),
            Self::Error(message) => write!(formatter, "Error({message})"),
        }
    }
}

/// The line the helper prints.
pub fn encode(outcome: &Outcome) -> String {
    match outcome {
        Outcome::SignedIn(cookies) => json!({ "status": "signed_in", "cookies": cookies }),
        Outcome::SignedOut => json!({ "status": "signed_out" }),
        Outcome::Cancelled => json!({ "status": "cancelled" }),
        Outcome::Error(message) => json!({ "status": "error", "message": message }),
    }
    .to_string()
}

/// Reads what a helper printed. Anything unreadable is an error.
pub fn decode(output: &str) -> Outcome {
    let unreadable = || Outcome::Error("the sign-in helper answered nothing readable".into());
    let Some(line) = output
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with('{'))
    else {
        return unreadable();
    };
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return unreadable();
    };

    match value.get("status").and_then(Value::as_str) {
        Some("signed_in") => {
            let cookies = value
                .get("cookies")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|pair| {
                            let pair = pair.as_array()?;

                            Some((
                                pair.first()?.as_str()?.to_owned(),
                                pair.get(1)?.as_str()?.to_owned(),
                            ))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            if cookies.is_empty() {
                unreadable()
            } else {
                Outcome::SignedIn(cookies)
            }
        }
        Some("signed_out") => Outcome::SignedOut,
        Some("cancelled") => Outcome::Cancelled,
        Some("error") => Outcome::Error(
            value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("the sign-in helper failed")
                .to_owned(),
        ),
        _ => unreadable(),
    }
}

/// Runs a helper and returns what it found. Hidden runs are killed after
/// `HIDDEN_TIMEOUT`.
pub async fn request(show: bool) -> Outcome {
    let Ok(exe) = std::env::current_exe() else {
        return Outcome::Error("could not find the Sonear program".into());
    };
    let mut command = tokio::process::Command::new(exe);

    command
        .arg("--yt-session")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);

    if show {
        command.arg("--show");
    }

    #[cfg(windows)]
    command.creation_flags(0x0800_0000);

    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return Outcome::Error(format!("could not start the helper: {error}")),
    };
    let limit = if show {
        INTERACTIVE_TIMEOUT
    } else {
        HIDDEN_TIMEOUT
    };

    match tokio::time::timeout(limit, child.wait_with_output()).await {
        Ok(Ok(output)) => decode(&String::from_utf8_lossy(&output.stdout)),
        Ok(Err(error)) => Outcome::Error(format!("the helper failed: {error}")),
        Err(_) => Outcome::Error("the sign-in helper took too long".into()),
    }
}

/// Reads the session through a helper process.
pub struct AppSessionProvider;

#[async_trait]
impl SessionProvider for AppSessionProvider {
    async fn fetch(&self, interactive: bool) -> SessionOutcome {
        match request(interactive).await {
            Outcome::SignedIn(cookies) => {
                let session = Session::from_pairs(cookies);

                if session.is_signed_in() {
                    SessionOutcome::SignedIn(session)
                } else {
                    SessionOutcome::SignedOut
                }
            }
            Outcome::SignedOut => SessionOutcome::SignedOut,
            Outcome::Cancelled => SessionOutcome::Cancelled,
            Outcome::Error(message) => SessionOutcome::Failed(message),
        }
    }
}

fn profile_dir() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(dir) = std::env::var_os("SONEAR_YT_WEBVIEW_DIR") {
        return Some(PathBuf::from(dir));
    }

    AppPaths::discover().map(|paths| paths.youtube_webview_dir())
}

/// Entry point of the helper process: does the work, prints one line and returns.
pub fn run(show: bool, title: String) {
    let outcome = if cfg!(feature = "demo") {
        demo(show)
    } else {
        match profile_dir() {
            Some(dir) => web(show, dir, title),
            None => Outcome::Error("could not locate the user profile directories".into()),
        }
    };

    println!("{}", encode(&outcome));
}

fn demo(show: bool) -> Outcome {
    if show {
        std::thread::sleep(Duration::from_secs(1));
    }

    Outcome::SignedIn(vec![(COOKIE.to_owned(), "demo".to_owned())])
}

#[cfg(not(feature = "demo"))]
fn web(show: bool, profile: PathBuf, title: String) -> Outcome {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Instant;

    use tao::dpi::LogicalSize;
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoop};
    use tao::platform::run_return::EventLoopExtRunReturn as _;
    use tao::window::WindowBuilder;
    use wry::{WebContext, WebViewBuilder};

    if let Err(error) = std::fs::create_dir_all(&profile) {
        return Outcome::Error(format!("could not create the sign-in profile: {error}"));
    }

    let mut event_loop = EventLoop::new();
    let window = match WindowBuilder::new()
        .with_title(title)
        .with_visible(show)
        .with_inner_size(LogicalSize::new(WINDOW.0, WINDOW.1))
        .build(&event_loop)
    {
        Ok(window) => window,
        Err(error) => return Outcome::Error(format!("could not open the window: {error}")),
    };
    let mut context = WebContext::new(Some(profile));
    let url = if show {
        SIGN_IN_URL
    } else {
        "https://music.youtube.com/"
    };
    let webview = match WebViewBuilder::new_with_web_context(&mut context)
        .with_url(url)
        .build(&window)
    {
        Ok(webview) => webview,
        Err(error) => return Outcome::Error(format!("could not start WebView2: {error}")),
    };
    let outcome = Rc::new(RefCell::new(None));
    let result = outcome.clone();
    let started = Instant::now();

    event_loop.run_return(move |event, _, flow| {
        *flow = ControlFlow::WaitUntil(Instant::now() + POLL);

        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *result.borrow_mut() = Some(Outcome::Cancelled);
            *flow = ControlFlow::Exit;

            return;
        }

        let elapsed = started.elapsed();
        let cookies: Vec<(String, String)> = webview
            .cookies_for_url(ORIGIN)
            .map(|cookies| {
                cookies
                    .iter()
                    .map(|cookie| (cookie.name().to_owned(), cookie.value().to_owned()))
                    .collect()
            })
            .unwrap_or_default();
        let signed_in = cookies.iter().any(|(name, _)| name == COOKIE);
        let done = if show {
            let on_music = webview
                .url()
                .is_ok_and(|url| url.starts_with("https://music.youtube.com"));

            signed_in && on_music
        } else {
            signed_in && elapsed >= SETTLE
        };

        if done {
            *result.borrow_mut() = Some(Outcome::SignedIn(cookies));
            *flow = ControlFlow::Exit;
        } else if !show && elapsed >= GIVE_UP {
            *result.borrow_mut() = Some(Outcome::SignedOut);
            *flow = ControlFlow::Exit;
        } else if !show && elapsed >= SIGNED_OUT_AFTER && !signed_in {
            *result.borrow_mut() = Some(Outcome::SignedOut);
            *flow = ControlFlow::Exit;
        }
    });

    let found = outcome.borrow_mut().take();

    found.unwrap_or(Outcome::Cancelled)
}

#[cfg(feature = "demo")]
fn web(_show: bool, _profile: PathBuf, _title: String) -> Outcome {
    Outcome::Error("the demo build has no web view".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookies() -> Vec<(String, String)> {
        vec![
            ("SAPISID".to_owned(), "secret-one".to_owned()),
            (
                "HSID".to_owned(),
                "has \"quotes\" and ; semicolons".to_owned(),
            ),
        ]
    }

    #[test]
    fn every_outcome_round_trips() {
        for outcome in [
            Outcome::SignedIn(cookies()),
            Outcome::SignedOut,
            Outcome::Cancelled,
            Outcome::Error("WebView2 is missing".into()),
        ] {
            assert_eq!(decode(&encode(&outcome)), outcome);
        }
    }

    #[test]
    fn an_encoded_outcome_is_one_line() {
        assert!(!encode(&Outcome::SignedIn(cookies())).contains('\n'));
    }

    #[test]
    fn the_status_line_is_found_among_other_output() {
        let line = encode(&Outcome::SignedOut);

        assert_eq!(decode(&format!("noise\n{line}\n")), Outcome::SignedOut);
    }

    #[test]
    fn unreadable_output_is_an_error() {
        for output in [
            "",
            "garbage",
            "{",
            r#"{"status":"mystery"}"#,
            r#"{"status":"signed_in"}"#,
            r#"{"status":"signed_in","cookies":[]}"#,
            r#"{"status":"signed_in","cookies":[["only-a-name"]]}"#,
        ] {
            assert!(matches!(decode(output), Outcome::Error(_)), "{output}");
        }
    }

    #[test]
    fn debug_never_shows_cookie_values() {
        let shown = format!("{:?}", Outcome::SignedIn(cookies()));

        assert_eq!(shown, "SignedIn(2 cookies)");
        assert!(!shown.contains("secret-one"));
    }
}
