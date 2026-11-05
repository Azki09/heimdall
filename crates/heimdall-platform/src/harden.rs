//! Process hardening on Windows (DESIGN §12): no crash dumps, key memory
//! kept out of the page file, the window hidden from screen capture, and
//! the Windows lock and sleep messages that lock the vault.
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicIsize, AtomicU8, Ordering};
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Diagnostics::Debug::{
    SetErrorMode, SetUnhandledExceptionFilter, EXCEPTION_EXECUTE_HANDLER, EXCEPTION_POINTERS,
    SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SEM_NOOPENFILEERRORBOX,
};
use windows_sys::Win32::System::Memory::VirtualLock;
use windows_sys::Win32::System::RemoteDesktop::{WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, SetWindowDisplayAffinity, SetWindowLongPtrW, GWLP_WNDPROC, PBT_APMSUSPEND,
    WDA_EXCLUDEFROMCAPTURE, WDA_NONE, WM_POWERBROADCAST, WM_WTSSESSION_CHANGE, WNDPROC, WTS_SESSION_LOCK,
};

use crate::SessionEvent;

pub fn init() {
    // SAFETY: takes and returns flags only.
    unsafe {
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOOPENFILEERRORBOX | SEM_NOGPFAULTERRORBOX);
    }
    // SAFETY: `die` has the signature the filter needs and lives forever.
    unsafe {
        SetUnhandledExceptionFilter(Some(die));
    }
}

/// A crash ends the process on the spot, before Windows Error Reporting
/// can write a dump of its memory (notes, the Master Key).
unsafe extern "system" fn die(_: *const EXCEPTION_POINTERS) -> i32 {
    // SAFETY: ends this process; the pseudo-handle needs no closing.
    unsafe {
        TerminateProcess(GetCurrentProcess(), 0xDEAD);
    }
    EXCEPTION_EXECUTE_HANDLER
}

pub fn lock_memory(address: usize, len: usize) -> bool {
    // SAFETY: VirtualLock only reads the address range to pin its pages; an
    // invalid range makes it fail, it doesn't touch the memory.
    unsafe { VirtualLock(address as *const _, len) != 0 }
}

pub fn set_capture_excluded(hwnd: isize, on: bool) -> bool {
    let affinity = if on { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
    // SAFETY: plain call on a window handle; a stale handle makes it fail.
    unsafe { SetWindowDisplayAffinity(hwnd as HWND, affinity) != 0 }
}

/// The window procedure we replaced, and what it reported.
static ORIGINAL: AtomicIsize = AtomicIsize::new(0);
static PENDING: AtomicU8 = AtomicU8::new(0);
static WAKE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

const LOCKED: u8 = 1;
const SUSPENDING: u8 = 2;

/// Subclasses the window to see `WM_WTSSESSION_CHANGE` and
/// `WM_POWERBROADCAST`, which eframe doesn't pass on. Call once, from the
/// thread that owns the window.
pub fn watch_session(hwnd: isize, wake: Box<dyn Fn() + Send + Sync>) -> bool {
    if WAKE.set(wake).is_err() {
        return false;
    }
    let hwnd = hwnd as HWND;
    // SAFETY: called on the window's own thread. The previous procedure is
    // stored before any message can reach `window_proc` (same thread).
    unsafe {
        let previous = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, window_proc as *const () as isize);
        if previous == 0 {
            return false;
        }
        ORIGINAL.store(previous, Ordering::SeqCst);
        WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) != 0
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let event = match (msg, wparam as u32) {
        (WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK) => LOCKED,
        (WM_POWERBROADCAST, PBT_APMSUSPEND) => SUSPENDING,
        _ => 0,
    };
    if event != 0 {
        PENDING.fetch_or(event, Ordering::SeqCst);
        if let Some(wake) = WAKE.get() {
            wake();
        }
    }
    // SAFETY: ORIGINAL holds the procedure SetWindowLongPtrW returned, a
    // valid WNDPROC for this window.
    unsafe {
        let original: WNDPROC = std::mem::transmute::<isize, WNDPROC>(ORIGINAL.load(Ordering::SeqCst));
        CallWindowProcW(original, hwnd, msg, wparam, lparam)
    }
}

pub fn take_session_event() -> Option<SessionEvent> {
    match PENDING.swap(0, Ordering::SeqCst) {
        0 => None,
        bits if bits & SUSPENDING != 0 => Some(SessionEvent::Suspending),
        _ => Some(SessionEvent::Locked),
    }
}
