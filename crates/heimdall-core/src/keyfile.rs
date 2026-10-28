//! The USB key file, `<name>.bifrost` (DESIGN §8.2).
//!
//! Layout: `magic (4) || vault_id (16) || secret (32)`. The secret is a real
//! second factor: it is mixed into the slot key with HKDF, so copying the
//! file alone opens nothing without the password.

use secrecy::ExposeSecret;
use zeroize::Zeroizing;

use crate::crypto::{self, Key, KEY_LEN};
use crate::error::{Error, Result};
use crate::format::{VaultId, HINT_LEN};

pub const KEYFILE_MAGIC: [u8; 4] = *b"HMKF";
pub const KEYFILE_LEN: usize = 4 + 16 + KEY_LEN;
pub const KEYFILE_EXTENSION: &str = "bifrost";

pub struct Keyfile {
    vault_id: VaultId,
    secret: Key,
}

impl Keyfile {
    pub fn generate(vault_id: VaultId) -> Result<Self> {
        Ok(Self {
            vault_id,
            secret: crypto::random_key()?,
        })
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != KEYFILE_LEN || bytes[..4] != KEYFILE_MAGIC {
            return Err(Error::BadKeyfile);
        }
        let vault_id: VaultId = bytes[4..20].try_into().expect("length checked");
        let mut secret = Zeroizing::new([0u8; KEY_LEN]);
        secret.copy_from_slice(&bytes[20..]);
        Ok(Self {
            vault_id,
            secret: crypto::new_key(*secret),
        })
    }

    /// The bytes to write to the USB drive. Wiped when dropped.
    pub fn to_bytes(&self) -> Zeroizing<Vec<u8>> {
        let mut out = Zeroizing::new(Vec::with_capacity(KEYFILE_LEN));
        out.extend_from_slice(&KEYFILE_MAGIC);
        out.extend_from_slice(&self.vault_id);
        out.extend_from_slice(self.secret.expose_secret());
        out
    }

    pub fn vault_id(&self) -> &VaultId {
        &self.vault_id
    }

    pub(crate) fn secret(&self) -> &Key {
        &self.secret
    }

    /// First bytes of SHA-256(secret), stored in the slot (DESIGN §5.2).
    pub fn hint(&self) -> [u8; HINT_LEN] {
        let digest = Zeroizing::new(crypto::sha256(self.secret.expose_secret()));
        digest[..HINT_LEN].try_into().expect("digest is 32 bytes")
    }
}

impl std::fmt::Debug for Keyfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keyfile")
            .field("hint", &self.hint())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let k = Keyfile::generate([4; 16]).unwrap();
        let bytes = k.to_bytes();
        assert_eq!(bytes.len(), KEYFILE_LEN);
        let back = Keyfile::parse(&bytes).unwrap();
        assert_eq!(back.vault_id(), &[4; 16]);
        assert_eq!(back.hint(), k.hint());
    }

    #[test]
    fn rejects_garbage() {
        assert!(Keyfile::parse(b"nope").is_err());
        let mut bytes = Keyfile::generate([0; 16]).unwrap().to_bytes().to_vec();
        bytes[0] = b'X';
        assert!(Keyfile::parse(&bytes).is_err());
    }

    #[test]
    fn debug_does_not_leak_secret() {
        let k = Keyfile::generate([0; 16]).unwrap();
        let dbg = format!("{k:?}");
        assert!(!dbg.contains("secret"));
    }
}
