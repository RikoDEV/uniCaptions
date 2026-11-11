//! Launch-at-login, via the `auto-launch` crate (the same one the Tauri
//! autostart plugin wrapped, and with the same app name so existing entries
//! are recognised).

pub trait Autostart: Send + Sync {
    fn is_enabled(&self) -> bool;
    fn set_enabled(&self, enabled: bool) -> Result<(), String>;
}

pub struct SystemAutostart {
    inner: Option<auto_launch::AutoLaunch>,
}

impl SystemAutostart {
    pub fn new() -> Self {
        let inner = std::env::current_exe().ok().and_then(|exe| {
            let path = exe.to_str()?.to_string();
            auto_launch::AutoLaunchBuilder::new()
                .set_app_name("uniCaptions")
                .set_app_path(&path)
                .set_macos_launch_mode(auto_launch::MacOSLaunchMode::LaunchAgent)
                .build()
                .ok()
        });
        Self { inner }
    }
}

impl Default for SystemAutostart {
    fn default() -> Self {
        Self::new()
    }
}

impl Autostart for SystemAutostart {
    fn is_enabled(&self) -> bool {
        self.inner
            .as_ref()
            .and_then(|a| a.is_enabled().ok())
            .unwrap_or(false)
    }

    fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        let a = self.inner.as_ref().ok_or("autostart is unavailable")?;
        if enabled {
            a.enable().map_err(|e| e.to_string())
        } else {
            a.disable().map_err(|e| e.to_string())
        }
    }
}

/// In-memory stand-in used by tests.
#[cfg(test)]
#[derive(Default)]
pub struct FakeAutostart(pub std::sync::atomic::AtomicBool);

#[cfg(test)]
impl Autostart for FakeAutostart {
    fn is_enabled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        self.0.store(enabled, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_toggles() {
        let a = FakeAutostart::default();
        assert!(!a.is_enabled());
        a.set_enabled(true).unwrap();
        assert!(a.is_enabled());
    }

    #[test]
    fn system_autostart_constructs_and_can_be_queried() {
        // Reading the state must never panic (it only reads the registry /
        // launch agent / .desktop entry).
        let a = SystemAutostart::new();
        let _ = a.is_enabled();
    }
}
