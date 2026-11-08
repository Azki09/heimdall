//! Compatibility promise for format 1 / content versions 4 and 5
//! (DESIGN §5.6).
//!
//! The files in `tests/fixtures/` were written once and are committed:
//! `basic.heim` and `hardened.heim` by the code of v0.5 (content version 4),
//! `basic-v5.heim` by v1.0.0 (content version 5, what 1.x writes). Every future version must still open them and read exactly
//! the same content. Never regenerate them to make this test pass: if it
//! fails, the change broke existing vaults.
//!
//! They were created with `cargo test -p heimdall-core --test format_v1 --
//! --ignored write_fixtures` (and `write_fixture_v5`), which only exist to
//! document how.

use std::path::PathBuf;

use heimdall_core::content::{CodeMode, Font, LineEnding, SpellLanguage, TabInsert};
use heimdall_core::{
    Content, KdfParams, Keyfile, LockedVault, Note, Profile, RecoveryCode, SecretString, SlotKind,
    UnlockMethod, Vault,
};
use uuid::Uuid;

const BASIC_PASSWORD: &str = "heimdall fixture basic";
const HARDENED_PASSWORD: &str = "heimdall fixture hardened";
/// Fixed timestamps so the expected content is exact.
const T0: i64 = 1_793_000_000;
/// The USB name shown in Settings → Access for the Hardened fixture.
const USB_NAME: &str = "FIXTURE";

