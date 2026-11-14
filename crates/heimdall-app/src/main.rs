// No console window in release builds of heimdall.exe. heimdall.com (the
// "console" feature) is a console program: the terminal mode on Windows.
#![cfg_attr(
    all(not(debug_assertions), not(feature = "console")),
    windows_subsystem = "windows"
)]
// heimdall.com never opens the window itself (it starts heimdall.exe), so
// most of the app is unused there.
#![cfg_attr(all(windows, feature = "console"), allow(dead_code, unused_imports))]

mod app;
mod attempts;
mod chrome;
mod code;
mod focus;
mod logo_mark;
mod path_prompt;
mod plain;
mod screens;
mod search;
mod secrets;
mod session;
mod spell;
mod support;
// Terminal mode (DESIGN §17): Linux, and heimdall.com on Windows
// (`term_mode`, set by build.rs). Its text model and argument parser build
// everywhere so their tests run on every platform.
#[cfg_attr(not(term_mode), allow(dead_code))]
mod term;
mod theme;
mod tips;
mod usb;
mod vault_lock;
mod widgets;

fn main() -> std::process::ExitCode {
    heimdall_platform::init();
    // `heimdall` in a terminal, or with a note or a command, is the
    // terminal mode; the app menu (and `--gui`) opens the window.
    #[cfg(target_os = "linux")]
    {
        use std::io::IsTerminal;
        let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
        let appimage = std::env::var_os("APPIMAGE").is_some();
        match term::cli::parse(std::env::args_os().skip(1).collect(), terminal, appimage) {
            term::cli::Parsed::Gui => {}
            term::cli::Parsed::Run(cli) => return term::run(cli),
            term::cli::Parsed::Usage(text) => return term::usage(&text),
        }
    }
    window()
}

/// heimdall.com: the terminal mode, but the window is heimdall.exe's job.
#[cfg(all(windows, feature = "console"))]
fn window() -> std::process::ExitCode {
    use std::io::IsTerminal;
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    // A double-click opens a console just for us: that means the window.
    if args.is_empty() && heimdall_platform::console_is_ours() {
        return term::open_window();
    }
    let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    match term::cli::parse(args, terminal, false) {
        term::cli::Parsed::Gui => term::open_window(),
        term::cli::Parsed::Run(cli) => term::run(cli),
        term::cli::Parsed::Usage(text) => term::usage(&text),
    }
}

#[cfg(not(all(windows, feature = "console")))]
fn window() -> std::process::ExitCode {
    match gui() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Heimdall couldn't open its window: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn gui() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Heimdall")
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon/heimdall-256.png"))
                    .unwrap_or_default(),
            )
            // Linux desktops match the window to `heimdall.desktop` by it.
            .with_app_id("heimdall")
            .with_inner_size([1100.0, 680.0])
            .with_min_inner_size([760.0, 520.0])
            .with_decorations(false)
            .with_resizable(true),
        // Nothing is persisted outside the vault: no window state file.
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "Heimdall",
        options,
        Box::new(|cc| Ok(Box::new(app::HeimdallApp::new(cc)))),
    )
}
