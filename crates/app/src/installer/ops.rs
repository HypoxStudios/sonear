//! Installing and uninstalling, with no window. Everything that touches the
//! machine goes through a `Target`, so tests can point it at temp folders and
//! a throwaway registry key.

use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};

use sonear_core::data_dir::{is_within, probe_writable, retry_busy, same_path};
use sonear_core::{AppPaths, KeyringStore, delete_local_data};
use thiserror::Error;
use tracing::{info, warn};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_Programs};

use crate::app_identity::{self, INSTALLED_APP_ID, SHORTCUT_NAME};
use crate::protocol::{self, ProtocolError};

pub const EXE_NAME: &str = "sonear.exe";
const STAGING_NAME: &str = "sonear.exe.new";
const LICENSE_NAME: &str = "LICENSE";
const NOTICE_NAME: &str = "NOTICE";
const LICENSE: &[u8] = include_bytes!("../../../../LICENSE");
const NOTICE: &[u8] = include_bytes!("../../../../NOTICE");
pub const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Sonear";
const ABOUT_URL: &str = "https://github.com/HypoxStudios/sonear";
const PUBLISHER: &str = "Hypox Studios";
const COPY_CHUNK: usize = 256 * 1024;
const VERSION: &str = env!("CARGO_PKG_VERSION");

