//! Settings → Access (ManageAccess mockup), the access dialogs (§15.5) and
//! changing profile (§6.2).
//!
//! Every change runs Argon2id several times, so it runs on a duplicate of
//! the vault in a background thread. When it succeeds the duplicate's slots
//! and Master Key replace the open vault's (the notes stay the live ones),
//! and the vault is saved right away. Changes that rotate the MK then show
//! the new recovery code, which must be confirmed before leaving.

use std::path::PathBuf;

use eframe::egui::{self, pos2, vec2, Align2, CornerRadius, Sense, Stroke, StrokeKind, Ui};
use heimdall_core::content::{Profile, SecuritySettings};
use heimdall_core::{KdfParams, Keyfile, RecoveryCode, SecretString, SlotId, SlotKind, Vault};

use crate::screens::keys::{self, CodeActions, CodeCheck};
use crate::session::{label_new_slots, Session};
use crate::support::{duration_label, error_message, Job, RecentVaults};
use crate::theme::*;
use crate::usb::{self, Drive, Scan, UsbKey};
use crate::widgets::{self, ButtonKind, SecretField};

const DIALOG_W: f32 = 440.0;
/// Room right of the dialog content for the scroll bar of a long dialog.
const GUTTER: f32 = 14.0;

enum Dialog {
    AddKey {
        password: SecretField,
        drive: Option<PathBuf>,
        name: String,
        /// The drive `name` was filled in from.
        named_for: Option<PathBuf>,
    },
    ChangePassword {
        current: SecretField,
        new: SecretField,
        confirm: SecretField,
        strength: Option<(u8, String)>,
    },
    Revoke {
        slot: SlotId,
        name: String,
        password: SecretField,
    },
    /// "Rotate keys now" or, with `regenerate`, "Regenerate recovery code":
    /// the same operation with different words (§15.5).
    Rotate {
        regenerate: bool,
        password: SecretField,
    },
    Profile {
        target: Profile,
        password: SecretField,
    },
}

/// What the background job hands back.
struct Done {
    vault: Vault,
    code: Option<RecoveryCode>,
    added: Option<UsbKey>,
}

/// What to do with the result on the UI thread.
struct Pending {
    before: Vec<SlotId>,
    access_changed: bool,
    security: Option<SecuritySettings>,
    /// Connected keys, to pick the one presence mode watches.
    keys: Vec<UsbKey>,
}

/// A new recovery code on screen, shown then checked (DESIGN §4.5).
struct NewCode {
    code: RecoveryCode,
    check: CodeCheck,
    confirming: bool,
    actions: CodeActions,
}

#[derive(Default)]
pub struct Access {
    dialog: Option<Dialog>,
    error: Option<String>,
    job: Option<Job<Result<Done, String>>>,
    pending: Option<Pending>,
    code: Option<NewCode>,
    keys: Option<Scan<Vec<UsbKey>>>,
    drives: Option<Scan<Vec<Drive>>>,
}

impl Access {
    /// Auto-lock waits while keys change or a new code is on screen.
    pub fn holds_lock(&self) -> bool {
        self.job.is_some() || self.code.is_some()
    }

    /// Esc and the side menu do nothing while a dialog or code is open.
    pub fn blocks_navigation(&self) -> bool {
        self.dialog.is_some() || self.holds_lock()
    }

    pub fn showing_code(&self) -> bool {
        self.code.is_some()
    }

    pub fn open_profile(&mut self, current: Profile) {
        let target = match current {
            Profile::Basic => Profile::Hardened,
            _ => Profile::Basic,
        };
        self.open(Dialog::Profile {
            target,
            password: SecretField::default(),
        });
    }

    fn open(&mut self, dialog: Dialog) {
        self.error = None;
        self.dialog = Some(dialog);
    }

    fn close(&mut self, ctx: &egui::Context) {
        self.dialog = None;
        self.error = None;
        widgets::forget_fields(ctx, &["dlg_pw", "dlg_cur", "dlg_new", "dlg_new2"]);
    }

    /// Call every frame: background scans and the result of a change.
    pub fn poll(&mut self, ctx: &egui::Context, s: &mut Session, recent: &mut RecentVaults) {
        if self.dialog.is_some() {
            self.keys
                .get_or_insert_with(|| usb::key_scan(*s.vault.vault_id()))
                .poll(ctx);
            if matches!(self.dialog, Some(Dialog::AddKey { .. })) {
                self.drives.get_or_insert_with(usb::drive_scan).poll(ctx);
            }
        }
        if let Some(result) = self.job.as_ref().and_then(Job::poll) {
            self.job = None;
            match (result, self.pending.take()) {
                (Ok(done), Some(p)) => self.finish(ctx, s, recent, done, p),
                (Err(e), _) => self.error = Some(e),
                (Ok(_), None) => {}
            }
        }
    }

