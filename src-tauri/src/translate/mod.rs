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
    num_decoder_layers: usize,
    num_heads: usize,
    head_dim: usize,
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
    // The "merged" decoder graph combines the no-cache (prefill) and
    // with-cache (decode) branches behind a `use_cache_branch` input, so a
    // single session can do KV-cached autoregressive decoding instead of
    // recomputing self- and cross-attention over the whole growing sequence
    // on every generated token.
    let decoder_path = dir.join("decoder_model_merged_int8.onnx");
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
        &format!("{base}/onnx/decoder_model_merged_int8.onnx"),
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
    // MarianMT (OPUS-MT) config fields needed to shape the past_key_values
    // cache tensors: [batch, num_heads, seq_len, head_dim] per decoder layer.
    let num_decoder_layers = config_json
        .get("decoder_layers")
        .and_then(|v| v.as_u64())
        .ok_or("config.json missing decoder_layers")? as usize;
    let num_heads = config_json
        .get("decoder_attention_heads")
        .and_then(|v| v.as_u64())
        .ok_or("config.json missing decoder_attention_heads")? as usize;
    let d_model = config_json
        .get("d_model")
        .and_then(|v| v.as_u64())
        .ok_or("config.json missing d_model")? as usize;
    if num_heads == 0 || d_model % num_heads != 0 {
        return Err(format!(
            "invalid decoder dims: d_model={d_model}, num_heads={num_heads}"
        ));
    }
    let head_dim = d_model / num_heads;

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
            num_decoder_layers,
            num_heads,
            head_dim,
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

/// Per-layer KV cache for the merged decoder. Self-attention (`decoder`)
/// keys/values grow by one position each step; cross-attention (`encoder`)
/// keys/values are the fixed projection of the encoder output, computed
/// once on the prefill step and fed back unchanged on every decode step.
#[derive(Default, Clone)]
struct LayerCache {
    decoder_key: Vec<f32>,
    decoder_value: Vec<f32>,
    encoder_key: Vec<f32>,
    encoder_value: Vec<f32>,
}

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
        drop(encoder_outputs);

        let num_layers = self.config.num_decoder_layers;
        let num_heads = self.config.num_heads;
        let head_dim = self.config.head_dim;

        let mut cache: Vec<LayerCache> = vec![LayerCache::default(); num_layers];
        let mut decoder_past_len: usize = 0;
        let mut encoder_past_len: usize = 0;

        let mut next_input_token = self.config.decoder_start_token_id;
        let mut output_ids: Vec<u32> = Vec::new();

        for step in 0..MAX_OUTPUT_TOKENS {
            let use_cache_branch = step > 0;

            let mut inputs: Vec<(std::borrow::Cow<str>, ort::session::SessionInputValue)> = Vec::new();
            let input_ids_tensor = Tensor::from_array(([1usize, 1usize], vec![next_input_token]))
                .map_err(|e| e.to_string())?;
            let encoder_hidden_tensor = Tensor::from_array((hidden_dims.clone(), hidden_vec.clone()))
                .map_err(|e| e.to_string())?;
            let encoder_attn_tensor = Tensor::from_array(([1usize, seq_len], attention_mask.clone()))
                .map_err(|e| e.to_string())?;
            let use_cache_branch_tensor = Tensor::from_array(([1usize], vec![use_cache_branch]))
                .map_err(|e| e.to_string())?;

            inputs.push(("input_ids".into(), input_ids_tensor.into()));
            inputs.push(("encoder_attention_mask".into(), encoder_attn_tensor.into()));
            inputs.push(("encoder_hidden_states".into(), encoder_hidden_tensor.into()));
            inputs.push(("use_cache_branch".into(), use_cache_branch_tensor.into()));

            for (i, layer) in cache.iter().enumerate() {
                let dec_key = Tensor::from_array((
                    [1usize, num_heads, decoder_past_len, head_dim],
                    layer.decoder_key.clone(),
                ))
                .map_err(|e| e.to_string())?;
                let dec_value = Tensor::from_array((
                    [1usize, num_heads, decoder_past_len, head_dim],
                    layer.decoder_value.clone(),
                ))
                .map_err(|e| e.to_string())?;
                let enc_key = Tensor::from_array((
                    [1usize, num_heads, encoder_past_len, head_dim],
                    layer.encoder_key.clone(),
                ))
                .map_err(|e| e.to_string())?;
                let enc_value = Tensor::from_array((
                    [1usize, num_heads, encoder_past_len, head_dim],
                    layer.encoder_value.clone(),
                ))
                .map_err(|e| e.to_string())?;

                inputs.push((format!("past_key_values.{i}.decoder.key").into(), dec_key.into()));
                inputs.push((format!("past_key_values.{i}.decoder.value").into(), dec_value.into()));
                inputs.push((format!("past_key_values.{i}.encoder.key").into(), enc_key.into()));
                inputs.push((format!("past_key_values.{i}.encoder.value").into(), enc_value.into()));
            }

            let decoder_outputs = self.decoder.run(inputs).map_err(|e| e.to_string())?;

            let (logits_shape, logits_data) = decoder_outputs["logits"]
                .try_extract_tensor::<f32>()
                .map_err(|e| e.to_string())?;
            let vocab_size = *logits_shape.last().ok_or("empty logits shape")? as usize;
            let last_step_logits = &logits_data[0..vocab_size];

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

            // Pull this step's present.* outputs into the cache before
            // `decoder_outputs` (and the borrows into it) go out of scope.
            for (i, layer) in cache.iter_mut().enumerate() {
                let (_, dec_key) = decoder_outputs[format!("present.{i}.decoder.key").as_str()]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| e.to_string())?;
                let (_, dec_value) = decoder_outputs[format!("present.{i}.decoder.value").as_str()]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| e.to_string())?;
                layer.decoder_key = dec_key.to_vec();
                layer.decoder_value = dec_value.to_vec();

                if !use_cache_branch {
                    // Cross-attention KV only needs computing once, on the
                    // prefill step; every decode step after that reuses it.
                    let (_, enc_key) = decoder_outputs[format!("present.{i}.encoder.key").as_str()]
                        .try_extract_tensor::<f32>()
                        .map_err(|e| e.to_string())?;
                    let (_, enc_value) = decoder_outputs[format!("present.{i}.encoder.value").as_str()]
                        .try_extract_tensor::<f32>()
                        .map_err(|e| e.to_string())?;
                    layer.encoder_key = enc_key.to_vec();
                    layer.encoder_value = enc_value.to_vec();
                }
            }
            decoder_past_len += 1;
            encoder_past_len = seq_len;

            if next_token as i64 == self.config.eos_token_id {
                break;
            }
            output_ids.push(next_token as u32);
            next_input_token = next_token as i64;
        }

        self.tokenizer
            .decode(&output_ids, true)
            .map_err(|e| e.to_string())
    }
}
