//! The editor: tabs in the title bar, the note text, and the status bar
//! (Editor mockup; behavior from DESIGN §10.1). A tab in code mode
//! (§10.6) adds line numbers, syntax colors, indent guides, matching
//! brackets and a minimap (Code mode mockups). Also find (Ctrl+F), find
//! in all notes (Ctrl+Shift+F), spell check (§10.5, §10.4) and masked
//! secrets (§11; layout and caret rules in `crate::secrets`).

use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use eframe::egui::{
    self, pos2, text::CCursor, text::CCursorRange, text::LayoutJob, vec2, Align2, CornerRadius, CursorIcon,
    FontId, Key, Modifiers, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2,
};
use heimdall_core::content::{CodeMode, Font, Note, Profile, SecuritySettings, TabInsert};
use heimdall_core::secret;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::app::{Action, LockReason};
use crate::chrome;
use crate::code::{self, Kind};
use crate::screens::confirm::{Outcome, PasswordCheck};
use crate::search::{self, Match};
use crate::secrets::{self, IconEvent, Pasted};
use crate::session::Session;
use crate::spell::Speller;
use crate::theme::*;
use crate::widgets::{self, ButtonKind, SecretField};

const TAB_H: f32 = 32.0;
/// Every tab has the same width: the widest that fits, within these bounds.
/// Past the minimum the tab strip scrolls.
const TAB_MAX_W: f32 = 200.0;
const TAB_MIN_W: f32 = 96.0;

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
    /// The tab last scrolled into view; a newly active tab is scrolled to once.
    shown_tab: Option<Uuid>,
    /// Syntax colors of the active note, recomputed when its text changes.
    highlight: Highlight,
    /// The bracket pair at the cursor, for the text and cursor it was found at.
    brackets: Option<BracketCache>,
    /// Scroll the note here next frame (a click on the minimap, a match).
    scroll_to: Option<Vec2>,
    /// The note's scroll offset this frame.
    offset: Vec2,
    /// The new-tab tip (code mode shortcut), while it shows.
    tip: Option<Tip>,
    /// The language picker of the status bar, while it is open.
    picker: Option<Picker>,
    /// Where the status bar's language button was last drawn.
    language_button: Option<Rect>,
    /// The find bar (Ctrl+F), while it is open.
    find: Option<Find>,
    /// What was last searched for, offered again by Ctrl+F.
    last_query: String,
    /// Find in all notes (Ctrl+Shift+F), while it is open.
    find_all: Option<FindAll>,
    /// Select this char range of the active note and scroll to it.
    reveal: Option<(usize, usize)>,
    /// The list of all tabs, open under the `⌄` button drawn here.
    tab_menu: Option<Rect>,
    /// The right-click menu of a tab, where it opened, and whether this is
    /// the frame it opened in (that click must not close it again).
    tab_context: Option<(Uuid, Pos2, bool)>,
    spell: Speller,
    /// The right-click menu of a misspelled word, while it is open.
    spell_menu: Option<SpellMenu>,
    /// Which secrets of the active note show their value.
    reveals: Reveals,
    /// Ctrl+Shift+H (or the menu): make the selection a secret.
    mark_secret: bool,
    /// The right-click menu of a secret, or the general one, while it is open.
    secret_menu: Option<SecretMenu>,
    /// Cut / Copy / Paste / Select all chosen in the right-click menu; done
    /// next frame, once the text has the focus back.
    edit_action: Option<EditAction>,
    /// The Edit secret / New secret dialog.
    secret_edit: Option<SecretEdit>,
    /// Reveal all asks for the password first (Basic only).
    reveal_check: Option<PasswordCheck>,
    /// A paste the detector changed, until the text edit has inserted it.
    paste: Option<PendingPaste>,
    /// The message after masking (with Undo), bottom right.
    toast: Option<Toast>,
}

impl Drop for EditorState {
    fn drop(&mut self) {
        self.last_query.zeroize();
    }
}

/// (text key, cursor char index) and the matching pair, as char indices.
type BracketCache = ((u64, usize), Option<(usize, usize)>);

/// Ctrl+F: matches of the query in the active note.
struct Find {
    query: String,
    match_case: bool,
    matches: Vec<Match>,
    current: Option<usize>,
    /// Where the search starts from: the cursor when the bar opened.
    origin: usize,
    /// (note, text) and (query, case) the matches were found for.
    text_key: u64,
    found_for: Option<(String, bool)>,
    focus: bool,
}

impl Find {
    fn new(query: String, match_case: bool, origin: usize) -> Self {
        Self {
            query,
            match_case,
            matches: Vec::new(),
            current: None,
            origin,
            text_key: 0,
            found_for: None,
            focus: true,
        }
    }

    /// Searches again if the note or the query changed. A new query moves
    /// to its first match from where the search started; returns it.
    fn update(&mut self, note: Uuid, text: &str, cursor: usize) -> Option<(usize, usize)> {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        note.hash(&mut h);
        text.hash(&mut h);
        let text_key = h.finish();
        let query_changed = self
            .found_for
            .as_ref()
            .is_none_or(|(q, c)| *q != self.query || *c != self.match_case);
        if !query_changed && text_key == self.text_key {
            return None;
        }
        self.matches = search::find_all(text, &self.query, self.match_case);
        self.text_key = text_key;
        if let Some((q, _)) = &mut self.found_for {
            q.zeroize();
        }
        self.found_for = Some((self.query.clone(), self.match_case));
        let from = if query_changed { self.origin } else { cursor };
        self.current = search::first_from(&self.matches, from);
        if query_changed {
            self.current.map(|i| self.matches[i].chars)
        } else {
            None
        }
    }
}

impl Drop for Find {
    fn drop(&mut self) {
        self.query.zeroize();
        if let Some((q, _)) = &mut self.found_for {
            q.zeroize();
        }
    }
}

/// Ctrl+Shift+F: matches in every open note.
#[derive(Default)]
struct FindAll {
    query: String,
    match_case: bool,
    /// The query, case and last edit the results are for.
    found_for: Option<(String, bool, Instant)>,
    results: Vec<NoteHits>,
    focus: bool,
}

impl Drop for FindAll {
    fn drop(&mut self) {
        self.query.zeroize();
        if let Some((q, _, _)) = &mut self.found_for {
            q.zeroize();
        }
        for r in &mut self.results {
            for (_, p) in &mut r.hits {
                p.text.zeroize();
            }
        }
    }
}

/// Show at most this many matching lines per note.
const MAX_HITS: usize = 20;

struct NoteHits {
    id: Uuid,
    title: String,
    code: bool,
    total: usize,
    hits: Vec<(Match, search::Preview)>,
}

struct SpellMenu {
    pos: Pos2,
    note: Uuid,
    bytes: Range<usize>,
    word: String,
    suggestions: Vec<String>,
}

impl Drop for SpellMenu {
    fn drop(&mut self) {
        self.word.zeroize();
        for s in &mut self.suggestions {
            s.zeroize();
        }
    }
}

/// Secrets showing their value: by index in the note, until an egui time.
/// All of them hide when the tab changes or the window loses focus.
#[derive(Default)]
struct Reveals {
    note: Option<Uuid>,
    until: Vec<(usize, f64)>,
    /// Reveal all (Basic, after the password).
    all: bool,
}

impl Reveals {
    fn hide_all(&mut self) {
        self.until.clear();
        self.all = false;
    }

    fn shown(&self) -> (bool, Vec<usize>) {
        (self.all, self.until.iter().map(|(i, _)| *i).collect())
    }

    fn is_shown(&self, i: usize) -> bool {
        self.all || self.until.iter().any(|(j, _)| *j == i)
    }

    fn show(&mut self, i: usize, until: f64) {
        self.until.retain(|(j, _)| *j != i);
        self.until.push((i, until));
    }

    fn toggle(&mut self, i: usize, until: f64) {
        if self.until.iter().any(|(j, _)| *j == i) {
            self.until.retain(|(j, _)| *j != i);
        } else {
            self.show(i, until);
        }
    }
}

struct SecretMenu {
    pos: Pos2,
    note: Uuid,
    /// The secret right-clicked (its index), or `None` for the general menu.
    target: Option<usize>,
    has_selection: bool,
    /// The clipboard had text when the menu opened (enables Paste).
    can_paste: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EditAction {
    Cut,
    Copy,
    Paste,
    SelectAll,
}

struct SecretEdit {
    note: Uuid,
    /// The secret being edited (its index), or `None` for a new one.
    target: Option<usize>,
    /// Where a new secret goes (char index).
    at: usize,
    value: SecretField,
    /// The current value was put in the field.
    loaded: bool,
    error: Option<&'static str>,
}

struct PendingPaste {
    kind: Pasted,
    original: String,
    inserted: String,
}

impl Drop for PendingPaste {
    fn drop(&mut self) {
        self.original.zeroize();
        self.inserted.zeroize();
    }
}

/// Undo for the paste detector: put `replacement` back where `inserted` is.
struct Undo {
    note: Uuid,
    chars: (usize, usize),
    inserted: String,
    replacement: String,
}

impl Drop for Undo {
    fn drop(&mut self) {
        self.inserted.zeroize();
        self.replacement.zeroize();
    }
}

struct Toast {
    text: String,
    undo: Option<Undo>,
    /// The first time the detector masks something, say why.
    explain: bool,
    left: f32,
}

impl Toast {
    fn message(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            undo: None,
            explain: false,
            left: TOAST_SECONDS,
        }
    }
}

const TOAST_SECONDS: f32 = 6.0;

/// How long the new-tab tip stays, in seconds (paused while hovered).
const TIP_SECONDS: f32 = 5.0;

struct Tip {
    left: f32,
}

#[derive(Default)]
struct Picker {
    query: String,
    focused: bool,
}

#[derive(Default)]
struct Highlight {
    key: Option<u64>,
    spans: Arc<Vec<code::Span>>,
}

impl Highlight {
    fn spans_for(&mut self, lang: Option<&code::Language>, text: &str) -> Arc<Vec<code::Span>> {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        lang.map(|l| l.id).hash(&mut h);
        text.hash(&mut h);
        let key = h.finish();
        if self.key != Some(key) {
            self.spans = Arc::new(code::highlight(lang, text));
            self.key = Some(key);
        }
        self.spans.clone()
    }
}

/// Word wrap for a tab: its own choice (Alt+Z), else off in code mode and
/// the global setting in plain notes.
fn wrap_for(note: &Note, global: bool) -> bool {
    note.wrap.unwrap_or(note.code.is_none() && global)
}

impl EditorState {
    pub fn new(session: &Session) -> Self {
        let mut st = Self::default();
        st.focus_editor = true;
        st.restore_cursor = session.vault.content.editor.reopen_tabs;
        st
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
        let avail = (bar.width() - right_w).max(120.0);
        // Tabs that don't fit scroll. The `⌄` button lists them all, with
        // export, import and backup underneath (DESIGN §10.7, §10.9); it
        // sits right after `+`, or at the end of the strip once it is full.
        let n = s.vault.content.notes.len().max(1) as f32;
        let tabs_rect = Rect::from_min_size(bar.min, vec2(avail - 36.0, chrome::TITLE_BAR_H));
        let tab_w = ((tabs_rect.width() - 32.0 - 4.0) / n - 4.0).clamp(TAB_MIN_W, TAB_MAX_W);
        let used = n * (tab_w + 4.0) + 32.0;
        tab_menu_button(ui, st, bar.left() + used.min(tabs_rect.width()) + 2.0);
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
                    tabs(ui, s, st, tabs_rect.width());
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

    let code_mode = s
        .vault
        .content
        .active_note
        .and_then(|id| s.vault.content.note(id))
        .is_some_and(|n| n.code.is_some());
    let margin = if code_mode {
        egui::Margin {
            left: 0,
            right: 0,
            top: 8,
            bottom: 0,
        }
    } else {
        egui::Margin {
            // The line numbers take the place of the left margin.
            left: if s.vault.content.editor.line_numbers {
                0
            } else {
                48
            },
            right: 40,
            top: 28,
            bottom: 8,
        }
    };
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(SURFACE).inner_margin(margin))
        .show(ctx, |ui| text_area(ui, s, st));

    language_picker(ctx, s, st);
    spell_menu(ctx, s, st);
    secret_menu(ctx, s, st);
    secret_edit(ctx, s, st);
    if let Some(check) = st.reveal_check.as_mut() {
        match check.ui(ctx, s) {
            Outcome::Open => {}
            Outcome::Canceled => {
                st.reveal_check = None;
                st.focus_editor = true;
            }
            Outcome::Confirmed => {
                st.reveal_check = None;
                st.reveals.all = true;
                st.focus_editor = true;
            }
        }
    }
    toast(ctx, s, st);
    if let Some(a) = tab_menu(ctx, s, st) {
        action = Some(a);
    }
    if let Some(a) = tab_context_menu(ctx, s, st) {
        action = Some(a);
    }
    // Clear of the minimap, which shows where the matches are anyway.
    let right = if code_mode { MINIMAP_W + 16.0 } else { 24.0 };
    find_bar(ctx, st, right, s.vault.content.active_note);
    find_all_panel(ctx, s, st, right);
    if let Some(a) = code_tip(ctx, s, st) {
        action = Some(a);
    }