pub type Progress<'a> = &'a dyn Fn(f32, &'static str);

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("Could not {step}: {source}")]
    Io {
        step: &'static str,
        source: io::Error,
    },
    #[error("Could not find the {0} folder: {1}")]
    Folder(&'static str, String),
    #[error("Could not create a shortcut: {0}")]
    Shortcut(String),
    #[error("{0}")]
    Registry(#[from] ProtocolError),
}

fn step(step: &'static str) -> impl FnOnce(io::Error) -> InstallError {
    move |source| InstallError::Io { step, source }
}

/// Where an install lives and what it registers.
#[derive(Debug, Clone)]
pub struct Target {
    pub dir: PathBuf,
    pub programs_dir: PathBuf,
    pub desktop_dir: PathBuf,
    pub uninstall_key: String,
}

impl Target {
    /// The current user's Start Menu, Desktop and installed-apps entry.
    pub fn real(dir: PathBuf) -> Result<Self, InstallError> {
        let folder = |name, id| {
            app_identity::known_folder(id).map_err(|error| InstallError::Folder(name, error))
        };

        Ok(Self {
            dir,
            programs_dir: folder("Start menu", &FOLDERID_Programs)?,
            desktop_dir: folder("Desktop", &FOLDERID_Desktop)?,
            uninstall_key: UNINSTALL_KEY.to_owned(),
        })
    }

    pub fn exe(&self) -> PathBuf {
        self.dir.join(EXE_NAME)
    }

    fn staging(&self) -> PathBuf {
        self.dir.join(STAGING_NAME)
    }

    fn start_menu_link(&self) -> PathBuf {
        self.programs_dir.join(SHORTCUT_NAME)
    }

    fn desktop_link(&self) -> PathBuf {
        self.desktop_dir.join(SHORTCUT_NAME)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct InstallOptions {
    pub desktop_shortcut: bool,
    pub is_update: bool,
}

/// An install found in the installed-apps list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub dir: PathBuf,
    pub version: String,
}

/// Initialises COM for the calling thread until dropped.
struct Com(bool);

impl Com {
    fn init() -> Self {
        // SAFETY: plain COM initialisation, balanced in `drop` when it succeeded.
        Self(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok())
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balances the successful `CoInitializeEx` above.
            unsafe { CoUninitialize() };
        }
    }
}

/// What a run created, so a failed fresh install can take it back.
#[derive(Default)]
struct Made {
    files: Vec<PathBuf>,
    registered: bool,
}

pub fn install(
    target: &Target,
    source: &Path,
    options: InstallOptions,
    progress: Progress,
) -> Result<(), InstallError> {
    let _com = Com::init();
    let mut made = Made::default();
    let result = run_install(target, source, options, progress, &mut made);

    if let Err(error) = &result {
        warn!(%error, "the install failed");
        roll_back(target, options, &made);
    }

    result
}

fn run_install(
    target: &Target,
    source: &Path,
    options: InstallOptions,
    progress: Progress,
    made: &mut Made,
) -> Result<(), InstallError> {
    let exe = target.exe();

    progress(0., "Preparing");
    fs::create_dir_all(&target.dir).map_err(step("create the install folder"))?;

    copy_staged(source, &target.staging(), &|done| {
        progress(done * 0.8, "Copying Sonear");
    })?;

    progress(0.84, "Adding the license");

    for (name, bytes) in [(LICENSE_NAME, LICENSE), (NOTICE_NAME, NOTICE)] {
        let path = target.dir.join(name);

        fs::write(&path, bytes).map_err(step("write the license"))?;
        made.files.push(path);
    }

    progress(0.88, "Replacing the program");
    retry_busy(|| fs::rename(target.staging(), &exe)).map_err(step("replace the program"))?;
    made.files.push(exe.clone());

    progress(0.92, "Adding shortcuts");
    write_link(&target.start_menu_link(), &exe)?;
    made.files.push(target.start_menu_link());

    if options.desktop_shortcut && !options.is_update {
        write_link(&target.desktop_link(), &exe)?;
        made.files.push(target.desktop_link());
    }

    progress(0.97, "Registering with Windows");
    register(target, &exe)?;
    made.registered = true;

    progress(1., "Done");
    info!(dir = %target.dir.display(), "installed");

    Ok(())
}

fn write_link(path: &Path, exe: &Path) -> Result<(), InstallError> {
    app_identity::write_shortcut(path, exe, INSTALLED_APP_ID)
        .map_err(|error| InstallError::Shortcut(error.to_string()))
}

/// Copies `source` to `staging` in chunks. `progress` gets a fraction.
fn copy_staged(source: &Path, staging: &Path, progress: &dyn Fn(f32)) -> Result<(), InstallError> {
    let total = fs::metadata(source)
        .map_err(step("read the setup file"))?
        .len();
    let mut input = File::open(source).map_err(step("read the setup file"))?;
    let mut output = File::create(staging).map_err(step("write the program"))?;
    let mut buffer = vec![0u8; COPY_CHUNK];
    let mut done = 0u64;

    loop {
        let read = input
            .read(&mut buffer)
            .map_err(step("read the setup file"))?;

        if read == 0 {
            break;
        }

        output
            .write_all(&buffer[..read])
            .map_err(step("write the program"))?;
        done += read as u64;
        progress(done as f32 / total.max(1) as f32);
    }

    output.sync_all().map_err(step("write the program"))
}

fn register(target: &Target, exe: &Path) -> Result<(), InstallError> {
    let key = target.uninstall_key.as_str();
    let size_kb = fs::metadata(exe)
        .map(|metadata| metadata.len() / 1024)
        .unwrap_or(0)
        .clamp(1, u64::from(u32::MAX)) as u32;
    let exe = exe.display();

    for (name, data) in [
        ("DisplayName", "Sonear".to_owned()),
        ("DisplayVersion", VERSION.to_owned()),
        ("Publisher", PUBLISHER.to_owned()),
        ("DisplayIcon", format!("\"{exe}\",0")),
        ("InstallLocation", target.dir.display().to_string()),
        ("UninstallString", format!("\"{exe}\" --uninstall")),
        ("URLInfoAbout", ABOUT_URL.to_owned()),
    ] {
        protocol::write_string(key, Some(name), &data)?;
    }

    protocol::write_dword(key, "NoModify", 1)?;
    protocol::write_dword(key, "NoRepair", 1)?;
    protocol::write_dword(key, "EstimatedSize", size_kb)?;

    Ok(())
}

fn roll_back(target: &Target, options: InstallOptions, made: &Made) {
    let _ = fs::remove_file(target.staging());

    if options.is_update {
        return;
    }

    for file in &made.files {
        let _ = fs::remove_file(file);
    }

    if made.registered {
        let _ = protocol::delete_tree(&target.uninstall_key);
    }

    let _ = fs::remove_dir(&target.dir);
}

/// Removes the known files, the shortcuts and the installed-apps entry.
/// Never deletes the folder recursively: it may be a shared one. Returns what
/// could not be removed.
pub fn uninstall(target: &Target, progress: Progress) -> Vec<String> {
    let _com = Com::init();
    let mut problems = Vec::new();

    progress(0.1, "Removing the program");

    for name in [EXE_NAME, STAGING_NAME, LICENSE_NAME, NOTICE_NAME] {
        remove_file(&target.dir.join(name), &mut problems);
    }

    let _ = fs::remove_dir(&target.dir);

    progress(0.5, "Removing shortcuts");
    remove_file(&target.start_menu_link(), &mut problems);

    let desktop = target.desktop_link();

    if app_identity::shortcut_target(&desktop).is_some_and(|link| same_path(&link, &target.exe())) {
        remove_file(&desktop, &mut problems);
    }

    progress(0.8, "Removing it from installed apps");

    if let Err(error) = protocol::delete_tree(&target.uninstall_key) {
        problems.push(error.to_string());
    }

    progress(1., "Done");

    problems
}

fn remove_file(path: &Path, problems: &mut Vec<String>) {
    match retry_busy(|| fs::remove_file(path)) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => problems.push(format!("{}: {error}", path.display())),
    }
}

