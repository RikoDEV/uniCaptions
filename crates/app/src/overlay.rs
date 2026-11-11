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
    workspaces_done: bool,
    tracker: crate::window_state::Tracker,
}

/// The visible caption plus the bookkeeping that keeps late events from
/// showing stale text.
#[derive(Default)]
struct CaptionState {
    view: CaptionView,
    /// Timestamp of the latest "clear" event.
    cleared_at: u128,
    /// Timestamp of the original text the shown translation belongs to.
    translation_at: u128,
}

impl CaptionState {
    fn apply(&mut self, event: &CaptionEvent) {
        if event.text.is_empty() {
            self.view = CaptionView::default();
            self.cleared_at = event.timestamp;
            self.translation_at = 0;
            return;
        }
        match &event.translated_text {
            // A new original caption. Keep the current translation (same
            // utterance) so it does not blink off between updates.
            None => self.view.text = event.text.clone(),
            // A translation of an earlier snippet: only ever move forward, and
            // never resurrect text from before the last clear. It does not
            // touch the original text.
            Some(tr) => {
                if event.timestamp > self.cleared_at && event.timestamp >= self.translation_at {
                    self.view.translated = Some(tr.clone());
                    self.translation_at = event.timestamp;
                }
            }
        }
    }
}

/// State shared between the main window (which receives engine events) and the
/// overlay viewport (which paints them).
pub struct OverlayShared {
    caption: Mutex<CaptionState>,
    store: Arc<SettingsStore>,
    window_file: PathBuf,
    runtime: Mutex<Runtime>,
}

impl OverlayShared {
    pub fn new(store: Arc<SettingsStore>, window_file: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            caption: Mutex::new(CaptionState::default()),
            store,
            window_file,
            runtime: Mutex::new(Runtime {
                passthrough: None,
                tracker_unset: true,
                chrome_stripped: false,
                workspaces_done: false,
                tracker: crate::window_state::Tracker::default(),
            }),
        })
    }

    /// Applies a caption event. Translation results arrive later and out of band,
    /// so they must never replace newer original text, and must never bring back
    /// a caption that has since been cleared.
    pub fn set_caption(&self, event: &CaptionEvent) {
        if let Ok(mut st) = self.caption.lock() {
            st.apply(event);
        }
    }

    pub fn caption(&self) -> CaptionView {
        self.caption
            .lock()
            .map(|c| c.view.clone())
            .unwrap_or_default()
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
    job.halign = Align::LEFT;
    ctx.fonts_mut(|f| f.layout_job(job))
}

/// Lays `text` out and keeps only its last `max_rows` rows, so newly added words
/// push older lines up and off the top (live-caption style) instead of the
/// newest words being cut off.
fn layout_tail(
    ctx: &Context,
    text: &str,
    font: FontId,
    color: Color32,
    width: f32,
    max_rows: usize,
) -> Arc<Galley> {
    let max_rows = max_rows.max(1);
    let full = layout(ctx, text, font.clone(), color, width, usize::MAX);
    if full.rows.len() <= max_rows {
        return full;
    }
    let skip_rows = full.rows.len() - max_rows;
    let skip_chars: usize = full.rows[..skip_rows]
        .iter()
        .map(|r| r.char_count_including_newline().0)
        .sum();
    let tail: String = text.chars().skip(skip_chars).collect();
    layout(ctx, tail.trim_start(), font, color, width, max_rows)
}

/// One laid-out block (original or translation) at a fixed offset inside the box.
struct Block {
    galley: Arc<Galley>,
    /// Distance from the top of the text area (inside the padding).
    y: f32,
}

/// Height of one text row for `font`.
fn row_height(ctx: &Context, font: FontId) -> f32 {
    let two_rows = ctx.fonts_mut(|f| {
        f.layout_no_wrap("Ag\nAg".to_string(), font, Color32::WHITE)
            .size()
            .y
    });
    two_rows / 2.0
}

/// What would be drawn, computed without painting (testable).
pub struct Layout {
    blocks: Vec<Block>,
    pub box_size: egui::Vec2,
}

/// Applies the "uppercase" caption option.
fn apply_case(text: &str, uppercase: bool) -> String {
    if uppercase {
        text.to_uppercase()
    } else {
        text.to_string()
    }
}

