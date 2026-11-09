//! Masked secrets in the editor (DESIGN §11): how they are laid out, the
//! Reveal and Copy icons drawn after them, keeping the caret out of them,
//! and the paste detector. What counts as a secret is decided in
//! `heimdall_core::secret`.
//!
//! The text edit's galley must keep every character of the note, so a
//! masked secret isn't removed from the layout: its characters are drawn
//! transparent, the value in a tiny font, and the `{*` gets the letter
//! spacing that makes every masked secret exactly as wide as the block
//! plus the icons. The block and icons are painted over that room; a
//! revealed secret gets the room for the icons from its `*}`.

use std::ops::Range;

use eframe::egui::{
    self, pos2, text::CCursor, text::LayoutJob, text::LayoutSection, vec2, Color32, CornerRadius, CursorIcon,
    FontId, Galley, Pos2, Rect, Sense, Shape, TextFormat, Ui,
};
use heimdall_core::secret;

use crate::theme::*;
use crate::widgets;

/// Each icon's box, both icons, and the gap between the block and them.
const ICON: f32 = 18.0;
const ICONS_W: f32 = ICON * 2.0;
const GAP: f32 = 6.0;
/// Font size of a masked value: next to nothing, so its length barely
/// changes the width (what is left is made up for, see [`restyle`]).
const HIDDEN_SIZE: f32 = 0.2;
/// Width of a masked secret's block, in font sizes: the same for every
/// secret, so it doesn't give the length away.
const MASK_EMS: f32 = 3.6;
/// How long a click on Reveal shows a secret (DESIGN §11.1).
pub const REVEAL_SECONDS: f64 = 5.0;

const BLOCK: Color32 = Color32::from_rgb(0x2E, 0x2E, 0x2E);

/// A secret of the note on screen, as byte and char ranges (with its
/// delimiters), and whether its value is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub bytes: Range<usize>,
    pub chars: (usize, usize),
    pub revealed: bool,
}

/// The secrets of `text`; `revealed(i)` says whether the i-th one shows.
pub fn items(text: &str, revealed: &impl Fn(usize) -> bool) -> Vec<Item> {
    let scan = secret::scan(text);
    let mut out = Vec::with_capacity(scan.secrets.len());
    let (mut chars, mut at) = (0, 0);
    for (i, r) in scan.secrets.into_iter().enumerate() {
        chars += text[at..r.start].chars().count();
        let start = chars;
        chars += text[r.clone()].chars().count();
        at = r.end;
        out.push(Item {
            bytes: r,
            chars: (start, chars),
            revealed: revealed(i),
        });
    }
    out
}

// ---------- Layout ----------

/// Restyles a laid-out note so its secrets take their masked (or revealed)
/// shape. `revealed(i)` as in [`items`].
pub fn restyle(
    job: &mut LayoutJob,
    fonts: &egui::epaint::Fonts,
    font: &FontId,
    revealed: &impl Fn(usize) -> bool,
) {
    if !job.text.contains(secret::OPEN) {
        return;
    }
    let scan = secret::scan(&job.text);
    let tiny = FontId::new(HIDDEN_SIZE, font.family.clone());
    // Measured by laying the text out, as the note itself will be: adding
    // up glyph widths drifts from that by a fraction of a pixel per char,
    // and the width would give away the length.
    let width = |f: &FontId, s: &str| {
        fonts
            .layout_no_wrap(s.to_owned(), f.clone(), Color32::TRANSPARENT)
            .size()
            .x
    };
    let open_w = width(font, secret::OPEN);
    let close_w = width(font, secret::CLOSE);
    let mask_w = font.size * MASK_EMS;

    // (range, letter spacing of a visible-width section or None for 1 px)
    let mut edits: Vec<(Range<usize>, Option<f32>)> = Vec::new();
    for (i, r) in scan.secrets.iter().enumerate() {
        let open = r.start..r.start + secret::OPEN.len();
        if revealed(i) {
            edits.push((open, None));
            let close = r.end - secret::CLOSE.len()..r.end;
            edits.push((close, Some((GAP + ICONS_W - close_w).max(0.0))));
        } else {
            let rest = width(&tiny, &job.text[open.end..r.end]);
            edits.push((
                open.clone(),
                Some((mask_w + GAP + ICONS_W - open_w - rest).max(0.0)),
            ));
            edits.push((open.end..r.end, None));
        }
    }
    for &e in &scan.escapes {
        edits.push((e..e + 1, None));
    }
    edits.sort_by_key(|(r, _)| r.start);

    let edits: Vec<(Range<usize>, TextFormat)> = edits
        .into_iter()
        .map(|(range, spacing)| {
            let base = format_at(&job.sections, range.start);
            let format = TextFormat {
                color: Color32::TRANSPARENT,
                extra_letter_spacing: spacing.unwrap_or(0.0),
                font_id: if spacing.is_some() {
                    base.font_id.clone()
                } else {
                    tiny.clone()
                },
                underline: egui::Stroke::NONE,
                background: Color32::TRANSPARENT,
                ..base
            };
            (range, format)
        })
        .collect();
    job.sections = replace_sections(std::mem::take(&mut job.sections), &edits);
}