    fn finish(
        &mut self,
        ctx: &egui::Context,
        s: &mut Session,
        recent: &mut RecentVaults,
        done: Done,
        p: Pending,
    ) {
        let mut vault = done.vault;
        // Keep the live notes and settings; take the new slots and MK.
        std::mem::swap(&mut vault.content, &mut s.vault.content);
        let profile_changed = p.security.is_some();
        if let Some(sec) = p.security {
            vault.content.security = sec;
        }
        label_new_slots(&mut vault, &p.before, done.added.as_ref());
        s.vault = vault;

        // Presence watches a key that still opens the vault.
        if !s.usb.as_ref().is_some_and(|k| s.key_slot(k).is_some()) {
            let next = p
                .keys
                .into_iter()
                .chain(done.added)
                .find(|k| s.key_slot(k).is_some());
            s.usb = next;
        }
        s.update_presence(ctx);

        s.access_changed |= p.access_changed;
        s.touch(None);
        s.save();
        if profile_changed {
            super::settings::sync_recent(s, recent);
        }
        self.close(ctx);
        if let Some(code) = done.code {
            self.code = Some(NewCode {
                code,
                check: CodeCheck::new(),
                confirming: false,
                actions: CodeActions::default(),
            });
        }
    }

    // ---------- Helpers over the open vault ----------

    /// Keys of this vault that are plugged in now and still enrolled.
    fn connected(&self, s: &Session) -> Vec<UsbKey> {
        let mut out: Vec<UsbKey> = Vec::new();
        let found = self.keys.as_ref().and_then(|k| k.value.as_ref());
        for k in found.into_iter().flatten() {
            let dup = out.iter().any(|o| o.keyfile.hint() == k.keyfile.hint());
            if !dup && s.key_slot(k).is_some() {
                out.push(k.clone());
            }
        }
        out
    }

    /// "Connect each USB key you keep": every key slot except `except`.
    fn needed(&self, s: &Session, except: Option<SlotId>) -> Vec<(String, bool)> {
        let connected = self.connected(s);
        s.vault
            .slot_infos()
            .into_iter()
            .filter(|i| Some(i.id) != except)
            .filter_map(|i| match i.kind {
                SlotKind::PasswordKeyfile { hint } => Some((
                    slot_name(s, i.id),
                    connected.iter().any(|k| k.keyfile.hint() == hint),
                )),
                _ => None,
            })
            .collect()
    }

    // ---------- The page ----------

