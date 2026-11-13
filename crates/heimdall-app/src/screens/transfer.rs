//! Export and import (DESIGN §10.9): notes to a new vault, notes from
//! another vault, notes as plain text, and backup copies. Each flow is a page laid out like
//! the wizard: the steps on the left, one column, the buttons at the bottom.

use std::path::{Path, PathBuf};

use eframe::egui::{self, vec2, Stroke, Ui};
use heimdall_core::content::{Note, Profile, SecuritySettings};
use heimdall_core::storage::{backup_path, write_atomic};
use heimdall_core::{Error, Keyfile, LockedVault, RecoveryCode, SecretString, Vault};
use uuid::Uuid;
use zeroize::Zeroize;

use crate::app::Action;
use crate::chrome;
use crate::path_prompt::{self, PathPrompt};
use crate::plain::{self, Secrets};
use crate::screens::confirm::{Outcome, PasswordCheck};
use crate::screens::keys::{self, CodeActions, CodeCheck};
use crate::screens::wizard::step_list;
use crate::session::Session;
use crate::support::{error_message, file_name, Job};
use crate::theme::*;
use crate::usb::{self, Scan};
use crate::widgets::{self, ButtonKind, SecretField};

/// Width of the page's column, as in the wizard.
const COL: f32 = 460.0;

/// Whether exporting asks for the vault password first: always in Hardened
/// and Paranoid, in Basic unless turned off (DESIGN §10.9).
pub fn export_needs_password(sec: &SecuritySettings) -> bool {
    sec.profile != Profile::Basic || sec.ask_password_export
}

fn password_gate() -> PasswordCheck {
    PasswordCheck::new(
        "Enter your password",
        "Exporting takes notes out of this vault. Enter the vault password to continue.",
        "Continue",
        false,
    )
}

/// Whether `a` and `b` name the same file. Windows paths ignore case (Linux
/// paths don't); a file that doesn't exist yet is compared through its
/// folder.
pub fn same_file(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| {
        std::fs::canonicalize(p).unwrap_or_else(|_| {
            match (
                p.parent().and_then(|d| std::fs::canonicalize(d).ok()),
                p.file_name(),
            ) {
                (Some(dir), Some(name)) => dir.join(name),
                _ => p.to_path_buf(),
            }
        })
    };
    let (a, b) = (canon(a), canon(b));
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

/// The open vault, its `.bak` or its `.tmp`: never the target of a copy.
pub fn is_vault_file(target: &Path, vault: &Path) -> bool {
    let mut tmp = vault.as_os_str().to_owned();
    tmp.push(".tmp");
    [vault.to_path_buf(), backup_path(vault), PathBuf::from(tmp)]
        .iter()
        .any(|p| same_file(target, p))
}

/// `x.heim` → `x`.
pub fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "vault".into())
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// "Import text files…" (DESIGN §10.9): each chosen file becomes a new
/// note. The files themselves are never touched.
pub fn import_text_files(s: &mut Session, paths: &[PathBuf]) {
    let mut notes = Vec::new();
    let mut skipped = Vec::new();
    for p in paths {
        match plain::read_text_file(p) {
            Ok(n) => notes.push(n),
            Err(e) => skipped.push(e),
        }
    }
    let added = s.vault.content.import_notes(notes);
    for id in &added {
        s.touch(Some(*id));
    }
    let mut text = String::new();
    if !added.is_empty() {
        text = format!(
            "Imported {}. The original files are still unencrypted on disk.",
            plural(added.len(), "file")
        );
    }
    if !skipped.is_empty() {
        text = format!("{text} Skipped: {}.", skipped.join("; "))
            .trim()
            .to_owned();
    }
    s.notify(text, added.is_empty());
}

/// "Save a backup copy…" (DESIGN §10.9) to `target`: the vault file
/// exactly as it is on disk, after saving pending changes, read back and
/// compared. The result goes to the status bar; `Err` is a target that
/// can't be used, to choose another.
pub fn backup(s: &mut Session, target: &Path) -> Result<(), String> {
    if is_vault_file(target, &s.path) {
        return Err("That is the vault itself. Choose another name or drive.".into());
    }
    if s.has_unsaved() && !s.save() {
        let e = s.save_error.clone().unwrap_or_default();
        s.notify(format!("No backup: the vault couldn't be saved first. {e}"), true);
        return Ok(());
    }
    match copy_verified(&s.path, target) {
        Ok(()) => {
            s.vault.content.last_backup = Some(heimdall_core::content::now());
            s.touch(None);
            s.notify(format!("Backup saved to {}", target.display()), false);
        }
        Err(e) => s.notify(format!("No backup: {e}"), true),
    }
    Ok(())
}

/// Copies `from` to `to` atomically, then reads `to` back and compares it
/// byte for byte: some USB drives don't keep what was written (§14.1).
/// A copy that doesn't match is deleted.
fn copy_verified(from: &Path, to: &Path) -> Result<(), String> {
    let bytes = heimdall_core::storage::read_vault(from).map_err(|e| error_message(&e))?;
    write_atomic(to, &bytes).map_err(|e| error_message(&e))?;
    match std::fs::read(to) {
        Ok(back) if back == bytes => Ok(()),
        Ok(_) => {
            let _ = std::fs::remove_file(to);
            Err("the copy on the drive doesn't match the vault. Try another drive.".into())
        }
        Err(e) => {
            let _ = std::fs::remove_file(to);
            Err(format!("the copy couldn't be read back: {e}"))
        }
    }
}

