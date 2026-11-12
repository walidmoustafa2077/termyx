//! The project's support page: the one address the UI links to.

use std::io;

/// Where the `Support` menu row, palette command, and About field point.
///
/// One constant, because the destination is expected to move. A payment page
/// replaces this the day one exists, and nothing else in the app changes.
pub(crate) const SUPPORT_URL: &str =
    "https://github.com/walidmoustafa2077/termyx#supporting-termyx";

/// The address as it reads on the About page, without the scheme or anchor.
///
/// The About grid has no hyperlinks, so the field has to read as a hint rather
/// than as a URL bar.
pub(crate) fn display() -> &'static str {
    let page = SUPPORT_URL.split('#').next().unwrap_or(SUPPORT_URL);
    page.trim_start_matches("https://")
}

#[cfg(target_os = "windows")]
pub(crate) fn open() -> io::Result<()> {
    use std::ptr;

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            window: *mut core::ffi::c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> isize;
    }

    const SW_SHOWNORMAL: i32 = 1;
    // `ShellExecuteW` reports success as any value above 32.
    const SUCCESS_FLOOR: isize = 32;

    let operation = wide("open");
    let file = wide(SUPPORT_URL);
    // SAFETY: both strings are NUL-terminated and outlive the call; the null
    // window, parameters, and directory are the documented "no owner, no
    // arguments" form.
    let code = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if code > SUCCESS_FLOOR {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "ShellExecuteW refused the support page with code {code}"
        )))
    }
}

/// A NUL-terminated UTF-16 copy of `text`, the shape the Win32 wide API wants.
#[cfg(target_os = "windows")]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn open() -> io::Result<()> {
    use std::process::Command;

    #[cfg(target_os = "macos")]
    const OPENER: &str = "open";
    #[cfg(not(target_os = "macos"))]
    const OPENER: &str = "xdg-open";

    Command::new(OPENER)
        .arg(SUPPORT_URL)
        .spawn()
        .map(|_child| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_display_address_drops_the_scheme_and_anchor() {
        // Short enough for a narrow About grid, and still the address a reader
        // would type after "https://".
        assert_eq!(display(), "github.com/walidmoustafa2077/termyx");
    }
}
