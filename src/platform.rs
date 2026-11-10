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

/// Restores the resize frame, snap, and shadow that removing the caption also
/// removes.
///
/// A frameless winit window loses its non-client area, so edge hit-testing and
/// Aero Snap go with the caption. Clearing `WS_CAPTION` and `WS_POPUP` while
/// setting `WS_THICKFRAME` (plus the system-menu and minimise/maximise bits)
/// restores the style `DefWindowProc` resizes and snaps with, and
/// `SWP_FRAMECHANGED` recomputes the frame without drawing a title bar.
///
/// winit 0.30 still answers `WM_NCCALCSIZE` for an undecorated window and keeps
/// the client area over the whole window, so these bits alone do not restore
/// edge hit-testing; that needs a `WM_NCHITTEST` path.
#[cfg(target_os = "windows")]
pub fn apply_custom_chrome(window: &winit::window::Window) {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    /// Index of a window's style bits (`GWL_STYLE`).
    const GWL_STYLE: i32 = -16;
    const WS_THICKFRAME: isize = 0x0004_0000;
    const WS_MINIMIZEBOX: isize = 0x0002_0000;
    const WS_MAXIMIZEBOX: isize = 0x0001_0000;
    const WS_SYSMENU: isize = 0x0008_0000;
    const WS_CAPTION: isize = 0x00C0_0000;
    const WS_POPUP: isize = 0x8000_0000u32 as isize;
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOZORDER: u32 = 0x0004;
    const SWP_FRAMECHANGED: u32 = 0x0020;

    unsafe extern "system" {
        fn GetWindowLongPtrW(hwnd: *mut core::ffi::c_void, index: i32) -> isize;
        fn SetWindowLongPtrW(hwnd: *mut core::ffi::c_void, index: i32, value: isize) -> isize;
        fn SetWindowPos(
            hwnd: *mut core::ffi::c_void,
            after: *mut core::ffi::c_void,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
    }

    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = win32.hwnd.get() as *mut core::ffi::c_void;

    // SAFETY: `hwnd` is winit's live window handle. `GWL_STYLE` reads a
    // `LONG_PTR`, which is `isize` — the type both pointer-width APIs below
    // take.
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
    if style == 0 {
        // An unreadable style is treated as "no bits yet": the frame bits
        // added below still make the window resizable, so startup continues.
        tracing::warn!(
            err = %std::io::Error::last_os_error(),
            "could not read the window style"
        );
    }
    let style = (style & !(WS_POPUP | WS_CAPTION))
        | WS_THICKFRAME
        | WS_MINIMIZEBOX
        | WS_MAXIMIZEBOX
        | WS_SYSMENU;

    // SAFETY: `style` holds only documented `WS_*` bits for the same live
    // window, and `SetWindowLongPtrW` takes a `LONG_PTR` in `isize`.
    let previous = unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, style) };
    if previous == 0 {
        // The window still opens; it just keeps its frameless style.
        tracing::warn!(
            err = %std::io::Error::last_os_error(),
            "could not restore the window frame style"
        );
    }

    // SAFETY: a null `after` with `SWP_NOZORDER` does not reorder; the zeroed
    // position and size are ignored under `SWP_NOMOVE | SWP_NOSIZE`; and
    // `SWP_FRAMECHANGED` makes Windows recompute the non-client area for the
    // style written above.
    let placed = unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
        )
    };
    if placed == 0 {
        tracing::warn!(
            err = %std::io::Error::last_os_error(),
            "could not recompute the window frame"
        );
    }
}

/// Other platforms have no caption to remove here; winit's frameless window
/// already keeps its own resize handling.
#[cfg(not(target_os = "windows"))]
pub fn apply_custom_chrome(window: &winit::window::Window) {
    let _ = window;
}

/// Opens `path` with the platform's default handler.
///
/// The child is not waited for: the opener outlives the call and the window
/// must keep painting. A spawn failure is returned so the caller can log it.
/// A spawn itself usually succeeds even when no handler is installed: the
/// shell then reports the missing association, which the user sees in its own
/// message rather than in the terminal's log.
#[cfg(target_os = "windows")]
pub fn open_path(path: &std::path::Path) -> std::io::Result<()> {
    // `start` reads its first quoted argument as the window title, so the
    // empty string is mandatory: without it a quoted path is swallowed as the
    // title and nothing opens.
    std::process::Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(path)
        .spawn()
        .map(|_| ())
}

/// Opens `path` with the platform's default handler.
///
/// See the Windows implementation for why the child is not waited for.
#[cfg(not(target_os = "windows"))]
pub fn open_path(path: &std::path::Path) -> std::io::Result<()> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener)
        .arg(path)
        .spawn()
        .map(|_| ())
}
