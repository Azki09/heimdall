//! Code that talks to the operating system (DESIGN §3, §13): listing USB
//! drives for the key file (§8) and process-wide settings. Only Windows is
//! implemented; other systems get stubs so the rest of the code compiles.

use std::path::PathBuf;

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
/// a disk" dialogs when a removed drive is probed.
pub fn init() {
    #[cfg(windows)]
    windows::init();
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
