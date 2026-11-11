//! Translation tab.

use super::general::{static_code, LANGUAGE_CODES};
use super::{apikey, combo, row, Cx};
use crate::widgets;
use eframe::egui::Ui;
use unicaptions_core::secrets::ApiProvider;
use unicaptions_core::settings::Provider;
use unicaptions_core::Engine;

pub fn draw(ui: &mut Ui, cx: &mut Cx) {
    let pal = cx.pal;
    widgets::panel(ui, &pal, |ui| {
        row(ui, &cx.t.t("translation.enable"), |ui| {
            widgets::toggle(ui, &pal, &mut cx.settings.translation_enabled, true);
        });
        let enabled = cx.settings.translation_enabled;

        super::field(ui, &cx.t.t("translation.translateInto"), |ui| {
            let options: Vec<(&'static str, String)> = LANGUAGE_CODES
                .iter()
                .filter(|c| **c != "auto")
                .map(|&c| (c, cx.t.t(&format!("languages.{c}"))))
                .collect();
            let mut current = static_code(&cx.settings.target_language, &LANGUAGE_CODES);
            if combo(ui, "translate-target", &mut current, &options, enabled) {
                cx.settings.target_language = current.to_string();
            }
        });

        super::field(ui, &cx.t.t("translation.provider"), |ui| {
            let options = [
                (Provider::Local, cx.t.t("translation.providerLocal")),
                (Provider::Cloud, cx.t.t("translation.providerCloud")),
            ];
            combo(
                ui,
                "translate-provider",
                &mut cx.settings.translate_provider,
                &options,
                enabled,
            );
        });

        row(ui, &cx.t.t("translation.showOriginal"), |ui| {
            widgets::toggle(
                ui,
                &pal,
                &mut cx.settings.caption_style.show_original,
                enabled,
            );
        });

        if enabled && cx.settings.translate_provider == Provider::Cloud {
            apikey::draw(
                ui,
                cx,
                ApiProvider::DeepL,
                &cx.t.t("translation.deeplKeyLabel"),
            );
        }

        if enabled
            && cx.settings.translate_provider == Provider::Local
            && !Engine::is_translation_supported(&cx.settings.target_language)
        {
            widgets::hint(ui, &pal, &cx.t.t("translation.localUnsupportedHint"));
        }

        widgets::hint(ui, &pal, &cx.t.t("translation.localAssumptionHint"));
    });
}
