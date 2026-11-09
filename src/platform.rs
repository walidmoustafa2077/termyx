//! Native window integration that winit does not expose.

use crate::theme::Rgb;

/// Paints the native window frame in the terminal background colour.
///
/// Windows 11 draws a light 1-pixel border around every window; against a
/// dark terminal it reads as a line above and below the content. Matching the
/// border to the background hides it without touching the client area.
#[cfg(target_os = "windows")]
pub fn apply_dark_frame(window: &winit::window::Window, background: Rgb) {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    /// Border colour of the window frame (`DwmSetWindowAttribute`).
    const DWMWA_BORDER_COLOR: u32 = 34;

    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    // COLORREF layout is 0x00BBGGRR.
    let color =
        u32::from(background.r) | u32::from(background.g) << 8 | u32::from(background.b) << 16;

    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: *mut core::ffi::c_void,
            attribute: u32,
            value: *const core::ffi::c_void,
            size: u32,
        ) -> i32;
    }

    // SAFETY: `hwnd` comes from winit's live window handle and `color` is a
    // plain `u32` matching the attribute's documented size.
    unsafe {
        DwmSetWindowAttribute(
            win32.hwnd.get() as *mut core::ffi::c_void,
            DWMWA_BORDER_COLOR,
            &color as *const u32 as *const core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        );
    }
}

#[cfg(not(target_os = "windows"))]
pub fn apply_dark_frame(_window: &winit::window::Window, _background: Rgb) {}
