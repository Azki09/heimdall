//! Code mode (DESIGN §10.6): the bundled languages, detecting them from the
//! tab title, and a small highlighter. Deliberately simple: keywords,
//! types, strings, comments, numbers and calls, from tables in this file.
//! No grammar files, nothing loaded at run time, no network.

use std::ops::Range;

use eframe::egui::Color32;
use heimdall_core::content::{CodeMode, Note};

use crate::theme::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Plain,
    Keyword,
    Type,
    Function,
    String,
    Number,
    Comment,
    /// Macros, attributes, variables (`$x`), decorators.
    Macro,
    /// PowerShell parameters (`-Path`).
    Param,
}

impl Kind {
    pub fn color(self) -> Color32 {
        match self {
            Kind::Plain => TEXT_BODY,
            Kind::Keyword => SYN_KEYWORD,
            Kind::Type => SYN_TYPE,
            Kind::Function => SYN_FUNCTION,
            Kind::String => SYN_STRING,
            Kind::Number => SYN_NUMBER,
            Kind::Comment => SYN_COMMENT,
            Kind::Macro => SYN_MACRO,
            Kind::Param => ICON,
        }
    }
}

/// A colored byte range of the text. Spans are sorted and never overlap;
/// the gaps between them are plain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub kind: Kind,
}

/// The kind of the byte at `at`.
pub fn kind_at(spans: &[Span], at: usize) -> Kind {
    let i = spans.partition_point(|s| s.range.end <= at);
    match spans.get(i) {
        Some(s) if s.range.start <= at => s.kind,
        _ => Kind::Plain,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Generic,
    Markup,
    Markdown,
    Css,
}

pub struct Language {
    pub id: &'static str,
    pub name: &'static str,
    extensions: &'static [&'static str],
    family: Family,
    line_comments: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    quotes: &'static str,
    /// Quotes whose strings may span lines.
    multiline_quotes: &'static str,
    escape: Option<u8>,
    /// `"""` and `'''` strings (Python, TOML).
    triple_quotes: bool,
    keywords: &'static [&'static str],
    types: &'static [&'static str],
    ignore_case: bool,
    /// Capitalized words are types.
    capitalized_types: bool,
    /// `Name(...)` is a type, not a call (Rust: `Some(x)`, `Ok(x)`).
    capitalized_calls_are_types: bool,
    /// The word after one of these is a function name.
    fn_keywords: &'static [&'static str],
    /// `$name` is a variable (PowerShell, Bash).
    sigil: Option<u8>,
    /// `name!` is a macro, `#[...]` an attribute, `'a` a lifetime (Rust).
    rust: bool,
    /// `@name` is a decorator or annotation.
    at_decorators: bool,
    /// `#` at the start of a line is a preprocessor directive (C family).
    preprocessor: bool,
    /// Words may contain `-` (PowerShell cmdlets, YAML and TOML keys).
    dash_words: bool,
    /// A word or string followed by this is a key (YAML, TOML, JSON).
    key_sep: Option<u8>,
    /// Keys may appear anywhere, not only first on a line (JSON).
    keys_anywhere: bool,
    /// `[table]` lines (TOML).
    table_headers: bool,
}

const BASE: Language = Language {
    id: "",
    name: "",
    extensions: &[],
    family: Family::Generic,
    line_comments: &[],
    block_comment: None,
    quotes: "\"'",
    multiline_quotes: "",
    escape: Some(b'\\'),
    triple_quotes: false,
    keywords: &[],
    types: &[],
    ignore_case: false,
    capitalized_types: false,
    capitalized_calls_are_types: false,
    fn_keywords: &[],
    sigil: None,
    rust: false,
    at_decorators: false,
    preprocessor: false,
    dash_words: false,
    key_sep: None,
    keys_anywhere: false,
    table_headers: false,
};

const C_KEYWORDS: &[&str] = &[
    "auto", "break", "case", "const", "continue", "default", "do", "else", "enum", "extern", "for", "goto",
    "if", "inline", "register", "restrict", "return", "sizeof", "static", "struct", "switch", "typedef",
    "union", "volatile", "while", "NULL", "true", "false",
];
const C_TYPES: &[&str] = &[
    "bool", "char", "double", "float", "int", "long", "short", "signed", "unsigned", "void", "size_t",
    "ssize_t", "int8_t", "int16_t", "int32_t", "int64_t", "uint8_t", "uint16_t", "uint32_t", "uint64_t",
    "FILE",
];
const CPP_KEYWORDS: &[&str] = &[
    "auto",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "constexpr",
    "const_cast",
    "continue",
    "default",
    "delete",
    "do",
    "dynamic_cast",
    "else",
    "enum",
    "explicit",
    "extern",
    "false",
    "final",
    "for",
    "friend",
    "goto",
    "if",
    "inline",
    "mutable",
    "namespace",
    "new",
    "noexcept",
    "nullptr",
    "operator",
    "override",
    "private",
    "protected",
    "public",
    "reinterpret_cast",
    "return",
    "sizeof",
    "static",
    "static_cast",
    "struct",
    "switch",
    "template",
    "this",
    "throw",
    "true",
    "try",
    "typedef",
    "typename",
    "union",
    "using",
    "virtual",
    "volatile",
    "while",
];
const CPP_TYPES: &[&str] = &[
    "bool", "char", "double", "float", "int", "long", "short", "signed", "unsigned", "void", "size_t",
    "string", "vector", "map", "int32_t", "int64_t", "uint8_t", "uint32_t", "uint64_t",
];
const JS_KEYWORDS: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "null",
    "of",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "with",
    "yield",
];
const TS_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "declare",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "is",
    "keyof",
    "let",
    "namespace",
    "new",
    "null",
    "of",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "yield",
];
const TS_TYPES: &[&str] = &[
    "any", "bigint", "boolean", "never", "number", "object", "string", "symbol", "unknown",
];

