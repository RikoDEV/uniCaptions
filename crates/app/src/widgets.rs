//! Custom egui widgets replacing the React components (`Toggle`,
//! `CardSelect`, level meter, panels, sidebar nav items).

use crate::icons::{Flag, Icon};
use crate::theme::Palette;
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, CursorIcon, FontId, Image, Margin, Rect,
    Response, RichText, Sense, Stroke, StrokeKind, Ui,
};

/// A pill-shaped on/off switch. Returns the response; `changed()` is set on toggle.
pub fn toggle(ui: &mut Ui, pal: &Palette, on: &mut bool, enabled: bool) -> Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, mut resp) = ui.allocate_exact_size(vec2(38.0, 22.0), sense);
    if enabled && resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let off_track = Color32::from_rgba_unmultiplied(127, 127, 127, 90);
    let mut track = off_track.lerp_to_gamma(pal.accent, t);
    let mut thumb = Color32::WHITE;
    if !enabled {
        track = track.gamma_multiply(0.5);
        thumb = thumb.gamma_multiply(0.7);
    }
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(11), track);
    let x = egui::lerp(rect.left() + 11.0..=rect.right() - 11.0, t);
    painter.circle_filled(pos2(x, rect.center().y), 8.0, thumb);
    if enabled {
        resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    }
    resp
}

/// A horizontal bar for audio level / download progress. `fraction` is 0..=1.
pub fn level_meter(ui: &mut Ui, pal: &Palette, fraction: f32, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 8.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius::same(4),
        Color32::from_rgba_unmultiplied(127, 127, 127, 64),
    );
    let f = fraction.clamp(0.0, 1.0);
    if f > 0.0 {
        let fill = Rect::from_min_size(rect.min, vec2(rect.width() * f, rect.height()));
        painter.rect_filled(fill, CornerRadius::same(4), pal.accent);
    }
}

/// White rounded card with a border, like the old `.panel`.
pub fn panel<R>(ui: &mut Ui, pal: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(pal.bg_elevated)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::symmetric(22, 20))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 14.0;
            add(ui)
        })
        .inner
}

pub fn panel_title(ui: &mut Ui, pal: &Palette, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .size(11.0)
            .strong()
            .color(pal.text_faint),
    );
}

pub fn field_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(14.0).strong());
}

pub fn hint(ui: &mut Ui, pal: &Palette, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(12.5).color(pal.text_faint)).wrap());
}

/// Button filled with the accent colour.
pub fn accent_button(ui: &mut Ui, pal: &Palette, text: &str, enabled: bool) -> Response {
    let btn = egui::Button::new(RichText::new(text).color(pal.accent_contrast).strong())
        .fill(pal.accent)
        .stroke(Stroke::NONE);
    ui.add_enabled(enabled, btn)
}

pub fn plain_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    ui.add_enabled(enabled, egui::Button::new(text))
}

/// Icon displayed on a card. `Flag::Auto` falls back to the tinted globe.
#[derive(Clone, Copy)]
pub enum CardIcon {
    None,
    Icon(Icon),
    Flag(Flag),
}

pub struct CardOption<T> {
    pub value: T,
    pub label: String,
    pub icon: CardIcon,
    pub description: Option<String>,
    /// `true` shows a small "downloaded" badge.
    pub downloaded: bool,
    /// 0..=1 while downloading; replaces the label with a progress bar.
    pub progress: Option<f32>,
}

