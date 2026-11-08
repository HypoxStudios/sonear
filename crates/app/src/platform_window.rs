//! Win32 calls GPUI has no API for: finding a window's handle and hiding or
//! showing it.

use std::cell::Cell;
use std::time::{Duration, Instant};

use gpui_kit::{AnyWindowHandle, App, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWM_WINDOW_CORNER_PREFERENCE, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUNDSMALL, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, GWL_EXSTYLE, GWL_STYLE, GetClientRect, GetCursorPos,
    GetForegroundWindow, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, HTBOTTOM,
    HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT,
    IsIconic, MA_NOACTIVATE, SW_HIDE, SW_RESTORE, SW_SHOW, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetForegroundWindow, SetWindowLongW, SetWindowPos,
    ShowWindow, WM_CAPTURECHANGED, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_NCHITTEST,
    WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WS_EX_NOACTIVATE, WS_POPUP, WS_VISIBLE,
};

use crate::theme;

const FOCUS_WAIT: Duration = Duration::from_secs(5);
const FOCUS_POLL: Duration = Duration::from_millis(50);
const NO_ACTIVATE_SUBCLASS: usize = 1;

thread_local! {
    /// The cursor's offset from the window's corner while a no-activate window is dragged.
    static DRAG_OFFSET: Cell<Option<POINT>> = const { Cell::new(None) };
    /// The edge drag in progress on a resizable no-activate window.
    static RESIZE: Cell<Option<ResizeDrag>> = const { Cell::new(None) };
    /// The smallest logical size of the window, once it can be resized by its edges.
    static MIN_SIZE: Cell<Option<(f32, f32)>> = const { Cell::new(None) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Edges {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

impl Edges {
    fn from_hit(code: u32) -> Option<Self> {
        let (left, right, top, bottom) = match code {
            HTLEFT => (true, false, false, false),
            HTRIGHT => (false, true, false, false),
            HTTOP => (false, false, true, false),
            HTBOTTOM => (false, false, false, true),
            HTTOPLEFT => (true, false, true, false),
            HTTOPRIGHT => (false, true, true, false),
            HTBOTTOMLEFT => (true, false, false, true),
            HTBOTTOMRIGHT => (false, true, false, true),
            _ => return None,
        };

        Some(Self {
            left,
            right,
            top,
            bottom,
        })
    }

    fn hit(self) -> u32 {
        match (self.left, self.right, self.top, self.bottom) {
            (true, _, true, _) => HTTOPLEFT,
            (_, true, true, _) => HTTOPRIGHT,
            (true, _, _, true) => HTBOTTOMLEFT,
            (_, true, _, true) => HTBOTTOMRIGHT,
            (true, ..) => HTLEFT,
            (_, true, ..) => HTRIGHT,
            (_, _, true, _) => HTTOP,
            _ => HTBOTTOM,
        }
    }

    /// The edges of `frame` the point is within `band` of, if any.
    fn at(frame: RECT, x: i32, y: i32, band: i32) -> Option<Self> {
        if x < frame.left || x >= frame.right || y < frame.top || y >= frame.bottom {
            return None;
        }

        let edges = Self {
            left: x < frame.left + band,
            right: x >= frame.right - band,
            top: y < frame.top + band,
            bottom: y >= frame.bottom - band,
        };

        (edges != Self::default()).then_some(edges)
    }
}

#[derive(Debug, Clone, Copy)]
struct ResizeDrag {
    edges: Edges,
    cursor: POINT,
    frame: RECT,
}

/// The frame after dragging `edges` of `start` by `(dx, dy)`: never smaller than
/// `min` and never past `work`, with the opposite edges staying put.
fn resized_frame(
    start: RECT,
    edges: Edges,
    (dx, dy): (i32, i32),
    min: (i32, i32),
    work: RECT,
) -> RECT {
    let mut frame = start;

    if edges.left {
        frame.left = (start.left + dx).max(work.left).min(start.right - min.0);
    }

    if edges.right {
        frame.right = (start.right + dx).min(work.right).max(start.left + min.0);
    }

    if edges.top {
        frame.top = (start.top + dy).max(work.top).min(start.bottom - min.1);
    }

    if edges.bottom {
        frame.bottom = (start.bottom + dy).min(work.bottom).max(start.top + min.1);
    }

    frame
}

/// Where a window of `width` by `height` goes when it replaces one at `frame`: the
/// bottom edge stays when the window sat in the lower half of `work`, else the top
/// edge stays. The result is moved fully inside `work`.
fn anchored_origin(frame: RECT, work: RECT, width: i32, height: i32) -> (i32, i32) {
    let centre = frame.top + (frame.bottom - frame.top) / 2;
    let lower = centre > work.top + (work.bottom - work.top) / 2;
    let y = if lower {
        frame.bottom - height
    } else {
        frame.top
    };
    let x = frame.left.min(work.right - width).max(work.left);

    (x, y.min(work.bottom - height).max(work.top))
}

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

pub fn is_minimized(hwnd: HWND) -> bool {
    // SAFETY: `hwnd` belongs to a live window of this process and the call is made on its thread.
    unsafe { IsIconic(hwnd).as_bool() }
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

/// Removes the 1 px outline Windows 11 draws around a window. The rounded corners stay.
pub fn hide_outline(hwnd: HWND) {
    let colour = DWMWA_COLOR_NONE;

    // SAFETY: `hwnd` belongs to a live window of this process; the value is a plain `u32`.
    // Windows 10 has no such attribute, so a failure here is fine.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            std::ptr::from_ref(&colour).cast(),
            size_of::<u32>() as u32,
        );
    }
}

/// Lets a no-activate window be resized by dragging its edges, by hand like the
/// move, since the system sizing loop activates the window. Call it again to
/// change the smallest logical size.
pub fn set_min_size(width: f32, height: f32) {
    MIN_SIZE.set(Some((width, height)));
}

fn scale_of(hwnd: HWND) -> f32 {
    // SAFETY: `hwnd` belongs to a live window of this process.
    let dpi = unsafe { GetDpiForWindow(hwnd) };

    if dpi == 0 { 1. } else { dpi as f32 / 96. }
}

fn work_area(hwnd: HWND) -> Option<RECT> {
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };

    // SAFETY: `hwnd` belongs to a live window of this process and `info` is a valid out pointer.
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);

        GetMonitorInfoW(monitor, &mut info)
            .as_bool()
            .then_some(info.rcWork)
    }
}

