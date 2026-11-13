//! One-time tips (DESIGN §10.11): a small card next to a feature the first
//! time someone reaches it, never again. Which ones were shown lives in
//! `tips.txt` in the config folder, outside any vault, so it is per
//! computer. At most one card shows at a time.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Mutex;

use eframe::egui::{self, pos2, vec2, Align2, CornerRadius, Key, Modifiers, Pos2, Rect, Shape, Stroke};

use crate::theme::*;
use crate::widgets::{self, ButtonKind};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tip {
    /// Welcome, with no vault yet: what a vault is.
    Welcome,
    /// The wizard's profile step.
    Profile,
    /// A recovery code on screen.
    Recovery,
    /// The editor, the first time a vault opens.
    FirstNote,
    /// A selection that looks like a password.
    Selection,
    /// Right after marking a secret.
    Secret,
    /// A tab in code mode.
    CodeMode,
    /// The plain-text export warning.
    Export,
    /// The lock screen after the vault locked by itself.
    Locked,
    /// Linux: the window says the terminal mode exists.
    Terminal,
    /// Linux: the terminal's own one-line hint.
    TerminalHint,
}

impl Tip {
    fn key(self) -> &'static str {
        match self {
            Tip::Welcome => "welcome",
            Tip::Profile => "profile",
            Tip::Recovery => "recovery",
            Tip::FirstNote => "first-note",
            Tip::Selection => "selection",
            Tip::Secret => "secret",
            Tip::CodeMode => "code-mode",
            Tip::Export => "export",
            Tip::Locked => "locked",
            Tip::Terminal => "terminal",
            Tip::TerminalHint => "terminal-hint",
        }
    }

    fn text(self) -> (&'static str, &'static str) {
        match self {
            Tip::Welcome => (
                "Welcome to Heimdall",
                "Your notes live in one encrypted vault file (.heim). Without your password nobody can read it. Start by creating one.",
            ),
            Tip::Profile => (
                "Not sure? Pick Basic",
                "You can move to a stronger profile later in Settings. Hardened and Paranoid need a USB drive as a second key.",
            ),
            Tip::Recovery => (
                "Write these 12 words down",
                "They open the vault if you forget the password or lose the USB key. Keep them on paper, somewhere safe. Anyone who has them can read your notes.",
            ),
            Tip::FirstNote => (
                "Just write",
                "Each tab is a note. Heimdall saves it encrypted while you type. Ctrl+T opens a new tab, Ctrl+Shift+F searches all notes.",
            ),
            Tip::Selection => (
                "Hide it as a secret",
                "Press Ctrl+Shift+H to keep this hidden behind a grey block. Hover it later to reveal or copy it.",
            ),
            Tip::Secret => (
                "Hidden, even on screen",
                "Hover the block to reveal or copy it. A copied secret is cleared from the clipboard after a few seconds.",
            ),
            Tip::CodeMode => (
                "Code mode is on",
                "This note is named like a file, so it gets colors, line numbers and auto-indent. Ctrl+Shift+M turns it off for this tab.",
            ),
            Tip::Export => (
                "This file won't be encrypted",
                "Export writes a plain text copy anyone can open. To keep a safe copy of everything, use Save a backup copy in the menu next to the tabs.",
            ),
            Tip::Locked => (
                "Locked to protect you",
                "Heimdall locks when you step away, lock the computer, or press Ctrl+L. Your notes were saved first. Change the time in Settings \u{2192} Security.",
            ),
            Tip::Terminal => (
                "Heimdall also runs in the terminal",
                "Type heimdall in any terminal to open this vault without a window: list, open, edit and search notes, all encrypted. heimdall --help shows every command.",
            ),
            Tip::TerminalHint => ("", ""),
        }
    }
}

/// Where the card goes, next to its anchor.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Below,
    Above,
    Right,
}

#[derive(Default)]
struct State {
    loaded: bool,
    file: Option<PathBuf>,
    off: bool,
    seen: BTreeSet<String>,
    /// What had been seen when the app started (the terminal tip waits
    /// for a second session).
    seen_at_start: BTreeSet<String>,
    /// The card showing now, and the pass it was last asked for in. A
    /// card nobody asks for any more is gone.
    open: Option<(Tip, u64)>,
}

/// Tips are asked for from many screens; one shared state keeps "one at a
/// time" simple.
static STATE: Mutex<State> = Mutex::new(State {
    loaded: false,
    file: None,
    off: false,
    seen: BTreeSet::new(),
    seen_at_start: BTreeSet::new(),
    open: None,
});

