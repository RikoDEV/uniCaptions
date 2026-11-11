mod cloud;
pub mod stream;

use crate::audio;
use crate::events::{CaptionEvent, EventSink, ModelKind, SharedSink, UiEvent};
use crate::paths::Paths;
use crate::translate::Router;
use futures_util::StreamExt;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use stream::{StreamState, Word};
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
const SILENCE_HOLD_SECONDS: f32 = 2.0;
/// How often the worker checks for new audio.
const POLL_INTERVAL: Duration = Duration::from_millis(25);
/// Level-meter events are emitted at most this often.
const LEVEL_INTERVAL: Duration = Duration::from_millis(100);
const SILENCE_RMS_THRESHOLD: f32 = 0.004;
/// Once words were committed, the next pass needs a little audio: Whisper is
/// unreliable on windows much shorter than this and drops words.
const LOCAL_MIN_AFTER_COMMIT_SECONDS: f32 = 0.8;
/// Hard cap on buffered audio if recognition ever falls far behind.
const MAX_AUDIO_SECONDS: f32 = 20.0;
/// Segments Whisper is this sure contain no speech are discarded (it tends to
/// hallucinate text on silence/noise).
const NO_SPEECH_THRESHOLD: f32 = 0.6;
/// How many trailing words are sent to the translator.
const TRANSLATE_TAIL_WORDS: usize = 24;
/// Cap on whisper.cpp threads: beyond this, per-thread scheduling overhead
/// outweighs the gains on the short (few-second) windows we transcribe.
const MAX_WHISPER_THREADS: i32 = 8;

pub enum AsrConfig {
    Local(PathBuf),
    Cloud(String),
}

