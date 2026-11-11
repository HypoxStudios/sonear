//! Console and file logs, and the markers that tell the next launch how this one ended.

use std::backtrace::Backtrace;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use tracing::{error, warn};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::filter::{FilterExt as _, LevelFilter, Targets, filter_fn};
use tracing_subscriber::fmt::time::ChronoLocal;
use tracing_subscriber::fmt::writer::MakeWriter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Layer as _, fmt};

const DEMO: bool = cfg!(feature = "demo");
const SOLOIST_TARGET: &str = "soloist_output";
const KEEP_FILES: usize = 7;
const CRATES: [&str; 4] = ["sonear", "sonear_core", "sonear_soloist", "sonear_spotify"];

pub struct Options {
    /// Where the log files go. `None` logs to the console only.
    pub dir: Option<PathBuf>,
    pub detailed: bool,
}

/// A log file that can be closed while the process keeps running.
#[derive(Clone)]
struct SwappableFile(Arc<Mutex<Option<RollingFileAppender>>>);

fn appender(dir: &Path, prefix: &str) -> Option<RollingFileAppender> {
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(prefix)
        .filename_suffix("log")
        .max_log_files(KEEP_FILES)
        .build(dir)
        .ok()
}

impl SwappableFile {
    fn open(dir: &Path, prefix: &str) -> Option<Self> {
        Some(Self(Arc::new(Mutex::new(Some(appender(dir, prefix)?)))))
    }

    fn replace(&self, file: Option<RollingFileAppender>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = file;
    }

    fn close(&self) {
        self.replace(None);
    }
}

impl Write for SwappableFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            Some(file) => file.write(buf),
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

impl<'a> MakeWriter<'a> for SwappableFile {
    type Writer = SwappableFile;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

struct Files {
    dir: PathBuf,
    sonear: SwappableFile,
    soloist: SwappableFile,
}

static FILES: OnceLock<Files> = OnceLock::new();

const SONEAR_PREFIX: &str = "sonear";
const SOLOIST_PREFIX: &str = "soloist";

fn crashed_marker(dir: &Path) -> PathBuf {
    dir.join("crashed")
}

fn running_marker(dir: &Path) -> PathBuf {
    dir.join("running")
}

fn level_filter(detailed: bool) -> EnvFilter {
    if let Ok(filter) = EnvFilter::try_from_default_env() {
        return filter;
    }

    let level = if detailed { "debug" } else { "info" };
    let mut directives: Vec<String> = CRATES
        .iter()
        .map(|name| format!("{name}={level}"))
        .collect();

    directives.push("panic=error".to_owned());
    directives.push(format!("{SOLOIST_TARGET}=info"));

    EnvFilter::new(directives.join(","))
}

fn now() -> String {
    chrono::Local::now().to_rfc3339()
}

/// Sets up logging and the panic hook. Call once, before anything logs.
pub fn init(options: Options) {
    // Pruning matches files by prefix, so demo logs get their own folder
    // instead of a prefix that real pruning would also match.
    let dir = options
        .dir
        .map(|dir| if DEMO { dir.join("demo") } else { dir });
    let files = dir.and_then(|dir| {
        let sonear = SwappableFile::open(&dir, SONEAR_PREFIX)?;
        let soloist = SwappableFile::open(&dir, SOLOIST_PREFIX)?;

        Some(Files {
            dir,
            sonear,
            soloist,
        })
    });

    let console = fmt::layer()
        .with_timer(ChronoLocal::rfc_3339())
        .with_filter(level_filter(options.detailed));
    let sonear_layer = files.as_ref().map(|files| {
        fmt::layer()
            .with_ansi(false)
            .with_timer(ChronoLocal::rfc_3339())
            .with_writer(files.sonear.clone())
            .with_filter(
                level_filter(options.detailed)
                    .and(filter_fn(|meta| meta.target() != SOLOIST_TARGET)),
            )
    });
    let soloist_layer = files.as_ref().map(|files| {
        fmt::layer()
            .with_ansi(false)
            .with_timer(ChronoLocal::rfc_3339())
            .with_writer(files.soloist.clone())
            .with_filter(Targets::new().with_target(SOLOIST_TARGET, LevelFilter::INFO))
    });

    let _ = tracing_subscriber::registry()
        .with(console)
        .with(sonear_layer)
        .with(soloist_layer)
        .try_init();

    if let Some(files) = files {
        let _ = FILES.set(files);
    }

    install_panic_hook();
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| (*text).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-text panic payload".to_owned());
        let location = info
            .location()
            .map_or_else(|| "unknown".to_owned(), ToString::to_string);
        let thread = std::thread::current();
        let backtrace = Backtrace::force_capture();

        error!(
            target: "panic",
            thread = thread.name().unwrap_or("unnamed"),
            %location,
            %backtrace,
            "{message}"
        );

        if let Some(files) = FILES.get() {
            let _ = std::fs::write(crashed_marker(&files.dir), now());
        }

        previous(info);
    }));
}

