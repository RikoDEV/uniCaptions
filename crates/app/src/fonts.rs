//! System font discovery and the caption font family.
//!
//! eframe already falls back to system fonts for glyphs the bundled fonts lack
//! (CJK etc.). What it cannot do is pick the *user's chosen* caption font, so
//! we enumerate installed families with `fontdb`, and register the chosen face
//! (closest weight) as the `unicaptions-caption` egui font family.

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use std::sync::{Arc, OnceLock};
use unicaptions_core::settings::primary_font_family;

pub const CAPTION_FAMILY: &str = "unicaptions-caption";

pub fn caption_family() -> FontFamily {
    FontFamily::Name(CAPTION_FAMILY.into())
}

/// CSS generic family names that do not map to a specific installed font.
pub fn is_generic_family(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "" | "system-ui"
            | "sans-serif"
            | "serif"
            | "monospace"
            | "cursive"
            | "fantasy"
            | "ui-sans-serif"
            | "ui-serif"
            | "ui-monospace"
            | "-apple-system"
    )
}

struct Loaded {
    db: fontdb::Database,
    families: Vec<String>,
}

/// Installed fonts, loaded on a background thread (scanning can take a while).
#[derive(Clone)]
pub struct FontCatalog {
    loaded: Arc<OnceLock<Loaded>>,
}

/// A concrete font face found for a family/weight request.
pub struct FoundFace {
    pub data: Vec<u8>,
    pub index: u32,
}

impl FontCatalog {
    /// Starts scanning system fonts in the background. `on_ready` is called
    /// when the scan finishes (use it to request a repaint).
    pub fn spawn(on_ready: impl FnOnce() + Send + 'static) -> Self {
        let loaded = Arc::new(OnceLock::new());
        let slot = loaded.clone();
        let _ = std::thread::Builder::new()
            .name("unicaptions-fonts".into())
            .spawn(move || {
                let mut db = fontdb::Database::new();
                db.load_system_fonts();
                let _ = slot.set(Loaded {
                    families: collect_families(&db),
                    db,
                });
                on_ready();
            });
        Self { loaded }
    }

    /// A catalog that never finishes loading, for tests.
    #[cfg(test)]
    pub fn empty() -> Self {
        Self {
            loaded: Arc::new(OnceLock::new()),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.loaded.get().is_some()
    }

    /// Sorted, de-duplicated installed family names; `None` until loaded.
    pub fn families(&self) -> Option<&[String]> {
        self.loaded.get().map(|l| l.families.as_slice())
    }

    #[cfg(test)]
    pub fn contains(&self, family: &str) -> bool {
        self.families()
            .map(|f| f.iter().any(|x| x.eq_ignore_ascii_case(family)))
            .unwrap_or(false)
    }

    /// The installed face of `family` closest to `weight` (100..=900).
    pub fn find_face(&self, family: &str, weight: u32) -> Option<FoundFace> {
        let loaded = self.loaded.get()?;
        if is_generic_family(family) {
            return None;
        }
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(family)],
            weight: fontdb::Weight(weight.clamp(100, 900) as u16),
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        let id = loaded.db.query(&query)?;
        loaded.db.with_face_data(id, |data, index| FoundFace {
            data: data.to_vec(),
            index,
        })
    }
}

fn collect_families(db: &fontdb::Database) -> Vec<String> {
    let mut names: Vec<String> = db
        .faces()
        .flat_map(|f| f.families.iter().map(|(name, _)| name.clone()))
        .filter(|n| !n.trim().is_empty())
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    names
}

/// What the caption font is currently set to, to avoid redundant re-installs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppliedFont {
    pub family: String,
    pub weight: u32,
    pub catalog_ready: bool,
}

/// Installs the `unicaptions-caption` family: the chosen face first (when
/// installed), then egui's default proportional chain as fallback.
/// Returns `true` if an installed face was found.
pub fn apply_caption_font(
    ctx: &egui::Context,
    catalog: &FontCatalog,
    css_family: &str,
    weight: u32,
) -> bool {
    let family = primary_font_family(css_family);
    let mut defs = FontDefinitions::default();
    let mut chain: Vec<String> = Vec::new();
    let mut found = false;

    if let Some(face) = catalog.find_face(&family, weight) {
        let mut data = FontData::from_owned(face.data);
        data.index = face.index;
        let wght = data
            .variation_axes()
            .into_iter()
            .find(|a| a.tag.to_string().eq_ignore_ascii_case("wght"));
        if let Some(axis) = wght {
            data.tweak.coords = egui::epaint::text::VariationCoords::new([(
                "wght",
                (weight as f32).clamp(axis.range.min, axis.range.max),
            )]);
        }
        defs.font_data
            .insert("unicaptions-caption-face".into(), Arc::new(data));
        chain.push("unicaptions-caption-face".into());
        found = true;
    }

    chain.extend(
        defs.families
            .get(&FontFamily::Proportional)
            .cloned()
            .unwrap_or_default(),
    );
    defs.families.insert(caption_family(), chain);
    ctx.set_fonts(defs);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_families_are_recognised() {
        assert!(is_generic_family("system-ui"));
        assert!(is_generic_family("Sans-Serif"));
        assert!(is_generic_family(""));
        assert!(!is_generic_family("Inter"));
        assert!(!is_generic_family("Segoe UI"));
    }

    #[test]
    fn empty_catalog_finds_nothing() {
        let c = FontCatalog::empty();
        assert!(!c.is_ready());
        assert!(c.families().is_none());
        assert!(!c.contains("Arial"));
        assert!(c.find_face("Arial", 400).is_none());
    }

    #[test]
    fn caption_font_installs_without_a_catalog() {
        let ctx = egui::Context::default();
        let found = apply_caption_font(&ctx, &FontCatalog::empty(), "Inter, system-ui", 600);
        assert!(!found);
    }

    #[test]
    fn family_collection_is_sorted_and_unique() {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let fams = collect_families(&db);
        let mut sorted = fams.clone();
        sorted.sort_by_key(|n| n.to_lowercase());
        assert_eq!(fams, sorted);
        let mut lower: Vec<String> = fams.iter().map(|f| f.to_lowercase()).collect();
        lower.dedup();
        assert_eq!(lower.len(), fams.len());
    }

    #[test]
    fn real_system_font_can_be_found_and_applied() {
        // Only meaningful on machines that have at least one font installed.
        let (tx, rx) = std::sync::mpsc::channel();
        let catalog = FontCatalog::spawn(move || {
            let _ = tx.send(());
        });
        rx.recv_timeout(std::time::Duration::from_secs(60)).unwrap();
        let Some(first) = catalog.families().and_then(|f| f.first().cloned()) else {
            return;
        };
        assert!(catalog.contains(&first));
        let ctx = egui::Context::default();
        let found = apply_caption_font(&ctx, &catalog, &first, 400);
        assert!(found, "expected to load face for {first}");
    }
}
