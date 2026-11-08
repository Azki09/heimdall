//! The Create vault wizard (mockups Create*) and the forced "new password"
//! flow after opening with the recovery code (DESIGN §9 steps 1 and 5).
//! Both share the Password, USB key, Recovery code and Confirm steps.

use std::path::{Path, PathBuf};

use eframe::egui::{self, pos2, vec2, Align2, CornerRadius, Sense, Stroke, StrokeKind, Ui};
use heimdall_core::storage::{save_vault, SaveKind};
use heimdall_core::{KdfParams, Keyfile, Profile, RecoveryCode, SecretString, Vault};

use crate::app::Action;
use crate::chrome;
use crate::screens::keys::{self, CodeActions, CodeCheck};
use crate::support::{error_message, Job};
use crate::theme::*;
use crate::usb::{self, Drive, Scan, UsbKey};
use crate::widgets::{self, ButtonKind, SecretField};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Name,
    Profile,
    Password,
    UsbKey,
    Recovery,
    Confirm,
}

impl Step {
    fn label(self) -> &'static str {
        match self {
            Step::Name => "Name",
            Step::Profile => "Profile",
            Step::Password => "Password",
            Step::UsbKey => "USB key",
            Step::Recovery => "Recovery code",
            Step::Confirm => "Confirm",
        }
    }
}

enum Mode {
    Create {
        name: String,
        folder: PathBuf,
        profile: Profile,
    },
    /// Opened with the recovery code; `vault` is `None` while a job runs.
    /// The profile is kept, so Hardened and Paranoid enroll a new USB key.
    Reset { path: PathBuf, profile: Profile },
}

type KeysJob = Job<(Option<Vault>, Result<(RecoveryCode, Option<UsbKey>), String>)>;

pub struct Wizard {
    mode: Mode,
    step: Step,
    password: SecretField,
    confirm: SecretField,
    strength: Option<(u8, String)>,
    vault: Option<Vault>,
    code: Option<RecoveryCode>,
    job: Option<KeysJob>,
    check: CodeCheck,
    code_actions: CodeActions,
    drives: Scan<Vec<Drive>>,
    drive: Option<PathBuf>,
    /// The key file written to the drive. Deleted again if the wizard is
    /// left before the vault is saved: it would open nothing.
    usb: Option<UsbKey>,
    error: Option<String>,
}

impl Drop for Wizard {
    fn drop(&mut self) {
        self.discard_key();
    }
}

