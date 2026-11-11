//! Filesystem locations.
//!
//! These intentionally resolve to the same directories the previous Tauri
//! build used (identifier `com.unicaptions.app`), so existing downloaded
//! models, `settings.json` and window state keep working after the rewrite.

use std::path::{Path, PathBuf};

pub const APP_IDENTIFIER: &str = "com.unicaptions.app";

#[derive(Debug, Clone)]
pub struct Paths {
    data_dir: PathBuf,
    config_dir: PathBuf,
}

impl Paths {
    /// Platform default locations (`%APPDATA%\com.unicaptions.app` on
    /// Windows, `~/Library/Application Support/com.unicaptions.app` on macOS,
    /// `~/.local/share` and `~/.config` on Linux).
    pub fn system() -> Result<Self, String> {
        let data = dirs::data_dir().ok_or("could not determine the user data directory")?;
        let config = dirs::config_dir().ok_or("could not determine the user config directory")?;
        Ok(Self::from_dirs(
            data.join(APP_IDENTIFIER),
            config.join(APP_IDENTIFIER),
        ))
    }

    pub fn from_dirs(data_dir: impl Into<PathBuf>, config_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            config_dir: config_dir.into(),
        }
    }

    /// Both directories rooted at a single folder. Used by tests.
    pub fn in_dir(root: &Path) -> Self {
        Self::from_dirs(root.join("data"), root.join("config"))
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// `settings.json` lives in the data dir (where tauri-plugin-store put it).
    pub fn settings_file(&self) -> PathBuf {
        self.data_dir.join("settings.json")
    }

    /// `.window-state.json` lived in the config dir with tauri-plugin-window-state;
    /// we use our own file next to it rather than reusing that format.
    pub fn window_state_file(&self) -> PathBuf {
        self.config_dir.join("overlay-window.json")
    }

    pub fn whisper_model_file(&self, size: &str) -> PathBuf {
        self.data_dir
            .join("models")
            .join("whisper")
            .join(format!("ggml-{size}.bin"))
    }

    pub fn translate_model_dir(&self, lang: &str) -> PathBuf {
        self.data_dir.join("models").join("translate").join(lang)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_paths_match_legacy_layout() {
        let p = Paths::from_dirs("/d", "/c");
        assert_eq!(p.whisper_model_file("base"), Path::new("/d/models/whisper/ggml-base.bin"));
        assert_eq!(p.translate_model_dir("es"), Path::new("/d/models/translate/es"));
        assert_eq!(p.settings_file(), Path::new("/d/settings.json"));
    }

    #[test]
    fn system_paths_use_app_identifier() {
        let p = Paths::system().unwrap();
        assert!(p.data_dir().ends_with(APP_IDENTIFIER));
        assert!(p.config_dir().ends_with(APP_IDENTIFIER));
    }
}
