use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::{PCWSTR, w};

/// Holds the named mutex for the life of the process.
pub struct InstanceGuard(#[allow(dead_code)] HANDLE);

/// Returns `None` if another Sonear already holds the mutex. The demo uses its
/// own name so it can run next to a real install.
pub fn acquire(demo: bool) -> Option<InstanceGuard> {
    let name: PCWSTR = if demo {
        w!(r"Local\SonearSingleInstanceDemo")
    } else {
        w!(r"Local\SonearSingleInstance")
    };

    // SAFETY: plain Win32 calls; the handle lives until the process exits.
    unsafe {
        let handle = CreateMutexW(None, true, name).ok()?;

        if GetLastError() == ERROR_ALREADY_EXISTS {
            return None;
        }

        Some(InstanceGuard(handle))
    }
}
