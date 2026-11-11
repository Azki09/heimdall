//! Finding, unlocking, watching and saving the vault from the terminal
//! (DESIGN §17.1). The same rules as the app: the exclusive lock, the USB
//! key when the vault requires it, the profile's idle lock, and only ever
//! ciphertext on disk.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use heimdall_core::storage::{read_vault, save_vault, SaveKind};
use heimdall_core::{Error, LockedVault, Profile, SecretString, Vault};

use super::prompt::{self, Input};
use crate::support::{duration_label, error_message, save_error_message, RecentVaults};
use crate::usb::{self, Presence, UsbKey};
use crate::vault_lock::{LockError, VaultLock};

/// Why a command stopped. `Cancelled` prints nothing more.
pub enum Fail {
    Message(String),
    Cancelled,
}

impl From<std::io::Error> for Fail {
    fn from(e: std::io::Error) -> Self {
        Fail::Message(format!("The terminal can't be used: {e}"))
    }
}

/// Something that locks the vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockEvent {
    Idle(u32),
    /// `^L` in the editor.
    Manual,
    UsbRemoved,
    SessionLocked,
    Suspended,
    /// The computer is shutting down.
    Ending,
    /// The terminal window was closed, or the process was asked to end.
    HungUp,
}

impl LockEvent {
    /// These lock even if saving fails; the others keep the note open so
    /// no work is lost (the same rule as the app, DESIGN §9 step 3).
    pub fn must_lock(self) -> bool {
        matches!(self, Self::UsbRemoved | Self::Ending | Self::HungUp)
    }

    pub fn reason(self) -> String {
        match self {
            Self::Idle(s) => format!("idle for {}", duration_label(s)),
            Self::Manual => "locked with ^L".into(),
            Self::UsbRemoved => "USB key removed".into(),
            Self::SessionLocked => "the session was locked".into(),
            Self::Suspended => "the computer went to sleep".into(),
            Self::Ending => "the computer is shutting down".into(),
            Self::HungUp => "the terminal was closed".into(),
        }
    }
}

pub struct TermSession {
    pub vault: Vault,
    pub path: PathBuf,
    presence: Option<Presence>,
    /// The last key press, for the idle lock.
    pub last_input: Instant,
    _lock: Option<VaultLock>,
}