    if s.vault.content.active_note != active_before {
        s.view_changed = true;
        st.picker = None;
        st.spell_menu = None;
        st.secret_menu = None;
        st.secret_edit = None;
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
    let mut open_find = false;
    let mut open_find_all = false;
    let mut find_step = 0;

    let global_wrap = content.editor.word_wrap;
    ctx.input_mut(|i| {
        // Shift variants first: consume_key ignores extra modifiers.
        if i.consume_key(cmd_shift, Key::F) {
            open_find_all = true;
        }
        if i.consume_key(cmd, Key::F) {
            open_find = true;
        }
        if i.consume_key(Modifiers::SHIFT, Key::F3) {
            find_step = -1;
        } else if i.consume_key(Modifiers::NONE, Key::F3) {
            find_step = 1;
        }
        // Before Ctrl+T: consume_key ignores extra modifiers, so Ctrl+Shift+T
        // would match Ctrl+T too.
        if i.consume_key(cmd_shift, Key::T) && content.reopen_last_closed().is_some() {
            st.focus_editor = true;
            changed = true;
        }
        if i.consume_key(cmd_shift, Key::H) {
            st.mark_secret = true;
            st.focus_editor = true;
        }
        if i.consume_key(cmd_shift, Key::M) {
            if let Some(n) = content.active_note.and_then(|id| content.note_mut(id)) {
                n.code = match n.code {
                    Some(_) => None,
                    None => Some(CodeMode {
                        language: String::new(),
                    }),
                };
                n.wrap = None;
                st.tip = None;
                st.picker = None;
                st.focus_editor = true;
                changed = true;
            }
        }
        if i.consume_key(Modifiers::ALT, Key::Z) {
            // Windows may send the "z" as typed text as well.
            i.events
                .retain(|e| !matches!(e, egui::Event::Text(t) if t.eq_ignore_ascii_case("z")));
            if let Some(n) = content.active_note.and_then(|id| content.note_mut(id)) {
                n.wrap = Some(!wrap_for(n, global_wrap));
                changed = true;
            }
        }
        if i.consume_key(cmd, Key::T) {
            content.new_note(now);
            if content.editor.code_tip {
                st.tip = Some(Tip { left: TIP_SECONDS });
            }
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
    if open_find {
        let (selected, origin) = selection(ctx, s);
        match &mut st.find {
            Some(f) => {
                if let Some(q) = selected {
                    f.query = q;
                    f.origin = origin;
                }
                f.focus = true;
            }
            None => {
                let query = selected.unwrap_or_else(|| st.last_query.clone());
                st.find = Some(Find::new(query, false, origin));
            }
        }
        st.find_all = None;
    }
    if open_find_all {
        let (selected, _) = selection(ctx, s);
        let fa = st.find_all.get_or_insert_with(FindAll::default);
        if let Some(q) = selected.or_else(|| st.find.as_ref().map(|f| f.query.clone())) {
            fa.query = q;
        }
        fa.focus = true;
        st.find = None;
    }
    if find_step != 0 {
        if st.find.is_none() {
            // F3 with the bar closed searches again for the last query.
            let (_, origin) = selection(ctx, s);
            let mut f = Find::new(st.last_query.clone(), false, origin);
            f.focus = st.last_query.is_empty();
            st.find = Some(f);
        } else {
            step_match(ctx, st, find_step, s.vault.content.active_note);
        }
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

/// The tab title, secrets masked (DESIGN §11.1).
fn tab_title(note: &Note) -> String {
    let t = secret::mask(note.display_title());
    if t.is_empty() {
        return "Untitled".into();
    }
    t.chars().take(200).collect()
}

/// `avail` is the width of the tab strip, new-tab button included.
fn tabs(ui: &mut Ui, s: &mut Session, st: &mut EditorState, avail: f32) {
    let active = s.vault.content.active_note;
    let spacing = ui.spacing().item_spacing.x;
    let n = s.vault.content.notes.len().max(1) as f32;
    let tab_w = ((avail - 32.0 - spacing) / n - spacing).clamp(TAB_MIN_W, TAB_MAX_W);
    let mut rects: Vec<(Uuid, Rect)> = Vec::new();
    let mut activate = None;
    let mut close = None;
    let mut commit_rename = false;

    for note in &s.vault.content.notes {
        let id = note.id;
        let is_active = active == Some(id);

        if let Some((rid, text)) = st.renaming.as_mut().filter(|(rid, _)| *rid == id) {
            let _ = rid;
            let (outer, _) = ui.allocate_exact_size(vec2(tab_w, chrome::TITLE_BAR_H), Sense::hover());
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
                    .desired_width(tab_w - 20.0),
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
        let icon_w = if note.code.is_some() { 20.0 } else { 0.0 };
        // Long titles are cut to the tab with "…"; the full one is in the tooltip.
        let text_max = tab_w - 14.0 - icon_w - if dirty { 16.0 } else { 0.0 } - 30.0;
        let mut job = egui::text::LayoutJob::simple_singleline(
            title,
            crate::theme::ui(13.0),
            egui::Color32::PLACEHOLDER,
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(text_max);
        let galley = ui.painter().layout_job(job);
        let (outer, resp) = ui.allocate_exact_size(vec2(tab_w, chrome::TITLE_BAR_H), Sense::click_and_drag());
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
        if note.code.is_some() {
            let color = if is_active || hovered { ICON } else { TEXT_FAINT };
            widgets::icon_code(painter, pos2(tab.left() + 20.0, tab.center().y), 12.0, color);
        }
        let text_pos = pos2(tab.left() + 14.0 + icon_w, tab.center().y - galley.size().y / 2.0);
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
        if resp.secondary_clicked() {
            st.tab_context = resp.interact_pointer_pos().map(|p| (id, p, true));
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
        if is_active && st.shown_tab != Some(id) {
            resp.scroll_to_me(None);
            st.shown_tab = Some(id);
        }
        resp.on_hover_text(title_hover(note));
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
        if content.editor.code_tip {
            st.tip = Some(Tip { left: TIP_SECONDS });
        }
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

/// Width of the minimap on the right of a tab in code mode.
const MINIMAP_W: f32 = 112.0;
/// Minimap scale: one text line is this tall, one character this wide.
const MINIMAP_LINE: f32 = 4.0;
const MINIMAP_CHAR: f32 = 1.2;

fn text_area(ui: &mut Ui, s: &mut Session, st: &mut EditorState) {
    let ctx = ui.ctx().clone();
    let settings = s.vault.content.editor.clone();
    let security = s.vault.content.security.clone();
    let mut secret_tip_seen = false;
    let Some(active) = s.vault.content.active_note else {
        return;
    };
    let id = egui::Id::new(("note", active));
    let spell_on = settings.spell_check;
    let personal = Zeroizing::new(if spell_on {
        s.vault.content.dictionary.clone()
    } else {
        Vec::new()
    });

    let Some(note) = s.vault.content.note_mut(active) else {
        return;
    };

    let code_mode = note.code.is_some();
    // Plain notes can show just the numbers; code mode always does.
    let numbers = code_mode || settings.line_numbers;
    let spell_on = spell_on && !code_mode;
    let lang = note.code.as_ref().and_then(|c| code::resolve(c, note));
    let zoom = settings.zoom_percent as f32 / 100.0;
    let size = settings.font_size as f32 * zoom;
    let (font, line_h) = if code_mode {
        (
            FontId::new(size, code_family(&ctx, settings.font)),
            (size * 22.0 / 14.0).round(),
        )
    } else {
        (
            FontId::new(size, theme_family(&ctx, settings.font)),
            (size * 26.0 / 14.0).round(),
        )
    };
    let wrap = wrap_for(note, settings.word_wrap);
    let tab_cols = match settings.tab_inserts {
        TabInsert::Spaces(n) => n as usize,
        TabInsert::Tab => 4,
    };

    // Masked secrets (DESIGN §11.1): which ones show, for this tab only,
    // while the window has focus.
    let now = ctx.input(|i| i.time);
    if st.reveals.note != Some(active) || !ctx.input(|i| i.focused) {
        st.reveals.hide_all();
        st.reveals.note = Some(active);
    }
    st.reveals.until.retain(|(_, t)| *t > now);
    if let Some(next) = st.reveals.until.iter().map(|(_, t)| *t).reduce(f64::min) {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((next - now).max(0.0)));
    }
    // Reveal all shows values in the text; one secret's Reveal shows it in
    // a bubble, so the text (and the icon under the pointer) stays put.
    let (all_shown, popped) = st.reveals.shown();
    let is_revealed = |_: usize| all_shown;

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
        if st.picker.is_none() {
            ctx.memory_mut(|m| m.request_focus(id));
        }
    }

    // Find: search again if the note or the query changed; a new query
    // goes to its first match.
    if let Some(find) = &mut st.find {
        if let Some(range) = find.update(active, &note.body, note.cursor as usize) {
            st.reveal = Some(range);
        }
    }
    // A match to show: select it (it is scrolled to below, once laid out).
    if let Some((a, b)) = st.reveal {
        let chars = note.body.chars().count();
        let (a, b) = (a.min(chars), b.min(chars));
        let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
        state.store(&ctx, id);
        note.cursor = b as u64;
    }

    // Tab inserts spaces, if so configured (DESIGN §10.4).
    let mut inserted = false;
    if let TabInsert::Spaces(n) = settings.tab_inserts {
        if ctx.memory(|m| m.has_focus(id)) && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab)) {
            if let Some(mut state) = egui::TextEdit::load_state(&ctx, id) {
                if let Some(range) = state.cursor.char_range() {
                    let a = range.primary.index.min(range.secondary.index);
                    let b = range.primary.index.max(range.secondary.index);
                    let (start, end) = (char_to_byte(&note.body, a), char_to_byte(&note.body, b));
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

    // Cut / Copy / Paste / Select all from the right-click menu. They send
    // the same events as the keyboard, so a copy goes through
    // `handle_copies` (kept out of the clipboard history, auto-cleared,
    // shorter timer with a secret) and a paste through the detector below.
    if let Some(action) = st.edit_action.take() {
        match action {
            EditAction::SelectAll => {
                let chars = note.body.chars().count();
                let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
                state
                    .cursor
                    .set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(chars))));
                state.store(&ctx, id);
            }
            EditAction::Cut => ctx.input_mut(|i| i.events.push(egui::Event::Cut)),
            EditAction::Copy => ctx.input_mut(|i| i.events.push(egui::Event::Copy)),
            EditAction::Paste => {
                if let Some(text) = crate::support::clipboard_text() {
                    ctx.input_mut(|i| i.events.push(egui::Event::Paste(String::clone(&text))));
                }
            }
        }
    }

    let has_focus = ctx.memory(|m| m.has_focus(id));
    if std::mem::take(&mut st.mark_secret) {
        inserted |= mark_secret(&ctx, id, note, st);
    }
    if has_focus {
        let before = secrets::items(&note.body, &is_revealed);
        if !before.is_empty() {
            inserted |= delete_secret(&ctx, id, note, &before);
        }
        // The paste detector (§11.3) works on the text before egui inserts it.
        ctx.input_mut(|i| {
            for e in &mut i.events {
                if let egui::Event::Paste(text) = e {
                    let mut original = text.clone();
                    match secrets::on_paste(text, settings.mask_pasted, code_mode) {
                        Some(kind) => {
                            st.paste = Some(PendingPaste {
                                kind,
                                original,
                                inserted: text.clone(),
                            })
                        }
                        None => original.zeroize(),
                    }
                }
            }
        });
    }
    // A right-click moves the caret in egui; keep the selection for the menu.
    let kept_selection = ctx
        .input(|i| i.pointer.button_pressed(egui::PointerButton::Secondary))
        .then(|| egui::TextEdit::load_state(&ctx, id).and_then(|s| s.cursor.char_range()))
        .flatten()
        .filter(|r| r.primary != r.secondary);

    let char_w = ui.fonts(|f| f.glyph_width(&font, ' '));
    let digit_w = ui.fonts(|f| f.glyph_width(&font, '0'));
    let glyph_h = ui.fonts(|f| f.row_height(&font));
    let full = ui.max_rect();
    let gutter = numbers.then(|| {
        let lines = note.body.split('\n').count();
        let digits = lines.to_string().len().max(2) as f32;
        let gutter_w = (16.0 + digits * digit_w + 22.0).round();
        Rect::from_min_max(full.min, pos2(full.left() + gutter_w, full.bottom()))
    });
    let minimap =
        code_mode.then(|| Rect::from_min_max(pos2(full.right() - MINIMAP_W, full.top() - 8.0), full.max));
    let text_rect = Rect::from_min_max(
        pos2(gutter.map_or(full.left(), |g| g.right()), full.top()),
        pos2(minimap.map_or(full.right(), |m| m.left()), full.bottom()),
    );

    let highlight = &mut st.highlight;
    let mut layouter = |ui: &Ui, text: &str, wrap_width: f32| {
        let wrap_width = if wrap { wrap_width } else { f32::INFINITY };
        let mut job = if code_mode {
            code_job(&highlight.spans_for(lang, text), text, &font, line_h, wrap_width)
        } else {
            let mut job = LayoutJob::simple(text.to_owned(), font.clone(), TEXT_BODY, wrap_width);
            for section in &mut job.sections {
                section.format.line_height = Some(line_h);
            }
            job
        };
        ui.fonts(|f| {
            secrets::restyle(&mut job, f, &font, &is_revealed);
            f.layout_job(job)
        })
    };

    let mut scroll = if wrap {
        egui::ScrollArea::vertical()
    } else {
        egui::ScrollArea::both()
    };
    if code_mode {
        // The minimap shows where you are; the wheel, Shift+wheel and the
        // caret scroll.
        scroll = scroll.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
    }
    if let Some(offset) = st.scroll_to.take() {
        scroll = scroll.scroll_offset(offset);
    }
    let mut text_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(text_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let output = scroll
        .id_salt(("note_scroll", active))
        .auto_shrink([false, false])
        .show(&mut text_ui, |ui| {
            // egui draws the cursor as tall as the whole (spaced) row; we draw our own below.
            ui.visuals_mut().text_cursor.stroke = Stroke::NONE;
            // Room for what goes under the text (current line, guides,
            // matches, spelling).
            let below = ui.painter().add(Shape::Noop);
            let output = egui::TextEdit::multiline(&mut note.body)
                .id(id)
                .frame(false)
                .lock_focus(true)
                .desired_width(if wrap { ui.available_width() } else { f32::INFINITY })
                .min_size(ui.available_size())
                .layouter(&mut layouter)
                .show(ui);
            (below, output)
        });
    let viewport = output.inner_rect;
    let scroll_state = (output.state.offset, output.content_size);
    st.offset = output.state.offset;
    let (below, output) = output.inner;
    let focused = output.response.has_focus() && ctx.input(|i| i.focused);
    if let Some(range) = kept_selection {
        let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
        state.cursor.set_char_range(Some(range));
        state.store(&ctx, id);
    }
    let cursor_char = output.cursor_range.map(|r| r.primary.ccursor.index);

    // A secret is one block: the caret never stops inside a masked one.
    let items = secrets::items(&note.body, &is_revealed);
    if let Some(range) = output
        .cursor_range
        .filter(|_| !items.is_empty() && kept_selection.is_none())
    {
        let (p, q) = (range.primary.ccursor.index, range.secondary.ccursor.index);
        if let Some((p, q)) = secrets::fix_cursor(&items, p, q, st.caret.0 .0) {
            let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
            state
                .cursor
                .set_char_range(Some(CCursorRange::two(CCursor::new(q), CCursor::new(p))));
            state.store(&ctx, id);
            ctx.request_repaint();
        }
    }

    // What the paste detector did, now that the text is in: offer Undo.
    if let Some(p) = st.paste.take() {
        if let Some(end) = cursor_char.filter(|_| output.response.changed()) {
            let n = p.inserted.chars().count();
            let start = end.saturating_sub(n);
            let (a, b) = (char_to_byte(&note.body, start), char_to_byte(&note.body, end));
            if end >= n && note.body.get(a..b) == Some(p.inserted.as_str()) {
                let (text, replacement) = match p.kind {
                    Pasted::Masked => (
                        "Pasted text looks like a secret, so it was masked.".to_owned(),
                        p.original.trim().to_owned(),
                    ),
                    Pasted::Secrets(n) => (
                        if n == 1 {
                            "1 pasted secret was masked.".to_owned()
                        } else {
                            format!("{n} pasted secrets were masked.")
                        },
                        secret::escape_all(&p.inserted),
                    ),
                };
                let explain = p.kind == Pasted::Masked && settings.secret_tip;
                secret_tip_seen |= explain;
                st.toast = Some(Toast {
                    text,
                    undo: Some(Undo {
                        note: active,
                        chars: (start, end),
                        inserted: p.inserted.clone(),
                        replacement,
                    }),
                    explain,
                    left: if explain {
                        3.0 * TOAST_SECONDS
                    } else {
                        TOAST_SECONDS
                    },
                });
            }
        }
    }

    let spans = if code_mode {
        st.highlight.spans.clone()
    } else {
        Arc::default()
    };
    let view = CodeView {
        text: &note.body,
        spans: &spans,
        galley: &output.galley,
        origin: output.galley_pos,
        viewport,
        char_w,
        glyph_h,
        tab_cols,
        cursor: output
            .cursor_range
            .filter(|r| r.is_empty())
            .map(|r| (r.primary.pcursor.paragraph, r.primary.ccursor.index)),
    };
    let lines = if numbers { view.lines() } else { Vec::new() };
    let rows = RowIndex::new(&output.galley);
    let screen = secrets::Screen {
        galley: &output.galley,
        origin: output.galley_pos,
        viewport,
        glyph_h,
        font: &font,
    };
    let mut shapes = Vec::new();
    if code_mode {
        shapes.extend(view.current_line(&lines, viewport.x_range()));
        shapes.extend(view.indent_guides(&lines));
        if focused {
            if let Some((_, at)) = view.cursor {
                let key = (st.highlight.key.unwrap_or(0), at);
                let pair = match st.brackets {
                    Some((k, pair)) if k == key => pair,
                    _ => {
                        let pair = matching_bracket(view.text, view.spans, at);
                        st.brackets = Some((key, pair));
                        pair
                    }
                };
                shapes.extend(view.bracket_boxes(pair));
            }
        }
    }

    shapes.extend(screen.backgrounds(&items));

    // Find: every match, the current one brighter.
    if let Some(find) = &st.find {
        let current = find.current;
        for (i, m) in find.matches.iter().enumerate() {
            if !view.on_screen(&rows, m.chars.0) {
                continue;
            }
            let fill = if Some(i) == current { MATCH_CURRENT } else { MATCH };
            for r in view.range_rects(&rows, m.chars) {
                shapes.push(Shape::rect_filled(r.expand2(vec2(1.0, 1.0)), 2.0, fill));
            }
        }
    }

    // Spelling: a wavy line under each unknown word, except the one being
    // typed.
    let mut misses = Arc::default();
    if spell_on {
        misses = st
            .spell
            .misspelled(&ctx, settings.spell_language, &personal, view.text);
        let typing = cursor_char.filter(|_| focused);
        for m in misses.iter() {
            if typing.is_some_and(|c| m.chars.0 <= c && c <= m.chars.1) || !view.on_screen(&rows, m.chars.0) {
                continue;
            }
            for r in view.range_rects(&rows, m.chars) {
                shapes.push(squiggle(r.left(), r.right(), r.top() + glyph_h + 0.5));
            }
        }
    }

    text_ui
        .painter()
        .with_clip_rect(viewport)
        .set(below, Shape::Vec(shapes));

    // Reveal and Copy on each secret. Paranoid reveals only while held.
    let hold = security.profile == Profile::Paranoid;
    let popped: Vec<usize> = if all_shown { Vec::new() } else { popped };
    screen.bubbles(ui, &note.body, &items, &popped);
    for (i, event) in screen.icons(ui, id, &items, &popped, hold) {
        // The click took focus from the text; give it back.
        st.focus_editor = true;
        match event {
            IconEvent::Reveal if hold => {
                st.reveals.show(i, now + 0.2);
                ctx.request_repaint();
            }
            IconEvent::Reveal => st.reveals.toggle(i, now + secrets::REVEAL_SECONDS),
            IconEvent::Copy => copy_secret(&ctx, &note.body, &items[i], &security),
            IconEvent::Menu(pos) => {
                st.spell_menu = None;
                st.secret_menu = Some(SecretMenu {
                    pos,
                    note: active,
                    target: Some(i),
                    has_selection: false,
                    can_paste: false,
                });
            }
        }
    }
    if let Some(gutter) = gutter {
        view.gutter(ui, gutter, &lines, &font, code_mode);
    }
    if let Some(minimap) = minimap {
        if let Some(y) = view.minimap(ui, minimap, &lines, scroll_state, id) {
            st.scroll_to = Some(vec2(st.offset.x, y));
            ctx.memory_mut(|m| m.request_focus(id));
            ctx.request_repaint();
        }
    }

    // Scroll a revealed match into view, a third of the way down.
    if let Some((a, b)) = st.reveal.take() {
        let start = output.galley.pos_from_ccursor(CCursor {
            index: a,
            prefer_next_row: true,
        });
        let end = output.galley.pos_from_ccursor(CCursor::new(b));
        let to_content = output.galley_pos - viewport.min + st.offset;
        let (top, bottom) = (
            start.top() + to_content.y,
            end.bottom().max(start.bottom()) + to_content.y,
        );
        let left = start.left() + to_content.x;
        let mut target = st.offset;
        if top < st.offset.y || bottom > st.offset.y + viewport.height() {
            target.y = (top - viewport.height() / 3.0).max(0.0);
        }
        if !wrap && (left < st.offset.x || left > st.offset.x + viewport.width() - 40.0) {
            target.x = (left - viewport.width() / 3.0).max(0.0);
        }
        if target != st.offset {
            st.scroll_to = Some(target);
            ctx.request_repaint();
        }
    }

    // Right-click: a secret's menu; else a misspelled word's suggestions;
    // else the general menu (Cut, Copy, Paste, Select all, and Mark as
    // secret / Reveal all when they apply).
    if output.response.secondary_clicked() {
        st.spell_menu = None;
        st.secret_menu = None;
        // The click ends on release, a frame after `kept_selection` put the
        // selection back, so read it from the text edit now.
        let has_selection = kept_selection.is_some()
            || egui::TextEdit::load_state(&ctx, id)
                .and_then(|s| s.cursor.char_range())
                .is_some_and(|r| r.primary != r.secondary);
        if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
            let at = output
                .galley
                .cursor_from_pos(pos - output.galley_pos)
                .ccursor
                .index;
            let byte = char_to_byte(&note.body, at);
            if let Some(i) = screen.at(&items, pos) {
                st.secret_menu = Some(SecretMenu {
                    pos,
                    note: active,
                    target: Some(i),
                    has_selection: false,
                    can_paste: false,
                });
            } else if let Some(m) = misses
                .iter()
                .filter(|_| spell_on && !has_selection)
                .find(|m| m.bytes.start <= byte && byte <= m.bytes.end)
            {
                let word = note.body[m.bytes.clone()].to_owned();
                let suggestions = st.spell.suggest(&ctx, settings.spell_language, &word);
                st.spell_menu = Some(SpellMenu {
                    pos,
                    note: active,
                    bytes: m.bytes.clone(),
                    word,
                    suggestions,
                });
            } else {
                st.secret_menu = Some(SecretMenu {
                    pos,
                    note: active,
                    target: None,
                    has_selection,
                    can_paste: crate::support::clipboard_text().is_some(),
                });
            }
        }
    }

    if let Some(range) = output.cursor_range {
        let now = ctx.input(|i| i.time);
        let key = (range.primary.ccursor.index, range.secondary.ccursor.index);
        if st.caret.0 != key || output.response.changed() {
            st.caret = (key, now);
        }
        if focused {
            // Glyphs sit at the top of each spaced row, so the caret spans
            // only the font's own height from there.
            let row = output
                .galley
                .pos_from_cursor(&range.primary)
                .translate(output.galley_pos.to_vec2());
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
    if secret_tip_seen {
        s.vault.content.editor.secret_tip = false;
        s.touch(None);
    }
}

/// The tab's tooltip: the whole title, secrets masked.
fn title_hover(note: &Note) -> String {
    secret::mask(note.display_title()).chars().take(80).collect()
}

/// Sorted char range of a text edit's selection, or of its caret.
fn char_selection(ctx: &egui::Context, id: egui::Id, fallback: usize) -> (usize, usize) {
    egui::TextEdit::load_state(ctx, id)
        .and_then(|s| s.cursor.char_range())
        .map(|r| {
            let (a, b) = (r.primary.index, r.secondary.index);
            (a.min(b), a.max(b))
        })
        .unwrap_or((fallback, fallback))
}

fn set_caret(ctx: &egui::Context, id: egui::Id, at: usize) {
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(at))));
    state.store(ctx, id);
}

/// Ctrl+Shift+H: the selection becomes a secret. With nothing selected, a
/// dialog asks for a new one. Returns whether the text changed.
fn mark_secret(ctx: &egui::Context, id: egui::Id, note: &mut Note, st: &mut EditorState) -> bool {
    let (a, b) = char_selection(ctx, id, note.cursor as usize);
    if a == b {
        st.secret_edit = Some(SecretEdit {
            note: note.id,
            target: None,
            at: a,
            value: SecretField::default(),
            loaded: false,
            error: None,
        });
        return false;
    }
    let (start, end) = (char_to_byte(&note.body, a), char_to_byte(&note.body, b));
    let selected = &note.body[start..end];
    if selected.contains(secret::OPEN) {
        st.toast = Some(Toast::message("The selection already has a secret in it."));
        return false;
    }
    let Some(wrapped) = secret::wrap(selected) else {
        st.toast = Some(Toast::message(
            "A secret has to be one line and can't contain *}.",
        ));
        return false;
    };
    let at = a + wrapped.chars().count();
    note.body.replace_range(start..end, &wrapped);
    set_caret(ctx, id, at);
    true
}

/// Backspace after a secret or Delete before it removes all of it, so its
/// value never turns into plain text (DESIGN §11.1).
fn delete_secret(ctx: &egui::Context, id: egui::Id, note: &mut Note, items: &[secrets::Item]) -> bool {
    let (a, b) = char_selection(ctx, id, note.cursor as usize);
    if a != b {
        return false;
    }
    // Only the key event is touched inside `input_mut`: egui's context is
    // locked there, and `set_caret` needs it too.
    let delete = ctx.input_mut(|i| {
        let pos = i.events.iter().position(|e| {
            matches!(
                e,
                egui::Event::Key {
                    key: Key::Backspace | Key::Delete,
                    pressed: true,
                    ..
                }
            )
        })?;
        let back = matches!(
            i.events[pos],
            egui::Event::Key {
                key: Key::Backspace,
                ..
            }
        );
        let what = secrets::deletion(items, a, back)?;
        i.events.remove(pos);
        what
    });
    let Some((s, e)) = delete else {
        return false;
    };
    let range = char_to_byte(&note.body, s)..char_to_byte(&note.body, e);
    note.body.replace_range(range, "");
    set_caret(ctx, id, s);
    true
}

/// The value only, with the secret clipboard timer (DESIGN §11.2).
fn copy_secret(ctx: &egui::Context, text: &str, item: &secrets::Item, security: &SecuritySettings) {
    let value = &text[secret::value(&item.bytes)];
    crate::support::copy_with_timer(
        ctx,
        value,
        security.secret_clear_s.min(security.clipboard_clear_s),
    );
}

const MATCH: egui::Color32 = egui::Color32::from_rgb(0x33, 0x33, 0x33);
const MATCH_CURRENT: egui::Color32 = egui::Color32::from_rgb(0x50, 0x50, 0x50);

/// The wavy line under a misspelled word.
fn squiggle(x0: f32, x1: f32, y: f32) -> Shape {
    let mut points = Vec::new();
    let mut x = x0;
    let mut up = false;
    while x < x1 {
        points.push(pos2(x, if up { y - 1.0 } else { y + 1.0 }));
        x += 2.0;
        up = !up;
    }
    points.push(pos2(x1, if up { y - 1.0 } else { y + 1.0 }));
    Shape::line(points, Stroke::new(1.0_f32, DANGER))
}

/// Byte offset of char index `c` in `text` (its end if past it).
fn char_to_byte(text: &str, c: usize) -> usize {
    text.char_indices().nth(c).map_or(text.len(), |(i, _)| i)
}

/// First char index of each galley row, to find the row of a char fast.
struct RowIndex {
    starts: Vec<usize>,
}

impl RowIndex {
    fn new(galley: &egui::Galley) -> Self {
        let mut starts = Vec::with_capacity(galley.rows.len());
        let mut c = 0;
        for row in &galley.rows {
            starts.push(c);
            c += row.char_count_including_newline();
        }
        Self { starts }
    }

    fn row(&self, c: usize) -> usize {
        self.starts.partition_point(|s| *s <= c).saturating_sub(1)
    }
}

/// Code mode needs a monospace font: Inter and Segoe UI fall back to
/// JetBrains Mono.
fn code_family(ctx: &egui::Context, font: Font) -> egui::FontFamily {
    match font {
        Font::Consolas => crate::theme::editor_family(ctx, font),
        _ => egui::FontFamily::Monospace,
    }
}

fn code_job(spans: &[code::Span], text: &str, font: &FontId, line_h: f32, wrap_width: f32) -> LayoutJob {
    let format = |kind: Kind| egui::TextFormat {
        font_id: font.clone(),
        color: kind.color(),
        line_height: Some(line_h),
        italics: kind == Kind::Comment,
        ..Default::default()
    };
    let mut job = LayoutJob {
        text: text.to_owned(),
        ..Default::default()
    };
    job.wrap.max_width = wrap_width;
    let mut at = 0;
    for span in spans.iter().filter(|s| s.range.end <= text.len()) {
        if span.range.start > at {
            job.append_section(at..span.range.start, format(Kind::Plain));
        }
        job.append_section(span.range.clone(), format(span.kind));
        at = span.range.end;
    }
    if at < text.len() || job.sections.is_empty() {
        job.append_section(at..text.len(), format(Kind::Plain));
    }
    job
}

trait AppendSection {
    fn append_section(&mut self, range: std::ops::Range<usize>, format: egui::TextFormat);
}

impl AppendSection for LayoutJob {
    fn append_section(&mut self, range: std::ops::Range<usize>, format: egui::TextFormat) {
        self.sections.push(egui::text::LayoutSection {
            leading_space: 0.0,
            byte_range: range,
            format,
        });
    }
}

/// One line of the note as laid out: its rows in the galley and its indent.
struct Line {
    first_row: usize,
    last_row: usize,
    /// Indent levels; for a blank line, those of the lines around it.
    level: usize,
    blank: bool,
}

/// What code mode draws around the text, from this frame's layout.
struct CodeView<'a> {
    text: &'a str,
    spans: &'a [code::Span],
    galley: &'a egui::Galley,
    /// Screen position of the galley (already scrolled).
    origin: egui::Pos2,
    viewport: Rect,
    char_w: f32,
    glyph_h: f32,
    tab_cols: usize,
    /// Line and char index of the cursor, when nothing is selected.
    cursor: Option<(usize, usize)>,
}

impl CodeView<'_> {
    fn lines(&self) -> Vec<Line> {
        let mut lines = Vec::new();
        let mut first_row = 0;
        for (r, row) in self.galley.rows.iter().enumerate() {
            if row.ends_with_newline || r + 1 == self.galley.rows.len() {
                lines.push(Line {
                    first_row,
                    last_row: r,
                    level: 0,
                    blank: true,
                });
                first_row = r + 1;
            }
        }
        for (line, text) in lines.iter_mut().zip(self.text.split('\n')) {
            let mut cols = 0;
            for c in text.chars() {
                match c {
                    ' ' => cols += 1,
                    '\t' => cols += self.tab_cols - cols % self.tab_cols,
                    _ => {
                        line.blank = false;
                        break;
                    }
                }
            }
            line.level = cols.div_ceil(self.tab_cols);
        }
        // A blank line inside a block keeps the block's guides.
        let mut prev = 0;
        let mut next = vec![0; lines.len()];
        let mut after = 0;
        for i in (0..lines.len()).rev() {
            next[i] = after;
            if !lines[i].blank {
                after = lines[i].level;
            }
        }
        for (i, line) in lines.iter_mut().enumerate() {
            if line.blank {
                line.level = prev.min(next[i]);
            } else {
                prev = line.level;
            }
        }
        lines
    }

    fn row_y(&self, row: usize) -> (f32, f32) {
        let r = self.galley.rows[row].rect;
        (self.origin.y + r.top(), self.origin.y + r.bottom())
    }

    fn visible(&self, line: &Line) -> bool {
        let (top, _) = self.row_y(line.first_row);
        let (_, bottom) = self.row_y(line.last_row);
        bottom >= self.viewport.top() && top <= self.viewport.bottom()
    }

    fn current_line(&self, lines: &[Line], x: egui::Rangef) -> Vec<Shape> {
        let Some(line) = self.cursor.and_then(|(l, _)| lines.get(l)) else {
            return Vec::new();
        };
        let (top, _) = self.row_y(line.first_row);
        let (_, bottom) = self.row_y(line.last_row);
        vec![Shape::rect_filled(
            Rect::from_x_y_ranges(x, top..=bottom),
            0.0,
            CURRENT_LINE,
        )]
    }

    /// The guide of the block the cursor is in, as (level, lines).
    fn active_guide(&self, lines: &[Line]) -> Option<(usize, std::ops::Range<usize>)> {
        let (cur, _) = self.cursor?;
        let level = lines.get(cur)?.level;
        let next = lines[cur + 1..].iter().find(|l| !l.blank).map(|l| l.level);
        if next.is_some_and(|n| n > level) {
            // The cursor line opens a block: its body.
            let end = lines[cur + 1..]
                .iter()
                .position(|l| !l.blank && l.level <= level)
                .map_or(lines.len(), |p| cur + 1 + p);
            return Some((level, cur + 1..end));
        }
        if level == 0 {
            return None;
        }
        let inside = |l: &Line| l.blank || l.level >= level;
        let start = lines[..cur].iter().rposition(|l| !inside(l)).map_or(0, |p| p + 1);
        let end = lines[cur..]
            .iter()
            .position(|l| !inside(l))
            .map_or(lines.len(), |p| cur + p);
        Some((level - 1, start..end))
    }

    fn indent_guides(&self, lines: &[Line]) -> Vec<Shape> {
        let active = self.active_guide(lines);
        let step = self.tab_cols as f32 * self.char_w;
        let mut shapes = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            if line.level == 0 || !self.visible(line) {
                continue;
            }
            let (top, _) = self.row_y(line.first_row);
            let (_, bottom) = self.row_y(line.last_row);
            for k in 0..line.level {
                let x = (self.origin.x + k as f32 * step).round() + 0.5;
                let is_active = active
                    .as_ref()
                    .is_some_and(|(lvl, r)| *lvl == k && r.contains(&i));
                shapes.push(Shape::line_segment(
                    [pos2(x, top), pos2(x, bottom)],
                    Stroke::new(1.0_f32, if is_active { GUIDE_ACTIVE } else { GUIDE }),
                ));
            }
        }
        shapes
    }

    fn bracket_boxes(&self, pair: Option<(usize, usize)>) -> Vec<Shape> {
        let Some((a, b)) = pair else {
            return Vec::new();
        };
        [a, b]
            .into_iter()
            .map(|c| {
                let at = self.galley.pos_from_ccursor(CCursor::new(c));
                Shape::rect_stroke(
                    Rect::from_min_size(at.min + self.origin.to_vec2(), vec2(self.char_w, self.glyph_h)),
                    CornerRadius::same(2),
                    Stroke::new(1.0_f32, BRACKET),
                    egui::StrokeKind::Inside,
                )
            })
            .collect()
    }

    /// Whether the galley row holding char `c` is on screen.
    fn on_screen(&self, rows: &RowIndex, c: usize) -> bool {
        let (top, bottom) = self.row_y(rows.row(c).min(self.galley.rows.len() - 1));
        bottom >= self.viewport.top() && top <= self.viewport.bottom()
    }

    /// Screen rects covering chars `a..b`, one per galley row, glyph high.
    fn range_rects(&self, rows: &RowIndex, (a, b): (usize, usize)) -> Vec<Rect> {
        let last = self.galley.rows.len() - 1;
        let (first_row, last_row) = (
            rows.row(a).min(last),
            rows.row(b.saturating_sub(1).max(a)).min(last),
        );
        let mut rects = Vec::new();
        for r in first_row..=last_row {
            let row_start = rows.starts[r];
            let row_end = row_start + self.galley.rows[r].char_count_excluding_newline();
            let (from, to) = (a.max(row_start), b.min(row_end));
            if to <= from {
                continue;
            }
            let x0 = self.galley.rows[r].x_offset(from - row_start);
            let x1 = self.galley.rows[r].x_offset(to - row_start);
            let (top, _) = self.row_y(r);
            rects.push(Rect::from_min_max(
                pos2(self.origin.x + x0, top),
                pos2(self.origin.x + x1, top + self.glyph_h),
            ));
        }
        rects
    }

    /// Line numbers, fixed on the left while the text scrolls sideways.
    /// `band` marks the cursor's line across the gutter too (code mode).
    fn gutter(&self, ui: &Ui, rect: Rect, lines: &[Line], font: &FontId, band: bool) {
        let clip = Rect::from_x_y_ranges(rect.x_range(), self.viewport.y_range());
        let painter = ui.painter_at(clip);
        let current = self.cursor.map(|(l, _)| l);
        if let Some(line) = current.and_then(|l| lines.get(l)).filter(|_| band) {
            let (top, _) = self.row_y(line.first_row);
            let (_, bottom) = self.row_y(line.last_row);
            painter.rect_filled(
                Rect::from_x_y_ranges(rect.x_range(), top..=bottom),
                0.0,
                CURRENT_LINE,
            );
        }
        for (i, line) in lines.iter().enumerate() {
            let (top, bottom) = self.row_y(line.first_row);
            if bottom < clip.top() || top > clip.bottom() {
                continue;
            }
            painter.text(
                pos2(rect.right() - 22.0, top),
                Align2::RIGHT_TOP,
                (i + 1).to_string(),
                font.clone(),
                if current == Some(i) { TEXT_BODY } else { GUTTER },
            );
        }
    }

    /// The whole note in miniature; click or drag to scroll. Returns the
    /// new vertical scroll offset.
    fn minimap(
        &self,
        ui: &Ui,
        rect: Rect,
        lines: &[Line],
        (offset, content): (egui::Vec2, egui::Vec2),
        id: egui::Id,
    ) -> Option<f32> {
        let painter = ui.painter_at(rect);
        painter.vline(
            rect.left(),
            rect.y_range(),
            Stroke::new(1.0_f32, egui::Color32::from_gray(0x23)),
        );
        let area = Rect::from_min_max(rect.min + vec2(10.0, 8.0), rect.max - vec2(8.0, 0.0));
        let view_h = self.viewport.height();
        let max_scroll = (content.y - view_h).max(0.0);
        let frac = if max_scroll > 0.0 {
            offset.y / max_scroll
        } else {
            0.0
        };
        let total = lines.len() as f32 * MINIMAP_LINE;
        let overflow = (total - area.height()).max(0.0);
        let shift = frac * overflow;

        // The part of the note on screen.
        if content.y > 0.0 {
            let top = area.top() + offset.y / content.y * total - shift;
            let h = (view_h / content.y * total).max(MINIMAP_LINE);
            painter.rect_filled(
                Rect::from_x_y_ranges(rect.x_range(), top..=top + h),
                0.0,
                egui::Color32::from_rgba_premultiplied(15, 15, 15, 15),
            );
        }

        let first = (shift / MINIMAP_LINE) as usize;
        let count = (area.height() / MINIMAP_LINE).ceil() as usize + 1;
        let max_cols = (area.width() / MINIMAP_CHAR) as usize;
        // Secrets are drawn six columns wide, like their dots.
        let hidden = secret::scan(self.text).secrets;
        let mut h = 0;
        let mut byte = 0;
        for (i, text) in self.text.split('\n').enumerate() {
            let start = byte;
            byte += text.len() + 1;
            if i < first {
                continue;
            }
            if i > first + count {
                break;
            }
            let y = area.top() + i as f32 * MINIMAP_LINE - shift;
            let mut col = 0usize;
            let mut run: Option<(usize, usize, Kind)> = None; // (start col, len, kind)
            let flush = |run: Option<(usize, usize, Kind)>| {
                if let Some((c, n, kind)) = run {
                    let x = area.left() + c as f32 * MINIMAP_CHAR;
                    painter.rect_filled(
                        Rect::from_min_size(pos2(x, y), vec2(n as f32 * MINIMAP_CHAR, 2.0)),
                        0.0,
                        kind.color().gamma_multiply(0.7),
                    );
                }
            };
            for (off, c) in text.char_indices() {
                if col >= max_cols {
                    break;
                }
                let abs = start + off;
                while hidden.get(h).is_some_and(|r| r.end <= abs) {
                    h += 1;
                }
                if let Some(r) = hidden.get(h).filter(|r| r.start <= abs) {
                    if abs == r.start {
                        flush(run.take());
                        flush(Some((col, 6.min(max_cols - col), Kind::Plain)));
                        col += 6;
                    }
                    continue;
                }
                if c.is_whitespace() {
                    flush(run.take());
                    col += if c == '\t' {
                        self.tab_cols - col % self.tab_cols
                    } else {
                        1
                    };
                    continue;
                }
                let kind = code::kind_at(self.spans, start + off);
                run = match run {
                    Some((c0, n, k)) if k == kind => Some((c0, n + 1, k)),
                    other => {
                        flush(other);
                        Some((col, 1, kind))
                    }
                };
                col += 1;
            }
            flush(run);
        }

        let resp = ui.interact(rect, id.with("minimap"), Sense::click_and_drag());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Default);
        }
        let pointer = resp
            .interact_pointer_pos()
            .filter(|_| resp.is_pointer_button_down_on())?;
        let y = if overflow > 0.0 {
            // A long note: the minimap's height stands for the whole note.
            (pointer.y - area.top()) / area.height() * max_scroll
        } else {
            (pointer.y - area.top()) / total * content.y - view_h / 2.0
        };
        Some(y.clamp(0.0, max_scroll))
    }
}

/// The bracket next to the cursor and its partner, as char indices.
/// Brackets in strings and comments don't count.
fn matching_bracket(text: &str, spans: &[code::Span], cursor: usize) -> Option<(usize, usize)> {
    const LIMIT: usize = 200_000;
    let b = text.as_bytes();
    let byte_at = |c: usize| text.char_indices().nth(c).map(|(i, _)| i);
    let counts = |i: usize| !matches!(code::kind_at(spans, i), Kind::String | Kind::Comment);
    let before = cursor.checked_sub(1).and_then(byte_at);
    let at = byte_at(cursor);
    let (pos, open, close, forward) = [before, at].into_iter().flatten().find_map(|i| {
        let pair = match b[i] {
            b'(' => (b'(', b')', true),
            b'[' => (b'[', b']', true),
            b'{' => (b'{', b'}', true),
            b')' => (b'(', b')', false),
            b']' => (b'[', b']', false),
            b'}' => (b'{', b'}', false),
            _ => return None,
        };
        counts(i).then_some((i, pair.0, pair.1, pair.2))
    })?;
    let mut depth = 0i32;
    let partner = if forward {
        (pos..b.len().min(pos + LIMIT)).find(|&i| {
            if b[i] == open && counts(i) {
                depth += 1;
            } else if b[i] == close && counts(i) {
                depth -= 1;
            }
            depth == 0
        })
    } else {
        (pos.saturating_sub(LIMIT)..=pos).rev().find(|&i| {
            if b[i] == close && counts(i) {
                depth += 1;
            } else if b[i] == open && counts(i) {
                depth -= 1;
            }
            depth == 0
        })
    }?;
    let char_index = |i: usize| text[..i].chars().count();
    Some((char_index(pos), char_index(partner)))
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

fn status_bar(ctx: &egui::Context, s: &Session, st: &mut EditorState, last_input: Instant) {
    let show = s.vault.content.editor.status_bar;
    let notice = s.current_notice();
    if !show && s.save_error.is_none() && notice.is_none() {
        return;
    }
    let note = s
        .vault
        .content
        .active_note
        .and_then(|id| s.vault.content.note(id));
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
                } else if let Some((text, error)) = notice {
                    widgets::label(ui, text, 12.0, if error { DANGER } else { TEXT_BODY });
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
                    if let Some(note) = note {
                        let resp = language_button(ui, note, st.picker.is_some());
                        st.language_button = Some(resp.rect);
                        if resp.clicked() {
                            st.picker = match st.picker {
                                Some(_) => None,
                                None => Some(Picker::default()),
                            };
                            if st.picker.is_none() {
                                st.focus_editor = true;
                            }
                        }
                        if note.code.is_some() {
                            small(
                                ui,
                                &match s.vault.content.editor.tab_inserts {
                                    TabInsert::Spaces(n) => format!("Spaces: {n}"),
                                    TabInsert::Tab => "Tab size: 4".to_owned(),
                                },
                            );
                        }
                    }
                    let (l, c) = st.line_col;
                    small(ui, &format!("Ln {l}, Col {c}"));
                });
            });
        });
}

