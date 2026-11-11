//! The transparent, always-on-top caption overlay window.
//!
//! Port of `Overlay.tsx` / `overlay.css`: a click-through window that shows the
//! current caption (and optionally its translation) in a rounded box anchored
//! to the bottom centre. When click-through is off the box can be dragged and
//! the window resized from its edges.

use crate::color::with_alpha;
use crate::fonts::caption_family;
use eframe::egui::text::LayoutJob;
use eframe::egui::viewport::ResizeDirection;
use eframe::egui::{
    self, pos2, vec2, Align, Color32, Context, CursorIcon, FontId, Galley, Id, PointerButton, Rect,
    Sense, Ui, ViewportBuilder, ViewportCommand, ViewportId,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use unicaptions_core::settings::{OverlayWindowState, SettingsStore};
use unicaptions_core::CaptionEvent;

const OUTER_MARGIN: f32 = 12.0;
const PAD_X: f32 = 18.0;
const PAD_Y: f32 = 10.0;
const CORNER_RADIUS: u8 = 10;
const TRANSLATION_SCALE: f32 = 0.85;
const TRANSLATION_GAP: f32 = 4.0;
const RESIZE_BAND: f32 = 8.0;

pub fn overlay_viewport_id() -> ViewportId {
    ViewportId::from_hash_of("unicaptions-overlay")
}

/// What the overlay is currently showing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CaptionView {
    pub text: String,
    pub translated: Option<String>,
}

struct Runtime {
    /// Last click-through state we told the OS about.
    passthrough: Option<bool>,
    tracker_unset: bool,
    chrome_stripped: bool,
    tracker: crate::window_state::Tracker,
}

/// State shared between the main window (which receives engine events) and the
/// overlay viewport (which paints them).
pub struct OverlayShared {
    caption: Mutex<CaptionView>,
    store: Arc<SettingsStore>,
    window_file: PathBuf,
    runtime: Mutex<Runtime>,
}

impl OverlayShared {
    pub fn new(store: Arc<SettingsStore>, window_file: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            caption: Mutex::new(CaptionView::default()),
            store,
            window_file,
            runtime: Mutex::new(Runtime {
                passthrough: None,
                tracker_unset: true,
                chrome_stripped: false,
                tracker: crate::window_state::Tracker::default(),
            }),
        })
    }

    pub fn set_caption(&self, event: &CaptionEvent) {
        if let Ok(mut c) = self.caption.lock() {
            c.text = event.text.clone();
            c.translated = event.translated_text.clone();
        }
    }

    pub fn caption(&self) -> CaptionView {
        self.caption.lock().map(|c| c.clone()).unwrap_or_default()
    }
}

/// Declares the overlay viewport. Call every frame from the main window.
pub fn show(ctx: &Context, shared: &Arc<OverlayShared>, initial: OverlayWindowState) {
    if let Ok(mut rt) = shared.runtime.lock() {
        if rt.tracker_unset {
            rt.tracker = crate::window_state::Tracker::new(Some(initial));
            rt.tracker_unset = false;
        }
    }
    let shared = shared.clone();
    ctx.show_viewport_deferred(
        overlay_viewport_id(),
        viewport_builder(initial),
        move |ui, _class| render(ui, &shared),
    );
}

pub fn viewport_builder(state: OverlayWindowState) -> ViewportBuilder {
    ViewportBuilder::default()
        .with_title(crate::platform::OVERLAY_TITLE)
        .with_position(pos2(state.x, state.y))
        .with_inner_size(vec2(state.width, state.height))
        .with_min_inner_size(vec2(240.0, 70.0))
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_taskbar(false)
        .with_mouse_passthrough(true)
        .with_active(false)
        .with_resizable(true)
        // macOS: a shadow around a transparent window leaves ghosting artifacts.
        .with_has_shadow(false)
}

// ---- pure helpers ------------------------------------------------------------

/// CJK scripts have no spaces, so wrapping must be allowed between any two characters.
pub fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x2E80..=0x2FDF   // radicals
        | 0x3000..=0x303F // CJK punctuation
        | 0x3040..=0x30FF // hiragana, katakana
        | 0x3100..=0x312F // bopomofo
        | 0x31F0..=0x31FF
        | 0x3400..=0x4DBF // extension A
        | 0x4E00..=0x9FFF // unified ideographs
        | 0xAC00..=0xD7AF // hangul
        | 0xF900..=0xFAFF
        | 0xFF00..=0xFFEF // full-width forms
        | 0x20000..=0x2FA1F)
}

pub fn has_cjk(s: &str) -> bool {
    s.chars().any(is_cjk)
}

