//! Spell check for plain notes (DESIGN §10.4): misspelled words get a wavy
//! underline and a right-click menu with suggestions. It never changes the
//! text by itself. English and Spanish Hunspell dictionaries are bundled
//! (`assets/dictionaries`) and loaded on a background thread the first time
//! they are needed; nothing is downloaded.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::{Arc, Once, OnceLock};

use eframe::egui;
use heimdall_core::content::SpellLanguage;
use spellbook::Dictionary;
use zeroize::Zeroize;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Lang {
    En,
    Es,
}

struct Bundled {
    dict: OnceLock<Dictionary>,
    load: Once,
    aff: &'static str,
    dic: &'static str,
}

static EN: Bundled = Bundled {
    dict: OnceLock::new(),
    load: Once::new(),
    aff: include_str!("../assets/dictionaries/en_US.aff"),
    dic: include_str!("../assets/dictionaries/en_US.dic"),
};

static ES: Bundled = Bundled {
    dict: OnceLock::new(),
    load: Once::new(),
    aff: include_str!("../assets/dictionaries/es_MX.aff"),
    dic: include_str!("../assets/dictionaries/es_MX.dic"),
};

fn bundled(lang: Lang) -> &'static Bundled {
    match lang {
        Lang::En => &EN,
        Lang::Es => &ES,
    }
}

fn langs(setting: SpellLanguage) -> &'static [Lang] {
    match setting {
        SpellLanguage::English => &[Lang::En],
        SpellLanguage::Spanish => &[Lang::Es],
        SpellLanguage::EnglishAndSpanish => &[Lang::En, Lang::Es],
    }
}

/// The dictionary, or `None` while it is still loading (the UI is woken
/// when it is ready). Parsing takes a moment, so it never runs on the UI
/// thread.
fn dictionary(lang: Lang, ctx: &egui::Context) -> Option<&'static Dictionary> {
    let b = bundled(lang);
    if let Some(d) = b.dict.get() {
        return Some(d);
    }
    b.load.call_once(|| {
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            if let Ok(d) = Dictionary::new(b.aff, b.dic) {
                let _ = b.dict.set(d);
            }
            ctx.request_repaint();
        });
    });
    None
}

/// All dictionaries for the setting, or `None` until every one is loaded.
fn dictionaries(setting: SpellLanguage, ctx: &egui::Context) -> Option<Vec<&'static Dictionary>> {
    langs(setting).iter().map(|l| dictionary(*l, ctx)).collect()
}

/// A misspelled word: byte range in the note and char range for the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Miss {
    pub bytes: Range<usize>,
    pub chars: (usize, usize),
}

/// Remembers which words were already checked, so typing only checks the
/// new ones, and the misspelled words of the last text seen.
#[derive(Default)]
pub struct Speller {
    setting: Option<SpellLanguage>,
    /// Personal dictionary, lower-cased; rebuilt when it changes.
    personal: HashSet<String>,
    personal_key: u64,
    known: HashMap<String, bool>,
    last: Option<(u64, Arc<Vec<Miss>>)>,
}

impl Drop for Speller {
    fn drop(&mut self) {
        self.wipe();
    }
}

impl Speller {
    /// Forgets every word it has seen (they come from the notes).
    fn wipe(&mut self) {
        for (mut w, _) in self.known.drain() {
            w.zeroize();
        }
        for mut w in self.personal.drain() {
            w.zeroize();
        }
        self.last = None;
    }

    fn sync(&mut self, setting: SpellLanguage, personal: &[String]) {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        personal.hash(&mut h);
        let key = h.finish();
        if self.setting != Some(setting) || self.personal_key != key {
            self.wipe();
            self.setting = Some(setting);
            self.personal_key = key;
            self.personal = personal.iter().map(|w| w.to_lowercase()).collect();
        }
    }

    /// Misspelled words of `text`, in order. Empty until the dictionaries
    /// have loaded.
    pub fn misspelled(
        &mut self,
        ctx: &egui::Context,
        setting: SpellLanguage,
        personal: &[String],
        text: &str,
    ) -> Arc<Vec<Miss>> {
        self.sync(setting, personal);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        let key = h.finish();
        if let Some((k, misses)) = &self.last {
            if *k == key {
                return misses.clone();
            }
        }
        let Some(dicts) = dictionaries(setting, ctx) else {
            return Arc::default();
        };
        // A huge cache only means a huge note was pasted; start over.
        if self.known.len() > 200_000 {
            self.known.clear();
        }
        let mut misses = Vec::new();
        let mut chars = 0;
        let mut at = 0;
        // Masked secrets are never checked (DESIGN §11.1).
        let secrets = heimdall_core::secret::scan(text).secrets;
        for range in words(text) {
            if secrets.iter().any(|s| range.start < s.end && s.start < range.end) {
                continue;
            }
            chars += text[at..range.start].chars().count();
            let word = &text[range.clone()];
            let len = word.chars().count();
            let ok = match self.known.get(word) {
                Some(ok) => *ok,
                None => {
                    let ok =
                        self.personal.contains(&word.to_lowercase()) || dicts.iter().any(|d| d.check(word));
                    self.known.insert(word.to_owned(), ok);
                    ok
                }
            };
            if !ok {
                misses.push(Miss {
                    bytes: range.clone(),
                    chars: (chars, chars + len),
                });
            }
            chars += len;
            at = range.end;
        }
        let misses = Arc::new(misses);
        self.last = Some((key, misses.clone()));
        misses
    }

