//! Finding text in a note (Ctrl+F) and in every note (Ctrl+Shift+F),
//! DESIGN §10.5. Plain substring search; "Match case" off folds each
//! character to lower case one-to-one, so positions stay exact.

/// Stop after this many matches in one note.
pub const MAX_MATCHES: usize = 10_000;

/// One occurrence, as char indices (for the cursor) and byte offsets (for
/// slicing the text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub chars: (usize, usize),
    pub bytes: (usize, usize),
}

fn fold(c: char, match_case: bool) -> char {
    if match_case {
        c
    } else {
        c.to_lowercase().next().unwrap_or(c)
    }
}

/// Every non-overlapping occurrence of `needle` in `text`, in order.
pub fn find_all(text: &str, needle: &str, match_case: bool) -> Vec<Match> {
    let needle: Vec<char> = needle.chars().map(|c| fold(c, match_case)).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let hay: Vec<(usize, char)> = text
        .char_indices()
        .map(|(b, c)| (b, fold(c, match_case)))
        .collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + needle.len() <= hay.len() && out.len() < MAX_MATCHES {
        if hay[i].1 == needle[0]
            && hay[i..i + needle.len()]
                .iter()
                .map(|(_, c)| *c)
                .eq(needle.iter().copied())
        {
            let end = i + needle.len();
            let end_byte = hay.get(end).map_or(text.len(), |(b, _)| *b);
            out.push(Match {
                chars: (i, end),
                bytes: (hay[i].0, end_byte),
            });
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// The match to go to from the cursor: the first one starting at or after
/// it, wrapping to the first.
pub fn first_from(matches: &[Match], cursor: usize) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    Some(matches.iter().position(|m| m.chars.0 >= cursor).unwrap_or(0))
}

/// The match to go back to from the cursor: the last one starting before
/// it, wrapping to the last.
pub fn last_before(matches: &[Match], cursor: usize) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    Some(
        matches
            .iter()
            .rposition(|m| m.chars.0 < cursor)
            .unwrap_or(matches.len() - 1),
    )
}

/// One line of an all-notes result: its number, and the text around the
/// match with the match's byte range inside that text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub line: usize,
    pub text: String,
    pub highlight: (usize, usize),
}

/// Show at most this much of a line around a match.
const PREVIEW_CHARS: usize = 80;

pub fn preview(text: &str, m: &Match) -> Preview {
    let line_start = text[..m.bytes.0].rfind('\n').map_or(0, |i| i + 1);
    let line_end = text[m.bytes.0..].find('\n').map_or(text.len(), |i| m.bytes.0 + i);
    let line = text[..m.bytes.0].matches('\n').count() + 1;
    let match_end = m.bytes.1.min(line_end);
    // Keep some context before the match, then cut the line to size.
    let before: Vec<(usize, char)> = text[line_start..m.bytes.0].char_indices().collect();
    let skip = before.len().saturating_sub(24);
    let mut start = line_start + before.get(skip).map_or(m.bytes.0 - line_start, |(b, _)| *b);
    // Leading indentation says nothing.
    while start < m.bytes.0 && text[start..].starts_with([' ', '\t']) {
        start += 1;
    }
    let end = text[start..line_end]
        .char_indices()
        .nth(PREVIEW_CHARS)
        .map_or(line_end, |(b, _)| start + b)
        .max(match_end);
    let prefix = if skip > 0 { "…" } else { "" };
    let suffix = if end < line_end { "…" } else { "" };
    Preview {
        line,
        text: format!("{prefix}{}{suffix}", &text[start..end]),
        highlight: (prefix.len() + m.bytes.0 - start, prefix.len() + match_end - start),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_every_match_in_order() {
        let m = find_all("abcabcab", "ab", true);
        assert_eq!(
            m.iter().map(|m| m.chars).collect::<Vec<_>>(),
            [(0, 2), (3, 5), (6, 8)]
        );
        assert!(find_all("abc", "", false).is_empty());
        assert!(find_all("ab", "abc", false).is_empty());
        // Non-overlapping.
        assert_eq!(find_all("aaaa", "aa", true).len(), 2);
    }

    #[test]
    fn case_folding_keeps_positions() {
        let text = "Ñandú y ÑANDÚ, café";
        let m = find_all(text, "ñandú", false);
        assert_eq!(m.len(), 2);
        assert_eq!(&text[m[1].bytes.0..m[1].bytes.1], "ÑANDÚ");
        assert_eq!(m[1].chars, (8, 13));
        assert_eq!(find_all(text, "ñandú", true).len(), 0);
        let m = find_all(text, "CAFÉ", false);
        assert_eq!(&text[m[0].bytes.0..m[0].bytes.1], "café");
    }

    #[test]
    fn next_match_from_cursor_wraps() {
        let m = find_all("x.x.x", "x", true);
        assert_eq!(first_from(&m, 0), Some(0));
        assert_eq!(first_from(&m, 1), Some(1));
        assert_eq!(first_from(&m, 5), Some(0));
        assert_eq!(first_from(&[], 0), None);
    }

    #[test]
    fn previous_match_from_cursor_wraps() {
        let m = find_all("x.x.x", "x", true);
        assert_eq!(last_before(&m, 5), Some(2));
        assert_eq!(last_before(&m, 2), Some(0));
        assert_eq!(last_before(&m, 0), Some(2));
        assert_eq!(last_before(&[], 0), None);
    }

    #[test]
    fn preview_shows_the_line_with_the_match() {
        let text = "first\n    let token = read();\nlast";
        let m = find_all(text, "token", true)[0];
        let p = preview(text, &m);
        assert_eq!(p.line, 2);
        assert_eq!(p.text, "let token = read();");
        assert_eq!(&p.text[p.highlight.0..p.highlight.1], "token");

        let long = format!("{}needle{}", "a".repeat(100), "b".repeat(200));
        let m = find_all(&long, "needle", true)[0];
        let p = preview(&long, &m);
        assert!(p.text.starts_with('…') && p.text.ends_with('…'));
        assert_eq!(&p.text[p.highlight.0..p.highlight.1], "needle");
    }
}
