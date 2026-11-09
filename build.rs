//! Embeds the window icon into the executable so Explorer and the taskbar
//! show the same image the window uses.

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
}
