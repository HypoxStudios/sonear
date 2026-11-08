//! Names Soloist's audio sessions after Sonear in the Windows volume mixer.
//! The audio comes from the supervised `soloist.exe`, so the mixer would
//! otherwise list it as "Soloist" with Soloist's icon.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

use sonear_core::data_dir::same_place;
use tracing::{debug, warn};
use windows::Win32::Foundation::{CloseHandle, HANDLE, PROPERTYKEY, S_OK};
use windows::Win32::Media::Audio::{
    DEVICE_STATE, DEVICE_STATE_ACTIVE, EDataFlow, ERole, IAudioSessionControl,
    IAudioSessionControl2, IAudioSessionManager2, IAudioSessionNotification,
    IAudioSessionNotification_Impl, IMMDeviceEnumerator, IMMNotificationClient,
    IMMNotificationClient_Impl, MMDeviceEnumerator, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::core::{GUID, HSTRING, Interface, PCWSTR, PWSTR};
use windows_core::implement;

use crate::app_identity::APP_NAME;

/// Names Soloist's audio sessions after Sonear in the Windows volume mixer.
pub fn start(soloist_exe: PathBuf) {
    if cfg!(feature = "demo") {
        return;
    }

    let spawned = std::thread::Builder::new()
        .name("sonear-audio-session".into())
        .spawn(move || run(soloist_exe));

    if let Err(error) = spawned {
        warn!(%error, "could not start the audio session thread");
    }
}

/// Everything a session callback needs to recognise and name Soloist.
#[derive(Clone)]
struct Naming {
    soloist_exe: PathBuf,
    icon: Option<HSTRING>,
}

struct Registration {
    _manager: IAudioSessionManager2,
    _notification: IAudioSessionNotification,
}

#[implement(IMMNotificationClient)]
struct DeviceWatcher {
    rescan: Sender<()>,
}

impl DeviceWatcher {
    // Registering anything from inside these callbacks can deadlock, so they only signal the thread.
    fn signal(&self) -> windows::core::Result<()> {
        let _ = self.rescan.send(());

        Ok(())
    }
}

impl IMMNotificationClient_Impl for DeviceWatcher_Impl {
    fn OnDeviceStateChanged(
        &self,
        _id: &PCWSTR,
        _state: DEVICE_STATE,
    ) -> windows::core::Result<()> {
        self.signal()
    }

    fn OnDeviceAdded(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        self.signal()
    }

    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnDefaultDeviceChanged(
        &self,
        _flow: EDataFlow,
        _role: ERole,
        _id: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.signal()
    }

    fn OnPropertyValueChanged(
        &self,
        _id: &PCWSTR,
        _key: &PROPERTYKEY,
    ) -> windows::core::Result<()> {
        Ok(())
    }
}

#[implement(IAudioSessionNotification)]
struct SessionWatcher {
    naming: Naming,
}

impl IAudioSessionNotification_Impl for SessionWatcher_Impl {
    fn OnSessionCreated(
        &self,
        new_session: windows_core::Ref<IAudioSessionControl>,
    ) -> windows::core::Result<()> {
        if let Some(session) = new_session.as_ref() {
            name_session(session, &self.naming);
        }

        Ok(())
    }
}

fn run(soloist_exe: PathBuf) {
    let icon = match std::env::current_exe() {
        Ok(exe) => Some(HSTRING::from(format!("{},0", exe.display()))),
        Err(error) => {
            warn!(%error, "could not resolve Sonear's icon for the volume mixer");
            None
        }
    };
    let naming = Naming { soloist_exe, icon };

    // MTA, because the audio callbacks arrive on threads COM owns.
    // SAFETY: called once on this thread, which stays alive for the whole process.
    let initialised = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };

    if initialised.is_err() {
        warn!(
            code = initialised.0,
            "could not initialise COM for the audio sessions"
        );
        return;
    }

    let (tx, rx) = mpsc::channel();

    // SAFETY: COM is initialised on this thread.
    let enumerator: IMMDeviceEnumerator =
        match unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) } {
            Ok(enumerator) => enumerator,
            Err(error) => {
                warn!(%error, "could not create the audio device enumerator");
                return;
            }
        };
    let watcher: IMMNotificationClient = DeviceWatcher { rescan: tx }.into();

    // SAFETY: `watcher` is a valid callback object and lives until the process ends.
    if let Err(error) = unsafe { enumerator.RegisterEndpointNotificationCallback(&watcher) } {
        warn!(%error, "could not watch audio devices");
    }

    let mut registered = HashMap::new();

    scan(&enumerator, &naming, &mut registered);
    rescan_on_signal(&rx, &enumerator, &naming, &mut registered);
}

