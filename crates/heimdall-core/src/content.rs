//! The decrypted vault content: settings, notes (tabs) and trash
//! (DESIGN §5.3, §6, §10). Serialized with `postcard` and padded before
//! encryption so the file size does not reveal how much text there is.

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::crypto::TAG_LEN;
use crate::error::{Error, Result};
use crate::format::SlotId;

/// 2 added [`Content::access`] (v0.3); 3 added code mode per note,
/// [`EditorSettings::code_tip`], the spell check language and the personal
/// dictionary; 4 added the secret clipboard timer, locking with Windows and
/// the paste detector settings (v0.4); 5 added the export password setting
/// and the last backup date (v1.0). Versions 1 to 4 are still read.
pub const CONTENT_VERSION: u16 = 5;
/// Plaintext is padded to a multiple of this (DESIGN §5.3).
pub const PAD_BLOCK: usize = 4096;
/// Notes stay in the trash this long before being purged (DESIGN §10.3).
pub const TRASH_RETENTION_S: i64 = 30 * 24 * 60 * 60;

/// Seconds since the Unix epoch, UTC.
pub type Timestamp = i64;

pub fn now() -> Timestamp {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    Basic,
    Hardened,
    Paranoid,
}

/// Security settings (DESIGN §6, §6.1). Defaults depend on the profile;
/// any difference from them shows the profile as "· Custom".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecuritySettings {
    pub profile: Profile,
    /// `None` = never (only allowed in Basic).
    pub lock_after_s: Option<u32>,
    pub clipboard_clear_s: u32,
    /// For copies that include a secret (DESIGN §11.2).
    pub secret_clear_s: u32,
    /// Only Basic can turn it off (§12).
    pub exclude_from_capture: bool,
    pub lock_on_minimize: bool,
    /// Win+L. Always on in Hardened and Paranoid (§12).
    pub lock_on_windows_lock: bool,
    pub remember_recent: bool,
    /// Ask for the vault password before exporting (DESIGN §10.9). Only
    /// Basic can turn it off.
    pub ask_password_export: bool,
}

impl SecuritySettings {
    pub fn defaults_for(profile: Profile) -> Self {
        match profile {
            Profile::Basic => Self {
                profile,
                lock_after_s: Some(10 * 60),
                clipboard_clear_s: 30,
                secret_clear_s: 10,
                exclude_from_capture: true,
                lock_on_minimize: false,
                lock_on_windows_lock: true,
                remember_recent: true,
                ask_password_export: true,
            },
            Profile::Hardened => Self {
                profile,
                lock_after_s: Some(5 * 60),
                clipboard_clear_s: 20,
                secret_clear_s: 10,
                exclude_from_capture: true,
                lock_on_minimize: false,
                lock_on_windows_lock: true,
                remember_recent: false,
                ask_password_export: true,
            },
            Profile::Paranoid => Self {
                profile,
                lock_after_s: Some(2 * 60),
                clipboard_clear_s: 10,
                secret_clear_s: 10,
                exclude_from_capture: true,
                lock_on_minimize: true,
                lock_on_windows_lock: true,
                remember_recent: false,
                ask_password_export: true,
            },
        }
    }

    pub fn is_custom(&self) -> bool {
        *self != Self::defaults_for(self.profile)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Font {
    JetBrainsMono,
    Consolas,
    Inter,
    SegoeUi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TabInsert {
    Spaces(u8),
    Tab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineEnding {
    Crlf,
    Lf,
}

/// Dictionaries the spell checker uses (DESIGN §10.4). A word is correct
/// if any of them accepts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpellLanguage {
    English,
    Spanish,
    EnglishAndSpanish,
}

/// Editor settings (DESIGN §10.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorSettings {
    pub font: Font,
    pub font_size: u8,
    pub tab_inserts: TabInsert,
    pub line_ending: LineEnding,
    pub word_wrap: bool,
    pub line_numbers: bool,
    pub status_bar: bool,
    pub spell_check: bool,
    pub spell_language: SpellLanguage,
    pub autosave: bool,
    pub reopen_tabs: bool,
    /// Ctrl+ / Ctrl- / Ctrl+0; not a setting, but remembered (§10.4).
    pub zoom_percent: u16,
    /// Show the code mode tip on every new tab (§10.6).
    pub code_tip: bool,
    /// Mask pasted text that looks like a secret (§11.3).
    pub mask_pasted: bool,
    /// Explain the paste detector the next time it masks something.
    pub secret_tip: bool,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            font: Font::JetBrainsMono,
            font_size: 14,
            tab_inserts: TabInsert::Spaces(4),
            line_ending: LineEnding::Crlf,
            word_wrap: true,
            line_numbers: false,
            status_bar: true,
            spell_check: false,
            spell_language: SpellLanguage::EnglishAndSpanish,
            autosave: true,
            reopen_tabs: true,
            zoom_percent: 100,
            code_tip: true,
            mask_pasted: true,
            secret_tip: true,
        }
    }
}

/// A tab in code mode (DESIGN §10.6): line numbers, syntax colors, minimap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeMode {
    /// Language id ("rust", "powershell"...). Empty = detect it from the title.
    pub language: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub id: Uuid,
    /// Empty = use the first line of the body (DESIGN §10.1).
    pub title: String,
    pub body: String,
    pub created: Timestamp,
    pub modified: Timestamp,
    /// Cursor position, in characters (not bytes) from the start of `body`.
    pub cursor: u64,
    /// Set only while the note is in the trash.
    pub deleted_at: Option<Timestamp>,
    /// `Some` = the tab is in code mode (Ctrl+Shift+M).
    pub code: Option<CodeMode>,
    /// Word wrap for this tab (Alt+Z). `None` = the global setting, or off
    /// in code mode.
    pub wrap: Option<bool>,
}

