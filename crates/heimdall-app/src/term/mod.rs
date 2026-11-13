//! Terminal mode (DESIGN §17): open, edit, list and trash the notes of the
//! normal vault from a terminal, like nano. Linux only. Deliberately small:
//! settings, access, export and everything new live in the app.
//!
//! Everything outside the editor is ordinary terminal output; warnings and
//! questions go to stderr and start with `[heimdall]`.

pub mod cli;
pub mod hto;
pub mod paths;
pub mod text;

#[cfg(target_os = "linux")]
mod editor;
#[cfg(target_os = "linux")]
mod prompt;
#[cfg(target_os = "linux")]
mod session;

#[cfg(target_os = "linux")]
pub use linux::{hung_up, run, usage};

#[cfg(target_os = "linux")]
mod linux {
    use std::io::IsTerminal;
    use std::path::PathBuf;
    use std::process::ExitCode;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, OnceLock};

    use crossterm::cursor::MoveTo;
    use crossterm::execute;
    use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
    use heimdall_core::content::{unique_title, Note};
    use heimdall_core::secret;
    use uuid::Uuid;

    use super::cli::{Cli, Command};
    use super::editor::{self, Exit, Target};
    use super::prompt::{self, Input};
    use super::session::{self, Fail, LockEvent, Source, TermSession};
    use super::text::{str_width, truncate};
    use crate::support::{RecentEntry, RecentVaults};

    const VERSION: &str = env!("CARGO_PKG_VERSION");
    /// The logo is 79 columns wide; narrower terminals get one line.
    const BANNER_COLS: usize = 80;
    const RULE: usize = 60;
    /// Column where `edited <date>` starts in a list of notes.
    const DATE_COL: usize = 39;

    static HUNG_UP: OnceLock<Arc<AtomicBool>> = OnceLock::new();

    /// The terminal window was closed (SIGHUP) or the process was asked to
    /// end (SIGTERM): save, encrypted, and leave.
    pub fn hung_up() -> bool {
        HUNG_UP.get().is_some_and(|f| f.load(Ordering::Relaxed))
    }

    pub fn run(cli: Cli) -> ExitCode {
        let flag = HUNG_UP.get_or_init(|| Arc::new(AtomicBool::new(false)));
        for signal in [signal_hook::consts::SIGHUP, signal_hook::consts::SIGTERM] {
            let _ = signal_hook::flag::register(signal, flag.clone());
        }
        // A panic must not leave the terminal in raw mode on the alternate screen.
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            editor::restore_terminal();
            default_hook(info);
        }));

        let result = match cli.command {
            Command::Help => {
                help();
                Ok(())
            }
            Command::Version => {
                println!("heimdall {VERSION}");
                Ok(())
            }
            Command::Menu => menu(cli.vault),
            Command::Open(title) => open(cli.vault, &title),
            Command::Ls(filter) => ls(cli.vault, filter.as_deref()),
            Command::Rm(title) => rm(cli.vault, &title),
            Command::Forget(vault) => forget(&vault),
            Command::Hto => super::hto::show(),
        };
        match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(Fail::Cancelled) => ExitCode::FAILURE,
            Err(Fail::Message(text)) => {
                eprintln!("[heimdall] {text}");
                ExitCode::FAILURE
            }
        }
    }

    pub fn usage(text: &str) -> ExitCode {
        eprintln!("[heimdall] {text} Try heimdall --help.");
        ExitCode::from(2)
    }

    fn columns() -> usize {
        crossterm::terminal::size().map_or(BANNER_COLS, |(w, _)| usize::from(w))
    }

    fn rule() -> String {
        "_".repeat(RULE.min(columns().saturating_sub(1)))
    }

    /// The start-screen logo (DESIGN §17.3). Only `heimdall` and
    /// `heimdall --help` print it.
    fn banner() {
        if columns() < BANNER_COLS {
            println!("HEIMDALL v{VERSION} :: encrypted notes");
        } else {
            println!();
            println!("{}", crate::widgets::LOGO);
            println!();
            println!("    v{VERSION}  ::  encrypted notes");
        }
        println!("{}", rule());
        println!();
    }

    const MENU_PROMPT: &str = " :: Open note [number or title, n = new, ? = help, Enter = quit] : ";

    fn help() {
        banner();
        println!(" Usage: heimdall [options] [command | note]");
        println!();
        println!(" :: Commands");
        println!("    heimdall                 Unlock a vault and pick a note");
        println!("    heimdall <note>          Open a note (created when you save)");
        println!("    heimdall ls [filter]     List the titles, optionally filtered");
        println!("    heimdall rm <note>       Move a note to the trash (asks first)");
        println!("    heimdall forget <vault>  Stop remembering a vault in the terminal");
        println!("    heimdall -- <note>       Open a note called \"ls\", \"rm\" or \"-x\"");
        println!();
        println!(" :: Options");
        println!("    -v, --vault <path>       Use this vault instead of the last one");
        println!("                             (or set $HEIMDALL_VAULT)");
        println!("        --gui                Open the Heimdall window");
        println!("    -h, --help               Show this help");
        println!("    -V, --version            Show the version");
        println!();
        println!(" :: Notes");
        println!("    Titles are unique: a second \"hola.txt\" becomes \"hola (2).txt\".");
        println!("    In the list, type a number or a title; Tab completes titles.");
        println!("    There, -n, -rm, -mv and -ls act on notes; ? lists them.");
        println!("    ^T in the editor is code mode: line numbers, colors and a map.");
        println!("    ^O renames the note (Save as); ^E writes it to a plain file.");
        println!();
        println!(" :: Editor keys");
        for line in editor::help_lines() {
            println!("{line}");
        }
        println!("{}", rule());
    }

    /// `?` in the list of notes: a short help on its own screen, so the
    /// list is untouched. False = the vault locked meanwhile.
    fn menu_help(session: &mut TermSession) -> Result<bool, Fail> {
        let mut out = std::io::stdout();
        execute!(out, EnterAlternateScreen, MoveTo(0, 0))?;
        println!();
        println!(" :: List of notes");
        println!("    1, 2, 3...               Open the note with that number");
        println!("    a title                  Open the note with that title");
        println!("    Tab                      Complete the title you are typing");
        println!("    n                        New note (asks for the title)");
        println!("    -n <title>               New note with this title, no questions");
        println!("    -rm <number or title>    Move a note to the trash (asks first)");
        println!("    -mv <number or title>    Rename a note");
        println!("    -ls [filter]             List only the titles with this text;");
        println!("                             -ls alone lists them all again");
        println!("    -lock, -q                Lock the vault and quit");
        println!("    ?, -help                 This help");
        println!("    Enter                    Quit; the vault locks");
        println!();
        println!(" :: Editor keys");
        for line in editor::help_lines() {
            println!("{line}");
        }
        println!();
        println!("    heimdall --help shows the commands and the options.");
        println!();
        let input = prompt::uncounted(|| {
            prompt::read_line(" Press Enter to go back ", true, None, &mut || {
                session.lock_event()
            })
        });
        let _ = execute!(out, LeaveAlternateScreen);
        match input? {
            Input::Locked(event) => {
                report_lock(event, None, "");
                Ok(false)
            }
            _ => {
                session.touch();
                Ok(true)
            }
        }
    }

    /// The title as lists show it: secrets hidden, never empty.
    fn shown_title(note: &Note) -> String {
        match secret::hide(note.display_title()) {
            t if t.is_empty() => "Untitled".to_owned(),
            t => t,
        }
    }

    /// A note as the terminal names it.
    #[derive(Clone)]
    struct Entry {
        id: Uuid,
        /// Its title as shown, different from every other note's.
        label: String,
        modified: i64,
    }

    /// The open notes, each under a name of its own. Titles are unique
    /// already (DESIGN §10.1); notes named after their first line, or whose
    /// titles only differ in a hidden secret, get ` (2)`, ` (3)`… here, in
    /// the list only.
    fn entries(session: &TermSession) -> Vec<Entry> {
        let notes = &session.vault.content.notes;
        let mut labels: Vec<Option<String>> = vec![None; notes.len()];
        // Notes with a title of their own keep it before the others.
        for untitled in [false, true] {
            for (i, note) in notes.iter().enumerate() {
                if note.title.is_empty() == untitled {
                    let label = unique_title(&shown_title(note), |t| labels.iter().flatten().any(|l| l == t));
                    labels[i] = Some(label);
                }
            }
        }
        notes
            .iter()
            .zip(labels)
            .map(|(note, label)| Entry {
                id: note.id,
                label: label.unwrap_or_default(),
                modified: note.modified,
            })
            .collect()
    }

    /// The note called `title`: written exactly, or else the only one that
    /// matches without looking at upper and lower case.
    fn find(entries: &[Entry], title: &str) -> Option<Uuid> {
        if let Some(entry) = entries.iter().find(|e| e.label == title) {
            return Some(entry.id);
        }
        let lower = title.to_lowercase();
        let mut same = entries.iter().filter(|e| e.label.to_lowercase() == lower);
        match (same.next(), same.next()) {
            (Some(entry), None) => Some(entry.id),
            _ => None,
        }
    }

    fn date(ts: i64) -> String {
        use chrono::{Local, TimeZone};
        Local
            .timestamp_opt(ts, 0)
            .single()
            .map(|t| t.format("%Y-%m-%d").to_string())
            .unwrap_or_default()
    }

    /// ` [1] hola.txt                          edited 2026-11-08`
    fn list_line(number: usize, entry: &Entry) -> String {
        let cols = columns().saturating_sub(1);
        let label = format!(" [{number}] ");
        let date = format!("edited {}", date(entry.modified));
        let room = cols.saturating_sub(str_width(&label) + date.len() + 2).max(8);
        let mut line = format!("{label}{}", truncate(&entry.label, room));
        let pad = DATE_COL.saturating_sub(str_width(&line)).max(2);
        line.push_str(&" ".repeat(pad));
        line.push_str(&date);
        line
    }

    /// Tells why the vault locked, on the normal screen.
    fn report_lock(event: LockEvent, saved: Option<Result<(), String>>, title: &str) {
        if event == LockEvent::HungUp {
            return;
        }
        let tail = match saved {
            Some(Ok(())) => format!(" \"{title}\" was saved."),
            Some(Err(e)) => format!(" The last changes to \"{title}\" could not be saved: {e}"),
            None => String::new(),
        };
        eprintln!("[heimdall] Vault locked: {}.{tail}", event.reason());
    }

    /// Asks a question while the vault is open: the idle lock and the USB
    /// key are still watched. `titles` are what Tab completes. `None` = the
    /// vault locked (already reported).
    fn ask(session: &mut TermSession, label: &str, titles: &[String]) -> Result<Option<String>, Fail> {
        let complete = |typed: &str| -> Vec<String> {
            let typed = typed.trim_start();
            // `-rm hol` completes to `-rm hola.txt`.
            let (command, typed) = match typed.split_once(' ') {
                Some((cmd @ ("-rm" | "-mv"), rest)) => (format!("{cmd} "), rest.trim_start()),
                _ => (String::new(), typed),
            };
            let typed = typed.to_lowercase();
            titles
                .iter()
                .filter(|t| t.to_lowercase().starts_with(&typed))
                .map(|t| format!("{command}{t}"))
                .collect()
        };
        let complete: Option<prompt::Complete> = if titles.is_empty() { None } else { Some(&complete) };
        // Typing counts as activity only once the line is entered: a prompt
        // left alone still locks.
        match prompt::read_line(label, false, complete, &mut || session.lock_event())? {
            Input::Line(line) => {
                session.touch();
                Ok(Some(line.trim().to_owned()))
            }
            Input::Cancelled => Ok(Some(String::new())),
            Input::Locked(event) => {
                report_lock(event, None, "");
                Ok(None)
            }
        }
    }

    /// Runs the editor on a note called `title`. False = the vault locked:
    /// stop.
    fn edit(session: &mut TermSession, target: Target, title: &str) -> Result<bool, Fail> {
        if !std::io::stdout().is_terminal() {
            return Err(Fail::Message("The editor needs a terminal.".into()));
        }
        match editor::edit(session, target, title.to_owned())? {
            Exit::Quit => {
                session.touch();
                Ok(true)
            }
            Exit::Locked { event, saved } => {
                report_lock(event, saved, title);
                Ok(false)
            }
        }
    }

    fn edit_entry(session: &mut TermSession, entries: &[Entry], id: Uuid) -> Result<bool, Fail> {
        let title = entries
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.label.clone())
            .unwrap_or_default();
        edit(session, Target::Note(id), &title)
    }

    /// `[Y/n]`: Enter is yes too (DESIGN §17.2).
    fn yes(answer: &str) -> bool {
        matches!(answer.trim().to_lowercase().as_str(), "" | "y" | "yes")
    }

    fn clean_title(title: &str) -> Option<String> {
        let title = title.trim();
        (!title.is_empty() && !title.chars().any(char::is_control)).then(|| title.to_owned())
    }

    const BAD_TITLE: &str = "That title can't be used.";

    /// `heimdall <note>`: the note with that title, or a new one.
    fn open(vault: Option<PathBuf>, title: &str) -> Result<(), Fail> {
        let title = clean_title(title).ok_or_else(|| Fail::Message(BAD_TITLE.into()))?;
        let mut session = TermSession::unlock(session::locate(vault)?.0)?;
        let entries = entries(&session);
        match find(&entries, &title) {
            Some(id) => edit_entry(&mut session, &entries, id)?,
            None => edit(&mut session, Target::New(title.clone()), &title)?,
        };
        Ok(())
    }

    /// `heimdall`: banner, vault, password, then the list until Enter. The
    /// password question is wiped once it is answered, and the list is
    /// drawn again in the same place after each note.
    fn menu(vault: Option<PathBuf>) -> Result<(), Fail> {
        banner();
        let (path, source) = match session::given(vault) {
            Some(given) if given.0.is_file() => given,
            Some((path, _)) => {
                return Err(Fail::Message(format!("There is no vault at {}.", path.display())));
            }
            None => pick_vault()?,
        };
        println!(" :: Vault          : {}", session::vault_name(&path));
        println!(" :: Stored in      : {}", session::short_path(&path));
        println!("{}", rule());
        println!();
        let mut session = TermSession::unlock(path)?;
        if source == Source::Option && !offer_to_remember(&mut session)? {
            return Ok(());
        }
        // Something to tell above the question, the next time it is asked.
        let mut notice: Option<String> = None;
        // `-ls <filter>`: only the titles with this text, until `-ls`.
        let mut filter: Option<String> = None;
        loop {
            prompt::erase_rows();
            let all = entries(&session);
            let wanted = filter.as_deref().map(str::to_lowercase);
            // Filtered as shown: a filter can't probe what a secret holds.
            let entries: Vec<Entry> = all
                .iter()
                .filter(|e| wanted.as_ref().is_none_or(|f| e.label.to_lowercase().contains(f)))
                .map(Entry::clone)
                .collect();
            for (i, entry) in entries.iter().enumerate() {
                prompt::show(&list_line(i + 1, entry));
            }
            if !entries.is_empty() {
                prompt::show("");
            }
            if let Some(f) = &filter {
                prompt::say(&format!(
                    " :: Showing {} of {} notes (filter \"{}\"). -ls shows them all.",
                    entries.len(),
                    all.len(),
                    secret::hide(f)
                ));
                prompt::say("");
            }
            if let Some(text) = notice.take() {
                prompt::say(&format!("[heimdall] {text}"));
                prompt::say("");
            }
            let titles: Vec<String> = all.iter().map(|e| e.label.clone()).collect();
            let Some(answer) = ask(&mut session, MENU_PROMPT, &titles)? else {
                return Ok(());
            };
            let go_on = match answer.as_str() {
                "" | "q" | "Q" => return Ok(()),
                "?" => menu_help(&mut session)?,
                typed if typed.starts_with('-') => {
                    let mut menu = Menu {
                        session: &mut session,
                        all: &all,
                        shown: &entries,
                        notice: &mut notice,
                        filter: &mut filter,
                    };
                    menu.command(typed)?
                }
                "n" | "N" => {
                    let Some(title) = ask(&mut session, " :: Title : ", &[])? else {
                        return Ok(());
                    };
                    match (title.as_str(), clean_title(&title)) {
                        ("", _) => true,
                        (_, None) => {
                            notice = Some(BAD_TITLE.into());
                            true
                        }
                        (_, Some(title)) => {
                            // A taken title gets " (2)", like any new note.
                            let title = session.vault.content.free_title(&title);
                            edit(&mut session, Target::New(title.clone()), &title)?
                        }
                    }
                }
                typed => {
                    let number = typed.parse::<usize>().ok();
                    let by_number = number.and_then(|n| entries.get(n.checked_sub(1)?)).map(|e| e.id);
                    match (
                        by_number.or_else(|| find(&all, typed)),
                        number,
                        clean_title(typed),
                    ) {
                        (Some(id), _, _) => edit_entry(&mut session, &all, id)?,
                        (None, Some(n), _) => {
                            notice = Some(format!("There is no note {n}."));
                            true
                        }
                        (None, None, None) => {
                            notice = Some(BAD_TITLE.into());
                            true
                        }
                        (None, None, Some(title)) => {
                            let label = format!(
                                " :: No note is called \"{}\". Create it? [Y/n] : ",
                                secret::hide(&title)
                            );
                            let Some(answer) = ask(&mut session, &label, &[])? else {
                                return Ok(());
                            };
                            if yes(&answer) {
                                edit(&mut session, Target::New(title.clone()), &title)?
                            } else {
                                true
                            }
                        }
                    }
                }
            };
            if !go_on {
                return Ok(());
            }
        }
    }

    /// What a `-command` typed in the list of notes works on (DESIGN §17.2).
    struct Menu<'a> {
        session: &'a mut TermSession,
        /// Every open note.
        all: &'a [Entry],
        /// The ones listed, numbered from 1.
        shown: &'a [Entry],
        notice: &'a mut Option<String>,
        filter: &'a mut Option<String>,
    }

    impl Menu<'_> {
        /// Runs `-rm 3`, `-mv hola`, `-ls x`, `-lock`... False = stop: the
        /// vault locked or the user quit.
        fn command(&mut self, typed: &str) -> Result<bool, Fail> {
            let (command, arg) = match typed.split_once(' ') {
                Some((c, a)) => (c, a.trim()),
                None => (typed, ""),
            };
            match command.to_lowercase().as_str() {
                "-lock" | "-q" | "-quit" => Ok(false),
                "-help" | "-h" => menu_help(self.session),
                "-ls" => {
                    *self.filter = (!arg.is_empty()).then(|| arg.to_owned());
                    Ok(true)
                }
                "-rm" => self.remove(arg),
                "-mv" => self.rename(arg),
                "-n" | "-new" => self.new_note(arg),
                _ => match find(self.all, typed) {
                    // A note whose title starts with `-`.
                    Some(id) => edit_entry(self.session, self.all, id),
                    None => {
                        *self.notice = Some(format!(
                            "Unknown command {}. ? shows the commands.",
                            secret::hide(command)
                        ));
                        Ok(true)
                    }
                },
            }
        }

        /// The note `arg` names: a number in the list, or a title.
        fn target(&mut self, command: &str, arg: &str) -> Option<(Uuid, String)> {
            if arg.is_empty() {
                *self.notice = Some(format!(
                    "{command} needs a note: {command} 2, or {command} and a title."
                ));
                return None;
            }
            let id = match arg.parse::<usize>() {
                Ok(n) => self.shown.get(n.wrapping_sub(1)).map(|e| e.id),
                Err(_) => find(self.all, arg),
            };
            match id.and_then(|id| self.all.iter().find(|e| e.id == id)) {
                Some(entry) => Some((entry.id, entry.label.clone())),
                None => {
                    *self.notice = Some(match arg.parse::<usize>() {
                        Ok(n) => format!("There is no note {n}."),
                        Err(_) => format!("No note is called \"{}\".", secret::hide(arg)),
                    });
                    None
                }
            }
        }

        /// `-n test`: a new note, straight into the editor. A title that
        /// is taken opens that note; `-n` alone asks for the title.
        fn new_note(&mut self, arg: &str) -> Result<bool, Fail> {
            let typed = if arg.is_empty() {
                let Some(title) = ask(self.session, " :: Title : ", &[])? else {
                    return Ok(false);
                };
                if title.is_empty() {
                    return Ok(true);
                }
                title
            } else {
                arg.to_owned()
            };
            let Some(title) = clean_title(&typed) else {
                *self.notice = Some(BAD_TITLE.into());
                return Ok(true);
            };
            if let Some(id) = find(self.all, &title) {
                *self.notice = Some(format!(
                    "There was already a note \"{}\", so it was opened.",
                    secret::hide(&title)
                ));
                return edit_entry(self.session, self.all, id);
            }
            edit(self.session, Target::New(title.clone()), &title)
        }

        /// `-rm`: to the trash, always asking first.
        fn remove(&mut self, arg: &str) -> Result<bool, Fail> {
            let Some((id, shown)) = self.target("-rm", arg) else {
                return Ok(true);
            };
            let label = format!(" :: Move \"{shown}\" to trash? [Y/n] : ");
            let Some(answer) = ask(self.session, &label, &[])? else {
                return Ok(false);
            };
            if !yes(&answer) {
                *self.notice = Some("Nothing was moved.".into());
                return Ok(true);
            }
            let content = &mut self.session.vault.content;
            content.close_note(id, heimdall_core::content::now());
            *self.notice = Some(match self.session.save() {
                Ok(()) => format!("\"{shown}\" is in the trash. Restore it from the Heimdall window."),
                Err(e) => format!("Not saved: {e}"),
            });
            Ok(true)
        }

        /// `-mv`: a new title, one no other note has (DESIGN §10.1).
        fn rename(&mut self, arg: &str) -> Result<bool, Fail> {
            let Some((id, shown)) = self.target("-mv", arg) else {
                return Ok(true);
            };
            let label = format!(" :: New title for \"{shown}\" : ");
            let Some(answer) = ask(self.session, &label, &[])? else {
                return Ok(false);
            };
            if answer.is_empty() {
                return Ok(true);
            }
            let Some(wanted) = clean_title(&answer) else {
                *self.notice = Some(BAD_TITLE.into());
                return Ok(true);
            };
            let content = &mut self.session.vault.content;
            let from_extension = content.editor.code_from_extension;
            if !content.rename_note(id, &wanted) {
                *self.notice = Some("The title is the same.".into());
                return Ok(true);
            }
            let Some(note) = content.note_mut(id) else {
                return Ok(true);
            };
            // A name like `x.py` is a new reason for code mode (DESIGN §10.6).
            if from_extension && crate::code::by_title(&note.title).is_some() {
                note.code_off = false;
            }
            note.modified = heimdall_core::content::now();
            let title = secret::hide(&note.title);
            *self.notice = Some(match self.session.save() {
                Ok(()) => format!("Renamed to \"{title}\"."),
                Err(e) => format!("Not saved: {e}"),
            });
            Ok(true)
        }
    }

    /// ` [1] personal          ~/Documents/personal.heim        opened 2026-11-12`
    fn vault_line(number: usize, entry: &RecentEntry) -> String {
        let cols = columns().saturating_sub(1);
        let mut line = format!(" [{number}] {}", session::vault_name(&entry.path));
        let pad = 22usize.saturating_sub(str_width(&line)).max(2);
        line.push_str(&" ".repeat(pad));
        let date = format!("opened {}", date(entry.opened));
        let room = cols.saturating_sub(str_width(&line) + date.len() + 2).max(8);
        line.push_str(truncate(&session::short_path(&entry.path), room));
        let pad = 55usize.saturating_sub(str_width(&line)).max(2);
        line.push_str(&" ".repeat(pad));
        line.push_str(&date);
        line
    }

    /// With more than one vault remembered, which one (DESIGN §17.1). The
    /// list is wiped once the answer is given.
    fn pick_vault() -> Result<(PathBuf, Source), Fail> {
        let list = session::remembered();
        match &list[..] {
            [] => return session::locate(None),
            [only] => return Ok((only.path.clone(), Source::Remembered)),
            _ => {}
        }
        let first = session::vault_name(&list[0].path);
        let label = format!(
            " :: Vault [1-{}, Enter = {first}, p = other path, q = quit] : ",
            list.len()
        );
        let mut notice: Option<String> = None;
        loop {
            prompt::forget_rows();
            prompt::show(" :: Vaults");
            for (i, entry) in list.iter().enumerate() {
                prompt::show(&vault_line(i + 1, entry));
            }
            prompt::show("");
            if let Some(text) = notice.take() {
                prompt::say(&format!("[heimdall] {text}"));
            }
            let answer = match prompt::read_line(&label, false, None, &mut || {
                super::hung_up().then_some(LockEvent::HungUp)
            })? {
                Input::Line(line) => line.trim().to_owned(),
                _ => {
                    prompt::erase_rows();
                    return Err(Fail::Cancelled);
                }
            };
            prompt::erase_rows();
            match answer.as_str() {
                "" => return Ok((list[0].path.clone(), Source::Remembered)),
                "p" | "P" => return session::ask_path(),
                "q" | "Q" => return Err(Fail::Cancelled),
                typed => match typed
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| list.get(n.checked_sub(1)?))
                {
                    Some(entry) => return Ok((entry.path.clone(), Source::Remembered)),
                    None => notice = Some(format!("There is no vault {typed}.")),
                },
            }
        }
    }

    /// A vault opened with `--vault` whose profile keeps no recent list:
    /// may the terminal remember it? Asked every time until the answer is
    /// yes (DESIGN §17.1). False = the vault locked meanwhile.
    fn offer_to_remember(session: &mut TermSession) -> Result<bool, Fail> {
        let security = &session.vault.content.security;
        let mut terminal = RecentVaults::terminal();
        if security.remember_recent || terminal.contains(&session.path) {
            return Ok(true);
        }
        let name = session.name();
        let first = match security.profile {
            heimdall_core::Profile::Basic => {
                format!("Vault \"{name}\" has Remember recent vaults off: Heimdall doesn't keep its path,")
            }
            profile => format!("\"{name}\" is a {profile:?} vault: Heimdall doesn't keep its path,"),
        };
        prompt::say(&format!("[heimdall] {first}"));
        prompt::say("           so next time you need --vault again.");
        let label = " :: Remember it for the terminal? Its path stays on this computer. [y/N] : ";
        let Some(answer) = ask(session, label, &[])? else {
            return Ok(false);
        };
        let short = session::short_path(&session.path);
        if matches!(answer.to_lowercase().as_str(), "y" | "yes") {
            terminal.touch(&session.path);
            prompt::say(&format!(
                "[heimdall] Remembered for the terminal. heimdall forget {name} removes it."
            ));
        } else {
            prompt::say("[heimdall] Not remembered. To skip --vault without leaving it in Heimdall,");
            prompt::say(&format!(
                "           add to ~/.bashrc:  export HEIMDALL_VAULT={short}"
            ));
        }
        prompt::say("");
        // The answer stays on screen above the list.
        prompt::forget_rows();
        Ok(true)
    }

    /// `heimdall forget <vault>`: by name (`trabajo`) or path.
    fn forget(vault: &str) -> Result<(), Fail> {
        let wanted = vault.trim();
        let path = super::paths::expand_home(std::path::Path::new(wanted));
        let matches = |p: &std::path::Path| p == path || session::vault_name(p) == wanted;
        let mut terminal = RecentVaults::terminal();
        let found: Vec<PathBuf> = terminal
            .entries
            .iter()
            .map(|e| e.path.clone())
            .filter(|p| matches(p))
            .collect();
        if found.is_empty() {
            let in_app = RecentVaults::load().entries.iter().any(|e| matches(&e.path));
            return Err(Fail::Message(if in_app {
                format!("\"{wanted}\" is in the app's recent list. Turn off Remember recent vaults in its Settings.")
            } else {
                format!("No vault called \"{wanted}\" is remembered for the terminal.")
            }));
        }
        for p in &found {
            terminal.remove(p);
            eprintln!(
                "[heimdall] Vault \"{}\" is no longer remembered for the terminal.",
                session::vault_name(p)
            );
        }
        Ok(())
    }

    /// `heimdall ls [filter]`: the titles, one per line, on stdout.
    fn ls(vault: Option<PathBuf>, filter: Option<&str>) -> Result<(), Fail> {
        let session = TermSession::unlock(session::locate(vault)?.0)?;
        let filter = filter.map(str::to_lowercase);
        for entry in entries(&session) {
            // Filtered as shown: a filter can't probe what a secret holds.
            if filter
                .as_ref()
                .is_none_or(|f| entry.label.to_lowercase().contains(f))
            {
                println!("{}", entry.label);
            }
        }
        Ok(())
    }

    /// `heimdall rm <note>`: to the trash, always asking first.
    fn rm(vault: Option<PathBuf>, title: &str) -> Result<(), Fail> {
        let title = clean_title(title).ok_or_else(|| Fail::Message(BAD_TITLE.into()))?;
        let mut session = TermSession::unlock(session::locate(vault)?.0)?;
        let entries = entries(&session);
        let Some(entry) = find(&entries, &title).and_then(|id| entries.iter().find(|e| e.id == id)) else {
            return Err(Fail::Message(format!(
                "No note is called \"{}\".",
                secret::hide(&title)
            )));
        };
        let (id, shown) = (entry.id, entry.label.clone());
        let label = format!(" :: Move \"{shown}\" to trash? [Y/n] : ");
        let Some(answer) = ask(&mut session, &label, &[])? else {
            return Ok(());
        };
        if !yes(&answer) {
            eprintln!("[heimdall] Nothing was moved.");
            return Ok(());
        }
        session
            .vault
            .content
            .close_note(id, heimdall_core::content::now());
        session.save().map_err(Fail::Message)?;
        eprintln!("[heimdall] \"{shown}\" is in the trash. Restore it from the Heimdall window.");
        Ok(())
    }
}
