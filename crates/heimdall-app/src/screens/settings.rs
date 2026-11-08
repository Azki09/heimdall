//! Settings: Security, Access, Vault, Editor, Shortcuts, Trash and About
//! (Settings mockups; Vault is DESIGN §10.9).
//! Access lives in its own module, `access.rs`.

use eframe::egui::{self, pos2, vec2, Align2, CornerRadius, CursorIcon, Sense, Stroke, StrokeKind, Ui};
use heimdall_core::content::{
    Font, LineEnding, Profile, SecuritySettings, SpellLanguage, TabInsert, TRASH_RETENTION_S,
};
use uuid::Uuid;

use crate::app::Action;
use crate::chrome;
use crate::screens::access::Access;
use crate::screens::confirm::{Outcome, PasswordCheck};
use crate::session::Session;
use crate::support::{duration_label, RecentVaults};
use crate::theme::*;
use crate::widgets::{self, setting_row, ButtonKind};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Page {
    #[default]
    Security,
    Access,
    Vault,
    Editor,
    Shortcuts,
    Trash,
    About,
}

/// A confirmation that is open over the page (DESIGN §15.5).
enum Confirm {
    EmptyTrash,
    DeleteNote(Uuid),
}

#[derive(Default)]
pub struct Settings {
    page: Page,
    confirm: Option<Confirm>,
    access: Access,
    /// Turning off "Hide window from screenshots" asks for the password.
    capture_check: Option<PasswordCheck>,
}

impl Settings {
    /// Opened from "All shortcuts" in the code mode tip.
    pub fn shortcuts() -> Self {
        Self {
            page: Page::Shortcuts,
            ..Default::default()
        }
    }

    /// See [`Access::holds_lock`].
    pub fn holds_lock(&self) -> bool {
        self.access.holds_lock()
    }

    pub fn ui(&mut self, ctx: &egui::Context, s: &mut Session, recent: &mut RecentVaults) -> Option<Action> {
        let mut action = None;
        self.access.poll(ctx, s, recent);
        let locked_in = self.access.blocks_navigation();
        // An open dropdown takes Escape first: it only closes the dropdown.
        if !locked_in
            && self.confirm.is_none()
            && self.capture_check.is_none()
            && !ctx.memory(|m| m.any_popup_open())
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            return Some(Action::Notes);
        }
        let title = format!("Settings · {}", s.file_name());
        chrome::title_bar(ctx, |ui| chrome::title_text(ui, &title));

