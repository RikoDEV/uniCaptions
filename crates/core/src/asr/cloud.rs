use reqwest::multipart;
use serde::Deserialize;

#[derive(Deserialize)]
struct TranscriptionResponse {
    text: String,
    /// Only present with `response_format=verbose_json`, as an English name ("polish").
    language: Option<String>,
}

/// What the API returned.
pub struct Transcript {
    pub text: String,
    /// Detected language as a two-letter code, when it is one the app knows.
    pub language: Option<&'static str>,
}

/// Whisper API language names -> the codes the app uses.
pub fn language_code(name: &str) -> Option<&'static str> {
    match name.trim().to_ascii_lowercase().as_str() {
        "english" => Some("en"),
        "polish" => Some("pl"),
        "german" => Some("de"),
        "spanish" => Some("es"),
        "french" => Some("fr"),
        "portuguese" => Some("pt"),
        "chinese" => Some("zh"),
        "japanese" => Some("ja"),
        _ => None,
    }
}

/// Transcribes a WAV audio chunk using OpenAI's Whisper API.
pub async fn transcribe(
    api_key: &str,
    wav_bytes: Vec<u8>,
    language: &str,
) -> Result<Transcript, String> {
    let part = multipart::Part::bytes(wav_bytes)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;
    let mut form = multipart::Form::new()
        .part("file", part)
        .text("model", "whisper-1")
        // verbose_json also reports the detected language, which translation needs.
        .text("response_format", "verbose_json");
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
    Ok(Transcript {
        language: parsed.language.as_deref().and_then(language_code),
        text: parsed.text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_whisper_language_names_to_codes() {
        assert_eq!(language_code("polish"), Some("pl"));
        assert_eq!(language_code("Polish"), Some("pl"));
        assert_eq!(language_code(" english "), Some("en"));
        assert_eq!(language_code("japanese"), Some("ja"));
        assert_eq!(language_code("klingon"), None);
    }

    #[test]
    fn parses_a_verbose_json_response() {
        let r: TranscriptionResponse = serde_json::from_str(
            r#"{"task":"transcribe","language":"polish","duration":3.0,"text":"Dzień dobry"}"#,
        )
        .unwrap();
        assert_eq!(r.text, "Dzień dobry");
        assert_eq!(r.language.as_deref().and_then(language_code), Some("pl"));
        // plain json (no language) still parses
        let r: TranscriptionResponse = serde_json::from_str(r#"{"text":"hi"}"#).unwrap();
        assert_eq!(r.language, None);
    }
}
