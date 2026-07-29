#[cfg(windows)]
mod loopback_windows;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

pub struct CaptureHandle {
    stop_flag: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl CaptureHandle {
    fn from_thread(stop_flag: Arc<AtomicBool>, thread: thread::JoinHandle<()>) -> Self {
        Self {
            stop_flag,
            thread: Some(thread),
        }
    }

    pub fn stop(mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Starts capturing either the default microphone or (Windows only) the
/// default output device's system audio (loopback), based on `source`
/// ("microphone" | "system").
pub fn start_capture_stream<F>(source: &str, on_frame: F) -> Result<CaptureHandle, String>
where
    F: FnMut(&[f32], u32, usize) + Send + 'static,
{
    match source {
        "system" => {
            #[cfg(windows)]
            {
                loopback_windows::start_loopback_stream(on_frame)
            }
            #[cfg(not(windows))]
            {
                let _ = on_frame;
                Err("system audio capture is only implemented on Windows so far".to_string())
            }
        }
        _ => start_microphone_stream(on_frame),
    }
}

/// Spawns a dedicated thread that owns the cpal input stream for its lifetime,
/// calling `on_frame(samples, sample_rate, channels)` for every buffer of raw
/// audio captured from the default microphone. The thread is kept alive by a
/// park loop until `stop()` is called, since the cpal `Stream` must stay in
/// scope for the callback to keep firing.
pub fn start_microphone_stream<F>(on_frame: F) -> Result<CaptureHandle, String>
where
    F: FnMut(&[f32], u32, usize) + Send + 'static,
{
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_thread = stop_flag.clone();

    let thread = thread::Builder::new()
        .name("unicaptions-audio-capture".into())
        .spawn(move || {
            if let Err(e) = run_capture(on_frame, stop_flag_thread) {
                eprintln!("audio capture error: {e}");
            }
        })
        .map_err(|e| e.to_string())?;

    Ok(CaptureHandle {
        stop_flag,
        thread: Some(thread),
    })
}

fn run_capture<F>(mut on_frame: F, stop_flag: Arc<AtomicBool>) -> Result<(), String>
where
    F: FnMut(&[f32], u32, usize) + Send + 'static,
{
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "no input device found".to_string())?;
    let config = device
        .default_input_config()
        .map_err(|e| e.to_string())?;
    let channels = config.channels() as usize;
    let sample_rate = config.sample_rate().0;
    let sample_format = config.sample_format();
    let stream_config: cpal::StreamConfig = config.into();

    let err_fn = |err| eprintln!("audio stream error: {err}");

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &stream_config,
            move |data: &[f32], _| on_frame(data, sample_rate, channels),
            err_fn,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &stream_config,
            move |data: &[i16], _| {
                let floats: Vec<f32> = data.iter().map(|s| *s as f32 / i16::MAX as f32).collect();
                on_frame(&floats, sample_rate, channels);
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream(
            &stream_config,
            move |data: &[u16], _| {
                let floats: Vec<f32> = data
                    .iter()
                    .map(|s| (*s as f32 - 32768.0) / 32768.0)
                    .collect();
                on_frame(&floats, sample_rate, channels);
            },
            err_fn,
            None,
        ),
        other => return Err(format!("unsupported sample format: {other:?}")),
    }
    .map_err(|e| e.to_string())?;

    stream.play().map_err(|e| e.to_string())?;

    while !stop_flag.load(Ordering::SeqCst) {
        thread::sleep(std::time::Duration::from_millis(100));
    }

    Ok(())
}

/// Downmixes interleaved multi-channel samples to mono.
pub fn downmix(data: &[f32], channels: usize) -> Vec<f32> {
    let channels = channels.max(1);
    data.chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect()
}

/// Root-mean-square level of a mono sample buffer, in [0, ~1].
pub fn rms(mono: &[f32]) -> f32 {
    if mono.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = mono.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    ((sum_sq / mono.len() as f64).sqrt()) as f32
}

/// Encodes mono f32 samples as a 16-bit PCM WAV file, for uploading to cloud
/// ASR APIs that expect a standard audio file rather than raw PCM.
pub fn encode_wav_pcm16(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let num_samples = samples.len();
    let byte_rate = sample_rate * 2;
    let data_size = (num_samples * 2) as u32;
    let riff_size = 36 + data_size;

    let mut out = Vec::with_capacity(44 + num_samples * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_size.to_le_bytes());
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let sample_i16 = (clamped * i16::MAX as f32) as i16;
        out.extend_from_slice(&sample_i16.to_le_bytes());
    }
    out
}

/// Simple linear-interpolation resampler. Not as high-quality as a sinc
/// resampler, but cheap and sufficient for feeding speech into Whisper.
pub fn resample_linear(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || input.is_empty() {
        return input.to_vec();
    }
    let ratio = to_rate as f64 / from_rate as f64;
    let out_len = ((input.len() as f64) * ratio).round() as usize;
    let mut output = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = i as f64 / ratio;
        let idx = src_pos.floor() as usize;
        let frac = (src_pos - idx as f64) as f32;
        let s0 = *input.get(idx).unwrap_or(&0.0);
        let s1 = *input.get(idx + 1).unwrap_or(&s0);
        output.push(s0 + (s1 - s0) * frac);
    }
    output
}
