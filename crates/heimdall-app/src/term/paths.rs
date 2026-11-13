//! Paths typed in the terminal: `~` for the home folder, and Tab
//! completion like a shell's.

use std::path::{Path, PathBuf};

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// `~/vaults/x.heim` → `/home/me/vaults/x.heim`.
pub fn expand_home(path: &Path) -> PathBuf {
    match (path.strip_prefix("~"), home()) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => path.to_owned(),
    }
}

/// `/home/me/vaults/x.heim` → `~/vaults/x.heim`.
pub fn short_path(path: &Path) -> String {
    match home().and_then(|h| path.strip_prefix(h).ok().map(Path::to_owned)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// What Tab can complete `typed` to: the entries of its folder whose
/// names start with what comes after the last `/`, written the way
/// `typed` is (a `~` stays a `~`). Folders end in `/`; files are listed
/// only if `keep` accepts them. Hidden entries only when the name typed
/// so far starts with a dot.
pub fn complete(typed: &str, keep: impl Fn(&Path) -> bool) -> Vec<String> {
    let (dir, prefix) = match typed.rfind('/') {
        Some(i) => typed.split_at(i + 1),
        None => ("", typed),
    };
    let real_dir = match dir {
        "" => PathBuf::from("."),
        d => expand_home(Path::new(d)),
    };
    let Ok(entries) = std::fs::read_dir(&real_dir) else {
        return Vec::new();
    };
    let mut found: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if !name.starts_with(prefix) || (name.starts_with('.') && !prefix.starts_with('.')) {
                return None;
            }
            let path = entry.path();
            if path.is_dir() {
                Some(format!("{dir}{name}/"))
            } else if keep(&path) {
                Some(format!("{dir}{name}"))
            } else {
                None
            }
        })
        .collect();
    found.sort();
    found
}

/// How a completion is listed: only its last part, like a shell.
/// `~/vaults/a.heim` → `a.heim`, `~/vaults/old/` → `old/`.
pub fn last_part(candidate: &str) -> &str {
    let trimmed = candidate.trim_end_matches('/');
    let start = trimmed.rfind('/').map_or(0, |i| i + 1);
    &candidate[start..]
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

    #[test]
    fn last_parts() {
        assert_eq!(last_part("~/vaults/a.heim"), "a.heim");
        assert_eq!(last_part("~/vaults/old/"), "old/");
        assert_eq!(last_part("plain"), "plain");
    }

    #[test]
    fn home_round_trip() {
        if let Some(h) = home() {
            let p = h.join("vaults/x.heim");
            assert_eq!(short_path(&p), "~/vaults/x.heim");
            assert_eq!(expand_home(Path::new("~/vaults/x.heim")), p);
        }
    }
}
