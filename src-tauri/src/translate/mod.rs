pub mod cloud;

use ort::session::Session;
use ort::value::Tensor;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager};
use tokenizers::Tokenizer;

/// Supported local translation targets: English -> target language, backed by
/// Helsinki-NLP's OPUS-MT models (ONNX exports via Xenova). Only language
/// pairs with a published ONNX export are listed here.
fn repo_for_target(lang: &str) -> Option<&'static str> {
    match lang {
        "es" => Some("Xenova/opus-mt-en-es"),
        "fr" => Some("Xenova/opus-mt-en-fr"),
        "de" => Some("Xenova/opus-mt-en-de"),
        "zh" => Some("Xenova/opus-mt-en-zh"),
        _ => None,
    }
}

pub fn is_supported(lang: &str) -> bool {
    repo_for_target(lang).is_some()
}

pub fn supported_targets() -> &'static [&'static str] {
    &["es", "fr", "de", "zh"]
}

pub struct TranslateModelInfo {
    pub lang: String,
    pub downloaded: bool,
    pub bytes_on_disk: u64,
}

pub fn list_translate_models(app: &AppHandle) -> Result<Vec<TranslateModelInfo>, String> {
    supported_targets()
        .iter()
        .map(|&lang| {
            let dir = model_dir(app, lang)?;
            let mut bytes_on_disk = 0u64;
            let mut downloaded = false;
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    if let Ok(meta) = entry.metadata() {
                        bytes_on_disk += meta.len();
                        downloaded = true;
                    }
                }
            }
            Ok(TranslateModelInfo {
                lang: lang.to_string(),
                downloaded,
                bytes_on_disk,
            })
        })
        .collect()
}

pub fn delete_translate_model(app: &AppHandle, lang: &str) -> Result<(), String> {
    let dir = model_dir(app, lang)?;
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    Ok(())
}

struct GenerationConfig {
    decoder_start_token_id: i64,
    eos_token_id: i64,
}

pub struct Translator {
    encoder: Session,
    decoder: Session,
    tokenizer: Tokenizer,
    config: GenerationConfig,
}

fn model_dir(app: &AppHandle, lang: &str) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("models").join("translate").join(lang))
}

/// Downloads `url` to `dest`, emitting `translate-model-download-progress`
/// as `{ lang, progress }` where `progress` is blended across all
/// `stage_count` files being downloaded for this language (file `stage_index`
/// of `stage_count`, 0-based), so the UI sees one smooth 0-1 bar rather than
/// four separate ones.
#[allow(clippy::too_many_arguments)]
async fn download_file(
    url: &str,
    dest: &Path,
    app: &AppHandle,
    lang: &str,
    stage_index: usize,
    stage_count: usize,
) -> Result<(), String> {
    let emit_progress = |file_fraction: f64| {
        let overall = (stage_index as f64 + file_fraction) / stage_count as f64;
        let _ = app.emit(
            "translate-model-download-progress",
            serde_json::json!({ "lang": lang, "progress": overall }),
        );
    };

    if dest.exists() {
        emit_progress(1.0);
        return Ok(());
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let response = reqwest::get(url).await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("download failed for {url}: HTTP {}", response.status()));
    }
    let total = response.content_length().unwrap_or(0);
    let mut downloaded: u64 = 0;
    let tmp_path = dest.with_extension("part");
    let mut file = fs::File::create(&tmp_path).map_err(|e| e.to_string())?;

    use futures_util::StreamExt;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;
        let file_fraction = if total > 0 {
            downloaded as f64 / total as f64
        } else {
            0.0
        };
        emit_progress(file_fraction);
    }
    drop(file);
    fs::rename(&tmp_path, dest).map_err(|e| e.to_string())?;
    Ok(())
}

struct ModelFiles {
    encoder_path: PathBuf,
    decoder_path: PathBuf,
    tokenizer_path: PathBuf,
    config_path: PathBuf,
}

/// Downloads the encoder/decoder/tokenizer/config for the given target
/// language if not already present, emitting `translate-model-download-progress`.
pub async fn ensure_model_downloaded(app: AppHandle, target_lang: String) -> Result<(), String> {
    download_model_files(&app, &target_lang).await?;
    Ok(())
}

async fn download_model_files(app: &AppHandle, target_lang: &str) -> Result<ModelFiles, String> {
    let repo = repo_for_target(target_lang)
        .ok_or_else(|| format!("no local translation model available for '{target_lang}'"))?;
    let dir = model_dir(app, target_lang)?;

    let base = format!("https://huggingface.co/{repo}/resolve/main");
    let encoder_path = dir.join("encoder_model_int8.onnx");
    let decoder_path = dir.join("decoder_model_int8.onnx");
    let tokenizer_path = dir.join("tokenizer.json");
    let config_path = dir.join("config.json");

    download_file(
        &format!("{base}/onnx/encoder_model_int8.onnx"),
        &encoder_path,
        app,
        target_lang,
        0,
        4,
    )
    .await?;
    download_file(
        &format!("{base}/onnx/decoder_model_int8.onnx"),
        &decoder_path,
        app,
        target_lang,
        1,
        4,
    )
    .await?;
    download_file(&format!("{base}/tokenizer.json"), &tokenizer_path, app, target_lang, 2, 4)
        .await?;
    download_file(&format!("{base}/config.json"), &config_path, app, target_lang, 3, 4).await?;
    let _ = app.emit(
        "translate-model-download-progress",
        serde_json::json!({ "lang": target_lang, "progress": 1.0 }),
    );

    Ok(ModelFiles {
        encoder_path,
        decoder_path,
        tokenizer_path,
        config_path,
    })
}