// ---------- The list of notes ----------

/// One row of the note list: masked title, kept out of memory once dropped.
struct Pick {
    id: Uuid,
    title: String,
    on: bool,
}

impl Drop for Pick {
    fn drop(&mut self) {
        self.title.zeroize();
    }
}

fn picks(notes: &[Note], on: impl Fn(&Note) -> bool) -> Vec<Pick> {
    notes
        .iter()
        .map(|n| {
            let masked = heimdall_core::secret::mask(n.display_title().trim());
            let mut title: String = masked.chars().take(60).collect();
            if masked.chars().count() > 60 {
                title.push('…');
            }
            if title.is_empty() {
                title = "Untitled".into();
            }
            Pick {
                id: n.id,
                title,
                on: on(n),
            }
        })
        .collect()
}

fn chosen(picks: &[Pick]) -> Vec<Uuid> {
    picks.iter().filter(|p| p.on).map(|p| p.id).collect()
}

/// Checkboxes, with "Select all" / "Select none" above them.
fn note_list(ui: &mut Ui, picks: &mut [Pick]) {
    let n = picks.iter().filter(|p| p.on).count();
    ui.horizontal(|ui| {
        ui.set_width(COL);
        widgets::label(ui, &format!("{n} of {} selected", picks.len()), 12.0, TEXT_MUTED);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            if widgets::link(ui, "Select none", 12.0).clicked() {
                picks.iter_mut().for_each(|p| p.on = false);
            }
            if widgets::link(ui, "Select all", 12.0).clicked() {
                picks.iter_mut().for_each(|p| p.on = true);
            }
        });
    });
    ui.add_space(8.0);
    let (rect, _) = ui.allocate_exact_size(vec2(COL, 1.0), egui::Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, Stroke::new(1.0_f32, DIVIDER));
    for p in picks.iter_mut() {
        ui.add_space(10.0);
        widgets::checkbox(ui, &mut p.on, &p.title);
    }
}

// ---------- Page layout ----------

/// What each export / import page shows.
trait Flow {
    fn title(&self) -> &'static str;
    fn steps(&self) -> Vec<&'static str>;
    fn current(&self) -> usize;
    fn body(&mut self, ui: &mut Ui, s: &Session);
    fn buttons(&mut self, ui: &mut Ui, s: &mut Session) -> Option<Action>;
}

fn show(ctx: &egui::Context, s: &mut Session, flow: &mut dyn Flow) -> Option<Action> {
    chrome::title_bar(ctx, |ui| chrome::title_text(ui, flow.title()));
    let steps = flow.steps();
    let current = flow.current();
    egui::SidePanel::left("transfer_steps")
        .exact_width(240.0)
        .resizable(false)
        .frame(egui::Frame::NONE.fill(BG).inner_margin(egui::Margin {
            left: 32,
            right: 32,
            top: 40,
            bottom: 40,
        }))
        .show_separator_line(false)
        .show(ctx, |ui| step_list(ui, &steps, current));

    let mut action = None;
    let side = ((ctx.screen_rect().width() - 240.0 - COL) / 2.0).clamp(24.0, 72.0);
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
                action = flow.buttons(ui, s);
                ui.add_space(16.0);
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                    egui::ScrollArea::vertical()
                        .id_salt(("transfer", steps.get(current).copied().unwrap_or("")))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_max_width(COL);
                            flow.body(ui, s);
                        });
                });
            });
        });
    action
}

/// Back / next at the bottom of a page, with the error under them.
fn button_row(
    ui: &mut Ui,
    back: &str,
    back_enabled: bool,
    next: &str,
    next_enabled: bool,
    error: Option<&str>,
) -> (bool, bool) {
    let mut out = (false, false);
    ui.horizontal(|ui| {
        // Escape does what Back / Cancel does (DESIGN §10.10), never Skip or
        // "Show the words again" on the recovery code steps.
        let escapes = back_enabled && matches!(back, "Back" | "Cancel");
        if widgets::button(ui, ButtonKind::Secondary, back, back_enabled, vec2(0.0, 40.0)).clicked()
            || (escapes && crate::focus::escape(ui.ctx()))
        {
            out.0 = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::button(ui, ButtonKind::Primary, next, next_enabled, vec2(0.0, 40.0)).clicked() {
                out.1 = true;
            }
        });
    });
    ui.add_space(8.0);
    if let Some(e) = error {
        ui.scope(|ui| {
            ui.set_max_width(COL);
            ui.add(
                egui::Label::new(egui::RichText::new(e).font(crate::theme::ui(12.0)).color(DANGER)).wrap(),
            );
        });
    }
    out
}

// ---------- The screen ----------

pub enum Transfer {
    Export(Box<ExportVault>),
    Import(Box<ImportVault>),
    Text(Box<ExportText>),
}

impl Transfer {
    /// "Export notes…", with `preselect` checked (or the active tab).
    pub fn export_vault(s: &Session, preselect: Option<Uuid>) -> Self {
        Self::Export(Box::new(ExportVault::new(s, preselect)))
    }