impl Note {
    pub fn new(now: Timestamp) -> Self {
        Self {
            id: Uuid::new_v4(),
            title: String::new(),
            body: String::new(),
            created: now,
            modified: now,
            cursor: 0,
            deleted_at: None,
            code: None,
            wrap: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.title.is_empty() && self.body.trim().is_empty()
    }

    /// The tab title: the explicit title, or the first non-empty line.
    pub fn display_title(&self) -> &str {
        if !self.title.is_empty() {
            return &self.title;
        }
        self.body
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("")
    }
}

impl Drop for Note {
    fn drop(&mut self) {
        self.title.zeroize();
        self.body.zeroize();
    }
}

impl std::fmt::Debug for Note {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Note")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// What Settings → Access shows for a key slot: the USB drive's name and
/// when it was added. Kept here, encrypted, because the header only holds
/// what is needed to open the file (DESIGN §5.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessLabel {
    pub slot: SlotId,
    /// Drive name for a USB key ("KINGSTON"); empty for other slots.
    pub name: String,
    pub added: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Content {
    pub security: SecuritySettings,
    pub editor: EditorSettings,
    /// In tab order.
    pub notes: Vec<Note>,
    pub active_note: Option<Uuid>,
    pub trash: Vec<Note>,
    /// Labels for the key slots. Slots from before v0.3 have none.
    pub access: Vec<AccessLabel>,
    /// Words added with "Add to dictionary"; the spell checker accepts them
    /// in every language (§10.4).
    pub dictionary: Vec<String>,
    /// When "Save a backup copy…" last succeeded (DESIGN §10.9).
    pub last_backup: Option<Timestamp>,
}

impl Drop for Content {
    fn drop(&mut self) {
        for word in &mut self.dictionary {
            word.zeroize();
        }
    }
}

/// Layouts of versions 1 to 4, still read (DESIGN §5.3).
mod legacy {
    use super::*;

    /// A note before version 3: no code mode, no wrap override.
    #[derive(Serialize, Deserialize)]
    pub(super) struct NoteV1 {
        pub id: Uuid,
        pub title: String,
        pub body: String,
        pub created: Timestamp,
        pub modified: Timestamp,
        pub cursor: u64,
        pub deleted_at: Option<Timestamp>,
    }

    impl Drop for NoteV1 {
        fn drop(&mut self) {
            self.title.zeroize();
            self.body.zeroize();
        }
    }

    impl From<NoteV1> for Note {
        fn from(mut n: NoteV1) -> Self {
            Self {
                id: n.id,
                title: std::mem::take(&mut n.title),
                body: std::mem::take(&mut n.body),
                created: n.created,
                modified: n.modified,
                cursor: n.cursor,
                deleted_at: n.deleted_at,
                code: None,
                wrap: None,
            }
        }
    }

    /// Editor settings before version 3: no `spell_language`, no `code_tip`.
    #[derive(Serialize, Deserialize)]
    pub(super) struct EditorSettingsV1 {
        pub font: Font,
        pub font_size: u8,
        pub tab_inserts: TabInsert,
        pub line_ending: LineEnding,
        pub word_wrap: bool,
        pub line_numbers: bool,
        pub status_bar: bool,
        pub spell_check: bool,
        pub autosave: bool,
        pub reopen_tabs: bool,
        pub zoom_percent: u16,
    }

    impl From<EditorSettingsV1> for EditorSettingsV3 {
        fn from(e: EditorSettingsV1) -> Self {
            Self {
                font: e.font,
                font_size: e.font_size,
                tab_inserts: e.tab_inserts,
                line_ending: e.line_ending,
                word_wrap: e.word_wrap,
                line_numbers: e.line_numbers,
                status_bar: e.status_bar,
                spell_check: e.spell_check,
                spell_language: SpellLanguage::EnglishAndSpanish,
                autosave: e.autosave,
                reopen_tabs: e.reopen_tabs,
                zoom_percent: e.zoom_percent,
                code_tip: true,
            }
        }
    }

    /// Security settings before version 4: no secret clipboard timer, no
    /// locking with Windows.
    #[derive(Serialize, Deserialize)]
    pub(super) struct SecuritySettingsV1 {
        pub profile: Profile,
        pub lock_after_s: Option<u32>,
        pub clipboard_clear_s: u32,
        pub exclude_from_capture: bool,
        pub lock_on_minimize: bool,
        pub remember_recent: bool,
    }

    impl From<SecuritySettingsV1> for SecuritySettings {
        /// Capture exclusion is turned on: Basic never offered the choice.
        fn from(s: SecuritySettingsV1) -> Self {
            Self {
                profile: s.profile,
                lock_after_s: s.lock_after_s,
                clipboard_clear_s: s.clipboard_clear_s,
                exclude_from_capture: true,
                lock_on_minimize: s.lock_on_minimize,
                remember_recent: s.remember_recent,
                ..Self::defaults_for(s.profile)
            }
        }
    }

    /// Editor settings of version 3: no paste detector.
    #[derive(Serialize, Deserialize)]
    pub(super) struct EditorSettingsV3 {
        pub font: Font,
        pub font_size: u8,
        pub tab_inserts: TabInsert,
        pub line_ending: LineEnding,
        pub word_wrap: bool,
        pub line_numbers: bool,
        pub status_bar: bool,
        pub spell_check: bool,
        pub spell_language: SpellLanguage,
        pub autosave: bool,
        pub reopen_tabs: bool,
        pub zoom_percent: u16,
        pub code_tip: bool,
    }

    impl From<EditorSettingsV3> for EditorSettings {
        fn from(e: EditorSettingsV3) -> Self {
            Self {
                font: e.font,
                font_size: e.font_size,
                tab_inserts: e.tab_inserts,
                line_ending: e.line_ending,
                word_wrap: e.word_wrap,
                line_numbers: e.line_numbers,
                status_bar: e.status_bar,
                spell_check: e.spell_check,
                spell_language: e.spell_language,
                autosave: e.autosave,
                reopen_tabs: e.reopen_tabs,
                zoom_percent: e.zoom_percent,
                code_tip: e.code_tip,
                ..Self::default()
            }
        }
    }

