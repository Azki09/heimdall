//! End-to-end tests of the vault lifecycle: create, save, unlock, and every
//! access change from DESIGN §4.5, §6.2 and §9.

use heimdall_core::{
    Error, KdfParams, Keyfile, LockedVault, Profile, SecretString, SlotKind, UnlockMethod, Vault,
};

/// Cheapest parameters the parser accepts, to keep the suite fast.
const FAST: KdfParams = KdfParams {
    m_kib: KdfParams::MIN_M_KIB,
    t: 1,
    p: 1,
};

fn pw(s: &str) -> SecretString {
    SecretString::from(s.to_string())
}

fn reopen(vault: &Vault) -> LockedVault {
    LockedVault::parse(&vault.to_bytes().unwrap()).unwrap()
}

#[test]
fn basic_create_save_unlock() {
    let mut new = Vault::create_with_kdf(&pw("correct horse"), Profile::Basic, FAST).unwrap();
    assert!(new.keyfile.is_none());
    let id = new.vault.content.active_note.unwrap();
    new.vault.content.note_mut(id).unwrap().body = "top secret".into();

    let bytes = new.vault.to_bytes().unwrap();
    assert!(
        !bytes.windows(10).any(|w| w == b"top secret"),
        "plaintext leaked into the file"
    );

    let locked = LockedVault::parse(&bytes).unwrap();
    assert!(!locked.requires_keyfile());
    assert!(!locked.has_unsupported_slots());

    let v = locked.unlock_with_password(&pw("correct horse"), None).unwrap();
    assert_eq!(v.content, new.vault.content);
    assert!(matches!(v.opened_with(), UnlockMethod::Password { .. }));

    assert!(matches!(
        locked.unlock_with_password(&pw("wrong"), None),
        Err(Error::WrongPassword)
    ));
}

#[test]
fn every_save_uses_a_fresh_nonce() {
    let new = Vault::create_with_kdf(&pw("pw"), Profile::Basic, FAST).unwrap();
    assert_ne!(new.vault.to_bytes().unwrap(), new.vault.to_bytes().unwrap());
}

#[test]
fn any_flipped_bit_is_rejected() {
    let new = Vault::create_with_kdf(&pw("pw"), Profile::Basic, FAST).unwrap();
    let bytes = new.vault.to_bytes().unwrap();
    // Every header byte, plus a sample of the content.
    let positions = (0..300)
        .chain((300..bytes.len()).step_by(97))
        .chain([bytes.len() - 1]);
    for i in positions {
        let mut b = bytes.clone();
        b[i] ^= 0x01;
        let opened = LockedVault::parse(&b).and_then(|l| l.unlock_with_password(&pw("pw"), None));
        assert!(opened.is_err(), "tampering at byte {i} went undetected");
    }
}

#[test]
fn recovery_code_opens_and_reset_access_replaces_everything() {
    let new = Vault::create_with_kdf(&pw("forgotten"), Profile::Basic, FAST).unwrap();
    let old_code_phrase = new.recovery_code.phrase();
    let locked = reopen(&new.vault);

    let wrong = heimdall_core::RecoveryCode::generate().unwrap();
    assert!(matches!(
        locked.unlock_with_recovery(&wrong),
        Err(Error::WrongRecoveryCode)
    ));

    let typed = heimdall_core::RecoveryCode::parse(&old_code_phrase).unwrap();
    let mut v = locked.unlock_with_recovery(&typed).unwrap();
    assert_eq!(v.opened_with(), UnlockMethod::Recovery);

    let new_code = v.reset_access(&pw("brand new"), &[], false, FAST).unwrap();
    let locked = reopen(&v);
    assert!(locked.unlock_with_password(&pw("brand new"), None).is_ok());
    assert!(matches!(
        locked.unlock_with_password(&pw("forgotten"), None),
        Err(Error::WrongPassword)
    ));
    assert!(matches!(
        locked.unlock_with_recovery(&typed),
        Err(Error::WrongRecoveryCode)
    ));
    assert!(locked.unlock_with_recovery(&new_code).is_ok());
}

