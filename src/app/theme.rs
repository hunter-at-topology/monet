//! Warm paper, soft ink, one coral accent.

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, Stroke, Visuals};

use super::highlight::SyntaxTheme;

pub const PAPER: Color32 = Color32::from_rgb(0xFB, 0xF9, 0xF5);
pub const SURROUND: Color32 = Color32::from_rgb(0xF0, 0xEC, 0xE4);
pub const GLASS: Color32 = Color32::from_rgba_premultiplied(0xF0, 0xEE, 0xEA, 0xF2);
pub const HOVER: Color32 = Color32::from_rgb(0xF1, 0xED, 0xE6);
pub const LINE: Color32 = Color32::from_rgb(0xE6, 0xE0, 0xD5);
pub const INK: Color32 = Color32::from_rgb(0x2A, 0x29, 0x26);
pub const MUTED: Color32 = Color32::from_rgb(0x85, 0x7F, 0x75);
pub const FAINT: Color32 = Color32::from_rgb(0xB9, 0xB2, 0xA5);
pub const GUTTER: Color32 = Color32::from_rgb(0xCF, 0xC9, 0xBE);
pub const ACCENT: Color32 = Color32::from_rgb(0xE2, 0x55, 0x3F);
pub const AMBER: Color32 = Color32::from_rgb(0xE0, 0xA4, 0x3A);
pub const ERR: Color32 = Color32::from_rgb(0xCF, 0x4A, 0x3A);
pub const ERR_BG: Color32 = Color32::from_rgb(0xFB, 0xEA, 0xE4);

pub fn syntax_theme() -> SyntaxTheme {
    SyntaxTheme {
        text: INK,
        comment: Color32::from_rgb(0xAE, 0xA8, 0x9B),
        number: Color32::from_rgb(0xC9, 0x6A, 0x2B),
        quote: Color32::from_rgb(0x9A, 0x93, 0x86),
        word: Color32::from_rgb(0x2E, 0x80, 0x86),
        source: Color32::from_rgb(0x3B, 0x63, 0xA6),
        effect: Color32::from_rgb(0x7A, 0x57, 0xA8),
        signal: Color32::from_rgb(0xB0, 0x42, 0x67),
        func: Color32::from_rgb(0xB5, 0x86, 0x3B),
        punct: Color32::from_rgb(0x8E, 0x88, 0x7D),
        active_bg: Color32::from_rgba_unmultiplied(0xE2, 0x55, 0x3F, 0x30),
        active_ink: Color32::from_rgb(0xB8, 0x3A, 0x26),
        error: ERR,
    }
}

/// Prefer the system's SF fonts on macOS; fall back to egui's bundled ones.
/// In the browser, code is set in JetBrains Mono (SIL Open Font License 1.1).
fn fonts() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    #[cfg(target_arch = "wasm32")]
    {
        defs.font_data.insert(
            "mono".into(),
            FontData::from_static(include_bytes!("../../web/fonts/JetBrainsMono.ttf")).into(),
        );
        defs.families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, "mono".into());
    }
    let try_load = |paths: &[&str]| paths.iter().find_map(|p| std::fs::read(p).ok());
    if let Some(bytes) = try_load(&[
        "/System/Library/Fonts/SFNSMono.ttf",
        "/System/Library/Fonts/Menlo.ttc",
    ]) {
        defs.font_data
            .insert("mono".into(), FontData::from_owned(bytes).into());
        defs.families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, "mono".into());
    }
    if let Some(bytes) = try_load(&["/System/Library/Fonts/SFNS.ttf"]) {
        defs.font_data
            .insert("sans".into(), FontData::from_owned(bytes).into());
        defs.families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "sans".into());
    }
    defs
}

pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    let mut v = Visuals::light();
    v.panel_fill = PAPER;
    v.window_fill = PAPER;
    v.extreme_bg_color = PAPER;
    v.faint_bg_color = HOVER;
    v.override_text_color = None;
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(0xE2, 0x55, 0x3F, 0x38);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.text_cursor.stroke = Stroke::new(2.0, ACCENT);
    v.hyperlink_color = ACCENT;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.slider_trailing_fill = true;
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
    ] {
        w.bg_stroke = Stroke::NONE;
        w.expansion = 0.0;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.inactive.bg_fill = LINE; // slider rail
    v.widgets.inactive.weak_bg_fill = LINE;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, INK);
    v.widgets.hovered.bg_fill = INK; // slider knob on hover
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.active.bg_fill = ACCENT;
    v.widgets.active.weak_bg_fill = HOVER;
    ctx.set_visuals_of(egui::Theme::Light, v);
    ctx.set_theme(egui::ThemePreference::Light);
    ctx.global_style_mut(|s| {
        s.spacing.slider_width = 222.0;
        s.spacing.slider_rail_height = 3.0;
        s.spacing.interact_size.y = 18.0;
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.interaction.selectable_labels = false;
    });
}
