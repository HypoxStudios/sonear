//! The notification-area icon and its menu, with direct Win32 calls.
//!
//! Everything here runs on the main thread: the icon's window belongs to the
//! thread that runs GPUI's message loop, so that loop delivers its messages.

use std::cell::Cell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use std::sync::{Once, OnceLock};

use thiserror::Error;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{debug, warn};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NIM_SETVERSION, NIN_SELECT, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, GetSystemMetrics, HICON, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, MF_CHECKED,
    MF_GRAYED, MF_SEPARATOR, MF_STRING, PostMessageW, RegisterClassW, RegisterWindowMessageW,
    SM_CXSMICON, SM_CYSMICON, SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
    WINDOW_EX_STYLE, WM_CONTEXTMENU, WM_NULL, WM_SETTINGCHANGE, WM_USER, WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::{HSTRING, PCSTR, PCWSTR, w};

const ICON_WHITE: u16 = 2;
const ICON_BLACK: u16 = 3;
const ICON_ID: u32 = 1;
const CALLBACK_MESSAGE: u32 = WM_USER + 1;
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
const TOOLTIP: &str = "Sonear";
const HINT_TITLE: &str = "Sonear is still running";
const HINT_TEXT: &str = "Music keeps playing. Quit from the tray icon.";
/// Windows 10 1903, the first build with the dark menu switch.
const DARK_MENUS_FROM_BUILD: u32 = 18362;
const SET_PREFERRED_APP_MODE: usize = 135;
const FLUSH_MENU_THEMES: usize = 136;
const ALLOW_DARK: i32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Show,
    TogglePlayPause,
    Next,
    Previous,
    ToggleMiniPlayer,
    Quit,
}

/// What the menu needs to know when it opens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MenuState {
    pub playing: bool,
    pub has_item: bool,
    pub mini_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    Item {
        id: u32,
        label: &'static str,
        enabled: bool,
        checked: bool,
    },
    Separator,
}

#[derive(Debug, Error)]
pub enum TrayError {
    #[error("could not {action}: {source}")]
    Win32 {
        action: &'static str,
        source: windows::core::Error,
    },
    #[error("the shell did not accept the tray icon")]
    Rejected,
}

const ID_PLAY_PAUSE: u32 = 1;
const ID_NEXT: u32 = 2;
const ID_PREVIOUS: u32 = 3;
const ID_MINI_PLAYER: u32 = 4;
const ID_SHOW: u32 = 5;
const ID_QUIT: u32 = 6;

/// The icon resource that stays readable on the taskbar. `None` means the
/// theme could not be read, which is treated as dark.
pub fn icon_resource(light_taskbar: Option<bool>) -> u16 {
    if light_taskbar == Some(true) {
        ICON_BLACK
    } else {
        ICON_WHITE
    }
}

pub fn menu_model(state: MenuState) -> Vec<MenuEntry> {
    let item = |id, label, enabled, checked| MenuEntry::Item {
        id,
        label,
        enabled,
        checked,
    };

    vec![
        item(
            ID_PLAY_PAUSE,
            if state.playing { "Pause" } else { "Play" },
            state.has_item,
            false,
        ),
        item(ID_NEXT, "Next", state.has_item, false),
        item(ID_PREVIOUS, "Previous", state.has_item, false),
        MenuEntry::Separator,
        item(ID_MINI_PLAYER, "Mini player", true, state.mini_open),
        item(ID_SHOW, "Show Sonear", true, false),
        MenuEntry::Separator,
        item(ID_QUIT, "Quit Sonear", true, false),
    ]
}

pub fn command_for(id: u32) -> Option<TrayCommand> {
    match id {
        ID_PLAY_PAUSE => Some(TrayCommand::TogglePlayPause),
        ID_NEXT => Some(TrayCommand::Next),
        ID_PREVIOUS => Some(TrayCommand::Previous),
        ID_MINI_PLAYER => Some(TrayCommand::ToggleMiniPlayer),
        ID_SHOW => Some(TrayCommand::Show),
        ID_QUIT => Some(TrayCommand::Quit),
        _ => None,
    }
}

fn copy_wide<const N: usize>(text: &str, out: &mut [u16; N]) {
    for (slot, unit) in out.iter_mut().zip(text.encode_utf16().take(N - 1)) {
        *slot = unit;
    }
}

