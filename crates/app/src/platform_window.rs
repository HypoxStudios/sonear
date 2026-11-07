//! Win32 calls GPUI has no API for: finding a window's handle and hiding or
//! showing it.

use std::time::{Duration, Instant};

use gpui_kit::{AnyWindowHandle, App, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{
    DWM_WINDOW_CORNER_PREFERENCE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUNDSMALL,
    DwmSetWindowAttribute,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, GWL_STYLE, GetClientRect, GetForegroundWindow, GetWindowLongW,
    GetWindowThreadProcessId, IsIconic, SW_HIDE, SW_RESTORE, SW_SHOW, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SetForegroundWindow, SetWindowLongW, SetWindowPos,
    ShowWindow, WS_POPUP, WS_VISIBLE,
};

const FOCUS_WAIT: Duration = Duration::from_secs(5);
const FOCUS_POLL: Duration = Duration::from_millis(50);

pub fn hwnd(window: &Window) -> Option<HWND> {
    // `Window` also has an inherent `window_handle` that returns GPUI's own handle type.
    let handle = HasWindowHandle::window_handle(window).ok()?;

    match handle.as_raw() {
        RawWindowHandle::Win32(win32) => Some(HWND(win32.hwnd.get() as *mut _)),
        _ => None,
    }
}

pub fn hide(hwnd: HWND) {
    // SAFETY: `hwnd` belongs to a live window of this process and the call is made on its thread.
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

/// Shows the window again, restores it if it was minimized, and brings it to the front.
pub fn show(hwnd: HWND) {
    // SAFETY: `hwnd` belongs to a live window of this process and the calls are made on its thread.
    unsafe {
        let command = if IsIconic(hwnd).as_bool() {
            SW_RESTORE
        } else {
            SW_SHOW
        };

        let _ = ShowWindow(hwnd, command);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Brings a window that was just opened to the front. GPUI's `cx.activate` does
/// nothing on Windows.
pub fn focus(window: AnyWindowHandle, cx: &mut App) {
    let _ = window.update(cx, |_, window, _| {
        if let Some(handle) = hwnd(window) {
            show(handle);
        }
    });
}

/// Lets a process this one just started take the focus. Fails harmlessly if this
/// process doesn't have the focus to give, so it never steals it from whatever
/// the user moved to.
pub fn allow_foreground(pid: u32) {
    // SAFETY: a plain Win32 call with a process ID.
    let _ = unsafe { AllowSetForegroundWindow(pid) };
}

fn this_process_has_focus() -> bool {
    let mut owner = 0;

    // SAFETY: plain Win32 calls with a valid out pointer. A null window just reports no owner.
    unsafe {
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut owner));

        owner == GetCurrentProcessId()
    }
}

/// Gives the focus to the process just started, then quits. Windows only honours
/// the grant while this process still has the focus, so its window stays up until
/// the focus has moved or the wait runs out.
pub fn quit_after_handing_focus_to(pid: u32, cx: &mut App) {
    allow_foreground(pid);

    cx.spawn(async move |cx| {
        let started = Instant::now();

        while this_process_has_focus() && started.elapsed() < FOCUS_WAIT {
            cx.background_executor().timer(FOCUS_POLL).await;
        }

        cx.update(|cx| cx.quit());
    })
    .detach();
}

/// Turns a frame-less pop-up into a truly borderless one. GPUI gives such windows
/// `WS_OVERLAPPED`, which leaves a frame around the content. The client size is kept.
pub fn make_borderless(hwnd: HWND) {
    let mut client = RECT::default();

    // SAFETY: `hwnd` belongs to a live window of this process and the calls are made on its thread.
    unsafe {
        if GetClientRect(hwnd, &mut client).is_err() {
            return;
        }

        let visible = GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_VISIBLE.0;

        SetWindowLongW(hwnd, GWL_STYLE, (WS_POPUP.0 | visible) as i32);

        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            client.right - client.left,
            client.bottom - client.top,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );

        // Windows 10 has no rounded corners, so a failure here is fine.
        let corners = DWMWCP_ROUNDSMALL;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            std::ptr::from_ref(&corners).cast(),
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        );
    }
}
