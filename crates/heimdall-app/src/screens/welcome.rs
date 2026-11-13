//! Welcome: recent vaults, Open vault, Create vault (Welcome mockup).

use eframe::egui::{self, pos2, vec2, Align2, CornerRadius, CursorIcon, Sense, Stroke, StrokeKind};
use heimdall_core::format::VaultId;
use heimdall_core::LockedVault;

use crate::app::Action;
use crate::support::{file_name, friendly_date, RecentVaults};
use crate::theme::*;
use crate::usb::{self, KeyFile, Scan};
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
    /// Vaults in the list, so a key file for one of them isn't pointed out.
    listed: Vec<VaultId>,
    /// Key files on connected USB drives. Hardened vaults aren't remembered,
    /// so a key whose vault isn't listed is a hint that it exists (§8).
    keys: Scan<Vec<KeyFile>>,
}

impl Welcome {
    pub fn new(recent: &RecentVaults) -> Self {
        let mut listed = Vec::new();
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
                let locked = heimdall_core::storage::read_vault(&e.path)
                    .ok()
                    .and_then(|b| LockedVault::parse(&b).ok());
                listed.extend(locked.as_ref().map(|l| *l.vault_id()));
                let needs_usb = locked.is_some_and(|l| l.requires_keyfile());
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
        Self {
            items,
            listed,
            keys: usb::key_file_scan(),
        }
    }

    /// "Key file test1.bifrost on USB (D:)…" for keys of unlisted vaults.
    fn key_hints(&self) -> Vec<String> {
        let files = self.keys.value.iter().flatten();
        files
            .filter(|f| f.keyfile.as_ref().is_some_and(|k| !self.listed.contains(k.vault_id())))
            .take(2)
            .map(|f| {
                let stem = f.path.file_stem().map(|s| s.to_string_lossy()).unwrap_or_default();
                // `test1-2.bifrost` belongs to `test1.heim`.
                let vault = match stem.rsplit_once('-') {
                    Some((base, n)) if n.parse::<u32>().is_ok() => base.to_owned(),
                    _ => stem.into_owned(),
                };
                format!(
                    "{} on {} is the key to a vault not listed here. Open it with Open vault… (look for {vault}.heim).",
                    f.file_name(),
                    usb::drive_label(&f.drive)
                )
            })
            .collect()
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Option<Action> {
        let mut action = None;
        self.keys.poll(ctx);
        let hints = self.key_hints();
        chrome::title_bar(ctx, |ui| chrome::title_text(ui, "Heimdall"));
        chrome::tagline_footer(ctx);

        let list_h = if self.items.is_empty() {
            0.0
        } else {
            30.0 + self.items.len() as f32 * 64.0 + 20.0
        };
        let hints_h = hints.len() as f32 * 44.0 + if hints.is_empty() { 0.0 } else { 14.0 };
        screens::centered_column(ctx, 440.0, 150.0 + 26.0 + 44.0 + list_h + 42.0 + hints_h, |ui| {
            ui.vertical_centered(|ui| {
                widgets::logo(ui, 8.0);
                widgets::version_label(ui);
            });
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
                let create =
                    widgets::button(ui, ButtonKind::Primary, "Create vault", true, vec2(215.0, 42.0));
                if self.items.is_empty() {
                    crate::tips::show(
                        ctx,
                        crate::tips::Tip::Welcome,
                        create.rect,
                        crate::tips::Side::Below,
                    );
                }
                if create.clicked() {
                    action = Some(Action::Create);
                }
            });
            if !hints.is_empty() {
                ui.add_space(14.0);
            }
            for hint in &hints {
                ui.scope(|ui| {
                    ui.set_max_width(440.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(hint)
                                .font(crate::theme::ui(12.0))
                                .color(TEXT_MUTED),
                        )
                        .wrap(),
                    );
                });
                ui.add_space(8.0);
            }
        });
        action
    }
}