pub enum TranslateConfig {
    Local(Box<Router>),
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
    /// Spoken language of `text`, when known.
    source: Option<String>,
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
    fn set(&self, text: String, timestamp: u128, source: Option<String>) {
        if let Ok(mut guard) = self.pending.lock() {
            *guard = Some(PendingTranslation {
                text,
                timestamp,
                source,
            });
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
                TranslateConfig::Local(router) => {
                    router.translate(&job.text, job.source.as_deref())
                }
                TranslateConfig::Cloud {
                    api_key,
                    target_lang,
                } if job.source.as_deref() == Some(target_lang.as_str()) => None,
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
    // whisper-rs defaults to English, so "auto" must be asked for explicitly.
    params.set_language(if language == "auto" {
        None
    } else {
        Some(language)
    });

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

struct Transcription {
    words: Vec<Word>,
    /// Language Whisper detected (only meaningful when asked to auto-detect).
    detected_lang: Option<String>,
}

fn is_special_token(text: &str) -> bool {
    text.starts_with("[_") || text.starts_with("<|")
}

/// `[MUSIC]`, `(applause)` and similar non-speech annotations.
fn is_annotation(text: &str) -> bool {
    let t = text.trim();
    (t.starts_with('[') && t.ends_with(']')) || (t.starts_with('(') && t.ends_with(')'))
}

/// Groups Whisper tokens into words with timings (centisecond token times).
fn group_tokens(tokens: &[(String, f32, f32)]) -> Vec<Word> {
    let mut words: Vec<Word> = Vec::new();
    for (text, t0, t1) in tokens {
        let starts_word = text.starts_with(' ')
            || words.is_empty()
            || text.chars().next().is_some_and(stream::is_cjk);
        let piece = text.trim();
        if piece.is_empty() {
            continue;
        }
        if starts_word {
            words.push(Word {
                text: piece.to_string(),
                t0: *t0,
                t1: *t1,
            });
        } else if let Some(last) = words.last_mut() {
            last.text.push_str(piece);
            last.t1 = last.t1.max(*t1);
        }
    }
    words
}

/// Like [`transcribe_local`], but returns timed words and the detected language.
fn transcribe_words(
    state: &mut whisper_rs::WhisperState,
    chunk: &[f32],
    language: &str,
) -> Result<Transcription, String> {
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
    params.set_single_segment(true);
    // We feed the whole uncommitted window every time, so earlier text must not
    // be carried over as a prompt (that makes the output repeat/drift).
    params.set_no_context(true);
    params.set_token_timestamps(true);
    // whisper-rs defaults to English, so "auto" must be asked for explicitly.
    params.set_language(if language == "auto" {
        None
    } else {
        Some(language)
    });

    state.full(params, chunk).map_err(|e| e.to_string())?;

    let mut tokens: Vec<(String, f32, f32)> = Vec::new();
    for i in 0..state.full_n_segments() {
        let Some(seg) = state.get_segment(i) else {
            continue;
        };
        if seg.no_speech_probability() > NO_SPEECH_THRESHOLD {
            continue;
        }
        for t in 0..seg.n_tokens() {
            let Some(tok) = seg.get_token(t) else {
                continue;
            };
            let Ok(text) = tok.to_str_lossy() else {
                continue;
            };
            if is_special_token(&text) {
                continue;
            }
            let d = tok.token_data();
            tokens.push((text.into_owned(), d.t0 as f32 / 100.0, d.t1 as f32 / 100.0));
        }
    }

    let mut words = group_tokens(&tokens);
    let joined = words
        .iter()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    if is_annotation(&joined) {
        words.clear();
    }

    let detected_lang = if language == "auto" {
        whisper_rs::get_lang_str(state.full_lang_id_from_state()).map(str::to_string)
    } else {
        None
    };
    Ok(Transcription {
        words,
        detected_lang,
    })
}

/// With "auto" language, detection on short windows flips between similar
/// languages and garbles the text. Lock onto a language once two consecutive
/// detections agree, until the utterance ends.
struct LanguageLock {
    auto: bool,
    configured: String,
    candidate: Option<(String, u32)>,
    locked: Option<String>,
}

impl LanguageLock {
    fn new(configured: &str) -> Self {
        Self {
            auto: configured == "auto",
            configured: configured.to_string(),
            candidate: None,
            locked: None,
        }
    }

    /// Language to pass to Whisper for the next pass.
    fn current(&self) -> &str {
        if !self.auto {
            &self.configured
        } else {
            self.locked.as_deref().unwrap_or("auto")
        }
    }

    fn observe(&mut self, detected: Option<String>, had_words: bool) {
        if !self.auto || self.locked.is_some() || !had_words {
            return;
        }
        let Some(lang) = detected else {
            return;
        };
        let count = match &self.candidate {
            Some((c, n)) if *c == lang => n + 1,
            _ => 1,
        };
        if count >= 2 {
            self.locked = Some(lang);
            self.candidate = None;
        } else {
            self.candidate = Some((lang, count));
        }
    }

    /// The spoken language if it is known: the setting, or the locked detection.
    fn effective(&self) -> Option<String> {
        if self.auto {
            self.locked.clone()
        } else {
            Some(self.configured.clone())
        }
    }

    /// A new utterance may be in a different language.
    fn reset_utterance(&mut self) {
        self.locked = None;
        self.candidate = None;
    }
}

/// Tracks how long the input has been silent, in real time. This must not be
/// derived from the (trimmed) recognition buffer, which after a commit may hold
/// only a short gap between two words.
#[derive(Default)]
struct SilenceTracker {
    trailing: usize,
}

impl SilenceTracker {
    fn push(&mut self, samples: &[f32], threshold: f32) {
        if samples.is_empty() {
            return;
        }
        if audio::rms(samples) < threshold {
            self.trailing += samples.len();
        } else {
            self.trailing = 0;
        }
    }

    fn silent_for(&self, samples: usize) -> bool {
        self.trailing >= samples
    }
}

/// If `audio` ends in silence lasting at least `min_secs`, returns the time
/// (seconds from the start of `audio`) where that silence began. "Silent" is
/// relative to the loudness of the window, so a noisy room still has pauses.
fn trailing_silence_start(audio: &[f32], min_secs: f32) -> Option<f32> {
    const FRAME: usize = 320; // 20 ms
    let sr = TARGET_SAMPLE_RATE as f32;
    let min_frames = (min_secs * sr / FRAME as f32).ceil() as usize;
    if audio.is_empty() || audio.len() < min_frames * FRAME {
        return None;
    }
    let threshold = (audio::rms(audio) * 0.15).clamp(0.003, 0.012);
    let frames: Vec<&[f32]> = audio.chunks(FRAME).collect();
    let silent = frames
        .iter()
        .rev()
        .take_while(|f| audio::rms(f) < threshold)
        .count();
    if silent < min_frames {
        return None;
    }
    let start_sample: usize = frames[..frames.len() - silent]
        .iter()
        .map(|f| f.len())
        .sum();
    Some(start_sample as f32 / sr)
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
    match (whisper_ctx, cloud_asr_key) {
        (Some(ctx), _) => match ctx.create_state() {
            Ok(state) => run_local_loop(
                state,
                buffer,
                level,
                sink,
                stop_flag,
                language,
                translate_slot,
            ),
            Err(e) => eprintln!("failed to create whisper state: {e}"),
        },
        (None, Some(key)) => run_cloud_loop(
            key,
            buffer,
            level,
            sink,
            rt,
            stop_flag,
            language,
            translate_slot,
        ),
        (None, None) => {}
    }
}

fn emit_clear(sink: &SharedSink, slot: &Option<Arc<TranslateSlot>>) {
    if let Some(slot) = slot {
        slot.clear();
    }
    sink.emit(UiEvent::Caption(CaptionEvent::clear(now_ms())));
}

fn emit_caption(
    sink: &SharedSink,
    slot: &Option<Arc<TranslateSlot>>,
    text: String,
    source: Option<String>,
) {
    let timestamp = now_ms();
    sink.emit(UiEvent::Caption(CaptionEvent {
        text: text.clone(),
        translated_text: None,
        is_final: true,
        timestamp,
    }));
    // Translating the whole accumulated text would be slow and its result
    // always stale; translate just the latest words.
    if let Some(slot) = slot {
        slot.set(
            StreamState::tail_words(&text, TRANSLATE_TAIL_WORDS),
            timestamp,
            source,
        );
    }
}

/// Local Whisper: YouTube-style captions. Audio accumulates; the whole
/// uncommitted window is re-transcribed every few hundred ms, words that two
/// consecutive passes agree on are committed (and their audio dropped), and
/// only the last few words may still change.
fn run_local_loop(
    mut state: whisper_rs::WhisperState,
    buffer: Arc<Mutex<Vec<f32>>>,
    level: Arc<Mutex<f32>>,
    sink: SharedSink,
    stop_flag: Arc<AtomicBool>,
    language: String,
    translate_slot: Option<Arc<TranslateSlot>>,
) {
    let cadence = Cadence::new(true);
    let sr = TARGET_SAMPLE_RATE as f32;
    let silence_hold = (sr * SILENCE_HOLD_SECONDS) as usize;
    let min_after_commit = (sr * LOCAL_MIN_AFTER_COMMIT_SECONDS) as usize;
    let max_audio = (sr * MAX_AUDIO_SECONDS) as usize;

    // The first inference on a GPU backend compiles its compute pipelines,
    // which can take ~2 s. Pay that now, on silence, so the first real caption
    // is not delayed by it (audio keeps buffering meanwhile).
    let silence = vec![0.0f32; TARGET_SAMPLE_RATE as usize];
    let _ = transcribe_local(&mut state, &silence, &language);

    let mut audio: Vec<f32> = Vec::new();
    let mut new_since = 0usize;
    let mut silence = SilenceTracker::default();
    let mut stream = StreamState::new();
    let mut lang = LanguageLock::new(&language);
    let mut last_display = String::new();
    let mut last_level_emit = std::time::Instant::now();

    while !stop_flag.load(Ordering::SeqCst) {
        thread::sleep(POLL_INTERVAL);

        if last_level_emit.elapsed() >= LEVEL_INTERVAL {
            last_level_emit = std::time::Instant::now();
            if let Ok(lvl) = level.lock() {
                sink.emit(UiEvent::AudioLevel(*lvl));
            }
        }

        // Take whatever the capture thread has gathered.
        if let Ok(mut buf) = buffer.lock() {
            silence.push(&buf, SILENCE_RMS_THRESHOLD);
            new_since += buf.len();
            audio.append(&mut buf);
        }
        if audio.len() > max_audio {
            let excess = audio.len() - max_audio;
            audio.drain(..excess);
        }

        // Idle (no caption in progress) and nothing but silence buffered: keep a
        // short lead-in so the first word is not clipped, but do not accumulate.
        if stream.is_empty() && !has_recent_speech(&audio, audio.len(), SILENCE_RMS_THRESHOLD) {
            let keep = (sr * 0.5) as usize;
            if audio.len() > keep {
                audio.drain(..audio.len() - keep);
            }
            new_since = 0;
            continue;
        }

        // Speech over: clear the caption and start fresh for the next utterance.
        // (Silence is measured in real time, not on the trimmed buffer.)
        if !stream.is_empty() && silence.silent_for(silence_hold) {
            if !last_display.is_empty() {
                last_display.clear();
                emit_clear(&sink, &translate_slot);
            }
            stream.reset();
            lang.reset_utterance();
            audio.clear();
            new_since = 0;
            continue;
        }

        let min_len = if stream.is_empty() {
            cadence.min
        } else {
            min_after_commit
        };
        if audio.len() < min_len || new_since < cadence.step {
            continue;
        }
        new_since = 0;

        // Nothing but silence is buffered (everything before it is already frozen):
        // there is nothing to transcribe, and Whisper invents words on silence.
        let trailing = trailing_silence_start(&audio, stream::PAUSE_SECS);
        if trailing.is_some_and(|start| start < 0.05) {
            let keep = (sr * 0.3) as usize; // lead-in for the next word
            if audio.len() > keep {
                audio.drain(..audio.len() - keep);
            }
            continue;
        }

        let result = match transcribe_words(&mut state, &audio, lang.current()) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("whisper inference error: {e}");
                continue;
            }
        };
        lang.observe(result.detected_lang, !result.words.is_empty());

        let audio_secs = audio.len() as f32 / sr;
        let update = stream.update(result.words, audio_secs, trailing);
        if let Some(cut) = update.cut_secs {
            let n = ((cut * sr) as usize).min(audio.len());
            audio.drain(..n);
        }

        // An empty pass (Whisper heard nothing in this window) keeps what is shown:
        // only real silence, handled above, clears the caption. Clearing here made
        // the box blink off and on between passes.
        if !update.display.is_empty() && update.display != last_display {
            last_display = update.display.clone();
            emit_caption(&sink, &translate_slot, update.display, lang.effective());
        }
    }
}

/// Cloud ASR: each request is a whole window of audio and returns plain text,
/// so keep the simple rolling-window behaviour with a coarser cadence.
#[allow(clippy::too_many_arguments)]
fn run_cloud_loop(
    api_key: String,
    buffer: Arc<Mutex<Vec<f32>>>,
    level: Arc<Mutex<f32>>,
    sink: SharedSink,
    rt: Handle,
    stop_flag: Arc<AtomicBool>,
    language: String,
    translate_slot: Option<Arc<TranslateSlot>>,
) {
    let cadence = Cadence::new(false);
    let silence_hold = (TARGET_SAMPLE_RATE as f32 * SILENCE_HOLD_SECONDS) as usize;
    let mut last_display = String::new();
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
            let Ok(mut buf) = buffer.lock() else {
                continue;
            };
            let new_since_last = buf.len().saturating_sub(seen_len);
            if !cadence.ready(buf.len(), new_since_last) {
                continue;
            }
            let start = buf.len().saturating_sub(cadence.window);
            let chunk: Vec<f32> = buf[start..].to_vec();
            buf.drain(0..start);
            seen_len = buf.len();
            chunk
        };

