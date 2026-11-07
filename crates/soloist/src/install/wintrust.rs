use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::Security::Cryptography::{CERT_NAME_SIMPLE_DISPLAY_TYPE, CertGetNameStringW};
use windows::Win32::Security::WinTrust::{
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
    WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
    WTHelperGetProvCertFromChain, WTHelperGetProvSignerFromChain, WTHelperProvDataFromStateData,
    WinVerifyTrust,
};
use windows::core::PCWSTR;

use super::{InstallError, SignatureVerifier};

/// Verifies embedded Authenticode signatures with `WinVerifyTrust`.
pub struct WinTrustVerifier;

impl SignatureVerifier for WinTrustVerifier {
    fn verify(&self, exe: &Path) -> Result<String, InstallError> {
        let wide: Vec<u16> = exe
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut file = WINTRUST_FILE_INFO {
            cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
            pcwszFilePath: PCWSTR(wide.as_ptr()),
            ..Default::default()
        };
        let mut data = WINTRUST_DATA {
            cbStruct: size_of::<WINTRUST_DATA>() as u32,
            dwUIChoice: WTD_UI_NONE,
            fdwRevocationChecks: WTD_REVOKE_NONE,
            dwUnionChoice: WTD_CHOICE_FILE,
            Anonymous: WINTRUST_DATA_0 { pFile: &mut file },
            dwStateAction: WTD_STATEACTION_VERIFY,
            ..Default::default()
        };
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;

        // SAFETY: `data` and `file` outlive both calls, and `wide` is
        // NUL-terminated. The state opened by VERIFY is closed below.
        let status = unsafe {
            WinVerifyTrust(
                HWND::default(),
                &mut action,
                &mut data as *mut WINTRUST_DATA as *mut c_void,
            )
        };

        let outcome = if status == 0 {
            // SAFETY: the state handle is valid until the CLOSE call.
            unsafe { signer_name(data.hWVTStateData) }
                .ok_or_else(|| InstallError::SignatureInvalid("no signer certificate".into()))
        } else {
            Err(InstallError::SignatureInvalid(describe(status)))
        };

        data.dwStateAction = WTD_STATEACTION_CLOSE;

        // SAFETY: same structures as above, releasing the verification state.
        unsafe {
            WinVerifyTrust(
                HWND::default(),
                &mut action,
                &mut data as *mut WINTRUST_DATA as *mut c_void,
            );
        }

        outcome
    }
}

/// # Safety
/// `state` must be the state handle of a successful VERIFY call that has not
/// been closed yet.
unsafe fn signer_name(state: HANDLE) -> Option<String> {
    unsafe {
        let provider = WTHelperProvDataFromStateData(state);

        if provider.is_null() {
            return None;
        }

        let signer = WTHelperGetProvSignerFromChain(provider, 0, false, 0);

        if signer.is_null() {
            return None;
        }

        let cert = WTHelperGetProvCertFromChain(signer, 0);

        if cert.is_null() || (*cert).pCert.is_null() {
            return None;
        }

        let mut buffer = [0u16; 256];
        let length = CertGetNameStringW(
            (*cert).pCert,
            CERT_NAME_SIMPLE_DISPLAY_TYPE,
            0,
            None,
            Some(&mut buffer),
        ) as usize;

        if length <= 1 {
            return None;
        }

        Some(String::from_utf16_lossy(&buffer[..length - 1]))
    }
}

fn describe(status: i32) -> String {
    match status as u32 {
        0x800B_0100 => "the file is not signed".to_owned(),
        0x800B_0101 | 0x800B_0004 => "the signing certificate is not trusted".to_owned(),
        0x800B_0109 => "the certificate chain ends at an untrusted root".to_owned(),
        0x8009_6010 => "the file has been modified after signing".to_owned(),
        0x800B_010C => "the signing certificate was revoked".to_owned(),
        code => format!("Windows rejected the signature (0x{code:08X})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsigned_file_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("plain.exe");

        std::fs::write(&file, b"MZ not really an exe").unwrap();

        assert!(matches!(
            WinTrustVerifier.verify(&file),
            Err(InstallError::SignatureInvalid(_))
        ));
    }

    #[test]
    #[ignore = "set SONEAR_SIGNED_EXE to an Authenticode-signed exe"]
    fn signed_file_reports_its_signer() {
        let path = std::env::var("SONEAR_SIGNED_EXE").unwrap();
        let signer = WinTrustVerifier.verify(Path::new(&path)).unwrap();

        println!("signer: {signer}");

        assert!(!signer.is_empty());
    }
}
