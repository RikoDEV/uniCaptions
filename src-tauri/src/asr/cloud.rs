use reqwest::multipart;
use serde::Deserialize;

#[derive(Deserialize)]
struct TranscriptionResponse {
    text: String,
}

/// Transcribes a WAV audio chunk using OpenAI's Whisper API.
pub async fn transcribe(api_key: &str, wav_bytes: Vec<u8>, language: &str) -> Result<String, String> {
    let part = multipart::Part::bytes(wav_bytes)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;
    let mut form = multipart::Form::new().part("file", part).text("model", "whisper-1");
    if language != "auto" {
        form = form.text("language", language.to_string());
    }

    let client = reqwest::Client::new();
    let response = client
        .post("https://api.openai.com/v1/audio/transcriptions")
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("OpenAI transcription request failed: {body}"));
    }

    let parsed: TranscriptionResponse = response.json().await.map_err(|e| e.to_string())?;
    Ok(parsed.text)
}