    /// Version 1 (v0.1, v0.2): no access labels.
    #[derive(Serialize, Deserialize)]
    pub(super) struct ContentV1 {
        pub security: SecuritySettingsV1,
        pub editor: EditorSettingsV1,
        pub notes: Vec<NoteV1>,
        pub active_note: Option<Uuid>,
        pub trash: Vec<NoteV1>,
    }

    /// Version 2 (v0.3).
    #[derive(Serialize, Deserialize)]
    pub(super) struct ContentV2 {
        pub security: SecuritySettingsV1,
        pub editor: EditorSettingsV1,
        pub notes: Vec<NoteV1>,
        pub active_note: Option<Uuid>,
        pub trash: Vec<NoteV1>,
        pub access: Vec<AccessLabel>,
    }

    impl From<ContentV1> for Content {
        fn from(v1: ContentV1) -> Self {
            ContentV2 {
                security: v1.security,
                editor: v1.editor,
                notes: v1.notes,
                active_note: v1.active_note,
                trash: v1.trash,
                access: Vec::new(),
            }
            .into()
        }
    }

    /// Version 3 (code mode, spell check).
    #[derive(Serialize, Deserialize)]
    pub(super) struct ContentV3 {
        pub security: SecuritySettingsV1,
        pub editor: EditorSettingsV3,
        pub notes: Vec<Note>,
        pub active_note: Option<Uuid>,
        pub trash: Vec<Note>,
        pub access: Vec<AccessLabel>,
        pub dictionary: Vec<String>,
    }

    impl From<ContentV2> for Content {
        fn from(v2: ContentV2) -> Self {
            ContentV3 {
                security: v2.security,
                editor: v2.editor.into(),
                notes: v2.notes.into_iter().map(Note::from).collect(),
                active_note: v2.active_note,
                trash: v2.trash.into_iter().map(Note::from).collect(),
                access: v2.access,
                dictionary: Vec::new(),
            }
            .into()
        }
    }

    impl From<ContentV3> for Content {
        fn from(v3: ContentV3) -> Self {
            ContentV4 {
                security: v3.security.into(),
                editor: v3.editor.into(),
                notes: v3.notes,
                active_note: v3.active_note,
                trash: v3.trash,
                access: v3.access,
                dictionary: v3.dictionary,
            }
            .into()
        }
    }

    /// Security settings of version 4: no export password setting.
    #[derive(Serialize, Deserialize)]
    pub(super) struct SecuritySettingsV4 {
        pub profile: Profile,
        pub lock_after_s: Option<u32>,
        pub clipboard_clear_s: u32,
        pub secret_clear_s: u32,
        pub exclude_from_capture: bool,
        pub lock_on_minimize: bool,
        pub lock_on_windows_lock: bool,
        pub remember_recent: bool,
    }

    impl From<SecuritySettingsV1> for SecuritySettingsV4 {
        fn from(s: SecuritySettingsV1) -> Self {
            let s = SecuritySettings::from(s);
            Self {
                profile: s.profile,
                lock_after_s: s.lock_after_s,
                clipboard_clear_s: s.clipboard_clear_s,
                secret_clear_s: s.secret_clear_s,
                exclude_from_capture: s.exclude_from_capture,
                lock_on_minimize: s.lock_on_minimize,
                lock_on_windows_lock: s.lock_on_windows_lock,
                remember_recent: s.remember_recent,
            }
        }
    }

    impl From<SecuritySettingsV4> for SecuritySettings {
        fn from(s: SecuritySettingsV4) -> Self {
            Self {
                profile: s.profile,
                lock_after_s: s.lock_after_s,
                clipboard_clear_s: s.clipboard_clear_s,
                secret_clear_s: s.secret_clear_s,
                exclude_from_capture: s.exclude_from_capture,
                lock_on_minimize: s.lock_on_minimize,
                lock_on_windows_lock: s.lock_on_windows_lock,
                remember_recent: s.remember_recent,
                ask_password_export: true,
            }
        }
    }

    /// Version 4 (v0.4, v0.5): the layout frozen in DESIGN §5.6.
    #[derive(Serialize, Deserialize)]
    pub(super) struct ContentV4 {
        pub security: SecuritySettingsV4,
        pub editor: EditorSettings,
        pub notes: Vec<Note>,
        pub active_note: Option<Uuid>,
        pub trash: Vec<Note>,
        pub access: Vec<AccessLabel>,
        pub dictionary: Vec<String>,
    }

    impl From<ContentV4> for Content {
        fn from(v4: ContentV4) -> Self {
            Self {
                security: v4.security.into(),
                editor: v4.editor,
                notes: v4.notes,
                active_note: v4.active_note,
                trash: v4.trash,
                access: v4.access,
                dictionary: v4.dictionary,
                last_backup: None,
            }
        }
    }
}

/// `wanted`, or the first of `wanted (2)`, `wanted (3)`… that `taken`
/// doesn't reject. Like Windows, the number goes before the extension, so
/// `main.rs` becomes `main (2).rs` and keeps its language (DESIGN §10.1).
pub fn unique_title(wanted: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(wanted) {
        return wanted.to_owned();
    }
    // An extension: a short last part after a dot, with no spaces.
    let (stem, ext) = match wanted.rfind('.') {
        Some(dot)
            if dot > 0
                && (1..=8).contains(&(wanted.len() - dot - 1))
                && wanted[dot + 1..].chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            wanted.split_at(dot)
        }
        _ => (wanted, ""),
    };
    (2..)
        .map(|n| format!("{stem} ({n}){ext}"))
        .find(|candidate| !taken(candidate))
        .expect("some number is free")
}

impl Content {
    /// Whether an open note other than `except` shows `title` on its tab.
    fn title_taken(&self, title: &str, except: Option<Uuid>) -> bool {
        self.notes
            .iter()
            .any(|n| Some(n.id) != except && n.display_title() == title)
    }