/// "Rust" / "Plain text" in the status bar; opens the language picker.
fn language_button(ui: &mut Ui, note: &Note, open: bool) -> egui::Response {
    let code_mode = note.code.is_some();
    let text = code::status_label(note);
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        crate::theme::ui(12.0),
        egui::Color32::PLACEHOLDER,
    );
    let icon_w = if code_mode { 17.0 } else { 0.0 };
    // The 8 px padding sits in the 20 px gap, like the mockup's -8 px margin.
    ui.add_space(-8.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 16.0 + icon_w, 22.0), Sense::click());
    ui.add_space(-8.0);
    let hot = open || resp.hovered();
    let painter = ui.painter();
    if hot {
        painter.rect_filled(rect, CornerRadius::same(4), egui::Color32::from_gray(0x26));
    }
    let color = if hot {
        TEXT
    } else if code_mode {
        TEXT_SECONDARY
    } else {
        TEXT_MUTED
    };
    if code_mode {
        widgets::icon_code(painter, pos2(rect.left() + 13.5, rect.center().y), 11.0, color);
    }
    painter.galley(
        pos2(
            rect.left() + 8.0 + icon_w,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        color,
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp.on_hover_text("Language (Ctrl+Shift+M turns code mode on and off)")
}

/// What the language picker can set.
#[derive(Clone, Copy, PartialEq)]
enum Choice {
    Plain,
    Auto,
    Language(&'static code::Language),
}

/// The searchable language list above the status bar (Code mode ·
/// language picker mockup).
fn language_picker(ctx: &egui::Context, s: &mut Session, st: &mut EditorState) {
    let Some(picker) = st.picker.as_mut() else {
        return;
    };
    let Some(note) = s
        .vault
        .content
        .active_note
        .and_then(|id| s.vault.content.note(id))
    else {
        st.picker = None;
        return;
    };
    let current = match &note.code {
        None => Choice::Plain,
        Some(c) if c.language.is_empty() => Choice::Auto,
        Some(c) => code::by_id(&c.language).map_or(Choice::Auto, Choice::Language),
    };
    let query = picker.query.trim().to_lowercase();
    let languages: Vec<&'static code::Language> = code::LANGUAGES
        .iter()
        .filter(|l| query.is_empty() || l.matches(&query))
        .collect();

    let mut chosen = None;
    let mut close = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    let Some(anchor) = st.language_button else {
        return;
    };
    let area = egui::Area::new(egui::Id::new("language_picker"))
        .order(egui::Order::Foreground)
        .pivot(Align2::RIGHT_BOTTOM)
        .fixed_pos(
            pos2(anchor.right() + 96.0 - 8.0, anchor.top() - 6.0)
                .min(pos2(ctx.screen_rect().right() - 8.0, f32::INFINITY)),
        )
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(POPUP)
                .stroke(Stroke::new(1.0_f32, POPUP_BORDER))
                .corner_radius(CornerRadius::same(8))
                .shadow(egui::Shadow {
                    offset: [0, 8],
                    blur: 24,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(100),
                })
                .show(ui, |ui| {
                    ui.set_width(288.0);
                    ui.spacing_mut().item_spacing.y = 0.0;

                    // Search field.
                    let (field, _) = ui.allocate_exact_size(vec2(288.0, 54.0), Sense::hover());
                    let boxr = field.shrink(10.0);
                    let field_id = egui::Id::new("language_query");
                    ui.painter().rect(
                        boxr,
                        CornerRadius::same(6),
                        SURFACE,
                        Stroke::new(1.0_f32, BORDER_FOCUS),
                        egui::StrokeKind::Inside,
                    );
                    widgets::icon_search(
                        ui.painter(),
                        pos2(boxr.left() + 16.0, boxr.center().y),
                        TEXT_MUTED,
                    );
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(Rect::from_min_max(
                                boxr.min + vec2(30.0, 0.0),
                                boxr.max - vec2(8.0, 0.0),
                            ))
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    let resp = child.add(
                        egui::TextEdit::singleline(&mut picker.query)
                            .id(field_id)
                            .frame(false)
                            .font(crate::theme::ui(13.0))
                            .text_color(TEXT)
                            .hint_text(egui::RichText::new("Search languages").color(TEXT_FAINT))
                            .desired_width(boxr.width() - 40.0),
                    );
                    if !std::mem::replace(&mut picker.focused, true) {
                        resp.request_focus();
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        match languages.first() {
                            Some(l) if !query.is_empty() => chosen = Some(Choice::Language(l)),
                            _ => close = true,
                        }
                    }
                    divider(ui);

                    ui.add_space(6.0);
                    if picker_row(
                        ui,
                        "Plain text",
                        Some("Leaves code mode"),
                        current == Choice::Plain,
                    )
                    .clicked()
                    {
                        chosen = Some(Choice::Plain);
                    }
                    if picker_row(
                        ui,
                        "Auto-detect",
                        Some("From the tab name"),
                        current == Choice::Auto,
                    )
                    .clicked()
                    {
                        chosen = Some(Choice::Auto);
                    }
                    ui.add_space(6.0);
                    divider(ui);
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical()
                        .max_height(238.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            if languages.is_empty() {
                                let (r, _) = ui.allocate_exact_size(vec2(288.0, 34.0), Sense::hover());
                                ui.painter().text(
                                    pos2(r.left() + 16.0, r.center().y),
                                    Align2::LEFT_CENTER,
                                    "No language matches",
                                    crate::theme::ui(13.0),
                                    TEXT_FAINT,
                                );
                            }
                            for l in &languages {
                                let selected = current == Choice::Language(l);
                                if picker_row(ui, l.name, None, selected).clicked() {
                                    chosen = Some(Choice::Language(l));
                                }
                            }
                        });
                    ui.add_space(6.0);
                });
        });
    let pointer = ctx.input(|i| i.pointer.interact_pos());
    if area.response.clicked_elsewhere() && !pointer.is_some_and(|p| anchor.contains(p)) {
        close = true;
    }

    if let Some(choice) = chosen {
        if let Some(note) = s
            .vault
            .content
            .active_note
            .and_then(|id| s.vault.content.note_mut(id))
        {
            let was_code = note.code.is_some();
            note.code = match choice {
                Choice::Plain => None,
                Choice::Auto => Some(CodeMode {
                    language: String::new(),
                }),
                Choice::Language(l) => Some(CodeMode {
                    language: l.id.to_owned(),
                }),
            };
            if was_code != note.code.is_some() {
                note.wrap = None;
            }
            s.touch(None);
        }
        close = true;
    }
    if close {
        st.picker = None;
        st.focus_editor = true;
    }
}

