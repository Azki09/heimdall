//! The full-screen editor (DESIGN §17.3), in the style of nano. It runs on
//! the alternate screen, so nothing of the note stays in the terminal's
//! scrollback.
//!
//! ```text
//!  Heimdall  personal                 hola.txt                       Modified
//! --------------------------------------------------------------------------
//! (the note)
//! --------------------------------------------------------------------------
//!  Ln 5, Col 24                                                  Lock in 0:42
//!  ^S Save        ^Q Quit        ^F Find        ^R Reveal      ^G Help
//! ```
//!
//! Notes are drawn with no colors at all, only keyboard characters. Code
//! mode (`^T`, §17.4) is the one exception: line numbers, the app's syntax
//! colors, and a braille minimap on the right that the mouse wheel scrolls.

use std::io::{self, Stdout, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::style::{Attribute, Color, Print, SetAttribute, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};
use heimdall_core::content::{CodeMode, Note, TabInsert};
use heimdall_core::secret;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use super::paths;
use super::prompt::{self, Completion, TICK};
use super::session::{LockEvent, TermSession};
use super::text::{str_width, truncate, Cell, Doc, Glyph, SECRET, TAB_W};
use crate::code::{self, Kind, Language, Span};
use crate::plain::{self, Secrets};
use crate::support::{error_message, MAX_NOTE_BYTES};

/// Which note the editor works on.
pub enum Target {
    Note(Uuid),
    /// A title with no note yet: it is created by the first save.
    New(String),
}

/// How the editor ended.
pub enum Exit {
    Quit,
    Locked {
        event: LockEvent,
        /// `Some(Ok)` = there were changes and they were saved.
        saved: Option<Result<(), String>>,
    },
}

/// A message in the status line goes away after this long.
const MESSAGE_FOR: Duration = Duration::from_secs(4);
/// Rows that are not text: header, two rules, status, shortcuts.
const CHROME_ROWS: usize = 5;
const MIN_COLS: usize = 30;
const MIN_ROWS: usize = CHROME_ROWS + 2;

/// The minimap (code mode): braille chars, each 2 x 4 dots.
const MAP_COLS: usize = 9;
/// Code lines per minimap row, and columns of code per dot.
const MAP_LINES: usize = 4;
const MAP_SCALE: usize = 4;
/// Narrower than this, the minimap gives its room to the code.
const MAP_MIN_COLS: usize = 70;
/// Lines one notch of the wheel scrolls: on the code, and on the minimap.
const WHEEL_CODE: isize = 3;
const WHEEL_MAP: isize = 3 * MAP_LINES as isize;

/// The app's greys (`theme.rs`), for the gutter and the minimap.
const GREY_DIM: Color = grey(0x6E);
const GREY_VIEW: Color = grey(0xB8);
const GREY_VIEW_BG: Color = grey(0x26);

const fn grey(v: u8) -> Color {
    Color::Rgb { r: v, g: v, b: v }
}

const KEYS_MAIN: &[&str] = &["^S Save", "^Q Quit", "^F Find", "^R Reveal", "^G Help"];
const KEYS_CODE: &[&str] = &["^S Save", "^Q Quit", "^F Find", "^T Code", "^G Help"];
const KEYS_FIND: &[&str] = &["Enter Next", "^P Previous", "Esc Cancel"];
const KEYS_QUIT: &[&str] = &["Y Yes", "N No", "Esc Cancel"];
const KEYS_SAVE_AS: &[&str] = &["Enter Save", "Esc Cancel"];
const KEYS_EXPORT: &[&str] = &["Enter Next", "Tab Complete", "Esc Cancel"];
const KEYS_SECRETS: &[&str] = &["H [secret]", "K {*...*}", "R Real values", "Esc Cancel"];

pub const HELP: &[(&str, &str)] = &[
    ("^S", "Save"),
    ("^O", "Save as (rename)"),
    ("^E", "Export to a file"),
    ("^Q", "Quit"),
    ("^F", "Find"),
    ("^P", "Find previous"),
    ("^R", "Show / hide secret"),
    ("^Z", "Undo"),
    ("^Y", "Redo"),
    ("^K", "Cut line"),
    ("^U", "Paste"),
    ("^T", "Code mode on / off"),
    ("^L", "Lock now"),
    ("^G", "Help"),
];

enum Mode {
    Edit,
    Find,
    ConfirmQuit,
    Help,
    /// `^O`: the new title, in `field`.
    SaveAs,
    /// `^E`, first step: where, in `field`.
    ExportPath,
    /// `^E`: what to do with the note's secrets.
    ExportSecrets(PathBuf),
    /// `^E`, last step: the file won't be encrypted.
    ExportConfirm {
        path: PathBuf,
        secrets: Secrets,
        replace: bool,
    },
}

/// The alternate screen in raw mode, for as long as this lives.
struct Screen {
    out: Stdout,
    mouse: bool,
}

impl Screen {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen, EnableBracketedPaste)?;
        Ok(Self { out, mouse: false })
    }

    /// Listening to the mouse takes the terminal's own selection away
    /// (Shift+drag still selects), so it is on only in code mode.
    fn set_mouse(&mut self, on: bool) -> io::Result<()> {
        if on != self.mouse {
            self.mouse = on;
            if on {
                execute!(self.out, EnableMouseCapture)?;
            } else {
                execute!(self.out, DisableMouseCapture)?;
            }
        }
        Ok(())
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// The start of a note, enough to guess its language from.
fn head(text: &str) -> &str {
    let mut end = text.len().min(8 * 1024);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Back to the normal screen. Also called from the panic hook.
pub fn restore_terminal() {
    let mut out = io::stdout();
    // Wipe the alternate screen before leaving it: some terminals keep it.
    let _ = execute!(
        out,
        SetAttribute(Attribute::Reset),
        Clear(ClearType::All),
        DisableMouseCapture,
        DisableBracketedPaste,
        LeaveAlternateScreen,
        Show
    );
    let _ = terminal::disable_raw_mode();
}

/// What the last draw put where, for the mouse.
#[derive(Default, Clone, Copy)]
struct Geometry {
    text_rows: usize,
    /// First screen column of the minimap panel, if it is showing.
    map_x: Option<usize>,
    /// First minimap row on screen.
    map_top: usize,
}

/// Things worked out from the text, redone only when it changes.
#[derive(Default)]
struct Cache {
    version: Option<u64>,
    spans: Vec<Span>,
    /// The minimap, one string of braille per [`MAP_LINES`] lines.
    map: Vec<String>,
}

struct Editor<'a> {
    session: &'a mut TermSession,
    target: Target,
    title: String,
    doc: Doc,
    mode: Mode,
    /// First text row on screen.
    top: usize,
    /// First text column on screen (code mode scrolls sideways).
    left: usize,
    query: String,
    message: Option<(String, Instant)>,
    /// Lines cut with `^K`, pasted with `^U`.
    cut: Zeroizing<String>,
    /// The last key was `^K`: the next one adds to `cut`.
    cutting: bool,
    tab: String,
    mask_pasted: bool,
    /// Code mode (`^T`).
    code: bool,
    /// Code mode was turned off by hand ([`Note::code_off`]).
    code_off: bool,
    /// What is typed in Save as and Export.
    field: String,
    /// The last name suggested (`tets.py`), so `^S` says it once.
    suggested: Option<String>,
    /// The language the note was saved with ("" = detect it).
    language_id: String,
    language: Option<&'static Language>,
    /// 24-bit color: the app's palette. Without it, bold and dim.
    truecolor: bool,
    geometry: Geometry,
    cache: Cache,
}

/// Opens `target` in the editor until `^Q` or a lock. `title` is what the
/// header shows.
pub fn edit(session: &mut TermSession, target: Target, title: String) -> io::Result<Exit> {
    let (doc, code_mode, code_off, own_title) = match &target {
        Target::Note(id) => {
            let note = session
                .vault
                .content
                .note(*id)
                .ok_or_else(|| io::Error::other("the note is gone"))?;
            let cursor = usize::try_from(note.cursor).unwrap_or(usize::MAX);
            (
                Doc::new(&note.body, cursor, MAX_NOTE_BYTES),
                note.code.clone(),
                note.code_off,
                note.title.clone(),
            )
        }
        Target::New(title) => (Doc::new("", 0, MAX_NOTE_BYTES), None, false, title.clone()),
    };
    // A note named like a source file (`x.py`, `notes.md`) is in code
    // mode, unless it was turned off by hand (DESIGN §10.6).
    let from_extension =
        (code_mode.is_none() && !code_off && session.vault.content.editor.code_from_extension)
            .then(|| code::by_title(&own_title))
            .flatten();
    let code_mode = code_mode.or_else(|| {
        from_extension.map(|_| CodeMode {
            language: String::new(),
        })
    });
    let editor_settings = &session.vault.content.editor;
    let tab = match editor_settings.tab_inserts {
        TabInsert::Spaces(n) => " ".repeat(usize::from(n.clamp(1, 16))),
        TabInsert::Tab => "\t".to_owned(),
    };
    let mask_pasted = editor_settings.mask_pasted;
    let truecolor = std::env::var("COLORTERM").is_ok_and(|v| v.contains("truecolor") || v.contains("24bit"));
    let mut editor = Editor {
        session,
        target,
        title,
        doc,
        mode: Mode::Edit,
        top: 0,
        left: 0,
        query: String::new(),
        message: None,
        cut: Zeroizing::new(String::new()),
        cutting: false,
        tab,
        mask_pasted,
        code: code_mode.is_some(),
        code_off,
        field: String::new(),
        suggested: None,
        language_id: code_mode.map(|c| c.language).unwrap_or_default(),
        language: None,
        truecolor,
        geometry: Geometry::default(),
        cache: Cache::default(),
    };
    editor.apply_code_mode();
    if matches!(editor.target, Target::New(_)) {
        editor.say("[ New note. It is created when you save ]");
    } else if let (Some(language), Some((_, ext))) = (from_extension, own_title.rsplit_once('.')) {
        editor.say(format!(
            "[ Code mode: {}, from .{ext}. ^T turns it off ]",
            language.name
        ));
    } else if editor.code {
        editor.suggest_extension();
    }
    let mut screen = Screen::enter()?;
    let exit = editor.run(&mut screen);
    drop(screen);
    exit
}

impl Editor<'_> {
    fn run(&mut self, screen: &mut Screen) -> io::Result<Exit> {
        loop {
            if let Some(event) = self.session.lock_event() {
                if let Some(exit) = self.lock(event) {
                    return Ok(exit);
                }
            }
            heimdall_platform::set_unsaved(self.doc.modified);
            screen.set_mouse(self.code)?;
            // A terminal that can't be drawn on or read from is gone:
            // save, encrypted, and leave, as when it is closed.
            let ready = self.draw(screen).and_then(|()| event::poll(TICK));
            match ready {
                Ok(false) => continue,
                Ok(true) => {}
                Err(_) => return Ok(self.hang_up()),
            }
            let Ok(event) = event::read() else {
                return Ok(self.hang_up());
            };
            match event {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    self.session.touch();
                    self.message = None;
                    let was_cut = std::mem::take(&mut self.cutting);
                    if let Some(exit) = self.key(key, was_cut) {
                        return Ok(exit);
                    }
                }
                Event::Paste(text) => {
                    self.session.touch();
                    self.cutting = false;
                    let text = Zeroizing::new(text);
                    let line = text.chars().take_while(|c| !c.is_control());
                    match self.mode {
                        Mode::Edit => self.paste(&text),
                        Mode::Find => self.query.extend(line),
                        Mode::SaveAs | Mode::ExportPath => self.field.extend(line),
                        _ => {}
                    }
                }
                Event::Mouse(mouse) if self.code && matches!(self.mode, Mode::Edit | Mode::Find) => {
                    self.session.touch();
                    self.mouse(mouse);
                }
                _ => {} // a resize is picked up by the next draw
            }
        }
    }

    fn hang_up(&mut self) -> Exit {
        self.lock(LockEvent::HungUp).unwrap_or(Exit::Locked {
            event: LockEvent::HungUp,
            saved: None,
        })
    }

    /// Saves and leaves. Stays open only if saving failed and the event
    /// isn't one that must lock anyway.
    fn lock(&mut self, event: LockEvent) -> Option<Exit> {
        let saved = self.doc.modified.then(|| self.save());
        if matches!(saved, Some(Err(_))) && !event.must_lock() {
            if let Some(Err(e)) = saved {
                self.say(format!("Not locked: {e}"));
            }
            self.session.touch();
            return None;
        }
        if event == LockEvent::Ending {
            heimdall_platform::allow_end_session();
        }
        Some(Exit::Locked { event, saved })
    }

    fn say(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), Instant::now()));
    }

    /// Writes the text into the vault and saves it, encrypted.
    fn save(&mut self) -> Result<(), String> {
        let now = heimdall_core::content::now();
        let content = &mut self.session.vault.content;
        let id = match &self.target {
            Target::Note(id) if content.note(*id).is_some() => *id,
            _ => {
                let mut note = Note::new(now);
                if let Target::New(title) = &self.target {
                    // Two notes never share a title (DESIGN §10.1).
                    note.title = content.free_title(title);
                }
                let id = note.id;
                content.notes.push(note);
                id
            }
        };
        self.target = Target::Note(id);
        let note = content.note_mut(id).expect("just found or made");
        note.body.zeroize();
        note.body = self.doc.text().to_owned();
        note.cursor = self.doc.cursor_chars() as u64;
        note.modified = now;
        // Code mode is the note's, so the app opens it the same way.
        match (self.code, &note.code) {
            (true, None) => {
                note.code = Some(CodeMode {
                    language: self.language_id.clone(),
                })
            }
            (false, Some(_)) => note.code = None,
            _ => {}
        }
        note.code_off = self.code_off;
        self.session.save()?;
        self.doc.modified = false;
        Ok(())
    }

    /// Turns the layout to what `self.code` says, and finds the language.
    fn apply_code_mode(&mut self) {
        self.doc.set_wrap(!self.code);
        self.left = 0;
        self.cache.version = None;
        self.language = if !self.code {
            None
        } else if self.language_id.is_empty() {
            // Detected like the app does: the title's extension, or a `#!` line.
            let mut probe = Note::new(0);
            probe.title = self.title.clone();
            probe.body = head(self.doc.text()).to_owned();
            code::detect(&probe)
        } else {
            code::by_id(&self.language_id)
        };
    }

    /// The note's own title: what Save as starts from. Empty for a note
    /// named after its first line.
    fn own_title(&self) -> String {
        match &self.target {
            Target::Note(id) => self
                .session
                .vault
                .content
                .note(*id)
                .map(|n| n.title.clone())
                .unwrap_or_default(),
            Target::New(title) => title.clone(),
        }
    }

    /// `tets` → `tets.py`, when the language is known (chosen, or guessed
    /// from the text) and the title has no extension of its own. Only a
    /// suggestion: code mode stays as it is (DESIGN §10.6).
    fn suggested_name(&self) -> Option<String> {
        let mut probe = Note::new(0);
        probe.title = self.own_title();
        probe.body = head(self.doc.text()).to_owned();
        probe.code = self.code.then(|| CodeMode {
            language: self.language_id.clone(),
        });
        code::suggested_title(&probe)
    }

    fn suggest_extension(&mut self) {
        self.suggest_extension_after("");
    }

    /// The suggestion, after `before` ("Saved. ") on the same line.
    fn suggest_extension_after(&mut self, before: &str) {
        let Some(name) = self.suggested_name() else {
            return;
        };
        let language = name
            .rsplit_once('.')
            .and_then(|(_, ext)| code::by_extension(ext))
            .map_or("code", |l| l.name);
        self.say(format!(
            "[ {before}Looks like {language}. ^O to save as {} ]",
            secret::hide(&name)
        ));
        self.suggested = Some(name);
    }

    /// `^O`: the note gets the title in `field` (one no other note has)
    /// and is saved. It stays in the vault, encrypted.
    fn save_as(&mut self) {
        let name = self.field.trim().to_owned();
        if name.is_empty() || name.chars().any(char::is_control) {
            self.say("[ That title can't be used ]");
            return;
        }
        self.mode = Mode::Edit;
        self.field.clear();
        let content = &mut self.session.vault.content;
        let title = match &self.target {
            Target::Note(id) if content.note(*id).is_some() => {
                content.rename_note(*id, &name);
                content.note(*id).map_or(name, |n| n.title.clone())
            }
            _ => {
                let title = content.free_title(&name);
                self.target = Target::New(title.clone());
                title
            }
        };
        self.title = title.clone();
        // A name like `x.py` is a new reason for code mode.
        if code::by_title(&title).is_some() && content.editor.code_from_extension {
            self.code = true;
            self.code_off = false;
            self.language_id.clear();
        }
        self.apply_code_mode();
        match self.save() {
            Ok(()) => self.say(format!("[ Saved as \"{}\" ]", secret::hide(&title))),
            Err(e) => self.say(e),
        }
    }

    /// `^E`: asks where, with the note's file name in the current folder.
    fn start_export(&mut self) {
        if !self.session.vault.content.security.terminal_export {
            self.say("[ Export is off for this vault. Turn it on in Settings > Vault ]");
            return;
        }
        let mut probe = Note::new(0);
        probe.title = self.title.clone();
        probe.body = head(self.doc.text()).to_owned();
        probe.code = self.code.then(|| CodeMode {
            language: self.language_id.clone(),
        });
        let name = plain::file_name_for(&probe);
        self.field = match std::env::current_dir() {
            Ok(dir) => paths::short_path(&dir.join(&name)),
            Err(_) => name,
        };
        self.mode = Mode::ExportPath;
    }

    fn export_path_entered(&mut self) {
        let typed = self.field.trim();
        if typed.is_empty() {
            self.mode = Mode::Edit;
            return;
        }
        let mut path = paths::expand_home(Path::new(typed));
        if path.is_relative() {
            if let Ok(dir) = std::env::current_dir() {
                path = dir.join(path);
            }
        }
        if path.is_dir() {
            self.say("[ That is a folder: add a file name ]");
            return;
        }
        if !path.parent().is_some_and(Path::is_dir) {
            self.say("[ That folder doesn't exist ]");
            return;
        }
        self.mode = if secret::scan(self.doc.text()).secrets.is_empty() {
            Mode::ExportConfirm {
                replace: path.exists(),
                path,
                secrets: Secrets::Keep,
            }
        } else {
            Mode::ExportSecrets(path)
        };
    }

    fn export(&mut self, path: &Path, secrets: Secrets) {
        let mut note = Note::new(0);
        note.title = self.title.clone();
        note.body = self.doc.text().to_owned();
        let text = plain::note_text(&note, secrets, false);
        let shown = paths::short_path(path);
        match heimdall_core::storage::write_atomic(path, text.as_bytes()) {
            Ok(()) => self.say(format!("[ Exported, not encrypted: {shown} ]")),
            Err(e) => self.say(format!("[ Not exported: {} ]", error_message(&e))),
        }
    }

    fn paste(&mut self, text: &str) {
        let trimmed = text.trim();
        let masked =
            (self.mask_pasted && !self.doc.revealed() && secret::looks_like_secret(trimmed, self.code))
                .then(|| secret::wrap(trimmed))
                .flatten()
                .map(Zeroizing::new);
        let result = match &masked {
            Some(wrapped) => self.doc.insert(wrapped),
            None => self.doc.insert(text),
        };
        match (result, masked) {
            (Err(_), _) => self.say("The note is at its 2 MB limit"),
            (Ok(()), Some(_)) => self.say("[ Looked like a secret: masked. ^Z undoes it ]"),
            _ => {}
        }
    }

    fn key(&mut self, key: KeyEvent, was_cut: bool) -> Option<Exit> {
        match self.mode {
            Mode::Edit => self.key_edit(key, was_cut),
            Mode::Find => {
                self.key_find(key);
                None
            }
            Mode::ConfirmQuit => self.key_quit(key),
            Mode::Help => {
                self.mode = Mode::Edit;
                None
            }
            Mode::SaveAs | Mode::ExportPath => {
                self.key_field(key);
                None
            }
            Mode::ExportSecrets(_) | Mode::ExportConfirm { .. } => {
                self.key_export(key);
                None
            }
        }
    }

    /// Typing in Save as and Export.
    fn key_field(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.mode = Mode::Edit,
            KeyCode::Char('c' | 'g') if ctrl => self.mode = Mode::Edit,
            KeyCode::Enter if matches!(self.mode, Mode::SaveAs) => self.save_as(),
            KeyCode::Enter => self.export_path_entered(),
            // Some terminals send ^H for Backspace.
            KeyCode::Backspace => {
                self.field.pop();
            }
            KeyCode::Char('h') if ctrl => {
                self.field.pop();
            }
            KeyCode::Char('u') if ctrl => self.field.clear(),
            KeyCode::Tab if matches!(self.mode, Mode::ExportPath) => {
                let found = paths::complete(&self.field, |_| true);
                match prompt::completion(&self.field, &found) {
                    Completion::Line(text) => self.field = text,
                    Completion::List(found) => {
                        let names: Vec<&str> = found.iter().map(|f| paths::last_part(f)).collect();
                        self.say(format!("[ {} ]", names.join("  ")));
                    }
                    Completion::Nothing => {}
                }
            }
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => self.field.push(c),
            _ => {}
        }
    }

    /// The last two steps of `^E`.
    fn key_export(&mut self, key: KeyEvent) {
        let mode = std::mem::replace(&mut self.mode, Mode::Edit);
        let pressed = match key.code {
            KeyCode::Char(c) => c.to_ascii_lowercase(),
            KeyCode::Enter => '\n',
            _ => '\0',
        };
        match (mode, pressed) {
            (Mode::ExportSecrets(path), 'h' | 'k' | 'r') => {
                let secrets = match pressed {
                    'h' => Secrets::Hide,
                    'k' => Secrets::Keep,
                    _ => Secrets::Reveal,
                };
                self.mode = Mode::ExportConfirm {
                    replace: path.exists(),
                    path,
                    secrets,
                };
            }
            // Enter is yes, except when a file would be replaced.
            (Mode::ExportConfirm { path, secrets, .. }, 'y') => self.export(&path, secrets),
            (
                Mode::ExportConfirm {
                    path,
                    secrets,
                    replace: false,
                },
                '\n',
            ) => self.export(&path, secrets),
            _ => self.say("[ Not exported ]"),
        }
    }

    fn key_edit(&mut self, key: KeyEvent, was_cut: bool) -> Option<Exit> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let page = self.text_rows() as isize;
        match key.code {
            KeyCode::Char(c) if ctrl => match c.to_ascii_lowercase() {
                's' => match self.save() {
                    Ok(()) => {
                        self.say("[ Saved ]");
                        // Once per name, not on every ^S.
                        if self.suggested_name() != self.suggested {
                            self.suggest_extension_after("Saved. ");
                        }
                    }
                    Err(e) => self.say(e),
                },
                'q' if self.doc.modified => self.mode = Mode::ConfirmQuit,
                'q' => return Some(Exit::Quit),
                'f' => self.mode = Mode::Find,
                'g' => self.mode = Mode::Help,
                'r' => {
                    if !self.doc.toggle_reveal() {
                        self.say("[ Put the cursor on a secret to show it ]");
                    }
                }
                't' => {
                    self.code = !self.code;
                    // Off by hand: the title's extension doesn't turn it on again.
                    self.code_off = !self.code;
                    self.doc.modified = true;
                    self.apply_code_mode();
                    let state = if self.code { "on" } else { "off" };
                    self.say(format!("[ Code mode {state} ]"));
                    if self.code {
                        self.suggest_extension();
                    }
                }
                'z' => {
                    if !self.doc.undo() {
                        self.say("[ Nothing to undo ]");
                    }
                }
                'y' => {
                    if !self.doc.redo() {
                        self.say("[ Nothing to redo ]");
                    }
                }
                'k' => {
                    if let Some(line) = self.doc.cut_line() {
                        if !was_cut {
                            self.cut.zeroize();
                        }
                        self.cut.push_str(&line);
                        self.cutting = true;
                    }
                }
                'u' => {
                    let cut = self.cut.clone();
                    if self.doc.insert(&cut).is_err() {
                        self.say("The note is at its 2 MB limit");
                    }
                }
                'l' => return self.lock(LockEvent::Manual),
                'o' => {
                    self.field = self.suggested_name().unwrap_or_else(|| match self.own_title() {
                        t if t.is_empty() => self.title.clone(),
                        t => t,
                    });
                    self.mode = Mode::SaveAs;
                }
                'e' => self.start_export(),
                // Some terminals send ^H for Backspace.
                'h' => self.doc.backspace(),
                'c' => self.say("[ ^Q quits ]"),
                _ => {}
            },
            KeyCode::Left if ctrl => self.doc.word_left(),
            KeyCode::Right if ctrl => self.doc.word_right(),
            KeyCode::Home if ctrl => self.doc.doc_start(),
            KeyCode::End if ctrl => self.doc.doc_end(),
            KeyCode::Left => self.doc.left(),
            KeyCode::Right => self.doc.right(),
            KeyCode::Up => self.doc.vertical(-1),
            KeyCode::Down => self.doc.vertical(1),
            KeyCode::PageUp => self.doc.vertical(-page),
            KeyCode::PageDown => self.doc.vertical(page),
            KeyCode::Home => self.doc.home(),
            KeyCode::End => self.doc.end(),
            KeyCode::Backspace => self.doc.backspace(),
            KeyCode::Delete => self.doc.delete(),
            // Enter ends the editing of a shown secret; it has no line breaks.
            KeyCode::Enter if self.doc.revealed() => self.doc.leave_secret(),
            KeyCode::Enter if self.code => {
                let line = format!("\n{}", self.doc.line_indent());
                self.type_text(&line);
            }
            KeyCode::Enter => self.type_text("\n"),
            KeyCode::Tab => {
                let tab = self.tab.clone();
                self.type_text(&tab);
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::ALT) => {
                let mut buf = [0u8; 4];
                self.type_text(c.encode_utf8(&mut buf));
                buf.zeroize();
            }
            _ => {}
        }
        None
    }

    fn type_text(&mut self, text: &str) {
        if self.doc.insert(text).is_err() {
            self.say("The note is at its 2 MB limit");
        }
    }

    fn key_find(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.mode = Mode::Edit,
            KeyCode::Char('c' | 'g') if ctrl => self.mode = Mode::Edit,
            KeyCode::Enter => {
                if !self.doc.find_next(&self.query) {
                    self.say("[ Not found ]");
                }
            }
            KeyCode::Char('p') if ctrl => {
                if !self.doc.find_prev(&self.query) {
                    self.say("[ Not found ]");
                }
            }
            KeyCode::Backspace => {
                self.query.pop();
            }
            KeyCode::Char('h') if ctrl => {
                self.query.pop();
            }
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => self.query.push(c),
            _ => {}
        }
    }

    fn key_quit(&mut self, key: KeyEvent) -> Option<Exit> {
        match key.code {
            KeyCode::Char('y' | 'Y') | KeyCode::Enter => match self.save() {
                Ok(()) => return Some(Exit::Quit),
                Err(e) => {
                    self.mode = Mode::Edit;
                    self.say(e);
                }
            },
            KeyCode::Char('n' | 'N') => return Some(Exit::Quit),
            _ => self.mode = Mode::Edit,
        }
        None
    }

    fn text_rows(&self) -> usize {
        let rows = terminal::size().map_or(24, |(_, h)| usize::from(h));
        rows.saturating_sub(CHROME_ROWS).max(1)
    }

    // ---------- Mouse (code mode) ----------

    /// The wheel scrolls three lines on the code and three minimap rows on
    /// the minimap; a click or a drag on the minimap jumps there.
    fn mouse(&mut self, mouse: MouseEvent) {
        let g = self.geometry;
        let on_map = g.map_x.is_some_and(|x| usize::from(mouse.column) >= x);
        let step = if on_map { WHEEL_MAP } else { WHEEL_CODE };
        match mouse.kind {
            MouseEventKind::ScrollUp => self.scroll_to(self.top as isize - step),
            MouseEventKind::ScrollDown => self.scroll_to(self.top as isize + step),
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) if on_map => {
                let Some(row) = usize::from(mouse.row).checked_sub(2).filter(|&r| r < g.text_rows) else {
                    return;
                };
                // The clicked part of the file, a little above the middle.
                let line = (g.map_top + row) * MAP_LINES;
                self.scroll_to(line as isize - (g.text_rows / 2) as isize + 2);
            }
            _ => {}
        }
    }

    /// Scrolls so that row `top` is the first on screen. The cursor follows
    /// to stay in view: a terminal has no cursor off screen.
    fn scroll_to(&mut self, top: isize) {
        let rows = self.geometry.text_rows.max(1);
        let max_top = self.doc.rows().len().saturating_sub(rows);
        self.top = (top.max(0) as usize).min(max_top);
        let at = self.doc.cursor_row();
        if at < self.top {
            self.doc.vertical((self.top - at) as isize);
        } else if at >= self.top + rows {
            self.doc.vertical(-((at - (self.top + rows - 1)) as isize));
        }
    }

    // ---------- Drawing ----------

    fn refresh_cache(&mut self) {
        if self.cache.version == Some(self.doc.version()) {
            return;
        }
        self.cache.version = Some(self.doc.version());
        self.cache.spans = code::highlight(self.language, self.doc.text());
        self.cache.map = minimap(&self.doc.line_shapes());
    }

    fn draw(&mut self, screen: &mut Screen) -> io::Result<()> {
        let (w, h) = terminal::size()?;
        let (w, h) = (usize::from(w), usize::from(h));
        let out = &mut screen.out;
        queue!(out, Hide, SetAttribute(Attribute::Reset))?;
        if w < MIN_COLS || h < MIN_ROWS {
            queue!(
                out,
                Clear(ClearType::All),
                MoveTo(0, 0),
                Print(truncate("Window too small", w))
            )?;
            return out.flush();
        }
        if matches!(self.mode, Mode::Help) {
            return self.draw_help(out, w, h);
        }
        // One spare column: writing in the last one scrolls some terminals.
        let cols = w - 1;
        let text_rows = h - CHROME_ROWS;
        // Code mode: a gutter of line numbers on the left and, if there is
        // room, the minimap on the right.
        let (gutter, panel) = if self.code {
            let digits = self.doc.rows().len().to_string().len().max(3);
            (digits + 2, if w >= MAP_MIN_COLS { MAP_COLS + 1 } else { 0 })
        } else {
            (0, 0)
        };
        let text_cols = cols - gutter - panel;
        self.doc.set_width(text_cols);
        if self.code {
            self.refresh_cache();
        }

        let cursor_row = self.doc.cursor_row();
        if cursor_row < self.top {
            self.top = cursor_row;
        } else if cursor_row >= self.top + text_rows {
            self.top = cursor_row + 1 - text_rows;
        }
        self.top = self.top.min(self.doc.rows().len().saturating_sub(1));
        let cursor_col = self.doc.cursor_col();
        if !self.code {
            self.left = 0;
        } else if cursor_col < self.left {
            self.left = cursor_col;
        } else if cursor_col >= self.left + text_cols {
            self.left = cursor_col + 1 - text_cols;
        }

        let mut state = String::new();
        match (&self.target, self.doc.modified) {
            (_, true) => state.push_str("Modified "),
            (Target::New(_), false) => state.push_str("New note "),
            _ => {}
        }
        if self.code {
            let name = self.language.map_or("text", |l| l.name);
            state.push_str(&format!(" Code: {name} "));
        }
        let rule = "-".repeat(w);
        let head = bar(
            &format!(" Heimdall  {}", self.session.name()),
            &self.title,
            &state,
            w,
        );
        line(out, 0, &head)?;
        line(out, 1, &rule)?;

        let map_rows = self.cache.map.len();
        let max_top = self.doc.rows().len().saturating_sub(text_rows);
        // The minimap scrolls too when the file is longer than it.
        let map_top = match (panel, max_top) {
            (0, _) | (_, 0) => 0,
            _ => (self.top * map_rows.saturating_sub(text_rows) + max_top / 2) / max_top,
        };
        let view = self.top / MAP_LINES..=(self.top + text_rows - 1) / MAP_LINES;
        for i in 0..text_rows {
            let y = (2 + i) as u16;
            queue!(out, MoveTo(0, y))?;
            let row = self.doc.rows().get(self.top + i).copied();
            if let Some(row) = row {
                if gutter > 0 {
                    let number = format!("{:>width$}  ", self.top + i + 1, width = gutter - 2);
                    self.dim(out)?;
                    queue!(out, Print(number), SetAttribute(Attribute::Reset))?;
                }
                let cells = self.doc.cells(row);
                self.draw_cells(out, &cells, text_cols)?;
            }
            queue!(
                out,
                SetAttribute(Attribute::Reset),
                Clear(ClearType::UntilNewLine)
            )?;
            if panel > 0 {
                queue!(out, MoveTo((cols - panel) as u16, y))?;
                self.dim(out)?;
                queue!(out, Print("|"))?;
                let r = map_top + i;
                if let Some(braille) = self.cache.map.get(r) {
                    if view.contains(&r) {
                        // The part of the file that is on screen.
                        queue!(out, SetAttribute(Attribute::Reset))?;
                        if self.truecolor {
                            queue!(
                                out,
                                SetForegroundColor(GREY_VIEW),
                                SetBackgroundColor(GREY_VIEW_BG)
                            )?;
                        }
                    }
                    queue!(out, Print(braille))?;
                }
                queue!(out, SetAttribute(Attribute::Reset))?;
            }
        }
        self.geometry = Geometry {
            text_rows,
            map_x: (panel > 0).then_some(cols - panel),
            map_top,
        };
        line(out, h - 3, &rule)?;

        if self
            .message
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= MESSAGE_FOR)
        {
            self.message = None;
        }
        let message = match (&self.message, self.doc.revealed()) {
            (Some((m, _)), _) => m.as_str(),
            (None, true) => "[ Secret shown. ^R to hide ]",
            _ => "",
        };
        // The countdown shows only in the last minute (DESIGN §17.3).
        let countdown = match self.session.idle_left() {
            Some(left) if left <= 60 => format!("Lock in 0:{left:02} "),
            _ => String::new(),
        };
        let main_keys = if self.code { KEYS_CODE } else { KEYS_MAIN };
        let question = |prompt: String, shortcuts: &[&str]| {
            let at = (str_width(&prompt) + 1).min(cols);
            (bar(&prompt, "", "", w), keys(shortcuts), (at, h - 2))
        };
        let (status, keys, cursor) = match &self.mode {
            Mode::Find => {
                let prompt = format!(" Find: {}", self.query);
                let at = str_width(&prompt).min(cols);
                (bar(&prompt, message, &countdown, w), keys(KEYS_FIND), (at, h - 2))
            }
            Mode::ConfirmQuit => {
                let prompt = format!(" Save changes to \"{}\"?", self.title);
                let at = (str_width(&prompt) + 1).min(cols);
                (bar(&prompt, "", &countdown, w), keys(KEYS_QUIT), (at, h - 2))
            }
            Mode::SaveAs | Mode::ExportPath => {
                let (label, shortcuts) = match self.mode {
                    Mode::SaveAs => (" Save as: ", KEYS_SAVE_AS),
                    _ => (" Export to: ", KEYS_EXPORT),
                };
                let prompt = field_line(label, &self.field, cols);
                let at = str_width(&prompt).min(cols);
                (bar(&prompt, message, "", w), keys(shortcuts), (at, h - 2))
            }
            Mode::ExportSecrets(_) => {
                let n = secret::scan(self.doc.text()).secrets.len();
                let plural = if n == 1 { "" } else { "s" };
                question(
                    format!(" This note has {n} secret{plural}. In the file:"),
                    KEYS_SECRETS,
                )
            }
            Mode::ExportConfirm { path, replace, .. } => {
                let shown = paths::short_path(path);
                let prompt = if *replace {
                    format!(" {shown} exists. Replace it with a file that is NOT encrypted? [y/N]")
                } else {
                    format!(" {shown} will NOT be encrypted. Export? [Y/n]")
                };
                question(prompt, KEYS_QUIT)
            }
            _ => {
                let (ln, col) = self.doc.line_col();
                let x = gutter + (cursor_col - self.left).min(text_cols);
                let at = (x, 2 + cursor_row - self.top);
                (
                    bar(&format!(" Ln {ln}, Col {col}"), message, &countdown, w),
                    keys(main_keys),
                    at,
                )
            }
        };
        line(out, h - 2, &status)?;
        line(out, h - 1, truncate(&keys, cols))?;
        queue!(out, MoveTo(cursor.0 as u16, cursor.1 as u16), Show)?;
        out.flush()
    }

    fn dim(&self, out: &mut Stdout) -> io::Result<()> {
        if self.truecolor {
            queue!(out, SetForegroundColor(GREY_DIM))
        } else {
            queue!(out, SetAttribute(Attribute::Dim))
        }
    }

    /// Draws the part of a row between columns `self.left` and
    /// `self.left + width`. A shown secret is underlined; in code mode each
    /// block takes the color of its kind.
    fn draw_cells(&self, out: &mut Stdout, cells: &[Cell], width: usize) -> io::Result<()> {
        let mut col = 0;
        let mut style = (Kind::Plain, false);
        for cell in cells {
            let w = cell.width();
            let (from, to) = (col, col + w);
            col = to;
            if to <= self.left {
                continue;
            }
            if from >= self.left + width {
                break;
            }
            let kind = match (self.code, cell.glyph) {
                (true, Glyph::Char(_)) => code::kind_at(&self.cache.spans, cell.at),
                _ => Kind::Plain,
            };
            if (kind, cell.underline) != style {
                style = (kind, cell.underline);
                queue!(out, SetAttribute(Attribute::Reset))?;
                if cell.underline {
                    queue!(out, SetAttribute(Attribute::Underlined))?;
                }
                self.set_kind(out, kind)?;
            }
            // A wide block cut by either edge leaves spaces.
            if from < self.left || to > self.left + width {
                let shown = to.min(self.left + width) - from.max(self.left);
                queue!(out, Print(" ".repeat(shown)))?;
                continue;
            }
            match cell.glyph {
                Glyph::Char(c) => queue!(out, Print(c))?,
                Glyph::Tab => queue!(out, Print(" ".repeat(TAB_W)))?,
                Glyph::Secret => queue!(out, Print(SECRET))?,
            }
        }
        Ok(())
    }

    fn set_kind(&self, out: &mut Stdout, kind: Kind) -> io::Result<()> {
        if kind == Kind::Plain {
            return Ok(());
        }
        if self.truecolor {
            let c = kind.color();
            return queue!(
                out,
                SetForegroundColor(Color::Rgb {
                    r: c.r(),
                    g: c.g(),
                    b: c.b()
                })
            );
        }
        // No 24-bit color: keywords in bold, comments dimmed.
        match kind {
            Kind::Keyword => queue!(out, SetAttribute(Attribute::Bold)),
            Kind::Comment => queue!(out, SetAttribute(Attribute::Dim)),
            _ => Ok(()),
        }
    }

    fn draw_help(&self, out: &mut Stdout, w: usize, h: usize) -> io::Result<()> {
        let mut lines = vec![
            bar(" Heimdall  help", "", "", w),
            "-".repeat(w),
            String::new(),
            " :: Editor keys".to_owned(),
        ];
        lines.extend(help_lines());
        lines.extend([
            String::new(),
            " :: Secrets".to_owned(),
            "    Text written as {*value*} is a secret. It shows as [secret],".to_owned(),
            "    moves and deletes as one block, and ^R shows the one at the cursor.".to_owned(),
            String::new(),
            " :: Code mode".to_owned(),
            "    Line numbers, colors and a map of the file. The wheel scrolls, faster".to_owned(),
            "    on the map, and a click there jumps. Shift+drag selects text.".to_owned(),
            String::new(),
            " :: Save as and export".to_owned(),
            "    ^O renames the note in the vault, where it stays encrypted.".to_owned(),
            "    ^E writes it to a plain file that is NOT encrypted.".to_owned(),
            " Press any key to go back.".to_owned(),
        ]);
        for row in 0..h {
            line(
                out,
                row,
                truncate(lines.get(row).map_or("", String::as_str), w - 1),
            )?;
        }
        out.flush()
    }
}