impl Wizard {
    pub fn create() -> Self {
        let folder = std::env::var_os("USERPROFILE")
            .map(|p| Path::new(&p).join("Documents"))
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| PathBuf::from("."));
        Self::with_mode(
            Mode::Create {
                name: "personal".into(),
                folder,
                profile: Profile::Basic,
            },
            Step::Name,
            None,
        )
    }

    pub fn reset(vault: Vault, path: PathBuf) -> Self {
        let profile = vault.content.security.profile;
        Self::with_mode(Mode::Reset { path, profile }, Step::Password, Some(vault))
    }

    fn with_mode(mode: Mode, step: Step, vault: Option<Vault>) -> Self {
        Self {
            mode,
            step,
            password: SecretField::default(),
            confirm: SecretField::default(),
            strength: None,
            vault,
            code: None,
            job: None,
            check: CodeCheck::new(),
            code_actions: CodeActions::default(),
            drives: usb::drive_scan(),
            drive: None,
            usb: None,
            error: None,
        }
    }

    fn profile(&self) -> Profile {
        match self.mode {
            Mode::Create { profile, .. } | Mode::Reset { profile, .. } => profile,
        }
    }

    /// Hardened and Paranoid need a USB key (DESIGN §6).
    fn needs_usb(&self) -> bool {
        self.profile() != Profile::Basic
    }

    fn steps(&self) -> Vec<Step> {
        let mut steps = match self.mode {
            Mode::Create { .. } => vec![Step::Name, Step::Profile, Step::Password],
            Mode::Reset { .. } => vec![Step::Password],
        };
        if self.needs_usb() {
            steps.push(Step::UsbKey);
        }
        steps.extend([Step::Recovery, Step::Confirm]);
        steps
    }

    fn target_path(&self) -> PathBuf {
        match &self.mode {
            Mode::Create { name, folder, .. } => folder.join(format!("{}.heim", name.trim())),
            Mode::Reset { path, .. } => path.clone(),
        }
    }

    /// `personal` for `personal.heim`: the key file is `personal.bifrost`.
    fn key_stem(&self) -> String {
        self.target_path()
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "vault".into())
    }

    fn selected_drive(&self) -> Option<Drive> {
        let root = self.drive.as_ref()?;
        self.drives
            .value
            .as_ref()?
            .iter()
            .find(|d| &d.root == root)
            .cloned()
    }

    fn discard_key(&mut self) {
        if let Some(key) = self.usb.take() {
            let _ = std::fs::remove_file(&key.path);
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Option<Action> {
        if let Some((vault, result)) = self.job.as_ref().and_then(Job::poll) {
            self.job = None;
            self.vault = vault;
            match result {
                Ok((code, usb)) => {
                    self.code = Some(code);
                    self.usb = usb;
                    self.password.clear();
                    self.confirm.clear();
                    self.strength = None;
                    self.step = Step::Recovery;
                }
                Err(e) => self.error = Some(e),
            }
        }
        if self.step == Step::UsbKey {
            self.drives.poll(ctx);
        }

        let title = match self.mode {
            Mode::Create { .. } => "Create vault",
            Mode::Reset { .. } => "Set a new password",
        };
        chrome::title_bar(ctx, |ui| chrome::title_text(ui, title));

        egui::SidePanel::left("wizard_steps")
            .exact_width(240.0)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(BG).inner_margin(egui::Margin {
                left: 32,
                right: 32,
                top: 40,
                bottom: 40,
            }))
            .show_separator_line(false)
            .show(ctx, |ui| self.steps_ui(ui));

        let mut action = None;
        // The 460 px column keeps 72 px margins, narrowed in a small window.
        let side = ((ctx.screen_rect().width() - 240.0 - 460.0) / 2.0).clamp(24.0, 72.0);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG).inner_margin(egui::Margin {
                left: side as i8,
                right: side as i8,
                top: 56,
                bottom: 40,
            }))
            .show(ctx, |ui| {
                let divider_x = ui.max_rect().left() - side;
                ui.painter().vline(
                    divider_x,
                    ui.max_rect().y_range().expand(96.0),
                    Stroke::new(1.0_f32, DIVIDER),
                );
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    action = self.buttons_ui(ui);
                    ui.add_space(16.0);
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        // A short window scrolls the step instead of drawing it
                        // over the buttons. Each step starts at the top. The bar
                        // takes its own room instead of floating over the fields.
                        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                        egui::ScrollArea::vertical()
                            .id_salt(self.step.label())
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_max_width(460.0);
                                match self.step {
                                    Step::Name => self.name_ui(ui),
                                    Step::Profile => self.profile_ui(ui),
                                    Step::Password => self.password_ui(ui),
                                    Step::UsbKey => self.usb_ui(ui),
                                    Step::Recovery => self.recovery_ui(ui),
                                    Step::Confirm => self.confirm_ui(ui),
                                }
                            });
                    });
                });
            });
        action
    }

    fn steps_ui(&self, ui: &mut Ui) {
        let current = self.steps().iter().position(|s| *s == self.step).unwrap_or(0);
        let labels: Vec<&str> = self.steps().iter().map(|s| s.label()).collect();
        step_list(ui, &labels, current);
    }

    fn buttons_ui(&mut self, ui: &mut Ui) -> Option<Action> {
        let mut action = None;
        let busy = self.job.is_some();
        ui.horizontal(|ui| {
            let (back_label, next_label) = match self.step {
                Step::Name => ("Cancel", "Continue"),
                Step::UsbKey => ("Back", "Write key to drive"),
                Step::Recovery => ("Back", "I've written it down"),
                Step::Confirm => (
                    "Show the words again",
                    if matches!(self.mode, Mode::Create { .. }) {
                        "Create vault"
                    } else {
                        "Save and open"
                    },
                ),
                _ => ("Back", "Continue"),
            };
            let back_enabled =
                !busy && !(matches!(self.mode, Mode::Reset { .. }) && self.step == Step::Password);
            if widgets::button(
                ui,
                ButtonKind::Secondary,
                back_label,
                back_enabled,
                vec2(0.0, 40.0),
            )
            .clicked()
            {
                action = self.back();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ready = !busy && self.can_continue();
                let label = if busy { "Generating keys…" } else { next_label };
                let next = widgets::button(ui, ButtonKind::Primary, label, ready, vec2(0.0, 40.0)).clicked();
                let enter =
                    ready && ui.input(|i| i.key_pressed(egui::Key::Enter)) && self.step != Step::Recovery;
                if next || enter {
                    action = self.next(ui.ctx());
                }
            });
        });
        ui.add_space(8.0);
        if let Some(e) = &self.error {
            widgets::label(ui, e, 12.0, DANGER);
        }
        action
    }

    fn can_continue(&self) -> bool {
        match self.step {
            Step::Name => self.name_problem().is_none(),
            Step::Profile => true,
            Step::Password => {
                self.strength
                    .as_ref()
                    .is_some_and(|(score, _)| *score >= keys::MIN_SCORE)
                    && self.password.text == self.confirm.text
            }
            Step::UsbKey => self.selected_drive().is_some(),
            Step::Recovery => self.code.is_some(),
            Step::Confirm => self.code.as_ref().is_some_and(|c| self.check.complete(c)),
        }
    }

    fn name_problem(&self) -> Option<&'static str> {
        let Mode::Create { name, folder, .. } = &self.mode else {
            return None;
        };
        let name = name.trim();
        if name.is_empty() {
            return Some("Give the vault a name.");
        }
        if name.chars().any(|c| r#"<>:"/\|?*"#.contains(c) || c.is_control()) {
            return Some(r#"A name can't contain < > : " / \ | ? *"#);
        }
        if !folder.is_dir() {
            return Some("That folder doesn't exist.");
        }
        if self.target_path().exists() {
            return Some("A vault with this name already exists in that folder.");
        }
        None
    }

    fn back(&mut self) -> Option<Action> {
        self.error = None;
        match self.step {
            Step::Name => return Some(Action::Welcome),
            Step::Profile => self.step = Step::Name,
            Step::Password => self.step = Step::Profile,
            Step::UsbKey => self.step = Step::Password,
            Step::Recovery => {
                self.code = None;
                self.discard_key();
                if matches!(self.mode, Mode::Create { .. }) {
                    self.vault = None;
                }
                // The password was wiped once the keys were made: type it again.
                self.step = Step::Password;
            }
            Step::Confirm => self.step = Step::Recovery,
        }
        None
    }

    fn next(&mut self, ctx: &egui::Context) -> Option<Action> {
        self.error = None;
        match self.step {
            Step::Name => self.step = Step::Profile,
            Step::Profile => self.step = Step::Password,
            Step::Password if self.needs_usb() => self.step = Step::UsbKey,
            Step::Password | Step::UsbKey => self.generate_keys(ctx),
            Step::Recovery => {
                self.check = CodeCheck::new();
                self.step = Step::Confirm;
            }
            Step::Confirm => return self.finish(),
        }
        None
    }

    /// Runs Argon2id in the background: creates the vault (or replaces all
    /// its access after recovery) and, for Hardened / Paranoid, writes the
    /// key file to the chosen drive. The password stays in its field until
    /// this succeeds, so a failed write can simply be retried.
    fn generate_keys(&mut self, ctx: &egui::Context) {
        let pw = SecretString::from(self.password.text.clone());
        let profile = self.profile();
        let drive = if self.needs_usb() {
            match self.selected_drive() {
                Some(d) => Some(d),
                None => return,
            }
        } else {
            None
        };
        let stem = self.key_stem();
        let write = move |kf: &Keyfile| -> Result<Option<UsbKey>, String> {
            match &drive {
                Some(d) => usb::write_key(d, &stem, kf)
                    .map(Some)
                    .map_err(|e| format!("Couldn't write the key to {}: {e}", usb::drive_label(d))),
                None => Ok(None),
            }
        };
        match &self.mode {
            Mode::Create { .. } => {
                self.job = Some(Job::spawn(ctx, move || match Vault::create(&pw, profile) {
                    Ok(new) => match new.keyfile.as_ref().map_or(Ok(None), write) {
                        Ok(usb) => (Some(new.vault), Ok((new.recovery_code, usb))),
                        Err(e) => (None, Err(e)),
                    },
                    Err(e) => (None, Err(error_message(&e))),
                }));
            }
            Mode::Reset { .. } => {
                let Some(mut vault) = self.vault.take() else {
                    return;
                };
                self.job = Some(Job::spawn(ctx, move || {
                    let kdf = KdfParams::for_profile(profile);
                    if profile == Profile::Basic {
                        let result = vault.reset_access(&pw, &[], true, kdf);
                        return (
                            Some(vault),
                            result.map(|c| (c, None)).map_err(|e| error_message(&e)),
                        );
                    }
                    // The old USB keys may be lost: every slot is replaced and a new key is written.
                    let kf = match Keyfile::generate(*vault.vault_id()) {
                        Ok(k) => k,
                        Err(e) => return (Some(vault), Err(error_message(&e))),
                    };
                    let usb = match write(&kf) {
                        Ok(u) => u,
                        Err(e) => return (Some(vault), Err(e)),
                    };
                    match vault.reset_access(&pw, &[&kf], false, kdf) {
                        Ok(code) => (Some(vault), Ok((code, usb))),
                        Err(e) => {
                            if let Some(u) = &usb {
                                let _ = std::fs::remove_file(&u.path);
                            }
                            (Some(vault), Err(error_message(&e)))
                        }
                    }
                }));
            }
        }
    }

    fn finish(&mut self) -> Option<Action> {
        let mut vault = self.vault.take()?;
        let path = self.target_path();
        // Every slot is new: label them for Settings → Access.
        crate::session::label_new_slots(&mut vault, &[], self.usb.as_ref());
        let kind = match self.mode {
            Mode::Create { .. } => SaveKind::Normal,
            // Access changed: the .bak must not keep opening with the old password (DESIGN §5.4).
            Mode::Reset { .. } => SaveKind::AccessChanged,
        };
        match vault.to_bytes().and_then(|b| save_vault(&path, &b, kind)) {
            Ok(()) => {
                self.code = None;
                let usb = self.usb.take();
                Some(Action::Opened { vault, path, usb })
            }
            Err(e) => {
                self.error = Some(error_message(&e));
                self.vault = Some(vault);
                None
            }
        }
    }

    // ---------- Steps ----------

    fn name_ui(&mut self, ui: &mut Ui) {
        widgets::heading(ui, "Name your vault");
        ui.add_space(2.0);
        widgets::paragraph(
            ui,
            "A vault is a single encrypted file. You can keep it anywhere: your documents, a cloud folder, a USB drive.",
            460.0,
        );
        ui.add_space(24.0);
        let problem = self.name_problem();
        let Mode::Create { name, folder, .. } = &mut self.mode else {
            return;
        };
        widgets::label(ui, "Vault name", 12.0, TEXT_MUTED);
        let resp = widgets::text_field(ui, egui::Id::new("vault_name"), name, 460.0, None, "");
        if ui.memory(|m| m.focused().is_none()) {
            resp.request_focus();
        }
        ui.add_space(20.0);
        widgets::label(ui, "Location", 12.0, TEXT_MUTED);
        let (rect, _) = ui.allocate_exact_size(vec2(460.0, 42.0), Sense::hover());
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(1.0_f32, BORDER),
            StrokeKind::Inside,
        );
        let shown = display_path(&folder.join(format!("{}.heim", name.trim())));
        ui.painter().text(
            pos2(rect.left() + 12.0, rect.center().y),
            Align2::LEFT_CENTER,
            shown,
            mono(13.0),
            TEXT_BODY,
        );
        let change_rect =
            egui::Rect::from_min_size(pos2(rect.right() - 84.0, rect.top() + 5.0), vec2(78.0, 32.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(change_rect));
        if widgets::button(&mut child, ButtonKind::Quiet, "Change", true, vec2(78.0, 32.0)).clicked() {
            if let Some(dir) = rfd::FileDialog::new().set_directory(&*folder).pick_folder() {
                *folder = dir;
            }
        }
        ui.add_space(10.0);
        if let Some(p) = problem {
            if !name.trim().is_empty() {
                widgets::label(ui, p, 12.0, TEXT_MUTED);
            }
        }
    }

    fn profile_ui(&mut self, ui: &mut Ui) {
        widgets::heading(ui, "Choose a security profile");
        ui.add_space(2.0);
        widgets::paragraph(ui, "You can switch to a stricter profile at any time.", 460.0);
        ui.add_space(24.0);
        let Mode::Create { profile, .. } = &mut self.mode else {
            return;
        };
        let options = [
            (
                Profile::Basic,
                "Basic",
                "Password only. Good for most people.",
                "Locks after 10 min · Clipboard clears in 30 s",
                true,
            ),
            (
                Profile::Hardened,
                "Hardened",
                "Password and a USB key. The vault locks when the key is removed.",
                "Locks after 5 min · Clipboard clears in 20 s",
                true,
            ),
            (
                Profile::Paranoid,
                "Paranoid",
                "Everything in Hardened, with shorter timeouts and slower unlocking.",
                "Locks after 2 min · Clipboard clears in 10 s · Locks when minimized",
                true,
            ),
        ];
        for (value, title, desc, detail, enabled) in options {
            let (tc, dc) = if enabled {
                (TEXT, TEXT_MUTED)
            } else {
                (TEXT_FAINT, TEXT_FAINT)
            };
            let desc = ui
                .painter()
                .layout(desc.to_owned(), crate::theme::ui(13.0), dc, 460.0 - 48.0 - 16.0);
            let extra = desc.size().y - 17.0;
            let sense = if enabled { Sense::click() } else { Sense::hover() };
            let (rect, resp) = ui.allocate_exact_size(vec2(460.0, 86.0 + extra), sense);
            let selected = *profile == value;
            let painter = ui.painter();
            painter.rect(
                rect,
                CornerRadius::same(8),
                if selected { SURFACE } else { BG },
                Stroke::new(1.0_f32, if selected { BORDER_FOCUS } else { BORDER }),
                StrokeKind::Inside,
            );
            let radio = pos2(rect.left() + 26.0, rect.top() + 24.0);
            painter.circle_stroke(
                radio,
                7.0,
                Stroke::new(1.5_f32, if enabled { TEXT_MUTED } else { TRACK_OFF }),
            );
            if selected {
                painter.circle_filled(radio, 4.0, TEXT);
            }
            let x = rect.left() + 48.0;
            painter.text(
                pos2(x, rect.top() + 16.0),
                Align2::LEFT_TOP,
                title,
                medium(14.0),
                tc,
            );
            painter.galley(pos2(x, rect.top() + 38.0), desc, dc);
            painter.text(
                pos2(x, rect.top() + 59.0 + extra),
                Align2::LEFT_TOP,
                detail,
                crate::theme::ui(12.0),
                TEXT_FAINT,
            );
            if resp.clicked() {
                *profile = value;
            }
            ui.add_space(10.0);
        }
    }

    fn password_ui(&mut self, ui: &mut Ui) {
        widgets::heading(
            ui,
            if matches!(self.mode, Mode::Create { .. }) {
                "Create a password"
            } else {
                "Choose a new password"
            },
        );
        ui.add_space(2.0);
        let intro = match self.mode {
            Mode::Create { .. } => "A few unrelated words make a strong password that is easy to remember. Nobody can reset it for you.",
            Mode::Reset { .. } => "You opened the vault with your recovery code. Choose a new password; the old one and the old recovery code will stop working.",
        };
        widgets::paragraph(ui, intro, 460.0);
        ui.add_space(24.0);

        widgets::label(ui, "Password", 12.0, TEXT_MUTED);
        let resp = widgets::text_field(
            ui,
            egui::Id::new("new_pw"),
            &mut self.password.text,
            460.0,
            Some(&mut self.password.reveal),
            "",
        );
        if ui.memory(|m| m.focused().is_none()) && self.job.is_none() {
            resp.request_focus();
        }
        if resp.changed() || (self.strength.is_none() && !self.password.text.is_empty()) {
            self.strength = keys::estimate_strength(&self.password.text);
        }
        ui.add_space(4.0);
        keys::strength_meter(ui, self.strength.as_ref(), 460.0);
        ui.add_space(20.0);

        widgets::label(ui, "Confirm password", 12.0, TEXT_MUTED);
        let mut reveal = self.password.reveal;
        widgets::text_field(
            ui,
            egui::Id::new("new_pw2"),
            &mut self.confirm.text,
            460.0,
            Some(&mut reveal),
            "",
        );
        self.password.reveal = reveal;
        ui.add_space(4.0);
        keys::match_line(ui, &self.password.text, &self.confirm.text);
    }

    fn usb_ui(&mut self, ui: &mut Ui) {
        widgets::heading(ui, "Choose your USB key");
        ui.add_space(2.0);
        let intro = match self.mode {
            Mode::Create { .. } => "The vault will only open, and only stay open, while this drive is connected.",
            Mode::Reset { .. } => "Your old USB keys stop working. Choose a drive for a new key; the vault will only open, and only stay open, while it is connected.",
        };
        widgets::paragraph(ui, intro, 460.0);
        ui.add_space(24.0);
        if keys::drive_picker(ui, self.drives.value.as_deref(), &mut self.drive, 460.0) {
            self.drives.refresh();
        }
        ui.add_space(20.0);
        let file = format!("{}.bifrost", self.key_stem());
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(460.0);
            ui.spacing_mut().item_spacing.x = 0.0;
            widgets::label(ui, "—  ", 13.0, TEXT_FAINT);
            widgets::label(ui, "A small key file, ", 13.0, TEXT_MUTED);
            ui.label(egui::RichText::new(file).font(mono(13.0)).color(TEXT_BODY));
            widgets::label(
                ui,
                ", is written to the drive. Nothing else on it is touched.",
                13.0,
                TEXT_MUTED,
            );
        });
        ui.add_space(4.0);
        keys::bullet(
            ui,
            "",
            "The key file alone opens nothing. Your password is still required.",
        );
        keys::bullet(
            ui,
            "",
            "If you lose the drive, your recovery code still opens the vault.",
        );
    }

    fn recovery_ui(&mut self, ui: &mut Ui) {
        widgets::heading(ui, "Write down your recovery code");
        ui.add_space(2.0);
        widgets::paragraph(
            ui,
            "If you forget your password, these 12 words are the only way back in. Write them on paper, or print them.",
            460.0,
        );
        ui.add_space(24.0);
        let vault = crate::support::file_name(&self.target_path());
        let clear_after =
            heimdall_core::content::SecuritySettings::defaults_for(self.profile()).clipboard_clear_s;
        let Some(code) = &self.code else { return };
        keys::code_grid(ui, code, 460.0);
        ui.add_space(14.0);
        self.code_actions.ui(ui, code, &vault, clear_after);
        ui.add_space(16.0);
        keys::code_notes(ui);
    }

    fn confirm_ui(&mut self, ui: &mut Ui) {
        widgets::heading(ui, "Confirm your recovery code");
        ui.add_space(2.0);
        widgets::paragraph(
            ui,
            "Type these three words from your sheet, so we know it was written down correctly.",
            460.0,
        );
        ui.add_space(24.0);
        let Some(code) = &self.code else { return };
        self.check.ui(ui, code, 460.0);

        if matches!(self.mode, Mode::Create { .. }) {
            ui.add_space(20.0);
            keys::bullet(
                ui,
                "Tip:",
                " Settings → Vault → Save a backup copy… keeps a copy on another drive.",
            );
        }

        // Hardened and Paranoid don't remember recent vaults (§6.1): the
        // vault won't be on the Welcome screen, so say where it is.
        if let Mode::Create { profile, .. } = &self.mode {
            if !heimdall_core::content::SecuritySettings::defaults_for(*profile).remember_recent {
                ui.add_space(20.0);
                widgets::paragraph(
                    ui,
                    &format!(
                        "{profile:?} vaults aren't listed under Recent vaults. To open this one later, use Open vault… and choose {}.",
                        display_path(&self.target_path())
                    ),
                    460.0,
                );
            }
        }
    }
}

