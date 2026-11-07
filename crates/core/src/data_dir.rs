//! Moving the data folder, and deleting everything Sonear keeps locally.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use rust_i18n::t;
use thiserror::Error;
use tracing::warn;

use crate::paths::AppPaths;
use crate::secrets::{self, SecretStore};

const FOLDER_NAME: &str = "Sonear";
const RETRY_FOR: Duration = Duration::from_secs(5);
const RETRY_EVERY: Duration = Duration::from_millis(100);
const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_SHARING_VIOLATION: i32 = 32;
const ERROR_LOCK_VIOLATION: i32 = 33;
const ERROR_NOT_SAME_DEVICE: i32 = 17;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

#[derive(Debug, Error)]
pub enum MoveError {
    #[error("That is the folder Sonear already uses.")]
    Same,
    #[error("Pick a folder outside the current data folder.")]
    InsideCurrent,
    #[error("Pick a folder that is not a parent of the current data folder.")]
    ContainsCurrent,
    #[error("{} already contains files.", .0.display())]
    NotEmpty(PathBuf),
    #[error("Sonear can't write to {}: {source}", .path.display())]
    Unwritable { path: PathBuf, source: io::Error },
    #[error("Moving the data failed: {0}")]
    Io(#[from] io::Error),
}

impl MoveError {
    /// The error in the interface language. `Display` stays English for logs.
    pub fn user_message(&self) -> String {
        match self {
            Self::Same => t!("move.same"),
            Self::InsideCurrent => t!("move.inside_current"),
            Self::ContainsCurrent => t!("move.contains_current"),
            Self::NotEmpty(path) => t!("move.not_empty", path = path.display().to_string()),
            Self::Unwritable { path, source } => t!(
                "move.unwritable",
                path = path.display().to_string(),
                detail = source.to_string()
            ),
            Self::Io(error) => t!("move.io", detail = error.to_string()),
        }
        .into_owned()
    }
}

/// How the data got to its new place, which decides how to undo it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moved {
    Renamed,
    Copied,
}

fn parts(path: &Path) -> Vec<OsString> {
    path.components()
        .filter(|part| !matches!(part, Component::CurDir))
        .map(|part| part.as_os_str().to_string_lossy().to_lowercase().into())
        .collect()
}

/// Whether `inner` is `outer` or sits under it, compared case-insensitively by components.
pub fn is_within(inner: &Path, outer: &Path) -> bool {
    let (inner, outer) = (parts(inner), parts(outer));

    inner.len() >= outer.len() && inner[..outer.len()] == outer[..]
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    parts(a) == parts(b)
}

/// Whether two paths are the same place on disk. Windows may spell a folder with
/// its short name (`RUNNER~1`), so existing paths are compared as their real ones.
pub fn same_place(a: &Path, b: &Path) -> bool {
    let real = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());

    same_path(&real(a), &real(b))
}

/// The folder Sonear's data goes in for a folder the user picked: always
/// `<picked>\Sonear`, unless that is already the name. The folder is later
/// removed recursively, so it must never be one of the user's own.
pub fn resolve_target(picked: &Path) -> PathBuf {
    let named = picked
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(FOLDER_NAME));

    if named {
        picked.to_owned()
    } else {
        picked.join(FOLDER_NAME)
    }
}

/// Checks that `from` can move to `to`. Leaves nothing behind.
pub fn check_target(from: &Path, to: &Path) -> Result<(), MoveError> {
    if same_path(from, to) {
        return Err(MoveError::Same);
    }

    if is_within(to, from) {
        return Err(MoveError::InsideCurrent);
    }

    if is_within(from, to) {
        return Err(MoveError::ContainsCurrent);
    }

    if to.exists() {
        let empty = fs::read_dir(to).map_err(MoveError::Io)?.next().is_none();

        if !empty {
            return Err(MoveError::NotEmpty(to.to_owned()));
        }
    }

    probe_writable(to).map_err(|source| MoveError::Unwritable {
        path: to.to_owned(),
        source,
    })
}

/// Creates `dir`, writes and removes a file in it, then removes what it created.
pub fn probe_writable(dir: &Path) -> io::Result<()> {
    let first_new = dir
        .ancestors()
        .filter(|path| !path.as_os_str().is_empty() && !path.exists())
        .last();

    fs::create_dir_all(dir)?;

    let probe = dir.join(".sonear-write-test");
    let written = fs::write(&probe, b"");

    let _ = fs::remove_file(&probe);

    if let Some(created) = first_new {
        let _ = fs::remove_dir_all(created);
    }

    written
}

fn is_busy(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION)
    )
}

