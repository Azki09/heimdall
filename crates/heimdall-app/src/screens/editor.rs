//! The editor: tabs in the title bar, the note text, and the status bar
//! (Editor mockup; behavior from DESIGN §10.1).

use std::time::Instant;

use eframe::egui::{
    self, pos2, text::CCursor, text::CCursorRange, text::LayoutJob, vec2, CornerRadius, CursorIcon, FontId,
    Key, Modifiers, Rect, Sense, Stroke, Ui,
};
use heimdall_core::content::{Note, TabInsert};
use uuid::Uuid;

use crate::app::{Action, LockReason};
use crate::chrome;
use crate::session::Session;
use crate::theme::*;
use crate::widgets::{self};

const TAB_H: f32 = 32.0;
const MAX_TITLE_CHARS: usize = 24;

#[derive(Default)]
pub struct EditorState {
    renaming: Option<(Uuid, String)>,
    dragging: Option<Uuid>,
    /// Put the cursor back where it was and focus the text (DESIGN §10.1).
    focus_editor: bool,
    restore_cursor: bool,
    line_col: (usize, usize),
    /// Last cursor/selection (char indices) and when it moved; restarts the blink.
    caret: ((usize, usize), f64),
}

impl EditorState {
    pub fn new(session: &Session) -> Self {
        Self {
            focus_editor: true,
            restore_cursor: session.vault.content.editor.reopen_tabs,
            ..Default::default()
        }
    }
}

pub fn ui(ctx: &egui::Context, s: &mut Session, st: &mut EditorState, last_input: Instant) -> Option<Action> {
    let active_before = s.vault.content.active_note;
    let mut action = shortcuts(ctx, s, st);

    if s.vault.content.notes.is_empty() {
        s.vault.content.new_note(heimdall_core::content::now());
        st.focus_editor = true;
    }
    if s.vault.content.active_note.is_none() {
        s.vault.content.active_note = s.vault.content.notes.first().map(|n| n.id);
    }

    chrome::title_bar(ctx, |ui| {
        let right_w = 200.0;
        let bar = ui.max_rect();
        let tabs_rect = Rect::from_min_size(
            bar.min,
            vec2((bar.width() - right_w).max(120.0), chrome::TITLE_BAR_H),
        );
        let mut tabs_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(tabs_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Min)),
        );
        tabs_ui.set_clip_rect(tabs_rect.intersect(ui.clip_rect()));
        egui::ScrollArea::horizontal()
            .id_salt("tabs_scroll")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .max_width(tabs_rect.width())
            .max_height(chrome::TITLE_BAR_H)
            .show(&mut tabs_ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    tabs(ui, s, st);
                });
            });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            if chrome_button(ui, "Lock", true)
                .on_hover_text("Lock (Ctrl+L)")
                .clicked()
            {
                action = Some(Action::Lock(LockReason::Manual));
            }
            ui.add_space(4.0);
            if chrome_button(ui, "Settings", false).clicked() {
                action = Some(Action::Settings);
            }
        });
    });

    status_bar(ctx, s, st, last_input);

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(SURFACE).inner_margin(egui::Margin {
            left: 48,
            right: 40,
            top: 28,
            bottom: 8,
        }))
        .show(ctx, |ui| text_area(ui, s, st));

    if s.vault.content.active_note != active_before {
        s.view_changed = true;
    }
    action
}