        if !has_recent_speech(&chunk, silence_hold, SILENCE_RMS_THRESHOLD) {
            if !last_display.is_empty() {
                last_display.clear();
                emit_clear(&sink, &translate_slot);
            }
            continue;
        }

        let wav = audio::encode_wav_pcm16(&chunk, TARGET_SAMPLE_RATE);
        let (text, detected) = match rt.block_on(cloud::transcribe(&api_key, wav, &language)) {
            Ok(t) => (t.text.trim().to_string(), t.language),
            Err(e) => {
                eprintln!("cloud transcription error: {e}");
                continue;
            }
        };
        let source = if language == "auto" {
            detected.map(str::to_string)
        } else {
            Some(language.clone())
        };
        if text.is_empty() || is_annotation(&text) {
            if !last_display.is_empty() {
                last_display.clear();
                emit_clear(&sink, &translate_slot);
            }
        } else if text != last_display {
            last_display = text.clone();
            emit_caption(&sink, &translate_slot, text, source);
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

#[cfg(test)]
mod word_tests {
    use super::*;

    fn tok(t: &str, a: f32, b: f32) -> (String, f32, f32) {
        (t.to_string(), a, b)
    }

    #[test]
    fn tokens_group_into_words_with_spans() {
        let words = group_tokens(&[
            tok(" Hel", 0.0, 0.2),
            tok("lo", 0.2, 0.4),
            tok(",", 0.4, 0.45),
            tok(" world", 0.5, 0.9),
        ]);
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "Hello,");
        assert_eq!((words[0].t0, words[0].t1), (0.0, 0.45));
        assert_eq!(words[1].text, "world");
    }

    #[test]
    fn cjk_tokens_are_separate_units() {
        let words = group_tokens(&[tok("字", 0.0, 0.2), tok("幕", 0.2, 0.4)]);
        assert_eq!(words.len(), 2);
    }

    #[test]
    fn annotations_and_special_tokens_are_recognised() {
        assert!(is_special_token("[_BEG_]"));
        assert!(is_special_token("<|endoftext|>"));
        assert!(!is_special_token(" hello"));
        assert!(is_annotation("[MUSIC]"));
        assert!(is_annotation(" (applause) "));
        assert!(!is_annotation("[not closed"));
        assert!(!is_annotation("real words"));
    }

    #[test]
    fn language_lock_needs_two_agreeing_detections() {
        let mut l = LanguageLock::new("auto");
        assert_eq!(l.current(), "auto");
        l.observe(Some("pl".into()), true);
        assert_eq!(l.current(), "auto");
        l.observe(Some("ru".into()), true); // flipped: not yet trusted
        assert_eq!(l.current(), "auto");
        // alternating detections never lock
        l.observe(Some("pl".into()), true);
        assert_eq!(l.current(), "auto");
        // two in a row lock it
        let mut l = LanguageLock::new("auto");
        l.observe(Some("pl".into()), true);
        l.observe(Some("pl".into()), true);
        assert_eq!(l.current(), "pl");
        // later detections do not unlock it mid-utterance
        l.observe(Some("en".into()), true);
        assert_eq!(l.current(), "pl");
        l.reset_utterance();
        assert_eq!(l.current(), "auto");
    }

    #[test]
    fn effective_language_is_the_setting_or_the_lock() {
        let mut l = LanguageLock::new("auto");
        assert_eq!(l.effective(), None);
        l.observe(Some("pl".into()), true);
        assert_eq!(l.effective(), None, "one detection is not trusted yet");
        l.observe(Some("pl".into()), true);
        assert_eq!(l.effective().as_deref(), Some("pl"));
        l.reset_utterance();
        assert_eq!(l.effective(), None);
        assert_eq!(LanguageLock::new("de").effective().as_deref(), Some("de"));
    }

    #[test]
    fn language_lock_ignores_empty_results_and_fixed_languages() {
        let mut l = LanguageLock::new("auto");
        l.observe(Some("pl".into()), false);
        l.observe(Some("pl".into()), false);
        assert_eq!(l.current(), "auto");
        let mut fixed = LanguageLock::new("pl");
        fixed.observe(Some("en".into()), true);
        fixed.observe(Some("en".into()), true);
        assert_eq!(fixed.current(), "pl");
    }
}

/// Feeds a WAV into the real local streaming loop in real time and reports how
/// the captions evolve. Ignored: needs a model and a recording.
///
/// `UNICAPTIONS_TEST_WAV=... UNICAPTIONS_TEST_MODEL=<ggml.bin> UNICAPTIONS_TEST_LANG=pl|auto|en \
///   cargo test --release -p unicaptions-core --lib streaming_timeline -- --ignored --nocapture`
#[cfg(test)]
mod streaming_profile {
    use super::*;
    use crate::events::FnSink;
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

