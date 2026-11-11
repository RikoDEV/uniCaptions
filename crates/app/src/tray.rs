//! System tray icon with "Show Settings" / "Quit".
//!
//! Windows and macOS: the tray is created on the main (event loop) thread.
//! Linux: `tray-icon` needs a GTK main loop, which eframe (winit) does not
//! run, so the tray lives on its own GTK thread.

use crate::icons::TRAY_ICON_PNG;
use std::sync::Arc;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayLabels {
    pub show: String,
    pub quit: String,
}

pub type Callback = Arc<dyn Fn() + Send + Sync>;

fn load_icon() -> Result<Icon, String> {
    let img = image::load_from_memory(TRAY_ICON_PNG)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    let (w, h) = img.dimensions();
    Icon::from_rgba(img.into_raw(), w, h).map_err(|e| e.to_string())
}

/// Builds the menu + icon and wires the event handlers. Returns the pieces the
/// owner must keep alive and the ids of the two menu items.
fn build(
    labels: &TrayLabels,
    on_show: Callback,
    on_quit: Callback,
) -> Result<(tray_icon::TrayIcon, MenuItem, MenuItem), String> {
    let menu = Menu::new();
    let show = MenuItem::new(&labels.show, true, None);
    let quit = MenuItem::new(&labels.quit, true, None);
    menu.append_items(&[&show, &quit]).map_err(|e| e.to_string())?;
    let (show_id, quit_id) = (show.id().clone(), quit.id().clone());

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("uniCaptions")
        .with_icon(load_icon()?)
        .build()
        .map_err(|e| e.to_string())?;

    let show_cb = on_show.clone();
    MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
        if ev.id == show_id {
            show_cb();
        } else if ev.id == quit_id {
            on_quit();
        }
    }));
    TrayIconEvent::set_event_handler(Some(move |ev: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = ev
        {
            on_show();
        }
    }));
    Ok((tray, show, quit))
}

#[cfg(not(target_os = "linux"))]
pub struct Tray {
    _tray: tray_icon::TrayIcon,
    show: MenuItem,
    quit: MenuItem,
}

#[cfg(not(target_os = "linux"))]
impl Tray {
    pub fn new(labels: &TrayLabels, on_show: Callback, on_quit: Callback) -> Result<Self, String> {
        let (tray, show, quit) = build(labels, on_show, on_quit)?;
        Ok(Self {
            _tray: tray,
            show,
            quit,
        })
    }

    /// Must be called on the thread that created the tray (the UI thread).
    pub fn set_labels(&self, labels: &TrayLabels) {
        self.show.set_text(&labels.show);
        self.quit.set_text(&labels.quit);
    }
}

#[cfg(target_os = "linux")]
pub struct Tray {
    labels_tx: std::sync::mpsc::Sender<TrayLabels>,
}

#[cfg(target_os = "linux")]
impl Tray {
    pub fn new(labels: &TrayLabels, on_show: Callback, on_quit: Callback) -> Result<Self, String> {
        let (labels_tx, labels_rx) = std::sync::mpsc::channel::<TrayLabels>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let initial = labels.clone();
        std::thread::Builder::new()
            .name("unicaptions-tray".into())
            .spawn(move || {
                if let Err(e) = gtk::init() {
                    let _ = ready_tx.send(Err(format!("failed to initialise GTK: {e}")));
                    return;
                }
                match build(&initial, on_show, on_quit) {
                    Ok((tray, show, quit)) => {
                        let _ = ready_tx.send(Ok(()));
                        gtk::glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
                            while let Ok(l) = labels_rx.try_recv() {
                                show.set_text(&l.show);
                                quit.set_text(&l.quit);
                            }
                            gtk::glib::ControlFlow::Continue
                        });
                        gtk::main();
                        drop(tray);
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| "tray thread did not start".to_string())??;
        Ok(Self { labels_tx })
    }

    pub fn set_labels(&self, labels: &TrayLabels) {
        let _ = self.labels_tx.send(labels.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_icon_asset_decodes() {
        assert!(load_icon().is_ok());
    }
}
