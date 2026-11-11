//! One-line questions on the normal screen (DESIGN §17.3): the password,
//! the note number or title, `[y/N]`. Written to stderr so
//! `heimdall ls | grep x` only pipes the titles. The terminal is in raw mode
//! only while a line is being typed, so the idle lock and a removed USB key
//! are noticed while waiting.
//!
//! What is written here is counted in screen rows, so it can be wiped once
//! it is no longer needed: the password question after the vault opens, the
//! old list when the menu is drawn again.

use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use zeroize::Zeroizing;

use super::session::LockEvent;
use super::text::{char_width, str_width};

/// How often the waiting loops look at the clock, the USB key and signals.
pub const TICK: Duration = Duration::from_millis(250);

pub enum Input {
    Line(Zeroizing<String>),
    /// Ctrl+C, Ctrl+D or Esc.
    Cancelled,
    /// `watch` said the vault must lock.
    Locked(LockEvent),
}

/// Titles that start with what was typed, for Tab.
pub type Complete<'a> = &'a dyn Fn(&str) -> Vec<String>;

/// Screen rows written since the last [`forget_rows`] or [`erase_rows`].
static ROWS: AtomicUsize = AtomicUsize::new(0);

fn columns() -> usize {
    terminal::size().map_or(80, |(w, _)| usize::from(w)).max(1)
}

/// Counts a line that is `width` columns long and ends in a line break.
fn add_rows(width: usize) {
    ROWS.fetch_add(width.div_ceil(columns()).max(1), Ordering::Relaxed);
}

/// A line on stdout that [`erase_rows`] takes away.
pub fn show(text: &str) {
    println!("{text}");
    add_rows(str_width(text));
}

/// A line on stderr that [`erase_rows`] takes away.
pub fn say(text: &str) {
    eprintln!("{text}");
    add_rows(str_width(text));
}

/// What is on screen stays there.
pub fn forget_rows() {
    ROWS.store(0, Ordering::Relaxed);
}

/// Wipes every row counted so far and leaves the cursor where the first
/// one started.
pub fn erase_rows() {
    let rows = ROWS.swap(0, Ordering::Relaxed);
    let mut err = io::stderr();
    if rows == 0 || !err.is_terminal() {
        return;
    }
    let _ = io::stdout().flush();
    let _ = write!(err, "\x1b[{rows}A\r\x1b[J");
    let _ = err.flush();
}

/// Runs `f` without counting what it writes (it is on another screen).
pub fn uncounted<T>(f: impl FnOnce() -> T) -> T {
    let before = ROWS.load(Ordering::Relaxed);
    let result = f();
    ROWS.store(before, Ordering::Relaxed);
    result
}

/// Raw mode for as long as this lives.
struct Raw;

impl Raw {
    fn on() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

/// Prints `label` and reads a line. `hidden` shows nothing while typing,
/// like `sudo`. With `complete`, Tab completes like a shell: as far as the
/// candidates agree, and if that adds nothing it lists them. `watch` runs a
/// few times a second.
pub fn read_line(
    label: &str,
    hidden: bool,
    complete: Option<Complete>,
    watch: &mut dyn FnMut() -> Option<LockEvent>,
) -> io::Result<Input> {
    let mut err = io::stderr();
    write!(err, "{label}")?;
    err.flush()?;
    let mut line = Zeroizing::new(String::with_capacity(256));
    let raw = Raw::on()?;
    let result = loop {
        if let Some(lock) = watch() {
            break Input::Locked(lock);
        }
        if !event::poll(TICK)? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Enter => break Input::Line(std::mem::take(&mut line)),
                    KeyCode::Esc => break Input::Cancelled,
                    KeyCode::Char('c' | 'd') if ctrl => break Input::Cancelled,
                    KeyCode::Char('u') if ctrl => {
                        while let Some(c) = line.pop() {
                            erase(&mut err, c, hidden)?;
                        }
                    }
                    KeyCode::Backspace => {
                        if let Some(c) = line.pop() {
                            erase(&mut err, c, hidden)?;
                        }
                    }
                    KeyCode::Tab => {
                        if let Some(complete) = complete.filter(|_| !hidden) {
                            match completion(&line, &complete(&line)) {
                                Completion::Nothing => {}
                                Completion::Line(text) if text.len() <= line.capacity() => {
                                    while let Some(c) = line.pop() {
                                        erase(&mut err, c, false)?;
                                    }
                                    line.push_str(&text);
                                    write!(err, "{text}")?;
                                }
                                Completion::Line(_) => {}
                                Completion::List(found) => {
                                    add_rows(str_width(label) + str_width(&line));
                                    write!(err, "\r\n")?;
                                    for row in in_columns(&found, columns().saturating_sub(1)) {
                                        add_rows(str_width(&row));
                                        write!(err, "{row}\r\n")?;
                                    }
                                    write!(err, "{label}{}", *line)?;
                                }
                            }
                        }
                    }
                    // Never grow past the first allocation: a moved String
                    // leaves its old copy unwiped.
                    KeyCode::Char(c)
                        if !ctrl
                            && !key.modifiers.contains(KeyModifiers::ALT)
                            && line.len() + c.len_utf8() <= line.capacity() =>
                    {
                        line.push(c);
                        if !hidden {
                            write!(err, "{c}")?;
                        }
                    }
                    _ => {}
                }
                err.flush()?;
            }
            Event::Paste(text) => {
                let text = Zeroizing::new(text);
                for c in text.chars().take_while(|&c| c != '\n' && c != '\r') {
                    if !c.is_control() && line.len() + c.len_utf8() <= line.capacity() {
                        line.push(c);
                        if !hidden {
                            write!(err, "{c}")?;
                        }
                    }
                }
                err.flush()?;
            }
            _ => {}
        }
    };
    drop(raw);
    let typed = match (&result, hidden) {
        (Input::Line(line), false) => str_width(line),
        (_, false) => str_width(&line),
        _ => 0,
    };
    add_rows(str_width(label) + typed);
    writeln!(err)?;
    Ok(result)
}