        egui::SidePanel::left("settings_nav")
            .exact_width(240.0)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(BG).inner_margin(egui::Margin {
                left: 16,
                right: 16,
                top: 28,
                bottom: 28,
            }))
            .show_separator_line(false)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                if nav_item(ui, "Back to notes", false, true).clicked() && !locked_in {
                    action = Some(Action::Notes);
                }
                ui.add_space(18.0);
                for (page, label) in [
                    (Page::Security, "Security"),
                    (Page::Access, "Access"),
                    (Page::Vault, "Vault"),
                    (Page::Editor, "Editor"),
                    (Page::Shortcuts, "Shortcuts"),
                    (Page::Trash, "Trash"),
                    (Page::About, "About"),
                ] {
                    if nav_item(ui, label, self.page == page, false).clicked() && !locked_in {
                        self.page = page;
                    }
                }
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG).inner_margin(egui::Margin {
                left: 72,
                right: 40,
                top: 40,
                bottom: 24,
            }))
            .show(ctx, |ui| {
                let left = ui.max_rect().left() - 72.0;
                ui.painter().vline(
                    left,
                    ui.max_rect().y_range().expand(64.0),
                    Stroke::new(1.0_f32, DIVIDER),
                );
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_max_width(600.0);
                        if self.access.showing_code() {
                            self.access.code_ui(ui, s);
                            return;
                        }
                        match self.page {
                            Page::Security => {
                                security(ui, s, recent, &mut self.access, &mut self.capture_check)
                            }
                            Page::Access => self.access.page(ui, s),
                            Page::Vault => {
                                if let Some(a) = vault(ui, s) {
                                    action = Some(a);
                                }
                            }
                            Page::Editor => editor(ui, s),
                            Page::Shortcuts => shortcuts(ui),
                            Page::Trash => self.trash(ui, s),
                            Page::About => about(ui),
                        }
                    });
            });

        self.confirm_dialog(ctx, s);
        self.access.dialog_ui(ctx, s);
        if let Some(check) = self.capture_check.as_mut() {
            match check.ui(ctx, s) {
                Outcome::Open => {}
                Outcome::Canceled => self.capture_check = None,
                Outcome::Confirmed => {
                    self.capture_check = None;
                    s.vault.content.security.exclude_from_capture = false;
                    s.touch(None);
                }
            }
        }
        action
    }

    fn trash(&mut self, ui: &mut Ui, s: &mut Session) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                widgets::heading(ui, "Trash");
                widgets::label(
                    ui,
                    "Closed tabs stay here, encrypted, for 30 days.",
                    14.0,
                    TEXT_MUTED,
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                let any = !s.vault.content.trash.is_empty();
                if widgets::button(ui, ButtonKind::Secondary, "Empty trash", any, vec2(0.0, 34.0)).clicked() {
                    self.confirm = Some(Confirm::EmptyTrash);
                }
            });
        });
        ui.add_space(24.0);
        if s.vault.content.trash.is_empty() {
            widgets::label(ui, "The trash is empty.", 13.0, TEXT_FAINT);
            return;
        }
        let now = heimdall_core::content::now();
        let mut restore = None;
        let items: Vec<(Uuid, String, String, i64)> = s
            .vault
            .content
            .trash
            .iter()
            .rev()
            .map(|n| {
                let title = match n.display_title() {
                    "" => "Untitled".to_owned(),
                    t => t.chars().take(40).collect(),
                };
                let preview: String = n
                    .body
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty() && *l != n.display_title())
                    .take(1)
                    .collect::<String>()
                    .chars()
                    .take(60)
                    .collect();
                let left = TRASH_RETENTION_S - now.saturating_sub(n.deleted_at.unwrap_or(now));
                (n.id, title, preview, (left / 86_400).max(0) + 1)
            })
            .collect();
        for (id, title, preview, days) in items {
            let (rect, _) = ui.allocate_exact_size(vec2(600.0, 64.0), Sense::hover());
            let painter = ui.painter();
            painter.hline(rect.x_range(), rect.bottom(), Stroke::new(1.0_f32, DIVIDER));
            painter.text(
                pos2(rect.left(), rect.top() + 14.0),
                Align2::LEFT_TOP,
                &title,
                crate::theme::ui(14.0),
                TEXT,
            );
            painter.text(
                pos2(rect.left(), rect.top() + 36.0),
                Align2::LEFT_TOP,
                &preview,
                mono(12.0),
                TEXT_MUTED,
            );
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect.shrink2(vec2(0.0, 14.0)))
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
            );
            let (bin, bin_resp) = child.allocate_exact_size(vec2(30.0, 30.0), Sense::click());
            if bin_resp.hovered() {
                child
                    .painter()
                    .rect_filled(bin, CornerRadius::same(6), NAV_ACTIVE);
                child.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            widgets::icon_trash(
                child.painter(),
                bin.center(),
                if bin_resp.hovered() { DANGER } else { TEXT_MUTED },
            );
            if bin_resp.on_hover_text("Delete now").clicked() {
                self.confirm = Some(Confirm::DeleteNote(id));
            }
            child.add_space(4.0);
            if widgets::button(
                &mut child,
                ButtonKind::Secondary,
                "Restore",
                true,
                vec2(0.0, 32.0),
            )
            .clicked()
            {
                restore = Some(id);
            }
            child.add_space(12.0);
            widgets::label(
                &mut child,
                &format!("Deletes in {days} day{}", if days == 1 { "" } else { "s" }),
                12.0,
                TEXT_MUTED,
            );
        }
        if let Some(id) = restore {
            s.vault.content.restore_note(id);
            s.touch(None);
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context, s: &mut Session) {
        let Some(confirm) = &self.confirm else { return };
        let (title, body) = match confirm {
            Confirm::EmptyTrash => {
                let n = s.vault.content.trash.len();
                (
                    "Empty trash?".to_owned(),
                    format!(
                        "{n} note{} will be deleted permanently. This can't be undone.",
                        if n == 1 { "" } else { "s" }
                    ),
                )
            }
            Confirm::DeleteNote(_) => (
                "Delete this note?".to_owned(),
                "The note will be deleted permanently. This can't be undone.".to_owned(),
            ),
        };
        let mut close = false;
        let mut confirmed = false;
        let modal = egui::Modal::new(egui::Id::new("confirm_dialog"))
            .backdrop_color(egui::Color32::from_black_alpha(160))
            .frame(
                egui::Frame::NONE
                    .fill(BG)
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(egui::Margin::same(28)),
            )
            .show(ctx, |ui| {
                ui.set_width(420.0);
                ui.label(egui::RichText::new(&title).font(medium(18.0)).color(TEXT));
                ui.add_space(8.0);
                widgets::paragraph(ui, &body, 420.0);
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    widgets::label(ui, "—  ", 13.0, TEXT_FAINT);
                    widgets::label(
                        ui,
                        "Older copies of the vault file still contain them.",
                        13.0,
                        TEXT_MUTED,
                    );
                });
                ui.add_space(24.0);
                ui.horizontal(|ui| {
                    if widgets::button(ui, ButtonKind::Secondary, "Cancel", true, vec2(0.0, 40.0)).clicked() {
                        close = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::button(
                            ui,
                            ButtonKind::Danger,
                            "Delete permanently",
                            true,
                            vec2(0.0, 40.0),
                        )
                        .clicked()
                        {
                            confirmed = true;
                        }
                    });
                });
            });
        if modal.should_close() {
            close = true;
        }
        if confirmed {
            match confirm {
                Confirm::EmptyTrash => {
                    s.vault.content.empty_trash();
                }
                Confirm::DeleteNote(id) => {
                    s.vault.content.delete_from_trash(*id);
                }
            }
            s.touch(None);
            close = true;
        }
        if close {
            self.confirm = None;
        }
    }
}