    /// "Export as plain text…".
    pub fn export_text(s: &Session) -> Self {
        Self::Text(Box::new(ExportText::new(s)))
    }

    /// The Export button of the tab bar: only this tab, no list of notes.
    pub fn export_tab(s: &Session, id: Uuid) -> Self {
        Self::Text(Box::new(ExportText::one(s, id)))
    }

    /// "Import notes from vault…" from `path` (already chosen).
    pub fn import_vault(path: PathBuf) -> Self {
        Self::Import(Box::new(ImportVault::new(path)))
    }

    /// While a recovery code is on screen, auto-lock waits (as in Settings
    /// → Access): locking would lose a code that is shown only once.
    pub fn holds_lock(&self) -> bool {
        match self {
            Self::Export(e) => e.code.is_some(),
            Self::Import(_) | Self::Text(_) => false,
        }
    }

    pub fn ui(&mut self, ctx: &egui::Context, s: &mut Session) -> Option<Action> {
        match self {
            Self::Export(e) => e.ui(ctx, s),
            Self::Import(i) => i.ui(ctx, s),
            Self::Text(t) => t.ui(ctx, s),
        }
    }
}

// ---------- Export notes… ----------

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExportStep {
    Notes,
    Password,
    Recovery,
    Confirm,
}

enum Exported {
    /// The new password is the one of the open vault: warn once.
    SamePassword,
    Done {
        code: RecoveryCode,
        notes: usize,
    },
    Failed(String),
}

pub struct ExportVault {
    step: ExportStep,
    gate: Option<PasswordCheck>,
    picks: Vec<Pick>,
    password: SecretField,
    confirm: SecretField,
    strength: Option<(u8, String)>,
    /// The warning about reusing the vault password was shown; exporting
    /// again uses it anyway.
    same_password: bool,
    target: Option<PathBuf>,
    job: Option<Job<Exported>>,
    code: Option<RecoveryCode>,
    check: CodeCheck,
    code_actions: CodeActions,
    error: Option<String>,
    /// "Exported 2 notes to x.heim", for the status bar once the page closes.
    exported: Option<String>,
    /// Where to save the new vault, while being asked.
    prompt: Option<PathPrompt>,
}

const NEW_PW: &str = "export_pw";
const NEW_PW2: &str = "export_pw2";

impl ExportVault {
    fn new(s: &Session, preselect: Option<Uuid>) -> Self {
        let content = &s.vault.content;
        let mark = preselect.or(content.active_note);
        Self {
            step: ExportStep::Notes,
            gate: export_needs_password(&content.security).then(password_gate),
            picks: picks(&content.notes, |n| Some(n.id) == mark),
            password: SecretField::default(),
            confirm: SecretField::default(),
            strength: None,
            same_password: false,
            target: None,
            job: None,
            code: None,
            check: CodeCheck::new(),
            code_actions: CodeActions::default(),
            error: None,
            exported: None,
            prompt: None,
        }
    }

    fn ui(&mut self, ctx: &egui::Context, s: &mut Session) -> Option<Action> {
        if let Some(prompt) = self.prompt.as_mut() {
            match prompt.ui(ctx) {
                path_prompt::Outcome::Open => {}
                path_prompt::Outcome::Canceled => self.prompt = None,
                path_prompt::Outcome::Chosen(paths) => {
                    if let Some(path) = paths.into_iter().next() {
                        if is_vault_file(&path, &s.path) {
                            prompt.error =
                                Some("That is the vault that is open. Choose another name.".into());
                        } else {
                            self.prompt = None;
                            self.target = Some(path.clone());
                            self.start(ctx, s, path);
                        }
                    }
                }
            }
        }
        if let Some(result) = self.job.as_ref().and_then(Job::poll) {
            self.job = None;
            match result {
                Exported::SamePassword => self.same_password = true,
                Exported::Done { code, notes } => {
                    let name = self.target.as_deref().map(file_name).unwrap_or_default();
                    self.exported = Some(format!("Exported {} to {name}", plural(notes, "note")));
                    self.code = Some(code);
                    self.password.clear();
                    self.confirm.clear();
                    widgets::forget_fields(ctx, &[NEW_PW, NEW_PW2]);
                    self.step = ExportStep::Recovery;
                }
                Exported::Failed(e) => self.error = Some(e),
            }
        }
        let action = show(ctx, s, self);
        if let Some(gate) = self.gate.as_mut() {
            match gate.ui(ctx, s) {
                Outcome::Open => {}
                Outcome::Canceled => return Some(Action::Notes),
                Outcome::Confirmed => self.gate = None,
            }
        }
        if action.is_some() {
            widgets::forget_fields(ctx, &[NEW_PW, NEW_PW2]);
            if let Some(text) = self.exported.take() {
                s.notify(text, false);
            }
        }
        action
    }