/// Rows left for the translation once the original has used `rows_used`
/// (the old CSS applied one `-webkit-line-clamp` to the whole box).
pub fn translation_row_budget(max_lines: u32, rows_used: usize) -> usize {
    (max_lines.max(1) as usize).saturating_sub(rows_used)
}

/// The eight invisible resize handles along the window edges.
pub fn resize_hotspots(full: Rect) -> [(Rect, ResizeDirection, CursorIcon); 8] {
    let b = RESIZE_BAND;
    let (l, r, t, bt) = (full.left(), full.right(), full.top(), full.bottom());
    let corner = |x: f32, y: f32| Rect::from_min_size(pos2(x, y), vec2(b, b));
    [
        (
            corner(l, t),
            ResizeDirection::NorthWest,
            CursorIcon::ResizeNorthWest,
        ),
        (
            corner(r - b, t),
            ResizeDirection::NorthEast,
            CursorIcon::ResizeNorthEast,
        ),
        (
            corner(l, bt - b),
            ResizeDirection::SouthWest,
            CursorIcon::ResizeSouthWest,
        ),
        (
            corner(r - b, bt - b),
            ResizeDirection::SouthEast,
            CursorIcon::ResizeSouthEast,
        ),
        (
            Rect::from_min_max(pos2(l + b, t), pos2(r - b, t + b)),
            ResizeDirection::North,
            CursorIcon::ResizeNorth,
        ),
        (
            Rect::from_min_max(pos2(l + b, bt - b), pos2(r - b, bt)),
            ResizeDirection::South,
            CursorIcon::ResizeSouth,
        ),
        (
            Rect::from_min_max(pos2(l, t + b), pos2(l + b, bt - b)),
            ResizeDirection::West,
            CursorIcon::ResizeWest,
        ),
        (
            Rect::from_min_max(pos2(r - b, t + b), pos2(r, bt - b)),
            ResizeDirection::East,
            CursorIcon::ResizeEast,
        ),
    ]
}

fn layout(
    ctx: &Context,
    text: &str,
    font: FontId,
    color: Color32,
    width: f32,
    max_rows: usize,
) -> Arc<Galley> {
    let mut job = LayoutJob::simple(text.to_string(), font, color, width);
    job.wrap.max_rows = max_rows.max(1);
    job.wrap.break_anywhere = has_cjk(text);
    job.wrap.overflow_character = Some('\u{2026}');
    job.halign = Align::Center;
    ctx.fonts_mut(|f| f.layout_job(job))
}

/// One laid-out block (original or translation) and the gap above it.
struct Block {
    galley: Arc<Galley>,
    gap_above: f32,
}

/// What would be drawn, computed without painting (testable).
pub struct Layout {
    blocks: Vec<Block>,
    pub box_size: egui::Vec2,
}

fn compute_layout(
    ctx: &Context,
    view: &CaptionView,
    style: &unicaptions_core::settings::CaptionStyle,
    window_width: f32,
) -> Option<Layout> {
    let max_text_w = (window_width - 2.0 * OUTER_MARGIN - 2.0 * PAD_X).max(60.0);
    let font = FontId::new(style.font_size.max(6.0), caption_family());
    let color = with_alpha(&style.text_color, [255, 255, 255], 1.0);
    let max_lines = style.max_lines.max(1);

    let mut blocks: Vec<Block> = Vec::new();
    let mut rows_used = 0usize;

    if style.show_original && !view.text.trim().is_empty() {
        let g = layout(
            ctx,
            view.text.trim(),
            font.clone(),
            color,
            max_text_w,
            max_lines as usize,
        );
        rows_used = g.rows.len();
        blocks.push(Block {
            galley: g,
            gap_above: 0.0,
        });
    }
    if style.show_translation {
        if let Some(tr) = view
            .translated
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            let budget = translation_row_budget(max_lines, rows_used);
            if budget > 0 {
                let tfont = FontId::new(font.size * TRANSLATION_SCALE, caption_family());
                let g = layout(
                    ctx,
                    tr,
                    tfont,
                    color.gamma_multiply(TRANSLATION_SCALE),
                    max_text_w,
                    budget,
                );
                let gap = if blocks.is_empty() {
                    0.0
                } else {
                    TRANSLATION_GAP
                };
                blocks.push(Block {
                    galley: g,
                    gap_above: gap,
                });
            }
        }
    }
    if blocks.is_empty() {
        return None;
    }

    let content_w = blocks
        .iter()
        .map(|b| b.galley.rect.width())
        .fold(0.0f32, f32::max);
    let content_h: f32 = blocks
        .iter()
        .map(|b| b.gap_above + b.galley.rect.height())
        .sum();
    Some(Layout {
        box_size: vec2(content_w + 2.0 * PAD_X, content_h + 2.0 * PAD_Y),
        blocks,
    })
}

// ---- rendering -----------------------------------------------------------------

