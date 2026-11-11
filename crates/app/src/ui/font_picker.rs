//! Font family picker backed by the installed system fonts.

use super::Cx;
use crate::fonts::is_generic_family;
use eframe::egui::{self, Ui};
use unicaptions_core::settings::primary_font_family;

/// Case-insensitive substring filter used by the picker's search box.
pub fn matches_search(family: &str, search: &str) -> bool {
    let s = search.trim();
    s.is_empty() || family.to_lowercase().contains(&s.to_lowercase())
}

/// Shows the picker. Returns the newly chosen family name, if any.
pub fn draw(ui: &mut Ui, cx: &mut Cx) -> Option<String> {
    let current = primary_font_family(&cx.settings.caption_style.font_family);
    let families = cx.fonts.families();
    let is_custom = match families {
        Some(list) => {
            !is_generic_family(&current) && !list.iter().any(|f| f.eq_ignore_ascii_case(&current))
        }
        None => false,
    };
    let shown = if is_custom {
        cx.t.tr("fontPicker.custom", &[("value", &current)])
    } else {
        current.clone()
    };

    let mut picked = None;
    egui::ComboBox::from_id_salt("caption-font-family")
        .selected_text(shown)
        .width(ui.available_width().min(420.0))
        .height(320.0)
        .show_ui(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut cx.st.font_search)
                    .hint_text(cx.t.t("fontPicker.search"))
                    .desired_width(f32::INFINITY),
            );
            ui.separator();
            match families {
                None => {
                    ui.spinner();
                }
                Some(list) => {
                    let search = cx.st.font_search.clone();
                    egui::ScrollArea::vertical()
                        .max_height(280.0)
                        .show(ui, |ui| {
                            for family in list.iter().filter(|f| matches_search(f, &search)) {
                                let selected = family.eq_ignore_ascii_case(&current);
                                if ui.selectable_label(selected, family).clicked() {
                                    picked = Some(family.clone());
                                }
                            }
                        });
                }
            }
        });
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_is_case_insensitive_substring() {
        assert!(matches_search("Segoe UI", ""));
        assert!(matches_search("Segoe UI", "  "));
        assert!(matches_search("Segoe UI", "goe"));
        assert!(matches_search("Segoe UI", "UI"));
        assert!(matches_search("Segoe UI", "SEGOE"));
        assert!(!matches_search("Segoe UI", "arial"));
    }
}
