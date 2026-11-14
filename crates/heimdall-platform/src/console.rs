//! The Windows console, for the terminal mode (`heimdall.com`, DESIGN
//! §17.6): closing the console window, the session messages a console
//! program never gets by itself, whether the console was opened just for
//! us, and its font.
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use windows_sys::core::BOOL;
use windows_sys::Win32::System::Console::{
    GetConsoleProcessList, GetCurrentConsoleFontEx, GetStdHandle, SetConsoleCtrlHandler, CONSOLE_FONT_INFOEX,
    CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DispatchMessageW, GetMessageW, TranslateMessage, MSG,
};

static CLOSED: OnceLock<&'static AtomicBool> = OnceLock::new();

/// Windows ends the process this long after the console is closed.
const GRACE: Duration = Duration::from_millis(4500);

/// Sets `flag` when the console window is closed (or the user signs out),
/// then holds Windows back for a few seconds so the main thread can save,
/// encrypted, and exit by itself.
pub fn on_close(flag: &'static AtomicBool) -> bool {
    if CLOSED.set(flag).is_err() {
        return false;
    }
    // SAFETY: `handler` has the signature Windows expects and lives forever.
    unsafe { SetConsoleCtrlHandler(Some(handler), 1) != 0 }
}

unsafe extern "system" fn handler(event: u32) -> BOOL {
    if !matches!(event, CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT) {
        // Ctrl+C and Ctrl+Break: the default. In raw mode Ctrl+C is a key.
        return 0;
    }
    if let Some(flag) = CLOSED.get() {
        flag.store(true, Ordering::SeqCst);
    }
    // Returning ends the process at once; the main thread exits first if
    // it can.
    let start = Instant::now();
    while start.elapsed() < GRACE {
        std::thread::sleep(Duration::from_millis(50));
    }
    1
}

/// A hidden window on a thread of its own, so Win+L, sleep and shutdown
/// reach the terminal mode the way they reach the app's window.
pub fn watch_session() -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("session-window".into())
        .spawn(move || {
            let class: Vec<u16> = "STATIC".encode_utf16().chain([0]).collect();
            // SAFETY: a plain hidden STATIC window owned by this thread,
            // which keeps it for the life of the process.
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    std::ptr::null(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            };
            let ok = !hwnd.is_null() && crate::harden::watch_session(hwnd as isize, Box::new(|| {}));
            let _ = tx.send(ok);
            if !ok {
                return;
            }
            // SAFETY: the usual message loop on the window's own thread;
            // `msg` is a plain struct Windows fills in.
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        });
    spawned.is_ok() && rx.recv().unwrap_or(false)
}

/// Whether this process is the only one on its console: it was started by
/// a double-click, not from cmd or PowerShell.
pub fn console_is_ours() -> bool {
    let mut ids = [0u32; 4];
    // SAFETY: writes at most `ids.len()` process ids into `ids`.
    let count = unsafe { GetConsoleProcessList(ids.as_mut_ptr(), ids.len() as u32) };
    count == 1
}

/// The console's font name ("Cascadia Mono"), if Windows says.
pub fn font_name() -> Option<String> {
    // SAFETY: `info` is a plain struct with its size set, as the call
    // requires; a handle that isn't a console makes it fail.
    unsafe {
        let mut info: CONSOLE_FONT_INFOEX = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<CONSOLE_FONT_INFOEX>() as u32;
        if GetCurrentConsoleFontEx(GetStdHandle(STD_OUTPUT_HANDLE), 0, &mut info) == 0 {
            return None;
        }
        let len = info
            .FaceName
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(info.FaceName.len());
        Some(String::from_utf16_lossy(&info.FaceName[..len]))
    }
}
