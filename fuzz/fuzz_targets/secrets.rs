//! Masked secrets `{*value*}` and the paste detector, on any text: byte
//! offsets must land on char boundaries, and Unmask must leave no secret.
#![no_main]

use heimdall_core::secret;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let scan = secret::scan(text);
    let mut end = 0;
    for r in &scan.secrets {
        assert!(r.start >= end, "secrets in order, apart");
        let s = &text[r.clone()]; // panics off a char boundary
        assert!(s.starts_with(secret::OPEN) && s.ends_with(secret::CLOSE));
        assert!(!text[secret::value(r)].is_empty());
        assert!(!s.contains('\n'));
        end = r.end;
    }
    for &e in &scan.escapes {
        assert!(text[e..].starts_with(r"\{*"));
    }

    let _ = secret::mask(text);
    assert_eq!(secret::contains_secret(text), !scan.secrets.is_empty());
    assert!(
        secret::scan(&secret::escape_all(text)).secrets.is_empty(),
        "Unmask leaves no secret: {text:?}"
    );

    // Unmask one secret: exactly that one goes, the others stay.
    if let Some(r) = scan.secrets.first() {
        let (after, end) = secret::unmask(text, r.clone());
        assert!(after.is_char_boundary(end));
        assert_eq!(
            secret::scan(&after).secrets.len(),
            scan.secrets.len() - 1,
            "Unmask removes exactly one secret: {text:?} -> {after:?}"
        );
    }

    let _ = secret::looks_like_secret(text, false);
    let _ = secret::looks_like_secret(text, true);
    if let Some(wrapped) = secret::wrap(text) {
        assert_eq!(secret::scan(&wrapped).secrets, vec![0..wrapped.len()]);
    }
});
