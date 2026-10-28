//! Recovery code: 128 random bits shown as 12 BIP39 English words (DESIGN §7).
//!
//! The slot key is derived from the raw 16-byte entropy, not from the text,
//! so spacing or capitalization never matters.

use bip39::{Language, Mnemonic};
use zeroize::Zeroizing;

use crate::crypto;
use crate::error::{Error, Result};

pub const ENTROPY_LEN: usize = 16;
pub const WORD_COUNT: usize = 12;

pub struct RecoveryCode {
    entropy: Zeroizing<[u8; ENTROPY_LEN]>,
}

impl RecoveryCode {
    pub fn generate() -> Result<Self> {
        Ok(Self {
            entropy: Zeroizing::new(crypto::random_bytes()?),
        })
    }

    /// Parses what the user typed. Case, extra spaces, commas and line
    /// breaks are ignored; the BIP39 checksum catches most typos.
    pub fn parse(input: &str) -> Result<Self> {
        let normalized: Zeroizing<String> = Zeroizing::new(
            input
                .split(|c: char| c.is_whitespace() || c == ',')
                .filter(|w| !w.is_empty())
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
                .join(" "),
        );
        let mnemonic = Mnemonic::parse_in_normalized(Language::English, &normalized)
            .map_err(|_| Error::InvalidRecoveryCode)?;
        let (bytes, len) = mnemonic.to_entropy_array();
        let bytes = Zeroizing::new(bytes);
        if len != ENTROPY_LEN {
            return Err(Error::InvalidRecoveryCode);
        }
        let mut entropy = Zeroizing::new([0u8; ENTROPY_LEN]);
        entropy.copy_from_slice(&bytes[..ENTROPY_LEN]);
        Ok(Self { entropy })
    }

    fn mnemonic(&self) -> Mnemonic {
        Mnemonic::from_entropy_in(Language::English, self.entropy.as_ref())
            .expect("16 bytes is valid BIP39 entropy")
    }

    /// The 12 words, in order, for display and printing.
    pub fn words(&self) -> Vec<&'static str> {
        self.mnemonic().words().collect()
    }

    /// The words joined by single spaces. Wiped when dropped.
    pub fn phrase(&self) -> Zeroizing<String> {
        Zeroizing::new(self.words().join(" "))
    }

    /// For the "confirm 3 random words" step (DESIGN §7). `index` is 0-based.
    pub fn check_word(&self, index: usize, word: &str) -> bool {
        self.words()
            .get(index)
            .is_some_and(|w| w.eq_ignore_ascii_case(word.trim()))
    }

    pub(crate) fn secret(&self) -> &[u8] {
        self.entropy.as_ref()
    }
}

impl std::fmt::Debug for RecoveryCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecoveryCode(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twelve_words_roundtrip() {
        let code = RecoveryCode::generate().unwrap();
        let words = code.words();
        assert_eq!(words.len(), WORD_COUNT);
        let back = RecoveryCode::parse(&code.phrase()).unwrap();
        assert_eq!(back.secret(), code.secret());
    }

    #[test]
    fn parse_is_forgiving_about_formatting() {
        let code = RecoveryCode::generate().unwrap();
        let messy = format!("  {}\n", code.words().join(" ,  ").to_uppercase());
        assert_eq!(RecoveryCode::parse(&messy).unwrap().secret(), code.secret());
    }

    #[test]
    fn known_vector() {
        // BIP39 test vector: 16 zero bytes.
        let code = RecoveryCode::parse(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        )
        .unwrap();
        assert_eq!(code.secret(), &[0u8; 16]);
    }

    #[test]
    fn rejects_bad_checksum_and_wrong_length() {
        let bad_checksum =
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon";
        assert!(matches!(
            RecoveryCode::parse(bad_checksum),
            Err(Error::InvalidRecoveryCode)
        ));
        // 24 valid words = 256 bits: valid BIP39, but not a Heimdall code.
        let long = "abandon ".repeat(23) + "art";
        assert!(matches!(
            RecoveryCode::parse(&long),
            Err(Error::InvalidRecoveryCode)
        ));
        assert!(RecoveryCode::parse("not a recovery code").is_err());
    }

    #[test]
    fn check_word() {
        let code = RecoveryCode::generate().unwrap();
        let w = code.words()[4];
        assert!(code.check_word(4, &w.to_uppercase()));
        assert!(!code.check_word(4, "zzz"));
        assert!(!code.check_word(99, w));
    }
}
