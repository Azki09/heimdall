//! Opening, saving and managing access to a vault (DESIGN §4, §6.2, §9).
//!
//! A [`LockedVault`] is a parsed file that has not been decrypted. Unlocking
//! it yields a [`Vault`], which holds the Master Key (MK) and the decrypted
//! [`Content`] in memory until it is dropped.

use secrecy::{ExposeSecret, SecretString};

use crate::content::{Content, Profile};
use crate::crypto::{self, KdfParams, Key};
use crate::error::{Error, Result};
use crate::format::{Header, KnownSlot, Slot, SlotId, SlotKind, VaultFile, VaultId};
use crate::keyfile::Keyfile;
use crate::recovery::RecoveryCode;

impl KdfParams {
    /// Argon2id cost for password slots of each profile (DESIGN §6).
    pub fn for_profile(profile: Profile) -> Self {
        match profile {
            Profile::Basic | Profile::Hardened => Self::STANDARD,
            Profile::Paranoid => Self::PARANOID,
        }
    }
}

/// How the vault in memory was opened. After [`UnlockMethod::Recovery`] the
/// app must force [`Vault::reset_access`] (DESIGN §7, §9 step 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnlockMethod {
    Created,
    Password { slot: SlotId },
    PasswordKeyfile { slot: SlotId },
    Recovery,
}

/// What the UI needs to list a slot in Settings → Access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotInfo {
    pub id: SlotId,
    pub kind: SlotKind,
    pub kdf: KdfParams,
}

/// A parsed but still encrypted vault file.
#[derive(Debug, Clone)]
pub struct LockedVault {
    file: VaultFile,
}

impl LockedVault {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Ok(Self {
            file: VaultFile::parse(bytes)?,
        })
    }

    pub fn vault_id(&self) -> &VaultId {
        &self.file.header.vault_id
    }

    /// Whether the Unlock screen must ask for the USB key (DESIGN §5.1).
    pub fn requires_keyfile(&self) -> bool {
        self.file.header.requires_keyfile()
    }

    /// Whether a USB key can open this vault, required or not (a Basic vault
    /// that was downgraded from Hardened keeps its USB slots, §6.2).
    pub fn has_keyfile_slots(&self) -> bool {
        self.file
            .header
            .known_slots()
            .any(|s| matches!(s.kind, SlotKind::PasswordKeyfile { .. }))
    }

    /// Whether `keyfile` is enrolled in this vault. A key file of the right
    /// vault that matches no slot was revoked (or belongs to an older copy).
    pub fn accepts_keyfile(&self, keyfile: &Keyfile) -> bool {
        let hint = keyfile.hint();
        keyfile.vault_id() == self.vault_id()
            && self
                .file
                .header
                .known_slots()
                .any(|s| s.kind == SlotKind::PasswordKeyfile { hint })
    }

    /// "This vault has an access method this version doesn't support" (§5.2).
    pub fn has_unsupported_slots(&self) -> bool {
        self.file.header.has_unknown_slots()
    }

    /// Opens with the password, plus the USB key file if one is connected.
    ///
    /// With a key file, the slot whose `keyfile_hint` matches is used, so only
    /// one Argon2id run is needed. A key file that matches no slot falls back
    /// to a password-only slot if the vault has one (Basic after a downgrade).
    pub fn unlock_with_password(&self, password: &SecretString, keyfile: Option<&Keyfile>) -> Result<Vault> {
        let header = &self.file.header;
        let vault_id = &header.vault_id;

        if let Some(kf) = keyfile {
            if kf.vault_id() != vault_id {
                return Err(Error::KeyfileForOtherVault);
            }
            let hint = kf.hint();
            for slot in header.known_slots() {
                if slot.kind == (SlotKind::PasswordKeyfile { hint }) {
                    let kek = slot_key(vault_id, slot, password.expose_secret().as_bytes(), Some(kf))?;
                    let mk = crypto::unwrap_key(&kek, &slot.nonce, &slot.wrapped_mk, &slot.aad(vault_id))
                        .ok_or(Error::WrongPassword)?;
                    return self.finish(mk, UnlockMethod::PasswordKeyfile { slot: slot.id });
                }
            }
        }

        let password_slots: Vec<&KnownSlot> = header
            .known_slots()
            .filter(|s| s.kind == SlotKind::Password)
            .collect();
        if password_slots.is_empty() {
            return Err(match (keyfile, header.requires_keyfile()) {
                (Some(_), true) => Error::WrongKeyfile,
                (None, true) => Error::KeyfileRequired,
                _ => Error::NoUsableSlot,
            });
        }
        for slot in password_slots {
            let kek = slot_key(vault_id, slot, password.expose_secret().as_bytes(), None)?;
            if let Some(mk) = crypto::unwrap_key(&kek, &slot.nonce, &slot.wrapped_mk, &slot.aad(vault_id)) {
                return self.finish(mk, UnlockMethod::Password { slot: slot.id });
            }
        }
        Err(Error::WrongPassword)
    }

    /// Opens with the 12-word recovery code. No password or USB needed
    /// (DESIGN §8.6). The caller must then run [`Vault::reset_access`].
    pub fn unlock_with_recovery(&self, code: &RecoveryCode) -> Result<Vault> {
        let header = &self.file.header;
        let vault_id = &header.vault_id;
        let mut found = false;
        for slot in header.known_slots().filter(|s| s.kind == SlotKind::Recovery) {
            found = true;
            let kek = crypto::argon2id(code.secret(), &slot.salt, slot.kdf)?;
            if let Some(mk) = crypto::unwrap_key(&kek, &slot.nonce, &slot.wrapped_mk, &slot.aad(vault_id)) {
                return self.finish(mk, UnlockMethod::Recovery);
            }
        }
        Err(if found {
            Error::WrongRecoveryCode
        } else {
            Error::NoUsableSlot
        })
    }

    fn finish(&self, mk: Key, opened_with: UnlockMethod) -> Result<Vault> {
        let mut buf = zeroize::Zeroizing::new(self.file.content.clone());
        crypto::open_in_place(&mk, &self.file.content_nonce, &self.file.header_bytes, &mut buf)?;
        let content = Content::from_padded_plaintext(&buf)?;
        Ok(Vault {
            vault_id: self.file.header.vault_id,
            slots: self.file.header.slots.clone(),
            mk,
            content,
            opened_with,
        })
    }
}

