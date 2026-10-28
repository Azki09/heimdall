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
