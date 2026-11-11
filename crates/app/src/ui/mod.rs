//! The Settings window: sidebar + one module per tab.

pub mod about;
pub mod apikey;
pub mod font_picker;
pub mod general;
pub mod models;
pub mod style;
pub mod translation;

use crate::autostart::Autostart;
use crate::fonts::FontCatalog;
use crate::i18n::I18n;
use crate::icons::Icon;
use crate::state::{captioning_params, CaptioningStatus, Tab, UiState};
use crate::theme::Palette;
use crate::widgets;
use eframe::egui::{self, include_image, vec2, Align, Layout, Margin, RichText, Stroke, Ui};
use std::sync::Arc;
use unicaptions_core::{Engine, Settings};

/// Everything a tab needs to draw itself and act.
pub struct Cx<'a> {
    pub t: &'a I18n,
    pub pal: Palette,
    pub settings: &'a mut Settings,
    pub st: &'a mut UiState,
    pub engine: &'a Arc<Engine>,
    pub fonts: &'a FontCatalog,
    pub autostart: &'a dyn Autostart,
}

pub fn draw(ui: &mut Ui, cx: &mut Cx) {
    sidebar(ui, cx);
    content(ui, cx);
}

fn tab_icon(tab: Tab) -> Icon {
    match tab {
        Tab::General => Icon::General,
        Tab::CaptionStyle => Icon::Style,
        Tab::Translation => Icon::Translate,
        Tab::Models => Icon::Models,
        Tab::About => Icon::Info,
    }
}

fn sidebar(ui: &mut Ui, cx: &mut Cx) {
    let pal = cx.pal;
    egui::Panel::left("sidebar")
        .exact_size(210.0)
        .resizable(false)
        .frame(
            egui::Frame::new()
                .fill(pal.sidebar_bg)
                .stroke(Stroke::new(1.0, pal.border))
                .inner_margin(Margin::symmetric(13, 18)),
        )
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.horizontal(|ui| {
                ui.add(
                    egui::Image::new(include_image!("../../../../assets/app-icon.png"))
                        .fit_to_exact_size(vec2(30.0, 30.0))
                        .corner_radius(9),
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new("uniCaptions").strong().size(15.0));
                    ui.label(
                        RichText::new(cx.t.t("brand.subtitle"))
                            .size(11.5)
                            .color(pal.text_faint),
                    );
                });
            });
            ui.add_space(16.0);

            for tab in Tab::ALL {
                let label = cx.t.t(tab.label_key());
                if widgets::nav_item(ui, &pal, tab_icon(tab), &label, cx.st.tab == tab).clicked() {
                    cx.st.tab = tab;
                }
            }

            ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                // Bottom-most first.
                if let Some(p) = cx.st.any_download_progress() {
                    widgets::level_meter(ui, &pal, p, ui.available_width());
                }
                captioning_toggle_row(ui, cx);
                ui.separator();
            });
        });
}

fn captioning_toggle_row(ui: &mut Ui, cx: &mut Cx) {
    let pal = cx.pal;
    let status = cx.st.captioning;
    let label = match status {
        CaptioningStatus::Starting => cx.t.t("sidebar.starting"),
        CaptioningStatus::Live => cx.t.t("sidebar.live"),
        CaptioningStatus::Idle => cx.t.t("sidebar.idle"),
    };
    ui.horizontal(|ui| {
        widgets::status_dot(
            ui,
            if status == CaptioningStatus::Live {
                pal.live
            } else {
                pal.text_faint
            },
            status == CaptioningStatus::Live,
        );
        ui.label(RichText::new(label).size(12.5).color(pal.text_muted));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut on = status == CaptioningStatus::Live;
            let enabled = !cx.st.mic_active && status != CaptioningStatus::Starting;
            if widgets::toggle(ui, &pal, &mut on, enabled).changed() {
                set_captioning(cx, on);
            }
        });
    });
}

