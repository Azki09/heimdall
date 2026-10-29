//! Pieces shared by the wizard and Settings → Access: the recovery code and
//! its three-word check, the password strength meter, the drive picker and
//! the "connect each USB key you keep" list.

use std::path::PathBuf;

use eframe::egui::{self, pos2, vec2, Align2, CornerRadius, Sense, Stroke, StrokeKind, Ui};
use heimdall_core::RecoveryCode;

use crate::theme::*;
use crate::usb::Drive;
use crate::widgets;

// ---------- Recovery code ----------

/// The 12 words in three columns.
pub fn code_grid(ui: &mut Ui, code: &RecoveryCode, width: f32) {
    let words = code.words();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 4.0 * 36.0 + 24.0), Sense::hover());
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(8),
        SURFACE,
        Stroke::new(1.0_f32, BORDER),
        StrokeKind::Inside,
    );
    let col_w = (width - 40.0) / 3.0;
    for (i, word) in words.iter().enumerate() {
        let col = (i / 4) as f32;
        let row = (i % 4) as f32;
        let x = rect.left() + 20.0 + col * col_w;
        let y = rect.top() + 12.0 + row * 36.0 + 18.0;
        painter.text(
            pos2(x + 18.0, y),
            Align2::RIGHT_CENTER,
            (i + 1).to_string(),
            crate::theme::ui(12.0),
            TEXT_FAINT,
        );
        painter.text(pos2(x + 30.0, y), Align2::LEFT_CENTER, *word, mono(15.0), TEXT);
    }
}

/// The two warnings under the code.
pub fn code_notes(ui: &mut Ui) {
    for (lead, rest) in [
        (
            "This code opens the vault without your password.",
            " Keep it like a passport.",
        ),
        (
            "It is shown only once.",
            " It is never saved to disk or copied to the clipboard.",
        ),
    ] {
        bullet(ui, lead, rest);
    }
}

/// `—  lead rest`, with the lead brighter.
pub fn bullet(ui: &mut Ui, lead: &str, rest: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        widgets::label(ui, "—  ", 13.0, TEXT_FAINT);
        if !lead.is_empty() {
            widgets::label(ui, lead, 13.0, TEXT_BODY);
        }
        widgets::label(ui, rest, 13.0, TEXT_MUTED);
    });
    ui.add_space(4.0);
}

/// Three random words the user types back from their sheet (DESIGN §7).
pub struct CodeCheck {
    idx: [usize; 3],
    words: [String; 3],
}

impl CodeCheck {
    pub fn new() -> Self {
        let mut idx: Vec<usize> = Vec::new();
        while idx.len() < 3 {
            let r = heimdall_core::crypto::random_bytes::<1>()
                .map(|b| b[0] as usize % 12)
                .unwrap_or(idx.len() * 4);
            if !idx.contains(&r) {
                idx.push(r);
            }
        }
        idx.sort_unstable();
        Self {
            idx: [idx[0], idx[1], idx[2]],
            words: Default::default(),
        }
    }

    pub fn complete(&self, code: &RecoveryCode) -> bool {
        self.idx
            .iter()
            .zip(&self.words)
            .all(|(i, w)| code.check_word(*i, w))
    }

    pub fn ui(&mut self, ui: &mut Ui, code: &RecoveryCode, width: f32) {
        let mut first_empty = None;
        for k in 0..3 {
            let idx = self.idx[k];
            widgets::label(ui, &format!("Word {}", idx + 1), 12.0, TEXT_MUTED);
            let id = egui::Id::new(("check_word", k));
            widgets::text_field(ui, id, &mut self.words[k], width, None, "");
            let typed = self.words[k].trim();
            if !typed.is_empty() {
                let ok = code.check_word(idx, typed);
                let text = if ok {
                    "Correct".to_owned()
                } else {
                    format!("Doesn't match word {}", idx + 1)
                };
                ui.horizontal(|ui| {
                    if ok {
                        let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                        widgets::icon_check(ui.painter(), r.center(), 12.0, OK);
                    }
                    widgets::label(ui, &text, 12.0, TEXT_MUTED);
                });
            } else if first_empty.is_none() {
                first_empty = Some(id);
            }
            ui.add_space(14.0);
        }
        if ui.memory(|m| m.focused().is_none()) {
            if let Some(id) = first_empty {
                ui.memory_mut(|m| m.request_focus(id));
            }
        }
    }
}

impl Drop for CodeCheck {
    fn drop(&mut self) {
        for w in &mut self.words {
            zeroize::Zeroize::zeroize(w);
        }
    }
}

// ---------- Password strength ----------

/// `(score 0-4, label)` using zxcvbn, entirely offline.
pub fn estimate_strength(pw: &str) -> Option<(u8, String)> {
    let entropy = zxcvbn::zxcvbn(pw, &[]).ok()?;
    let score = entropy.score();
    let word = ["Very weak", "Weak", "Fair", "Good", "Strong"][score.min(4) as usize];
    // zxcvbn assumes 10k guesses/s for a slow hash; Argon2id at 256 MiB is far slower.
    let time = entropy
        .crack_times()
        .offline_slow_hashing_1e4_per_second()
        .to_string();
    let label = if score < 2 {
        format!("{word} · try a longer password")
    } else {
        format!("{word} · {time} to guess")
    };
    Some((score, label))
}

/// Minimum zxcvbn score for a new password (DESIGN §14.1).
pub const MIN_SCORE: u8 = 2;