/// Result of [`Vault::create`]. The recovery code must be shown, printed and
/// confirmed before the vault is first saved (DESIGN §7, §9 step 1).
pub struct NewVault {
    pub vault: Vault,
    pub recovery_code: RecoveryCode,
    /// Present for Hardened / Paranoid: write it to the USB drive as `.bifrost`.
    pub keyfile: Option<Keyfile>,
}

/// An unlocked vault. Dropping it wipes the MK and the note text.
pub struct Vault {
    vault_id: VaultId,
    slots: Vec<Slot>,
    mk: Key,
    pub content: Content,
    opened_with: UnlockMethod,
}

impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vault")
            .field("slots", &self.slot_infos())
            .field("opened_with", &self.opened_with)
            .finish_non_exhaustive()
    }
}

impl Vault {
    /// Creates a new vault. Basic gets a password slot; Hardened and
    /// Paranoid get a password + USB slot and a freshly generated key file.
    /// Every vault gets a recovery slot.
    pub fn create(password: &SecretString, profile: Profile) -> Result<NewVault> {
        Self::create_with_kdf(password, profile, KdfParams::for_profile(profile))
    }

    /// Like [`Vault::create`] with an explicit Argon2id cost (calibration, tests).
    pub fn create_with_kdf(password: &SecretString, profile: Profile, kdf: KdfParams) -> Result<NewVault> {
        let vault_id: VaultId = crypto::random_bytes()?;
        let mk = crypto::random_key()?;

        let keyfile = match profile {
            Profile::Basic => None,
            Profile::Hardened | Profile::Paranoid => Some(Keyfile::generate(vault_id)?),
        };
        let recovery_code = RecoveryCode::generate()?;
        let slots = vec![
            Slot::Known(new_password_slot(
                &vault_id,
                &mk,
                new_slot_id()?,
                password,
                keyfile.as_ref(),
                kdf,
            )?),
            Slot::Known(new_recovery_slot(&vault_id, &mk, &recovery_code)?),
        ];

        let vault = Vault {
            vault_id,
            slots,
            mk,
            content: Content::new(profile, crate::content::now()),
            opened_with: UnlockMethod::Created,
        };
        Ok(NewVault {
            vault,
            recovery_code,
            keyfile,
        })
    }

    pub fn vault_id(&self) -> &VaultId {
        &self.vault_id
    }