fn nav_item(ui: &mut Ui, text: &str, selected: bool, back: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(208.0, 34.0), Sense::click());
    let painter = ui.painter();
    if selected || resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(6), NAV_ACTIVE);
    }
    let mut x = rect.left() + 12.0;
    let color = if selected || resp.hovered() {
        TEXT
    } else {
        TEXT_MUTED
    };
    if back {
        widgets::icon_chevron_left(painter, pos2(x + 6.0, rect.center().y), color);
        x += 22.0;
    }
    painter.text(
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        crate::theme::ui(13.0),
        color,
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp
}

fn combo<T: PartialEq + Copy>(ui: &mut Ui, id: &str, value: &mut T, options: &[(T, String)]) -> bool {
    let mut changed = false;
    let current = options
        .iter()
        .find(|(v, _)| v == value)
        .map(|(_, l)| l.clone())
        .unwrap_or_default();
    egui::ComboBox::from_id_salt(id)
        .selected_text(current)
        .width(140.0)
        .show_ui(ui, |ui| {
            for (v, label) in options {
                if ui.selectable_label(v == value, label).clicked() && v != value {
                    *value = *v;
                    changed = true;
                }
            }
        });
    changed
}

fn security(
    ui: &mut Ui,
    s: &mut Session,
    recent: &mut RecentVaults,
    access: &mut Access,
    capture_check: &mut Option<PasswordCheck>,
) {
    widgets::heading(ui, "Security");
    ui.add_space(24.0);
    let sec = s.vault.content.security.clone();
    let defaults = SecuritySettings::defaults_for(sec.profile);

    // Profile card.
    let (card, _) = ui.allocate_exact_size(vec2(600.0, 70.0), Sense::hover());
    ui.painter().rect_stroke(
        card,
        CornerRadius::same(8),
        Stroke::new(1.0_f32, BORDER),
        StrokeKind::Inside,
    );
    ui.painter().text(
        pos2(card.left() + 18.0, card.top() + 16.0),
        Align2::LEFT_TOP,
        "Profile",
        crate::theme::ui(12.0),
        TEXT_MUTED,
    );
    let profile = format!("{:?}", sec.profile);
    let galley = ui.painter().layout_no_wrap(profile, crate::theme::ui(15.0), TEXT);
    let pw = galley.size().x;
    ui.painter()
        .galley(pos2(card.left() + 18.0, card.top() + 36.0), galley, TEXT);
    if sec.is_custom() {
        ui.painter().text(
            pos2(card.left() + 18.0 + pw, card.top() + 36.0),
            Align2::LEFT_TOP,
            " · Custom",
            crate::theme::ui(15.0),
            TEXT_MUTED,
        );
    }
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(card.shrink(18.0))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    if widgets::button(
        &mut child,
        ButtonKind::Secondary,
        "Change profile",
        true,
        vec2(0.0, 34.0),
    )
    .clicked()
    {
        access.open_profile(sec.profile);
    }
    if sec.is_custom() {
        child.add_space(4.0);
        if widgets::button(
            &mut child,
            ButtonKind::Quiet,
            "Reset to profile defaults",
            true,
            vec2(0.0, 34.0),
        )
        .clicked()
        {
            s.vault.content.security = defaults.clone();
            sync_recent(s, recent);
            s.touch(None);
            return;
        }
    }
    ui.add_space(20.0);

    let mut sec = sec;
    let mut changed = false;

    let mut lock_opts: Vec<(Option<u32>, String)> = [60, 120, 300, 600, 900, 1800, 3600]
        .iter()
        .map(|&x| (Some(x), duration_label(x)))
        .collect();
    if sec.profile == Profile::Basic {
        lock_opts.push((None, "Never".into()));
    }
    let lock_note = match (sec.lock_after_s, defaults.lock_after_s) {
        (None, _) => Some(("Never locks by itself. Lock with Ctrl+L.".to_owned(), DANGER)),
        (Some(a), Some(d)) if a > d => Some((
            format!(
                "Longer than the {:?} default ({}).",
                sec.profile,
                duration_label(d)
            ),
            TEXT_MUTED,
        )),
        _ => None,
    };
    setting_row(
        ui,
        "Lock after inactivity",
        lock_note.as_ref().map(|(t, c)| (t.as_str(), *c)),
        |ui| {
            changed |= combo(ui, "lock_after", &mut sec.lock_after_s, &lock_opts);
        },
    );

    let clip_opts: Vec<(u32, String)> = [10, 20, 30, 60].iter().map(|&x| (x, format!("{x} s"))).collect();
    setting_row(
        ui,
        "Clear clipboard after",
        Some(("Only if it still holds what Heimdall copied.", TEXT_MUTED)),
        |ui| {
            changed |= combo(ui, "clip", &mut sec.clipboard_clear_s, &clip_opts);
        },
    );

    let secret_opts: Vec<(u32, String)> = [5, 10, 20, 30].iter().map(|&x| (x, format!("{x} s"))).collect();
    setting_row(
        ui,
        "Clear secrets from clipboard after",
        Some(("For copies that include a masked secret.", TEXT_MUTED)),
        |ui| {
            changed |= combo(ui, "secret_clip", &mut sec.secret_clear_s, &secret_opts);
        },
    );

    setting_row(ui, "Lock when minimized", None, |ui| {
        changed |= widgets::toggle(ui, &mut sec.lock_on_minimize).changed();
    });

    // Basic can turn these off; the other profiles always have them (§12).
    let basic = sec.profile == Profile::Basic;
    setting_row(
        ui,
        "Lock when Windows locks",
        Some(("Going to sleep always locks.", TEXT_MUTED)),
        |ui| {
            if basic {
                changed |= widgets::toggle(ui, &mut sec.lock_on_windows_lock).changed();
            } else {
                widgets::label(ui, "Always on", 13.0, TEXT_MUTED);
            }
        },
    );
    let capture_note = if sec.exclude_from_capture {
        (
            "Screenshots, recordings and screen sharing show this window black.",
            TEXT_MUTED,
        )
    } else {
        ("Screenshots and screen sharing can show your notes.", DANGER)
    };
    setting_row(ui, "Hide window from screenshots", Some(capture_note), |ui| {
        if !basic {
            widgets::label(ui, "Always on", 13.0, TEXT_MUTED);
            return;
        }
        let mut on = sec.exclude_from_capture;
        if widgets::toggle(ui, &mut on).changed() {
            if on {
                sec.exclude_from_capture = true;
                changed = true;
            } else {
                *capture_check = Some(PasswordCheck::new(
                    "Show this window in screenshots?",
                    "Screenshots, screen recordings and screen sharing (Zoom, Teams…) will be able to capture your notes, and so will any app that records the screen. While it's on, Heimdall shows black in them, in your own screenshots too.",
                    "Turn off",
                    true,
                ));
            }
        }
    });

    let mut remember_changed = false;
    setting_row(
        ui,
        "Remember recent vaults",
        Some((
            "Off: this vault's path is never saved and is removed from the Welcome list.",
            TEXT_MUTED,
        )),
        |ui| {
            remember_changed = widgets::toggle(ui, &mut sec.remember_recent).changed();
        },
    );

    if changed || remember_changed {
        s.vault.content.security = sec;
        if remember_changed {
            sync_recent(s, recent);
        }
        s.touch(None);
    }
}

