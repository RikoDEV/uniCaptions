//! Translation tab.

use super::general::{static_code, LANGUAGE_CODES};
use super::{apikey, combo, row, Cx};
use crate::widgets;
use eframe::egui::Ui;
use unicaptions_core::secrets::ApiProvider;
use unicaptions_core::settings::Provider;
use unicaptions_core::Engine;

/// Will local translation be unable to do this combination? With an automatic
/// spoken language, only the target matters (can *any* language reach it?).
pub fn local_translation_unsupported(source: &str, target: &str) -> bool {
    if source == target {
        return false; // nothing to translate, not an error
    }
    if source == "auto" {
        !unicaptions_core::translate::local_supports_target(target)
    } else {
        !Engine::local_translation_available(source, target)
    }
}

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
            && local_translation_unsupported(
                &cx.settings.source_language,
                &cx.settings.target_language,
            )
        {
            widgets::hint(ui, &pal, &cx.t.t("translation.localUnsupportedHint"));
        }

        widgets::hint(ui, &pal, &cx.t.t("translation.localAssumptionHint"));
    });
}

#[cfg(test)]
mod tests {
    use super::local_translation_unsupported as unsupported;

    #[test]
    fn hint_rules() {
        // the user's case: Polish speech into English works locally
        assert!(!unsupported("pl", "en"));
        assert!(!unsupported("auto", "en"));
        assert!(!unsupported("pl", "de")); // pivot through English
                                           // no model produces Polish / Portuguese / Japanese
        assert!(unsupported("en", "pl"));
        assert!(unsupported("auto", "pl"));
        assert!(unsupported("de", "pt"));
        assert!(unsupported("pt", "en"));
        // same language is not "unsupported"
        assert!(!unsupported("pl", "pl"));
    }
}
