//! API-key entry (stored in the OS credential store, never in settings.json).

use super::{field, Cx};
use crate::widgets;
use eframe::egui::{self, Ui};
use unicaptions_core::secrets::ApiProvider;

pub fn draw(ui: &mut Ui, cx: &mut Cx, provider: ApiProvider, label: &str) {
    let pal = cx.pal;
    let has_key = *cx
        .st
        .key_present
        .entry(provider)
        .or_insert_with(|| cx.engine.has_api_key(provider).unwrap_or(false));

    field(ui, label, |ui| {
        let mut save = false;
        let mut clear = false;
        let placeholder = if has_key {
            cx.t.t("apiKey.placeholderSaved")
        } else {
            cx.t.t("apiKey.placeholder")
        };
        let input = cx.st.key_input.entry(provider).or_default();
        ui.horizontal(|ui| {
            let buttons_w = if has_key { 150.0 } else { 80.0 };
            let edit = egui::TextEdit::singleline(input)
                .password(true)
                .hint_text(placeholder)
                .desired_width((ui.available_width() - buttons_w).max(120.0));
            let resp = ui.add(edit);
            let can_save = !input.trim().is_empty();
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && can_save {
                save = true;
            }
            if widgets::plain_button(ui, &cx.t.t("apiKey.save"), can_save).clicked() {
                save = true;
            }
            if has_key && widgets::plain_button(ui, &cx.t.t("apiKey.clear"), true).clicked() {
                clear = true;
            }
        });
        let hint = if has_key { "apiKey.savedHint" } else { "apiKey.notSavedHint" };
        widgets::hint(ui, &pal, &cx.t.t(hint));

        if save {
            let value = cx.st.key_input.get(&provider).cloned().unwrap_or_default();
            match cx.engine.save_api_key(provider, value.trim()) {
                Ok(()) => {
                    cx.st.key_present.insert(provider, true);
                    cx.st.key_input.remove(&provider);
                }
                Err(e) => cx.st.error = Some(cx.t.tr("errors.generic", &[("error", &e)])),
            }
        }
        if clear {
            match cx.engine.delete_api_key(provider) {
                Ok(()) => {
                    cx.st.key_present.insert(provider, false);
                }
                Err(e) => cx.st.error = Some(cx.t.tr("errors.generic", &[("error", &e)])),
            }
        }
    });
}
