//! Small custom widgets matching the mockups: buttons, fields, toggles,
//! line icons and the ASCII logo. No emojis anywhere (DESIGN §15.1).

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, CursorIcon, FontId, Painter, Pos2, Rect, Response,
    Sense, Stroke, StrokeKind, TextEdit, Ui, Vec2,
};
use zeroize::Zeroize;

use crate::theme::*;

pub const LOGO: &str = r"     #####    ##                                    ##           ###   ###
  ######  /  #### /        #                         ##           ###   ###
 /#   /  /   ####/        ###                        ##            ##    ##
/    /  /    # #           #                         ##            ##    ##
    /  /     #                                       ##            ##    ##
   ## ##     #      /##  ###   ### /### /###     ### ##    /###    ##    ##
   ## ##     #     / ###  ###   ##/ ###/ /##  / ######### / ###  / ##    ##
   ## ########    /   ###  ##    ##  ###/ ###/ ##   #### /   ###/  ##    ##
   ## ##     #   ##    ### ##    ##   ##   ##  ##    ## ##    ##   ##    ##
   ## ##     ##  ########  ##    ##   ##   ##  ##    ## ##    ##   ##    ##
   #  ##     ##  #######   ##    ##   ##   ##  ##    ## ##    ##   ##    ##
      /       ## ##        ##    ##   ##   ##  ##    ## ##    ##   ##    ##
  /##/        ## ####    / ##    ##   ##   ##  ##    /# ##    /#   ##    ##
 /  #####      ## ######/  ### / ###  ###  ###  ####/    ####/ ##  ### / ### /
/     ##           #####    ##/   ###  ###  ###  ###      ###   ##  ##/   ##/
#
 ##";

/// The ASCII "Heimdall" logo, decorative and muted (DESIGN §15.1).
pub fn logo(ui: &mut Ui, font_size: f32) {
    let line_h = font_size * 1.0625;
    let font = mono(font_size);
    let lines: Vec<&str> = LOGO.lines().collect();
    let char_w = ui.fonts(|f| f.glyph_width(&font, '#'));
    let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f32 * char_w;
    let (rect, _) = ui.allocate_exact_size(vec2(width, line_h * lines.len() as f32), Sense::hover());
    let painter = ui.painter();
    for (i, line) in lines.iter().enumerate() {
        painter.text(
            pos2(rect.left(), rect.top() + i as f32 * line_h),
            Align2::LEFT_TOP,
            line,
            font.clone(),
            TEXT_DIM,
        );
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Danger,
    /// Outlined with danger-colored text ("Revoke" in the Access mockup).
    DangerOutline,
    /// Text only, muted (e.g. "Reset to profile defaults").
    Quiet,
}

/// A flat button sized like the mockups (40-42 px tall, 6 px radius).
pub fn button(ui: &mut Ui, kind: ButtonKind, text: &str, enabled: bool, size: Vec2) -> Response {
    let font = match kind {
        ButtonKind::Primary | ButtonKind::Danger => medium(if size.y >= 40.0 { 14.0 } else { 13.0 }),
        _ => ui_font_for(size.y),
    };
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER);
    let width = size
        .x
        .max(galley.size().x + if size.y >= 40.0 { 40.0 } else { 28.0 });
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(width, size.y), sense);
    let hovered = enabled && response.hovered();
    let (fill, stroke, text_color) = match (kind, enabled) {
        (ButtonKind::Primary, true) => (if hovered { Color32::WHITE } else { TEXT }, Stroke::NONE, BG),
        (ButtonKind::Danger, true) => (
            if hovered {
                Color32::from_rgb(0xD6, 0x99, 0x93)
            } else {
                DANGER
            },
            Stroke::NONE,
            BG,
        ),
        (ButtonKind::Primary | ButtonKind::Danger, false) => (TRACK_OFF, Stroke::NONE, TEXT_DIM),
        (ButtonKind::Secondary, _) => (
            if hovered { NAV_ACTIVE } else { Color32::TRANSPARENT },
            Stroke::new(1.0_f32, BORDER),
            if enabled { TEXT_SECONDARY } else { TEXT_DIM },
        ),
        (ButtonKind::DangerOutline, _) => (
            if hovered { NAV_ACTIVE } else { Color32::TRANSPARENT },
            Stroke::new(1.0_f32, BORDER),
            if enabled { DANGER } else { TEXT_DIM },
        ),
        (ButtonKind::Quiet, _) => (
            Color32::TRANSPARENT,
            Stroke::NONE,
            if hovered {
                TEXT
            } else if enabled {
                TEXT_MUTED
            } else {
                TEXT_DIM
            },
        ),
    };
    let painter = ui.painter();
    painter.rect(rect, CornerRadius::same(6), fill, stroke, StrokeKind::Inside);
    painter.galley(rect.center() - galley.size() / 2.0, galley, text_color);
    if hovered {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    response
}