    pub fn page(&mut self, ui: &mut Ui, s: &Session) {
        widgets::heading(ui, "Access");
        widgets::label(ui, "Every way this vault can be opened.", 14.0, TEXT_MUTED);
        ui.add_space(24.0);

        let mut infos = s.vault.slot_infos();
        infos.sort_by_key(|i| match i.kind {
            SlotKind::Password => 0,
            SlotKind::PasswordKeyfile { .. } => 1,
            SlotKind::Recovery => 2,
        });
        let has_password = infos.iter().any(|i| i.kind == SlotKind::Password);
        let key_count = infos
            .iter()
            .filter(|i| matches!(i.kind, SlotKind::PasswordKeyfile { .. }))
            .count();
        let watching = s.presence.as_ref().and(s.usb.as_ref()).map(|k| k.keyfile.hint());

        let width = ui.available_width().min(600.0);
        let row_h = 70.0;
        let (card, _) = ui.allocate_exact_size(vec2(width, row_h * infos.len() as f32), Sense::hover());
        ui.painter().rect_stroke(
            card,
            CornerRadius::same(8),
            Stroke::new(1.0_f32, BORDER),
            StrokeKind::Inside,
        );
        let mut open = None;
        for (n, info) in infos.iter().enumerate() {
            let rect = egui::Rect::from_min_size(
                pos2(card.left(), card.top() + row_h * n as f32),
                vec2(width, row_h),
            );
            if n > 0 {
                ui.painter()
                    .hline(rect.x_range(), rect.top(), Stroke::new(1.0_f32, DIVIDER));
            }
            let added = s.vault.content.label(info.id).map(|l| long_date(l.added));
            let (title, name, sub, button) = match info.kind {
                SlotKind::Password => {
                    let mut parts: Vec<String> = added.map(|d| format!("Added {d}")).into_iter().collect();
                    parts.push(if key_count > 0 {
                        "Opens without a USB key".into()
                    } else {
                        "Opens with your password".into()
                    });
                    ("Password", None, parts.join(" · "), None)
                }
                SlotKind::PasswordKeyfile { hint } => {
                    let mut parts: Vec<String> = added.map(|d| format!("Added {d}")).into_iter().collect();
                    if watching == Some(hint) {
                        parts.push("In use now".into());
                    }
                    if parts.is_empty() {
                        parts.push("Opens with your password and this key".into());
                    }
                    // Revoking must leave a way in with the password.
                    let can_revoke = has_password || key_count > 1;
                    let button = can_revoke.then_some(("Revoke", ButtonKind::DangerOutline));
                    (
                        "Password + USB key",
                        Some(slot_name(s, info.id)),
                        parts.join(" · "),
                        button,
                    )
                }
                SlotKind::Recovery => {
                    let mut parts: Vec<String> = added.map(|d| format!("Created {d}")).into_iter().collect();
                    parts.push("Opens without password or USB key".into());
                    (
                        "Recovery code",
                        None,
                        parts.join(" · "),
                        Some(("Regenerate", ButtonKind::Quiet)),
                    )
                }
            };
            let painter = ui.painter();
            let icon = pos2(rect.left() + 26.0, rect.center().y);
            match info.kind {
                SlotKind::Password => widgets::icon_lock(painter, icon, ICON),
                SlotKind::PasswordKeyfile { .. } => widgets::icon_usb(painter, icon, ICON),
                SlotKind::Recovery => widgets::icon_sheet(painter, icon, ICON),
            }
            let x = rect.left() + 52.0;
            let t = painter.layout_no_wrap(title.to_owned(), crate::theme::ui(14.0), TEXT);
            let tw = t.size().x;
            painter.galley(pos2(x, rect.top() + 16.0), t, TEXT);
            if let Some(name) = name {
                painter.text(
                    pos2(x + tw, rect.top() + 16.0),
                    Align2::LEFT_TOP,
                    format!(" · {name}"),
                    crate::theme::ui(14.0),
                    TEXT_MUTED,
                );
            }
            painter.text(
                pos2(x, rect.top() + 40.0),
                Align2::LEFT_TOP,
                sub,
                crate::theme::ui(12.0),
                TEXT_MUTED,
            );
            if let Some((label, kind)) = button {
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect.shrink2(vec2(18.0, 19.0)))
                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                );
                if widgets::button(&mut child, kind, label, true, vec2(0.0, 32.0)).clicked() {
                    open = Some(match info.kind {
                        SlotKind::Recovery => Dialog::Rotate {
                            regenerate: true,
                            password: SecretField::default(),
                        },
                        _ => Dialog::Revoke {
                            slot: info.id,
                            name: slot_name(s, info.id),
                            password: SecretField::default(),
                        },
                    });
                }
            }
        }

        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if widgets::button(ui, ButtonKind::Secondary, "Add USB key", true, vec2(0.0, 36.0)).clicked() {
                open = Some(Dialog::AddKey {
                    password: SecretField::default(),
                    drive: None,
                    name: String::new(),
                    named_for: None,
                });
            }
            if widgets::button(
                ui,
                ButtonKind::Secondary,
                "Change password",
                true,
                vec2(0.0, 36.0),
            )
            .clicked()
            {
                open = Some(Dialog::ChangePassword {
                    current: SecretField::default(),
                    new: SecretField::default(),
                    confirm: SecretField::default(),
                    strength: None,
                });
            }
        });

        ui.add_space(28.0);
        widgets::setting_row(
            ui,
            "Rotate keys",
            Some((
                "Re-encrypts the vault with a new key and issues a new recovery code. Old copies of this file stop working for anything written afterwards. Revoking access does this automatically.",
                TEXT_MUTED,
            )),
            |ui| {
                if widgets::button(ui, ButtonKind::Secondary, "Rotate now", true, vec2(0.0, 36.0)).clicked() {
                    open = Some(Dialog::Rotate {
                        regenerate: false,
                        password: SecretField::default(),
                    });
                }
            },
        );
        if let Some(d) = open {
            self.open(d);
        }
    }

    // ---------- New recovery code ----------

    /// Replaces the page while a new code is on screen.
    pub fn code_ui(&mut self, ui: &mut Ui, s: &Session) {
        let Some(nc) = &mut self.code else { return };
        let mut done = false;
        if !nc.confirming {
            widgets::heading(ui, "Write down your new recovery code");
            ui.add_space(2.0);
            widgets::paragraph(
                ui,
                "Your previous code no longer opens the vault. These 12 words replace it; destroy the old sheet.",
                460.0,
            );
            ui.add_space(24.0);
            keys::code_grid(ui, &nc.code, 460.0);
            ui.add_space(14.0);
            nc.actions.ui(
                ui,
                &nc.code,
                &s.file_name(),
                s.vault.content.security.clipboard_clear_s,
            );
            ui.add_space(16.0);
            keys::code_notes(ui);
            ui.add_space(24.0);
            if widgets::button(
                ui,
                ButtonKind::Primary,
                "I've written it down",
                true,
                vec2(0.0, 40.0),
            )
            .clicked()
            {
                nc.check = CodeCheck::new();
                nc.confirming = true;
            }
        } else {
            widgets::heading(ui, "Confirm your new recovery code");
            ui.add_space(2.0);
            widgets::paragraph(
                ui,
                "Type these three words from your new sheet, so we know it was written down correctly.",
                460.0,
            );
            ui.add_space(24.0);
            nc.check.ui(ui, &nc.code, 460.0);
            ui.add_space(10.0);
            let complete = nc.check.complete(&nc.code);
            ui.horizontal(|ui| {
                ui.set_width(460.0);
                if widgets::button(
                    ui,
                    ButtonKind::Secondary,
                    "Show the words again",
                    true,
                    vec2(0.0, 40.0),
                )
                .clicked()
                {
                    nc.confirming = false;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let enter = complete && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if widgets::button(ui, ButtonKind::Primary, "Done", complete, vec2(0.0, 40.0)).clicked()
                        || enter
                    {
                        done = true;
                    }
                });
            });
        }
        if done {
            self.code = None;
        }
    }

    // ---------- Dialogs ----------

    pub fn dialog_ui(&mut self, ctx: &egui::Context, s: &Session) {
        if self.dialog.is_none() {
            return;
        }
        let busy = self.job.is_some();
        let profile = s.vault.content.security.profile;
        let file = s.file_name();
        let stem = s
            .path
            .file_stem()
            .map(|x| x.to_string_lossy().into_owned())
            .unwrap_or_else(|| "vault".into());
        let connected = self.connected(s);
        let requires_key = s.vault.requires_keyfile();
        let key_slots = self.needed(s, None);
        let drives: Option<Vec<Drive>> = self.drives.as_ref().and_then(|d| d.value.clone());
        let error = self.error.clone();

        let mut submit = false;
        let mut cancel = false;
        let mut add_key_instead = false;
        let mut refresh_drives = false;

        // Revoking keeps every key but the revoked one.
        let revoked = match &self.dialog {
            Some(Dialog::Revoke { slot, .. }) => Some(*slot),
            _ => None,
        };
        let needed = self.needed(s, revoked);
        let all_connected = needed.iter().all(|(_, c)| *c);
        let Some(dialog) = self.dialog.as_mut() else {
            return;
        };

        let modal = egui::Modal::new(egui::Id::new("access_dialog"))
            .backdrop_color(egui::Color32::from_black_alpha(160))
            .frame(
                egui::Frame::NONE
                    .fill(BG)
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .corner_radius(CornerRadius::same(10))
                    // The right margin is split around the scroll bar's gutter.
                    .inner_margin(egui::Margin {
                        left: 28,
                        right: 28 - GUTTER as i8,
                        top: 28,
                        bottom: 28,
                    }),
            )
            .show(ctx, |ui| {
                ui.set_width(DIALOG_W + GUTTER);
                // Leaves room for the margins, an error line and the buttons.
                let max_h = (ctx.screen_rect().height() - 200.0).max(200.0);
                let mut action: Option<(&str, ButtonKind, bool)> = None;
                egui::ScrollArea::vertical()
                    .max_height(max_h)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.set_width(DIALOG_W);
                        action = Some(match dialog {
                            Dialog::AddKey {
                                password,
                                drive,
                                name,
                                named_for,
                            } => {
                                title(ui, "Add a USB key");
                                widgets::paragraph(
                                    ui,
                                    if requires_key {
                                        "The new key opens the vault with your password, like your current key. Keep it somewhere safe as a spare."
                                    } else {
                                        "The new key opens the vault with your password. While the profile is Basic, your password alone still opens it too."
                                    },
                                    DIALOG_W,
                                );
                                ui.add_space(18.0);
                                // A spare belongs on another drive: start on one without a key.
                                let has_key = |root: &PathBuf| connected.iter().any(|k| &k.drive.root == root);
                                if drive.is_none() {
                                    *drive = drives
                                        .iter()
                                        .flatten()
                                        .find(|d| !has_key(&d.root))
                                        .map(|d| d.root.clone());
                                }
                                refresh_drives =
                                    keys::drive_picker(ui, drives.as_deref(), drive, DIALOG_W);
                                if drive.as_ref().is_some_and(has_key) {
                                    widgets::label(
                                        ui,
                                        "This drive already holds a key for this vault.",
                                        12.0,
                                        TEXT_MUTED,
                                    );
                                }
                                if *drive != *named_for {
                                    let picked =
                                        drives.iter().flatten().find(|d| Some(&d.root) == drive.as_ref());
                                    if let Some(d) = picked {
                                        *name = d.name.clone();
                                    }
                                    *named_for = drive.clone();
                                }
                                ui.add_space(12.0);
                                widgets::label(ui, "Name", 12.0, TEXT_MUTED);
                                widgets::text_field(ui, egui::Id::new("key_name"), name, DIALOG_W, None, "");
                                ui.add_space(14.0);
                                let submit_key = password_field(ui, "Password", "dlg_pw", password, true);
                                ui.add_space(14.0);
                                keys::bullet(
                                    ui,
                                    "",
                                    &format!(
                                        "A small key file, {stem}.bifrost, is written to the drive. Nothing else on it is touched."
                                    ),
                                );
                                let ready = drive.is_some()
                                    && !name.trim().is_empty()
                                    && !password.text.is_empty()
                                    && (!requires_key || !connected.is_empty());
                                submit |= submit_key && ready;
                                ("Write key to drive", ButtonKind::Primary, ready)
                            }
                            Dialog::ChangePassword {
                                current,
                                new,
                                confirm,
                                strength,
                            } => {
                                title(ui, "Change password");
                                widgets::paragraph(
                                    ui,
                                    &format!(
                                        "The new password applies to every USB key, so the vault never has two passwords. Your recovery code stays the same, and {file}.bak is updated so the old password stops opening it."
                                    ),
                                    DIALOG_W,
                                );
                                ui.add_space(18.0);
                                if !needed.is_empty() {
                                    keys::key_list(ui, &needed, DIALOG_W);
                                    ui.add_space(6.0);
                                    widgets::label(ui, "Lost one? Revoke it in Access first.", 12.0, TEXT_MUTED);
                                    ui.add_space(14.0);
                                }
                                password_field(ui, "Current password", "dlg_cur", current, true);
                                ui.add_space(14.0);
                                widgets::label(ui, "New password", 12.0, TEXT_MUTED);
                                let resp = widgets::text_field(
                                    ui,
                                    egui::Id::new("dlg_new"),
                                    &mut new.text,
                                    DIALOG_W,
                                    Some(&mut new.reveal),
                                    "",
                                );
                                if resp.changed() {
                                    *strength = keys::estimate_strength(&new.text);
                                }
                                ui.add_space(4.0);
                                keys::strength_meter(ui, strength.as_ref(), DIALOG_W);
                                ui.add_space(12.0);
                                widgets::label(ui, "Confirm new password", 12.0, TEXT_MUTED);
                                let mut reveal = new.reveal;
                                let resp = widgets::text_field(
                                    ui,
                                    egui::Id::new("dlg_new2"),
                                    &mut confirm.text,
                                    DIALOG_W,
                                    Some(&mut reveal),
                                    "",
                                );
                                new.reveal = reveal;
                                ui.add_space(4.0);
                                keys::match_line(ui, &new.text, &confirm.text);
                                let ready = !current.text.is_empty()
                                    && strength.as_ref().is_some_and(|(sc, _)| *sc >= keys::MIN_SCORE)
                                    && new.text == confirm.text
                                    && all_connected;
                                submit |= ready && resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                ("Change password", ButtonKind::Primary, ready)
                            }
                            Dialog::Revoke { name, password, .. } => {
                                title(ui, &format!("Revoke {name}?"));
                                widgets::paragraph(
                                    ui,
                                    "This USB key will no longer open the vault. To make sure of that, Heimdall re-encrypts the vault with a new key.",
                                    DIALOG_W,
                                );
                                ui.add_space(12.0);
                                rotation_bullets(ui, &file);
                                keys::bullet(
                                    ui,
                                    "",
                                    "Copies of this file made before now (backups, cloud) still open with this key and show your notes as they are today. Nothing you write afterwards.",
                                );
                                ui.add_space(14.0);
                                if !needed.is_empty() {
                                    keys::key_list(ui, &needed, DIALOG_W);
                                    ui.add_space(14.0);
                                }
                                let enter = password_field(ui, "Password", "dlg_pw", password, true);
                                let ready = !password.text.is_empty() && all_connected;
                                submit |= enter && ready;
                                ("Revoke and rotate keys", ButtonKind::Danger, ready)
                            }
                            Dialog::Rotate { regenerate, password } => {
                                if *regenerate {
                                    title(ui, "Regenerate recovery code?");
                                    widgets::paragraph(
                                        ui,
                                        "Do this if someone may have seen your recovery sheet. Your current code stops working and the vault is re-encrypted with a new key.",
                                        DIALOG_W,
                                    );
                                    ui.add_space(12.0);
                                    keys::bullet(ui, "", "Next you write down the new 12 words and confirm three of them.");
                                    keys::bullet(
                                        ui,
                                        "",
                                        &format!("Destroy the old sheet. {file}.bak is replaced so the old code can't open it."),
                                    );
                                } else {
                                    title(ui, "Rotate keys now?");
                                    widgets::paragraph(
                                        ui,
                                        "Heimdall re-encrypts the vault with a new key. You keep opening it the same way: same password, same USB keys.",
                                        DIALOG_W,
                                    );
                                    ui.add_space(12.0);
                                    rotation_bullets(ui, &file);
                                }
                                ui.add_space(14.0);
                                if !needed.is_empty() {
                                    keys::key_list(ui, &needed, DIALOG_W);
                                    ui.add_space(14.0);
                                }
                                let enter = password_field(ui, "Password", "dlg_pw", password, true);
                                let ready = !password.text.is_empty() && all_connected;
                                submit |= enter && ready;
                                if *regenerate {
                                    ("Regenerate code", ButtonKind::Primary, ready)
                                } else {
                                    ("Rotate keys", ButtonKind::Primary, ready)
                                }
                            }
                            Dialog::Profile { target, password } => {
                                title(ui, "Change profile");
                                ui.add_space(4.0);
                                for p in [Profile::Basic, Profile::Hardened, Profile::Paranoid] {
                                    if profile_option(ui, p, *target == p, p == profile) {
                                        *target = p;
                                    }
                                }
                                ui.add_space(16.0);
                                let target = *target;
                                let upgrading = profile == Profile::Basic && target != Profile::Basic;
                                if target == profile {
                                    widgets::label(ui, "This is the current profile.", 13.0, TEXT_MUTED);
                                    ("Switch", ButtonKind::Primary, false)
                                } else if upgrading && key_slots.is_empty() {
                                    widgets::paragraph(
                                        ui,
                                        &format!(
                                            "{target:?} needs a USB key, and this vault has none yet. Add one first, then come back here."
                                        ),
                                        DIALOG_W,
                                    );
                                    ui.add_space(14.0);
                                    if widgets::button(ui, ButtonKind::Secondary, "Add USB key", true, vec2(0.0, 36.0))
                                        .clicked()
                                    {
                                        add_key_instead = true;
                                    }
                                    ("Switch", ButtonKind::Primary, false)
                                } else {
                                    profile_change_text(ui, profile, target, &key_slots);
                                    ui.add_space(14.0);
                                    let need_all = target != Profile::Basic;
                                    if need_all && !needed.is_empty() {
                                        keys::key_list(ui, &needed, DIALOG_W);
                                        ui.add_space(14.0);
                                    }
                                    let enter = password_field(ui, "Password", "dlg_pw", password, true);
                                    let keys_ok = if need_all {
                                        all_connected
                                    } else {
                                        !requires_key || !connected.is_empty()
                                    };
                                    let ready = !password.text.is_empty() && keys_ok;
                                    submit |= enter && ready;
                                    let label = match target {
                                        Profile::Basic => "Switch to Basic",
                                        Profile::Hardened => "Switch to Hardened",
                                        Profile::Paranoid => "Switch to Paranoid",
                                    };
                                    (label, ButtonKind::Primary, ready)
                                }
                            }
                        });
                    });

                ui.add_space(14.0);
                match (&error, busy) {
                    (_, true) => {
                        widgets::label(ui, "Working… this takes a few seconds.", 12.0, TEXT_MUTED);
                    }
                    (Some(e), false) => {
                        widgets::label(ui, e, 12.0, DANGER);
                    }
                    (None, false) => {}
                }
                ui.add_space(14.0);
                let (label, kind, ready) = action.unwrap_or(("OK", ButtonKind::Primary, false));
                // As wide as the content, leaving the scroll bar's gutter empty.
                let row = egui::Layout::left_to_right(egui::Align::Center);
                ui.allocate_ui_with_layout(vec2(DIALOG_W, 40.0), row, |ui| {
                    if widgets::button(ui, ButtonKind::Secondary, "Cancel", !busy, vec2(0.0, 40.0)).clicked() {
                        cancel = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if busy { "Working…" } else { label };
                        if widgets::button(ui, kind, label, ready && !busy, vec2(0.0, 40.0)).clicked() {
                            submit = true;
                        }
                    });
                });
            });

        if refresh_drives {
            if let Some(d) = self.drives.as_mut() {
                d.refresh();
            }
        }
        if (modal.should_close() || cancel) && !busy {
            self.close(ctx);
            return;
        }
        if add_key_instead {
            self.open(Dialog::AddKey {
                password: SecretField::default(),
                drive: None,
                name: String::new(),
                named_for: None,
            });
            return;
        }
        if submit && !busy {
            self.submit(ctx, s, connected, drives.unwrap_or_default(), stem);
        }
    }

    /// Starts the change described by the open dialog on a duplicate vault.
    fn submit(
        &mut self,
        ctx: &egui::Context,
        s: &Session,
        keys: Vec<UsbKey>,
        drives: Vec<Drive>,
        stem: String,
    ) {
        let Some(dialog) = self.dialog.as_mut() else {
            return;
        };
        self.error = None;
        let profile = s.vault.content.security.profile;
        let kdf = KdfParams::for_profile(profile);
        let mut dup = s.vault.duplicate();
        let before: Vec<SlotId> = s.vault.slot_infos().iter().map(|i| i.id).collect();
        let job_keys = keys.clone();
        let mut pending = Pending {
            before,
            access_changed: false,
            security: None,
            keys,
        };
        let em = |e: heimdall_core::Error| error_message(&e);

        let work: Box<dyn FnOnce() -> Result<Done, String> + Send> = match dialog {
            Dialog::AddKey {
                password,
                drive,
                name,
                ..
            } => {
                let Some(drive) = drives.into_iter().find(|d| Some(&d.root) == drive.as_ref()) else {
                    return;
                };
                let pw = password.take();
                let name = name.trim().to_owned();
                Box::new(move || {
                    let refs: Vec<&Keyfile> = job_keys.iter().map(|k| &k.keyfile).collect();
                    let kf = Keyfile::generate(*dup.vault_id()).map_err(em)?;
                    dup.add_keyfile_slot(&pw, &kf, &refs, kdf).map_err(em)?;
                    let mut added = usb::write_key(&drive, &stem, &kf).map_err(|e| {
                        format!("Couldn't write the key to {}: {e}", usb::drive_label(&drive))
                    })?;
                    added.drive.name = name;
                    Ok(Done {
                        vault: dup,
                        code: None,
                        added: Some(added),
                    })
                })
            }
            Dialog::ChangePassword { current, new, .. } => {
                // A wrong current password only clears that field; the new one
                // stays typed until the change succeeds and the dialog closes.
                let (cur, new) = (current.take(), SecretString::from(new.text.clone()));
                pending.access_changed = true;
                Box::new(move || {
                    let refs: Vec<&Keyfile> = job_keys.iter().map(|k| &k.keyfile).collect();
                    dup.change_password(&cur, &new, &refs, kdf).map_err(em)?;
                    Ok(Done {
                        vault: dup,
                        code: None,
                        added: None,
                    })
                })
            }
            Dialog::Revoke { slot, password, .. } => {
                let (pw, slot) = (password.take(), *slot);
                pending.access_changed = true;
                Box::new(move || {
                    let refs: Vec<&Keyfile> = job_keys.iter().map(|k| &k.keyfile).collect();
                    let code = dup.rotate_master_key(&pw, &refs, &[slot], kdf).map_err(em)?;
                    Ok(Done {
                        vault: dup,
                        code: Some(code),
                        added: None,
                    })
                })
            }
            Dialog::Rotate { password, .. } => {
                let pw = password.take();
                pending.access_changed = true;
                Box::new(move || {
                    let refs: Vec<&Keyfile> = job_keys.iter().map(|k| &k.keyfile).collect();
                    let code = dup.rotate_master_key(&pw, &refs, &[], kdf).map_err(em)?;
                    Ok(Done {
                        vault: dup,
                        code: Some(code),
                        added: None,
                    })
                })
            }
            Dialog::Profile { target, password } => {
                let (pw, target) = (password.take(), *target);
                pending.security = Some(SecuritySettings::defaults_for(target));
                let password_slots: Vec<SlotId> = s
                    .vault
                    .slot_infos()
                    .iter()
                    .filter(|i| i.kind == SlotKind::Password)
                    .map(|i| i.id)
                    .collect();
                let new_kdf = KdfParams::for_profile(target);
                // DESIGN §6.2: which change each switch needs.
                pending.access_changed = profile == Profile::Basic && target != Profile::Basic;
                Box::new(move || {
                    let refs: Vec<&Keyfile> = job_keys.iter().map(|k| &k.keyfile).collect();
                    let code = match (profile, target) {
                        // Downgrade: add a password-only slot; USB keys keep working.
                        (_, Profile::Basic) => {
                            dup.add_password_slot(&pw, &refs, new_kdf).map_err(em)?;
                            None
                        }
                        // Upgrade: drop the password-only slot, so the MK rotates (§8.4).
                        (Profile::Basic, _) => Some(
                            dup.rotate_master_key(&pw, &refs, &password_slots, new_kdf)
                                .map_err(em)?,
                        ),
                        // Hardened <-> Paranoid: same password, new Argon2id cost.
                        _ => {
                            dup.change_password(&pw, &pw, &refs, new_kdf).map_err(em)?;
                            None
                        }
                    };
                    Ok(Done {
                        vault: dup,
                        code,
                        added: None,
                    })
                })
            }
        };
        self.pending = Some(pending);
        self.job = Some(Job::spawn(ctx, work));
    }
}

