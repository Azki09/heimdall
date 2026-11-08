//! Plain text in and out of the vault (DESIGN §10.9): file names for
//! exported notes, what happens to masked secrets, the `.zip` for several
//! notes, and reading text files to import. No UI here.

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

use heimdall_core::content::{CodeMode, Note};
use heimdall_core::secret;
use zeroize::Zeroizing;

use crate::code;

/// A text file bigger than this isn't imported.
pub const MAX_IMPORT_BYTES: u64 = 10 * 1024 * 1024;

/// What a plain-text export does with masked secrets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Secrets {
    /// Each secret becomes `[secret]`.
    Hide,
    /// `{*value*}` as it is: readable, and masked again if imported.
    Keep,
    /// The value, without the markers.
    Reveal,
}

/// One exported file: its name and its text.
pub struct TextFile {
    pub name: String,
    pub text: Zeroizing<String>,
}

/// The files for `notes`, with names that are valid on Windows and unique
/// (case doesn't matter), and line endings as the editor setting says.
pub fn export_files(notes: &[&Note], secrets: Secrets, crlf: bool) -> Vec<TextFile> {
    let mut taken = HashSet::new();
    notes
        .iter()
        .map(|n| TextFile {
            name: unique(&file_name_for(n), &mut taken),
            text: note_text(n, secrets, crlf),
        })
        .collect()
}

/// The text of a note as it goes into the file.
pub fn note_text(note: &Note, secrets: Secrets, crlf: bool) -> Zeroizing<String> {
    let body = Zeroizing::new(match secrets {
        Secrets::Hide => secret::hide(&note.body),
        Secrets::Keep => note.body.clone(),
        Secrets::Reveal => secret::reveal(&note.body),
    });
    let lf = Zeroizing::new(body.replace("\r\n", "\n"));
    if crlf {
        Zeroizing::new(lf.replace('\n', "\r\n"))
    } else {
        lf
    }
}

/// `main.rs`, `Shopping.txt`, `notes.md`… The title never carries a
/// secret's value: secrets in it are written as `[secret]`.
pub fn file_name_for(note: &Note) -> String {
    let title = Zeroizing::new(secret::hide(note.display_title().trim()));
    let mut base = sanitize(&title);
    if base.is_empty() {
        base = "Untitled".into();
    }
    let ext = match &note.code {
        Some(c) => code::resolve(c, note)
            .and_then(|l| l.extension())
            .unwrap_or("txt"),
        None => "txt",
    };
    let lower = base.to_lowercase();
    let has_ext = lower.ends_with(&format!(".{ext}"))
        || (note.code.is_none() && (lower.ends_with(".txt") || lower.ends_with(".md")));
    if has_ext {
        base
    } else {
        format!("{base}.{ext}")
    }
}

/// A Windows file name: no `<>:"/\|?*` or control characters, no trailing
/// dot or space, not a reserved device name, at most 100 characters.
fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(100)
        .collect();
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']).to_owned();
    let stem = cleaned.split('.').next().unwrap_or("").trim().to_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        format!("_{cleaned}")
    } else {
        cleaned
    }
}

/// `name`, or `name (2).ext`, `name (3).ext`… if it is taken.
fn unique(name: &str, taken: &mut HashSet<String>) -> String {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    let mut candidate = name.to_owned();
    let mut n = 2;
    while !taken.insert(candidate.to_lowercase()) {
        candidate = format!("{stem} ({n}){ext}");
        n += 1;
    }
    candidate
}