const PLAYING: u8 = 1;
const HAS_ITEM: u8 = 2;
const MINI_OPEN: u8 = 4;

static MENU_STATE: AtomicU8 = AtomicU8::new(0);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static COMMANDS: OnceLock<UnboundedSender<TrayCommand>> = OnceLock::new();

#[derive(Clone, Copy)]
struct Installed {
    hwnd: HWND,
    white: HICON,
    black: HICON,
}

thread_local! {
    static TRAY: Cell<Option<Installed>> = const { Cell::new(None) };
}

/// Records what the menu shows the next time it opens. Nothing repaints.
pub fn set_state(state: MenuState) {
    let mut bits = 0;

    if state.playing {
        bits |= PLAYING;
    }

    if state.has_item {
        bits |= HAS_ITEM;
    }

    if state.mini_open {
        bits |= MINI_OPEN;
    }

    MENU_STATE.store(bits, Ordering::Relaxed);
}

fn menu_state() -> MenuState {
    let bits = MENU_STATE.load(Ordering::Relaxed);

    MenuState {
        playing: bits & PLAYING != 0,
        has_item: bits & HAS_ITEM != 0,
        mini_open: bits & MINI_OPEN != 0,
    }
}

pub fn installed() -> bool {
    TRAY.with(|tray| tray.get().is_some())
}

/// Adds the icon. Commands from its menu and clicks arrive on `commands`.
pub fn install(commands: UnboundedSender<TrayCommand>) -> Result<(), TrayError> {
    let _ = COMMANDS.set(commands);

    // SAFETY: every call below takes valid arguments, runs on the main thread and
    // keeps the handles it creates until `remove` destroys them.
    unsafe {
        let module = GetModuleHandleW(None).map_err(|source| TrayError::Win32 {
            action: "find the program module",
            source,
        })?;
        let instance = HINSTANCE(module.0);
        let class = w!("SonearTrayWindow");
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class,
            ..Default::default()
        };

        // A second registration fails harmlessly with the same class in place.
        RegisterClassW(&window_class);
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(w!("TaskbarCreated")),
            Ordering::Relaxed,
        );

        let white = load_icon(instance, ICON_WHITE)?;
        let black = load_icon(instance, ICON_BLACK)?;
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("Sonear tray"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )
        .map_err(|source| TrayError::Win32 {
            action: "create the tray window",
            source,
        })?;
        let installed = Installed { hwnd, white, black };

        TRAY.with(|tray| tray.set(Some(installed)));

        if let Err(error) = add_icon(installed) {
            remove();

            return Err(error);
        }
    }

    Ok(())
}

/// Takes the icon away so no ghost stays in the notification area.
pub fn remove() {
    let Some(installed) = TRAY.with(Cell::take) else {
        return;
    };

    // SAFETY: the handles were created by `install` and are destroyed once, here.
    unsafe {
        let data = base_data(installed.hwnd);

        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        let _ = DestroyWindow(installed.hwnd);
        let _ = DestroyIcon(installed.white);
        let _ = DestroyIcon(installed.black);
    }
}

/// Shows the one-time balloon that says Sonear keeps running.
pub fn show_hint() {
    let Some(installed) = TRAY.with(Cell::get) else {
        return;
    };
    let mut data = base_data(installed.hwnd);

    data.uFlags = NIF_INFO;
    data.dwInfoFlags = NIIF_INFO;
    copy_wide(HINT_TITLE, &mut data.szInfoTitle);
    copy_wide(HINT_TEXT, &mut data.szInfo);

    // SAFETY: `data` is fully initialised and outlives the call.
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
    }
}

fn load_icon(instance: HINSTANCE, resource: u16) -> Result<HICON, TrayError> {
    // SAFETY: a resource id passed as a pointer is the documented MAKEINTRESOURCE form.
    unsafe {
        let handle = LoadImageW(
            Some(instance),
            PCWSTR(usize::from(resource) as *const u16),
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_DEFAULTCOLOR,
        )
        .map_err(|source| TrayError::Win32 {
            action: "load a tray icon",
            source,
        })?;

        Ok(HICON(handle.0))
    }
}

fn base_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        ..Default::default()
    }
}

fn themed_icon(installed: Installed) -> HICON {
    if icon_resource(taskbar_is_light()) == ICON_BLACK {
        installed.black
    } else {
        installed.white
    }
}

