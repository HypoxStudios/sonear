use std::io::Write as _;
use std::time::{Duration, Instant};

use sonear_core::SpotifyLink;
use tokio::io::AsyncReadExt as _;
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tracing::{info, warn};
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcessId, OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE,
};
use windows::core::{PCWSTR, w};

/// Longest message the pipe accepts.
const MAX_MESSAGE: usize = 1024;
const CONNECT_FOR: Duration = Duration::from_secs(2);
const READ_FOR: Duration = Duration::from_secs(2);
const RETRY_EVERY: Duration = Duration::from_millis(50);
const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_PIPE_BUSY: i32 = 231;

/// Holds the named mutex until it is dropped or the process exits.
pub struct InstanceGuard(HANDLE);

// SAFETY: the handle is only ever closed, which any thread may do.
unsafe impl Send for InstanceGuard {}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        // SAFETY: the handle is owned by this guard and not used again.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// What a second launch asks the running Sonear to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handoff {
    Open(SpotifyLink),
    Show,
    /// Close, so an installer or a data move can take over.
    Quit,
}

fn mutex_name(demo: bool) -> PCWSTR {
    if demo {
        w!(r"Local\SonearSingleInstanceDemo")
    } else {
        w!(r"Local\SonearSingleInstance")
    }
}

/// Returns `None` if another Sonear already holds the mutex. The demo uses its
/// own name so it can run next to a real install.
pub fn acquire(demo: bool) -> Option<InstanceGuard> {
    // SAFETY: plain Win32 calls; the handle is closed when the guard drops.
    unsafe {
        let handle = CreateMutexW(None, true, mutex_name(demo)).ok()?;

        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(handle);

            return None;
        }

        Some(InstanceGuard(handle))
    }
}

/// Whether a Sonear is running, without taking the mutex.
pub fn is_running(demo: bool) -> bool {
    // SAFETY: plain Win32 calls; an opened handle is closed right away.
    unsafe {
        match OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, mutex_name(demo)) {
            Ok(handle) => {
                let _ = CloseHandle(handle);

                true
            }
            Err(_) => false,
        }
    }
}

/// Asks the running Sonear to quit. Does not wait for it.
pub fn request_quit(demo: bool) {
    forward(demo, &Handoff::Quit);
}

pub fn encode(handoff: &Handoff) -> String {
    match handoff {
        Handoff::Open(link) => format!("open {}\n", link.uri()),
        Handoff::Show => "show\n".to_owned(),
        Handoff::Quit => "quit\n".to_owned(),
    }
}

/// Reads one message. The link is parsed again and must be in its canonical
/// form, because the pipe is not trusted.
pub fn decode(bytes: &[u8]) -> Option<Handoff> {
    if bytes.len() > MAX_MESSAGE {
        return None;
    }

    let line = std::str::from_utf8(bytes)
        .ok()?
        .trim_end_matches(['\r', '\n']);

    match line {
        "show" => return Some(Handoff::Show),
        "quit" => return Some(Handoff::Quit),
        _ => {}
    }

    let uri = line.strip_prefix("open ")?;
    let link = SpotifyLink::parse(uri)?;

    (link.uri() == uri).then_some(Handoff::Open(link))
}

fn pipe_name(demo: bool) -> String {
    let mut session = 0u32;

    // SAFETY: plain Win32 call with a valid out pointer.
    let found = unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) };

    if found.is_err() {
        session = 0;
    }

    if demo {
        format!(r"\\.\pipe\sonear-demo-{session}")
    } else {
        format!(r"\\.\pipe\sonear-{session}")
    }
}

/// Sends `handoff` to the running Sonear, waiting up to two seconds for it to
/// start listening. Logs and gives up on failure.
pub fn forward(demo: bool, handoff: &Handoff) {
    match send(&pipe_name(demo), handoff, CONNECT_FOR) {
        Ok(()) => info!("handed over to the running Sonear"),
        Err(error) => warn!(%error, "could not reach the running Sonear"),
    }
}

fn send(name: &str, handoff: &Handoff, wait: Duration) -> std::io::Result<()> {
    let started = Instant::now();

    loop {
        match std::fs::OpenOptions::new().write(true).open(name) {
            Ok(mut pipe) => return pipe.write_all(encode(handoff).as_bytes()),
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(ERROR_FILE_NOT_FOUND | ERROR_PIPE_BUSY)
                ) && started.elapsed() < wait =>
            {
                std::thread::sleep(RETRY_EVERY);
            }
            Err(error) => return Err(error),
        }
    }
}

