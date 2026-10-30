//! Printing the recovery sheet straight to a printer with GDI (DESIGN §7).
//! The app writes no file: the page goes to the Windows spooler. Every
//! `unsafe` block is a plain Win32 call on handles and buffers owned here.
#![allow(unsafe_code)]

use std::ptr;

use windows_sys::Win32::Foundation::{GetLastError, GlobalFree, ERROR_CANCELLED, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, CreatePen, DeleteDC, DeleteObject, GetDeviceCaps, GetStockObject, GetTextExtentPoint32W,
    RoundRect, SelectObject, SetBkMode, SetTextColor, TextOutW, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
    DEFAULT_CHARSET, DEFAULT_PITCH, FF_DONTCARE, FW_NORMAL, FW_SEMIBOLD, HDC, HFONT, HGDIOBJ, HORZRES,
    LOGPIXELSX, LOGPIXELSY, NULL_BRUSH, OUT_DEFAULT_PRECIS, PS_SOLID, TRANSPARENT, VERTRES,
};
use windows_sys::Win32::Storage::Xps::{AbortDoc, EndDoc, EndPage, StartDocW, StartPage, DOCINFOW};
use windows_sys::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, PrintDlgW, PD_HIDEPRINTTOFILE, PD_NOPAGENUMS, PD_NOSELECTION, PD_RETURNDC,
    PRINTDLGW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;

use crate::{PrintOutcome, RecoverySheet};

pub fn print_recovery_sheet(sheet: &RecoverySheet) -> PrintOutcome {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("HEIMDALL_PRINT_TO") {
        return print_to_pdf(sheet, &path);
    }

    // SAFETY: PRINTDLGW is a plain C struct; all-zero is its documented
    // empty state (null handles, no hooks, no templates).
    let mut pd: PRINTDLGW = unsafe { std::mem::zeroed() };
    pd.lStructSize = size_of::<PRINTDLGW>() as u32;
    // SAFETY: no arguments; returns this thread's active window or null.
    pd.hwndOwner = unsafe { GetActiveWindow() };
    pd.Flags = PD_RETURNDC | PD_NOPAGENUMS | PD_NOSELECTION | PD_HIDEPRINTTOFILE;
    pd.nCopies = 1;
    // SAFETY: `pd` is initialized above and outlives the (modal) call.
    let ok = unsafe { PrintDlgW(&mut pd) };
    // SAFETY: the dialog allocated these (or left them null); we own them now
    // and free each once.
    unsafe {
        if !pd.hDevMode.is_null() {
            GlobalFree(pd.hDevMode);
        }
        if !pd.hDevNames.is_null() {
            GlobalFree(pd.hDevNames);
        }
    }
    if ok == 0 {
        // SAFETY: no arguments.
        let err = unsafe { CommDlgExtendedError() };
        return if err == 0 {
            PrintOutcome::Canceled
        } else {
            PrintOutcome::Failed(format!("The print dialog failed (error {err:#x})."))
        };
    }
    if pd.hDC.is_null() {
        return PrintOutcome::Failed("No printer was selected.".into());
    }
    let outcome = print_page(pd.hDC, sheet, None);
    // SAFETY: the dialog created this device context for us; deleted once.
    unsafe {
        DeleteDC(pd.hDC);
    }
    outcome
}

/// Debug builds only: `HEIMDALL_PRINT_TO=<file.pdf>` prints to "Microsoft
/// Print to PDF" without any dialog, so the sheet can be checked in tests.
#[cfg(debug_assertions)]
fn print_to_pdf(sheet: &RecoverySheet, path: &std::ffi::OsStr) -> PrintOutcome {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Graphics::Gdi::CreateDCW;
    let device = wide("Microsoft Print to PDF");
    let output: Vec<u16> = path.encode_wide().chain(Some(0)).collect();
    // SAFETY: `device` is NUL-terminated; driver, port and devmode are optional.
    let hdc = unsafe { CreateDCW(ptr::null(), device.as_ptr(), ptr::null(), ptr::null()) };
    if hdc.is_null() {
        return PrintOutcome::Failed("Microsoft Print to PDF isn't installed.".into());
    }
    let outcome = print_page(hdc, sheet, Some(&output));
    // SAFETY: created above; deleted once.
    unsafe {
        DeleteDC(hdc);
    }
    outcome
}

