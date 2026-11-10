//! Icons: the executable's own, and the tab strip's per-shell images.

use std::sync::OnceLock;

#[cfg(any(target_os = "windows", test))]
use std::path::PathBuf;

use anyhow::Context;

/// 256×256 RGBA rendering of `assets/icons/termyx.ico`.
const ICON_PNG: &[u8] = include_bytes!("../assets/icons/termyx.png");

/// One square RGBA8 icon, top row first, ready for the tab strip's blit.
#[derive(Clone)]
pub(crate) struct TabIcon {
    pub rgba: Vec<u8>,
    pub size: usize,
}

/// Decodes the bundled icon for the window manager.
pub fn window_icon() -> anyhow::Result<winit::window::Icon> {
    let (rgba, size) = rgba()?;
    winit::window::Icon::from_rgba(rgba, size, size).context("build window icon")
}

/// The bundled app icon as straight RGBA8 rows, top row first.
///
/// Decoded on demand: `window_icon` wants winit's own buffer type and the tab
/// strip wants raw bytes to downsample, so both build from one decoder.
pub fn rgba() -> anyhow::Result<(Vec<u8>, u32)> {
    decode_png(ICON_PNG)
}

/// The bundled app icon, decoded once for the strip to fall back on.
///
/// Every tab without a program icon stamps this, so decoding it per frame
/// would repeat the same PNG work for each tab; the clone is of the raw bytes,
/// and the tab strip keeps its own copy.
pub fn app_icon() -> Option<TabIcon> {
    static ICON: OnceLock<Option<TabIcon>> = OnceLock::new();
    ICON.get_or_init(|| match decode_png(ICON_PNG) {
        Ok((rgba, size)) => Some(TabIcon {
            rgba,
            size: size as usize,
        }),
        Err(error) => {
            tracing::warn!(%error, "the app icon could not be decoded");
            None
        }
    })
    .clone()
}

/// The icon of the shell program `program`, or `None` when it has none.
///
/// The profile names a program, not a path, so the icon comes from whatever
/// the shell would actually start: `PATH` is searched the same way
/// `CreateProcess` would. A program with no icon on disk is not an error; the
/// tab strip then keeps the bundled icon.
#[cfg(target_os = "windows")]
pub fn program_icon(program: &str) -> Option<TabIcon> {
    let path_var = std::env::var_os("PATH")?;
    let path = resolve_program(program, std::env::split_paths(&path_var))?;
    windows::icon(&path)
}

/// Non-Windows targets have no shell icons to read.
#[cfg(not(target_os = "windows"))]
pub fn program_icon(_program: &str) -> Option<TabIcon> {
    None
}

