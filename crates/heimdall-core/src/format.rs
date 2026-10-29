//! Binary layout of `.heim` files (DESIGN §5.1, §5.2). All integers are
//! little-endian. This module only parses and serializes; it never decrypts.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::crypto::{KdfParams, KEY_LEN, NONCE_LEN, SALT_LEN, TAG_LEN};
use crate::error::{Error, Result};

pub const MAGIC: [u8; 4] = *b"HMDL";
pub const FORMAT_VERSION: u16 = 1;
pub const MAX_SLOTS: usize = 16;

pub const SLOT_TYPE_PASSWORD: u8 = 1;
pub const SLOT_TYPE_RECOVERY: u8 = 2;
pub const SLOT_TYPE_PASSWORD_KEYFILE: u8 = 3;

pub const WRAPPED_LEN: usize = KEY_LEN + TAG_LEN;
pub const HINT_LEN: usize = 8;

/// Body length (everything after `slot_len`) of slot types 1 and 2.
const SLOT_BODY_LEN: usize = 8 + 4 + 4 + 1 + SALT_LEN + NONCE_LEN + WRAPPED_LEN;
/// Body length of slot type 3 (adds `keyfile_hint`).
const SLOT_BODY_LEN_KEYFILE: usize = SLOT_BODY_LEN + HINT_LEN;

pub type VaultId = [u8; 16];

/// Serializable so the encrypted content can name each access (DESIGN §5.3).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SlotId(pub [u8; 8]);

impl fmt::Display for SlotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for SlotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SlotId({self})")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    Password,
    Recovery,
    /// `hint` = first 8 bytes of SHA-256(keyfile secret), only used to tell
    /// "wrong USB" apart from "wrong password".
    PasswordKeyfile {
        hint: [u8; HINT_LEN],
    },
}

impl SlotKind {
    pub fn type_byte(&self) -> u8 {
        match self {
            SlotKind::Password => SLOT_TYPE_PASSWORD,
            SlotKind::Recovery => SLOT_TYPE_RECOVERY,
            SlotKind::PasswordKeyfile { .. } => SLOT_TYPE_PASSWORD_KEYFILE,
        }
    }

    /// Whether this slot is opened with the vault password.
    pub fn uses_password(&self) -> bool {
        !matches!(self, SlotKind::Recovery)
    }
}

/// A slot of a type this version understands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownSlot {
    pub kind: SlotKind,
    pub id: SlotId,
    pub kdf: KdfParams,
    pub salt: [u8; SALT_LEN],
    pub nonce: [u8; NONCE_LEN],
    pub wrapped_mk: [u8; WRAPPED_LEN],
}

impl KnownSlot {
    /// `slot_aad = magic || version || vault_id || slot_type || slot_id
    ///             || kdf_m || kdf_t || kdf_p || salt || keyfile_hint?` (DESIGN §4.3)
    pub fn aad(&self, vault_id: &VaultId) -> Vec<u8> {
        let mut a = Vec::with_capacity(64);
        a.extend_from_slice(&MAGIC);
        a.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        a.extend_from_slice(vault_id);
        a.push(self.kind.type_byte());
        a.extend_from_slice(&self.id.0);
        a.extend_from_slice(&self.kdf.m_kib.to_le_bytes());
        a.extend_from_slice(&self.kdf.t.to_le_bytes());
        a.push(self.kdf.p);
        a.extend_from_slice(&self.salt);
        if let SlotKind::PasswordKeyfile { hint } = &self.kind {
            a.extend_from_slice(hint);
        }
        a
    }
}

/// A slot as stored in the header. Unknown types (e.g. a future YubiKey
/// slot) are kept verbatim: they are skipped for unlocking but still part of
/// the content AAD (DESIGN §5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    Known(KnownSlot),
    Unknown { slot_type: u8, body: Vec<u8> },
}

/// The plaintext header: only what is needed to open the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub vault_id: VaultId,
    pub slots: Vec<Slot>,
}

impl Header {
    pub fn known_slots(&self) -> impl Iterator<Item = &KnownSlot> {
        self.slots.iter().filter_map(|s| match s {
            Slot::Known(k) => Some(k),
            Slot::Unknown { .. } => None,
        })
    }

    pub fn has_unknown_slots(&self) -> bool {
        self.slots.iter().any(|s| matches!(s, Slot::Unknown { .. }))
    }

    /// A vault requires its USB key if it has keyfile slots and no
    /// password-only slot (DESIGN §5.1).
    pub fn requires_keyfile(&self) -> bool {
        let mut has_keyfile = false;
        for s in self.known_slots() {
            match s.kind {
                SlotKind::Password => return false,
                SlotKind::PasswordKeyfile { .. } => has_keyfile = true,
                SlotKind::Recovery => {}
            }
        }
        has_keyfile
    }

