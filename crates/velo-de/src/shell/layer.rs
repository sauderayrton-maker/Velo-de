//! Pure geometry for `wlr_layer_shell` surfaces: where a layer surface sits
//! on the output ([`arrange`]), and how much of the output its exclusive
//! zone reserves ([`shrink_by_exclusive_zone`]).

use smithay::utils::{Logical, Rectangle, Size};
use smithay::wayland::shell::wlr_layer::{Anchor, ExclusiveZone, Margins};

/// Compute a layer surface's on-screen geometry within `output`.
///
/// A `size` component of `0` means "fill along that axis" and is also
/// implied by being anchored to both edges on that axis.
pub fn arrange(output: Rectangle<i32, Logical>, anchor: Anchor, size: Size<i32, Logical>, margin: Margins) -> Rectangle<i32, Logical> {
    let width = if size.w == 0 || anchor.anchored_horizontally() {
        output.size.w - margin.left - margin.right
    } else {
        size.w
    };
    let height = if size.h == 0 || anchor.anchored_vertically() {
        output.size.h - margin.top - margin.bottom
    } else {
        size.h
    };

    let x = if anchor.contains(Anchor::LEFT) {
        output.loc.x + margin.left
    } else if anchor.contains(Anchor::RIGHT) {
        output.loc.x + output.size.w - width - margin.right
    } else {
        output.loc.x + (output.size.w - width) / 2
    };

    let y = if anchor.contains(Anchor::TOP) {
        output.loc.y + margin.top
    } else if anchor.contains(Anchor::BOTTOM) {
        output.loc.y + output.size.h - height - margin.bottom
    } else {
        output.loc.y + (output.size.h - height) / 2
    };

    Rectangle::new((x, y).into(), (width.max(0), height.max(0)).into())
}

/// The single edge an exclusive zone reserves space against, per the
/// `wlr_layer_shell` spec: a lone edge, or one edge plus both of its
/// perpendicular edges. Corners, parallel-edge pairs, all-edges and
/// no-anchor are not meaningful and return `None`.
fn exclusive_edge(anchor: Anchor) -> Option<Anchor> {
    let h = anchor.anchored_horizontally();
    let v = anchor.anchored_vertically();

    match (h, v) {
        (true, true) => None,
        (true, false) => match (anchor.contains(Anchor::TOP), anchor.contains(Anchor::BOTTOM)) {
            (true, false) => Some(Anchor::TOP),
            (false, true) => Some(Anchor::BOTTOM),
            _ => None,
        },
        (false, true) => match (anchor.contains(Anchor::LEFT), anchor.contains(Anchor::RIGHT)) {
            (true, false) => Some(Anchor::LEFT),
            (false, true) => Some(Anchor::RIGHT),
            _ => None,
        },
        (false, false) => {
            let edges: Vec<Anchor> = [Anchor::TOP, Anchor::BOTTOM, Anchor::LEFT, Anchor::RIGHT].into_iter().filter(|&e| anchor.contains(e)).collect();
            if edges.len() == 1 {
                Some(edges[0])
            } else {
                None
            }
        }
    }
}

/// Shrink `area` by `zone`'s exclusive pixels from whichever edge `anchor`
/// reserves against (see [`exclusive_edge`]). A non-[`ExclusiveZone::Exclusive`]
/// zone, or an anchor with no single reserved edge, leaves `area` unchanged.
pub fn shrink_by_exclusive_zone(area: Rectangle<i32, Logical>, anchor: Anchor, zone: ExclusiveZone) -> Rectangle<i32, Logical> {
    let ExclusiveZone::Exclusive(zone) = zone else { return area };
    let zone = zone as i32;
    let mut area = area;

    if let Some(edge) = exclusive_edge(anchor) {
        if edge == Anchor::TOP {
            area.loc.y += zone;
            area.size.h -= zone;
        } else if edge == Anchor::BOTTOM {
            area.size.h -= zone;
        } else if edge == Anchor::LEFT {
            area.loc.x += zone;
            area.size.w -= zone;
        } else if edge == Anchor::RIGHT {
            area.size.w -= zone;
        }
    }

    area
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output() -> Rectangle<i32, Logical> {
        Rectangle::new((0, 0).into(), (1920, 1080).into())
    }

    #[test]
    fn top_bar_anchored_left_right_top_fills_width() {
        let rect = arrange(output(), Anchor::TOP | Anchor::LEFT | Anchor::RIGHT, Size::from((0, 32)), Margins::default());
        assert_eq!(rect, Rectangle::new((0, 0).into(), (1920, 32).into()));
    }

    #[test]
    fn bottom_center_popup_uses_its_own_size_and_margin() {
        let margin = Margins { bottom: 20, ..Default::default() };
        let rect = arrange(output(), Anchor::BOTTOM, Size::from((480, 300)), margin);
        assert_eq!(rect, Rectangle::new((720, 760).into(), (480, 300).into()));
    }

    #[test]
    fn unanchored_zero_size_fills_and_centers() {
        let rect = arrange(output(), Anchor::empty(), Size::from((0, 0)), Margins::default());
        assert_eq!(rect, output());
    }

    #[test]
    fn shrink_top_bar_reserves_top_strip() {
        let usable = shrink_by_exclusive_zone(output(), Anchor::TOP | Anchor::LEFT | Anchor::RIGHT, ExclusiveZone::Exclusive(32));
        assert_eq!(usable, Rectangle::new((0, 32).into(), (1920, 1080 - 32).into()));
    }

    #[test]
    fn shrink_left_sidebar_with_full_height_reserves_left_strip() {
        let usable = shrink_by_exclusive_zone(output(), Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM, ExclusiveZone::Exclusive(50));
        assert_eq!(usable, Rectangle::new((50, 0).into(), (1920 - 50, 1080).into()));
    }

    #[test]
    fn shrink_neutral_zone_has_no_effect() {
        let usable = shrink_by_exclusive_zone(output(), Anchor::BOTTOM, ExclusiveZone::Neutral);
        assert_eq!(usable, output());
    }

    #[test]
    fn shrink_corner_anchor_has_no_effect() {
        let usable = shrink_by_exclusive_zone(output(), Anchor::TOP | Anchor::LEFT, ExclusiveZone::Exclusive(50));
        assert_eq!(usable, output());
    }

    #[test]
    fn shrink_all_edges_has_no_effect() {
        let anchor = Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT;
        let usable = shrink_by_exclusive_zone(output(), anchor, ExclusiveZone::Exclusive(50));
        assert_eq!(usable, output());
    }
}
