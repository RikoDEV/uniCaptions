//! SVG icons and flags, embedded in the binary. Icons are drawn with white
//! strokes so they can be tinted to any colour; flags are drawn untinted.

use eframe::egui::{include_image, ImageSource};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    General,
    Style,
    Translate,
    Models,
    Mic,
    Speaker,
    Offline,
    Cloud,
    Info,
    Globe,
    Download,
    TierTiny,
    TierBase,
    TierSmall,
    TierMedium,
}

impl Icon {
    pub fn source(self) -> ImageSource<'static> {
        match self {
            Icon::General => include_image!("../../../assets/icons/general.svg"),
            Icon::Style => include_image!("../../../assets/icons/style.svg"),
            Icon::Translate => include_image!("../../../assets/icons/translate.svg"),
            Icon::Models => include_image!("../../../assets/icons/models.svg"),
            Icon::Mic => include_image!("../../../assets/icons/mic.svg"),
            Icon::Speaker => include_image!("../../../assets/icons/speaker.svg"),
            Icon::Offline => include_image!("../../../assets/icons/offline.svg"),
            Icon::Cloud => include_image!("../../../assets/icons/cloud.svg"),
            Icon::Info => include_image!("../../../assets/icons/info.svg"),
            Icon::Globe => include_image!("../../../assets/icons/globe.svg"),
            Icon::Download => include_image!("../../../assets/icons/download.svg"),
            Icon::TierTiny => include_image!("../../../assets/icons/tier_tiny.svg"),
            Icon::TierBase => include_image!("../../../assets/icons/tier_base.svg"),
            Icon::TierSmall => include_image!("../../../assets/icons/tier_small.svg"),
            Icon::TierMedium => include_image!("../../../assets/icons/tier_medium.svg"),
        }
    }
}

/// Flag (or globe for `auto`) for a language code used in the language pickers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    Auto,
    En,
    Es,
    Fr,
    De,
    Pl,
    Pt,
    Zh,
    Ja,
}

impl Flag {
    pub fn from_code(code: &str) -> Flag {
        match code {
            "en" => Flag::En,
            "es" => Flag::Es,
            "fr" => Flag::Fr,
            "de" => Flag::De,
            "pl" => Flag::Pl,
            "pt" => Flag::Pt,
            "zh" => Flag::Zh,
            "ja" => Flag::Ja,
            _ => Flag::Auto,
        }
    }

    /// `None` for `Auto`, which uses the (tinted) globe icon instead.
    pub fn source(self) -> Option<ImageSource<'static>> {
        Some(match self {
            Flag::Auto => return None,
            Flag::En => include_image!("../../../assets/flags/en.svg"),
            Flag::Es => include_image!("../../../assets/flags/es.svg"),
            Flag::Fr => include_image!("../../../assets/flags/fr.svg"),
            Flag::De => include_image!("../../../assets/flags/de.svg"),
            Flag::Pl => include_image!("../../../assets/flags/pl.svg"),
            Flag::Pt => include_image!("../../../assets/flags/pt.svg"),
            Flag::Zh => include_image!("../../../assets/flags/zh.svg"),
            Flag::Ja => include_image!("../../../assets/flags/ja.svg"),
        })
    }
}

pub const TRAY_ICON_PNG: &[u8] = include_bytes!("../../../assets/32x32.png");
pub const WINDOW_ICON_PNG: &[u8] = include_bytes!("../../../assets/128x128.png");