fn format_at(sections: &[LayoutSection], byte: usize) -> TextFormat {
    let i = sections.partition_point(|s| s.byte_range.end <= byte);
    sections
        .get(i)
        .or(sections.last())
        .map(|s| s.format.clone())
        .unwrap_or_default()
}

/// Replaces each (sorted, disjoint) range with one section of its own
/// format: letter spacing only applies inside a section.
fn replace_sections(
    sections: Vec<LayoutSection>,
    edits: &[(Range<usize>, TextFormat)],
) -> Vec<LayoutSection> {
    let mut out = Vec::with_capacity(sections.len() + edits.len() * 2);
    let mut e = 0;
    for sec in sections {
        let end = sec.byte_range.end;
        let mut pos = sec.byte_range.start;
        let mut leading = sec.leading_space;
        while pos < end {
            while edits.get(e).is_some_and(|(r, _)| r.end <= pos) {
                e += 1;
            }
            match edits.get(e) {
                Some((r, format)) if r.start <= pos => {
                    if pos == r.start {
                        out.push(LayoutSection {
                            leading_space: leading,
                            byte_range: r.clone(),
                            format: format.clone(),
                        });
                    }
                    pos = r.end.min(end);
                }
                Some((r, _)) if r.start < end => {
                    out.push(LayoutSection {
                        leading_space: leading,
                        byte_range: pos..r.start,
                        format: sec.format.clone(),
                    });
                    pos = r.start;
                }
                _ => {
                    out.push(LayoutSection {
                        leading_space: leading,
                        byte_range: pos..end,
                        format: sec.format.clone(),
                    });
                    pos = end;
                }
            }
            leading = 0.0;
        }
    }
    out
}

// ---------- Drawing ----------

pub enum IconEvent {
    /// Reveal was clicked (or, holding to reveal, is held down).
    Reveal,
    Copy,
    /// Right-clicked on an icon, at this point: the secret's menu.
    Menu(Pos2),
}

/// Where the secrets are on screen, to paint and click them.
pub struct Screen<'a> {
    pub galley: &'a Galley,
    pub origin: Pos2,
    pub viewport: Rect,
    pub glyph_h: f32,
    pub font: &'a FontId,
}