fn shortcuts(ctx: &egui::Context, s: &mut Session, st: &mut EditorState) -> Option<Action> {
    let cmd = Modifiers::COMMAND;
    let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
    let content = &mut s.vault.content;
    let now = heimdall_core::content::now();
    let mut action = None;
    let mut changed = false;

    ctx.input_mut(|i| {
        if i.consume_key(cmd, Key::T) {
            content.new_note(now);
            st.focus_editor = true;
            changed = true;
        }
        if i.consume_key(cmd, Key::W) {
            if let Some(id) = content.active_note {
                content.close_note(id, now);
                st.focus_editor = true;
                changed = true;
            }
        }
        if i.consume_key(cmd, Key::S) {
            action = Some(Save);
        }
        if i.consume_key(cmd, Key::L) {
            action = Some(Lock);
        }
        if i.consume_key(cmd, Key::Comma) {
            action = Some(OpenSettings);
        }
        let step = if i.consume_key(cmd_shift, Key::Tab) {
            Some(-1)
        } else if i.consume_key(cmd, Key::Tab) {
            Some(1)
        } else {
            None
        };
        let pos = content
            .active_note
            .and_then(|id| content.notes.iter().position(|n| n.id == id))
            .unwrap_or(0);
        let count = content.notes.len().max(1) as isize;
        if let Some(d) = step {
            let next = (pos as isize + d).rem_euclid(count) as usize;
            content.active_note = content.notes.get(next).map(|n| n.id);
            st.focus_editor = true;
        }
        let digits = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
        ];
        for (k, key) in digits.into_iter().enumerate() {
            if i.consume_key(cmd, key) {
                let target = if k == 8 {
                    content.notes.len().saturating_sub(1)
                } else {
                    k
                };
                if let Some(n) = content.notes.get(target) {
                    content.active_note = Some(n.id);
                    st.focus_editor = true;
                }
            }
        }
        let zoom = &mut content.editor.zoom_percent;
        let before = *zoom;
        if i.consume_key(cmd, Key::Plus) || i.consume_key(cmd, Key::Equals) {
            *zoom = (*zoom + 10).min(300);
        }
        if i.consume_key(cmd, Key::Minus) {
            *zoom = zoom.saturating_sub(10).max(50);
        }
        if i.consume_key(cmd, Key::Num0) {
            *zoom = 100;
        }
        changed |= *zoom != before;
    });

    if changed {
        s.touch(None);
    }
    match action {
        Some(Save) => {
            s.save();
            None
        }
        Some(Lock) => Some(Action::Lock(LockReason::Manual)),
        Some(OpenSettings) => Some(Action::Settings),
        None => None,
    }
}

enum Shortcut {
    Save,
    Lock,
    OpenSettings,
}
use Shortcut::*;

