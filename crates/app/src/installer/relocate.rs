//! `--uninstall` runs from the install folder, so it cannot delete itself.
//! It copies itself to the temp folder, keeps that copy open with
//! delete-on-close, and starts it with the open handle inherited. The copy owns
//! the wizard and Windows deletes it when the last handle to it closes.

use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use rust_i18n::t;
use thiserror::Error;
use windows::Win32::Foundation::{GENERIC_READ, HANDLE};
use windows::Win32::Security::SECURITY_ATTRIBUTES;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, DELETE, FILE_FLAG_DELETE_ON_CLOSE, FILE_SHARE_DELETE, FILE_SHARE_READ,
    OPEN_EXISTING,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::core::PCWSTR;

use crate::platform_window;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Error)]
pub enum RelocateError {
    #[error("Could not copy the uninstaller to the temp folder: {0}")]
    Copy(std::io::Error),
    #[error("Could not open the copied uninstaller: {0}")]
    Open(windows::core::Error),
    #[error("Could not start the copied uninstaller: {0}")]
    Spawn(std::io::Error),
}

impl RelocateError {
    pub fn user_message(&self) -> String {
        match self {
            Self::Copy(error) => t!("installer.relocate.copy", detail = error.to_string()),
            Self::Open(error) => t!("installer.relocate.open", detail = error.to_string()),
            Self::Spawn(error) => t!("installer.relocate.spawn", detail = error.to_string()),
        }
        .into_owned()
    }
}

pub fn temp_copy_path(pid: u32) -> PathBuf {
    std::env::temp_dir().join(format!("sonear-uninstall-{pid}.exe"))
}

/// Opens `path` so it is deleted once every handle to it is closed. The
/// handle can be inherited by child processes.
fn open_delete_on_close(path: &Path) -> windows::core::Result<HANDLE> {
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: true.into(),
    };

    // SAFETY: `wide` is nul-terminated and outlives the call; `attributes` is valid for it.
    unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            GENERIC_READ.0 | DELETE.0,
            FILE_SHARE_READ | FILE_SHARE_DELETE,
            Some(&attributes),
            OPEN_EXISTING,
            FILE_FLAG_DELETE_ON_CLOSE,
            None,
        )
    }
}

/// Starts a temp copy of this program that uninstalls the copy in `install_dir`.
/// The caller exits right after; the copy waits for it. The handle that keeps the
/// copy deletable is left open on purpose, for the copy to inherit.
pub fn start_from_temp(install_dir: &Path) -> Result<(), RelocateError> {
    let exe = std::env::current_exe().map_err(RelocateError::Copy)?;
    // SAFETY: plain Win32 call without arguments.
    let pid = unsafe { GetCurrentProcessId() };
    let copy = temp_copy_path(pid);

    std::fs::copy(&exe, &copy).map_err(RelocateError::Copy)?;

    if let Err(error) = open_delete_on_close(&copy) {
        let _ = std::fs::remove_file(&copy);

        return Err(RelocateError::Open(error));
    }

    let started = Command::new(&copy)
        .arg("--after")
        .arg(pid.to_string())
        .arg("--uninstall-from")
        .arg(install_dir)
        .spawn();

    match started {
        Ok(child) => {
            platform_window::allow_foreground(child.id());

            Ok(())
        }
        Err(error) => {
            let _ = std::fs::remove_file(&copy);

            Err(RelocateError::Spawn(error))
        }
    }
}

/// Called by the temp copy as it is about to exit. A short-lived hidden process
/// inherits the delete-on-close handle and outlives this one, because the
/// copy's image is still mapped while its own handles close.
pub fn keep_handle_alive_briefly() {
    let _ = Command::new("cmd")
        .raw_arg("/c ping -n 3 127.0.0.1 >nul")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use windows::Win32::Foundation::CloseHandle;

    use super::*;

    #[test]
    fn the_copy_lives_in_the_temp_folder() {
        let path = temp_copy_path(42);

        assert_eq!(path.parent(), Some(std::env::temp_dir().as_path()));
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("sonear-uninstall-42.exe")
        );
    }

    #[test]
    fn a_file_opened_for_delete_on_close_goes_when_the_handle_does() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("copy.exe");

        std::fs::write(&path, b"x").unwrap();

        let handle = open_delete_on_close(&path).unwrap();

        assert!(path.exists());

        // SAFETY: the handle was just opened and is not used again.
        unsafe { CloseHandle(handle).unwrap() };

        assert!(!path.exists());
    }
}