/// Settings → Vault: export, import and backup copies (DESIGN §10.9).
fn vault(ui: &mut Ui, s: &mut Session) -> Option<Action> {
    let mut action = None;
    widgets::heading(ui, "Vault");
    widgets::label(
        ui,
        "Copies of your notes, inside and outside the vault.",
        14.0,
        TEXT_MUTED,
    );
    ui.add_space(24.0);

    widgets::label(ui, "Export and import", 12.0, TEXT_MUTED);
    setting_row(
        ui,
        "Export notes",
        Some((
            "To a new vault file with its own password and recovery code.",
            TEXT_MUTED,
        )),
        |ui| {
            if widgets::button(ui, ButtonKind::Secondary, "Export notes…", true, vec2(0.0, 34.0)).clicked()
            {
                action = Some(Action::ExportNotes(None));
            }
        },
    );
    setting_row(
        ui,
        "Import notes from a vault",
        Some(("Copies notes from another .heim file as new tabs.", TEXT_MUTED)),
        |ui| {
            if widgets::button(ui, ButtonKind::Secondary, "Import…", true, vec2(0.0, 34.0)).clicked() {
                action = Some(Action::ImportVault);
            }
        },
    );
    let sec = &mut s.vault.content.security;
    let basic = sec.profile == Profile::Basic;
    let mut changed = false;
    setting_row(
        ui,
        "Ask for password before exporting",
        Some(("Exporting takes notes out of the vault.", TEXT_MUTED)),
        |ui| {
            if basic {
                changed = widgets::toggle(ui, &mut sec.ask_password_export).changed();
            } else {
                widgets::label(ui, "Always on", 13.0, TEXT_MUTED);
            }
        },
    );
    if changed {
        s.touch(None);
    }
    action
}