    /// Renames a note. Two notes never share a title (DESIGN §10.1): if
    /// another one has it, this one gets ` (2)`, ` (3)`… Returns false if
    /// the note doesn't exist or already has that title.
    pub fn rename_note(&mut self, id: Uuid, wanted: &str) -> bool {
        let wanted = wanted.trim();
        let title = match wanted {
            "" => String::new(),
            _ => unique_title(wanted, |t| self.title_taken(t, Some(id))),
        };
        match self.note_mut(id) {
            Some(note) if note.title != title => {
                note.title.zeroize();
                note.title = title;
                true
            }
            _ => false,
        }
    }

    /// A title for a new note that no open note has.
    pub fn free_title(&self, wanted: &str) -> String {
        unique_title(wanted.trim(), |t| self.title_taken(t, None))
    }

    /// Gives every note with a title of its own a different one, for vaults
    /// from before titles were unique: of the notes that share a title, the
    /// one edited last keeps it and the others get ` (2)`, ` (3)`… Notes
    /// named after their first line are left alone. Returns how many were
    /// renamed.
    pub fn dedupe_titles(&mut self) -> usize {
        let mut order: Vec<usize> = (0..self.notes.len())
            .filter(|&i| !self.notes[i].title.is_empty())
            .collect();
        // Newest first, so it is the one that keeps the name.
        order.sort_by_key(|&i| std::cmp::Reverse(self.notes[i].modified));
        let mut seen: Vec<Zeroizing<String>> = Vec::new();
        let mut renamed = 0;
        for i in order {
            let current = Zeroizing::new(self.notes[i].title.clone());
            if seen.iter().any(|t| **t == *current) {
                let free = unique_title(&current, |t| {
                    seen.iter().any(|s| **s == t) || self.notes.iter().any(|n| n.title == t)
                });
                self.notes[i].title.zeroize();
                self.notes[i].title = free;
                renamed += 1;
            }
            seen.push(Zeroizing::new(self.notes[i].title.clone()));
        }
        renamed
    }

    /// A fresh vault: one empty tab (DESIGN §9, step 1).
    pub fn new(profile: Profile, now: Timestamp) -> Self {
        let note = Note::new(now);
        Self {
            security: SecuritySettings::defaults_for(profile),
            editor: EditorSettings::default(),
            active_note: Some(note.id),
            notes: vec![note],
            trash: Vec::new(),
            access: Vec::new(),
            dictionary: Vec::new(),
            last_backup: None,
        }
    }

    /// "Add to dictionary": the spell checker accepts `word` from now on.
    /// Returns false if it was already there.
    pub fn add_to_dictionary(&mut self, word: &str) -> bool {
        let word = word.trim();
        if word.is_empty() || self.dictionary.iter().any(|w| w == word) {
            return false;
        }
        self.dictionary.push(word.to_owned());
        true
    }

    pub fn label(&self, slot: SlotId) -> Option<&AccessLabel> {
        self.access.iter().find(|l| l.slot == slot)
    }

    pub fn set_label(&mut self, slot: SlotId, name: &str, added: Timestamp) {
        self.access.retain(|l| l.slot != slot);
        self.access.push(AccessLabel {
            slot,
            name: name.to_owned(),
            added,
        });
    }

    /// Drops the labels of slots that no longer exist.
    pub fn retain_labels(&mut self, live: &[SlotId]) {
        self.access.retain(|l| live.contains(&l.slot));
    }

    pub fn note(&self, id: Uuid) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    pub fn note_mut(&mut self, id: Uuid) -> Option<&mut Note> {
        self.notes.iter_mut().find(|n| n.id == id)
    }

    /// Ctrl+T: a new empty note at the end, made active.
    pub fn new_note(&mut self, now: Timestamp) -> Uuid {
        let note = Note::new(now);
        let id = note.id;
        self.notes.push(note);
        self.active_note = Some(id);
        id
    }

    /// The content of an exported vault (DESIGN §10.9): copies of the notes
    /// in `ids`, in tab order, with the defaults of a new Basic vault. No
    /// trash, dictionary, access labels or settings of this vault.
    pub fn export(&self, ids: &[Uuid], now: Timestamp) -> Content {
        let mut out = Content::new(Profile::Basic, now);
        out.notes = self
            .notes
            .iter()
            .filter(|n| ids.contains(&n.id))
            .cloned()
            .collect();
        out.active_note = out.notes.first().map(|n| n.id);
        out
    }

    /// Adds `notes` as new tabs at the end (import, DESIGN §10.9): each gets
    /// a new id and leaves the trash, and a title that is already taken
    /// gets " (2)", " (3)"… before its extension ([`unique_title`]). The
    /// first one becomes active. Returns the ids.
    pub fn import_notes(&mut self, notes: impl IntoIterator<Item = Note>) -> Vec<Uuid> {
        let mut taken: Vec<Zeroizing<String>> = self
            .notes
            .iter()
            .map(|n| Zeroizing::new(n.display_title().to_owned()))
            .collect();
        let mut ids = Vec::new();
        for mut note in notes {
            note.id = Uuid::new_v4();
            note.deleted_at = None;
            let title = Zeroizing::new(note.display_title().to_owned());
            if !title.is_empty() && taken.iter().any(|t| **t == *title) {
                let free = unique_title(&title, |c| taken.iter().any(|t| **t == c));
                note.title.zeroize();
                note.title = free;
            }
            taken.push(Zeroizing::new(note.display_title().to_owned()));
            ids.push(note.id);
            self.notes.push(note);
        }
        if let Some(&first) = ids.first() {
            self.active_note = Some(first);
        }
        ids
    }

