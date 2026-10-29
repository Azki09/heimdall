//! Unlock and Locked screens (mockups Unlock, Locked). Unlocking runs
//! Argon2id on a background thread so the window stays responsive. Vaults
//! with USB keys show whether the key is connected (DESIGN §9 step 2).

use std::path::PathBuf;

use eframe::egui::{self, pos2, vec2, Align2, CornerRadius, Key, Sense, Stroke, StrokeKind, Ui};
use heimdall_core::{Error, LockedVault, RecoveryCode, Vault};

use crate::app::Action;
use crate::support::{error_message, file_name, Job};
use crate::theme::*;
use crate::usb::{self, Scan, UsbKey};
use crate::widgets::{self, ButtonKind, SecretField};
use crate::{chrome, screens};

/// Shown on the Locked variant of the screen.
pub struct LockInfo {
    pub reason: String,
    pub saved_at: Option<String>,
}

/// The unlocked vault, and the USB key it was opened with.
type UnlockJob = Job<(Result<Vault, Error>, Option<UsbKey>)>;

pub struct Unlock {
    path: PathBuf,
    locked: Result<LockedVault, String>,
    info: Option<LockInfo>,
    password: SecretField,
    recovery: SecretField,
    use_recovery: bool,
    error: Option<String>,
    job: Option<UnlockJob>,
    focus_pending: bool,
    /// Looks for this vault's key files while the screen is open.
    keys: Option<Scan<Vec<UsbKey>>>,
}

/// What the Key row says.
enum KeyState<'a> {
    Searching,
    Found(&'a UsbKey),
    /// A key file of this vault whose slot was revoked.
    NotEnrolled(&'a UsbKey),
    Missing,
}

impl Unlock {
    pub fn new(path: PathBuf, info: Option<LockInfo>) -> Self {
        let locked = heimdall_core::storage::read_vault(&path)
            .and_then(|b| LockedVault::parse(&b))
            .map_err(|e| error_message(&e));
        let keys = locked
            .as_ref()
            .ok()
            .filter(|l| l.has_keyfile_slots())
            .map(|l| usb::key_scan(*l.vault_id()));
        Self {
            path,
            locked,
            info,
            password: SecretField::default(),
            recovery: SecretField::default(),
            use_recovery: false,
            error: None,
            job: None,
            focus_pending: true,
            keys,
        }
    }

    fn key_state(&self) -> KeyState<'_> {
        let (Ok(locked), Some(scan)) = (&self.locked, &self.keys) else {
            return KeyState::Missing;
        };
        let Some(found) = &scan.value else {
            return KeyState::Searching;
        };
        if let Some(k) = found.iter().find(|k| locked.accepts_keyfile(&k.keyfile)) {
            return KeyState::Found(k);
        }
        found.first().map_or(KeyState::Missing, KeyState::NotEnrolled)
    }

    fn requires_key(&self) -> bool {
        self.locked.as_ref().is_ok_and(|l| l.requires_keyfile())
    }

    fn start(&mut self, ctx: &egui::Context) {
        let Ok(locked) = &self.locked else { return };
        let locked = locked.clone();
        self.error = None;
        if self.use_recovery {
            let code = match RecoveryCode::parse(&self.recovery.text) {
                Ok(c) => c,
                Err(e) => {
                    self.error = Some(error_message(&e));
                    return;
                }
            };
            self.recovery.clear();
            self.job = Some(Job::spawn(ctx, move || {
                (locked.unlock_with_recovery(&code), None)
            }));
        } else {
            let pw = self.password.take();
            let key = match self.key_state() {
                KeyState::Found(k) => Some(k.clone()),
                _ => None,
            };
            self.job = Some(Job::spawn(ctx, move || {
                let result = locked.unlock_with_password(&pw, key.as_ref().map(|k| &k.keyfile));
                (result, key)
            }));
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Option<Action> {
        if let Some(keys) = self.keys.as_mut().filter(|_| !self.use_recovery) {
            keys.poll(ctx);
        }
        if let Some((result, usb)) = self.job.as_ref().and_then(Job::poll) {
            self.job = None;
            match result {
                Ok(vault) if self.use_recovery => {
                    return Some(Action::Recovered {
                        vault,
                        path: self.path.clone(),
                    });
                }
                Ok(vault) => {
                    return Some(Action::Opened {
                        vault,
                        path: self.path.clone(),
                        usb,
                    })
                }
                Err(e) => {
                    self.error = Some(error_message(&e));
                    self.focus_pending = true;
                }
            }
        }

        let mut action = None;
        let busy = self.job.is_some();
        chrome::title_bar(ctx, |ui| chrome::title_text(ui, "Heimdall"));
        chrome::tagline_footer(ctx);

        let info_h = if self.info.is_some() { 50.0 } else { 0.0 };
        let field_h = if self.use_recovery { 110.0 } else { 42.0 };
        let show_key = self.keys.is_some() && !self.use_recovery;
        let key_h = if show_key { 82.0 } else { 0.0 };
        screens::centered_column(
            ctx,
            340.0,
            200.0 + info_h + 30.0 + field_h + key_h + 150.0,
            |ui| {
                ui.vertical_centered(|ui| {
                    widgets::logo(ui, 6.0);
                    ui.add_space(14.0);
                    ui.label(
                        egui::RichText::new(file_name(&self.path))
                            .font(mono(13.0))
                            .color(TEXT_MUTED),
                    );
                    if let Some(info) = &self.info {
                        ui.add_space(14.0);
                        widgets::label(ui, &info.reason, 13.0, TEXT_BODY);
                        if let Some(saved) = &info.saved_at {
                            widgets::label(ui, saved, 12.0, TEXT_MUTED);
                        }
                    }
                });
                ui.add_space(36.0);

                if let Err(msg) = &self.locked {
                    widgets::label(ui, msg, 13.0, DANGER);
                    ui.add_space(20.0);
                    if widgets::link(ui, "Open another vault", 13.0).clicked() {
                        action = Some(Action::Welcome);
                    }
                    return;
                }

                let submit = if self.use_recovery {
                    widgets::label(ui, "Recovery code", 12.0, TEXT_MUTED);
                    let id = egui::Id::new("recovery_words");
                    let resp = ui.add(
                        egui::TextEdit::multiline(&mut self.recovery.text)
                            .id(id)
                            .font(mono(13.0))
                            .text_color(TEXT)
                            .hint_text(egui::RichText::new("12 words, separated by spaces").color(TEXT_FAINT))
                            .desired_width(340.0)
                            .desired_rows(4)
                            .margin(vec2(12.0, 10.0)),
                    );
                    if std::mem::take(&mut self.focus_pending) {
                        resp.request_focus();
                    }
                    resp.has_focus() && ui.input(|i| i.key_pressed(Key::Enter) && i.modifiers.command)
                } else {
                    widgets::label(ui, "Password", 12.0, TEXT_MUTED);
                    let id = egui::Id::new("unlock_password");
                    let resp = widgets::text_field(
                        ui,
                        id,
                        &mut self.password.text,
                        340.0,
                        Some(&mut self.password.reveal),
                        "",
                    );
                    if std::mem::take(&mut self.focus_pending) {
                        resp.request_focus();
                    }
                    resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))
                };

                ui.add_space(8.0);
                match (&self.error, busy) {
                    (_, true) => {
                        widgets::label(ui, "Unlocking…", 12.0, TEXT_MUTED);
                    }
                    (Some(e), false) => {
                        widgets::label(ui, e, 12.0, DANGER);
                    }
                    (None, false) => {
                        ui.add_space(17.0);
                    }
                }
                ui.add_space(12.0);

                if show_key {
                    widgets::label(ui, "Key", 12.0, TEXT_MUTED);
                    key_row(ui, &self.key_state(), self.requires_key());
                    ui.add_space(28.0);
                }

                let key_ok = !self.requires_key() || matches!(self.key_state(), KeyState::Found(_));
                let ready = !busy
                    && if self.use_recovery {
                        !self.recovery.text.trim().is_empty()
                    } else {
                        !self.password.text.is_empty() && key_ok
                    };
                let clicked =
                    widgets::button(ui, ButtonKind::Primary, "Unlock", ready, vec2(340.0, 42.0)).clicked();
                if ready && (clicked || submit) {
                    self.start(ctx);
                }

                ui.add_space(24.0);
                ui.horizontal(|ui| {
                    let toggle = if self.use_recovery {
                        "Use password"
                    } else {
                        "Use recovery code"
                    };
                    let other = if self.info.is_some() {
                        "Close vault"
                    } else {
                        "Open another vault"
                    };
                    let w = ui.fonts(|f| {
                        f.layout_no_wrap(toggle.into(), crate::theme::ui(13.0), TEXT)
                            .size()
                            .x
                            + f.layout_no_wrap(other.into(), crate::theme::ui(13.0), TEXT)
                                .size()
                                .x
                    });
                    ui.add_space(((340.0 - w - 24.0) / 2.0).max(0.0));
                    ui.spacing_mut().item_spacing.x = 24.0;
                    if widgets::link(ui, toggle, 13.0).clicked() && !busy {
                        self.use_recovery = !self.use_recovery;
                        self.password.clear();
                        self.recovery.clear();
                        self.error = None;
                        self.focus_pending = true;
                    }
                    if widgets::link(ui, other, 13.0).clicked() && !busy {
                        action = Some(Action::Welcome);
                    }
                });
            },
        );
        action
    }
}

