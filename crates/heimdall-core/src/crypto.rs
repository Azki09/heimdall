//! Thin wrappers over audited primitives (DESIGN §4.2). No custom crypto.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{AeadInOut, KeyInit};
use chacha20poly1305::{Tag, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use secrecy::{ExposeSecret, SecretBox};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::error::{Error, Result};

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;
pub const SALT_LEN: usize = 16;

/// HKDF `info` for the password + keyfile slot (DESIGN §4.2).
const HKDF_INFO_KEYFILE: &[u8] = b"slot:password+keyfile:v1";

/// A 32-byte symmetric key that is wiped from memory on drop.
pub type Key = SecretBox<[u8; KEY_LEN]>;

pub fn new_key(bytes: [u8; KEY_LEN]) -> Key {
    SecretBox::new(Box::new(bytes))
}

pub fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).map_err(|_| Error::Rng)?;
    Ok(buf)
}

pub fn random_key() -> Result<Key> {
    let mut k = Zeroizing::new([0u8; KEY_LEN]);
    getrandom::fill(k.as_mut()).map_err(|_| Error::Rng)?;
    Ok(new_key(*k))
}

/// Argon2id cost parameters, stored per slot (DESIGN §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    /// Memory in KiB.
    pub m_kib: u32,
    pub t: u32,
    pub p: u8,
}

impl KdfParams {
    /// Basic and Hardened profiles: 256 MiB, t = 3, p = 4.
    pub const STANDARD: Self = Self {
        m_kib: 256 * 1024,
        t: 3,
        p: 4,
    };
    /// Paranoid profile: 512 MiB, t = 3, p = 4.
    pub const PARANOID: Self = Self {
        m_kib: 512 * 1024,
        t: 3,
        p: 4,
    };
    /// Recovery slot: the code already has 128 bits of entropy (DESIGN §4.2).
    pub const RECOVERY: Self = Self {
        m_kib: 64 * 1024,
        t: 2,
        p: 4,
    };

    /// Hard limits enforced on every parsed slot, so a malicious file cannot
    /// make us allocate absurd amounts of memory (DESIGN §5.2).
    pub const MIN_M_KIB: u32 = 8 * 1024;
    pub const MAX_M_KIB: u32 = 2 * 1024 * 1024;
    pub const MAX_T: u32 = 16;
    pub const MAX_P: u8 = 16;

    pub fn validate(&self) -> Result<()> {
        let ok = (Self::MIN_M_KIB..=Self::MAX_M_KIB).contains(&self.m_kib)
            && (1..=Self::MAX_T).contains(&self.t)
            && (1..=Self::MAX_P).contains(&self.p);
        if ok {
            Ok(())
        } else {
            Err(Error::KdfParamsOutOfRange)
        }
    }
}

/// `K = Argon2id(secret, salt)` with a 32-byte output.
pub fn argon2id(secret: &[u8], salt: &[u8; SALT_LEN], params: KdfParams) -> Result<Key> {
    params.validate()?;
    let p = Params::new(params.m_kib, params.t, params.p as u32, Some(KEY_LEN))
        .map_err(|_| Error::KdfParamsOutOfRange)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, p);
    let mut out = Zeroizing::new([0u8; KEY_LEN]);
    argon
        .hash_password_into(secret, salt, out.as_mut())
        .map_err(|_| Error::Crypto)?;
    Ok(new_key(*out))
}

