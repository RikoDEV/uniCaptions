//! uniCaptions core: audio capture, speech recognition, translation,
//! settings and model management. No GUI dependencies.

pub mod asr;
pub mod audio;
pub mod engine;
pub mod events;
pub mod models;
pub mod paths;
pub mod secrets;
pub mod settings;
pub mod translate;

pub use engine::{CaptioningParams, Engine};
pub use events::{CaptionEvent, EventSink, ModelKind, SharedSink, UiEvent};
pub use paths::Paths;
pub use settings::{Settings, SettingsStore};
