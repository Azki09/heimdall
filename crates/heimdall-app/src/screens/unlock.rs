//! Unlock and Locked screens (mockups Unlock, Locked). Unlocking runs
//! Argon2id on a background thread so the window stays responsive.

use std::path::PathBuf;

use eframe::egui::{self, vec2, Key};
use heimdall_core::{Error, LockedVault, RecoveryCode, Vault};

use crate::app::Action;
use crate::support::{error_message, file_name, Job};
use crate::theme::*;
use crate::widgets::{self, ButtonKind, SecretField};
use crate::{chrome, screens};

/// Shown on the Locked variant of the screen.
pub struct LockInfo {
    pub reason: String,
    pub saved_at: Option<String>,
}

pub struct Unlock {
    path: PathBuf,
    locked: Result<LockedVault, String>,
    info: Option<LockInfo>,
    password: SecretField,
    recovery: SecretField,
    use_recovery: bool,
    error: Option<String>,
    job: Option<Job<Result<Vault, Error>>>,
    focus_pending: bool,
}

impl Unlock {
    pub fn new(path: PathBuf, info: Option<LockInfo>) -> Self {
        let locked = heimdall_core::storage::read_vault(&path)
            .and_then(|b| LockedVault::parse(&b))
            .map_err(|e| error_message(&e));
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
        }
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
            self.job = Some(Job::spawn(ctx, move || locked.unlock_with_recovery(&code)));
        } else {
            let pw = self.password.take();
            self.job = Some(Job::spawn(ctx, move || locked.unlock_with_password(&pw, None)));
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Option<Action> {
        if let Some(result) = self.job.as_ref().and_then(Job::poll) {
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
        screens::centered_column(ctx, 340.0, 200.0 + info_h + 30.0 + field_h + 150.0, |ui| {
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

            let ready = !busy
                && if self.use_recovery {
                    !self.recovery.text.trim().is_empty()
                } else {
                    !self.password.text.is_empty()
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
        });
        action
    }
}
