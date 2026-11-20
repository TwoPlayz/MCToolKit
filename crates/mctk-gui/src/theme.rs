//! Colors, fonts and base style: the dark theme, and a light one (see [`set_light`]).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle,
};

/// The colors that change with the theme: the page, cards, borders, fields, buttons and text.
struct Palette {
    bg: Color32,
    card: Color32,
    border: Color32,
    separator: Color32,
    deep: Color32,
    field: Color32,
    button: Color32,
    button_hover: Color32,
    tile: Color32,
    chip: Color32,
    text: Color32,
    text_muted: Color32,
    text_dim: Color32,
}

const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x1D, 0x21, 0x28),
    card: Color32::from_rgb(0x23, 0x28, 0x30),
    border: Color32::from_rgb(0x2C, 0x32, 0x3A),
    separator: Color32::from_rgb(0x3A, 0x41, 0x4C),
    deep: Color32::from_rgb(0x16, 0x18, 0x1E),
    field: Color32::from_rgb(0x18, 0x1C, 0x22),
    button: Color32::from_rgb(0x2D, 0x33, 0x3E),
    button_hover: Color32::from_rgb(0x36, 0x3C, 0x47),
    tile: Color32::from_rgb(0x2A, 0x2F, 0x38),
    chip: Color32::from_rgb(0x37, 0x3F, 0x49),
    text: Color32::from_rgb(0xD7, 0xDC, 0xE2),
    text_muted: Color32::from_rgb(0x8A, 0x93, 0x9E),
    text_dim: Color32::from_rgb(0x58, 0x60, 0x6B),
};

const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xF2, 0xF3, 0xF5),
    card: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    border: Color32::from_rgb(0xD3, 0xD7, 0xDD),
    separator: Color32::from_rgb(0xC4, 0xC9, 0xD1),
    deep: Color32::from_rgb(0xE4, 0xE7, 0xEB),
    field: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    button: Color32::from_rgb(0xE1, 0xE4, 0xE9),
    button_hover: Color32::from_rgb(0xD2, 0xD6, 0xDD),
    tile: Color32::from_rgb(0xE8, 0xEB, 0xEF),
    chip: Color32::from_rgb(0xC9, 0xCE, 0xD6),
    text: Color32::from_rgb(0x1E, 0x22, 0x28),
    text_muted: Color32::from_rgb(0x55, 0x5D, 0x68),
    text_dim: Color32::from_rgb(0x98, 0x9F, 0xA9),
};

static LIGHT_ON: AtomicBool = AtomicBool::new(false);

fn pal() -> &'static Palette {
    if LIGHT_ON.load(Ordering::Relaxed) { &LIGHT } else { &DARK }
}

/// Whether the light theme is in use.
pub fn is_light() -> bool {
    LIGHT_ON.load(Ordering::Relaxed)
}

/// The page's background.
pub fn bg() -> Color32 {
    pal().bg
}
pub fn card() -> Color32 {
    pal().card
}
pub fn border() -> Color32 {
    pal().border
}
/// The lines that divide the window into parts (under the title bar, beside the left panels).
pub fn separator() -> Color32 {
    pal().separator
}
pub fn deep() -> Color32 {
    pal().deep
}
pub fn field() -> Color32 {
    pal().field
}
pub fn button() -> Color32 {
    pal().button
}
pub fn button_hover() -> Color32 {
    pal().button_hover
}
pub fn tile() -> Color32 {
    pal().tile
}
pub fn chip() -> Color32 {
    pal().chip
}
pub fn text() -> Color32 {
    pal().text
}
pub fn text_muted() -> Color32 {
    pal().text_muted
}
pub fn text_dim() -> Color32 {
    pal().text_dim
}

/// Text or an icon picked out from the rest (selected, hovered): white on dark, near black on
/// light.
pub fn strong() -> Color32 {
    if is_light() { Color32::from_rgb(0x10, 0x12, 0x16) } else { Color32::WHITE }
}

/// A see-through wash over something hovered: white over dark, black over light.
pub fn wash(alpha: u8) -> Color32 {
    if is_light() { Color32::from_black_alpha(alpha / 2 + alpha / 4) } else { Color32::from_white_alpha(alpha) }
}