/// The minimap: the shape of the file in braille. Each char is 2 x 4 dots;
/// a dot is [`MAP_SCALE`] columns of one line, on where that line has text.
fn minimap(shapes: &[(usize, usize)]) -> Vec<String> {
    const LEFT: [u32; 4] = [0x01, 0x02, 0x04, 0x40];
    const RIGHT: [u32; 4] = [0x08, 0x10, 0x20, 0x80];
    shapes
        .chunks(MAP_LINES)
        .map(|lines| {
            (0..MAP_COLS)
                .map(|c| {
                    let mut bits = 0;
                    for (d, &(indent, len)) in lines.iter().enumerate() {
                        for (half, dots) in [LEFT, RIGHT].iter().enumerate() {
                            let x = (c * 2 + half) * MAP_SCALE;
                            if x + MAP_SCALE > indent && x < len {
                                bits |= dots[d];
                            }
                        }
                    }
                    char::from_u32(0x2800 + bits).unwrap_or(' ')
                })
                .collect()
        })
        .collect()
}

/// The editor keys in two columns, as `heimdall --help` and `^G` show them.
pub fn help_lines() -> Vec<String> {
    let half = HELP.len().div_ceil(2);
    (0..half)
        .map(|i| {
            let cell = |&(key, what): &(&str, &str)| format!("{key}  {what}");
            let left = cell(&HELP[i]);
            let right = HELP.get(i + half).map(cell).unwrap_or_default();
            format!("    {left:<25}{right}").trim_end().to_owned()
        })
        .collect()
}

