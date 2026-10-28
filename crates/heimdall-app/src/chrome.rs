//! Custom window chrome: the window has no system title bar, so we draw our
//! own (bridge mark, title or tabs, window buttons) and handle dragging and
//! edge resizing ourselves, like Windows 11 Notepad.

use eframe::egui::{self, pos2, vec2, CursorIcon, Rect, ResizeDirection, Sense, Stroke, Ui, ViewportCommand};

use crate::theme::*;
use crate::widgets;

pub const TITLE_BAR_H: f32 = 44.0;
const BUTTON_W: f32 = 46.0;
const RESIZE_MARGIN: f32 = 5.0;

/// Draws the title bar. `content` fills the space between the bridge mark
/// and the window buttons; whatever area it leaves free drags the window.
pub fn title_bar(ctx: &egui::Context, content: impl FnOnce(&mut Ui)) {
    egui::TopBottomPanel::top("title_bar")
        .exact_height(TITLE_BAR_H)
        .frame(egui::Frame::NONE.fill(BG))
        .show_separator_line(false)
        .show(ctx, |ui| {
            let bar = ui.max_rect();

            // Background drag area first, so widgets on top take priority.
            let drag = ui.interact(bar, egui::Id::new("title_drag"), Sense::click_and_drag());
            if drag.drag_started_by(egui::PointerButton::Primary) {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }
            if drag.double_clicked() {
                toggle_maximize(ctx);
            }

            let buttons_w = BUTTON_W * 3.0;
            let content_rect = Rect::from_min_max(
                pos2(bar.left() + 14.0, bar.top()),
                pos2(bar.right() - buttons_w, bar.bottom()),
            );

            widgets::icon_bridge(
                ui.painter(),
                pos2(content_rect.left() + 8.0, bar.center().y),
                16.0,
                ICON,
            );
            let inner = Rect::from_min_max(pos2(content_rect.left() + 26.0, bar.top()), content_rect.max);
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(inner)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            content(&mut child);

            window_buttons(
                ui,
                ctx,
                Rect::from_min_max(pos2(bar.right() - buttons_w, bar.top()), bar.max),
            );
        });
}

/// Plain title text, used on every screen except the editor.
pub fn title_text(ui: &mut Ui, text: &str) {
    widgets::label(ui, text, 12.0, TEXT_MUTED);
}

fn window_buttons(ui: &mut Ui, ctx: &egui::Context, rect: Rect) {
    let kinds = [0, 1, 2];
    for (i, kind) in kinds.into_iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(rect.left() + i as f32 * BUTTON_W, rect.top()),
            vec2(BUTTON_W, TITLE_BAR_H),
        );
        let resp = ui.interact(r, egui::Id::new(("win_btn", kind)), Sense::click());
        let painter = ui.painter();
        if resp.hovered() {
            let fill = if kind == 2 {
                egui::Color32::from_rgb(0xC4, 0x2B, 0x1C)
            } else {
                NAV_ACTIVE
            };
            painter.rect_filled(r, 0.0, fill);
        }
        let c = r.center();
        let color = if resp.hovered() { TEXT } else { TEXT_MUTED };
        let s = Stroke::new(1.0_f32, color);
        match kind {
            0 => {
                painter.line_segment([c + vec2(-5.0, 0.0), c + vec2(5.0, 0.0)], s);
            }
            1 => {
                painter.rect_stroke(
                    Rect::from_center_size(c, vec2(9.0, 9.0)),
                    0.0,
                    s,
                    egui::StrokeKind::Middle,
                );
            }
            _ => widgets::icon_close(painter, c, 10.0, color),
        }
        if resp.clicked() {
            match kind {
                0 => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
                1 => toggle_maximize(ctx),
                _ => ctx.send_viewport_cmd(ViewportCommand::Close),
            }
        }
    }
}

fn toggle_maximize(ctx: &egui::Context) {
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
}

/// Lets the user resize the borderless window from its edges and corners.
pub fn handle_resize(ctx: &egui::Context) {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let screen = ctx.screen_rect();
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    let m = RESIZE_MARGIN;
    let left = pos.x <= screen.left() + m;
    let right = pos.x >= screen.right() - m;
    let top = pos.y <= screen.top() + m;
    let bottom = pos.y >= screen.bottom() - m;
    let dir = match (left, right, top, bottom) {
        (true, _, true, _) => ResizeDirection::NorthWest,
        (_, true, true, _) => ResizeDirection::NorthEast,
        (true, _, _, true) => ResizeDirection::SouthWest,
        (_, true, _, true) => ResizeDirection::SouthEast,
        (true, ..) => ResizeDirection::West,
        (_, true, ..) => ResizeDirection::East,
        (_, _, true, _) => ResizeDirection::North,
        (_, _, _, true) => ResizeDirection::South,
        _ => return,
    };
    let icon = match dir {
        ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
        ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
        ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::ResizeNeSw,
    };
    ctx.set_cursor_icon(icon);
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
    }
}

/// The 1 px window border from the mockups.
pub fn paint_border(ctx: &egui::Context) {
    let rect = ctx.screen_rect().shrink(0.5);
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("window_border"),
    ))
    .rect_stroke(rect, 0.0, Stroke::new(1.0_f32, BORDER), egui::StrokeKind::Middle);
}

/// Footer tagline on Welcome / Unlock / Locked.
pub fn tagline_footer(ctx: &egui::Context) {
    egui::TopBottomPanel::bottom("tagline")
        .exact_height(30.0)
        .frame(egui::Frame::NONE.fill(BG))
        .show_separator_line(false)
        .show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                widgets::label(ui, "No signal. No trace.", 12.0, TEXT_FAINT);
            });
        });
}