pub static LANGUAGES: &[Language] = &[
    Language {
        id: "bash",
        name: "Bash",
        extensions: &["sh", "bash", "zsh"],
        line_comments: &["#"],
        multiline_quotes: "\"'",
        keywords: &[
            "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "in",
            "function", "return", "local", "export", "readonly", "declare", "unset", "shift", "exit",
            "break", "continue", "select", "source", "alias", "true", "false",
        ],
        fn_keywords: &["function"],
        sigil: Some(b'$'),
        ..BASE
    },
    Language {
        id: "c",
        name: "C",
        extensions: &["c", "h"],
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        keywords: C_KEYWORDS,
        types: C_TYPES,
        preprocessor: true,
        ..BASE
    },
    Language {
        id: "cpp",
        name: "C++",
        extensions: &["cpp", "cc", "cxx", "hpp", "hh", "hxx"],
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        keywords: CPP_KEYWORDS,
        types: CPP_TYPES,
        capitalized_types: true,
        preprocessor: true,
        ..BASE
    },
    Language {
        id: "csharp",
        name: "C#",
        extensions: &["cs"],
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        keywords: &[
            "abstract",
            "as",
            "async",
            "await",
            "base",
            "break",
            "case",
            "catch",
            "checked",
            "class",
            "const",
            "continue",
            "default",
            "delegate",
            "do",
            "else",
            "enum",
            "event",
            "explicit",
            "extern",
            "false",
            "finally",
            "fixed",
            "for",
            "foreach",
            "get",
            "goto",
            "if",
            "implicit",
            "in",
            "init",
            "interface",
            "internal",
            "is",
            "lock",
            "namespace",
            "new",
            "null",
            "operator",
            "out",
            "override",
            "params",
            "private",
            "protected",
            "public",
            "readonly",
            "record",
            "ref",
            "return",
            "sealed",
            "set",
            "sizeof",
            "stackalloc",
            "static",
            "struct",
            "switch",
            "this",
            "throw",
            "true",
            "try",
            "typeof",
            "unchecked",
            "unsafe",
            "using",
            "value",
            "var",
            "virtual",
            "volatile",
            "while",
            "yield",
        ],
        types: &[
            "bool", "byte", "char", "decimal", "double", "dynamic", "float", "int", "long", "object",
            "sbyte", "short", "string", "uint", "ulong", "ushort", "void",
        ],
        capitalized_types: true,
        preprocessor: true,
        ..BASE
    },
    Language {
        id: "css",
        name: "CSS",
        extensions: &["css"],
        family: Family::Css,
        block_comment: Some(("/*", "*/")),
        ..BASE
    },
    Language {
        id: "go",
        name: "Go",
        extensions: &["go"],
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        quotes: "\"'`",
        multiline_quotes: "`",
        keywords: &[
            "break",
            "case",
            "chan",
            "const",
            "continue",
            "default",
            "defer",
            "else",
            "fallthrough",
            "for",
            "func",
            "go",
            "goto",
            "if",
            "import",
            "interface",
            "map",
            "package",
            "range",
            "return",
            "select",
            "struct",
            "switch",
            "type",
            "var",
            "true",
            "false",
            "nil",
            "iota",
        ],
        types: &[
            "any",
            "bool",
            "byte",
            "complex64",
            "complex128",
            "error",
            "float32",
            "float64",
            "int",
            "int8",
            "int16",
            "int32",
            "int64",
            "rune",
            "string",
            "uint",
            "uint8",
            "uint16",
            "uint32",
            "uint64",
            "uintptr",
        ],
        fn_keywords: &["func"],
        ..BASE
    },
    Language {
        id: "html",
        name: "HTML",
        extensions: &["html", "htm"],
        family: Family::Markup,
        ..BASE
    },
    Language {
        id: "java",
        name: "Java",
        extensions: &["java"],
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        keywords: &[
            "abstract",
            "assert",
            "break",
            "case",
            "catch",
            "class",
            "const",
            "continue",
            "default",
            "do",
            "else",
            "enum",
            "extends",
            "false",
            "final",
            "finally",
            "for",
            "goto",
            "if",
            "implements",
            "import",
            "instanceof",
            "interface",
            "native",
            "new",
            "null",
            "package",
            "private",
            "protected",
            "public",
            "record",
            "return",
            "static",
            "strictfp",
            "super",
            "switch",
            "synchronized",
            "this",
            "throw",
            "throws",
            "transient",
            "true",
            "try",
            "var",
            "volatile",
            "while",
        ],
        types: &[
            "boolean", "byte", "char", "double", "float", "int", "long", "short", "void",
        ],
        capitalized_types: true,
        at_decorators: true,
        ..BASE
    },
    Language {
        id: "javascript",
        name: "JavaScript",
        extensions: &["js", "mjs", "cjs", "jsx"],
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        quotes: "\"'`",
        multiline_quotes: "`",
        keywords: JS_KEYWORDS,
        capitalized_types: true,
        fn_keywords: &["function"],
        ..BASE
    },
    Language {
        id: "json",
        name: "JSON",
        extensions: &["json", "jsonc"],
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        quotes: "\"",
        keywords: &["true", "false", "null"],
        key_sep: Some(b':'),
        keys_anywhere: true,
        ..BASE
    },
    Language {
        id: "markdown",
        name: "Markdown",
        extensions: &["md", "markdown"],
        family: Family::Markdown,
        ..BASE
    },
    Language {
        id: "powershell",
        name: "PowerShell",
        extensions: &["ps1", "psm1", "psd1"],
        line_comments: &["#"],
        block_comment: Some(("<#", "#>")),
        multiline_quotes: "\"'",
        escape: Some(b'`'),
        keywords: &[
            "begin",
            "break",
            "catch",
            "class",
            "continue",
            "data",
            "do",
            "dynamicparam",
            "else",
            "elseif",
            "end",
            "enum",
            "exit",
            "filter",
            "finally",
            "for",
            "foreach",
            "function",
            "if",
            "in",
            "param",
            "process",
            "return",
            "switch",
            "throw",
            "trap",
            "try",
            "until",
            "using",
            "while",
        ],
        types: &[
            "array",
            "bool",
            "byte",
            "char",
            "datetime",
            "double",
            "hashtable",
            "int",
            "long",
            "object",
            "pscustomobject",
            "psobject",
            "string",
            "void",
            "securestring",
        ],
        ignore_case: true,
        fn_keywords: &["function", "filter"],
        sigil: Some(b'$'),
        dash_words: true,
        ..BASE
    },
    Language {
        id: "python",
        name: "Python",
        extensions: &["py", "pyw"],
        line_comments: &["#"],
        triple_quotes: true,
        keywords: &[
            "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
            "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda",
            "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield", "None",
            "True", "False", "self",
        ],
        types: &[
            "bool", "bytes", "dict", "float", "int", "list", "object", "set", "str", "tuple",
        ],
        capitalized_types: true,
        fn_keywords: &["def"],
        at_decorators: true,
        ..BASE
    },
    Language {
        id: "rust",
        name: "Rust",
        extensions: &["rs"],
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        quotes: "\"'",
        multiline_quotes: "\"",
        keywords: &[
            "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
            "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
            "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe",
            "use", "where", "while", "yield",
        ],
        types: &[
            "bool", "char", "str", "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64",
            "i128", "isize", "f32", "f64",
        ],
        capitalized_types: true,
        capitalized_calls_are_types: true,
        fn_keywords: &["fn"],
        rust: true,
        ..BASE
    },
    Language {
        id: "sql",
        name: "SQL",
        extensions: &["sql"],
        line_comments: &["--"],
        block_comment: Some(("/*", "*/")),
        escape: None,
        keywords: &[
            "add",
            "all",
            "alter",
            "and",
            "as",
            "asc",
            "begin",
            "between",
            "by",
            "case",
            "check",
            "commit",
            "constraint",
            "create",
            "default",
            "delete",
            "desc",
            "distinct",
            "drop",
            "else",
            "end",
            "exists",
            "foreign",
            "from",
            "full",
            "group",
            "having",
            "if",
            "in",
            "index",
            "inner",
            "insert",
            "into",
            "is",
            "join",
            "key",
            "left",
            "like",
            "limit",
            "not",
            "null",
            "offset",
            "on",
            "or",
            "order",
            "outer",
            "primary",
            "references",
            "returning",
            "right",
            "rollback",
            "select",
            "set",
            "table",
            "then",
            "transaction",
            "union",
            "unique",
            "update",
            "values",
            "view",
            "when",
            "where",
            "with",
        ],
        types: &[
            "bigint",
            "blob",
            "bool",
            "boolean",
            "char",
            "date",
            "datetime",
            "decimal",
            "double",
            "float",
            "int",
            "integer",
            "numeric",
            "real",
            "serial",
            "smallint",
            "text",
            "time",
            "timestamp",
            "tinyint",
            "uuid",
            "varchar",
        ],
        ignore_case: true,
        ..BASE
    },
    Language {
        id: "toml",
        name: "TOML",
        extensions: &["toml"],
        line_comments: &["#"],
        multiline_quotes: "",
        triple_quotes: true,
        keywords: &["true", "false"],
        dash_words: true,
        key_sep: Some(b'='),
        table_headers: true,
        ..BASE
    },
    Language {
        id: "typescript",
        name: "TypeScript",
        extensions: &["ts", "tsx", "mts", "cts"],
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        quotes: "\"'`",
        multiline_quotes: "`",
        keywords: TS_KEYWORDS,
        types: TS_TYPES,
        capitalized_types: true,
        fn_keywords: &["function"],
        at_decorators: true,
        ..BASE
    },
    Language {
        id: "xml",
        name: "XML",
        extensions: &["xml", "svg", "xaml", "csproj", "props", "plist", "config"],
        family: Family::Markup,
        ..BASE
    },
    Language {
        id: "yaml",
        name: "YAML",
        extensions: &["yml", "yaml"],
        line_comments: &["#"],
        keywords: &[
            "true", "false", "null", "yes", "no", "on", "off", "True", "False", "Null",
        ],
        dash_words: true,
        key_sep: Some(b':'),
        ..BASE
    },
];

