mod cloud;

use crate::audio;
use crate::events::{CaptionEvent, EventSink, ModelKind, SharedSink, UiEvent};
use crate::paths::Paths;
use crate::translate::Translator;
use futures_util::StreamExt;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::runtime::Handle;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const WHISPER_SIZES: [&str; 4] = ["tiny", "base", "small", "medium"];
const TARGET_SAMPLE_RATE: u32 = 16_000;
/// Longest audio context handed to whisper per call. Longer context gives
/// better accuracy; shorter windows keep the text responsive.
const WINDOW_SECONDS: f32 = 3.0;
/// Local models are fast (tens to a few hundred ms per window on a GPU), so we
/// show a first caption as soon as there is this much audio and then refresh
/// it every `LOCAL_STEP_SECONDS` of new audio over a rolling window. (The old
/// design waited to fill a whole 3 s window and then fired every 1.5 s, which
/// added 2-4.5 s of avoidable delay.)
const LOCAL_MIN_SECONDS: f32 = 1.0;
const LOCAL_STEP_SECONDS: f32 = 0.4;
/// Cloud ASR costs money per request and has network latency, so it keeps a
/// coarser cadence.
const CLOUD_MIN_SECONDS: f32 = 1.5;
const CLOUD_STEP_SECONDS: f32 = 1.5;
/// A caption is cleared once this much trailing audio is silent. Holding it a
/// little keeps captions readable across short pauses.
const SILENCE_HOLD_SECONDS: f32 = 1.5;
/// How often the worker checks for new audio.
const POLL_INTERVAL: Duration = Duration::from_millis(25);
/// Level-meter events are emitted at most this often.
const LEVEL_INTERVAL: Duration = Duration::from_millis(100);
const SILENCE_RMS_THRESHOLD: f32 = 0.004;
/// Cap on whisper.cpp threads: beyond this, per-thread scheduling overhead
/// outweighs the gains on the short (few-second) windows we transcribe.
const MAX_WHISPER_THREADS: i32 = 8;

pub enum AsrConfig {
    Local(PathBuf),
    Cloud(String),
}

