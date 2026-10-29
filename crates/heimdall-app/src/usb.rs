//! USB keys (DESIGN §8): finding `.bifrost` files on connected drives,
//! writing a new one, and presence mode (lock when the key is removed).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use heimdall_core::format::{VaultId, HINT_LEN};
use heimdall_core::keyfile::{KEYFILE_EXTENSION, KEYFILE_LEN};
use heimdall_core::Keyfile;
pub use heimdall_platform::Drive;

use crate::support::Job;

/// A key file found on (or just written to) a drive.
#[derive(Clone)]
pub struct UsbKey {
    pub keyfile: Keyfile,
    pub path: PathBuf,
    pub drive: Drive,
}

impl UsbKey {
    /// `KINGSTON (E:)`
    pub fn label(&self) -> String {
        drive_label(&self.drive)
    }
}

pub fn drive_label(d: &Drive) -> String {
    format!("{} ({})", d.name, d.letter())
}

/// Every key file for `vault_id` in the root folder of each USB drive.
/// Only the root is searched: that is where Heimdall writes them.
pub fn find_keys(vault_id: &VaultId) -> Vec<UsbKey> {
    let mut found = Vec::new();
    for drive in heimdall_platform::usb_drives() {
        let Ok(entries) = fs::read_dir(&drive.root) else {
            continue;
        };
        for entry in entries.flatten().take(512) {
            let path = entry.path();
            let is_key = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(KEYFILE_EXTENSION))
                && entry
                    .metadata()
                    .is_ok_and(|m| m.is_file() && m.len() == KEYFILE_LEN as u64);
            if !is_key {
                continue;
            }
            let Some(keyfile) = read_keyfile(&path) else {
                continue;
            };
            if keyfile.vault_id() == vault_id {
                found.push(UsbKey {
                    keyfile,
                    path,
                    drive: drive.clone(),
                });
            }
        }
    }
    found
}

fn read_keyfile(path: &Path) -> Option<Keyfile> {
    let bytes = zeroize::Zeroizing::new(fs::read(path).ok()?);
    Keyfile::parse(&bytes).ok()
}

/// Writes `keyfile` to the root of `drive` as `<stem>.bifrost`, or
/// `<stem>-2.bifrost` and so on if that name is taken: an existing key file
/// is never overwritten, it may be the only key to another vault. The file
/// is read back to make sure the drive really holds it.
pub fn write_key(drive: &Drive, stem: &str, keyfile: &Keyfile) -> std::io::Result<UsbKey> {
    let bytes = keyfile.to_bytes();
    for n in 1..100 {
        let name = if n == 1 {
            format!("{stem}.{KEYFILE_EXTENSION}")
        } else {
            format!("{stem}-{n}.{KEYFILE_EXTENSION}")
        };
        let path = drive.root.join(name);
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let written = file.write_all(&bytes).and_then(|()| file.sync_all());
        drop(file);
        let verified = written.is_ok() && read_keyfile(&path).is_some_and(|k| k.hint() == keyfile.hint());
        if !verified {
            let _ = fs::remove_file(&path);
            return Err(written.err().unwrap_or_else(|| {
                std::io::Error::other("the key file could not be read back from the drive")
            }));
        }
        return Ok(UsbKey {
            keyfile: keyfile.clone(),
            path,
            drive: drive.clone(),
        });
    }
    Err(std::io::Error::other(
        "too many key files with this name on the drive",
    ))
}

/// Re-runs a slow lookup (listing drives, reading key files) in the
/// background every second, so screens can show what is plugged in now.
pub struct Scan<T> {
    work: Arc<dyn Fn() -> T + Send + Sync>,
    job: Option<Job<T>>,
    pub value: Option<T>,
    next: Instant,
}