    /// Ctrl+W: moves the note to the trash, or drops it if it was empty
    /// (DESIGN §10.3). The neighbouring tab becomes active.
    pub fn close_note(&mut self, id: Uuid, now: Timestamp) -> bool {
        let Some(pos) = self.notes.iter().position(|n| n.id == id) else {
            return false;
        };
        let mut note = self.notes.remove(pos);
        if !note.is_empty() {
            note.deleted_at = Some(now);
            self.trash.push(note);
        }
        if self.active_note == Some(id) {
            let next = pos.min(self.notes.len().saturating_sub(1));
            self.active_note = self.notes.get(next).map(|n| n.id);
        }
        true
    }

    /// Restores a note from the trash as the last tab and activates it.
    pub fn restore_note(&mut self, id: Uuid) -> bool {
        let Some(pos) = self.trash.iter().position(|n| n.id == id) else {
            return false;
        };
        let mut note = self.trash.remove(pos);
        note.deleted_at = None;
        if !note.title.is_empty() {
            let free = self.free_title(&note.title);
            if free != note.title {
                note.title.zeroize();
                note.title = free;
            }
        }
        self.notes.push(note);
        self.active_note = Some(id);
        true
    }

    /// Ctrl+Shift+T: restores the most recently closed note, if any.
    pub fn reopen_last_closed(&mut self) -> Option<Uuid> {
        let id = self.trash.iter().max_by_key(|n| n.deleted_at)?.id;
        self.restore_note(id).then_some(id)
    }

    pub fn delete_from_trash(&mut self, id: Uuid) -> bool {
        let before = self.trash.len();
        self.trash.retain(|n| n.id != id);
        self.trash.len() != before
    }

    pub fn empty_trash(&mut self) -> usize {
        let n = self.trash.len();
        self.trash.clear();
        n
    }

    /// Permanently deletes notes that have been in the trash for 30 days.
    pub fn purge_expired_trash(&mut self, now: Timestamp) -> usize {
        let before = self.trash.len();
        self.trash.retain(|n| {
            n.deleted_at
                .is_none_or(|t| now.saturating_sub(t) < TRASH_RETENTION_S)
        });
        before - self.trash.len()
    }

    /// Plaintext ready for encryption: `len (u32) || postcard(version) ||
    /// postcard(content) || zero padding` to a multiple of 4 KiB. Capacity
    /// leaves room for the AEAD tag so encryption never reallocates.
    pub(crate) fn to_padded_plaintext(&self) -> Result<Zeroizing<Vec<u8>>> {
        let mut body =
            Zeroizing::new(postcard::to_allocvec(&CONTENT_VERSION).map_err(|_| Error::BadContent)?);
        let inner = Zeroizing::new(postcard::to_allocvec(self).map_err(|_| Error::BadContent)?);
        body.extend_from_slice(&inner);

        let len = u32::try_from(body.len()).map_err(|_| Error::BadContent)?;
        let padded_len = (4 + body.len()).div_ceil(PAD_BLOCK) * PAD_BLOCK;
        let mut out = Zeroizing::new(Vec::with_capacity(padded_len + TAG_LEN));
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&body);
        out.resize(padded_len, 0);
        Ok(out)
    }