/// Closes the log files and removes the running marker, for when the data
/// folder is about to be deleted or moved.
pub fn close_files() {
    let Some(files) = FILES.get() else {
        return;
    };

    files.sonear.close();
    files.soloist.close();
    remove_marker_if_ours(&running_marker(&files.dir), std::process::id());
}

/// Opens the log files again and marks this session as running, after
/// `close_files` for a hand-over that did not happen.
pub fn reopen_files() {
    let Some(files) = FILES.get() else {
        return;
    };

    files.sonear.replace(appender(&files.dir, SONEAR_PREFIX));
    files.soloist.replace(appender(&files.dir, SOLOIST_PREFIX));
    mark_running();
}

/// After a source switch the new process owns the marker, and this one must
/// not delete it on its way out.
fn remove_marker_if_ours(marker: &Path, pid: u32) {
    let ours =
        std::fs::read_to_string(marker).is_ok_and(|text| text.starts_with(&format!("pid={pid} ")));

    if ours {
        let _ = std::fs::remove_file(marker);
    }
}

/// Deletes every log file and starts new ones, for "Delete all Sonear data".
/// The running marker stays, since this session is still running.
pub fn delete_files() {
    let Some(files) = FILES.get() else {
        return;
    };

    files.sonear.close();
    files.soloist.close();

    if let Ok(entries) = std::fs::read_dir(&files.dir) {
        for entry in entries.flatten() {
            let path = entry.path();

            if path.extension().is_some_and(|extension| extension == "log") {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    let _ = std::fs::remove_file(crashed_marker(&files.dir));

    files.sonear.replace(appender(&files.dir, SONEAR_PREFIX));
    files.soloist.replace(appender(&files.dir, SOLOIST_PREFIX));
}

/// The folder this process writes its logs to, if it writes any.
pub fn dir() -> Option<PathBuf> {
    FILES.get().map(|files| files.dir.clone())
}

/// Whether the last session panicked. Reads and deletes the marker.
pub fn take_crash_marker() -> bool {
    let Some(files) = FILES.get() else {
        return false;
    };

    std::fs::remove_file(crashed_marker(&files.dir)).is_ok()
}

/// Notes that this session is running. A marker left by the previous one means
/// it never got to exit cleanly. Call before `take_crash_marker`.
pub fn mark_running() {
    let Some(files) = FILES.get() else {
        return;
    };
    let marker = running_marker(&files.dir);

    if let Ok(text) = std::fs::read_to_string(&marker) {
        warn!(
            details = text.trim(),
            after_panic = crashed_marker(&files.dir).exists(),
            "the previous session ended without a clean exit"
        );
    }

    let _ = std::fs::write(
        &marker,
        format!("pid={} started={}", std::process::id(), now()),
    );
}

/// Removes the running marker. Safe to call more than once.
pub fn clear_running() {
    if let Some(files) = FILES.get() {
        remove_marker_if_ours(&running_marker(&files.dir), std::process::id());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_owner_removes_the_running_marker() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("running");

        std::fs::write(&marker, "pid=42 started=now").unwrap();
        remove_marker_if_ours(&marker, 43);

        assert!(marker.exists());

        remove_marker_if_ours(&marker, 4);

        assert!(marker.exists());

        remove_marker_if_ours(&marker, 42);

        assert!(!marker.exists());
        remove_marker_if_ours(&marker, 42);
    }
}
