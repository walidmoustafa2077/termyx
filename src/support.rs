//! The project's support page: the one address the UI links to.

use std::io;

/// Where the `Support` menu row, palette command, and About field point.
///
/// One constant, because the destination is expected to move: a different
/// payment page replaces it in one edit and nothing else in the app changes.
pub(crate) const SUPPORT_URL: &str = "https://paypal.me/WalidMostafa77";

/// The address as it reads on the About page, without the scheme.
///
/// The About grid has no hyperlinks, so the field has to read as a hint rather
/// than as a URL bar.
pub(crate) fn display() -> &'static str {
    SUPPORT_URL.trim_start_matches("https://")
}

#[cfg(target_os = "windows")]
pub(crate) fn open() -> io::Result<()> {
    open_target(SUPPORT_URL)
}

/// Opens a URL or a file path with the OS handler, the same way the support
/// page opens: a link underlined in the grid arrives here on click.
#[cfg(target_os = "windows")]
pub(crate) fn open_target(target: &str) -> io::Result<()> {
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
    let file = wide(&expand_home(target));
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
            "ShellExecuteW refused {target} with code {code}"
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
    open_target(SUPPORT_URL)
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn open_target(target: &str) -> io::Result<()> {
    use std::process::Command;

    #[cfg(target_os = "macos")]
    const OPENER: &str = "open";
    #[cfg(not(target_os = "macos"))]
    const OPENER: &str = "xdg-open";

    Command::new(OPENER)
        .arg(expand_home(target))
        .spawn()
        .map(|_child| ())
}

/// Expands a leading `~/` to the user's home directory, so a link the shell
/// printed as `~/notes.txt` opens. Anything else passes through unchanged.
fn expand_home(target: &str) -> String {
    let Some(rest) = target
        .strip_prefix("~/")
        .or_else(|| target.strip_prefix("~\\"))
    else {
        return target.to_string();
    };
    #[cfg(target_os = "windows")]
    let home = std::env::var("USERPROFILE");
    #[cfg(not(target_os = "windows"))]
    let home = std::env::var("HOME");
    match home {
        Ok(home) => std::path::Path::new(&home)
            .join(rest)
            .to_string_lossy()
            .into_owned(),
        Err(_) => target.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_display_address_drops_the_scheme() {
        // Short enough for a narrow About grid, and still the address a reader
        // would type after "https://".
        assert_eq!(display(), "paypal.me/WalidMostafa77");
    }

    #[test]
    fn the_home_prefix_expands_to_the_user_home() {
        #[cfg(target_os = "windows")]
        let home = std::env::var("USERPROFILE").expect("USERPROFILE");
        #[cfg(not(target_os = "windows"))]
        let home = std::env::var("HOME").expect("HOME");
        assert_eq!(
            expand_home("~/notes.txt"),
            std::path::Path::new(&home)
                .join("notes.txt")
                .to_string_lossy()
        );
    }

    #[test]
    fn ordinary_targets_pass_through_the_home_expansion() {
        assert_eq!(expand_home("https://x.com"), "https://x.com");
        assert_eq!(expand_home(r"C:\x\y.txt"), r"C:\x\y.txt");
        assert_eq!(expand_home(r"\\server\share"), r"\\server\share");
    }
}
