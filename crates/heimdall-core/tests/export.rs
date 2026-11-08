//! Export to a new vault and import from another one (DESIGN §10.9).

use std::path::PathBuf;

use heimdall_core::content::CodeMode;
use heimdall_core::{
    Error, KdfParams, Keyfile, LockedVault, Profile, RecoveryCode, SecretString, SlotKind, Vault,
};

const FAST: KdfParams = KdfParams {
    m_kib: KdfParams::MIN_M_KIB,
    t: 1,
    p: 1,
};
/// A little more costly than [`FAST`], to tell them apart.
const SLOWER: KdfParams = KdfParams {
    m_kib: KdfParams::MIN_M_KIB * 2,
    t: 2,
    p: 1,
};

fn pw(s: &str) -> SecretString {
    SecretString::from(s.to_string())
}

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// A Hardened vault with three notes (one in code mode), a trashed note,
/// a dictionary word and a USB label.
fn source() -> (Vault, Keyfile) {
    let new = Vault::create_with_kdf(&pw("source password"), Profile::Hardened, SLOWER).unwrap();
    let keyfile = new.keyfile.unwrap();
    let mut v = new.vault;
    let c = &mut v.content;
    c.notes[0].title = "Servers".into();
    c.notes[0].body = "db: {*Pg$ecret*}".into();
    let b = c.new_note(10);
    c.note_mut(b).unwrap().body = "Shopping\nmilk".into();
    let code = c.new_note(20);
    let n = c.note_mut(code).unwrap();
    n.title = "main.rs".into();
    n.body = "fn main() {}".into();
    n.code = Some(CodeMode {
        language: "rust".into(),
    });
    let gone = c.new_note(30);
    c.note_mut(gone).unwrap().body = "in the trash".into();
    c.close_note(gone, 40);
    c.add_to_dictionary("Heimdall");
    let usb = v
        .slot_infos()
        .into_iter()
        .find(|s| matches!(s.kind, SlotKind::PasswordKeyfile { .. }))
        .unwrap();
    v.content.set_label(usb.id, "KINGSTON", 50);
    (v, keyfile)
}

#[test]
fn export_is_an_independent_basic_vault() {
    let (src, _) = source();
    let ids = [src.content.notes[0].id, src.content.notes[2].id];
    assert_eq!(src.export_kdf(), SLOWER);

    let new = src.export_notes(&ids, &pw("export password"), FAST).unwrap();
    assert!(new.keyfile.is_none());
    assert_ne!(new.vault.vault_id(), src.vault_id());
    let bytes = new.vault.to_bytes().unwrap();
    assert!(!bytes.windows(8).any(|w| w == b"Pg$ecret"));

    let locked = LockedVault::parse(&bytes).unwrap();
    assert!(!locked.requires_keyfile() && !locked.has_keyfile_slots());
    let v = locked.unlock_with_password(&pw("export password"), None).unwrap();
    assert!(locked.unlock_with_password(&pw("source password"), None).is_err());

    let kinds: Vec<_> = v.slot_infos().iter().map(|s| (s.kind, s.kdf)).collect();
    assert_eq!(kinds[0], (SlotKind::Password, FAST));
    assert_eq!(kinds[1].0, SlotKind::Recovery);

    let c = &v.content;
    assert_eq!(c.security.profile, Profile::Basic);
    assert_eq!(c.notes.len(), 2);
    assert_eq!(c.notes[0].body, "db: {*Pg$ecret*}");
    assert_eq!(c.notes[1].title, "main.rs");
    assert!(c.notes[1].code.is_some());
    assert!(c.trash.is_empty());
    assert!(c.dictionary.is_empty());
    assert!(c.access.is_empty());

    let r = locked.unlock_with_recovery(&new.recovery_code).unwrap();
    assert_eq!(r.content, v.content);
}

#[test]
fn export_inherits_the_most_costly_password_slot() {
    let (mut src, kf) = source();
    // A password-only slot with a cheaper cost (downgraded to Basic).
    src.add_password_slot(&pw("source password"), &[&kf], FAST)
        .unwrap();
    assert_eq!(src.export_kdf(), SLOWER);

    let basic = Vault::create_with_kdf(&pw("x"), Profile::Basic, FAST)
        .unwrap()
        .vault;
    assert_eq!(basic.export_kdf(), FAST);
}

#[test]
fn export_needs_a_note() {
    let (src, _) = source();
    let trashed = src.content.trash[0].id;
    assert!(matches!(
        src.export_notes(&[], &pw("p"), FAST),
        Err(Error::NothingToExport)
    ));
    assert!(matches!(
        src.export_notes(&[trashed], &pw("p"), FAST),
        Err(Error::NothingToExport)
    ));
}

/// The same-password warning checks the new password against the slots.
#[test]
fn same_password_is_detected_without_storing_it() {
    let (src, kf) = source();
    assert!(src.verify_password(&pw("source password"), &[&kf]).is_ok());
    assert!(src.verify_password(&pw("export password"), &[&kf]).is_err());
}

#[test]
fn fixtures_can_be_imported() {
    let (mut dest, _) = source();
    let before = dest.content.notes.len();

    let basic = LockedVault::parse(&fixture("basic.heim"))
        .unwrap()
        .unlock_with_password(&pw("heimdall fixture basic"), None)
        .unwrap();
    let hardened_locked = LockedVault::parse(&fixture("hardened.heim")).unwrap();
    let kf = Keyfile::parse(&fixture("hardened.bifrost")).unwrap();
    let hardened = hardened_locked
        .unlock_with_password(&pw("heimdall fixture hardened"), Some(&kf))
        .unwrap();
    let code = String::from_utf8(fixture("hardened.recovery.txt")).unwrap();
    let recovered = hardened_locked
        .unlock_with_recovery(&RecoveryCode::parse(code.trim()).unwrap())
        .unwrap();
    assert_eq!(recovered.content, hardened.content);

    let ids = dest.content.import_notes(basic.content.notes.iter().cloned());
    assert_eq!(ids.len(), basic.content.notes.len());
    dest.content.import_notes(hardened.content.notes.iter().cloned());

    let notes = &dest.content.notes;
    assert_eq!(notes.len(), before + 2 * basic.content.notes.len());
    // Same titles twice: the second copy is renamed. The trash isn't imported.
    let titles: Vec<&str> = notes[before..].iter().map(|n| n.display_title()).collect();
    assert_eq!(
        titles,
        [
            "Wi-Fi",
            "Servers (2)",
            "main.rs (2)",
            "Wi-Fi (2)",
            "Servers (3)",
            "main.rs (3)"
        ]
    );
    assert_eq!(notes[before + 1].body, basic.content.notes[1].body);
    assert!(notes[before + 2].code.is_some());
    assert_eq!(dest.content.trash.len(), 1);
}