/// Code mode with no language detected: strings, comments, numbers, calls.
static GENERIC: Language = Language {
    id: "",
    name: "Code",
    line_comments: &["//", "#"],
    block_comment: Some(("/*", "*/")),
    quotes: "\"'`",
    ..BASE
};

impl PartialEq for Language {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Language {
    /// For the picker's search: `query` is lowercase.
    pub fn matches(&self, query: &str) -> bool {
        self.name.to_lowercase().contains(query)
            || self.id.contains(query)
            || self.extensions.contains(&query)
    }
}

pub fn by_id(id: &str) -> Option<&'static Language> {
    LANGUAGES.iter().find(|l| l.id == id)
}

impl Language {
    /// The usual file extension ("rs"), for plain-text export (DESIGN §10.9).
    pub fn extension(&self) -> Option<&'static str> {
        self.extensions.first().copied()
    }
}

/// The language a file extension belongs to (`ext` without the dot).
pub fn by_extension(ext: &str) -> Option<&'static Language> {
    let ext = ext.to_ascii_lowercase();
    LANGUAGES.iter().find(|l| l.extensions.contains(&ext.as_str()))
}

/// The language a note's own title names by its extension: `main.py`,
/// `notes.md`. A title taken from the first line doesn't count.
pub fn by_title(title: &str) -> Option<&'static Language> {
    let (stem, ext) = title.trim().rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    by_extension(ext)
}

/// Code mode from the file extension (DESIGN §10.6): a note named like a
/// source file is in code mode, unless it was turned off by hand. Returns
/// true if it turned it on.
pub fn auto_code(note: &mut Note, enabled: bool) -> bool {
    if !enabled || note.code.is_some() || note.code_off || by_title(&note.title).is_none() {
        return false;
    }
    note.code = Some(CodeMode {
        language: String::new(),
    });
    note.wrap = None;
    true
}

/// Every extension of every language, for the Import text files dialog.
pub fn all_extensions() -> Vec<&'static str> {
    LANGUAGES
        .iter()
        .flat_map(|l| l.extensions.iter().copied())
        .collect()
}

/// Guesses the language from the tab title's extension ("sync.ps1"), then
/// from a `#!` line or an XML/HTML start.
pub fn detect(note: &Note) -> Option<&'static Language> {
    let title = note.display_title().trim();
    if let Some((_, ext)) = title.rsplit_once('.') {
        let ext = ext.to_ascii_lowercase();
        if let Some(l) = LANGUAGES.iter().find(|l| l.extensions.contains(&ext.as_str())) {
            return Some(l);
        }
    }
    let body = note.body.trim_start();
    let first = body.lines().next().unwrap_or("");
    if let Some(shebang) = first.strip_prefix("#!") {
        let id = [
            ("python", "python"),
            ("pwsh", "powershell"),
            ("node", "javascript"),
            ("bash", "bash"),
            ("zsh", "bash"),
            ("sh", "bash"),
        ]
        .into_iter()
        .find(|(word, _)| shebang.contains(word))
        .map(|(_, id)| id)?;
        return by_id(id);
    }
    let head: String = body.chars().take(15).collect::<String>().to_ascii_lowercase();
    if head.starts_with("<?xml") {
        by_id("xml")
    } else if head.starts_with("<!doctype html") || head.starts_with("<html") {
        by_id("html")
    } else {
        guess_from_text(body)
    }
}

/// How many lines [`guess_from_text`] looks at.
const GUESS_LINES: usize = 50;
/// The points the best language needs, so that prose stays plain.
const GUESS_MIN: u32 = 3;

/// A clue in one line of text: the language it points to, and how much.
struct Clue {
    id: &'static str,
    points: u32,
    test: fn(&str) -> bool,
}

const fn clue(id: &'static str, points: u32, test: fn(&str) -> bool) -> Clue {
    Clue { id, points, test }
}

fn starts_word(line: &str, word: &str) -> bool {
    line.strip_prefix(word)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '(', '\t']))
}

/// `name: value` with a lowercase key (YAML).
fn yaml_key(line: &str) -> bool {
    let Some((key, rest)) = line.split_once(':') else {
        return false;
    };
    !key.is_empty()
        && key.as_bytes()[0].is_ascii_lowercase()
        && key
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
        && (rest.is_empty() || rest.starts_with(' '))
}

