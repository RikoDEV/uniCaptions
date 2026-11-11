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
    /// Keyring entry name. Unchanged from earlier versions so keys saved
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

    /// Guards against the in-memory mock store (keyring 3.x without a platform
    /// feature): a *new* `Entry` must see what an earlier one saved. Always runs
    /// on Windows; on Linux only with `UNICAPTIONS_TEST_KEYRING=1` (needs a running
    /// Secret Service such as gnome-keyring); skipped on macOS, whose CI keychains
    /// are locked.
    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn keys_persist_in_the_real_os_credential_store() {
        #[cfg(target_os = "linux")]
        if std::env::var_os("UNICAPTIONS_TEST_KEYRING").is_none() {
            return;
        }
        let name = format!("unicaptions-test-{}", std::process::id());
        assert_eq!(get(&name).unwrap(), None);
        save(&name, "s3cret-value").unwrap();
        assert!(has(&name).unwrap());
        assert_eq!(get(&name).unwrap().as_deref(), Some("s3cret-value"));
        delete(&name).unwrap();
        assert_eq!(get(&name).unwrap(), None);
        delete(&name).unwrap(); // deleting a missing key is fine
    }

    #[test]
    fn secret_keys_match_legacy_names() {
        assert_eq!(ApiProvider::OpenAiAsr.secret_key(), "openai_asr");
        assert_eq!(ApiProvider::DeepL.secret_key(), "deepl_translate");
        assert_eq!(SERVICE, "com.unicaptions.app");
    }
}