/// The drive name stored for a key slot, or a generic one for keys added
/// before v0.3.
fn slot_name(s: &Session, id: SlotId) -> String {
    s.vault
        .content
        .label(id)
        .map(|l| l.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "USB key".into())
}

/// `Oct 27, 2026`
fn long_date(ts: i64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|t| t.format("%b %-d, %Y").to_string())
        .unwrap_or_default()
}

fn title(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(medium(18.0)).color(TEXT));
    ui.add_space(8.0);
}

/// Returns true when Enter was pressed in the field.
fn password_field(ui: &mut Ui, label: &str, id: &str, field: &mut SecretField, focus: bool) -> bool {
    widgets::label(ui, label, 12.0, TEXT_MUTED);
    let resp = widgets::text_field(
        ui,
        egui::Id::new(id),
        &mut field.text,
        DIALOG_W,
        Some(&mut field.reveal),
        "",
    );
    // Read Enter before taking focus back: Enter makes the field give up focus,
    // and re-focusing it first would hide that from `lost_focus`.
    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if focus && ui.memory(|m| m.focused().is_none()) {
        resp.request_focus();
    }
    // Long dialogs scroll; keep the field being typed in on screen.
    if resp.gained_focus() {
        resp.scroll_to_me(None);
    }
    enter
}