fn divider(ui: &mut Ui) {
    menu_divider(ui, 288.0);
}

fn menu_divider(ui: &mut Ui, width: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
    ui.painter()
        .hline(r.x_range(), r.center().y, Stroke::new(1.0_f32, BORDER));
}

fn picker_row(ui: &mut Ui, text: &str, hint: Option<&str>, selected: bool) -> egui::Response {
    menu_row(ui, 288.0, text, hint, selected)
}

/// The frame of the editor's popups (language picker, menus, find).
fn popup_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(POPUP)
        .stroke(Stroke::new(1.0_f32, POPUP_BORDER))
        .corner_radius(CornerRadius::same(8))
        .shadow(egui::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: egui::Color32::from_black_alpha(100),
        })
}

/// A one-line text field with a magnifier, as in the language picker.
fn search_field(ui: &mut Ui, text: &mut String, id: egui::Id, hint: &str, width: f32) -> egui::Response {
    let (boxr, _) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
    let focused = ui.memory(|m| m.has_focus(id));
    ui.painter().rect(
        boxr,
        CornerRadius::same(6),
        SURFACE,
        Stroke::new(1.0_f32, if focused { BORDER_FOCUS } else { BORDER }),
        egui::StrokeKind::Inside,
    );
    widgets::icon_search(
        ui.painter(),
        pos2(boxr.left() + 16.0, boxr.center().y),
        TEXT_MUTED,
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(
                boxr.min + vec2(30.0, 0.0),
                boxr.max - vec2(8.0, 0.0),
            ))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.add(
        egui::TextEdit::singleline(text)
            .id(id)
            .frame(false)
            .font(crate::theme::ui(13.0))
            .text_color(TEXT)
            .hint_text(egui::RichText::new(hint).color(TEXT_FAINT))
            .desired_width(boxr.width() - 40.0),
    )
}