/// Runs `action` again for up to five seconds while it fails because another
/// process still holds the file (a sharing violation, or access denied while
/// a program that is exiting lets go).
pub fn retry_busy<T>(mut action: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let started = Instant::now();

    loop {
        match action() {
            Err(error) if is_busy(&error) && started.elapsed() < RETRY_FOR => {
                std::thread::sleep(RETRY_EVERY);
            }
            other => return other,
        }
    }
}

/// Moves the data folder. A missing `from` just creates `to`. `progress` gets
/// bytes done and total while files are copied across volumes.
pub fn move_data(from: &Path, to: &Path, progress: &dyn Fn(u64, u64)) -> Result<Moved, MoveError> {
    check_target(from, to)?;

    if !from.exists() {
        fs::create_dir_all(to)?;

        return Ok(Moved::Renamed);
    }

    if to.exists() {
        fs::remove_dir(to)?;
    }

    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }

    match retry_busy(|| fs::rename(from, to)) {
        Ok(()) => {
            progress(1, 1);

            Ok(Moved::Renamed)
        }
        Err(error) if error.raw_os_error() == Some(ERROR_NOT_SAME_DEVICE) => {
            if let Err(error) = copy_tree(from, to, progress) {
                let _ = fs::remove_dir_all(to);

                return Err(error.into());
            }

            Ok(Moved::Copied)
        }
        Err(error) => Err(error.into()),
    }
}

/// Puts the data back after a move whose follow-up step failed.
pub fn undo_move(from: &Path, to: &Path, how: Moved) -> io::Result<()> {
    match how {
        Moved::Renamed => retry_busy(|| fs::rename(to, from)),
        Moved::Copied => fs::remove_dir_all(to),
    }
}

/// Removes the old folder once the move is complete.
pub fn finish_move(from: &Path) -> io::Result<()> {
    retry_busy(|| fs::remove_dir_all(from))
}

fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;

        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    #[cfg(not(windows))]
    {
        let _ = FILE_ATTRIBUTE_REPARSE_POINT;

        metadata.file_type().is_symlink()
    }
}

/// Copies every file under `from` into `to`, skipping links and junctions.
fn copy_tree(from: &Path, to: &Path, progress: &dyn Fn(u64, u64)) -> io::Result<()> {
    let mut files = Vec::new();
    let mut folders = vec![PathBuf::new()];
    let mut total = 0;
    let mut next = 0;

    while next < folders.len() {
        let relative = folders[next].clone();

        next += 1;

        for entry in fs::read_dir(from.join(&relative))? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;

            if is_reparse_point(&metadata) {
                continue;
            }

            let path = relative.join(entry.file_name());

            if metadata.is_dir() {
                folders.push(path);
            } else if metadata.is_file() {
                total += metadata.len();
                files.push(path);
            }
        }
    }

    for folder in &folders {
        fs::create_dir_all(to.join(folder))?;
    }

    let mut done = 0;

    progress(0, total);

    for file in &files {
        done += fs::copy(from.join(file), to.join(file))?;

        progress(done, total);
    }

    Ok(())
}