pub(super) fn sync_recent(s: &Session, recent: &mut RecentVaults) {
    if s.vault.content.security.remember_recent {
        recent.touch(&s.path);
    } else {
        recent.remove(&s.path);
    }
}

fn editor(ui: &mut Ui, s: &mut Session) {
    widgets::heading(ui, "Editor");
    ui.add_space(12.0);
    let mut e = s.vault.content.editor.clone();
    let mut changed = false;

    setting_row(ui, "Font", None, |ui| {
        let sizes: Vec<(u8, String)> = (12..=20).map(|x| (x, x.to_string())).collect();
        egui::ComboBox::from_id_salt("font_size")
            .selected_text(e.font_size.to_string())
            .width(60.0)
            .show_ui(ui, |ui| {
                for (v, l) in &sizes {
                    if ui.selectable_label(*v == e.font_size, l).clicked() {
                        e.font_size = *v;
                        changed = true;
                    }
                }
            });
        let fonts = [
            (Font::JetBrainsMono, "JetBrains Mono".to_owned()),
            (Font::Consolas, "Consolas".to_owned()),
            (Font::Inter, "Inter".to_owned()),
            (Font::SegoeUi, "Segoe UI".to_owned()),
        ];
        changed |= combo(ui, "font", &mut e.font, &fonts);
    });
    setting_row(ui, "Tab key inserts", None, |ui| {
        let opts = [
            (TabInsert::Spaces(2), "2 spaces".to_owned()),
            (TabInsert::Spaces(4), "4 spaces".to_owned()),
            (TabInsert::Spaces(8), "8 spaces".to_owned()),
            (TabInsert::Tab, "Tab character".to_owned()),
        ];
        changed |= combo(ui, "tab", &mut e.tab_inserts, &opts);
    });
    setting_row(
        ui,
        "Line endings",
        Some((
            "Used when copying text out. Text is always stored as UTF-8.",
            TEXT_MUTED,
        )),
        |ui| {
            let opts = [
                (LineEnding::Crlf, "Windows (CRLF)".to_owned()),
                (LineEnding::Lf, "Unix (LF)".to_owned()),
            ];
            changed |= combo(ui, "eol", &mut e.line_ending, &opts);
        },
    );
    setting_row(ui, "Word wrap", None, |ui| {
        changed |= widgets::toggle(ui, &mut e.word_wrap).changed()
    });
    setting_row(
        ui,
        "Line numbers",
        Some(("Numbers in plain notes. Code mode always shows them.", TEXT_MUTED)),
        |ui| changed |= widgets::toggle(ui, &mut e.line_numbers).changed(),
    );
    setting_row(
        ui,
        "Spell check",
        Some((
            "Underlines unknown words; right-click one for suggestions. It never changes your text by itself. Off in code mode.",
            TEXT_MUTED,
        )),
        |ui| changed |= widgets::toggle(ui, &mut e.spell_check).changed(),
    );
    if e.spell_check {
        setting_row(
            ui,
            "Spell check language",
            Some(("Dictionaries are built in; nothing is downloaded.", TEXT_MUTED)),
            |ui| {
                let opts = [
                    (SpellLanguage::English, "English".to_owned()),
                    (SpellLanguage::Spanish, "Spanish".to_owned()),
                    (SpellLanguage::EnglishAndSpanish, "English + Spanish".to_owned()),
                ];
                changed |= combo(ui, "spell_language", &mut e.spell_language, &opts);
            },
        );
    }
    if e.spell_check && !s.vault.content.dictionary.is_empty() {
        personal_dictionary(ui, s);
    }
    setting_row(ui, "Status bar", None, |ui| {
        changed |= widgets::toggle(ui, &mut e.status_bar).changed()
    });
    setting_row(
        ui,
        "Autosave",
        Some((
            "Always encrypted. Changes are always saved before locking.",
            TEXT_MUTED,
        )),
        |ui| changed |= widgets::toggle(ui, &mut e.autosave).changed(),
    );
    setting_row(
        ui,
        "Reopen tabs on unlock",
        Some(("Return to the tab and cursor position you left.", TEXT_MUTED)),
        |ui| changed |= widgets::toggle(ui, &mut e.reopen_tabs).changed(),
    );
    setting_row(
        ui,
        "Code mode tip on new tabs",
        Some((
            "Reminds you that Ctrl+Shift+M turns a tab into a code editor.",
            TEXT_MUTED,
        )),
        |ui| changed |= widgets::toggle(ui, &mut e.code_tip).changed(),
    );
    setting_row(
        ui,
        "Mask pasted secrets",
        Some((
            "Pasted passwords, API keys and tokens become {*secrets*}, shown as dots. Undo right after pasting keeps them plain.",
            TEXT_MUTED,
        )),
        |ui| changed |= widgets::toggle(ui, &mut e.mask_pasted).changed(),
    );
    ui.add_space(12.0);
    widgets::label(ui, "Zoom: Ctrl + / Ctrl − / Ctrl 0.", 12.0, TEXT_FAINT);

    if changed {
        s.vault.content.editor = e;
        s.touch(None);
    }
}