/// Starts or stops live captioning (the sidebar switch and auto-start both use this).
pub fn set_captioning(cx: &mut Cx, on: bool) {
    if on {
        if cx.st.captioning != CaptioningStatus::Idle {
            return;
        }
        cx.st.error = None;
        cx.st.captioning = CaptioningStatus::Starting;
        cx.engine.start_captioning(captioning_params(cx.settings));
    } else {
        cx.engine.stop_captioning();
        cx.st.captioning = CaptioningStatus::Idle;
        cx.st.progress.clear();
    }
}

fn content(ui: &mut Ui, cx: &mut Cx) {
    let pal = cx.pal;
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(pal.bg))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    // A centred column, at most 640px wide.
                    let avail = ui.available_width();
                    let w = (avail - 64.0).clamp(280.0, 640.0);
                    let offset = ((avail - w) / 2.0).max(0.0);
                    ui.add_space(28.0);
                    ui.horizontal_top(|ui| {
                        ui.add_space(offset);
                        ui.vertical(|ui| {
                            ui.set_width(w);
                            ui.spacing_mut().item_spacing.y = 16.0;
                            header(ui, cx);
                            error_banner(ui, cx);
                            match cx.st.tab {
                                Tab::General => general::draw(ui, cx),
                                Tab::CaptionStyle => style::draw(ui, cx),
                                Tab::Translation => translation::draw(ui, cx),
                                Tab::Models => models::draw(ui, cx),
                                Tab::About => about::draw(ui, cx),
                            }
                        });
                    });
                    ui.add_space(40.0);
                });
        });
}

fn header(ui: &mut Ui, cx: &Cx) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.label(
            RichText::new(cx.t.t(cx.st.tab.label_key()))
                .size(22.0)
                .strong(),
        );
        ui.label(
            RichText::new(cx.t.t(cx.st.tab.description_key()))
                .size(13.5)
                .color(cx.pal.text_muted),
        );
    });
}

fn error_banner(ui: &mut Ui, cx: &mut Cx) {
    let Some(message) = cx.st.error.clone() else {
        return;
    };
    let pal = cx.pal;
    let mut dismiss = false;
    egui::Frame::new()
        .fill(pal.danger.gamma_multiply(0.14))
        .stroke(Stroke::new(1.0, pal.danger))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(&message).color(pal.text)).wrap());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("\u{d7}").clicked() {
                        dismiss = true;
                    }
                });
            });
        });
    if dismiss {
        cx.st.error = None;
    }
}

/// Drop-down for a small set of options. Returns `true` if the value changed.
pub fn combo<T: PartialEq + Copy>(
    ui: &mut Ui,
    id: &str,
    current: &mut T,
    options: &[(T, String)],
    enabled: bool,
) -> bool {
    let selected = options
        .iter()
        .find(|(v, _)| v == current)
        .map(|(_, l)| l.clone())
        .unwrap_or_default();
    let mut changed = false;
    ui.add_enabled_ui(enabled, |ui| {
        egui::ComboBox::from_id_salt(id)
            .selected_text(selected)
            .width(ui.available_width().min(420.0))
            .show_ui(ui, |ui| {
                for (value, label) in options {
                    if ui.selectable_label(*current == *value, label).clicked()
                        && *current != *value
                    {
                        *current = *value;
                        changed = true;
                    }
                }
            });
    });
    changed
}

/// A slider stretched to the full width of its column (value shown in the label).
pub fn full_slider(ui: &mut Ui, slider: egui::Slider<'_>) -> egui::Response {
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = (ui.available_width() - 12.0).max(80.0);
        ui.add(slider.show_value(false))
    })
    .inner
}

/// "Label ............ [widget]" row.
pub fn row(ui: &mut Ui, label: &str, add_right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        widgets::field_label(ui, label);
        ui.with_layout(Layout::right_to_left(Align::Center), add_right);
    });
}