/// ` Save as: respaldo.py`, its start cut off if it is too long for
/// `cols`, so the end, where the cursor is, stays in view.
fn field_line(label: &str, field: &str, cols: usize) -> String {
    let full = format!("{label}{field}");
    if str_width(&full) < cols {
        return full;
    }
    let room = cols.saturating_sub(str_width(label) + 4);
    let mut tail: Vec<char> = Vec::new();
    let mut width = 0;
    for c in field.chars().rev() {
        width += super::text::char_width(c);
        if width > room {
            break;
        }
        tail.push(c);
    }
    let tail: String = tail.into_iter().rev().collect();
    format!("{label}...{tail}")
}

/// Writes one whole screen row.
fn line(out: &mut Stdout, row: usize, text: &str) -> io::Result<()> {
    queue!(
        out,
        MoveTo(0, row as u16),
        Print(text),
        Clear(ClearType::UntilNewLine)
    )
}

/// ` ^S Save        ^Q Quit        …`: each shortcut in a 15-column cell.
fn keys(items: &[&str]) -> String {
    let all: String = items.iter().map(|k| format!("{k:<15}")).collect();
    format!(" {}", all.trim_end())
}

/// `left`, `center` in the middle of the row and `right` at its end, in
/// `width` columns. What doesn't fit is cut, the center first.
fn bar(left: &str, center: &str, right: &str, width: usize) -> String {
    let width = width.saturating_sub(1);
    let right = truncate(right, width);
    let left = truncate(left, width - str_width(right));
    let free = width - str_width(left) - str_width(right);
    let center = truncate(center, free.saturating_sub(2));
    let start = (width.saturating_sub(str_width(center)) / 2)
        .max(str_width(left) + 1)
        .min(width - str_width(right) - str_width(center));
    let mut out = String::with_capacity(width);
    out.push_str(left);
    out.push_str(&" ".repeat(start.saturating_sub(str_width(left))));
    out.push_str(center);
    let used = str_width(&out);
    out.push_str(&" ".repeat(width.saturating_sub(used + str_width(right))));
    out.push_str(right);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_places_three_parts() {
        let b = bar(" Heimdall  personal", "hola.txt", "Modified ", 41);
        assert_eq!(b, " Heimdall  personal hola.txt   Modified ");
        assert_eq!(str_width(&b), 40);
        let b = bar(" Ln 1, Col 1", "", "", 21);
        assert_eq!(b, " Ln 1, Col 1        ");
        // Too narrow for everything: nothing overflows.
        let b = bar(" a long left side", "a long center", "right ", 16);
        assert!(str_width(&b) <= 15);
    }

    #[test]
    fn shortcut_cells() {
        assert_eq!(keys(&["^S Save", "^Q Quit"]), " ^S Save        ^Q Quit");
    }

    #[test]
    fn help_has_every_key() {
        let text = help_lines().join("\n");
        for (key, what) in HELP {
            assert!(text.contains(&format!("{key}  {what}")));
        }
    }

    #[test]
    fn minimap_draws_the_shape_of_the_file() {
        // Four lines make one row. A full-width line lights every top dot.
        let map = minimap(&[(0, 200), (0, 0), (0, 0), (0, 0), (8, 16)]);
        assert_eq!(map.len(), 2);
        assert_eq!(map[0], "\u{2809}".repeat(MAP_COLS));
        // Columns 8..16 of the fifth line: the second char, both dots.
        let second: Vec<char> = map[1].chars().collect();
        assert_eq!(second[0], '\u{2800}');
        assert_eq!(second[1], '\u{2809}');
        assert_eq!(second[2], '\u{2800}');
    }
}
