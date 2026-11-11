//! Colours and egui styling, ported from the design tokens in the old
//! `App.css` (`--bg`, `--accent`, ...), with light and dark variants.

use eframe::egui::{self, Color32, CornerRadius, Stroke, Theme};

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub bg_elevated: Color32,
    pub sidebar_bg: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub accent_contrast: Color32,
    pub danger: Color32,
    pub live: Color32,
    pub hover: Color32,
}

fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, (a * 255.0).round() as u8)
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            bg: Color32::from_rgb(0x1c, 0x1d, 0x22),
            bg_elevated: Color32::from_rgb(0x23, 0x24, 0x29),
            sidebar_bg: Color32::from_rgb(0x18, 0x19, 0x1d),
            border: rgba(255, 255, 255, 0.08),
            text: Color32::from_rgb(0xf2, 0xf2, 0xf4),
            text_muted: rgba(0xf2, 0xf2, 0xf4, 0.62),
            text_faint: rgba(0xf2, 0xf2, 0xf4, 0.40),
            accent: Color32::from_rgb(0xa3, 0xe6, 0x35),
            accent_contrast: Color32::from_rgb(0x0d, 0x1a, 0x03),
            danger: Color32::from_rgb(0xe5, 0x48, 0x4d),
            live: Color32::from_rgb(0x2d, 0xd4, 0xa7),
            hover: rgba(127, 127, 127, 0.14),
        }
    } else {
        Palette {
            bg: Color32::from_rgb(0xf4, 0xf5, 0xf7),
            bg_elevated: Color32::WHITE,
            sidebar_bg: Color32::from_rgb(0xec, 0xee, 0xf2),
            border: rgba(15, 15, 20, 0.09),
            text: Color32::from_rgb(0x17, 0x18, 0x1c),
            text_muted: rgba(0x17, 0x18, 0x1c, 0.62),
            text_faint: rgba(0x17, 0x18, 0x1c, 0.45),
            accent: Color32::from_rgb(0x65, 0xa3, 0x0d),
            accent_contrast: Color32::from_rgb(0x0d, 0x1a, 0x03),
            danger: Color32::from_rgb(0xe5, 0x48, 0x4d),
            live: Color32::from_rgb(0x2d, 0xd4, 0xa7),
            hover: rgba(127, 127, 127, 0.10),
        }
    }
}

pub fn palette_for(ctx: &egui::Context) -> Palette {
    palette(ctx.theme() == Theme::Dark)
}

fn visuals_for(theme: Theme) -> egui::Visuals {
    let dark = theme == Theme::Dark;
    let p = palette(dark);
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let radius = CornerRadius::same(6);

    v.panel_fill = p.bg;
    v.window_fill = p.bg_elevated;
    v.extreme_bg_color = p.bg;
    v.faint_bg_color = p.hover;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent.gamma_multiply(0.45);
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = radius;

    let border = Stroke::new(1.0, p.border);
    v.widgets.noninteractive.bg_stroke = border;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.noninteractive.corner_radius = radius;

    v.widgets.inactive.bg_fill = p.bg;
    v.widgets.inactive.weak_bg_fill = p.bg;
    v.widgets.inactive.bg_stroke = border;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.inactive.corner_radius = radius;

    v.widgets.hovered.bg_fill = p.hover;
    v.widgets.hovered.weak_bg_fill = p.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.hovered.corner_radius = radius;

    // egui draws `strong()` text in the *active* widget colour, so keep that the
    // normal text colour and tint the pressed background instead.
    v.widgets.active.bg_fill = p.accent.gamma_multiply(0.4);
    v.widgets.active.weak_bg_fill = p.accent.gamma_multiply(0.4);
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    v.widgets.active.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.active.corner_radius = radius;

    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;

    v.widgets.open.bg_fill = p.bg;
    v.widgets.open.weak_bg_fill = p.bg;
    v.widgets.open.bg_stroke = Stroke::new(1.0, p.accent);
    v.widgets.open.corner_radius = radius;

    v
}

/// Installs the visuals and text sizes for both themes.
pub fn install(ctx: &egui::Context) {
    use egui::{FontFamily, FontId, TextStyle};
    for theme in [Theme::Light, Theme::Dark] {
        ctx.set_visuals_of(theme, visuals_for(theme));
        ctx.style_mut_of(theme, |style| {
            style.spacing.item_spacing = egui::vec2(8.0, 8.0);
            style.spacing.button_padding = egui::vec2(12.0, 6.0);
            style.spacing.interact_size.y = 30.0;
            style.spacing.slider_width = 260.0;
            style.text_styles = [
                (
                    TextStyle::Heading,
                    FontId::new(22.0, FontFamily::Proportional),
                ),
                (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
                (
                    TextStyle::Button,
                    FontId::new(14.0, FontFamily::Proportional),
                ),
                (
                    TextStyle::Small,
                    FontId::new(12.0, FontFamily::Proportional),
                ),
                (
                    TextStyle::Monospace,
                    FontId::new(13.0, FontFamily::Monospace),
                ),
            ]
            .into();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_and_dark_palettes_differ() {
        let (l, d) = (palette(false), palette(true));
        assert_ne!(l.bg, d.bg);
        assert_ne!(l.accent, d.accent);
        assert_eq!(l.accent, Color32::from_rgb(0x65, 0xa3, 0x0d));
        assert_eq!(d.accent, Color32::from_rgb(0xa3, 0xe6, 0x35));
    }

    #[test]
    fn install_does_not_panic() {
        let ctx = egui::Context::default();
        install(&ctx);
    }
}