fn erase(err: &mut io::Stderr, c: char, hidden: bool) -> io::Result<()> {
    if !hidden {
        for _ in 0..char_width(c) {
            write!(err, "\x08 \x08")?;
        }
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum Completion {
    Nothing,
    /// The line becomes this.
    Line(String),
    /// Nothing to add: these are the candidates.
    List(Vec<String>),
}

/// What Tab does with `typed` when `found` are the titles that start with
/// it, upper or lower case alike.
fn completion(typed: &str, found: &[String]) -> Completion {
    let Some(first) = found.first() else {
        return Completion::Nothing;
    };
    let same = |a: char, b: char| a.to_lowercase().eq(b.to_lowercase());
    // As far as every candidate agrees, written like the first one.
    let agreed = first
        .char_indices()
        .take_while(|&(i, c)| {
            found[1..].iter().all(|other| {
                other[..]
                    .get(i..)
                    .and_then(|rest| rest.chars().next())
                    .is_some_and(|o| same(c, o))
            })
        })
        .last()
        .map_or(0, |(i, c)| i + c.len_utf8());
    let agreed = &first[..agreed];
    if agreed != typed && agreed.chars().count() >= typed.chars().count() {
        Completion::Line(agreed.to_owned())
    } else if found.len() > 1 {
        Completion::List(found.to_vec())
    } else {
        Completion::Nothing
    }
}

/// The candidates in as many columns as fit in `width`, like a shell.
fn in_columns(items: &[String], width: usize) -> Vec<String> {
    let cell = items.iter().map(|i| str_width(i)).max().unwrap_or(0) + 2;
    let per_row = (width.saturating_sub(4) / cell).max(1);
    items
        .chunks(per_row)
        .map(|row| {
            let mut line = "    ".to_owned();
            for item in row {
                line.push_str(item);
                line.push_str(&" ".repeat(cell - str_width(item)));
            }
            line.trim_end().to_owned()
        })
        .collect()
}

/// Waits `for_`, or less if Ctrl+C / Esc is pressed: then returns true.
pub fn cancelled_within(for_: Duration) -> bool {
    let Ok(_raw) = Raw::on() else {
        std::thread::sleep(for_);
        return false;
    };
    let until = std::time::Instant::now() + for_;
    loop {
        let left = until.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() || !event::poll(left).unwrap_or(false) {
            return false;
        }
        if let Ok(Event::Key(key)) = event::read() {
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            if key.code == KeyCode::Esc || (ctrl && matches!(key.code, KeyCode::Char('c' | 'd'))) {
                return true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn tab_completes_like_a_shell() {
        // One candidate: the whole title, with its own capitals.
        assert_eq!(
            completion("ho", &found(&["Hola.txt"])),
            Completion::Line("Hola.txt".into())
        );
        assert_eq!(completion("Hola.txt", &found(&["Hola.txt"])), Completion::Nothing);
        assert_eq!(completion("zz", &[]), Completion::Nothing);
        // Several: as far as they agree, then the list.
        let two = found(&["hola.txt", "hola (2).txt"]);
        assert_eq!(completion("h", &two), Completion::Line("hola".into()));
        assert_eq!(completion("hola", &two), Completion::List(two.clone()));
        assert_eq!(
            completion("", &found(&["a", "b"])),
            Completion::List(found(&["a", "b"]))
        );
        // Not only ASCII.
        assert_eq!(
            completion("c", &found(&["Cañón 1", "cañón 2"])),
            Completion::Line("Cañón ".into())
        );
    }

    #[test]
    fn candidates_in_columns() {
        let rows = in_columns(&found(&["a.txt", "notas", "c"]), 22);
        assert_eq!(rows, ["    a.txt  notas", "    c"]);
        assert_eq!(in_columns(&found(&["a", "b"]), 3), ["    a", "    b"]);
    }
}
