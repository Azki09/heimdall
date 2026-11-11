//! Where to open or save a file: a field to paste a path, next to Browse…
//! for the system dialog (DESIGN §10.9). Browse works as before: what is
//! picked there is used right away (the system dialog already asked about
//! replacing a file). A pasted path is checked here first.

use std::path::{Path, PathBuf};

use eframe::egui::{self, vec2, CornerRadius, Stroke};

use crate::theme::*;
use crate::widgets::{self, ButtonKind};

const W: f32 = 460.0;
const FIELD: &str = "path_prompt";

pub enum Kind {
    /// One existing file.
    Open,
    /// One or more existing files (Browse can pick several).
    OpenMany,
    /// A file to write, given this extension when it has none.
    Save { ext: String },
}

pub enum Outcome {
    Open,
    Canceled,
    Chosen(Vec<PathBuf>),
}

pub struct PathPrompt {
    title: &'static str,
    button: &'static str,
    kind: Kind,
    /// System dialog filters: (name, extensions).
    filters: Vec<(String, Vec<String>)>,
    text: String,
    /// Shown under the field; the caller can set it to refuse a path.
    pub error: Option<String>,
    /// A pasted path that exists, waiting for "Replace".
    replace: Option<PathBuf>,
    focus: bool,
}

impl PathPrompt {
    /// `suggested` fills the field: a folder (ends with a separator) or a
    /// full path.
    pub fn new(title: &'static str, button: &'static str, kind: Kind, suggested: &Path) -> Self {
        let mut text = suggested.display().to_string();
        if suggested.is_dir() && !text.ends_with(std::path::MAIN_SEPARATOR) {
            text.push(std::path::MAIN_SEPARATOR);
        }
        Self {
            title,
            button,
            kind,
            filters: Vec::new(),
            text,
            error: None,
            replace: None,
            focus: true,
        }
    }

    pub fn filter(mut self, name: &str, exts: &[&str]) -> Self {
        self.filters
            .push((name.to_owned(), exts.iter().map(|e| (*e).to_owned()).collect()));
        self
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Outcome {
        let mut submit = false;
        let mut cancel = false;
        let mut browse = false;
        let mut replace = false;
        let modal = egui::Modal::new(egui::Id::new("path_prompt"))
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
                let help = match self.kind {
                    Kind::Save { .. } => "Paste or type where to save it, or browse for a place.",
                    Kind::Open | Kind::OpenMany => "Paste or type the path of the file, or browse for it.",
                };
                widgets::paragraph(ui, help, W);
                ui.add_space(16.0);
                widgets::label(ui, "Path", 12.0, TEXT_MUTED);
                let before = self.text.clone();
                let resp = widgets::text_field(ui, egui::Id::new(FIELD), &mut self.text, W, None, "");
                if self.text != before {
                    self.error = None;
                    self.replace = None;
                }
                if std::mem::take(&mut self.focus) {
                    resp.request_focus();
                }
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                submit |= enter && !self.text.trim().is_empty();
                ui.add_space(10.0);
                if let Some(p) = &self.replace {
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    widgets::label(ui, &format!("{name} already exists. Replace it?"), 12.0, DANGER);
                } else if let Some(e) = &self.error {
                    widgets::label(ui, e, 12.0, DANGER);
                }
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if widgets::button(ui, ButtonKind::Secondary, "Cancel", true, vec2(0.0, 40.0)).clicked() {
                        cancel = true;
                    }
                    ui.add_space(4.0);
                    if widgets::button(ui, ButtonKind::Secondary, "Browse…", true, vec2(0.0, 40.0)).clicked()
                    {
                        browse = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.replace.is_some() {
                            if widgets::button(ui, ButtonKind::Danger, "Replace", true, vec2(0.0, 40.0))
                                .clicked()
                            {
                                replace = true;
                            }
                        } else {
                            let ready = !self.text.trim().is_empty();
                            if widgets::button(ui, ButtonKind::Primary, self.button, ready, vec2(0.0, 40.0))
                                .clicked()
                            {
                                submit = true;
                            }
                        }
                    });
                });
            });
        if modal.should_close() || cancel {
            widgets::forget_fields(ctx, &[FIELD]);
            return Outcome::Canceled;
        }
        if browse {
            if let Some(paths) = self.browse() {
                return Outcome::Chosen(paths);
            }
            self.focus = self.replace.is_none();
        }
        if replace {
            if let Some(p) = self.replace.take() {
                return Outcome::Chosen(vec![p]);
            }
        }
        if submit {
            match self.check() {
                Ok(Some(path)) => return Outcome::Chosen(vec![path]),
                Ok(None) => {}
                Err(e) => self.error = Some(e),
            }
        }
        Outcome::Open
    }

    /// The system dialog, starting where the field points.
    fn browse(&mut self) -> Option<Vec<PathBuf>> {
        let typed = parse(&self.text);
        let mut dialog = rfd::FileDialog::new().set_title(self.title);
        for (name, exts) in &self.filters {
            dialog = dialog.add_filter(name, exts);
        }
        if let Some(p) = &typed {
            let (dir, name) = if p.is_dir() {
                (Some(p.as_path()), None)
            } else {
                (p.parent().filter(|d| d.is_dir()), p.file_name())
            };
            if let Some(dir) = dir {
                dialog = dialog.set_directory(dir);
            }
            if let (Kind::Save { .. }, Some(name)) = (&self.kind, name) {
                dialog = dialog.set_file_name(name.to_string_lossy());
            }
        }
        match &self.kind {
            Kind::Open => dialog.pick_file().map(|p| vec![p]),
            Kind::OpenMany => dialog.pick_files(),
            Kind::Save { ext } => {
                let picked = dialog.save_file()?;
                // The system dialog asked about replacing the name as picked;
                // with the extension added, it may be another file.
                let had_ext = picked.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext));
                let path = with_ext(picked, ext);
                if !had_ext && path.exists() {
                    self.text = path.display().to_string();
                    self.replace = Some(path);
                    return None;
                }
                Some(vec![path])
            }
        }
    }

    /// Checks a typed or pasted path. `Ok(None)`: it exists and needs a
    /// "Replace" first.
    fn check(&mut self) -> Result<Option<PathBuf>, String> {
        let path = parse(&self.text).ok_or("Type or paste a path.")?;
        match &self.kind {
            Kind::Open | Kind::OpenMany => {
                if path.is_dir() {
                    return Err("That is a folder. Add the file name.".into());
                }
                if !path.is_file() {
                    return Err("There is no file there. Check the path.".into());
                }
                Ok(Some(path))
            }
            Kind::Save { ext } => {
                if path.is_dir() || self.text.trim_end().ends_with(['/', '\\']) {
                    return Err("That is a folder. Add a file name at the end.".into());
                }
                let path = with_ext(path, ext);
                match path.parent().filter(|d| !d.as_os_str().is_empty()) {
                    Some(dir) if !dir.is_dir() => {
                        return Err(format!("The folder {} doesn't exist.", dir.display()));
                    }
                    None => return Err("Use a full path, starting with the drive or folder.".into()),
                    _ => {}
                }
                if path.exists() {
                    self.replace = Some(path);
                    return Ok(None);
                }
                Ok(Some(path))
            }
        }
    }
}

