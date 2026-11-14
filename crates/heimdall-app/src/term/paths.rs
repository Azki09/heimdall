//! Paths typed in the terminal: `~` for the home folder, and Tab
//! completion like a shell's. On Windows also `\`, drive letters and
//! `%USERPROFILE%`-style variables, with names compared ignoring case
//! (DESIGN §17.6).

use std::path::{Path, PathBuf};

fn home() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var).filter(|h| !h.is_empty()).map(PathBuf::from)
}

fn is_sep(c: char) -> bool {
    c == '/' || (cfg!(windows) && c == '\\')
}

/// `~/vaults/x.heim` → `/home/me/vaults/x.heim`. On Windows also
/// `%USERPROFILE%\x.heim` (any variable, at the start).
pub fn expand_home(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        if let Some(rest) = text.strip_prefix('%') {
            if let Some((name, rest)) = rest.split_once('%') {
                if let Some(value) = std::env::var_os(name).filter(|v| !v.is_empty()) {
                    let rest = rest.trim_start_matches(is_sep);
                    return PathBuf::from(value).join(rest);
                }
            }
        }
    }
    let mut chars = text.chars();
    if chars.next() == Some('~') {
        let rest = chars.as_str();
        if rest.is_empty() || rest.starts_with(is_sep) {
            if let Some(home) = home() {
                return home.join(rest.trim_start_matches(is_sep));
            }
        }
    }
    path.to_owned()
}

/// How a path is shown. Linux: `/home/me/vaults/x.heim` → `~/vaults/x.heim`.
/// Windows: the full path, as Windows programs show it.
pub fn short_path(path: &Path) -> String {
    if cfg!(windows) {
        let shown = path.display().to_string();
        // What canonicalize() adds; nobody types it.
        return shown.strip_prefix(r"\\?\").map(str::to_owned).unwrap_or(shown);
    }
    match home().and_then(|h| path.strip_prefix(h).ok().map(Path::to_owned)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// What Tab can complete `typed` to: the entries of its folder whose
/// names start with what comes after the last `/` (or `\` on Windows),
/// written the way `typed` is (a `~` stays a `~`). Folders end in the
/// separator typed last (`\` on Windows if none); files are listed only if
/// `keep` accepts them. Hidden entries only when the name typed so far
/// starts with a dot. Windows: names ignore case; network paths
/// (`\\server\share`) are not completed, it would be slow.
pub fn complete(typed: &str, keep: impl Fn(&Path) -> bool) -> Vec<String> {
    if cfg!(windows) {
        if typed.starts_with(r"\\") || typed.starts_with("//") {
            return Vec::new();
        }
        // `D:` → `D:\`
        let bytes = typed.as_bytes();
        if bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            return vec![format!("{typed}\\")];
        }
    }
    let (dir, prefix) = match typed.rfind(is_sep) {
        Some(i) => typed.split_at(i + 1),
        None => ("", typed),
    };
    let sep = match dir.chars().last() {
        Some(c) => c,
        None if cfg!(windows) => '\\',
        None => '/',
    };
    let real_dir = match dir {
        "" => PathBuf::from("."),
        d => expand_home(Path::new(d)),
    };
    let Ok(entries) = std::fs::read_dir(&real_dir) else {
        return Vec::new();
    };
    let starts = |name: &str| {
        if cfg!(windows) {
            name.to_lowercase().starts_with(&prefix.to_lowercase())
        } else {
            name.starts_with(prefix)
        }
    };
    let mut found: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if !starts(&name) || (name.starts_with('.') && !prefix.starts_with('.')) {
                return None;
            }
            let path = entry.path();
            if path.is_dir() {
                Some(format!("{dir}{name}{sep}"))
            } else if keep(&path) {
                Some(format!("{dir}{name}"))
            } else {
                None
            }
        })
        .collect();
    found.sort_by_key(|f| f.to_lowercase());
    found
}

/// How a completion is listed: only its last part, like a shell.
/// `~/vaults/a.heim` → `a.heim`, `~/vaults/old/` → `old/`.
pub fn last_part(candidate: &str) -> &str {
    let trimmed = candidate.trim_end_matches(is_sep);
    let start = trimmed.rfind(is_sep).map_or(0, |i| i + 1);
    &candidate[start..]
}

/// Whether a line typed at a question is a path (its candidates are
/// listed by their last part).
pub fn looks_like_path(line: &str) -> bool {
    line.contains(is_sep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completes_names_in_a_folder() {
        let root = std::env::temp_dir().join(format!("heimdall-paths-{}", std::process::id()));
        std::fs::create_dir_all(root.join("vaults")).unwrap();
        std::fs::write(root.join("personal.heim"), b"").unwrap();
        std::fs::write(root.join("notes.txt"), b"").unwrap();
        std::fs::write(root.join(".hidden.heim"), b"").unwrap();
        let base = format!("{}/", root.display());
        let heim = |p: &Path| p.extension().is_some_and(|e| e == "heim");

        let all = complete(&base, heim);
        assert_eq!(
            all,
            vec![format!("{base}personal.heim"), format!("{base}vaults/")]
        );
        assert_eq!(
            complete(&format!("{base}p"), heim),
            vec![format!("{base}personal.heim")]
        );
        assert_eq!(
            complete(&format!("{base}."), heim),
            vec![format!("{base}.hidden.heim")]
        );
        assert_eq!(
            complete(&format!("{base}n"), |_| true),
            vec![format!("{base}notes.txt")]
        );
        assert!(complete(&format!("{base}zz"), heim).is_empty());
        assert!(complete("/no/such/folder/", heim).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn completes_windows_paths() {
        let root = std::env::temp_dir().join(format!("heimdall-wpaths-{}", std::process::id()));
        std::fs::create_dir_all(root.join("Bovedas")).unwrap();
        std::fs::write(root.join("Personal.heim"), b"").unwrap();
        let base = format!("{}\\", root.display());
        let heim = |p: &Path| p.extension().is_some_and(|e| e == "heim");

        // Backslashes, and names ignore case; folders end in `\`.
        assert_eq!(
            complete(&format!("{base}b"), heim),
            vec![format!("{base}Bovedas\\")]
        );
        assert_eq!(
            complete(&format!("{base}PER"), heim),
            vec![format!("{base}Personal.heim")]
        );
        // A forward slash keeps being one.
        let fwd = format!("{}/", root.display()).replace('\\', "/");
        assert_eq!(
            complete(&format!("{fwd}bov"), heim),
            vec![format!("{fwd}Bovedas/")]
        );
        assert_eq!(complete("c:", heim), vec!["c:\\".to_owned()]);
        assert!(complete(r"\\server\share\x", heim).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn last_parts() {
        assert_eq!(last_part("~/vaults/a.heim"), "a.heim");
        assert_eq!(last_part("~/vaults/old/"), "old/");
        assert_eq!(last_part("plain"), "plain");
        if cfg!(windows) {
            assert_eq!(last_part(r"C:\Users\ana\old\"), r"old\");
        }
    }

    #[test]
    fn home_round_trip() {
        if let Some(h) = home() {
            let p = h.join("vaults").join("x.heim");
            if cfg!(windows) {
                assert_eq!(short_path(&p), p.display().to_string());
                assert_eq!(expand_home(Path::new(r"~\vaults\x.heim")), p);
                assert_eq!(expand_home(Path::new(r"%USERPROFILE%\vaults\x.heim")), p);
            } else {
                assert_eq!(short_path(&p), "~/vaults/x.heim");
            }
            assert_eq!(expand_home(Path::new("~/vaults/x.heim")), h.join("vaults/x.heim"));
        }
    }
}
