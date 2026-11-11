use serde::Deserialize;

#[derive(Deserialize)]
struct DeepLResponse {
    translations: Vec<DeepLTranslation>,
}

#[derive(Deserialize)]
struct DeepLTranslation {
    text: String,
}

/// DeepL wants regional variants for some targets (plain `EN` / `PT` are
/// deprecated as *target* languages).
pub fn deepl_target(lang: &str) -> String {
    match lang.to_ascii_lowercase().as_str() {
        "en" => "EN-US".to_string(),
        "pt" => "PT-PT".to_string(),
        other => other.to_uppercase(),
    }
}

/// Translates text using the DeepL API. Free-tier keys (suffixed `:fx`) use
/// the free API host; paid keys use the standard host.
pub async fn translate(api_key: &str, text: &str, target_lang: &str) -> Result<String, String> {
    let base = if api_key.trim_end().ends_with(":fx") {
        "https://api-free.deepl.com"
    } else {
        "https://api.deepl.com"
    };
    let target = deepl_target(target_lang);

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{base}/v2/translate"))
        .form(&[
            ("auth_key", api_key),
            ("text", text),
            ("target_lang", &target),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("DeepL request failed: {body}"));
    }

    let parsed: DeepLResponse = response.json().await.map_err(|e| e.to_string())?;
    parsed
        .translations
        .into_iter()
        .next()
        .map(|t| t.text)
        .ok_or_else(|| "DeepL returned no translation".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepl_target_codes() {
        assert_eq!(deepl_target("en"), "EN-US");
        assert_eq!(deepl_target("pt"), "PT-PT");
        assert_eq!(deepl_target("pl"), "PL");
        assert_eq!(deepl_target("zh"), "ZH");
        assert_eq!(deepl_target("DE"), "DE");
    }
}
