//! Masked secrets in note text (DESIGN §11.1): `{*value*}` is shown as six
//! dots. A secret is plain text with delimiters, so the file format doesn't
//! change and copying it into another note keeps it masked.
//!
//! Rules: one line, a value that isn't empty, closed by the first `*}`.
//! An unclosed `{*` is plain text, and `\{*` writes the delimiter itself.

use std::ops::Range;

pub const OPEN: &str = "{*";
pub const CLOSE: &str = "*}";
/// What a masked secret looks like, whatever its length.
pub const MASK: &str = "••••••";

/// The secrets and escapes of a text, as byte offsets, in order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Scan {
    /// Each secret with its delimiters: `{*value*}`.
    pub secrets: Vec<Range<usize>>,
    /// The `\` of each `\{*`.
    pub escapes: Vec<usize>,
}

/// The value inside a secret's range (without the delimiters).
pub fn value(secret: &Range<usize>) -> Range<usize> {
    secret.start + OPEN.len()..secret.end - CLOSE.len()
}

pub fn scan(text: &str) -> Scan {
    let mut out = Scan::default();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        // Byte tests: `i` may be inside a multi-byte char.
        if bytes[i] == b'\\' && bytes[i + 1..].starts_with(OPEN.as_bytes()) {
            out.escapes.push(i);
            i += 1 + OPEN.len();
            continue;
        }
        if !bytes[i..].starts_with(OPEN.as_bytes()) {
            i += 1;
            continue;
        }
        let from = i + OPEN.len();
        let line_end = text[from..].find('\n').map_or(text.len(), |n| from + n);
        match text[from..line_end].find(CLOSE) {
            Some(n) if n > 0 => {
                let end = from + n + CLOSE.len();
                out.secrets.push(i..end);
                i = end;
            }
            _ => i = from,
        }
    }
    out
}

pub fn contains_secret(text: &str) -> bool {
    text.contains(OPEN) && !scan(text).secrets.is_empty()
}

/// The text as it is shown masked: each secret becomes [`MASK`] and each
/// escape loses its `\`. For tab titles and search results.
pub fn mask(text: &str) -> String {
    if !text.contains(OPEN) {
        return text.to_owned();
    }
    let scan = scan(text);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    let mut escapes = scan.escapes.iter().peekable();
    for s in &scan.secrets {
        while let Some(&&e) = escapes.peek().filter(|&&&e| e < s.start) {
            out.push_str(&text[at..e]);
            at = e + 1;
            escapes.next();
        }
        out.push_str(&text[at..s.start]);
        out.push_str(MASK);
        at = s.end;
    }
    for &e in escapes {
        out.push_str(&text[at..e]);
        at = e + 1;
    }
    out.push_str(&text[at..]);
    out
}

/// Whether `value` can be a secret: not empty, one line, no `*}`.
pub fn can_wrap(value: &str) -> bool {
    !value.is_empty() && !value.contains('\n') && !value.contains(CLOSE)
}

/// `{*value*}`, if `value` can be a secret.
pub fn wrap(value: &str) -> Option<String> {
    can_wrap(value).then(|| format!("{OPEN}{value}{CLOSE}"))
}

/// Turns every secret of `text` into plain text by escaping its `{*`.
pub fn escape_all(text: &str) -> String {
    let scan = scan(text);
    let mut out = String::with_capacity(text.len() + scan.secrets.len());
    let mut at = 0;
    for s in &scan.secrets {
        out.push_str(&text[at..s.start]);
        out.push('\\');
        at = s.start;
    }
    out.push_str(&text[at..]);
    out
}

/// Prefixes of API keys and tokens that are secret by themselves.
const PREFIXES: &[&str] = &[
    "sk-",
    "sk_live_",
    "sk_test_",
    "rk_live_",
    "pk_live_",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "glpat-",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "xoxs-",
    "AKIA",
    "ASIA",
    "AIza",
    "SG.",
    "npm_",
    "hf_",
    "pypi-",
];

/// The paste detector (DESIGN §11.3): whether pasted text looks like a
/// password, an API key or a token. One word only, never a URL or a path.
///
/// `strict` (code mode) only accepts known prefixes and JWTs: code is full
/// of long identifiers with digits.
pub fn looks_like_secret(text: &str, strict: bool) -> bool {
    let s = text.trim();
    if s.len() < 12
        || s.len() > 2048
        || !s.chars().all(|c| c.is_ascii_graphic())
        || s.contains("://")
        || s.contains(OPEN)
    {
        return false;
    }
    let token_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    if s.len() >= 20 && PREFIXES.iter().any(|p| s.starts_with(p)) && s.chars().all(token_char) {
        return true;
    }
    if is_jwt(s) {
        return true;
    }
    if strict || s.len() < 16 || s.len() > 128 || s.starts_with(['/', '\\', '.', '~']) || s.contains('\\') {
        return false;
    }
    let upper = s.chars().any(|c| c.is_ascii_uppercase());
    let lower = s.chars().any(|c| c.is_ascii_lowercase());
    let digits = s.chars().filter(|c| c.is_ascii_digit()).count();
    let symbol = s
        .chars()
        .any(|c| c.is_ascii_punctuation() && !matches!(c, '-' | '_' | '.'));
    let mixed = digits >= 2 && ((upper && lower) || ((upper || lower) && symbol));
    // Identifiers and passphrases are made of words: long lower-case runs.
    let longest_word = s
        .split(|c: char| !c.is_ascii_lowercase())
        .map(str::len)
        .max()
        .unwrap_or(0);
    mixed && longest_word < 7 && entropy(s) >= 3.0
}