impl<T: Send + 'static> Scan<T> {
    pub fn new(work: impl Fn() -> T + Send + Sync + 'static) -> Self {
        Self {
            work: Arc::new(work),
            job: None,
            value: None,
            next: Instant::now(),
        }
    }

    /// Call every frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        if let Some(v) = self.job.as_ref().and_then(Job::poll) {
            self.value = Some(v);
            self.job = None;
            self.next = Instant::now() + SCAN_EVERY;
        }
        if self.job.is_none() && Instant::now() >= self.next {
            let work = self.work.clone();
            self.job = Some(Job::spawn(ctx, move || work()));
        }
        ctx.request_repaint_after(SCAN_EVERY);
    }

    /// Scan again right away (the Refresh link).
    pub fn refresh(&mut self) {
        self.next = Instant::now();
    }
}

const SCAN_EVERY: Duration = Duration::from_secs(1);

pub fn key_scan(vault_id: VaultId) -> Scan<Vec<UsbKey>> {
    Scan::new(move || find_keys(&vault_id))
}

pub fn drive_scan() -> Scan<Vec<Drive>> {
    Scan::new(heimdall_platform::usb_drives)
}

/// Presence mode (DESIGN §8.3): a thread checks once a second that the key
/// file is still readable and still the same key. Only the key's hint is
/// kept here, never its secret. Stops when dropped.
pub struct Presence {
    pub key_label: String,
    stop: Arc<AtomicBool>,
    removed: Arc<AtomicBool>,
}

impl Presence {
    pub fn start(ctx: &egui::Context, key: &UsbKey) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let removed = Arc::new(AtomicBool::new(false));
        let path = key.path.clone();
        let hint: [u8; HINT_LEN] = key.keyfile.hint();
        let (stop_t, removed_t, ctx) = (stop.clone(), removed.clone(), ctx.clone());
        std::thread::spawn(move || {
            while !stop_t.load(Ordering::Relaxed) {
                let present = read_keyfile(&path).is_some_and(|k| k.hint() == hint);
                if !present {
                    removed_t.store(true, Ordering::Relaxed);
                    ctx.request_repaint();
                    return;
                }
                std::thread::sleep(PRESENCE_EVERY);
            }
        });
        Self {
            key_label: key.label(),
            stop,
            removed,
        }
    }

    pub fn removed(&self) -> bool {
        self.removed.load(Ordering::Relaxed)
    }
}

const PRESENCE_EVERY: Duration = Duration::from_millis(1000);

impl Drop for Presence {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(root: &Path) -> Drive {
        Drive {
            root: root.to_owned(),
            name: "TEST".into(),
            size: 0,
        }
    }

    #[test]
    fn write_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("heimdall-usb-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let d = drive(&dir);
        let a = Keyfile::generate([1; 16]).unwrap();
        let b = Keyfile::generate([2; 16]).unwrap();
        let ka = write_key(&d, "personal", &a).unwrap();
        let kb = write_key(&d, "personal", &b).unwrap();
        assert_eq!(ka.path.file_name().unwrap(), "personal.bifrost");
        assert_eq!(kb.path.file_name().unwrap(), "personal-2.bifrost");
        assert_eq!(read_keyfile(&ka.path).unwrap().hint(), a.hint());
        assert_eq!(read_keyfile(&kb.path).unwrap().hint(), b.hint());
        fs::remove_dir_all(&dir).unwrap();
    }

    /// The Unlock screen's lookup: only key files of the right vault, from
    /// the root of each drive. Uses the debug-only fake drive.
    #[test]
    fn finds_keys_of_this_vault() {
        let dir = std::env::temp_dir().join(format!("heimdall-find-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mine = Keyfile::generate([7; 16]).unwrap();
        write_key(&drive(&dir), "a", &mine).unwrap();
        write_key(&drive(&dir), "b", &Keyfile::generate([8; 16]).unwrap()).unwrap();
        fs::write(dir.join("junk.bifrost"), b"not a key").unwrap();
        std::env::set_var("HEIMDALL_FAKE_USB", &dir);

        let found = find_keys(&[7; 16]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].keyfile.hint(), mine.hint());
        assert_eq!(found[0].path.file_name().unwrap(), "a.bifrost");
        fs::remove_dir_all(&dir).unwrap();
    }
}
