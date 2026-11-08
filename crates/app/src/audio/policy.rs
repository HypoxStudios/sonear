//! Per-application output device, through the undocumented WinRT factory
//! behind Windows Settings > Volume mixer. Windows stores the choice per
//! executable path, so it survives restarts of the process.

use std::ffi::c_void;

use thiserror::Error;
use windows::Win32::Media::Audio::{EDataFlow, ERole, eConsole, eMultimedia, eRender};
use windows::Win32::System::WinRT::RoGetActivationFactory;
use windows::core::{HRESULT, HSTRING};
use windows_core::interface;

const CLASS_NAME: &str = "Windows.Media.Internal.AudioPolicyConfig";
const DEVICE_PREFIX: &str = r"\\?\SWD#MMDEVAPI#";
const RENDER_SUFFIX: &str = "#{e6327cad-dcec-4949-ae8a-991e976a79d2}";

/// Layout shared by the Windows 10 21H2 and Windows 11 factories. These are
/// IInspectable interfaces, declared on IUnknown with its three extra slots.
/// Only the slots are declared: the 19 methods before the endpoint ones are never called.
macro_rules! policy_interface {
    ($name:ident, $iid:literal) => {
        #[interface($iid)]
        unsafe trait $name: windows_core::IUnknown {
            fn inspectable_iids(&self) -> HRESULT;
            fn inspectable_class_name(&self) -> HRESULT;
            fn inspectable_trust_level(&self) -> HRESULT;
            fn unused_0(&self) -> HRESULT;
            fn unused_1(&self) -> HRESULT;
            fn unused_2(&self) -> HRESULT;
            fn unused_3(&self) -> HRESULT;
            fn unused_4(&self) -> HRESULT;
            fn unused_5(&self) -> HRESULT;
            fn unused_6(&self) -> HRESULT;
            fn unused_7(&self) -> HRESULT;
            fn unused_8(&self) -> HRESULT;
            fn unused_9(&self) -> HRESULT;
            fn unused_10(&self) -> HRESULT;
            fn unused_11(&self) -> HRESULT;
            fn unused_12(&self) -> HRESULT;
            fn unused_13(&self) -> HRESULT;
            fn unused_14(&self) -> HRESULT;
            fn unused_15(&self) -> HRESULT;
            fn unused_16(&self) -> HRESULT;
            fn unused_17(&self) -> HRESULT;
            fn unused_18(&self) -> HRESULT;
            fn set_persisted_default_audio_endpoint(
                &self,
                process_id: u32,
                flow: EDataFlow,
                role: ERole,
                device_id: *mut c_void,
            ) -> HRESULT;
            fn get_persisted_default_audio_endpoint(
                &self,
                process_id: u32,
                flow: EDataFlow,
                role: ERole,
                device_id: *mut *mut c_void,
            ) -> HRESULT;
        }
    };
}

policy_interface!(
    IAudioPolicyConfigFactory,
    "ab3d4648-e242-459f-b02f-541c70306324"
);
policy_interface!(
    IAudioPolicyConfigFactoryLegacy,
    "2a59116d-6c4f-45e0-a74f-707e3fef9258"
);

#[derive(Debug, Error)]
pub enum PolicyError {
    #[error("per-app output devices are not available on this Windows")]
    Unavailable,
    #[error("Windows refused the change: {0}")]
    Windows(#[from] windows::core::Error),
}

enum Factory {
    Current(IAudioPolicyConfigFactory),
    Legacy(IAudioPolicyConfigFactoryLegacy),
}

pub struct Policy {
    factory: Factory,
}

impl Policy {
    /// Opens the factory. COM must be initialised on the calling thread.
    pub fn open() -> Result<Self, PolicyError> {
        let class = HSTRING::from(CLASS_NAME);

        // SAFETY: plain activation call with a valid class name.
        let factory = unsafe {
            RoGetActivationFactory::<IAudioPolicyConfigFactory>(&class)
                .map(Factory::Current)
                .or_else(|_| {
                    RoGetActivationFactory::<IAudioPolicyConfigFactoryLegacy>(&class)
                        .map(Factory::Legacy)
                })
        }
        .map_err(|_| PolicyError::Unavailable)?;

        Ok(Self { factory })
    }

