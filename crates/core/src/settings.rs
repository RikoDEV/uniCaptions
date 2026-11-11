//! Persistent user settings.
//!
//! The on-disk format is unchanged from earlier versions: `settings.json`
//! containing `{ "settings": { ... } }` with camelCase keys. Missing keys fall back to defaults (deeply, for
//! `captionStyle`), so older and partial files keep loading.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

const SETTINGS_KEY: &str = "settings";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioSource {
    Microphone,
    System,
}

impl AudioSource {
    pub fn as_str(self) -> &'static str {
        match self {
            AudioSource::Microphone => "microphone",
            AudioSource::System => "system",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Local,
    Cloud,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WhisperSize {
    Tiny,
    Base,
    Small,
    Medium,
}

impl WhisperSize {
    pub const ALL: [WhisperSize; 4] = [
        WhisperSize::Tiny,
        WhisperSize::Base,
        WhisperSize::Small,
        WhisperSize::Medium,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            WhisperSize::Tiny => "tiny",
            WhisperSize::Base => "base",
            WhisperSize::Small => "small",
            WhisperSize::Medium => "medium",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionStyle {
    pub font_family: String,
    pub font_size: f32,
    pub font_weight: u32,
    pub text_color: String,
    pub background_color: String,
    pub background_opacity: f32,
    pub outline: bool,
    pub max_lines: u32,
    pub show_original: bool,
    pub show_translation: bool,
    /// Show captions in UPPERCASE.
    pub uppercase: bool,
}

impl Default for CaptionStyle {
    fn default() -> Self {
        Self {
            font_family: "Inter, system-ui, sans-serif".into(),
            font_size: 28.0,
            font_weight: 600,
            text_color: "#ffffff".into(),
            background_color: "#000000".into(),
            background_opacity: 0.55,
            outline: true,
            max_lines: 2,
            show_original: true,
            show_translation: true,
            uppercase: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub caption_style: CaptionStyle,
    pub audio_source: AudioSource,
    pub source_language: String,
    pub target_language: String,
    pub translation_enabled: bool,
    pub asr_provider: Provider,
    pub translate_provider: Provider,
    pub asr_model: WhisperSize,
    pub click_through: bool,
    pub ui_language: String,
    pub auto_start_captioning: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            caption_style: CaptionStyle::default(),
            audio_source: AudioSource::Microphone,
            source_language: "auto".into(),
            target_language: "es".into(),
            translation_enabled: false,
            asr_provider: Provider::Local,
            translate_provider: Provider::Local,
            asr_model: WhisperSize::Base,
            click_through: true,
            ui_language: "en".into(),
            auto_start_captioning: false,
        }
    }
}

/// Recursively overlays `saved` on top of `base` (objects merge key by key,
/// everything else is replaced).
fn deep_merge(base: &mut Value, saved: &Value) {
    match (base, saved) {
        (Value::Object(b), Value::Object(s)) => {
            for (k, v) in s {
                match b.get_mut(k) {
                    Some(existing) => deep_merge(existing, v),
                    None => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, s) => *b = s.clone(),
    }
}

impl Settings {
    /// Builds settings from a saved JSON value, filling in defaults. Fields
    /// with an invalid value (e.g. an unknown provider) fall back to their
    /// default instead of discarding every other setting.
    pub fn from_saved(saved: &Value) -> Settings {
        let defaults = serde_json::to_value(Settings::default()).expect("serialize defaults");
        let mut merged = defaults.clone();
        deep_merge(&mut merged, saved);
        if let Ok(s) = serde_json::from_value::<Settings>(merged.clone()) {
            return s;
        }
        // Something had the wrong type/variant: drop offending keys one at a
        // time, falling back to the default for each.
        let (Value::Object(merged_obj), Value::Object(default_obj)) = (&mut merged, &defaults)
        else {
            return Settings::default();
        };
        let keys: Vec<String> = merged_obj.keys().cloned().collect();
        for key in keys {
            let mut probe = default_obj.clone();
            if let Some(v) = merged_obj.get(&key) {
                probe.insert(key.clone(), v.clone());
            }
            if serde_json::from_value::<Settings>(Value::Object(probe)).is_err() {
                if let Some(d) = default_obj.get(&key) {
                    merged_obj.insert(key.clone(), d.clone());
                }
            }
        }
        serde_json::from_value(merged).unwrap_or_default()
    }

    /// Loads settings from `path`. A missing or unreadable file yields defaults.
    pub fn load(path: &Path) -> Settings {
        let Ok(text) = fs::read_to_string(path) else {
            return Settings::default();
        };
        let Ok(root) = serde_json::from_str::<Value>(&text) else {
            return Settings::default();
        };
        match root.get(SETTINGS_KEY) {
            Some(saved) => Settings::from_saved(saved),
            None => Settings::default(),
        }
    }

    /// Atomically writes `{ "settings": ... }` to `path`.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        // Preserve any unrelated top-level keys another version may have written.
        let mut root = fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .filter(|v| v.is_object())
            .unwrap_or_else(|| json!({}));
        root[SETTINGS_KEY] = serde_json::to_value(self).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
        atomic_write(path, text.as_bytes())
    }
}

/// Writes via a temp file and rename so a crash never leaves a truncated file.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Extracts the first family name from a CSS `font-family` list, e.g.
/// `'Segoe UI', sans-serif` -> `Segoe UI`, `Inter, system-ui` -> `Inter`.
/// Generic keywords (`system-ui`, `sans-serif`, ...) are returned as-is.
pub fn primary_font_family(css: &str) -> String {
    let first = css.split(',').next().unwrap_or("").trim();
    first
        .trim_matches(|c| c == '\'' || c == '"')
        .trim()
        .to_string()
}

/// Settings shared by every window, with debounced background persistence.
pub struct SettingsStore {
    path: PathBuf,
    inner: RwLock<Settings>,
    dirty: AtomicBool,
    last_error: Mutex<Option<String>>,
}

impl SettingsStore {
    /// Loads settings and starts the background saver. The saver holds only a
    /// weak reference, so dropping the store stops it.
    pub fn open(path: PathBuf) -> Arc<Self> {
        let store = Arc::new(Self {
            inner: RwLock::new(Settings::load(&path)),
            path,
            dirty: AtomicBool::new(false),
            last_error: Mutex::new(None),
        });
        let weak = Arc::downgrade(&store);
        let _ = std::thread::Builder::new()
            .name("unicaptions-settings-saver".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(300));
                match weak.upgrade() {
                    Some(s) => s.flush(),
                    None => return,
                }
            });
        store
    }

    pub fn get(&self) -> Settings {
        self.inner.read().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn set(&self, settings: Settings) {
        if let Ok(mut g) = self.inner.write() {
            if *g == settings {
                return;
            }
            *g = settings;
        }
        self.dirty.store(true, Ordering::SeqCst);
    }

    pub fn update(&self, f: impl FnOnce(&mut Settings)) {
        let mut s = self.get();
        f(&mut s);
        self.set(s);
    }

    /// Writes pending changes now. Call on shutdown.
    pub fn flush(&self) {
        if self.dirty.swap(false, Ordering::SeqCst) {
            let result = self.get().save(&self.path);
            if let Ok(mut e) = self.last_error.lock() {
                *e = result.err();
            }
        }
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().ok().and_then(|e| e.clone())
    }
}

/// Where the overlay window was last placed (logical pixels).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OverlayWindowState {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl OverlayWindowState {
    pub fn load(path: &Path) -> Option<Self> {
        let text = fs::read_to_string(path).ok()?;
        let s: Self = serde_json::from_str(&text).ok()?;
        let finite = [s.x, s.y, s.width, s.height].iter().all(|v| v.is_finite());
        (finite && s.width >= 100.0 && s.height >= 40.0).then_some(s)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        atomic_write(path, text.as_bytes())
    }

    /// Default placement: centred horizontally, 60px above the screen bottom.
    pub fn default_for_screen(screen_w: f32, screen_h: f32) -> Self {
        let (width, height) = (900.0, 160.0);
        Self {
            x: ((screen_w - width) / 2.0).max(0.0),
            y: (screen_h - height - 60.0).max(0.0),
            width,
            height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_original_app() {
        let s = Settings::default();
        assert_eq!(s.caption_style.font_size, 28.0);
        assert_eq!(s.caption_style.font_weight, 600);
        assert_eq!(s.asr_model, WhisperSize::Base);
        assert!(s.click_through);
        assert_eq!(s.target_language, "es");
    }

    #[test]
    fn uppercase_defaults_off_and_old_files_still_load() {
        assert!(!Settings::default().caption_style.uppercase);
        // a file written before the option existed has no "uppercase" key
        let old = json!({"captionStyle": {"fontSize": 30}});
        let s = Settings::from_saved(&old);
        assert!(!s.caption_style.uppercase);
        assert_eq!(s.caption_style.font_size, 30.0);
        let on = Settings::from_saved(&json!({"captionStyle": {"uppercase": true}}));
        assert!(on.caption_style.uppercase);
        let v = serde_json::to_value(&on).unwrap();
        assert_eq!(v["captionStyle"]["uppercase"], true);
    }

    #[test]
    fn serializes_in_the_legacy_camel_case_shape() {
        let v = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(v["audioSource"], "microphone");
        assert_eq!(v["asrProvider"], "local");
        assert_eq!(v["captionStyle"]["backgroundOpacity"], 0.55f32 as f64);
        assert!(v.get("autoStartCaptioning").is_some());
    }

    #[test]
    fn loads_a_legacy_store_file_with_missing_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r##"{"settings":{"audioSource":"system","uiLanguage":"pl",
                "captionStyle":{"fontSize":40,"textColor":"#ff0000"}}}"##,
        )
        .unwrap();
        let s = Settings::load(&path);
        assert_eq!(s.audio_source, AudioSource::System);
        assert_eq!(s.ui_language, "pl");
        assert_eq!(s.caption_style.font_size, 40.0);
        assert_eq!(s.caption_style.text_color, "#ff0000");
        // untouched nested fields keep their defaults
        assert_eq!(s.caption_style.font_weight, 600);
        assert_eq!(s.caption_style.max_lines, 2);
        assert_eq!(s.asr_model, WhisperSize::Base);
    }

    #[test]
    fn invalid_values_fall_back_per_field() {
        let saved = json!({"asrProvider": "quantum", "uiLanguage": "de", "clickThrough": "yes"});
        let s = Settings::from_saved(&saved);
        assert_eq!(s.asr_provider, Provider::Local);
        assert!(s.click_through);
        assert_eq!(s.ui_language, "de");
    }

    #[test]
    fn missing_or_corrupt_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            Settings::load(&dir.path().join("nope.json")),
            Settings::default()
        );
        let bad = dir.path().join("bad.json");
        fs::write(&bad, "{not json").unwrap();
        assert_eq!(Settings::load(&bad), Settings::default());
    }

    #[test]
    fn save_round_trips_and_preserves_unrelated_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("settings.json");
        let mut s = Settings {
            ui_language: "ja".into(),
            ..Settings::default()
        };
        s.caption_style.max_lines = 4;
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);

        let mut root: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        root["other"] = json!(1);
        fs::write(&path, root.to_string()).unwrap();
        s.save(&path).unwrap();
        let root: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["other"], 1);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn store_flush_persists_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let store = SettingsStore::open(path.clone());
        store.update(|s| s.ui_language = "fr".into());
        store.flush();
        assert_eq!(Settings::load(&path).ui_language, "fr");
        assert_eq!(store.last_error(), None);
    }

    #[test]
    fn primary_font_family_parses_css_lists() {
        assert_eq!(primary_font_family("Inter, system-ui, sans-serif"), "Inter");
        assert_eq!(primary_font_family("'Segoe UI', sans-serif"), "Segoe UI");
        assert_eq!(
            primary_font_family("\"Times New Roman\", serif"),
            "Times New Roman"
        );
        assert_eq!(primary_font_family("system-ui, sans-serif"), "system-ui");
        assert_eq!(primary_font_family(""), "");
    }

    #[test]
    fn overlay_window_state_default_and_validation() {
        let d = OverlayWindowState::default_for_screen(1920.0, 1080.0);
        assert_eq!((d.x, d.y, d.width, d.height), (510.0, 860.0, 900.0, 160.0));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("w.json");
        d.save(&path).unwrap();
        assert_eq!(OverlayWindowState::load(&path), Some(d));
        fs::write(&path, r#"{"x":1,"y":1,"width":5,"height":5}"#).unwrap();
        assert_eq!(OverlayWindowState::load(&path), None);
    }
}