/// Cheapest parameters the parser accepts, to keep the suite fast.
const FAST: KdfParams = KdfParams {
    m_kib: KdfParams::MIN_M_KIB,
    t: 1,
    p: 1,
};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn read(name: &str) -> Vec<u8> {
    std::fs::read(fixtures().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn pw(s: &str) -> SecretString {
    SecretString::from(s.to_string())
}

fn note(n: u128, title: &str, body: &str, cursor: u64) -> Note {
    let mut note = Note::new(T0);
    note.id = Uuid::from_u128(n);
    note.title = title.into();
    note.body = body.into();
    note.modified = T0 + n as i64;
    note.cursor = cursor;
    note
}

/// Everything content version 4 can hold: several tabs, a masked secret, an
/// escaped `{*`, a code-mode tab with its wrap override, the trash, the
/// personal dictionary and non-default settings.
fn expected(profile: Profile) -> Content {
    let mut c = Content::new(profile, T0);
    c.notes = vec![
        note(
            1,
            "",
            "Wi-Fi\r\npassword: {*hunter2 is not a password*}\r\nliteral: \\{*not a secret*}\r\nñandú, 日本語",
            5,
        ),
        note(2, "Servers", "db: {*Pg$ecret*}\napi key: {*sk-123*}", 0),
        {
            let mut n = note(3, "main.rs", "fn main() {\n    println!(\"hi\");\n}\n", 12);
            n.code = Some(CodeMode {
                language: "rust".into(),
            });
            n.wrap = Some(true);
            n
        },
    ];
    c.active_note = Some(Uuid::from_u128(2));
    c.trash = vec![{
        let mut n = note(4, "Old", "closed note", 3);
        n.deleted_at = Some(T0 + 100);
        n
    }];
    c.dictionary = vec!["Heimdall".into(), "Bifrost".into()];

    c.security.lock_after_s = Some(600);
    c.security.clipboard_clear_s = 20;
    c.security.secret_clear_s = 7;
    c.security.lock_on_minimize = true;

    c.editor.font = Font::Consolas;
    c.editor.font_size = 16;
    c.editor.tab_inserts = TabInsert::Spaces(2);
    c.editor.line_ending = LineEnding::Lf;
    c.editor.word_wrap = false;
    c.editor.line_numbers = true;
    c.editor.spell_check = true;
    c.editor.spell_language = SpellLanguage::Spanish;
    c.editor.zoom_percent = 125;
    c.editor.code_tip = false;
    c.editor.secret_tip = false;
    c
}

#[test]
fn basic_fixture_opens_with_password_and_recovery() {
    let locked = LockedVault::parse(&read("basic.heim")).unwrap();
    assert!(!locked.requires_keyfile());
    assert!(!locked.has_unsupported_slots());

    let v = locked.unlock_with_password(&pw(BASIC_PASSWORD), None).unwrap();
    assert!(matches!(v.opened_with(), UnlockMethod::Password { .. }));
    assert_eq!(v.content, expected(Profile::Basic));
    let kinds: Vec<_> = v.slot_infos().iter().map(|s| s.kind).collect();
    assert_eq!(kinds, [SlotKind::Password, SlotKind::Recovery]);

    let code = std::fs::read_to_string(fixtures().join("basic.recovery.txt")).unwrap();
    let r = locked
        .unlock_with_recovery(&RecoveryCode::parse(code.trim()).unwrap())
        .unwrap();
    assert_eq!(r.content, expected(Profile::Basic));
}

#[test]
fn hardened_fixture_needs_password_and_keyfile() {
    let locked = LockedVault::parse(&read("hardened.heim")).unwrap();
    assert!(locked.requires_keyfile());
    let keyfile = Keyfile::parse(&read("hardened.bifrost")).unwrap();
    assert!(locked.accepts_keyfile(&keyfile));

    assert!(locked.unlock_with_password(&pw(HARDENED_PASSWORD), None).is_err());
    let v = locked
        .unlock_with_password(&pw(HARDENED_PASSWORD), Some(&keyfile))
        .unwrap();
    assert!(matches!(v.opened_with(), UnlockMethod::PasswordKeyfile { .. }));

    let usb = v
        .slot_infos()
        .into_iter()
        .find(|s| matches!(s.kind, SlotKind::PasswordKeyfile { .. }))
        .expect("password + keyfile slot");
    assert_eq!(usb.kdf, FAST);
    let mut content = v.content.clone();
    let label = content.label(usb.id).expect("access label").clone();
    assert_eq!((label.name.as_str(), label.added), (USB_NAME, T0));
    content.access.clear();
    assert_eq!(content, expected(Profile::Hardened));

    let code = std::fs::read_to_string(fixtures().join("hardened.recovery.txt")).unwrap();
    let r = locked
        .unlock_with_recovery(&RecoveryCode::parse(code.trim()).unwrap())
        .unwrap();
    assert_eq!(r.opened_with(), UnlockMethod::Recovery);
}

/// Everything content version 5 adds on top of [`expected`].
fn expected_v5() -> Content {
    let mut c = expected(Profile::Basic);
    c.security.ask_password_export = false;
    c.last_backup = Some(T0 + 200);
    c
}

#[test]
fn basic_v5_fixture_opens_with_password_and_recovery() {
    let locked = LockedVault::parse(&read("basic-v5.heim")).unwrap();
    assert!(!locked.requires_keyfile());
    let v = locked.unlock_with_password(&pw(BASIC_PASSWORD), None).unwrap();
    assert_eq!(v.content, expected_v5());

    let code = std::fs::read_to_string(fixtures().join("basic-v5.recovery.txt")).unwrap();
    let r = locked
        .unlock_with_recovery(&RecoveryCode::parse(code.trim()).unwrap())
        .unwrap();
    assert_eq!(r.content, expected_v5());
}

/// Writes the content version 5 fixture (v1.0.0). Reference only.
#[test]
#[ignore = "writes a committed fixture; never run it to fix a failure"]
fn write_fixture_v5() {
    let dir = fixtures();
    let mut basic = Vault::create_with_kdf(&pw(BASIC_PASSWORD), Profile::Basic, FAST).unwrap();
    basic.vault.content = expected_v5();
    std::fs::write(dir.join("basic-v5.heim"), basic.vault.to_bytes().unwrap()).unwrap();
    std::fs::write(
        dir.join("basic-v5.recovery.txt"),
        format!("{}\n", *basic.recovery_code.phrase()),
    )
    .unwrap();
}

/// Writes the fixtures. Kept for reference only; see the module comment.
#[test]
#[ignore = "writes the committed fixtures; never run it to fix a failure"]
fn write_fixtures() {
    let dir = fixtures();
    std::fs::create_dir_all(&dir).unwrap();

    let mut basic = Vault::create_with_kdf(&pw(BASIC_PASSWORD), Profile::Basic, FAST).unwrap();
    basic.vault.content = expected(Profile::Basic);
    std::fs::write(dir.join("basic.heim"), basic.vault.to_bytes().unwrap()).unwrap();
    std::fs::write(
        dir.join("basic.recovery.txt"),
        format!("{}\n", *basic.recovery_code.phrase()),
    )
    .unwrap();

    let mut hardened = Vault::create_with_kdf(&pw(HARDENED_PASSWORD), Profile::Hardened, FAST).unwrap();
    let keyfile = hardened.keyfile.take().unwrap();
    let usb = hardened
        .vault
        .slot_infos()
        .into_iter()
        .find(|s| matches!(s.kind, SlotKind::PasswordKeyfile { .. }))
        .unwrap();
    hardened.vault.content = expected(Profile::Hardened);
    hardened.vault.content.set_label(usb.id, USB_NAME, T0);
    std::fs::write(dir.join("hardened.heim"), hardened.vault.to_bytes().unwrap()).unwrap();
    std::fs::write(dir.join("hardened.bifrost"), &*keyfile.to_bytes()).unwrap();
    std::fs::write(
        dir.join("hardened.recovery.txt"),
        format!("{}\n", *hardened.recovery_code.phrase()),
    )
    .unwrap();
}