fn ui_font_for(height: f32) -> FontId {
    ui(if height >= 40.0 { 14.0 } else { 13.0 })
}

/// A muted text link that turns bright on hover.
pub fn link(ui: &mut Ui, text: &str, size: f32) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), ui_font(size), Color32::PLACEHOLDER);
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::click());
    let color = if response.hovered() { TEXT } else { TEXT_MUTED };
    ui.painter().galley(rect.min, galley, color);
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    response
}

fn ui_font(size: f32) -> FontId {
    crate::theme::ui(size)
}

pub fn label(ui: &mut Ui, text: &str, size: f32, color: Color32) -> Response {
    ui.label(egui::RichText::new(text).font(ui_font(size)).color(color))
}

pub fn heading(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(medium(22.0)).color(TEXT));
}

pub fn paragraph(ui: &mut Ui, text: &str, width: f32) {
    ui.scope(|ui| {
        ui.set_max_width(width);
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.add(egui::Label::new(egui::RichText::new(text).font(ui_font(14.0)).color(TEXT_MUTED)).wrap());
    });
}

/// A password or text value that is wiped from memory when dropped.
#[derive(Default)]
pub struct SecretField {
    pub text: String,
    pub reveal: bool,
}

impl SecretField {
    pub fn clear(&mut self) {
        self.text.zeroize();
        self.reveal = false;
    }

    /// Moves the text into a `SecretString`, wiping the field.
    pub fn take(&mut self) -> heimdall_core::SecretString {
        let s = heimdall_core::SecretString::from(std::mem::take(&mut self.text));
        self.reveal = false;
        s
    }
}

impl Drop for SecretField {
    fn drop(&mut self) {
        self.text.zeroize();
    }
}

/// Drops what egui remembers about these text fields (their text and undo
/// history), once the password typed in them has been used.
pub fn forget_fields(ctx: &egui::Context, ids: &[&str]) {
    ctx.data_mut(|d| {
        for id in ids {
            d.remove::<egui::text_edit::TextEditState>(egui::Id::new(*id));
        }
    });
}

/// A 42 px framed text field, optionally masked with a show/hide eye.
pub fn text_field(
    ui: &mut Ui,
    id: egui::Id,
    text: &mut String,
    width: f32,
    mut password: Option<&mut bool>,
    hint: &str,
) -> Response {
    let height = 42.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let focused = ui.ctx().memory(|m| m.has_focus(id));
    ui.painter().rect(
        rect,
        CornerRadius::same(6),
        SURFACE,
        Stroke::new(1.0_f32, if focused { BORDER_FOCUS } else { BORDER }),
        StrokeKind::Inside,
    );
    let eye_w = if password.is_some() { 36.0 } else { 0.0 };
    let inner = Rect::from_min_max(
        pos2(rect.left() + 12.0, rect.top()),
        pos2(rect.right() - 6.0 - eye_w, rect.bottom()),
    );
    // Alt+F8 shows or hides the password, as on the Windows sign-in
    // screen; the eye itself stays mouse-only (DESIGN §10.10).
    if focused {
        if let Some(reveal) = password.as_deref_mut() {
            if crate::focus::reveal_key(ui.ctx()) {
                *reveal = !*reveal;
            }
        }
    }
    let masked = password.as_ref().is_some_and(|r| !**r);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let response = child.add(
        TextEdit::singleline(text)
            .id(id)
            .frame(false)
            .password(masked)
            .font(ui_font(14.0))
            .text_color(TEXT)
            .hint_text(egui::RichText::new(hint).color(TEXT_FAINT))
            .desired_width(inner.width()),
    );
    if let Some(reveal) = password {
        let eye_rect =
            Rect::from_center_size(pos2(rect.right() - 6.0 - 15.0, rect.center().y), vec2(30.0, 30.0));
        // Mouse only: if Tab could focus it, typing a space would toggle it.
        let eye = ui.interact(eye_rect, id.with("eye"), Sense::CLICK);
        icon_eye(
            ui.painter(),
            eye_rect.center(),
            if eye.hovered() {
                TEXT
            } else {
                Color32::from_gray(0x82)
            },
        );
        if eye.clicked() {
            *reveal = !*reveal;
        }
        eye.on_hover_text(if *reveal {
            "Hide password (Alt+F8)"
        } else {
            "Show password (Alt+F8)"
        });
    }
    response
}