const UPGRADE_SEEN: [Tip; 3] = [Tip::Welcome, Tip::Profile, Tip::Recovery];

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if !s.loaded {
        s.loaded = true;
        let dir = crate::support::config_dir();
        s.file = dir.as_ref().map(|d| d.join("tips.txt"));
        match s.file.as_ref().map(std::fs::read_to_string) {
            Some(Ok(text)) => {
                let (off, seen) = parse(&text);
                s.off = off;
                s.seen = seen;
            }
            _ => {
                // Updated from a version without tips: the vault already
                // exists, so the tips about creating one are not needed.
                let upgraded = dir
                    .is_some_and(|d| d.join("recent.txt").exists() || d.join("terminal-vaults.txt").exists());
                if upgraded {
                    s.seen = UPGRADE_SEEN.iter().map(|t| t.key().to_owned()).collect();
                    save(&s);
                }
            }
        }
        s.seen_at_start = s.seen.clone();
    }
    f(&mut s)
}

fn parse(text: &str) -> (bool, BTreeSet<String>) {
    let mut off = false;
    let mut seen = BTreeSet::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line == "off" {
            off = true;
        } else {
            seen.insert(line.to_owned());
        }
    }
    (off, seen)
}

fn render(off: bool, seen: &BTreeSet<String>) -> String {
    let mut out = String::new();
    if off {
        out.push_str("off\n");
    }
    for k in seen {
        out.push_str(k);
        out.push('\n');
    }
    out
}

fn save(s: &State) {
    let Some(file) = &s.file else { return };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, render(s.off, &s.seen));
}

/// Whether tips are on (Settings → About).
pub fn enabled() -> bool {
    with(|s| !s.off)
}

pub fn set_enabled(on: bool) {
    with(|s| {
        s.off = !on;
        if !on {
            s.open = None;
        }
        save(s);
    });
}

/// Every tip shows once more.
pub fn reset() {
    with(|s| {
        s.off = false;
        s.seen.clear();
        s.seen_at_start.clear();
        s.open = None;
        save(s);
    });
}

/// Whether the tip was shown in an earlier session.
pub fn seen_before(tip: Tip) -> bool {
    with(|s| s.seen_at_start.contains(tip.key()))
}

pub fn seen(tip: Tip) -> bool {
    with(|s| s.seen.contains(tip.key()))
}

/// Closes the card, e.g. because the person did what it explains.
pub fn close(tip: Tip) {
    with(|s| {
        if s.open.is_some_and(|(t, _)| t == tip) {
            s.open = None;
        }
    });
}

/// The terminal's hint, once: `true` the one time it should print.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn take_terminal_hint() -> bool {
    with(|s| {
        let key = Tip::TerminalHint.key();
        if s.off || s.seen.contains(key) {
            return false;
        }
        s.seen.insert(key.to_owned());
        save(s);
        true
    })
}

/// Claims the card for `tip` this pass, if it may show: tips on, this one
/// never shown (or showing now), and no other card open. Marks it seen
/// the first time, so it never comes back.
fn claim(tip: Tip, pass: u64) -> bool {
    with(|s| {
        if s.off {
            return false;
        }
        if let Some((t, last)) = s.open {
            let alive = last + 1 >= pass;
            if t == tip && alive {
                s.open = Some((tip, pass));
                return true;
            }
            if alive {
                return false;
            }
            s.open = None;
        }
        if s.seen.contains(tip.key()) {
            return false;
        }
        s.seen.insert(tip.key().to_owned());
        save(s);
        s.open = Some((tip, pass));
        true
    })
}

const W: f32 = 280.0;
const GAP: f32 = 12.0;