    /// Serializes `magic` through the last slot. These exact bytes are the
    /// AAD of the content (DESIGN §4.4).
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        if self.slots.len() > MAX_SLOTS {
            return Err(Error::TooManySlots(self.slots.len()));
        }
        let mut out = Vec::with_capacity(32 + self.slots.len() * 120);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&self.vault_id);
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.push(self.slots.len() as u8);
        for slot in &self.slots {
            match slot {
                Slot::Known(k) => {
                    let body_len = match k.kind {
                        SlotKind::PasswordKeyfile { .. } => SLOT_BODY_LEN_KEYFILE,
                        _ => SLOT_BODY_LEN,
                    };
                    out.push(k.kind.type_byte());
                    out.extend_from_slice(&(body_len as u16).to_le_bytes());
                    out.extend_from_slice(&k.id.0);
                    out.extend_from_slice(&k.kdf.m_kib.to_le_bytes());
                    out.extend_from_slice(&k.kdf.t.to_le_bytes());
                    out.push(k.kdf.p);
                    out.extend_from_slice(&k.salt);
                    out.extend_from_slice(&k.nonce);
                    out.extend_from_slice(&k.wrapped_mk);
                    if let SlotKind::PasswordKeyfile { hint } = &k.kind {
                        out.extend_from_slice(hint);
                    }
                }
                Slot::Unknown { slot_type, body } => {
                    let len = u16::try_from(body.len()).map_err(|_| Error::Malformed)?;
                    out.push(*slot_type);
                    out.extend_from_slice(&len.to_le_bytes());
                    out.extend_from_slice(body);
                }
            }
        }
        Ok(out)
    }
}

/// A whole `.heim` file, still encrypted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultFile {
    pub header: Header,
    /// Exact header bytes as read from disk, used as the content AAD.
    pub header_bytes: Vec<u8>,
    pub content_nonce: [u8; NONCE_LEN],
    /// Encrypted content including the 16-byte tag.
    pub content: Vec<u8>,
}

impl VaultFile {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        if r.take(4)? != MAGIC {
            return Err(Error::BadMagic);
        }
        let version = r.u16()?;
        if version != FORMAT_VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        let vault_id: VaultId = r.array()?;
        if r.u16()? != 0 {
            return Err(Error::ReservedFlags);
        }
        let slot_count = r.u8()? as usize;
        if slot_count > MAX_SLOTS {
            return Err(Error::TooManySlots(slot_count));
        }
        let mut slots = Vec::with_capacity(slot_count);
        for _ in 0..slot_count {
            slots.push(parse_slot(&mut r)?);
        }
        let header_bytes = bytes[..r.pos].to_vec();

        let content_nonce: [u8; NONCE_LEN] = r.array()?;
        let content_len = r.u64()?;
        if content_len < TAG_LEN as u64 || content_len != r.remaining() as u64 {
            return Err(Error::Malformed);
        }
        let content = r.take(content_len as usize)?.to_vec();

        Ok(Self {
            header: Header { vault_id, slots },
            header_bytes,
            content_nonce,
            content,
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.header_bytes.len() + NONCE_LEN + 8 + self.content.len());
        out.extend_from_slice(&self.header_bytes);
        out.extend_from_slice(&self.content_nonce);
        out.extend_from_slice(&(self.content.len() as u64).to_le_bytes());
        out.extend_from_slice(&self.content);
        out
    }
}

