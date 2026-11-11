//! Native window integration that winit does not expose.

/// One side or corner of the resize frame.
///
/// The geometry helper below compiles on every platform so its tests can run
/// anywhere; only the Windows hit-test proc maps these to `HT*` codes.
#[cfg(any(target_os = "windows", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Edge {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Where a client-area point falls on the resize border, if anywhere.
///
/// Kept as pure arithmetic so the edge rules can be tested without a window. A
/// window narrower or shorter than two borders has no unambiguous edge, so
/// every point is interior there.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn resize_edge(x: i32, y: i32, width: i32, height: i32, border: i32) -> Option<Edge> {
    // The caller converts a screen point before asking, so a point that missed
    // the window can still reach this helper; it is outside the client rect and
    // on no border at all.
    if x < 0 || y < 0 || x >= width || y >= height {
        return None;
    }
    if border <= 0 || width < 2 * border || height < 2 * border {
        return None;
    }
    let left = x < border;
    let right = x >= width - border;
    let top = y < border;
    let bottom = y >= height - border;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(Edge::TopLeft),
        (_, true, true, _) => Some(Edge::TopRight),
        (true, _, _, true) => Some(Edge::BottomLeft),
        (_, true, _, true) => Some(Edge::BottomRight),
        (true, ..) => Some(Edge::Left),
        (_, true, ..) => Some(Edge::Right),
        (_, _, true, _) => Some(Edge::Top),
        (_, _, _, true) => Some(Edge::Bottom),
        _ => None,
    }
}

/// The physical width a logical resize border grows to at `dpi`.
///
/// The hit test works in physical pixels, so a fixed 6 px band would be only
/// four logical pixels wide on a 150% display. `0` is what `GetDpiForWindow`
/// returns when it cannot read the window; the standard 96 then keeps the
/// border at its unscaled width instead of erasing it.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn scaled_border(border: i32, dpi: u32) -> i32 {
    let dpi = if dpi == 0 { 96 } else { dpi as i32 };
    (border * dpi + 48) / 96
}

#[cfg(target_os = "windows")]
type Hwnd = *mut core::ffi::c_void;
#[cfg(target_os = "windows")]
type Wparam = usize;
#[cfg(target_os = "windows")]
type Lparam = isize;
#[cfg(target_os = "windows")]
type Lresult = isize;
#[cfg(target_os = "windows")]
type SubclassProc = unsafe extern "system" fn(Hwnd, u32, Wparam, Lparam, usize, usize) -> Lresult;

#[cfg(target_os = "windows")]
const WM_NCHITTEST: u32 = 0x0084;
/// Width of the invisible border that starts a resize, in physical pixels at
/// 96 DPI. `hit_test_edge` scales it by the window's current DPI.
#[cfg(target_os = "windows")]
const RESIZE_BORDER: i32 = 6;
#[cfg(target_os = "windows")]
const HTLEFT: Lresult = 10;
#[cfg(target_os = "windows")]
const HTRIGHT: Lresult = 11;
#[cfg(target_os = "windows")]
const HTTOP: Lresult = 12;
#[cfg(target_os = "windows")]
const HTTOPLEFT: Lresult = 13;
#[cfg(target_os = "windows")]
const HTTOPRIGHT: Lresult = 14;
#[cfg(target_os = "windows")]
const HTBOTTOM: Lresult = 15;
#[cfg(target_os = "windows")]
const HTBOTTOMLEFT: Lresult = 16;
#[cfg(target_os = "windows")]
const HTBOTTOMRIGHT: Lresult = 17;

#[cfg(target_os = "windows")]
#[repr(C)]
struct WinPoint {
    x: i32,
    y: i32,
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct WinRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[cfg(target_os = "windows")]
impl Edge {
    /// The `WM_NCHITTEST` code `DefWindowProc` turns into a resize loop.
    fn hit_test(self) -> Lresult {
        match self {
            Edge::Left => HTLEFT,
            Edge::Right => HTRIGHT,
            Edge::Top => HTTOP,
            Edge::TopLeft => HTTOPLEFT,
            Edge::TopRight => HTTOPRIGHT,
            Edge::Bottom => HTBOTTOM,
            Edge::BottomLeft => HTBOTTOMLEFT,
            Edge::BottomRight => HTBOTTOMRIGHT,
        }
    }
}

#[cfg(target_os = "windows")]
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(
        hwnd: Hwnd,
        subclass_proc: Option<SubclassProc>,
        id: usize,
        data: usize,
    ) -> i32;
    fn DefSubclassProc(hwnd: Hwnd, msg: u32, wparam: Wparam, lparam: Lparam) -> Lresult;
}

