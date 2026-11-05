//! Decrypted content: padding, version and every legacy layout. Only a
//! vault's own key reaches this, but a bug here would still crash the app
//! on a damaged file. What decodes must encode and decode to the same.
#![no_main]

use heimdall_core::fuzzing::{content_from_plaintext, content_to_plaintext};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The real plaintext is padded to 4 KiB blocks; pad the input so the
    // fuzzer gets past that check instead of guessing lengths.
    let mut plain = data.to_vec();
    plain.resize(plain.len().div_ceil(4096).max(1) * 4096, 0);
    for input in [data, &plain[..]] {
        if let Ok(content) = content_from_plaintext(input) {
            let again = content_to_plaintext(&content).expect("decoded content encodes");
            let back = content_from_plaintext(&again).expect("encoded content decodes");
            assert_eq!(back, content, "content round trip");
        }
    }
});