/// The lighter cell a labeled field's label sits in, and the well its value is in (the Locate
/// bar, number fields).
pub fn cell() -> Color32 {
    if is_light() { Color32::from_rgb(0xDC, 0xE0, 0xE6) } else { Color32::from_rgb(43, 48, 59) }
}
pub fn well() -> Color32 {
    if is_light() { Color32::from_rgb(0xFB, 0xFB, 0xFC) } else { Color32::from_rgb(12, 14, 18) }
}

/// [`deep`] a little see-through, for cards and panels over the map.
pub fn deep_alpha(alpha: u8) -> Color32 {
    let d = deep();
    Color32::from_rgba_unmultiplied(d.r(), d.g(), d.b(), alpha)
}
pub const GREEN: Color32 = Color32::from_rgb(0x43, 0xA9, 0x75);
pub const GREEN_HOVER: Color32 = Color32::from_rgb(0x52, 0xBF, 0x88);
pub const BLUE: Color32 = Color32::from_rgb(0x3D, 0x7F, 0xE0);
pub const RED: Color32 = Color32::from_rgb(0xE0, 0x52, 0x52);
pub const YELLOW: Color32 = Color32::from_rgb(0xD9, 0xB4, 0x3C);

pub const EDITOR_ACCENT: Color32 = Color32::from_rgb(0x2D, 0xC2, 0x73);
pub const CONVERTER_ACCENT: Color32 = Color32::from_rgb(0x4A, 0x7D, 0xF0);
pub const PRUNER_ACCENT: Color32 = Color32::from_rgb(0xD9, 0x4A, 0x4A);

pub const BADGE_BYTE: Color32 = Color32::from_rgb(0xD8, 0x71, 0x7C);
pub const BADGE_SHORT: Color32 = Color32::from_rgb(0xD8, 0x9A, 0x5A);
pub const BADGE_INT: Color32 = Color32::from_rgb(0x59, 0x5F, 0xDA);
pub const BADGE_LONG: Color32 = Color32::from_rgb(0x6B, 0x77, 0x86);
pub const BADGE_FLOAT: Color32 = Color32::from_rgb(0x4F, 0xB0, 0x9E);
pub const BADGE_DOUBLE: Color32 = Color32::from_rgb(0x9B, 0x6A, 0xD8);
pub const FOLDER: Color32 = Color32::from_rgb(0xB2, 0x90, 0x5E);

/// The color a page uses for what is active or primary in it: green in the NBT editor, blue in
/// the converter, red in the pruner and blue on the other pages.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Accent {
    Green,
    Blue,
    Red,
}

static ACCENT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(1);

/// Sets the accent for everything drawn from now on (the app does this once per frame).
pub fn set_accent(a: Accent) {
    ACCENT.store(a as u8, std::sync::atomic::Ordering::Relaxed);
}

pub fn current_accent() -> Accent {
    match ACCENT.load(std::sync::atomic::Ordering::Relaxed) {
        0 => Accent::Green,
        2 => Accent::Red,
        _ => Accent::Blue,
    }
}

pub fn accent() -> Color32 {
    match current_accent() {
        Accent::Green => GREEN,
        Accent::Blue => BLUE,
        Accent::Red => PRUNER_ACCENT,
    }
}

pub fn accent_hover() -> Color32 {
    match current_accent() {
        Accent::Green => GREEN_HOVER,
        Accent::Blue => Color32::from_rgb(0x55, 0x92, 0xEA),
        Accent::Red => Color32::from_rgb(0xE2, 0x60, 0x60),
    }
}