/// Every clue counts once, however many lines have it: ten lines of
/// `- item` are one clue, not ten.
static CLUES: &[Clue] = &[
    // Python
    clue("python", 3, |l| {
        l.starts_with("def ") && l.contains('(') && l.ends_with(':')
    }),
    clue("python", 3, |l| l.starts_with("from ") && l.contains(" import ")),
    clue("python", 3, |l| l.starts_with("if __name__")),
    clue("python", 2, |l| starts_word(l, "elif") && l.ends_with(':')),
    clue("python", 2, |l| {
        (starts_word(l, "except") || l == "try:" || l == "else:") && l.ends_with(':')
    }),
    clue("python", 1, |l| {
        l.starts_with("import ") && !l.ends_with(';') && !l.contains(" from ") && !l.contains('{')
    }),
    clue("python", 1, |l| l.starts_with("print(") && !l.ends_with(';')),
    clue("python", 1, |l| l.contains("self.")),
    clue("python", 1, |l| {
        (l.starts_with("class ") || starts_word(l, "for") || starts_word(l, "while") || starts_word(l, "if"))
            && l.ends_with(':')
    }),
    // C, and C++ on top of it
    clue("c", 3, |l| l.starts_with("#include")),
    clue("c", 1, |l| l.starts_with("#include") && l.contains(".h")),
    clue("c", 1, |l| {
        l.contains("printf(") || l.contains("malloc(") || l.contains("scanf(")
    }),
    clue("c", 1, |l| {
        l.starts_with("int main(") || l.starts_with("void ") && l.contains('(')
    }),
    clue("cpp", 2, |l| {
        l.starts_with("#include <") && !l.contains(".h") && l.ends_with('>')
    }),
    clue("cpp", 2, |l| l.contains("std::")),
    clue("cpp", 2, |l| l.contains("cout <<") || l.contains("cin >>")),
    clue("cpp", 1, |l| {
        l.starts_with("template<") || l.starts_with("template <")
    }),
    clue("cpp", 1, |l| l.starts_with("using namespace ")),
    // Rust
    clue("rust", 3, |l| l.starts_with("fn main()")),
    clue("rust", 3, |l| {
        l.starts_with("use std::") || l.starts_with("use crate::")
    }),
    clue("rust", 2, |l| l.contains("let mut ")),
    clue("rust", 2, |l| {
        l.contains("println!(") || l.contains("format!(") || l.contains("vec![")
    }),
    clue("rust", 2, |l| l.starts_with("impl ") || l.starts_with("impl<")),
    clue("rust", 2, |l| {
        l.starts_with("pub fn ") || l.starts_with("fn ") && l.contains("->")
    }),
    clue("rust", 1, |l| l.starts_with("#[derive(")),
    // JavaScript, and TypeScript on top of it
    clue("javascript", 3, |l| l.contains("console.log(")),
    clue("javascript", 2, |l| l.contains("=> {") || l.contains(") =>")),
    clue("javascript", 2, |l| {
        l.contains("require(") || l.contains("document.") || l.contains("window.")
    }),
    clue("javascript", 2, |l| {
        l.starts_with("export default ")
            || l.starts_with("export const ")
            || l.starts_with("export function ")
    }),
    clue("javascript", 2, |l| {
        l.starts_with("import ") && (l.contains(" from '") || l.contains(" from \""))
    }),
    clue("javascript", 1, |l| l.starts_with("function ") && l.contains('(')),
    clue("javascript", 1, |l| {
        (l.starts_with("const ") || l.starts_with("let ") || l.starts_with("var ")) && l.contains(" = ")
    }),
    clue("typescript", 2, |l| {
        l.starts_with("interface ") || l.starts_with("export interface ")
    }),
    clue("typescript", 2, |l| {
        [": string", ": number", ": boolean", ": void", ": any"]
            .iter()
            .any(|t| l.contains(t))
    }),
    clue("typescript", 1, |l| l.starts_with("type ") && l.contains(" = ")),
    // Bash
    clue("bash", 3, |l| l.starts_with("if [ ") || l.starts_with("if [[ ")),
    clue("bash", 2, |l| l == "fi" || l == "done" || l == "esac"),
    clue("bash", 2, |l| l.ends_with("; then") || l.ends_with("; do")),
    clue("bash", 1, |l| l.starts_with("echo ")),
    clue("bash", 1, |l| l.contains("$(") || l.contains("${")),
    clue("bash", 1, |l| l.starts_with("export ") && l.contains('=')),
    clue("bash", 1, |l| starts_word(l, "sudo")),
    // PowerShell
    clue("powershell", 3, |l| {
        l.contains("Write-Host") || l.contains("$PSVersionTable")
    }),
    clue("powershell", 2, |l| {
        ["Get-", "Set-", "New-", "Remove-", "Invoke-", "Import-", "Export-"]
            .iter()
            .any(|v| l.contains(v))
    }),
    clue("powershell", 2, |l| {
        l.starts_with("param(") || l.starts_with("[CmdletBinding")
    }),
    clue("powershell", 1, |l| {
        [" -eq ", " -ne ", " -gt ", " -lt ", " -and ", " -or "]
            .iter()
            .any(|o| l.contains(o))
    }),
    clue("powershell", 1, |l| l.contains("$_.") || l.contains("$env:")),
    // HTML
    clue("html", 2, |l| {
        [
            "<div", "<p>", "<span", "<a href", "<body", "<head", "<script", "<ul>", "<li>", "<table", "<img ",
        ]
        .iter()
        .any(|t| l.to_ascii_lowercase().contains(t))
    }),
    clue("html", 1, |l| l.contains("</")),
    // SQL
    clue("sql", 3, |l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("select ") && (l.contains(" from ") || l.ends_with(','))
            || l.starts_with("create table")
            || l.starts_with("insert into")
    }),
    clue("sql", 1, |l| {
        let l = l.to_ascii_lowercase();
        l.starts_with("where ")
            || l.starts_with("from ")
            || l.starts_with("order by ")
            || l.starts_with("join ")
    }),
    // Go
    clue("go", 3, |l| l == "package main" || l.starts_with("import (")),
    clue("go", 2, |l| l.starts_with("func ")),
    clue("go", 2, |l| l.contains("fmt.")),
    clue("go", 1, |l| l.contains(" := ")),
    // Java
    clue("java", 3, |l| {
        l.contains("System.out.print") || l.starts_with("import java.")
    }),
    clue("java", 3, |l| l.contains("public static void main")),
    clue("java", 2, |l| l.starts_with("public class ")),
    // C#
    clue("csharp", 3, |l| {
        l.starts_with("using System") || l.contains("Console.Write")
    }),
    clue("csharp", 2, |l| {
        l.starts_with("public class ") || l.starts_with("namespace ")
    }),
    // Markdown
    clue("markdown", 2, |l| {
        let hashes = l.bytes().take_while(|c| *c == b'#').count();
        (1..=6).contains(&hashes) && l[hashes..].starts_with(' ')
    }),
    clue("markdown", 2, |l| l.starts_with("```")),
    clue("markdown", 1, |l| {
        l.starts_with("- ") || l.starts_with("* ") || l.starts_with("1. ")
    }),
    clue("markdown", 1, |l| l.contains("](")),
    clue("markdown", 1, |l| l.contains("**")),
    // CSS
    clue("css", 2, |l| {
        (l.starts_with('.') || l.starts_with('#') || starts_word(l, "body")) && l.ends_with('{')
    }),
    clue("css", 2, |l| l.starts_with("@media") || l.starts_with("@import")),
    clue("css", 2, |l| {
        l.ends_with(';')
            && l.split_once(':')
                .is_some_and(|(k, _)| !k.is_empty() && k.bytes().all(|c| c.is_ascii_lowercase() || c == b'-'))
    }),
];

/// Guesses a language from the text alone, for a note whose title says
/// nothing (DESIGN §10.6): clues in its first lines add points to a
/// language, and the best one wins if it has [`GUESS_MIN`] and no other
/// has as many. Ordinary text gets `None`. Only suggests: it never turns
/// code mode on.
pub fn guess_from_text(text: &str) -> Option<&'static Language> {
    // The start is enough, and keeps this cheap on a long note.
    let mut end = text.len().min(8 * 1024);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let whole = end == text.len();
    let text = &text[..end];
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(GUESS_LINES)
        .collect();
    if lines.len() < 2 {
        return None;
    }
    let mut points: Vec<(&'static str, u32)> = Vec::new();
    let mut add = |id: &'static str, n: u32| match points.iter_mut().find(|(i, _)| *i == id) {
        Some((_, p)) => *p += n,
        None => points.push((id, n)),
    };
    for clue in CLUES {
        if lines.iter().any(|l| (clue.test)(l)) {
            add(clue.id, clue.points);
        }
    }
    // Whole-text shapes.
    let all = text.trim();
    let closed = |open: char, close: char| all.starts_with(open) && (!whole || all.ends_with(close));
    if (closed('{', '}') || closed('[', ']')) && all.contains("\":") {
        add("json", 4);
    }
    let yaml_lines = lines.iter().filter(|l| yaml_key(l)).count();
    let nested = text
        .lines()
        .any(|l| l.starts_with([' ', '\t']) && (yaml_key(l.trim()) || l.trim_start().starts_with("- ")));
    let yamlish = lines
        .iter()
        .all(|l| yaml_key(l) || l.starts_with("- ") || l.starts_with('#') || *l == "---");
    if yaml_lines >= 3 && nested && yamlish {
        add("yaml", 4);
    }
    let toml_keys = lines
        .iter()
        .filter(|l| l.split_once(" = ").is_some_and(|(k, _)| !k.contains(' ')))
        .count();
    if toml_keys >= 2 && lines.iter().any(|l| l.starts_with('[') && l.ends_with(']')) {
        add("toml", 4);
    }
    // A language built on another gets the other's points too, when it
    // has some of its own.
    let get = |points: &[(&str, u32)], id: &str| points.iter().find(|(i, _)| *i == id).map_or(0, |(_, p)| *p);
    for (child, parent) in [("cpp", "c"), ("typescript", "javascript")] {
        let base = get(&points, parent);
        if let Some((_, p)) = points.iter_mut().find(|(i, _)| *i == child) {
            *p += base;
        }
    }
    points.sort_by_key(|(_, p)| std::cmp::Reverse(*p));
    match points[..] {
        [(id, best), ..] if best >= GUESS_MIN && points.get(1).is_none_or(|(_, second)| *second < best) => {
            by_id(id)
        }
        _ => None,
    }
}

/// `tets` → `tets.py`: a note with a title of its own and no extension,
/// whose language is known (chosen in code mode, or guessed from the
/// text). Only a suggestion, for Save as and Rename.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn suggested_title(note: &Note) -> Option<String> {
    let title = note.title.trim();
    if title.is_empty() || has_extension(title) {
        return None;
    }
    let language = match &note.code {
        Some(code) => resolve(code, note),
        None => detect(note),
    }?;
    Some(format!("{title}.{}", language.extension()?))
}

