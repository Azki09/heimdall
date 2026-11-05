//! The recovery code as typed by the user.
#![no_main]

use heimdall_core::RecoveryCode;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &str| {
    if let Ok(code) = RecoveryCode::parse(input) {
        assert_eq!(code.words().len(), 12);
        let again = RecoveryCode::parse(&code.phrase()).expect("own phrase parses");
        assert_eq!(again.words(), code.words());
    }
});
