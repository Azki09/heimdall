//! Atomic save and the `.bak` copy (DESIGN §5.4). Only ever writes
//! ciphertext: callers pass the bytes from [`crate::Vault::to_bytes`].

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Refuse to read anything bigger than this; a vault of notes is tiny.
pub const MAX_VAULT_FILE: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveKind {
    /// Ordinary save: the previous version becomes the `.bak`.
    Normal,
    /// Password, USB keys, MK or recovery code changed: the `.bak` is
    /// overwritten with the new version, or it would still open with the
    /// old access (DESIGN §5.4).
    AccessChanged,
}

pub fn read_vault(path: &Path) -> Result<Vec<u8>> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_VAULT_FILE {
        return Err(Error::Malformed);
    }
    let mut buf = Vec::new();
    file.take(MAX_VAULT_FILE + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > MAX_VAULT_FILE {
        return Err(Error::Malformed);
    }
    Ok(buf)
}

pub fn backup_path(path: &Path) -> PathBuf {
    with_suffix(path, ".bak")
}

/// Writes `bytes` to `path` so that `path` always holds either the old or
/// the new complete version: write to `.tmp`, flush + fsync, rename over.
pub fn save_vault(path: &Path, bytes: &[u8], kind: SaveKind) -> Result<()> {
    let bak = backup_path(path);
    match kind {
        SaveKind::Normal => {
            if path.exists() {
                // Copy (not rename) so `path` is never missing, even if we crash here.
                let old = fs::read(path)?;
                write_atomic(&bak, &old)?;
            }
            write_atomic(path, bytes)?;
        }
        SaveKind::AccessChanged => {
            write_atomic(path, bytes)?;
            write_atomic(&bak, bytes)?;
        }
    }
    Ok(())
}

/// Writes a whole file so that `path` never holds half of it: `.tmp`,
/// flush + fsync, rename over. For new files that are not the open vault
/// (exports, backup copies, DESIGN §10.9); the vault itself goes through
/// [`save_vault`].
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = with_suffix(path, ".tmp");
    let result = (|| {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.flush()?;
        f.sync_all()?;
        drop(f);
        // On Windows this is MoveFileExW with MOVEFILE_REPLACE_EXISTING.
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    Ok(result?)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_save_keeps_previous_as_bak() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("notes.heim");

        save_vault(&p, b"v1", SaveKind::Normal).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"v1");
        assert!(!backup_path(&p).exists());

        save_vault(&p, b"v2", SaveKind::Normal).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"v2");
        assert_eq!(fs::read(backup_path(&p)).unwrap(), b"v1");
        assert!(!with_suffix(&p, ".tmp").exists());
    }

    #[test]
    fn access_change_overwrites_bak() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("notes.heim");
        save_vault(&p, b"old-access", SaveKind::Normal).unwrap();
        save_vault(&p, b"still-old", SaveKind::Normal).unwrap();
        save_vault(&p, b"new-access", SaveKind::AccessChanged).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"new-access");
        assert_eq!(fs::read(backup_path(&p)).unwrap(), b"new-access");
    }

    #[test]
    fn read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("notes.heim");
        save_vault(&p, b"data", SaveKind::Normal).unwrap();
        assert_eq!(read_vault(&p).unwrap(), b"data");
    }
}
