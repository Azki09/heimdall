//! Windows implementation. Every `unsafe` block is a plain Win32 call with
//! buffers owned by the caller; none of them keep pointers past the call.
#![allow(unsafe_code)]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    BusTypeUsb, CreateFileW, GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Ioctl::{
    PropertyStandardQuery, StorageDeviceProperty, IOCTL_STORAGE_QUERY_PROPERTY, STORAGE_DEVICE_DESCRIPTOR,
    STORAGE_PROPERTY_QUERY,
};
use windows_sys::Win32::System::WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE};
use windows_sys::Win32::System::IO::DeviceIoControl;

use crate::Drive;

pub fn usb_drives() -> Vec<Drive> {
    let system = std::env::var("SystemDrive")
        .unwrap_or_else(|_| "C:".into())
        .to_ascii_uppercase();
    // SAFETY: no arguments; returns a bit mask of drive letters.
    let mask = unsafe { GetLogicalDrives() };
    let mut drives = Vec::new();
    for i in 0..26u8 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i) as char;
        if format!("{letter}:") == system {
            continue;
        }
        let root = format!("{letter}:\\");
        let root_w = wide(&root);
        // SAFETY: `root_w` is a NUL-terminated UTF-16 string that outlives the call.
        let kind = unsafe { GetDriveTypeW(root_w.as_ptr()) };
        let device = device_info(letter);
        let on_usb = device.as_ref().is_some_and(|d| d.usb);
        if !(kind == DRIVE_REMOVABLE || (kind == DRIVE_FIXED && on_usb)) {
            continue;
        }
        // A card reader with no card has a letter but no volume: skip it.
        let Some(label) = volume_label(&root_w) else {
            continue;
        };
        let name = if !label.is_empty() {
            label
        } else {
            device
                .map(|d| d.product)
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| "USB drive".into())
        };
        drives.push(Drive {
            root: PathBuf::from(root),
            name,
            size: total_bytes(&root_w),
        });
    }
    drives
}

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

fn volume_label(root_w: &[u16]) -> Option<String> {
    let mut name = [0u16; 261];
    // SAFETY: `root_w` is NUL-terminated; `name` is a writable buffer of the
    // length passed; the optional out-pointers are null.
    let ok = unsafe {
        GetVolumeInformationW(
            root_w.as_ptr(),
            name.as_mut_ptr(),
            name.len() as u32,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            0,
        )
    };
    if ok == 0 {
        return None;
    }
    let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    Some(String::from_utf16_lossy(&name[..len]).trim().to_owned())
}

fn total_bytes(root_w: &[u16]) -> u64 {
    let mut total = 0u64;
    // SAFETY: `root_w` is NUL-terminated; `total` is a valid out-pointer; the
    // other out-pointers are optional and null.
    let ok = unsafe { GetDiskFreeSpaceExW(root_w.as_ptr(), ptr::null_mut(), &mut total, ptr::null_mut()) };
    if ok == 0 {
        0
    } else {
        total
    }
}

struct DeviceInfo {
    usb: bool,
    product: String,
}

/// Bus type and product name of the disk behind a drive letter. Opening the
/// volume with no access rights is enough for this query; no admin needed.
fn device_info(letter: char) -> Option<DeviceInfo> {
    let path = wide(&format!(r"\\.\{letter}:"));
    // SAFETY: `path` is NUL-terminated; no security attributes or template.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    let mut buf = [0u8; 1024];
    let mut returned = 0u32;
    // SAFETY: `handle` is open; input and output buffers are valid for the
    // sizes passed; synchronous call (no OVERLAPPED).
    let ok = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            (&query as *const STORAGE_PROPERTY_QUERY).cast(),
            size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
            &mut returned,
            ptr::null_mut(),
        )
    };
    // SAFETY: `handle` was opened above and is closed exactly once.
    unsafe {
        CloseHandle(handle);
    }
    let returned = returned as usize;
    if ok == 0 || returned < size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
        return None;
    }
    // SAFETY: the buffer holds at least one descriptor (checked above);
    // `read_unaligned` copies it out without alignment requirements.
    let desc: STORAGE_DEVICE_DESCRIPTOR = unsafe { ptr::read_unaligned(buf.as_ptr().cast()) };
    let ascii_at = |offset: u32| -> String {
        let start = offset as usize;
        if start == 0 || start >= returned {
            return String::new();
        }
        let bytes = &buf[start..returned];
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end]).trim().to_owned()
    };
    Some(DeviceInfo {
        usb: desc.BusType == BusTypeUsb,
        product: ascii_at(desc.ProductIdOffset),
    })
}
