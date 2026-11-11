//! Window-independent UI state, updated from engine events.

use crate::i18n::I18n;
use std::collections::HashMap;
use unicaptions_core::models::ModelEntry;
use unicaptions_core::secrets::ApiProvider;
use unicaptions_core::{CaptioningParams, ModelKind, Settings, UiEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    General,
    CaptionStyle,
    Translation,
    Models,
    About,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::General,
        Tab::CaptionStyle,
        Tab::Translation,
        Tab::Models,
        Tab::About,
    ];

    pub fn label_key(self) -> &'static str {
        match self {
            Tab::General => "nav.general",
            Tab::CaptionStyle => "nav.captionStyle",
            Tab::Translation => "nav.translation",
            Tab::Models => "nav.models",
            Tab::About => "nav.about",
        }
    }

    pub fn description_key(self) -> &'static str {
        match self {
            Tab::General => "tabDesc.general",
            Tab::CaptionStyle => "tabDesc.captionStyle",
            Tab::Translation => "tabDesc.translation",
            Tab::Models => "tabDesc.models",
            Tab::About => "tabDesc.about",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptioningStatus {
    Idle,
    Starting,
    Live,
}

pub struct UiState {
    pub tab: Tab,
    pub mic_active: bool,
    pub mic_level: f32,
    pub captioning: CaptioningStatus,
    /// In-flight model downloads, 0..=1.
    pub progress: HashMap<(ModelKind, String), f32>,
    pub models: Vec<ModelEntry>,
    pub models_dirty: bool,
    /// A model currently being deleted (disables its button for a frame).
    pub error: Option<String>,
    pub key_present: HashMap<ApiProvider, bool>,
    pub key_input: HashMap<ApiProvider, String>,
    pub font_search: String,
    pub autostart: Option<bool>,
    /// Starts captioning once, on first frame, if `autoStartCaptioning` is set.
    pub auto_start_done: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            tab: Tab::General,
            mic_active: false,
            mic_level: 0.0,
            captioning: CaptioningStatus::Idle,
            progress: HashMap::new(),
            models: Vec::new(),
            models_dirty: true,
            error: None,
            key_present: HashMap::new(),
            key_input: HashMap::new(),
            font_search: String::new(),
            autostart: None,
            auto_start_done: false,
        }
    }
}

impl UiState {
    /// Applies one engine event. Caption events are handled by the overlay and
    /// are ignored here.
    pub fn apply_event(&mut self, event: &UiEvent, t: &I18n) {
        match event {
            UiEvent::AudioLevel(level) => {
                if self.mic_active || self.captioning == CaptioningStatus::Live {
                    self.mic_level = *level;
                }
            }
            UiEvent::Caption(_) => {}
            UiEvent::ModelProgress { kind, id, progress } => {
                if *progress < 1.0 {
                    self.progress.insert((*kind, id.clone()), *progress as f32);
                }
            }
            UiEvent::ModelDownloadFinished { kind, id, result } => {
                self.progress.remove(&(*kind, id.clone()));
                self.models_dirty = true;
                if let Err(e) = result {
                    self.error = Some(t.tr("errors.generic", &[("error", e)]));
                }
            }
            UiEvent::CaptioningStarted => {
                self.captioning = CaptioningStatus::Live;
                self.models_dirty = true;
            }
            UiEvent::CaptioningFailed(e) => {
                self.captioning = CaptioningStatus::Idle;
                self.progress.clear();
                self.error = Some(t.tr("errors.captioningFailed", &[("error", e)]));
            }
            UiEvent::CaptioningStopped => {
                self.captioning = CaptioningStatus::Idle;
                self.progress.clear();
                if !self.mic_active {
                    self.mic_level = 0.0;
                }
            }
        }
    }

    pub fn download_progress(&self, kind: ModelKind, id: &str) -> Option<f32> {
        self.progress.get(&(kind, id.to_string())).copied()
    }

    /// Progress of whichever download is furthest along, for the sidebar bar.
    pub fn any_download_progress(&self) -> Option<f32> {
        self.progress.values().copied().reduce(f32::max)
    }
}

