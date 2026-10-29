pub mod access;
pub mod editor;
pub mod keys;
pub mod settings;
pub mod unlock;
pub mod welcome;
pub mod wizard;

use eframe::egui::{self, Ui};

use crate::theme::BG;

/// A full-height central panel with a column of `width` centered in it,
/// as on Welcome / Unlock.
pub fn centered_column(ctx: &egui::Context, width: f32, height: f32, add: impl FnOnce(&mut Ui)) {
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(BG))
        .show(ctx, |ui| {
            let avail = ui.available_rect_before_wrap();
            let top = avail.top() + ((avail.height() - height) / 2.0).max(16.0);
            let rect = egui::Rect::from_min_size(
                egui::pos2(avail.center().x - width / 2.0, top),
                egui::vec2(width, avail.bottom() - top),
            );
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            child.set_width(width);
            add(&mut child);
        });
}