/// The full path `program` resolves to, or `None` when no search directory
/// holds it.
///
/// Compiled for the tests on every target too: it is the one part of the icon
/// lookup that can be checked without Windows, so it stays free of any
/// platform API.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn resolve_program(
    program: &str,
    search: impl Iterator<Item = PathBuf>,
) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    // A program that names its own location is used as given, exactly as a
    // shell would start it.
    if program.contains('/') || program.contains(std::path::MAIN_SEPARATOR) {
        return Some(PathBuf::from(program));
    }
    // Windows appends `.exe` to a bare name; a name that already carries an
    // extension must not gain a second one.
    let with_exe = std::path::Path::new(program)
        .extension()
        .is_none()
        .then(|| format!("{program}.exe"));
    for dir in search {
        // Prefer the name as typed so a launcher script next to the binary
        // wins over its `.exe`, the way `PATH` lookup does.
        let direct = dir.join(program);
        if direct.is_file() {
            return Some(direct);
        }
        if let Some(name) = &with_exe {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn decode_png(bytes: &[u8]) -> anyhow::Result<(Vec<u8>, u32)> {
    let mut reader = png::Decoder::new(bytes)
        .read_info()
        .context("read icon header")?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).context("decode icon")?;
    anyhow::ensure!(
        info.color_type == png::ColorType::Rgba,
        "icon must be RGBA, found {:?}",
        info.color_type
    );
    anyhow::ensure!(info.width == info.height, "icon must be square");
    buffer.truncate(info.buffer_size());
    Ok((buffer, info.width))
}

/// Reads one program's icon out of its executable's resources.
///
/// The shell API hands back an `HICON`, which is a GDI object rather than
/// pixels, so it goes through a bitmap and `GetDIBits` to reach the tab
/// strip's RGBA buffer.
#[cfg(target_os = "windows")]
mod windows {
    use super::TabIcon;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    type Handle = *mut core::ffi::c_void;

    /// `SHGFI_ICON`: fill `SHFILEINFOW::h_icon`.
    const SHGFI_ICON: u32 = 0x0000_0100;
    /// `SHGFI_SMALLICON`: ask for the 16 px shell icon, the size a tab shows.
    const SHGFI_SMALLICON: u32 = 0x0000_0001;
    /// `BI_RGB`: no compression, so a 32 bpp buffer is straight BGRA.
    const BI_RGB: u32 = 0;
    /// `DIB_RGB_COLORS`: the palette is literal RGB values.
    const DIB_RGB_COLORS: u32 = 0;
    /// `GetObjectW` size of `BITMAP`; the API writes exactly this many bytes.
    const BITMAP_SIZE: i32 = std::mem::size_of::<Bitmap>() as i32;

    #[repr(C)]
    struct ShFileInfoW {
        h_icon: Handle,
        i_icon: i32,
        attributes: u32,
        display_name: [u16; 260],
        type_name: [u16; 80],
    }

    #[repr(C)]
    struct IconInfo {
        is_icon: i32,
        hotspot_x: u32,
        hotspot_y: u32,
        mask: Handle,
        color: Handle,
    }

    #[repr(C)]
    struct Bitmap {
        kind: i32,
        width: i32,
        height: i32,
        width_bytes: i32,
        planes: u16,
        bits_per_pixel: u16,
        bits: *mut core::ffi::c_void,
    }

    #[repr(C)]
    struct BitmapInfoHeader {
        size: u32,
        width: i32,
        height: i32,
        planes: u16,
        bit_count: u16,
        compression: u32,
        image_size: u32,
        x_pels_per_meter: i32,
        y_pels_per_meter: i32,
        colors_used: u32,
        colors_important: u32,
    }

    #[repr(C)]
    struct BitmapInfo {
        header: BitmapInfoHeader,
        // 32 bpp BI_RGB reads no palette, but every sample sizes the header
        // with one `RGBQUAD` included, so the struct keeps that byte count.
        colors: [u32; 1],
    }

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHGetFileInfoW(
            path: *const u16,
            attributes: u32,
            info: *mut ShFileInfoW,
            info_size: u32,
            flags: u32,
        ) -> usize;
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetIconInfo(icon: Handle, info: *mut IconInfo) -> i32;
        fn DestroyIcon(icon: Handle) -> i32;
        fn GetDC(window: Handle) -> Handle;
        fn ReleaseDC(window: Handle, dc: Handle) -> i32;
    }

    #[link(name = "gdi32")]
    unsafe extern "system" {
        fn GetObjectW(object: Handle, size: i32, buffer: *mut core::ffi::c_void) -> i32;
        fn GetDIBits(
            dc: Handle,
            bitmap: Handle,
            start: u32,
            lines: u32,
            bits: *mut core::ffi::c_void,
            info: *mut BitmapInfo,
            usage: u32,
        ) -> i32;
        fn DeleteObject(object: Handle) -> i32;
    }

    /// Extracts `path`'s 16 px shell icon as RGBA rows.
    ///
    /// The strip draws tab icons at 16 px, so asking the shell for the same
    /// size Windows Terminal asks for keeps every pixel of the artist's work:
    /// a 32 px icon scaled down to 16 would be resampled twice.
    pub(super) fn icon(path: &Path) -> Option<TabIcon> {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut info = ShFileInfoW {
            h_icon: std::ptr::null_mut(),
            i_icon: 0,
            attributes: 0,
            display_name: [0; 260],
            type_name: [0; 80],
        };
        // SAFETY: `wide` is a NUL-terminated UTF-16 path and `info` is a
        // writable `SHFILEINFOW` of the size passed.
        let found = unsafe {
            SHGetFileInfoW(
                wide.as_ptr(),
                0,
                &mut info,
                std::mem::size_of::<ShFileInfoW>() as u32,
                SHGFI_ICON | SHGFI_SMALLICON,
            )
        };
        if found == 0 || info.h_icon.is_null() {
            return None;
        }
        // SAFETY: `info.h_icon` is the live icon the call above filled in.
        let icon = rgba(info.h_icon);
        // SAFETY: every path from here owned that icon and is done with it.
        unsafe { DestroyIcon(info.h_icon) };
        icon
    }

    /// The icon's colour bitmap as straight RGBA, releasing both bitmaps.
    fn rgba(icon: Handle) -> Option<TabIcon> {
        let mut info = IconInfo {
            is_icon: 0,
            hotspot_x: 0,
            hotspot_y: 0,
            mask: std::ptr::null_mut(),
            color: std::ptr::null_mut(),
        };
        // SAFETY: `icon` is a live `HICON` and `info` is a writable
        // `ICONINFO`.
        if unsafe { GetIconInfo(icon, &mut info) } == 0 {
            return None;
        }
        let pixels = color_bitmap(info.color);
        // SAFETY: `GetIconInfo` copies both bitmaps to the caller, so they are
        // this call's to delete; a null handle is skipped.
        unsafe {
            if !info.mask.is_null() {
                DeleteObject(info.mask);
            }
            if !info.color.is_null() {
                DeleteObject(info.color);
            }
        }
        pixels
    }

    /// Reads a 32 bpp colour bitmap into top-down RGBA rows.
    ///
    /// A monochrome icon has no colour bitmap; the strip then keeps the
    /// bundled icon rather than inventing pixels.
    fn color_bitmap(color: Handle) -> Option<TabIcon> {
        if color.is_null() {
            return None;
        }
        let mut bitmap = Bitmap {
            kind: 0,
            width: 0,
            height: 0,
            width_bytes: 0,
            planes: 0,
            bits_per_pixel: 0,
            bits: std::ptr::null_mut(),
        };
        // SAFETY: `bitmap` is a writable `BITMAP` and `BITMAP_SIZE` is exactly
        // the size `GetObjectW` documents for that type.
        if unsafe {
            GetObjectW(
                color,
                BITMAP_SIZE,
                (&mut bitmap as *mut Bitmap).cast::<core::ffi::c_void>(),
            )
        } == 0
        {
            return None;
        }
        if bitmap.width <= 0 || bitmap.height <= 0 {
            return None;
        }
        let width = bitmap.width as usize;
        let height = bitmap.height as usize;
        let mut rgba = vec![0u8; width * height * 4];
        let mut info = BitmapInfo {
            header: BitmapInfoHeader {
                size: std::mem::size_of::<BitmapInfoHeader>() as u32,
                width: bitmap.width,
                // A negative height asks for top-down rows, the order the tab
                // strip expects.
                height: -bitmap.height,
                planes: 1,
                bit_count: 32,
                compression: BI_RGB,
                image_size: 0,
                x_pels_per_meter: 0,
                y_pels_per_meter: 0,
                colors_used: 0,
                colors_important: 0,
            },
            colors: [0],
        };
        // SAFETY: a screen DC is valid for the read-only conversion, and null
        // asks for one covering the whole display.
        let dc = unsafe { GetDC(std::ptr::null_mut()) };
        if dc.is_null() {
            return None;
        }
        // SAFETY: `rgba` holds `width * height * 4` bytes, which is what the
        // header describes, and `info` is a writable `BITMAPINFO`.
        let lines = unsafe {
            GetDIBits(
                dc,
                color,
                0,
                bitmap.height as u32,
                rgba.as_mut_ptr().cast::<core::ffi::c_void>(),
                &mut info,
                DIB_RGB_COLORS,
            )
        };
        // SAFETY: `dc` came from `GetDC` above and is released once.
        unsafe { ReleaseDC(std::ptr::null_mut(), dc) };
        if lines == 0 {
            return None;
        }
        // `GetDIBits` writes BGRA for `BI_RGB`; the strip blits RGBA. The
        // alpha byte is already straight for icons, so only R and B swap.
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        Some(TabIcon { rgba, size: width })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_fixture() -> (Vec<u8>, u32) {
        decode_png(ICON_PNG).expect("decode icon")
    }

    #[test]
    fn bundled_icon_is_a_square_rgba_image() {
        let (rgba, size) = decode_fixture();

        assert_eq!(size, 256);
        assert_eq!(rgba.len(), 256 * 256 * 4);
    }

    #[test]
    fn decoding_rejects_a_non_png() {
        assert!(decode_png(b"not a png").is_err());
    }

    #[test]
    fn the_bundled_icon_is_available_as_rgba() {
        let (rgba, size) = rgba().expect("decode the bundled icon");
        assert_eq!(size, 256);
        assert_eq!(rgba.len(), (size * size * 4) as usize);
        assert!(
            rgba.chunks_exact(4).any(|px| px[3] > 0),
            "the icon is not transparent everywhere"
        );
    }

    #[test]
    fn the_app_icon_is_a_square_rgba_buffer_for_the_strip() {
        let icon = app_icon().expect("the bundled icon decodes");
        assert_eq!(icon.size, 256);
        assert_eq!(icon.rgba.len(), icon.size * icon.size * 4);
    }

    /// A directory unique to one test, cleared before use so a leftover file
    /// from an earlier run cannot satisfy the lookup.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("termyx-icon-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the scratch directory");
        dir
    }

    #[test]
    fn resolve_program_finds_an_existing_file_on_the_search_path() {
        let dir = scratch("finds");
        let program = dir.join("tool.exe");
        std::fs::write(&program, b"").expect("write the fake program");

        let found = resolve_program("tool.exe", [dir.clone()].into_iter());

        assert_eq!(found, Some(program));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolve_program_prefers_the_extension_less_match() {
        let dir = scratch("extensionless");
        let plain = dir.join("tool");
        let executable = dir.join("tool.exe");
        std::fs::write(&plain, b"").expect("write the launcher");
        std::fs::write(&executable, b"").expect("write the executable");

        // A bare name searches both spellings; the typed one wins.
        let found = resolve_program("tool", [dir.clone()].into_iter());

        assert_eq!(found, Some(plain));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolve_program_appends_exe_to_a_name_without_an_extension() {
        let dir = scratch("appends-exe");
        let executable = dir.join("tool.exe");
        std::fs::write(&executable, b"").expect("write the executable");

        let found = resolve_program("tool", [dir.clone()].into_iter());

        assert_eq!(found, Some(executable));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resolve_program_returns_none_for_a_miss() {
        let dir = scratch("miss");

        let found = resolve_program("absent.exe", [dir.clone()].into_iter());

        assert_eq!(found, None);
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn the_powershell_icon_is_a_square_rgba_buffer() {
        let icon = program_icon("powershell.exe").expect("Windows ships a PowerShell icon");

        assert_eq!(
            icon.rgba.len(),
            icon.size * icon.size * 4,
            "the icon must be one RGBA row per pixel"
        );
        assert_eq!(icon.size, 16, "the strip draws tab icons at 16 px");
        // The mask-only icons carry no alpha channel; a fully transparent buffer
        // would let the strip paint nothing at all.
        let opaque = icon.rgba.chunks_exact(4).filter(|px| px[3] > 200).count();
        assert!(
            opaque > 0,
            "a shell icon must have opaque pixels or the strip paints nothing"
        );
    }
}