/// The real uninstall: `uninstall`, then the Windows registrations that
/// belong to the installed app, then optionally every piece of local data.
pub fn uninstall_real(dir: PathBuf, delete_data: bool, progress: Progress) -> Vec<String> {
    let target = match Target::real(dir) {
        Ok(target) => target,
        Err(error) => return vec![error.to_string()],
    };
    let mut problems = uninstall(&target, &|fraction, label| progress(fraction * 0.6, label));

    progress(0.7, "Removing Windows registrations");

    if let Err(error) = app_identity::remove() {
        problems.push(error);
    }

    if let Err(error) = protocol::unregister() {
        problems.push(error.to_string());
    }

    if delete_data {
        progress(0.85, "Deleting your data");

        match AppPaths::discover() {
            Some(paths) => problems.extend(delete_local_data(&paths, &KeyringStore)),
            None => problems.push("Could not find your data folder.".to_owned()),
        }
    }

    progress(1., "Done");

    problems
}

/// The install recorded under `key`, ignored unless its program is still there.
/// The demo build never reads the registry for it.
#[cfg_attr(feature = "demo", allow(dead_code))]
pub fn installed_at(key: &str) -> Option<Installed> {
    let dir = PathBuf::from(protocol::read_string(key, "InstallLocation")?);

    if !dir.join(EXE_NAME).is_file() {
        return None;
    }

    Some(Installed {
        dir,
        version: protocol::read_string(key, "DisplayVersion").unwrap_or_default(),
    })
}

/// Compares dotted versions as numbers, so `0.10.0` is newer than `0.9.0`.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let numbers = |version: &str| -> Vec<u64> {
        version
            .split('.')
            .map(|part| part.trim().parse().unwrap_or(0))
            .collect()
    };
    let (mut a, mut b) = (numbers(a), numbers(b));
    let length = a.len().max(b.len());

    a.resize(length, 0);
    b.resize(length, 0);

    a.cmp(&b)
}

fn explain_probe_error(error: &io::Error) -> String {
    if error.kind() == io::ErrorKind::PermissionDenied {
        "You can't write here without administrator rights. Pick a folder in your user profile."
            .to_owned()
    } else {
        format!("Sonear can't write to this folder: {error}")
    }
}

/// Whether Sonear can be installed into `dir`, as a message to show if not.
/// Creates and removes a probe file to find out.
pub fn check_install_dir(dir: &Path, data_root: &Path, config_dir: &Path) -> Result<(), String> {
    if !dir.is_absolute() {
        return Err("Enter a full path, such as C:\\Users\\you\\Sonear.".to_owned());
    }

    if dir.parent().is_none() {
        return Err("Pick a folder, not a whole drive.".to_owned());
    }

    if is_within(dir, data_root) || is_within(dir, config_dir) {
        return Err("Pick a folder outside Sonear's data folders.".to_owned());
    }

    if has_other_files(dir) {
        return Err(
            "This folder already has files in it. Pick an empty one, or add \\Sonear to the path."
                .to_owned(),
        );
    }

    probe_writable(dir).map_err(|error| explain_probe_error(&error))
}