fn add_icon(installed: Installed) -> Result<(), TrayError> {
    let mut data = base_data(installed.hwnd);

    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    data.uCallbackMessage = CALLBACK_MESSAGE;
    data.hIcon = themed_icon(installed);
    copy_wide(TOOLTIP, &mut data.szTip);

    // SAFETY: `data` is fully initialised and outlives the calls.
    unsafe {
        if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
            return Err(TrayError::Rejected);
        }

        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;

        if !Shell_NotifyIconW(NIM_SETVERSION, &data).as_bool() {
            warn!("the shell kept the old tray icon protocol");
        }
    }

    Ok(())
}

fn refresh_icon(installed: Installed) {
    let mut data = base_data(installed.hwnd);

    data.uFlags = NIF_ICON;
    data.hIcon = themed_icon(installed);

    // SAFETY: `data` is fully initialised and outlives the call.
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
    }
}

fn send(command: TrayCommand) {
    if let Some(commands) = COMMANDS.get() {
        let _ = commands.send(command);
    }
}

/// Whether the taskbar uses the light theme, which is separate from the apps' theme.
fn taskbar_is_light() -> Option<bool> {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;

    // SAFETY: the buffer is a `u32` and `size` states its length.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast::<c_void>()),
            Some(&raw mut size),
        )
    };

    result.is_ok().then_some(value != 0)
}

fn windows_build() -> Option<u32> {
    let mut buffer = [0u16; 16];
    let mut size = size_of_val(&buffer) as u32;

    // SAFETY: the buffer is writable for `size` bytes.
    let result = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"),
            w!("CurrentBuildNumber"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast::<c_void>()),
            Some(&raw mut size),
        )
    };

    if result.is_err() {
        return None;
    }

    let length = buffer.iter().position(|unit| *unit == 0)?;

    String::from_utf16_lossy(&buffer[..length]).parse().ok()
}

/// Win32 menus draw light unless the process opts in. The switch is only
/// exported by ordinal in `uxtheme.dll`, so every step is optional and a
/// failure leaves the light menu.
fn allow_dark_menus() {
    static ONCE: Once = Once::new();

    ONCE.call_once(|| {
        if windows_build().is_none_or(|build| build < DARK_MENUS_FROM_BUILD) {
            return;
        }

        type SetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
        type FlushMenuThemes = unsafe extern "system" fn();

        // SAFETY: the module is the system's own uxtheme, and each ordinal is only called
        // through the signature it has had since the build checked above.
        unsafe {
            let Ok(module) = GetModuleHandleW(w!("uxtheme.dll")) else {
                return;
            };
            let set_mode = GetProcAddress(module, PCSTR(SET_PREFERRED_APP_MODE as *const u8));
            let flush = GetProcAddress(module, PCSTR(FLUSH_MENU_THEMES as *const u8));
            let (Some(set_mode), Some(flush)) = (set_mode, flush) else {
                debug!("uxtheme does not export the dark menu switch");

                return;
            };
            let set_mode: SetPreferredAppMode = std::mem::transmute(set_mode);
            let flush: FlushMenuThemes = std::mem::transmute(flush);

            set_mode(ALLOW_DARK);
            flush();
        }
    });
}

fn show_menu(hwnd: HWND, x: i32, y: i32) {
    allow_dark_menus();

    // SAFETY: the menu is created, used and destroyed here, on the thread that owns `hwnd`.
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            warn!("could not create the tray menu");

            return;
        };

        for entry in menu_model(menu_state()) {
            let _ = match entry {
                MenuEntry::Separator => AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()),
                MenuEntry::Item {
                    id,
                    label,
                    enabled,
                    checked,
                } => {
                    let mut flags = MF_STRING;

                    if !enabled {
                        flags |= MF_GRAYED;
                    }

                    if checked {
                        flags |= MF_CHECKED;
                    }

                    AppendMenuW(menu, flags, id as usize, &HSTRING::from(label))
                }
            };
        }

        // The menu would not close when the user clicks elsewhere unless the owner is in
        // front, and the WM_NULL afterwards lets it close properly (both from the docs).
        let _ = SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            x,
            y,
            None,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);

        if let Some(command) = u32::try_from(chosen.0).ok().and_then(command_for) {
            send(command);
        }
    }
}

