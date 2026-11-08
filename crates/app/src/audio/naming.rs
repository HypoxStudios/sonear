//! Names Soloist's audio sessions after Sonear in the Windows volume mixer.
//! The audio comes from the supervised `soloist.exe`, so the mixer would
//! otherwise list it as "Soloist" with Soloist's icon.

use std::path::{Path, PathBuf};

use sonear_core::data_dir::same_place;
use tracing::debug;
use windows::Win32::Foundation::{CloseHandle, HANDLE, S_OK};
use windows::Win32::Media::Audio::{IAudioSessionControl, IAudioSessionControl2};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::core::{GUID, HSTRING, Interface, PWSTR};

use crate::app_identity::APP_NAME;

/// Everything a session callback needs to recognise and name Soloist.
#[derive(Clone)]
pub struct Naming {
    pub soloist_exe: PathBuf,
    pub icon: Option<HSTRING>,
}

impl Naming {
    pub fn new(soloist_exe: PathBuf, icon: Option<HSTRING>) -> Self {
        Self { soloist_exe, icon }
    }
}

/// Names the session when it is Soloist's. Returns whether it is.
pub fn name_session(session: &IAudioSessionControl, naming: &Naming) -> bool {
    let Ok(session2) = session.cast::<IAudioSessionControl2>() else {
        return false;
    };

    // SAFETY: plain calls on a live session object.
    let pid = unsafe {
        // S_FALSE means "not the system sounds session", and it counts as success too.
        if session2.IsSystemSoundsSession() == S_OK {
            return false;
        }

        match session2.GetProcessId() {
            Ok(pid) if pid != 0 => pid,
            _ => return false,
        }
    };

    let Some(image) = process_image(pid) else {
        return false;
    };

    if !is_soloist(&image, &naming.soloist_exe) {
        return false;
    }

    // SAFETY: the returned string is copied and then freed with `CoTaskMemFree`.
    let current = unsafe {
        session.GetDisplayName().ok().map(|name| {
            let text = name.to_string().unwrap_or_default();

            CoTaskMemFree(Some(name.0.cast()));

            text
        })
    };

    if current.as_deref() == Some(APP_NAME) {
        return true;
    }

    let name = HSTRING::from(APP_NAME);

    // SAFETY: the strings outlive the calls and a zero GUID means no event context.
    unsafe {
        if let Err(error) = session.SetDisplayName(&name, &GUID::zeroed()) {
            debug!(%error, pid, "could not name an audio session");
            return true;
        }

        if let Some(icon) = &naming.icon
            && let Err(error) = session.SetIconPath(icon, &GUID::zeroed())
        {
            debug!(%error, pid, "could not set an audio session's icon");
        }
    }

    debug!(pid, "named a Soloist audio session");

    true
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: the handle came from `OpenProcess` and is closed exactly once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn process_image(pid: u32) -> Option<PathBuf> {
    // SAFETY: the handle is closed by the guard, and the buffer length is passed in `size`.
    unsafe {
        let process = OwnedHandle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?);
        let mut buffer = vec![0u16; 1024];
        let mut size = buffer.len() as u32;

        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )
        .ok()?;

        Some(PathBuf::from(String::from_utf16_lossy(
            &buffer[..size as usize],
        )))
    }
}

fn is_soloist(image: &Path, soloist_exe: &Path) -> bool {
    same_place(image, soloist_exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_same_path_in_any_case() {
        let exe = Path::new(r"C:\Users\Someone\AppData\Local\Sonear\soloist\soloist.exe");
        let image = Path::new(r"c:\users\someone\appdata\local\sonear\SOLOIST\Soloist.exe");

        assert!(is_soloist(image, exe));
    }

    #[test]
    fn ignores_other_programs() {
        let exe = Path::new(r"C:\Users\Someone\AppData\Local\Sonear\soloist\soloist.exe");
        let other = Path::new(r"C:\Program Files\Browser\browser.exe");

        assert!(!is_soloist(other, exe));
    }
}
