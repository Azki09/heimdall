//! Embeds the icon (and version info) in heimdall.exe on Windows, so
//! Explorer and the taskbar show the Heimdall logo. Nothing to do elsewhere:
//! Linux takes the icon from heimdall.desktop and the window icon set in
//! main.rs.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon/heimdall.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/heimdall.ico");
        res.set("FileDescription", "Heimdall");
        res.set("ProductName", "Heimdall");
        res.set("LegalCopyright", "GPL-3.0-only");
        if let Err(e) = res.compile() {
            panic!("couldn't embed the icon: {e}");
        }
    }
}
