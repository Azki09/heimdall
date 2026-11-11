//! The terminal editor's text (DESIGN §17.3): the note body, the cursor,
//! soft-wrapped rows, find and undo. No terminal code here, so it is
//! tested on every platform.
//!
//! Masked secrets (`{*value*}`) are one block while hidden: the cursor
//! jumps over them, Backspace/Delete remove them whole, and they are drawn
//! as [`SECRET`], the same width whatever their length. `^R` shows the
//! value of the one at the cursor, underlined and without its `{*` `*}`
//! marks, to read or edit; it hides again when the cursor leaves it.
//!
//! Code mode turns word wrap off: a row is a whole line, and the editor
//! scrolls sideways.

use std::ops::Range;

use heimdall_core::secret;
use unicode_width::UnicodeWidthChar;
use zeroize::Zeroizing;

/// How a hidden secret is drawn.
pub const SECRET: &str = "[secret]";
const SECRET_W: usize = SECRET.len();
/// A tab is drawn as this many spaces.
pub const TAB_W: usize = 4;
/// Undo keeps at most this many steps, and at most this many bytes of
/// copies in all (a 2 MB note shouldn't hold 200 copies of itself).
const UNDO_STEPS: usize = 200;
const UNDO_BYTES: usize = 16 * 1024 * 1024;

/// One screen row: a byte range of the text, never including the `\n`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub start: usize,
    pub end: usize,
}

/// What one block of a row looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Char(char),
    /// A tab: [`TAB_W`] spaces.
    Tab,
    /// A hidden secret: [`SECRET`].
    Secret,
}

/// One drawn block of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    /// Where it starts in the text, in bytes (for syntax colors).
    pub at: usize,
    pub glyph: Glyph,
    /// Part of the secret shown with `^R`.
    pub underline: bool,
}

impl Cell {
    pub fn width(&self) -> usize {
        match self.glyph {
            Glyph::Char(c) => char_width(c),
            Glyph::Tab => TAB_W,
            Glyph::Secret => SECRET_W,
        }
    }
}

/// What [`Doc::insert`] refused.
#[derive(Debug, PartialEq, Eq)]
pub struct TooBig;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Edit {
    None,
    Type,
    Backspace,
    Delete,
    Other,
}

struct Step {
    text: Zeroizing<String>,
    cursor: usize,
}

pub struct Doc {
    text: Zeroizing<String>,
    /// Byte offset; always on a char boundary, and never inside a hidden
    /// secret.
    cursor: usize,
    /// Start of the secret shown with `^R`, if any.
    shown: Option<usize>,
    secrets: Vec<Range<usize>>,
    /// Bytes the text may grow to (the app's note limit).
    limit: usize,
    /// Wanted column for Up/Down, kept across short rows.
    goal: Option<usize>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    last_edit: Edit,
    pub modified: bool,
    width: usize,
    /// Off in code mode: one row per line.
    wrap: bool,
    /// Goes up with every change to the text, for caches.
    version: u64,
    rows: Vec<Row>,
}