    /// Where the Master Key lives (address, length), so the app can keep
    /// that memory out of the page file (DESIGN §12). Not the key itself.
    pub fn key_memory(&self) -> (usize, usize) {
        let mk = self.mk.expose_secret();
        (mk.as_ptr() as usize, mk.len())
    }

    /// A second copy of the open vault, MK included. Access changes run
    /// Argon2id several times; the app runs them on a copy in a background
    /// thread and swaps it in when done, so the window never freezes.
    pub fn duplicate(&self) -> Vault {
        Vault {
            vault_id: self.vault_id,
            slots: self.slots.clone(),
            mk: crypto::new_key(*zeroize::Zeroizing::new(*self.mk.expose_secret())),
            content: self.content.clone(),
            opened_with: self.opened_with,
        }
    }

    pub fn opened_with(&self) -> UnlockMethod {
        self.opened_with
    }

    pub fn slot_infos(&self) -> Vec<SlotInfo> {
        self.known_slots()
            .map(|s| SlotInfo {
                id: s.id,
                kind: s.kind,
                kdf: s.kdf,
            })
            .collect()
    }

    pub fn requires_keyfile(&self) -> bool {
        self.header().requires_keyfile()
    }

    /// Encrypts the whole vault with a fresh content nonce (DESIGN §4.2).
    /// The header is the content AAD, so any slot change is covered.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let header = self.header();
        let header_bytes = header.to_bytes()?;
        let content_nonce = crypto::random_bytes()?;
        let mut buf = self.content.to_padded_plaintext()?;
        crypto::seal_in_place(&self.mk, &content_nonce, &header_bytes, &mut buf)?;
        // `buf` now holds ciphertext; move it out without leaving a copy behind.
        let content = std::mem::take(&mut *buf);
        Ok(VaultFile {
            header,
            header_bytes,
            content_nonce,
            content,
        }
        .to_bytes())
    }

    /// Checks the password (and USB keys) against the current slots without
    /// changing anything. Used before every access change so a typo can
    /// never lock the user out.
    pub fn verify_password(&self, password: &SecretString, keyfiles: &[&Keyfile]) -> Result<()> {
        let pw = password.expose_secret().as_bytes();
        let mut saw_keyfile_slot = false;
        // Prefer a password-only slot (one Argon2id run, no USB needed).
        let mut candidates: Vec<(&KnownSlot, Option<&Keyfile>)> = Vec::new();
        for slot in self.known_slots() {
            match slot.kind {
                SlotKind::Password => candidates.insert(0, (slot, None)),
                SlotKind::PasswordKeyfile { hint } => {
                    saw_keyfile_slot = true;
                    if let Some(kf) = keyfiles.iter().find(|k| k.hint() == hint) {
                        candidates.push((slot, Some(kf)));
                    }
                }
                SlotKind::Recovery => {}
            }
        }
        let Some(&(slot, kf)) = candidates.first() else {
            return Err(if saw_keyfile_slot {
                Error::KeyfileRequired
            } else {
                Error::NoUsableSlot
            });
        };
        let kek = slot_key(&self.vault_id, slot, pw, kf)?;
        crypto::unwrap_key(&kek, &slot.nonce, &slot.wrapped_mk, &slot.aad(&self.vault_id))
            .map(|_| ())
            .ok_or(Error::WrongPassword)
    }

    /// Change password (DESIGN §9 step 4): re-wraps the MK in every slot that
    /// uses the password, so the vault never has two passwords. Every kept
    /// USB key must be connected. The MK is not rotated.
    ///
    /// Also used to switch between Hardened and Paranoid (same password, new
    /// Argon2id cost, DESIGN §6.2).
    pub fn change_password(
        &mut self,
        current: &SecretString,
        new: &SecretString,
        keyfiles: &[&Keyfile],
        kdf: KdfParams,
    ) -> Result<()> {
        self.verify_password(current, keyfiles)?;
        let mut slots = Vec::with_capacity(self.slots.len());
        for slot in &self.slots {
            match slot {
                Slot::Known(k) if k.kind.uses_password() => {
                    let kf = keyfile_for(k, keyfiles)?;
                    slots.push(Slot::Known(new_password_slot(
                        &self.vault_id,
                        &self.mk,
                        k.id,
                        new,
                        kf,
                        kdf,
                    )?));
                }
                other => slots.push(other.clone()),
            }
        }
        self.slots = slots;
        Ok(())
    }

    /// Adds a password-only slot: downgrading to Basic keeps the USB slots
    /// but makes the USB optional (DESIGN §6.2). No MK rotation.
    pub fn add_password_slot(
        &mut self,
        password: &SecretString,
        keyfiles: &[&Keyfile],
        kdf: KdfParams,
    ) -> Result<SlotId> {
        self.verify_password(password, keyfiles)?;
        if let Some(existing) = self.known_slots().find(|s| s.kind == SlotKind::Password) {
            return Ok(existing.id);
        }
        let slot = new_password_slot(&self.vault_id, &self.mk, new_slot_id()?, password, None, kdf)?;
        let id = slot.id;
        self.push_slot(slot)?;
        Ok(id)
    }

    /// Enrolls a new USB key (DESIGN §8). No MK rotation: no access is lost.
    /// `existing_keyfiles` is only needed if the vault has no password-only slot.
    pub fn add_keyfile_slot(
        &mut self,
        password: &SecretString,
        new_keyfile: &Keyfile,
        existing_keyfiles: &[&Keyfile],
        kdf: KdfParams,
    ) -> Result<SlotId> {
        if new_keyfile.vault_id() != &self.vault_id {
            return Err(Error::KeyfileForOtherVault);
        }
        self.verify_password(password, existing_keyfiles)?;
        let hint = new_keyfile.hint();
        if let Some(existing) = self
            .known_slots()
            .find(|s| s.kind == SlotKind::PasswordKeyfile { hint })
        {
            return Ok(existing.id);
        }
        let slot = new_password_slot(
            &self.vault_id,
            &self.mk,
            new_slot_id()?,
            password,
            Some(new_keyfile),
            kdf,
        )?;
        let id = slot.id;
        self.push_slot(slot)?;
        Ok(id)
    }

    /// Generates a new MK, re-wraps it in every password slot that is kept,
    /// drops the slots in `remove`, and replaces the recovery slot with a new
    /// code (DESIGN §4.5). Covers: revoke a USB, "Rotate keys now",
    /// "Regenerate recovery code" and upgrading from Basic (remove the
    /// password-only slot).
    ///
    /// Slots of unknown types cannot be re-wrapped and are dropped.
    pub fn rotate_master_key(
        &mut self,
        password: &SecretString,
        keyfiles: &[&Keyfile],
        remove: &[SlotId],
        kdf: KdfParams,
    ) -> Result<RecoveryCode> {
        for id in remove {
            match self.known_slots().find(|s| s.id == *id) {
                None => return Err(Error::SlotNotFound(*id)),
                Some(s) if s.kind == SlotKind::Recovery => return Err(Error::CannotRevokeRecovery),
                Some(_) => {}
            }
        }
        self.verify_password(password, keyfiles)?;

        let kept: Vec<&KnownSlot> = self
            .known_slots()
            .filter(|s| s.kind.uses_password() && !remove.contains(&s.id))
            .collect();
        if kept.is_empty() {
            return Err(Error::NoPasswordSlotLeft);
        }

        let new_mk = crypto::random_key()?;
        let mut slots = Vec::with_capacity(kept.len() + 1);
        for k in kept {
            let kf = keyfile_for(k, keyfiles)?;
            slots.push(Slot::Known(new_password_slot(
                &self.vault_id,
                &new_mk,
                k.id,
                password,
                kf,
                kdf,
            )?));
        }
        let code = RecoveryCode::generate()?;
        slots.push(Slot::Known(new_recovery_slot(&self.vault_id, &new_mk, &code)?));

        self.slots = slots;
        self.mk = new_mk;
        Ok(code)
    }

    /// After opening with the recovery code (DESIGN §9 step 5, §8.5): the
    /// old password may be lost, so every slot is replaced. A new MK, a slot
    /// for `new_password` with each given USB key, a password-only slot if
    /// `password_only` (or if no USB key is given), and a new recovery code.
    pub fn reset_access(
        &mut self,
        new_password: &SecretString,
        keyfiles: &[&Keyfile],
        password_only: bool,
        kdf: KdfParams,
    ) -> Result<RecoveryCode> {
        if keyfiles.iter().any(|k| k.vault_id() != &self.vault_id) {
            return Err(Error::KeyfileForOtherVault);
        }
        let new_mk = crypto::random_key()?;
        let mut slots = Vec::with_capacity(keyfiles.len() + 2);
        if password_only || keyfiles.is_empty() {
            slots.push(Slot::Known(new_password_slot(
                &self.vault_id,
                &new_mk,
                new_slot_id()?,
                new_password,
                None,
                kdf,
            )?));
        }
        for kf in keyfiles {
            let hint = kf.hint();
            let dup = slots
                .iter()
                .any(|s| matches!(s, Slot::Known(k) if k.kind == SlotKind::PasswordKeyfile { hint }));
            if !dup {
                slots.push(Slot::Known(new_password_slot(
                    &self.vault_id,
                    &new_mk,
                    new_slot_id()?,
                    new_password,
                    Some(kf),
                    kdf,
                )?));
            }
        }
        let code = RecoveryCode::generate()?;
        slots.push(Slot::Known(new_recovery_slot(&self.vault_id, &new_mk, &code)?));
        if slots.len() > crate::format::MAX_SLOTS {
            return Err(Error::TooManySlots(slots.len()));
        }

        self.slots = slots;
        self.mk = new_mk;
        self.opened_with = UnlockMethod::Created;
        Ok(code)
    }

    fn known_slots(&self) -> impl Iterator<Item = &KnownSlot> {
        self.slots.iter().filter_map(|s| match s {
            Slot::Known(k) => Some(k),
            Slot::Unknown { .. } => None,
        })
    }

    fn header(&self) -> Header {
        Header {
            vault_id: self.vault_id,
            slots: self.slots.clone(),
        }
    }

    fn push_slot(&mut self, slot: KnownSlot) -> Result<()> {
        if self.slots.len() >= crate::format::MAX_SLOTS {
            return Err(Error::TooManySlots(self.slots.len() + 1));
        }
        self.slots.push(Slot::Known(slot));
        Ok(())
    }
}

