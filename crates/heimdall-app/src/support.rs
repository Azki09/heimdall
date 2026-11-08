//! Plumbing shared by the screens: background jobs, the recent-vaults list,
//! clipboard auto-clear and user-facing error messages.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui;
use heimdall_core::Error;

/// Runs slow work (Argon2id takes about a second by design) off the UI
/// thread and wakes the UI when it is done.
pub struct Job<T> {
    rx: mpsc::Receiver<T>,
}

impl<T: Send + 'static> Job<T> {
    pub fn spawn(ctx: &egui::Context, work: impl FnOnce() -> T + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work());
            ctx.request_repaint();
        });
        Self { rx }
    }

    pub fn poll(&self) -> Option<T> {
        self.rx.try_recv().ok()
    }
}

/// Wording for errors shown on screen (UI is in English, DESIGN §15.1).
pub fn error_message(e: &Error) -> String {
    match e {
        Error::WrongPassword => "Wrong password.".into(),
        Error::WrongRecoveryCode => "That recovery code doesn't open this vault.".into(),
        Error::InvalidRecoveryCode => "That isn't a valid recovery code. Check each word.".into(),
        Error::KeyfileRequired => "Connect the USB key for this vault.".into(),
        Error::WrongKeyfile => "This USB key no longer opens the vault. It may have been revoked.".into(),
        Error::KeyfileForOtherVault => "That USB key belongs to a different vault.".into(),
        Error::MissingKeyfile(_) => "Connect every USB key you keep.".into(),
        Error::NoPasswordSlotLeft => "That would leave no way to open the vault with your password.".into(),
        Error::BadMagic | Error::Malformed | Error::BadSlotLength => {
            "This file isn't a Heimdall vault, or it is damaged.".into()
        }
        Error::ContentAuthFailed => "The vault is damaged or was modified. Try the .bak copy.".into(),
        Error::Io(io) => format!("Couldn't access the file: {io}"),
        other => format!("{other}."),
    }
}

// ---------- Recent vaults ----------

/// The Welcome list. It lives outside any vault, in plain text, which is
/// why each vault decides whether it appears here (DESIGN §6.1).
pub struct RecentVaults {
    file: Option<PathBuf>,
    pub entries: Vec<RecentEntry>,
}

#[derive(Clone)]
pub struct RecentEntry {
    pub path: PathBuf,
    pub opened: i64,
}

impl RecentVaults {
    pub fn load() -> Self {
        let file = std::env::var_os("APPDATA").map(|d| Path::new(&d).join("Heimdall").join("recent.txt"));
        let entries = file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|s| {
                s.lines()
                    .filter_map(|l| {
                        let (ts, path) = l.split_once('\t')?;
                        Some(RecentEntry {
                            path: PathBuf::from(path),
                            opened: ts.parse().ok()?,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { file, entries }
    }

    pub fn touch(&mut self, path: &Path) {
        self.entries.retain(|e| e.path != path);
        self.entries.insert(
            0,
            RecentEntry {
                path: path.to_owned(),
                opened: heimdall_core::content::now(),
            },
        );
        self.entries.truncate(8);
        self.save();
    }

    pub fn remove(&mut self, path: &Path) {
        let before = self.entries.len();
        self.entries.retain(|e| e.path != path);
        if self.entries.len() != before {
            self.save();
        }
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let body: String = self
            .entries
            .iter()
            .map(|e| format!("{}\t{}\n", e.opened, e.path.display()))
            .collect();
        let _ = std::fs::write(file, body);
    }
}

// ---------- Clipboard ----------

/// Puts text on the clipboard, marked so Windows keeps it out of the
/// clipboard history (Win+V) and the cloud clipboard, and so clipboard
/// managers that honor it ignore it (DESIGN §11). Every copy from Heimdall
/// goes through here (see `HeimdallApp::handle_copies`).
pub fn set_clipboard(text: &str) -> bool {
    let Ok(mut cb) = arboard::Clipboard::new() else {
        return false;
    };
    #[cfg(windows)]
    {
        use arboard::SetExtWindows;
        cb.set()
            .exclude_from_monitoring()
            .exclude_from_history()
            .exclude_from_cloud()
            .text(text)
            .is_ok()
    }
    #[cfg(not(windows))]
    {
        cb.set_text(text).is_ok()
    }
}

/// The text on the clipboard, for Paste in the editor's right-click menu.
/// Wiped when dropped. `None` if the clipboard holds no text.
pub fn clipboard_text() -> Option<zeroize::Zeroizing<String>> {
    let text = zeroize::Zeroizing::new(arboard::Clipboard::new().ok()?.get_text().ok()?);
    (!text.is_empty()).then_some(text)
}

/// Copies text that has no vault open to take the timer from (the recovery
/// code in the wizard): cleared after `clear_after_s` like any other copy.
pub fn copy_with_timer(ctx: &egui::Context, text: &str, clear_after_s: u32) {
    ctx.data_mut(|d| d.insert_temp(clear_after_id(), clear_after_s));
    ctx.copy_text(text.to_owned());
}

pub fn clear_after_id() -> egui::Id {
    egui::Id::new("clipboard_clear_after")
}

// ---------- Clipboard auto-clear ----------

/// Clears the clipboard N seconds after Heimdall copied something, but only
/// if it still holds what Heimdall put there (DESIGN §11). Only a hash of
/// the copied text is kept.
#[derive(Default)]
pub struct ClipboardGuard {
    pending: Option<(Instant, [u8; 32])>,
}

impl ClipboardGuard {
    pub fn copied(&mut self, text: &str, clear_after_s: u32) {
        let hash = heimdall_core::crypto::sha256(text.as_bytes());
        self.pending = Some((Instant::now() + Duration::from_secs(clear_after_s as u64), hash));
    }

    /// Call every frame; returns how long until the next check is due.
    pub fn tick(&mut self) -> Option<Duration> {
        let (deadline, _) = self.pending?;
        let now = Instant::now();
        if now >= deadline {
            self.clear_now();
            None
        } else {
            Some(deadline - now)
        }
    }

    pub fn clear_now(&mut self) {
        let Some((_, hash)) = self.pending.take() else {
            return;
        };
        let Ok(mut cb) = arboard::Clipboard::new() else {
            return;
        };
        let ours = cb.get_text().map(|t| {
            let mut t = t;
            let same = heimdall_core::crypto::sha256(t.as_bytes()) == hash;
            zeroize::Zeroize::zeroize(&mut t);
            same
        });
        if ours.unwrap_or(false) {
            let _ = cb.clear();
        }
    }
}

// ---------- Small formatting helpers ----------

pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn friendly_date(ts: i64) -> String {
    use chrono::{Local, TimeZone};
    let Some(when) = Local.timestamp_opt(ts, 0).single() else {
        return String::new();
    };
    let today = Local::now().date_naive();
    match (today - when.date_naive()).num_days() {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ => when.format("%b %-d").to_string(),
    }
}

pub fn clock(ts: i64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_default()
}

pub fn duration_label(secs: u32) -> String {
    match secs {
        s if s >= 3600 && s % 3600 == 0 => format!("{} hour{}", s / 3600, if s == 3600 { "" } else { "s" }),
        s if s >= 60 => format!("{} min", s / 60),
        s => format!("{s} s"),
    }
}