/// Installing writes `LICENSE` and `NOTICE`, which would replace a project's own.
fn has_other_files(dir: &Path) -> bool {
    let has_files = fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some());

    has_files && !dir.join(EXE_NAME).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A registry key under `Software\SonearTest` that is deleted when it drops.
    struct TestKey(String);

    impl Drop for TestKey {
        fn drop(&mut self) {
            let _ = protocol::delete_tree(&self.0);
        }
    }

    fn setup(root: &Path, line: u32) -> (Target, TestKey) {
        let key = format!(r"Software\SonearTest\{}-{line}", std::process::id());
        let target = Target {
            dir: root.join("Programs").join("Sonear"),
            programs_dir: root.join("StartMenu"),
            desktop_dir: root.join("Desktop"),
            uninstall_key: key.clone(),
        };

        fs::create_dir_all(&target.programs_dir).unwrap();
        fs::create_dir_all(&target.desktop_dir).unwrap();

        (target, TestKey(key))
    }

    fn source(root: &Path, bytes: &[u8]) -> PathBuf {
        let path = root.join("setup.exe");

        fs::write(&path, bytes).unwrap();

        path
    }

    const FRESH: InstallOptions = InstallOptions {
        desktop_shortcut: true,
        is_update: false,
    };
    const UPDATE: InstallOptions = InstallOptions {
        desktop_shortcut: false,
        is_update: true,
    };

    fn quiet(_: f32, _: &'static str) {}

    #[test]
    fn a_fresh_install_writes_everything() {
        let root = tempfile::tempdir().unwrap();
        let (target, _key) = setup(root.path(), line!());
        let setup = source(root.path(), &vec![9u8; 700_000]);
        let steps = std::cell::RefCell::new(Vec::new());

        install(&target, &setup, FRESH, &|fraction, _| {
            steps.borrow_mut().push(fraction);
        })
        .unwrap();

        let exe = target.exe();

        assert_eq!(fs::read(&exe).unwrap().len(), 700_000);
        assert!(!target.staging().exists());
        assert!(!fs::read(target.dir.join(LICENSE_NAME)).unwrap().is_empty());
        assert!(!fs::read(target.dir.join(NOTICE_NAME)).unwrap().is_empty());
        assert_eq!(
            app_identity::shortcut_target(&target.start_menu_link()),
            Some(exe.clone())
        );
        assert_eq!(
            app_identity::shortcut_id(&target.start_menu_link()).as_deref(),
            Some(INSTALLED_APP_ID)
        );
        assert!(target.desktop_link().exists());
        assert_eq!(steps.borrow().last(), Some(&1.));
        assert!(steps.borrow().windows(2).all(|pair| pair[0] <= pair[1]));

        let key = &target.uninstall_key;
        let read = |name| protocol::read_string(key, name);

        assert_eq!(read("DisplayName").as_deref(), Some("Sonear"));
        assert_eq!(read("DisplayVersion").as_deref(), Some(VERSION));
        assert_eq!(read("Publisher").as_deref(), Some("Hypox Studios"));
        assert_eq!(
            read("UninstallString"),
            Some(format!("\"{}\" --uninstall", exe.display()))
        );
        assert_eq!(
            read("DisplayIcon"),
            Some(format!("\"{}\",0", exe.display()))
        );
        assert_eq!(
            installed_at(key),
            Some(Installed {
                dir: target.dir.clone(),
                version: VERSION.to_owned()
            })
        );
    }

    #[test]
    fn an_install_whose_program_is_gone_is_not_listed() {
        let root = tempfile::tempdir().unwrap();
        let (target, _key) = setup(root.path(), line!());

        install(&target, &source(root.path(), b"exe"), FRESH, &quiet).unwrap();
        fs::remove_file(target.exe()).unwrap();

        assert_eq!(installed_at(&target.uninstall_key), None);
        assert_eq!(installed_at(r"Software\SonearTest\missing"), None);
    }

    #[test]
    fn a_failed_fresh_install_leaves_nothing() {
        let root = tempfile::tempdir().unwrap();
        let (target, _key) = setup(root.path(), line!());

        let result = install(&target, &root.path().join("missing.exe"), FRESH, &quiet);

        assert!(result.is_err());
        assert!(!target.dir.exists());
        assert!(!target.start_menu_link().exists());
        assert_eq!(installed_at(&target.uninstall_key), None);
    }

    #[test]
    fn a_failed_update_keeps_the_old_program() {
        let root = tempfile::tempdir().unwrap();
        let (target, _key) = setup(root.path(), line!());

        install(&target, &source(root.path(), b"old"), FRESH, &quiet).unwrap();

        let result = install(&target, &root.path().join("missing.exe"), UPDATE, &quiet);

        assert!(result.is_err());
        assert_eq!(fs::read(target.exe()).unwrap(), b"old");
        assert!(!target.staging().exists());
        assert!(installed_at(&target.uninstall_key).is_some());
    }

    #[test]
    fn an_update_replaces_the_program_and_leaves_the_desktop_link() {
        let root = tempfile::tempdir().unwrap();
        let (target, _key) = setup(root.path(), line!());

        install(&target, &source(root.path(), b"old"), FRESH, &quiet).unwrap();
        fs::remove_file(target.desktop_link()).unwrap();
        install(&target, &source(root.path(), b"new!"), UPDATE, &quiet).unwrap();

        assert_eq!(fs::read(target.exe()).unwrap(), b"new!");
        assert!(!target.desktop_link().exists());
    }

    #[test]
    fn uninstalling_removes_only_what_was_installed() {
        let root = tempfile::tempdir().unwrap();
        let (target, _key) = setup(root.path(), line!());

        install(&target, &source(root.path(), b"exe"), FRESH, &quiet).unwrap();
        fs::write(target.dir.join("mine.txt"), b"keep").unwrap();

        assert!(uninstall(&target, &quiet).is_empty());
        assert!(!target.exe().exists());
        assert!(!target.dir.join(LICENSE_NAME).exists());
        assert!(!target.start_menu_link().exists());
        assert!(!target.desktop_link().exists());
        assert_eq!(fs::read(target.dir.join("mine.txt")).unwrap(), b"keep");
        assert_eq!(installed_at(&target.uninstall_key), None);
        assert_eq!(
            protocol::read_string(&target.uninstall_key, "DisplayName"),
            None
        );
    }

    #[test]
    fn an_empty_install_folder_is_removed_with_the_program() {
        let root = tempfile::tempdir().unwrap();
        let (target, _key) = setup(root.path(), line!());

        install(&target, &source(root.path(), b"exe"), FRESH, &quiet).unwrap();
        uninstall(&target, &quiet);

        assert!(!target.dir.exists());
    }

    #[test]
    fn a_desktop_link_to_another_program_stays() {
        let root = tempfile::tempdir().unwrap();
        let (target, _key) = setup(root.path(), line!());
        let other = root.path().join("Other").join(EXE_NAME);

        install(
            &target,
            &source(root.path(), b"exe"),
            UPDATE_NO_DESKTOP,
            &quiet,
        )
        .unwrap();

        let _com = Com::init();

        write_link(&target.desktop_link(), &other).unwrap();
        uninstall(&target, &quiet);

        assert!(target.desktop_link().exists());
    }

    const UPDATE_NO_DESKTOP: InstallOptions = InstallOptions {
        desktop_shortcut: false,
        is_update: false,
    };

    #[test]
    fn versions_compare_as_numbers() {
        assert_eq!(compare_versions("0.9.0", "0.10.0"), Ordering::Less);
        assert_eq!(compare_versions("0.2.0", "0.1.0"), Ordering::Greater);
        assert_eq!(compare_versions("1.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("", "0.1.0"), Ordering::Less);
    }

    #[test]
    fn install_folders_are_checked() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("Sonear");
        let config = root.path().join("Config");
        let check = |dir: &Path| check_install_dir(dir, &data, &config);

        assert!(check(Path::new("relative\\Sonear")).is_err());
        assert!(check(Path::new("C:\\")).is_err());
        assert!(check(&data).is_err());
        assert!(check(&data.join("app")).is_err());
        assert!(check(&config).is_err());

        let fine = root.path().join("Programs").join("Sonear");

        assert!(check(&fine).is_ok());
        assert!(!root.path().join("Programs").exists());

        fs::create_dir_all(&fine).unwrap();

        assert!(check(&fine).is_ok());

        fs::write(fine.join("LICENSE"), b"theirs").unwrap();

        assert!(check(&fine).is_err());
        assert_eq!(fs::read(fine.join("LICENSE")).unwrap(), b"theirs");

        fs::write(fine.join(EXE_NAME), b"exe").unwrap();

        assert!(check(&fine).is_ok());
    }

    #[test]
    fn a_denied_write_explains_administrator_rights() {
        let denied = io::Error::from(io::ErrorKind::PermissionDenied);

        assert!(explain_probe_error(&denied).contains("administrator rights"));
        assert!(explain_probe_error(&io::Error::other("disk full")).contains("disk full"));
    }
}
