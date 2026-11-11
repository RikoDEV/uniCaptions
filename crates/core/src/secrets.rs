const SERVICE: &str = "com.unicaptions.app";

fn entry(key: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, key).map_err(|e| e.to_string())
}

/// Persists an API key in the OS-native credential store (Windows Credential
/// Manager / macOS Keychain / Linux Secret Service) rather than the plain
/// JSON settings file.
pub fn save(key: &str, value: &str) -> Result<(), String> {
    entry(key)?.set_password(value).map_err(|e| e.to_string())
}

pub fn get(key: &str) -> Result<Option<String>, String> {
    match entry(key)?.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn delete(key: &str) -> Result<(), String> {
    match entry(key)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn has(key: &str) -> Result<bool, String> {
    Ok(get(key)?.is_some())
}

/// The cloud services whose API keys we store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApiProvider {
    /// OpenAI Whisper API (cloud speech recognition).
    OpenAiAsr,
    /// DeepL API (cloud translation).
    DeepL,
}

impl ApiProvider {
    /// Keyring entry name. Unchanged from the Tauri version so keys saved
    /// there keep working.
    pub fn secret_key(self) -> &'static str {
        match self {
            ApiProvider::OpenAiAsr => "openai_asr",
            ApiProvider::DeepL => "deepl_translate",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_keys_match_legacy_names() {
        assert_eq!(ApiProvider::OpenAiAsr.secret_key(), "openai_asr");
        assert_eq!(ApiProvider::DeepL.secret_key(), "deepl_translate");
        assert_eq!(SERVICE, "com.unicaptions.app");
    }
}
