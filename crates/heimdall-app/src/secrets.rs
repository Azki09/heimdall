//! Masked secrets in the editor (DESIGN §11): how they are laid out, the
//! Reveal and Copy icons that float above them, keeping the caret out of
//! them, and the paste detector. What counts as a secret is decided in
//! `heimdall_core::secret`.
//!
//! The text edit's galley must keep every character of the note, so a
//! masked secret isn't removed from the layout: its characters are drawn
//! transparent, the value in a tiny font, and the `{*` gets the letter
//! spacing that makes every masked secret exactly as wide as the block.
//! The block is painted under that room. A revealed secret shows its value
//! inside the block: its delimiters shrink to a small padding on each side.
//! The icons take no room in the text: they float above the block.

use std::ops::Range;

use eframe::egui::{
    self, pos2, text::CCursor, text::LayoutJob, text::LayoutSection, vec2, Color32, CornerRadius, CursorIcon,
    FontId, Galley, Pos2, Rect, Sense, Shape, TextFormat, Ui,
};
use heimdall_core::secret;

use crate::theme::*;
use crate::widgets;

/// Each icon's box, and both icons.
const ICON: f32 = 18.0;
const ICONS_W: f32 = ICON * 2.0;
/// Room around the icons inside their floating pill, and between the pill
/// and the block.
const PILL_PAD: f32 = 3.0;
const PILL_GAP: f32 = 4.0;
/// Room on each side of a revealed value, inside its block.
const VALUE_PAD: f32 = 4.0;
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
    let mask_w = font.size * MASK_EMS;

    // (range, in the note's font (else the tiny one), letter spacing)
    let mut edits: Vec<(Range<usize>, bool, f32)> = Vec::new();
    for (i, r) in scan.secrets.iter().enumerate() {
        let open = r.start..r.start + secret::OPEN.len();
        if revealed(i) {
            // Both delimiters shrink to the padding inside the block.
            let close = r.end - secret::CLOSE.len()..r.end;
            edits.push((open, false, VALUE_PAD));
            edits.push((close, false, VALUE_PAD));
        } else {
            let rest = width(&tiny, &job.text[open.end..r.end]);
            edits.push((open.clone(), true, (mask_w - open_w - rest).max(0.0)));
            edits.push((open.end..r.end, false, 0.0));
        }
    }
    for &e in &scan.escapes {
        edits.push((e..e + 1, false, 0.0));
    }
    edits.sort_by_key(|(r, _, _)| r.start);

    let edits: Vec<(Range<usize>, TextFormat)> = edits
        .into_iter()
        .map(|(range, full_size, spacing)| {
            let base = format_at(&job.sections, range.start);
            let format = TextFormat {
                color: Color32::TRANSPARENT,
                extra_letter_spacing: spacing,
                font_id: if full_size {
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
    /// Reveal was clicked (or, holding to reveal, is held down), or a
    /// revealed value was clicked to hide it.
    Reveal,
    /// Copy was clicked; the point is above the icon, for the "Copied" note.
    Copy(Pos2),
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

/// Where one secret is on screen.
struct Slot {
    /// The block, one rect per row it spans (a revealed value can wrap).
    blocks: Vec<Rect>,
    /// The pill with the icons, above the block's first row (below it at
    /// the top of the view). It doesn't move when the value is revealed,
    /// so the eye stays under the pointer.
    pill: Rect,
}

impl Screen<'_> {
    fn slot(&self, item: &Item) -> Slot {
        let start_c = self.galley.from_ccursor(CCursor {
            index: item.chars.0,
            prefer_next_row: true,
        });
        let end_c = self.galley.from_ccursor(CCursor::new(item.chars.1));
        let start = self.galley.pos_from_cursor(&start_c);
        let end = self.galley.pos_from_cursor(&end_c);
        let at = |left: f32, right: f32, top: f32| {
            Rect::from_min_max(pos2(left, top), pos2(right, top + self.glyph_h))
                .translate(self.origin.to_vec2())
        };
        let (first_row, last_row) = (start_c.rcursor.row, end_c.rcursor.row);
        let mut blocks = Vec::new();
        if first_row == last_row {
            blocks.push(at(start.left(), end.left(), start.top()));
        } else if item.revealed {
            for r in first_row..=last_row {
                let Some(row) = self.galley.rows.get(r) else {
                    break;
                };
                let left = if r == first_row {
                    start.left()
                } else {
                    row.rect.left()
                };
                let right = if r == last_row {
                    end.left()
                } else {
                    row.rect.right()
                };
                if right - left >= 1.0 {
                    blocks.push(at(left, right, row.rect.top()));
                }
            }
        }
        if blocks.is_empty() {
            // A masked secret wrapped inside its room: draw it whole where it starts.
            let x = start.left();
            blocks.push(at(x, x + self.font.size * MASK_EMS, start.top()));
        }

        let first = blocks[0];
        let size = vec2(ICONS_W + 2.0 * PILL_PAD, self.glyph_h + 2.0 * PILL_PAD);
        let mut pill = Rect::from_min_size(
            pos2(first.left() - 2.0, first.top() - 2.0 - PILL_GAP - size.y),
            size,
        );
        if pill.top() < self.viewport.top() {
            pill = pill.translate(vec2(0.0, size.y + first.height() + 2.0 * (PILL_GAP + 2.0)));
        }
        let shift = (pill.right() - self.viewport.right() + 2.0).max(0.0);
        pill = pill.translate(vec2(-shift, 0.0));
        Slot { blocks, pill }
    }

    /// The block behind each secret on screen (drawn under the text).
    pub fn backgrounds(&self, items: &[Item]) -> Vec<Shape> {
        items
            .iter()
            .flat_map(|it| self.slot(it).blocks)
            .filter(|r| r.width() > 0.0 && self.viewport.intersects(*r))
            .map(|r| Shape::rect_filled(r.expand2(vec2(2.0, 2.0)), CornerRadius::same(4), BLOCK))
            .collect()
    }

    /// The pill with Reveal and Copy, for one secret at a time: the one
    /// the pointer is on (DESIGN §11.1). A click on a value in `shown` (one
    /// secret's Reveal) hides it. Returns what was clicked, by index. With
    /// `hold`, Reveal reports while held on it.
    pub fn icons(
        &self,
        ui: &Ui,
        id: egui::Id,
        items: &[Item],
        shown: &[usize],
        hold: bool,
    ) -> Vec<(usize, IconEvent)> {
        let pointer = ui
            .ctx()
            .pointer_hover_pos()
            .filter(|p| self.viewport.contains(*p));
        let slots: Vec<Option<Slot>> = items
            .iter()
            .map(|it| Some(self.slot(it)).filter(|s| self.viewport.intersects(s.blocks[0])))
            .collect();
        let mut events = Vec::new();

        // A revealed value hides with a click on it (not when holding).
        // Registered before the pill, so the pill wins where they overlap.
        if !hold {
            for &i in shown {
                let Some(Some(slot)) = slots.get(i) else {
                    continue;
                };
                for (k, b) in slot.blocks.iter().enumerate() {
                    let resp = ui.interact(
                        b.expand(2.0).intersect(self.viewport),
                        id.with(("secret_value", i, k)),
                        Sense::click(),
                    );
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                    }
                    if resp.clicked() {
                        events.push((i, IconEvent::Reveal));
                    }
                    if resp.secondary_clicked() {
                        if let Some(pos) = resp.interact_pointer_pos() {
                            events.push((i, IconEvent::Menu(pos)));
                        }
                    }
                }
            }
        }

        let active_id = id.with("secret_active");
        let previous: Option<usize> = ui.ctx().data(|d| d.get_temp(active_id)).flatten();
        let active = active_secret(&slots, previous, pointer);
        ui.ctx().data_mut(|d| d.insert_temp(active_id, active));
        let Some(i) = active else {
            return events;
        };
        let (Some(Some(slot)), Some(it)) = (slots.get(i), items.get(i)) else {
            return events;
        };
        let shown = shown.contains(&i);

        let painter = ui
            .ctx()
            .layer_painter(egui::LayerId::new(egui::Order::Middle, id.with("secret_pills")))
            .with_clip_rect(self.viewport);
        painter.rect(
            slot.pill,
            CornerRadius::same(6),
            POPUP,
            egui::Stroke::new(1.0_f32, POPUP_BORDER),
            egui::StrokeKind::Inside,
        );
        let eye = Rect::from_min_size(slot.pill.min + vec2(PILL_PAD, PILL_PAD), vec2(ICON, self.glyph_h));
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
            color(eye_resp.hovered(), it.revealed || shown),
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
        } else if shown {
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
            events.push((i, IconEvent::Copy(pos2(copy.center().x, slot.pill.top()))));
        }
        copy_resp.on_hover_text("Copy");
        events
    }

    /// The secret under a point, if any.
    pub fn at(&self, items: &[Item], pos: Pos2) -> Option<usize> {
        items
            .iter()
            .position(|it| self.slot(it).blocks.iter().any(|b| b.expand(2.0).contains(pos)))
    }
}

/// Which secret shows its pill: only one at a time, because a pill can
/// cover the secret on the line above. The pill on screen keeps it while
/// the pointer is on the pill, its own block or the gap between them;
/// otherwise the block under the pointer gets it.
fn active_secret(slots: &[Option<Slot>], previous: Option<usize>, pointer: Option<Pos2>) -> Option<usize> {
    let p = pointer?;
    if let Some(Some(slot)) = previous.and_then(|i| slots.get(i)) {
        let first = slot.blocks[0];
        // The strip between the pill and its block, as wide as the pill.
        let (top, bottom) = if slot.pill.center().y < first.center().y {
            (slot.pill.bottom(), first.top())
        } else {
            (first.bottom(), slot.pill.top())
        };
        let gap = Rect::from_x_y_ranges(slot.pill.x_range(), top..=bottom.max(top)).expand(2.0);
        let on_own_block = slot.blocks.iter().any(|b| b.expand(2.0).contains(p));
        let on_pill_or_gap = slot.pill.expand(2.0).contains(p) || gap.contains(p);
        if on_own_block || on_pill_or_gap {
            return previous;
        }
    }
    slots.iter().position(|s| {
        s.as_ref()
            .is_some_and(|s| s.blocks.iter().any(|b| b.expand(2.0).contains(p)))
    })
}

/// "Copied · clears in N s": a small note centered above `at`, which the
/// caller shows for a moment (DESIGN §11.1).
pub fn copied_note(ctx: &egui::Context, at: Pos2, clear_after_s: u32) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("secret_copied"),
    ));
    let text = format!("Copied \u{b7} clears in {clear_after_s} s");
    let galley = painter.layout_no_wrap(text, crate::theme::ui(12.0), TEXT);
    let pad = vec2(9.0, 6.0);
    let size = galley.size() + 2.0 * pad;
    let screen = ctx.screen_rect();
    let mut r = Rect::from_min_size(pos2(at.x - size.x / 2.0, at.y - 6.0 - size.y), size);
    let dx = (screen.left() + 4.0 - r.left()).max(0.0) - (r.right() - screen.right() + 4.0).max(0.0);
    r = r.translate(vec2(dx, 0.0));
    if r.top() < screen.top() + 4.0 {
        r = r.translate(vec2(0.0, size.y + 12.0));
    }
    painter.rect(
        r,
        CornerRadius::same(6),
        POPUP,
        egui::Stroke::new(1.0_f32, POPUP_BORDER),
        egui::StrokeKind::Inside,
    );
    painter.galley(r.min + pad, galley, TEXT);
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
    fn one_pill_at_a_time_when_secrets_are_stacked() {
        // Secret 0 on the upper line, secret 1 right below it; the lower
        // one's pill covers the upper block.
        let upper = Rect::from_min_size(pos2(100.0, 100.0), vec2(50.0, 18.0));
        let lower = upper.translate(vec2(0.0, 24.0));
        let pill_of =
            |b: Rect| Rect::from_min_size(pos2(b.left() - 2.0, b.top() - 2.0 - 4.0 - 24.0), vec2(42.0, 24.0));
        let slots = vec![
            Some(Slot {
                blocks: vec![upper],
                pill: pill_of(upper),
            }),
            Some(Slot {
                blocks: vec![lower],
                pill: pill_of(lower),
            }),
        ];
        let on_upper = upper.center();
        // Pointer on the upper block, nothing active before: the upper one.
        assert_eq!(active_secret(&slots, None, Some(on_upper)), Some(0));
        // The lower pill is showing and the pointer moved onto it (it sits
        // over the upper block): the lower one keeps it.
        let on_lower_pill = pill_of(lower).center();
        assert!(upper.expand(2.0).contains(on_lower_pill));
        assert_eq!(active_secret(&slots, Some(1), Some(on_lower_pill)), Some(1));
        // Off its pill, onto the upper block: the upper one takes over.
        let upper_right = pos2(upper.right() - 2.0, upper.center().y);
        assert_eq!(active_secret(&slots, Some(1), Some(upper_right)), Some(0));
        // Away from both: none.
        assert_eq!(active_secret(&slots, Some(0), Some(pos2(400.0, 400.0))), None);
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
