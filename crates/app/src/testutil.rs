//! Helpers for headless egui tests.

use eframe::egui::{self, Context, RawInput};

/// Runs one headless pass and discards its output. egui panics if a pass's
/// texture updates are dropped unhandled, so they are cleared explicitly.
pub fn run_pass(ctx: &Context, input: RawInput, run_ui: impl FnMut(&mut egui::Ui)) {
    let mut out = ctx.run_ui(input, run_ui);
    out.textures_delta.clear();
}
