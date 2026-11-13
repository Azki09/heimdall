//! Finding, unlocking, watching and saving the vault from the terminal
//! (DESIGN §17.1). The same rules as the app: the exclusive lock, the USB
//! key when the vault requires it, the profile's idle lock, and only ever
//! ciphertext on disk.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use heimdall_core::storage::{read_vault, save_vault, SaveKind};
use heimdall_core::{Error, LockedVault, Profile, SecretString, Vault};

use super::paths::{self, expand_home};
use super::prompt::{self, Input};
use crate::attempts::Attempts;
use crate::support::{duration_label, error_message, save_error_message, RecentEntry, RecentVaults};

pub use super::paths::short_path;
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

/// Where the vault's path came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `--vault`
    Option,
    /// `$HEIMDALL_VAULT`
    Env,
    /// The app's recent list or the terminal's own.
    Remembered,
    /// Typed at the question.
    Asked,
}

/// `--vault`, else `$HEIMDALL_VAULT`.
pub fn given(arg: Option<PathBuf>) -> Option<(PathBuf, Source)> {
    match arg {
        Some(p) => Some((expand_home(&p), Source::Option)),
        None => std::env::var_os("HEIMDALL_VAULT")
            .filter(|v| !v.is_empty())
            .map(|p| (expand_home(Path::new(&p)), Source::Env)),
    }
}

/// The vaults this computer remembers, newest first: the app's recent
/// list and the terminal's own (DESIGN §17.1). Only files that exist.
pub fn remembered() -> Vec<RecentEntry> {
    let mut all = RecentVaults::load().entries;
    all.extend(RecentVaults::terminal().entries);
    all.sort_by_key(|e| std::cmp::Reverse(e.opened));
    let mut seen = Vec::new();
    all.retain(|e| {
        let new = e.path.is_file() && !seen.contains(&e.path);
        seen.push(e.path.clone());
        new
    });
    all
}

/// The vault to use: `--vault`, then `$HEIMDALL_VAULT`, then the most
/// recent one remembered, then asking. Hardened and Paranoid vaults are
/// remembered only if the user asked for it in the terminal.
pub fn locate(arg: Option<PathBuf>) -> Result<(PathBuf, Source), Fail> {
    if let Some(given) = given(arg) {
        return check(given);
    }
    if let Some(entry) = remembered().into_iter().next() {
        return Ok((entry.path, Source::Remembered));
    }
    eprintln!(
        "[heimdall] No vault opened recently. Which one? (--vault <path> or $HEIMDALL_VAULT skip this)"
    );
    ask_path()
}

/// ` :: Vault file : `, with Tab completing folders and `.heim` files.
pub fn ask_path() -> Result<(PathBuf, Source), Fail> {
    let complete = |typed: &str| -> Vec<String> {
        paths::complete(typed.trim_start(), |p| p.extension().is_some_and(|e| e == "heim"))
    };
    let complete: prompt::Complete = &complete;
    match prompt::read_line(" :: Vault file : ", false, Some(complete), &mut || {
        super::hung_up().then_some(LockEvent::HungUp)
    })? {
        Input::Line(line) if !line.trim().is_empty() => {
            check((expand_home(Path::new(line.trim())), Source::Asked))
        }
        _ => Err(Fail::Cancelled),
    }
}

fn check((path, source): (PathBuf, Source)) -> Result<(PathBuf, Source), Fail> {
    if !path.is_file() {
        return Err(Fail::Message(format!("There is no vault at {}.", path.display())));
    }
    Ok((path, source))
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

        // Too many wrong passwords: no question until the wait is over
        // (DESIGN §9.1).
        let attempts = Attempts::for_vault(locked.vault_id());
        let status = attempts.status();
        if let Some((first, second)) = status.locked_message(&name) {
            return Err(Fail::Message(format!("{first}\n           {second}")));
        }

        // The questions are wiped once the vault opens (DESIGN §17.3).
        prompt::forget_rows();
        if let Some(warning) = status.warning(&name) {
            prompt::say(&format!("[heimdall] {warning}"));
        }
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
                Err(Error::WrongPassword) => {
                    let status = attempts.failed();
                    prompt::say(&format!("[heimdall] {}", status.after_wrong(&name)));
                    if status.is_locked() {
                        prompt::forget_rows();
                        return Err(Fail::Cancelled);
                    }
                }
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
        attempts.succeeded(&vault.content.security);

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
        let mut terminal = RecentVaults::terminal();
        if terminal.contains(&path) {
            terminal.touch(&path);
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
