//! About tab: version, author, credits.

use super::Cx;
use crate::widgets;
use eframe::egui::{self, include_image, vec2, Align, Layout, RichText, Ui};

pub const AUTHOR_URL: &str = "https://riko.dev";

/// (name key or literal, description key). Names with a key are translated.
const CREDITS: [(&str, &str); 4] = [
    ("credits.egui", "credits.eguiDesc"),
    ("credits.whisperCpp", "credits.whisperCppDesc"),
    ("credits.onnx", "credits.onnxDesc"),
    ("credits.opusMt", "credits.opusMtDesc"),
];

pub fn draw(ui: &mut Ui, cx: &mut Cx) {
    let pal = cx.pal;
    widgets::panel(ui, &pal, |ui| {
        ui.with_layout(Layout::top_down(Align::Center), |ui| {
            ui.add(
                egui::Image::new(include_image!("../../../../assets/app-icon.png"))
                    .fit_to_exact_size(vec2(72.0, 72.0))
                    .corner_radius(16),
            );
            ui.label(RichText::new("uniCaptions").size(20.0).strong());
            ui.label(
                RichText::new(
                    cx.t.tr("about.version", &[("version", env!("CARGO_PKG_VERSION"))]),
                )
                .color(pal.text_faint),
            );
            // Centre "Created by RikoDEV" (label + link) by measuring it first.
            let created_by = cx.t.t("about.createdBy");
            let font = egui::TextStyle::Body.resolve(ui.style());
            let width = ui.fonts_mut(|f| {
                let a = f.layout_no_wrap(created_by.clone(), font.clone(), pal.text).size().x;
                let b = f.layout_no_wrap("RikoDEV".into(), font.clone(), pal.text).size().x;
                a + b + ui.spacing().item_spacing.x
            });
            ui.allocate_ui_with_layout(
                vec2(width + 2.0, ui.spacing().interact_size.y),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.label(&created_by);
                    ui.hyperlink_to("RikoDEV", AUTHOR_URL);
                },
            );
        });
    });

    widgets::panel(ui, &pal, |ui| {
        widgets::panel_title(ui, &pal, &cx.t.t("credits.title"));
        for (name, desc) in CREDITS {
            ui.horizontal(|ui| {
                ui.label(RichText::new(cx.t.t(name)).strong());
                ui.label(RichText::new(cx.t.t(desc)).size(12.5).color(pal.text_faint));
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credit_keys_exist_in_every_locale() {
        for lang in crate::i18n::UI_LANGUAGES {
            let t = crate::i18n::I18n::new(lang);
            for (name, desc) in CREDITS {
                assert_ne!(t.t(name), name, "{lang}: {name}");
                assert_ne!(t.t(desc), desc, "{lang}: {desc}");
            }
        }
    }
}
