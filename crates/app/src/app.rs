//! The eframe application: owns the engine, settings and the two windows
//! (Settings = root viewport, caption overlay = deferred viewport).

use crate::autostart::{Autostart, SystemAutostart};
use crate::fonts::{apply_caption_font, AppliedFont, FontCatalog};
use crate::i18n::I18n;
use crate::overlay::{self, overlay_viewport_id, OverlayShared};
use crate::state::{captioning_params, CaptioningStatus, UiState};
use crate::theme;
use crate::tray::{Callback, Tray, TrayLabels};
use crate::ui;
use eframe::egui::{self, ViewportCommand, ViewportId};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use unicaptions_core::settings::OverlayWindowState;
use unicaptions_core::{Engine, EventSink, Paths, Settings, SettingsStore, SharedSink, UiEvent};

/// Forwards engine events to the UI: caption updates go straight to the
/// overlay (which repaints independently of the Settings window); everything
/// else is queued for the Settings window and wakes it up.
struct AppSink {
    tx: Sender<UiEvent>,
    ctx: egui::Context,
    overlay: Arc<OverlayShared>,
    settings_visible: Arc<AtomicBool>,
}

impl EventSink for AppSink {
    fn emit(&self, event: UiEvent) {
        match &event {
            UiEvent::Caption(c) => {
                self.overlay.set_caption(c);
                self.ctx.request_repaint_of(overlay_viewport_id());
                return;
            }
            // The level meter is only visible with the Settings window open.
            UiEvent::AudioLevel(_) if !self.settings_visible.load(Ordering::Relaxed) => return,
            _ => {}
        }
        if self.tx.send(event).is_ok() {
            self.ctx.request_repaint_of(ViewportId::ROOT);
        }
    }
}

pub fn show_settings(ctx: &egui::Context, visible: &AtomicBool) {
    visible.store(true, Ordering::Relaxed);
    ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Visible(true));
    ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Focus);
    ctx.request_repaint_of(ViewportId::ROOT);
}

fn tray_labels(t: &I18n) -> TrayLabels {
    TrayLabels {
        show: t.t("tray.show"),
        quit: t.t("tray.quit"),
    }
}

pub struct UniApp {
    engine: Arc<Engine>,
    store: Arc<SettingsStore>,
    paths: Paths,
    i18n: I18n,
    rx: Receiver<UiEvent>,
    overlay: Arc<OverlayShared>,
    overlay_initial: Option<OverlayWindowState>,
    frames: u32,
    st: UiState,
    fonts: FontCatalog,
    applied_font: AppliedFont,
    autostart: Box<dyn Autostart>,
    tray: Option<Tray>,
    settings_visible: Arc<AtomicBool>,
    window_tracker: crate::window_state::Tracker,
}