fn render(ui: &mut Ui, shared: &OverlayShared) {
    let ctx = ui.ctx().clone();
    let settings = shared.store.get();
    let view = shared.caption();
    let has_text = !view.text.trim().is_empty();

    // Windows draws a shadow/border/rounded frame around undecorated windows;
    // remove it (retried until the native window exists).
    if let Ok(mut rt) = shared.runtime.lock() {
        if !rt.chrome_stripped {
            rt.chrome_stripped = crate::platform::strip_overlay_chrome();
        }
    }

    // With no caption visible there is nothing to grab or click, so the window
    // must be click-through regardless of the user's preference; otherwise the
    // invisible window would still eat clicks over its whole area.
    let want_passthrough = !has_text || settings.click_through;
    if let Ok(mut rt) = shared.runtime.lock() {
        if rt.passthrough != Some(want_passthrough) {
            ctx.send_viewport_cmd(ViewportCommand::MousePassthrough(want_passthrough));
            rt.passthrough = Some(want_passthrough);
        }
    }

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let full = ui.max_rect();
            if has_text {
                paint_caption(ui, &ctx, &settings, &view, full, !want_passthrough);
            }
        });

    if let Ok(mut rt) = shared.runtime.lock() {
        crate::window_state::track(&ctx, &mut rt.tracker, &shared.window_file);
    }
}