impl Screen<'_> {
    /// The whole room a secret takes on screen (block and icons), the
    /// block behind its value, and where the icons go.
    fn rects(&self, item: &Item) -> (Rect, Rect, Rect) {
        let start = self.galley.pos_from_ccursor(CCursor {
            index: item.chars.0,
            prefer_next_row: true,
        });
        let end = self.galley.pos_from_ccursor(CCursor::new(item.chars.1));
        let row = |left: f32, right: f32, top: f32| {
            Rect::from_min_max(pos2(left, top), pos2(right, top + self.glyph_h))
                .translate(self.origin.to_vec2())
        };
        let mask_w = self.font.size * MASK_EMS;
        let one_row = (end.top() - start.top()).abs() < 1.0;
        if one_row {
            let icons_left = end.left() - ICONS_W;
            let all = row(start.left(), end.left(), start.top());
            let block = row(start.left(), (icons_left - GAP).max(start.left()), start.top());
            return (all, block, row(icons_left, end.left(), start.top()));
        }
        if item.revealed && end.left() >= ICONS_W {
            // Wrapped inside the value: the icons follow it on its last row.
            let icons = row(end.left() - ICONS_W, end.left(), end.top());
            let block = row(start.left(), start.left(), start.top());
            return (block.union(icons), block, icons);
        }
        // Wrapped inside the room: block and icons from the start.
        let (x, top) = (start.left(), start.top());
        let block = row(x, x + mask_w, top);
        let icons = row(x + mask_w + GAP, x + mask_w + GAP + ICONS_W, top);
        (block.union(icons), block, icons)
    }

    /// The block behind each secret on screen (drawn under the text).
    pub fn backgrounds(&self, items: &[Item]) -> Vec<Shape> {
        items
            .iter()
            .map(|it| self.rects(it).1)
            .filter(|r| r.width() > 0.0 && self.viewport.intersects(*r))
            .map(|r| Shape::rect_filled(r.expand2(vec2(2.0, 2.0)), CornerRadius::same(4), BLOCK))
            .collect()
    }

    /// The icons after each secret, only while the pointer is over it (or
    /// its value is showing in a bubble, `popped`); returns what was
    /// clicked, by index. With `hold`, Reveal reports while held on it.
    pub fn icons(
        &self,
        ui: &Ui,
        id: egui::Id,
        items: &[Item],
        popped: &[usize],
        hold: bool,
    ) -> Vec<(usize, IconEvent)> {
        let painter = ui.painter_at(self.viewport);
        let pointer = ui
            .ctx()
            .pointer_hover_pos()
            .filter(|p| self.viewport.contains(*p));
        let mut events = Vec::new();
        for (i, it) in items.iter().enumerate() {
            let (all, _, icons) = self.rects(it);
            let popped = popped.contains(&i);
            let over = pointer.is_some_and(|p| all.expand(2.0).contains(p));
            if !self.viewport.intersects(icons) || !(over || popped) {
                continue;
            }
            let eye = Rect::from_min_size(icons.min, vec2(ICON, self.glyph_h));
            let copy = eye.translate(vec2(ICON, 0.0));
            let eye_resp = ui.interact(
                eye.intersect(self.viewport),
                id.with(("secret_eye", i)),
                Sense::click(),
            );
            let copy_resp = ui.interact(
                copy.intersect(self.viewport),
                id.with(("secret_copy", i)),
                Sense::click(),
            );

            let color = |hovered: bool, on: bool| if hovered || on { TEXT } else { ICON_MUTED };
            widgets::icon_eye(
                &painter,
                eye.center(),
                color(eye_resp.hovered(), it.revealed || popped),
            );
            widgets::icon_copy(&painter, copy.center(), color(copy_resp.hovered(), false));
            for r in [&eye_resp, &copy_resp] {
                if r.hovered() {
                    ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                }
                if r.secondary_clicked() {
                    if let Some(pos) = r.interact_pointer_pos() {
                        events.push((i, IconEvent::Menu(pos)));
                    }
                }
            }
            let reveal_tip = if hold {
                "Hold to reveal"
            } else if it.revealed || popped {
                "Hide"
            } else {
                "Reveal for 5 seconds"
            };
            let eye_down = eye_resp.is_pointer_button_down_on();
            if (hold && eye_down) || (!hold && eye_resp.clicked()) {
                events.push((i, IconEvent::Reveal));
            }
            eye_resp.on_hover_text(reveal_tip);
            if copy_resp.clicked() {
                events.push((i, IconEvent::Copy));
            }
            copy_resp.on_hover_text("Copy");
        }
        events
    }

    /// The value of each secret in `popped`, in a bubble above it (below
    /// it at the top of the view). The text doesn't move, so neither do the
    /// icons under the pointer.
    pub fn bubbles(&self, ui: &Ui, text: &str, items: &[Item], popped: &[usize]) {
        let painter = ui
            .ctx()
            .layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("secret_bubbles"),
            ))
            .with_clip_rect(self.viewport);
        let pad = vec2(8.0, 4.0);
        for &i in popped {
            let Some(it) = items.get(i) else {
                continue;
            };
            let (_, block, _) = self.rects(it);
            if !self.viewport.intersects(block) {
                continue;
            }
            let value = text[secret::value(&it.bytes)].to_owned();
            let wrap = (self.viewport.width() - 2.0 * pad.x - 8.0).max(40.0);
            let galley = painter.layout(value, self.font.clone(), TEXT, wrap);
            let size = galley.size() + 2.0 * pad;
            let mut r = Rect::from_min_size(pos2(block.left() - 2.0, block.top() - 6.0 - size.y), size);
            if r.top() < self.viewport.top() {
                r = r.translate(vec2(0.0, size.y + block.height() + 12.0));
            }
            let shift = (r.right() - self.viewport.right() + 4.0).max(0.0);
            let r = r.translate(vec2(-shift.min(r.left() - self.viewport.left()).max(0.0), 0.0));
            painter.rect(
                r,
                CornerRadius::same(6),
                POPUP,
                egui::Stroke::new(1.0_f32, POPUP_BORDER),
                egui::StrokeKind::Inside,
            );
            painter.galley(r.min + pad, galley, TEXT);
        }
    }

    /// The secret under a point, if any.
    pub fn at(&self, items: &[Item], pos: Pos2) -> Option<usize> {
        items
            .iter()
            .position(|it| self.rects(it).0.expand(2.0).contains(pos))
    }
}

