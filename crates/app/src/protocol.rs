//! Optional `spotify:` link registration. Sonear only registers itself as a
//! capable handler; the user chooses it under Windows' Default apps, so it
//! never takes the links from the Spotify app on its own.

use std::path::Path;

use thiserror::Error;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteKeyValueW, RegDeleteTreeW, RegOpenKeyExW, RegQueryValueExW,
    RegSetValueExW,
};
use windows::core::{HSTRING, PCWSTR};

const CLASS_KEY: &str = r"Software\Classes\Sonear.SpotifyLink";
const COMMAND_KEY: &str = r"Software\Classes\Sonear.SpotifyLink\shell\open\command";
const APP_KEY: &str = r"Software\Sonear";
const CAPABILITIES_KEY: &str = r"Software\Sonear\Capabilities";
const URL_KEY: &str = r"Software\Sonear\Capabilities\URLAssociations";
const REGISTERED_KEY: &str = r"Software\RegisteredApplications";
const PROG_ID: &str = "Sonear.SpotifyLink";
const APP_NAME: &str = "Sonear";
const DEFAULT_APPS_URL: &str = "ms-settings:defaultapps?registeredAppUser=Sonear";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registration {
    NotRegistered,
    Registered,
    /// Registered, but the command points at another copy of Sonear.
    Outdated,
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("could not find the Sonear program: {0}")]
    Exe(String),
    #[error("could not {action} the Windows registry (error {code})")]
    Registry { action: &'static str, code: u32 },
}

/// One registry value. `name` is `None` for a key's default value.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Value {
    key: &'static str,
    name: Option<&'static str>,
    data: String,
}

fn value(key: &'static str, name: Option<&'static str>, data: impl Into<String>) -> Value {
    Value {
        key,
        name,
        data: data.into(),
    }
}

fn command_line(exe: &Path) -> String {
    format!("\"{}\" \"%1\"", exe.display())
}

fn values(exe: &Path) -> Vec<Value> {
    vec![
        value(CLASS_KEY, None, "Spotify link"),
        value(
            r"Software\Classes\Sonear.SpotifyLink\DefaultIcon",
            None,
            format!("\"{}\",0", exe.display()),
        ),
        value(COMMAND_KEY, None, command_line(exe)),
        value(CAPABILITIES_KEY, Some("ApplicationName"), APP_NAME),
        value(
            CAPABILITIES_KEY,
            Some("ApplicationDescription"),
            "Desktop player for Spotify",
        ),
        value(URL_KEY, Some("spotify"), PROG_ID),
        value(REGISTERED_KEY, Some(APP_NAME), CAPABILITIES_KEY),
    ]
}

pub fn status() -> Registration {
    let Ok(exe) = std::env::current_exe() else {
        return Registration::NotRegistered;
    };
    let Some(command) = read_default(COMMAND_KEY) else {
        return Registration::NotRegistered;
    };

    if command == command_line(&exe) {
        Registration::Registered
    } else {
        Registration::Outdated
    }
}

pub fn register() -> Result<(), ProtocolError> {
    let exe = std::env::current_exe().map_err(|error| ProtocolError::Exe(error.to_string()))?;

    for value in values(&exe) {
        write(&value)?;
    }

    Ok(())
}

/// Removes what `register` adds. Keys that are already gone are fine.
pub fn unregister() -> Result<(), ProtocolError> {
    for key in [CLASS_KEY, APP_KEY] {
        let code = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(key)) };

        check("remove", code.0, true)?;
    }

    let code = unsafe {
        RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(REGISTERED_KEY),
            &HSTRING::from(APP_NAME),
        )
    };

    check("remove", code.0, true)
}

/// Opens Windows' Default apps page, where the user picks Sonear for `spotify:` links.
pub fn open_default_apps(cx: &mut gpui_kit::App) {
    cx.open_url(DEFAULT_APPS_URL);
}

fn check(action: &'static str, code: u32, missing_ok: bool) -> Result<(), ProtocolError> {
    if code == ERROR_SUCCESS.0 {
        return Ok(());
    }

    if missing_ok && (code == ERROR_FILE_NOT_FOUND.0 || code == ERROR_PATH_NOT_FOUND.0) {
        return Ok(());
    }

    Err(ProtocolError::Registry { action, code })
}