/// A `.zip` with one file per note, in memory: deflate, UTF-8 names, and
/// the fixed 1980-01-01 date instead of the notes' dates.
///
/// Written by hand (local headers, central directory, end record; no
/// ZIP64) rather than with the `zip` crate: Heimdall only ever writes
/// archives, and that crate's open advisories are all about extracting.
pub fn zip(files: &[TextFile]) -> Result<Zeroizing<Vec<u8>>, String> {
    const DOS_DATE: u16 = 0x21; // 1980-01-01
    const UTF8_NAMES: u16 = 1 << 11;
    const DEFLATE: u16 = 8;
    let too_big = || "The notes are too big for a .zip.".to_owned();

    let mut out = Zeroizing::new(Vec::new());
    let mut central = Vec::new();
    for f in files {
        let data = f.text.as_bytes();
        let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(data)
            .map_err(|e| format!("Couldn't build the .zip: {e}"))?;
        let packed = Zeroizing::new(
            enc.finish()
                .map_err(|e| format!("Couldn't build the .zip: {e}"))?,
        );
        let crc = crc32fast::hash(data);
        let size = u32::try_from(data.len()).map_err(|_| too_big())?;
        let packed_size = u32::try_from(packed.len()).map_err(|_| too_big())?;
        let name = f.name.as_bytes();
        let name_len = u16::try_from(name.len()).map_err(|_| too_big())?;
        let offset = u32::try_from(out.len()).map_err(|_| too_big())?;

        // Fields shared by the local header and the central directory entry.
        let mut common = Vec::with_capacity(26);
        for v in [20u16, UTF8_NAMES, DEFLATE, 0, DOS_DATE] {
            common.extend_from_slice(&v.to_le_bytes());
        }
        for v in [crc, packed_size, size] {
            common.extend_from_slice(&v.to_le_bytes());
        }
        common.extend_from_slice(&name_len.to_le_bytes());
        common.extend_from_slice(&0u16.to_le_bytes()); // extra field length

        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&common);
        out.extend_from_slice(name);
        out.extend_from_slice(&packed);

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // made by
        central.extend_from_slice(&common);
        for v in [0u16, 0, 0] {
            // comment length, disk number, internal attributes
            central.extend_from_slice(&v.to_le_bytes());
        }
        central.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
    }
    let count = u16::try_from(files.len()).map_err(|_| too_big())?;
    let central_offset = u32::try_from(out.len()).map_err(|_| too_big())?;
    let central_size = u32::try_from(central.len()).map_err(|_| too_big())?;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    for v in [0u16, 0, count, count] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length
    Ok(out)
}

