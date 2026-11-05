//! Heimdall core: cryptography, the `.heim` file format, key slots, Master
//! Key rotation, recovery codes and the note model. No UI and no OS-specific
//! code, so everything here is unit-testable and fuzzable.
//!
//! The design lives in `docs/DESIGN.md`; section numbers in comments (§4.3,
//! §5.2, ...) refer to it.

pub mod content;
pub mod crypto;
pub mod error;
pub mod format;
pub mod keyfile;
pub mod recovery;
pub mod secret;
pub mod storage;
pub mod vault;

pub use content::{Content, Note, Profile};
pub use crypto::KdfParams;
pub use error::{Error, Result};
pub use format::{SlotId, SlotKind};
pub use keyfile::Keyfile;
pub use recovery::RecoveryCode;
pub use secrecy::SecretString;
pub use vault::{LockedVault, NewVault, SlotInfo, UnlockMethod, Vault};

/// Entry points for the fuzz targets in `fuzz/` that reach code the app
/// only runs after decryption. Not part of the API.
#[cfg(feature = "fuzzing")]
#[doc(hidden)]
pub mod fuzzing {
    use crate::{Content, Result};

    /// Decodes decrypted content (padding, version, every legacy layout).
    pub fn content_from_plaintext(plain: &[u8]) -> Result<Content> {
        Content::from_padded_plaintext(plain)
    }

    /// Encodes content the way it is encrypted.
    pub fn content_to_plaintext(content: &Content) -> Result<Vec<u8>> {
        content.to_padded_plaintext().map(|p| p.to_vec())
    }
}