    /// Asks where to save (the first time), then [`Self::start`].
    fn export(&mut self, ctx: &egui::Context, s: &Session) {
        self.error = None;
        if let (Some(p), true) = (&self.target, self.same_password) {
            let p = p.clone();
            self.start(ctx, s, p);
            return;
        }
        let name = format!("{}-export.heim", file_stem(&s.path));
        let start = s
            .path
            .parent()
            .map(|d| d.join(&name))
            .unwrap_or_else(|| name.into());
        self.prompt = Some(
            PathPrompt::new(
                "Export notes",
                "Save",
                path_prompt::Kind::Save { ext: "heim".into() },
                &start,
            )
            .filter("Heimdall vault", &["heim"]),
        );
    }

    /// Runs Argon2id in the background: checks the password against the
    /// open vault, creates the new vault and writes it to `path`.
    fn start(&mut self, ctx: &egui::Context, s: &Session, path: PathBuf) {
        self.error = None;
        let ids = chosen(&self.picks);
        let password = SecretString::from(self.password.text.clone());
        let check_same = !self.same_password;
        let vault = s.vault.duplicate();
        let keys: Vec<Keyfile> = s.usb.iter().map(|k| k.keyfile.clone()).collect();
        self.job = Some(Job::spawn(ctx, move || {
            if check_same {
                let refs: Vec<&Keyfile> = keys.iter().collect();
                if vault.verify_password(&password, &refs).is_ok() {
                    return Exported::SamePassword;
                }
            }
            let result = vault
                .export_notes(&ids, &password, vault.export_kdf())
                .and_then(|new| {
                    let bytes = new.vault.to_bytes()?;
                    write_atomic(&path, &bytes)?;
                    Ok(Exported::Done {
                        notes: new.vault.content.notes.len(),
                        code: new.recovery_code,
                    })
                });
            result.unwrap_or_else(|e| Exported::Failed(error_message(&e)))
        }));
    }

    fn export_name(&self) -> String {
        self.target.as_deref().map(file_name).unwrap_or_default()
    }
}

impl Flow for ExportVault {
    fn title(&self) -> &'static str {
        "Export notes"
    }

    fn steps(&self) -> Vec<&'static str> {
        vec!["Notes", "Password", "Recovery code", "Confirm"]
    }

    fn current(&self) -> usize {
        self.step as usize
    }

    fn body(&mut self, ui: &mut Ui, s: &Session) {
        match self.step {
            ExportStep::Notes => {
                widgets::heading(ui, "Choose notes to export");
                ui.add_space(2.0);
                widgets::paragraph(
                    ui,
                    "They are copied to a new vault file with its own password. Nothing else goes with them: not the trash, your dictionary, your USB keys or your settings.",
                    COL,
                );
                ui.add_space(24.0);
                note_list(ui, &mut self.picks);
            }
            ExportStep::Password => {
                widgets::heading(ui, "Choose a password for the new file");
                ui.add_space(2.0);
                widgets::paragraph(
                    ui,
                    "Whoever opens the file needs only this password, not your USB key. Use a different one from this vault's.",
                    COL,
                );
                ui.add_space(24.0);
                widgets::label(ui, "Password", 12.0, TEXT_MUTED);
                let resp = widgets::text_field(
                    ui,
                    egui::Id::new(NEW_PW),
                    &mut self.password.text,
                    COL,
                    Some(&mut self.password.reveal),
                    "",
                );
                if ui.memory(|m| m.focused().is_none()) && self.job.is_none() && self.gate.is_none() {
                    resp.request_focus();
                }
                if resp.changed() {
                    self.same_password = false;
                }
                if resp.changed() || (self.strength.is_none() && !self.password.text.is_empty()) {
                    self.strength = keys::estimate_strength(&self.password.text);
                }
                ui.add_space(4.0);
                keys::strength_meter(ui, self.strength.as_ref(), COL);
                ui.add_space(20.0);
                widgets::label(ui, "Confirm password", 12.0, TEXT_MUTED);
                let mut reveal = self.password.reveal;
                widgets::text_field(
                    ui,
                    egui::Id::new(NEW_PW2),
                    &mut self.confirm.text,
                    COL,
                    Some(&mut reveal),
                    "",
                );
                self.password.reveal = reveal;
                ui.add_space(4.0);
                keys::match_line(ui, &self.password.text, &self.confirm.text);
                ui.add_space(16.0);
                if self.same_password {
                    ui.scope(|ui| {
                        ui.set_max_width(COL);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(
                                    "This is the password of your current vault. Anyone who gets the file could also open this vault. Choose Export anyway to use it.",
                                )
                                .font(crate::theme::ui(13.0))
                                .color(DANGER),
                            )
                            .wrap(),
                        );
                    });
                    ui.add_space(8.0);
                }
                let kdf = s.vault.export_kdf();
                keys::bullet(
                    ui,
                    "",
                    &format!(
                        "It is as slow to unlock as this vault (Argon2id, {} MiB), so it is just as hard to guess.",
                        kdf.m_kib / 1024
                    ),
                );
                keys::bullet(ui, "", "It gets its own recovery code, shown next.");
            }
            ExportStep::Recovery => {
                widgets::heading(ui, "Recovery code of the new file");
                ui.add_space(2.0);
                widgets::paragraph(
                    ui,
                    &format!(
                        "If the password of {} is forgotten, these 12 words open it. Write them down, or skip this if you don't need a way back.",
                        self.export_name()
                    ),
                    COL,
                );
                ui.add_space(24.0);
                let name = self.export_name();
                let clear_after = s.vault.content.security.clipboard_clear_s;
                let Some(code) = &self.code else { return };
                keys::code_grid(ui, code, COL);
                ui.add_space(14.0);
                self.code_actions.ui(ui, code, &name, clear_after);
                ui.add_space(16.0);
                keys::code_notes(ui);
            }
            ExportStep::Confirm => {
                widgets::heading(ui, "Confirm the recovery code");
                ui.add_space(2.0);
                widgets::paragraph(
                    ui,
                    "Type these three words from your sheet, so we know it was written down correctly.",
                    COL,
                );
                ui.add_space(24.0);
                let Some(code) = &self.code else { return };
                self.check.ui(ui, code, COL);
            }
        }
    }

    fn buttons(&mut self, ui: &mut Ui, s: &mut Session) -> Option<Action> {
        let busy = self.job.is_some();
        let error = self.error.clone();
        match self.step {
            ExportStep::Notes => {
                let any = self.picks.iter().any(|p| p.on);
                let (back, next) = button_row(ui, "Cancel", true, "Continue", any, error.as_deref());
                if back {
                    return Some(Action::Notes);
                }
                if next {
                    self.step = ExportStep::Password;
                }
            }
            ExportStep::Password => {
                let ready = !busy
                    && self
                        .strength
                        .as_ref()
                        .is_some_and(|(score, _)| *score >= keys::MIN_SCORE)
                    && self.password.text == self.confirm.text;
                let label = match (busy, self.same_password) {
                    (true, _) => "Exporting…",
                    (false, true) => "Export anyway",
                    (false, false) => "Export…",
                };
                let (back, next) = button_row(ui, "Back", !busy, label, ready, error.as_deref());
                let enter = ready && crate::focus::enter_submits(ui);
                if back {
                    self.step = ExportStep::Notes;
                    self.error = None;
                } else if next || enter {
                    self.export(ui.ctx(), s);
                }
            }
            ExportStep::Recovery => {
                let (skip, next) = button_row(ui, "Skip", true, "I've written it down", true, None);
                if skip {
                    self.code = None;
                    return Some(Action::Notes);
                }
                if next {
                    self.check = CodeCheck::new();
                    self.step = ExportStep::Confirm;
                }
            }
            ExportStep::Confirm => {
                let done = self.code.as_ref().is_some_and(|c| self.check.complete(c));
                let (back, next) = button_row(ui, "Show the words again", true, "Done", done, None);
                ui.add_space(4.0);
                let skip = widgets::button(ui, ButtonKind::Quiet, "Skip", true, vec2(0.0, 32.0)).clicked();
                if back {
                    self.step = ExportStep::Recovery;
                } else if next || skip {
                    self.code = None;
                    return Some(Action::Notes);
                }
            }
        }
        None
    }
}

