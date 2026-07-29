mod asr;
mod audio;
mod secrets;
mod translate;

const OPENAI_ASR_KEY: &str = "openai_asr";
const DEEPL_TRANSLATE_KEY: &str = "deepl_translate";

use std::sync::Mutex;
use tauri::{Emitter, Manager};

#[derive(Default)]
struct AppState {
    capture: Mutex<Option<audio::CaptureHandle>>,
    captioning: Mutex<Option<asr::CaptioningHandle>>,
    /// Serializes model downloads so a manual download from the Models UI
    /// can't race with one implicitly triggered by starting captioning.
    model_download_lock: tauri::async_runtime::Mutex<()>,
}

#[tauri::command]
fn set_overlay_click_through(app: tauri::AppHandle, ignore: bool) -> Result<(), String> {
    if let Some(overlay) = app.get_webview_window("overlay") {
        overlay.set_ignore_cursor_events(ignore).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn start_audio_capture(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    source: String,
) -> Result<(), String> {
    let mut guard = state.capture.lock().map_err(|e| e.to_string())?;
    if guard.is_some() {
        return Ok(());
    }
    let app_cb = app.clone();
    let handle = audio::start_capture_stream(&source, move |data, _rate, channels| {
        let mono = audio::downmix(data, channels);
        let level = audio::rms(&mono);
        let _ = app_cb.emit("audio-level", level);
    })?;
    *guard = Some(handle);
    Ok(())
}

#[tauri::command]
fn stop_audio_capture(state: tauri::State<AppState>) -> Result<(), String> {
    let mut guard = state.capture.lock().map_err(|e| e.to_string())?;
    if let Some(handle) = guard.take() {
        handle.stop();
    }
    Ok(())
}

#[derive(serde::Serialize)]
struct ModelEntry {
    id: String,
    label: String,
    downloaded: bool,
    #[serde(rename = "bytesOnDisk")]
    bytes_on_disk: u64,
}

#[tauri::command]
fn list_models(app: tauri::AppHandle) -> Result<Vec<ModelEntry>, String> {
    let mut entries: Vec<ModelEntry> = asr::list_whisper_models(&app)?
        .into_iter()
        .map(|m| ModelEntry {
            id: format!("whisper:{}", m.size),
            label: format!("Speech recognition ({})", m.size),
            downloaded: m.downloaded,
            bytes_on_disk: m.bytes_on_disk,
        })
        .collect();
    entries.extend(translate::list_translate_models(&app)?.into_iter().map(|m| {
        ModelEntry {
            id: format!("translate:{}", m.lang),
            label: format!("Translation (English -> {})", m.lang),
            downloaded: m.downloaded,
            bytes_on_disk: m.bytes_on_disk,
        }
    }));
    Ok(entries)
}

#[tauri::command]
fn delete_model(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let (kind, value) = id
        .split_once(':')
        .ok_or_else(|| format!("invalid model id '{id}'"))?;
    match kind {
        "whisper" => asr::delete_whisper_model(&app, value),
        "translate" => translate::delete_translate_model(&app, value),
        other => Err(format!("unknown model kind '{other}'")),
    }
}

#[tauri::command]
async fn download_whisper_model(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    size: String,
) -> Result<(), String> {
    let _guard = state.model_download_lock.lock().await;
    asr::ensure_model_downloaded(app, size).await?;
    Ok(())
}

#[tauri::command]
async fn download_translate_model(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    lang: String,
) -> Result<(), String> {
    let _guard = state.model_download_lock.lock().await;
    translate::ensure_model_downloaded(app, lang).await?;
    Ok(())
}

#[tauri::command]
fn is_translation_supported(target_lang: String) -> bool {
    translate::is_supported(&target_lang)
}

#[tauri::command]
fn save_api_key(provider: String, key: String) -> Result<(), String> {
    secrets::save(&provider_secret_key(&provider)?, &key)
}

#[tauri::command]
fn delete_api_key(provider: String) -> Result<(), String> {
    secrets::delete(&provider_secret_key(&provider)?)
}

#[tauri::command]
fn has_api_key(provider: String) -> Result<bool, String> {
    secrets::has(&provider_secret_key(&provider)?)
}

fn provider_secret_key(provider: &str) -> Result<&'static str, String> {
    match provider {
        "openai_asr" => Ok(OPENAI_ASR_KEY),
        "deepl_translate" => Ok(DEEPL_TRANSLATE_KEY),
        other => Err(format!("unknown provider '{other}'")),
    }
}

#[tauri::command]
async fn start_captioning(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    language: String,
    target_language: Option<String>,
    audio_source: String,
    asr_provider: String,
    asr_model: String,
    translate_provider: String,
) -> Result<(), String> {
    {
        let guard = state.captioning.lock().map_err(|e| e.to_string())?;
        if guard.is_some() {
            return Ok(());
        }
    }

    let asr_config = if asr_provider == "cloud" {
        let key = secrets::get(OPENAI_ASR_KEY)?
            .ok_or_else(|| "no OpenAI API key saved for cloud speech recognition".to_string())?;
        asr::AsrConfig::Cloud(key)
    } else {
        let model_path = {
            let _guard = state.model_download_lock.lock().await;
            asr::ensure_model_downloaded(app.clone(), asr_model).await?
        };
        asr::AsrConfig::Local(model_path)
    };

    let translate_config = match target_language {
        Some(lang) if translate_provider == "cloud" => {
            let key = secrets::get(DEEPL_TRANSLATE_KEY)?.ok_or_else(|| {
                "no DeepL API key saved for cloud translation".to_string()
            })?;
            asr::TranslateConfig::Cloud {
                api_key: key,
                target_lang: lang,
            }
        }
        Some(lang) if translate::is_supported(&lang) => {
            let translator = {
                let _guard = state.model_download_lock.lock().await;
                translate::load_translator(app.clone(), lang).await?
            };
            asr::TranslateConfig::Local(translator)
        }
        _ => asr::TranslateConfig::None,
    };

    let app_for_blocking = app.clone();
    let handle = tauri::async_runtime::spawn_blocking(move || {
        asr::start_captioning(
            app_for_blocking,
            asr_config,
            language,
            translate_config,
            audio_source,
        )
    })
    .await
    .map_err(|e| e.to_string())??;

    let mut guard = state.captioning.lock().map_err(|e| e.to_string())?;
    *guard = Some(handle);
    Ok(())
}

#[tauri::command]
fn stop_captioning(state: tauri::State<AppState>) -> Result<(), String> {
    let mut guard = state.captioning.lock().map_err(|e| e.to_string())?;
    if let Some(handle) = guard.take() {
        handle.stop();
    }
    Ok(())
}

/// Places the overlay window centered horizontally near the bottom of the
/// primary monitor, but only on first run — if `tauri-plugin-window-state`
/// already has a saved position for it (the user dragged it somewhere),
/// that takes precedence and this is a no-op.
fn position_overlay_if_unset(app: &tauri::AppHandle, overlay: &tauri::WebviewWindow) {
    let already_saved = app
        .path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join(".window-state.json"))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
        .map(|json| json.get("overlay").is_some())
        .unwrap_or(false);

    if already_saved {
        return;
    }

    let Ok(Some(monitor)) = overlay.primary_monitor() else {
        return;
    };
    let scale = monitor.scale_factor();
    let screen_size = monitor.size().to_logical::<f64>(scale);
    let win_width = 900.0;
    let win_height = 160.0;
    let x = ((screen_size.width - win_width) / 2.0).max(0.0);
    let y = (screen_size.height - win_height - 60.0).max(0.0);

    let _ = overlay.set_size(tauri::LogicalSize::new(win_width, win_height));
    let _ = overlay.set_position(tauri::LogicalPosition::new(x, y));
}

fn show_settings(app: &tauri::AppHandle) {
    if let Some(settings) = app.get_webview_window("settings") {
        let _ = settings.show();
        let _ = settings.unminimize();
        let _ = settings.set_focus();
    }
}

fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};

    let show_item = MenuItem::with_id(app, "show", "Show Settings", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

    TrayIconBuilder::new()
        .icon(app.default_window_icon().cloned().unwrap())
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_settings(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            } = event
            {
                show_settings(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(AppState::default())
        .setup(|app| {
            if let Some(overlay) = app.get_webview_window("overlay") {
                let _ = overlay.set_ignore_cursor_events(true);
                position_overlay_if_unset(app.handle(), &overlay);
            }
            build_tray(app.handle())?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "settings" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            set_overlay_click_through,
            start_audio_capture,
            stop_audio_capture,
            list_models,
            delete_model,
            download_whisper_model,
            download_translate_model,
            is_translation_supported,
            save_api_key,
            delete_api_key,
            has_api_key,
            start_captioning,
            stop_captioning
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