fn print_page(hdc: HDC, sheet: &RecoverySheet, output: Option<&[u16]>) -> PrintOutcome {
    let name = wide("Heimdall recovery code");
    let doc = DOCINFOW {
        cbSize: size_of::<DOCINFOW>() as i32,
        lpszDocName: name.as_ptr(),
        lpszOutput: output.map_or(ptr::null(), |o| o.as_ptr()),
        lpszDatatype: ptr::null(),
        fwType: 0,
    };
    // SAFETY: `hdc` is a printer DC; `doc` and the strings it points to
    // outlive the call.
    if unsafe { StartDocW(hdc, &doc) } <= 0 {
        // SAFETY: no arguments.
        let err = unsafe { GetLastError() };
        // A "Save as" printer (PDF, XPS) whose file dialog was canceled.
        return if err == ERROR_CANCELLED {
            PrintOutcome::Canceled
        } else {
            PrintOutcome::Failed(format!("The printer didn't accept the page (error {err})."))
        };
    }
    // SAFETY: a document was started on `hdc` above.
    if unsafe { StartPage(hdc) } <= 0 {
        // SAFETY: as above.
        unsafe { AbortDoc(hdc) };
        return PrintOutcome::Failed("The printer didn't accept the page.".into());
    }
    draw(hdc, sheet);
    // SAFETY: a page and a document are open on `hdc`.
    let ok = unsafe { EndPage(hdc) > 0 && EndDoc(hdc) > 0 };
    if ok {
        PrintOutcome::Printed
    } else {
        // SAFETY: as above.
        unsafe { AbortDoc(hdc) };
        PrintOutcome::Failed("Printing failed.".into())
    }
}

const BLACK: u32 = 0x0014_1414;
const GREY: u32 = 0x0055_5555;
const LIGHT: u32 = 0x0088_8888;
const RULE: u32 = 0x00B0_B0B0;

/// Lays out the sheet: title, vault and date, the 12 words in a box, and
/// what to do with the sheet.
fn draw(hdc: HDC, sheet: &RecoverySheet) {
    // SAFETY: plain queries on a valid DC.
    let (dpi_x, dpi_y, page_w, page_h) = unsafe {
        (
            GetDeviceCaps(hdc, LOGPIXELSX as i32),
            GetDeviceCaps(hdc, LOGPIXELSY as i32),
            GetDeviceCaps(hdc, HORZRES as i32),
            GetDeviceCaps(hdc, VERTRES as i32),
        )
    };
    let x_pt = |pt: f32| (pt * dpi_x as f32 / 72.0).round() as i32;
    let y_pt = |pt: f32| (pt * dpi_y as f32 / 72.0).round() as i32;
    let mut g = Gdi::new(hdc, dpi_y);

    // The printable area already excludes the printer's own margins.
    let left = x_pt(40.0);
    let right = page_w - x_pt(40.0);
    let mut y = y_pt(36.0);

    g.text(left, y, "HEIMDALL", 9.0, FW_SEMIBOLD, UI, LIGHT);
    y += y_pt(20.0);
    g.text(left, y, "Recovery code", 22.0, FW_SEMIBOLD, UI, BLACK);
    y += y_pt(36.0);
    g.text(
        left,
        y,
        &format!("Vault: {}    Printed: {}", sheet.vault, sheet.date),
        10.0,
        FW_NORMAL,
        UI,
        GREY,
    );
    y += y_pt(28.0);

    // The words, numbered down each of three columns like on screen.
    let row_h = y_pt(36.0);
    let pad = y_pt(14.0);
    let box_h = 4 * row_h + 2 * pad;
    g.rounded_box(left, y, right, y + box_h, x_pt(8.0), RULE);
    let col_w = (right - left - 2 * x_pt(20.0)) / 3;
    for (i, word) in sheet.words.iter().enumerate() {
        let col = (i / 4) as i32;
        let row = (i % 4) as i32;
        let x = left + x_pt(20.0) + col * col_w;
        let cell_top = y + pad + row * row_h;
        let number = (i + 1).to_string();
        let (nw, nh) = g.measure(&number, 10.0, FW_NORMAL, UI);
        g.text(
            x + x_pt(18.0) - nw,
            cell_top + (row_h - nh) / 2,
            &number,
            10.0,
            FW_NORMAL,
            UI,
            LIGHT,
        );
        let (_, wh) = g.measure("Mg", 16.0, FW_NORMAL, MONO);
        g.text(
            x + x_pt(28.0),
            cell_top + (row_h - wh) / 2,
            word,
            16.0,
            FW_NORMAL,
            MONO,
            BLACK,
        );
    }
    y += box_h + y_pt(26.0);

    for (lead, rest) in [
        (
            "This code opens the vault without your password.",
            " Keep it like a passport.",
        ),
        (
            "Anyone with this sheet and the vault file can read your notes.",
            "",
        ),
        (
            "Store it away from the computer.",
            " Don't photograph it or keep a copy as a file.",
        ),
        (
            "A new recovery code replaces this one.",
            " Then destroy this sheet.",
        ),
    ] {
        g.text(left, y, "—", 10.5, FW_NORMAL, UI, LIGHT);
        let x = left + x_pt(18.0);
        let (lw, _) = g.measure(lead, 10.5, FW_SEMIBOLD, UI);
        g.text(x, y, lead, 10.5, FW_SEMIBOLD, UI, BLACK);
        g.text(x + lw, y, rest, 10.5, FW_NORMAL, UI, GREY);
        y += y_pt(22.0);
    }

    g.text(
        left,
        page_h - y_pt(36.0),
        "No signal. No trace.",
        9.0,
        FW_NORMAL,
        UI,
        LIGHT,
    );
}