pub fn strength_meter(ui: &mut Ui, strength: Option<&(u8, String)>, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 18.0), Sense::hover());
    let bars_w = width - 200.0;
    let filled = strength.map(|(s, _)| (*s).max(1) as usize).unwrap_or(0);
    let painter = ui.painter();
    for i in 0..4 {
        let w = (bars_w - 12.0) / 4.0;
        let r = egui::Rect::from_min_size(
            pos2(rect.left() + i as f32 * (w + 4.0), rect.center().y - 1.5),
            vec2(w, 3.0),
        );
        painter.rect_filled(
            r,
            CornerRadius::same(2),
            if i < filled { TEXT_SECONDARY } else { TRACK_OFF },
        );
    }
    if let Some((_, label)) = strength {
        painter.text(
            pos2(rect.right(), rect.center().y),
            Align2::RIGHT_CENTER,
            label,
            crate::theme::ui(12.0),
            TEXT_MUTED,
        );
    }
}

/// "Passwords match" under a confirmation field.
pub fn match_line(ui: &mut Ui, password: &str, confirm: &str) {
    if confirm.is_empty() {
        return;
    }
    let matches = confirm == password;
    ui.horizontal(|ui| {
        if matches {
            let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
            widgets::icon_check(ui.painter(), r.center(), 12.0, OK);
        }
        widgets::label(
            ui,
            if matches {
                "Passwords match"
            } else {
                "Passwords don't match"
            },
            12.0,
            TEXT_MUTED,
        );
    });
}

// ---------- USB drives ----------

/// The "Detected drives" list (CreateKey mockup). Returns true when
/// Refresh was clicked. `selected` holds the chosen drive's root.
pub fn drive_picker(
    ui: &mut Ui,
    drives: Option<&[Drive]>,
    selected: &mut Option<PathBuf>,
    width: f32,
) -> bool {
    let mut refresh = false;
    ui.horizontal(|ui| {
        ui.set_width(width);
        widgets::label(ui, "Detected drives", 12.0, TEXT_MUTED);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            refresh = widgets::link(ui, "Refresh", 12.0).clicked();
        });
    });
    ui.add_space(8.0);
    let drives = drives.unwrap_or_default();
    if drives.is_empty() {
        let (rect, _) = ui.allocate_exact_size(vec2(width, 52.0), Sense::hover());
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(8),
            Stroke::new(1.0_f32, BORDER),
            StrokeKind::Inside,
        );
        ui.painter().text(
            pos2(rect.left() + 18.0, rect.center().y),
            Align2::LEFT_CENTER,
            "Plug in a USB drive.",
            crate::theme::ui(13.0),
            TEXT_MUTED,
        );
        *selected = None;
        return refresh;
    }
    if !selected
        .as_ref()
        .is_some_and(|s| drives.iter().any(|d| &d.root == s))
    {
        *selected = Some(drives[0].root.clone());
    }
    for d in drives {
        let (rect, resp) = ui.allocate_exact_size(vec2(width, 52.0), Sense::click());
        let is_sel = selected.as_ref() == Some(&d.root);
        let painter = ui.painter().with_clip_rect(rect);
        let painter = &painter;
        painter.rect(
            rect,
            CornerRadius::same(8),
            if is_sel { SURFACE } else { BG },
            Stroke::new(1.0_f32, if is_sel { SELECTED_BORDER } else { BORDER }),
            StrokeKind::Inside,
        );
        let radio = pos2(rect.left() + 26.0, rect.center().y);
        painter.circle_stroke(radio, 7.0, Stroke::new(1.5_f32, TEXT_MUTED));
        if is_sel {
            painter.circle_filled(radio, 4.0, TEXT);
        }
        let name = painter.layout_no_wrap(d.name.clone(), crate::theme::ui(14.0), TEXT);
        let name_w = name.size().x;
        painter.galley(
            pos2(rect.left() + 48.0, rect.center().y - name.size().y / 2.0),
            name,
            TEXT,
        );
        painter.text(
            pos2(rect.left() + 48.0 + name_w + 14.0, rect.center().y),
            Align2::LEFT_CENTER,
            d.letter(),
            mono(13.0),
            TEXT_MUTED,
        );
        painter.text(
            pos2(rect.right() - 18.0, rect.center().y),
            Align2::RIGHT_CENTER,
            d.size_label(),
            crate::theme::ui(12.0),
            TEXT_MUTED,
        );
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            *selected = Some(d.root.clone());
        }
        ui.add_space(8.0);
    }
    refresh
}

const SELECTED_BORDER: egui::Color32 = egui::Color32::from_rgb(0x6A, 0x6A, 0x6A);

/// "Connect each USB key you keep": one row per key, Connected or not.
pub fn key_list(ui: &mut Ui, keys: &[(String, bool)], width: f32) {
    widgets::label(ui, "Connect each USB key you keep", 12.0, TEXT_MUTED);
    ui.add_space(8.0);
    let h = 40.0 * keys.len() as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(width, h), Sense::hover());
    let painter = ui.painter();
    painter.rect_stroke(
        rect,
        CornerRadius::same(6),
        Stroke::new(1.0_f32, BORDER),
        StrokeKind::Inside,
    );
    for (i, (name, connected)) in keys.iter().enumerate() {
        let y = rect.top() + 40.0 * i as f32 + 20.0;
        if i > 0 {
            painter.hline(rect.x_range(), y - 20.0, Stroke::new(1.0_f32, DIVIDER));
        }
        widgets::dot(
            painter,
            pos2(rect.left() + 16.0, y),
            if *connected { OK } else { TRACK_OFF },
        );
        painter.text(
            pos2(rect.left() + 30.0, y),
            Align2::LEFT_CENTER,
            name,
            crate::theme::ui(13.0),
            TEXT_BODY,
        );
        painter.text(
            pos2(rect.right() - 14.0, y),
            Align2::RIGHT_CENTER,
            if *connected { "Connected" } else { "Not connected" },
            crate::theme::ui(12.0),
            TEXT_MUTED,
        );
    }
}