    #[test]
    #[ignore = "profiling aid; needs a model and a wav"]
    fn streaming_timeline() {
        let samples = read_wav_16k(&std::env::var("UNICAPTIONS_TEST_WAV").unwrap());
        let model = std::env::var("UNICAPTIONS_TEST_MODEL").unwrap();
        let lang = std::env::var("UNICAPTIONS_TEST_LANG").unwrap_or_else(|_| "auto".into());

        let ctx =
            WhisperContext::new_with_params(&model, WhisperContextParameters::default()).unwrap();
        let state = ctx.create_state().unwrap();

        let events: Arc<Mutex<Vec<(f32, UiEvent)>>> = Arc::new(Mutex::new(Vec::new()));
        let t0 = Instant::now();
        let log = events.clone();
        let sink: SharedSink = Arc::new(FnSink(move |e| {
            if matches!(e, UiEvent::Caption(_)) {
                log.lock().unwrap().push((t0.elapsed().as_secs_f32(), e));
            }
        }));

        let buffer = Arc::new(Mutex::new(Vec::<f32>::new()));
        let level = Arc::new(Mutex::new(0.0f32));
        let stop = Arc::new(AtomicBool::new(false));

        // Optional: translate into UNICAPTIONS_TEST_TARGET with the local models
        // (needs UNICAPTIONS_TEST_HOME with the model downloaded).
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let (slot, translate_handle) = match std::env::var("UNICAPTIONS_TEST_TARGET") {
            Ok(target) => {
                let paths = Paths::in_dir(std::path::Path::new(
                    &std::env::var("UNICAPTIONS_TEST_HOME").unwrap(),
                ));
                let router = Router::new(
                    paths,
                    sink.clone(),
                    rt.handle().clone(),
                    Arc::new(tokio::sync::Mutex::new(())),
                    &lang,
                    &target,
                );
                spawn_translate_worker(
                    sink.clone(),
                    rt.handle().clone(),
                    stop.clone(),
                    TranslateConfig::Local(Box::new(router)),
                )
            }
            Err(_) => (None, None),
        };
        let worker = {
            let (b, l, s, st) = (buffer.clone(), level.clone(), sink.clone(), stop.clone());
            thread::spawn(move || run_local_loop(state, b, l, s, st, lang, slot))
        };

        // Let the warm-up finish, then stream the recording in real time
        // (20 ms chunks), followed by 3.5 s of silence.
        thread::sleep(Duration::from_millis(1500));
        let feed_start = Instant::now();
        let mut padded = samples.clone();
        padded.extend(std::iter::repeat_n(0.0f32, 16_000 * 7 / 2));
        let t_feed0 = t0.elapsed().as_secs_f32();
        for (i, chunk) in padded.chunks(320).enumerate() {
            buffer.lock().unwrap().extend_from_slice(chunk);
            let due = Duration::from_millis(20 * (i as u64 + 1));
            if let Some(wait) = due.checked_sub(feed_start.elapsed()) {
                thread::sleep(wait);
            }
        }
        thread::sleep(Duration::from_millis(500));
        stop.store(true, Ordering::SeqCst);
        worker.join().unwrap();
        if let Some(h) = translate_handle {
            h.join().unwrap();
        }

        let ev = events.lock().unwrap();
        println!(
            "audio: {:.1}s speech + 3.5s silence",
            samples.len() as f32 / 16_000.0
        );
        let mut prev = String::new();
        let (mut revisions, mut regress) = (0, 0);
        for (t, e) in ev.iter() {
            let UiEvent::Caption(c) = e else { continue };
            let rel = t - t_feed0;
            if c.translated_text.is_some() {
                println!(
                    "  +{rel:5.2}s  [translation] {:?}",
                    c.translated_text.as_deref().unwrap()
                );
                continue;
            }
            println!("  +{rel:5.2}s  {:?}", c.text);
            if !prev.is_empty() && !c.text.is_empty() {
                revisions += 1;
                // everything except the last ~7 words (about 2.5 s of speech) of the previous caption
                // must still be a prefix of the new one
                let words: Vec<&str> = prev.split_whitespace().collect();
                let stable = words[..words.len().saturating_sub(7)].join(" ");
                let flat = c.text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !flat.starts_with(&stable) {
                    regress += 1;
                    println!("    ^ head changed (expected prefix {stable:?})");
                }
            }
            prev = c.text.clone();
        }
        println!("updates: {revisions}, head regressions: {regress}");
    }
}

#[cfg(test)]
mod silence_tests {
    use super::*;

