//! Welcome: recent vaults, Open vault, Create vault (Welcome mockup).

use eframe::egui::{self, pos2, vec2, Align2, CornerRadius, CursorIcon, Sense, Stroke, StrokeKind};
use heimdall_core::LockedVault;

use crate::app::Action;
use crate::support::{file_name, friendly_date, RecentVaults};
use crate::theme::*;
use crate::widgets::{self, ButtonKind};
use crate::{chrome, screens};

struct Item {
    path: std::path::PathBuf,
    name: String,
    detail: String,
    date: String,
}

pub struct Welcome {
    items: Vec<Item>,
}

impl Welcome {
    pub fn new(recent: &RecentVaults) -> Self {
        let items = recent
            .entries
            .iter()
            .filter(|e| e.path.is_file())
            .map(|e| {
                let folder = e
                    .path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                // Only the plaintext header is read, to say whether a USB key is needed.
                let needs_usb = heimdall_core::storage::read_vault(&e.path)
                    .ok()
                    .and_then(|b| LockedVault::parse(&b).ok())
                    .is_some_and(|l| l.requires_keyfile());
                let detail = if needs_usb {
                    format!("{folder} · USB key required")
                } else {
                    folder
                };
                Item {
                    path: e.path.clone(),
                    name: file_name(&e.path),
                    detail,
                    date: friendly_date(e.opened),
                }
            })
            .collect();
        Self { items }
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Option<Action> {
        let mut action = None;
        chrome::title_bar(ctx, |ui| chrome::title_text(ui, "Heimdall"));
        chrome::tagline_footer(ctx);

        let list_h = if self.items.is_empty() {
            0.0
        } else {
            30.0 + self.items.len() as f32 * 64.0 + 20.0
        };
        screens::centered_column(ctx, 440.0, 150.0 + 44.0 + list_h + 42.0, |ui| {
            ui.vertical_centered(|ui| widgets::logo(ui, 8.0));
            ui.add_space(44.0);

            if !self.items.is_empty() {
                widgets::label(ui, "Recent vaults", 12.0, TEXT_MUTED);
                ui.add_space(2.0);
                let (frame_rect, _) =
                    ui.allocate_exact_size(vec2(440.0, self.items.len() as f32 * 64.0), Sense::hover());
                ui.painter().rect_stroke(
                    frame_rect,
                    CornerRadius::same(8),
                    Stroke::new(1.0_f32, BORDER),
                    StrokeKind::Inside,
                );
                for (i, item) in self.items.iter().enumerate() {
                    let row = egui::Rect::from_min_size(
                        pos2(frame_rect.left(), frame_rect.top() + i as f32 * 64.0),
                        vec2(440.0, 64.0),
                    );
                    let resp = ui.interact(row, egui::Id::new(("recent", i)), Sense::click());
                    let painter = ui.painter();
                    if resp.hovered() {
                        painter.rect_filled(row.shrink(1.0), CornerRadius::same(7), NAV_ACTIVE);
                        ctx.set_cursor_icon(CursorIcon::PointingHand);
                    }
                    if i + 1 < self.items.len() {
                        painter.hline(row.x_range(), row.bottom(), Stroke::new(1.0_f32, DIVIDER));
                    }
                    painter.text(
                        pos2(row.left() + 16.0, row.top() + 14.0),
                        Align2::LEFT_TOP,
                        &item.name,
                        mono(14.0),
                        TEXT,
                    );
                    painter.text(
                        pos2(row.left() + 16.0, row.top() + 36.0),
                        Align2::LEFT_TOP,
                        &item.detail,
                        crate::theme::ui(12.0),
                        TEXT_MUTED,
                    );
                    painter.text(
                        pos2(row.right() - 16.0, row.center().y),
                        Align2::RIGHT_CENTER,
                        &item.date,
                        crate::theme::ui(12.0),
                        TEXT_MUTED,
                    );
                    if resp.clicked() {
                        action = Some(Action::Open(item.path.clone()));
                    }
                }
                ui.add_space(20.0);
            }

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if widgets::button(ui, ButtonKind::Secondary, "Open vault…", true, vec2(215.0, 42.0))
                    .clicked()
                {
                    action = Some(Action::PickFile);
                }
                if widgets::button(ui, ButtonKind::Primary, "Create vault", true, vec2(215.0, 42.0)).clicked()
                {
                    action = Some(Action::Create);
                }
            });
        });
        action
    }
}
