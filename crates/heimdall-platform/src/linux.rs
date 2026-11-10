//! Linux (DESIGN §13.1): process hardening, USB drives, and the session
//! events that lock the vault, read from logind and the screen saver over
//! D-Bus. Every D-Bus call may fail (no logind in a container, a minimal
//! desktop without a screen saver service); then that event just never
//! comes, as documented.
#![allow(unsafe_code)]

use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::{Drive, SessionEvent};

pub fn init() {
    // SAFETY: plain calls with constant arguments. PR_SET_DUMPABLE 0 stops
    // core dumps and ptrace by other processes of the same user; a zero
    // core limit stops dumps even if something sets it back.
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
        let none = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        libc::setrlimit(libc::RLIMIT_CORE, &none);
    }
}

pub fn lock_memory(address: usize, len: usize) -> bool {
    // SAFETY: mlock only pins the pages of the range; an invalid range
    // makes it fail, it doesn't touch the memory.
    unsafe { libc::mlock(address as *const libc::c_void, len) == 0 }
}

// ---------- USB drives ----------

/// Removable media the desktop mounted: udisks puts it under
/// `/media/<user>/` (Debian, Ubuntu) or `/run/media/<user>/` (Fedora, Arch).
pub fn usb_drives() -> Vec<Drive> {
    let Ok(mounts) = std::fs::read_to_string("/proc/self/mounts") else {
        return Vec::new();
    };
    let mut out: Vec<Drive> = Vec::new();
    for line in mounts.lines() {
        let mut fields = line.split(' ');
        let (Some(_dev), Some(point)) = (fields.next(), fields.next()) else {
            continue;
        };
        let root = PathBuf::from(unescape(point));
        if !is_media_mount(&root) || out.iter().any(|d| d.root == root) {
            continue;
        }
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let size = capacity(&root);
        out.push(Drive { root, name, size });
    }
    out
}

/// `/media/<user>/<label>` or `/run/media/<user>/<label>`, exactly.
fn is_media_mount(p: &Path) -> bool {
    let parts: Vec<_> = p.components().skip(1).collect();
    match parts.as_slice() {
        [a, _user, _label] => a.as_os_str() == "media",
        [a, b, _user, _label] => a.as_os_str() == "run" && b.as_os_str() == "media",
        _ => false,
    }
}