    /// 16 kHz test signal: `(seconds, loud)` segments.
    fn signal(segments: &[(f32, bool)]) -> Vec<f32> {
        let mut out = Vec::new();
        for (secs, loud) in segments {
            let n = (secs * 16_000.0) as usize;
            out.extend((0..n).map(|i| {
                if *loud {
                    0.2 * ((i as f32) * 0.3).sin()
                } else {
                    0.0
                }
            }));
        }
        out
    }

    #[test]
    fn a_pause_at_the_end_is_found_with_its_start_time() {
        // speech 0-1.5 s, then 1 s of silence
        let a = signal(&[(1.5, true), (1.0, false)]);
        let start = trailing_silence_start(&a, 0.6).expect("pause");
        assert!((1.4..=1.6).contains(&start), "start = {start}");
    }

    #[test]
    fn no_pause_while_speech_continues_to_the_end() {
        let a = signal(&[(0.5, false), (2.0, true)]);
        assert_eq!(trailing_silence_start(&a, 0.6), None);
    }

    #[test]
    fn a_too_short_pause_is_not_a_pause() {
        let a = signal(&[(1.5, true), (0.3, false)]);
        assert_eq!(trailing_silence_start(&a, 0.6), None);
        assert_eq!(trailing_silence_start(&[], 0.6), None);
    }

    #[test]
    fn a_silence_in_the_middle_is_ignored() {
        let a = signal(&[(1.0, true), (1.0, false), (1.0, true)]);
        assert_eq!(trailing_silence_start(&a, 0.6), None);
    }

    #[test]
    fn all_silent_audio_reports_a_pause_from_the_start() {
        let a = signal(&[(1.5, false)]);
        let start = trailing_silence_start(&a, 0.6).expect("pause");
        assert!(start < 0.05, "start = {start}");
    }

    #[test]
    fn silence_is_tracked_across_pushes_and_reset_by_speech() {
        let mut t = SilenceTracker::default();
        let quiet = vec![0.0f32; 400];
        let loud = vec![0.2f32; 400];
        for _ in 0..10 {
            t.push(&quiet, 0.004);
        }
        assert!(t.silent_for(4_000));
        assert!(!t.silent_for(4_001));
        t.push(&loud, 0.004);
        assert!(!t.silent_for(1));
        t.push(&[], 0.004);
        assert!(!t.silent_for(1));
    }
}