/// iOS-like switch from the Settings mockups.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(36.0, 20.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let painter = ui.painter();
    let (track, knob, x) = if *on {
        (TEXT_BODY, BG, rect.right() - 10.0)
    } else {
        (Color32::from_gray(0x2A), TEXT_MUTED, rect.left() + 10.0)
    };
    painter.rect_filled(rect, CornerRadius::same(10), track);
    painter.circle_filled(pos2(x, rect.center().y), 7.0, knob);
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    response
}

/// A 14 px checkbox with its label; the label is clickable too.
pub fn checkbox(ui: &mut Ui, on: &mut bool, text: &str) -> Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), ui_font(12.0), Color32::PLACEHOLDER);
    let (rect, mut response) = ui.allocate_exact_size(vec2(22.0 + galley.size().x, 18.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let hovered = response.hovered();
    let boxr = Rect::from_center_size(pos2(rect.left() + 7.0, rect.center().y), vec2(14.0, 14.0));
    let painter = ui.painter();
    painter.rect(
        boxr,
        CornerRadius::same(3),
        if *on { TEXT_BODY } else { SURFACE },
        Stroke::new(1.0_f32, if hovered { ICON } else { BORDER_FOCUS }),
        StrokeKind::Inside,
    );
    if *on {
        icon_check(painter, boxr.center(), 10.0, BG);
    }
    painter.galley(
        pos2(rect.left() + 22.0, rect.center().y - galley.size().y / 2.0),
        galley,
        if hovered { TEXT } else { TEXT_MUTED },
    );
    if hovered {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    response
}

/// Keyboard keys drawn as small chips: `keys(ui, &["Ctrl", "Shift", "M"])`.
pub fn keys(ui: &mut Ui, keys: &[&str]) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for key in keys {
            let galley = ui
                .painter()
                .layout_no_wrap((*key).to_owned(), mono(11.0), TEXT_BODY);
            let (rect, _) = ui.allocate_exact_size(vec2(galley.size().x + 14.0, 20.0), Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, CornerRadius::same(4), Color32::from_gray(0x3A));
            painter.rect_filled(
                Rect::from_min_max(rect.min + vec2(1.0, 1.0), rect.max - vec2(1.0, 2.0)),
                CornerRadius::same(3),
                SURFACE,
            );
            painter.galley(
                rect.center() - vec2(0.0, 1.0) - galley.size() / 2.0,
                galley,
                TEXT_BODY,
            );
        }
    });
}

