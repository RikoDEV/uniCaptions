mod cloud;

use crate::audio;
use crate::translate::Translator;
use futures_util::StreamExt;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const WHISPER_SIZES: [&str; 4] = ["tiny", "base", "small", "medium"];
const TARGET_SAMPLE_RATE: u32 = 16_000;
/// Audio context handed to whisper per inference call. Kept at 3s for
/// transcription quality (shorter windows starve whisper of context and hurt
/// accuracy); latency is instead cut by sliding this window forward in
/// smaller STEP_SECONDS increments rather than shrinking it. See
/// STEP_SECONDS below.
const WINDOW_SECONDS: usize = 3;
/// How far the window advances (and how often we transcribe) between calls.
/// Smaller than WINDOW_SECONDS so windows overlap: this is the same
/// step/length/overlap approach whisper.cpp's own streaming example uses.
/// Roughly halves the wait for a caption update vs. re-filling the whole
/// window from empty, at the cost of ~2x more whisper calls per second of
/// audio (mitigated by GPU acceleration where available).
const STEP_SECONDS: f32 = 1.5;
const SILENCE_RMS_THRESHOLD: f32 = 0.004;
/// Cap on whisper.cpp threads: beyond this, per-thread scheduling overhead
/// outweighs the gains on the short (few-second) windows we transcribe.
const MAX_WHISPER_THREADS: i32 = 8;

pub enum AsrConfig {
    Local(PathBuf),
    Cloud(String),
}

pub enum TranslateConfig {
    Local(Translator),
    Cloud { api_key: String, target_lang: String },
    None,
}

pub struct CaptioningHandle {
    stop_flag: Arc<AtomicBool>,
    capture: Option<audio::CaptureHandle>,
    worker: Option<thread::JoinHandle<()>>,
    translate_worker: Option<thread::JoinHandle<()>>,
}

impl CaptioningHandle {
    pub fn stop(mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Some(c) = self.capture.take() {
            c.stop();
        }
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        if let Some(w) = self.translate_worker.take() {
            let _ = w.join();
        }
    }
}

struct PendingTranslation {
    text: String,
    timestamp: u128,
}

/// Hands the latest untranslated caption off to the translation worker
/// thread. Only the most recent text is kept: if translation (a blocking,
/// non-KV-cached autoregressive decode for local models) falls behind the
/// rate of new captions, we want the freshest text next, not a growing
/// backlog of stale ones.
struct TranslateSlot {
    pending: Mutex<Option<PendingTranslation>>,
    cvar: Condvar,
}

impl TranslateSlot {
    fn set(&self, text: String, timestamp: u128) {
        if let Ok(mut guard) = self.pending.lock() {
            *guard = Some(PendingTranslation { text, timestamp });
            self.cvar.notify_one();
        }
    }

    fn clear(&self) {
        if let Ok(mut guard) = self.pending.lock() {
            *guard = None;
        }
    }
}