/// A typed or pasted path, cleaned up: surrounding spaces and quotes
/// (Windows' "Copy as path" adds them) go, and `~` is the home folder.
pub fn parse(text: &str) -> Option<PathBuf> {
    let mut t = text.trim();
    for q in ['"', '\''] {
        if t.len() >= 2 && t.starts_with(q) && t.ends_with(q) {
            t = t[1..t.len() - 1].trim();
        }
    }
    if t.is_empty() {
        return None;
    }
    if t == "~" || t.starts_with("~/") || t.starts_with("~\\") {
        if let Some(home) = home_dir() {
            return Some(if t == "~" { home } else { home.join(&t[2..]) });
        }
    }
    Some(PathBuf::from(t))
}

/// The user's home folder: `%USERPROFILE%` on Windows, `$HOME` elsewhere.
pub fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// Where a new vault or an "Open vault" prompt starts: the Documents
/// folder, else the home folder.
pub fn documents_dir() -> PathBuf {
    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
    let docs = home.join("Documents");
    if docs.is_dir() {
        docs
    } else {
        home
    }
}

/// `path` with `ext` added when it has another extension or none.
pub fn with_ext(path: PathBuf, ext: &str) -> PathBuf {
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)) {
        return path;
    }
    let mut s = path.into_os_string();
    s.push(".");
    s.push(ext);
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_and_spaces_are_removed() {
        assert_eq!(
            parse("  \"C:\\Users\\a b\\v.heim\" "),
            Some(PathBuf::from("C:\\Users\\a b\\v.heim"))
        );
        assert_eq!(parse("'/home/a/v.heim'"), Some(PathBuf::from("/home/a/v.heim")));
        assert_eq!(parse("   "), None);
        assert_eq!(parse("\"\""), None);
    }

    #[test]
    fn tilde_is_the_home_folder() {
        if let Some(home) = home_dir() {
            assert_eq!(parse("~/notes/v.heim"), Some(home.join("notes/v.heim")));
            assert_eq!(parse("~"), Some(home));
        }
        assert_eq!(parse("a~b"), Some(PathBuf::from("a~b")));
    }

    #[test]
    fn extension_is_added_once() {
        assert_eq!(with_ext(PathBuf::from("a/v"), "heim"), PathBuf::from("a/v.heim"));
        assert_eq!(
            with_ext(PathBuf::from("a/v.HEIM"), "heim"),
            PathBuf::from("a/v.HEIM")
        );
        assert_eq!(
            with_ext(PathBuf::from("a/v.txt"), "heim"),
            PathBuf::from("a/v.txt.heim")
        );
    }
}