fn chrome_button(ui: &mut Ui, text: &str, lock_icon: bool) -> egui::Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), crate::theme::ui(12.0), TEXT_SECONDARY);
    let icon_w = if lock_icon { 20.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 24.0 + icon_w, 28.0), Sense::click());
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(6),
        if resp.hovered() {
            NAV_ACTIVE
        } else {
            egui::Color32::TRANSPARENT
        },
        Stroke::new(1.0_f32, BORDER),
        egui::StrokeKind::Inside,
    );
    if lock_icon {
        widgets::icon_lock(painter, pos2(rect.left() + 18.0, rect.center().y), TEXT_SECONDARY);
    }
    painter.galley(
        pos2(
            rect.left() + 12.0 + icon_w,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        TEXT_SECONDARY,
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp
}

fn tab_title(note: &Note) -> String {
    let t = note.display_title();
    if t.is_empty() {
        return "Untitled".into();
    }
    let mut out: String = t.chars().take(MAX_TITLE_CHARS).collect();
    if t.chars().count() > MAX_TITLE_CHARS {
        out.push('…');
    }
    out
}

fn tabs(ui: &mut Ui, s: &mut Session, st: &mut EditorState) {
    let active = s.vault.content.active_note;
    let mut rects: Vec<(Uuid, Rect)> = Vec::new();
    let mut activate = None;
    let mut close = None;
    let mut commit_rename = false;

    for note in &s.vault.content.notes {
        let id = note.id;
        let is_active = active == Some(id);

        if let Some((rid, text)) = st.renaming.as_mut().filter(|(rid, _)| *rid == id) {
            let _ = rid;
            let (outer, _) = ui.allocate_exact_size(vec2(170.0, chrome::TITLE_BAR_H), Sense::hover());
            let tab = Rect::from_min_max(pos2(outer.left(), outer.bottom() - TAB_H), outer.max);
            ui.painter().rect_filled(
                tab,
                CornerRadius {
                    nw: 6,
                    ne: 6,
                    sw: 0,
                    se: 0,
                },
                SURFACE,
            );
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(tab.shrink2(vec2(10.0, 6.0))));
            let edit_id = egui::Id::new(("rename", id));
            let resp = child.add(
                egui::TextEdit::singleline(text)
                    .id(edit_id)
                    .frame(false)
                    .font(crate::theme::ui(13.0))
                    .text_color(TEXT)
                    .hint_text(egui::RichText::new("First line").color(TEXT_FAINT))
                    .desired_width(150.0),
            );
            if !resp.has_focus() && !resp.lost_focus() {
                resp.request_focus();
            }
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                st.renaming = None;
                st.focus_editor = true;
            } else if resp.lost_focus() {
                commit_rename = true;
                // The Enter that confirmed the name must not reach the note text.
                ui.ctx().input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
            }
            rects.push((id, tab));
            continue;
        }

        let title = tab_title(note);
        let dirty = s.dirty_notes.contains(&id);
        let galley = ui
            .painter()
            .layout_no_wrap(title, crate::theme::ui(13.0), egui::Color32::PLACEHOLDER);
        let width = 14.0 + galley.size().x + if dirty { 16.0 } else { 0.0 } + 30.0;
        let (outer, resp) = ui.allocate_exact_size(vec2(width, chrome::TITLE_BAR_H), Sense::click_and_drag());
        let tab = Rect::from_min_max(pos2(outer.left(), outer.bottom() - TAB_H), outer.max);
        let hovered = resp.hovered();
        let painter = ui.painter();
        if is_active {
            painter.rect_filled(
                tab,
                CornerRadius {
                    nw: 6,
                    ne: 6,
                    sw: 0,
                    se: 0,
                },
                SURFACE,
            );
        } else if hovered {
            painter.rect_filled(tab.shrink2(vec2(0.0, 3.0)), CornerRadius::same(6), NAV_ACTIVE);
        }
        let text_color = if is_active || hovered { TEXT } else { TEXT_MUTED };
        let text_pos = pos2(tab.left() + 14.0, tab.center().y - galley.size().y / 2.0);
        let text_w = galley.size().x;
        painter.galley(text_pos, galley, text_color);
        if dirty {
            widgets::dot(painter, pos2(text_pos.x + text_w + 10.0, tab.center().y), ICON);
        }
        let close_rect = Rect::from_center_size(pos2(tab.right() - 16.0, tab.center().y), vec2(20.0, 20.0));
        let close_resp = ui.interact(close_rect, egui::Id::new(("tab_close", id)), Sense::click());
        if is_active || hovered || close_resp.hovered() {
            if close_resp.hovered() {
                ui.painter()
                    .rect_filled(close_rect, CornerRadius::same(4), TRACK_OFF);
            }
            widgets::icon_close(
                ui.painter(),
                close_rect.center(),
                9.0,
                if close_resp.hovered() { TEXT } else { TEXT_MUTED },
            );
        }
        if close_resp.clicked() {
            close = Some(id);
        } else if resp.double_clicked() {
            st.renaming = Some((id, note.title.clone()));
        } else if resp.clicked() || resp.drag_started() {
            activate = Some(id);
        }
        if resp.drag_started() {
            st.dragging = Some(id);
        }
        resp.on_hover_text(note.display_title().chars().take(80).collect::<String>());
        rects.push((id, tab));
    }

    // New tab button.
    let (plus, plus_resp) = ui.allocate_exact_size(vec2(32.0, chrome::TITLE_BAR_H), Sense::click());
    let plus_box = Rect::from_center_size(
        pos2(plus.center().x, plus.bottom() - TAB_H / 2.0),
        vec2(32.0, 32.0),
    );
    if plus_resp.hovered() {
        ui.painter()
            .rect_filled(plus_box, CornerRadius::same(6), NAV_ACTIVE);
    }
    widgets::icon_plus(
        ui.painter(),
        plus_box.center(),
        12.0,
        if plus_resp.hovered() { TEXT } else { TEXT_MUTED },
    );
    let plus_resp = plus_resp.on_hover_text("New tab (Ctrl+T)");

    let content = &mut s.vault.content;
    let now = heimdall_core::content::now();
    let mut changed_note = None;
    let mut changed = false;

    if commit_rename {
        if let Some((id, text)) = st.renaming.take() {
            if let Some(n) = content.note_mut(id) {
                let title = text.trim().to_owned();
                if n.title != title {
                    n.title = title;
                    n.modified = now;
                    changed_note = Some(id);
                }
            }
            st.focus_editor = true;
        }
    }
    if let Some(id) = activate {
        if content.active_note != Some(id) {
            content.active_note = Some(id);
            st.focus_editor = true;
        }
    }
    if let Some(id) = close {
        content.close_note(id, now);
        st.focus_editor = true;
        changed = true;
    }
    if plus_resp.clicked() {
        content.new_note(now);
        st.focus_editor = true;
        changed = true;
    }

    // Drag to reorder: swap with the tab under the pointer.
    if let Some(dragged) = st.dragging {
        if !ui.input(|i| i.pointer.primary_down()) {
            st.dragging = None;
        } else if let Some(x) = ui.input(|i| i.pointer.hover_pos()).map(|p| p.x) {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            let from = content.notes.iter().position(|n| n.id == dragged);
            let to = rects.iter().position(|(_, r)| x >= r.left() && x <= r.right());
            if let (Some(from), Some(to)) = (from, to) {
                if from != to {
                    let note = content.notes.remove(from);
                    content.notes.insert(to, note);
                    changed = true;
                }
            }
        }
    }

    if changed || changed_note.is_some() {
        s.touch(changed_note);
    }
}

