//! Caption Style tab.

use super::{combo, field, font_picker, full_slider, row, Cx};
use crate::color::{parse_hex, to_hex};
use crate::widgets;
use eframe::egui::{self, Ui};

const WEIGHTS: [(u32, &str); 4] = [
    (400, "captionStyle.weightRegular"),
    (600, "captionStyle.weightSemibold"),
    (700, "captionStyle.weightBold"),
    (800, "captionStyle.weightExtraBold"),
];

fn color_row(ui: &mut Ui, label: &str, hex: &mut String, fallback: [u8; 3]) {
    row(ui, label, |ui| {
        let mut rgb = parse_hex(hex, fallback);
        if egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed() {
            *hex = to_hex(rgb);
        }
    });
}

pub fn draw(ui: &mut Ui, cx: &mut Cx) {
    let pal = cx.pal;
    widgets::panel(ui, &pal, |ui| {
        field(ui, &cx.t.t("captionStyle.fontFamily"), |ui| {
            if let Some(family) = font_picker::draw(ui, cx) {
                cx.settings.caption_style.font_family = family;
            }
        });

        let size_label = cx.t.tr(
            "captionStyle.fontSize",
            &[(
                "size",
                &format!("{}", cx.settings.caption_style.font_size.round() as i32),
            )],
        );
        field(ui, &size_label, |ui| {
            full_slider(
                ui,
                egui::Slider::new(&mut cx.settings.caption_style.font_size, 14.0..=64.0)
                    .step_by(1.0),
            );
        });

        field(ui, &cx.t.t("captionStyle.fontWeight"), |ui| {
            let options: Vec<(u32, String)> =
                WEIGHTS.iter().map(|(w, key)| (*w, cx.t.t(key))).collect();
            combo(
                ui,
                "caption-weight",
                &mut cx.settings.caption_style.font_weight,
                &options,
                true,
            );
        });

        color_row(
            ui,
            &cx.t.t("captionStyle.textColor"),
            &mut cx.settings.caption_style.text_color,
            [255, 255, 255],
        );
        color_row(
            ui,
            &cx.t.t("captionStyle.backgroundColor"),
            &mut cx.settings.caption_style.background_color,
            [0, 0, 0],
        );

        let pct = (cx.settings.caption_style.background_opacity * 100.0).round() as i32;
        field(
            ui,
            &cx.t.tr(
                "captionStyle.backgroundOpacity",
                &[("opacity", &pct.to_string())],
            ),
            |ui| {
                full_slider(
                    ui,
                    egui::Slider::new(&mut cx.settings.caption_style.background_opacity, 0.0..=1.0)
                        .step_by(0.05),
                );
            },
        );

        let lines = cx.settings.caption_style.max_lines;
        field(
            ui,
            &cx.t
                .tr("captionStyle.maxLines", &[("lines", &lines.to_string())]),
            |ui| {
                full_slider(
                    ui,
                    egui::Slider::new(&mut cx.settings.caption_style.max_lines, 1..=5),
                );
            },
        );

        row(ui, &cx.t.t("captionStyle.textOutline"), |ui| {
            widgets::toggle(ui, &pal, &mut cx.settings.caption_style.outline, true);
        });

        widgets::hint(ui, &pal, &cx.t.t("captionStyle.dragHint"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_keys_exist() {
        let t = crate::i18n::I18n::new("en");
        for (_, key) in WEIGHTS {
            assert_ne!(t.t(key), key);
        }
    }
}