// ---------- Import notes from vault… ----------

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImportStep {
    Unlock,
    Notes,
}

pub struct ImportVault {
    path: PathBuf,
    locked: Result<LockedVault, String>,
    step: ImportStep,
    password: SecretField,
    recovery: SecretField,
    use_recovery: bool,
    /// This vault's USB keys, while it needs one.
    keys: Option<Scan<usb::Lookup>>,
    job: Option<Job<Result<Vault, Error>>>,
    /// The other vault, open while its notes are chosen. Dropping it
    /// wipes it.
    source: Option<Vault>,
    picks: Vec<Pick>,
    error: Option<String>,
}

const IMPORT_PW: &str = "import_pw";
const IMPORT_CODE: &str = "import_code";

impl ImportVault {
    fn new(path: PathBuf) -> Self {
        let locked = heimdall_core::storage::read_vault(&path)
            .and_then(|b| LockedVault::parse(&b))
            .map_err(|e| error_message(&e));
        let keys = locked
            .as_ref()
            .ok()
            .filter(|l| l.requires_keyfile())
            .map(|l| usb::lookup_scan(*l.vault_id()));
        Self {
            path,
            locked,
            step: ImportStep::Unlock,
            password: SecretField::default(),
            recovery: SecretField::default(),
            use_recovery: false,
            keys,
            job: None,
            source: None,
            picks: Vec::new(),
            error: None,
        }
    }

    /// The connected USB key that opens the other vault, if any.
    fn key(&self) -> Option<&usb::UsbKey> {
        let (Ok(locked), Some(scan)) = (&self.locked, &self.keys) else {
            return None;
        };
        scan.value
            .as_ref()?
            .keys
            .iter()
            .find(|k| locked.accepts_keyfile(&k.keyfile))
    }

    fn can_open(&self) -> bool {
        if self.job.is_some() || self.locked.is_err() {
            return false;
        }
        if self.use_recovery {
            !self.recovery.text.trim().is_empty()
        } else {
            !self.password.text.is_empty() && (self.keys.is_none() || self.key().is_some())
        }
    }

