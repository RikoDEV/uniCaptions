//! Debounced persistence of a window's position and size (replaces
//! `tauri-plugin-window-state`).

use eframe::egui::Context;
use std::path::Path;
use std::time::{Duration, Instant};
use unicaptions_core::settings::OverlayWindowState as WindowGeometry;

const DEBOUNCE: Duration = Duration::from_millis(600);

fn near(a: &WindowGeometry, b: &WindowGeometry) -> bool {
    (a.x - b.x).abs() < 0.5
        && (a.y - b.y).abs() < 0.5
        && (a.width - b.width).abs() < 0.5
        && (a.height - b.height).abs() < 0.5
}

/// What the caller should do after observing the current geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Poll {
    /// Nothing to do.
    Idle,
    /// Geometry changed but isn't stable yet; poll again after this long.
    Wait(Duration),
    /// Geometry has been stable long enough: write it, then call `saved`.
    Save(WindowGeometry),
}

/// Pure state machine: a change must stay unchanged for [`DEBOUNCE`] before it is saved.
#[derive(Default)]
pub struct Tracker {
    last_saved: Option<WindowGeometry>,
    pending: Option<(WindowGeometry, Instant)>,
}

impl Tracker {
    /// `known` is the geometry already on disk (or the initial placement), so
    /// that merely opening the window doesn't trigger a write.
    pub fn new(known: Option<WindowGeometry>) -> Self {
        Self {
            last_saved: known,
            pending: None,
        }
    }

    pub fn poll(&mut self, current: WindowGeometry, now: Instant) -> Poll {
        if self.last_saved.as_ref().is_some_and(|s| near(s, &current)) {
            self.pending = None;
            return Poll::Idle;
        }
        match self.pending {
            Some((p, since)) if near(&p, &current) => {
                let elapsed = now.saturating_duration_since(since);
                if elapsed >= DEBOUNCE {
                    self.pending = None;
                    Poll::Save(current)
                } else {
                    Poll::Wait(DEBOUNCE - elapsed + Duration::from_millis(20))
                }
            }
            _ => {
                self.pending = Some((current, now));
                Poll::Wait(DEBOUNCE + Duration::from_millis(50))
            }
        }
    }

    /// Record that `geometry` was written successfully.
    pub fn saved(&mut self, geometry: WindowGeometry) {
        self.last_saved = Some(geometry);
    }
}

/// Reads the current viewport's geometry and persists it when it settles.
pub fn track(ctx: &Context, tracker: &mut Tracker, file: &Path) {
    let (outer, inner) = ctx.input(|i| {
        let v = i.viewport();
        (v.outer_rect, v.inner_rect)
    });
    let (Some(outer), Some(inner)) = (outer, inner) else {
        return;
    };
    let current = WindowGeometry {
        x: outer.min.x,
        y: outer.min.y,
        width: inner.width(),
        height: inner.height(),
    };
    match tracker.poll(current, Instant::now()) {
        Poll::Idle => {}
        Poll::Wait(d) => ctx.request_repaint_after(d),
        Poll::Save(g) => {
            if g.save(file).is_ok() {
                tracker.saved(g);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(x: f32, w: f32) -> WindowGeometry {
        WindowGeometry {
            x,
            y: 10.0,
            width: w,
            height: 200.0,
        }
    }

    #[test]
    fn unchanged_geometry_never_saves() {
        let mut t = Tracker::new(Some(g(5.0, 900.0)));
        let now = Instant::now();
        assert_eq!(t.poll(g(5.0, 900.0), now), Poll::Idle);
        assert_eq!(
            t.poll(g(5.3, 900.2), now + Duration::from_secs(5)),
            Poll::Idle
        );
    }

    #[test]
    fn a_change_is_saved_only_after_it_settles() {
        let mut t = Tracker::new(Some(g(5.0, 900.0)));
        let t0 = Instant::now();
        assert!(matches!(t.poll(g(50.0, 900.0), t0), Poll::Wait(_)));
        assert!(matches!(
            t.poll(g(50.0, 900.0), t0 + Duration::from_millis(300)),
            Poll::Wait(_)
        ));
        let done = t.poll(g(50.0, 900.0), t0 + Duration::from_millis(700));
        assert_eq!(done, Poll::Save(g(50.0, 900.0)));
        t.saved(g(50.0, 900.0));
        assert_eq!(
            t.poll(g(50.0, 900.0), t0 + Duration::from_secs(2)),
            Poll::Idle
        );
    }

    #[test]
    fn continuous_dragging_keeps_resetting_the_timer() {
        let mut t = Tracker::new(None);
        let t0 = Instant::now();
        for i in 0..10u64 {
            let r = t.poll(
                g(i as f32 * 10.0, 900.0),
                t0 + Duration::from_millis(i * 200),
            );
            assert!(matches!(r, Poll::Wait(_)), "step {i}: {r:?}");
        }
    }

    #[test]
    fn failed_write_is_retried() {
        let mut t = Tracker::new(None);
        let t0 = Instant::now();
        let _ = t.poll(g(1.0, 900.0), t0);
        let Poll::Save(_) = t.poll(g(1.0, 900.0), t0 + Duration::from_secs(1)) else {
            panic!("expected save");
        };
        // caller did not call `saved()` (write failed): next stable poll starts over and saves again
        let _ = t.poll(g(1.0, 900.0), t0 + Duration::from_secs(2));
        assert!(matches!(
            t.poll(g(1.0, 900.0), t0 + Duration::from_secs(3)),
            Poll::Save(_)
        ));
    }
}
