pub mod cloud;

use crate::events::{EventSink, ModelKind, UiEvent};
use crate::paths::Paths;
use ort::session::Session;
use ort::value::Tensor;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tokenizers::Tokenizer;

/// A translation direction, e.g. Polish -> English.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Pair {
    pub source: String,
    pub target: String,
}

impl Pair {
    pub fn new(source: &str, target: &str) -> Self {
        Self {
            source: source.to_string(),
            target: target.to_string(),
        }
    }

    /// Stable id used in model keys and on disk. English -> X keeps the bare
    /// target code (`es`) so models downloaded by earlier versions are found;
    /// every other pair is `source-target` (`pl-en`).
    pub fn id(&self) -> String {
        if self.source == "en" {
            self.target.clone()
        } else {
            format!("{}-{}", self.source, self.target)
        }
    }

    /// Inverse of [`Pair::id`]; `None` for anything that is not a supported pair.
    pub fn parse_id(id: &str) -> Option<Pair> {
        let pair = match id.split_once('-') {
            Some((s, t)) => Pair::new(s, t),
            None => Pair::new("en", id),
        };
        repo_for_pair(&pair).map(|_| pair)
    }
}

/// Helsinki-NLP OPUS-MT models with a published ONNX export (via Xenova):
/// English -> es/fr/de/zh, and pl/de/es/fr/zh/ja -> English. Everything else is
/// reached by pivoting through English, or needs the cloud provider.
const DIRECT_PAIRS: [(&str, &str, &str); 10] = [
    ("en", "es", "Xenova/opus-mt-en-es"),
    ("en", "fr", "Xenova/opus-mt-en-fr"),
    ("en", "de", "Xenova/opus-mt-en-de"),
    ("en", "zh", "Xenova/opus-mt-en-zh"),
    ("pl", "en", "Xenova/opus-mt-pl-en"),
    ("de", "en", "Xenova/opus-mt-de-en"),
    ("es", "en", "Xenova/opus-mt-es-en"),
    ("fr", "en", "Xenova/opus-mt-fr-en"),
    ("zh", "en", "Xenova/opus-mt-zh-en"),
    ("ja", "en", "Xenova/opus-mt-ja-en"),
];

fn repo_for_pair(pair: &Pair) -> Option<&'static str> {
    DIRECT_PAIRS
        .iter()
        .find(|(s, t, _)| *s == pair.source && *t == pair.target)
        .map(|(_, _, repo)| *repo)
}

/// Every pair with a local model, in display order.
pub fn supported_pairs() -> Vec<Pair> {
    DIRECT_PAIRS
        .iter()
        .map(|(s, t, _)| Pair::new(s, t))
        .collect()
}

/// The models needed to translate `source` -> `target` locally, in order.
/// `Some(vec![])` means nothing to do (same language); `None` means the
/// combination is not available locally.
pub fn plan(source: &str, target: &str) -> Option<Vec<Pair>> {
    if source == target {
        return Some(Vec::new());
    }
    let direct = Pair::new(source, target);
    if repo_for_pair(&direct).is_some() {
        return Some(vec![direct]);
    }
    // Pivot through English.
    let (first, second) = (Pair::new(source, "en"), Pair::new("en", target));
    if source != "en"
        && target != "en"
        && repo_for_pair(&first).is_some()
        && repo_for_pair(&second).is_some()
    {
        return Some(vec![first, second]);
    }
    None
}

/// Can *any* spoken language be translated into `target` locally?
pub fn local_supports_target(target: &str) -> bool {
    DIRECT_PAIRS.iter().any(|(_, t, _)| *t == target)
}

pub struct TranslateModelInfo {
    /// [`Pair::id`].
    pub id: String,
    pub pair: Pair,
    pub downloaded: bool,
    pub bytes_on_disk: u64,
}

const MODEL_FILES: [&str; 4] = [
    "encoder_model_int8.onnx",
    "decoder_model_merged_int8.onnx",
    "tokenizer.json",
    "config.json",
];

/// A model counts as downloaded only if every required file is present
/// (older versions stored a different decoder file and half-finished downloads
/// leave `.part` files, neither of which is usable).
fn is_complete(dir: &Path) -> bool {
    MODEL_FILES.iter().all(|f| dir.join(f).is_file())
}

pub fn list_translate_models(paths: &Paths) -> Result<Vec<TranslateModelInfo>, String> {
    Ok(supported_pairs()
        .into_iter()
        .map(|pair| {
            let dir = pair_dir(paths, &pair);
            let bytes_on_disk = fs::read_dir(&dir)
                .map(|entries| {
                    entries
                        .flatten()
                        .filter_map(|e| e.metadata().ok())
                        .map(|m| m.len())
                        .sum()
                })
                .unwrap_or(0);
            TranslateModelInfo {
                id: pair.id(),
                downloaded: is_complete(&dir),
                pair,
                bytes_on_disk,
            }
        })
        .collect())
}