    pub(crate) fn from_padded_plaintext(plain: &[u8]) -> Result<Self> {
        if plain.len() < 4 || !plain.len().is_multiple_of(PAD_BLOCK) {
            return Err(Error::BadContent);
        }
        let len = u32::from_le_bytes(plain[..4].try_into().expect("4 bytes")) as usize;
        let body = plain.get(4..4 + len).ok_or(Error::BadContent)?;
        if plain[4 + len..].iter().any(|&b| b != 0) {
            return Err(Error::BadContent);
        }
        let (version, rest) = postcard::take_from_bytes::<u16>(body).map_err(|_| Error::BadContent)?;
        let (content, rest) = match version {
            1 => postcard::take_from_bytes::<legacy::ContentV1>(rest).map(|(c, r)| (Content::from(c), r)),
            2 => postcard::take_from_bytes::<legacy::ContentV2>(rest).map(|(c, r)| (Content::from(c), r)),
            3 => postcard::take_from_bytes::<legacy::ContentV3>(rest).map(|(c, r)| (Content::from(c), r)),
            4 => postcard::take_from_bytes::<legacy::ContentV4>(rest).map(|(c, r)| (Content::from(c), r)),
            CONTENT_VERSION => postcard::take_from_bytes::<Content>(rest),
            other => return Err(Error::UnsupportedContentVersion(other)),
        }
        .map_err(|_| Error::BadContent)?;
        if !rest.is_empty() {
            return Err(Error::BadContent);
        }
        Ok(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Content {
        let mut c = Content::new(Profile::Hardened, 1_000);
        let id = c.active_note.unwrap();
        let n = c.note_mut(id).unwrap();
        n.body = "Shopping\nmilk, eggs".into();
        n.cursor = 8;
        c.new_note(2_000);
        c
    }

    /// `sample()` with code mode in use (version 3).
    fn sample_v3() -> Content {
        let mut c = sample();
        c.notes[1].body = "fn main() {}".into();
        c.notes[1].code = Some(CodeMode {
            language: "rust".into(),
        });
        c.notes[0].wrap = Some(false);
        c.editor.code_tip = false;
        c.editor.spell_language = SpellLanguage::Spanish;
        c.add_to_dictionary("Heimdall");
        c
    }

    fn padded(version: u16, inner: &[u8]) -> Vec<u8> {
        let mut body = postcard::to_allocvec(&version).unwrap();
        body.extend_from_slice(inner);
        let mut plain = (body.len() as u32).to_le_bytes().to_vec();
        plain.extend_from_slice(&body);
        plain.resize(PAD_BLOCK, 0);
        plain
    }

    fn old_note(n: &Note) -> legacy::NoteV1 {
        legacy::NoteV1 {
            id: n.id,
            title: n.title.clone(),
            body: n.body.clone(),
            created: n.created,
            modified: n.modified,
            cursor: n.cursor,
            deleted_at: n.deleted_at,
        }
    }

    fn old_security(s: &SecuritySettings) -> legacy::SecuritySettingsV1 {
        legacy::SecuritySettingsV1 {
            profile: s.profile,
            lock_after_s: s.lock_after_s,
            clipboard_clear_s: s.clipboard_clear_s,
            exclude_from_capture: s.exclude_from_capture,
            lock_on_minimize: s.lock_on_minimize,
            remember_recent: s.remember_recent,
        }
    }

    fn old_editor(e: &EditorSettings) -> legacy::EditorSettingsV1 {
        legacy::EditorSettingsV1 {
            font: e.font,
            font_size: e.font_size,
            tab_inserts: e.tab_inserts,
            line_ending: e.line_ending,
            word_wrap: e.word_wrap,
            line_numbers: e.line_numbers,
            status_bar: e.status_bar,
            spell_check: e.spell_check,
            autosave: e.autosave,
            reopen_tabs: e.reopen_tabs,
            zoom_percent: e.zoom_percent,
        }
    }

    #[test]
    fn padded_roundtrip() {
        let c = sample_v3();
        let plain = c.to_padded_plaintext().unwrap();
        assert_eq!(plain.len() % PAD_BLOCK, 0);
        assert!(plain.capacity() >= plain.len() + TAG_LEN);
        assert_eq!(Content::from_padded_plaintext(&plain).unwrap(), c);
    }

    #[test]
    fn padding_hides_small_size_differences() {
        let a = Content::new(Profile::Basic, 0);
        let mut b = a.clone();
        b.notes[0].body = "x".repeat(1000);
        assert_eq!(
            a.to_padded_plaintext().unwrap().len(),
            b.to_padded_plaintext().unwrap().len()
        );
    }

    #[test]
    fn rejects_bad_padding_and_versions() {
        let c = sample();
        let mut plain = c.to_padded_plaintext().unwrap().to_vec();
        let last = plain.len() - 1;
        plain[last] = 1;
        assert!(matches!(
            Content::from_padded_plaintext(&plain),
            Err(Error::BadContent)
        ));

        let mut plain = c.to_padded_plaintext().unwrap().to_vec();
        plain[4] = 6; // version varint
        assert!(matches!(
            Content::from_padded_plaintext(&plain),
            Err(Error::UnsupportedContentVersion(6))
        ));

        assert!(Content::from_padded_plaintext(&[0; 10]).is_err());
    }

    /// A vault saved by v0.1/v0.2 must keep opening.
    #[test]
    fn reads_version_1() {
        let c = sample();
        let v1 = legacy::ContentV1 {
            security: old_security(&c.security),
            editor: old_editor(&c.editor),
            notes: c.notes.iter().map(old_note).collect(),
            active_note: c.active_note,
            trash: c.trash.iter().map(old_note).collect(),
        };
        let plain = padded(1, &postcard::to_allocvec(&v1).unwrap());
        assert_eq!(Content::from_padded_plaintext(&plain).unwrap(), c);
    }

    /// A vault saved by v0.3 opens with every tab in plain text and the
    /// code mode tip on; saved again, it becomes version 3.
    #[test]
    fn reads_version_2() {
        let mut c = sample();
        c.set_label(SlotId([7; 8]), "KINGSTON", 5);
        c.close_note(c.notes[0].id, 9);
        let v2 = legacy::ContentV2 {
            security: old_security(&c.security),
            editor: old_editor(&c.editor),
            notes: c.notes.iter().map(old_note).collect(),
            active_note: c.active_note,
            trash: c.trash.iter().map(old_note).collect(),
            access: c.access.clone(),
        };
        let plain = padded(2, &postcard::to_allocvec(&v2).unwrap());
        let read = Content::from_padded_plaintext(&plain).unwrap();
        assert_eq!(read, c);
        assert!(read.editor.code_tip);
        assert_eq!(read.editor.spell_language, SpellLanguage::EnglishAndSpanish);
        assert!(read.dictionary.is_empty());
        assert!(read
            .notes
            .iter()
            .chain(&read.trash)
            .all(|n| n.code.is_none() && n.wrap.is_none()));

        let again = read.to_padded_plaintext().unwrap();
        assert_eq!(
            postcard::take_from_bytes::<u16>(&again[4..]).unwrap().0,
            CONTENT_VERSION
        );
        assert_eq!(Content::from_padded_plaintext(&again).unwrap(), c);
    }

    /// A vault saved by v0.3.x with code mode opens with capture exclusion
    /// on (Basic never offered it) and the v0.4 defaults.
    #[test]
    fn reads_version_3() {
        let mut c = sample_v3();
        c.security = SecuritySettings::defaults_for(Profile::Basic);
        let mut old = old_security(&c.security);
        old.exclude_from_capture = false;
        old.clipboard_clear_s = 60;
        let e = &c.editor;
        let v3 = legacy::ContentV3 {
            security: old,
            editor: legacy::EditorSettingsV3 {
                font: e.font,
                font_size: e.font_size,
                tab_inserts: e.tab_inserts,
                line_ending: e.line_ending,
                word_wrap: e.word_wrap,
                line_numbers: e.line_numbers,
                status_bar: e.status_bar,
                spell_check: e.spell_check,
                spell_language: e.spell_language,
                autosave: e.autosave,
                reopen_tabs: e.reopen_tabs,
                zoom_percent: e.zoom_percent,
                code_tip: e.code_tip,
            },
            notes: c.notes.clone(),
            active_note: c.active_note,
            trash: c.trash.clone(),
            access: c.access.clone(),
            dictionary: c.dictionary.clone(),
        };
        let plain = padded(3, &postcard::to_allocvec(&v3).unwrap());
        let read = Content::from_padded_plaintext(&plain).unwrap();
        c.security.clipboard_clear_s = 60;
        assert_eq!(read, c);
        assert!(read.security.exclude_from_capture);
        assert!(read.security.lock_on_windows_lock);
        assert_eq!(read.security.secret_clear_s, 10);
        assert!(read.editor.mask_pasted && read.editor.secret_tip);
        assert!(!read.editor.code_tip);
    }

    /// A vault saved by v0.4/v0.5 (the layout frozen in DESIGN §5.6) opens
    /// with the export password on and no backup date.
    #[test]
    fn reads_version_4() {
        let mut c = sample_v3();
        c.security.lock_on_minimize = true;
        let s = &c.security;
        let v4 = legacy::ContentV4 {
            security: legacy::SecuritySettingsV4 {
                profile: s.profile,
                lock_after_s: s.lock_after_s,
                clipboard_clear_s: s.clipboard_clear_s,
                secret_clear_s: s.secret_clear_s,
                exclude_from_capture: s.exclude_from_capture,
                lock_on_minimize: s.lock_on_minimize,
                lock_on_windows_lock: s.lock_on_windows_lock,
                remember_recent: s.remember_recent,
            },
            editor: c.editor.clone(),
            notes: c.notes.clone(),
            active_note: c.active_note,
            trash: c.trash.clone(),
            access: c.access.clone(),
            dictionary: c.dictionary.clone(),
        };
        let plain = padded(4, &postcard::to_allocvec(&v4).unwrap());
        let read = Content::from_padded_plaintext(&plain).unwrap();
        assert_eq!(read, c);
        assert!(read.security.ask_password_export);
        assert_eq!(read.last_backup, None);
    }

    #[test]
    fn version_5_fields_roundtrip() {
        let mut c = sample();
        c.security = SecuritySettings::defaults_for(Profile::Basic);
        c.security.ask_password_export = false;
        c.last_backup = Some(1_234);
        assert!(c.security.is_custom());
        let plain = c.to_padded_plaintext().unwrap();
        assert_eq!(Content::from_padded_plaintext(&plain).unwrap(), c);
    }

    #[test]
    fn export_keeps_only_the_chosen_notes() {
        let mut c = sample_v3();
        let a = c.notes[0].id;
        let b = c.notes[1].id;
        c.new_note(3_000);
        c.notes[2].body = "not exported".into();
        c.close_note(c.notes[2].id, 4_000);
        c.set_label(SlotId([7; 8]), "KINGSTON", 5);
        c.security = SecuritySettings::defaults_for(Profile::Paranoid);
        c.editor.font_size = 18;

        let out = c.export(&[b, a], 9_000);
        // Tab order of the source, not the order of `ids`.
        assert_eq!(out.notes.iter().map(|n| n.id).collect::<Vec<_>>(), [a, b]);
        assert_eq!(out.notes[1].code, c.notes[1].code);
        assert_eq!(out.notes[0].wrap, c.notes[0].wrap);
        assert_eq!(out.active_note, Some(a));
        assert!(out.trash.is_empty() && out.access.is_empty() && out.dictionary.is_empty());
        assert_eq!(out.security, SecuritySettings::defaults_for(Profile::Basic));
        assert_eq!(out.editor, EditorSettings::default());
        assert_eq!(out.last_backup, None);
    }

    #[test]
    fn import_renames_taken_titles() {
        let mut c = Content::new(Profile::Basic, 0);
        c.notes[0].title = "Servers".into();
        let first_line = c.new_note(0);
        c.note_mut(first_line).unwrap().body = "Wi-Fi
password"
            .into();

        let mut a = Note::new(1);
        a.title = "Servers".into();
        let mut b = Note::new(1);
        b.title = "Servers".into();
        let mut d = Note::new(1);
        d.body = "Wi-Fi
other"
            .into();
        d.deleted_at = Some(5);
        let e = Note::new(1); // empty: no title to clash
        let old_ids = [a.id, b.id, d.id, e.id];

        let ids = c.import_notes([a, b, d, e]);
        assert_eq!(ids.len(), 4);
        assert!(ids.iter().all(|id| !old_ids.contains(id)));
        let titles: Vec<&str> = c.notes[2..].iter().map(|n| n.display_title()).collect();
        assert_eq!(titles, ["Servers (2)", "Servers (3)", "Wi-Fi (2)", ""]);
        assert!(c.notes.iter().all(|n| n.deleted_at.is_none()));
        assert_eq!(c.active_note, Some(ids[0]));
        assert_eq!(
            c.notes[4].body,
            "Wi-Fi
other"
        );
    }

    #[test]
    fn personal_dictionary() {
        let mut c = sample();
        assert!(c.add_to_dictionary(" Heimdall "));
        assert!(!c.add_to_dictionary("Heimdall"));
        assert!(!c.add_to_dictionary("  "));
        assert!(c.add_to_dictionary("Bifrost"));
        assert_eq!(c.dictionary, ["Heimdall", "Bifrost"]);
        let plain = c.to_padded_plaintext().unwrap();
        assert_eq!(Content::from_padded_plaintext(&plain).unwrap(), c);
    }

    #[test]
    fn access_labels() {
        let mut c = sample();
        let (a, b) = (SlotId([1; 8]), SlotId([2; 8]));
        c.set_label(a, "KINGSTON", 5);
        c.set_label(b, "", 6);
        c.set_label(a, "KINGSTON (old)", 7);
        assert_eq!(c.access.len(), 2);
        assert_eq!(c.label(a).unwrap().name, "KINGSTON (old)");
        c.retain_labels(&[b]);
        assert!(c.label(a).is_none());
        assert_eq!(c.label(b).unwrap().added, 6);
        let plain = c.to_padded_plaintext().unwrap();
        assert_eq!(Content::from_padded_plaintext(&plain).unwrap(), c);
    }

    #[test]
    fn profile_defaults_and_custom() {
        let mut s = SecuritySettings::defaults_for(Profile::Hardened);
        assert!(!s.is_custom());
        assert!(!s.remember_recent);
        s.lock_after_s = Some(15 * 60);
        assert!(s.is_custom());
        assert!(SecuritySettings::defaults_for(Profile::Basic).remember_recent);
        assert!(SecuritySettings::defaults_for(Profile::Paranoid).lock_on_minimize);
    }

    #[test]
    fn display_title_falls_back_to_first_line() {
        let mut n = Note::new(0);
        n.body = "\n  First line \nsecond".into();
        assert_eq!(n.display_title(), "First line");
        n.title = "Named".into();
        assert_eq!(n.display_title(), "Named");
    }

    #[test]
    fn close_restore_and_trash() {
        let mut c = sample();
        let first = c.notes[0].id;
        let second = c.notes[1].id; // empty

        // Empty note is dropped, not trashed.
        assert!(c.close_note(second, 10));
        assert!(c.trash.is_empty());
        assert_eq!(c.active_note, Some(first));

        // Non-empty note goes to the trash.
        assert!(c.close_note(first, 20));
        assert_eq!(c.trash.len(), 1);
        assert!(c.notes.is_empty());
        assert_eq!(c.active_note, None);

        assert!(c.restore_note(first));
        assert_eq!(c.active_note, Some(first));
        assert_eq!(c.notes[0].deleted_at, None);

        c.close_note(first, 100);
        assert_eq!(c.purge_expired_trash(100 + TRASH_RETENTION_S - 1), 0);
        assert_eq!(c.purge_expired_trash(100 + TRASH_RETENTION_S), 1);
        assert!(c.trash.is_empty());
    }

    #[test]
    fn reopen_last_closed_restores_the_newest() {
        let mut c = Content::new(Profile::Basic, 0);
        let a = c.notes[0].id;
        let b = c.new_note(0);
        c.notes[1].code = Some(CodeMode {
            language: String::new(),
        });
        for n in &mut c.notes {
            n.body = "x".into();
        }
        c.close_note(a, 10);
        c.close_note(b, 20);
        assert_eq!(c.reopen_last_closed(), Some(b));
        assert_eq!(c.active_note, Some(b));
        assert!(c.note(b).unwrap().code.is_some());
        assert_eq!(c.reopen_last_closed(), Some(a));
        assert_eq!(c.reopen_last_closed(), None);
    }

    #[test]
    fn closing_middle_tab_activates_neighbour() {
        let mut c = Content::new(Profile::Basic, 0);
        let a = c.notes[0].id;
        let b = c.new_note(0);
        let d = c.new_note(0);
        for n in &mut c.notes {
            n.body = "x".into();
        }
        c.active_note = Some(b);
        c.close_note(b, 1);
        assert_eq!(c.active_note, Some(d));
        assert_eq!(c.notes.iter().map(|n| n.id).collect::<Vec<_>>(), vec![a, d]);
    }

    #[test]
    fn titles_get_a_number_like_windows() {
        let taken = |t: &str| ["hola.txt", "hola (2).txt", "notas", ".env", "v1.2 final"].contains(&t);
        assert_eq!(unique_title("libre", taken), "libre");
        assert_eq!(unique_title("hola.txt", taken), "hola (3).txt");
        assert_eq!(unique_title("notas", taken), "notas (2)");
        // Not extensions: a leading dot, or a last part with spaces.
        assert_eq!(unique_title(".env", taken), ".env (2)");
        assert_eq!(unique_title("v1.2 final", taken), "v1.2 final (2)");
    }

    #[test]
    fn renaming_never_makes_a_duplicate() {
        let mut c = Content::new(Profile::Basic, 1);
        let a = c.notes[0].id;
        let b = c.new_note(2);
        assert!(c.rename_note(a, "sync.ps1"));
        assert!(c.rename_note(b, " sync.ps1 "));
        assert_eq!(c.note(b).unwrap().title, "sync (2).ps1");
        // Its own title is not a clash, and an empty title is always allowed.
        assert!(!c.rename_note(b, "sync (2).ps1"));
        assert!(c.rename_note(b, ""));
        assert_eq!(c.free_title("sync.ps1"), "sync (2).ps1");
    }

    #[test]
    fn old_duplicates_are_renamed_newest_keeps_the_name() {
        let mut c = Content::new(Profile::Basic, 1);
        c.notes.clear();
        for (title, modified) in [
            ("hola.txt", 10),
            ("hola.txt", 30),
            ("otra", 5),
            ("hola.txt", 20),
            ("", 1),
            ("", 2),
        ] {
            let mut n = Note::new(modified);
            n.title = title.into();
            c.notes.push(n);
        }
        assert_eq!(c.dedupe_titles(), 2);
        let titles: Vec<&str> = c.notes.iter().map(|n| n.title.as_str()).collect();
        assert_eq!(
            titles,
            ["hola (3).txt", "hola.txt", "otra", "hola (2).txt", "", ""]
        );
        assert_eq!(c.dedupe_titles(), 0);
    }

    #[test]
    fn restoring_from_the_trash_keeps_titles_unique() {
        let mut c = Content::new(Profile::Basic, 1);
        let a = c.notes[0].id;
        c.rename_note(a, "lista");
        c.note_mut(a).unwrap().body = "x".into();
        c.close_note(a, 2);
        let b = c.new_note(3);
        c.rename_note(b, "lista");
        assert!(c.restore_note(a));
        assert_eq!(c.note(a).unwrap().title, "lista (2)");
    }
}