/// What the window offers for a tab whose text looks like code (DESIGN
/// §10.6): the name to start from and the language. The name is the
/// tab's own title, or for an untitled tab the first function or class
/// in the text (`fetch` from `def fetch(url):`), or empty. Tabs whose
/// title already has an extension get nothing.
pub fn code_offer(note: &Note) -> Option<(String, &'static Language)> {
    let title = note.title.trim();
    if !title.is_empty() && has_extension(title) {
        return None;
    }
    let language = match &note.code {
        Some(code) => resolve(code, note),
        None => detect(note),
    }?;
    language.extension()?;
    let stem = if title.is_empty() {
        name_from_text(&note.body).unwrap_or_default()
    } else {
        title.to_owned()
    };
    Some((stem, language))
}

/// `def fetch(url):` → `fetch`; also `fn`, `func`, `function`, `class`.
fn name_from_text(text: &str) -> Option<String> {
    const STARTS: &[&str] = &[
        "def ",
        "async def ",
        "fn ",
        "pub fn ",
        "func ",
        "function ",
        "class ",
        "pub struct ",
        "struct ",
    ];
    text.lines().take(GUESS_LINES).find_map(|line| {
        let line = line.trim_start();
        let rest = STARTS.iter().find_map(|s| line.strip_prefix(s))?;
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .take(40)
            .collect();
        (!name.is_empty()).then_some(name)
    })
}

/// `notes.txt`, `x.py`: the title ends in a short extension of its own,
/// known or not. A name chosen with an extension is left alone.
fn has_extension(title: &str) -> bool {
    title.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty() && (1..=5).contains(&ext.len()) && ext.bytes().all(|c| c.is_ascii_alphanumeric())
    })
}

/// The language a code-mode tab uses: the chosen one, or the detected one.
/// `None` = nothing detected; highlighted as generic code.
pub fn resolve(code: &CodeMode, note: &Note) -> Option<&'static Language> {
    if code.language.is_empty() {
        detect(note)
    } else {
        by_id(&code.language)
    }
}

/// What the status bar shows for a tab.
pub fn status_label(note: &Note) -> &'static str {
    match &note.code {
        None => "Plain text",
        Some(code) => resolve(code, note).map_or(GENERIC.name, |l| l.name),
    }
}

pub fn highlight(lang: Option<&Language>, text: &str) -> Vec<Span> {
    let lang = lang.unwrap_or(&GENERIC);
    match lang.family {
        Family::Generic => generic(lang, text),
        Family::Markup => markup(text),
        Family::Markdown => markdown(text),
        Family::Css => css(text),
    }
}

#[derive(Default)]
struct Out(Vec<Span>);

impl Out {
    fn push(&mut self, start: usize, end: usize, kind: Kind) {
        if start >= end || kind == Kind::Plain {
            return;
        }
        if let Some(last) = self.0.last_mut() {
            if last.kind == kind && last.range.end == start {
                last.range.end = end;
                return;
            }
        }
        self.0.push(Span {
            range: start..end,
            kind,
        });
    }
}

fn starts(b: &[u8], i: usize, s: &str) -> bool {
    b[i..].starts_with(s.as_bytes())
}

fn find(b: &[u8], from: usize, s: &str) -> Option<usize> {
    let s = s.as_bytes();
    (from..b.len()).find(|&j| b[j..].starts_with(s))
}

fn line_end(b: &[u8], from: usize) -> usize {
    (from..b.len()).find(|&j| b[j] == b'\n').unwrap_or(b.len())
}

/// Letters, `_` and any non-ASCII byte (so "café" stays one word and spans
/// always end on a character boundary).
fn word_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

fn word_char(c: u8, dash: bool) -> bool {
    word_start(c) || c.is_ascii_digit() || (dash && c == b'-')
}

fn word_end(b: &[u8], from: usize, dash: bool) -> usize {
    let mut j = from;
    while j < b.len() && word_char(b[j], dash) {
        j += 1;
    }
    // A word never ends in '-' ("$a -" is not "a-").
    while dash && j > from + 1 && b[j - 1] == b'-' {
        j -= 1;
    }
    j
}

fn next_non_space(b: &[u8], from: usize) -> Option<u8> {
    b[from..].iter().copied().find(|c| *c != b' ' && *c != b'\t')
}

fn number_end(b: &[u8], from: usize) -> usize {
    let mut j = from;
    while j < b.len() {
        let c = b[j];
        if c.is_ascii_alphanumeric() || c == b'_' {
            j += 1;
        } else if c == b'.' && b.get(j + 1).is_some_and(u8::is_ascii_digit) {
            j += 2;
        } else {
            break;
        }
    }
    j
}

fn string_end(l: &Language, b: &[u8], from: usize, quote: u8) -> usize {
    let multiline = l.multiline_quotes.as_bytes().contains(&quote);
    let mut j = from + 1;
    while j < b.len() {
        let c = b[j];
        if Some(c) == l.escape && j + 1 < b.len() && b[j + 1] != b'\n' {
            j += 2;
            continue;
        }
        if c == quote {
            return j + 1;
        }
        if c == b'\n' && !multiline {
            return j;
        }
        j += 1;
    }
    b.len()
}

/// `$name`, `${...}`, `$(...)`, `$env:NAME`, `$1`, `$?`.
fn variable_end(b: &[u8], from: usize) -> usize {
    let j = from + 1;
    match b.get(j) {
        Some(b'{') => find(b, j, "}").map_or(b.len(), |e| e + 1),
        Some(b'(') => {
            let mut depth = 0;
            for (k, &c) in b.iter().enumerate().skip(j) {
                match c {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            return k + 1;
                        }
                    }
                    b'\n' => return k,
                    _ => {}
                }
            }
            b.len()
        }
        Some(c) if c.is_ascii_digit() || b"?@#*!$-".contains(c) => j + 1,
        _ => {
            let mut e = word_end(b, j, false);
            if b.get(e) == Some(&b':') && b.get(e + 1).is_some_and(|c| word_start(*c)) {
                e = word_end(b, e + 1, false);
            }
            e
        }
    }
}

fn is_variable(l: &Language, b: &[u8], i: usize) -> bool {
    l.sigil == Some(b[i])
        && b.get(i + 1)
            .is_some_and(|&c| word_start(c) || c == b'{' || c == b'(' || c.is_ascii_digit() || c == b'?')
}

/// A double-quoted string in a shell language: variables inside stand out.
fn interpolated(out: &mut Out, l: &Language, b: &[u8], start: usize, end: usize) {
    let mut from = start;
    let mut j = start + 1;
    while j < end {
        if Some(b[j]) == l.escape {
            j += 2;
            continue;
        }
        if is_variable(l, b, j) {
            let e = variable_end(b, j).min(end);
            out.push(from, j, Kind::String);
            out.push(j, e, Kind::Macro);
            from = e;
            j = e;
            continue;
        }
        j += 1;
    }
    out.push(from, end, Kind::String);
}

fn has_word(list: &[&str], word: &str, ignore_case: bool) -> bool {
    if ignore_case {
        list.iter().any(|k| k.eq_ignore_ascii_case(word))
    } else {
        list.contains(&word)
    }
}

/// The opening `"` of a Rust raw string at `i` (`r"`, `r#"`, `br#"`), with
/// its closing delimiter.
fn rust_raw_string(b: &[u8], word_start_at: usize, i: usize) -> Option<usize> {
    let word = &b[word_start_at..i];
    if word != b"r" && word != b"br" {
        return None;
    }
    let mut j = i;
    while b.get(j) == Some(&b'#') {
        j += 1;
    }
    if b.get(j) != Some(&b'"') {
        return None;
    }
    let mut close = vec![b'"'];
    close.extend(std::iter::repeat_n(b'#', j - i));
    let end = (j + 1..b.len())
        .find(|&k| b[k..].starts_with(&close))
        .map_or(b.len(), |k| k + close.len());
    Some(end)
}