fn new_slot_id() -> Result<SlotId> {
    Ok(SlotId(crypto::random_bytes()?))
}

/// The USB key needed to re-wrap `slot`, if it is a keyfile slot.
fn keyfile_for<'a>(slot: &KnownSlot, keyfiles: &[&'a Keyfile]) -> Result<Option<&'a Keyfile>> {
    match slot.kind {
        SlotKind::PasswordKeyfile { hint } => keyfiles
            .iter()
            .find(|k| k.hint() == hint)
            .copied()
            .map(Some)
            .ok_or(Error::MissingKeyfile(slot.id)),
        _ => Ok(None),
    }
}

/// Derives the key-encryption key of a password or password+USB slot (§4.3).
fn slot_key(vault_id: &VaultId, slot: &KnownSlot, password: &[u8], keyfile: Option<&Keyfile>) -> Result<Key> {
    let p = crypto::argon2id(password, &slot.salt, slot.kdf)?;
    match keyfile {
        Some(kf) => crypto::combine_password_keyfile(&p, kf.secret(), vault_id),
        None => Ok(p),
    }
}

/// A password slot (type 1) or, with a key file, a password+USB slot (type 3),
/// with fresh salt and nonce.
fn new_password_slot(
    vault_id: &VaultId,
    mk: &Key,
    id: SlotId,
    password: &SecretString,
    keyfile: Option<&Keyfile>,
    kdf: KdfParams,
) -> Result<KnownSlot> {
    let kind = match keyfile {
        Some(kf) => SlotKind::PasswordKeyfile { hint: kf.hint() },
        None => SlotKind::Password,
    };
    let mut slot = KnownSlot {
        kind,
        id,
        kdf,
        salt: crypto::random_bytes()?,
        nonce: crypto::random_bytes()?,
        wrapped_mk: [0; crate::format::WRAPPED_LEN],
    };
    let kek = slot_key(vault_id, &slot, password.expose_secret().as_bytes(), keyfile)?;
    slot.wrapped_mk = crypto::wrap_key(&kek, &slot.nonce, mk, &slot.aad(vault_id))?;
    Ok(slot)
}

fn new_recovery_slot(vault_id: &VaultId, mk: &Key, code: &RecoveryCode) -> Result<KnownSlot> {
    let mut slot = KnownSlot {
        kind: SlotKind::Recovery,
        id: new_slot_id()?,
        kdf: KdfParams::RECOVERY,
        salt: crypto::random_bytes()?,
        nonce: crypto::random_bytes()?,
        wrapped_mk: [0; crate::format::WRAPPED_LEN],
    };
    let kek = crypto::argon2id(code.secret(), &slot.salt, slot.kdf)?;
    slot.wrapped_mk = crypto::wrap_key(&kek, &slot.nonce, mk, &slot.aad(vault_id))?;
    Ok(slot)
}
