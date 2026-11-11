//! The engine: all captioning, audio, model and API-key operations as plain
//! methods the UI can call. Long-running work happens on an owned tokio
//! runtime or dedicated threads and reports back through [`UiEvent`]s.

use crate::asr::{self, AsrConfig, CaptioningHandle, TranslateConfig};
use crate::audio::{self, CaptureHandle};
use crate::events::{ModelKind, SharedSink, UiEvent};
use crate::models::{self, ModelEntry};
use crate::paths::Paths;
use crate::secrets::{self, ApiProvider};
use crate::settings::{AudioSource, Provider, WhisperSize};
use crate::translate;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Everything needed to start live captioning (mirrors the old
/// `start_captioning` command arguments).
#[derive(Debug, Clone)]
pub struct CaptioningParams {
    pub language: String,
    /// `Some` only when translation is enabled.
    pub target_language: Option<String>,
    pub audio_source: AudioSource,
    pub asr_provider: Provider,
    pub asr_model: WhisperSize,
    pub translate_provider: Provider,
}

enum CaptioningState {
    Idle,
    /// Starting (model download/load in progress). The number identifies the
    /// attempt so a stop-then-start can't be confused with the earlier one.
    Starting(u64),
    Active(CaptioningHandle),
}

pub struct Engine {
    paths: Paths,
    sink: SharedSink,
    rt: tokio::runtime::Runtime,
    capture: Mutex<Option<CaptureHandle>>,
    captioning: Mutex<CaptioningState>,
    generation: Mutex<u64>,
    /// Serialises model downloads so a manual download from the Models tab
    /// can't race with one implicitly triggered by starting captioning.
    download_lock: Arc<tokio::sync::Mutex<()>>,
}

/// How often the mic-test level meter is allowed to emit.
const LEVEL_EMIT_INTERVAL: Duration = Duration::from_millis(50);

impl Engine {
    pub fn new(paths: Paths, sink: SharedSink) -> Result<Arc<Self>, String> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("unicaptions-rt")
            .enable_all()
            .build()
            .map_err(|e| format!("failed to start async runtime: {e}"))?;
        Ok(Arc::new(Self {
            paths,
            sink,
            rt,
            capture: Mutex::new(None),
            captioning: Mutex::new(CaptioningState::Idle),
            generation: Mutex::new(0),
            download_lock: Arc::new(tokio::sync::Mutex::new(())),
        }))
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    // ---- mic test / level meter -------------------------------------------

    pub fn start_audio_capture(&self, source: AudioSource) -> Result<(), String> {
        let mut guard = self.capture.lock().map_err(|e| e.to_string())?;
        if guard.is_some() {
            return Ok(());
        }
        let sink = self.sink.clone();
        let mut last = Instant::now() - LEVEL_EMIT_INTERVAL;
        let handle = audio::start_capture_stream(source.as_str(), move |data, _rate, channels| {
            if last.elapsed() < LEVEL_EMIT_INTERVAL {
                return;
            }
            last = Instant::now();
            let mono = audio::downmix(data, channels);
            sink.emit(UiEvent::AudioLevel(audio::rms(&mono)));
        })?;
        *guard = Some(handle);
        Ok(())
    }

    pub fn stop_audio_capture(&self) {
        if let Ok(mut guard) = self.capture.lock() {
            if let Some(handle) = guard.take() {
                handle.stop();
            }
        }
        self.sink.emit(UiEvent::AudioLevel(0.0));
    }

    pub fn is_audio_capture_active(&self) -> bool {
        self.capture.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    // ---- models -------------------------------------------------------------

    pub fn list_models(&self) -> Result<Vec<ModelEntry>, String> {
        models::list_models(&self.paths)
    }

    pub fn delete_model(&self, key: &str) -> Result<(), String> {
        models::delete_model(&self.paths, key)
    }

    /// Can `source` -> `target` be translated with the local models?
    pub fn local_translation_available(source: &str, target: &str) -> bool {
        translate::plan(source, target).is_some()
    }

    /// Downloads a model in the background. Progress arrives as
    /// `UiEvent::ModelProgress`, completion as `UiEvent::ModelDownloadFinished`.
    pub fn download_model(self: &Arc<Self>, kind: ModelKind, id: String) {
        let this = self.clone();
        self.rt.spawn(async move {
            let result = {
                let _guard = this.download_lock.lock().await;
                match kind {
                    ModelKind::Whisper => {
                        asr::ensure_model_downloaded(&this.paths, &*this.sink, &id)
                            .await
                            .map(|_| ())
                    }
                    ModelKind::Translate => {
                        translate::ensure_model_downloaded(&this.paths, &*this.sink, &id).await
                    }
                }
            };
            this.sink
                .emit(UiEvent::ModelDownloadFinished { kind, id, result });
        });
    }

    // ---- API keys -----------------------------------------------------------

    pub fn save_api_key(&self, provider: ApiProvider, key: &str) -> Result<(), String> {
        secrets::save(provider.secret_key(), key)
    }

    pub fn delete_api_key(&self, provider: ApiProvider) -> Result<(), String> {
        secrets::delete(provider.secret_key())
    }

    pub fn has_api_key(&self, provider: ApiProvider) -> Result<bool, String> {
        secrets::has(provider.secret_key())
    }

    // ---- captioning -----------------------------------------------------------

    pub fn is_captioning_active(&self) -> bool {
        matches!(
            self.captioning.lock().as_deref(),
            Ok(CaptioningState::Active(_))
        )
    }

    pub fn is_captioning_starting(&self) -> bool {
        matches!(
            self.captioning.lock().as_deref(),
            Ok(CaptioningState::Starting(_))
        )
    }

    /// Starts captioning in the background. Completion arrives as
    /// `UiEvent::CaptioningStarted` or `UiEvent::CaptioningFailed`.
    /// A no-op while already starting or running.
    pub fn start_captioning(self: &Arc<Self>, params: CaptioningParams) {
        let attempt = {
            let Ok(mut state) = self.captioning.lock() else {
                return;
            };
            if !matches!(*state, CaptioningState::Idle) {
                return;
            }
            let mut gen = self.generation.lock().expect("generation lock");
            *gen += 1;
            *state = CaptioningState::Starting(*gen);
            *gen
        };

        let this = self.clone();
        self.rt.spawn(async move {
            let result = this.build_captioning(params).await;
            let Ok(mut state) = this.captioning.lock() else {
                return;
            };
            let still_wanted = matches!(*state, CaptioningState::Starting(g) if g == attempt);
            match result {
                Ok(handle) if still_wanted => {
                    *state = CaptioningState::Active(handle);
                    drop(state);
                    this.sink.emit(UiEvent::CaptioningStarted);
                }
                Ok(handle) => {
                    // stop_captioning() was called while we were starting.
                    drop(state);
                    handle.stop();
                }
                Err(e) if still_wanted => {
                    *state = CaptioningState::Idle;
                    drop(state);
                    this.sink.emit(UiEvent::CaptioningFailed(e));
                }
                Err(_) => {}
            }
        });
    }

    async fn build_captioning(&self, p: CaptioningParams) -> Result<CaptioningHandle, String> {
        let asr_config = if p.asr_provider == Provider::Cloud {
            let key = secrets::get(ApiProvider::OpenAiAsr.secret_key())?
                .ok_or("no OpenAI API key saved for cloud speech recognition")?;
            AsrConfig::Cloud(key)
        } else {
            let model_path = {
                let _guard = self.download_lock.lock().await;
                asr::ensure_model_downloaded(&self.paths, &*self.sink, p.asr_model.as_str()).await?
            };
            AsrConfig::Local(model_path)
        };

        let translate_config = match p.target_language {
            Some(lang) if p.translate_provider == Provider::Cloud => {
                let key = secrets::get(ApiProvider::DeepL.secret_key())?
                    .ok_or("no DeepL API key saved for cloud translation")?;
                TranslateConfig::Cloud {
                    api_key: key,
                    target_lang: lang,
                }
            }
            Some(lang) => {
                // Models are loaded lazily for whichever language pair is needed
                // (the spoken language may only be known once it is detected).
                if p.language != "auto" && translate::plan(&p.language, &lang).is_none() {
                    self.sink.emit(UiEvent::TranslationUnavailable {
                        source: p.language.clone(),
                        target: lang.clone(),
                    });
                }
                TranslateConfig::Local(Box::new(translate::Router::new(
                    self.paths.clone(),
                    self.sink.clone(),
                    self.rt.handle().clone(),
                    self.download_lock.clone(),
                    &p.language,
                    &lang,
                )))
            }
            None => TranslateConfig::None,
        };

        let sink = self.sink.clone();
        let handle = self.rt.handle().clone();
        let source = p.audio_source.as_str().to_string();
        let language = p.language;
        tokio::task::spawn_blocking(move || {
            let rt = handle.clone();
            asr::start_captioning(sink, rt, asr_config, language, translate_config, source)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// Stops captioning (or cancels a start in progress) and clears the overlay.
    pub fn stop_captioning(&self) {
        let previous = {
            let Ok(mut state) = self.captioning.lock() else {
                return;
            };
            std::mem::replace(&mut *state, CaptioningState::Idle)
        };
        match previous {
            CaptioningState::Active(handle) => handle.stop(),
            CaptioningState::Starting(_) | CaptioningState::Idle => {}
        }
        self.sink
            .emit(UiEvent::Caption(crate::events::CaptionEvent::clear(
                asr::now_ms(),
            )));
        self.sink.emit(UiEvent::CaptioningStopped);
    }

    /// Stops all background work. Call before exiting.
    pub fn shutdown(&self) {
        self.stop_captioning();
        self.stop_audio_capture();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::FnSink;

    fn engine_with_log() -> (Arc<Engine>, Arc<Mutex<Vec<UiEvent>>>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let log2 = log.clone();
        let sink: SharedSink = Arc::new(FnSink(move |e| log2.lock().unwrap().push(e)));
        let engine = Engine::new(Paths::in_dir(dir.path()), sink).unwrap();
        (engine, log, dir)
    }

    #[test]
    fn stop_captioning_when_idle_clears_the_overlay() {
        let (engine, log, _dir) = engine_with_log();
        assert!(!engine.is_captioning_active());
        engine.stop_captioning();
        let events = log.lock().unwrap();
        assert!(matches!(&events[0], UiEvent::Caption(c) if c.text.is_empty()));
        assert_eq!(events[1], UiEvent::CaptioningStopped);
    }

    #[test]
    fn list_models_reports_nothing_downloaded_in_a_fresh_dir() {
        let (engine, _log, _dir) = engine_with_log();
        let models = engine.list_models().unwrap();
        assert!(models.iter().all(|m| !m.downloaded));
        assert!(Engine::local_translation_available("en", "es"));
        assert!(Engine::local_translation_available("pl", "en"));
        assert!(Engine::local_translation_available("pl", "de")); // via English
        assert!(!Engine::local_translation_available("en", "pl"));
    }

    #[test]
    fn invalid_model_download_reports_failure() {
        let (engine, log, _dir) = engine_with_log();
        engine.download_model(ModelKind::Whisper, "gigantic".into());
        for _ in 0..100 {
            if !log.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let events = log.lock().unwrap();
        assert!(matches!(
            &events[0],
            UiEvent::ModelDownloadFinished {
                kind: ModelKind::Whisper,
                result: Err(_),
                ..
            }
        ));
    }
}