impl Doc {
    /// `cursor` in chars, as the vault stores it.
    pub fn new(body: &str, cursor_chars: usize, limit: usize) -> Self {
        // Room for the whole note up front: growing a String copies it and
        // leaves the old copy in freed memory, unwiped.
        let mut text = Zeroizing::new(String::with_capacity(limit.max(body.len()) + 8));
        text.push_str(body);
        let cursor = body
            .char_indices()
            .nth(cursor_chars)
            .map_or(body.len(), |(i, _)| i);
        let mut doc = Self {
            text,
            cursor,
            shown: None,
            secrets: Vec::new(),
            limit,
            goal: None,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: Edit::None,
            modified: false,
            width: 0,
            wrap: true,
            version: 0,
            rows: Vec::new(),
        };
        doc.changed();
        doc
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    #[cfg(test)]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The cursor in chars from the start, for the vault.
    pub fn cursor_chars(&self) -> usize {
        self.text[..self.cursor].chars().count()
    }

    /// Whether a secret is shown (`^R`).
    pub fn revealed(&self) -> bool {
        self.shown.is_some()
    }

    /// `^R`: hides the secret that is shown, or shows the one at the
    /// cursor. Returns false if there was nothing to do.
    pub fn toggle_reveal(&mut self) -> bool {
        if self.shown.is_some() {
            self.leave_secret();
            return true;
        }
        let at = self.cursor;
        let Some(s) = self
            .secrets
            .iter()
            .find(|s| s.start <= at && at <= s.end)
            .cloned()
        else {
            return false;
        };
        self.shown = Some(s.start);
        // Shown, the cursor lives inside the value, between the marks.
        let value = secret::value(&s);
        self.cursor = self.cursor.clamp(value.start, value.end);
        self.relayout();
        true
    }

    /// Hides the shown secret and puts the cursor after it.
    pub fn leave_secret(&mut self) {
        if let Some(s) = self.shown_secret() {
            self.cursor = s.end;
        }
        self.shown = None;
        self.goal = None;
        self.snap_out_of_secret();
        self.relayout();
    }

    fn hidden(&self, s: &Range<usize>) -> bool {
        self.shown != Some(s.start)
    }

    /// The secret shown with `^R`, while it still is one.
    fn shown_secret(&self) -> Option<Range<usize>> {
        let start = self.shown?;
        let i = self.secrets.partition_point(|s| s.start < start);
        self.secrets.get(i).filter(|s| s.start == start).cloned()
    }

    /// Whether `atom` is the `{*` or the `*}` of the shown secret: kept in
    /// the text, never drawn, and never deleted on their own.
    fn is_mark(&self, atom: &Range<usize>) -> bool {
        self.shown_secret().is_some_and(|s| {
            let value = secret::value(&s);
            *atom == (s.start..value.start) || *atom == (value.end..s.end)
        })
    }

    /// The shown secret hides again once the cursor is outside its value,
    /// or once it stops being a secret.
    fn check_shown(&mut self) -> bool {
        if self.shown.is_none() {
            return false;
        }
        let at = self.cursor;
        let still = self.shown_secret().is_some_and(|s| {
            let value = secret::value(&s);
            value.start <= at && at <= value.end
        });
        if !still {
            self.shown = None;
        }
        !still
    }

    // ---------- Layout ----------

    /// Wraps the text to `width` columns. Call when the terminal resizes.
    pub fn set_width(&mut self, width: usize) {
        let width = width.max(SECRET_W);
        if width != self.width {
            self.width = width;
            self.relayout();
        }
    }

    /// Word wrap on (notes) or off (code mode).
    pub fn set_wrap(&mut self, on: bool) {
        if on != self.wrap {
            self.wrap = on;
            self.relayout();
        }
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Changes whenever the text does.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The hidden secret starting at `pos`, if any.
    fn secret_at(&self, pos: usize) -> Option<Range<usize>> {
        let i = self.secrets.partition_point(|s| s.start < pos);
        self.secrets
            .get(i)
            .filter(|s| s.start == pos && self.hidden(s))
            .cloned()
    }

    /// The hidden secret ending at `pos`, if any.
    fn secret_ending(&self, pos: usize) -> Option<Range<usize>> {
        let i = self.secrets.partition_point(|s| s.end < pos);
        self.secrets
            .get(i)
            .filter(|s| s.end == pos && self.hidden(s))
            .cloned()
    }

    /// The block after `pos`: a hidden secret, a mark of the shown secret,
    /// or one char.
    fn atom_at(&self, pos: usize) -> Option<Range<usize>> {
        if let Some(s) = self.secret_at(pos) {
            return Some(s);
        }
        if let Some(s) = self.shown_secret() {
            let value = secret::value(&s);
            if pos == s.start {
                return Some(s.start..value.start);
            }
            if pos == value.end {
                return Some(value.end..s.end);
            }
        }
        let c = self.text[pos..].chars().next()?;
        Some(pos..pos + c.len_utf8())
    }

    fn atom_before(&self, pos: usize) -> Option<Range<usize>> {
        if let Some(s) = self.secret_ending(pos) {
            return Some(s);
        }
        if let Some(s) = self.shown_secret() {
            let value = secret::value(&s);
            if pos == value.start {
                return Some(s.start..value.start);
            }
            if pos == s.end {
                return Some(value.end..s.end);
            }
        }
        let c = self.text[..pos].chars().next_back()?;
        Some(pos - c.len_utf8()..pos)
    }

    /// Columns `atom` takes on screen.
    fn atom_width(&self, atom: &Range<usize>) -> usize {
        if self.is_mark(atom) {
            return 0;
        }
        if atom.len() > 4 {
            return SECRET_W; // only a hidden secret is longer than one char
        }
        let c = self.text[atom.clone()].chars().next().unwrap_or(' ');
        char_width(c)
    }

    fn relayout(&mut self) {
        let mut rows = Vec::new();
        let width = self.width.max(SECRET_W);
        let mut line_start = 0;
        loop {
            let line_end = self.text[line_start..]
                .find('\n')
                .map_or(self.text.len(), |n| line_start + n);
            if self.wrap {
                self.wrap_line(line_start, line_end, width, &mut rows);
            } else {
                rows.push(Row {
                    start: line_start,
                    end: line_end,
                });
            }
            if line_end >= self.text.len() {
                break;
            }
            line_start = line_end + 1;
        }
        self.rows = rows;
    }

    /// Word wrap: a row breaks after the last space that fits, or anywhere
    /// in a word longer than the row.
    fn wrap_line(&self, start: usize, end: usize, width: usize, rows: &mut Vec<Row>) {
        let mut row_start = start;
        let mut col = 0;
        // Byte offset and column just after the last space in this row.
        let mut last_break: Option<(usize, usize)> = None;
        let mut pos = start;
        while pos < end {
            let atom = self.atom_at(pos).expect("pos < end");
            let w = self.atom_width(&atom);
            if col + w > width && pos > row_start {
                match last_break.filter(|&(at, _)| at > row_start) {
                    Some((at, at_col)) => {
                        rows.push(Row {
                            start: row_start,
                            end: at,
                        });
                        row_start = at;
                        col -= at_col;
                    }
                    None => {
                        rows.push(Row {
                            start: row_start,
                            end: pos,
                        });
                        row_start = pos;
                        col = 0;
                    }
                }
                last_break = None;
                continue;
            }
            col += w;
            pos = atom.end;
            if &self.text[atom] == " " {
                last_break = Some((pos, col));
            }
        }
        rows.push(Row {
            start: row_start,
            end,
        });
    }

    /// The row the cursor is on. At a soft wrap the cursor belongs to the
    /// row that starts there.
    pub fn cursor_row(&self) -> usize {
        let i = self.rows.partition_point(|r| r.start <= self.cursor);
        i.saturating_sub(1)
    }

    /// Column of byte `pos` within `row`.
    fn col_in(&self, row: Row, pos: usize) -> usize {
        let mut col = 0;
        let mut at = row.start;
        while at < pos.min(row.end) {
            let atom = self.atom_at(at).expect("at < end");
            col += self.atom_width(&atom);
            at = atom.end;
        }
        col
    }

    pub fn cursor_col(&self) -> usize {
        self.col_in(self.rows[self.cursor_row()], self.cursor)
    }

    /// The byte in `row` closest to column `goal`.
    fn pos_at_col(&self, row: Row, goal: usize) -> usize {
        let mut col = 0;
        let mut at = row.start;
        while at < row.end {
            let atom = self.atom_at(at).expect("at < end");
            let w = self.atom_width(&atom);
            if col + w > goal {
                break;
            }
            col += w;
            at = atom.end;
        }
        at
    }

    /// What `row` draws, block by block: secrets as [`Glyph::Secret`],
    /// control chars as `?`. The marks of the shown secret draw nothing.
    pub fn cells(&self, row: Row) -> Vec<Cell> {
        let shown = self.shown_secret().map(|s| secret::value(&s));
        let mut out = Vec::new();
        let mut at = row.start;
        while at < row.end {
            let atom = self.atom_at(at).expect("at < end");
            let glyph = if self.secret_at(at).is_some() {
                Some(Glyph::Secret)
            } else if self.is_mark(&atom) {
                None
            } else {
                match self.text[atom.clone()].chars().next().unwrap_or(' ') {
                    '\t' => Some(Glyph::Tab),
                    '\r' => None,
                    c if c.is_control() => Some(Glyph::Char('?')),
                    c => Some(Glyph::Char(c)),
                }
            };
            if let Some(glyph) = glyph {
                out.push(Cell {
                    at,
                    glyph,
                    underline: shown.as_ref().is_some_and(|v| v.contains(&at)),
                });
            }
            at = atom.end;
        }
        out
    }

    /// The text of `row` as drawn, without styles.
    #[cfg(test)]
    pub fn render_row(&self, row: Row) -> String {
        let mut out = String::new();
        for cell in self.cells(row) {
            match cell.glyph {
                Glyph::Char(c) => out.push(c),
                Glyph::Tab => out.push_str(&" ".repeat(TAB_W)),
                Glyph::Secret => out.push_str(SECRET),
            }
        }
        out
    }

    /// Indent and length of every line, in columns, for the minimap. Counted
    /// as drawn, so a hidden secret doesn't give its length away. Only
    /// meaningful without word wrap.
    pub fn line_shapes(&self) -> Vec<(usize, usize)> {
        self.rows
            .iter()
            .map(|&row| {
                let (mut indent, mut len, mut leading) = (0, 0, true);
                let mut col = 0;
                for cell in self.cells(row) {
                    let blank = matches!(cell.glyph, Glyph::Tab | Glyph::Char(' '));
                    col += cell.width();
                    if !blank {
                        leading = false;
                        len = col;
                    } else if leading {
                        indent = col;
                    }
                }
                (indent, len)
            })
            .collect()
    }

    /// The spaces and tabs the cursor's line starts with (up to the cursor),
    /// for Enter in code mode.
    pub fn line_indent(&self) -> String {
        let line_start = self.text[..self.cursor].rfind('\n').map_or(0, |n| n + 1);
        self.text[line_start..self.cursor]
            .chars()
            .take_while(|&c| c == ' ' || c == '\t')
            .collect()
    }

    /// Line and column for the status line, both from 1. The column counts
    /// what is on screen, so a hidden secret doesn't give its length away.
    pub fn line_col(&self) -> (usize, usize) {
        let line_start = self.text[..self.cursor].rfind('\n').map_or(0, |n| n + 1);
        let line = self.text[..self.cursor].matches('\n').count() + 1;
        let mut col = 0;
        let mut at = line_start;
        while at < self.cursor {
            let atom = self.atom_at(at).expect("at < cursor");
            col += self.atom_width(&atom);
            at = atom.end;
        }
        (line, col + 1)
    }

    // ---------- Moving ----------

    pub fn left(&mut self) {
        if let Some(a) = self.atom_before(self.cursor) {
            self.cursor = a.start;
        }
        self.moved();
    }

    pub fn right(&mut self) {
        if let Some(a) = self.atom_at(self.cursor) {
            self.cursor = a.end;
        }
        self.moved();
    }

    /// Ctrl+Left: to the start of the previous word.
    pub fn word_left(&mut self) {
        while let Some(a) = self.atom_before(self.cursor).filter(|a| self.is_space(a)) {
            self.cursor = a.start;
        }
        while let Some(a) = self.atom_before(self.cursor).filter(|a| !self.is_space(a)) {
            self.cursor = a.start;
        }
        self.moved();
    }

    /// Ctrl+Right: to the end of the next word.
    pub fn word_right(&mut self) {
        while let Some(a) = self.atom_at(self.cursor).filter(|a| self.is_space(a)) {
            self.cursor = a.end;
        }
        while let Some(a) = self.atom_at(self.cursor).filter(|a| !self.is_space(a)) {
            self.cursor = a.end;
        }
        self.moved();
    }

    fn is_space(&self, atom: &Range<usize>) -> bool {
        self.text[atom.clone()].chars().all(char::is_whitespace)
    }

    /// Up (negative) or down by `n` rows, keeping the column.
    pub fn vertical(&mut self, n: isize) {
        let goal = self.goal.unwrap_or_else(|| self.cursor_col());
        let row = self.cursor_row();
        let target = row.saturating_add_signed(n).min(self.rows.len() - 1);
        if target == row {
            // First or last row: go to its start or end, like most editors.
            self.cursor = if n < 0 { 0 } else { self.text.len() };
        } else {
            let r = self.rows[target];
            self.cursor = self.pos_at_col(r, goal);
            // A soft-wrapped row's end is the next row's start: stay on this one.
            if self.cursor == r.end && target + 1 < self.rows.len() && self.rows[target + 1].start == r.end {
                self.cursor = self.atom_before(r.end).map_or(r.start, |a| a.start.max(r.start));
            }
        }
        self.goal = Some(goal);
        self.last_edit = Edit::None;
        if self.check_shown() {
            self.snap_out_of_secret();
            self.relayout();
        }
    }

    pub fn home(&mut self) {
        self.cursor = self.rows[self.cursor_row()].start;
        self.moved();
    }

    pub fn end(&mut self) {
        let row = self.cursor_row();
        let r = self.rows[row];
        self.cursor = r.end;
        if row + 1 < self.rows.len() && self.rows[row + 1].start == r.end {
            self.cursor = self.atom_before(r.end).map_or(r.start, |a| a.start.max(r.start));
        }
        self.moved();
    }

    pub fn doc_start(&mut self) {
        self.cursor = 0;
        self.moved();
    }

    pub fn doc_end(&mut self) {
        self.cursor = self.text.len();
        self.moved();
    }

    fn moved(&mut self) {
        self.goal = None;
        self.last_edit = Edit::None;
        if self.check_shown() {
            self.snap_out_of_secret();
            self.relayout();
        }
    }

    // ---------- Editing ----------

    /// Types or pastes `s` at the cursor. CRLF becomes LF; other control
    /// chars except tab are dropped.
    pub fn insert(&mut self, s: &str) -> Result<(), TooBig> {
        let clean: Zeroizing<String> = Zeroizing::new(
            s.replace("\r\n", "\n")
                .chars()
                .map(|c| if c == '\r' { '\n' } else { c })
                .filter(|&c| c == '\n' || c == '\t' || !c.is_control())
                .collect(),
        );
        // A secret is one line: inside the shown one, a line break is a space.
        let clean: Zeroizing<String> = if self.shown.is_some() {
            Zeroizing::new(clean.replace('\n', " "))
        } else {
            clean
        };
        if clean.is_empty() {
            return Ok(());
        }
        if self.text.len() + clean.len() > self.limit {
            return Err(TooBig);
        }
        let typing = clean.chars().count() == 1 && !clean.contains('\n');
        self.checkpoint(if typing { Edit::Type } else { Edit::Other });
        self.text.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
        self.changed();
        Ok(())
    }

    /// Backspace or Delete on the shown secret: a mark is stepped over,
    /// not deleted, and deleting the last char of the value removes the
    /// whole secret (an empty `{**}` isn't one). True = handled.
    fn edit_shown(&mut self, atom: &Range<usize>, forward: bool) -> bool {
        let Some(s) = self.shown_secret() else {
            return false;
        };
        if self.is_mark(atom) {
            if forward {
                self.right();
            } else {
                self.left();
            }
            return true;
        }
        if *atom != secret::value(&s) {
            return false;
        }
        self.checkpoint(Edit::Other);
        self.text.replace_range(s.clone(), "");
        self.cursor = s.start;
        self.shown = None;
        self.changed();
        true
    }

    pub fn backspace(&mut self) {
        let Some(a) = self.atom_before(self.cursor) else {
            return;
        };
        if self.edit_shown(&a, false) {
            return;
        }
        let kind = if a.len() > 4 { Edit::Other } else { Edit::Backspace };
        self.checkpoint(kind);
        self.text.replace_range(a.clone(), "");
        self.cursor = a.start;
        self.changed();
    }

    pub fn delete(&mut self) {
        let Some(a) = self.atom_at(self.cursor) else {
            return;
        };
        if self.edit_shown(&a, true) {
            return;
        }
        let kind = if a.len() > 4 { Edit::Other } else { Edit::Delete };
        self.checkpoint(kind);
        self.text.replace_range(a, "");
        self.changed();
    }

    /// Saves the state before an edit for undo. A run of typing (or of
    /// Backspace, or of Delete) is one step.
    fn checkpoint(&mut self, kind: Edit) {
        let continues = kind == self.last_edit && kind != Edit::Other;
        self.last_edit = kind;
        self.redo.clear();
        if continues {
            return;
        }
        let step = self.step();
        push_step(&mut self.undo, step);
    }

    fn step(&self) -> Step {
        let mut text = Zeroizing::new(String::with_capacity(self.text.len()));
        text.push_str(&self.text);
        Step {
            text,
            cursor: self.cursor,
        }
    }

    pub fn undo(&mut self) -> bool {
        let Some(step) = self.undo.pop() else { return false };
        let current = self.step();
        push_step(&mut self.redo, current);
        self.restore(step);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(step) = self.redo.pop() else { return false };
        let current = self.step();
        push_step(&mut self.undo, current);
        self.restore(step);
        true
    }

    fn restore(&mut self, step: Step) {
        // Copy into the existing buffer: it already has room for the whole note.
        self.text.clear();
        self.text.push_str(&step.text);
        self.cursor = step.cursor.min(self.text.len());
        self.last_edit = Edit::None;
        self.changed();
    }

    fn changed(&mut self) {
        self.version += 1;
        self.secrets = secret::scan(&self.text).secrets;
        self.modified = self.modified || !self.undo.is_empty() || !self.redo.is_empty();
        self.goal = None;
        self.check_shown();
        self.snap_out_of_secret();
        self.relayout();
    }

    /// Typing a `{*` in front of `abc*}` makes a secret around the cursor;
    /// hidden, the cursor goes after it.
    fn snap_out_of_secret(&mut self) {
        let at = self.cursor;
        if let Some(s) = self
            .secrets
            .iter()
            .find(|s| s.start < at && at < s.end && self.hidden(s))
        {
            self.cursor = s.end;
        }
    }

    // ---------- Find ----------

    /// The next match of `query` after the cursor (wrapping around), case
    /// insensitive. Matches inside a hidden secret are skipped: finding
    /// one would tell what the secret contains.
    pub fn find_next(&mut self, query: &str) -> bool {
        let from = self.atom_at(self.cursor).map_or(self.cursor, |a| a.end);
        let hits = self.matches(query);
        let found = hits.iter().find(|&&h| h >= from).or(hits.first()).copied();
        self.jump(found)
    }

    /// `^P` in Find: the previous match, wrapping around.
    pub fn find_prev(&mut self, query: &str) -> bool {
        let hits = self.matches(query);
        let found = hits
            .iter()
            .rev()
            .find(|&&h| h < self.cursor)
            .or(hits.last())
            .copied();
        self.jump(found)
    }

    fn jump(&mut self, to: Option<usize>) -> bool {
        if let Some(at) = to {
            self.cursor = at;
            self.moved();
            self.snap_out_of_secret();
        }
        to.is_some()
    }

    /// Where each match starts.
    fn matches(&self, query: &str) -> Vec<usize> {
        let mut out = Vec::new();
        if query.is_empty() {
            return out;
        }
        for (at, _) in self.text.char_indices() {
            if let Some(len) = match_len(&self.text[at..], query) {
                let hit = at..at + len;
                let in_secret = self
                    .secrets
                    .iter()
                    .any(|s| s.start < hit.end && hit.start < s.end && self.hidden(s));
                if !in_secret {
                    out.push(at);
                }
            }
        }
        out
    }

    // ---------- Cut and paste ----------

    /// `^K`: removes the cursor's line (with its line break) and returns it.
    pub fn cut_line(&mut self) -> Option<Zeroizing<String>> {
        if self.text.is_empty() {
            return None;
        }
        let start = self.text[..self.cursor].rfind('\n').map_or(0, |n| n + 1);
        let end = self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |n| self.cursor + n + 1);
        self.checkpoint(Edit::Other);
        let mut cut = Zeroizing::new(String::with_capacity(end - start));
        cut.push_str(&self.text[start..end]);
        self.text.replace_range(start..end, "");
        self.cursor = start;
        self.changed();
        Some(cut)
    }
}

/// If `text` starts with `query` ignoring case, the bytes of `text` it spans.
fn match_len(text: &str, query: &str) -> Option<usize> {
    let mut t = text.char_indices();
    let mut len = 0;
    for q in query.chars() {
        let (i, c) = t.next()?;
        if !c.to_lowercase().eq(q.to_lowercase()) {
            return None;
        }
        len = i + c.len_utf8();
    }
    Some(len)
}

fn push_step(stack: &mut Vec<Step>, step: Step) {
    stack.push(step);
    let mut bytes: usize = stack.iter().map(|s| s.text.len()).sum();
    while stack.len() > UNDO_STEPS || (bytes > UNDO_BYTES && stack.len() > 1) {
        bytes -= stack[0].text.len();
        stack.remove(0);
    }
}

pub fn char_width(c: char) -> usize {
    match c {
        '\t' => TAB_W,
        '\r' => 0,
        c if c.is_control() => 1,
        c => c.width().unwrap_or(0),
    }
}

/// Columns `s` takes on screen.
pub fn str_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// `s` cut to at most `width` columns.
pub fn truncate(s: &str, width: usize) -> &str {
    let mut col = 0;
    for (i, c) in s.char_indices() {
        col += char_width(c);
        if col > width {
            return &s[..i];
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str, width: usize) -> Doc {
        let mut d = Doc::new(text, 0, 1 << 20);
        d.set_width(width);
        d
    }

    fn rows(d: &Doc) -> Vec<String> {
        d.rows().iter().map(|&r| d.render_row(r)).collect()
    }

    #[test]
    fn wraps_at_spaces() {
        let d = doc("the quick brown fox jumps", 10);
        assert_eq!(rows(&d), ["the quick ", "brown fox ", "jumps"]);
        let d = doc("abcdefghijklmnop", 10);
        assert_eq!(rows(&d), ["abcdefghij", "klmnop"]);
        let d = doc("a\n\nb", 10);
        assert_eq!(rows(&d), ["a", "", "b"]);
    }

    #[test]
    fn secrets_are_one_block() {
        let mut d = doc("pw {*hunter2*} ok", 40);
        assert_eq!(rows(&d), ["pw [secret] ok"]);
        d.doc_start();
        for _ in 0..3 {
            d.right();
        }
        assert_eq!(d.cursor(), 3);
        d.right();
        assert_eq!(&d.text()[..d.cursor()], "pw {*hunter2*}");
        assert_eq!(d.line_col(), (1, 12));
        d.backspace();
        assert_eq!(d.text(), "pw  ok");
        assert!(d.undo());
        assert_eq!(d.text(), "pw {*hunter2*} ok");

        // ^R shows the value of the secret at the cursor, without its marks.
        assert!(d.toggle_reveal());
        assert_eq!(rows(&d), ["pw hunter2 ok"]);
        let cells = d.cells(d.rows()[0]);
        let underlined: String = cells
            .iter()
            .filter(|c| c.underline)
            .map(|c| match c.glyph {
                Glyph::Char(c) => c,
                _ => '?',
            })
            .collect();
        assert_eq!(underlined, "hunter2");
        // The cursor is inside the value: keys edit it, the marks stay.
        assert_eq!(d.line_col(), (1, 11));
        d.backspace();
        assert_eq!(d.text(), "pw {*hunter*} ok");
        d.insert("X").unwrap();
        assert_eq!(d.text(), "pw {*hunterX*} ok");
        assert!(d.revealed());
        // A line break can't go inside a secret.
        d.insert("a\nb").unwrap();
        assert_eq!(d.text(), "pw {*hunterXa b*} ok");
        // Leaving it hides it again.
        d.right();
        assert!(!d.revealed());
        assert_eq!(rows(&d), ["pw [secret] ok"]);
        assert_eq!(&d.text()[..d.cursor()], "pw {*hunterXa b*}");
        // Nothing to show away from a secret.
        d.doc_start();
        assert!(!d.toggle_reveal());
    }

    #[test]
    fn marks_of_a_shown_secret_are_never_deleted_alone() {
        let mut d = doc("a{*xy*}b", 40);
        d.right();
        assert!(d.toggle_reveal());
        assert_eq!(rows(&d), ["axyb"]);
        // Backspace at the start of the value steps out instead of eating `{*`.
        d.backspace();
        assert_eq!(d.text(), "a{*xy*}b");
        assert!(!d.revealed());
        // Deleting the whole value removes the secret, marks included.
        d.right();
        assert!(d.toggle_reveal());
        d.backspace();
        assert_eq!(d.text(), "a{*x*}b");
        d.backspace();
        assert_eq!(d.text(), "ab");
        assert!(!d.revealed());
        assert!(d.undo());
        assert_eq!(d.text(), "a{*x*}b");
    }

    #[test]
    fn code_mode_rows_are_lines() {
        let mut d = doc("def f():\n    return {*abc*} + a_rather_long_name\n", 12);
        assert!(d.rows().len() > 3);
        d.set_wrap(false);
        assert_eq!(d.rows().len(), 3);
        // The minimap sees the secret as drawn, not its real length.
        assert_eq!(d.line_shapes(), [(0, 8), (4, 40), (0, 0)]);
        d.vertical(1);
        d.end();
        assert_eq!(d.line_indent(), "    ");
    }

    #[test]
    fn only_the_shown_secret_shows() {
        let mut d = doc("{*one*} {*two*}", 40);
        d.doc_end();
        assert!(d.toggle_reveal());
        assert_eq!(rows(&d), ["[secret] two"]);
        assert!(d.toggle_reveal());
        assert_eq!(rows(&d), ["[secret] [secret]"]);
        assert_eq!(d.cursor(), d.text().len());
    }

    #[test]
    fn cut_line_takes_the_line_break() {
        let mut d = doc("one\ntwo\nthree", 40);
        d.vertical(1);
        assert_eq!(d.cut_line().unwrap().as_str(), "two\n");
        assert_eq!(d.text(), "one\nthree");
        assert_eq!(d.line_col(), (2, 1));
        assert!(d.undo());
        assert_eq!(d.text(), "one\ntwo\nthree");
    }

    #[test]
    fn length_of_a_secret_never_shows() {
        let a = doc("{*a*}", 40);
        let b = doc("{*a much longer secret value*}", 40);
        assert_eq!(rows(&a), rows(&b));
    }

    #[test]
    fn typing_a_secret_moves_the_cursor_out() {
        let mut d = doc("abc*}", 40);
        d.insert("{").unwrap();
        d.insert("*").unwrap();
        assert_eq!(d.text(), "{*abc*}");
        assert_eq!(d.cursor(), d.text().len());
    }

    #[test]
    fn find_skips_hidden_secrets() {
        let mut d = doc("{*needle*} and needle", 40);
        assert!(d.find_next("NEEDLE"));
        assert_eq!(d.cursor(), 15);
        // Only one visible match: next and previous both land on it.
        assert!(d.find_next("needle"));
        assert_eq!(d.cursor(), 15);
        assert!(d.find_prev("needle"));
        assert_eq!(d.cursor(), 15);
        assert!(!d.find_next("missing"));
        // Shown, the secret's text can be found.
        d.doc_start();
        assert!(d.toggle_reveal());
        for _ in 0..3 {
            d.right();
        }
        assert!(d.find_prev("needle"));
        assert_eq!(d.cursor(), 2);
        assert!(d.revealed());
    }

    #[test]
    fn undo_groups_typing() {
        let mut d = doc("", 40);
        for c in ["h", "e", "y"] {
            d.insert(c).unwrap();
        }
        d.insert("\n").unwrap();
        d.insert("x").unwrap();
        assert!(d.undo());
        assert_eq!(d.text(), "hey\n");
        assert!(d.undo());
        assert_eq!(d.text(), "hey");
        assert!(d.undo());
        assert_eq!(d.text(), "");
        assert!(!d.undo());
        assert!(d.redo());
        assert_eq!(d.text(), "hey");
        assert!(d.modified);
    }

    #[test]
    fn limit_and_cleanup() {
        let mut d = Doc::new("", 0, 5);
        assert_eq!(d.insert("123456"), Err(TooBig));
        d.insert("a\r\nb\x07").unwrap();
        assert_eq!(d.text(), "a\nb");
    }

    #[test]
    fn vertical_keeps_column() {
        let mut d = doc("abcdef\nab\nabcdef", 40);
        for _ in 0..4 {
            d.right();
        }
        d.vertical(1);
        assert_eq!(d.line_col(), (2, 3));
        d.vertical(1);
        assert_eq!(d.line_col(), (3, 5));
        d.vertical(1);
        assert_eq!(d.cursor(), d.text().len());
    }

    #[test]
    fn cursor_from_and_to_chars() {
        let d = Doc::new("añb", 2, 100);
        assert_eq!(d.cursor(), 3);
        assert_eq!(d.cursor_chars(), 2);
    }

    #[test]
    fn wide_chars() {
        let d = doc("日本語テキスト", 8);
        assert_eq!(rows(&d), ["日本語テ", "キスト"]);
        assert_eq!(truncate("日本語", 5), "日本");
        assert_eq!(str_width("日本"), 4);
    }
}