#[test]
fn hardened_needs_the_right_usb_key() {
    let new = Vault::create_with_kdf(&pw("pw"), Profile::Hardened, FAST).unwrap();
    let kf = new.keyfile.as_ref().expect("Hardened creates a key file");
    let locked = reopen(&new.vault);
    assert!(locked.requires_keyfile());

    // The key file survives a round-trip through the USB drive.
    let kf = Keyfile::parse(&kf.to_bytes()).unwrap();

    assert!(matches!(
        locked.unlock_with_password(&pw("pw"), None),
        Err(Error::KeyfileRequired)
    ));

    let other_vault = Keyfile::generate([0xEE; 16]).unwrap();
    assert!(matches!(
        locked.unlock_with_password(&pw("pw"), Some(&other_vault)),
        Err(Error::KeyfileForOtherVault)
    ));

    let not_enrolled = Keyfile::generate(*locked.vault_id()).unwrap();
    assert!(matches!(
        locked.unlock_with_password(&pw("pw"), Some(&not_enrolled)),
        Err(Error::WrongKeyfile)
    ));

    assert!(matches!(
        locked.unlock_with_password(&pw("nope"), Some(&kf)),
        Err(Error::WrongPassword)
    ));

    let v = locked.unlock_with_password(&pw("pw"), Some(&kf)).unwrap();
    assert!(matches!(v.opened_with(), UnlockMethod::PasswordKeyfile { .. }));

    // The recovery code never needs the USB key (§8.6).
    assert!(locked.unlock_with_recovery(&new.recovery_code).is_ok());
}

#[test]
fn change_password_rewraps_every_password_slot() {
    let new = Vault::create_with_kdf(&pw("old"), Profile::Hardened, FAST).unwrap();
    let kf1 = new.keyfile.unwrap();
    let mut v = new.vault;
    let kf2 = Keyfile::generate(*v.vault_id()).unwrap();
    v.add_keyfile_slot(&pw("old"), &kf2, &[&kf1], FAST).unwrap();

    // Both USB keys must be connected (§9 step 4).
    let err = v
        .change_password(&pw("old"), &pw("new"), &[&kf1], FAST)
        .unwrap_err();
    assert!(matches!(err, Error::MissingKeyfile(_)));
    // Wrong current password changes nothing.
    assert!(matches!(
        v.change_password(&pw("typo"), &pw("new"), &[&kf1, &kf2], FAST),
        Err(Error::WrongPassword)
    ));

    v.change_password(&pw("old"), &pw("new"), &[&kf1, &kf2], FAST)
        .unwrap();
    let locked = reopen(&v);
    for kf in [&kf1, &kf2] {
        assert!(locked.unlock_with_password(&pw("new"), Some(kf)).is_ok());
        assert!(matches!(
            locked.unlock_with_password(&pw("old"), Some(kf)),
            Err(Error::WrongPassword)
        ));
    }
    // No MK rotation: the recovery code is unchanged.
    assert!(locked.unlock_with_recovery(&new.recovery_code).is_ok());
}

#[test]
fn hardened_to_paranoid_changes_kdf_cost() {
    let new = Vault::create_with_kdf(&pw("pw"), Profile::Hardened, FAST).unwrap();
    let kf = new.keyfile.unwrap();
    let mut v = new.vault;
    let heavier = KdfParams {
        m_kib: 2 * FAST.m_kib,
        ..FAST
    };
    v.change_password(&pw("pw"), &pw("pw"), &[&kf], heavier).unwrap();
    let slot = v
        .slot_infos()
        .into_iter()
        .find(|s| s.kind != SlotKind::Recovery)
        .unwrap();
    assert_eq!(slot.kdf, heavier);
    assert!(reopen(&v).unlock_with_password(&pw("pw"), Some(&kf)).is_ok());
}