#[cfg(target_os = "windows")]
unsafe extern "system" {
    fn ScreenToClient(hwnd: Hwnd, point: *mut WinPoint) -> i32;
    fn GetClientRect(hwnd: Hwnd, rect: *mut WinRect) -> i32;
    fn IsZoomed(hwnd: Hwnd) -> i32;
    fn GetDpiForWindow(hwnd: Hwnd) -> u32;
}

/// Answers `WM_NCHITTEST` with the resize edge under the pointer.
///
/// winit removes the non-client area of an undecorated window, so
/// `DefWindowProc` never reports a frame edge on its own. This subclass supplies
/// the missing codes; every other message, and every interior point, is chained
/// back to winit's procedure.
///
/// # Safety
///
/// Windows calls this only on the thread owning the window, with the handle the
/// subclass was installed on.
#[cfg(target_os = "windows")]
unsafe extern "system" fn chrome_hit_test(
    hwnd: Hwnd,
    msg: u32,
    wparam: Wparam,
    lparam: Lparam,
    _id: usize,
    _data: usize,
) -> Lresult {
    if msg == WM_NCHITTEST && unsafe { IsZoomed(hwnd) } == 0 {
        // SAFETY: `lparam` is the screen point the `WM_NCHITTEST` contract
        // documents, and `hwnd` is the live window this subclass belongs to.
        if let Some(edge) = unsafe { hit_test_edge(hwnd, lparam) } {
            return edge.hit_test();
        }
    }
    // SAFETY: chains to the procedure this subclass replaced (winit's) for
    // every message and point this handler does not consume.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// The resize edge under a `WM_NCHITTEST` screen point, if the pointer is on
/// the client area's border.
///
/// # Safety
///
/// `hwnd` must be a live window handle and `lparam` a `WM_NCHITTEST` point.
#[cfg(target_os = "windows")]
unsafe fn hit_test_edge(hwnd: Hwnd, lparam: Lparam) -> Option<Edge> {
    // The message packs the signed screen coordinates into its two halves.
    let mut point = WinPoint {
        x: lparam as u16 as i16 as i32,
        y: (lparam >> 16) as u16 as i16 as i32,
    };
    // SAFETY: `point` is a writable `POINT` holding a screen coordinate, the
    // layout `ScreenToClient` converts, and `hwnd` is live.
    if unsafe { ScreenToClient(hwnd, &mut point) } == 0 {
        return None;
    }
    let mut rect = WinRect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: `rect` is a writable `RECT` for the same live window.
    if unsafe { GetClientRect(hwnd, &mut rect) } == 0 {
        return None;
    }
    // The DPI is read per message rather than cached at startup because the
    // window can move to a monitor with a different scale without restarting.
    // SAFETY: `hwnd` is the live window the hit test was sent to.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    resize_edge(
        point.x,
        point.y,
        rect.right - rect.left,
        rect.bottom - rect.top,
        scaled_border(RESIZE_BORDER, dpi),
    )
}

/// Restores the resize frame and gives the frameless window a resize border.
///
/// winit removes the non-client area of an undecorated window, so the style
/// bits alone cannot make `DefWindowProc` report frame edges. This clears
/// `WS_POPUP` and restores `WS_CAPTION` with `WS_THICKFRAME` and the
/// system-menu and minimise/maximise bits, recomputes the frame, and installs
/// the subclass that answers the missing edge hit-test.
///
/// `WS_CAPTION` stays set on purpose even though the client area covers the
/// whole window: it is what makes the Desktop Window Manager treat the window
/// as a normal framed one, so it gets Windows Terminal's rounded corners,
/// drop shadow and frame line without any `DwmSetWindowAttribute` call.
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
    let style = (style & !WS_POPUP)
        | WS_CAPTION
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

    // SAFETY: `hwnd` is winit's live window and the subclass is installed once
    // per window, right after creation; `chrome_hit_test` chains every message
    // it does not consume back to winit's procedure.
    if unsafe { SetWindowSubclass(hwnd, Some(chrome_hit_test), 0, 0) } == 0 {
        // Without the subclass the window still opens and paints; it just
        // cannot be resized from its border.
        tracing::warn!(
            err = %std::io::Error::last_os_error(),
            "could not install the resize hit-test subclass"
        );
    }
}

