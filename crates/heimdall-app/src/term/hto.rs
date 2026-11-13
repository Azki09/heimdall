//! A hidden full-screen picture, reached by an option `--help` does not
//! list. The layout is plain text and builds everywhere so it can be
//! tested; drawing it is Linux only, like the rest of the terminal mode.

const SKULL: &str = include_str!("../../assets/hto/skull.txt");
const TITLE: &str = include_str!("../../assets/hto/title.txt");
const MOTTO: &str = include_str!("../../assets/hto/motto.txt");
const HINT: &str = "Press Enter to return";

/// Empty rows between the skull, the title, the motto and the hint.
const GAP: usize = 2;
/// The background: one star in this many free cells, as in the art itself.
const STAR_ONE_IN: u64 = 45;
/// The art often has pairs, `zz` or `z z`: one star in this many gets one.
const PAIR_ONE_IN: u64 = 6;
const STAR: char = 'z';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    Skull,
    Star,
    Title,
    Motto,
    Hint,
}

/// One row of the screen: each cell a char (space = empty) and its ink.
pub type Row = Vec<(char, Ink)>;

fn block(text: &str) -> Vec<Vec<char>> {
    text.lines().map(|l| l.trim_end().chars().collect()).collect()
}

fn width(rows: &[Vec<char>]) -> usize {
    rows.iter().map(Vec::len).max().unwrap_or(0)
}

/// Columns of the skull itself, without the stars around it.
fn skull_span(rows: &[Vec<char>]) -> (usize, usize) {
    let cols = rows.iter().flat_map(|r| {
        r.iter()
            .enumerate()
            .filter(|(_, &c)| c != ' ' && c != STAR)
            .map(|(i, _)| i)
    });
    let (lo, hi) = cols.fold((usize::MAX, 0), |(lo, hi), i| (lo.min(i), hi.max(i)));
    (lo, hi)
}

/// The smallest terminal it fits in, as (columns, rows).
pub fn needed() -> (usize, usize) {
    let (skull, title, motto) = (block(SKULL), block(TITLE), block(MOTTO));
    let cols = width(&title).max(width(&motto)).max(HINT.len());
    let (lo, hi) = skull_span(&skull);
    let cols = cols.max(hi - lo + 1);
    let rows = skull.len() + GAP + title.len() + GAP + motto.len() + GAP + 1;
    (cols, rows)
}

/// A small xorshift: the stars only need to look scattered.
struct Rng(u64);

impl Rng {
    fn one_in(&mut self, n: u64) -> bool {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0.is_multiple_of(n)
    }
}

/// Keeps the background off row `y` and the rows above and below it, in `xs`.
fn mark(keep: &mut [Vec<bool>], y: usize, xs: std::ops::Range<usize>) {
    let last = keep.len() - 1;
    for row in &mut keep[y.saturating_sub(1)..=(y + 1).min(last)] {
        row[xs.clone()].fill(true);
    }
}