fn generic(l: &Language, text: &str) -> Vec<Span> {
    let b = text.as_bytes();
    let mut out = Out::default();
    let mut i = 0;
    let mut line_start = true;
    let mut after_fn_keyword = false;
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            line_start = true;
            i += 1;
            continue;
        }
        if c == b' ' || c == b'\t' || c == b'\r' {
            i += 1;
            continue;
        }
        let first_on_line = std::mem::replace(&mut line_start, false);
        let after_fn = std::mem::replace(&mut after_fn_keyword, false);

        if let Some((open, close)) = l.block_comment {
            if starts(b, i, open) {
                let end = find(b, i + open.len(), close).map_or(b.len(), |e| e + close.len());
                out.push(i, end, Kind::Comment);
                i = end;
                continue;
            }
        }
        if l.line_comments.iter().any(|p| starts(b, i, p))
            && (c != b'#' || i == 0 || matches!(b[i - 1], b' ' | b'\t' | b'\n' | b'\r' | b';' | b'('))
            && !(l.rust && starts(b, i, "#["))
            && !(l.preprocessor && first_on_line)
        {
            let end = line_end(b, i);
            out.push(i, end, Kind::Comment);
            i = end;
            continue;
        }
        if l.preprocessor && c == b'#' && first_on_line {
            let end = line_end(b, i);
            out.push(i, end, Kind::Macro);
            i = end;
            continue;
        }
        if l.rust && (starts(b, i, "#[") || starts(b, i, "#![")) {
            let eol = line_end(b, i);
            let end = find(b, i, "]").filter(|&e| e < eol).map_or(eol, |e| e + 1);
            out.push(i, end, Kind::Macro);
            i = end;
            continue;
        }
        if l.table_headers && first_on_line && c == b'[' {
            let eol = line_end(b, i);
            let end = (i..eol).rev().find(|&e| b[e] == b']').map_or(eol, |e| e + 1);
            out.push(i, end, Kind::Keyword);
            i = end;
            continue;
        }
        if l.key_sep == Some(b':')
            && !l.keys_anywhere
            && first_on_line
            && c == b'-'
            && b.get(i + 1) == Some(&b' ')
        {
            // A YAML list item: what follows still counts as first on the line.
            line_start = true;
            i += 1;
            continue;
        }
        if l.triple_quotes && (starts(b, i, "\"\"\"") || starts(b, i, "'''")) {
            let q = &text[i..i + 3];
            let end = find(b, i + 3, q).map_or(b.len(), |e| e + 3);
            out.push(i, end, Kind::String);
            i = end;
            continue;
        }
        if l.quotes.as_bytes().contains(&c) {
            if l.rust
                && c == b'\''
                && b.get(i + 1).is_some_and(|c| word_start(*c))
                && b.get(i + 2) != Some(&b'\'')
            {
                // A lifetime: 'a, 'static.
                let end = word_end(b, i + 1, false);
                out.push(i, end, Kind::Keyword);
                i = end;
                continue;
            }
            let end = string_end(l, b, i, c);
            let is_key = l.key_sep.is_some()
                && (first_on_line || l.keys_anywhere)
                && next_non_space(b, end) == l.key_sep;
            if is_key {
                out.push(i, end, Kind::Type);
            } else if c == b'"' && l.sigil.is_some() {
                interpolated(&mut out, l, b, i, end);
            } else {
                out.push(i, end, Kind::String);
            }
            i = end;
            continue;
        }
        if is_variable(l, b, i) {
            let end = variable_end(b, i);
            out.push(i, end, Kind::Macro);
            i = end;
            continue;
        }
        if l.at_decorators && c == b'@' && b.get(i + 1).is_some_and(|c| word_start(*c)) {
            let mut end = word_end(b, i + 1, false);
            while b.get(end) == Some(&b'.') && b.get(end + 1).is_some_and(|c| word_start(*c)) {
                end = word_end(b, end + 1, false);
            }
            out.push(i, end, Kind::Macro);
            i = end;
            continue;
        }
        if c.is_ascii_digit() {
            let end = number_end(b, i);
            let kind = if l.key_sep.is_some() && first_on_line && next_non_space(b, end) == l.key_sep {
                Kind::Type
            } else {
                Kind::Number
            };
            out.push(i, end, kind);
            i = end;
            continue;
        }
        if l.sigil.is_some()
            && l.dash_words
            && c == b'-'
            && b.get(i + 1).is_some_and(u8::is_ascii_alphabetic)
            && matches!(b.get(i.wrapping_sub(1)), None | Some(b' ' | b'\t' | b'(' | b'\n'))
        {
            let end = word_end(b, i + 1, false);
            out.push(i, end, Kind::Param);
            i = end;
            continue;
        }
        if word_start(c) {
            let end = word_end(b, i, l.dash_words);
            if l.rust {
                if let Some(raw_end) = rust_raw_string(b, i, end) {
                    out.push(i, raw_end, Kind::String);
                    i = raw_end;
                    continue;
                }
            }
            let word = &text[i..end];
            let kind = classify(l, word, b, end, after_fn, first_on_line);
            let end = if kind == Kind::Macro && l.rust {
                end + 1
            } else {
                end
            };
            out.push(i, end, kind);
            after_fn_keyword = has_word(l.fn_keywords, word, l.ignore_case);
            i = end;
            continue;
        }
        i += 1;
    }
    out.0
}

fn classify(l: &Language, word: &str, b: &[u8], end: usize, after_fn: bool, first_on_line: bool) -> Kind {
    let capitalized = word.as_bytes()[0].is_ascii_uppercase();
    if after_fn {
        return Kind::Function;
    }
    if l.rust && b.get(end) == Some(&b'!') && b.get(end + 1) != Some(&b'=') {
        return Kind::Macro;
    }
    if l.key_sep.is_some() && (first_on_line || l.keys_anywhere) && next_non_space(b, end) == l.key_sep {
        return Kind::Type;
    }
    if has_word(l.keywords, word, l.ignore_case) {
        return Kind::Keyword;
    }
    if has_word(l.types, word, l.ignore_case) {
        return Kind::Type;
    }
    if b.get(end) == Some(&b'(') {
        return if capitalized && l.capitalized_calls_are_types {
            Kind::Type
        } else {
            Kind::Function
        };
    }
    if capitalized && l.capitalized_types {
        return Kind::Type;
    }
    if l.sigil.is_some() && l.dash_words && word.contains('-') {
        // A PowerShell cmdlet: Get-ChildItem.
        return Kind::Function;
    }
    Kind::Plain
}