fn rescan_on_signal(
    rx: &Receiver<()>,
    enumerator: &IMMDeviceEnumerator,
    naming: &Naming,
    registered: &mut HashMap<String, Registration>,
) {
    while rx.recv().is_ok() {
        while rx.try_recv().is_ok() {}

        scan(enumerator, naming, registered);
    }
}

fn scan(
    enumerator: &IMMDeviceEnumerator,
    naming: &Naming,
    registered: &mut HashMap<String, Registration>,
) {
    // SAFETY: plain COM calls on an initialised thread; every returned ID string is freed.
    unsafe {
        let devices = match enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) {
            Ok(devices) => devices,
            Err(error) => {
                debug!(%error, "could not list audio devices");
                return;
            }
        };
        let count = devices.GetCount().unwrap_or(0);

        for index in 0..count {
            let Ok(device) = devices.Item(index) else {
                continue;
            };
            let Ok(raw_id) = device.GetId() else {
                continue;
            };
            let id = raw_id.to_string().unwrap_or_default();

            CoTaskMemFree(Some(raw_id.0.cast()));

            if id.is_empty() || registered.contains_key(&id) {
                continue;
            }

            let manager = match device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) {
                Ok(manager) => manager,
                Err(error) => {
                    debug!(%error, "could not open a device's audio sessions");
                    continue;
                }
            };

            if let Some(registration) = register(manager, naming) {
                registered.insert(id, registration);
            }
        }
    }
}

/// # Safety
/// COM must be initialised on the calling thread.
unsafe fn register(manager: IAudioSessionManager2, naming: &Naming) -> Option<Registration> {
    let notification: IAudioSessionNotification = SessionWatcher {
        naming: naming.clone(),
    }
    .into();

    // SAFETY: COM is initialised by the caller, and `notification` is kept alive in the
    // returned registration.
    unsafe {
        // Session notifications only start once the enumerator has been fetched.
        let sessions = manager
            .GetSessionEnumerator()
            .inspect_err(|error| debug!(%error, "could not list a device's audio sessions"))
            .ok()?;
        let count = sessions.GetCount().unwrap_or(0);

        for index in 0..count {
            if let Ok(session) = sessions.GetSession(index) {
                name_session(&session, naming);
            }
        }

        manager
            .RegisterSessionNotification(&notification)
            .inspect_err(|error| debug!(%error, "could not watch a device's audio sessions"))
            .ok()?;
    }

    Some(Registration {
        _manager: manager,
        _notification: notification,
    })
}

fn name_session(session: &IAudioSessionControl, naming: &Naming) {
    let Ok(session2) = session.cast::<IAudioSessionControl2>() else {
        return;
    };

    // SAFETY: plain calls on a live session object.
    let pid = unsafe {
        // S_FALSE means "not the system sounds session", and it counts as success too.
        if session2.IsSystemSoundsSession() == S_OK {
            return;
        }

        match session2.GetProcessId() {
            Ok(pid) if pid != 0 => pid,
            _ => return,
        }
    };

    let Some(image) = process_image(pid) else {
        return;
    };

    if !is_soloist(&image, &naming.soloist_exe) {
        return;
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
        return;
    }

    let name = HSTRING::from(APP_NAME);

    // SAFETY: the strings outlive the calls and a zero GUID means no event context.
    unsafe {
        if let Err(error) = session.SetDisplayName(&name, &GUID::zeroed()) {
            debug!(%error, pid, "could not name an audio session");
            return;
        }

        if let Some(icon) = &naming.icon
            && let Err(error) = session.SetIconPath(icon, &GUID::zeroed())
        {
            debug!(%error, pid, "could not set an audio session's icon");
        }
    }

    debug!(pid, "named a Soloist audio session");
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
