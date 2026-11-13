//! The wrong password limit (DESIGN §9.1). After `max_tries` wrong
//! passwords in a row the vault waits `lockout_s` before the next try, and
//! each wait in a row is twice as long as the one before, up to a day.
//!
//! This only slows down guessing on this computer, with Heimdall. Someone
//! with a copy of the `.heim` file can try passwords with their own program;
//! against that, Argon2id and a strong password are what count.
//!
//! The wait is measured with [`heimdall_platform::boot_clock_ms`], which
//! changing the date, time or time zone doesn't move. After a restart that
//! clock starts again from 0, and so does the wait.
//!
//! The counts live in `attempts.txt` next to the recent list, one line per
//! vault, keyed by a hash of the vault's id, never its name or path. The
//! limit itself is in the encrypted settings, which can't be read before
//! unlocking, so each successful unlock copies it here.

use std::path::PathBuf;

use heimdall_core::content::SecuritySettings;
use heimdall_core::format::VaultId;
use sha2::{Digest, Sha256};

use crate::support::{config_dir, duration_label};

const FILE: &str = "attempts.txt";
/// The longest wait.
const MAX_WAIT_S: u64 = 24 * 60 * 60;

/// How many wrong passwords and how long to wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// `None` = no limit.
    pub max_tries: Option<u8>,
    pub lockout_s: u32,
}

impl Policy {
    /// For a vault never opened on this computer: Basic's defaults.
    const UNKNOWN: Self = Self {
        max_tries: Some(5),
        lockout_s: 5 * 60,
    };

    pub fn of(security: &SecuritySettings) -> Self {
        Self {
            max_tries: security.max_tries.filter(|&n| n > 0),
            lockout_s: security.lockout_s.max(1),
        }
    }
}

/// Where a vault stands before the next password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Open {
        /// `None` = no limit.
        tries_left: Option<u32>,
        /// What running out of tries costs.
        next_wait_s: u64,
        /// Wrong passwords since the last right one.
        failed: u32,
    },
    Locked {
        left_s: u64,
        failed: u32,
        /// The computer restarted during the wait, which started again.
        restarted: bool,
    },
}

impl Status {
    pub fn is_locked(&self) -> bool {
        matches!(self, Self::Locked { .. })
    }

    /// What to say after a wrong password.
    pub fn after_wrong(&self, vault: &str) -> String {
        match *self {
            Self::Locked { left_s, .. } => format!(
                "Wrong password. Vault \"{vault}\" is locked for {}.",
                clock(left_s)
            ),
            Self::Open {
                tries_left: Some(1),
                next_wait_s,
                ..
            } => format!(
                "Wrong password. Last try before a {} wait.",
                wait_label(next_wait_s)
            ),
            Self::Open {
                tries_left: Some(n), ..
            } => format!("Wrong password. {n} tries left."),
            Self::Open { tries_left: None, .. } => "Wrong password.".to_owned(),
        }
    }

    /// Said before the first password, if something went wrong before:
    /// `2 tries left before vault "personal" locks for 5 min.`
    pub fn warning(&self, vault: &str) -> Option<String> {
        match *self {
            Self::Open {
                tries_left: Some(n),
                next_wait_s,
                failed,
            } if failed > 0 => Some(format!(
                "{n} {} left before vault \"{vault}\" locks for {}.",
                if n == 1 { "try" } else { "tries" },
                wait_label(next_wait_s)
            )),
            _ => None,
        }
    }

    /// Why no password is asked: two lines.
    pub fn locked_message(&self, vault: &str) -> Option<(String, String)> {
        match *self {
            Self::Locked {
                left_s,
                failed,
                restarted,
            } => {
                let first = format!("Vault \"{vault}\" is locked after {failed} wrong passwords.");
                let second = if restarted {
                    format!(
                        "The computer restarted, so the wait started again: {}.",
                        clock(left_s)
                    )
                } else {
                    format!("Try again in {}.", clock(left_s))
                };
                Some((first, second))
            }
            Self::Open { .. } => None,
        }
    }
}

/// `4:12`, `1:05:00`
pub fn clock(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn wait_label(secs: u64) -> String {
    duration_label(u32::try_from(secs).unwrap_or(u32::MAX))
}

/// One vault's line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    key: String,
    /// Wrong passwords since the last wait (or the last right one).
    failures: u32,
    /// Wrong passwords since the last right one, for the message.
    total: u32,
    /// Waits in a row: the next one is twice as long.
    lockouts: u32,
    /// Boot clock when the current wait started.
    locked_at: Option<u64>,
    wait_s: u64,
    policy: Policy,
}

impl Entry {
    fn new(key: String) -> Self {
        Self {
            key,
            failures: 0,
            total: 0,
            lockouts: 0,
            locked_at: None,
            wait_s: 0,
            policy: Policy::UNKNOWN,
        }
    }