/// HTML and XML: tag names, attributes, strings, comments, entities.
fn markup(text: &str) -> Vec<Span> {
    let b = text.as_bytes();
    let mut out = Out::default();
    let mut i = 0;
    while i < b.len() {
        if starts(b, i, "<!--") {
            let end = find(b, i + 4, "-->").map_or(b.len(), |e| e + 3);
            out.push(i, end, Kind::Comment);
            i = end;
        } else if starts(b, i, "<![CDATA[") {
            let end = find(b, i, "]]>").map_or(b.len(), |e| e + 3);
            out.push(i, end, Kind::String);
            i = end;
        } else if b[i] == b'<'
            && b.get(i + 1)
                .is_some_and(|&c| word_start(c) || c == b'/' || c == b'!' || c == b'?')
        {
            let mut j = i + 1;
            if matches!(b[j], b'/' | b'!' | b'?') {
                j += 1;
            }
            let name_end = (j..b.len())
                .find(|&k| !(word_char(b[k], true) || b[k] == b':' || b[k] == b'.'))
                .unwrap_or(b.len());
            out.push(j, name_end, Kind::Keyword);
            j = name_end;
            while j < b.len() && b[j] != b'>' {
                let c = b[j];
                if c == b'"' || c == b'\'' {
                    let end = (j + 1..b.len()).find(|&k| b[k] == c).map_or(b.len(), |k| k + 1);
                    out.push(j, end, Kind::String);
                    j = end;
                } else if word_start(c) {
                    let end = (j..b.len())
                        .find(|&k| !(word_char(b[k], true) || b[k] == b':'))
                        .unwrap_or(b.len());
                    out.push(j, end, Kind::Type);
                    j = end;
                } else {
                    j += 1;
                }
            }
            i = (j + 1).min(b.len());
        } else if b[i] == b'&' {
            let end = (i + 1..b.len().min(i + 12)).find(|&k| b[k] == b';');
            match end {
                Some(e)
                    if b[i + 1..e]
                        .iter()
                        .all(|c| c.is_ascii_alphanumeric() || *c == b'#') =>
                {
                    out.push(i, e + 1, Kind::Number);
                    i = e + 1;
                }
                _ => i += 1,
            }
        } else {
            i += 1;
        }
    }
    out.0
}

/// Markdown: headings, quotes, list markers, code, links.
fn markdown(text: &str) -> Vec<Span> {
    let b = text.as_bytes();
    let mut out = Out::default();
    let mut in_fence = false;
    let mut start = 0;
    while start < b.len() {
        let end = line_end(b, start);
        let line = &text[start..end];
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();
        let fence = trimmed.starts_with("```") || trimmed.starts_with("~~~");
        if in_fence || fence {
            out.push(start, end, Kind::String);
            if fence {
                in_fence = !in_fence;
            }
        } else if trimmed.starts_with('#') {
            out.push(start, end, Kind::Keyword);
        } else if trimmed.starts_with('>') {
            out.push(start, end, Kind::Comment);
        } else {
            let tb = trimmed.as_bytes();
            let digits = tb.iter().take_while(|c| c.is_ascii_digit()).count();
            let marker = if tb.len() >= 2 && matches!(tb[0], b'-' | b'*' | b'+') && tb[1] == b' ' {
                1
            } else if digits > 0 && tb.get(digits) == Some(&b'.') && tb.get(digits + 1) == Some(&b' ') {
                digits + 1
            } else {
                0
            };
            out.push(start + indent, start + indent + marker, Kind::Number);
            let mut j = start + indent + marker;
            while j < end {
                if b[j] == b'`' {
                    let e = (j + 1..end).find(|&k| b[k] == b'`').map_or(end, |k| k + 1);
                    out.push(j, e, Kind::String);
                    j = e;
                } else if b[j] == b'[' {
                    let close = (j + 1..end).find(|&k| b[k] == b']');
                    match close {
                        Some(cl) if b.get(cl + 1) == Some(&b'(') => {
                            let e = (cl + 2..end).find(|&k| b[k] == b')').map_or(end, |k| k + 1);
                            out.push(j, cl + 1, Kind::Function);
                            out.push(cl + 1, e, Kind::String);
                            j = e;
                        }
                        _ => j += 1,
                    }
                } else {
                    j += 1;
                }
            }
        }
        start = end + 1;
    }
    out.0
}

