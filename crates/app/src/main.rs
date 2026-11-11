// No console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod color;
mod fonts;
mod i18n;
mod icons;
mod overlay;
mod state;
mod theme;
#[cfg(test)]
mod testutil;
mod tray;
mod ui;
mod widgets;

use eframe::egui;
use unicaptions_core::{Paths, SettingsStore};

fn window_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(icons::WINDOW_ICON_PNG).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    })
}

fn main() -> eframe::Result {
    // One instance at a time: a second launch would fight over the microphone,
    // the tray icon and the settings file.
    let instance = single_instance::SingleInstance::new("com.unicaptions.app").ok();
    if instance.as_ref().is_some_and(|i| !i.is_single()) {
        eprintln!("uniCaptions is already running");
        return Ok(());
    }

    let paths = Paths::system().expect("could not determine the user data directory");
    let store = SettingsStore::open(paths.settings_file());

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("uniCaptions Settings")
        .with_inner_size([900.0, 640.0])
        .with_min_inner_size([720.0, 480.0]);
    // On macOS/Linux the GL pixel format is chosen when the root window is
    // created and shared by every viewport, so the root needs an alpha channel
    // for the caption overlay to be see-through. (The Settings panels paint
    // opaque backgrounds, so it looks the same.) Not needed on Windows.
    #[cfg(not(windows))]
    {
        viewport = viewport.with_transparent(true);
    }
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        centered: true,
        ..Default::default()
    };

    let result = eframe::run_native(
        "uniCaptions",
        options,
        Box::new(move |cc| Ok(Box::new(app::UniApp::new(cc, paths, store)?))),
    );
    drop(instance);
    result
}