/// Spawns a dedicated thread that translates captions off the ASR hot path,
/// so a slow translation (cloud round-trip, or the non-KV-cached local ONNX
/// decode loop) never delays the next transcription window or the original-
/// language caption reaching the overlay. Returns `None`/`None` when
/// translation is disabled, so callers can skip the hand-off entirely.
fn spawn_translate_worker(
    app: AppHandle,
    stop_flag: Arc<AtomicBool>,
    mut translate_config: TranslateConfig,
) -> (Option<Arc<TranslateSlot>>, Option<thread::JoinHandle<()>>) {
    if matches!(translate_config, TranslateConfig::None) {
        return (None, None);
    }

    let slot = Arc::new(TranslateSlot {
        pending: Mutex::new(None),
        cvar: Condvar::new(),
    });
    let slot_worker = slot.clone();

    let handle = thread::Builder::new()
        .name("unicaptions-translate".into())
        .spawn(move || loop {
            let job = {
                let guard = match slot_worker.pending.lock() {
                    Ok(g) => g,
                    Err(_) => return,
                };
                let mut guard = guard;
                loop {
                    if let Some(job) = guard.take() {
                        break job;
                    }
                    if stop_flag.load(Ordering::SeqCst) {
                        return;
                    }
                    guard = match slot_worker
                        .cvar
                        .wait_timeout(guard, Duration::from_millis(200))
                    {
                        Ok((g, _)) => g,
                        Err(_) => return,
                    };
                }
            };

            let translated = match &mut translate_config {
                TranslateConfig::Local(translator) => match translator.translate(&job.text) {
                    Ok(t) => Some(t),
                    Err(e) => {
                        eprintln!("translation error: {e}");
                        None
                    }
                },
                TranslateConfig::Cloud { api_key, target_lang } => {
                    match tauri::async_runtime::block_on(crate::translate::cloud::translate(
                        api_key,
                        &job.text,
                        target_lang,
                    )) {
                        Ok(t) => Some(t),
                        Err(e) => {
                            eprintln!("cloud translation error: {e}");
                            None
                        }
                    }
                }
                TranslateConfig::None => None,
            };

            if let Some(translated) = translated {
                let _ = app.emit(
                    "caption-update",
                    serde_json::json!({
                        "text": job.text,
                        "translatedText": translated,
                        "isFinal": true,
                        "timestamp": job.timestamp,
                    }),
                );
            }
        })
        .ok();

    (Some(slot), handle)
}

fn validate_size(size: &str) -> Result<&str, String> {
    if WHISPER_SIZES.contains(&size) {
        Ok(size)
    } else {
        Err(format!("unknown whisper model size '{size}'"))
    }
}

fn model_filename(size: &str) -> String {
    format!("ggml-{size}.bin")
}

fn model_url(size: &str) -> String {
    format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        model_filename(size)
    )
}

fn model_path(app: &AppHandle, size: &str) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("models").join("whisper").join(model_filename(size)))
}

/// Info about a Whisper model size: whether it's downloaded and its size on disk.
pub struct WhisperModelInfo {
    pub size: String,
    pub downloaded: bool,
    pub bytes_on_disk: u64,
}

pub fn list_whisper_models(app: &AppHandle) -> Result<Vec<WhisperModelInfo>, String> {
    WHISPER_SIZES
        .iter()
        .map(|&size| {
            let path = model_path(app, size)?;
            let (downloaded, bytes_on_disk) = match fs::metadata(&path) {
                Ok(meta) => (true, meta.len()),
                Err(_) => (false, 0),
            };
            Ok(WhisperModelInfo {
                size: size.to_string(),
                downloaded,
                bytes_on_disk,
            })
        })
        .collect()
}

pub fn delete_whisper_model(app: &AppHandle, size: &str) -> Result<(), String> {
    let path = model_path(app, validate_size(size)?)?;
    if path.exists() {
        fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Downloads the given Whisper model size on first use, emitting
/// `model-download-progress` events (0.0-1.0) as it streams.
pub async fn ensure_model_downloaded(app: AppHandle, size: String) -> Result<PathBuf, String> {
    let size = validate_size(&size)?.to_string();
    let path = model_path(&app, &size)?;
    if path.exists() {
        return Ok(path);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let response = reqwest::get(model_url(&size)).await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("model download failed: HTTP {}", response.status()));
    }
    let total = response.content_length().unwrap_or(0);
    let mut downloaded: u64 = 0;
    let tmp_path = path.with_extension("part");
    let mut file = fs::File::create(&tmp_path).map_err(|e| e.to_string())?;

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;
        let progress = if total > 0 {
            downloaded as f64 / total as f64
        } else {
            0.0
        };
        let _ = app.emit(
            "model-download-progress",
            serde_json::json!({ "size": size, "progress": progress }),
        );
    }
    drop(file);
    fs::rename(&tmp_path, &path).map_err(|e| e.to_string())?;
    let _ = app.emit(
        "model-download-progress",
        serde_json::json!({ "size": size, "progress": 1.0 }),
    );
    Ok(path)
}

