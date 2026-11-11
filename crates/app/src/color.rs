//! Hex colour helpers for the stored `#rrggbb` settings.

use eframe::egui::Color32;

/// Parses `#rgb` / `#rrggbb` (the leading `#` is optional). Invalid input
/// yields `fallback`.
pub fn parse_hex(hex: &str, fallback: [u8; 3]) -> [u8; 3] {
    let clean = hex.trim().trim_start_matches('#');
    let expanded: String = match clean.len() {
        3 => clean.chars().flat_map(|c| [c, c]).collect(),
        6 => clean.to_string(),
        _ => return fallback,
    };
    match u32::from_str_radix(&expanded, 16) {
        Ok(v) => [(v >> 16) as u8, (v >> 8) as u8, v as u8],
        Err(_) => fallback,
    }
}

pub fn to_hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

/// Colour from a hex string and a 0..=1 alpha (non-premultiplied input).
pub fn with_alpha(hex: &str, fallback: [u8; 3], alpha: f32) -> Color32 {
    let [r, g, b] = parse_hex(hex, fallback);
    Color32::from_rgba_unmultiplied(r, g, b, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_long_and_short_hex() {
        assert_eq!(parse_hex("#ffffff", [0; 3]), [255, 255, 255]);
        assert_eq!(parse_hex("#1a2b3c", [0; 3]), [0x1a, 0x2b, 0x3c]);
        assert_eq!(parse_hex("f80", [0; 3]), [0xff, 0x88, 0x00]);
        assert_eq!(parse_hex("  #000  ", [9; 3]), [0, 0, 0]);
    }

    #[test]
    fn invalid_hex_uses_fallback() {
        assert_eq!(parse_hex("", [1, 2, 3]), [1, 2, 3]);
        assert_eq!(parse_hex("#12", [1, 2, 3]), [1, 2, 3]);
        assert_eq!(parse_hex("#gggggg", [1, 2, 3]), [1, 2, 3]);
    }

    #[test]
    fn round_trips() {
        assert_eq!(to_hex([0x65, 0xa3, 0x0d]), "#65a30d");
        assert_eq!(parse_hex(&to_hex([10, 20, 30]), [0; 3]), [10, 20, 30]);
    }

    #[test]
    fn alpha_is_applied() {
        let c = with_alpha("#000000", [0; 3], 0.55);
        assert_eq!(c.a(), 140);
        assert_eq!(with_alpha("#ffffff", [0; 3], 2.0).a(), 255);
        assert_eq!(with_alpha("#ffffff", [0; 3], -1.0).a(), 0);
    }
}