/// The words added with "Add to dictionary"; stored encrypted in the vault.
fn personal_dictionary(ui: &mut Ui, s: &mut Session) {
    ui.add_space(14.0);
    widgets::label(ui, "Personal dictionary", 14.0, TEXT);
    ui.add_space(4.0);
    widgets::label(
        ui,
        "Saved inside this vault, encrypted. Click × to remove a word.",
        12.0,
        TEXT_MUTED,
    );
    ui.add_space(10.0);
    let mut remove = None;
    let width = ui.available_width();
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(width.min(600.0));
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for (i, word) in s.vault.content.dictionary.iter().enumerate() {
            let galley = ui
                .painter()
                .layout_no_wrap(word.clone(), crate::theme::ui(12.0), TEXT_BODY);
            let size = vec2(galley.size().x + 12.0 + 22.0, 26.0);
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            let x_rect =
                egui::Rect::from_center_size(pos2(rect.right() - 13.0, rect.center().y), vec2(18.0, 18.0));
            let x = ui.interact(x_rect, egui::Id::new(("dict_remove", i)), Sense::click());
            let painter = ui.painter();
            painter.rect(
                rect,
                CornerRadius::same(13),
                SURFACE,
                Stroke::new(1.0_f32, BORDER),
                StrokeKind::Inside,
            );
            painter.galley(
                pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0),
                galley,
                TEXT_BODY,
            );
            if x.hovered() {
                painter.circle_filled(x_rect.center(), 8.0, NAV_ACTIVE);
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            widgets::icon_close(
                painter,
                x_rect.center(),
                7.0,
                if x.hovered() { TEXT } else { TEXT_MUTED },
            );
            if x.on_hover_text("Remove").clicked() {
                remove = Some(i);
            }
        }
    });
    ui.add_space(14.0);
    let y = ui.cursor().top();
    ui.painter().hline(
        ui.min_rect().left()..=ui.min_rect().left() + width,
        y,
        Stroke::new(1.0_f32, DIVIDER),
    );
    ui.add_space(1.0);
    if let Some(i) = remove {
        let mut word = s.vault.content.dictionary.remove(i);
        zeroize::Zeroize::zeroize(&mut word);
        s.touch(None);
    }
}