/// Downloads (if needed) and loads the encoder/decoder/tokenizer for the
/// given target language, ready to translate English text into it.
pub async fn load_translator(app: AppHandle, target_lang: String) -> Result<Translator, String> {
    let ModelFiles {
        encoder_path,
        decoder_path,
        tokenizer_path,
        config_path,
    } = download_model_files(&app, &target_lang).await?;

    let config_json: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&config_path).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let decoder_start_token_id = config_json
        .get("decoder_start_token_id")
        .and_then(|v| v.as_i64())
        .or_else(|| config_json.get("pad_token_id").and_then(|v| v.as_i64()))
        .unwrap_or(0);
    let eos_token_id = config_json
        .get("eos_token_id")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let encoder = Session::builder()
        .map_err(|e| e.to_string())?
        .commit_from_file(&encoder_path)
        .map_err(|e| e.to_string())?;
    let decoder = Session::builder()
        .map_err(|e| e.to_string())?
        .commit_from_file(&decoder_path)
        .map_err(|e| e.to_string())?;
    let tokenizer = load_tokenizer(&tokenizer_path)?;

    Ok(Translator {
        encoder,
        decoder,
        tokenizer,
        config: GenerationConfig {
            decoder_start_token_id,
            eos_token_id,
        },
    })
}

/// Some Xenova ONNX exports ship a `tokenizer.json` whose `Precompiled`
/// normalizer has `precompiled_charsmap: null`, which the `tokenizers` crate
/// fails to deserialize (it expects real charsmap bytes). transformers.js
/// treats a null charsmap as a no-op, so we strip the normalizer here to
/// match that behavior rather than failing to load the tokenizer at all.
fn load_tokenizer(path: &Path) -> Result<Tokenizer, String> {
    use std::str::FromStr;

    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut json: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;

    let is_broken_precompiled = json
        .get("normalizer")
        .and_then(|n| n.get("type"))
        .and_then(|t| t.as_str())
        == Some("Precompiled")
        && json
            .get("normalizer")
            .and_then(|n| n.get("precompiled_charsmap"))
            .map(|c| c.is_null())
            .unwrap_or(false);

    if is_broken_precompiled {
        json["normalizer"] = serde_json::Value::Null;
    }

    Tokenizer::from_str(&json.to_string()).map_err(|e| e.to_string())
}

const MAX_OUTPUT_TOKENS: usize = 128;

impl Translator {
    pub fn translate(&mut self, text: &str) -> Result<String, String> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| e.to_string())?;
        let input_ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
        let seq_len = input_ids.len();
        if seq_len == 0 {
            return Ok(String::new());
        }
        let attention_mask: Vec<i64> = vec![1; seq_len];

        let input_ids_tensor = Tensor::from_array(([1usize, seq_len], input_ids))
            .map_err(|e| e.to_string())?;
        let attention_mask_tensor = Tensor::from_array(([1usize, seq_len], attention_mask.clone()))
            .map_err(|e| e.to_string())?;

        let encoder_outputs = self
            .encoder
            .run(ort::inputs![
                "input_ids" => input_ids_tensor,
                "attention_mask" => attention_mask_tensor,
            ])
            .map_err(|e| e.to_string())?;

        let (hidden_shape, hidden_data) = encoder_outputs["last_hidden_state"]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        let hidden_dims: Vec<usize> = hidden_shape.iter().map(|&d| d as usize).collect();
        let hidden_vec: Vec<f32> = hidden_data.to_vec();

        let mut decoder_ids: Vec<i64> = vec![self.config.decoder_start_token_id];

        for _ in 0..MAX_OUTPUT_TOKENS {
            let dec_len = decoder_ids.len();
            let decoder_input_tensor =
                Tensor::from_array(([1usize, dec_len], decoder_ids.clone()))
                    .map_err(|e| e.to_string())?;
            let encoder_hidden_tensor =
                Tensor::from_array((hidden_dims.clone(), hidden_vec.clone()))
                    .map_err(|e| e.to_string())?;
            let encoder_attn_tensor =
                Tensor::from_array(([1usize, seq_len], attention_mask.clone()))
                    .map_err(|e| e.to_string())?;

            let decoder_outputs = self
                .decoder
                .run(ort::inputs![
                    "input_ids" => decoder_input_tensor,
                    "encoder_hidden_states" => encoder_hidden_tensor,
                    "encoder_attention_mask" => encoder_attn_tensor,
                ])
                .map_err(|e| e.to_string())?;

            let (logits_shape, logits_data) = decoder_outputs["logits"]
                .try_extract_tensor::<f32>()
                .map_err(|e| e.to_string())?;
            let vocab_size = *logits_shape.last().ok_or("empty logits shape")? as usize;
            let last_step_start = (dec_len - 1) * vocab_size;
            let last_step_logits = &logits_data[last_step_start..last_step_start + vocab_size];

            let (next_token, _) = last_step_logits
                .iter()
                .enumerate()
                .fold((0usize, f32::MIN), |acc, (i, &v)| {
                    if v > acc.1 {
                        (i, v)
                    } else {
                        acc
                    }
                });

            if next_token as i64 == self.config.eos_token_id {
                break;
            }
            decoder_ids.push(next_token as i64);
        }

        let output_ids: Vec<u32> = decoder_ids[1..].iter().map(|&id| id as u32).collect();
        self.tokenizer
            .decode(&output_ids, true)
            .map_err(|e| e.to_string())
    }
}
