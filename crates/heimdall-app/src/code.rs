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
        None
    }
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

    #[test]
    fn kind_lookup() {
        let spans = highlight(by_id("rust"), "let x = 1;");
        assert_eq!(kind_at(&spans, 0), Kind::Keyword);
        assert_eq!(kind_at(&spans, 3), Kind::Plain);
        assert_eq!(kind_at(&spans, 8), Kind::Number);
        assert_eq!(kind_at(&spans, 100), Kind::Plain);
    }
}