/// The whole screen for a `cols` x `rows` terminal, or `None` when it
/// doesn't fit. The skull is centred on itself (its stars are uneven), the
/// text under it, and stars fill every free cell around them.
pub fn compose(cols: usize, rows: usize, seed: u64) -> Option<Vec<Row>> {
    let (need_cols, need_rows) = needed();
    if cols < need_cols || rows < need_rows {
        return None;
    }
    let mut grid: Vec<Row> = vec![vec![(' ', Ink::Star); cols]; rows];
    // Cells the background must leave alone: around the text and the skull.
    let mut keep = vec![vec![false; cols]; rows];
    let mut y = (rows - need_rows) / 2;

    let skull = block(SKULL);
    let (lo, hi) = skull_span(&skull);
    let left = (cols / 2) as isize - (lo + hi).div_ceil(2) as isize;
    // The art's own stars stay as drawn; no new ones inside its box.
    let star_box = {
        let xs = skull
            .iter()
            .flat_map(|r| r.iter().enumerate().filter(|(_, &c)| c == STAR).map(|(i, _)| i));
        let (a, b) = xs.fold((usize::MAX, 0), |(a, b), i| (a.min(i), b.max(i)));
        (left + a as isize, left + b as isize)
    };
    let skull_top = y;
    for line in &skull {
        for (i, &c) in line.iter().enumerate() {
            let x = left + i as isize;
            if c == ' ' || x < 0 || x as usize >= cols {
                continue;
            }
            let x = x as usize;
            grid[y][x] = (c, if c == STAR { Ink::Star } else { Ink::Skull });
            if c != STAR {
                mark(&mut keep, y, x.saturating_sub(2)..(x + 3).min(cols));
            }
        }
        y += 1;
    }
    let skull_rows = skull_top..y;

    let mut put = |y: &mut usize, lines: Vec<Vec<char>>, ink: Ink| {
        *y += GAP;
        let w = width(&lines);
        let left = (cols - w) / 2;
        for line in lines {
            for (i, &c) in line.iter().enumerate() {
                if c != ' ' {
                    grid[*y][left + i] = (c, ink);
                }
            }
            mark(&mut keep, *y, left.saturating_sub(3)..(left + w + 3).min(cols));
            *y += 1;
        }
    };
    put(&mut y, block(TITLE), Ink::Title);
    put(&mut y, block(MOTTO), Ink::Motto);
    put(&mut y, vec![HINT.chars().collect()], Ink::Hint);

    let mut rng = Rng(seed | 1);
    for y in 0..rows {
        // The last column stays empty: writing there can scroll the screen.
        for x in 0..cols - 1 {
            let in_art = skull_rows.contains(&y) && (star_box.0..=star_box.1).contains(&(x as isize));
            if in_art || keep[y][x] || grid[y][x].0 != ' ' || !rng.one_in(STAR_ONE_IN) {
                continue;
            }
            grid[y][x] = (STAR, Ink::Star);
            if rng.one_in(PAIR_ONE_IN) {
                let x2 = x + if rng.one_in(2) { 1 } else { 2 };
                if x2 < cols - 1 && !keep[y][x2] && grid[y][x2].0 == ' ' {
                    grid[y][x2] = (STAR, Ink::Star);
                }
            }
        }
    }
    Some(grid)
}

#[cfg(target_os = "linux")]
pub use linux::show;

