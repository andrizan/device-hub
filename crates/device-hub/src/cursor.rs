//! Pin the cursor inside the app window during measurement (ClipCursor, as games do).
//!
//! Used for the measured polling rate (docs/POLLING_RATE.md) — only during the 5-second
//! measurement. Confining the cursor to the window naturally makes it trace fast circles inside it,
//! and also prevents the accident of the app going to the background and raw input being throttled.
//! Always call release as a pair (the OS releases it automatically at process exit, but
//! release it from the tick side too if the window closes during measurement).

#[cfg(windows)]
pub fn clip_to_window(window: &gpui_kit::Window) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::{HWND, RECT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{ClipCursor, GetWindowRect};

    let Ok(h) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(w) = h.as_raw() else {
        return;
    };
    unsafe {
        let hwnd = w.hwnd.get() as HWND;
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r) != 0 {
            ClipCursor(&r);
        }
    }
}

#[cfg(windows)]
pub fn unclip() {
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::ClipCursor(std::ptr::null()) };
}

#[cfg(not(windows))]
pub fn clip_to_window(_window: &gpui_kit::Window) {}
#[cfg(not(windows))]
pub fn unclip() {}
