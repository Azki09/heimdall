//! Code that talks to the operating system (DESIGN §3, §13): listing USB
//! drives for the key file (§8), printing the recovery sheet (§7) and
//! process hardening (§12). Only Windows is implemented; other systems get
//! stubs so the rest of the code compiles.

use std::path::PathBuf;

#[cfg(windows)]
mod harden;
#[cfg(windows)]
mod print;
#[cfg(windows)]
mod windows;

/// A drive a USB key file can live on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drive {
    /// `E:\`
    pub root: PathBuf,
    /// Volume label, or the device name if the volume has none ("KINGSTON").
    pub name: String,
    /// Capacity in bytes; 0 if unknown.
    pub size: u64,
}

impl Drive {
    /// `E:` for a real drive.
    pub fn letter(&self) -> String {
        match self.root.components().next() {
            Some(std::path::Component::Prefix(p)) => p.as_os_str().to_string_lossy().into_owned(),
            _ => self.root.display().to_string(),
        }
    }

    /// `32 GB`, as printed on the drive (decimal units).
    pub fn size_label(&self) -> String {
        const GB: f64 = 1e9;
        match self.size {
            0 => String::new(),
            s if (s as f64) < GB => format!("{} MB", (s as f64 / 1e6).round()),
            s => format!("{} GB", (s as f64 / GB).round()),
        }
    }
}

/// Call once at startup. On Windows, stops the system from showing "insert
/// a disk" dialogs when a removed drive is probed, and makes a crash end
/// the process without a crash dump (DESIGN §12).
pub fn init() {
    #[cfg(windows)]
    harden::init();
}

/// Keeps the memory at `address` out of the page file (`VirtualLock`), for
/// the Master Key. Whole pages are locked. Returns false if Windows refused.
pub fn lock_memory(address: usize, len: usize) -> bool {
    #[cfg(windows)]
    return harden::lock_memory(address, len);
    #[cfg(not(windows))]
    {
        let _ = (address, len);
        false
    }
}

/// Hides the window from screenshots, screen recording and screen sharing
/// (it shows black there). `window` is the native handle (`HWND`).
pub fn set_capture_excluded(window: isize, on: bool) -> bool {
    #[cfg(windows)]
    return harden::set_capture_excluded(window, on);
    #[cfg(not(windows))]
    {
        let _ = (window, on);
        false
    }
}

/// Something that must lock the vault (DESIGN §12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent {
    /// Windows was locked (Win+L).
    Locked,
    /// The computer is going to sleep or hibernate.
    Suspending,
}

/// Starts watching for [`SessionEvent`]s on `window`; `wake` is called
/// when one arrives (to repaint, so the app picks it up). Call once, from
/// the UI thread.
pub fn watch_session(window: isize, wake: impl Fn() + Send + Sync + 'static) -> bool {
    #[cfg(windows)]
    return harden::watch_session(window, Box::new(wake));
    #[cfg(not(windows))]
    {
        let _ = (window, wake);
        false
    }
}

/// The last event since the previous call; sleep wins over lock.
pub fn take_session_event() -> Option<SessionEvent> {
    #[cfg(windows)]
    return harden::take_session_event();
    #[cfg(not(windows))]
    None
}

/// USB drives currently connected: removable drives, plus fixed drives on
/// the USB bus (external SSDs report themselves as fixed). Never the
/// system drive.
///
/// Debug builds also list each folder in `HEIMDALL_FAKE_USB` (separated by
/// `;`) as a drive, so presence mode can be tested without hardware:
/// renaming the folder "removes" the drive.
pub fn usb_drives() -> Vec<Drive> {
    #[allow(unused_mut)]
    let mut drives = Vec::new();
    #[cfg(windows)]
    drives.extend(windows::usb_drives());
    #[cfg(debug_assertions)]
    drives.extend(fake_drives());
    drives
}

/// What goes on the printed recovery sheet.
pub struct RecoverySheet<'a> {
    /// File name of the vault ("personal.heim").
    pub vault: &'a str,
    /// When it was printed, already formatted.
    pub date: &'a str,
    /// The 12 words, in order.
    pub words: &'a [&'a str],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrintOutcome {
    Printed,
    /// The user closed the print dialog (or a "save as" printer's file dialog).
    Canceled,
    Failed(String),
}

/// Shows the Windows print dialog and prints the sheet on the chosen
/// printer. Nothing is written to disk by Heimdall; the Windows spooler may
/// keep the page in its own queue until it is printed.
///
/// Debug builds print to "Microsoft Print to PDF" without a dialog when
/// `HEIMDALL_PRINT_TO` names the output file, for testing.
pub fn print_recovery_sheet(sheet: &RecoverySheet) -> PrintOutcome {
    #[cfg(windows)]
    return print::print_recovery_sheet(sheet);
    #[cfg(not(windows))]
    {
        let _ = sheet;
        PrintOutcome::Failed("Printing is only available on Windows for now.".into())
    }
}

#[cfg(debug_assertions)]
fn fake_drives() -> Vec<Drive> {
    let Some(list) = std::env::var_os("HEIMDALL_FAKE_USB") else {
        return Vec::new();
    };
    std::env::split_paths(&list)
        .filter(|p| p.is_dir())
        .map(|root| Drive {
            name: root
                .file_name()
                .map(|n| n.to_string_lossy().to_uppercase())
                .unwrap_or_default(),
            root,
            size: 0,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        let d = Drive {
            root: PathBuf::from(r"E:\"),
            name: "KINGSTON".into(),
            size: 32_017_047_552,
        };
        assert_eq!(d.letter(), "E:");
        assert_eq!(d.size_label(), "32 GB");
        assert_eq!(Drive { size: 0, ..d.clone() }.size_label(), "");
        assert_eq!(
            Drive {
                size: 512_000_000,
                ..d
            }
            .size_label(),
            "512 MB"
        );
    }

    #[test]
    fn listing_does_not_panic() {
        let _ = usb_drives();
    }
}
