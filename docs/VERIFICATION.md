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

## macOS and Linux — NOT verified

Nothing in this repository has been compiled or run on macOS or Linux. CI
(`.github/workflows/ci.yml`) is configured for both but has not been executed.
Specific risks, in order of likelihood:

1. **Linux tray** (`crates/app/src/tray.rs`): runs on a dedicated GTK thread;
   needs `libayatana-appindicator3` + `libxdo` at build time.
2. **Linux keyring** (`sync-secret-service`): needs a running Secret Service
   (GNOME Keyring / KWallet) at runtime.
3. **Packaging** (`cargo packager`): DMG and deb/AppImage config is untested;
   the release-upload step has never run.
4. **macOS microphone permission**: `Info.plist` adds
   `NSMicrophoneUsageDescription`; behaviour untested.
5. **macOS overlay shadow**: `with_has_shadow(false)` is set but unobserved.
6. Root viewport is created transparent on non-Windows so the overlay's GL
   config has an alpha channel; unverified there.

## Known gaps (not implemented)

- **Overlay on all virtual desktops/workspaces.** The old Tauri build set
  `visibleOnAllWorkspaces`. There is no public Windows API for this; macOS
  (`NSWindowCollectionBehaviorCanJoinAllSpaces`) and X11 (`_NET_WM_DESKTOP`)
  need native code that could not be tested here.
- **System-audio capture on macOS/Linux** (was Windows-only before the rewrite).
- Settings window *position* is not restored (size is), to avoid restoring onto
  a disconnected monitor.

## To finish cross-platform verification

Push the branch (CI runs fmt/clippy/tests on Ubuntu, Windows, macOS), then push
a `v*` pre-release tag to build installers on all three, and install + launch
each by hand.