    fn next_wait_s(&self) -> u64 {
        let base = u64::from(self.policy.lockout_s);
        base.saturating_mul(1u64 << self.lockouts.min(20)).min(MAX_WAIT_S)
    }

    /// Ends a wait that is over and restarts one the computer restarted
    /// in. Returns the status and whether the entry changed.
    fn check(&mut self, now_ms: u64) -> (Status, bool) {
        let mut changed = false;
        if let Some(at) = self.locked_at {
            let mut restarted = false;
            let at = if now_ms < at {
                // The boot clock went back: a restart. Wait again from now.
                self.locked_at = Some(now_ms);
                changed = true;
                restarted = true;
                now_ms
            } else {
                at
            };
            let waited = (now_ms - at) / 1000;
            if waited < self.wait_s {
                let status = Status::Locked {
                    left_s: self.wait_s - waited,
                    failed: self.total,
                    restarted,
                };
                return (status, changed);
            }
            // Over: one try, then a longer wait.
            self.locked_at = None;
            self.failures = u32::from(self.policy.max_tries.unwrap_or(1)).saturating_sub(1);
            changed = true;
        }
        let tries_left = self
            .policy
            .max_tries
            .map(|max| u32::from(max).saturating_sub(self.failures).max(1));
        let status = Status::Open {
            tries_left,
            next_wait_s: self.next_wait_s(),
            failed: self.total,
        };
        (status, changed)
    }

    fn failed(&mut self, now_ms: u64) -> Status {
        if let (status @ Status::Locked { .. }, _) = self.check(now_ms) {
            return status;
        }
        self.failures += 1;
        self.total += 1;
        if let Some(max) = self.policy.max_tries {
            if self.failures >= u32::from(max) {
                self.wait_s = self.next_wait_s();
                self.lockouts += 1;
                self.locked_at = Some(now_ms);
                self.failures = 0;
            }
        }
        self.check(now_ms).0
    }

    fn is_blank(&self) -> bool {
        self.failures == 0 && self.total == 0 && self.lockouts == 0 && self.locked_at.is_none()
    }

    /// `key failures total lockouts locked_at wait_s max_tries lockout_s`,
    /// with `-` for no wait and 0 for no limit.
    fn line(&self) -> String {
        let at = self.locked_at.map_or("-".to_owned(), |t| t.to_string());
        format!(
            "{} {} {} {} {at} {} {} {}",
            self.key,
            self.failures,
            self.total,
            self.lockouts,
            self.wait_s,
            self.policy.max_tries.unwrap_or(0),
            self.policy.lockout_s
        )
    }

    fn parse(line: &str) -> Option<Self> {
        let f: Vec<&str> = line.split_whitespace().collect();
        let [key, failures, total, lockouts, at, wait, max, lockout] = f[..] else {
            return None;
        };
        let max: u8 = max.parse().ok()?;
        Some(Self {
            key: key.to_owned(),
            failures: failures.parse().ok()?,
            total: total.parse().ok()?,
            lockouts: lockouts.parse().ok()?,
            locked_at: match at {
                "-" => None,
                t => Some(t.parse().ok()?),
            },
            wait_s: wait.parse().ok()?,
            policy: Policy {
                max_tries: (max > 0).then_some(max),
                lockout_s: lockout.parse::<u32>().ok()?.max(1),
            },
        })
    }
}

/// One vault's wrong passwords on this computer.
pub struct Attempts {
    file: Option<PathBuf>,
    key: String,
}

impl Attempts {
    pub fn for_vault(id: &VaultId) -> Self {
        let mut hash = Sha256::new();
        hash.update(b"heimdall attempts v1");
        hash.update(id);
        let digest = hash.finalize();
        let key = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
        Self {
            file: config_dir().map(|d| d.join(FILE)),
            key,
        }
    }