/// CSS: selectors, properties, values, at-rules.
fn css(text: &str) -> Vec<Span> {
    let b = text.as_bytes();
    let mut out = Out::default();
    let mut depth = 0usize;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if starts(b, i, "/*") {
            let end = find(b, i + 2, "*/").map_or(b.len(), |e| e + 2);
            out.push(i, end, Kind::Comment);
            i = end;
        } else if c == b'"' || c == b'\'' {
            let end = string_end(&BASE, b, i, c);
            out.push(i, end, Kind::String);
            i = end;
        } else if c == b'{' {
            depth += 1;
            i += 1;
        } else if c == b'}' {
            depth = depth.saturating_sub(1);
            i += 1;
        } else if c == b'@' {
            let end = word_end(b, i + 1, true);
            out.push(i, end, Kind::Keyword);
            i = end;
        } else if c == b'!' && starts(b, i, "!important") {
            out.push(i, i + 10, Kind::Keyword);
            i += 10;
        } else if depth > 0
            && (c.is_ascii_digit() || (c == b'#' && b.get(i + 1).is_some_and(u8::is_ascii_hexdigit)))
        {
            let end = number_end(b, i + 1).max(i + 1);
            out.push(i, end, Kind::Number);
            i = end;
        } else if depth == 0
            && matches!(c, b'.' | b'#' | b':')
            && b.get(i + 1).is_some_and(|c| word_start(*c))
        {
            let end = word_end(b, i + 1, true);
            out.push(i, end, if c == b':' { Kind::Type } else { Kind::Function });
            i = end;
        } else if word_start(c) || (c == b'-' && b.get(i + 1).is_some_and(|c| word_start(*c) || *c == b'-')) {
            let end = word_end(b, i, true);
            let kind = if depth == 0 {
                Kind::Keyword
            } else if text[i..end].starts_with("--") {
                Kind::Macro
            } else if next_non_space(b, end) == Some(b':') {
                Kind::Type
            } else if b.get(end) == Some(&b'(') {
                Kind::Function
            } else {
                Kind::Plain
            };
            out.push(i, end, kind);
            i = end;
        } else {
            i += 1;
        }
    }
    out.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lang: &str, text: &str) -> Vec<(String, Kind)> {
        highlight(by_id(lang), text)
            .into_iter()
            .map(|s| (text[s.range].to_owned(), s.kind))
            .collect()
    }

    fn has(spans: &[(String, Kind)], text: &str, kind: Kind) -> bool {
        spans.iter().any(|(t, k)| t == text && *k == kind)
    }

    #[test]
    fn rust() {
        let s = kinds(
            "rust",
            "/// Doc\n#[test]\npub fn rotate(src: &Path) -> Result<usize> {\n    let x = Some(5); // c\n    println!(\"a {}\", r#\"raw \\\"#);\n    fs::copy(&a, 'b', 0x1F)?;\n    f::<'static>();\n}",
        );
        assert!(has(&s, "/// Doc", Kind::Comment));
        assert!(has(&s, "#[test]", Kind::Macro));
        assert!(has(&s, "pub", Kind::Keyword));
        assert!(has(&s, "rotate", Kind::Function));
        assert!(has(&s, "Path", Kind::Type));
        assert!(has(&s, "usize", Kind::Type));
        assert!(has(&s, "Some", Kind::Type));
        assert!(has(&s, "5", Kind::Number));
        assert!(has(&s, "// c", Kind::Comment));
        assert!(has(&s, "println!", Kind::Macro));
        assert!(has(&s, "r#\"raw \\\"#", Kind::String));
        assert!(has(&s, "copy", Kind::Function));
        assert!(has(&s, "'b'", Kind::String));
        assert!(has(&s, "0x1F", Kind::Number));
        assert!(has(&s, "'static", Kind::Keyword));
    }

    #[test]
    fn powershell() {
        let s = kinds(
            "powershell",
            "<# block #>\n[string]$Source = \"$env:USERPROFILE\\Docs\"\nGet-ChildItem -Path $Source # c\nWrite-Host \"Copied $($v.Name)\"",
        );
        assert!(has(&s, "<# block #>", Kind::Comment));
        assert!(has(&s, "string", Kind::Type));
        assert!(has(&s, "$Source", Kind::Macro));
        assert!(has(&s, "$env:USERPROFILE", Kind::Macro));
        assert!(has(&s, "Get-ChildItem", Kind::Function));
        assert!(has(&s, "-Path", Kind::Param));
        assert!(has(&s, "# c", Kind::Comment));
        assert!(has(&s, "$($v.Name)", Kind::Macro));
        assert!(has(&s, "\"Copied ", Kind::String));
    }

    #[test]
    fn data_formats() {
        let s = kinds("json", "{\"name\": \"x\", \"n\": 1.5, \"ok\": true}");
        assert!(has(&s, "\"name\"", Kind::Type));
        assert!(has(&s, "\"x\"", Kind::String));
        assert!(has(&s, "1.5", Kind::Number));
        assert!(has(&s, "true", Kind::Keyword));

        let s = kinds("yaml", "# c\nserver:\n  - port: 8080\n    tls: true\n");
        assert!(has(&s, "# c", Kind::Comment));
        assert!(has(&s, "server", Kind::Type));
        assert!(has(&s, "port", Kind::Type));
        assert!(has(&s, "8080", Kind::Number));

        let s = kinds("toml", "[package]\nname = \"heimdall\"\nedition-x = 2021 # y\n");
        assert!(has(&s, "[package]", Kind::Keyword));
        assert!(has(&s, "name", Kind::Type));
        assert!(has(&s, "edition-x", Kind::Type));
        assert!(has(&s, "\"heimdall\"", Kind::String));
        assert!(has(&s, "2021", Kind::Number));
    }

    #[test]
    fn markup_and_markdown() {
        let s = kinds("html", "<!-- c --><a href=\"x\">A &amp; B</a>");
        assert!(has(&s, "<!-- c -->", Kind::Comment));
        assert!(has(&s, "a", Kind::Keyword));
        assert!(has(&s, "href", Kind::Type));
        assert!(has(&s, "\"x\"", Kind::String));
        assert!(has(&s, "&amp;", Kind::Number));

        let s = kinds("markdown", "# Title\n- item `code`\n```\nlet x\n```\n");
        assert!(has(&s, "# Title", Kind::Keyword));
        assert!(has(&s, "-", Kind::Number));
        assert!(has(&s, "`code`", Kind::String));
        assert!(has(&s, "let x", Kind::String));
    }

    #[test]
    fn spans_are_sorted_and_on_char_boundaries() {
        let text = "fn café() { \"ñ\" } // ü\n$x = 'é'";
        for lang in LANGUAGES.iter().map(Some).chain([None]) {
            let spans = highlight(lang, text);
            let mut last = 0;
            for s in &spans {
                assert!(s.range.start >= last && s.range.start < s.range.end);
                assert!(text.is_char_boundary(s.range.start) && text.is_char_boundary(s.range.end));
                last = s.range.end;
            }
        }
    }

    #[test]
    fn detection() {
        let mut n = Note::new(0);
        n.title = "sync.ps1".into();
        assert_eq!(detect(&n).map(|l| l.id), Some("powershell"));
        n.title = "Backup.RS".into();
        assert_eq!(detect(&n).map(|l| l.id), Some("rust"));
        n.title.clear();
        n.body = "#!/usr/bin/env python3\nprint(1)".into();
        assert_eq!(detect(&n).map(|l| l.id), Some("python"));
        n.body = "Shopping list\nmilk".into();
        assert!(detect(&n).is_none());
        n.code = Some(CodeMode {
            language: String::new(),
        });
        assert_eq!(status_label(&n), "Code");
        n.code = Some(CodeMode {
            language: "go".into(),
        });
        assert_eq!(status_label(&n), "Go");
    }

    fn guess(text: &str) -> Option<&'static str> {
        guess_from_text(text).map(|l| l.id)
    }

    #[test]
    fn guesses_from_the_text() {
        assert_eq!(
            guess("import os\n\ndef hola(nombre):\n    print(f\"hola {nombre}\")\n\nhola(\"mundo\")\n"),
            Some("python")
        );
        assert_eq!(guess("def suma(a, b):\n    return a + b\n"), Some("python"));
        assert_eq!(
            guess("#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\\n\");\n    return 0;\n}\n"),
            Some("c")
        );
        assert_eq!(
            guess("#include <iostream>\nint main() {\n    std::cout << \"hi\";\n}\n"),
            Some("cpp")
        );
        assert_eq!(
            guess("use std::fs;\n\nfn main() {\n    let mut n = 0;\n    println!(\"{n}\");\n}\n"),
            Some("rust")
        );
        assert_eq!(
            guess("if [ -f ~/.bashrc ]; then\n    echo \"found\"\nfi\n"),
            Some("bash")
        );
        assert_eq!(
            guess("# Ideas\n\n- one\n- two\n\n```\ncode\n```\n"),
            Some("markdown")
        );
        assert_eq!(guess("{\n  \"name\": \"x\",\n  \"n\": 1\n}\n"), Some("json"));
        assert_eq!(
            guess("const x = 5;\nconsole.log(x);\nconst f = (a) => {\n  return a;\n};\n"),
            Some("javascript")
        );
        assert_eq!(
            guess("interface User {\n  name: string;\n}\nconsole.log(\"x\");\n"),
            Some("typescript")
        );
        assert_eq!(
            guess("SELECT name, age FROM users\nWHERE age > 30;\n"),
            Some("sql")
        );
        assert_eq!(
            guess("package main\n\nimport \"fmt\"\n\nfunc main() {\n    fmt.Println(\"hi\")\n}\n"),
            Some("go")
        );
    }

    #[test]
    fn prose_is_not_code() {
        assert_eq!(guess(""), None);
        assert_eq!(guess("print(1)"), None);
        assert_eq!(
            guess("Lista del súper\n- leche\n- pan\n- huevos\n\nMañana: llamar a mamá a las 5.\n"),
            None
        );
        assert_eq!(
            guess("Gmail\nusuario: juan@example.com\nclave: [secreto]\nnotas: la de siempre\n"),
            None
        );
        assert_eq!(
            guess("Querido diario, hoy fue un buen día.\nFuimos al parque y luego a comer.\nDe ahí regresamos a casa.\n"),
            None
        );
    }

    #[test]
    fn code_offers() {
        let mut n = Note::new(0);
        n.body = "import requests\n\ndef fetch(url):\n    r = requests.get(url)\n    return r.json()\n\nif __name__ == \"__main__\":\n    print(fetch(\"x\"))\n".into();
        let (stem, lang) = code_offer(&n).unwrap();
        assert_eq!((stem.as_str(), lang.extension()), ("fetch", Some("py")));
        n.title = "tets".into();
        assert_eq!(code_offer(&n).unwrap().0, "tets");
        n.title = "tets.txt".into();
        assert!(code_offer(&n).is_none());
        n.title = String::new();
        n.body = "Lista del super\n- leche\n".into();
        assert!(code_offer(&n).is_none());
    }

    #[test]
    fn suggested_titles() {
        let mut n = Note::new(0);
        n.title = "tets".into();
        n.body = "def main():\n    print(\"hi\")\n\nmain()\n".into();
        assert_eq!(suggested_title(&n).as_deref(), Some("tets.py"));
        // A chosen language wins over the guess.
        n.code = Some(CodeMode {
            language: "go".into(),
        });
        assert_eq!(suggested_title(&n).as_deref(), Some("tets.go"));
        n.code = None;
        n.title = "tets.txt".into();
        assert_eq!(suggested_title(&n), None);
        n.title = "main.py".into();
        assert_eq!(suggested_title(&n), None);
        // No title of its own: named after the first line.
        n.title.clear();
        assert_eq!(suggested_title(&n), None);
        n.title = "plain".into();
        n.body = "just some words\nand more words\n".into();
        assert_eq!(suggested_title(&n), None);
    }

    #[test]
    fn kind_lookup() {
        let spans = highlight(by_id("rust"), "let x = 1;");
        assert_eq!(kind_at(&spans, 0), Kind::Keyword);
        assert_eq!(kind_at(&spans, 3), Kind::Plain);
        assert_eq!(kind_at(&spans, 8), Kind::Number);
        assert_eq!(kind_at(&spans, 100), Kind::Plain);
    }
}