/// A line of faint text in a menu ("No suggestions").
fn menu_note(ui: &mut Ui, width: f32, text: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
    ui.painter().text(
        pos2(r.left() + 16.0, r.center().y),
        Align2::LEFT_CENTER,
        text,
        crate::theme::ui(12.0),
        TEXT_FAINT,
    );
}

/// A menu row that can be greyed out (Cut and Copy with nothing selected).
/// A disabled row never reports a click.
fn menu_row_enabled(
    ui: &mut Ui,
    width: f32,
    text: &str,
    hint: Option<&str>,
    enabled: bool,
) -> egui::Response {
    if enabled {
        return menu_row(ui, width, text, hint, false);
    }
    let (outer, resp) = ui.allocate_exact_size(vec2(width, 34.0), Sense::hover());
    let rect = outer.shrink2(vec2(6.0, 0.0));
    let painter = ui.painter();
    painter.text(
        pos2(rect.left() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        crate::theme::ui(13.0),
        TEXT_FAINT,
    );
    if let Some(hint) = hint {
        painter.text(
            pos2(rect.right() - 10.0, rect.center().y),
            Align2::RIGHT_CENTER,
            hint,
            crate::theme::ui(12.0),
            TEXT_FAINT,
        );
    }
    resp
}

fn menu_row(ui: &mut Ui, width: f32, text: &str, hint: Option<&str>, selected: bool) -> egui::Response {
    let (outer, resp) = ui.allocate_exact_size(vec2(width, 34.0), Sense::click());
    let rect = outer.shrink2(vec2(6.0, 0.0));
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, CornerRadius::same(5), BORDER);
    } else if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(5), NAV_ACTIVE);
    }
    painter.text(
        pos2(rect.left() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        crate::theme::ui(13.0),
        if selected || resp.hovered() {
            TEXT
        } else {
            TEXT_BODY
        },
    );
    if let Some(hint) = hint {
        painter.text(
            pos2(rect.right() - 10.0, rect.center().y),
            Align2::RIGHT_CENTER,
            hint,
            crate::theme::ui(12.0),
            TEXT_MUTED,
        );
    } else if selected {
        widgets::icon_check(
            painter,
            pos2(rect.right() - 16.0, rect.center().y),
            12.0,
            TEXT_BODY,
        );
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp
}

/// The selected text of the active note, if it is a short single line,
/// and where a search starts: the selection's start, else the cursor.
fn selection(ctx: &egui::Context, s: &Session) -> (Option<String>, usize) {
    let Some(note) = s
        .vault
        .content
        .active_note
        .and_then(|id| s.vault.content.note(id))
    else {
        return (None, 0);
    };
    let Some((a, b)) = note_selection(ctx, note.id) else {
        return (None, note.cursor as usize);
    };
    if a == b {
        return (None, a);
    }
    let (start, end) = (char_to_byte(&note.body, a), char_to_byte(&note.body, b));
    let text = &note.body[start..end];
    let short = b - a <= 200 && !text.contains('\n');
    // A secret's value must not end up in the find field.
    let secret = secret::scan(&note.body)
        .secrets
        .iter()
        .any(|s| start < s.end && s.start < end);
    ((short && !secret).then(|| text.to_owned()), a)
}

/// A note's selection as sorted char indices (equal when it is just the
/// cursor), if its text edit has one.
fn note_selection(ctx: &egui::Context, note: Uuid) -> Option<(usize, usize)> {
    let range = egui::TextEdit::load_state(ctx, egui::Id::new(("note", note)))?
        .cursor
        .char_range()?;
    let a = range.primary.index.min(range.secondary.index);
    let b = range.primary.index.max(range.secondary.index);
    Some((a, b))
}

/// F3 / Shift+F3, Enter / Shift+Enter: the next or previous match, wrapping.
/// From the current match while it is still selected; once the cursor has
/// moved (a click, typing), from the cursor instead.
fn step_match(ctx: &egui::Context, st: &mut EditorState, dir: i32, note: Option<Uuid>) {
    let Some(f) = st.find.as_mut() else {
        return;
    };
    let n = f.matches.len();
    if n == 0 {
        return;
    }
    let caret = note.and_then(|id| note_selection(ctx, id));
    let on_current = f
        .current
        .is_some_and(|i| caret.is_none_or(|r| r == f.matches[i].chars));
    let i = match (f.current, caret) {
        (Some(i), _) if on_current => (i as i64 + dir as i64).rem_euclid(n as i64) as usize,
        (_, Some((_, b))) if dir > 0 => search::first_from(&f.matches, b).unwrap_or(0),
        (_, Some((a, _))) => search::last_before(&f.matches, a).unwrap_or(n - 1),
        _ if dir > 0 => 0,
        _ => n - 1,
    };
    f.current = Some(i);
    st.reveal = Some(f.matches[i].chars);
}

/// The `⌄` after the tabs when they don't all fit; opens the list of tabs.
fn tab_menu_button(ui: &mut Ui, st: &mut EditorState, x: f32) {
    let bar = ui.max_rect();
    let rect = Rect::from_center_size(pos2(x + 16.0, bar.bottom() - TAB_H / 2.0), vec2(28.0, 28.0));
    let resp = ui.interact(rect, egui::Id::new("tab_menu_button"), Sense::click());
    let open = st.tab_menu.is_some();
    let painter = ui.painter();
    if open {
        painter.rect_filled(rect, CornerRadius::same(5), BORDER);
    } else if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(5), NAV_ACTIVE);
    }
    let color = if open || resp.hovered() { TEXT } else { TEXT_MUTED };
    widgets::icon_chevron_down(painter, rect.center(), 10.0, color);
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let clicked = resp.on_hover_text("All tabs, export and import").clicked();
    st.tab_menu = match (clicked, open) {
        (true, true) | (false, false) => None,
        _ => Some(rect),
    };
}

