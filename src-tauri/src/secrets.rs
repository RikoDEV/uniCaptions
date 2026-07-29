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
