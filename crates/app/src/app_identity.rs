//! The AppUserModelID Windows groups Sonear's windows, media session and
//! notifications under, and the Start Menu shortcut that gives it a name.
//! Without the shortcut Windows shows the media overlay as "Unknown application".

use std::path::{Path, PathBuf};

use tracing::warn;
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0, PropVariantClear,
    PropVariantToStringAlloc,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree, IPersistFile, STGM_READ,
};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    FOLDERID_Programs, IShellLinkW, KF_FLAG_DEFAULT, SHGetKnownFolderPath, ShellLink,
};
use windows::core::{HSTRING, Interface, PCWSTR, PWSTR};

#[cfg(not(feature = "demo"))]
pub const APP_ID: &str = "HypoxStudios.Sonear";
#[cfg(feature = "demo")]
pub const APP_ID: &str = "HypoxStudios.Sonear.Demo";
pub const APP_NAME: &str = "Sonear";

const SHORTCUT_NAME: &str = "Sonear.lnk";
const REGISTRY_ROOT: &str = r"Software\Classes\AppUserModelId";
const DEMO: bool = cfg!(feature = "demo");

fn registry_key() -> String {
    format!(r"{REGISTRY_ROOT}\{APP_ID}")
}

fn shortcut_path() -> Result<PathBuf, String> {
    // SAFETY: the returned string is copied and then freed with `CoTaskMemFree`.
    unsafe {
        let folder = SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_DEFAULT, None)
            .map_err(|error| error.to_string())?;
        let path = folder.to_string();

        CoTaskMemFree(Some(folder.0.cast()));

        path.map(|folder| PathBuf::from(folder).join(SHORTCUT_NAME))
            .map_err(|error| error.to_string())
    }
}

/// Whether the shortcut has to be (re)written.
fn needs_write(target: Option<&Path>, id: Option<&str>, exe: &Path, app_id: &str) -> bool {
    let Some(target) = target else {
        return true;
    };

    let same_target = target
        .to_string_lossy()
        .eq_ignore_ascii_case(&exe.to_string_lossy());

    !same_target || id != Some(app_id)
}

/// Makes sure the Start Menu shortcut and the notification registration exist.
/// Never runs for the demo build, so it does not claim a Start Menu entry.
pub fn install() {
    if DEMO {
        return;
    }

    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            warn!(%error, "could not find the Sonear program for its Start Menu shortcut");

            return;
        }
    };

    match shortcut_path() {
        Ok(path) => {
            let target = shortcut_target(&path);
            let id = shortcut_id(&path);

            if needs_write(target.as_deref(), id.as_deref(), &exe, APP_ID)
                && let Err(error) = write_shortcut(&path, &exe, APP_ID)
            {
                warn!(%error, "could not write the Start Menu shortcut");
            }
        }
        Err(error) => warn!(%error, "could not find the Start Menu folder"),
    }

    if let Err(error) =
        crate::protocol::write_string(&registry_key(), Some("DisplayName"), APP_NAME)
    {
        warn!(%error, "could not register the Sonear name with Windows");
    }
}

/// Deletes what `install` adds. Missing is fine.
pub fn remove() -> Result<(), String> {
    let mut problems = Vec::new();

    match shortcut_path() {
        Ok(path) => {
            if let Err(error) = std::fs::remove_file(&path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                problems.push(error.to_string());
            }
        }
        Err(error) => problems.push(error),
    }

    if let Err(error) = crate::protocol::delete_tree(&registry_key()) {
        problems.push(error.to_string());
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join(", "))
    }
}

/// A `VT_LPWSTR` variant that borrows `text`. Wrap it in `ManuallyDrop`: dropping a
/// variant frees its string, which `text` owns.
fn lpwstr_variant(text: &mut Vec<u16>) -> PROPVARIANT {
    text.push(0);

    PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_LPWSTR,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    pwszVal: PWSTR(text.as_mut_ptr()),
                },
            }),
        },
    }
}