impl UniApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        paths: Paths,
        store: Arc<SettingsStore>,
    ) -> Result<Self, String> {
        let ctx = cc.egui_ctx.clone();
        egui_extras::install_image_loaders(&ctx);
        theme::install(&ctx);

        let settings = store.get();
        let i18n = I18n::new(&settings.ui_language);
        let overlay = OverlayShared::new(store.clone(), paths.window_state_file());
        let settings_visible = Arc::new(AtomicBool::new(true));

        let (tx, rx) = channel();
        let sink: SharedSink = Arc::new(AppSink {
            tx,
            ctx: ctx.clone(),
            overlay: overlay.clone(),
            settings_visible: settings_visible.clone(),
        });
        let engine = Engine::new(paths.clone(), sink)?;

        let fonts = {
            let ctx = ctx.clone();
            FontCatalog::spawn(move || ctx.request_repaint_of(ViewportId::ROOT))
        };
        let style = &settings.caption_style;
        apply_caption_font(&ctx, &fonts, &style.font_family, style.font_weight);
        let applied_font = AppliedFont {
            family: style.font_family.clone(),
            weight: style.font_weight,
            catalog_ready: false,
        };

        let on_show: Callback = {
            let ctx = ctx.clone();
            let visible = settings_visible.clone();
            Arc::new(move || show_settings(&ctx, &visible))
        };
        let on_quit: Callback = {
            let engine = engine.clone();
            let store = store.clone();
            Arc::new(move || {
                engine.shutdown();
                store.flush();
                std::process::exit(0);
            })
        };
        let tray = match Tray::new(&tray_labels(&i18n), on_show, on_quit) {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!("system tray unavailable: {e}");
                None
            }
        };

        // Development aids (debug builds only): pick the initial tab / theme.
        #[cfg(debug_assertions)]
        let mut st = UiState::default();
        #[cfg(not(debug_assertions))]
        let st = UiState::default();
        #[cfg(debug_assertions)]
        {
            use crate::state::Tab;
            if let Ok(tab) = std::env::var("UNICAPTIONS_DEMO_TAB") {
                st.tab = match tab.as_str() {
                    "style" => Tab::CaptionStyle,
                    "translation" => Tab::Translation,
                    "models" => Tab::Models,
                    "about" => Tab::About,
                    _ => Tab::General,
                };
            }
            match std::env::var("UNICAPTIONS_DEMO_THEME").as_deref() {
                Ok("light") => ctx.set_theme(egui::ThemePreference::Light),
                Ok("dark") => ctx.set_theme(egui::ThemePreference::Dark),
                _ => {}
            }
        }

        // Development aid: show a fixed caption without needing a microphone.
        #[cfg(debug_assertions)]
        if let Ok(text) = std::env::var("UNICAPTIONS_DEMO_CAPTION") {
            overlay.set_caption(&unicaptions_core::CaptionEvent {
                text,
                translated_text: std::env::var("UNICAPTIONS_DEMO_TRANSLATION").ok(),
                is_final: true,
                timestamp: 0,
            });
        }

        // Development aid: re-show the Settings window after N seconds, through
        // the same path the tray callback uses (lets us test hide -> show).
        #[cfg(debug_assertions)]
        if let Some(secs) = std::env::var("UNICAPTIONS_TEST_SHOW_AFTER")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
        {
            let ctx = ctx.clone();
            let visible = settings_visible.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(secs));
                show_settings(&ctx, &visible);
            });
        }

        let paths_for_tracker = paths.settings_window_file();
        Ok(Self {
            engine,
            store,
            paths,
            i18n,
            rx,
            overlay,
            overlay_initial: None,
            frames: 0,
            st,
            fonts,
            applied_font,
            autostart: Box::new(SystemAutostart::new()),
            tray,
            settings_visible,
            window_tracker: crate::window_state::Tracker::new(
                unicaptions_core::settings::OverlayWindowState::load(&paths_for_tracker),
            ),
        })
    }

    /// Per-pass bookkeeping shared by `logic` (which also runs while the
    /// Settings window is hidden) and `ui`.
    fn sync(&mut self, ctx: &egui::Context) {
        let settings = self.store.get();

        while let Ok(event) = self.rx.try_recv() {
            self.st.apply_event(&event, &self.i18n);
        }

        if self.i18n.language() != settings.ui_language {
            self.i18n.set_language(&settings.ui_language);
            if let Some(tray) = &self.tray {
                tray.set_labels(&tray_labels(&self.i18n));
            }
        }

        let style = &settings.caption_style;
        let want = AppliedFont {
            family: style.font_family.clone(),
            weight: style.font_weight,
            catalog_ready: self.fonts.is_ready(),
        };
        if want != self.applied_font {
            apply_caption_font(ctx, &self.fonts, &want.family, want.weight);
            self.applied_font = want;
            ctx.request_repaint_of(overlay_viewport_id());
        }

        if self.st.models_dirty {
            self.st.models = self.engine.list_models().unwrap_or_default();
            self.st.models_dirty = false;
        }

        if !self.st.auto_start_done {
            self.st.auto_start_done = true;
            if settings.auto_start_captioning && self.st.captioning == CaptioningStatus::Idle {
                self.st.captioning = CaptioningStatus::Starting;
                self.engine.start_captioning(captioning_params(&settings));
            }
        }
    }

    /// Where the overlay window goes: the saved placement, or bottom-centre of the screen.
    fn overlay_placement(&mut self, ctx: &egui::Context) -> Option<OverlayWindowState> {
        if let Some(s) = self.overlay_initial {
            return Some(s);
        }
        let placement = OverlayWindowState::load(&self.paths.window_state_file()).or_else(|| {
            let monitor = ctx.input(|i| i.viewport().monitor_size);
            match monitor {
                Some(m) => Some(OverlayWindowState::default_for_screen(m.x, m.y)),
                // The monitor size arrives with the first frames; fall back after a few.
                None if self.frames > 5 => {
                    Some(OverlayWindowState::default_for_screen(1920.0, 1080.0))
                }
                None => None,
            }
        });
        self.overlay_initial = placement;
        placement
    }
}

impl eframe::App for UniApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Closing the Settings window hides it to the tray (the app keeps
        // captioning); without a tray there would be no way back, so quit.
        if self.tray.is_some() && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            self.settings_visible.store(false, Ordering::Relaxed);
        }
        self.sync(ctx);
    }

    fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frames = self.frames.saturating_add(1);
        let ctx = root_ui.ctx().clone();

        match self.overlay_placement(&ctx) {
            Some(initial) => overlay::show(&ctx, &self.overlay, initial),
            None => ctx.request_repaint(),
        }

        let mut settings: Settings = self.store.get();
        let before = settings.clone();
        {
            let mut cx = ui::Cx {
                t: &self.i18n,
                pal: theme::palette_for(&ctx),
                settings: &mut settings,
                st: &mut self.st,
                engine: &self.engine,
                fonts: &self.fonts,
                autostart: self.autostart.as_ref(),
            };
            ui::draw(root_ui, &mut cx);
        }
        if settings != before {
            self.store.set(settings);
            ctx.request_repaint_of(overlay_viewport_id());
        }

        crate::window_state::track(
            &ctx,
            &mut self.window_tracker,
            &self.paths.settings_window_file(),
        );

        // Keep the level meter / progress bars moving while active.
        if self.st.mic_active
            || self.st.captioning != CaptioningStatus::Idle
            || !self.st.progress.is_empty()
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// The overlay is transparent; the Settings panels paint their own backgrounds.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array()
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.engine.shutdown();
        self.store.flush();
    }
}
