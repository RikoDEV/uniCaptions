//! General tab: audio source + test, speech recognition, app behaviour.

use super::{apikey, field, row, Cx};
use crate::i18n::UI_LANGUAGES;
use crate::icons::{Flag, Icon};
use crate::state::CaptioningStatus;
use crate::widgets::{self, card_select, CardOption};
use eframe::egui::Ui;
use unicaptions_core::secrets::ApiProvider;
use unicaptions_core::settings::{AudioSource, Provider, WhisperSize};
use unicaptions_core::ModelKind;

/// Spoken-language choices (`auto` = detect).
pub const LANGUAGE_CODES: [&str; 9] = ["auto", "en", "es", "fr", "de", "pl", "pt", "zh", "ja"];

/// Maps a stored code onto its `'static` entry, or `""` if unknown.
pub fn static_code<'a>(code: &str, list: &[&'a str]) -> &'a str {
    list.iter().copied().find(|c| *c == code).unwrap_or("")
}

pub fn language_options(cx: &Cx, codes: &[&'static str]) -> Vec<CardOption<&'static str>> {
    codes
        .iter()
        .map(|&code| {
            CardOption::new(code, cx.t.t(&format!("languages.{code}"))).flag(Flag::from_code(code))
        })
        .collect()
}

pub fn draw(ui: &mut Ui, cx: &mut Cx) {
    audio_panel(ui, cx);
    speech_panel(ui, cx);
    behavior_panel(ui, cx);
}

fn audio_panel(ui: &mut Ui, cx: &mut Cx) {
    let pal = cx.pal;
    widgets::panel(ui, &pal, |ui| {
        widgets::panel_title(ui, &pal, &cx.t.t("general.audioTitle"));

        field(ui, &cx.t.t("general.audioSource"), |ui| {
            let options = [
                CardOption::new(AudioSource::Microphone, cx.t.t("general.microphone")).icon(Icon::Mic),
                CardOption::new(AudioSource::System, cx.t.t("general.systemAudio")).icon(Icon::Speaker),
            ];
            if let Some(v) = card_select(ui, &pal, cx.settings.audio_source, &options, false) {
                cx.settings.audio_source = v;
            }
        });

        let source_name = match cx.settings.audio_source {
            AudioSource::Microphone => cx.t.t("general.microphone"),
            AudioSource::System => cx.t.t("general.systemAudio"),
        };
        field(
            ui,
            &cx.t.tr("general.audioTestLabel", &[("source", &source_name)]),
            |ui| {
                ui.horizontal(|ui| {
                    let label = if cx.st.mic_active {
                        cx.t.t("general.audioTestStop")
                    } else {
                        cx.t.t("general.audioTestStart")
                    };
                    let enabled = cx.st.captioning == CaptioningStatus::Idle;
                    if widgets::plain_button(ui, &label, enabled).clicked() {
                        toggle_mic(cx);
                    }
                    let fraction = (cx.st.mic_level * 4.0).min(1.0);
                    widgets::level_meter(ui, &pal, fraction, ui.available_width());
                });
                widgets::hint(ui, &pal, &cx.t.t("general.audioTestHint"));
            },
        );

        field(ui, &cx.t.t("general.spokenLanguage"), |ui| {
            let options = language_options(cx, &LANGUAGE_CODES);
            let current = static_code(&cx.settings.source_language, &LANGUAGE_CODES);
            if let Some(v) = card_select(ui, &pal, current, &options, true) {
                cx.settings.source_language = v.to_string();
            }
        });
    });
}

fn toggle_mic(cx: &mut Cx) {
    if cx.st.mic_active {
        cx.engine.stop_audio_capture();
        cx.st.mic_active = false;
        cx.st.mic_level = 0.0;
    } else {
        match cx.engine.start_audio_capture(cx.settings.audio_source) {
            Ok(()) => cx.st.mic_active = true,
            Err(e) => cx.st.error = Some(cx.t.tr("errors.generic", &[("error", &e)])),
        }
    }
}

fn whisper_label_key(size: WhisperSize) -> (&'static str, &'static str) {
    match size {
        WhisperSize::Tiny => ("general.modelTiny", "general.modelTinyDesc"),
        WhisperSize::Base => ("general.modelBase", "general.modelBaseDesc"),
        WhisperSize::Small => ("general.modelSmall", "general.modelSmallDesc"),
        WhisperSize::Medium => ("general.modelMedium", "general.modelMediumDesc"),
    }
}

fn whisper_icon(size: WhisperSize) -> Icon {
    match size {
        WhisperSize::Tiny => Icon::TierTiny,
        WhisperSize::Base => Icon::TierBase,
        WhisperSize::Small => Icon::TierSmall,
        WhisperSize::Medium => Icon::TierMedium,
    }
}

fn is_whisper_downloaded(cx: &Cx, size: WhisperSize) -> bool {
    cx.st
        .models
        .iter()
        .any(|m| m.kind == ModelKind::Whisper && m.id == size.as_str() && m.downloaded)
}

fn speech_panel(ui: &mut Ui, cx: &mut Cx) {
    let pal = cx.pal;
    widgets::panel(ui, &pal, |ui| {
        widgets::panel_title(ui, &pal, &cx.t.t("general.speechTitle"));

        field(ui, &cx.t.t("general.provider"), |ui| {
            let options = [
                CardOption::new(Provider::Local, cx.t.t("general.providerLocal")).icon(Icon::Offline),
                CardOption::new(Provider::Cloud, cx.t.t("general.providerCloud")).icon(Icon::Cloud),
            ];
            if let Some(v) = card_select(ui, &pal, cx.settings.asr_provider, &options, false) {
                cx.settings.asr_provider = v;
            }
        });

        if cx.settings.asr_provider == Provider::Local {
            field(ui, &cx.t.t("general.model"), |ui| {
                let options: Vec<CardOption<WhisperSize>> = WhisperSize::ALL
                    .iter()
                    .map(|&size| {
                        let (label, desc) = whisper_label_key(size);
                        CardOption::new(size, cx.t.t(label))
                            .icon(whisper_icon(size))
                            .description(cx.t.t(desc))
                            .downloaded(is_whisper_downloaded(cx, size))
                            .progress(cx.st.download_progress(ModelKind::Whisper, size.as_str()))
                    })
                    .collect();
                if let Some(size) = card_select(ui, &pal, cx.settings.asr_model, &options, false) {
                    cx.settings.asr_model = size;
                    select_asr_model(cx, size);
                }
            });
        } else {
            apikey::draw(ui, cx, ApiProvider::OpenAiAsr, &cx.t.t("general.openaiKeyLabel"));
        }

        widgets::hint(ui, &pal, &cx.t.t("general.speechHint"));
    });
}

/// Choosing a model that isn't on disk starts downloading it right away.
fn select_asr_model(cx: &mut Cx, size: WhisperSize) {
    let downloading = cx.st.download_progress(ModelKind::Whisper, size.as_str()).is_some();
    if is_whisper_downloaded(cx, size) || downloading {
        return;
    }
    cx.st
        .progress
        .insert((ModelKind::Whisper, size.as_str().to_string()), 0.0);
    cx.engine.download_model(ModelKind::Whisper, size.as_str().to_string());
}

fn behavior_panel(ui: &mut Ui, cx: &mut Cx) {
    let pal = cx.pal;
    widgets::panel(ui, &pal, |ui| {
        widgets::panel_title(ui, &pal, &cx.t.t("general.appBehaviorTitle"));

        ui.vertical(|ui| {
            row(ui, &cx.t.t("general.clickThrough"), |ui| {
                widgets::toggle(ui, &pal, &mut cx.settings.click_through, true);
            });
            widgets::hint(ui, &pal, &cx.t.t("general.clickThroughHint"));
        });

        let mut autostart = cx
            .st
            .autostart
            .unwrap_or_else(|| cx.autostart.is_enabled());
        cx.st.autostart = Some(autostart);
        row(ui, &cx.t.t("general.launchAtStartup"), |ui| {
            if widgets::toggle(ui, &pal, &mut autostart, true).changed() {
                match cx.autostart.set_enabled(autostart) {
                    Ok(()) => cx.st.autostart = Some(autostart),
                    Err(e) => {
                        cx.st.autostart = Some(cx.autostart.is_enabled());
                        cx.st.error = Some(cx.t.tr("errors.generic", &[("error", &e)]));
                    }
                }
            }
        });

        row(ui, &cx.t.t("general.autoStartCaptioning"), |ui| {
            widgets::toggle(ui, &pal, &mut cx.settings.auto_start_captioning, true);
        });

        field(ui, &cx.t.t("general.uiLanguage"), |ui| {
            let options = language_options(cx, &UI_LANGUAGES);
            let current = static_code(&cx.settings.ui_language, &UI_LANGUAGES);
            if let Some(v) = card_select(ui, &pal, current, &options, true) {
                cx.settings.ui_language = v.to_string();
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_code_maps_known_and_unknown() {
        assert_eq!(static_code("pl", &LANGUAGE_CODES), "pl");
        assert_eq!(static_code("xx", &LANGUAGE_CODES), "");
        assert_eq!(static_code("auto", &LANGUAGE_CODES), "auto");
    }

    #[test]
    fn whisper_keys_exist_in_locales() {
        let t = crate::i18n::I18n::new("en");
        for size in WhisperSize::ALL {
            let (l, d) = whisper_label_key(size);
            assert_ne!(t.t(l), l);
            assert_ne!(t.t(d), d);
        }
    }
}