/// `K = HKDF-SHA256(ikm = P || keyfile_secret, salt = vault_id, info)` (DESIGN §4.3).
pub fn combine_password_keyfile(p: &Key, keyfile_secret: &Key, vault_id: &[u8; 16]) -> Result<Key> {
    let mut ikm = Zeroizing::new([0u8; 2 * KEY_LEN]);
    ikm[..KEY_LEN].copy_from_slice(p.expose_secret());
    ikm[KEY_LEN..].copy_from_slice(keyfile_secret.expose_secret());
    let hk = Hkdf::<Sha256>::new(Some(vault_id), ikm.as_ref());
    let mut okm = Zeroizing::new([0u8; KEY_LEN]);
    hk.expand(HKDF_INFO_KEYFILE, okm.as_mut())
        .map_err(|_| Error::Crypto)?;
    Ok(new_key(*okm))
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

fn cipher(key: &Key) -> XChaCha20Poly1305 {
    // `new_from_slice` reads the key in place; building a `Key` array first
    // would leave an unwiped copy on the stack.
    XChaCha20Poly1305::new_from_slice(key.expose_secret()).expect("keys are 32 bytes")
}

/// Encrypts a 32-byte key (the MK) under `kek`. Returns `ciphertext || tag`.
pub fn wrap_key(
    kek: &Key,
    nonce: &[u8; NONCE_LEN],
    key: &Key,
    aad: &[u8],
) -> Result<[u8; KEY_LEN + TAG_LEN]> {
    let mut buf = Zeroizing::new(*key.expose_secret());
    let tag = cipher(kek)
        .encrypt_inout_detached(&XNonce::from(*nonce), aad, buf.as_mut_slice().into())
        .map_err(|_| Error::Crypto)?;
    let mut out = [0u8; KEY_LEN + TAG_LEN];
    out[..KEY_LEN].copy_from_slice(buf.as_ref());
    out[KEY_LEN..].copy_from_slice(&tag);
    Ok(out)
}

/// Inverse of [`wrap_key`]. Returns `None` if authentication fails
/// (wrong key or tampered slot); callers map that to a specific error.
pub fn unwrap_key(
    kek: &Key,
    nonce: &[u8; NONCE_LEN],
    wrapped: &[u8; KEY_LEN + TAG_LEN],
    aad: &[u8],
) -> Option<Key> {
    let mut buf = Zeroizing::new([0u8; KEY_LEN]);
    buf.copy_from_slice(&wrapped[..KEY_LEN]);
    let tag = Tag::try_from(&wrapped[KEY_LEN..]).ok()?;
    cipher(kek)
        .decrypt_inout_detached(&XNonce::from(*nonce), aad, buf.as_mut_slice().into(), &tag)
        .ok()?;
    Some(new_key(*buf))
}

/// Encrypts `buf` in place and appends the tag.
pub fn seal_in_place(key: &Key, nonce: &[u8; NONCE_LEN], aad: &[u8], buf: &mut Vec<u8>) -> Result<()> {
    cipher(key)
        .encrypt_in_place(&XNonce::from(*nonce), aad, buf)
        .map_err(|_| Error::Crypto)
}

/// Decrypts `buf` (ciphertext || tag) in place. On failure `buf` is left
/// unspecified and must be discarded.
pub fn open_in_place(key: &Key, nonce: &[u8; NONCE_LEN], aad: &[u8], buf: &mut Vec<u8>) -> Result<()> {
    cipher(key)
        .decrypt_in_place(&XNonce::from(*nonce), aad, buf)
        .map_err(|_| Error::ContentAuthFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAST: KdfParams = KdfParams {
        m_kib: KdfParams::MIN_M_KIB,
        t: 1,
        p: 1,
    };

    #[test]
    fn argon2id_is_deterministic_and_salted() {
        let salt_a = [1u8; SALT_LEN];
        let salt_b = [2u8; SALT_LEN];
        let a1 = argon2id(b"pw", &salt_a, FAST).unwrap();
        let a2 = argon2id(b"pw", &salt_a, FAST).unwrap();
        let b = argon2id(b"pw", &salt_b, FAST).unwrap();
        assert_eq!(a1.expose_secret(), a2.expose_secret());
        assert_ne!(a1.expose_secret(), b.expose_secret());
    }

    #[test]
    fn kdf_limits_are_enforced() {
        let salt = [0u8; SALT_LEN];
        let huge = KdfParams {
            m_kib: KdfParams::MAX_M_KIB + 1,
            t: 1,
            p: 1,
        };
        assert!(matches!(
            argon2id(b"pw", &salt, huge),
            Err(Error::KdfParamsOutOfRange)
        ));
        let zero_t = KdfParams { t: 0, ..FAST };
        assert!(zero_t.validate().is_err());
        KdfParams::STANDARD.validate().unwrap();
        KdfParams::PARANOID.validate().unwrap();
        KdfParams::RECOVERY.validate().unwrap();
    }

    #[test]
    fn wrap_roundtrip_and_aad_binding() {
        let kek = new_key([7u8; KEY_LEN]);
        let mk = new_key([9u8; KEY_LEN]);
        let nonce = [3u8; NONCE_LEN];
        let wrapped = wrap_key(&kek, &nonce, &mk, b"aad").unwrap();
        assert_ne!(&wrapped[..KEY_LEN], mk.expose_secret());
        let back = unwrap_key(&kek, &nonce, &wrapped, b"aad").unwrap();
        assert_eq!(back.expose_secret(), mk.expose_secret());
        assert!(unwrap_key(&kek, &nonce, &wrapped, b"other").is_none());
        assert!(unwrap_key(&new_key([8u8; KEY_LEN]), &nonce, &wrapped, b"aad").is_none());
    }

    #[test]
    fn hkdf_depends_on_every_input() {
        let p = new_key([1u8; KEY_LEN]);
        let s = new_key([2u8; KEY_LEN]);
        let base = combine_password_keyfile(&p, &s, &[0u8; 16]).unwrap();
        let other_vault = combine_password_keyfile(&p, &s, &[1u8; 16]).unwrap();
        let other_secret = combine_password_keyfile(&p, &new_key([3u8; KEY_LEN]), &[0u8; 16]).unwrap();
        assert_ne!(base.expose_secret(), other_vault.expose_secret());
        assert_ne!(base.expose_secret(), other_secret.expose_secret());
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    /// Fixed outputs of every primitive, recorded with the crate versions of
    /// v0.5 (argon2 0.5, hkdf 0.12, chacha20poly1305 0.10, sha2 0.10). A
    /// dependency update must never change them: existing vaults would stop
    /// opening. SHA-256("abc") is the FIPS 180-2 vector.
    #[test]
    fn known_answers() {
        let a = argon2id(b"password", &[7u8; SALT_LEN], FAST).unwrap();
        assert_eq!(
            hex(a.expose_secret()),
            "f7ff8309c5b730e9290f02b7fe1da817f037631ac327391b38e70a43ef5cdc17"
        );
        let k = combine_password_keyfile(&new_key([1; 32]), &new_key([2; 32]), &[3; 16]).unwrap();
        assert_eq!(
            hex(k.expose_secret()),
            "c09dd30c46a78ec0acfd7fb5104feac7349e4b7febdbfd60d44b720b254db49b"
        );
        let w = wrap_key(&new_key([4; 32]), &[5; NONCE_LEN], &new_key([6; 32]), b"aad").unwrap();
        assert_eq!(
            hex(&w),
            "92051937c28b302c8f814feac3835b84892d6722e3fb51e6b571a252fd46a93c\
             77324cdba0533a2a85f40d066ac51ad0"
        );
        let mut buf = b"hello heimdall".to_vec();
        seal_in_place(&new_key([8; 32]), &[9; NONCE_LEN], b"hdr", &mut buf).unwrap();
        assert_eq!(
            hex(&buf),
            "a30309a21679173bfd1369eb954d13f11c3177e68c7ba8775c777dba8924"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