/// The framed "USB key detected · KINGSTON (E:)" row.
fn key_row(ui: &mut Ui, state: &KeyState<'_>, required: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(340.0, 42.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_stroke(
        rect,
        CornerRadius::same(6),
        Stroke::new(1.0_f32, BORDER),
        StrokeKind::Inside,
    );
    let label;
    let (dot, text, right) = match state {
        KeyState::Searching => (TRACK_OFF, "Looking for your USB key", "…"),
        KeyState::Found(k) => {
            label = k.label();
            (OK, "USB key detected", label.as_str())
        }
        KeyState::NotEnrolled(k) => {
            label = k.label();
            (DANGER, "This key was revoked", label.as_str())
        }
        KeyState::Missing if required => (TRACK_OFF, "Insert your USB key", "Waiting…"),
        KeyState::Missing => (TRACK_OFF, "No USB key connected", "Optional"),
    };
    widgets::dot(painter, pos2(rect.left() + 15.0, rect.center().y), dot);
    let left = painter.text(
        pos2(rect.left() + 28.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        crate::theme::ui(13.0),
        TEXT_BODY,
    );
    // A long drive name is cut off rather than drawn over the status text.
    let clip = egui::Rect::from_min_max(pos2(left.right() + 12.0, rect.top()), rect.max);
    painter.with_clip_rect(clip).text(
        pos2(rect.right() - 12.0, rect.center().y),
        Align2::RIGHT_CENTER,
        right,
        crate::theme::ui(13.0),
        TEXT_MUTED,
    );
}
