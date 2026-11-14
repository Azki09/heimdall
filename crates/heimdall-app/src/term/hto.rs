//! A hidden full-screen picture, reached by an option `--help` does not
//! list. The gear on the skull turns and the stars drift behind it, with a
//! shooting star now and then. The layout and the animation are plain data
//! and build everywhere so they can be tested; drawing them needs the
//! terminal mode.

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

/// Frames per second.
pub const FPS: u32 = 6;

// The gear on top of the skull, in cells of skull.txt. A cell is about
// twice as tall as it is wide, so columns are divided by ASPECT. The ring
// is solid up to R_BODY; the teeth go from there to R_TIP, one every PITCH
// degrees, and only the arc above the skull shows.
const GEAR_CX: f64 = 70.5;
const GEAR_CY: f64 = 29.5;
const ASPECT: f64 = 1.95;
const R_BODY: f64 = 22.6;
const R_TIP: f64 = 25.8;
const PITCH: f64 = 30.0;
/// A cell of the gear, where the search for the rest of it starts.
const GEAR_SEED: (usize, usize) = (4, 70);
/// One tooth every two seconds, clockwise.
const TURN_PER_FRAME: f64 = PITCH / (2.0 * FPS as f64);

/// Columns per frame of the far and the near stars (half a column and one
/// column per second), to the left.
const FAR_SPEED: f64 = 0.5 / FPS as f64;
const NEAR_SPEED: f64 = 1.0 / FPS as f64;
/// One star in this many is near (brighter and faster).
const NEAR_SHARE: f64 = 0.35;
/// A shooting star: its length, the first one after this many seconds,
/// then one every 10 to 20 seconds.
const SHOT_LEN: usize = 6;
const FIRST_SHOT: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    Skull,
    Far,
    Near,
    /// A shooting star; 0 is the head, the brightest.
    Shot(u8),
    Title,
    Motto,
    Hint,
}

/// One row of the screen: each cell a char (space = empty) and its ink.
pub type Row = Vec<(char, Ink)>;

const EMPTY: (char, Ink) = (' ', Ink::Far);

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
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn one_in(&mut self, n: u64) -> bool {
        self.next().is_multiple_of(n)
    }

    /// In [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Keeps the background off row `y` and the rows above and below it, in `xs`.
fn mark(keep: &mut [Vec<bool>], y: usize, xs: std::ops::Range<usize>) {
    let last = keep.len() - 1;
    for row in &mut keep[y.saturating_sub(1)..=(y + 1).min(last)] {
        row[xs.clone()].fill(true);
    }
}

/// Distance from the gear's centre, in rows.
fn rho(r: f64, c: f64) -> f64 {
    ((c - GEAR_CX) / ASPECT).hypot(GEAR_CY - r)
}

/// Whether skull cell (`r`, `c`) is part of a tooth with the gear turned to
/// `theta` degrees.
fn tooth(r: isize, c: isize, theta: f64) -> bool {
    let (r, c) = (r as f64, c as f64);
    let p = rho(r, c);
    if p <= R_BODY || p > R_TIP {
        return false;
    }
    let a = (GEAR_CY - r)
        .atan2((c - GEAR_CX) / ASPECT)
        .to_degrees()
        .rem_euclid(360.0);
    if !(a >= 348.0 || a <= 192.0) {
        return false;
    }
    let d = (a - theta).rem_euclid(PITCH);
    d.min(PITCH - d) <= 8.0 - 1.6 * (p - R_BODY) / (R_TIP - R_BODY)
}

/// The same letter for the same cell every frame, so the teeth don't shimmer.
fn hash(r: isize, c: isize) -> u32 {
    let h = (r as u32).wrapping_mul(73_856_093) ^ (c as u32).wrapping_mul(19_349_663);
    let h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^ (h >> 16)
}

struct Star {
    x: f64,
    y: usize,
    near: bool,
}

struct Shot {
    x: f64,
    y: f64,
}