fn paint_caption(
    ui: &mut Ui,
    ctx: &Context,
    settings: &unicaptions_core::Settings,
    view: &CaptionView,
    full: Rect,
    interactive: bool,
) {
    let style = &settings.caption_style;
    let Some(layout) = compute_layout(ctx, view, style, full.width()) else {
        return;
    };

    let box_rect = Rect::from_min_size(
        pos2(
            full.center().x - layout.box_size.x / 2.0,
            full.bottom() - OUTER_MARGIN - layout.box_size.y,
        ),
        layout.box_size,
    );

    let bg = with_alpha(&style.background_color, [0, 0, 0], style.background_opacity);
    ui.painter()
        .rect_filled(box_rect, egui::CornerRadius::same(CORNER_RADIUS), bg);

    let outline = Color32::from_rgba_unmultiplied(0, 0, 0, 150);
    let mut y = box_rect.top() + PAD_Y;
    for block in &layout.blocks {
        y += block.gap_above;
        let g = &block.galley;
        // Centre the galley's actual extent regardless of how its origin is defined.
        let pos = pos2(box_rect.center().x - g.rect.center().x, y - g.rect.top());
        if style.outline {
            for (dx, dy) in [(1.0, 1.0), (-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0)] {
                ui.painter().galley_with_override_text_color(
                    pos + vec2(dx, dy),
                    g.clone(),
                    outline,
                );
            }
        }
        ui.painter().galley(pos, g.clone(), Color32::WHITE);
        y += g.rect.height();
    }

    if interactive {
        let drag = ui.interact(
            box_rect,
            Id::unique("overlay-drag"),
            Sense::click_and_drag(),
        );
        if drag.hovered() || drag.dragged() {
            ctx.set_cursor_icon(CursorIcon::Grab);
        }
        if drag.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        for (i, (rect, dir, cursor)) in resize_hotspots(full).into_iter().enumerate() {
            let r = ui.interact(rect, Id::unique(("overlay-resize", i)), Sense::drag());
            if r.hovered() || r.dragged() {
                ctx.set_cursor_icon(cursor);
            }
            if r.drag_started_by(PointerButton::Primary) {
                ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::{apply_caption_font, FontCatalog};
    use unicaptions_core::settings::CaptionStyle;

    #[test]
    fn detects_cjk() {
        assert!(has_cjk("你好"));
        assert!(has_cjk("こんにちは"));
        assert!(has_cjk("한국어"));
        assert!(!has_cjk("Hello, world"));
        assert!(!has_cjk("Zażółć gęślą jaźń"));
    }

    #[test]
    fn translation_gets_the_remaining_line_budget() {
        assert_eq!(translation_row_budget(2, 1), 1);
        assert_eq!(translation_row_budget(2, 2), 0);
        assert_eq!(translation_row_budget(5, 1), 4);
        assert_eq!(translation_row_budget(0, 0), 1);
        assert_eq!(translation_row_budget(2, 9), 0);
    }

    #[test]
    fn hotspots_cover_the_edges_without_overlap() {
        let full = Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 160.0));
        let spots = resize_hotspots(full);
        assert_eq!(spots.len(), 8);
        for (i, (a, _, _)) in spots.iter().enumerate() {
            assert!(full.contains_rect(*a), "hotspot {i} outside window");
            for (j, (b, _, _)) in spots.iter().enumerate().skip(i + 1) {
                assert!(
                    !a.intersects(*b) || a.intersect(*b).area() < 0.01,
                    "{i} overlaps {j}"
                );
            }
        }
    }

    fn headless_ctx() -> Context {
        let ctx = Context::default();
        apply_caption_font(&ctx, &FontCatalog::empty(), "Inter", 600);
        ctx
    }

    fn layout_in_pass(view: &CaptionView, style: &CaptionStyle, width: f32) -> Option<egui::Vec2> {
        let ctx = headless_ctx();
        let mut out = None;
        crate::testutil::run_pass(&ctx, egui::RawInput::default(), |ui| {
            out = compute_layout(ui.ctx(), view, style, width).map(|l| l.box_size);
        });
        out
    }

    #[test]
    fn empty_caption_has_no_layout() {
        let style = CaptionStyle::default();
        assert!(layout_in_pass(&CaptionView::default(), &style, 900.0).is_none());
    }

    #[test]
    fn box_grows_with_translation_and_respects_toggles() {
        let style = CaptionStyle {
            max_lines: 5,
            ..CaptionStyle::default()
        };
        let only = CaptionView {
            text: "Hello there".into(),
            translated: None,
        };
        let both = CaptionView {
            text: "Hello there".into(),
            translated: Some("Hola".into()),
        };
        let a = layout_in_pass(&only, &style, 900.0).unwrap();
        let b = layout_in_pass(&both, &style, 900.0).unwrap();
        assert!(b.y > a.y, "translation should add height: {a:?} vs {b:?}");

        let hide_tr = CaptionStyle {
            show_translation: false,
            ..style.clone()
        };
        let c = layout_in_pass(&both, &hide_tr, 900.0).unwrap();
        assert!((c.y - a.y).abs() < 0.5);

        let hide_orig = CaptionStyle {
            show_original: false,
            ..style.clone()
        };
        let d = layout_in_pass(&both, &hide_orig, 900.0).unwrap();
        assert!(d.y < b.y);

        let none = CaptionStyle {
            show_original: false,
            show_translation: false,
            ..style
        };
        assert!(layout_in_pass(&both, &none, 900.0).is_none());
    }

    #[test]
    fn long_text_wraps_and_is_clamped_to_max_lines() {
        let long = "word ".repeat(200);
        let view = CaptionView {
            text: long,
            translated: None,
        };
        let one = CaptionStyle {
            max_lines: 1,
            ..CaptionStyle::default()
        };
        let three = CaptionStyle {
            max_lines: 3,
            ..CaptionStyle::default()
        };
        let h1 = layout_in_pass(&view, &one, 600.0).unwrap();
        let h3 = layout_in_pass(&view, &three, 600.0).unwrap();
        assert!(
            h3.y > h1.y * 2.0,
            "3 lines should be ~3x taller: {h1:?} {h3:?}"
        );
        assert!(
            h3.x <= 600.0 - 2.0 * OUTER_MARGIN + 1.0,
            "box must fit in the window: {h3:?}"
        );
    }

    #[test]
    fn cjk_text_without_spaces_still_wraps() {
        let view = CaptionView {
            text: "字幕".repeat(100),
            translated: None,
        };
        let style = CaptionStyle {
            max_lines: 2,
            ..CaptionStyle::default()
        };
        let size = layout_in_pass(&view, &style, 500.0).unwrap();
        assert!(size.x <= 500.0, "cjk line overflowed the window: {size:?}");
    }

    #[test]
    fn shared_caption_updates() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::open(dir.path().join("s.json"));
        let shared = OverlayShared::new(store, dir.path().join("w.json"));
        assert_eq!(shared.caption(), CaptionView::default());
        shared.set_caption(&CaptionEvent {
            text: "hi".into(),
            translated_text: Some("hola".into()),
            is_final: true,
            timestamp: 1,
        });
        assert_eq!(shared.caption().translated.as_deref(), Some("hola"));
        shared.set_caption(&CaptionEvent::clear(2));
        assert_eq!(shared.caption(), CaptionView::default());
    }

    #[test]
    fn render_runs_headless_with_and_without_text() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::open(dir.path().join("s.json"));
        let shared = OverlayShared::new(store, dir.path().join("w.json"));
        let ctx = headless_ctx();
        for text in ["", "A caption", "字幕テスト"] {
            shared.set_caption(&CaptionEvent {
                text: text.into(),
                translated_text: (!text.is_empty()).then(|| "translated".to_string()),
                is_final: true,
                timestamp: 0,
            });
            crate::testutil::run_pass(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 160.0))),
                    ..Default::default()
                },
                |ui| render(ui, &shared),
            );
        }
        // click-through is forced on while there is no text
        assert_eq!(shared.runtime.lock().unwrap().passthrough, Some(true));
    }
}
