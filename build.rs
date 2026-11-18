//! Embeds the window icon into the executable so Explorer and the taskbar
//! show the same image the window uses, and stages the console host that
//! forwards graphics sequences beside the executable.

/// The console host from the `Microsoft.Windows.Console.ConPTY` package. The
/// one in the OS drops the graphics sequences an application sends, so the
/// Windows Terminal build ships next to the app and is loaded in its place.
const CONSOLE_HOST: [&str; 2] = ["conpty.dll", "OpenConsole.exe"];

fn main() {
    println!("cargo:rerun-if-changed=assets/icons/termyx.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("assets/icons/termyx.ico");
    if let Err(err) = resource.compile() {
        panic!("failed to embed the Windows icon resource: {err}");
    }

    stage_console_host();
}

/// Copies the console host next to the executable, where the pty library looks
/// for it before falling back to the system one.
fn stage_console_host() {
    let source_dir = std::path::Path::new("assets/conpty/x64");
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let exe_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("the target profile directory sits above the build directory");

    for name in CONSOLE_HOST {
        let source = source_dir.join(name);
        println!("cargo:rerun-if-changed={}", source.display());
        if let Err(err) = std::fs::copy(&source, exe_dir.join(name)) {
            panic!("failed to stage {}: {err}", source.display());
        }
    }
}