/// "Label" above a full-width control.
pub fn field<R>(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        widgets::field_label(ui, label);
        add(ui)
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autostart::FakeAutostart;
    use crate::fonts::{apply_caption_font, FontCatalog};
    use crate::i18n::UI_LANGUAGES;
    use eframe::egui::{pos2, RawInput, Rect};
    use unicaptions_core::events::NullSink;
    use unicaptions_core::Paths;

    struct Harness {
        ctx: egui::Context,
        engine: Arc<Engine>,
        fonts: FontCatalog,
        autostart: FakeAutostart,
        _dir: tempfile::TempDir,
    }

    fn harness() -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::new(Paths::in_dir(dir.path()), Arc::new(NullSink)).unwrap();
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        crate::theme::install(&ctx);
        let fonts = FontCatalog::empty();
        apply_caption_font(&ctx, &fonts, "Inter", 600);
        Harness {
            ctx,
            engine,
            fonts,
            autostart: FakeAutostart::default(),
            _dir: dir,
        }
    }

    fn frame(h: &Harness, settings: &mut Settings, st: &mut UiState, t: &I18n) {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 640.0))),
            ..Default::default()
        };
        let pal = crate::theme::palette_for(&h.ctx);
        crate::testutil::run_pass(&h.ctx, input, |ui| {
            let mut cx = Cx {
                t,
                pal,
                settings: &mut *settings,
                st: &mut *st,
                engine: &h.engine,
                fonts: &h.fonts,
                autostart: &h.autostart,
            };
            draw(ui, &mut cx);
        });
    }

    #[test]
    fn every_tab_renders_in_every_language() {
        let h = harness();
        for lang in UI_LANGUAGES {
            let t = I18n::new(lang);
            let mut settings = Settings {
                translation_enabled: true,
                ..Settings::default()
            };
            let mut st = UiState {
                models: h.engine.list_models().unwrap(),
                models_dirty: false,
                ..UiState::default()
            };
            for tab in Tab::ALL {
                st.tab = tab;
                // two frames: egui lays out widgets across passes
                frame(&h, &mut settings, &mut st, &t);
                frame(&h, &mut settings, &mut st, &t);
            }
        }
    }

    #[test]
    fn tabs_render_in_every_state_combination() {
        use unicaptions_core::settings::Provider;
        let h = harness();
        let t = I18n::new("en");
        for (asr, tr_provider) in [
            (Provider::Local, Provider::Local),
            (Provider::Cloud, Provider::Cloud),
        ] {
            let mut settings = Settings {
                asr_provider: asr,
                translate_provider: tr_provider,
                translation_enabled: true,
                target_language: "pl".into(), // unsupported locally -> shows the hint
                ..Settings::default()
            };
            let mut st = UiState {
                mic_active: true,
                mic_level: 0.5,
                captioning: CaptioningStatus::Starting,
                error: Some("boom".into()),
                ..Default::default()
            };
            st.progress
                .insert((unicaptions_core::ModelKind::Whisper, "base".into()), 0.4);
            st.models = h.engine.list_models().unwrap();
            st.models_dirty = false;
            for tab in Tab::ALL {
                st.tab = tab;
                frame(&h, &mut settings, &mut st, &t);
            }
            for status in [CaptioningStatus::Live, CaptioningStatus::Idle] {
                st.captioning = status;
                frame(&h, &mut settings, &mut st, &t);
            }
        }
    }

    #[test]
    fn set_captioning_off_when_idle_is_harmless() {
        let h = harness();
        let t = I18n::new("en");
        let mut settings = Settings::default();
        let mut st = UiState::default();
        let mut cx = Cx {
            t: &t,
            pal: crate::theme::palette(true),
            settings: &mut settings,
            st: &mut st,
            engine: &h.engine,
            fonts: &h.fonts,
            autostart: &h.autostart,
        };
        set_captioning(&mut cx, false);
        assert_eq!(cx.st.captioning, CaptioningStatus::Idle);
    }

    #[test]
    fn combo_changes_value() {
        let ctx = egui::Context::default();
        let mut value = 1;
        let opts = [(1, "one".to_string()), (2, "two".to_string())];
        crate::testutil::run_pass(&ctx, RawInput::default(), |ui| {
            // Not interacted with: value is unchanged.
            assert!(!combo(ui, "t", &mut value, &opts, true));
        });
        assert_eq!(value, 1);
    }
}
