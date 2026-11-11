//! One open copy of a vault at a time (DESIGN §17.1): the app and the
//! terminal mode take an exclusive OS lock (`flock` on Linux, `LockFileEx`
//! on Windows) before opening a vault, and hold it while it is unlocked.
//!
//! The lock is a small empty file named after a hash of the vault's path,
//! kept away from the vault: `$XDG_RUNTIME_DIR/heimdall/` on Linux (a
//! per-user tmpfs, emptied at logout), the config folder elsewhere. Nothing
//! is ever written next to the vault, and the OS drops the lock if the
//! process dies, so a lock can't be left behind.

use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// What the app and the terminal say when the vault is open elsewhere.
pub const BUSY: &str =
    "This vault is open in another Heimdall window or in a terminal. Lock or close it there first.";

pub struct VaultLock {
    _file: File,
}

pub enum LockError {
    /// Another Heimdall process has this vault open.
    Busy,
}

impl VaultLock {
    /// Locks `vault`. If the lock file can't be made at all (no config
    /// folder, read-only home), opening goes on without one: refusing to
    /// open the vault would be worse than the protection it gives.
    pub fn acquire(vault: &Path) -> Result<Option<Self>, LockError> {
        let Some(path) = lock_path(vault) else {
            return Ok(None);
        };
        if let Some(dir) = path.parent() {
            if create_private_dir(dir).is_err() {
                return Ok(None);
            }
        }
        let Ok(file) = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
        else {
            return Ok(None);
        };
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(TryLockError::WouldBlock) => Err(LockError::Busy),
            Err(TryLockError::Error(_)) => Ok(None),
        }
    }

    /// Whether another process holds the lock right now. For a quick check
    /// before the slow unlock; the real lock is taken with [`Self::acquire`].
    pub fn is_busy(vault: &Path) -> bool {
        matches!(Self::acquire(vault), Err(LockError::Busy))
    }
}

fn lock_dir() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    if let Some(run) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| Path::new(d).is_absolute()) {
        return Some(Path::new(&run).join("heimdall"));
    }
    crate::support::config_dir().map(|d| d.join("locks"))
}

/// `<dir>/<first 16 bytes of SHA-256(canonical path), hex>.lock`. The same
/// vault reached through a link or a relative path gets the same name.
fn lock_path(vault: &Path) -> Option<PathBuf> {
    let canonical = std::fs::canonicalize(vault)
        .or_else(|_| std::path::absolute(vault))
        .ok()?;
    let digest = Sha256::digest(canonical.as_os_str().as_encoded_bytes());
    let name: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    Some(lock_dir()?.join(format!("{name}.lock")))
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_vault_same_lock() {
        let dir = std::env::temp_dir().join(format!("heimdall-lock-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let vault = dir.join("v.heim");
        std::fs::write(&vault, b"x").unwrap();
        let a = lock_path(&vault).unwrap();
        let b = lock_path(&dir.join(".").join("v.heim")).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, lock_path(&dir.join("other.heim")).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Two handles in one process behave like two processes: the second
    /// one is refused until the first is dropped.
    #[test]
    fn second_open_is_refused() {
        let dir = std::env::temp_dir().join(format!("heimdall-lock-test2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let vault = dir.join("v.heim");
        std::fs::write(&vault, b"x").unwrap();
        let Ok(Some(first)) = VaultLock::acquire(&vault) else {
            // No config folder in this environment: nothing to test.
            return;
        };
        assert!(VaultLock::is_busy(&vault));
        drop(first);
        assert!(!VaultLock::is_busy(&vault));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