/// Shows the card for `tip` next to `anchor`, if it may show (see
/// [`claim`]). Call it every frame while the situation lasts; once the
/// calls stop the card is gone for good.
pub fn show(ctx: &egui::Context, tip: Tip, anchor: Rect, side: Side) {
    if !anchor.is_finite() || !claim(tip, ctx.cumulative_pass_nr()) {
        return;
    }
    let screen = ctx.screen_rect();
    let side = match side {
        Side::Right if anchor.right() + GAP + W + 12.0 > screen.right() => Side::Below,
        other => other,
    };
    let clamp_x = |x: f32| {
        x.clamp(
            screen.left() + 12.0,
            (screen.right() - 12.0 - W).max(screen.left() + 12.0),
        )
    };
    let (pos, pivot) = match side {
        Side::Below => (
            pos2(clamp_x(anchor.left() - 6.0), anchor.bottom() + GAP),
            Align2::LEFT_TOP,
        ),
        Side::Above => (
            pos2(clamp_x(anchor.left() - 6.0), anchor.top() - GAP),
            Align2::LEFT_BOTTOM,
        ),
        Side::Right => (
            pos2(anchor.right() + GAP + 4.0, anchor.top() - 4.0),
            Align2::LEFT_TOP,
        ),
    };
    let (title, text) = tip.text();
    let mut close = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    let mut turn_off = false;
    let area = egui::Area::new(egui::Id::new(("tip", tip.key())))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .pivot(pivot)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(POPUP)
                .stroke(Stroke::new(1.0_f32, BORDER))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(egui::Margin {
                    left: 14,
                    right: 14,
                    top: 12,
                    bottom: 10,
                })
                .shadow(egui::Shadow {
                    offset: [0, 8],
                    blur: 24,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(110),
                })
                .show(ui, |ui| {
                    ui.set_width(W - 28.0);
                    ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
                    ui.label(egui::RichText::new(title).font(medium(13.0)).color(TEXT));
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(text)
                                .font(crate::theme::ui(12.0))
                                .color(TEXT_SECONDARY),
                        )
                        .wrap(),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if widgets::link(ui, "Turn off tips", 11.0).clicked() {
                            turn_off = true;
                        }
                        widgets::label(ui, "· shown once", 11.0, TEXT_DIM);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::button(ui, ButtonKind::Primary, "Got it", true, vec2(64.0, 26.0))
                                .clicked()
                            {
                                close = true;
                            }
                        });
                    });
                });
        });
    // The arrow, from the card's edge to the anchor.
    let card = area.response.rect;
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new(("tip", tip.key())),
    ));
    let arrow = |tipp: Pos2, a: Pos2, b: Pos2| Shape::convex_polygon(vec![tipp, a, b], POPUP, Stroke::NONE);
    let shape = match side {
        Side::Below if card.top() > anchor.center().y => {
            let x = anchor.center().x.clamp(card.left() + 16.0, card.right() - 16.0);
            Some((
                arrow(
                    pos2(x, card.top() - 6.0),
                    pos2(x - 6.0, card.top() + 0.5),
                    pos2(x + 6.0, card.top() + 0.5),
                ),
                [
                    pos2(x - 6.0, card.top()),
                    pos2(x, card.top() - 6.0),
                    pos2(x + 6.0, card.top()),
                ],
            ))
        }
        Side::Above if card.bottom() < anchor.center().y => {
            let x = anchor.center().x.clamp(card.left() + 16.0, card.right() - 16.0);
            Some((
                arrow(
                    pos2(x, card.bottom() + 6.0),
                    pos2(x - 6.0, card.bottom() - 0.5),
                    pos2(x + 6.0, card.bottom() - 0.5),
                ),
                [
                    pos2(x - 6.0, card.bottom()),
                    pos2(x, card.bottom() + 6.0),
                    pos2(x + 6.0, card.bottom()),
                ],
            ))
        }
        Side::Right if card.left() > anchor.right() => {
            let y = card.top() + 20.0;
            Some((
                arrow(
                    pos2(card.left() - 6.0, y),
                    pos2(card.left() + 0.5, y - 6.0),
                    pos2(card.left() + 0.5, y + 6.0),
                ),
                [
                    pos2(card.left(), y - 6.0),
                    pos2(card.left() - 6.0, y),
                    pos2(card.left(), y + 6.0),
                ],
            ))
        }
        _ => None,
    };
    if let Some((fill, edge)) = shape {
        painter.add(fill);
        painter.add(Shape::line(edge.to_vec(), Stroke::new(1.0_f32, BORDER)));
    }

    if turn_off {
        set_enabled(false);
    } else if close {
        self::close(tip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_round_trips() {
        let seen: BTreeSet<String> = ["welcome", "secret"].iter().map(|s| s.to_string()).collect();
        assert_eq!(parse(&render(true, &seen)), (true, seen.clone()));
        assert_eq!(parse(&render(false, &seen)), (false, seen));
        assert_eq!(parse("\n  \n"), (false, BTreeSet::new()));
    }

    #[test]
    fn keys_are_unique() {
        let all = [
            Tip::Welcome,
            Tip::Profile,
            Tip::Recovery,
            Tip::FirstNote,
            Tip::Selection,
            Tip::Secret,
            Tip::CodeMode,
            Tip::Export,
            Tip::Locked,
            Tip::Terminal,
            Tip::TerminalHint,
        ];
        let keys: BTreeSet<_> = all.iter().map(|t| t.key()).collect();
        assert_eq!(keys.len(), all.len());
    }
}