/// A settings row: title + description on the left, a control on the right,
/// divider underneath (Settings mockups).
pub fn setting_row(
    ui: &mut Ui,
    title: &str,
    description: Option<(&str, Color32)>,
    control: impl FnOnce(&mut Ui),
) {
    let width = ui.available_width();
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.set_width(width);
        ui.vertical(|ui| {
            ui.set_width(width - 180.0);
            ui.spacing_mut().item_spacing.y = 4.0;
            label(ui, title, 14.0, TEXT);
            if let Some((d, color)) = description {
                ui.add(egui::Label::new(egui::RichText::new(d).font(ui_font(12.0)).color(color)).wrap());
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
    ui.add_space(14.0);
    let y = ui.cursor().top();
    ui.painter().hline(
        ui.min_rect().left()..=ui.min_rect().left() + width,
        y,
        Stroke::new(1.0_f32, DIVIDER),
    );
    ui.add_space(1.0);
}

// ---------- Line icons (stroke only, same color as text) ----------

fn scaled(center: Pos2, size: f32, view: f32) -> impl Fn(f32, f32) -> Pos2 {
    let s = size / view;
    let origin = center - vec2(size, size) / 2.0;
    move |x, y| pos2(origin.x + x * s, origin.y + y * s)
}

/// The title bar mark (DESIGN §15.1), from `assets/icon/logo_mini.svg`.
/// Filled at the screen's exact pixel size, so it stays sharp on any
/// display scale; each size is made once and kept.
pub fn logo_mark(ui: &Ui, center: Pos2, size: f32, color: Color32) {
    let ctx = ui.ctx();
    let px = (size * ctx.pixels_per_point()).round().max(1.0) as usize;
    let id = egui::Id::new(("logo_mark", px));
    let texture = ctx
        .data_mut(|d| d.get_temp::<egui::TextureHandle>(id))
        .unwrap_or_else(|| {
            let coverage = rasterize(crate::logo_mark::POLYGONS, px);
            let pixels = coverage
                .into_iter()
                .map(|a| Color32::from_rgba_premultiplied(a, a, a, a))
                .collect();
            let image = egui::ColorImage {
                size: [px, px],
                pixels,
            };
            let handle = ctx.load_texture("logo_mark", image, egui::TextureOptions::LINEAR);
            ctx.data_mut(|d| d.insert_temp(id, handle.clone()));
            handle
        });
    let rect = Rect::from_center_size(center, vec2(size, size));
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter().image(texture.id(), rect, uv, color);
}

/// Coverage (0-255) of an `n`x`n` square by `polygons` (unit square,
/// even-odd rule), with 4x4 samples per pixel.
fn rasterize(polygons: &[&[[f32; 2]]], n: usize) -> Vec<u8> {
    const SUB: usize = 4;
    let mut hits = vec![0u16; n * n];
    let mut xs: Vec<f32> = Vec::new();
    let scale = (n * SUB) as f32;
    for sy in 0..n * SUB {
        let y = (sy as f32 + 0.5) / scale;
        xs.clear();
        for poly in polygons {
            for (i, a) in poly.iter().enumerate() {
                let b = poly[(i + 1) % poly.len()];
                if (a[1] <= y) != (b[1] <= y) {
                    xs.push(a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]));
                }
            }
        }
        xs.sort_by(f32::total_cmp);
        let row = (sy / SUB) * n;
        for span in xs.as_chunks::<2>().0 {
            // Samples sit at the middle of each sub-column.
            let from = (span[0] * scale - 0.5).ceil().max(0.0) as usize;
            let to = ((span[1] * scale - 0.5).ceil().max(0.0) as usize).min(n * SUB);
            for sx in from..to {
                hits[row + sx / SUB] += 1;
            }
        }
    }
    hits.into_iter()
        .map(|h| (h as u32 * 255 / (SUB * SUB) as u32) as u8)
        .collect()
}

pub fn icon_close(painter: &Painter, center: Pos2, size: f32, color: Color32) {
    let h = size / 2.0;
    let s = Stroke::new(1.2_f32, color);
    painter.line_segment([center + vec2(-h, -h), center + vec2(h, h)], s);
    painter.line_segment([center + vec2(h, -h), center + vec2(-h, h)], s);
}

pub fn icon_plus(painter: &Painter, center: Pos2, size: f32, color: Color32) {
    let h = size / 2.0;
    let s = Stroke::new(1.3_f32, color);
    painter.line_segment([center + vec2(0.0, -h), center + vec2(0.0, h)], s);
    painter.line_segment([center + vec2(-h, 0.0), center + vec2(h, 0.0)], s);
}

pub fn icon_lock(painter: &Painter, center: Pos2, color: Color32) {
    let p = scaled(center, 12.0, 12.0);
    let s = Stroke::new(1.2_f32, color);
    painter.rect_stroke(
        Rect::from_min_max(p(2.0, 5.5), p(10.0, 11.0)),
        1.0,
        s,
        StrokeKind::Middle,
    );
    let arc: Vec<Pos2> = (0..=10)
        .map(|i| {
            let a = std::f32::consts::PI * (1.0 + i as f32 / 10.0);
            p(6.0 + 2.0 * a.cos(), 4.0 + 2.0 * a.sin())
        })
        .collect();
    painter.add(egui::Shape::line(arc, s));
    painter.line_segment([p(4.0, 4.0), p(4.0, 5.5)], s);
    painter.line_segment([p(8.0, 4.0), p(8.0, 5.5)], s);
}

/// A USB stick on its side (Access mockup): body plus connector.
pub fn icon_usb(painter: &Painter, center: Pos2, color: Color32) {
    let p = scaled(center, 16.0, 16.0);
    let s = Stroke::new(1.2_f32, color);
    painter.rect_stroke(
        Rect::from_min_max(p(2.0, 5.0), p(14.0, 12.0)),
        1.5,
        s,
        StrokeKind::Middle,
    );
    painter.add(egui::Shape::line(
        vec![p(5.0, 5.0), p(5.0, 3.5), p(11.0, 3.5), p(11.0, 5.0)],
        s,
    ));
}

/// A sheet with a folded corner and two lines: the recovery sheet.
pub fn icon_sheet(painter: &Painter, center: Pos2, color: Color32) {
    let p = scaled(center, 16.0, 16.0);
    let s = Stroke::new(1.2_f32, color);
    painter.add(egui::Shape::closed_line(
        vec![
            p(4.0, 2.0),
            p(10.0, 2.0),
            p(13.0, 5.0),
            p(13.0, 14.0),
            p(4.0, 14.0),
        ],
        s,
    ));
    painter.line_segment([p(6.0, 8.0), p(11.0, 8.0)], s);
    painter.line_segment([p(6.0, 11.0), p(11.0, 11.0)], s);
}

/// `</>`: a tab in code mode.
pub fn icon_code(painter: &Painter, center: Pos2, size: f32, color: Color32) {
    let p = scaled(center, size, 12.0);
    let s = Stroke::new(1.3_f32, color);
    painter.add(egui::Shape::line(vec![p(4.0, 3.0), p(1.0, 6.0), p(4.0, 9.0)], s));
    painter.add(egui::Shape::line(vec![p(8.0, 3.0), p(11.0, 6.0), p(8.0, 9.0)], s));
}

pub fn icon_search(painter: &Painter, center: Pos2, color: Color32) {
    let p = scaled(center, 12.0, 12.0);
    let s = Stroke::new(1.3_f32, color);
    painter.circle_stroke(p(5.0, 5.0), 3.5, s);
    painter.line_segment([p(7.7, 7.7), p(10.5, 10.5)], s);
}

pub fn icon_chevron_right(painter: &Painter, center: Pos2, size: f32, color: Color32) {
    let p = scaled(center, size, 12.0);
    painter.add(egui::Shape::line(
        vec![p(4.5, 2.0), p(8.5, 6.0), p(4.5, 10.0)],
        Stroke::new(1.3_f32, color),
    ));
}

pub fn icon_chevron_down(painter: &Painter, center: Pos2, size: f32, color: Color32) {
    let p = scaled(center, size, 12.0);
    painter.add(egui::Shape::line(
        vec![p(2.0, 4.5), p(6.0, 8.5), p(10.0, 4.5)],
        Stroke::new(1.3_f32, color),
    ));
}

pub fn icon_chevron_up(painter: &Painter, center: Pos2, size: f32, color: Color32) {
    let p = scaled(center, size, 12.0);
    painter.add(egui::Shape::line(
        vec![p(2.0, 7.5), p(6.0, 3.5), p(10.0, 7.5)],
        Stroke::new(1.3_f32, color),
    ));
}

/// A square icon button (28 px), highlighted while `on`.
pub fn icon_button(
    ui: &mut Ui,
    on: bool,
    tooltip: &str,
    paint: impl FnOnce(&Painter, Pos2, Color32),
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
    let hovered = resp.hovered();
    if on {
        ui.painter().rect_filled(rect, CornerRadius::same(5), BORDER);
    } else if hovered {
        ui.painter().rect_filled(rect, CornerRadius::same(5), NAV_ACTIVE);
    }
    paint(
        ui.painter(),
        rect.center(),
        if on || hovered { TEXT } else { TEXT_MUTED },
    );
    if hovered {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp.on_hover_text(tooltip)
}

pub fn icon_eye(painter: &Painter, center: Pos2, color: Color32) {
    let s = Stroke::new(1.2_f32, color);
    let pts = |sign: f32| -> Vec<Pos2> {
        (0..=12)
            .map(|i| {
                let t = i as f32 / 12.0;
                let x = -7.0 + 14.0 * t;
                let y = sign * 5.0 * (1.0 - (2.0 * t - 1.0).powi(2)).sqrt() * 0.9;
                center + vec2(x, y)
            })
            .collect()
    };
    painter.add(egui::Shape::line(pts(1.0), s));
    painter.add(egui::Shape::line(pts(-1.0), s));
    painter.circle_stroke(center, 2.0, s);
}

pub fn icon_check(painter: &Painter, center: Pos2, size: f32, color: Color32) {
    let p = scaled(center, size, 12.0);
    let s = Stroke::new(1.5_f32, color);
    painter.add(egui::Shape::line(vec![p(2.5, 6.5), p(5.0, 9.0), p(9.5, 3.5)], s));
}

pub fn icon_chevron_left(painter: &Painter, center: Pos2, color: Color32) {
    let p = scaled(center, 12.0, 12.0);
    painter.add(egui::Shape::line(
        vec![p(7.5, 2.0), p(3.5, 6.0), p(7.5, 10.0)],
        Stroke::new(1.3_f32, color),
    ));
}

pub fn icon_trash(painter: &Painter, center: Pos2, color: Color32) {
    let p = scaled(center, 14.0, 16.0);
    let s = Stroke::new(1.2_f32, color);
    painter.line_segment([p(2.5, 4.0), p(13.5, 4.0)], s);
    painter.add(egui::Shape::line(
        vec![p(6.0, 4.0), p(6.0, 2.5), p(10.0, 2.5), p(10.0, 4.0)],
        s,
    ));
    painter.add(egui::Shape::line(
        vec![p(4.0, 4.0), p(4.8, 14.0), p(11.2, 14.0), p(12.0, 4.0)],
        s,
    ));
}

/// A filled 6 px status dot (USB connected, unsaved changes).
pub fn dot(painter: &Painter, center: Pos2, color: Color32) {
    painter.circle_filled(center, 3.0, color);
}

/// Two overlapping sheets: Copy.
pub fn icon_copy(painter: &Painter, center: Pos2, color: Color32) {
    let s = Stroke::new(1.2_f32, color);
    let back = Rect::from_center_size(center + vec2(-1.5, -1.5), vec2(8.0, 9.0));
    let front = back.translate(vec2(3.0, 3.0));
    painter.add(egui::Shape::line(
        vec![
            pos2(back.left(), front.top() - 0.5),
            back.left_top(),
            pos2(front.right() - 0.5, back.top()),
        ],
        s,
    ));
    painter.rect_stroke(front, CornerRadius::same(1), s, StrokeKind::Middle);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_mark_fills_the_rune_and_keeps_the_hole() {
        let n = 64;
        let px = rasterize(crate::logo_mark::POLYGONS, n);
        let at = |x: usize, y: usize| px[y * n + x];
        assert_eq!(at(0, 0), 0, "corner is empty");
        assert_eq!(at(32, 10), 255, "the vertical stroke is solid");
        assert_eq!(at(38, 36), 0, "the triangle inside the rune is a hole");
    }
}