    /// Sets the output device of process `pid`, or clears it with `None`.
    pub fn set(&self, pid: u32, device: Option<&str>) -> Result<(), PolicyError> {
        let wrapped = device.map(|id| HSTRING::from(wrap_device_id(id)));

        for role in [eConsole, eMultimedia] {
            // SAFETY: the HSTRING outlives the call, and a null pointer clears the setting.
            let result = unsafe {
                let raw = wrapped
                    .as_ref()
                    .map_or(std::ptr::null_mut(), |text| std::mem::transmute_copy(text));

                match &self.factory {
                    Factory::Current(factory) => {
                        factory.set_persisted_default_audio_endpoint(pid, eRender, role, raw)
                    }
                    Factory::Legacy(factory) => {
                        factory.set_persisted_default_audio_endpoint(pid, eRender, role, raw)
                    }
                }
            };

            result.ok()?;
        }

        Ok(())
    }

    /// The bare device ID set for process `pid`, or `None` for the Windows default.
    pub fn get(&self, pid: u32) -> Result<Option<String>, PolicyError> {
        let mut raw: *mut c_void = std::ptr::null_mut();

        // SAFETY: `raw` receives an HSTRING that is taken over right below.
        let result = unsafe {
            match &self.factory {
                Factory::Current(factory) => factory.get_persisted_default_audio_endpoint(
                    pid,
                    eRender,
                    eMultimedia,
                    &mut raw,
                ),
                Factory::Legacy(factory) => factory.get_persisted_default_audio_endpoint(
                    pid,
                    eRender,
                    eMultimedia,
                    &mut raw,
                ),
            }
        };

        // SAFETY: a non-null result is an owned HSTRING, released when `text` drops.
        let text: HSTRING = unsafe { std::mem::transmute::<*mut c_void, HSTRING>(raw) };

        result.ok()?;

        Ok(unwrap_device_id(&text.to_string_lossy()).map(str::to_owned))
    }
}

/// The ID Windows stores for an `IMMDevice` ID.
pub fn wrap_device_id(id: &str) -> String {
    format!("{DEVICE_PREFIX}{id}{RENDER_SUFFIX}")
}

/// The `IMMDevice` ID inside a stored ID, matching the fixed parts in any case.
/// `None` for an empty string, which is how Windows reports "default".
pub fn unwrap_device_id(stored: &str) -> Option<&str> {
    if stored.is_empty() {
        return None;
    }

    let rest = match stored.get(..DEVICE_PREFIX.len()) {
        Some(head) if head.eq_ignore_ascii_case(DEVICE_PREFIX) => &stored[DEVICE_PREFIX.len()..],
        _ => stored,
    };
    let cut = rest.len().checked_sub(RENDER_SUFFIX.len());
    let id = match cut.and_then(|cut| rest.get(cut..).map(|tail| (cut, tail))) {
        Some((cut, tail)) if tail.eq_ignore_ascii_case(RENDER_SUFFIX) => &rest[..cut],
        _ => rest,
    };

    (!id.is_empty()).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "{0.0.0.00000000}.{8d776c5f-1111-2222-3333-444444444444}";

    #[test]
    fn wrapping_adds_the_prefix_and_render_suffix() {
        assert_eq!(
            wrap_device_id(ID),
            format!(r"\\?\SWD#MMDEVAPI#{ID}#{{e6327cad-dcec-4949-ae8a-991e976a79d2}}")
        );
    }

    #[test]
    fn unwrapping_undoes_wrapping() {
        assert_eq!(unwrap_device_id(&wrap_device_id(ID)), Some(ID));
    }

    #[test]
    fn unwrapping_ignores_case_of_the_fixed_parts() {
        let stored = wrap_device_id(ID).to_uppercase();

        assert_eq!(unwrap_device_id(&stored), Some(ID.to_uppercase().as_str()));
    }

    #[test]
    fn a_bare_id_is_kept_and_empty_means_default() {
        assert_eq!(unwrap_device_id(ID), Some(ID));
        assert_eq!(unwrap_device_id(""), None);
    }

    #[test]
    fn short_or_odd_text_does_not_panic() {
        assert_eq!(unwrap_device_id("é"), Some("é"));
        assert_eq!(unwrap_device_id(r"\\?\SWD#MMDEVAPI#"), None);
    }
}
