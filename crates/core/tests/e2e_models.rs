//! End-to-end tests against the real models. They download hundreds of MB and
//! need network access, so they are `#[ignore]`d; run them with
//!
//! ```text
//! UNICAPTIONS_TEST_HOME=<scratch dir> UNICAPTIONS_TEST_WAV=<speech.wav> \
//!     cargo test -p unicaptions-core --test e2e_models -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use unicaptions_core::events::NullSink;
use unicaptions_core::{asr, audio, translate, Paths};

fn test_paths() -> Paths {
    let home = std::env::var_os("UNICAPTIONS_TEST_HOME")
        .map(PathBuf::from)
        .expect("set UNICAPTIONS_TEST_HOME to a scratch directory");
    Paths::in_dir(&home)
}

/// Minimal RIFF/WAVE reader: 16-bit PCM, mono or stereo (stereo is downmixed).
fn read_wav(path: &str) -> (Vec<f32>, u32) {
    let bytes = std::fs::read(path).expect("read wav");
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WAVE");
    let (mut pos, mut rate, mut channels, mut bits) = (12usize, 0u32, 0u16, 0u16);
    let mut data: &[u8] = &[];
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = &bytes[pos + 8..(pos + 8 + size).min(bytes.len())];
        if id == b"fmt " {
            channels = u16::from_le_bytes(body[2..4].try_into().unwrap());
            rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            bits = u16::from_le_bytes(body[14..16].try_into().unwrap());
        } else if id == b"data" {
            data = body;
        }
        pos += 8 + size + (size & 1);
    }
    assert_eq!(bits, 16, "only 16-bit PCM supported");
    #[allow(clippy::chunks_exact_to_as_chunks)]
    let interleaved: Vec<f32> = data
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / i16::MAX as f32)
        .collect();
    (audio::downmix(&interleaved, channels as usize), rate)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads the Whisper tiny model (~75 MB)"]
async fn whisper_tiny_transcribes_synthesized_speech() {
    let wav = std::env::var("UNICAPTIONS_TEST_WAV").expect("set UNICAPTIONS_TEST_WAV");
    let paths = test_paths();
    let model = asr::ensure_model_downloaded(&paths, &NullSink, "tiny")
        .await
        .expect("download model");

    let (mono, rate) = read_wav(&wav);
    let samples = audio::resample_linear(&mono, rate, 16_000);
    let started = std::time::Instant::now();
    let text = tokio::task::spawn_blocking(move || asr::transcribe_samples(&model, &samples, "en"))
        .await
        .unwrap()
        .expect("transcribe");
    println!("transcript ({:?}): {text:?}", started.elapsed());

    let lower = text.to_lowercase();
    assert!(
        lower.contains("test") || lower.contains("caption") || lower.contains("hello"),
        "unexpected transcript: {text:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads the OPUS-MT en->es ONNX model (~250 MB)"]
async fn local_translation_english_to_spanish() {
    let paths = test_paths();
    let mut translator = translate::load_translator(&paths, &NullSink, "es")
        .await
        .expect("load translator");
    let out =
        tokio::task::spawn_blocking(move || translator.translate("Hello, how are you today?"))
            .await
            .unwrap()
            .expect("translate");
    println!("translation: {out:?}");

    let lower = out.to_lowercase();
    assert!(
        ["hola", "cómo", "como", "estás", "estas", "hoy"]
            .iter()
            .any(|w| lower.contains(w)),
        "does not look Spanish: {out:?}"
    );
}

/// Full live pipeline on Windows: plays a speech WAV through the default
/// output device while the engine captures it via WASAPI loopback, runs local
/// Whisper (tiny) and emits caption events. Audible for a few seconds.
#[cfg(windows)]
#[test]
#[ignore = "plays audio through the speakers; needs the Whisper tiny model (run the whisper test first)"]
fn live_pipeline_captions_system_audio_loopback() {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use unicaptions_core::events::FnSink;
    use unicaptions_core::settings::{AudioSource, Provider, WhisperSize};
    use unicaptions_core::{CaptioningParams, Engine, UiEvent};

    let wav = std::env::var("UNICAPTIONS_TEST_WAV").expect("set UNICAPTIONS_TEST_WAV");
    let paths = test_paths();
    assert!(
        paths.whisper_model_file("tiny").exists(),
        "run the whisper e2e test first to download the model"
    );

    let events = Arc::new(Mutex::new(Vec::<UiEvent>::new()));
    let log = events.clone();
    let engine = Engine::new(
        paths,
        Arc::new(FnSink(move |e| log.lock().unwrap().push(e))),
    )
    .unwrap();

    engine.start_captioning(CaptioningParams {
        language: "en".into(),
        target_language: None,
        audio_source: AudioSource::System,
        asr_provider: Provider::Local,
        asr_model: WhisperSize::Tiny,
        translate_provider: Provider::Local,
    });

    // Wait for the engine to report it is live.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let ev = events.lock().unwrap().clone();
        if ev.contains(&UiEvent::CaptioningStarted) {
            break;
        }
        if let Some(UiEvent::CaptioningFailed(e)) = ev
            .iter()
            .find(|e| matches!(e, UiEvent::CaptioningFailed(_)))
        {
            panic!("captioning failed to start: {e}");
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for CaptioningStarted"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(engine.is_captioning_active());

    // Play the speech (blocking) through the speakers; loopback hears it.
    let status = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            &format!("(New-Object System.Media.SoundPlayer '{wav}').PlaySync()"),
        ])
        .status()
        .expect("play wav");
    assert!(status.success());
    std::thread::sleep(Duration::from_secs(5)); // let the last window get transcribed

    engine.stop_captioning();
    assert!(!engine.is_captioning_active());

    let ev = events.lock().unwrap().clone();
    let captions: Vec<String> = ev
        .iter()
        .filter_map(|e| match e {
            UiEvent::Caption(c) if !c.text.is_empty() => Some(c.text.clone()),
            _ => None,
        })
        .collect();
    println!("captions: {captions:#?}");
    let levels = ev
        .iter()
        .filter(|e| matches!(e, UiEvent::AudioLevel(l) if *l > 0.01))
        .count();
    println!("audio-level events above noise floor: {levels}");

    let joined = captions.join(" ").to_lowercase();
    assert!(
        joined.contains("test") || joined.contains("caption") || joined.contains("weather"),
        "no recognisable caption from loopback audio: {captions:?}"
    );
    assert!(ev.contains(&UiEvent::CaptioningStopped));
}