/// Keyboard shortcuts (Settings · Shortcuts mockup). Read-only.
fn shortcuts(ui: &mut Ui) {
    widgets::heading(ui, "Keyboard shortcuts");
    ui.add_space(6.0);
    /// What a row does, and its key combinations.
    type Row = (&'static str, &'static [&'static [&'static str]]);
    let groups: [(&str, &[Row]); 5] = [
        (
            "Tabs",
            &[
                ("New tab", &[&["Ctrl", "T"]]),
                ("Close tab · moves the note to Trash", &[&["Ctrl", "W"]]),
                ("Reopen closed tab", &[&["Ctrl", "Shift", "T"]]),
                (
                    "Next / previous tab",
                    &[&["Ctrl", "Tab"], &["Ctrl", "Shift", "Tab"]],
                ),
                (
                    "Go to tab 1–8 / the last tab",
                    &[&["Ctrl", "1…8"], &["Ctrl", "9"]],
                ),
            ],
        ),
        (
            "Find",
            &[
                ("Find in this note", &[&["Ctrl", "F"]]),
                ("Next / previous match", &[&["F3"], &["Shift", "F3"]]),
                ("Find in all notes", &[&["Ctrl", "Shift", "F"]]),
            ],
        ),
        (
            "Secrets",
            &[
                ("Mark the selection as secret", &[&["Ctrl", "Shift", "H"]]),
                ("Delete a secret · next to it", &[&["Backspace"], &["Delete"]]),
            ],
        ),
        (
            "View",
            &[
                ("Code mode for this tab", &[&["Ctrl", "Shift", "M"]]),
                ("Word wrap for this tab", &[&["Alt", "Z"]]),
                (
                    "Zoom in / out / reset",
                    &[&["Ctrl", "+"], &["Ctrl", "−"], &["Ctrl", "0"]],
                ),
            ],
        ),
        (
            "Vault",
            &[
                ("Save all notes", &[&["Ctrl", "S"]]),
                ("Lock now", &[&["Ctrl", "L"]]),
                ("Settings", &[&["Ctrl", ","]]),
            ],
        ),
    ];
    for (title, rows) in groups {
        ui.add_space(14.0);
        widgets::label(ui, title, 12.0, TEXT_MUTED);
        ui.add_space(2.0);
        for (i, (label, combos)) in rows.iter().enumerate() {
            let (rect, _) = ui.allocate_exact_size(vec2(600.0, 36.0), Sense::hover());
            if i + 1 < rows.len() {
                ui.painter()
                    .hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0_f32, DIVIDER));
            }
            let (main, note) = label
                .split_once(" · ")
                .map_or((*label, None), |(a, b)| (a, Some(b)));
            let galley = ui
                .painter()
                .layout_no_wrap(main.to_owned(), crate::theme::ui(13.0), TEXT);
            let w = galley.size().x;
            ui.painter().galley(
                pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
                galley,
                TEXT,
            );
            if let Some(note) = note {
                ui.painter().text(
                    pos2(rect.left() + w, rect.center().y),
                    Align2::LEFT_CENTER,
                    format!(" · {note}"),
                    crate::theme::ui(13.0),
                    TEXT_MUTED,
                );
            }
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
            );
            child.spacing_mut().item_spacing.x = 4.0;
            for (k, keys) in combos.iter().enumerate().rev() {
                // Right to left: draw each combination's keys reversed.
                let reversed: Vec<&str> = keys.iter().rev().copied().collect();
                for key in reversed {
                    widgets::keys(&mut child, &[key]);
                }
                if k > 0 {
                    widgets::label(&mut child, " / ", 12.0, TEXT_FAINT);
                }
            }
        }
    }
}

