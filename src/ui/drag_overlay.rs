//! A borderless window that carries a dragged tab under the pointer.
//!
//! The tab strip lives inside one window, so a card painted into that window is
//! clipped the moment the pointer leaves its frame. This overlay is a layered
//! popup of its own, so the tab stays visible over the desktop and any other
//! window until the drop lands.

/// A tiny always-on-top window showing one image, without a frame or a taskbar
/// button.
///
/// The overlay never takes focus and ignores the mouse: the dragging window
/// keeps the capture and decides where the tab goes.
pub struct DragOverlay {
    #[cfg(windows)]
    hwnd: isize,
    #[cfg(windows)]
    /// The memory DC holding the card, reused while the drag moves it.
    dc: isize,
    #[cfg(windows)]
    bitmap: win::Hgdiobj,
    #[cfg(windows)]
    size: (i32, i32),
    #[cfg(not(windows))]
    _unused: (),
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use std::ptr;
    use std::sync::Once;

    pub type Hwnd = isize;
    pub type Hinstance = isize;
    pub type Hdc = isize;
    pub type Hgdiobj = *mut core::ffi::c_void;

    pub const WS_EX_LAYERED: u32 = 0x0008_0000;
    pub const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
    pub const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
    pub const WS_EX_TOPMOST: u32 = 0x0000_0008;
    pub const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
    pub const WS_POPUP: u32 = 0x8000_0000;
    pub const SW_SHOWNOACTIVATE: i32 = 4;
    pub const SW_HIDE: i32 = 0;
    pub const ULW_ALPHA: u32 = 2;
    pub const AC_SRC_OVER: u8 = 0;
    pub const AC_SRC_ALPHA: u8 = 1;
    pub const BI_RGB: u32 = 0;
    pub const DIB_RGB_COLORS: u32 = 0;

    #[repr(C)]
    pub struct WndClassExW {
        pub cb_size: u32,
        pub style: u32,
        pub wnd_proc: Option<unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize>,
        pub cls_extra: i32,
        pub wnd_extra: i32,
        pub instance: Hinstance,
        pub icon: isize,
        pub cursor: isize,
        pub background: isize,
        pub menu_name: *const u16,
        pub class_name: *const u16,
        pub icon_sm: isize,
    }

    #[repr(C)]
    pub struct Point {
        pub x: i32,
        pub y: i32,
    }

    #[repr(C)]
    pub struct Size {
        pub cx: i32,
        pub cy: i32,
    }

    #[repr(C)]
    pub struct BlendFunction {
        pub blend_op: u8,
        pub blend_flags: u8,
        pub source_constant_alpha: u8,
        pub alpha_format: u8,
    }

    #[repr(C)]
    pub struct BitmapInfoHeader {
        pub size: u32,
        pub width: i32,
        pub height: i32,
        pub planes: u16,
        pub bit_count: u16,
        pub compression: u32,
        pub size_image: u32,
        pub x_pels: i32,
        pub y_pels: i32,
        pub clr_used: u32,
        pub clr_important: u32,
    }

    #[repr(C)]
    pub struct BitmapInfo {
        pub header: BitmapInfoHeader,
        pub colors: [u32; 1],
    }

    unsafe extern "system" {
        pub fn GetModuleHandleW(name: *const u16) -> Hinstance;
        pub fn RegisterClassExW(class: *const WndClassExW) -> u16;
        pub fn CreateWindowExW(
            ex_style: u32,
            class_name: *const u16,
            window_name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            parent: Hwnd,
            menu: isize,
            instance: Hinstance,
            param: *mut c_void,
        ) -> Hwnd;
        pub fn DefWindowProcW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
        pub fn DestroyWindow(hwnd: Hwnd) -> i32;
        pub fn ShowWindow(hwnd: Hwnd, command: i32) -> i32;
        pub fn UpdateLayeredWindow(
            hwnd: Hwnd,
            screen: Hdc,
            dst: *const Point,
            size: *const Size,
            src: Hdc,
            src_origin: *const Point,
            color_key: u32,
            blend: *const BlendFunction,
            flags: u32,
        ) -> i32;
        pub fn CreateCompatibleDC(dc: Hdc) -> Hdc;
        pub fn CreateDIBSection(
            dc: Hdc,
            info: *const BitmapInfo,
            usage: u32,
            bits: *mut *mut c_void,
            section: isize,
            offset: u32,
        ) -> Hgdiobj;
        pub fn SelectObject(dc: Hdc, object: Hgdiobj) -> Hgdiobj;
        #[link_name = "DeleteObject"]
        pub fn delete_object(object: Hgdiobj) -> i32;
        pub fn DeleteDC(dc: Hdc) -> i32;
    }

    /// The class name, as UTF-16 with its terminator.
    pub const CLASS: [u16; 20] = [
        b'T' as u16,
        b'e' as u16,
        b'r' as u16,
        b'm' as u16,
        b'y' as u16,
        b'x' as u16,
        b'D' as u16,
        b'r' as u16,
        b'a' as u16,
        b'g' as u16,
        b'O' as u16,
        b'v' as u16,
        b'e' as u16,
        b'r' as u16,
        b'l' as u16,
        b'a' as u16,
        b'y' as u16,
        b'\0' as u16,
        0,
        0,
    ];

    static REGISTER: Once = Once::new();

