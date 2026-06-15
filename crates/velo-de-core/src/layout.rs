//! Pure geometry: turning a [`Strip`](crate::model::Strip) of columns into
//! window rectangles.

use crate::geometry::{Rect, Size};
use crate::model::{ColumnLayout, Strip, WindowId};

/// The placement of a single window within a Space's local
/// `viewport`-sized coordinate space (origin top-left, `(0,0)..(viewport.w,
/// viewport.h)`). The compositor maps this into output coordinates by
/// scaling/translating according to the owning [`crate::model::SpaceFrame::rect`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowLayout {
    pub id: WindowId,
    pub rect: Rect,
    /// `false` for the non-active windows of a [`ColumnLayout::Tabbed`]
    /// column - still positioned (so they're pre-sized when raised) but
    /// should not be drawn or receive input.
    pub visible: bool,
}

/// Lay out every window in `strip` within a `viewport`-sized area, leaving
/// `gap` logical pixels between/around tiles.
pub fn compute_strip_layout(strip: &Strip, viewport: Size, gap: f64) -> Vec<WindowLayout> {
    let mut out = Vec::new();
    let content_h = (viewport.h - 2.0 * gap).max(0.0);
    let mut x_cursor = gap - strip.scroll.value();

    for column in &strip.columns {
        let col_w = (viewport.w * column.width_frac).max(1.0);
        let col_rect = Rect::new(x_cursor, gap, col_w, content_h);

        match column.layout {
            ColumnLayout::Tabbed => {
                for (i, &id) in column.windows.iter().enumerate() {
                    out.push(WindowLayout { id, rect: col_rect, visible: i == column.active });
                }
            }
            ColumnLayout::Split => {
                let n = column.windows.len().max(1) as f64;
                let win_h = ((content_h - gap * (n - 1.0)) / n).max(1.0);
                for (i, &id) in column.windows.iter().enumerate() {
                    let y = gap + (i as f64) * (win_h + gap);
                    out.push(WindowLayout { id, rect: Rect::new(x_cursor, y, col_w, win_h), visible: true });
                }
            }
        }

        x_cursor += col_w + gap;
    }

    out
}

/// Map a window's rect from a Space's `viewport`-sized local coordinate
/// space into output-relative screen pixels, given that Space's current
/// on-screen placement `frame_rect` (a [`crate::model::SpaceFrame::rect`]).
///
/// Degenerates to the identity transform when `frame_rect` exactly covers
/// `viewport` (the common, non-animated single-Space case).
pub fn place_window(frame_rect: Rect, viewport: Size, w: Rect) -> Rect {
    let sx = frame_rect.w / viewport.w;
    let sy = frame_rect.h / viewport.h;
    Rect::new(frame_rect.x + w.x * sx, frame_rect.y + w.y * sy, w.w * sx, w.h * sy)
}

#[cfg(test)]
mod place_window_tests {
    use super::*;

    #[test]
    fn identity_when_frame_covers_viewport() {
        let viewport = Size::new(1000.0, 800.0);
        let frame_rect = Rect::new(0.0, 0.0, 1000.0, 800.0);
        let w = Rect::new(10.0, 10.0, 480.0, 780.0);
        assert_eq!(place_window(frame_rect, viewport, w), w);
    }

    #[test]
    fn scales_and_translates_for_overview_tile() {
        let viewport = Size::new(1000.0, 800.0);
        // an overview tile at half scale, offset by (100, 50)
        let frame_rect = Rect::new(100.0, 50.0, 500.0, 400.0);
        let w = Rect::new(10.0, 10.0, 480.0, 780.0);
        let placed = place_window(frame_rect, viewport, w);
        assert_eq!(placed, Rect::new(100.0 + 5.0, 50.0 + 5.0, 240.0, 390.0));
    }
}
