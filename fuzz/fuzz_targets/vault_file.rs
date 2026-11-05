//! A `.heim` file from disk: anything at all must be rejected cleanly, and
//! what is accepted must write back byte for byte (the header bytes are the
//! content's AAD, so a lossy round trip would lock the user out).
#![no_main]

use heimdall_core::format::VaultFile;
use heimdall_core::LockedVault;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(file) = VaultFile::parse(data) {
        assert_eq!(file.to_bytes(), data, "file round trip");
        let header = file.header.to_bytes().expect("a parsed header serializes");
        assert_eq!(header, file.header_bytes, "header round trip");
        let _ = file.header.requires_keyfile();
        let _ = file.header.has_unknown_slots();
    }
    if let Ok(locked) = LockedVault::parse(data) {
        let _ = locked.requires_keyfile();
        let _ = locked.has_keyfile_slots();
        let _ = locked.has_unsupported_slots();
    }
});
