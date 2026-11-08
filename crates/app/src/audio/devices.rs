//! Output devices: what is connected, what Windows uses by default, and what
//! changed between two looks.

use tracing::debug;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    DEVICE_STATE_ACTIVE, IMMDevice, IMMDeviceEnumerator, eMultimedia, eRender,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
use windows::Win32::System::Com::{CoTaskMemFree, STGM_READ};
use windows::Win32::System::Variant::VT_LPWSTR;

use super::OutputDevice;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Diff {
    pub added: Vec<OutputDevice>,
    pub removed: Vec<OutputDevice>,
}

/// Which devices appeared in and vanished from `current` since `previous`, by ID.
pub fn diff(previous: &[OutputDevice], current: &[OutputDevice]) -> Diff {
    let missing_from = |list: &[OutputDevice], device: &OutputDevice| {
        !list.iter().any(|other| other.id == device.id)
    };

    Diff {
        added: current
            .iter()
            .filter(|device| missing_from(previous, device))
            .cloned()
            .collect(),
        removed: previous
            .iter()
            .filter(|device| missing_from(current, device))
            .cloned()
            .collect(),
    }
}

/// Active render endpoints with their friendly names.
///
/// # Safety
/// COM must be initialised on the calling thread.
pub unsafe fn list(enumerator: &IMMDeviceEnumerator) -> Option<Vec<OutputDevice>> {
    // SAFETY: plain COM calls on an initialised thread.
    unsafe {
        let devices = enumerator
            .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
            .inspect_err(|error| debug!(%error, "could not list audio devices"))
            .ok()?;
        let count = devices.GetCount().unwrap_or(0);
        let mut found = Vec::new();

        for index in 0..count {
            let Ok(device) = devices.Item(index) else {
                continue;
            };
            let Some(id) = device_id(&device) else {
                continue;
            };
            let name = friendly_name(&device).unwrap_or_else(|| id.clone());

            found.push(OutputDevice { id, name });
        }

        Some(found)
    }
}

/// ID of the device Windows plays media on by default.
///
/// # Safety
/// COM must be initialised on the calling thread.
pub unsafe fn default_id(enumerator: &IMMDeviceEnumerator) -> Option<String> {
    // SAFETY: plain COM calls on an initialised thread.
    unsafe {
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eMultimedia)
            .ok()?;

        device_id(&device)
    }
}

unsafe fn device_id(device: &IMMDevice) -> Option<String> {
    // SAFETY: the returned string is copied and then freed with `CoTaskMemFree`.
    unsafe {
        let raw = device.GetId().ok()?;
        let id = raw.to_string().ok();

        CoTaskMemFree(Some(raw.0.cast()));

        id.filter(|id| !id.is_empty())
    }
}

unsafe fn friendly_name(device: &IMMDevice) -> Option<String> {
    // SAFETY: the property value is read only when it is a wide string, then cleared.
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let mut value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        let inner = &value.Anonymous.Anonymous;
        let name = (inner.vt == VT_LPWSTR && !inner.Anonymous.pwszVal.is_null())
            .then(|| inner.Anonymous.pwszVal.to_string().ok())
            .flatten()
            .filter(|name| !name.is_empty());

        let _ = PropVariantClear(&mut value);

        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str) -> OutputDevice {
        OutputDevice {
            id: id.into(),
            name: format!("Device {id}"),
        }
    }

    #[test]
    fn nothing_changes_between_equal_scans() {
        let devices = [device("a"), device("b")];

        assert_eq!(diff(&devices, &devices), Diff::default());
    }

    #[test]
    fn new_and_missing_devices_are_found_by_id() {
        let before = [device("a"), device("b")];
        let after = [device("b"), device("c")];

        assert_eq!(
            diff(&before, &after),
            Diff {
                added: vec![device("c")],
                removed: vec![device("a")],
            }
        );
    }

    #[test]
    fn a_rename_is_not_a_new_device() {
        let before = [device("a")];
        let after = [OutputDevice {
            id: "a".into(),
            name: "Renamed".into(),
        }];

        assert_eq!(diff(&before, &after), Diff::default());
    }
}