const ICON_MUTED: Color32 = Color32::from_rgb(0x82, 0x82, 0x82);

// ---------- Caret ----------

/// Keeps the caret out of masked secrets and off a revealed secret's
/// delimiters (DESIGN §11.1): a secret behaves as one block. A collapsed
/// caret moves on in the direction it came from; a selection grows to
/// cover the secrets it touches. Returns the fixed (primary, secondary).
pub fn fix_cursor(
    items: &[Item],
    primary: usize,
    secondary: usize,
    previous: usize,
) -> Option<(usize, usize)> {
    let fix = |c: usize, forward: bool| -> usize {
        for it in items {
            let (s, e) = it.chars;
            if it.revealed {
                if c == s + 1 {
                    return if forward { s + 2 } else { s };
                }
                if c + 1 == e {
                    return if forward { e } else { e - 2 };
                }
            } else if s < c && c < e {
                return if forward { e } else { s };
            }
        }
        c
    };
    let fixed = if primary == secondary {
        let c = fix(primary, primary > previous);
        (c, c)
    } else if primary < secondary {
        (fix(primary, false), fix(secondary, true))
    } else {
        (fix(primary, true), fix(secondary, false))
    };
    (fixed != (primary, secondary)).then_some(fixed)
}

/// What Backspace (`back`) or Delete does at a collapsed caret next to a
/// secret: `None` = the usual; `Some(None)` = nothing (it would break a
/// revealed secret's delimiter); `Some(Some(chars))` = delete the whole
/// secret.
pub fn deletion(items: &[Item], caret: usize, back: bool) -> Option<Option<(usize, usize)>> {
    for it in items {
        let (s, e) = it.chars;
        match (back, it.revealed) {
            (true, _) if caret == e => return Some(Some(it.chars)),
            (false, _) if caret == s => return Some(Some(it.chars)),
            (true, true) if caret == s + 2 => return Some(None),
            (false, true) if caret + 2 == e => return Some(None),
            _ => {}
        }
    }
    None
}

// ---------- Paste ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pasted {
    /// It looked like a secret and was wrapped in `{* *}`.
    Masked,
    /// It already held this many secrets.
    Secrets(usize),
}