pub enum TranslateConfig {
    Local(Box<Translator>),
    Cloud {
        api_key: String,
        target_lang: String,
    },
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
    sink: SharedSink,
    rt: Handle,
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
                TranslateConfig::Cloud {
                    api_key,
                    target_lang,
                } => {
                    match rt.block_on(crate::translate::cloud::translate(
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
                sink.emit(UiEvent::Caption(CaptionEvent {
                    text: job.text,
                    translated_text: Some(translated),
                    is_final: true,
                    timestamp: job.timestamp,
                }));
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

fn model_path(paths: &Paths, size: &str) -> PathBuf {
    paths.whisper_model_file(size)
}

/// Info about a Whisper model size: whether it's downloaded and its size on disk.
pub struct WhisperModelInfo {
    pub size: String,
    pub downloaded: bool,
    pub bytes_on_disk: u64,
}

pub fn list_whisper_models(paths: &Paths) -> Result<Vec<WhisperModelInfo>, String> {
    WHISPER_SIZES
        .iter()
        .map(|&size| {
            let path = model_path(paths, size);
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

pub fn delete_whisper_model(paths: &Paths, size: &str) -> Result<(), String> {
    let path = model_path(paths, validate_size(size)?);
    if path.exists() {
        fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Downloads the given Whisper model size on first use, emitting
/// `ModelProgress` events (0.0-1.0) as it streams.
pub async fn ensure_model_downloaded(
    paths: &Paths,
    sink: &dyn EventSink,
    size: &str,
) -> Result<PathBuf, String> {
    let size = validate_size(size)?.to_string();
    let path = model_path(paths, &size);
    if path.exists() {
        return Ok(path);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let response = reqwest::get(model_url(&size))
        .await
        .map_err(|e| e.to_string())?;
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
        sink.emit(UiEvent::ModelProgress {
            kind: ModelKind::Whisper,
            id: size.clone(),
            progress,
        });
    }
    drop(file);
    fs::rename(&tmp_path, &path).map_err(|e| e.to_string())?;
    sink.emit(UiEvent::ModelProgress {
        kind: ModelKind::Whisper,
        id: size.clone(),
        progress: 1.0,
    });
    Ok(path)
}

pub fn start_captioning(
    sink: SharedSink,
    rt: Handle,
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

    let capture =
        audio::start_capture_stream(&audio_source, move |data, sample_rate, channels| {
            let mono = audio::downmix(data, channels);
            if let Ok(mut l) = level_capture.lock() {
                *l = audio::rms(&mono);
            }
            let resampled = audio::resample_linear(&mono, sample_rate, TARGET_SAMPLE_RATE);
            if let Ok(mut buf) = buffer_capture.lock() {
                buf.extend_from_slice(&resampled);
            }
        })?;

    let (translate_slot, translate_worker) = spawn_translate_worker(
        sink.clone(),
        rt.clone(),
        stop_flag.clone(),
        translate_config,
    );

    let stop_worker = stop_flag.clone();
    let worker = thread::Builder::new()
        .name("unicaptions-asr".into())
        .spawn(move || {
            run_asr_loop(
                whisper_ctx,
                cloud_asr_key,
                buffer,
                level,
                sink,
                rt,
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

/// One-shot transcription of 16 kHz mono samples with a local Whisper model.
/// Handy for tests and tooling; live captioning uses a streaming loop instead.
pub fn transcribe_samples(
    model_path: &std::path::Path,
    samples_16k: &[f32],
    language: &str,
) -> Result<String, String> {
    let ctx = WhisperContext::new_with_params(
        model_path.to_str().ok_or("invalid model path")?,
        WhisperContextParameters::default(),
    )
    .map_err(|e| format!("failed to load whisper model: {e}"))?;
    let mut state = ctx.create_state().map_err(|e| e.to_string())?;
    transcribe_local(&mut state, samples_16k, language)
}

/// When to run the recogniser, in samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cadence {
    window: usize,
    min: usize,
    step: usize,
}

impl Cadence {
    fn new(local: bool) -> Self {
        let samples = |secs: f32| (TARGET_SAMPLE_RATE as f32 * secs) as usize;
        let (min, step) = if local {
            (LOCAL_MIN_SECONDS, LOCAL_STEP_SECONDS)
        } else {
            (CLOUD_MIN_SECONDS, CLOUD_STEP_SECONDS)
        };
        Self {
            window: samples(WINDOW_SECONDS),
            min: samples(min),
            step: samples(step),
        }
    }

    /// `buffered` samples are available, `new_since_last` of them arrived since
    /// the previous run.
    fn ready(&self, buffered: usize, new_since_last: usize) -> bool {
        buffered >= self.min && new_since_last >= self.step
    }
}

/// Is any 100 ms frame within the trailing `hold` samples louder than `threshold`?
fn has_recent_speech(samples: &[f32], hold: usize, threshold: f32) -> bool {
    const FRAME: usize = (TARGET_SAMPLE_RATE as usize) / 10;
    let tail = &samples[samples.len().saturating_sub(hold)..];
    tail.chunks(FRAME).any(|f| audio::rms(f) >= threshold)
}

#[allow(clippy::too_many_arguments)]
fn run_asr_loop(
    whisper_ctx: Option<WhisperContext>,
    cloud_asr_key: Option<String>,
    buffer: Arc<Mutex<Vec<f32>>>,
    level: Arc<Mutex<f32>>,
    sink: SharedSink,
    rt: Handle,
    stop_flag: Arc<AtomicBool>,
    language: String,
    translate_slot: Option<Arc<TranslateSlot>>,
) {
    let cadence = Cadence::new(cloud_asr_key.is_none());
    let silence_hold = (TARGET_SAMPLE_RATE as f32 * SILENCE_HOLD_SECONDS) as usize;
    let mut whisper_state = whisper_ctx
        .as_ref()
        .and_then(|ctx| match ctx.create_state() {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("failed to create whisper state: {e}");
                None
            }
        });
    // The first inference on a GPU backend compiles its compute pipelines,
    // which can take ~2 s. Pay that now, on silence, so the first real caption
    // is not delayed by it (audio keeps buffering meanwhile).
    if let Some(state) = whisper_state.as_mut() {
        let silence = vec![0.0f32; TARGET_SAMPLE_RATE as usize];
        let _ = transcribe_local(state, &silence, &language);
    }
    let mut last_caption_was_empty = true;
    // Buffer length right after the previous run, to count newly arrived audio.
    let mut seen_len = 0usize;
    let mut last_level_emit = std::time::Instant::now();

    while !stop_flag.load(Ordering::SeqCst) {
        thread::sleep(POLL_INTERVAL);

        if last_level_emit.elapsed() >= LEVEL_INTERVAL {
            last_level_emit = std::time::Instant::now();
            if let Ok(lvl) = level.lock() {
                sink.emit(UiEvent::AudioLevel(*lvl));
            }
        }

        let chunk = {
            let mut buf = match buffer.lock() {
                Ok(b) => b,
                Err(_) => continue,
            };
            let new_since_last = buf.len().saturating_sub(seen_len);
            if !cadence.ready(buf.len(), new_since_last) {
                continue;
            }
            // Always transcribe the most recent window, not the oldest
            // backlog: if inference is ever slower than real time (bigger
            // model, slower CPU) captions stay live instead of falling
            // further behind. The buffer keeps just one window of history.
            let start = buf.len().saturating_sub(cadence.window);
            let chunk: Vec<f32> = buf[start..].to_vec();
            buf.drain(0..start);
            seen_len = buf.len();
            chunk
        };

        if !has_recent_speech(&chunk, silence_hold, SILENCE_RMS_THRESHOLD) {
            // Speech has stopped (e.g. a paused video) — clear the overlay
            // instead of leaving the last caption stuck on screen forever.
            if !last_caption_was_empty {
                last_caption_was_empty = true;
                if let Some(slot) = &translate_slot {
                    slot.clear();
                }
                sink.emit(UiEvent::Caption(CaptionEvent::clear(now_ms())));
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
            match rt.block_on(cloud::transcribe(&key, wav, &lang)) {
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
                sink.emit(UiEvent::Caption(CaptionEvent::clear(now_ms())));
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
        sink.emit(UiEvent::Caption(CaptionEvent {
            text: text.clone(),
            translated_text: None,
            is_final: true,
            timestamp,
        }));

        if let Some(slot) = &translate_slot {
            slot.set(text, timestamp);
        }
    }
}

pub(crate) fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod latency_profile {
    use super::*;
    use std::time::Instant;

    fn read_wav_16k(path: &str) -> Vec<f32> {
        let bytes = std::fs::read(path).unwrap();
        let (mut pos, mut rate, mut ch) = (12usize, 0u32, 1u16);
        let mut data: &[u8] = &[];
        while pos + 8 <= bytes.len() {
            let id = &bytes[pos..pos + 4];
            let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
            let body = &bytes[pos + 8..(pos + 8 + size).min(bytes.len())];
            if id == b"fmt " {
                ch = u16::from_le_bytes(body[2..4].try_into().unwrap());
                rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            } else if id == b"data" {
                data = body;
            }
            pos += 8 + size + (size & 1);
        }
        #[allow(clippy::chunks_exact_to_as_chunks)]
        let il: Vec<f32> = data
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect();
        audio::resample_linear(&audio::downmix(&il, ch as usize), rate, 16_000)
    }

    /// Prints per-call inference time for different window sizes and models.
    /// `UNICAPTIONS_TEST_WAV=... UNICAPTIONS_TEST_MODELS=tiny=<path>,base=<path> cargo test --release -p unicaptions-core latency_profile -- --ignored --nocapture`
    #[test]
    #[ignore = "profiling aid; needs models and a wav"]
    fn inference_time_per_window() {
        let samples = read_wav_16k(&std::env::var("UNICAPTIONS_TEST_WAV").unwrap());
        for spec in std::env::var("UNICAPTIONS_TEST_MODELS").unwrap().split(',') {
            let (name, path) = spec.split_once('=').unwrap();
            let t = Instant::now();
            let ctx =
                WhisperContext::new_with_params(path, WhisperContextParameters::default()).unwrap();
            let mut state = ctx.create_state().unwrap();
            println!("[{name}] model load + state: {:?}", t.elapsed());
            for secs in [3.0f32, 3.0, 3.0, 1.5, 1.0, 3.0] {
                let n = (secs * 16_000.0) as usize;
                let chunk = &samples[16_000..(16_000 + n).min(samples.len())];
                let t = Instant::now();
                let text = transcribe_local(&mut state, chunk, "en").unwrap();
                println!("[{name}] {secs}s window -> {:?}  {text:?}", t.elapsed());
            }
        }
    }
}

#[cfg(test)]
mod cadence_tests {
    use super::*;

    #[test]
    fn local_cadence_shows_a_caption_after_one_second() {
        let c = Cadence::new(true);
        assert_eq!(c.window, 48_000);
        assert!(!c.ready(8_000, 8_000), "0.5 s is too little audio");
        assert!(c.ready(16_000, 16_000), "1 s of audio fires immediately");
        assert!(!c.ready(40_000, 3_000), "wait for ~0.4 s of new audio");
        assert!(c.ready(40_000, 6_400));
    }

    #[test]
    fn cloud_cadence_is_coarser() {
        let c = Cadence::new(false);
        assert!(!c.ready(16_000, 16_000));
        assert!(c.ready(24_000, 24_000));
        assert!(!c.ready(48_000, 12_000));
    }

    #[test]
    fn first_caption_latency_is_a_fraction_of_the_old_design() {
        // Old design: nothing until 3 s of audio were buffered.
        let c = Cadence::new(true);
        let first_ready_secs = (1..=1000)
            .map(|i| i * 160) // 10 ms of audio per tick
            .find(|&n| c.ready(n, n))
            .unwrap() as f32
            / 16_000.0;
        assert!(first_ready_secs <= 1.01, "{first_ready_secs}");
    }

    #[test]
    fn detects_recent_speech_only_in_the_trailing_window() {
        let mut audio = vec![0.0f32; 48_000];
        // speech early in the window, silence for the last 2 s
        for s in &mut audio[..16_000] {
            *s = 0.2;
        }
        let hold = 24_000;
        assert!(!has_recent_speech(&audio, hold, 0.004));
        assert!(has_recent_speech(&audio, 48_000, 0.004));
        // speech right at the end
        for s in &mut audio[44_000..] {
            *s = 0.2;
        }
        assert!(has_recent_speech(&audio, hold, 0.004));
        assert!(!has_recent_speech(&[], hold, 0.004));
    }

    #[test]
    fn a_short_word_in_a_long_silent_window_still_counts_as_speech() {
        // The old whole-window RMS check diluted a short utterance below the
        // threshold, delaying the first caption.
        let mut audio = vec![0.0f32; 48_000];
        for s in &mut audio[40_000..43_200] {
            *s = 0.15;
        }
        assert!(audio::rms(&audio) < 0.05);
        assert!(has_recent_speech(&audio, 24_000, SILENCE_RMS_THRESHOLD));
    }
}
