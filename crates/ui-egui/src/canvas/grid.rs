//! View > Show Grid: the page grid, laid out over a page.
//!
//! The arithmetic is the engine's [`Grid`]: a line every `spacing / subdivisions` points, one of each axis through the
//! offset, vertical lines measured from the displayed page's left edge and horizontal ones from its top edge (the guides'
//! frame, after `/Rotate`). A line past the page is not drawn. The grid is painted only (by `PageOverlays`, in theme
//! colours): it takes no input and never reaches a render for export or print.

use egui::Pos2;
use pdfcraft_engine::guides::Grid;

use super::xform::PageXform;

/// The closest two drawn lines may be on screen, in points. A grid finer than this at the current zoom draws its main
/// lines only, and none at all when even those are closer.
pub(super) const MIN_SCREEN_PITCH: f32 = 5.0;

/// The most lines one axis of one page draws, whatever the zoom.
const MAX_LINES_PER_AXIS: usize = 2_000;

/// One grid line on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct GridLine {
    /// Its two end points on screen.
    pub points: [Pos2; 2],
    /// A main line (a multiple of the spacing from the offset); the others are subdivisions.
    pub major: bool,
}

/// The positions on one axis, in points from the page's edge, of the lines to draw, each with whether it is a main line.
fn axis_positions(limit: f64, offset: f64, grid: &Grid, majors_only: bool) -> Vec<(f64, bool)> {
    let pitch = grid.pitch();
    let parts = i64::from(grid.subdivisions.max(1));
    let base = offset.rem_euclid(pitch);
    // The index (counted from `base`) of the line through the offset: every `parts` lines from it are main lines.
    let through = ((offset - base) / pitch).round() as i64;
    let (first, stride) = if majors_only { (through.rem_euclid(parts), parts) } else { (0, 1) };
    let mut out = Vec::new();
    let mut k = first;
    while out.len() < MAX_LINES_PER_AXIS {
        let position = base + k as f64 * pitch;
        let position = (position * 1000.0).round() / 1000.0 + 0.0;
        if position > limit {
            break;
        }
        out.push((position, (k - through).rem_euclid(parts) == 0));
        k += stride;
    }
    out
}

/// The lines of `grid` over the page `xf` places: none when the grid is refused by [`Grid::check`], the page has no
/// size, or even the main lines would be closer than [`MIN_SCREEN_PITCH`] on screen.
pub(super) fn lines(xf: &PageXform, grid: &Grid) -> Vec<GridLine> {
    if grid.check().is_err() || !(xf.pw > 0.0 && xf.ph > 0.0) {
        return Vec::new();
    }
    let zoom = xf.zoom();
    if !zoom.is_finite() || zoom <= 0.0 {
        return Vec::new();
    }
    let fine_enough = grid.pitch() as f32 * zoom >= MIN_SCREEN_PITCH;
    if !fine_enough && (grid.spacing as f32 * zoom) < MIN_SCREEN_PITCH {
        return Vec::new();
    }
    let majors_only = !fine_enough;
    let mut out = Vec::new();
    for (position, major) in axis_positions(f64::from(xf.pw), grid.offset_x, grid, majors_only) {
        let u = position as f32 / xf.pw;
        out.push(GridLine { points: [xf.norm_to_screen(u, 0.0), xf.norm_to_screen(u, 1.0)], major });
    }
    for (position, major) in axis_positions(f64::from(xf.ph), grid.offset_y, grid, majors_only) {
        let v = position as f32 / xf.ph;
        out.push(GridLine { points: [xf.norm_to_screen(0.0, v), xf.norm_to_screen(1.0, v)], major });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Rect, pos2, vec2};

    /// A 200 x 300 pt page shown 2 screen points per page point.
    fn xf(rot: u16) -> PageXform {
        let rect = if rot % 180 == 90 {
            Rect::from_min_size(pos2(10.0, 20.0), vec2(600.0, 400.0))
        } else {
            Rect::from_min_size(pos2(10.0, 20.0), vec2(400.0, 600.0))
        };
        PageXform { rect, rot, pw: 200.0, ph: 300.0 }
    }

    fn vertical(lines: &[GridLine]) -> Vec<&GridLine> {
        lines.iter().filter(|l| (l.points[0].x - l.points[1].x).abs() < 1e-3).collect()
    }

    #[test]
    fn lines_run_every_pitch_from_the_edge_and_main_lines_are_every_spacing() {
        let grid = Grid { spacing: 50.0, subdivisions: 2, offset_x: 0.0, offset_y: 0.0 };
        let all = lines(&xf(0), &grid);
        let v = vertical(&all);
        // 0, 25, 50, 75, 100, 125, 150, 175, 200.
        assert_eq!(v.len(), 9);
        assert_eq!(v.iter().filter(|l| l.major).count(), 5);
        assert!(v.iter().any(|l| (l.points[0].x - 110.0).abs() < 1e-3 && l.major), "x = 50 pt is a main line at 10 + 100");
    }

    #[test]
    fn an_offset_moves_the_lines_and_the_main_ones_with_it() {
        let grid = Grid { spacing: 100.0, subdivisions: 2, offset_x: 30.0, offset_y: 0.0 };
        let all = lines(&xf(0), &grid);
        let v = vertical(&all);
        let at = |pt: f32| v.iter().find(|l| (l.points[0].x - (10.0 + pt * 2.0)).abs() < 1e-2);
        assert!(at(30.0).is_some_and(|l| l.major));
        assert!(at(80.0).is_some_and(|l| !l.major));
        assert!(at(130.0).is_some_and(|l| l.major));
        assert!(at(0.0).is_none(), "nothing at the edge when the offset puts the line elsewhere");
    }

    #[test]
    fn a_grid_too_fine_for_the_screen_draws_its_main_lines_only_and_then_none() {
        let fine = Grid { subdivisions: 10, ..Grid::new(50.0) };
        let all = lines(&xf(0), &fine);
        assert!(all.iter().any(|l| !l.major), "subdivisions of 5 points are 10 screen points: kept");
        let finer = Grid { subdivisions: 10, ..Grid::new(20.0) };
        let majors = lines(&xf(0), &finer);
        assert!(!majors.is_empty() && majors.iter().all(|l| l.major), "2 pt pitch is 4 screen points: main lines only");
        let none = lines(&xf(0), &Grid::new(2.0));
        assert!(none.is_empty());
    }

    #[test]
    fn the_grid_follows_the_views_rotation() {
        let grid = Grid::new(50.0);
        let turned = lines(&xf(90), &grid);
        assert!(!turned.is_empty());
        let (min, max) = (pos2(10.0, 20.0), pos2(610.0, 420.0));
        assert!(
            turned.iter().flat_map(|l| l.points).all(|p| p.x >= min.x - 1e-3 && p.x <= max.x + 1e-3 && p.y >= min.y - 1e-3 && p.y <= max.y + 1e-3)
        );
    }

    #[test]
    fn a_refused_grid_draws_nothing() {
        assert!(lines(&xf(0), &Grid::new(0.0)).is_empty());
        assert!(lines(&xf(0), &Grid { subdivisions: 0, ..Grid::new(50.0) }).is_empty());
    }
}