    /// Registers the overlay class once per process.
    ///
    /// A second registration would fail with `ERROR_CLASS_ALREADY_EXISTS`,
    /// which is the same usable state, so the result is only logged.
    pub fn register() {
        REGISTER.call_once(|| unsafe {
            let instance = GetModuleHandleW(ptr::null());
            let class = WndClassExW {
                cb_size: std::mem::size_of::<WndClassExW>() as u32,
                style: 0,
                wnd_proc: Some(wnd_proc),
                cls_extra: 0,
                wnd_extra: 0,
                instance,
                icon: 0,
                cursor: 0,
                background: 0,
                menu_name: ptr::null(),
                class_name: CLASS.as_ptr(),
                icon_sm: 0,
            };
            if RegisterClassExW(&class) == 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() != Some(1410) {
                    tracing::warn!(%err, "could not register the drag overlay class");
                }
            }
        });
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: Hwnd,
        msg: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    /// Creates the hidden window, leaving the caller to size and show it.
    pub fn create(title: *const u16) -> Hwnd {
        unsafe {
            CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST
                    | WS_EX_NOACTIVATE,
                CLASS.as_ptr(),
                title,
                WS_POPUP,
                0,
                0,
                0,
                0,
                0,
                0,
                GetModuleHandleW(ptr::null()),
                ptr::null_mut(),
            )
        }
    }
}

#[cfg(windows)]
impl DragOverlay {
    /// Creates the hidden overlay window, or `None` when Windows refuses.
    pub fn new() -> Option<Self> {
        win::register();
        let title = [0u16];
        let hwnd = win::create(title.as_ptr());
        if hwnd == 0 {
            let err = std::io::Error::last_os_error();
            tracing::warn!(%err, "could not create the drag overlay window");
            return None;
        }
        Some(Self {
            hwnd,
            dc: 0,
            bitmap: std::ptr::null_mut(),
            size: (0, 0),
        })
    }

    /// Sets the image the overlay shows, replacing any earlier one.
    ///
    /// `pixels` is premultiplied BGRA, row by row from the top, `width * height`
    /// pixels long. The overlay keeps the pixels in a DIB section so moving it
    /// only has to re-blit, not re-upload.
    pub fn set_card(&mut self, width: u32, height: u32, pixels: &[u8]) {
        let (width, height) = (width.max(1) as i32, height.max(1) as i32);
        let expected = (width as usize) * (height as usize) * 4;
        if pixels.len() < expected {
            tracing::warn!(len = pixels.len(), expected, "drag overlay image is short");
            return;
        }
        self.release_surface();
        unsafe {
            let dc = win::CreateCompatibleDC(0);
            if dc == 0 {
                return;
            }
            let info = win::BitmapInfo {
                header: win::BitmapInfoHeader {
                    size: std::mem::size_of::<win::BitmapInfoHeader>() as u32,
                    width,
                    // A negative height stores the rows top-down, which is the
                    // order the renderer hands them over in.
                    height: -height,
                    planes: 1,
                    bit_count: 32,
                    compression: win::BI_RGB,
                    size_image: 0,
                    x_pels: 0,
                    y_pels: 0,
                    clr_used: 0,
                    clr_important: 0,
                },
                colors: [0],
            };
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let bitmap = win::CreateDIBSection(dc, &info, win::DIB_RGB_COLORS, &mut bits, 0, 0);
            if bitmap.is_null() || bits.is_null() {
                win::DeleteDC(dc);
                return;
            }
            win::SelectObject(dc, bitmap);
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, expected);
            self.dc = dc;
            self.bitmap = bitmap;
            self.size = (width, height);
        }
    }

    /// Moves the overlay so the card's top-left corner sits at `x, y` on the
    /// virtual screen.
    pub fn place(&mut self, x: i32, y: i32) {
        if self.hwnd == 0 || self.dc == 0 {
            return;
        }
        let (width, height) = self.size;
        unsafe {
            let dst = win::Point { x, y };
            let size = win::Size {
                cx: width,
                cy: height,
            };
            let origin = win::Point { x: 0, y: 0 };
            let blend = win::BlendFunction {
                blend_op: win::AC_SRC_OVER,
                blend_flags: 0,
                source_constant_alpha: 255,
                alpha_format: win::AC_SRC_ALPHA,
            };
            let ok = win::UpdateLayeredWindow(
                self.hwnd,
                0,
                &dst,
                &size,
                self.dc,
                &origin,
                0,
                &blend,
                win::ULW_ALPHA,
            );
            if ok == 0 {
                let err = std::io::Error::last_os_error();
                tracing::warn!(%err, "could not place the drag overlay");
                return;
            }
            win::ShowWindow(self.hwnd, win::SW_SHOWNOACTIVATE);
        }
    }

    /// Hides the overlay without destroying it, so the next drag is cheap.
    pub fn hide(&mut self) {
        if self.hwnd == 0 {
            return;
        }
        unsafe {
            win::ShowWindow(self.hwnd, win::SW_HIDE);
        }
    }

    fn release_surface(&mut self) {
        unsafe {
            if !self.bitmap.is_null() {
                win::delete_object(self.bitmap);
                self.bitmap = std::ptr::null_mut();
            }
            if self.dc != 0 {
                win::DeleteDC(self.dc);
                self.dc = 0;
            }
        }
        self.size = (0, 0);
    }
}

#[cfg(windows)]
impl Drop for DragOverlay {
    fn drop(&mut self) {
        self.release_surface();
        if self.hwnd != 0 {
            unsafe {
                win::DestroyWindow(self.hwnd);
            }
            self.hwnd = 0;
        }
    }
}

#[cfg(not(windows))]
impl DragOverlay {
    pub fn new() -> Option<Self> {
        None
    }

    pub fn set_card(&mut self, _width: u32, _height: u32, _pixels: &[u8]) {}

    pub fn place(&mut self, _x: i32, _y: i32) {}

    pub fn hide(&mut self) {}
}
