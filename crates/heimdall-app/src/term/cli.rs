//! The command line (DESIGN §17.2). The same executable is the app and the
//! terminal mode; this decides which one a run is.

use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// `heimdall`: unlock, list the notes, pick one.
    Menu,
    /// `heimdall <note>`
    Open(String),
    /// `heimdall ls [filter]`
    Ls(Option<String>),
    /// `heimdall rm <note>`
    Rm(String),
    Help,
    Version,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Cli {
    pub command: Command,
    /// `--vault <path>`
    pub vault: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    /// Open the window.
    Gui,
    Run(Cli),
    /// A mistake in the arguments; the text says which.
    Usage(String),
}

/// `terminal`: stdin and stdout are a terminal. `appimage`: running from
/// the AppImage, which people start from a terminal to get the window.
///
/// With no arguments, a terminal gets the note menu and anything else
/// (the app menu, a file manager) gets the window. `--gui` always opens
/// the window; any other argument is for the terminal mode.
pub fn parse(args: Vec<OsString>, terminal: bool, appimage: bool) -> Parsed {
    let mut vault = None;
    let mut words: Vec<String> = Vec::new();
    let mut command = None;
    // After `--` every word is part of a title, even `ls` or `-x`.
    let mut literal = false;
    let mut first_is_literal = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let Some(text) = arg.to_str() else {
            // Only a path may be something other than UTF-8, and it comes
            // after --vault, below.
            return Parsed::Usage("That argument isn't valid text.".into());
        };
        if literal || !text.starts_with('-') || text == "-" {
            if words.is_empty() {
                first_is_literal = literal;
            }
            words.push(text.to_owned());
            continue;
        }
        match text {
            "--" => literal = true,
            "--gui" => return Parsed::Gui,
            "-h" | "--help" => command = Some(Command::Help),
            "-V" | "--version" => command = command.or(Some(Command::Version)),
            "-v" | "--vault" => match args.next() {
                Some(path) if !path.is_empty() => vault = Some(PathBuf::from(path)),
                _ => return Parsed::Usage("--vault needs the path of a .heim file.".into()),
            },
            _ => match text.strip_prefix("--vault=") {
                Some(path) if !path.is_empty() => vault = Some(PathBuf::from(path)),
                _ => return Parsed::Usage(format!("Unknown option {text}.")),
            },
        }
    }
    if let Some(command) = command {
        return Parsed::Run(Cli { command, vault });
    }
    let command = match words.split_first() {
        None if vault.is_some() || (terminal && !appimage) => Command::Menu,
        None => return Parsed::Gui,
        Some((first, rest)) if !first_is_literal && first == "ls" => {
            Command::Ls((!rest.is_empty()).then(|| rest.join(" ")))
        }
        Some((first, rest)) if !first_is_literal && first == "rm" => {
            if rest.is_empty() {
                return Parsed::Usage("rm needs the title of a note.".into());
            }
            Command::Rm(rest.join(" "))
        }
        Some(_) => Command::Open(words.join(" ")),
    };
    Parsed::Run(Cli { command, vault })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str], terminal: bool) -> Parsed {
        parse(args.iter().map(OsString::from).collect(), terminal, false)
    }

    fn run(command: Command) -> Parsed {
        Parsed::Run(Cli { command, vault: None })
    }

    #[test]
    fn window_or_terminal() {
        // The app menu and file managers start it without a terminal.
        assert_eq!(p(&[], false), Parsed::Gui);
        assert_eq!(p(&[], true), run(Command::Menu));
        assert_eq!(p(&["--gui"], true), Parsed::Gui);
        assert_eq!(p(&["hola.txt", "--gui"], true), Parsed::Gui);
        // From the AppImage, a bare run is the window even in a terminal.
        assert_eq!(parse(vec![], true, true), Parsed::Gui);
        assert_eq!(parse(vec!["ls".into()], true, true), run(Command::Ls(None)));
    }

    #[test]
    fn commands() {
        assert_eq!(p(&["hola.txt"], true), run(Command::Open("hola.txt".into())));
        assert_eq!(
            p(&["lista", "de", "compras"], false),
            run(Command::Open("lista de compras".into()))
        );
        assert_eq!(p(&["ls"], true), run(Command::Ls(None)));
        assert_eq!(p(&["ls", "banco"], true), run(Command::Ls(Some("banco".into()))));
        assert_eq!(p(&["rm", "recetas"], true), run(Command::Rm("recetas".into())));
        assert!(matches!(p(&["rm"], true), Parsed::Usage(_)));
        assert_eq!(p(&["--help"], false), run(Command::Help));
        assert_eq!(p(&["-V"], false), run(Command::Version));
        assert!(matches!(p(&["--frobnicate"], true), Parsed::Usage(_)));
    }

    #[test]
    fn titles_that_look_like_commands() {
        assert_eq!(p(&["--", "ls"], true), run(Command::Open("ls".into())));
        assert_eq!(p(&["--", "-notas-"], true), run(Command::Open("-notas-".into())));
        assert_eq!(p(&["rm", "--", "--gui"], true), run(Command::Rm("--gui".into())));
    }

    #[test]
    fn vault_option() {
        let with = |command| {
            Parsed::Run(Cli {
                command,
                vault: Some(PathBuf::from("/tmp/v.heim")),
            })
        };
        assert_eq!(p(&["--vault", "/tmp/v.heim"], false), with(Command::Menu));
        assert_eq!(p(&["-v", "/tmp/v.heim", "ls"], true), with(Command::Ls(None)));
        assert_eq!(
            p(&["hola", "--vault=/tmp/v.heim"], true),
            with(Command::Open("hola".into()))
        );
        assert!(matches!(p(&["--vault"], true), Parsed::Usage(_)));
    }
}