/// The screen and what moves on it.
pub struct Scene {
    cols: usize,
    rows: usize,
    /// The skull without its teeth, and the text.
    base: Vec<Row>,
    /// Cells the stars stay out of: around the skull and the text.
    keep: Vec<Vec<bool>>,
    /// Where skull.txt starts on the screen.
    left: isize,
    top: isize,
    /// Skull cells a tooth can reach.
    ring: Vec<(isize, isize)>,
    /// The letters on the teeth's outline in the art.
    edge: Vec<char>,
    theta: f64,
    stars: Vec<Star>,
    shot: Option<Shot>,
    /// Frames until the next shooting star.
    next_shot: u32,
    rng: Rng,
}

impl Scene {
    /// The scene for a `cols` x `rows` terminal, or `None` when it doesn't
    /// fit. The skull is centred on itself (its stars are uneven), the text
    /// under it, and stars fill the free cells around them.
    pub fn new(cols: usize, rows: usize, seed: u64) -> Option<Self> {
        let (need_cols, need_rows) = needed();
        if cols < need_cols || rows < need_rows {
            return None;
        }
        let mut base: Vec<Row> = vec![vec![EMPTY; cols]; rows];
        let mut keep = vec![vec![false; cols]; rows];
        let mut y = (rows - need_rows) / 2;

        // The art's own stars are left out: the moving ones replace them.
        let skull = block(SKULL);
        let (lo, hi) = skull_span(&skull);
        let left = (cols / 2) as isize - (lo + hi).div_ceil(2) as isize;
        let top = y as isize;
        for line in &skull {
            for (i, &c) in line.iter().enumerate() {
                let x = left + i as isize;
                if c == ' ' || c == STAR || x < 0 || x as usize >= cols {
                    continue;
                }
                let x = x as usize;
                base[y][x] = (c, Ink::Skull);
                mark(&mut keep, y, x.saturating_sub(2)..(x + 3).min(cols));
            }
            y += 1;
        }

        let mut put = |y: &mut usize, lines: Vec<Vec<char>>, ink: Ink| {
            *y += GAP;
            let w = width(&lines);
            let left = (cols - w) / 2;
            for line in lines {
                for (i, &c) in line.iter().enumerate() {
                    if c != ' ' {
                        base[*y][left + i] = (c, ink);
                    }
                }
                mark(&mut keep, *y, left.saturating_sub(3)..(left + w + 3).min(cols));
                *y += 1;
            }
        };
        put(&mut y, block(TITLE), Ink::Title);
        put(&mut y, block(MOTTO), Ink::Motto);
        put(&mut y, vec![HINT.chars().collect()], Ink::Hint);

        // The gear: every skull cell joined to GEAR_SEED. Its teeth come out
        // of the picture and are drawn turned each frame.
        let (sh, sw) = (skull.len(), width(&skull));
        let solid = |r: usize, c: usize| skull[r].get(c).is_some_and(|&ch| ch != ' ' && ch != STAR);
        let mut gear = vec![vec![false; sw]; sh];
        let mut todo = vec![GEAR_SEED];
        while let Some((r, c)) = todo.pop() {
            if r >= sh || c >= sw || gear[r][c] || !solid(r, c) {
                continue;
            }
            gear[r][c] = true;
            todo.extend([
                (r + 1, c),
                (r.wrapping_sub(1), c),
                (r, c + 1),
                (r, c.wrapping_sub(1)),
            ]);
        }
        let in_gear = |r: isize, c: isize| {
            r >= 0 && c >= 0 && gear.get(r as usize).and_then(|row| row.get(c as usize)) == Some(&true)
        };
        let mut edge = Vec::new();
        let mut ring = Vec::new();
        for r in 0..sh as isize {
            for c in 0..sw as isize {
                let p = rho(r as f64, c as f64);
                if p > R_BODY && p <= R_TIP {
                    ring.push((r, c));
                }
                if !in_gear(r, c) || p <= R_BODY {
                    continue;
                }
                let (y, x) = ((top + r) as usize, (left + c) as usize);
                base[y][x] = EMPTY;
                if [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .any(|(dr, dc)| !in_gear(r + dr, c + dc))
                {
                    edge.push(skull[r as usize][c as usize]);
                }
            }
        }
        if edge.is_empty() {
            edge.push('A');
        }

        let mut rng = Rng(seed | 1);
        let mut stars = Vec::new();
        for y in 0..rows {
            // The last column stays empty: writing there can scroll the screen.
            for x in 0..cols - 1 {
                if keep[y][x] || base[y][x].0 != ' ' || !rng.one_in(STAR_ONE_IN) {
                    continue;
                }
                stars.push(Star {
                    x: x as f64,
                    y,
                    near: rng.unit() < NEAR_SHARE,
                });
                if rng.one_in(PAIR_ONE_IN) {
                    let x2 = x + if rng.one_in(2) { 1 } else { 2 };
                    if x2 < cols - 1 && !keep[y][x2] {
                        stars.push(Star {
                            x: x2 as f64,
                            y,
                            near: rng.unit() < NEAR_SHARE,
                        });
                    }
                }
            }
        }

        Some(Self {
            cols,
            rows,
            base,
            keep,
            left,
            top,
            ring,
            edge,
            theta: 0.0,
            stars,
            shot: None,
            next_shot: FIRST_SHOT * FPS,
            rng,
        })
    }

    fn free(&self, x: isize, y: isize) -> bool {
        x >= 0
            && y >= 0
            && (x as usize) < self.cols - 1
            && (y as usize) < self.rows
            && !self.keep[y as usize][x as usize]
            && self.base[y as usize][x as usize].0 == ' '
    }

    /// Moves everything on by one frame.
    pub fn step(&mut self) {
        self.theta = (self.theta - TURN_PER_FRAME).rem_euclid(360.0);
        let wrap = (self.cols - 1) as f64;
        for s in &mut self.stars {
            s.x -= if s.near { NEAR_SPEED } else { FAR_SPEED };
            if s.x < 0.0 {
                s.x += wrap;
            }
        }
        match &mut self.shot {
            Some(shot) => {
                shot.x -= 3.0;
                shot.y += 1.0;
                if shot.x < -10.0 || shot.y > self.rows as f64 + 6.0 {
                    self.shot = None;
                }
            }
            None => {
                self.next_shot = self.next_shot.saturating_sub(1);
                if self.next_shot == 0 {
                    let x = 40.0 + self.rng.unit() * (self.cols as f64 - 40.0);
                    let y = self.rng.unit() * 18.0;
                    self.shot = Some(Shot { x, y });
                    self.next_shot = ((10.0 + self.rng.unit() * 10.0) * FPS as f64) as u32;
                }
            }
        }
    }

    /// What the screen shows now.
    pub fn frame(&self) -> Vec<Row> {
        let mut grid = self.base.clone();
        for s in &self.stars {
            let (x, y) = (s.x.floor() as isize, s.y as isize);
            if self.free(x, y) {
                grid[s.y][x as usize] = (STAR, if s.near { Ink::Near } else { Ink::Far });
            }
        }
        if let Some(shot) = &self.shot {
            for i in 0..SHOT_LEN {
                let x = (shot.x + i as f64 * 1.5).round() as isize;
                let y = (shot.y - i as f64 * 0.5).round() as isize;
                if self.free(x, y) {
                    let c = if i == 0 { STAR } else { '.' };
                    grid[y as usize][x as usize] = (c, Ink::Shot(i as u8));
                }
            }
        }
        let theta = self.theta;
        for &(r, c) in &self.ring {
            if !tooth(r, c, theta) {
                continue;
            }
            let (y, x) = ((self.top + r) as usize, (self.left + c) as usize);
            if self.base[y][x].0 != ' ' {
                continue;
            }
            let outline = !tooth(r - 1, c, theta)
                || !tooth(r + 1, c, theta)
                || !tooth(r, c - 1, theta)
                || !tooth(r, c + 1, theta);
            let h = hash(r, c);
            let ch = if outline {
                self.edge[h as usize % self.edge.len()]
            } else if h.is_multiple_of(9) {
                'B'
            } else {
                'A'
            };
            grid[y][x] = (ch, Ink::Skull);
        }
        grid
    }
}

/// The first frame for a `cols` x `rows` terminal, or `None` when it
/// doesn't fit.
pub fn compose(cols: usize, rows: usize, seed: u64) -> Option<Vec<Row>> {
    Scene::new(cols, rows, seed).map(|s| s.frame())
}

#[cfg(term_mode)]
pub use imp::show;

#[cfg(term_mode)]
mod imp {
    use std::io::{self, Write};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use crossterm::cursor::{Hide, MoveTo};
    use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
    use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
    use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen};
    use crossterm::{execute, queue};

