use crate::audio::CaptureHandle;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use wasapi::{Direction, SampleType, StreamMode, WaveFormat};

/// Captures whatever is currently playing through the default output device
/// (system audio loopback), calling `on_frame(samples, sample_rate, channels)`
/// with interleaved f32 samples as they arrive. Windows-only (WASAPI).
pub fn start_loopback_stream<F>(mut on_frame: F) -> Result<CaptureHandle, String>
where
    F: FnMut(&[f32], u32, usize) + Send + 'static,
{
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_thread = stop_flag.clone();

    let thread = thread::Builder::new()
        .name("unicaptions-loopback-capture".into())
        .spawn(move || {
            if let Err(e) = run_loopback(&mut on_frame, stop_flag_thread) {
                eprintln!("system audio capture error: {e}");
            }
        })
        .map_err(|e| e.to_string())?;

    Ok(CaptureHandle::from_thread(stop_flag, thread))
}

fn run_loopback<F>(on_frame: &mut F, stop_flag: Arc<AtomicBool>) -> Result<(), String>
where
    F: FnMut(&[f32], u32, usize),
{
    let _ = wasapi::initialize_mta();

    let device = wasapi::get_default_device(&Direction::Render).map_err(|e| e.to_string())?;
    let mut audio_client = device.get_iaudioclient().map_err(|e| e.to_string())?;

    let sample_rate = 48_000u32;
    let channels = 2usize;
    let desired_format = WaveFormat::new(32, 32, &SampleType::Float, sample_rate as usize, channels, None);

    let stream_mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: 0,
    };
    audio_client
        .initialize_client(&desired_format, &Direction::Capture, &stream_mode)
        .map_err(|e| e.to_string())?;

    let h_event = audio_client.set_get_eventhandle().map_err(|e| e.to_string())?;
    let capture_client = audio_client.get_audiocaptureclient().map_err(|e| e.to_string())?;
    let mut sample_queue: VecDeque<u8> = VecDeque::new();

    audio_client.start_stream().map_err(|e| e.to_string())?;

    let bytes_per_frame = (32 / 8) * channels;
    while !stop_flag.load(Ordering::SeqCst) {
        capture_client
            .read_from_device_to_deque(&mut sample_queue)
            .map_err(|e| e.to_string())?;

        while sample_queue.len() >= bytes_per_frame {
            let bytes: Vec<u8> = sample_queue.drain(..bytes_per_frame * 512.min(sample_queue.len() / bytes_per_frame).max(1)).collect();
            let samples: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            if !samples.is_empty() {
                on_frame(&samples, sample_rate, channels);
            }
        }

        let _ = h_event.wait_for_event(1000);
    }

    audio_client.stop_stream().map_err(|e| e.to_string())?;
    Ok(())
}