/// `personal` for `~/vaults/personal.heim`.
pub fn vault_name(path: &Path) -> String {
    path.file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The vault to use: `--vault`, then `$HEIMDALL_VAULT`, then the most
/// recent one the app remembers, then asking. Hardened and Paranoid vaults
/// are not remembered (DESIGN §6.1), so those need one of the first two.
pub fn locate(arg: Option<PathBuf>) -> Result<PathBuf, Fail> {
    let chosen = arg
        .or_else(|| {
            std::env::var_os("HEIMDALL_VAULT")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
        .map(|p| expand_home(&p))
        .or_else(|| {
            RecentVaults::load()
                .entries
                .into_iter()
                .map(|e| e.path)
                .find(|p| p.is_file())
        });
    let path = match chosen {
        Some(p) => p,
        None => {
            eprintln!("[heimdall] No vault opened recently. Which one? (--vault <path> or $HEIMDALL_VAULT skip this)");
            match prompt::read_line(" :: Vault file : ", false, None, &mut || None)? {
                Input::Line(line) if !line.trim().is_empty() => expand_home(Path::new(line.trim())),
                _ => return Err(Fail::Cancelled),
            }
        }
    };
    if !path.is_file() {
        return Err(Fail::Message(format!("There is no vault at {}.", path.display())));
    }
    Ok(path)
}

fn expand_home(path: &Path) -> PathBuf {
    match (path.strip_prefix("~"), std::env::var_os("HOME")) {
        (Ok(rest), Some(home)) => Path::new(&home).join(rest),
        _ => path.to_owned(),
    }
}

/// `~/vaults/personal.heim`
pub fn short_path(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home.and_then(|h| path.strip_prefix(h).ok().map(Path::to_owned)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

const TRIES: usize = 3;

impl TermSession {
    /// Takes the exclusive lock, waits for the USB key if the vault needs
    /// one, and asks for the password (three tries, like `sudo`).
    pub fn unlock(path: PathBuf) -> Result<Self, Fail> {
        let name = vault_name(&path);
        let locked = read_vault(&path)
            .and_then(|b| LockedVault::parse(&b))
            .map_err(|e| Fail::Message(error_message(&e)))?;
        let lock = match VaultLock::acquire(&path) {
            Ok(lock) => lock,
            Err(LockError::Busy) => {
                return Err(Fail::Message(format!(
                    "Vault \"{name}\" is open in the Heimdall window or in another terminal.\n           Lock or close it there, then try again."
                )))
            }
        };

        // The questions are wiped once the vault opens (DESIGN §17.3).
        prompt::forget_rows();
        let key = Self::find_key(&locked, &name)?;
        let mut vault = None;
        for _ in 0..TRIES {
            let label = format!("[heimdall] Password for vault \"{name}\": ");
            let mut line = match prompt::read_line(&label, true, None, &mut || {
                super::hung_up().then_some(LockEvent::HungUp)
            })? {
                Input::Line(line) if !line.is_empty() => line,
                _ => {
                    prompt::forget_rows();
                    return Err(Fail::Cancelled);
                }
            };
            let password = SecretString::from(std::mem::take(&mut *line));
            match locked.unlock_with_password(&password, key.as_ref().map(|k| &k.keyfile)) {
                Ok(v) => {
                    vault = Some(v);
                    break;
                }
                Err(Error::WrongPassword) => prompt::say("[heimdall] Wrong password."),
                Err(e) => {
                    prompt::forget_rows();
                    return Err(Fail::Message(error_message(&e)));
                }
            }
        }
        let Some(mut vault) = vault else {
            prompt::forget_rows();
            return Err(Fail::Cancelled);
        };
        prompt::erase_rows();

        let (address, len) = vault.key_memory();
        heimdall_platform::lock_memory(address, len);
        vault.content.purge_expired_trash(heimdall_core::content::now());
        // Vaults from before titles were unique (DESIGN §10.1).
        let renamed = vault.content.dedupe_titles();
        let mut recent = RecentVaults::load();
        if vault.content.security.remember_recent {
            recent.touch(&path);
        } else {
            recent.remove(&path);
        }
        // Presence mode: only while the vault can't be opened without the key (§8.3).
        let presence = key
            .as_ref()
            .filter(|_| vault.requires_keyfile())
            .map(|k| Presence::start(k, || {}));
        heimdall_platform::watch_session(0, || {});
        let mut session = Self {
            vault,
            path,
            presence,
            last_input: Instant::now(),
            _lock: lock,
        };
        if renamed > 0 {
            // If this fails, the next save writes the new titles.
            let _ = session.save();
        }
        Ok(session)
    }

    /// This vault's key among the connected USB drives. Waits for it if the
    /// vault can't be opened without one.
    fn find_key(locked: &LockedVault, name: &str) -> Result<Option<UsbKey>, Fail> {
        if !locked.has_keyfile_slots() {
            return Ok(None);
        }
        let look = || {
            usb::find_keys(locked.vault_id())
                .into_iter()
                .find(|k| locked.accepts_keyfile(&k.keyfile))
        };
        if !locked.requires_keyfile() {
            return Ok(look());
        }
        let waiting = format!("[heimdall] Insert the USB key for vault \"{name}\"... ");
        eprint!("{waiting}");
        loop {
            if let Some(key) = look() {
                prompt::say("found.");
                return Ok(Some(key));
            }
            if super::hung_up() || prompt::cancelled_within(Duration::from_secs(1)) {
                eprintln!();
                return Err(Fail::Cancelled);
            }
        }
    }

    pub fn name(&self) -> String {
        vault_name(&self.path)
    }

    /// A key was pressed: the idle lock starts counting again.
    pub fn touch(&mut self) {
        self.last_input = Instant::now();
    }

    /// Whether the vault must lock now, and why.
    pub fn lock_event(&mut self) -> Option<LockEvent> {
        if super::hung_up() {
            return Some(LockEvent::HungUp);
        }
        if self.presence.as_ref().is_some_and(Presence::removed) {
            return Some(LockEvent::UsbRemoved);
        }
        let security = &self.vault.content.security;
        match heimdall_platform::take_session_event() {
            Some(heimdall_platform::SessionEvent::Ending) => return Some(LockEvent::Ending),
            Some(heimdall_platform::SessionEvent::Suspending) => return Some(LockEvent::Suspended),
            Some(heimdall_platform::SessionEvent::Locked)
                if security.profile != Profile::Basic || security.lock_on_windows_lock =>
            {
                return Some(LockEvent::SessionLocked)
            }
            _ => {}
        }
        let secs = security.lock_after_s?;
        (self.last_input.elapsed() >= Duration::from_secs(u64::from(secs))).then_some(LockEvent::Idle(secs))
    }

    /// Seconds until the idle lock, if the vault has one.
    pub fn idle_left(&self) -> Option<u64> {
        let secs = u64::from(self.vault.content.security.lock_after_s?);
        Some(secs.saturating_sub(self.last_input.elapsed().as_secs()))
    }

    /// Encrypts and writes the whole vault (atomic, previous version kept
    /// as `.bak`), exactly like the app.
    pub fn save(&mut self) -> Result<(), String> {
        self.vault
            .to_bytes()
            .and_then(|bytes| save_vault(&self.path, &bytes, SaveKind::Normal))
            .map_err(|e| save_error_message(&e))
    }
}

impl Drop for TermSession {
    fn drop(&mut self) {
        heimdall_platform::set_unsaved(false);
    }
}