    fn open(&mut self, ctx: &egui::Context) {
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
            let password = self.password.take();
            let key = self.key().map(|k| k.keyfile.clone());
            self.job = Some(Job::spawn(ctx, move || {
                locked.unlock_with_password(&password, key.as_ref())
            }));
        }
    }

    fn ui(&mut self, ctx: &egui::Context, s: &mut Session) -> Option<Action> {
        if let Some(keys) = self.keys.as_mut().filter(|_| !self.use_recovery) {
            keys.poll(ctx);
        }
        if let Some(result) = self.job.as_ref().and_then(Job::poll) {
            self.job = None;
            match result {
                Ok(vault) => {
                    self.picks = picks(&vault.content.notes, |_| true);
                    self.source = Some(vault);
                    self.step = ImportStep::Notes;
                    widgets::forget_fields(ctx, &[IMPORT_PW, IMPORT_CODE]);
                }
                Err(e) => self.error = Some(error_message(&e)),
            }
        }
        let action = show(ctx, s, self);
        if action.is_some() {
            widgets::forget_fields(ctx, &[IMPORT_PW, IMPORT_CODE]);
        }
        action
    }

    fn import(&mut self, s: &mut Session) -> Action {
        let ids = chosen(&self.picks);
        let Some(source) = self.source.take() else {
            return Action::Notes;
        };
        let notes: Vec<Note> = source
            .content
            .notes
            .iter()
            .filter(|n| ids.contains(&n.id))
            .cloned()
            .collect();
        drop(source);
        let added = s.vault.content.import_notes(notes);
        for id in &added {
            s.touch(Some(*id));
        }
        s.notify(
            format!(
                "Imported {} from {}",
                plural(added.len(), "note"),
                file_name(&self.path)
            ),
            false,
        );
        Action::Notes
    }
}

impl Flow for ImportVault {
    fn title(&self) -> &'static str {
        "Import notes"
    }

    fn steps(&self) -> Vec<&'static str> {
        vec!["Open vault", "Notes"]
    }

    fn current(&self) -> usize {
        self.step as usize
    }

    fn body(&mut self, ui: &mut Ui, _s: &Session) {
        match self.step {
            ImportStep::Unlock => {
                widgets::heading(ui, "Open the other vault");
                ui.add_space(2.0);
                widgets::paragraph(
                    ui,
                    "Its notes are copied into this vault as new tabs. The other file isn't changed.",
                    COL,
                );
                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new(file_name(&self.path))
                        .font(mono(13.0))
                        .color(TEXT_BODY),
                );
                ui.add_space(20.0);
                if let Err(e) = &self.locked {
                    widgets::label(ui, e, 13.0, DANGER);
                    return;
                }
                if self.use_recovery {
                    widgets::label(ui, "Recovery code", 12.0, TEXT_MUTED);
                    let resp = widgets::text_field(
                        ui,
                        egui::Id::new(IMPORT_CODE),
                        &mut self.recovery.text,
                        COL,
                        None,
                        "12 words, separated by spaces",
                    );
                    if ui.memory(|m| m.focused().is_none()) && self.job.is_none() {
                        resp.request_focus();
                    }
                } else {
                    widgets::label(ui, "Password", 12.0, TEXT_MUTED);
                    let resp = widgets::text_field(
                        ui,
                        egui::Id::new(IMPORT_PW),
                        &mut self.password.text,
                        COL,
                        Some(&mut self.password.reveal),
                        "",
                    );
                    if ui.memory(|m| m.focused().is_none()) && self.job.is_none() {
                        resp.request_focus();
                    }
                    if self.keys.is_some() {
                        ui.add_space(14.0);
                        let (text, color) = match (self.key(), &self.keys) {
                            (Some(k), _) => (format!("USB key: {}", k.label()), TEXT_BODY),
                            (None, Some(scan)) if scan.value.is_none() => {
                                ("Looking for its USB key…".to_owned(), TEXT_MUTED)
                            }
                            _ => ("Connect the USB key of this vault.".to_owned(), TEXT_MUTED),
                        };
                        widgets::label(ui, &text, 13.0, color);
                    }
                }
                ui.add_space(14.0);
                let switch = if self.use_recovery {
                    "Use the password instead"
                } else {
                    "Use the recovery code instead"
                };
                if widgets::link(ui, switch, 12.0).clicked() {
                    self.use_recovery = !self.use_recovery;
                    self.error = None;
                }
                if self.job.is_some() {
                    ui.add_space(14.0);
                    widgets::label(ui, "Opening…", 12.0, TEXT_MUTED);
                }
            }
            ImportStep::Notes => {
                widgets::heading(ui, "Choose notes to import");
                ui.add_space(2.0);
                widgets::paragraph(
                    ui,
                    "They are added at the end, as new tabs. A title that is already taken gets a number.",
                    COL,
                );
                ui.add_space(24.0);
                note_list(ui, &mut self.picks);
            }
        }
    }

    fn buttons(&mut self, ui: &mut Ui, s: &mut Session) -> Option<Action> {
        let error = self.error.clone();
        match self.step {
            ImportStep::Unlock => {
                let ready = self.can_open();
                let label = if self.job.is_some() { "Opening…" } else { "Open" };
                let (back, next) =
                    button_row(ui, "Cancel", self.job.is_none(), label, ready, error.as_deref());
                let enter = ready && crate::focus::enter_submits(ui);
                if back {
                    return Some(Action::Notes);
                }
                if next || enter {
                    self.open(ui.ctx());
                }
            }
            ImportStep::Notes => {
                let n = self.picks.iter().filter(|p| p.on).count();
                let label = format!("Import {}", plural(n, "note"));
                let (back, next) = button_row(ui, "Cancel", true, &label, n > 0, None);
                if back {
                    self.source = None;
                    return Some(Action::Notes);
                }
                if next {
                    return Some(self.import(s));
                }
            }
        }
        None
    }
}