/// Looks at text about to be pasted (DESIGN §11.3) and wraps it if it
/// looks like a secret. `strict` in code mode.
pub fn on_paste(text: &mut String, detect: bool, strict: bool) -> Option<Pasted> {
    let n = secret::scan(text).secrets.len();
    if n > 0 {
        return Some(Pasted::Secrets(n));
    }
    if !detect || !secret::looks_like_secret(text, strict) {
        return None;
    }
    let wrapped = secret::wrap(text.trim())?;
    zeroize::Zeroize::zeroize(text);
    *text = wrapped;
    Some(Pasted::Masked)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masked(text: &str) -> Vec<Item> {
        items(text, &|_| false)
    }

    #[test]
    fn items_have_char_ranges() {
        let it = masked("ñ {*pw*} é {*x*}");
        assert_eq!(it.len(), 2);
        assert_eq!(it[0].chars, (2, 8));
        assert_eq!(it[1].chars, (11, 16));
        assert_eq!(it[0].bytes, 3..9);
    }

    #[test]
    fn caret_skips_masked_secrets() {
        let it = masked("ab{*pw*}cd"); // secret at chars 2..8
                                       // Moving right into it: to its end; moving left: to its start.
        assert_eq!(fix_cursor(&it, 3, 3, 2), Some((8, 8)));
        assert_eq!(fix_cursor(&it, 7, 7, 8), Some((2, 2)));
        assert_eq!(fix_cursor(&it, 2, 2, 1), None);
        assert_eq!(fix_cursor(&it, 8, 8, 9), None);
        // A selection grows to cover it.
        assert_eq!(fix_cursor(&it, 0, 5, 0), Some((0, 8)));
        assert_eq!(fix_cursor(&it, 9, 4, 9), Some((9, 2)));
    }

    #[test]
    fn caret_stays_off_revealed_delimiters() {
        let it = items("ab{*pw*}cd", &|_| true);
        assert_eq!(fix_cursor(&it, 3, 3, 2), Some((4, 4)));
        assert_eq!(fix_cursor(&it, 3, 3, 4), Some((2, 2)));
        assert_eq!(fix_cursor(&it, 5, 5, 4), None); // inside the value
        assert_eq!(fix_cursor(&it, 7, 7, 6), Some((8, 8)));
    }

    #[test]
    fn backspace_and_delete_take_the_whole_secret() {
        let it = masked("ab{*pw*}cd");
        assert_eq!(deletion(&it, 8, true), Some(Some((2, 8))));
        assert_eq!(deletion(&it, 2, false), Some(Some((2, 8))));
        assert_eq!(deletion(&it, 9, true), None);
        let it = items("ab{*pw*}cd", &|_| true);
        assert_eq!(deletion(&it, 4, true), Some(None));
        assert_eq!(deletion(&it, 6, false), Some(None));
        assert_eq!(deletion(&it, 5, true), None);
    }

    #[test]
    fn paste_masks_what_looks_secret() {
        let mut t = " ghp_aB3dE5fG7hI9jK1lM3nO5pQ7rS9tU1vW3xY5\r\n".to_owned();
        assert_eq!(on_paste(&mut t, true, false), Some(Pasted::Masked));
        assert_eq!(t, "{*ghp_aB3dE5fG7hI9jK1lM3nO5pQ7rS9tU1vW3xY5*}");
        let mut t = "x7Gk2pQ9vLm4Tz8R".to_owned();
        assert_eq!(on_paste(&mut t, false, false), None);
        let mut t = "a {*b*} {*c*}".to_owned();
        assert_eq!(on_paste(&mut t, true, false), Some(Pasted::Secrets(2)));
        let mut t = "hello world".to_owned();
        assert_eq!(on_paste(&mut t, true, false), None);
    }

    #[test]
    fn sections_are_replaced_whole() {
        let job = LayoutJob::simple("ab{*pw*}cd".into(), FontId::monospace(14.0), TEXT, 100.0);
        let f = TextFormat {
            extra_letter_spacing: 3.0,
            ..Default::default()
        };
        let out = replace_sections(job.sections, &[(2..4, f), (4..8, TextFormat::default())]);
        let ranges: Vec<_> = out.iter().map(|s| s.byte_range.clone()).collect();
        assert_eq!(ranges, [0..2, 2..4, 4..8, 8..10]);
        assert_eq!(out[1].format.extra_letter_spacing, 3.0);
    }
}