/// `/proc/mounts` writes spaces, tabs, newlines and backslashes as octal
/// escapes (`\040`).
fn unescape(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            let oct = &bytes[i + 1..i + 4];
            if oct.iter().all(|b| (b'0'..=b'7').contains(b)) {
                let v = oct.iter().fold(0u16, |acc, b| acc * 8 + u16::from(b - b'0'));
                if let Ok(v) = u8::try_from(v) {
                    out.push(v);
                    i += 4;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn capacity(root: &Path) -> u64 {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(root.as_os_str().as_bytes()) else {
        return 0;
    };
    // SAFETY: `c` is a valid NUL-terminated path; `st` is plain data that
    // statvfs fills in.
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c.as_ptr(), &mut st) != 0 {
            return 0;
        }
        st.f_blocks as u64 * st.f_frsize as u64
    }
}

// ---------- Session events ----------

static PENDING: AtomicU8 = AtomicU8::new(0);
static WAKE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static UNSAVED: AtomicBool = AtomicBool::new(false);
/// logind's "delay" lock on shutdown, held while there are unsaved changes.
static INHIBITOR: Mutex<Option<OwnedFd>> = Mutex::new(None);

const LOCKED: u8 = 1;
const SUSPENDING: u8 = 2;
const ENDING: u8 = 4;

const LOGIND: &str = "org.freedesktop.login1";
const LOGIND_PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";

fn report(event: u8) {
    PENDING.fetch_or(event, Ordering::SeqCst);
    if let Some(wake) = WAKE.get() {
        wake();
    }
}

/// `_window` is unused: logind and the screen saver report on D-Bus.
pub fn watch_session(_window: isize, wake: Box<dyn Fn() + Send + Sync>) -> bool {
    if WAKE.set(wake).is_err() {
        return false;
    }
    let system = zbus::blocking::Connection::system().ok().map(Arc::new);
    let session = zbus::blocking::Connection::session().ok().map(Arc::new);
    if let Some(sys) = &system {
        // Sleep, shutdown, and lock requests for our session.
        listen(
            sys.clone(),
            format!("type='signal',sender='{LOGIND}',interface='{MANAGER}',member='PrepareForSleep'"),
            |m| {
                if m.body().deserialize::<bool>().unwrap_or(false) {
                    report(SUSPENDING);
                }
            },
        );
        listen(
            sys.clone(),
            format!("type='signal',sender='{LOGIND}',interface='{MANAGER}',member='PrepareForShutdown'"),
            |m| {
                if m.body().deserialize::<bool>().unwrap_or(false) && UNSAVED.load(Ordering::SeqCst) {
                    report(ENDING);
                }
            },
        );
        if let Some(path) = own_session(sys) {
            listen(sys.clone(), format!("type='signal',sender='{LOGIND}',path='{path}',interface='org.freedesktop.login1.Session',member='Lock'"), |_| report(LOCKED));
            // GNOME and KDE set LockedHint when their lock screen shows.
            listen(sys.clone(), format!("type='signal',sender='{LOGIND}',path='{path}',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged'"), |m| {
                type Changed = (String, std::collections::HashMap<String, zbus::zvariant::OwnedValue>, Vec<String>);
                if let Ok((_, changed, _)) = m.body().deserialize::<Changed>() {
                    if changed.get("LockedHint").and_then(|v| bool::try_from(v).ok()) == Some(true) {
                        report(LOCKED);
                    }
                }
            });
        }
    }
    if let Some(ses) = &session {
        // The screen saver services of KDE / Xfce and of GNOME.
        for iface in ["org.freedesktop.ScreenSaver", "org.gnome.ScreenSaver"] {
            listen(
                ses.clone(),
                format!("type='signal',interface='{iface}',member='ActiveChanged'"),
                |m| {
                    if m.body().deserialize::<bool>().unwrap_or(false) {
                        report(LOCKED);
                    }
                },
            );
        }
    }
    system.is_some() || session.is_some()
}

/// Our logind session's object path.
fn own_session(sys: &zbus::blocking::Connection) -> Option<String> {
    let pid = std::process::id();
    let reply = sys
        .call_method(
            Some(LOGIND),
            LOGIND_PATH,
            Some(MANAGER),
            "GetSessionByPID",
            &(pid,),
        )
        .ok()?;
    let path: zbus::zvariant::OwnedObjectPath = reply.body().deserialize().ok()?;
    Some(path.as_str().to_owned())
}

/// Runs `on` for every message matching `rule`, on its own thread.
fn listen(conn: Arc<zbus::blocking::Connection>, rule: String, on: impl Fn(&zbus::Message) + Send + 'static) {
    let _ = std::thread::Builder::new()
        .name("heimdall-dbus".into())
        .spawn(move || {
            let Ok(iter) = zbus::blocking::MessageIterator::for_match_rule(rule.as_str(), &conn, Some(16))
            else {
                return;
            };
            for msg in iter.flatten() {
                on(&msg);
            }
        });
}

pub fn take_session_event() -> Option<SessionEvent> {
    match PENDING.swap(0, Ordering::SeqCst) {
        0 => None,
        bits if bits & ENDING != 0 => Some(SessionEvent::Ending),
        bits if bits & SUSPENDING != 0 => Some(SessionEvent::Suspending),
        _ => Some(SessionEvent::Locked),
    }
}

/// While there are unsaved changes, hold a logind "delay" lock so a
/// shutdown waits (up to logind's limit, usually 5 s) for the save.
pub fn set_unsaved(unsaved: bool) {
    if UNSAVED.swap(unsaved, Ordering::SeqCst) == unsaved {
        return;
    }
    if !unsaved {
        release_inhibitor();
        return;
    }
    let _ = std::thread::Builder::new()
        .name("heimdall-inhibit".into())
        .spawn(|| {
            let fd = zbus::blocking::Connection::system().ok().and_then(|sys| {
                let reply = sys
                    .call_method(
                        Some(LOGIND),
                        LOGIND_PATH,
                        Some(MANAGER),
                        "Inhibit",
                        &("shutdown", "Heimdall", "Saving your notes", "delay"),
                    )
                    .ok()?;
                let fd: zbus::zvariant::OwnedFd = reply.body().deserialize().ok()?;
                Some(OwnedFd::from(fd))
            });
            // Saved meanwhile: drop it right away.
            if UNSAVED.load(Ordering::SeqCst) {
                if let Ok(mut held) = INHIBITOR.lock() {
                    *held = fd;
                }
            }
        });
}

fn release_inhibitor() {
    if let Ok(mut held) = INHIBITOR.lock() {
        *held = None;
    }
}

pub fn allow_end_session() {
    release_inhibitor();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_mounts() {
        assert!(is_media_mount(Path::new("/media/ana/KINGSTON")));
        assert!(is_media_mount(Path::new("/run/media/ana/KINGSTON")));
        assert!(!is_media_mount(Path::new("/media/ana")));
        assert!(!is_media_mount(Path::new("/home/ana/media/x")));
        assert!(!is_media_mount(Path::new("/")));
    }

    #[test]
    fn mount_escapes() {
        assert_eq!(unescape(r"/media/ana/MY\040DRIVE"), "/media/ana/MY DRIVE");
        assert_eq!(unescape(r"/media/ana/a\134b"), r"/media/ana/a\b");
        assert_eq!(unescape(r"/media/ana/end\04"), r"/media/ana/end\04");
    }
}