fn rotation_bullets(ui: &mut Ui, file: &str) {
    keys::bullet(
        ui,
        "",
        "You get a new recovery code. The current one stops working; you write down and confirm the new one next.",
    );
    keys::bullet(
        ui,
        "",
        &format!(
            "Old copies of this file stop working for anything written afterwards. {file}.bak is replaced."
        ),
    );
}

/// A compact radio row in the Change profile dialog.
fn profile_option(ui: &mut Ui, p: Profile, selected: bool, current: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(DIALOG_W, 36.0), Sense::click());
    let painter = ui.painter();
    if selected || resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(6), NAV_ACTIVE);
    }
    let radio = pos2(rect.left() + 16.0, rect.center().y);
    painter.circle_stroke(radio, 7.0, Stroke::new(1.5_f32, TEXT_MUTED));
    if selected {
        painter.circle_filled(radio, 4.0, TEXT);
    }
    painter.text(
        pos2(rect.left() + 34.0, rect.center().y),
        Align2::LEFT_CENTER,
        format!("{p:?}"),
        crate::theme::ui(14.0),
        TEXT,
    );
    if current {
        painter.text(
            pos2(rect.right() - 12.0, rect.center().y),
            Align2::RIGHT_CENTER,
            "Current",
            crate::theme::ui(12.0),
            TEXT_MUTED,
        );
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}