/// Asks the Desktop Window Manager for Windows 11's rounded window corners.
///
/// winit creates the window undecorated, so the system has no frame of its own
/// to round; asking explicitly is the only way to get the same soft corner as
/// a normal window. The border-colour attribute is deliberately left alone:
/// painting a custom frame line is what made the corners read as a drawn
/// outline instead of the system's edge.
#[cfg(target_os = "windows")]
pub fn apply_rounded_frame(window: &winit::window::Window) {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    /// `DWMWA_WINDOW_CORNER_PREFERENCE`.
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    /// `DWMWCP_ROUND`: round every corner to the system radius.
    const DWMWCP_ROUND: u32 = 2;

    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: *mut core::ffi::c_void,
            attribute: u32,
            value: *const core::ffi::c_void,
            size: u32,
        ) -> i32;
    }

    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = win32.hwnd.get() as *mut core::ffi::c_void;

    let preference = DWMWCP_ROUND;
    // SAFETY: `hwnd` is winit's live window handle and `preference` is a `u32`
    // that outlives the call; `size` is that value's size.
    let applied = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            std::ptr::from_ref(&preference).cast(),
            std::mem::size_of_val(&preference) as u32,
        )
    };
    // Windows 10 has no corner preference; the square corners there are the
    // platform's own look, so a failure is only worth a log line.
    if applied != 0 {
        tracing::warn!(
            err = %std::io::Error::last_os_error(),
            "could not request rounded window corners"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_corner_and_edge_midpoint_maps_to_its_edge() {
        // 6 px border on a 200 x 100 client area: four corners and four side
        // midpoints, eight points in all.
        let (w, h, b) = (200, 100, 6);
        assert_eq!(resize_edge(0, 0, w, h, b), Some(Edge::TopLeft));
        assert_eq!(resize_edge(w - 1, 0, w, h, b), Some(Edge::TopRight));
        assert_eq!(resize_edge(0, h - 1, w, h, b), Some(Edge::BottomLeft));
        assert_eq!(resize_edge(w - 1, h - 1, w, h, b), Some(Edge::BottomRight));
        assert_eq!(resize_edge(0, h / 2, w, h, b), Some(Edge::Left));
        assert_eq!(resize_edge(w - 1, h / 2, w, h, b), Some(Edge::Right));
        assert_eq!(resize_edge(w / 2, 0, w, h, b), Some(Edge::Top));
        assert_eq!(resize_edge(w / 2, h - 1, w, h, b), Some(Edge::Bottom));
    }

    #[test]
    fn the_resize_border_follows_the_display_scale() {
        // A fixed physical band would shrink to four logical pixels on a 150%
        // display, where the same drag is much harder to start.
        assert_eq!(scaled_border(6, 96), 6);
        assert_eq!(scaled_border(6, 120), 8);
        assert_eq!(scaled_border(6, 144), 9);
        assert_eq!(scaled_border(6, 192), 12);
        // `GetDpiForWindow` reports zero when it cannot read the window's DPI.
        assert_eq!(scaled_border(6, 0), 6);
    }

    #[test]
    fn an_interior_point_is_not_an_edge() {
        assert_eq!(resize_edge(100, 50, 200, 100, 6), None);
        // The border is a strict outer band: one pixel inside it is interior.
        assert_eq!(resize_edge(6, 6, 200, 100, 6), None);
    }

    #[test]
    fn a_point_outside_the_client_area_is_not_an_edge() {
        // `WM_NCHITTEST` converts the screen point to client space before this
        // helper sees it. A point past the right edge, or above-left of the
        // window, is outside the client rect and must fall through to winit
        // rather than start a resize from a border it is nowhere near.
        assert_eq!(resize_edge(240, 50, 200, 100, 6), None);
        assert_eq!(resize_edge(-10, -10, 200, 100, 6), None);
        // One pixel inside the top-left corner is still on the border.
        assert_eq!(resize_edge(1, 1, 200, 100, 6), Some(Edge::TopLeft));
    }

    #[test]
    fn a_window_too_small_for_two_borders_has_no_edges() {
        // 10 px is narrower and shorter than 2 * 6, so no point is unambiguous.
        assert_eq!(resize_edge(0, 0, 10, 10, 6), None);
        assert_eq!(resize_edge(5, 5, 10, 10, 6), None);
        assert_eq!(resize_edge(9, 9, 10, 10, 6), None);
    }
}