pub fn captioning_params(s: &Settings) -> CaptioningParams {
    CaptioningParams {
        language: s.source_language.clone(),
        target_language: s.translation_enabled.then(|| s.target_language.clone()),
        audio_source: s.audio_source,
        asr_provider: s.asr_provider,
        asr_model: s.asr_model,
        translate_provider: s.translate_provider,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicaptions_core::settings::{AudioSource, Provider};

    fn t() -> I18n {
        I18n::new("en")
    }

    #[test]
    fn captioning_lifecycle() {
        let mut st = UiState::default();
        let t = t();
        st.captioning = CaptioningStatus::Starting;
        st.apply_event(&UiEvent::CaptioningStarted, &t);
        assert_eq!(st.captioning, CaptioningStatus::Live);
        st.apply_event(&UiEvent::AudioLevel(0.2), &t);
        assert_eq!(st.mic_level, 0.2);
        st.apply_event(&UiEvent::CaptioningStopped, &t);
        assert_eq!(st.captioning, CaptioningStatus::Idle);
        assert_eq!(st.mic_level, 0.0);
    }

    #[test]
    fn failure_surfaces_a_translated_error() {
        let mut st = UiState {
            captioning: CaptioningStatus::Starting,
            ..Default::default()
        };
        st.progress.insert((ModelKind::Whisper, "base".into()), 0.3);
        st.apply_event(&UiEvent::CaptioningFailed("no key".into()), &t());
        assert_eq!(st.captioning, CaptioningStatus::Idle);
        assert!(st.progress.is_empty());
        assert_eq!(st.error.as_deref(), Some("Captioning failed: no key"));
    }

    #[test]
    fn audio_level_ignored_when_nothing_is_listening() {
        let mut st = UiState::default();
        st.apply_event(&UiEvent::AudioLevel(0.9), &t());
        assert_eq!(st.mic_level, 0.0);
        st.mic_active = true;
        st.apply_event(&UiEvent::AudioLevel(0.9), &t());
        assert_eq!(st.mic_level, 0.9);
    }

    #[test]
    fn download_progress_tracks_and_clears() {
        let mut st = UiState::default();
        let t = t();
        let ev = |p| UiEvent::ModelProgress {
            kind: ModelKind::Whisper,
            id: "base".into(),
            progress: p,
        };
        st.apply_event(&ev(0.4), &t);
        assert_eq!(st.download_progress(ModelKind::Whisper, "base"), Some(0.4));
        assert_eq!(st.download_progress(ModelKind::Translate, "base"), None);
        assert_eq!(st.any_download_progress(), Some(0.4));
        st.models_dirty = false;
        st.apply_event(
            &UiEvent::ModelDownloadFinished {
                kind: ModelKind::Whisper,
                id: "base".into(),
                result: Ok(()),
            },
            &t,
        );
        assert_eq!(st.download_progress(ModelKind::Whisper, "base"), None);
        assert!(st.models_dirty);
        assert!(st.error.is_none());
    }

    #[test]
    fn failed_download_sets_error() {
        let mut st = UiState::default();
        st.apply_event(
            &UiEvent::ModelDownloadFinished {
                kind: ModelKind::Translate,
                id: "es".into(),
                result: Err("offline".into()),
            },
            &t(),
        );
        assert_eq!(st.error.as_deref(), Some("Error: offline"));
    }

    #[test]
    fn params_follow_settings() {
        let mut s = Settings::default();
        let p = captioning_params(&s);
        assert_eq!(p.target_language, None);
        assert_eq!(p.language, "auto");
        s.translation_enabled = true;
        s.audio_source = AudioSource::System;
        s.translate_provider = Provider::Cloud;
        let p = captioning_params(&s);
        assert_eq!(p.target_language.as_deref(), Some("es"));
        assert_eq!(p.audio_source, AudioSource::System);
        assert_eq!(p.translate_provider, Provider::Cloud);
    }

    #[test]
    fn tabs_have_translation_keys() {
        let i = t();
        for tab in Tab::ALL {
            assert_ne!(i.t(tab.label_key()), tab.label_key());
            assert_ne!(i.t(tab.description_key()), tab.description_key());
        }
    }
}