/// Every tab, under the `⌄` button, then export, import and backup
/// (DESIGN §10.9).
fn tab_menu(ctx: &egui::Context, s: &mut Session, st: &mut EditorState) -> Option<Action> {
    let anchor = st.tab_menu?;
    let mut action = None;
    const W: f32 = 288.0;
    let mut close = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    let mut chosen = None;
    let active = s.vault.content.active_note;
    let max_h = (ctx.screen_rect().bottom() - anchor.bottom() - 80.0).max(120.0);
    let area = egui::Area::new(egui::Id::new("tab_menu"))
        .order(egui::Order::Foreground)
        .pivot(Align2::RIGHT_TOP)
        .fixed_pos(pos2(anchor.right(), anchor.bottom() + 6.0))
        .constrain(true)
        .show(ctx, |ui| {
            popup_frame().show(ui, |ui| {
                ui.set_width(W);
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(max_h)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for note in &s.vault.content.notes {
                            let dirty = s.dirty_notes.contains(&note.id);
                            let selected = Some(note.id) == active;
                            if tab_row(ui, &tab_title(note), note.code.is_some(), dirty, selected).clicked() {
                                chosen = Some(note.id);
                            }
                        }
                    });
                ui.add_space(6.0);
                menu_divider(ui, W);
                ui.add_space(6.0);
                for (text, a) in file_actions() {
                    if menu_row(ui, W, text, None, false).clicked() {
                        action = Some(a);
                    }
                }
                ui.add_space(6.0);
            });
        });
    let pointer = ctx.input(|i| i.pointer.interact_pos());
    if area.response.clicked_elsewhere() && !pointer.is_some_and(|p| anchor.contains(p)) {
        close = true;
    }
    if let Some(id) = chosen {
        s.vault.content.active_note = Some(id);
        close = true;
    }
    if close || action.is_some() {
        st.tab_menu = None;
        st.focus_editor = true;
    }
    action
}

/// The export, import and backup entries of the `⌄` menu.
fn file_actions() -> Vec<(&'static str, Action)> {
    vec![
        ("Export notes…", Action::ExportNotes(None)),
        ("Import notes from vault…", Action::ImportVault),
        ("Export as plain text…", Action::ExportText),
        ("Import text files…", Action::ImportText),
        ("Save a backup copy…", Action::Backup),
    ]
}

/// Right-click on a tab: Rename, Export this note…, Close tab.
fn tab_context_menu(ctx: &egui::Context, s: &mut Session, st: &mut EditorState) -> Option<Action> {
    let (id, at, fresh) = st.tab_context?;
    if s.vault.content.note(id).is_none() {
        st.tab_context = None;
        return None;
    }
    const W: f32 = 220.0;
    let mut close = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    let mut action = None;
    let mut rename = false;
    let mut close_tab = false;
    let area = egui::Area::new(egui::Id::new("tab_context_menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(at + vec2(2.0, 2.0))
        .constrain(true)
        .show(ctx, |ui| {
            popup_frame().show(ui, |ui| {
                ui.set_width(W);
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(6.0);
                rename = menu_row(ui, W, "Rename", Some("Double-click"), false).clicked();
                if menu_row(ui, W, "Export this note…", None, false).clicked() {
                    action = Some(Action::ExportNotes(Some(id)));
                }
                menu_divider(ui, W);
                close_tab = menu_row(ui, W, "Close tab", Some("Ctrl+W"), false).clicked();
                ui.add_space(6.0);
            });
        });
    if area.response.clicked_elsewhere() && !fresh {
        close = true;
    }
    if let Some((_, _, f)) = st.tab_context.as_mut() {
        *f = false;
    }
    if rename {
        let title = s
            .vault
            .content
            .note(id)
            .map(|n| n.title.clone())
            .unwrap_or_default();
        st.renaming = Some((id, title));
    }
    if close_tab {
        s.vault.content.close_note(id, heimdall_core::content::now());
        s.touch(None);
        st.focus_editor = true;
    }
    if close || rename || close_tab || action.is_some() {
        st.tab_context = None;
    }
    action
}

fn tab_row(ui: &mut Ui, title: &str, code_mode: bool, dirty: bool, selected: bool) -> egui::Response {
    let (outer, resp) = ui.allocate_exact_size(vec2(288.0, 34.0), Sense::click());
    let rect = outer.shrink2(vec2(6.0, 0.0));
    let painter = ui.painter();
    let hot = selected || resp.hovered();
    if selected {
        painter.rect_filled(rect, CornerRadius::same(5), BORDER);
    } else if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(5), NAV_ACTIVE);
    }
    let mut x = rect.left() + 10.0;
    if code_mode {
        widgets::icon_code(
            painter,
            pos2(x + 6.0, rect.center().y),
            11.0,
            if hot { TEXT_SECONDARY } else { TEXT_MUTED },
        );
        x += 18.0;
    }
    let right = rect.right() - 30.0;
    let mut job = LayoutJob::simple_singleline(
        title.to_owned(),
        crate::theme::ui(13.0),
        if hot { TEXT } else { TEXT_BODY },
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(right - x - if dirty { 14.0 } else { 0.0 });
    let galley = ui.fonts(|f| f.layout_job(job));
    let w = galley.size().x;
    painter.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, TEXT);
    if dirty {
        widgets::dot(painter, pos2(x + w + 9.0, rect.center().y), TEXT_MUTED);
    }
    if selected {
        widgets::icon_check(
            painter,
            pos2(rect.right() - 16.0, rect.center().y),
            12.0,
            TEXT_BODY,
        );
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp
}

/// Ctrl+F: the find bar at the top right of the note (DESIGN §10.5).
fn find_bar(ctx: &egui::Context, st: &mut EditorState, right: f32, note: Option<Uuid>) {
    let Some(find) = st.find.as_mut() else {
        return;
    };
    // Escape closes the bar, unless a menu took it first.
    let mut close = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    let mut step = 0;
    let field_id = egui::Id::new("find_query");
    egui::Area::new(egui::Id::new("find_bar"))
        .order(egui::Order::Foreground)
        .anchor(Align2::RIGHT_TOP, vec2(-right, chrome::TITLE_BAR_H + 10.0))
        .show(ctx, |ui| {
            popup_frame().inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let resp = search_field(ui, &mut find.query, field_id, "Find in note", 220.0);
                    if std::mem::take(&mut find.focus) {
                        resp.request_focus();
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        step = if ui.input(|i| i.modifiers.shift) { -1 } else { 1 };
                        resp.request_focus();
                    }
                    let n = find.matches.len();
                    let more = if n >= search::MAX_MATCHES { "+" } else { "" };
                    let count = match find.current {
                        _ if find.query.is_empty() => String::new(),
                        _ if n == 0 => "No results".to_owned(),
                        Some(i) => format!("{} of {n}{more}", i + 1),
                        None => format!("{n}{more} results"),
                    };
                    let (r, _) = ui.allocate_exact_size(vec2(86.0, 28.0), Sense::hover());
                    ui.painter().text(
                        r.center(),
                        Align2::CENTER_CENTER,
                        count,
                        crate::theme::ui(12.0),
                        TEXT_MUTED,
                    );
                    let case = widgets::icon_button(ui, find.match_case, "Match case", |p, c, color| {
                        p.text(c, Align2::CENTER_CENTER, "Aa", crate::theme::ui(12.0), color);
                    });
                    if case.clicked() {
                        find.match_case = !find.match_case;
                        find.focus = true;
                    }
                    let prev = widgets::icon_button(ui, false, "Previous match (Shift+F3)", |p, c, color| {
                        widgets::icon_chevron_up(p, c, 10.0, color)
                    });
                    if prev.clicked() {
                        step = -1;
                    }
                    let next = widgets::icon_button(ui, false, "Next match (F3)", |p, c, color| {
                        widgets::icon_chevron_down(p, c, 10.0, color)
                    });
                    if next.clicked() {
                        step = 1;
                    }
                    if widgets::icon_button(ui, false, "Close (Esc)", |p, c, color| {
                        widgets::icon_close(p, c, 9.0, color)
                    })
                    .clicked()
                    {
                        close = true;
                    }
                });
            });
        });
    if close {
        if let Some(mut f) = st.find.take() {
            st.last_query.zeroize();
            st.last_query = std::mem::take(&mut f.query);
        }
        st.focus_editor = true;
        return;
    }
    if step != 0 {
        step_match(ctx, st, step, note);
    }
}

