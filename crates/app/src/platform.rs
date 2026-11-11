//! Small per-OS window tweaks that egui/winit do not expose.

/// Title of the overlay window (also used to find its native handle).
pub const OVERLAY_TITLE: &str = "uniCaptions";

/// Removes the OS-drawn chrome from the caption overlay: on Windows 10/11 an
/// undecorated *resizable* window still gets a DWM drop shadow, a 1 px border
/// and rounded corners, which show up as a frame around the transparent area.
///
/// Returns `true` once the window was found and updated (so callers can stop
/// retrying); always `true` on platforms where nothing needs doing.
#[cfg(windows)]
pub fn strip_overlay_chrome() -> bool {
    // Debug aid for A/B-testing the effect on screen pixels.
    #[cfg(debug_assertions)]
    if std::env::var_os("UNICAPTIONS_KEEP_CHROME").is_some() {
        return true;
    }
    use std::ffi::c_void;
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_NCRENDERING_POLICY,
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextW, GetWindowThreadProcessId,
    };

    /// `DWMWA_COLOR_NONE`: draw no border at all.
    const COLOR_NONE: u32 = 0xFFFF_FFFE;

    struct Search {
        pid: u32,
        found: HWND,
    }

    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = &mut *(lparam as *mut Search);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid != search.pid {
            return 1;
        }
        let mut buf = [0u16; 64];
        let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        let title = String::from_utf16_lossy(&buf[..len.max(0) as usize]);
        if title == OVERLAY_TITLE {
            search.found = hwnd;
            return 0; // stop
        }
        1
    }

    unsafe {
        let mut search = Search {
            pid: GetCurrentProcessId(),
            found: std::ptr::null_mut(),
        };
        EnumWindows(Some(visit), &mut search as *mut Search as LPARAM);
        if search.found.is_null() {
            return false;
        }
        let set = |attr: i32, value: u32| {
            DwmSetWindowAttribute(
                search.found,
                attr as u32,
                &value as *const u32 as *const c_void,
                std::mem::size_of::<u32>() as u32,
            )
        };
        // Each call can fail on older Windows versions that lack the attribute;
        // that is fine (those versions also lack the chrome it controls).
        set(DWMWA_NCRENDERING_POLICY, DWMNCRP_DISABLED as u32); // no shadow
        set(DWMWA_BORDER_COLOR, COLOR_NONE); // no border
        set(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND as u32);
        true
    }
}

#[cfg(not(windows))]
pub fn strip_overlay_chrome() -> bool {
    true
}