/// The step list on the left of the wizard and the export pages.
pub fn step_list(ui: &mut Ui, labels: &[&str], current: usize) {
    for (i, label) in labels.iter().enumerate() {
        let (rect, _) = ui.allocate_exact_size(vec2(176.0, 18.0), Sense::hover());
        let c = pos2(rect.left() + 8.0, rect.center().y);
        let painter = ui.painter();
        let color = match i.cmp(&current) {
            std::cmp::Ordering::Less => {
                painter.circle_filled(c, 8.0, TRACK_OFF);
                widgets::icon_check(painter, c, 10.0, TEXT_SECONDARY);
                TEXT_MUTED
            }
            std::cmp::Ordering::Equal => {
                painter.circle_stroke(c, 7.25, Stroke::new(1.5_f32, TEXT));
                TEXT
            }
            std::cmp::Ordering::Greater => {
                painter.circle_stroke(c, 7.25, Stroke::new(1.5_f32, STEP_TODO));
                STEP_TODO_TEXT
            }
        };
        painter.text(
            pos2(rect.left() + 28.0, rect.center().y),
            Align2::LEFT_CENTER,
            *label,
            crate::theme::ui(13.0),
            color,
        );
        ui.add_space(18.0);
    }
}

const STEP_TODO: egui::Color32 = egui::Color32::from_rgb(0x3A, 0x3A, 0x3A);
const STEP_TODO_TEXT: egui::Color32 = egui::Color32::from_rgb(0x7A, 0x7A, 0x7A);

/// `C:\Users\me\Documents\x.heim` → `Documents\x.heim`.
fn display_path(p: &Path) -> String {
    if let Some(home) = std::env::var_os("USERPROFILE") {
        if let Ok(rel) = p.strip_prefix(Path::new(&home)) {
            return rel.display().to_string();
        }
    }
    p.display().to_string()
}