/// Writes a shortcut to `exe` at `path` that carries the AppUserModelID `id`.
/// COM must be initialised on the calling thread.
pub fn write_shortcut(path: &Path, exe: &Path, id: &str) -> windows::core::Result<()> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // SAFETY: COM is initialised by the caller and every interface stays on this thread. The
    // variant only borrows `text`, which outlives `SetValue` (the store copies it).
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;

        link.SetPath(&HSTRING::from(exe.as_os_str()))?;
        link.SetDescription(&HSTRING::from(APP_NAME))?;

        let store: IPropertyStore = link.cast()?;
        let mut text: Vec<u16> = id.encode_utf16().collect();
        let variant = std::mem::ManuallyDrop::new(lpwstr_variant(&mut text));

        store.SetValue(&PKEY_AppUserModel_ID, &*variant)?;
        store.Commit()?;

        let file: IPersistFile = link.cast()?;

        file.Save(&HSTRING::from(path.as_os_str()), true)
    }
}

fn load(path: &Path) -> Option<IShellLinkW> {
    if !path.exists() {
        return None;
    }

    // SAFETY: COM is initialised by the caller; the interfaces stay on this thread.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let file: IPersistFile = link.cast().ok()?;

        file.Load(&HSTRING::from(path.as_os_str()), STGM_READ)
            .ok()?;

        Some(link)
    }
}

/// The program an existing shortcut points at.
pub fn shortcut_target(path: &Path) -> Option<PathBuf> {
    let link = load(path)?;
    let mut buffer = [0u16; 1024];

    // SAFETY: `buffer` is valid for the call; no find data is requested.
    unsafe {
        link.GetPath(&mut buffer, std::ptr::null_mut(), 0).ok()?;
    }

    let length = buffer.iter().position(|unit| *unit == 0)?;

    Some(PathBuf::from(String::from_utf16_lossy(&buffer[..length])))
}

/// The AppUserModelID stored in an existing shortcut.
pub fn shortcut_id(path: &Path) -> Option<String> {
    let link = load(path)?;

    // SAFETY: the variant from `GetValue` is read, then cleared; the string from
    // `PropVariantToStringAlloc` is copied and freed with `CoTaskMemFree`.
    unsafe {
        let store: IPropertyStore = link.cast().ok()?;
        let mut variant = store.GetValue(&PKEY_AppUserModel_ID).ok()?;
        let id = PropVariantToStringAlloc(&variant).ok().and_then(|text| {
            let id = PCWSTR(text.0).to_string().ok();

            CoTaskMemFree(Some(text.0.cast()));

            id
        });

        let _ = PropVariantClear(&mut variant);

        id.filter(|id| !id.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};

    use super::*;

    #[test]
    fn a_missing_shortcut_is_written() {
        assert!(needs_write(None, None, Path::new(r"C:\a.exe"), "id"));
    }

    #[test]
    fn a_matching_shortcut_is_left_alone() {
        let exe = Path::new(r"C:\Apps\sonear.exe");

        assert!(!needs_write(
            Some(Path::new(r"c:\apps\Sonear.exe")),
            Some("id"),
            exe,
            "id"
        ));
    }

    #[test]
    fn a_shortcut_to_another_copy_or_id_is_rewritten() {
        let exe = Path::new(r"C:\Apps\sonear.exe");

        assert!(needs_write(
            Some(Path::new(r"C:\Old\sonear.exe")),
            Some("id"),
            exe,
            "id"
        ));
        assert!(needs_write(Some(exe), Some("other"), exe, "id"));
        assert!(needs_write(Some(exe), None, exe, "id"));
    }

    #[test]
    fn a_written_shortcut_reads_back() {
        // SAFETY: plain COM initialisation for this test thread.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }

        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("Test.lnk");
        let exe = std::env::current_exe().expect("test exe");

        write_shortcut(&path, &exe, "HypoxStudios.Test").expect("shortcut is written");

        assert_eq!(shortcut_target(&path), Some(exe));
        assert_eq!(shortcut_id(&path).as_deref(), Some("HypoxStudios.Test"));
    }
}