fn text_area(ui: &mut Ui, s: &mut Session, st: &mut EditorState) {
    let ctx = ui.ctx().clone();
    let settings = s.vault.content.editor.clone();
    let Some(active) = s.vault.content.active_note else {
        return;
    };
    let id = egui::Id::new(("note", active));

    let zoom = settings.zoom_percent as f32 / 100.0;
    let size = settings.font_size as f32 * zoom;
    let font = FontId::new(size, theme_family(&ctx, settings.font));
    let line_h = (size * 26.0 / 14.0).round();
    let wrap = settings.word_wrap;

    let Some(note) = s.vault.content.note_mut(active) else {
        return;
    };

    if std::mem::take(&mut st.focus_editor) {
        if st.restore_cursor {
            st.restore_cursor = false;
            let chars = note.body.chars().count();
            let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
            let at = (note.cursor as usize).min(chars);
            state
                .cursor
                .set_char_range(Some(CCursorRange::one(CCursor::new(at))));
            state.store(&ctx, id);
        }
        ctx.memory_mut(|m| m.request_focus(id));
    }

    // Tab inserts spaces, if so configured (DESIGN §10.4).
    let mut inserted = false;
    if let TabInsert::Spaces(n) = settings.tab_inserts {
        if ctx.memory(|m| m.has_focus(id)) && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab)) {
            if let Some(mut state) = egui::TextEdit::load_state(&ctx, id) {
                if let Some(range) = state.cursor.char_range() {
                    let a = range.primary.index.min(range.secondary.index);
                    let b = range.primary.index.max(range.secondary.index);
                    let byte = |c: usize| {
                        note.body
                            .char_indices()
                            .nth(c)
                            .map(|(i, _)| i)
                            .unwrap_or(note.body.len())
                    };
                    let (start, end) = (byte(a), byte(b));
                    note.body.replace_range(start..end, &" ".repeat(n as usize));
                    state
                        .cursor
                        .set_char_range(Some(CCursorRange::one(CCursor::new(a + n as usize))));
                    state.store(&ctx, id);
                    inserted = true;
                }
            }
        }
    }

    let mut layouter = |ui: &Ui, text: &str, wrap_width: f32| {
        let mut job = LayoutJob::simple(
            text.to_owned(),
            font.clone(),
            TEXT_BODY,
            if wrap { wrap_width } else { f32::INFINITY },
        );
        for section in &mut job.sections {
            section.format.line_height = Some(line_h);
        }
        ui.fonts(|f| f.layout_job(job))
    };

    let scroll = if wrap {
        egui::ScrollArea::vertical()
    } else {
        egui::ScrollArea::both()
    };
    let output = scroll
        .id_salt(("note_scroll", active))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // egui draws the cursor as tall as the whole (spaced) row; we draw our own below.
            ui.visuals_mut().text_cursor.stroke = Stroke::NONE;
            egui::TextEdit::multiline(&mut note.body)
                .id(id)
                .frame(false)
                .lock_focus(true)
                .desired_width(if wrap { ui.available_width() } else { f32::INFINITY })
                .min_size(ui.available_size())
                .layouter(&mut layouter)
                .show(ui)
        });
    let viewport = output.inner_rect;
    let output = output.inner;

    if let Some(range) = output.cursor_range {
        let now = ctx.input(|i| i.time);
        let key = (range.primary.ccursor.index, range.secondary.ccursor.index);
        if st.caret.0 != key || output.response.changed() {
            st.caret = (key, now);
        }
        if output.response.has_focus() && ctx.input(|i| i.focused) {
            // Glyphs sit at the top of each spaced row, so the caret spans
            // only the font's own height from there.
            let row = output
                .galley
                .pos_from_cursor(&range.primary)
                .translate(output.galley_pos.to_vec2());
            let glyph_h = ui.fonts(|f| f.row_height(&font));
            let caret = Rect::from_min_size(row.min, vec2(0.0, glyph_h));
            egui::text_selection::visuals::paint_text_cursor(
                ui,
                &ui.painter_at(viewport),
                caret,
                now - st.caret.1,
            );
        }

        let idx = range.primary.ccursor.index;
        if note.cursor != idx as u64 {
            note.cursor = idx as u64;
            s.view_changed = true;
        }
        st.line_col = line_col(&note.body, idx);
    }
    if output.response.changed() || inserted {
        note.modified = heimdall_core::content::now();
        s.touch(Some(active));
    }
}