// ---------- Export as plain text… ----------

#[derive(Clone, Copy, PartialEq, Eq)]
enum TextStep {
    Warning,
    Notes,
}

pub struct ExportText {
    step: TextStep,
    /// Asks for the vault password after "I understand", if required.
    needs_password: bool,
    gate: Option<PasswordCheck>,
    picks: Vec<Pick>,
    secrets: Secrets,
    error: Option<String>,
    /// Where to save, while being asked.
    prompt: Option<PathPrompt>,
    /// From the Export button: this tab only, shown by this title.
    single: Option<(Uuid, String)>,
}

impl ExportText {
    fn one(s: &Session, id: Uuid) -> Self {
        let mut export = Self::new(s);
        export.picks = picks(&s.vault.content.notes, |n| n.id == id);
        let shown = s
            .vault
            .content
            .note(id)
            .map(|n| heimdall_core::secret::hide(n.display_title()))
            .unwrap_or_default();
        export.single = Some((id, shown));
        export
    }

    /// The step after the warning (and the password): for one note
    /// without secrets there is nothing to choose, so straight to saving.
    fn after_warning(&mut self, s: &mut Session) -> Option<Action> {
        self.step = TextStep::Notes;
        let plain = self.single.as_ref().is_some_and(|(id, _)| {
            s.vault
                .content
                .note(*id)
                .is_some_and(|n| heimdall_core::secret::scan(&n.body).secrets.is_empty())
        });
        if plain {
            return self.export(s);
        }
        None
    }

    fn new(s: &Session) -> Self {
        let content = &s.vault.content;
        let active = content.active_note;
        Self {
            step: TextStep::Warning,
            needs_password: export_needs_password(&content.security),
            gate: None,
            picks: picks(&content.notes, |n| Some(n.id) == active),
            secrets: Secrets::Hide,
            error: None,
            prompt: None,
            single: None,
        }
    }

    fn ui(&mut self, ctx: &egui::Context, s: &mut Session) -> Option<Action> {
        let mut action = show(ctx, s, self);
        if let Some(prompt) = self.prompt.as_mut() {
            match prompt.ui(ctx) {
                path_prompt::Outcome::Open => {}
                path_prompt::Outcome::Canceled => self.prompt = None,
                path_prompt::Outcome::Chosen(paths) => {
                    if let Some(path) = paths.into_iter().next() {
                        if is_vault_file(&path, &s.path) {
                            prompt.error = Some("That is the vault file. Choose another name.".into());
                        } else {
                            self.prompt = None;
                            action = self.write(s, &path).or(action);
                        }
                    }
                }
            }
        }
        if let Some(gate) = self.gate.as_mut() {
            match gate.ui(ctx, s) {
                Outcome::Open => {}
                Outcome::Canceled => self.gate = None,
                Outcome::Confirmed => {
                    self.gate = None;
                    self.needs_password = false;
                    action = self.after_warning(s).or(action);
                }
            }
        }
        action
    }

    /// The files to write: one per note, and the name to suggest with its
    /// extension (a `.zip` for several notes).
    fn files(&self, s: &Session) -> (Vec<plain::TextFile>, String, String) {
        let ids = chosen(&self.picks);
        let notes: Vec<&Note> = s
            .vault
            .content
            .notes
            .iter()
            .filter(|n| ids.contains(&n.id))
            .collect();
        let crlf = s.vault.content.editor.line_ending == heimdall_core::content::LineEnding::Crlf;
        let files = plain::export_files(&notes, self.secrets, crlf);
        let (name, ext) = match files.as_slice() {
            [one] => {
                let ext = one.name.rsplit_once('.').map_or("txt", |(_, e)| e).to_owned();
                (one.name.clone(), ext)
            }
            _ => (format!("{}-notes.zip", file_stem(&s.path)), "zip".to_owned()),
        };
        (files, name, ext)
    }

    /// Asks where to save, then [`Self::write`].
    fn export(&mut self, s: &mut Session) -> Option<Action> {
        self.error = None;
        let (_, name, ext) = self.files(s);
        let filter = if ext == "zip" { "Zip archive" } else { "Text file" };
        let start = s
            .path
            .parent()
            .map(|d| d.join(&name))
            .unwrap_or_else(|| name.clone().into());
        let mut prompt = PathPrompt::new(
            "Export as plain text",
            "Save",
            path_prompt::Kind::Save { ext: ext.clone() },
            &start,
        );
        prompt = prompt.filter(filter, &[ext.as_str()]);
        self.prompt = Some(prompt);
        None
    }