pub fn start_captioning(
    app: AppHandle,
    asr_config: AsrConfig,
    language: String,
    translate_config: TranslateConfig,
    audio_source: String,
) -> Result<CaptioningHandle, String> {
    let whisper_ctx = match &asr_config {
        AsrConfig::Local(model_path) => Some(
            WhisperContext::new_with_params(
                model_path.to_str().ok_or("invalid model path")?,
                WhisperContextParameters::default(),
            )
            .map_err(|e| format!("failed to load whisper model: {e}"))?,
        ),
        AsrConfig::Cloud(_) => None,
    };
    let cloud_asr_key = match asr_config {
        AsrConfig::Cloud(key) => Some(key),
        AsrConfig::Local(_) => None,
    };

    let stop_flag = Arc::new(AtomicBool::new(false));
    let buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let buffer_capture = buffer.clone();
    let level: Arc<Mutex<f32>> = Arc::new(Mutex::new(0.0));
    let level_capture = level.clone();

    let capture = audio::start_capture_stream(&audio_source, move |data, sample_rate, channels| {
        let mono = audio::downmix(data, channels);
        if let Ok(mut l) = level_capture.lock() {
            *l = audio::rms(&mono);
        }
        let resampled = audio::resample_linear(&mono, sample_rate, TARGET_SAMPLE_RATE);
        if let Ok(mut buf) = buffer_capture.lock() {
            buf.extend_from_slice(&resampled);
        }
    })?;

    let (translate_slot, translate_worker) =
        spawn_translate_worker(app.clone(), stop_flag.clone(), translate_config);

    let stop_worker = stop_flag.clone();
    let app_worker = app.clone();
    let worker = thread::Builder::new()
        .name("unicaptions-asr".into())
        .spawn(move || {
            run_asr_loop(
                whisper_ctx,
                cloud_asr_key,
                buffer,
                level,
                app_worker,
                stop_worker,
                language,
                translate_slot,
            )
        })
        .map_err(|e| e.to_string())?;

    Ok(CaptioningHandle {
        stop_flag,
        capture: Some(capture),
        worker: Some(worker),
        translate_worker,
    })
}

fn transcribe_local(
    state: &mut whisper_rs::WhisperState,
    chunk: &[f32],
    language: &str,
) -> Result<String, String> {
    let n_threads = thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4)
        .min(MAX_WHISPER_THREADS);

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_print_progress(false);
    params.set_print_special(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_n_threads(n_threads);
    // Each call transcribes one fixed-length window in isolation, so there's
    // no benefit to whisper.cpp's multi-segment splitting/timestamping work.
    params.set_single_segment(true);
    if language != "auto" {
        params.set_language(Some(language));
    }

    state.full(params, chunk).map_err(|e| e.to_string())?;

    let num_segments = state.full_n_segments();
    let mut text = String::new();
    for i in 0..num_segments {
        if let Some(seg) = state.get_segment(i) {
            if let Ok(s) = seg.to_str() {
                text.push_str(s);
            }
        }
    }
    Ok(text)
}

