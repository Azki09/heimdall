//! An open vault: the decrypted [`Vault`] plus bookkeeping for saving.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use heimdall_core::storage::{save_vault, SaveKind};
use heimdall_core::Vault;
use uuid::Uuid;

use crate::support::{error_message, file_name};

/// Encrypted autosave runs this long after the last change (DESIGN §10.2).
pub const AUTOSAVE_DELAY: Duration = Duration::from_secs(2);

pub struct Session {
    pub vault: Vault,
    pub path: PathBuf,
    /// Notes with unsaved edits, for the dot on their tab.
    pub dirty_notes: HashSet<Uuid>,
    /// Anything unsaved at all (text, tab order, settings, trash).
    pub dirty: bool,
    /// Active tab or cursor moved. Saved on lock/close, but doesn't trigger autosave.
    pub view_changed: bool,
    pub last_change: Instant,
    pub last_saved: Option<i64>,
    pub save_error: Option<String>,
}

impl Session {
    pub fn new(vault: Vault, path: PathBuf) -> Self {
        Self {
            vault,
            path,
            dirty_notes: HashSet::new(),
            dirty: false,
            view_changed: false,
            last_change: Instant::now(),
            last_saved: None,
            save_error: None,
        }
    }

    pub fn file_name(&self) -> String {
        file_name(&self.path)
    }

    /// Records a change; `note` gets the unsaved-changes dot.
    pub fn touch(&mut self, note: Option<Uuid>) {
        self.dirty = true;
        self.last_change = Instant::now();
        if let Some(id) = note {
            self.dirty_notes.insert(id);
        }
    }

    pub fn has_unsaved(&self) -> bool {
        self.dirty || self.view_changed
    }

    pub fn autosave_due(&self) -> bool {
        self.dirty && self.vault.content.editor.autosave && self.last_change.elapsed() >= AUTOSAVE_DELAY
    }

    /// Encrypts and writes the whole vault (atomic, keeps `.bak`).
    pub fn save(&mut self) -> bool {
        let result = self
            .vault
            .to_bytes()
            .and_then(|bytes| save_vault(&self.path, &bytes, SaveKind::Normal));
        match result {
            Ok(()) => {
                self.dirty = false;
                self.view_changed = false;
                self.dirty_notes.clear();
                self.last_saved = Some(heimdall_core::content::now());
                self.save_error = None;
                true
            }
            Err(e) => {
                self.save_error = Some(error_message(&e));
                false
            }
        }
    }
}