/// `header.payload.signature`, the first two being base64url JSON.
fn is_jwt(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 3
        && s.starts_with("eyJ")
        && parts[1].starts_with("eyJ")
        && parts.iter().all(|p| {
            p.len() >= 4
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

/// Shannon entropy in bits per character.
fn entropy(s: &str) -> f64 {
    let mut counts = [0u32; 128];
    for b in s.bytes() {
        counts[(b & 0x7f) as usize] += 1;
    }
    let n = s.len() as f64;
    counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secrets(text: &str) -> Vec<&str> {
        scan(text).secrets.iter().map(|r| &text[r.clone()]).collect()
    }

    #[test]
    fn finds_secrets() {
        assert_eq!(secrets("a {*x*} b {*y z*}"), ["{*x*}", "{*y z*}"]);
        assert_eq!(
            &"Key: {*Hx92!kLp*}"[value(&scan("Key: {*Hx92!kLp*}").secrets[0])],
            "Hx92!kLp"
        );
        // Closed by the first `*}`; a value may hold `{*` and end in `*`.
        assert_eq!(secrets("{*a{*b*}c*}"), ["{*a{*b*}"]);
        assert_eq!(secrets("{*a**}"), ["{*a**}"]);
        // Empty, unclosed or across lines: plain text.
        assert!(secrets("{**} {* x\ny*} {*open").is_empty());
        assert_eq!(secrets("{**}{*x*}"), ["{*x*}"]);
        assert_eq!(secrets("ñ{*é*}ñ"), ["{*é*}"]);
    }

    #[test]
    fn escapes() {
        let s = scan(r"\{*not*} {*yes*}");
        assert_eq!(s.escapes, [0]);
        assert_eq!(s.secrets, vec![9..16]);
        assert_eq!(mask(r"write \{*x*} for {*pw*}."), "write {*x*} for ••••••.");
        assert_eq!(escape_all("a {*b*} {*c*}"), r"a \{*b*} \{*c*}");
        assert!(scan(&escape_all("a {*b*} {*c*}")).secrets.is_empty());
    }

    #[test]
    fn masks_titles() {
        assert_eq!(mask("plain"), "plain");
        assert_eq!(mask("wifi {*hunter2*} home {*x*}"), "wifi •••••• home ••••••");
        assert_eq!(mask("{*a*}"), MASK);
    }

    #[test]
    fn wraps_values() {
        assert_eq!(wrap("pw").as_deref(), Some("{*pw*}"));
        assert_eq!(wrap(""), None);
        assert_eq!(wrap("a\nb"), None);
        assert_eq!(wrap("a*}b"), None);
        assert_eq!(secrets(&wrap("x*").unwrap()), ["{*x**}"]);
        assert!(contains_secret("k {*v*}"));
        assert!(!contains_secret("k {*v"));
    }

    #[test]
    fn detects_secrets() {
        for s in [
            "sk-proj-4fJk2LmN8pQrStUvWxYz0123",
            "ghp_aB3dE5fG7hI9jK1lM3nO5pQ7rS9tU1vW3xY5",
            "AKIAIOSFODNN7EXAMPLE",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
            "  x7Gk2pQ9vLm4Tz8R  ",
            "Hx92!kLp#Qw7zB4u",
        ] {
            assert!(looks_like_secret(s, false), "{s}");
        }
        for s in [
            "hello",
            "convertUTF8ToUTF16",
            "MyPassword2024Secure",
            "https://example.com/a1B2c3D4e5F6",
            "550e8400-e29b-41d4-a716-446655440000",
            "two words Abc123Def456Ghi",
            r"C:\Users\x7Gk2pQ9vLm4Tz8R",
            "/usr/lib/x7Gk2pQ9vLm4Tz8R",
            "AAAAAAAAaaaaaaaa11",
            "{*x7Gk2pQ9vLm4Tz8R*}",
        ] {
            assert!(!looks_like_secret(s, false), "{s}");
        }
        // Code mode: only the sure ones.
        assert!(!looks_like_secret("x7Gk2pQ9vLm4Tz8R", true));
        assert!(looks_like_secret("AKIAIOSFODNN7EXAMPLE", true));
    }
}