/// Reads a text file as a new note: the file name is the title, and a file
/// of a code-mode language opens in code mode. UTF-8 only (a BOM is
/// dropped); line endings become `\n`.
pub fn read_text_file(path: &Path) -> Result<Note, String> {
    let name = crate::support::file_name(path);
    let len = std::fs::metadata(path).map_err(|e| format!("{name}: {e}"))?.len();
    if len > MAX_IMPORT_BYTES {
        return Err(format!("{name} is bigger than 10 MB"));
    }
    let bytes = Zeroizing::new(std::fs::read(path).map_err(|e| format!("{name}: {e}"))?);
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let text = Zeroizing::new(
        std::str::from_utf8(bytes)
            .map_err(|_| format!("{name} isn't UTF-8 text"))?
            .replace("\r\n", "\n"),
    );
    let mut note = Note::new(heimdall_core::content::now());
    note.title = name;
    note.body = text.to_string();
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if ext != "txt" {
        note.code = code::by_extension(&ext).map(|l| CodeMode {
            language: l.id.to_owned(),
        });
    }
    Ok(note)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(title: &str, body: &str, lang: Option<&str>) -> Note {
        let mut n = Note::new(0);
        n.title = title.into();
        n.body = body.into();
        n.code = lang.map(|l| CodeMode { language: l.into() });
        n
    }

    #[test]
    fn file_names() {
        assert_eq!(file_name_for(&note("Shopping", "", None)), "Shopping.txt");
        assert_eq!(file_name_for(&note("notes.md", "", None)), "notes.md");
        assert_eq!(file_name_for(&note("main.rs", "", Some(""))), "main.rs");
        assert_eq!(file_name_for(&note("script", "", Some("python"))), "script.py");
        assert_eq!(file_name_for(&note("README", "", Some("markdown"))), "README.md");
        assert_eq!(file_name_for(&note("odd", "", Some(""))), "odd.txt");
        assert_eq!(file_name_for(&note("", "", None)), "Untitled.txt");
        assert_eq!(
            file_name_for(&note("", "\n  First line\nx", None)),
            "First line.txt"
        );
        assert_eq!(file_name_for(&note("a/b:c*d?", "", None)), "a_b_c_d_.txt");
        assert_eq!(file_name_for(&note("trailing. ", "", None)), "trailing.txt");
        assert_eq!(file_name_for(&note("con", "", None)), "_con.txt");
        assert_eq!(file_name_for(&note("COM1.log", "", None)), "_COM1.log.txt");
        assert_eq!(file_name_for(&note("Console", "", None)), "Console.txt");
        // A secret in the title never reaches the file name.
        assert_eq!(
            file_name_for(&note("pw {*hunter2*}", "", None)),
            "pw [secret].txt"
        );
    }

    #[test]
    fn names_are_unique_ignoring_case() {
        let a = note("Shopping", "1", None);
        let b = note("shopping", "2", None);
        let c = note("Shopping", "3", None);
        let files = export_files(&[&a, &b, &c], Secrets::Keep, false);
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Shopping.txt", "shopping (2).txt", "Shopping (3).txt"]);
    }

    #[test]
    fn secrets_and_line_endings() {
        let n = note("x", "pw: {*hunter2*}\r\nnext\nlast", None);
        assert_eq!(&*note_text(&n, Secrets::Hide, false), "pw: [secret]\nnext\nlast");
        assert_eq!(
            &*note_text(&n, Secrets::Keep, false),
            "pw: {*hunter2*}\nnext\nlast"
        );
        assert_eq!(
            &*note_text(&n, Secrets::Reveal, true),
            "pw: hunter2\r\nnext\r\nlast"
        );
    }

    #[test]
    fn zip_holds_every_file() {
        let a = note("a", "first {*s*}", None);
        let b = note("b", "second", Some("rust"));
        let files = export_files(&[&a, &b], Secrets::Hide, false);
        let bytes = zip(&files).unwrap();
        let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
        let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;

        // End record: two entries, and where the central directory starts.
        let end = bytes.len() - 22;
        assert_eq!(u32_at(end), 0x0605_4b50);
        assert_eq!((u16_at(end + 8), u16_at(end + 10)), (2, 2));
        let mut at = u32_at(end + 16);
        let mut seen = Vec::new();
        for _ in 0..2 {
            assert_eq!(u32_at(at), 0x0201_4b50);
            let packed = u32_at(at + 20);
            let crc = u32_at(at + 16) as u32;
            let name_len = u16_at(at + 28);
            let name = std::str::from_utf8(&bytes[at + 46..at + 46 + name_len]).unwrap();
            // The local header it points to, then the deflated data.
            let local = u32_at(at + 42);
            assert_eq!(u32_at(local), 0x0403_4b50);
            let data = local + 30 + u16_at(local + 26) + u16_at(local + 28);
            let mut text = String::new();
            std::io::Read::read_to_string(
                &mut flate2::read::DeflateDecoder::new(&bytes[data..data + packed]),
                &mut text,
            )
            .unwrap();
            assert_eq!(crc32fast::hash(text.as_bytes()), crc);
            seen.push((name.to_owned(), text));
            at += 46 + name_len;
        }
        assert_eq!(
            seen,
            [
                ("a.txt".to_owned(), "first [secret]".to_owned()),
                ("b.rs".to_owned(), "second".to_owned())
            ]
        );
    }

    #[test]
    fn reads_text_files() {
        let dir = std::env::temp_dir().join(format!("heimdall-plain-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let txt = dir.join("Notes.txt");
        std::fs::write(&txt, b"\xEF\xBB\xBFline 1\r\npw {*x*}\r\n").unwrap();
        let rs = dir.join("main.RS");
        std::fs::write(&rs, b"fn main() {}\n").unwrap();
        let bin = dir.join("blob.txt");
        std::fs::write(&bin, [0xFF, 0xFE, 0x00, 0x41]).unwrap();

        let n = read_text_file(&txt).unwrap();
        assert_eq!(n.title, "Notes.txt");
        assert_eq!(n.body, "line 1\npw {*x*}\n");
        assert!(n.code.is_none());
        let n = read_text_file(&rs).unwrap();
        assert_eq!(n.code.as_ref().unwrap().language, "rust");
        assert!(read_text_file(&bin).unwrap_err().contains("isn't UTF-8"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
