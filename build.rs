// Embed the app icon and version info in the Windows executable, so Explorer,
// the taskbar and the "Apps" list show it. Only runs when building on Windows
// for Windows (cross-checks from other OSes skip it).
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Audio Interface");
        res.set("FileDescription", "Audio Interface: audio device manager");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the Windows icon: {e}");
        }
    }
}