/// Reads a `WM_SETTINGCHANGE` string, which Windows passes as a pointer in `lparam`.
fn setting_name(lparam: LPARAM) -> Option<String> {
    const LONGEST: usize = 64;

    let pointer = lparam.0 as *const u16;

    if pointer.is_null() {
        return None;
    }

    let mut units = Vec::new();

    for offset in 0..LONGEST {
        // SAFETY: Windows documents this lParam as a null-terminated string, and the loop
        // stops at the terminator.
        let unit = unsafe { *pointer.add(offset) };

        if unit == 0 {
            return Some(String::from_utf16_lossy(&units));
        }

        units.push(unit);
    }

    None
}

fn signed_low(value: usize) -> i32 {
    i32::from(value as u16 as i16)
}

fn signed_high(value: usize) -> i32 {
    i32::from((value >> 16) as u16 as i16)
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let installed = TRAY.with(Cell::get);

    if message == TASKBAR_CREATED.load(Ordering::Relaxed)
        && message != 0
        && let Some(installed) = installed
    {
        if let Err(error) = add_icon(installed) {
            warn!(%error, "could not bring the tray icon back after Explorer restarted");
        }

        return LRESULT(0);
    }

    match message {
        CALLBACK_MESSAGE => {
            match (lparam.0 as usize & 0xFFFF) as u32 {
                NIN_SELECT | NIN_KEYSELECT => send(TrayCommand::Show),
                WM_CONTEXTMENU => show_menu(hwnd, signed_low(wparam.0), signed_high(wparam.0)),
                _ => {}
            }

            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            if let Some(installed) = installed
                && setting_name(lparam).as_deref() == Some("ImmersiveColorSet")
            {
                refresh_icon(installed);
            }

            LRESULT(0)
        }
        // SAFETY: the arguments are the ones Windows gave this window procedure.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(state: MenuState) -> Vec<(&'static str, bool, bool)> {
        menu_model(state)
            .into_iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item {
                    label,
                    enabled,
                    checked,
                    ..
                } => Some((label, enabled, checked)),
                MenuEntry::Separator => None,
            })
            .collect()
    }

    #[test]
    fn light_taskbars_get_the_black_icon() {
        assert_eq!(icon_resource(Some(true)), ICON_BLACK);
        assert_eq!(icon_resource(Some(false)), ICON_WHITE);
        assert_eq!(icon_resource(None), ICON_WHITE);
    }

    #[test]
    fn menu_follows_the_playback_state() {
        let idle = labels(MenuState::default());

        assert_eq!(idle[0], ("Play", false, false));
        assert_eq!(idle[1], ("Next", false, false));
        assert_eq!(idle[2], ("Previous", false, false));
        assert_eq!(idle[3], ("Mini player", true, false));
        assert_eq!(idle[4], ("Show Sonear", true, false));
        assert_eq!(idle[5], ("Quit Sonear", true, false));

        let playing = labels(MenuState {
            playing: true,
            has_item: true,
            mini_open: true,
        });

        assert_eq!(playing[0], ("Pause", true, false));
        assert_eq!(playing[1], ("Next", true, false));
        assert_eq!(playing[2], ("Previous", true, false));
        assert_eq!(playing[3], ("Mini player", true, true));
    }

    #[test]
    fn menu_has_two_separators_and_every_item_maps_to_a_command() {
        let model = menu_model(MenuState::default());
        let separators = model
            .iter()
            .filter(|entry| **entry == MenuEntry::Separator)
            .count();

        assert_eq!(separators, 2);

        for entry in model {
            if let MenuEntry::Item { id, .. } = entry {
                assert!(command_for(id).is_some());
            }
        }

        assert_eq!(command_for(0), None);
        assert_eq!(command_for(ID_QUIT), Some(TrayCommand::Quit));
    }

    #[test]
    fn wide_text_is_cut_to_leave_a_terminator() {
        let mut out = [7u16; 4];

        copy_wide("abcdef", &mut out);

        assert_eq!(out, [97, 98, 99, 7]);

        let mut short = [0u16; 8];

        copy_wide("Sonear", &mut short);

        assert_eq!(&short[..7], &[83, 111, 110, 101, 97, 114, 0]);
    }

    #[test]
    fn anchor_coordinates_keep_their_sign() {
        let packed = ((-20i16 as u16 as usize) << 16) | (300i16 as u16 as usize);

        assert_eq!(signed_low(packed), 300);
        assert_eq!(signed_high(packed), -20);
    }
}