impl<T> CardOption<T> {
    pub fn new(value: T, label: impl Into<String>) -> Self {
        Self {
            value,
            label: label.into(),
            icon: CardIcon::None,
            description: None,
            downloaded: false,
            progress: None,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = CardIcon::Icon(icon);
        self
    }

    pub fn flag(mut self, flag: Flag) -> Self {
        self.icon = CardIcon::Flag(flag);
        self
    }

    pub fn description(mut self, text: impl Into<String>) -> Self {
        self.description = Some(text.into());
        self
    }

    pub fn downloaded(mut self, downloaded: bool) -> Self {
        self.downloaded = downloaded;
        self
    }

    pub fn progress(mut self, progress: Option<f32>) -> Self {
        self.progress = progress;
        self
    }
}

/// Row of selectable cards (the old `CardSelect`). Returns the newly picked value.
pub fn card_select<T: Copy + PartialEq>(
    ui: &mut Ui,
    pal: &Palette,
    current: T,
    options: &[CardOption<T>],
    wrap: bool,
) -> Option<T> {
    const GAP: f32 = 8.0;
    const HEIGHT: f32 = 66.0;
    let n = options.len().max(1) as f32;
    let width = if wrap {
        108.0
    } else {
        ((ui.available_width() - GAP * (n - 1.0)) / n).clamp(92.0, 220.0)
    };

    let mut picked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(GAP, GAP);
        for opt in options {
            let (rect, resp) = ui.allocate_exact_size(vec2(width, HEIGHT), Sense::click());
            let active = opt.value == current;
            let fill = if active {
                pal.accent.gamma_multiply(0.16)
            } else if resp.hovered() {
                pal.hover
            } else {
                pal.bg
            };
            let stroke = if active {
                Stroke::new(1.5, pal.accent)
            } else {
                Stroke::new(1.0, pal.border)
            };
            let painter = ui.painter();
            painter.rect(
                rect,
                CornerRadius::same(10),
                fill,
                stroke,
                StrokeKind::Inside,
            );

            let tint = if active { pal.accent } else { pal.text_muted };
            let icon_center = pos2(rect.center().x, rect.top() + 23.0);
            match opt.icon {
                CardIcon::None => {}
                CardIcon::Icon(icon) => {
                    let r = Rect::from_center_size(icon_center, vec2(20.0, 20.0));
                    Image::new(icon.source()).tint(tint).paint_at(ui, r);
                }
                CardIcon::Flag(flag) => match flag.source() {
                    Some(src) => {
                        let r = Rect::from_center_size(icon_center, vec2(22.0, 15.0));
                        Image::new(src).paint_at(ui, r);
                    }
                    None => {
                        let r = Rect::from_center_size(icon_center, vec2(20.0, 20.0));
                        Image::new(Icon::Globe.source()).tint(tint).paint_at(ui, r);
                    }
                },
            }

            let label_pos = pos2(rect.center().x, rect.bottom() - 15.0);
            match opt.progress {
                Some(p) if p < 1.0 => {
                    let bar = Rect::from_center_size(label_pos, vec2(rect.width() - 28.0, 6.0));
                    let painter = ui.painter();
                    painter.rect_filled(
                        bar,
                        CornerRadius::same(3),
                        Color32::from_rgba_unmultiplied(127, 127, 127, 70),
                    );
                    let fill = Rect::from_min_size(
                        bar.min,
                        vec2(bar.width() * p.clamp(0.0, 1.0), bar.height()),
                    );
                    painter.rect_filled(fill, CornerRadius::same(3), pal.accent);
                }
                _ => {
                    let color = if active { pal.text } else { pal.text_muted };
                    ui.painter().text(
                        label_pos,
                        Align2::CENTER_CENTER,
                        &opt.label,
                        FontId::proportional(13.0),
                        color,
                    );
                }
            }

            if opt.downloaded {
                let badge = pos2(rect.right() - 11.0, rect.top() + 11.0);
                ui.painter().circle_filled(badge, 7.0, pal.accent);
                let r = Rect::from_center_size(badge, vec2(9.0, 9.0));
                Image::new(Icon::Download.source())
                    .tint(pal.accent_contrast)
                    .paint_at(ui, r);
            }

            let resp = match &opt.description {
                Some(d) => resp.on_hover_text(d),
                None => resp,
            };
            if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                picked = Some(opt.value);
            }
        }
    });
    picked
}

/// Sidebar navigation row.
pub fn nav_item(ui: &mut Ui, pal: &Palette, icon: Icon, label: &str, active: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
    let (fill, fg) = if active {
        (pal.accent, pal.accent_contrast)
    } else if resp.hovered() {
        (pal.hover, pal.text)
    } else {
        (Color32::TRANSPARENT, pal.text_muted)
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(6), fill);
    let icon_rect =
        Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(18.0, 18.0));
    Image::new(icon.source()).tint(fg).paint_at(ui, icon_rect);
    ui.painter().text(
        pos2(rect.left() + 38.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        fg,
    );
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// Small coloured dot (captioning status).
pub fn status_dot(ui: &mut Ui, color: Color32, glow: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    let painter = ui.painter();
    if glow {
        painter.circle_filled(rect.center(), 7.0, color.gamma_multiply(0.25));
    }
    painter.circle_filled(rect.center(), 4.0, color);
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return "0 MB".into();
    }
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1024.0 {
        format!("{:.2} GB", mb / 1024.0)
    } else {
        format!("{mb:.1} MB")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes_like_the_old_models_panel() {
        assert_eq!(format_bytes(0), "0 MB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(150 * 1024 * 1024), "150.0 MB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.00 GB");
    }

    #[test]
    fn card_option_builder() {
        let o = CardOption::new(1, "One")
            .icon(Icon::Mic)
            .description("d")
            .downloaded(true)
            .progress(Some(0.5));
        assert_eq!(o.label, "One");
        assert!(o.downloaded);
        assert_eq!(o.progress, Some(0.5));
        assert!(o.description.is_some());
    }
}