/// Deletes every secret and the local data and config folders. A folder that
/// is already gone is fine. Returns what could not be deleted.
pub fn delete_local_data(paths: &AppPaths, secrets: &dyn SecretStore) -> Vec<String> {
    let mut problems = Vec::new();

    for name in secrets::ALL_KEYS {
        if let Err(error) = secrets.delete(name) {
            problems.push(format!("{name}: {error}"));
        }
    }

    for folder in [paths.local_root(), paths.config_dir().to_owned()] {
        match retry_busy(|| fs::remove_dir_all(&folder)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                warn!(%error, "could not delete a data folder");
                problems.push(format!("{}: {error}", folder.display()));
            }
        }
    }

    problems
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::secrets::{MemoryStore, Secret};

    fn tree(root: &Path) {
        fs::create_dir_all(root.join("cache/covers")).unwrap();
        fs::write(root.join("history.db"), vec![7u8; 100]).unwrap();
        fs::write(root.join("cache/covers/a.jpg"), vec![1u8; 50]).unwrap();
    }

    #[test]
    fn the_target_always_ends_in_sonear() {
        assert_eq!(
            resolve_target(Path::new(r"D:\Music")),
            PathBuf::from(r"D:\Music\Sonear")
        );
        assert_eq!(
            resolve_target(Path::new(r"D:\Stuff\sonear")),
            PathBuf::from(r"D:\Stuff\sonear")
        );
    }

    #[test]
    fn paths_compare_by_component_without_case() {
        assert!(is_within(Path::new(r"C:\A\B\c"), Path::new(r"c:\a\b")));
        assert!(is_within(Path::new(r"C:\A\B"), Path::new(r"C:\A\B")));
        assert!(!is_within(Path::new(r"C:\A\Bc"), Path::new(r"C:\A\B")));
        assert!(!is_within(Path::new(r"C:\A"), Path::new(r"C:\A\B")));
        assert!(same_path(Path::new(r"C:\A\B"), Path::new(r"c:\a\b")));
    }

    #[test]
    fn the_same_place_can_be_spelled_differently() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("other");

        fs::create_dir(&other).unwrap();

        assert!(same_place(dir.path(), &other.join("..")));
        assert!(!same_path(dir.path(), &other.join("..")));
        assert!(!same_place(dir.path(), &other));
        assert!(same_place(
            Path::new(r"C:\Gone\Here"),
            Path::new(r"c:\gone\here")
        ));
    }

    #[test]
    fn unfit_targets_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        let full = dir.path().join("full");

        tree(&from);
        fs::create_dir_all(&full).unwrap();
        fs::write(full.join("x"), b"x").unwrap();

        assert!(matches!(check_target(&from, &from), Err(MoveError::Same)));
        assert!(matches!(
            check_target(&from, &from.join("inner")),
            Err(MoveError::InsideCurrent)
        ));
        assert!(matches!(
            check_target(&from, dir.path()),
            Err(MoveError::ContainsCurrent)
        ));
        assert!(matches!(
            check_target(&from, &full),
            Err(MoveError::NotEmpty(_))
        ));
    }

    #[test]
    fn a_blocked_target_is_unwritable_and_checking_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        let blocker = dir.path().join("file");
        let fresh = dir.path().join("new").join("Sonear");

        fs::write(&blocker, b"x").unwrap();

        assert!(matches!(
            check_target(&from, &blocker.join("Sonear")),
            Err(MoveError::Unwritable { .. })
        ));
        assert!(check_target(&from, &fresh).is_ok());
        assert!(!dir.path().join("new").exists());
    }

    #[test]
    fn data_moves_by_rename() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        let to = dir.path().join("deep/er/Sonear");

        tree(&from);

        assert_eq!(move_data(&from, &to, &|_, _| {}).unwrap(), Moved::Renamed);
        assert!(!from.exists());
        assert_eq!(fs::read(to.join("history.db")).unwrap().len(), 100);
        assert!(to.join("cache/covers/a.jpg").exists());
    }

    #[test]
    fn an_empty_target_is_replaced_and_a_missing_source_just_creates_it() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        let to = dir.path().join("to");

        fs::create_dir_all(&to).unwrap();
        tree(&from);
        move_data(&from, &to, &|_, _| {}).unwrap();

        assert!(to.join("history.db").exists());

        let nothing = dir.path().join("nothing");
        let made = dir.path().join("made");

        move_data(&nothing, &made, &|_, _| {}).unwrap();

        assert!(made.is_dir());
    }

    #[test]
    fn copying_reports_bytes_and_keeps_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        let to = dir.path().join("to");
        let seen = RefCell::new(Vec::new());

        tree(&from);
        copy_tree(&from, &to, &|done, total| {
            seen.borrow_mut().push((done, total))
        })
        .unwrap();

        assert_eq!(seen.borrow().last(), Some(&(150, 150)));
        assert!(from.join("history.db").exists());
        assert_eq!(fs::read(to.join("cache/covers/a.jpg")).unwrap().len(), 50);
    }

    #[test]
    fn a_move_can_be_undone_and_finished() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        let to = dir.path().join("to");

        tree(&from);
        let how = move_data(&from, &to, &|_, _| {}).unwrap();
        undo_move(&from, &to, how).unwrap();

        assert!(from.join("history.db").exists());
        assert!(!to.exists());

        finish_move(&from).unwrap();

        assert!(!from.exists());
    }

    #[test]
    fn deleting_local_data_removes_folders_and_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(dir.path().join("config"), dir.path().join("local"));
        let store = MemoryStore::default();

        tree(&paths.local_root());
        fs::create_dir_all(paths.config_dir()).unwrap();
        fs::write(paths.config_file(), b"x").unwrap();
        store
            .set(secrets::SOLOIST_API_KEY, &Secret::new("k"))
            .unwrap();
        store
            .set(secrets::SPOTIFY_REFRESH_TOKEN, &Secret::new("t"))
            .unwrap();

        assert!(delete_local_data(&paths, &store).is_empty());
        assert!(!paths.local_root().exists());
        assert!(!paths.config_dir().exists());
        assert_eq!(store.get(secrets::SOLOIST_API_KEY).unwrap(), None);
        assert_eq!(store.get(secrets::SPOTIFY_REFRESH_TOKEN).unwrap(), None);
        assert!(delete_local_data(&paths, &store).is_empty());
    }
}