/// Ctrl+Shift+F: every open note's matches, grouped by note. The trash
/// isn't searched.
fn find_all_panel(ctx: &egui::Context, s: &mut Session, st: &mut EditorState, right: f32) {
    let Some(fa) = st.find_all.as_mut() else {
        return;
    };
    let stale = fa
        .found_for
        .as_ref()
        .is_none_or(|(q, c, t)| *q != fa.query || *c != fa.match_case || *t != s.last_change);
    if stale {
        for r in &mut fa.results {
            for (_, p) in &mut r.hits {
                p.text.zeroize();
            }
        }
        fa.results.clear();
        if !fa.query.is_empty() {
            for note in &s.vault.content.notes {
                let matches = search::find_all(&note.body, &fa.query, fa.match_case);
                if matches.is_empty() {
                    continue;
                }
                let hits = matches
                    .iter()
                    .take(MAX_HITS)
                    .map(|m| (*m, search::preview(&note.body, m)))
                    .collect();
                fa.results.push(NoteHits {
                    id: note.id,
                    title: tab_title(note),
                    code: note.code.is_some(),
                    total: matches.len(),
                    hits,
                });
            }
        }
        if let Some((q, _, _)) = &mut fa.found_for {
            q.zeroize();
        }
        fa.found_for = Some((fa.query.clone(), fa.match_case, s.last_change));
    }

    const W: f32 = 460.0;
    let mut close = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    let mut chosen: Option<(Uuid, Match)> = None;
    let field_id = egui::Id::new("find_all_query");
    let max_h = (ctx.screen_rect().height() - chrome::TITLE_BAR_H - 170.0).clamp(120.0, 520.0);
    let area = egui::Area::new(egui::Id::new("find_all"))
        .order(egui::Order::Foreground)
        .anchor(Align2::RIGHT_TOP, vec2(-right, chrome::TITLE_BAR_H + 10.0))
        .show(ctx, |ui| {
            popup_frame().show(ui, |ui| {
                ui.set_width(W);
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.add_space(10.0);
                    let resp =
                        search_field(ui, &mut fa.query, field_id, "Find in all notes", W - 20.0 - 60.0);
                    if std::mem::take(&mut fa.focus) {
                        resp.request_focus();
                    }
                    let case = widgets::icon_button(ui, fa.match_case, "Match case", |p, c, color| {
                        p.text(c, Align2::CENTER_CENTER, "Aa", crate::theme::ui(12.0), color);
                    });
                    if case.clicked() {
                        fa.match_case = !fa.match_case;
                        fa.focus = true;
                    }
                    if widgets::icon_button(ui, false, "Close (Esc)", |p, c, color| {
                        widgets::icon_close(p, c, 9.0, color)
                    })
                    .clicked()
                    {
                        close = true;
                    }
                });
                let total: usize = fa.results.iter().map(|r| r.total).sum();
                let summary = match (fa.query.is_empty(), total, fa.results.len()) {
                    (true, _, _) => "Searches every open tab. The trash isn't searched.".to_owned(),
                    (false, 0, _) => "No results".to_owned(),
                    (false, 1, _) => "1 result in 1 note".to_owned(),
                    (false, t, 1) => format!("{t} results in 1 note"),
                    (false, t, m) => format!("{t} results in {m} notes"),
                };
                ui.add_space(4.0);
                menu_note(ui, W, &summary);
                if fa.results.is_empty() {
                    ui.add_space(4.0);
                    return;
                }
                menu_divider(ui, W);
                egui::ScrollArea::vertical()
                    .max_height(max_h)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.add_space(6.0);
                        for r in &fa.results {
                            note_header(ui, W, r);
                            for (m, p) in &r.hits {
                                if hit_row(ui, W, p).clicked() {
                                    chosen = Some((r.id, *m));
                                }
                            }
                            let more = r.total - r.hits.len();
                            if more > 0 {
                                menu_note(ui, W, &format!("{more} more in this note"));
                            }
                            ui.add_space(6.0);
                        }
                    });
            });
        });
    if area.response.clicked_elsewhere() {
        close = true;
    }
    if let Some((id, m)) = chosen {
        let mut f = Find::new(fa.query.clone(), fa.match_case, m.chars.0);
        f.focus = false;
        s.vault.content.active_note = Some(id);
        st.find = Some(f);
        st.reveal = Some(m.chars);
        close = true;
    }
    if close {
        st.find_all = None;
        st.focus_editor = true;
    }
}

fn note_header(ui: &mut Ui, width: f32, r: &NoteHits) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
    let painter = ui.painter();
    let mut x = rect.left() + 16.0;
    if r.code {
        widgets::icon_code(painter, pos2(x + 6.0, rect.center().y), 11.0, TEXT_MUTED);
        x += 18.0;
    }
    let count = painter.text(
        pos2(rect.right() - 16.0, rect.center().y),
        Align2::RIGHT_CENTER,
        r.total.to_string(),
        crate::theme::ui(12.0),
        TEXT_MUTED,
    );
    let mut job = LayoutJob::simple_singleline(
        r.title.clone(),
        egui::FontId::new(13.0, egui::FontFamily::Name(MEDIUM.into())),
        TEXT,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(count.left() - 12.0 - x);
    let galley = ui.fonts(|f| f.layout_job(job));
    painter.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, TEXT);
}

/// One matching line: its number, then the text with the match marked.
fn hit_row(ui: &mut Ui, width: f32, p: &search::Preview) -> egui::Response {
    let (outer, resp) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
    let rect = outer.shrink2(vec2(6.0, 0.0));
    let painter = ui.painter();
    if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(5), NAV_ACTIVE);
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let font = crate::theme::ui(12.5);
    painter.text(
        pos2(rect.left() + 40.0, rect.center().y),
        Align2::RIGHT_CENTER,
        p.line.to_string(),
        crate::theme::ui(11.0),
        TEXT_FAINT,
    );
    let plain = egui::TextFormat::simple(font.clone(), TEXT_SECONDARY);
    let marked = egui::TextFormat {
        background: egui::Color32::from_gray(0x4A),
        color: TEXT,
        ..egui::TextFormat::simple(font, TEXT)
    };
    let (a, b) = p.highlight;
    let mut job = LayoutJob::default();
    job.append(&p.text[..a], 0.0, plain.clone());
    job.append(&p.text[a..b], 0.0, marked);
    job.append(&p.text[b..], 0.0, plain);
    let x = rect.left() + 52.0;
    job.wrap = egui::text::TextWrapping::truncate_at_width(rect.right() - 10.0 - x);
    let galley = ui.fonts(|f| f.layout_job(job));
    painter.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, TEXT);
    resp
}

/// The right-click menu of a misspelled word: suggestions and "Add to
/// dictionary". Nothing is ever replaced without a click (DESIGN §10.4).
fn spell_menu(ctx: &egui::Context, s: &mut Session, st: &mut EditorState) {
    let Some(menu) = st.spell_menu.as_ref() else {
        return;
    };
    const W: f32 = 240.0;
    let mut close = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    let mut replace = None;
    let mut add = false;
    let area = egui::Area::new(egui::Id::new("spell_menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(menu.pos + vec2(2.0, 2.0))
        .constrain(true)
        .show(ctx, |ui| {
            popup_frame().show(ui, |ui| {
                ui.set_width(W);
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(6.0);
                if menu.suggestions.is_empty() {
                    menu_note(ui, W, "No suggestions");
                }
                for (i, word) in menu.suggestions.iter().enumerate() {
                    if menu_row(ui, W, word, None, false).clicked() {
                        replace = Some(i);
                    }
                }
                ui.add_space(6.0);
                menu_divider(ui, W);
                ui.add_space(6.0);
                if menu_row(ui, W, "Add to dictionary", None, false).clicked() {
                    add = true;
                }
                ui.add_space(6.0);
            });
        });
    // A right-click elsewhere was handled by the text: another word's menu.
    if area.response.clicked_elsewhere() && !ctx.input(|i| i.pointer.secondary_clicked()) {
        close = true;
    }
    if !(close || add || replace.is_some()) {
        return;
    }
    let Some(menu) = st.spell_menu.take() else {
        return;
    };
    st.focus_editor = true;
    if let Some(i) = replace {
        let Some(note) = s.vault.content.note_mut(menu.note) else {
            return;
        };
        // Only if the word is still there, unchanged.
        if note.body.get(menu.bytes.clone()) != Some(menu.word.as_str()) {
            return;
        }
        let with = &menu.suggestions[i];
        note.body.replace_range(menu.bytes.clone(), with);
        let at = note.body[..menu.bytes.start].chars().count() + with.chars().count();
        note.cursor = at as u64;
        note.modified = heimdall_core::content::now();
        let id = egui::Id::new(("note", menu.note));
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(at))));
        state.store(ctx, id);
        s.touch(Some(menu.note));
    } else if add && s.vault.content.add_to_dictionary(&menu.word) {
        s.touch(None);
    }
}

/// The tip on every new tab: Ctrl+Shift+M turns on code mode (New tab ·
/// shortcut tip mockup). Leaves after 5 s, longer while hovered.
fn code_tip(ctx: &egui::Context, s: &mut Session, st: &mut EditorState) -> Option<Action> {
    let tip = st.tip.as_mut()?;
    let mut action = None;
    let mut dismiss = false;
    let mut opt_out = !s.vault.content.editor.code_tip;
    let bottom = if s.vault.content.editor.status_bar {
        50.0
    } else {
        20.0
    };
    let area = egui::Area::new(egui::Id::new("code_tip"))
        .order(egui::Order::Foreground)
        .anchor(Align2::RIGHT_BOTTOM, vec2(-20.0, -bottom))
        .show(ctx, |ui| {
            let frame = egui::Frame::NONE
                .fill(POPUP)
                .stroke(Stroke::new(1.0_f32, POPUP_BORDER))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(egui::Margin {
                    left: 16,
                    right: 16,
                    top: 14,
                    bottom: 14,
                })
                .shadow(egui::Shadow {
                    offset: [0, 8],
                    blur: 24,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(90),
                })
                .show(ui, |ui| {
                    ui.set_width(306.0);
                    ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(270.0);
                            ui.label(
                                egui::RichText::new("Writing code in this tab?")
                                    .font(medium(13.0))
                                    .color(TEXT),
                            );
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(
                                        "Code mode adds line numbers, syntax colors and a minimap. Only this tab changes.",
                                    )
                                    .font(crate::theme::ui(12.0))
                                    .color(TEXT_MUTED),
                                )
                                .wrap(),
                            );
                        });
                        let (r, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
                        let r = r.translate(vec2(6.0, -4.0));
                        if resp.hovered() {
                            ui.painter().rect_filled(r, CornerRadius::same(4), BORDER);
                            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                        }
                        widgets::icon_close(ui.painter(), r.center(), 8.0, if resp.hovered() { TEXT } else { TEXT_MUTED });
                        if resp.on_hover_text("Dismiss").clicked() {
                            dismiss = true;
                        }
                    });
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        widgets::keys(ui, &["Ctrl", "Shift", "M"]);
                        ui.add_space(2.0);
                        widgets::label(ui, "turns it on and off", 12.0, TEXT_MUTED);
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        widgets::checkbox(ui, &mut opt_out, "Don't show again");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let resp = shortcuts_link(ui);
                            if resp.clicked() {
                                action = Some(Action::Shortcuts);
                            }
                        });
                    });
                });
            // Time left, as a thin bar along the bottom edge.
            let r = frame.response.rect;
            let bar = Rect::from_min_max(pos2(r.left() + 6.0, r.bottom() - 3.0), pos2(r.right() - 6.0, r.bottom() - 1.0));
            ui.painter().rect_filled(bar, 0.0, egui::Color32::from_gray(0x2A));
            let done = (tip.left / TIP_SECONDS).clamp(0.0, 1.0);
            ui.painter().rect_filled(
                Rect::from_min_size(bar.min, vec2(bar.width() * done, bar.height())),
                0.0,
                egui::Color32::from_gray(0x6A),
            );
        });

    if opt_out == s.vault.content.editor.code_tip {
        s.vault.content.editor.code_tip = !opt_out;
        s.touch(None);
    }
    let hovered = ctx
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| area.response.rect.contains(p));
    if !hovered {
        tip.left -= ctx.input(|i| i.stable_dt).min(0.1);
    }
    if dismiss || action.is_some() || tip.left <= 0.0 {
        st.tip = None;
        st.focus_editor = true;
    } else {
        ctx.request_repaint();
    }
    action
}