    use super::{needed, Ink, Row, Scene, FPS, HINT};
    use crate::term::editor::{restore_terminal, terminal_traits};
    use crate::term::prompt::TICK;
    use crate::term::session::Fail;

    fn color(ink: Ink, truecolor: bool) -> Color {
        let grey = |v| Color::Rgb { r: v, g: v, b: v };
        match (ink, truecolor) {
            (Ink::Skull, true) => grey(0xC9),
            (Ink::Far, true) => grey(0x45),
            (Ink::Near, true) => grey(0x6A),
            (Ink::Shot(i), true) => grey(0xE2u8.saturating_sub(i.saturating_mul(0x22))),
            (Ink::Title, true) => grey(0xE2),
            (Ink::Motto, true) => grey(0xB4),
            (Ink::Hint, true) => grey(0x4F),
            (Ink::Skull | Ink::Motto, false) => Color::Grey,
            (Ink::Title, false) | (Ink::Shot(0), false) => Color::White,
            (Ink::Shot(1 | 2), false) => Color::Grey,
            (Ink::Far | Ink::Near | Ink::Shot(_) | Ink::Hint, false) => Color::DarkGrey,
        }
    }

    /// The whole frame, from a cleared screen.
    fn draw_all(out: &mut impl Write, grid: &[Row], truecolor: bool) -> io::Result<()> {
        queue!(out, ResetColor, Clear(ClearType::All))?;
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
        queue!(out, ResetColor)?;
        out.flush()
    }