fn about(ui: &mut Ui) {
    widgets::logo(ui, 5.0);
    ui.add_space(10.0);
    widgets::label(ui, &format!("v{}", env!("CARGO_PKG_VERSION")), 13.0, TEXT_MUTED);
    ui.add_space(20.0);

    let link_row = |ui: &mut Ui, title: &str, value: &str, link: Option<(&str, &str)>| {
        setting_row(ui, title, Some((value, TEXT_MUTED)), |ui| {
            if let Some((label, url)) = link {
                if widgets::link(ui, label, 13.0).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                }
            }
        });
    };
    link_row(
        ui,
        "License",
        "GPL-3.0",
        Some(("Full text", "https://www.gnu.org/licenses/gpl-3.0.html")),
    );
    link_row(
        ui,
        "Source code",
        "Anyone can read and audit it.",
        Some(("Repository", "https://github.com/heimdall-notes/heimdall")),
    );
    link_row(
        ui,
        "Updates",
        "Heimdall never connects to the network. Check the repository for new releases and update manually.",
        None,
    );
    link_row(
        ui,
        "Encryption",
        "Argon2id · XChaCha20-Poly1305 · HKDF-SHA256 · BIP39 recovery code (12 words, 128 bits) · .heim version 1",
        None,
    );

    ui.add_space(20.0);
    widgets::label(ui, "Open-source components", 14.0, TEXT);
    ui.add_space(8.0);
    let components = [
        ("argon2", "MIT / Apache-2.0"),
        ("chacha20poly1305", "MIT / Apache-2.0"),
        ("hkdf, sha2", "MIT / Apache-2.0"),
        ("zeroize, secrecy", "MIT / Apache-2.0"),
        ("serde, postcard", "MIT / Apache-2.0"),
        ("uuid", "MIT / Apache-2.0"),
        ("bip39", "CC0-1.0"),
        ("egui, eframe", "MIT / Apache-2.0"),
        ("rfd", "MIT"),
        ("arboard", "MIT / Apache-2.0"),
        ("zxcvbn", "MIT"),
        ("chrono", "MIT / Apache-2.0"),
        ("spellbook", "MPL-2.0"),
        ("en_US dictionary (SCOWL)", "Permissive"),
        ("es_MX dictionary", "GPL-3.0 / LGPL-3.0 / MPL-1.1"),
        ("Inter", "OFL-1.1"),
        ("JetBrains Mono", "OFL-1.1"),
    ];
    egui::Grid::new("components")
        .num_columns(4)
        .spacing(vec2(16.0, 6.0))
        .show(ui, |ui| {
            for (i, (name, license)) in components.iter().enumerate() {
                widgets::label(ui, name, 12.0, TEXT_BODY);
                widgets::label(ui, license, 12.0, TEXT_MUTED);
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });
    ui.add_space(18.0);
    widgets::label(ui, "Hail the Omnissiah!", 11.0, TEXT_FAINT);
    ui.add_space(28.0);
    widgets::label(ui, "No signal. No trace.", 12.0, TEXT_FAINT);
}