/// Resizes the window to a logical size without activating it, keeping the bottom
/// edge when it sits in the lower half of its display and the top edge otherwise,
/// then moves it fully into the display's work area.
pub fn resize_anchored(hwnd: HWND, width: f32, height: f32) {
    let scale = scale_of(hwnd);
    let width = (width * scale).round() as i32;
    let height = (height * scale).round() as i32;
    let mut frame = RECT::default();

    // SAFETY: `hwnd` belongs to a live window of this process and the calls are made on its thread.
    unsafe {
        if GetWindowRect(hwnd, &mut frame).is_err() {
            return;
        }

        let work = work_area(hwnd).unwrap_or(frame);
        let (x, y) = anchored_origin(frame, work, width, height);

        let _ = SetWindowPos(
            hwnd,
            None,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
    }
}

/// Keeps a window from ever taking the foreground, so clicking or dragging it
/// leaves a full-screen game in front. GPUI activates on every click and the
/// system move loop activates too, so clicks answer `MA_NOACTIVATE` and
/// dragging by a drag area moves the window by hand.
pub fn never_activate(hwnd: HWND) {
    // SAFETY: `hwnd` belongs to a live window of this process and the calls are made on its thread.
    unsafe {
        let style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;

        SetWindowLongW(hwnd, GWL_EXSTYLE, (style | WS_EX_NOACTIVATE.0) as i32);

        let _ = SetWindowSubclass(hwnd, Some(no_activate_proc), NO_ACTIVATE_SUBCLASS, 0);
    }
}

unsafe extern "system" fn no_activate_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    match message {
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_NCHITTEST if MIN_SIZE.get().is_some() => {
            if let Some(edges) = edge_under(hwnd, lparam) {
                return LRESULT(edges.hit() as isize);
            }
        }
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK
            if MIN_SIZE.get().is_some() && Edges::from_hit(wparam.0 as u32).is_some() =>
        {
            if message == WM_NCLBUTTONDOWN
                && let Some(edges) = Edges::from_hit(wparam.0 as u32)
            {
                start_resize(hwnd, edges);
            }

            return LRESULT(0);
        }
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if wparam.0 as u32 == HTCAPTION => {
            start_drag(hwnd);

            return LRESULT(0);
        }
        WM_MOUSEMOVE if RESIZE.get().is_some() => {
            follow_resize(hwnd);

            return LRESULT(0);
        }
        WM_MOUSEMOVE if DRAG_OFFSET.get().is_some() => {
            follow_cursor(hwnd);

            return LRESULT(0);
        }
        WM_LBUTTONUP if DRAG_OFFSET.get().is_some() || RESIZE.get().is_some() => {
            DRAG_OFFSET.set(None);
            RESIZE.set(None);

            // SAFETY: releasing the capture this thread set has no preconditions.
            let _ = unsafe { ReleaseCapture() };

            return LRESULT(0);
        }
        WM_CAPTURECHANGED => {
            DRAG_OFFSET.set(None);
            RESIZE.set(None);
        }
        _ => {}
    }

    // SAFETY: forwards the message unchanged to the next procedure in the chain.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

fn start_drag(hwnd: HWND) {
    let mut cursor = POINT::default();
    let mut frame = RECT::default();

    // SAFETY: `hwnd` is the live window receiving the message, on its own thread.
    unsafe {
        if GetCursorPos(&mut cursor).is_err() || GetWindowRect(hwnd, &mut frame).is_err() {
            return;
        }

        SetCapture(hwnd);
    }

    DRAG_OFFSET.set(Some(POINT {
        x: cursor.x - frame.left,
        y: cursor.y - frame.top,
    }));
}

fn follow_cursor(hwnd: HWND) {
    let Some(offset) = DRAG_OFFSET.get() else {
        return;
    };
    let mut cursor = POINT::default();

    // SAFETY: `hwnd` is the live window receiving the message, on its own thread.
    unsafe {
        if GetCursorPos(&mut cursor).is_err() {
            return;
        }

        let _ = SetWindowPos(
            hwnd,
            None,
            cursor.x - offset.x,
            cursor.y - offset.y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

fn edge_under(hwnd: HWND, lparam: LPARAM) -> Option<Edges> {
    let x = i32::from(lparam.0 as i16);
    let y = i32::from((lparam.0 >> 16) as i16);
    let band = (theme::MINI_RESIZE_BAND * scale_of(hwnd)).round() as i32;
    let mut frame = RECT::default();

    // SAFETY: `hwnd` is the live window receiving the message, on its own thread.
    unsafe { GetWindowRect(hwnd, &mut frame).ok()? };

    Edges::at(frame, x, y, band)
}

fn start_resize(hwnd: HWND, edges: Edges) {
    let mut cursor = POINT::default();
    let mut frame = RECT::default();

    // SAFETY: `hwnd` is the live window receiving the message, on its own thread.
    unsafe {
        if GetCursorPos(&mut cursor).is_err() || GetWindowRect(hwnd, &mut frame).is_err() {
            return;
        }

        SetCapture(hwnd);
    }

    RESIZE.set(Some(ResizeDrag {
        edges,
        cursor,
        frame,
    }));
}

fn follow_resize(hwnd: HWND) {
    let (Some(drag), Some((min_width, min_height))) = (RESIZE.get(), MIN_SIZE.get()) else {
        return;
    };
    let mut cursor = POINT::default();

    // SAFETY: `hwnd` is the live window receiving the message, on its own thread.
    unsafe {
        if GetCursorPos(&mut cursor).is_err() {
            return;
        }

        let scale = scale_of(hwnd);
        let work = work_area(hwnd).unwrap_or(drag.frame);
        let frame = resized_frame(
            drag.frame,
            drag.edges,
            (cursor.x - drag.cursor.x, cursor.y - drag.cursor.y),
            (
                (min_width * scale).round() as i32,
                (min_height * scale).round() as i32,
            ),
            work,
        );

        let _ = SetWindowPos(
            hwnd,
            None,
            frame.left,
            frame.top,
            frame.right - frame.left,
            frame.bottom - frame.top,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: RECT = RECT {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1040,
    };

    fn frame(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn edges_are_found_inside_the_band() {
        let window = frame(100, 100, 460, 196);

        assert_eq!(Edges::at(window, 100, 150, 6), Edges::from_hit(HTLEFT));
        assert_eq!(
            Edges::at(window, 459, 195, 6),
            Edges::from_hit(HTBOTTOMRIGHT)
        );
        assert_eq!(Edges::at(window, 200, 100, 6), Edges::from_hit(HTTOP));
        assert_eq!(Edges::at(window, 200, 150, 6), None);
        assert_eq!(Edges::at(window, 99, 150, 6), None);
    }

    #[test]
    fn hit_codes_round_trip() {
        for code in [
            HTLEFT,
            HTRIGHT,
            HTTOP,
            HTBOTTOM,
            HTTOPLEFT,
            HTTOPRIGHT,
            HTBOTTOMLEFT,
            HTBOTTOMRIGHT,
        ] {
            assert_eq!(Edges::from_hit(code).map(Edges::hit), Some(code));
        }
    }

    #[test]
    fn dragging_the_left_edge_keeps_the_right_edge() {
        let start = frame(500, 500, 860, 596);
        let left = Edges::from_hit(HTLEFT).unwrap();

        assert_eq!(
            resized_frame(start, left, (-40, 0), (300, 88), WORK),
            frame(460, 500, 860, 596)
        );
        assert_eq!(
            resized_frame(start, left, (400, 0), (300, 88), WORK),
            frame(560, 500, 860, 596)
        );
    }

    #[test]
    fn dragging_the_top_left_corner_keeps_the_far_corner() {
        let start = frame(500, 500, 860, 596);
        let corner = Edges::from_hit(HTTOPLEFT).unwrap();

        assert_eq!(
            resized_frame(start, corner, (-20, -30), (300, 88), WORK),
            frame(480, 470, 860, 596)
        );
    }

    #[test]
    fn a_resize_stops_at_the_work_area() {
        let start = frame(1500, 900, 1860, 996);
        let corner = Edges::from_hit(HTBOTTOMRIGHT).unwrap();

        assert_eq!(
            resized_frame(start, corner, (500, 500), (300, 88), WORK),
            frame(1500, 900, 1920, 1040)
        );
    }

    #[test]
    fn a_window_in_the_lower_half_grows_upward() {
        let window = frame(1500, 900, 1860, 996);

        assert_eq!(anchored_origin(window, WORK, 360, 300), (1500, 696));
    }

    #[test]
    fn a_window_in_the_upper_half_grows_downward() {
        let window = frame(100, 100, 460, 196);

        assert_eq!(anchored_origin(window, WORK, 360, 300), (100, 100));
    }

    #[test]
    fn the_new_frame_is_moved_inside_the_work_area() {
        let window = frame(1700, 20, 2060, 116);

        assert_eq!(anchored_origin(window, WORK, 360, 300), (1560, 20));

        let low = frame(100, 960, 460, 1056);

        assert_eq!(anchored_origin(low, WORK, 360, 96), (100, 944));
    }
}