fn utf16_bytes(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn write(value: &Value) -> Result<(), ProtocolError> {
    let mut key = HKEY::default();

    // SAFETY: `key` is a valid out pointer and is closed below.
    let code = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(value.key),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
    };

    check("create a key in", code.0, false)?;

    let name = value.name.map(HSTRING::from);
    let name = name
        .as_ref()
        .map_or(PCWSTR::null(), |name| PCWSTR(name.as_ptr()));

    // SAFETY: `key` was opened above; the data outlives the call.
    let code = unsafe { RegSetValueExW(key, name, None, REG_SZ, Some(&utf16_bytes(&value.data))) };

    // SAFETY: `key` is open and not used again.
    unsafe {
        let _ = RegCloseKey(key);
    }

    check("write to", code.0, false)
}

fn read_default(key_path: &str) -> Option<String> {
    let mut key = HKEY::default();

    // SAFETY: `key` is a valid out pointer and is closed below.
    let code = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(key_path),
            None,
            KEY_READ,
            &mut key,
        )
    };

    if code != ERROR_SUCCESS {
        return None;
    }

    let mut size = 0u32;

    // SAFETY: the first call only asks for the size.
    let sized = unsafe { RegQueryValueExW(key, PCWSTR::null(), None, None, None, Some(&mut size)) };
    let mut buffer = vec![0u8; size as usize];
    let read = sized == ERROR_SUCCESS
        && unsafe {
            RegQueryValueExW(
                key,
                PCWSTR::null(),
                None,
                None,
                Some(buffer.as_mut_ptr()),
                Some(&mut size),
            )
        } == ERROR_SUCCESS;

    // SAFETY: `key` is open and not used again.
    unsafe {
        let _ = RegCloseKey(key);
    }

    if !read {
        return None;
    }

    let units: Vec<u16> = buffer[..size as usize]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|unit| *unit != 0)
        .collect();

    Some(String::from_utf16_lossy(&units))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn find<'a>(values: &'a [Value], key: &str, name: Option<&str>) -> &'a Value {
        values
            .iter()
            .find(|value| value.key == key && value.name == name)
            .expect("value is registered")
    }

    #[test]
    fn exe_paths_with_spaces_are_quoted() {
        let exe = PathBuf::from(r"C:\Program Files\Sonear\sonear.exe");
        let values = values(&exe);

        assert_eq!(
            find(&values, COMMAND_KEY, None).data,
            r#""C:\Program Files\Sonear\sonear.exe" "%1""#
        );
        assert_eq!(
            find(
                &values,
                r"Software\Classes\Sonear.SpotifyLink\DefaultIcon",
                None
            )
            .data,
            r#""C:\Program Files\Sonear\sonear.exe",0"#
        );
    }

    #[test]
    fn registration_describes_a_capable_handler() {
        let values = values(Path::new(r"C:\sonear.exe"));

        assert_eq!(find(&values, CLASS_KEY, None).data, "Spotify link");
        assert_eq!(
            find(&values, CAPABILITIES_KEY, Some("ApplicationName")).data,
            "Sonear"
        );
        assert_eq!(
            find(&values, CAPABILITIES_KEY, Some("ApplicationDescription")).data,
            "Desktop player for Spotify"
        );
        assert_eq!(
            find(&values, URL_KEY, Some("spotify")).data,
            "Sonear.SpotifyLink"
        );
        assert_eq!(
            find(&values, REGISTERED_KEY, Some("Sonear")).data,
            r"Software\Sonear\Capabilities"
        );
    }

    #[test]
    fn every_value_lives_under_a_key_unregister_removes() {
        for value in values(Path::new(r"C:\sonear.exe")) {
            let removed = value.key.starts_with(CLASS_KEY)
                || value.key.starts_with(APP_KEY)
                || (value.key == REGISTERED_KEY && value.name == Some(APP_NAME));

            assert!(removed, "{} would be left behind", value.key);
        }
    }

    #[test]
    fn strings_are_nul_terminated_utf16() {
        assert_eq!(utf16_bytes("ab"), vec![b'a', 0, b'b', 0, 0, 0]);
    }
}
