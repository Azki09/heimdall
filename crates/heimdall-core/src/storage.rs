//! Atomic save and the `.bak` copy (DESIGN §5.4). Only ever writes
//! ciphertext: callers pass the bytes from [`crate::Vault::to_bytes`].

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

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
/// the new complete version: write to a temporary file next to it, flush +
/// fsync, rename over.
///
/// A vault marked read-only is never replaced. Windows refuses the rename
/// by itself; on Linux a rename only needs the folder to be writable, so
/// the flag is checked here, before the `.bak` is touched.
pub fn save_vault(path: &Path, bytes: &[u8], kind: SaveKind) -> Result<()> {
    if fs::metadata(path).is_ok_and(|m| m.permissions().readonly()) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the vault file is read-only").into());
    }
    let bak = backup_path(path);
    match kind {
        SaveKind::Normal => {
            if path.exists() {
                // Copy (not rename) so `path` is never missing, even if we crash here.
                let old = retry(|| fs::read(path))?;
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

/// Writes a whole file so that `path` never holds half of it: `<name>.<random>.tmp`,
/// flush + fsync, rename over. For new files that are not the open vault
/// (exports, backup copies, DESIGN §10.9); the vault itself goes through
/// [`save_vault`].
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    // A fresh, random name opened with `create_new`: an existing file (or a
    // link planted under a predictable name) is never truncated or followed.
    let mut tag = [0u8; 6];
    getrandom::fill(&mut tag).map_err(|_| Error::Rng)?;
    let tag: String = tag.iter().map(|b| format!("{b:02x}")).collect();
    let tmp = with_suffix(path, &format!(".{tag}.tmp"));
    let result = (|| {
        let mut f = retry(|| File::options().write(true).create_new(true).open(&tmp))?;
        f.write_all(bytes)?;
        f.flush()?;
        f.sync_all()?;
        drop(f);
        // On Windows this is MoveFileExW with MOVEFILE_REPLACE_EXISTING.
        retry(|| fs::rename(&tmp, path))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    Ok(result?)
}

/// Runs a file operation again for a moment while another program holds the
/// file: OneDrive, an antivirus or the search indexer often open a file
/// that was just written, without letting it be replaced. About 0.8 s in
/// all, then the error goes to the caller (and the status bar).
fn retry<T>(mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut wait = Duration::from_millis(25);
    for _ in 0..5 {
        match op() {
            Err(e) if is_transient(&e) => {
                std::thread::sleep(wait);
                wait *= 2;
            }
            other => return other,
        }
    }
    op()
}

/// ERROR_ACCESS_DENIED (also what replacing a file that is open elsewhere
/// gives), ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
#[cfg(windows)]
fn is_transient(e: &io::Error) -> bool {
    matches!(e.raw_os_error(), Some(5 | 32 | 33))
}

#[cfg(not(windows))]
fn is_transient(_: &io::Error) -> bool {
    false
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_tmp_left(dir: &Path) -> bool {
        fs::read_dir(dir)
            .unwrap()
            .all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp"))
    }

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
        assert!(no_tmp_left(dir.path()));
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

    /// Opens `path` the way an antivirus scan might: no sharing at all.
    #[cfg(windows)]
    fn hold(path: &Path) -> File {
        use std::os::windows::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(path)
            .unwrap()
    }

    #[cfg(windows)]
    #[test]
    fn briefly_locked_file_is_saved_after_a_retry() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("notes.heim");
        save_vault(&p, b"v1", SaveKind::Normal).unwrap();
        let h = hold(&p);
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(h);
        });
        save_vault(&p, b"v2", SaveKind::Normal).unwrap();
        release.join().unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"v2");
        assert_eq!(fs::read(backup_path(&p)).unwrap(), b"v1");
    }

    #[cfg(windows)]
    #[test]
    fn locked_file_fails_without_losing_anything() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("notes.heim");
        save_vault(&p, b"v1", SaveKind::Normal).unwrap();
        save_vault(&p, b"v2", SaveKind::Normal).unwrap();
        let h = hold(&p);
        let err = save_vault(&p, b"v3", SaveKind::Normal).unwrap_err();
        drop(h);
        assert!(
            matches!(err, Error::Io(ref e) if e.raw_os_error() == Some(32)),
            "{err:?}"
        );
        assert_eq!(fs::read(&p).unwrap(), b"v2");
        assert_eq!(fs::read(backup_path(&p)).unwrap(), b"v1");
        assert!(no_tmp_left(dir.path()));
    }

    #[test]
    fn read_only_vault_fails_without_losing_anything() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("notes.heim");
        save_vault(&p, b"v1", SaveKind::Normal).unwrap();
        let mut perm = fs::metadata(&p).unwrap().permissions();
        perm.set_readonly(true);
        fs::set_permissions(&p, perm.clone()).unwrap();
        let result = save_vault(&p, b"v2", SaveKind::Normal);
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        fs::set_permissions(&p, perm).unwrap();
        assert!(result.is_err());
        assert_eq!(fs::read(&p).unwrap(), b"v1");
        assert!(no_tmp_left(dir.path()));
    }

    #[test]
    fn missing_folder_fails_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("gone").join("notes.heim");
        assert!(matches!(
            save_vault(&p, b"v1", SaveKind::Normal),
            Err(Error::Io(_))
        ));
    }

    #[test]
    fn read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("notes.heim");
        save_vault(&p, b"data", SaveKind::Normal).unwrap();
        assert_eq!(read_vault(&p).unwrap(), b"data");
    }
}
