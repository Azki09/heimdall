//! The Create vault wizard (mockups Create*) and the forced "new password"
//! flow after opening with the recovery code (DESIGN §9 steps 1 and 5).
//! Both share the Password, Recovery code and Confirm steps.

use std::path::{Path, PathBuf};

use eframe::egui::{self, pos2, vec2, Align2, CornerRadius, Sense, Stroke, StrokeKind, Ui};
use heimdall_core::content::SecuritySettings;
use heimdall_core::storage::{save_vault, SaveKind};
use heimdall_core::{Error, KdfParams, Profile, RecoveryCode, Vault};

use crate::app::Action;
use crate::chrome;
use crate::support::{error_message, Job};
use crate::theme::*;
use crate::widgets::{self, ButtonKind, SecretField};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Name,
    Profile,
    Password,
    Recovery,
    Confirm,
}

impl Step {
    fn label(self) -> &'static str {
        match self {
            Step::Name => "Name",
            Step::Profile => "Profile",
            Step::Password => "Password",
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
    Reset { path: PathBuf },
}

type KeysJob = Job<(Option<Vault>, Result<RecoveryCode, Error>)>;

pub struct Wizard {
    mode: Mode,
    step: Step,
    password: SecretField,
    confirm: SecretField,
    strength: Option<(u8, String)>,
    vault: Option<Vault>,
    code: Option<RecoveryCode>,
    job: Option<KeysJob>,
    check_idx: [usize; 3],
    check_words: [String; 3],
    error: Option<String>,
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
        Self::with_mode(Mode::Reset { path }, Step::Password, Some(vault))
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
            check_idx: [0, 1, 2],
            check_words: Default::default(),
            error: None,
        }
    }

    fn steps(&self) -> &'static [Step] {
        match self.mode {
            Mode::Create { .. } => &[
                Step::Name,
                Step::Profile,
                Step::Password,
                Step::Recovery,
                Step::Confirm,
            ],
            Mode::Reset { .. } => &[Step::Password, Step::Recovery, Step::Confirm],
        }
    }

    fn target_path(&self) -> PathBuf {
        match &self.mode {
            Mode::Create { name, folder, .. } => folder.join(format!("{}.heim", name.trim())),
            Mode::Reset { path } => path.clone(),
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Option<Action> {
        if let Some((vault, result)) = self.job.as_ref().and_then(Job::poll) {
            self.job = None;
            self.vault = vault;
            match result {
                Ok(code) => {
                    self.code = Some(code);
                    self.step = Step::Recovery;
                }
                Err(e) => self.error = Some(error_message(&e)),
            }
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
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG).inner_margin(egui::Margin {
                left: 72,
                right: 72,
                top: 56,
                bottom: 40,
            }))
            .show(ctx, |ui| {
                let divider_x = ui.max_rect().left() - 72.0;
                ui.painter().vline(
                    divider_x,
                    ui.max_rect().y_range().expand(96.0),
                    Stroke::new(1.0_f32, DIVIDER),
                );
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    action = self.buttons_ui(ui);
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_max_width(460.0);
                        match self.step {
                            Step::Name => self.name_ui(ui),
                            Step::Profile => self.profile_ui(ui),
                            Step::Password => self.password_ui(ui),
                            Step::Recovery => self.recovery_ui(ui),
                            Step::Confirm => self.confirm_ui(ui),
                        }
                    });
                });
            });
        action
    }

    fn steps_ui(&self, ui: &mut Ui) {
        let current = self.steps().iter().position(|s| *s == self.step).unwrap_or(0);
        for (i, step) in self.steps().iter().enumerate() {
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
                step.label(),
                crate::theme::ui(13.0),
                color,
            );
            ui.add_space(18.0);
        }
    }

    fn buttons_ui(&mut self, ui: &mut Ui) -> Option<Action> {
        let mut action = None;
        let busy = self.job.is_some();
        ui.horizontal(|ui| {
            let (back_label, next_label) = match self.step {
                Step::Name => ("Cancel", "Continue"),
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
                self.strength.as_ref().is_some_and(|(score, _)| *score >= 2)
                    && self.password.text == self.confirm.text
            }
            Step::Recovery => self.code.is_some(),
            Step::Confirm => match &self.code {
                Some(code) => self
                    .check_idx
                    .iter()
                    .zip(&self.check_words)
                    .all(|(i, w)| code.check_word(*i, w)),
                None => false,
            },
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
            Step::Recovery => {
                self.code = None;
                if matches!(self.mode, Mode::Create { .. }) {
                    self.vault = None;
                }
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
            Step::Password => self.generate_keys(ctx),
            Step::Recovery => {
                self.pick_check_words();
                self.step = Step::Confirm;
            }
            Step::Confirm => return self.finish(),
        }
        None
    }

    fn generate_keys(&mut self, ctx: &egui::Context) {
        let pw = self.password.take();
        self.confirm.clear();
        self.strength = None;
        match &self.mode {
            Mode::Create { profile, .. } => {
                let profile = *profile;
                self.job = Some(Job::spawn(ctx, move || match Vault::create(&pw, profile) {
                    Ok(new) => (Some(new.vault), Ok(new.recovery_code)),
                    Err(e) => (None, Err(e)),
                }));
            }
            Mode::Reset { .. } => {
                let Some(mut vault) = self.vault.take() else {
                    return;
                };
                self.job = Some(Job::spawn(ctx, move || {
                    // USB keys arrive in v0.3: after recovery the vault opens with the password alone.
                    let result = vault.reset_access(&pw, &[], true, KdfParams::for_profile(Profile::Basic));
                    if result.is_ok() && vault.content.security.profile != Profile::Basic {
                        vault.content.security = SecuritySettings::defaults_for(Profile::Basic);
                    }
                    (Some(vault), result)
                }));
            }
        }
    }

    fn pick_check_words(&mut self) {
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
        self.check_idx = [idx[0], idx[1], idx[2]];
        self.check_words = Default::default();
    }

    fn finish(&mut self) -> Option<Action> {
        let vault = self.vault.take()?;
        let path = self.target_path();
        let kind = match self.mode {
            Mode::Create { .. } => SaveKind::Normal,
            // Access changed: the .bak must not keep opening with the old password (DESIGN §5.4).
            Mode::Reset { .. } => SaveKind::AccessChanged,
        };
        match vault.to_bytes().and_then(|b| save_vault(&path, &b, kind)) {
            Ok(()) => {
                self.code = None;
                Some(Action::Opened { vault, path })
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
                "Needs USB key support, which arrives in version 0.3.",
                false,
            ),
            (
                Profile::Paranoid,
                "Paranoid",
                "Everything in Hardened, with shorter timeouts and slower unlocking.",
                "Needs USB key support, which arrives in version 0.3.",
                false,
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
            self.strength = estimate_strength(&self.password.text);
        }
        ui.add_space(4.0);
        strength_meter(ui, self.strength.as_ref());
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
        if !self.confirm.text.is_empty() {
            let matches = self.confirm.text == self.password.text;
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
    }

    fn recovery_ui(&mut self, ui: &mut Ui) {
        widgets::heading(ui, "Write down your recovery code");
        ui.add_space(2.0);
        widgets::paragraph(
            ui,
            "If you forget your password, these 12 words are the only way back in. Write them on paper.",
            460.0,
        );
        ui.add_space(24.0);
        let Some(code) = &self.code else { return };
        let words = code.words();
        let (rect, _) = ui.allocate_exact_size(vec2(460.0, 4.0 * 36.0 + 24.0), Sense::hover());
        let painter = ui.painter();
        painter.rect(
            rect,
            CornerRadius::same(8),
            SURFACE,
            Stroke::new(1.0_f32, BORDER),
            StrokeKind::Inside,
        );
        for (i, word) in words.iter().enumerate() {
            let col = (i / 4) as f32;
            let row = (i % 4) as f32;
            let x = rect.left() + 20.0 + col * 146.0;
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
        ui.add_space(20.0);
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
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                widgets::label(ui, "—  ", 13.0, TEXT_FAINT);
                widgets::label(ui, lead, 13.0, TEXT_BODY);
                widgets::label(ui, rest, 13.0, TEXT_MUTED);
            });
            ui.add_space(4.0);
        }
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
        let mut first_empty = None;
        for k in 0..3 {
            let idx = self.check_idx[k];
            widgets::label(ui, &format!("Word {}", idx + 1), 12.0, TEXT_MUTED);
            let id = egui::Id::new(("check_word", k));
            widgets::text_field(ui, id, &mut self.check_words[k], 460.0, None, "");
            let typed = self.check_words[k].trim();
            if !typed.is_empty() {
                let ok = code.check_word(idx, typed);
                let (text, color) = if ok {
                    ("Correct", TEXT_MUTED)
                } else {
                    ("Doesn't match word {n}", TEXT_MUTED)
                };
                let text = text.replace("{n}", &(idx + 1).to_string());
                ui.horizontal(|ui| {
                    if ok {
                        let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                        widgets::icon_check(ui.painter(), r.center(), 12.0, OK);
                    }
                    widgets::label(ui, &text, 12.0, color);
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

const STEP_TODO: egui::Color32 = egui::Color32::from_rgb(0x3A, 0x3A, 0x3A);
const STEP_TODO_TEXT: egui::Color32 = egui::Color32::from_rgb(0x7A, 0x7A, 0x7A);

/// `(score 0-4, label)` using zxcvbn, entirely offline.
fn estimate_strength(pw: &str) -> Option<(u8, String)> {
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

fn strength_meter(ui: &mut Ui, strength: Option<&(u8, String)>) {
    let (rect, _) = ui.allocate_exact_size(vec2(460.0, 18.0), Sense::hover());
    let bars_w = 460.0 - 200.0;
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

/// `C:\Users\me\Documents\x.heim` → `Documents\x.heim`.
fn display_path(p: &Path) -> String {
    if let Some(home) = std::env::var_os("USERPROFILE") {
        if let Ok(rel) = p.strip_prefix(Path::new(&home)) {
            return rel.display().to_string();
        }
    }
    p.display().to_string()
}
