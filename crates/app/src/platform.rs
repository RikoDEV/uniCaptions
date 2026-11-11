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
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_NCRENDERING_POLICY,
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowLongPtrW, GetWindowTextW, GetWindowThreadProcessId,
        SetWindowLongPtrW, SetWindowPos, GWL_STYLE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOSIZE, SWP_NOZORDER, WM_NCCALCSIZE, WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX,
        WS_SYSMENU,
    };

    const SUBCLASS_ID: usize = 0x756e_6963; // "unic"

    /// Window procedure hook: no non-client area.
    unsafe extern "system" fn no_frame_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        // wParam != 0: the proposed client rectangle (= whole window) is used as is.
        if msg == WM_NCCALCSIZE && wparam != 0 {
            return 0;
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }

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

        // Disabling DWM's non-client rendering makes Windows paint the legacy
        // sizing frame (a visible line on every edge) instead. Remove the
        // non-client area altogether: with WM_NCCALCSIZE answered as "the client
        // area is the whole window" there is nothing left to draw. Resizing keeps
        // working because the window keeps its WS_THICKFRAME style.
        SetWindowSubclass(search.found, Some(no_frame_proc), SUBCLASS_ID, 0);
        // winit leaves the overlay with a full WS_OVERLAPPEDWINDOW style (caption,
        // system menu, min/max boxes) and merely hides the title bar by zeroing the
        // non-client area; DWM can still paint a line of that frame at the window
        // edge. Remove the caption-related styles outright. WS_THICKFRAME stays so
        // the OS-driven edge resize keeps working.
        let style = GetWindowLongPtrW(search.found, GWL_STYLE);
        let remove = (WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX) as isize;
        SetWindowLongPtrW(search.found, GWL_STYLE, style & !remove);
        SetWindowPos(
            search.found,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        true
    }
}

#[cfg(not(windows))]
pub fn strip_overlay_chrome() -> bool {
    true
}

/// Makes the overlay visible on every virtual desktop / Space (earlier
/// versions had a "visible on all workspaces" overlay). Best effort, called repeatedly until it
/// returns `true` (window found and updated, or nothing can be done here).
///
/// * macOS: `NSWindow.collectionBehavior` = CanJoinAllSpaces | Stationary |
///   FullScreenAuxiliary (so it also floats over full-screen apps).
/// * Linux/X11: the EWMH `_NET_WM_STATE_STICKY` state, requested from the window manager.
/// * Windows: there is no public API for pinning a window to all virtual
///   desktops, so this is a no-op there. Wayland has no client-side API either.
#[cfg(target_os = "macos")]
pub fn follow_workspaces() -> bool {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    use std::ffi::{c_char, CStr};

    // NSWindowCollectionBehavior bits.
    const CAN_JOIN_ALL_SPACES: usize = 1 << 0;
    const STATIONARY: usize = 1 << 4;
    const FULL_SCREEN_AUXILIARY: usize = 1 << 8;

    // Must run on the main thread, which is where eframe runs every viewport.
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        if app.is_null() {
            return false;
        }
        let windows: *mut AnyObject = msg_send![app, windows];
        if windows.is_null() {
            return false;
        }
        let count: usize = msg_send![windows, count];
        for i in 0..count {
            let window: *mut AnyObject = msg_send![windows, objectAtIndex: i];
            if window.is_null() {
                continue;
            }
            let title: *mut AnyObject = msg_send![window, title];
            if title.is_null() {
                continue;
            }
            let utf8: *const c_char = msg_send![title, UTF8String];
            if utf8.is_null() || CStr::from_ptr(utf8).to_bytes() != OVERLAY_TITLE.as_bytes() {
                continue;
            }
            let behavior = CAN_JOIN_ALL_SPACES | STATIONARY | FULL_SCREEN_AUXILIARY;
            let _: () = msg_send![window, setCollectionBehavior: behavior];
            return true;
        }
    }
    false
}

#[cfg(target_os = "linux")]
pub fn follow_workspaces() -> bool {
    use std::sync::atomic::{AtomicU32, Ordering};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, Window};

    /// Give up after this many tries (e.g. under Wayland or a window manager
    /// without EWMH support the window never shows up in `_NET_CLIENT_LIST`).
    const MAX_ATTEMPTS: u32 = 60;
    static ATTEMPTS: AtomicU32 = AtomicU32::new(0);
    if ATTEMPTS.fetch_add(1, Ordering::Relaxed) >= MAX_ATTEMPTS {
        return true;
    }

    let attempt = || -> Option<bool> {
        let (conn, screen_num) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen_num)?.root;
        let atom = |name: &str| -> Option<u32> {
            Some(
                conn.intern_atom(false, name.as_bytes())
                    .ok()?
                    .reply()
                    .ok()?
                    .atom,
            )
        };
        let client_list = atom("_NET_CLIENT_LIST")?;
        let wm_pid = atom("_NET_WM_PID")?;
        let wm_name = atom("_NET_WM_NAME")?;
        let utf8 = atom("UTF8_STRING")?;
        let wm_state = atom("_NET_WM_STATE")?;
        let sticky = atom("_NET_WM_STATE_STICKY")?;

        let list = conn
            .get_property(false, root, client_list, AtomEnum::WINDOW, 0, 4096)
            .ok()?
            .reply()
            .ok()?;
        let windows: Vec<Window> = list.value32()?.collect();
        let my_pid = std::process::id();

        for win in windows {
            let pid = conn
                .get_property(false, win, wm_pid, AtomEnum::CARDINAL, 0, 1)
                .ok()?
                .reply()
                .ok()?
                .value32()
                .and_then(|mut v| v.next());
            if pid != Some(my_pid) {
                continue;
            }
            let name = conn
                .get_property(false, win, wm_name, utf8, 0, 64)
                .ok()?
                .reply()
                .ok()?;
            if name.value != OVERLAY_TITLE.as_bytes() {
                continue;
            }
            // _NET_WM_STATE client message: [action (1 = add), property, 0, source (1 = app), 0]
            let event = ClientMessageEvent::new(32, win, wm_state, [1, sticky, 0, 1, 0]);
            conn.send_event(
                false,
                root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                event,
            )
            .ok()?;
            conn.flush().ok()?;
            return Some(true);
        }
        Some(false)
    };

    // No X server reachable (pure Wayland): nothing we can do, so stop retrying.
    attempt().unwrap_or(true)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn follow_workspaces() -> bool {
    true
}
