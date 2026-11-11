//! Tiny i18n layer over the JSON locale files in `assets/locales`.
//!
//! Locale files are nested objects; they are flattened to dotted keys
//! (`nav.general`). `{{name}}` placeholders are substituted by [`I18n::tr`].
//! Missing keys fall back to English, then to the key itself.

use serde_json::Value;
use std::collections::HashMap;

pub const UI_LANGUAGES: [&str; 8] = ["en", "es", "fr", "de", "pl", "pt", "zh", "ja"];

const LOCALE_SOURCES: [(&str, &str); 8] = [
    ("en", include_str!("../../../assets/locales/en.json")),
    ("es", include_str!("../../../assets/locales/es.json")),
    ("fr", include_str!("../../../assets/locales/fr.json")),
    ("de", include_str!("../../../assets/locales/de.json")),
    ("pl", include_str!("../../../assets/locales/pl.json")),
    ("pt", include_str!("../../../assets/locales/pt.json")),
    ("zh", include_str!("../../../assets/locales/zh.json")),
    ("ja", include_str!("../../../assets/locales/ja.json")),
];

type Table = HashMap<String, String>;

fn flatten(value: &Value, prefix: &str, out: &mut Table) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(v, &key, out);
            }
        }
        Value::String(s) => {
            out.insert(prefix.to_string(), s.clone());
        }
        _ => {}
    }
}

fn parse_locale(json: &str) -> Table {
    let mut table = Table::new();
    if let Ok(v) = serde_json::from_str::<Value>(json) {
        flatten(&v, "", &mut table);
    }
    table
}

pub struct I18n {
    language: String,
    tables: HashMap<&'static str, Table>,
}

impl I18n {
    pub fn new(language: &str) -> Self {
        let tables = LOCALE_SOURCES
            .iter()
            .map(|(code, json)| (*code, parse_locale(json)))
            .collect();
        let mut i18n = Self {
            language: "en".into(),
            tables,
        };
        i18n.set_language(language);
        i18n
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    /// Unknown languages fall back to English.
    pub fn set_language(&mut self, language: &str) {
        self.language = if self.tables.contains_key(language) {
            language.to_string()
        } else {
            "en".to_string()
        };
    }

    pub fn t(&self, key: &str) -> String {
        self.lookup(key).unwrap_or(key).to_string()
    }

    /// Translate with `{{name}}` placeholders replaced.
    pub fn tr(&self, key: &str, args: &[(&str, &str)]) -> String {
        let mut text = self.t(key);
        for (name, value) in args {
            text = text.replace(&format!("{{{{{name}}}}}"), value);
        }
        text
    }

    fn lookup(&self, key: &str) -> Option<&str> {
        self.tables
            .get(self.language.as_str())
            .and_then(|t| t.get(key))
            .or_else(|| self.tables.get("en").and_then(|t| t.get(key)))
            .map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_locale_has_exactly_the_english_keys() {
        let en = parse_locale(LOCALE_SOURCES[0].1);
        assert!(en.len() > 100, "english table suspiciously small: {}", en.len());
        for (code, json) in LOCALE_SOURCES {
            let t = parse_locale(json);
            let mut missing: Vec<_> = en.keys().filter(|k| !t.contains_key(*k)).collect();
            let mut extra: Vec<_> = t.keys().filter(|k| !en.contains_key(*k)).collect();
            missing.sort();
            extra.sort();
            assert!(missing.is_empty(), "{code} is missing keys: {missing:?}");
            assert!(extra.is_empty(), "{code} has extra keys: {extra:?}");
        }
    }

    #[test]
    fn placeholders_survive_in_every_locale() {
        let en = parse_locale(LOCALE_SOURCES[0].1);
        for (code, json) in LOCALE_SOURCES {
            let t = parse_locale(json);
            for (key, en_text) in &en {
                for name in [
                    "size", "lang", "source", "opacity", "lines", "version", "value", "error",
                ] {
                    let needle = format!("{{{{{name}}}}}");
                    if en_text.contains(&needle) {
                        assert!(t[key].contains(&needle), "{code}:{key} lost {needle}");
                    }
                }
            }
        }
    }

    #[test]
    fn translates_and_interpolates() {
        let mut i = I18n::new("en");
        assert_eq!(i.t("nav.general"), "General");
        assert_eq!(
            i.tr("captionStyle.fontSize", &[("size", "28")]),
            "Font size (28px)"
        );
        i.set_language("pl");
        assert_eq!(i.language(), "pl");
        assert_ne!(i.t("nav.general"), "General");
        assert!(i.tr("about.version", &[("version", "1.2.3")]).contains("1.2.3"));
    }

    #[test]
    fn unknown_language_and_key_fall_back() {
        let i = I18n::new("xx");
        assert_eq!(i.language(), "en");
        assert_eq!(i.t("no.such.key"), "no.such.key");
    }

    #[test]
    fn all_ui_languages_are_loadable() {
        for code in UI_LANGUAGES {
            let i = I18n::new(code);
            assert_eq!(i.language(), code);
            assert!(!i.t("languages.en").is_empty());
        }
    }
}