    /// Up to six replacements for `word`, best first.
    pub fn suggest(&self, ctx: &egui::Context, setting: SpellLanguage, word: &str) -> Vec<String> {
        let Some(dicts) = dictionaries(setting, ctx) else {
            return Vec::new();
        };
        let mut lists: Vec<Vec<String>> = dicts
            .iter()
            .map(|d| {
                let mut out = Vec::new();
                d.suggest(word, &mut out);
                out
            })
            .collect();
        // Interleave the languages so each one's best guesses come first.
        let mut out: Vec<String> = Vec::new();
        for i in 0..lists.iter().map(Vec::len).max().unwrap_or(0) {
            for list in &mut lists {
                if let Some(s) = list.get_mut(i) {
                    let s = std::mem::take(s);
                    if !out.contains(&s) {
                        out.push(s);
                    }
                }
            }
        }
        out.truncate(6);
        out
    }
}

/// Byte ranges of the words to check. Skips anything with a digit, words
/// in capitals (acronyms), and chunks that look like links or addresses.
pub fn words(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut chunk_start = None;
    for (i, c) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
        match (c.is_whitespace(), chunk_start) {
            (false, None) => chunk_start = Some(i),
            (true, Some(start)) => {
                chunk_words(text, start, i, &mut out);
                chunk_start = None;
            }
            _ => {}
        }
    }
    out
}

fn chunk_words(text: &str, start: usize, end: usize, out: &mut Vec<Range<usize>>) {
    let chunk = &text[start..end];
    if chunk.contains("://") || chunk.contains('@') || chunk.starts_with("www.") {
        return;
    }
    let chars: Vec<(usize, char)> = chunk.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].1.is_alphanumeric() {
            i += 1;
            continue;
        }
        let first = i;
        // Letters and digits, plus an apostrophe between letters (don't).
        while i < chars.len() {
            let c = chars[i].1;
            let inner_apostrophe = matches!(c, '\'' | '’')
                && i > first
                && chars.get(i + 1).is_some_and(|(_, n)| n.is_alphabetic());
            if c.is_alphanumeric() || inner_apostrophe {
                i += 1;
            } else {
                break;
            }
        }
        let a = start + chars[first].0;
        let b = start + chars.get(i).map_or(chunk.len(), |(o, _)| *o);
        let word = &text[a..b];
        let has_digit = word.chars().any(|c| c.is_numeric());
        let letters = word.chars().filter(|c| c.is_alphabetic()).count();
        let all_caps = letters > 1 && word.chars().all(|c| !c.is_lowercase());
        if !has_digit && !all_caps {
            out.push(a..b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(text: &str) -> Vec<&str> {
        words(text).into_iter().map(|r| &text[r]).collect()
    }

    #[test]
    fn splits_words_and_skips_noise() {
        assert_eq!(
            split("Don't  check USB, abc123 or https://x.org/path — but café, well-known."),
            ["Don't", "check", "or", "but", "café", "well", "known"]
        );
        assert_eq!(split("me@example.com www.site.com ok"), ["ok"]);
        assert_eq!(split("l’eau 'quoted'"), ["l’eau", "quoted"]);
        assert!(split("").is_empty());
    }

    /// The bundled dictionaries parse and know their languages.
    #[test]
    fn bundled_dictionaries_work() {
        let en = Dictionary::new(EN.aff, EN.dic).unwrap();
        let es = Dictionary::new(ES.aff, ES.dic).unwrap();
        for w in ["hello", "Hello", "don't", "notebook", "encrypted"] {
            assert!(en.check(w), "{w}");
        }
        for w in ["hola", "contraseña", "café", "también", "Cifrado"] {
            assert!(es.check(w), "{w}");
        }
        assert!(!en.check("helo"));
        assert!(!es.check("contrasena"));
        let mut out = Vec::new();
        en.suggest("helo", &mut out);
        assert!(out.iter().any(|s| s == "hello"), "{out:?}");
    }

    #[test]
    fn misspelled_words_with_personal_dictionary() {
        let ctx = egui::Context::default();
        let mut sp = Speller::default();
        // Load synchronously for the test.
        let _ = EN.dict.set(Dictionary::new(EN.aff, EN.dic).unwrap());
        let text = "Helo Heimdall, the vault is encryptd.";
        let m = sp.misspelled(&ctx, SpellLanguage::English, &[], text);
        let bad: Vec<&str> = m.iter().map(|m| &text[m.bytes.clone()]).collect();
        assert_eq!(bad, ["Helo", "Heimdall", "encryptd"]);
        assert_eq!(m[1].chars, (5, 13));

        let m = sp.misspelled(&ctx, SpellLanguage::English, &["heimdall".to_owned()], text);
        assert_eq!(m.len(), 2);
        assert!(sp
            .suggest(&ctx, SpellLanguage::English, "encryptd")
            .contains(&"encrypted".to_owned()));
    }
}
