//! The decrypted vault content: settings, notes (tabs) and trash
//! (DESIGN §5.3, §6, §10). Serialized with `postcard` and padded before
//! encryption so the file size does not reveal how much text there is.

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::crypto::TAG_LEN;
use crate::error::{Error, Result};

pub const CONTENT_VERSION: u16 = 1;
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
    pub exclude_from_capture: bool,
    pub lock_on_minimize: bool,
    pub remember_recent: bool,
}

impl SecuritySettings {
    pub fn defaults_for(profile: Profile) -> Self {
        match profile {
            Profile::Basic => Self {
                profile,
                lock_after_s: Some(10 * 60),
                clipboard_clear_s: 30,
                exclude_from_capture: false,
                lock_on_minimize: false,
                remember_recent: true,
            },
            Profile::Hardened => Self {
                profile,
                lock_after_s: Some(5 * 60),
                clipboard_clear_s: 20,
                exclude_from_capture: true,
                lock_on_minimize: false,
                remember_recent: false,
            },
            Profile::Paranoid => Self {
                profile,
                lock_after_s: Some(2 * 60),
                clipboard_clear_s: 10,
                exclude_from_capture: true,
                lock_on_minimize: true,
                remember_recent: false,
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
    pub autosave: bool,
    pub reopen_tabs: bool,
    /// Ctrl+ / Ctrl- / Ctrl+0; not a setting, but remembered (§10.4).
    pub zoom_percent: u16,
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
            autosave: true,
            reopen_tabs: true,
            zoom_percent: 100,
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Content {
    pub security: SecuritySettings,
    pub editor: EditorSettings,
    /// In tab order.
    pub notes: Vec<Note>,
    pub active_note: Option<Uuid>,
    pub trash: Vec<Note>,
}

impl Content {
    /// A fresh vault: one empty tab (DESIGN §9, step 1).
    pub fn new(profile: Profile, now: Timestamp) -> Self {
        let note = Note::new(now);
        Self {
            security: SecuritySettings::defaults_for(profile),
            editor: EditorSettings::default(),
            active_note: Some(note.id),
            notes: vec![note],
            trash: Vec::new(),
        }
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
        self.notes.push(note);
        self.active_note = Some(id);
        true
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
        if plain.len() < 4 || plain.len() % PAD_BLOCK != 0 {
            return Err(Error::BadContent);
        }
        let len = u32::from_le_bytes(plain[..4].try_into().expect("4 bytes")) as usize;
        let body = plain.get(4..4 + len).ok_or(Error::BadContent)?;
        if plain[4 + len..].iter().any(|&b| b != 0) {
            return Err(Error::BadContent);
        }
        let (version, rest) = postcard::take_from_bytes::<u16>(body).map_err(|_| Error::BadContent)?;
        if version != CONTENT_VERSION {
            return Err(Error::UnsupportedContentVersion(version));
        }
        let (content, rest) = postcard::take_from_bytes::<Content>(rest).map_err(|_| Error::BadContent)?;
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

    #[test]
    fn padded_roundtrip() {
        let c = sample();
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
        plain[4] = 2; // version varint
        assert!(matches!(
            Content::from_padded_plaintext(&plain),
            Err(Error::UnsupportedContentVersion(2))
        ));

        assert!(Content::from_padded_plaintext(&[0; 10]).is_err());
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
}
