//! Events flowing from the engine's worker threads to the UI.
//!
//! Core code only knows about the [`EventSink`] trait; the GUI crate implements
//! it with a channel plus a repaint request.

use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelKind {
    Whisper,
    Translate,
}

impl ModelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelKind::Whisper => "whisper",
            ModelKind::Translate => "translate",
        }
    }
}

/// One caption update. An empty `text` clears the overlay.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptionEvent {
    pub text: String,
    pub translated_text: Option<String>,
    pub is_final: bool,
    pub timestamp: u128,
}

impl CaptionEvent {
    pub fn clear(timestamp: u128) -> Self {
        Self {
            text: String::new(),
            translated_text: None,
            is_final: true,
            timestamp,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum UiEvent {
    /// RMS input level in [0, ~1].
    AudioLevel(f32),
    Caption(CaptionEvent),
    /// Download progress in [0, 1]. `id` is the whisper size or translate language.
    ModelProgress {
        kind: ModelKind,
        id: String,
        progress: f64,
    },
    /// A download finished (successfully or not).
    ModelDownloadFinished {
        kind: ModelKind,
        id: String,
        result: Result<(), String>,
    },
    CaptioningStarted,
    CaptioningFailed(String),
    CaptioningStopped,
    /// Local translation cannot do this language combination.
    TranslationUnavailable {
        source: String,
        target: String,
    },
    /// Translation failed (model could not be loaded, ...). Captioning goes on.
    TranslationFailed(String),
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: UiEvent);
}

pub type SharedSink = Arc<dyn EventSink>;

/// Sink that drops every event. Handy for tests and headless use.
pub struct NullSink;

impl EventSink for NullSink {
    fn emit(&self, _event: UiEvent) {}
}

/// Sink that forwards events to a closure.
pub struct FnSink<F: Fn(UiEvent) + Send + Sync>(pub F);

impl<F: Fn(UiEvent) + Send + Sync> EventSink for FnSink<F> {
    fn emit(&self, event: UiEvent) {
        (self.0)(event)
    }
}