fn parse_slot(r: &mut Reader<'_>) -> Result<Slot> {
    let slot_type = r.u8()?;
    let len = r.u16()? as usize;
    let expected = match slot_type {
        SLOT_TYPE_PASSWORD | SLOT_TYPE_RECOVERY => SLOT_BODY_LEN,
        SLOT_TYPE_PASSWORD_KEYFILE => SLOT_BODY_LEN_KEYFILE,
        _ => {
            let body = r.take(len)?.to_vec();
            return Ok(Slot::Unknown { slot_type, body });
        }
    };
    if len != expected {
        return Err(Error::BadSlotLength);
    }
    let id = SlotId(r.array()?);
    let kdf = KdfParams {
        m_kib: r.u32()?,
        t: r.u32()?,
        p: r.u8()?,
    };
    kdf.validate()?;
    let salt = r.array()?;
    let nonce = r.array()?;
    let wrapped_mk = r.array()?;
    let kind = match slot_type {
        SLOT_TYPE_PASSWORD => SlotKind::Password,
        SLOT_TYPE_RECOVERY => SlotKind::Recovery,
        _ => SlotKind::PasswordKeyfile { hint: r.array()? },
    };
    Ok(Slot::Known(KnownSlot {
        kind,
        id,
        kdf,
        salt,
        nonce,
        wrapped_mk,
    }))
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.remaining() {
            return Err(Error::Malformed);
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.take(N)?.try_into().expect("take returned N bytes"))
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(kind: SlotKind, n: u8) -> Slot {
        Slot::Known(KnownSlot {
            kind,
            id: SlotId([n; 8]),
            kdf: KdfParams::STANDARD,
            salt: [n; SALT_LEN],
            nonce: [n; NONCE_LEN],
            wrapped_mk: [n; WRAPPED_LEN],
        })
    }

    fn sample() -> VaultFile {
        let header = Header {
            vault_id: [0xAB; 16],
            slots: vec![
                slot(SlotKind::Password, 1),
                slot(SlotKind::Recovery, 2),
                slot(SlotKind::PasswordKeyfile { hint: [9; HINT_LEN] }, 3),
                Slot::Unknown {
                    slot_type: 42,
                    body: vec![1, 2, 3],
                },
            ],
        };
        let header_bytes = header.to_bytes().unwrap();
        VaultFile {
            header,
            header_bytes,
            content_nonce: [5; NONCE_LEN],
            content: vec![7; 4096 + TAG_LEN],
        }
    }

    #[test]
    fn roundtrip() {
        let f = sample();
        let bytes = f.to_bytes();
        assert_eq!(&bytes[..4], b"HMDL");
        let parsed = VaultFile::parse(&bytes).unwrap();
        assert_eq!(parsed, f);
        assert!(parsed.header.has_unknown_slots());
    }

    #[test]
    fn slot_sizes_match_design() {
        // type(1) + slot_len(2) + body
        assert_eq!(3 + SLOT_BODY_LEN, 108);
        assert_eq!(3 + SLOT_BODY_LEN_KEYFILE, 116);
        // magic + version + vault_id + flags + slot_count
        let empty = Header {
            vault_id: [0; 16],
            slots: vec![],
        };
        assert_eq!(empty.to_bytes().unwrap().len(), 4 + 2 + 16 + 2 + 1);
    }

    #[test]
    fn rejects_bad_magic_version_flags() {
        let good = sample().to_bytes();

        let mut b = good.clone();
        b[0] = b'X';
        assert!(matches!(VaultFile::parse(&b), Err(Error::BadMagic)));

        let mut b = good.clone();
        b[4] = 2;
        assert!(matches!(VaultFile::parse(&b), Err(Error::UnsupportedVersion(2))));

        let mut b = good.clone();
        b[22] = 1;
        assert!(matches!(VaultFile::parse(&b), Err(Error::ReservedFlags)));
    }

    #[test]
    fn rejects_every_truncation() {
        let good = sample().to_bytes();
        for len in 0..good.len() {
            assert!(
                VaultFile::parse(&good[..len]).is_err(),
                "accepted truncation at {len}"
            );
        }
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut b = sample().to_bytes();
        b.push(0);
        assert!(matches!(VaultFile::parse(&b), Err(Error::Malformed)));
    }

    #[test]
    fn rejects_wrong_slot_len_for_known_type() {
        let mut b = sample().to_bytes();
        // First slot starts right after the 25-byte fixed header; its slot_len follows the type byte.
        b[26] = b[26].wrapping_add(1);
        assert!(matches!(VaultFile::parse(&b), Err(Error::BadSlotLength)));
    }

    #[test]
    fn rejects_too_many_slots() {
        let mut b = sample().to_bytes();
        b[24] = (MAX_SLOTS + 1) as u8;
        assert!(matches!(VaultFile::parse(&b), Err(Error::TooManySlots(_))));
    }

    #[test]
    fn rejects_absurd_kdf_memory() {
        let mut f = sample();
        if let Slot::Known(k) = &mut f.header.slots[0] {
            k.kdf.m_kib = 64 * 1024 * 1024; // 64 GiB
        }
        f.header_bytes = f.header.to_bytes().unwrap();
        assert!(matches!(
            VaultFile::parse(&f.to_bytes()),
            Err(Error::KdfParamsOutOfRange)
        ));
    }

    #[test]
    fn requires_keyfile_rule() {
        let pw = slot(SlotKind::Password, 1);
        let rec = slot(SlotKind::Recovery, 2);
        let kf = slot(SlotKind::PasswordKeyfile { hint: [0; HINT_LEN] }, 3);
        let h = |slots: Vec<Slot>| Header {
            vault_id: [0; 16],
            slots,
        };
        assert!(!h(vec![pw.clone(), rec.clone()]).requires_keyfile());
        assert!(h(vec![kf.clone(), rec.clone()]).requires_keyfile());
        assert!(!h(vec![pw, kf, rec]).requires_keyfile()); // downgraded to Basic (§6.2)
    }
}
