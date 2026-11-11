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
    /// Contains every letter the app's languages need (see [`REQUIRED_TEXT`]).
    pub complete: bool,
}

/// Letters a caption font must contain to be usable as a text font: ASCII plus
/// the diacritics of the UI languages (Polish, German, French, Spanish,
/// Portuguese). A font lacking some of them (symbol fonts, decorative fonts)
/// would make egui fall back glyph by glyph and mix several typefaces in one word.
pub const REQUIRED_TEXT: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 .,;:!?'\"-()\
    \u{105}\u{107}\u{119}\u{142}\u{144}\u{f3}\u{15b}\u{17a}\u{17c}\u{104}\u{106}\u{118}\u{141}\u{143}\u{d3}\u{15a}\u{179}\u{17b}\
    \u{e4}\u{f6}\u{fc}\u{df}\u{c4}\u{d6}\u{dc}\u{e9}\u{e8}\u{ea}\u{eb}\u{e0}\u{e2}\u{e7}\u{ee}\u{ef}\u{f4}\u{f9}\u{fb}\u{ff}\
    \u{f1}\u{e1}\u{ed}\u{fa}\u{e3}\u{f5}\u{c9}\u{c8}\u{c0}\u{c2}\u{c7}\u{d1}";

/// Text fonts tried, in order, when the chosen font is not a usable text font.
const FALLBACK_TEXT_FONTS: [&str; 12] = [
    "Segoe UI",
    "Inter",
    "SF Pro Text",
    "Helvetica Neue",
    "Arial",
    "Noto Sans",
    "DejaVu Sans",
    "Liberation Sans",
    "Ubuntu",
    "Roboto",
    "Cantarell",
    "Verdana",
];

/// Does the font face contain a glyph for every character of `text`?
pub fn face_covers(data: &[u8], index: u32, text: &str) -> bool {
    use skrifa::{FontRef, MetadataProvider};
    let Ok(font) = FontRef::from_index(data, index) else {
        return false;
    };
    let charmap = font.charmap();
    text.chars().all(|c| charmap.map(c).is_some())
}

/// What [`apply_caption_font`] ended up using.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontOutcome {
    /// The font is not installed (or the list is still loading): built-in font.
    Default,
    /// The chosen font is used.
    Chosen,
    /// The chosen font lacks letters; `using` is used for the whole caption instead.
    Substituted { requested: String, using: String },
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
            complete: face_covers(data, index, REQUIRED_TEXT),
        })
    }

    /// The first installed font from [`FALLBACK_TEXT_FONTS`] that has every
    /// required letter, as `(family name, face)`.
    fn find_fallback_face(&self, weight: u32) -> Option<(&'static str, FoundFace)> {
        FALLBACK_TEXT_FONTS.iter().find_map(|name| {
            self.find_face(name, weight)
                .filter(|f| f.complete)
                .map(|f| (*name, f))
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
) -> FontOutcome {
    let family = primary_font_family(css_family);
    let mut defs = FontDefinitions::default();
    let mut chain: Vec<String> = Vec::new();
    let mut outcome = FontOutcome::Default;

    let face = match catalog.find_face(&family, weight) {
        Some(face) if face.complete => Some(face),
        // Not a usable text font: use one consistent installed text font instead.
        Some(_) => match catalog.find_fallback_face(weight) {
            Some((name, face)) => {
                outcome = FontOutcome::Substituted {
                    requested: family.clone(),
                    using: name.to_string(),
                };
                Some(face)
            }
            None => {
                outcome = FontOutcome::Substituted {
                    requested: family.clone(),
                    using: "Ubuntu".to_string(),
                };
                None
            }
        },
        None => None,
    };

    if let Some(face) = face {
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
        if outcome == FontOutcome::Default {
            outcome = FontOutcome::Chosen;
        }
    }

    chain.extend(
        defs.families
            .get(&FontFamily::Proportional)
            .cloned()
            .unwrap_or_default(),
    );
    defs.families.insert(caption_family(), chain);
    ctx.set_fonts(defs);
    outcome
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
        let outcome = apply_caption_font(&ctx, &FontCatalog::empty(), "Inter, system-ui", 600);
        assert_eq!(outcome, FontOutcome::Default);
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
        let outcome = apply_caption_font(&ctx, &catalog, &first, 400);
        assert_ne!(
            outcome,
            FontOutcome::Default,
            "expected to load a face for {first}"
        );
    }

    fn ready_catalog() -> FontCatalog {
        let (tx, rx) = std::sync::mpsc::channel();
        let catalog = FontCatalog::spawn(move || {
            let _ = tx.send(());
        });
        rx.recv_timeout(std::time::Duration::from_secs(60)).unwrap();
        catalog
    }

    #[test]
    fn required_text_covers_the_polish_letters() {
        for c in "\u{105}\u{119}\u{144}\u{142}\u{f3}\u{15b}\u{107}\u{17a}\u{17c}".chars() {
            assert!(REQUIRED_TEXT.contains(c), "missing {c:?}");
        }
    }

    #[test]
    fn a_real_text_font_is_complete_and_a_symbol_font_is_not() {
        let catalog = ready_catalog();
        // Any common text font installed on the machine must pass...
        let text_font = FALLBACK_TEXT_FONTS
            .iter()
            .find_map(|n| catalog.find_face(n, 400).map(|f| (*n, f)));
        if let Some((name, face)) = text_font {
            assert!(face.complete, "{name} should cover the required letters");
        }
        // ...and the Windows symbol font must not (it only has part of Latin).
        if let Some(face) = catalog.find_face("Segoe UI Symbol", 400) {
            assert!(
                !face.complete,
                "Segoe UI Symbol must be rejected as a text font"
            );
        }
    }

    #[test]
    fn an_incomplete_font_is_substituted_by_one_consistent_text_font() {
        let catalog = ready_catalog();
        if catalog.find_face("Segoe UI Symbol", 600).is_none() {
            return; // not a Windows machine
        }
        let ctx = egui::Context::default();
        match apply_caption_font(&ctx, &catalog, "'Segoe UI Symbol'", 600) {
            FontOutcome::Substituted { requested, using } => {
                assert_eq!(requested, "Segoe UI Symbol");
                assert!(!using.is_empty());
            }
            other => panic!("expected a substitution, got {other:?}"),
        }
        // a complete font is used as chosen
        if catalog.find_face("Segoe UI", 600).is_some() {
            assert_eq!(
                apply_caption_font(&ctx, &catalog, "Segoe UI", 600),
                FontOutcome::Chosen
            );
        }
    }
}