    fn load(&self) -> Vec<Entry> {
        self.file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|text| text.lines().filter_map(Entry::parse).collect())
            .unwrap_or_default()
    }

    fn store(&self, entries: &[Entry]) {
        let Some(file) = &self.file else { return };
        if entries.is_empty() {
            let _ = std::fs::remove_file(file);
            return;
        }
        let text: String = entries.iter().map(|e| e.line() + "\n").collect();
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = file.with_extension("tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, file);
        }
    }

    /// Runs `f` on this vault's entry and writes the file if it changed.
    fn with_entry<T>(&self, f: impl FnOnce(&mut Entry) -> T) -> T {
        let mut entries = self.load();
        let i = match entries.iter().position(|e| e.key == self.key) {
            Some(i) => i,
            None => {
                entries.push(Entry::new(self.key.clone()));
                entries.len() - 1
            }
        };
        let before = entries[i].clone();
        let result = f(&mut entries[i]);
        if entries[i] != before {
            // A vault with nothing to remember leaves no line behind.
            if entries[i].is_blank() && entries[i].policy == Policy::UNKNOWN {
                entries.remove(i);
            }
            self.store(&entries);
        }
        result
    }

    pub fn status(&self) -> Status {
        let now = heimdall_platform::boot_clock_ms();
        self.with_entry(|e| e.check(now).0)
    }

    /// A wrong password: the status after it.
    pub fn failed(&self) -> Status {
        let now = heimdall_platform::boot_clock_ms();
        self.with_entry(|e| e.failed(now))
    }

    /// The right password (or the recovery code): the count starts again,
    /// and the vault's limit is kept for the next time.
    pub fn succeeded(&self, security: &SecuritySettings) {
        let policy = Policy::of(security);
        self.with_entry(|e| {
            *e = Entry::new(e.key.clone());
            e.policy = policy;
        });
    }

    /// The limit changed in Settings.
    pub fn set_policy(&self, security: &SecuritySettings) {
        let policy = Policy::of(security);
        self.with_entry(|e| e.policy = policy);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(max: Option<u8>, lockout_s: u32) -> Entry {
        let mut e = Entry::new("k".into());
        e.policy = Policy {
            max_tries: max,
            lockout_s,
        };
        e
    }

    fn left(s: Status) -> u64 {
        match s {
            Status::Locked { left_s, .. } => left_s,
            Status::Open { .. } => panic!("open: {s:?}"),
        }
    }

    fn tries(s: Status) -> Option<u32> {
        match s {
            Status::Open { tries_left, .. } => tries_left,
            Status::Locked { .. } => panic!("locked: {s:?}"),
        }
    }

    #[test]
    fn counts_down_then_waits() {
        let mut e = entry(Some(5), 300);
        assert_eq!(tries(e.check(0).0), Some(5));
        for left in [4, 3, 2, 1] {
            assert_eq!(tries(e.failed(1_000)), Some(left));
        }
        assert_eq!(left(e.failed(10_000)), 300);
        assert_eq!(left(e.check(70_000).0), 240);
        // Wrong passwords can't even be tried while waiting.
        assert_eq!(left(e.failed(70_000)), 240);
    }

    #[test]
    fn after_a_wait_one_try_then_twice_as_long() {
        let mut e = entry(Some(3), 60);
        for _ in 0..3 {
            e.failed(0);
        }
        let s = e.check(60_000).0;
        assert_eq!(tries(s), Some(1));
        assert!(matches!(s, Status::Open { next_wait_s: 120, .. }));
        assert_eq!(left(e.failed(60_000)), 120);
        for t in [180_000, 180_000 + 240_000] {
            e.check(t);
            e.failed(t);
        }
        assert_eq!(e.wait_s, 480);
    }

    #[test]
    fn waits_never_pass_a_day() {
        let mut e = entry(Some(1), 3600);
        let mut t = 0;
        for _ in 0..10 {
            e.failed(t);
            t += e.wait_s * 1000;
        }
        assert_eq!(e.wait_s, MAX_WAIT_S);
    }

    #[test]
    fn restart_starts_the_wait_again() {
        let mut e = entry(Some(1), 600);
        e.failed(5_000_000);
        // Restarted: the boot clock is now below where the wait started.
        let (s, changed) = e.check(1_000);
        assert!(changed);
        assert!(matches!(
            s,
            Status::Locked {
                left_s: 600,
                restarted: true,
                ..
            }
        ));
        assert!(matches!(
            e.check(301_000).0,
            Status::Locked {
                left_s: 300,
                restarted: false,
                ..
            }
        ));
        assert_eq!(tries(e.check(601_000).0), Some(1));
    }

    #[test]
    fn no_limit_never_waits() {
        let mut e = entry(None, 300);
        for _ in 0..50 {
            assert_eq!(tries(e.failed(0)), None);
        }
    }

    #[test]
    fn lines_roundtrip() {
        let mut e = entry(Some(3), 1800);
        e.failed(42);
        e.failed(43);
        e.failed(44);
        assert_eq!(Entry::parse(&e.line()), Some(e.clone()));
        let open = entry(None, 60);
        assert_eq!(Entry::parse(&open.line()), Some(open));
        assert_eq!(Entry::parse("garbage"), None);
    }

    #[test]
    fn messages() {
        let mut e = entry(Some(5), 300);
        assert_eq!(e.failed(0).after_wrong("p"), "Wrong password. 4 tries left.");
        e.failed(0);
        e.failed(0);
        let s = e.failed(0);
        assert_eq!(
            s.after_wrong("p"),
            "Wrong password. Last try before a 5 min wait."
        );
        assert_eq!(
            s.warning("p").as_deref(),
            Some("1 try left before vault \"p\" locks for 5 min.")
        );
        let s = e.failed(0);
        assert_eq!(
            s.after_wrong("p"),
            "Wrong password. Vault \"p\" is locked for 5:00."
        );
        let (a, b) = s.locked_message("p").unwrap();
        assert_eq!(a, "Vault \"p\" is locked after 5 wrong passwords.");
        assert_eq!(b, "Try again in 5:00.");
        assert_eq!(clock(3725), "1:02:05");
    }
}
