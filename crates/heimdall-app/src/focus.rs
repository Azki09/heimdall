//! Keyboard focus helpers (DESIGN §10.10): the focus ring and the
//! "keyboard mode" that decides whether it shows.
//!
//! egui already moves focus with Tab / Shift+Tab and the arrow keys, and a
//! focused control is pressed with Space or Enter. What it does not do for
//! our hand-painted controls is show *where* the focus is, so after every
//! frame we draw a ring around the focused control. The ring only shows
//! while the user is driving the app with the keyboard: a mouse press hides
//! it again, so with the mouse the app looks exactly as before.

use eframe::egui::{self, CornerRadius, Id, Key, Stroke, StrokeKind};

use crate::theme::ICON;

fn mode_id() -> Id {
    Id::new("heimdall_keyboard_mode")
}

/// True while the last thing the user did was a navigation key.
pub fn keyboard_mode(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(mode_id()).unwrap_or(false))
}

pub fn set_keyboard_mode(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(mode_id(), on));
}

/// Call at the start of every frame.
pub fn begin_frame(ctx: &egui::Context) {
    let (nav, mouse) = ctx.input(|i| {
        let nav = i.events.iter().any(|e| {
            matches!(
                e,
                egui::Event::Key {
                    key: Key::Tab
                        | Key::ArrowUp
                        | Key::ArrowDown
                        | Key::ArrowLeft
                        | Key::ArrowRight
                        | Key::F6
                        | Key::F10,
                    pressed: true,
                    ..
                }
            )
        });
        (nav, i.pointer.any_pressed())
    });
    if mouse {
        set_keyboard_mode(ctx, false);
    } else if nav {
        set_keyboard_mode(ctx, true);
    }
}

/// Call at the end of every frame, after all screens drew.
pub fn paint_ring(ctx: &egui::Context) {
    if !keyboard_mode(ctx) {
        return;
    }
    // A text field shows focus with its own border and caret.
    if ctx.wants_keyboard_input() {
        return;
    }
    let Some(id) = ctx.memory(|m| m.focused()) else {
        return;
    };
    let Some(resp) = ctx.read_response(id) else {
        return;
    };
    if !resp.rect.is_positive() {
        return;
    }
    ctx.layer_painter(resp.layer_id)
        .with_clip_rect(ctx.screen_rect())
        .rect_stroke(
            resp.rect.expand(2.0),
            CornerRadius::same(7),
            Stroke::new(1.5_f32, ICON),
            StrokeKind::Outside,
        );
}

fn seed_id() -> Id {
    Id::new("heimdall_seed_menu")
}

/// A menu is opening from the keyboard: its first row takes the focus, so
/// the arrow keys and Enter work in it right away. Does nothing with the
/// mouse, so a clicked menu looks as before.
pub fn seed_menu(ctx: &egui::Context) {
    if keyboard_mode(ctx) || ctx.input(|i| !i.keys_down.is_empty()) {
        set_keyboard_mode(ctx, true);
        ctx.data_mut(|d| d.insert_temp(seed_id(), true));
    }
}

/// Called by every menu row: the first one drawn after [`seed_menu`] takes
/// the focus.
pub fn claim_seed(resp: &egui::Response) {
    let ctx = &resp.ctx;
    if ctx.data(|d| d.get_temp::<bool>(seed_id()).unwrap_or(false)) {
        ctx.data_mut(|d| d.remove::<bool>(seed_id()));
        resp.request_focus();
    }
}

/// Enter pressed to submit a dialog. Not when Enter is meant for a focused
/// button (Cancel, Back...): egui presses that button instead.
pub fn enter_submits(ui: &egui::Ui) -> bool {
    let ctx = ui.ctx();
    let on_button = ctx.memory(|m| m.focused()).is_some() && !ctx.wants_keyboard_input();
    !on_button && ui.input(|i| i.key_pressed(Key::Enter))
}

/// Escape pressed for the page itself: an open menu, dropdown or dialog on
/// top takes Escape first. Consumes the key.
pub fn escape(ctx: &egui::Context) -> bool {
    let covered = ctx.memory(|m| m.any_popup_open() || m.top_modal_layer().is_some());
    !covered && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape))
}

/// Shift+F10 or the Menu key: open the context menu at the caret. egui has
/// no key for the Menu key, so the window procedure reports it (and the
/// keyboard context-menu message Windows makes from both keys).
pub fn context_menu_key(ctx: &egui::Context) -> bool {
    let shift_f10 = ctx.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, Key::F10));
    let menu = heimdall_platform::take_menu_key();
    if shift_f10 || menu {
        set_keyboard_mode(ctx, true);
    }
    shift_f10 || menu
}

/// Alt+F8 in a password field: show or hide the password (as on the
/// Windows sign-in screen).
pub fn reveal_key(ctx: &egui::Context) -> bool {
    ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, Key::F8))
}
