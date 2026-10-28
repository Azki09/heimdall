//! Colors, fonts and the global egui style (DESIGN §15.2, §15.3).

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, Visuals,
};
use heimdall_core::content::Font;

pub const BG: Color32 = Color32::from_rgb(0x14, 0x14, 0x14);
pub const SURFACE: Color32 = Color32::from_rgb(0x1B, 0x1B, 0x1B);
pub const NAV_ACTIVE: Color32 = Color32::from_rgb(0x1E, 0x1E, 0x1E);
pub const BORDER: Color32 = Color32::from_rgb(0x2C, 0x2C, 0x2C);
pub const DIVIDER: Color32 = Color32::from_rgb(0x22, 0x22, 0x22);
pub const BORDER_FOCUS: Color32 = Color32::from_rgb(0x4A, 0x4A, 0x4A);
pub const TEXT: Color32 = Color32::from_rgb(0xE4, 0xE4, 0xE4);
pub const TEXT_BODY: Color32 = Color32::from_rgb(0xD4, 0xD4, 0xD4);
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(0xB8, 0xB8, 0xB8);
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x8A, 0x8A, 0x8A);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x6E, 0x6E, 0x6E);
pub const ICON: Color32 = Color32::from_rgb(0x9A, 0x9A, 0x9A);
pub const TRACK_OFF: Color32 = Color32::from_rgb(0x2E, 0x2E, 0x2E);
pub const OK: Color32 = Color32::from_rgb(0x6E, 0x9C, 0x7A);
pub const DANGER: Color32 = Color32::from_rgb(0xC9, 0x8A, 0x84);

pub const MEDIUM: &str = "Inter Medium";
const CONSOLAS: &str = "Consolas";
const SEGOE: &str = "Segoe UI";

pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MEDIUM.into()))
}

pub fn ui(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

/// Font family for the editor setting (DESIGN §10.4). Consolas and Segoe UI
/// come from Windows; if missing, fall back to the bundled fonts.
pub fn editor_family(ctx: &egui::Context, font: Font) -> FontFamily {
    let wanted = match font {
        Font::JetBrainsMono => return FontFamily::Monospace,
        Font::Inter => return FontFamily::Proportional,
        Font::Consolas => CONSOLAS,
        Font::SegoeUi => SEGOE,
    };
    let family = FontFamily::Name(wanted.into());
    if ctx.fonts(|f| f.families().contains(&family)) {
        family
    } else if font == Font::SegoeUi {
        FontFamily::Proportional
    } else {
        FontFamily::Monospace
    }
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, data: FontData| {
        fonts.font_data.insert(name.to_owned(), Arc::new(data));
    };
    add(
        "Inter",
        FontData::from_static(include_bytes!("../assets/fonts/Inter-Regular.ttf")),
    );
    add(
        MEDIUM,
        FontData::from_static(include_bytes!("../assets/fonts/Inter-Medium.ttf")),
    );
    add(
        "JetBrains Mono",
        FontData::from_static(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf")),
    );

    let prop = fonts.families.entry(FontFamily::Proportional).or_default();
    prop.insert(0, "Inter".to_owned());
    let mono = fonts.families.entry(FontFamily::Monospace).or_default();
    mono.insert(0, "JetBrains Mono".to_owned());
    let fallback: Vec<String> = fonts.families[&FontFamily::Proportional].clone();
    fonts.families.insert(
        FontFamily::Name(MEDIUM.into()),
        [vec![MEDIUM.to_owned()], fallback.clone()].concat(),
    );

    for (name, file) in [(CONSOLAS, "consola.ttf"), (SEGOE, "segoeui.ttf")] {
        if let Some(bytes) = system_font(file) {
            fonts
                .font_data
                .insert(name.to_owned(), Arc::new(FontData::from_owned(bytes)));
            fonts.families.insert(
                FontFamily::Name(name.into()),
                [vec![name.to_owned()], fallback.clone()].concat(),
            );
        }
    }
    ctx.set_fonts(fonts);
}

fn system_font(file: &str) -> Option<Vec<u8>> {
    let windir = std::env::var_os("WINDIR")?;
    std::fs::read(std::path::Path::new(&windir).join("Fonts").join(file)).ok()
}

pub fn apply_style(ctx: &egui::Context) {
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    ctx.style_mut(|style| {
        let mut v = Visuals::dark();
        v.panel_fill = BG;
        v.window_fill = BG;
        v.extreme_bg_color = SURFACE;
        v.faint_bg_color = SURFACE;
        v.override_text_color = Some(TEXT);
        v.selection.bg_fill = Color32::from_rgb(0x3A, 0x3A, 0x3A);
        v.selection.stroke = Stroke::new(1.0_f32, TEXT);
        v.text_cursor.stroke = Stroke::new(1.5_f32, TEXT);
        v.window_stroke = Stroke::new(1.0_f32, BORDER);
        v.window_corner_radius = CornerRadius::same(8);
        v.window_shadow = egui::Shadow::NONE;
        v.popup_shadow = egui::Shadow::NONE;
        v.hyperlink_color = TEXT_MUTED;
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = CornerRadius::same(6);
            w.bg_fill = SURFACE;
            w.weak_bg_fill = SURFACE;
            w.bg_stroke = Stroke::new(1.0_f32, BORDER);
            w.fg_stroke = Stroke::new(1.0_f32, TEXT);
            w.expansion = 0.0;
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, DIVIDER);
        v.widgets.hovered.weak_bg_fill = NAV_ACTIVE;
        v.widgets.hovered.bg_fill = NAV_ACTIVE;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, BORDER_FOCUS);
        v.widgets.active.bg_stroke = Stroke::new(1.0_f32, BORDER_FOCUS);
        style.visuals = v;
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.interact_size.y = 28.0;
        style.text_styles.insert(egui::TextStyle::Body, ui(14.0));
        style.text_styles.insert(egui::TextStyle::Button, ui(14.0));
        style.text_styles.insert(egui::TextStyle::Small, ui(12.0));
        style.text_styles.insert(egui::TextStyle::Monospace, mono(14.0));
        style.text_styles.insert(egui::TextStyle::Heading, medium(22.0));
    });
}
