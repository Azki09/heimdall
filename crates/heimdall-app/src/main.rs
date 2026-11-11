// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
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
// Terminal mode is Linux only (DESIGN §17); its text model and argument
// parser build everywhere so their tests run on every platform.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod term;
mod theme;
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
