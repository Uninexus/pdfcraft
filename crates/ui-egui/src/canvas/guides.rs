//! View > Guides > Show Guides: the document's ruler guides, laid out over a page.
//!
//! A guide's position is in points on the displayed page (after `/Rotate`, before the view's own
//! rotation), measured from the left edge (vertical) or the top edge (horizontal), the space
//! [`PageXform::norm_to_screen`] takes as `position / page size`. Guides are drawn only (by
//! `PageOverlays`, in a theme colour): they take no input, never reach a render for export or print
//! (those do not go through the canvas), and are not part of any hit test. With the rulers shown
//! (View > Show Rulers) each guide is also a page part, a thin band along its line a press can drag
//! ([`parts`], [`drag_position`]); the app moves the guide ([`GuideMove`]) and drops it when the drag
//! ends over a ruler (`rulers`).

use craft::behavior::{CursorIcon, HitShape, PressPhase};
use egui::{Pos2, Rect};
use pdfcraft_engine::guides::{Guide, GuideAxis};

use super::page_parts::{Owner, PageLayer, PagePart};
use super::xform::PageXform;

/// Where a drag has taken a guide: the guide's id and the position on its page it asks for, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct GuideMove {
    /// The guide's id.
    pub id: u64,
    /// The position along its axis, as the engine's `move_guide` takes it.
    pub position: f64,
}

/// The parts for the guides of `page` on the page `xf` places: a band `reach` screen points either side of each
/// line, which a press can take and drag. A guide past the page's edge has no line and so no part.
pub(super) fn parts(xf: &PageXform, page: usize, guides: &[Guide], reach: f32) -> Vec<PagePart> {
    guides
        .iter()
        .filter(|g| g.page == page)
        .filter_map(|guide| {
            let [a, b] = line(xf, guide)?;
            let owner = Owner::Guide { id: usize::try_from(guide.id).ok()? };
            let across = (a.y - b.y).abs() < (a.x - b.x).abs();
            let cursor = if across { CursorIcon::ResizeVertical } else { CursorIcon::ResizeHorizontal };
            let band = Rect::from_two_pos(a, b).expand(reach.max(1.0));
            Some(PagePart::new(owner, page, HitShape::Rect(band), PageLayer::Placed).cursor(cursor))
        })
        .collect()
}

/// The position on its page that the pointer in `phase` gives `guide`, while the press drags it; `None` for a phase
/// that is no drag.
pub(super) fn drag_position(xf: &PageXform, guide: &Guide, phase: &PressPhase) -> Option<f64> {
    let at = match phase {
        PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } | PressPhase::Release { pos, click: false, .. } => *pos,
        _ => return None,
    };
    Some(position_at(xf, guide.axis, at))
}

/// The position along `axis` of the screen point `at` on the page `xf` places, held to the page.
pub(super) fn position_at(xf: &PageXform, axis: GuideAxis, at: Pos2) -> f64 {
    let (x, y) = xf.screen_to_view(at);
    let (along, limit) = if axis == GuideAxis::Vertical { (x, xf.pw) } else { (y, xf.ph) };
    // A guide stays on its page: a pointer past the edge holds it at the edge.
    f64::from(along).clamp(0.0, f64::from(limit))
}

/// The guide a drag from a ruler makes when dropped at `at` on the page `xf` places: the axis it has in the page's
/// frame and its position. `from_top` is a drag from the top ruler, whose guide runs across the screen; with the view
/// turned a quarter, a line across the screen is a vertical guide of the page.
pub(super) fn dropped(xf: &PageXform, from_top: bool, at: Pos2) -> (GuideAxis, f64) {
    let across = if from_top { GuideAxis::Horizontal } else { GuideAxis::Vertical };
    let axis = match (xf.rot % 180 == 90, across) {
        (false, axis) => axis,
        (true, GuideAxis::Horizontal) => GuideAxis::Vertical,
        (true, GuideAxis::Vertical) => GuideAxis::Horizontal,
    };
    (axis, position_at(xf, axis, at))
}