/// Lays the caption out in a box whose size never depends on the text: it is as
/// wide as the window allows and as tall as `max_lines` rows (plus the same for
/// the translation when it is, or is about to be, shown). Text is left-aligned
/// and starts at a fixed spot, so new words only extend the line instead of
/// resizing and re-centring the box, which is what made captions "jump".
fn compute_layout(
    ctx: &Context,
    view: &CaptionView,
    style: &unicaptions_core::settings::CaptionStyle,
    window_width: f32,
    reserve_translation: bool,
) -> Option<Layout> {
    let max_text_w = (window_width - 2.0 * OUTER_MARGIN - 2.0 * PAD_X).max(60.0);
    let font = FontId::new(style.font_size.max(6.0), caption_family());
    let color = with_alpha(&style.text_color, [255, 255, 255], 1.0);
    let max_lines = style.max_lines.max(1);

    let tfont = FontId::new(font.size * TRANSLATION_SCALE, caption_family());
    let original = Some(view.text.trim())
        .filter(|t| style.show_original && !t.is_empty())
        .map(|t| apply_case(t, style.uppercase));
    let translation = view
        .translated
        .as_deref()
        .map(str::trim)
        .filter(|t| style.show_translation && !t.is_empty())
        .map(|t| apply_case(t, style.uppercase));
    if original.is_none() && translation.is_none() {
        return None;
    }

    // Space is reserved for each part that is (or will be) shown, whether or not
    // it has text yet, so nothing shifts when the translation arrives.
    let show_original_area = style.show_original;
    let show_translation_area =
        style.show_translation && (translation.is_some() || reserve_translation);
    let original_h = row_height(ctx, font.clone()) * max_lines as f32;
    let translation_h = row_height(ctx, tfont.clone()) * max_lines as f32;

    let mut blocks: Vec<Block> = Vec::new();
    let mut y = 0.0f32;
    if show_original_area {
        if let Some(text) = &original {
            let g = layout_tail(
                ctx,
                text,
                font.clone(),
                color,
                max_text_w,
                max_lines as usize,
            );
            blocks.push(Block { galley: g, y });
        }
        y += original_h;
    }
    if show_translation_area {
        if show_original_area {
            y += TRANSLATION_GAP;
        }
        if let Some(text) = &translation {
            let g = layout_tail(
                ctx,
                text,
                tfont,
                color.gamma_multiply(TRANSLATION_SCALE),
                max_text_w,
                max_lines as usize,
            );
            blocks.push(Block { galley: g, y });
        }
        y += translation_h;
    }

    Some(Layout {
        box_size: vec2(max_text_w + 2.0 * PAD_X, y + 2.0 * PAD_Y),
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
        if !rt.workspaces_done {
            rt.workspaces_done = crate::platform::follow_workspaces();
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

/// The background bar behind each non-empty text row of a galley drawn at `pos`:
/// as wide as the row's text plus padding, so the background hugs the words.
fn line_bars(galley: &Galley, pos: egui::Pos2) -> Vec<Rect> {
    let rows: Vec<Rect> = galley
        .rows
        .iter()
        .map(|r| r.rect())
        .filter(|r| r.width() > 0.5)
        .collect();
    let last = rows.len().saturating_sub(1);
    rows.iter()
        .enumerate()
        .map(|(i, r)| {
            // Rows touch (no overlap, so a translucent background is not doubled);
            // only the first and last get extra room above and below.
            let top = if i == 0 { PAD_Y / 2.0 } else { 0.0 };
            let bottom = if i == last { PAD_Y / 2.0 } else { 0.0 };
            Rect::from_min_max(
                pos + r.min.to_vec2() - vec2(PAD_X, top),
                pos + r.max.to_vec2() + vec2(PAD_X, bottom),
            )
        })
        .collect()
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
    let Some(layout) = compute_layout(ctx, view, style, full.width(), settings.translation_enabled)
    else {
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

    let outline = Color32::from_rgba_unmultiplied(0, 0, 0, 150);
    for block in &layout.blocks {
        let g = &block.galley;
        // Left-aligned at a fixed spot, whatever the galley's origin convention.
        let pos = pos2(
            box_rect.left() + PAD_X - g.rect.left(),
            box_rect.top() + PAD_Y + block.y - g.rect.top(),
        );
        // The background follows the words, YouTube-style: one bar per line that
        // is exactly as wide as that line. Its left edge and position are fixed,
        // so it only grows to the right as words arrive.
        for bar in line_bars(g, pos) {
            ui.painter()
                .rect_filled(bar, egui::CornerRadius::same(CORNER_RADIUS), bg);
        }
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
            out = compute_layout(ui.ctx(), view, style, width, false).map(|l| l.box_size);
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
    fn apply_case_uppercases_polish_letters() {
        assert_eq!(apply_case("zażółć gęślą jaźń", true), "ZAŻÓŁĆ GĘŚLĄ JAŹŃ");
        assert_eq!(apply_case("zażółć", false), "zażółć");
        assert_eq!(apply_case("", true), "");
    }

    fn shown_text(view: &CaptionView, style: &CaptionStyle) -> Vec<String> {
        let ctx = headless_ctx();
        let mut out = Vec::new();
        crate::testutil::run_pass(&ctx, egui::RawInput::default(), |ui| {
            let l = compute_layout(ui.ctx(), view, style, 900.0, false).unwrap();
            out = l
                .blocks
                .iter()
                .map(|b| b.galley.text().to_string())
                .collect();
        });
        out
    }

    #[test]
    fn uppercase_applies_to_the_original_and_the_translation() {
        let view = CaptionView {
            text: "dzień dobry".into(),
            translated: Some("good morning".into()),
        };
        let off = shown_text(&view, &CaptionStyle::default());
        assert_eq!(off, ["dzień dobry", "good morning"]);
        let on = shown_text(
            &view,
            &CaptionStyle {
                uppercase: true,
                ..CaptionStyle::default()
            },
        );
        assert_eq!(on, ["DZIEŃ DOBRY", "GOOD MORNING"]);
    }

    #[test]
    fn the_box_never_changes_size_as_the_text_grows() {
        let style = CaptionStyle {
            max_lines: 2,
            ..CaptionStyle::default()
        };
        let sizes: Vec<_> = ["Hi", "Hello there my friend", &"word ".repeat(80)]
            .iter()
            .map(|t| {
                let view = CaptionView {
                    text: (*t).into(),
                    translated: None,
                };
                layout_in_pass(&view, &style, 900.0).unwrap()
            })
            .collect();
        assert_eq!(sizes[0], sizes[1], "{sizes:?}");
        assert_eq!(sizes[1], sizes[2], "{sizes:?}");
    }

    #[test]
    fn space_for_the_translation_is_reserved_before_it_arrives() {
        let ctx = headless_ctx();
        let style = CaptionStyle::default();
        let size = |translated: Option<&str>, reserve: bool| {
            let view = CaptionView {
                text: "Dzień dobry".into(),
                translated: translated.map(str::to_string),
            };
            let mut out = None;
            crate::testutil::run_pass(&ctx, egui::RawInput::default(), |ui| {
                out = compute_layout(ui.ctx(), &view, &style, 900.0, reserve).map(|l| l.box_size);
            });
            out.unwrap()
        };
        let before = size(None, true); // translation is on, not here yet
        let after = size(Some("Good morning"), true);
        assert_eq!(
            before, after,
            "the box must not grow when the translation arrives"
        );
        assert!(
            before.y > size(None, false).y,
            "reserved space must be taller than none"
        );
    }

    #[test]
    fn text_is_left_aligned_not_centred() {
        let ctx = headless_ctx();
        let style = CaptionStyle::default();
        let view = CaptionView {
            text: "Short".into(),
            translated: None,
        };
        let mut left = f32::MAX;
        let mut width = 0.0;
        crate::testutil::run_pass(&ctx, egui::RawInput::default(), |ui| {
            let l = compute_layout(ui.ctx(), &view, &style, 900.0, false).unwrap();
            left = l.blocks[0].galley.rect.left();
            width = l.blocks[0].galley.rect.width();
        });
        assert!(
            left.abs() < 0.5,
            "text must start at the left edge, started at {left}"
        );
        assert!(
            width < 300.0,
            "a short word must not be stretched across the box: {width}"
        );
    }

    #[test]
    fn the_background_hugs_each_line_and_keeps_its_left_edge() {
        let ctx = headless_ctx();
        let style = CaptionStyle::default();
        let bars = |text: &str| {
            let view = CaptionView {
                text: text.into(),
                translated: None,
            };
            let mut out = Vec::new();
            crate::testutil::run_pass(&ctx, egui::RawInput::default(), |ui| {
                let l = compute_layout(ui.ctx(), &view, &style, 900.0, false).unwrap();
                let g = &l.blocks[0].galley;
                out = line_bars(g, pos2(PAD_X - g.rect.left(), PAD_Y - g.rect.top()));
            });
            out
        };
        let short = bars("Hi");
        let longer = bars("Hi there, this is a longer caption");
        assert_eq!(short.len(), 1);
        assert!(
            longer[0].width() > short[0].width() + 50.0,
            "bar must follow the words"
        );
        assert!(
            (short[0].left() - longer[0].left()).abs() < 0.01,
            "left edge must not move"
        );
        assert!(
            (short[0].top() - longer[0].top()).abs() < 0.01,
            "top must not move"
        );
        // two lines: bars touch, never overlap
        let two = bars(
            "First line.
Second, much longer line of text",
        );
        assert_eq!(two.len(), 2);
        assert!((two[0].bottom() - two[1].top()).abs() < 0.5 + PAD_Y / 2.0);
        assert!(two[1].width() > two[0].width());
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
        shared.set_caption(&original("hi", 1));
        shared.set_caption(&translation("hi", "hola", 1));
        assert_eq!(shared.caption().text, "hi");
        assert_eq!(shared.caption().translated.as_deref(), Some("hola"));
        shared.set_caption(&CaptionEvent::clear(2));
        assert_eq!(shared.caption(), CaptionView::default());
    }

    fn original(text: &str, ts: u128) -> CaptionEvent {
        CaptionEvent {
            text: text.into(),
            translated_text: None,
            is_final: true,
            timestamp: ts,
        }
    }

    fn translation(text: &str, tr: &str, ts: u128) -> CaptionEvent {
        CaptionEvent {
            text: text.into(),
            translated_text: Some(tr.into()),
            is_final: true,
            timestamp: ts,
        }
    }

    fn state_with() -> CaptionState {
        CaptionState::default()
    }

    #[test]
    fn a_late_translation_never_replaces_newer_original_text() {
        let mut st = state_with();
        st.apply(&original("hello", 10));
        st.apply(&original("hello there my friend", 20));
        // translation of the *older* snippet arrives afterwards
        st.apply(&translation("hello", "hola", 10));
        assert_eq!(st.view.text, "hello there my friend");
        assert_eq!(st.view.translated.as_deref(), Some("hola"));
    }

    #[test]
    fn a_translation_from_before_a_clear_does_not_resurrect_the_caption() {
        let mut st = state_with();
        st.apply(&original("old sentence", 10));
        st.apply(&CaptionEvent::clear(30));
        st.apply(&translation("old sentence", "vieja frase", 10));
        assert_eq!(st.view, CaptionView::default());
        // and a translation for the next utterance still works
        st.apply(&original("new one", 40));
        st.apply(&translation("new one", "nueva", 40));
        assert_eq!(st.view.translated.as_deref(), Some("nueva"));
    }

    #[test]
    fn translations_only_move_forward() {
        let mut st = state_with();
        st.apply(&original("a b c", 50));
        st.apply(&translation("a b c", "newer", 50));
        st.apply(&translation("a b", "older", 40));
        assert_eq!(st.view.translated.as_deref(), Some("newer"));
    }

    #[test]
    fn translation_survives_original_updates_within_one_utterance() {
        let mut st = state_with();
        st.apply(&original("one", 1));
        st.apply(&translation("one", "uno", 1));
        st.apply(&original("one two", 2));
        assert_eq!(
            st.view.translated.as_deref(),
            Some("uno"),
            "must not blink off"
        );
    }

    #[test]
    fn new_words_push_old_lines_up_and_the_latest_words_stay_visible() {
        let ctx = headless_ctx();
        let style = CaptionStyle {
            max_lines: 2,
            ..CaptionStyle::default()
        };
        let text = (1..=60)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let view = CaptionView {
            text,
            translated: None,
        };
        let mut shown = String::new();
        crate::testutil::run_pass(&ctx, egui::RawInput::default(), |ui| {
            let l = compute_layout(ui.ctx(), &view, &style, 700.0, false).unwrap();
            shown = l.blocks[0].galley.text().to_string();
            assert_eq!(l.blocks[0].galley.rows.len(), 2);
        });
        assert!(
            shown.contains("word60"),
            "newest words must be visible: {shown}"
        );
        assert!(
            !shown.contains("word1 "),
            "oldest words must scroll off: {shown}"
        );
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