pub fn delete_translate_model(paths: &Paths, id: &str) -> Result<(), String> {
    let pair = Pair::parse_id(id).ok_or_else(|| format!("unknown translation model '{id}'"))?;
    let dir = pair_dir(paths, &pair);
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

fn pair_dir(paths: &Paths, pair: &Pair) -> PathBuf {
    paths.translate_model_dir(&pair.id())
}

/// Downloads `url` to `dest`, emitting `ModelProgress` for the language
/// where `progress` is blended across all
/// `stage_count` files being downloaded for this language (file `stage_index`
/// of `stage_count`, 0-based), so the UI sees one smooth 0-1 bar rather than
/// four separate ones.
#[allow(clippy::too_many_arguments)]
async fn download_file(
    url: &str,
    dest: &Path,
    sink: &dyn EventSink,
    lang: &str,
    stage_index: usize,
    stage_count: usize,
) -> Result<(), String> {
    let emit_progress = |file_fraction: f64| {
        let overall = (stage_index as f64 + file_fraction) / stage_count as f64;
        sink.emit(UiEvent::ModelProgress {
            kind: ModelKind::Translate,
            id: lang.to_string(),
            progress: overall,
        });
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
        return Err(format!(
            "download failed for {url}: HTTP {}",
            response.status()
        ));
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

/// Downloads the encoder/decoder/tokenizer/config for the given model id
/// (see [`Pair::id`]) if not already present, emitting `ModelProgress` events.
pub async fn ensure_model_downloaded(
    paths: &Paths,
    sink: &dyn EventSink,
    id: &str,
) -> Result<(), String> {
    let pair = Pair::parse_id(id).ok_or_else(|| format!("unknown translation model '{id}'"))?;
    download_model_files(paths, sink, &pair).await?;
    Ok(())
}

async fn download_model_files(
    paths: &Paths,
    sink: &dyn EventSink,
    pair: &Pair,
) -> Result<ModelFiles, String> {
    let repo = repo_for_pair(pair).ok_or_else(|| {
        format!(
            "no local translation model for {} -> {}",
            pair.source, pair.target
        )
    })?;
    let id = pair.id();
    let target_lang = id.as_str();
    let dir = pair_dir(paths, pair);

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
        sink,
        target_lang,
        0,
        4,
    )
    .await?;
    download_file(
        &format!("{base}/onnx/decoder_model_merged_int8.onnx"),
        &decoder_path,
        sink,
        target_lang,
        1,
        4,
    )
    .await?;
    download_file(
        &format!("{base}/tokenizer.json"),
        &tokenizer_path,
        sink,
        target_lang,
        2,
        4,
    )
    .await?;
    download_file(
        &format!("{base}/config.json"),
        &config_path,
        sink,
        target_lang,
        3,
        4,
    )
    .await?;
    sink.emit(UiEvent::ModelProgress {
        kind: ModelKind::Translate,
        id: target_lang.to_string(),
        progress: 1.0,
    });

    Ok(ModelFiles {
        encoder_path,
        decoder_path,
        tokenizer_path,
        config_path,
    })
}

/// Downloads (if needed) and loads the encoder/decoder/tokenizer for the
/// given target language, ready to translate English text into it.
pub async fn load_translator(
    paths: &Paths,
    sink: &dyn EventSink,
    pair: &Pair,
) -> Result<Translator, String> {
    let ModelFiles {
        encoder_path,
        decoder_path,
        tokenizer_path,
        config_path,
    } = download_model_files(paths, sink, pair).await?;

    let config_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&config_path).map_err(|e| e.to_string())?)
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
    if num_heads == 0 || !d_model.is_multiple_of(num_heads) {
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

        let input_ids_tensor =
            Tensor::from_array(([1usize, seq_len], input_ids)).map_err(|e| e.to_string())?;
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

        #[allow(clippy::explicit_counter_loop)]
        for step in 0..MAX_OUTPUT_TOKENS {
            let use_cache_branch = step > 0;

            let mut inputs: Vec<(std::borrow::Cow<str>, ort::session::SessionInputValue)> =
                Vec::new();
            let input_ids_tensor = Tensor::from_array(([1usize, 1usize], vec![next_input_token]))
                .map_err(|e| e.to_string())?;
            let encoder_hidden_tensor =
                Tensor::from_array((hidden_dims.clone(), hidden_vec.clone()))
                    .map_err(|e| e.to_string())?;
            let encoder_attn_tensor =
                Tensor::from_array(([1usize, seq_len], attention_mask.clone()))
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

                inputs.push((
                    format!("past_key_values.{i}.decoder.key").into(),
                    dec_key.into(),
                ));
                inputs.push((
                    format!("past_key_values.{i}.decoder.value").into(),
                    dec_value.into(),
                ));
                inputs.push((
                    format!("past_key_values.{i}.encoder.key").into(),
                    enc_key.into(),
                ));
                inputs.push((
                    format!("past_key_values.{i}.encoder.value").into(),
                    enc_value.into(),
                ));
            }

            let decoder_outputs = self.decoder.run(inputs).map_err(|e| e.to_string())?;

            let (logits_shape, logits_data) = decoder_outputs["logits"]
                .try_extract_tensor::<f32>()
                .map_err(|e| e.to_string())?;
            let vocab_size = *logits_shape.last().ok_or("empty logits shape")? as usize;
            let last_step_logits = &logits_data[0..vocab_size];

            let (next_token, _) =
                last_step_logits
                    .iter()
                    .enumerate()
                    .fold(
                        (0usize, f32::MIN),
                        |acc, (i, &v)| {
                            if v > acc.1 {
                                (i, v)
                            } else {
                                acc
                            }
                        },
                    );

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
                    let (_, enc_value) = decoder_outputs
                        [format!("present.{i}.encoder.value").as_str()]
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

/// Translates captions locally for whatever language pair is needed: loads the
/// model(s) on first use (downloading them if necessary), keeps a few in
/// memory, and reports combinations that are not available instead of silently
/// doing nothing.
pub struct Router {
    paths: Paths,
    sink: crate::events::SharedSink,
    rt: tokio::runtime::Handle,
    /// Serialises downloads with the Models tab and captioning start-up.
    download_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    target: String,
    /// The user's spoken-language setting; `auto` means "use what was detected".
    configured_source: String,
    loaded: Vec<(Pair, Translator)>,
    warned: std::collections::HashSet<String>,
}

/// Models kept in memory at once (a pivot translation needs two).
const MAX_LOADED: usize = 3;

impl Router {
    pub fn new(
        paths: Paths,
        sink: crate::events::SharedSink,
        rt: tokio::runtime::Handle,
        download_lock: std::sync::Arc<tokio::sync::Mutex<()>>,
        configured_source: &str,
        target: &str,
    ) -> Self {
        Self {
            paths,
            sink,
            rt,
            download_lock,
            target: target.to_string(),
            configured_source: configured_source.to_string(),
            loaded: Vec::new(),
            warned: Default::default(),
        }
    }

    /// The spoken language to translate from: the setting, or the detected one.
    pub fn resolve_source<'a>(configured: &'a str, detected: Option<&'a str>) -> Option<&'a str> {
        if configured != "auto" {
            Some(configured)
        } else {
            detected
        }
    }

    fn warn_once(&mut self, key: String, event: UiEvent) {
        if self.warned.insert(key) {
            self.sink.emit(event);
        }
    }

    fn translator(&mut self, pair: &Pair) -> Result<&mut Translator, String> {
        if let Some(i) = self.loaded.iter().position(|(p, _)| p == pair) {
            let entry = self.loaded.remove(i);
            self.loaded.push(entry); // most recently used last
        } else {
            let (paths, sink, lock) = (&self.paths, &self.sink, &self.download_lock);
            let t = self.rt.block_on(async {
                let _guard = lock.lock().await;
                load_translator(paths, &**sink, pair).await
            })?;
            self.loaded.push((pair.clone(), t));
            if self.loaded.len() > MAX_LOADED {
                self.loaded.remove(0);
            }
        }
        Ok(&mut self.loaded.last_mut().expect("just pushed").1)
    }

    /// Translates `text`. `detected` is the language the recogniser detected,
    /// used when the spoken-language setting is `auto`. Returns `None` when
    /// there is nothing to translate or it is not possible (the user is told).
    pub fn translate(&mut self, text: &str, detected: Option<&str>) -> Option<String> {
        // Still waiting for language detection: nothing to do yet.
        let source = Self::resolve_source(&self.configured_source, detected)?.to_string();
        let target = self.target.clone();
        let steps = match plan(&source, &target) {
            Some(steps) => steps,
            None => {
                self.warn_once(
                    format!("{source}>{target}"),
                    UiEvent::TranslationUnavailable { source, target },
                );
                return None;
            }
        };
        if steps.is_empty() {
            return None; // already in the target language
        }
        let mut current = text.to_string();
        for pair in steps {
            let key = format!("fail:{}", pair.id());
            let result = self.translator(&pair).and_then(|t| t.translate(&current));
            match result {
                Ok(t) => current = t,
                Err(e) => {
                    eprintln!("translation error ({}): {e}", pair.id());
                    self.warn_once(key, UiEvent::TranslationFailed(e));
                    return None;
                }
            }
        }
        Some(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(plan: Option<Vec<Pair>>) -> Option<Vec<String>> {
        plan.map(|p| p.iter().map(Pair::id).collect())
    }

    #[test]
    fn direct_pairs_need_one_model() {
        assert_eq!(ids(plan("pl", "en")), Some(vec!["pl-en".into()]));
        assert_eq!(ids(plan("en", "es")), Some(vec!["es".into()]));
        assert_eq!(ids(plan("ja", "en")), Some(vec!["ja-en".into()]));
    }

    #[test]
    fn other_pairs_pivot_through_english() {
        assert_eq!(
            ids(plan("pl", "de")),
            Some(vec!["pl-en".into(), "de".into()])
        );
        assert_eq!(
            ids(plan("fr", "zh")),
            Some(vec!["fr-en".into(), "zh".into()])
        );
    }

    #[test]
    fn same_language_needs_nothing() {
        assert_eq!(plan("pl", "pl"), Some(vec![]));
        assert_eq!(plan("en", "en"), Some(vec![]));
    }

    #[test]
    fn unsupported_combinations_are_reported_as_none() {
        assert_eq!(plan("en", "pl"), None); // no en->pl model
        assert_eq!(plan("pt", "en"), None); // no pt->en model
        assert_eq!(plan("en", "ja"), None);
        assert_eq!(plan("pl", "pt"), None); // pivot target missing
        assert_eq!(plan("xx", "en"), None);
    }

    #[test]
    fn which_targets_are_reachable_locally() {
        for t in ["en", "es", "fr", "de", "zh"] {
            assert!(local_supports_target(t), "{t}");
        }
        for t in ["pl", "pt", "ja", "auto"] {
            assert!(!local_supports_target(t), "{t}");
        }
    }

    #[test]
    fn ids_round_trip_and_keep_the_legacy_english_source_form() {
        for pair in supported_pairs() {
            assert_eq!(Pair::parse_id(&pair.id()), Some(pair.clone()), "{pair:?}");
        }
        assert_eq!(Pair::new("en", "es").id(), "es");
        assert_eq!(Pair::new("pl", "en").id(), "pl-en");
        assert_eq!(Pair::parse_id("es"), Some(Pair::new("en", "es")));
        assert_eq!(Pair::parse_id("pl-en"), Some(Pair::new("pl", "en")));
    }

    #[test]
    fn bad_ids_are_rejected() {
        for bad in [
            "", "xx", "../x", "pl-pl", "en-pl", "pl-en-x", "pl-", "-en", "es-en-",
        ] {
            assert_eq!(Pair::parse_id(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn source_resolution_prefers_the_setting_over_detection() {
        assert_eq!(Router::resolve_source("pl", Some("en")), Some("pl"));
        assert_eq!(Router::resolve_source("auto", Some("de")), Some("de"));
        assert_eq!(Router::resolve_source("auto", None), None);
    }

    #[test]
    fn a_model_is_downloaded_only_when_every_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::in_dir(dir.path());
        let model = paths.translate_model_dir("pl-en");
        fs::create_dir_all(&model).unwrap();
        // older layout / interrupted download: not usable
        fs::write(model.join("encoder_model_int8.onnx"), b"x").unwrap();
        fs::write(model.join("decoder_model_int8.onnx"), b"x").unwrap();
        fs::write(model.join("decoder_model_merged_int8.onnx.part"), b"x").unwrap();
        let info = |paths: &Paths| {
            list_translate_models(paths)
                .unwrap()
                .into_iter()
                .find(|m| m.id == "pl-en")
                .unwrap()
        };
        assert!(!info(&paths).downloaded);
        assert!(info(&paths).bytes_on_disk > 0);
        for f in MODEL_FILES {
            fs::write(model.join(f), b"data").unwrap();
        }
        assert!(info(&paths).downloaded);
    }

    #[test]
    fn delete_removes_a_pair_directory_and_rejects_bad_ids() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::in_dir(dir.path());
        let model = paths.translate_model_dir("pl-en");
        fs::create_dir_all(&model).unwrap();
        fs::write(model.join("config.json"), b"{}").unwrap();
        delete_translate_model(&paths, "pl-en").unwrap();
        assert!(!model.exists());
        delete_translate_model(&paths, "pl-en").unwrap(); // already gone
        assert!(delete_translate_model(&paths, "../evil").is_err());
    }
}