    /// Only the cells that changed since `old`.
    fn draw_changes(out: &mut impl Write, old: &[Row], new: &[Row], truecolor: bool) -> io::Result<()> {
        let mut ink = None;
        for (y, (a, b)) in old.iter().zip(new).enumerate() {
            let mut at = None;
            for (x, (&was, &now)) in a.iter().zip(b).enumerate() {
                if was == now {
                    continue;
                }
                if at != Some(x) {
                    queue!(out, MoveTo(x as u16, y as u16))?;
                }
                if now.0 != ' ' && ink != Some(now.1) {
                    queue!(out, SetForegroundColor(color(now.1, truecolor)))?;
                    ink = Some(now.1);
                }
                queue!(out, Print(now.0))?;
                at = Some(x + 1);
            }
        }
        queue!(out, ResetColor)?;
        out.flush()
    }

    fn draw_too_small(out: &mut impl Write, cols: usize, rows: usize, truecolor: bool) -> io::Result<()> {
        queue!(out, ResetColor, Clear(ClearType::All))?;
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
        queue!(out, ResetColor)?;
        out.flush()
    }

    /// Builds the scene for the current size (keeping how far the gear has
    /// turned) and draws it whole.
    fn start(
        out: &mut impl Write,
        truecolor: bool,
        seed: u64,
        theta: f64,
    ) -> io::Result<Option<(Scene, Vec<Row>)>> {
        let (cols, rows) = terminal::size()?;
        let (cols, rows) = (usize::from(cols), usize::from(rows));
        match Scene::new(cols, rows, seed) {
            Some(mut scene) => {
                scene.theta = theta;
                let grid = scene.frame();
                draw_all(out, &grid, truecolor)?;
                Ok(Some((scene, grid)))
            }
            None => {
                draw_too_small(out, cols, rows, truecolor)?;
                Ok(None)
            }
        }
    }

