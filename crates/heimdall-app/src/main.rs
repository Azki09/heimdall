// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod chrome;
mod screens;
mod session;
mod support;
mod theme;
mod widgets;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Heimdall")
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