const UI: &str = "Segoe UI";
const MONO: &str = "Consolas";

/// Text and lines on a DC, with the fonts it created freed on drop.
struct Gdi {
    hdc: HDC,
    dpi_y: i32,
    fonts: Vec<HFONT>,
    original_font: Option<HGDIOBJ>,
}

impl Gdi {
    fn new(hdc: HDC, dpi_y: i32) -> Self {
        // SAFETY: valid DC; the mode is a plain flag.
        unsafe {
            SetBkMode(hdc, TRANSPARENT as i32);
        }
        Self {
            hdc,
            dpi_y,
            fonts: Vec::new(),
            original_font: None,
        }
    }

    fn select_font(&mut self, size_pt: f32, weight: u32, face: &str) {
        let face_w = wide(face);
        let height = -(size_pt * self.dpi_y as f32 / 72.0).round() as i32;
        // SAFETY: `face_w` is NUL-terminated and outlives the call; the
        // other arguments are plain values.
        let font = unsafe {
            CreateFontW(
                height,
                0,
                0,
                0,
                weight as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                (DEFAULT_PITCH | FF_DONTCARE) as u32,
                face_w.as_ptr(),
            )
        };
        // SAFETY: `font` was just created (or is null, which SelectObject rejects).
        let previous = unsafe { SelectObject(self.hdc, font) };
        if self.original_font.is_none() {
            self.original_font = Some(previous);
        }
        self.fonts.push(font);
    }

    fn measure(&mut self, text: &str, size_pt: f32, weight: u32, face: &str) -> (i32, i32) {
        self.select_font(size_pt, weight, face);
        let mut w = wide_unterminated(text);
        let mut size = SIZE { cx: 0, cy: 0 };
        // SAFETY: `w` holds `w.len()` UTF-16 units; `size` is a valid out-pointer.
        unsafe {
            GetTextExtentPoint32W(self.hdc, w.as_ptr(), w.len() as i32, &mut size);
        }
        wipe(&mut w);
        (size.cx, size.cy)
    }

    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, x: i32, y: i32, text: &str, size_pt: f32, weight: u32, face: &str, color: u32) {
        if text.is_empty() {
            return;
        }
        self.select_font(size_pt, weight, face);
        let mut w = wide_unterminated(text);
        // SAFETY: valid DC; `w` holds `w.len()` UTF-16 units for the call.
        unsafe {
            SetTextColor(self.hdc, color);
            TextOutW(self.hdc, x, y, w.as_ptr(), w.len() as i32);
        }
        // The words of the code pass through here.
        wipe(&mut w);
    }

    fn rounded_box(&mut self, left: i32, top: i32, right: i32, bottom: i32, radius: i32, color: u32) {
        let width = (self.dpi_y / 96).max(1);
        // SAFETY: plain GDI object creation and selection on a valid DC; the
        // pen is deselected and deleted before returning.
        unsafe {
            let pen = CreatePen(PS_SOLID, width, color);
            let old_pen = SelectObject(self.hdc, pen);
            let old_brush = SelectObject(self.hdc, GetStockObject(NULL_BRUSH));
            RoundRect(self.hdc, left, top, right, bottom, radius, radius);
            SelectObject(self.hdc, old_brush);
            SelectObject(self.hdc, old_pen);
            DeleteObject(pen);
        }
    }
}

impl Drop for Gdi {
    fn drop(&mut self) {
        // SAFETY: restores the DC's own font, then deletes each font we
        // created exactly once (none is selected any more).
        unsafe {
            if let Some(f) = self.original_font {
                SelectObject(self.hdc, f);
            }
            for f in self.fonts.drain(..) {
                if !f.is_null() {
                    DeleteObject(f);
                }
            }
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn wide_unterminated(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn wipe(buf: &mut [u16]) {
    buf.fill(0);
    std::hint::black_box(buf);
}