fn theme_family(ctx: &egui::Context, font: heimdall_core::content::Font) -> egui::FontFamily {
    crate::theme::editor_family(ctx, font)
}

fn line_col(text: &str, char_idx: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for c in text.chars().take(char_idx) {
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

fn status_bar(ctx: &egui::Context, s: &Session, st: &EditorState, last_input: Instant) {
    let show = s.vault.content.editor.status_bar;
    if !show && s.save_error.is_none() {
        return;
    }
    egui::TopBottomPanel::bottom("status_bar")
        .exact_height(30.0)
        .frame(
            egui::Frame::NONE
                .fill(BG)
                .inner_margin(egui::Margin::symmetric(16, 0)),
        )
        .show_separator_line(false)
        .show(ctx, |ui| {
            let r = ui.max_rect();
            ui.painter().hline(
                r.x_range().expand(16.0),
                r.top(),
                Stroke::new(1.0_f32, egui::Color32::from_gray(0x24)),
            );
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 20.0;
                let small = |ui: &mut Ui, t: &str| widgets::label(ui, t, 12.0, TEXT_MUTED);
                if let Some(err) = &s.save_error {
                    widgets::label(ui, &format!("Not saved: {err}"), 12.0, DANGER);
                } else {
                    small(ui, "Encrypted");
                }
                let sec = &s.vault.content.security;
                let profile = format!(
                    "{:?}{}",
                    sec.profile,
                    if sec.is_custom() { " · Custom" } else { "" }
                );
                small(ui, &profile);
                if let Some(p) = &s.presence {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let (r, _) = ui.allocate_exact_size(vec2(6.0, 6.0), Sense::hover());
                        widgets::dot(ui.painter(), r.center(), OK);
                        small(ui, "USB key connected").on_hover_text(&p.key_label);
                    });
                }
                if let Some(secs) = sec.lock_after_s {
                    let left = (secs as u64).saturating_sub(last_input.elapsed().as_secs());
                    small(ui, &format!("Locks in {}:{:02}", left / 60, left % 60));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    small(ui, "UTF-8");
                    let (l, c) = st.line_col;
                    small(ui, &format!("Ln {l}, Col {c}"));
                });
            });
        });
}
