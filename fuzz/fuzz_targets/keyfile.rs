//! A `.bifrost` key file read from a USB drive.
#![no_main]

use heimdall_core::Keyfile;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(key) = Keyfile::parse(data) {
        assert_eq!(&key.to_bytes()[..], data, "key file round trip");
        let _ = key.hint();
    }
});