#[cfg(target_os = "linux")]
mod linux {
    use std::io::{self, Write};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crossterm::cursor::{Hide, MoveTo};
    use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
    use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
    use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen};
    use crossterm::{execute, queue};

    use super::{compose, needed, Ink, HINT};
    use crate::term::editor::restore_terminal;
    use crate::term::prompt::TICK;
    use crate::term::session::Fail;

    fn color(ink: Ink, truecolor: bool) -> Color {
        let grey = |v| Color::Rgb { r: v, g: v, b: v };
        match (ink, truecolor) {
            (Ink::Skull, true) => grey(0xC9),
            (Ink::Star, true) => grey(0x5C),
            (Ink::Title, true) => grey(0xE2),
            (Ink::Motto, true) => grey(0xB4),
            (Ink::Hint, true) => grey(0x4F),
            (Ink::Skull | Ink::Motto, false) => Color::Grey,
            (Ink::Title, false) => Color::White,
            (Ink::Star | Ink::Hint, false) => Color::DarkGrey,
        }
    }

    fn draw(out: &mut impl Write, truecolor: bool, seed: u64) -> io::Result<()> {
        let (cols, rows) = terminal::size()?;
        let (cols, rows) = (usize::from(cols), usize::from(rows));
        queue!(out, ResetColor, Clear(ClearType::All))?;
        match compose(cols, rows, seed) {
            Some(grid) => {
                for (y, row) in grid.iter().enumerate() {
                    let end = row.iter().rposition(|c| c.0 != ' ').map_or(0, |i| i + 1);
                    queue!(out, MoveTo(0, y as u16))?;
                    let mut ink = None;
                    let mut run = String::new();
                    for &(c, k) in &row[..end] {
                        if c != ' ' && ink != Some(k) {
                            if let Some(prev) = ink {
                                queue!(out, SetForegroundColor(color(prev, truecolor)), Print(&run))?;
                                run.clear();
                            }
                            ink = Some(k);
                        }
                        run.push(c);
                    }
                    if let Some(k) = ink {
                        queue!(out, SetForegroundColor(color(k, truecolor)), Print(&run))?;
                    }
                }
            }
            None => {
                let (need_cols, need_rows) = needed();
                let lines = [
                    "Make the terminal bigger to see this.".to_owned(),
                    format!("It needs {need_cols} x {need_rows}; this one is {cols} x {rows}."),
                    "Maximize the window or zoom out (Ctrl + -).".to_owned(),
                    String::new(),
                    HINT.to_owned(),
                ];
                let top = rows.saturating_sub(lines.len()) / 2;
                for (i, line) in lines.iter().enumerate() {
                    let x = cols.saturating_sub(line.len()) / 2;
                    let ink = if i == lines.len() - 1 {
                        Ink::Hint
                    } else {
                        Ink::Motto
                    };
                    let text: String = line.chars().take(cols.saturating_sub(1)).collect();
                    queue!(
                        out,
                        MoveTo(x as u16, (top + i) as u16),
                        SetForegroundColor(color(ink, truecolor)),
                        Print(text)
                    )?;
                }
            }
        }
        queue!(out, ResetColor)?;
        out.flush()
    }

    /// Full screen until Enter (or Esc, or ^C: raw mode keeps the signal).
    /// Redrawn on resize, so making the window bigger shows it.
    pub fn show() -> Result<(), Fail> {
        use std::io::IsTerminal;
        if !io::stdout().is_terminal() {
            return Err(Fail::Message("This needs a terminal.".into()));
        }
        let truecolor =
            std::env::var("COLORTERM").is_ok_and(|v| v.contains("truecolor") || v.contains("24bit"));
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64);
        let result = (|| -> io::Result<()> {
            terminal::enable_raw_mode()?;
            let mut out = io::stdout();
            execute!(out, EnterAlternateScreen, Hide)?;
            draw(&mut out, truecolor, seed)?;
            loop {
                if crate::term::hung_up() {
                    return Ok(());
                }
                if !event::poll(TICK)? {
                    continue;
                }
                match event::read()? {
                    Event::Resize(..) => draw(&mut out, truecolor, seed)?,
                    Event::Key(key) if key.kind != KeyEventKind::Release => {
                        let ctrl_c =
                            key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
                        if matches!(key.code, KeyCode::Enter | KeyCode::Esc) || ctrl_c {
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
        })();
        restore_terminal();
        result.map_err(|e| Fail::Message(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(grid: &[Row]) -> Vec<String> {
        grid.iter()
            .map(|r| r.iter().map(|c| c.0).collect::<String>())
            .collect()
    }

    #[test]
    fn too_small_is_none() {
        let (c, r) = needed();
        assert!(compose(c - 1, r, 7).is_none());
        assert!(compose(c, r - 1, 7).is_none());
        assert!(compose(c, r, 7).is_some());
    }

    #[test]
    fn art_is_drawn_exactly() {
        let (c, r) = needed();
        let grid = compose(c + 30, r + 10, 42).unwrap();
        let lines = text(&grid);
        // Every line of the title appears whole and untouched by stars.
        for want in TITLE.lines() {
            let want = want.trim_end();
            assert!(
                lines.iter().any(|l| l.contains(want)),
                "missing title line {want:?}"
            );
        }
        for want in MOTTO.lines() {
            assert!(lines.iter().any(|l| l.contains(want.trim_end())));
        }
        assert!(lines.iter().any(|l| l.contains(HINT)));
        // The skull's letters are all there.
        let count = |s: &str| s.chars().filter(|&ch| ch != ' ' && ch != STAR).count();
        let skull: usize = SKULL.lines().map(count).sum();
        let drawn = grid.iter().flatten().filter(|c| c.1 == Ink::Skull).count();
        assert_eq!(skull, drawn);
    }

    #[test]
    fn skull_is_centred_and_stars_fill_the_edges() {
        let (c, r) = needed();
        let cols = c + 40;
        let grid = compose(cols, r + 20, 3).unwrap();
        let xs: Vec<usize> = grid
            .iter()
            .flat_map(|row| {
                row.iter()
                    .enumerate()
                    .filter(|(_, c)| c.1 == Ink::Skull)
                    .map(|(i, _)| i)
            })
            .collect();
        let mid = (xs.iter().min().unwrap() + xs.iter().max().unwrap()).div_ceil(2);
        assert!(mid.abs_diff(cols / 2) <= 1);
        // Stars in the top row, the bottom row and both side strips.
        let star = |row: &Row, range: std::ops::Range<usize>| row[range].iter().any(|c| c.0 == STAR);
        assert!(grid[..3].iter().any(|row| star(row, 0..cols)));
        assert!(grid[grid.len() - 3..].iter().any(|row| star(row, 0..cols)));
        assert!(grid.iter().any(|row| star(row, 0..15)));
        assert!(grid.iter().any(|row| star(row, cols - 16..cols - 1)));
        // Never in the last column.
        assert!(grid.iter().all(|row| row[cols - 1].0 == ' '));
    }
}