#[test]
fn revoking_a_usb_key_rotates_the_master_key() {
    let new = Vault::create_with_kdf(&pw("pw"), Profile::Hardened, FAST).unwrap();
    let kept = new.keyfile.unwrap();
    let mut v = new.vault;
    let lost = Keyfile::generate(*v.vault_id()).unwrap();
    let lost_slot = v.add_keyfile_slot(&pw("pw"), &lost, &[&kept], FAST).unwrap();
    let before = v.to_bytes().unwrap();

    let new_code = v
        .rotate_master_key(&pw("pw"), &[&kept], &[lost_slot], FAST)
        .unwrap();
    let locked = reopen(&v);

    assert!(matches!(
        locked.unlock_with_password(&pw("pw"), Some(&lost)),
        Err(Error::WrongKeyfile)
    ));
    assert!(locked.unlock_with_password(&pw("pw"), Some(&kept)).is_ok());
    assert!(matches!(
        locked.unlock_with_recovery(&new.recovery_code),
        Err(Error::WrongRecoveryCode)
    ));
    assert!(locked.unlock_with_recovery(&new_code).is_ok());

    // Limitation from §4.5: an old copy still opens with the revoked key...
    let old_copy = LockedVault::parse(&before).unwrap();
    assert!(old_copy.unlock_with_password(&pw("pw"), Some(&lost)).is_ok());
    // ...but grafting the old slots onto the new file does not open it,
    // because the content AAD covers the header and the MK changed.
    let new_bytes = v.to_bytes().unwrap();
    let old_header_len = old_copy_header_len(&before);
    let new_header_len = old_copy_header_len(&new_bytes);
    let mut grafted = before[..old_header_len].to_vec();
    grafted.extend_from_slice(&new_bytes[new_header_len..]);
    let grafted = LockedVault::parse(&grafted).unwrap();
    assert!(matches!(
        grafted.unlock_with_password(&pw("pw"), Some(&lost)),
        Err(Error::ContentAuthFailed)
    ));
}

/// Length of `magic .. last slot` (where the content nonce starts).
fn old_copy_header_len(bytes: &[u8]) -> usize {
    let slot_count = bytes[24] as usize;
    let mut pos = 25;
    for _ in 0..slot_count {
        let len = u16::from_le_bytes([bytes[pos + 1], bytes[pos + 2]]) as usize;
        pos += 3 + len;
    }
    pos
}

#[test]
fn rotation_with_a_typo_changes_nothing() {
    let new = Vault::create_with_kdf(&pw("pw"), Profile::Basic, FAST).unwrap();
    let mut v = new.vault;
    assert!(matches!(
        v.rotate_master_key(&pw("pW"), &[], &[], FAST),
        Err(Error::WrongPassword)
    ));
    let locked = reopen(&v);
    assert!(locked.unlock_with_password(&pw("pw"), None).is_ok());
    assert!(locked.unlock_with_recovery(&new.recovery_code).is_ok());
}

#[test]
fn rotation_guards() {
    let new = Vault::create_with_kdf(&pw("pw"), Profile::Basic, FAST).unwrap();
    let mut v = new.vault;
    let infos = v.slot_infos();
    let recovery = infos.iter().find(|s| s.kind == SlotKind::Recovery).unwrap().id;
    let password = infos.iter().find(|s| s.kind == SlotKind::Password).unwrap().id;

    assert!(matches!(
        v.rotate_master_key(&pw("pw"), &[], &[recovery], FAST),
        Err(Error::CannotRevokeRecovery)
    ));
    assert!(matches!(
        v.rotate_master_key(&pw("pw"), &[], &[password], FAST),
        Err(Error::NoPasswordSlotLeft)
    ));
    let bogus = heimdall_core::SlotId([0; 8]);
    assert!(matches!(
        v.rotate_master_key(&pw("pw"), &[], &[bogus], FAST),
        Err(Error::SlotNotFound(_))
    ));
}

