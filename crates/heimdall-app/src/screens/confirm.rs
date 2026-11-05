//! A confirmation that asks for the vault password, for things that make a
//! vault easier to spy on: turning off capture exclusion and Reveal all
//! (DESIGN §11.1, §12). Argon2id runs in a background job.

use eframe::egui::{self, vec2, CornerRadius, Stroke};
use heimdall_core::Keyfile;

use crate::session::Session;
use crate::support::{error_message, Job};
use crate::theme::*;
use crate::widgets::{self, ButtonKind, SecretField};

const W: f32 = 420.0;
const FIELD: &str = "confirm_pw";

pub enum Outcome {
    Open,
    Canceled,
    Confirmed,
}

pub struct PasswordCheck {
    title: &'static str,
    body: &'static str,
    button: &'static str,
    danger: bool,
    password: SecretField,
    job: Option<Job<Result<(), String>>>,
    error: Option<String>,
}

impl PasswordCheck {
    pub fn new(title: &'static str, body: &'static str, button: &'static str, danger: bool) -> Self {
        Self {
            title,
            body,
            button,
            danger,
            password: SecretField::default(),
            job: None,
            error: None,
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context, s: &Session) -> Outcome {
        if let Some(result) = self.job.as_ref().and_then(Job::poll) {
            self.job = None;
            match result {
                Ok(()) => {
                    widgets::forget_fields(ctx, &[FIELD]);
                    return Outcome::Confirmed;
                }
                Err(e) => self.error = Some(e),
            }
        }
        let busy = self.job.is_some();
        let mut submit = false;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("password_check"))
            .backdrop_color(egui::Color32::from_black_alpha(160))
            .frame(
                egui::Frame::NONE
                    .fill(BG)
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(egui::Margin::same(28)),
            )
            .show(ctx, |ui| {
                ui.set_width(W);
                ui.label(egui::RichText::new(self.title).font(medium(18.0)).color(TEXT));
                ui.add_space(8.0);
                widgets::paragraph(ui, self.body, W);
                ui.add_space(16.0);
                widgets::label(ui, "Password", 12.0, TEXT_MUTED);
                let resp = widgets::text_field(
                    ui,
                    egui::Id::new(FIELD),
                    &mut self.password.text,
                    W,
                    Some(&mut self.password.reveal),
                    "",
                );
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.memory(|m| m.focused().is_none()) {
                    resp.request_focus();
                }
                let ready = !self.password.text.is_empty();
                submit |= enter && ready;
                ui.add_space(14.0);
                match (&self.error, busy) {
                    (_, true) => {
                        widgets::label(ui, "Checking…", 12.0, TEXT_MUTED);
                    }
                    (Some(e), false) => {
                        widgets::label(ui, e, 12.0, DANGER);
                    }
                    (None, false) => {}
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if widgets::button(ui, ButtonKind::Secondary, "Cancel", !busy, vec2(0.0, 40.0)).clicked()
                    {
                        cancel = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let kind = if self.danger {
                            ButtonKind::Danger
                        } else {
                            ButtonKind::Primary
                        };
                        let label = if busy { "Checking…" } else { self.button };
                        if widgets::button(ui, kind, label, ready && !busy, vec2(0.0, 40.0)).clicked() {
                            submit = true;
                        }
                    });
                });
            });
        if (modal.should_close() || cancel) && !busy {
            widgets::forget_fields(ctx, &[FIELD]);
            return Outcome::Canceled;
        }
        if submit && !busy {
            self.error = None;
            let password = self.password.take();
            let vault = s.vault.duplicate();
            let keys: Vec<Keyfile> = s.usb.iter().map(|k| k.keyfile.clone()).collect();
            self.job = Some(Job::spawn(ctx, move || {
                let refs: Vec<&Keyfile> = keys.iter().collect();
                vault
                    .verify_password(&password, &refs)
                    .map_err(|e| error_message(&e))
            }));
        }
        Outcome::Open
    }
}