    /// Full screen until Enter (or Esc, or ^C: raw mode keeps the signal).
    /// Built again on resize, so making the window bigger shows it.
    pub fn show() -> Result<(), Fail> {
        use std::io::IsTerminal;
        if !io::stdout().is_terminal() {
            return Err(Fail::Message("This needs a terminal.".into()));
        }
        let truecolor = terminal_traits().0;
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64);
        let frame = Duration::from_secs(1) / FPS;
        let result = (|| -> io::Result<()> {
            terminal::enable_raw_mode()?;
            let mut out = io::stdout();
            execute!(out, EnterAlternateScreen, Hide)?;
            let mut shown = start(&mut out, truecolor, seed, 0.0)?;
            let mut due = Instant::now() + frame;
            loop {
                if crate::term::hung_up() {
                    return Ok(());
                }
                let wait = due.saturating_duration_since(Instant::now()).min(TICK);
                if event::poll(wait)? {
                    match event::read()? {
                        Event::Resize(..) => {
                            let theta = shown.as_ref().map_or(0.0, |(s, _)| s.theta);
                            shown = start(&mut out, truecolor, seed, theta)?;
                        }
                        Event::Key(key) if key.kind != KeyEventKind::Release => {
                            let ctrl_c = key.code == KeyCode::Char('c')
                                && key.modifiers.contains(KeyModifiers::CONTROL);
                            if matches!(key.code, KeyCode::Enter | KeyCode::Esc) || ctrl_c {
                                return Ok(());
                            }
                        }
                        _ => {}
                    }
                }
                let now = Instant::now();
                if now < due {
                    continue;
                }
                // Late (a slow terminal, a suspended laptop): skip, don't catch up.
                due = if now - due > frame {
                    now + frame
                } else {
                    due + frame
                };
                if let Some((scene, grid)) = &mut shown {
                    scene.step();
                    let next = scene.frame();
                    draw_changes(&mut out, grid, &next, truecolor)?;
                    *grid = next;
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

    fn skull_cells(grid: &[Row]) -> usize {
        grid.iter().flatten().filter(|c| c.1 == Ink::Skull).count()
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
        // The skull is all there; the teeth are redrawn, so about the same size.
        let count = |s: &str| s.chars().filter(|&ch| ch != ' ' && ch != STAR).count();
        let skull: usize = SKULL.lines().map(count).sum();
        let drawn = skull_cells(&grid);
        assert!(drawn.abs_diff(skull) * 20 < skull, "{drawn} vs {skull}");
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

    #[test]
    fn only_the_teeth_and_the_sky_move() {
        let (c, r) = needed();
        let mut scene = Scene::new(c + 20, r + 6, 9).unwrap();
        let first = scene.frame();
        let mut moved = false;
        for _ in 0..FPS * 30 {
            scene.step();
            let now = scene.frame();
            for (y, (a, b)) in first.iter().zip(&now).enumerate() {
                for (x, (was, is)) in a.iter().zip(b).enumerate() {
                    if was == is {
                        continue;
                    }
                    moved = true;
                    let skull = [was.1, is.1].contains(&Ink::Skull);
                    if skull {
                        // A tooth: inside the ring.
                        let p = rho((y as isize - scene.top) as f64, (x as isize - scene.left) as f64);
                        assert!(p > R_BODY && p <= R_TIP, "skull changed at {x},{y}");
                    } else {
                        // Sky: never on the text or its margins, never in the last column.
                        assert!(!scene.keep[y][x] && x < scene.cols - 1, "sky at {x},{y}");
                        assert!(![was.1, is.1]
                            .iter()
                            .any(|k| matches!(k, Ink::Title | Ink::Motto | Ink::Hint)));
                    }
                }
            }
            // The gear always shows teeth.
            assert!(skull_cells(&now).abs_diff(skull_cells(&first)) * 10 < skull_cells(&first));
        }
        assert!(moved);
    }

    #[test]
    fn a_shooting_star_comes_by() {
        let (c, r) = needed();
        let mut scene = Scene::new(c, r, 5).unwrap();
        let seen = (0..FPS * 30).any(|_| {
            scene.step();
            scene
                .frame()
                .iter()
                .flatten()
                .any(|c| matches!(c.1, Ink::Shot(_)))
        });
        assert!(seen);
    }

    #[test]
    fn a_full_turn_of_one_tooth_takes_two_seconds() {
        let (c, r) = needed();
        let mut scene = Scene::new(c, r, 1).unwrap();
        for _ in 0..FPS * 2 {
            scene.step();
        }
        let back = (scene.theta - (360.0 - PITCH)).abs();
        assert!(back < 1e-9, "{}", scene.theta);
    }
}