#[test]
fn upgrade_basic_to_hardened() {
    // §6.2: add a USB key, then rotate the MK removing the password-only slot.
    let new = Vault::create_with_kdf(&pw("pw"), Profile::Basic, FAST).unwrap();
    let mut v = new.vault;
    let kf = Keyfile::generate(*v.vault_id()).unwrap();
    v.add_keyfile_slot(&pw("pw"), &kf, &[], FAST).unwrap();
    let pw_slot = v
        .slot_infos()
        .into_iter()
        .find(|s| s.kind == SlotKind::Password)
        .unwrap()
        .id;
    v.rotate_master_key(&pw("pw"), &[&kf], &[pw_slot], FAST).unwrap();
    v.content.security = heimdall_core::content::SecuritySettings::defaults_for(Profile::Hardened);

    assert!(v.requires_keyfile());
    let locked = reopen(&v);
    assert!(locked.requires_keyfile());
    assert!(matches!(
        locked.unlock_with_password(&pw("pw"), None),
        Err(Error::KeyfileRequired)
    ));
    let opened = locked.unlock_with_password(&pw("pw"), Some(&kf)).unwrap();
    assert_eq!(opened.content.security.profile, Profile::Hardened);
}

#[test]
fn downgrade_hardened_to_basic_keeps_usb_keys() {
    // §6.2: add a password-only slot; USB keys keep working; no rotation.
    let new = Vault::create_with_kdf(&pw("pw"), Profile::Hardened, FAST).unwrap();
    let kf = new.keyfile.unwrap();
    let mut v = new.vault;
    v.add_password_slot(&pw("pw"), &[&kf], FAST).unwrap();
    assert!(!v.requires_keyfile());

    let locked = reopen(&v);
    assert!(!locked.requires_keyfile());
    assert!(locked.unlock_with_password(&pw("pw"), None).is_ok());
    assert!(locked.unlock_with_password(&pw("pw"), Some(&kf)).is_ok());
    assert!(locked.unlock_with_recovery(&new.recovery_code).is_ok());

    // A USB key that is not enrolled falls back to the password-only slot.
    let stray = Keyfile::generate(*locked.vault_id()).unwrap();
    assert!(locked.unlock_with_password(&pw("pw"), Some(&stray)).is_ok());
}

#[test]
fn adding_access_requires_the_password() {
    let new = Vault::create_with_kdf(&pw("pw"), Profile::Basic, FAST).unwrap();
    let mut v = new.vault;
    let kf = Keyfile::generate(*v.vault_id()).unwrap();
    assert!(matches!(
        v.add_keyfile_slot(&pw("bad"), &kf, &[], FAST),
        Err(Error::WrongPassword)
    ));
    let foreign = Keyfile::generate([1; 16]).unwrap();
    assert!(matches!(
        v.add_keyfile_slot(&pw("pw"), &foreign, &[], FAST),
        Err(Error::KeyfileForOtherVault)
    ));
    // Enrolling the same key twice is a no-op.
    let a = v.add_keyfile_slot(&pw("pw"), &kf, &[], FAST).unwrap();
    let b = v.add_keyfile_slot(&pw("pw"), &kf, &[], FAST).unwrap();
    assert_eq!(a, b);
    assert_eq!(v.slot_infos().len(), 3);
}

#[test]
fn notes_survive_a_full_save_cycle_on_disk() {
    use heimdall_core::storage::{read_vault, save_vault, SaveKind};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("personal.heim");

    let mut v = Vault::create_with_kdf(&pw("pw"), Profile::Basic, FAST)
        .unwrap()
        .vault;
    let id = v.content.active_note.unwrap();
    v.content.note_mut(id).unwrap().body = "first".into();
    save_vault(&path, &v.to_bytes().unwrap(), SaveKind::Normal).unwrap();
    v.content.note_mut(id).unwrap().body = "second".into();
    save_vault(&path, &v.to_bytes().unwrap(), SaveKind::Normal).unwrap();

    let main = LockedVault::parse(&read_vault(&path).unwrap()).unwrap();
    let bak = LockedVault::parse(&read_vault(&heimdall_core::storage::backup_path(&path)).unwrap()).unwrap();
    let main = main.unlock_with_password(&pw("pw"), None).unwrap();
    let bak = bak.unlock_with_password(&pw("pw"), None).unwrap();
    assert_eq!(main.content.note(id).unwrap().body, "second");
    assert_eq!(bak.content.note(id).unwrap().body, "first");
}