/// "All shortcuts >" in the tip.
fn shortcuts_link(ui: &mut Ui) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        "All shortcuts".to_owned(),
        crate::theme::ui(12.0),
        egui::Color32::PLACEHOLDER,
    );
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 14.0, 18.0), Sense::click());
    let color = if resp.hovered() { TEXT } else { TEXT_SECONDARY };
    ui.painter().galley(
        pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    widgets::icon_chevron_right(
        ui.painter(),
        pos2(rect.right() - 5.0, rect.center().y),
        10.0,
        color,
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp
}

/// Right-click on a secret: Reveal, Copy, Edit, Unmask. Anywhere else in
/// the text: Cut, Copy, Paste, Select all; plus Mark as secret on a
/// selection and, in Basic, Reveal all (DESIGN §10.1, §11.1).
fn secret_menu(ctx: &egui::Context, s: &mut Session, st: &mut EditorState) {
    let Some(menu) = st.secret_menu.as_ref() else {
        return;
    };
    const W: f32 = 240.0;
    let profile = s.vault.content.security.profile;
    let Some(note) = s.vault.content.note(menu.note) else {
        st.secret_menu = None;
        return;
    };
    let items = secrets::items(&note.body, &|i| st.reveals.is_shown(i));
    let mut close = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    let mut chosen = None;
    let area = egui::Area::new(egui::Id::new("secret_menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(menu.pos + vec2(2.0, 2.0))
        .constrain(true)
        .show(ctx, |ui| {
            popup_frame().show(ui, |ui| {
                ui.set_width(W);
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(6.0);
                match menu.target {
                    Some(i) => {
                        let shown = st.reveals.is_shown(i);
                        if profile != Profile::Paranoid {
                            let label = if shown { "Hide" } else { "Reveal for 5 seconds" };
                            if menu_row(ui, W, label, None, false).clicked() {
                                chosen = Some(MenuChoice::Reveal);
                            }
                        }
                        if menu_row(ui, W, "Copy", None, false).clicked() {
                            chosen = Some(MenuChoice::Copy);
                        }
                        if menu_row(ui, W, "Edit secret…", None, false).clicked() {
                            chosen = Some(MenuChoice::EditSecret);
                        }
                        ui.add_space(6.0);
                        menu_divider(ui, W);
                        ui.add_space(6.0);
                        if menu_row(ui, W, "Unmask", None, false)
                            .on_hover_text("Turn it back into plain text")
                            .clicked()
                        {
                            chosen = Some(MenuChoice::Unmask);
                        }
                    }
                    None => {
                        let sel = menu.has_selection;
                        for (label, hint, enabled, action) in [
                            ("Cut", "Ctrl+X", sel, EditAction::Cut),
                            ("Copy", "Ctrl+C", sel, EditAction::Copy),
                            ("Paste", "Ctrl+V", menu.can_paste, EditAction::Paste),
                        ] {
                            if menu_row_enabled(ui, W, label, Some(hint), enabled).clicked() {
                                chosen = Some(MenuChoice::Edit(action));
                            }
                        }
                        ui.add_space(6.0);
                        menu_divider(ui, W);
                        ui.add_space(6.0);
                        if menu_row(ui, W, "Select all", Some("Ctrl+A"), false).clicked() {
                            chosen = Some(MenuChoice::Edit(EditAction::SelectAll));
                        }
                        let reveal_all = profile == Profile::Basic && !items.is_empty();
                        if sel || reveal_all {
                            ui.add_space(6.0);
                            menu_divider(ui, W);
                            ui.add_space(6.0);
                        }
                        if sel && menu_row(ui, W, "Mark as secret", Some("Ctrl+Shift+H"), false).clicked() {
                            chosen = Some(MenuChoice::Mark);
                        }
                        if reveal_all {
                            let label = if st.reveals.all {
                                "Hide all secrets"
                            } else {
                                "Reveal all secrets…"
                            };
                            if menu_row(ui, W, label, None, false).clicked() {
                                chosen = Some(MenuChoice::RevealAll);
                            }
                        }
                    }
                }
                ui.add_space(6.0);
            });
        });
    if area.response.clicked_elsewhere() && !ctx.input(|i| i.pointer.secondary_clicked()) {
        close = true;
    }
    if !close && chosen.is_none() {
        return;
    }
    let Some(menu) = st.secret_menu.take() else {
        return;
    };
    st.focus_editor = true;
    let item = menu.target.and_then(|i| items.get(i).cloned());
    let now = ctx.input(|i| i.time);
    match (chosen, item) {
        (Some(MenuChoice::Reveal), Some(_)) => {
            if let Some(i) = menu.target {
                st.reveals.toggle(i, now + secrets::REVEAL_SECONDS);
            }
        }
        (Some(MenuChoice::Copy), Some(item)) => {
            let security = s.vault.content.security.clone();
            copy_secret(ctx, &note.body, &item, &security);
        }
        (Some(MenuChoice::EditSecret), Some(_)) => {
            st.secret_edit = Some(SecretEdit {
                note: menu.note,
                target: menu.target,
                at: 0,
                value: SecretField::default(),
                loaded: false,
                error: None,
            });
        }
        (Some(MenuChoice::Unmask), Some(item)) => {
            let Some(note) = s.vault.content.note_mut(menu.note) else {
                return;
            };
            let (body, end) = secret::unmask(&note.body, item.bytes.clone());
            let caret = body[..end].chars().count();
            drop(Zeroizing::new(std::mem::replace(&mut note.body, body)));
            note.modified = heimdall_core::content::now();
            set_caret(ctx, egui::Id::new(("note", menu.note)), caret);
            st.reveals.hide_all();
            s.touch(Some(menu.note));
        }
        (Some(MenuChoice::Mark), _) => st.mark_secret = true,
        (Some(MenuChoice::Edit(action)), _) => {
            st.edit_action = Some(action);
            ctx.request_repaint();
        }
        (Some(MenuChoice::RevealAll), _) => {
            if st.reveals.all {
                st.reveals.hide_all();
            } else {
                st.reveal_check = Some(PasswordCheck::new(
                    "Reveal all secrets?",
                    "Every secret in this note shows until you switch tabs or leave the window. Anyone who can see your screen can read them.",
                    "Reveal all",
                    false,
                ));
            }
        }
        _ => {}
    }
}

#[derive(Clone, Copy)]
enum MenuChoice {
    Reveal,
    Copy,
    EditSecret,
    Unmask,
    Mark,
    RevealAll,
    /// Cut, Copy, Paste or Select all on the text.
    Edit(EditAction),
}

/// Edit secret / New secret: a masked field, so the value is never on
/// screen unless the eye is clicked. What is pasted here is taken as is.
fn secret_edit(ctx: &egui::Context, s: &mut Session, st: &mut EditorState) {
    let Some(edit) = st.secret_edit.as_mut() else {
        return;
    };
    let Some(note) = s.vault.content.note(edit.note) else {
        st.secret_edit = None;
        return;
    };
    let target = match edit.target {
        Some(i) => match secret::scan(&note.body).secrets.get(i) {
            Some(r) => Some(r.clone()),
            None => {
                st.secret_edit = None;
                return;
            }
        },
        None => None,
    };
    if !edit.loaded {
        edit.loaded = true;
        if let Some(r) = &target {
            edit.value.text = note.body[secret::value(r)].to_owned();
        }
    }
    const W: f32 = 380.0;
    const FIELD: &str = "secret_value";
    let mut save = false;
    let mut cancel = false;
    let modal = egui::Modal::new(egui::Id::new("secret_edit"))
        .backdrop_color(egui::Color32::from_black_alpha(160))
        .frame(
            egui::Frame::NONE
                .fill(BG)
                .stroke(Stroke::new(1.0_f32, BORDER))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(egui::Margin::same(28)),
        )
        .show(ctx, |ui| {
            ui.set_width(W);
            let title = if target.is_some() {
                "Edit secret"
            } else {
                "New secret"
            };
            ui.label(egui::RichText::new(title).font(medium(18.0)).color(TEXT));
            ui.add_space(8.0);
            widgets::paragraph(ui, "Shown as dots in the note. One line.", W);
            ui.add_space(16.0);
            let resp = widgets::text_field(
                ui,
                egui::Id::new(FIELD),
                &mut edit.value.text,
                W,
                Some(&mut edit.value.reveal),
                "",
            );
            // Read Enter before taking focus back: Enter makes the field give
            // up focus, and re-focusing it first would hide that.
            save |= resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if ui.memory(|m| m.focused().is_none()) {
                resp.request_focus();
            }
            if let Some(e) = edit.error {
                ui.add_space(10.0);
                widgets::label(ui, e, 12.0, DANGER);
            }
            ui.add_space(24.0);
            ui.horizontal(|ui| {
                if widgets::button(ui, ButtonKind::Secondary, "Cancel", true, vec2(0.0, 40.0)).clicked() {
                    cancel = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ready = !edit.value.text.is_empty();
                    if widgets::button(ui, ButtonKind::Primary, "Save", ready, vec2(0.0, 40.0)).clicked() {
                        save = true;
                    }
                });
            });
        });
    if modal.should_close() || cancel {
        widgets::forget_fields(ctx, &[FIELD]);
        st.secret_edit = None;
        st.focus_editor = true;
        return;
    }
    if !save || edit.value.text.is_empty() {
        return;
    }
    let Some(wrapped) = secret::wrap(&edit.value.text) else {
        edit.error = Some(if edit.value.text.contains('\n') {
            "A secret has to be one line."
        } else {
            "A secret can't contain *}."
        });
        return;
    };
    let wrapped = Zeroizing::new(wrapped);
    let (note_id, at) = (edit.note, edit.at);
    st.secret_edit = None;
    st.focus_editor = true;
    widgets::forget_fields(ctx, &[FIELD]);
    let Some(note) = s.vault.content.note_mut(note_id) else {
        return;
    };
    let (start_char, range) = match target {
        Some(r) => (note.body[..r.start].chars().count(), r),
        None => {
            let b = char_to_byte(&note.body, at);
            (at, b..b)
        }
    };
    note.body.replace_range(range, &wrapped);
    note.modified = heimdall_core::content::now();
    set_caret(
        ctx,
        egui::Id::new(("note", note_id)),
        start_char + wrapped.chars().count(),
    );
    s.touch(Some(note_id));
}

/// Bottom right: what the paste detector did, with Undo; the first time,
/// why (DESIGN §11.3). Also short messages about secrets.
fn toast(ctx: &egui::Context, s: &mut Session, st: &mut EditorState) {
    let Some(t) = st.toast.as_mut() else {
        return;
    };
    let mut undo = false;
    let mut dismiss = false;
    let bottom = if s.vault.content.editor.status_bar {
        50.0
    } else {
        20.0
    };
    let area = egui::Area::new(egui::Id::new("secret_toast"))
        .order(egui::Order::Foreground)
        .anchor(Align2::RIGHT_BOTTOM, vec2(-20.0, -bottom))
        .show(ctx, |ui| {
            popup_frame()
                .inner_margin(egui::Margin {
                    left: 16,
                    right: 12,
                    top: 12,
                    bottom: 12,
                })
                .show(ui, |ui| {
                    ui.set_width(330.0);
                    ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(250.0);
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&t.text)
                                        .font(crate::theme::ui(13.0))
                                        .color(TEXT),
                                )
                                .wrap(),
                            );
                            if t.explain {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(SECRET_TIP)
                                            .font(crate::theme::ui(12.0))
                                            .color(TEXT_MUTED),
                                    )
                                    .wrap(),
                                );
                            }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                            let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                            if resp.hovered() {
                                ui.painter().rect_filled(r, CornerRadius::same(4), BORDER);
                                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                            }
                            let color = if resp.hovered() { TEXT } else { TEXT_MUTED };
                            widgets::icon_close(ui.painter(), r.center(), 8.0, color);
                            dismiss |= resp.on_hover_text("Dismiss").clicked();
                            if t.undo.is_some() {
                                ui.add_space(4.0);
                                undo |= widgets::link(ui, "Undo", 13.0).clicked();
                            }
                        });
                    });
                });
        });
    let hovered = ctx
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| area.response.rect.contains(p));
    if !hovered {
        t.left -= ctx.input(|i| i.stable_dt).min(0.1);
    }
    if undo {
        if let Some(u) = t.undo.take() {
            undo_paste(ctx, s, u);
        }
        st.toast = None;
        st.focus_editor = true;
    } else if dismiss || t.left <= 0.0 {
        st.toast = None;
    } else {
        ctx.request_repaint();
    }
}

/// The first time the paste detector masks something (DESIGN §11.3).
const SECRET_TIP: &str = "Heimdall masks pasted passwords, API keys and tokens so they don't sit in plain view. The value is still in the note: the eye shows it and the copy icon copies it. You can turn this off in Settings → Editor.";

/// Puts the pasted text back as it was, if it is still there unchanged.
fn undo_paste(ctx: &egui::Context, s: &mut Session, u: Undo) {
    let Some(note) = s.vault.content.note_mut(u.note) else {
        return;
    };
    let range = char_to_byte(&note.body, u.chars.0)..char_to_byte(&note.body, u.chars.1);
    if note.body.get(range.clone()) != Some(u.inserted.as_str()) {
        return;
    }
    note.body.replace_range(range, &u.replacement);
    note.modified = heimdall_core::content::now();
    set_caret(
        ctx,
        egui::Id::new(("note", u.note)),
        u.chars.0 + u.replacement.chars().count(),
    );
    s.touch(Some(u.note));
}
