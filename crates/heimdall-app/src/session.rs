//! An open vault: the decrypted [`Vault`], the USB key it was opened with,
//! and bookkeeping for saving.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use heimdall_core::storage::{save_vault, SaveKind};
use heimdall_core::{SlotId, SlotKind, Vault};
use uuid::Uuid;

use crate::support::{error_message, file_name};
use crate::usb::{Presence, UsbKey};

/// Encrypted autosave runs this long after the last change (DESIGN §10.2).
pub const AUTOSAVE_DELAY: Duration = Duration::from_secs(2);

/// After an access change: labels the slots that are not in `before` (just
/// created) with today's date, naming the one `usb` opens after its drive,
/// and forgets the labels of slots that are gone.
pub fn label_new_slots(vault: &mut Vault, before: &[SlotId], usb: Option<&UsbKey>) {
    let now = heimdall_core::content::now();
    let infos = vault.slot_infos();
    for info in infos.iter().filter(|i| !before.contains(&i.id)) {
        let name = match (info.kind, usb) {
            (SlotKind::PasswordKeyfile { hint }, Some(k)) if k.keyfile.hint() == hint => k.drive.name.clone(),
            _ => String::new(),
        };
        vault.content.set_label(info.id, &name, now);
    }
    let live: Vec<SlotId> = infos.iter().map(|i| i.id).collect();
    vault.content.retain_labels(&live);
}

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
    /// The USB key used to unlock (or enrolled at creation), if any.
    pub usb: Option<UsbKey>,
    /// Watching `usb`; only while the vault requires its key (DESIGN §8.3).
    pub presence: Option<Presence>,
    /// The slots changed since the last save: the next save also replaces
    /// the `.bak`, or it would keep opening the old way (DESIGN §5.4).
    pub access_changed: bool,
    /// A message for the status bar ("Exported 3 notes to …"), and when it
    /// was set. Shown for [`NOTICE_FOR`]. `true` = an error.
    pub notice: Option<(String, bool, Instant)>,
}

/// How long a status bar notice stays.
pub const NOTICE_FOR: Duration = Duration::from_secs(10);

impl Session {
    pub fn new(ctx: &egui::Context, vault: Vault, path: PathBuf, usb: Option<UsbKey>) -> Self {
        let mut s = Self {
            vault,
            path,
            dirty_notes: HashSet::new(),
            dirty: false,
            view_changed: false,
            last_change: Instant::now(),
            last_saved: None,
            save_error: None,
            usb,
            presence: None,
            access_changed: false,
            notice: None,
        };
        s.update_presence(ctx);
        s.lock_key_memory();
        s
    }

    /// Keeps the Master Key out of the page file (DESIGN §12). Call again
    /// whenever `vault` is replaced.
    pub fn lock_key_memory(&self) {
        let (address, len) = self.vault.key_memory();
        heimdall_platform::lock_memory(address, len);
    }

    /// Starts or stops presence mode after the slots changed: it is on
    /// exactly when the vault can't be opened without a USB key.
    pub fn update_presence(&mut self, ctx: &egui::Context) {
        let usb = self.usb.as_ref().filter(|k| self.key_slot(k).is_some());
        self.presence = match (usb, self.vault.requires_keyfile()) {
            (Some(key), true) => Some(Presence::start(ctx, key)),
            _ => None,
        };
    }

    /// The slot `key` opens, if it is still enrolled.
    pub fn key_slot(&self, key: &UsbKey) -> Option<SlotId> {
        let hint = key.keyfile.hint();
        self.vault
            .slot_infos()
            .into_iter()
            .find(|s| s.kind == SlotKind::PasswordKeyfile { hint })
            .map(|s| s.id)
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

    /// Shows `text` in the status bar for a while.
    pub fn notify(&mut self, text: impl Into<String>, error: bool) {
        self.notice = Some((text.into(), error, Instant::now()));
    }

    /// The current notice, if it hasn't expired.
    pub fn current_notice(&self) -> Option<(&str, bool)> {
        self.notice
            .as_ref()
            .filter(|(_, _, at)| at.elapsed() < NOTICE_FOR)
            .map(|(t, e, _)| (t.as_str(), *e))
    }

    pub fn has_unsaved(&self) -> bool {
        self.dirty || self.view_changed
    }

    pub fn autosave_due(&self) -> bool {
        self.dirty && self.vault.content.editor.autosave && self.last_change.elapsed() >= AUTOSAVE_DELAY
    }

    /// Encrypts and writes the whole vault (atomic). Keeps the previous
    /// version as `.bak`, unless the access changed.
    pub fn save(&mut self) -> bool {
        let kind = if self.access_changed {
            SaveKind::AccessChanged
        } else {
            SaveKind::Normal
        };
        let result = self
            .vault
            .to_bytes()
            .and_then(|bytes| save_vault(&self.path, &bytes, kind));
        match result {
            Ok(()) => {
                self.dirty = false;
                self.view_changed = false;
                self.dirty_notes.clear();
                self.last_saved = Some(heimdall_core::content::now());
                self.save_error = None;
                self.access_changed = false;
                true
            }
            Err(e) => {
                self.save_error = Some(error_message(&e));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heimdall_core::{KdfParams, Profile, SecretString};

    const FAST: KdfParams = KdfParams {
        m_kib: KdfParams::MIN_M_KIB,
        t: 1,
        p: 1,
    };

    #[test]
    fn labels_follow_the_slots() {
        let pw = SecretString::from("pw".to_owned());
        let new = Vault::create_with_kdf(&pw, Profile::Hardened, FAST).unwrap();
        let kf = new.keyfile.unwrap();
        let mut vault = new.vault;
        let key = UsbKey {
            keyfile: kf.clone(),
            path: "E:\\v.bifrost".into(),
            drive: heimdall_platform::Drive {
                root: "E:\\".into(),
                name: "KINGSTON".into(),
                size: 0,
            },
        };
        label_new_slots(&mut vault, &[], Some(&key));
        let infos = vault.slot_infos();
        assert_eq!(vault.content.access.len(), 2);
        let key_slot = infos
            .iter()
            .find(|i| matches!(i.kind, SlotKind::PasswordKeyfile { .. }))
            .unwrap();
        assert_eq!(vault.content.label(key_slot.id).unwrap().name, "KINGSTON");

        // Rotation keeps the key slot's id (and label) and replaces the recovery slot.
        let before: Vec<SlotId> = infos.iter().map(|i| i.id).collect();
        vault.rotate_master_key(&pw, &[&kf], &[], FAST).unwrap();
        label_new_slots(&mut vault, &before, None);
        assert_eq!(vault.content.access.len(), 2);
        assert_eq!(vault.content.label(key_slot.id).unwrap().name, "KINGSTON");
        let live: Vec<SlotId> = vault.slot_infos().iter().map(|i| i.id).collect();
        assert!(vault.content.access.iter().all(|l| live.contains(&l.slot)));
    }
}
