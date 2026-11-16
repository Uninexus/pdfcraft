//! View ▸ Show Guides: the document's ruler guides drawn as thin lines over a page.
//!
//! A guide's position is in points on the displayed page (after `/Rotate`, before the view's own
//! rotation), measured from the left edge (vertical) or the top edge (horizontal), the space
//! [`PageXform::norm_to_screen`] takes as `position / page size`. Guides are drawn only: they take
//! no input, never reach a render for export or print (those do not go through the canvas), and
//! are not part of any hit test.

use egui::{Color32, Painter, Pos2, Stroke};
use pdfcraft_engine::guides::{Guide, GuideAxis};

use super::xform::PageXform;

/// The line colour: a cyan that reads on white paper and on dark pages alike.
pub const GUIDE_COLOR: Color32 = Color32::from_rgb(0, 160, 200);
/// Thin: one screen point.
const WIDTH: f32 = 1.0;

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

/// Draws the guides that belong to `page` (0-based) and lie on it.
pub(super) fn paint(painter: &Painter, xf: &PageXform, page: usize, guides: &[Guide]) {
    for guide in guides.iter().filter(|g| g.page == page) {
        if let Some(points) = line(xf, guide) {
            painter.line_segment(points, Stroke::new(WIDTH, GUIDE_COLOR));
        }
    }
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
    fn a_guide_that_cannot_be_placed_is_skipped() {
        assert!(line(&xf(0), &guide(GuideAxis::Vertical, f64::NAN)).is_none());
        let flat = PageXform { pw: 0.0, ..xf(0) };
        assert!(line(&flat, &guide(GuideAxis::Vertical, 1.0)).is_none());
    }
}
