//! Listing and deleting downloaded models (speech recognition + translation).

use crate::events::ModelKind;
use crate::paths::Paths;
use crate::{asr, translate};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelEntry {
    pub kind: ModelKind,
    /// Whisper size (`base`) or translation target language (`es`).
    pub id: String,
    pub downloaded: bool,
    pub bytes_on_disk: u64,
}

impl ModelEntry {
    /// Stable `kind:id` key, the same format the old IPC layer used.
    pub fn key(&self) -> String {
        format!("{}:{}", self.kind.as_str(), self.id)
    }
}

pub fn list_models(paths: &Paths) -> Result<Vec<ModelEntry>, String> {
    let mut entries: Vec<ModelEntry> = asr::list_whisper_models(paths)?
        .into_iter()
        .map(|m| ModelEntry {
            kind: ModelKind::Whisper,
            id: m.size,
            downloaded: m.downloaded,
            bytes_on_disk: m.bytes_on_disk,
        })
        .collect();
    entries.extend(
        translate::list_translate_models(paths)?
            .into_iter()
            .map(|m| ModelEntry {
                kind: ModelKind::Translate,
                id: m.lang,
                downloaded: m.downloaded,
                bytes_on_disk: m.bytes_on_disk,
            }),
    );
    Ok(entries)
}

pub fn parse_model_key(key: &str) -> Result<(ModelKind, &str), String> {
    let (kind, value) = key
        .split_once(':')
        .ok_or_else(|| format!("invalid model id '{key}'"))?;
    match kind {
        "whisper" => Ok((ModelKind::Whisper, value)),
        "translate" => Ok((ModelKind::Translate, value)),
        other => Err(format!("unknown model kind '{other}'")),
    }
}

pub fn delete_model(paths: &Paths, key: &str) -> Result<(), String> {
    match parse_model_key(key)? {
        (ModelKind::Whisper, size) => asr::delete_whisper_model(paths, size),
        (ModelKind::Translate, lang) => translate::delete_translate_model(paths, lang),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parses_model_keys() {
        assert_eq!(
            parse_model_key("whisper:base").unwrap(),
            (ModelKind::Whisper, "base")
        );
        assert_eq!(
            parse_model_key("translate:es").unwrap(),
            (ModelKind::Translate, "es")
        );
        assert!(parse_model_key("nokind").is_err());
        assert!(parse_model_key("gpt:4").is_err());
    }

    #[test]
    fn lists_and_deletes_models() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::in_dir(dir.path());

        let all = list_models(&paths).unwrap();
        assert_eq!(all.len(), 4 + 4);
        assert!(all.iter().all(|m| !m.downloaded && m.bytes_on_disk == 0));

        let whisper = paths.whisper_model_file("tiny");
        fs::create_dir_all(whisper.parent().unwrap()).unwrap();
        fs::write(&whisper, vec![0u8; 123]).unwrap();
        let tdir = paths.translate_model_dir("de");
        fs::create_dir_all(&tdir).unwrap();
        fs::write(tdir.join("a.onnx"), vec![0u8; 10]).unwrap();
        fs::write(tdir.join("b.json"), vec![0u8; 5]).unwrap();

        let all = list_models(&paths).unwrap();
        let tiny = all.iter().find(|m| m.key() == "whisper:tiny").unwrap();
        assert!(tiny.downloaded);
        assert_eq!(tiny.bytes_on_disk, 123);
        let de = all.iter().find(|m| m.key() == "translate:de").unwrap();
        assert!(de.downloaded);
        assert_eq!(de.bytes_on_disk, 15);

        delete_model(&paths, "whisper:tiny").unwrap();
        delete_model(&paths, "translate:de").unwrap();
        assert!(list_models(&paths).unwrap().iter().all(|m| !m.downloaded));
        // deleting something that's not there is fine
        delete_model(&paths, "whisper:tiny").unwrap();
    }

    #[test]
    fn rejects_unknown_or_traversal_ids() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::in_dir(dir.path());
        assert!(delete_model(&paths, "whisper:../../evil").is_err());
        assert!(delete_model(&paths, "translate:../x").is_err());
        assert!(delete_model(&paths, "translate:xx").is_err());
    }
}