/// The accent at partial strength over the page background: a quiet selection fill.
pub fn accent_wash(t: f32) -> Color32 {
    let (a, b) = (accent(), bg());
    let mix = |x: u8, y: u8| (y as f32 + (x as f32 - y as f32) * t) as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

/// The accent with the given opacity, for a translucent fill over something.
pub fn accent_alpha(alpha: u8) -> Color32 {
    let a = accent();
    Color32::from_rgba_unmultiplied(a.r(), a.g(), a.b(), alpha)
}

/// A shade darker than [`accent`], for a part that sits against something of the accent color.
pub fn accent_dark() -> Color32 {
    match current_accent() {
        Accent::Green => Color32::from_rgb(0x3A, 0x97, 0x68),
        Accent::Blue => Color32::from_rgb(0x33, 0x69, 0xBD),
        Accent::Red => Color32::from_rgb(0xB9, 0x3E, 0x3E),
    }
}

/// How much smaller a toolbar being squeezed into a narrow window is drawn (1 normally); the
/// buttons and segmented choices in it read this.
static UI_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

pub fn ui_scale() -> f32 {
    f32::from_bits(UI_SCALE.load(std::sync::atomic::Ordering::Relaxed))
}

pub fn set_ui_scale(k: f32) {
    UI_SCALE.store(k.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

pub fn italic(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("italic".into()))
}

pub fn radius(r: u8) -> CornerRadius {
    CornerRadius::same(r)
}

/// Lato (SIL Open Font License, see `assets/fonts/OFL.txt`), with egui's fonts as
/// fallback for symbols.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "lato".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/Lato-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "lato-bold".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/Lato-Bold.ttf"
        ))),
    );
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    fonts
        .families
        .get_mut(&FontFamily::Proportional)
        .expect("proportional family")
        .insert(0, "lato".into());
    fonts.font_data.insert(
        "lato-italic".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/Lato-Italic.ttf"
        ))),
    );
    let mut bold = vec!["lato-bold".to_string()];
    bold.extend(fallback.clone());
    fonts.families.insert(FontFamily::Name("bold".into()), bold);
    let mut italic = vec!["lato-italic".to_string()];
    italic.extend(fallback);
    fonts
        .families
        .insert(FontFamily::Name("italic".into()), italic);
    ctx.set_fonts(fonts);
    set_light(ctx, false);
}

/// Picks the theme: light or dark. The app's own drawing reads the colors above; egui's
/// widgets get a style made from them. (egui keeps a style for each of its themes and follows
/// Windows' light or dark mode: it is told which one is used, so Windows' mode never picks
/// egui's own defaults.)
pub fn set_light(ctx: &egui::Context, light: bool) {
    LIGHT_ON.store(light, Ordering::Relaxed);
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        // Labels are not selectable text (no I-beam cursor over them).
        s.interaction.selectable_labels = false;
        let v = &mut s.visuals;
        *v = if light { egui::Visuals::light() } else { egui::Visuals::dark() };
        v.panel_fill = bg();
        v.window_fill = card();
        v.window_stroke = Stroke::new(1.0_f32, border());
        v.extreme_bg_color = field();
        v.faint_bg_color = card();
        v.code_bg_color = field();
        v.override_text_color = Some(text());
        v.selection.bg_fill = BLUE;
        v.selection.stroke = Stroke::new(1.0_f32, text());
        v.hyperlink_color = BLUE;
        // Anything that can be clicked shows the pointing hand, as in the original tool.
        v.interact_cursor = Some(egui::CursorIcon::PointingHand);
        v.window_corner_radius = radius(6);
        v.menu_corner_radius = radius(4);
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = radius(4);
        }
        v.widgets.noninteractive.bg_fill = card();
        v.widgets.noninteractive.weak_bg_fill = card();
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, border());
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, text());
        v.widgets.inactive.bg_fill = button();
        v.widgets.inactive.weak_bg_fill = button();
        v.widgets.inactive.bg_stroke = Stroke::NONE;
        v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, text());
        v.widgets.hovered.bg_fill = button_hover();
        v.widgets.hovered.weak_bg_fill = button_hover();
        v.widgets.hovered.bg_stroke = Stroke::NONE;
        v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, text());
        v.widgets.active.bg_fill = chip();
        v.widgets.active.weak_bg_fill = chip();
        v.widgets.active.fg_stroke = Stroke::new(1.0_f32, text());
        v.widgets.open.weak_bg_fill = button_hover();
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
        s.spacing.interact_size.y = 26.0;
        s.text_styles = [
            (TextStyle::Small, regular(11.0)),
            (TextStyle::Body, regular(14.0)),
            (TextStyle::Button, regular(14.0)),
            (TextStyle::Heading, regular(22.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
    });
}