    /// Writes one file, or a `.zip` for several.
    fn write(&mut self, s: &mut Session, path: &Path) -> Option<Action> {
        let (files, _, _) = self.files(s);
        let written = match files.as_slice() {
            [one] => write_atomic(path, one.text.as_bytes()).map_err(|e| error_message(&e)),
            _ => {
                plain::zip(&files).and_then(|bytes| write_atomic(path, &bytes).map_err(|e| error_message(&e)))
            }
        };
        match written {
            Ok(()) => {
                s.notify(
                    format!(
                        "Exported {} as plain text to {}",
                        plural(files.len(), "note"),
                        file_name(path)
                    ),
                    false,
                );
                Some(Action::Notes)
            }
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }
}

/// A radio button with a title and a description under it.
fn radio(ui: &mut Ui, selected: bool, title: &str, description: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(COL, 44.0), egui::Sense::click());
    let painter = ui.painter();
    let c = egui::pos2(rect.left() + 8.0, rect.top() + 11.0);
    painter.circle_stroke(
        c,
        7.0,
        Stroke::new(1.5_f32, if resp.hovered() { TEXT_BODY } else { TEXT_MUTED }),
    );
    if selected {
        painter.circle_filled(c, 4.0, TEXT);
    }
    painter.text(
        egui::pos2(rect.left() + 26.0, rect.top() + 2.0),
        egui::Align2::LEFT_TOP,
        title,
        crate::theme::ui(13.0),
        if selected || resp.hovered() {
            TEXT
        } else {
            TEXT_BODY
        },
    );
    painter.text(
        egui::pos2(rect.left() + 26.0, rect.top() + 21.0),
        egui::Align2::LEFT_TOP,
        description,
        crate::theme::ui(12.0),
        TEXT_MUTED,
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.clicked()
}

impl Flow for ExportText {
    fn title(&self) -> &'static str {
        "Export as plain text"
    }

    fn steps(&self) -> Vec<&'static str> {
        match self.single {
            Some(_) => vec!["Warning", "Secrets"],
            None => vec!["Warning", "Notes"],
        }
    }

    fn current(&self) -> usize {
        self.step as usize
    }

    fn body(&mut self, ui: &mut Ui, _s: &Session) {
        match self.step {
            TextStep::Warning => {
                widgets::heading(ui, "Export as plain text");
                ui.add_space(2.0);
                widgets::paragraph(
                    ui,
                    "Plain text files are not encrypted. Anyone with access to this computer, its backups or its cloud sync can read them.",
                    COL,
                );
                ui.add_space(20.0);
                keys::bullet(
                    ui,
                    "Heimdall can't take them back.",
                    " OneDrive, File History or the Recycle Bin may keep copies after you delete them.",
                );
                keys::bullet(
                    ui,
                    "",
                    "To share notes safely, use Export notes… instead: it makes an encrypted vault file.",
                );
            }
            TextStep::Notes => {
                if let Some((_, shown)) = &self.single {
                    widgets::heading(ui, &format!("Export \"{shown}\""));
                    ui.add_space(2.0);
                    widgets::paragraph(
                        ui,
                        "This note has masked secrets. How should the file show them?",
                        COL,
                    );
                    ui.add_space(24.0);
                } else {
                    widgets::heading(ui, "Choose notes to export");
                    ui.add_space(2.0);
                    widgets::paragraph(
                        ui,
                        "One note is saved as a text file; several, as a .zip with one file per note.",
                        COL,
                    );
                    ui.add_space(24.0);
                    note_list(ui, &mut self.picks);
                    ui.add_space(28.0);
                }
                widgets::label(ui, "Masked secrets", 12.0, TEXT_MUTED);
                ui.add_space(10.0);
                for (value, title, description) in [
                    (Secrets::Hide, "Hide", "Each secret is written as [secret]."),
                    (
                        Secrets::Keep,
                        "Keep markers",
                        "Written as {*value*}. Imported again, they are masked again.",
                    ),
                    (Secrets::Reveal, "Reveal", "The value is written in plain text."),
                ] {
                    if radio(ui, self.secrets == value, title, description) {
                        self.secrets = value;
                    }
                    ui.add_space(4.0);
                }
            }
        }
    }

    fn buttons(&mut self, ui: &mut Ui, s: &mut Session) -> Option<Action> {
        let error = self.error.clone();
        match self.step {
            TextStep::Warning => {
                let (back, next) = button_row(ui, "Cancel", true, "I understand", true, None);
                if back {
                    return Some(Action::Notes);
                }
                if next {
                    if self.needs_password {
                        self.gate = Some(password_gate());
                    } else {
                        return self.after_warning(s);
                    }
                }
            }
            TextStep::Notes => {
                let any = self.picks.iter().any(|p| p.on);
                let (back, next) = button_row(ui, "Back", true, "Export…", any, error.as_deref());
                if back {
                    self.step = TextStep::Warning;
                    self.error = None;
                }
                if next {
                    return self.export(s);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_copy_is_exact_and_refuses_the_vault() {
        let dir = std::env::temp_dir().join(format!("heimdall-backup-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let vault = dir.join("notes.heim");
        let bytes: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
        std::fs::write(&vault, &bytes).unwrap();

        let copy = dir.join("notes-backup-2026-11-07.heim");
        copy_verified(&vault, &copy).unwrap();
        assert_eq!(std::fs::read(&copy).unwrap(), bytes);

        assert!(is_vault_file(&vault, &vault));
        assert!(is_vault_file(&backup_path(&vault), &vault));
        assert!(!is_vault_file(&copy, &vault));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