/// Where `guide` runs on screen, as its two end points, or `None` when its position can't be
/// placed (not finite) or the page has no size.
pub(super) fn line(xf: &PageXform, guide: &Guide) -> Option<[Pos2; 2]> {
    if !guide.position.is_finite() || xf.pw <= 0.0 || xf.ph <= 0.0 {
        return None;
    }
    let position = guide.position as f32;
    let (a, b) = match guide.axis {
        GuideAxis::Vertical => {
            let u = position / xf.pw;
            ((u, 0.0), (u, 1.0))
        }
        GuideAxis::Horizontal => {
            let v = position / xf.ph;
            ((0.0, v), (1.0, v))
        }
    };
    // A guide can outlive a crop (it keeps its position when the page shrinks): one now past the
    // page's edge is not drawn, or it would show over the page beside this one.
    let along = if guide.axis == GuideAxis::Vertical { a.0 } else { a.1 };
    if !along.is_finite() || !(0.0..=1.0).contains(&along) {
        return None;
    }
    Some([xf.norm_to_screen(a.0, a.1), xf.norm_to_screen(b.0, b.1)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Rect, pos2};

    fn guide(axis: GuideAxis, position: f64) -> Guide {
        Guide { id: 1, page: 0, axis, position }
    }

    fn xf(rot: u16) -> PageXform {
        // A 200 x 300 pt page shown in a 100 x 150 screen rect (rotation swaps the sides).
        let rect = if rot % 180 == 90 {
            Rect::from_min_size(pos2(10.0, 20.0), egui::vec2(150.0, 100.0))
        } else {
            Rect::from_min_size(pos2(10.0, 20.0), egui::vec2(100.0, 150.0))
        };
        PageXform { rect, rot, pw: 200.0, ph: 300.0 }
    }

    #[test]
    fn a_vertical_guide_runs_down_the_page_at_its_distance_from_the_left_edge() {
        let [top, bottom] = line(&xf(0), &guide(GuideAxis::Vertical, 50.0)).unwrap();
        assert_eq!((top, bottom), (pos2(35.0, 20.0), pos2(35.0, 170.0)));
    }

    #[test]
    fn a_horizontal_guide_runs_across_the_page_at_its_distance_from_the_top_edge() {
        let [left, right] = line(&xf(0), &guide(GuideAxis::Horizontal, 150.0)).unwrap();
        assert_eq!((left, right), (pos2(10.0, 95.0), pos2(110.0, 95.0)));
    }

    #[test]
    fn guides_follow_the_views_rotation() {
        // Turned a quarter, the page's left edge is its top on screen: a vertical guide runs across.
        let [a, b] = line(&xf(90), &guide(GuideAxis::Vertical, 50.0)).unwrap();
        assert!((a.y - b.y).abs() < 1e-3 && (a.x - b.x).abs() > 1.0, "{a:?} {b:?}");
        assert!(a.y > 20.0 && a.y < 120.0, "inside the page rect: {a:?}");
    }

    #[test]
    fn a_guide_past_the_pages_edge_is_not_drawn() {
        // The page is 200 x 300 pt: as after a crop that made it smaller than the guide's position.
        assert!(line(&xf(0), &guide(GuideAxis::Vertical, 200.5)).is_none());
        assert!(line(&xf(0), &guide(GuideAxis::Horizontal, 300.5)).is_none());
        assert!(line(&xf(0), &guide(GuideAxis::Vertical, -1.0)).is_none());
        assert!(line(&xf(0), &guide(GuideAxis::Vertical, 200.0)).is_some(), "on the edge still draws");
        assert!(line(&xf(0), &guide(GuideAxis::Horizontal, 0.0)).is_some());
    }

    #[test]
    fn a_dragged_guide_follows_the_pointer_along_its_axis_only() {
        let g = guide(GuideAxis::Horizontal, 10.0);
        assert_eq!(position_at(&xf(0), g.axis, pos2(60.0, 95.0)), 150.0, "95 on screen is 150 pt down the page");
        assert_eq!(position_at(&xf(0), GuideAxis::Vertical, pos2(60.0, 95.0)), 100.0);
        assert_eq!(drag_position(&xf(0), &g, &PressPhase::Hover), None);
    }

    #[test]
    fn a_guide_dropped_from_the_top_ruler_runs_across_the_page_whatever_the_view_rotation() {
        let at = pos2(60.0, 95.0);
        assert_eq!(dropped(&xf(0), true, at).0, GuideAxis::Horizontal);
        assert_eq!(dropped(&xf(90), true, at).0, GuideAxis::Vertical, "turned a quarter, a line across the screen is the page's vertical");
        assert_eq!(dropped(&xf(0), false, at).0, GuideAxis::Vertical);
    }

    #[test]
    fn a_guide_that_cannot_be_placed_is_skipped() {
        assert!(line(&xf(0), &guide(GuideAxis::Vertical, f64::NAN)).is_none());
        let flat = PageXform { pw: 0.0, ..xf(0) };
        assert!(line(&flat, &guide(GuideAxis::Vertical, 1.0)).is_none());
    }
}
