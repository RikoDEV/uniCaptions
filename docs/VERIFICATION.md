# Rewrite verification record

Status of the Tauri → Rust/egui rewrite, per platform. "Verified" means it was
run on real hardware/OS, not just compiled.

## Windows 11 — verified

| Area | Result |
|---|---|
| `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace` | pass (78 tests; 4 `#[ignore]`d end-to-end/profiling tests run manually) |
| Settings window: 5 tabs, dark + light theme, Japanese/Polish UI (CJK glyphs) | rendered and inspected |
| Overlay window: topmost, transparent, click-through, translucent caption box | verified from real window styles and screen pixels |
| Overlay drag + corner resize with real mouse input; click-through blocks both | verified; geometry persisted and restored |
| Settings window size persisted and restored | verified |
| Hide to tray on close, re-show path, 0 % CPU while hidden/idle | verified |
| Overlay shadow/border (DWM) removed | A/B on screen pixels: before 1.5–2 k of 2.6 k samples changed, after 0 |
| Local Whisper (Vulkan GPU), local OPUS-MT translation | verified against real models |
| Live pipeline: WASAPI loopback → Whisper → caption events → real overlay | verified; first word ~1 s after speech starts |
| OS credential store persistence (keyring) | verified (regression test) |
| NSIS installer via `cargo-packager` | built; contents inspected (not installed) |
| Release build | 0 % idle CPU, ~113 MB working set |

## Linux (Debian 13 clone under WSL2 + WSLg) — verified, with caveats

Built and run natively in a throwaway clone of a Debian 13 WSL distro (since
deleted), with the same system packages as CI.

| Area | Result |
|---|---|
| `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace` | pass (78 tests) |
| Whisper (CPU) and OPUS-MT translation end-to-end tests | pass: correct transcript in ~0.95 s, "Hola, ¿cómo estás hoy?" |
| Keyring persistence against a real gnome-keyring Secret Service | pass (needs an initialised default collection, as a desktop login provides) |
| App launch under X11 (WSLg): Settings 900x640 + overlay 900x160 created, stable, ~0 % idle CPU, ~150 MB | verified |
| `.deb` via `cargo packager`: builds, `apt install` resolves deps, binary + desktop entry + icon installed, launches | verified |
| AppImage via `cargo packager`: builds (26 MB), launches | verified |

Bugs this found and fixed: Linux CI/release workflows were missing `libssl-dev`
(needed by `ort-sys`'s build script); `CaptureHandle::from_thread` was dead code
off Windows (failed `-D warnings`); the `.deb` did not declare runtime libraries
(`libxkbcommon-x11-0` is dlopen-ed by winit and crashed the app on X11 without it).

Caveats — NOT covered:
- No real desktop environment: the overlay's `_NET_WM_STATE` showed no "above"
  flag under WSLg's minimal window manager, so **always-on-top on Linux is unconfirmed**,
  as are skip-taskbar and click-through (`mousePassthrough`).
- **Tray:** the GTK tray thread starts and runs, but there was no StatusNotifier
  host, so the icon/menu/"show settings" were never exercised.
- Wayland session behaviour (positioning is ignored by Wayland compositors).
- No microphone / live captioning on Linux.
- `cargo packager` exits 0 even when AppImage bundling fails; the release
  workflow relies on `upload-artifact`'s `if-no-files-found: error` to catch that.

## macOS — NOT verified

Nothing has been compiled or run on macOS. CI is configured but has not run.
Risks: DMG packaging; `NSMicrophoneUsageDescription` handling; `with_has_shadow(false)`;
transparent root viewport (needed for the overlay's GL alpha); tray on the main thread.

## Known gaps (not implemented)

- **Overlay on all virtual desktops/workspaces** (old `visibleOnAllWorkspaces`):
  implemented best-effort in `crates/app/src/platform.rs::follow_workspaces`, but
  **not behaviourally verified anywhere**.
  - macOS: sets `NSWindow.collectionBehavior` (CanJoinAllSpaces | Stationary |
    FullScreenAuxiliary) via the Objective-C runtime. Type-checked for
    `aarch64-apple-darwin`; never run on a Mac.
  - Linux/X11: requests EWMH `_NET_WM_STATE_STICKY` via `x11rb`. Type-checked for
    `x86_64-unknown-linux-gnu`; never run (WSLg's window manager ignores EWMH
    window-state hints, so it could not have confirmed it anyway).
  - Windows: no public API exists, so it is a no-op. Wayland: no client-side API.
- **System-audio capture on macOS/Linux** (was Windows-only before the rewrite).
- Settings window *position* is not restored (size is), to avoid restoring onto
  a disconnected monitor.

## To finish cross-platform verification

macOS still needs a real run: push the branch (CI runs fmt/clippy/tests on
Ubuntu, Windows, macOS) and push a `v*` pre-release tag to build the installers;
then install + launch on a Mac. On Linux, also try a real desktop session with a
tray host to confirm the tray, always-on-top and click-through.
