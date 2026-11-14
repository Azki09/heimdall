//! Embeds the icon (and version info) in heimdall.exe on Windows, so
//! Explorer and the taskbar show the Heimdall logo. Nothing to do elsewhere:
//! Linux takes the icon from heimdall.desktop and the window icon set in
//! main.rs.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon/heimdall.ico");
    // `term_mode`: this build has the terminal mode (DESIGN §17). Linux
    // always; Windows only in the console build, heimdall.com.
    println!("cargo::rustc-check-cfg=cfg(term_mode)");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os == "linux" || (os == "windows" && std::env::var_os("CARGO_FEATURE_CONSOLE").is_some()) {
        println!("cargo::rustc-cfg=term_mode");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/heimdall.ico");
        let console = std::env::var_os("CARGO_FEATURE_CONSOLE").is_some();
        res.set(
            "FileDescription",
            if console {
                "Heimdall (terminal)"
            } else {
                "Heimdall"
            },
        );
        res.set("ProductName", "Heimdall");
        res.set("LegalCopyright", "GPL-3.0-only");
        if let Err(e) = res.compile() {
            panic!("couldn't embed the icon: {e}");
        }
    }
}