/// Listens for hand-offs from later launches. `None` if the pipe could not be
/// created; Sonear then runs without hand-off.
pub fn serve(runtime: &Handle, demo: bool) -> Option<mpsc::UnboundedReceiver<Handoff>> {
    let name = pipe_name(demo);
    let _entered = runtime.enter();

    match serve_on(name.clone()) {
        Ok(inbox) => Some(inbox),
        Err(error) => {
            warn!(%error, "could not listen for other launches");

            None
        }
    }
}

/// The default DACL only lets the creating user, administrators and SYSTEM
/// write to the pipe, which is what is wanted. Must run inside the runtime.
fn serve_on(name: String) -> std::io::Result<mpsc::UnboundedReceiver<Handoff>> {
    let first = ServerOptions::new()
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .access_inbound(true)
        .create(&name)?;
    let (sender, inbox) = mpsc::unbounded_channel();

    tokio::spawn(accept(first, name, sender));

    Ok(inbox)
}

async fn accept(mut server: NamedPipeServer, name: String, sender: mpsc::UnboundedSender<Handoff>) {
    loop {
        if let Err(error) = server.connect().await {
            warn!(%error, "the hand-off pipe stopped accepting");

            return;
        }

        let connected = server;

        server = match ServerOptions::new()
            .reject_remote_clients(true)
            .access_inbound(true)
            .create(&name)
        {
            Ok(next) => next,
            Err(error) => {
                warn!(%error, "could not reopen the hand-off pipe");
                tokio::spawn(receive(connected, sender.clone()));

                return;
            }
        };

        tokio::spawn(receive(connected, sender.clone()));

        if sender.is_closed() {
            return;
        }
    }
}

async fn receive(pipe: NamedPipeServer, sender: mpsc::UnboundedSender<Handoff>) {
    let mut bytes = Vec::new();
    let mut limited = pipe.take(MAX_MESSAGE as u64 + 1);
    let read = tokio::time::timeout(READ_FOR, limited.read_to_end(&mut bytes)).await;

    if !matches!(read, Ok(Ok(_))) {
        warn!("ignored an unfinished hand-off message");

        return;
    }

    match decode(&bytes) {
        Some(handoff) => {
            let _ = sender.send(handoff);
        }
        None => warn!("ignored an invalid hand-off message"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "4uLU6hMCjMI75M1A2tKUQC";

    fn link() -> SpotifyLink {
        SpotifyLink::Album(ID.to_owned())
    }

    #[test]
    fn messages_round_trip() {
        for handoff in [Handoff::Open(link()), Handoff::Show, Handoff::Quit] {
            assert_eq!(decode(encode(&handoff).as_bytes()), Some(handoff));
        }
    }

    #[test]
    fn decode_rejects_everything_else() {
        let long = format!("open spotify:album:{ID}{}", " ".repeat(MAX_MESSAGE));

        assert_eq!(decode(b""), None);
        assert_eq!(decode(b"quit now\n"), None);
        assert_eq!(decode(b"Quit\n"), None);
        assert_eq!(decode(b"show now\n"), None);
        assert_eq!(decode(b"open \n"), None);
        assert_eq!(decode(b"open spotify:track:../me\n"), None);
        assert_eq!(decode(b"open https://evil.com/track/x\n"), None);
        assert_eq!(decode(&[0xff, 0xfe]), None);
        assert_eq!(decode(long.as_bytes()), None);
        assert_eq!(
            decode(format!("open https://open.spotify.com/album/{ID}\n").as_bytes()),
            None
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_message_crosses_a_real_pipe() {
        let name = format!(r"\\.\pipe\sonear-test-{}-{}", std::process::id(), line!());
        let mut inbox = serve_on(name.clone()).unwrap();

        for handoff in [Handoff::Open(link()), Handoff::Show] {
            let (name, sent) = (name.clone(), handoff.clone());

            tokio::task::spawn_blocking(move || send(&name, &sent, Duration::from_secs(2)))
                .await
                .unwrap()
                .unwrap();

            assert_eq!(inbox.recv().await, Some(handoff));
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_second_server_cannot_take_the_name() {
        let name = format!(r"\\.\pipe\sonear-test-{}-{}", std::process::id(), line!());
        let _first = serve_on(name.clone()).unwrap();

        assert!(serve_on(name).is_err());
    }

    #[test]
    fn sending_to_a_missing_pipe_gives_up() {
        let name = format!(r"\\.\pipe\sonear-test-missing-{}", std::process::id());

        assert!(send(&name, &Handoff::Show, Duration::from_millis(120)).is_err());
    }
}
