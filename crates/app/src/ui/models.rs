//! Models tab: download/delete speech and translation models.

use super::Cx;
use crate::widgets::{self, format_bytes};
use eframe::egui::{self, Align, Layout, RichText, Stroke, Ui};
use unicaptions_core::models::ModelEntry;
use unicaptions_core::ModelKind;

pub fn model_label(cx: &Cx, m: &ModelEntry) -> String {
    match m.kind {
        ModelKind::Whisper => cx.t.tr("models.whisperLabel", &[("size", &m.id)]),
        ModelKind::Translate => match unicaptions_core::translate::Pair::parse_id(&m.id) {
            // English -> X keeps its original label.
            Some(pair) if pair.source == "en" => {
                let lang = cx.t.t(&format!("languages.{}", pair.target));
                cx.t.tr("models.translateLabel", &[("lang", &lang)])
            }
            Some(pair) => {
                let from = cx.t.t(&format!("languages.{}", pair.source));
                let to = cx.t.t(&format!("languages.{}", pair.target));
                cx.t.tr("models.pairLabel", &[("from", &from), ("to", &to)])
            }
            None => m.id.clone(),
        },
    }
}

pub fn draw(ui: &mut Ui, cx: &mut Cx) {
    let pal = cx.pal;
    widgets::panel(ui, &pal, |ui| {
        let downloaded: Vec<&ModelEntry> = cx.st.models.iter().filter(|m| m.downloaded).collect();
        let total: u64 = downloaded.iter().map(|m| m.bytes_on_disk).sum();
        let mut intro = cx.t.t("models.intro");
        if !downloaded.is_empty() {
            intro.push(' ');
            intro.push_str(&cx.t.tr("models.inUse", &[("size", &format_bytes(total))]));
        }
        widgets::hint(ui, &pal, &intro);

        let models = cx.st.models.clone();
        let mut delete: Option<String> = None;
        let mut download: Option<(ModelKind, String)> = None;

        for m in &models {
            let progress = cx.st.download_progress(m.kind, &m.id);
            egui::Frame::new()
                .stroke(Stroke::new(1.0, pal.border))
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 4.0;
                            ui.label(RichText::new(model_label(cx, m)).strong());
                            match progress {
                                Some(p) => {
                                    widgets::level_meter(ui, &pal, p, 200.0);
                                }
                                None => {
                                    let status = if m.downloaded {
                                        format_bytes(m.bytes_on_disk)
                                    } else {
                                        cx.t.t("models.notDownloaded")
                                    };
                                    ui.label(
                                        RichText::new(status).size(12.5).color(pal.text_faint),
                                    );
                                }
                            }
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if m.downloaded {
                                if widgets::plain_button(ui, &cx.t.t("models.delete"), true)
                                    .clicked()
                                {
                                    delete = Some(format!("{}:{}", m.kind.as_str(), m.id));
                                }
                            } else {
                                let busy = progress.is_some();
                                let label = if busy {
                                    cx.t.t("models.downloading")
                                } else {
                                    cx.t.t("models.download")
                                };
                                if widgets::accent_button(ui, &pal, &label, !busy).clicked() {
                                    download = Some((m.kind, m.id.clone()));
                                }
                            }
                        });
                    });
                });
        }

        if let Some(key) = delete {
            match cx.engine.delete_model(&key) {
                Ok(()) => cx.st.models_dirty = true,
                Err(e) => cx.st.error = Some(cx.t.tr("errors.generic", &[("error", &e)])),
            }
        }
        if let Some((kind, id)) = download {
            cx.st.progress.insert((kind, id.clone()), 0.0);
            cx.engine.download_model(kind, id);
        }
    });
}