#[allow(clippy::too_many_arguments)]
fn run_asr_loop(
    whisper_ctx: Option<WhisperContext>,
    cloud_asr_key: Option<String>,
    buffer: Arc<Mutex<Vec<f32>>>,
    level: Arc<Mutex<f32>>,
    app: AppHandle,
    stop_flag: Arc<AtomicBool>,
    language: String,
    translate_slot: Option<Arc<TranslateSlot>>,
) {
    let window_size = TARGET_SAMPLE_RATE as usize * WINDOW_SECONDS;
    let step_size = (TARGET_SAMPLE_RATE as f32 * STEP_SECONDS) as usize;
    let overlap_size = window_size - step_size;
    let mut whisper_state = whisper_ctx.as_ref().and_then(|ctx| match ctx.create_state() {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("failed to create whisper state: {e}");
            None
        }
    });
    let mut last_caption_was_empty = true;

    while !stop_flag.load(Ordering::SeqCst) {
        // Poll cadence, not the transcription latency floor: the window
        // still has to fill (WINDOW_SECONDS) before there's anything new to
        // transcribe, but a shorter poll means less time waiting to notice
        // that it has.
        thread::sleep(Duration::from_millis(100));

        if let Ok(lvl) = level.lock() {
            let _ = app.emit("audio-level", *lvl);
        }

        let chunk = {
            let mut buf = match buffer.lock() {
                Ok(b) => b,
                Err(_) => continue,
            };
            if buf.len() < window_size {
                continue;
            }
            // Always transcribe the most recent window, not the oldest
            // backlog. Transcription is synchronous and can occasionally
            // run slower than real-time (bigger model, slower CPU); if we
            // only ever drained from the front, the buffer would keep
            // growing and captions would fall further and further behind.
            // Dropping stale audio keeps captions live instead of catching
            // up through a growing queue.
            let start = buf.len() - window_size;
            let chunk: Vec<f32> = buf[start..].to_vec();
            // Keep only the trailing `overlap_size` samples (the second
            // half of this window) so the next window slides forward by
            // `step_size` instead of starting from empty — this is what
            // lets us transcribe every STEP_SECONDS instead of waiting a
            // full WINDOW_SECONDS between captions. Also correctly drops
            // any backlog beyond one window, same as the old buf.clear().
            let keep_from = buf.len() - overlap_size;
            buf.drain(0..keep_from);
            chunk
        };

        if audio::rms(&chunk) < SILENCE_RMS_THRESHOLD {
            // Speech has stopped (e.g. a paused video) — clear the overlay
            // instead of leaving the last caption stuck on screen forever.
            if !last_caption_was_empty {
                last_caption_was_empty = true;
                if let Some(slot) = &translate_slot {
                    slot.clear();
                }
                let _ = app.emit(
                    "caption-update",
                    serde_json::json!({
                        "text": "",
                        "translatedText": null,
                        "isFinal": true,
                        "timestamp": now_ms(),
                    }),
                );
            }
            continue;
        }

        let text = if let Some(state) = whisper_state.as_mut() {
            match transcribe_local(state, &chunk, &language) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("whisper inference error: {e}");
                    continue;
                }
            }
        } else if let Some(api_key) = &cloud_asr_key {
            let wav = audio::encode_wav_pcm16(&chunk, TARGET_SAMPLE_RATE);
            let lang = language.clone();
            let key = api_key.clone();
            match tauri::async_runtime::block_on(cloud::transcribe(&key, wav, &lang)) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("cloud transcription error: {e}");
                    continue;
                }
            }
        } else {
            continue;
        };

        let text = text.trim().to_string();
        if text.is_empty() {
            if !last_caption_was_empty {
                last_caption_was_empty = true;
                if let Some(slot) = &translate_slot {
                    slot.clear();
                }
                let _ = app.emit(
                    "caption-update",
                    serde_json::json!({
                        "text": "",
                        "translatedText": null,
                        "isFinal": true,
                        "timestamp": now_ms(),
                    }),
                );
            }
            continue;
        }
        last_caption_was_empty = false;
        let timestamp = now_ms();

        // Emit the original-language caption immediately rather than
        // blocking on translation (a cloud round-trip, or the local ONNX
        // decode loop, which has no KV cache and is easily the single
        // slowest step in the pipeline). The translation worker thread
        // fills in `translatedText` with a follow-up event once it's ready.
        let _ = app.emit(
            "caption-update",
            serde_json::json!({
                "text": text,
                "translatedText": null,
                "isFinal": true,
                "timestamp": timestamp,
            }),
        );

        if let Some(slot) = &translate_slot {
            slot.set(text, timestamp);
        }
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