/// "What changes / What stays" for a profile switch (DialogProfile mockup).
fn profile_change_text(ui: &mut Ui, from: Profile, to: Profile, key_slots: &[(String, bool)]) {
    let d = SecuritySettings::defaults_for(to);
    let lock = d.lock_after_s.map_or("never lock by itself".to_owned(), |s| {
        format!("lock after {}", duration_label(s))
    });
    let mut defaults = format!(
        "Security settings reset to {to:?} defaults: {lock}, clear clipboard after {} s, remember recent vaults {}",
        d.clipboard_clear_s,
        if d.remember_recent { "on" } else { "off" },
    );
    if d.lock_on_minimize {
        defaults.push_str(", lock when minimized");
    }
    defaults.push('.');
    let names = key_slots
        .iter()
        .map(|(n, _)| n.as_str())
        .collect::<Vec<_>>()
        .join(" and ");

    let section = |ui: &mut Ui, heading: &str| {
        ui.add_space(6.0);
        widgets::label(ui, heading, 12.0, TEXT_MUTED);
        ui.add_space(6.0);
    };
    match (from, to) {
        (_, Profile::Basic) => {
            widgets::paragraph(
                ui,
                "Your password alone will open this vault. USB keys become optional.",
                DIALOG_W,
            );
            section(ui, "What changes");
            keys::bullet(ui, "", "Unplugging a USB key no longer locks the vault.");
            keys::bullet(ui, "", &defaults);
            section(ui, "What stays");
            keys::bullet(
                ui,
                "",
                &if key_slots.len() == 1 {
                    format!("{names} still opens the vault. To remove it, revoke it in Access.")
                } else {
                    format!("{names} still open the vault. To remove them, revoke them in Access.")
                },
            );
            keys::bullet(
                ui,
                "",
                "Your recovery code. Nothing is re-encrypted with a new key.",
            );
            ui.add_space(6.0);
            widgets::paragraph(
                ui,
                "Copies of this file made before now still need a USB key. Copies saved from now on open with the password only.",
                DIALOG_W,
            );
        }
        (Profile::Basic, _) => {
            widgets::paragraph(
                ui,
                "Only your password together with a USB key will open this vault.",
                DIALOG_W,
            );
            section(ui, "What changes");
            keys::bullet(
                ui,
                "",
                "The password-only access is removed, so Heimdall re-encrypts the vault with a new key.",
            );
            keys::bullet(
                ui,
                "",
                "You get a new recovery code; you write down and confirm it next.",
            );
            keys::bullet(ui, "", "Unplugging the USB key locks the vault.");
            keys::bullet(ui, "", &defaults);
            if to == Profile::Paranoid {
                keys::bullet(
                    ui,
                    "",
                    "Unlocking takes longer: the password is stretched with 512 MiB of memory.",
                );
            }
            ui.add_space(6.0);
            widgets::paragraph(
                ui,
                "Copies of this file made before now still open with the password alone, but not what you write afterwards.",
                DIALOG_W,
            );
        }
        _ => {
            widgets::paragraph(
                ui,
                "You keep opening the vault the same way: same password, same USB keys.",
                DIALOG_W,
            );
            section(ui, "What changes");
            if to == Profile::Paranoid {
                keys::bullet(ui, "", "Unlocking takes longer: the password is stretched with 512 MiB of memory instead of 256 MiB.");
            } else {
                keys::bullet(ui, "", "Unlocking gets faster: the password is stretched with 256 MiB of memory instead of 512 MiB.");
            }
            keys::bullet(ui, "", &defaults);
        }
    }
}
