//! View ▸ Guides: ruler guides, the lines a user drags out of the rulers to line things up.
//!
//! A guide belongs to one page of one open document. It is view state, as in Acrobat: it is never
//! written to the PDF, never printed or rendered, and is not an edit (no undo step, no dirty
//! flag). It lasts as long as the document stays open in the session. Guides are kept by page
//! number and are not moved when pages are inserted, deleted or reordered.
//!
//! A position is in points on the displayed page (after `/Rotate` and `/UserUnit`): a
//! [`GuideAxis::Vertical`] guide is that far from the page's left edge, a
//! [`GuideAxis::Horizontal`] one that far from its top edge. A guide may lie on the page edge but
//! not beyond it. The page size is the one at the time of adding: a later crop does not move or
//! remove a guide. Positions are kept to a thousandth of a point (a position is rounded to that,
//! and `-0` becomes `0`), and two guides on one page and axis at the same rounded position are
//! one. Reloading the file from disk keeps the guides (and their ids) of the pages that are
//! still there; a guide may otherwise name a page the document no longer has after pages are
//! deleted, and can still be removed by id or by that page number.

use crate::{DocId, EditError, Session};

/// The widest snap distance accepted, in points.
pub const MAX_SNAP_TOLERANCE: f64 = 72.0;

/// What one axis of a point snapped to.
#[derive(Clone, Debug, PartialEq)]
pub struct AxisSnap {
    /// The position after snapping: the guide's when one was in reach, else the one given.
    pub position: f64,
    /// The guide it snapped to, when there was one.
    pub guide: Option<u64>,
}

/// The nearest of `candidates` (guide id, position) to `value` within `tolerance` points, both
/// ends included; on a tie the lower position wins. (The lower-id tie-break is unreachable in practice: `add_guide` merges guides at the same page, axis and position, so two candidates never share a position.)
fn nearest(candidates: impl Iterator<Item = (u64, f64)>, value: f64, tolerance: f64) -> AxisSnap {
    let mut best: Option<(f64, f64, u64)> = None;
    for (id, at) in candidates {
        let d = (at - value).abs();
        if d > tolerance {
            continue;
        }
        let better = best.is_none_or(|(bd, bat, bid)| d < bd || (d == bd && (at < bat || (at == bat && id < bid))));
        if better {
            best = Some((d, at, id));
        }
    }
    match best {
        Some((_, at, id)) => AxisSnap { position: at, guide: Some(id) },
        None => AxisSnap { position: value, guide: None },
    }
}

/// The most guides one document keeps, so that a script can't grow the list without bound.
pub const MAX_GUIDES: usize = 200;

/// Which way a guide runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuideAxis {
    /// A vertical line: its position is measured from the page's left edge.
    Vertical,
    /// A horizontal line: its position is measured from the page's top edge.
    Horizontal,
}

impl GuideAxis {
    /// `"vertical"` or `"horizontal"`.
    pub fn name(self) -> &'static str {
        match self {
            GuideAxis::Vertical => "vertical",
            GuideAxis::Horizontal => "horizontal",
        }
    }

    /// The axis a name from [`GuideAxis::name`] stands for.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "vertical" => Some(GuideAxis::Vertical),
            "horizontal" => Some(GuideAxis::Horizontal),
            _ => None,
        }
    }
}

/// One guide.
#[derive(Clone, Debug, PartialEq)]
pub struct Guide {
    /// Stable for the life of the document in the session; never reused.
    pub id: u64,
    /// 0-based page.
    pub page: usize,
    pub axis: GuideAxis,
    /// Points from the left edge (vertical) or top edge (horizontal) of the displayed page.
    pub position: f64,
}

/// A document's guides.
#[derive(Clone, Debug, Default)]
pub struct Guides {
    list: Vec<Guide>,
    next_id: u64,
}

impl Guides {
    /// These guides without those on pages from `pages` on, ids kept.
    pub(crate) fn kept_within(mut self, pages: usize) -> Self {
        self.list.retain(|g| g.page < pages);
        self
    }
}

/// `position` on `axis` of page `page` (0-based) of `doc`, rounded to a thousandth of a point
/// (`-0` becomes `0`): refused for a page the document does not have and for a value that is not
/// finite or lies off the displayed page (its edges are on it).
fn position_on_page(doc: &crate::Document, page: usize, axis: GuideAxis, position: f64) -> Result<f64, EditError> {
    let Some(info) = doc.info.pages.get(page) else {
        return Err(EditError::Invalid(format!("page {} is out of range: the document has {} pages", page + 1, doc.info.pages.len())));
    };
    let limit = f64::from(match axis {
        GuideAxis::Vertical => info.width,
        GuideAxis::Horizontal => info.height,
    });
    if !position.is_finite() || position < 0.0 || position > limit {
        return Err(EditError::Invalid(format!("a {} guide must lie on the page, between 0 and {limit} points (got {position})", axis.name())));
    }
    // To a thousandth of a point; adding zero turns -0.0 into 0.0.
    Ok((position * 1000.0).round() / 1000.0 + 0.0)
}

impl Session {
    /// The guides of a document, in the order they were added; `page` (0-based) limits them to
    /// one page.
    pub fn guides(&self, id: DocId, page: Option<usize>) -> Result<Vec<Guide>, EditError> {
        let doc = self.get(id).ok_or(EditError::NoDocument)?;
        Ok(doc.guides.list.iter().filter(|g| page.is_none_or(|p| g.page == p)).cloned().collect())
    }

    /// Adds a guide at the position rounded to 1/1000 point. One already there at the same place is returned instead of a second (the
    /// bool is `true` when a new guide was made). Refused for a page that does not exist, a
    /// position off the page or not finite, and past [`MAX_GUIDES`].
    pub fn add_guide(&mut self, id: DocId, page: usize, axis: GuideAxis, position: f64) -> Result<(Guide, bool), EditError> {
        let doc = self.doc_mut(id)?;
        let position = position_on_page(doc, page, axis, position)?;
        if let Some(found) = doc.guides.list.iter().find(|g| g.page == page && g.axis == axis && g.position == position) {
            return Ok((found.clone(), false));
        }
        if doc.guides.list.len() >= MAX_GUIDES {
            return Err(EditError::Invalid(format!("a document keeps at most {MAX_GUIDES} guides; remove some first")));
        }
        doc.guides.next_id = doc.guides.next_id.saturating_add(1);
        let guide = Guide { id: doc.guides.next_id, page, axis, position };
        doc.guides.list.push(guide.clone());
        Ok((guide, true))
    }

    /// Moves the guide with this id along its axis to `position` (dragging it, in Acrobat), keeping
    /// its id, page and axis; the bool is `true` when it moved and `false` when it was already
    /// there. The position is checked as [`Session::add_guide`] checks one and rounded as it is.
    /// Refused, with nothing changed, for an id no guide has, a guide whose page the document no
    /// longer has (there is no page to measure the position against), a position off the page or
    /// not finite, and a position where another guide of that page and axis already lies (two
    /// guides are never one place; remove one instead).
    pub fn move_guide(&mut self, id: DocId, guide: u64, position: f64) -> Result<(Guide, bool), EditError> {
        let doc = self.doc_mut(id)?;
        let Some(current) = doc.guides.list.iter().find(|g| g.id == guide).cloned() else {
            return Err(EditError::Invalid(format!("there is no guide {guide} (see doc_guides)")));
        };
        if current.page >= doc.info.pages.len() {
            return Err(EditError::Invalid(format!(
                "guide {guide} is on page {}, which the document no longer has, so there is nothing to measure its position against; remove it with doc_guide_remove",
                current.page + 1
            )));
        }
        let position = position_on_page(doc, current.page, current.axis, position)?;
        if position == current.position {
            return Ok((current, false));
        }
        if let Some(other) = doc.guides.list.iter().find(|g| g.page == current.page && g.axis == current.axis && g.position == position) {
            return Err(EditError::Invalid(format!(
                "guide {} is already at {position} points; remove one of the two instead of moving onto it",
                other.id
            )));
        }
        let Some(slot) = doc.guides.list.iter_mut().find(|g| g.id == guide) else {
            return Err(EditError::Invalid(format!("there is no guide {guide} (see doc_guides)")));
        };
        slot.position = position;
        Ok((slot.clone(), true))
    }

    /// Removes the guide with this id; `false` when there is none.
    pub fn remove_guide(&mut self, id: DocId, guide: u64) -> Result<bool, EditError> {
        let doc = self.doc_mut(id)?;
        let before = doc.guides.list.len();
        doc.guides.list.retain(|g| g.id != guide);
        Ok(doc.guides.list.len() != before)
    }

    /// Removes every guide, or only those of one page (0-based); returns how many went.
    pub fn clear_guides(&mut self, id: DocId, page: Option<usize>) -> Result<usize, EditError> {
        let doc = self.doc_mut(id)?;
        let before = doc.guides.list.len();
        doc.guides.list.retain(|g| page.is_some_and(|p| g.page != p));
        Ok(before - doc.guides.list.len())
    }

    /// Where a point on `page` (0-based) lands when it snaps to that page's guides: `x` to the
    /// nearest vertical guide and `y` to the nearest horizontal one, each only when a guide lies
    /// within `tolerance` points (0 to [`MAX_SNAP_TOLERANCE`]; 0 snaps only onto a guide). The
    /// point is in the guides' own frame: points on the displayed page from its left and top
    /// edges. Read-only, and a page without guides, or one the document no longer has, snaps
    /// to nothing. Refused for a value or tolerance that is not finite, or a tolerance out of range.
    pub fn snap_to_guides(
        &self,
        id: DocId,
        page: usize,
        x: Option<f64>,
        y: Option<f64>,
        tolerance: f64,
    ) -> Result<(Option<AxisSnap>, Option<AxisSnap>), EditError> {
        let doc = self.get(id).ok_or(EditError::NoDocument)?;
        if !tolerance.is_finite() || !(0.0..=MAX_SNAP_TOLERANCE).contains(&tolerance) {
            return Err(EditError::Invalid(format!("a snap tolerance must be between 0 and {MAX_SNAP_TOLERANCE} points (got {tolerance})")));
        }
        for v in [x, y].into_iter().flatten() {
            if !v.is_finite() {
                return Err(EditError::Invalid(format!("a point to snap must be finite (got {v})")));
            }
        }
        let on = |axis: GuideAxis| doc.guides.list.iter().filter(move |g| g.page == page && g.axis == axis).map(|g| (g.id, g.position));
        Ok((x.map(|v| nearest(on(GuideAxis::Vertical), v, tolerance)), y.map(|v| nearest(on(GuideAxis::Horizontal), v, tolerance))))
    }
}

/// The smallest distance between two grid lines, in points.
pub const MIN_GRID_PITCH: f64 = 0.01;
/// The widest distance between grid lines, in points.
pub const MAX_GRID_SPACING: f64 = 14_400.0;
/// The most divisions one grid spacing may have.
pub const MAX_GRID_SUBDIVISIONS: u32 = 100;
/// The furthest a grid may be shifted, in points either way.
pub const MAX_GRID_OFFSET: f64 = 1_000_000.0;

/// A grid of lines over a page, in the guides' frame: vertical lines are measured from the page's
/// left edge, horizontal ones from its top edge. Lines run every `spacing / subdivisions` points
/// and one of each runs through the offset. The page is the displayed one, after `/Rotate` and `/UserUnit`, as for guides. This is the arithmetic of a snap-to-grid and of the app's View > Show Grid, which draws it on screen
/// only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    /// Points between two main lines.
    pub spacing: f64,
    /// Equal parts a main spacing is divided into (1 for none).
    pub subdivisions: u32,
    /// Where a vertical line runs, from the left edge (any other is a whole pitch away).
    pub offset_x: f64,
    /// Where a horizontal line runs, from the top edge.
    pub offset_y: f64,
}

impl Grid {
    /// A grid of this spacing with no subdivisions and lines through the page's top left corner.
    pub fn new(spacing: f64) -> Self {
        Grid { spacing, subdivisions: 1, offset_x: 0.0, offset_y: 0.0 }
    }

    /// Points between two neighbouring lines.
    pub fn pitch(&self) -> f64 {
        self.spacing / f64::from(self.subdivisions.max(1))
    }

    /// Refuses a grid that is not finite, whose spacing is not 0.01 to 14,400 points, whose
    /// subdivisions are not 1 to 100, whose pitch is under 0.01 point or whose offset is past a
    /// million points.
    pub fn check(&self) -> Result<(), EditError> {
        if !self.spacing.is_finite() || !(MIN_GRID_PITCH..=MAX_GRID_SPACING).contains(&self.spacing) {
            return Err(EditError::Invalid(format!(
                "a grid spacing must be between {MIN_GRID_PITCH} and {MAX_GRID_SPACING} points (got {})",
                self.spacing
            )));
        }
        if !(1..=MAX_GRID_SUBDIVISIONS).contains(&self.subdivisions) {
            return Err(EditError::Invalid(format!("grid subdivisions must be between 1 and {MAX_GRID_SUBDIVISIONS} (got {})", self.subdivisions)));
        }
        if self.pitch() < MIN_GRID_PITCH {
            return Err(EditError::Invalid(format!(
                "grid lines must be at least {MIN_GRID_PITCH} point apart: a spacing of {} points in {} parts is too fine",
                self.spacing, self.subdivisions
            )));
        }
        for v in [self.offset_x, self.offset_y] {
            if !v.is_finite() || v.abs() > MAX_GRID_OFFSET {
                return Err(EditError::Invalid(format!("a grid offset must be within {MAX_GRID_OFFSET} points either way (got {v})")));
            }
        }
        Ok(())
    }
}

/// What one axis of a point snapped to on a grid.
#[derive(Clone, Debug, PartialEq)]
pub struct GridSnap {
    /// The grid line's position, rounded to a thousandth of a point, when one was in reach; else
    /// the position given, exactly as given.
    pub position: f64,
    /// Whether it moved to a grid line.
    pub snapped: bool,
}

/// The nearest grid line to `value` that lies on the page (0 to `limit`), both ends included, within
/// `tolerance` points when one is given; on a tie the lower line wins (the lower of the two candidates is tried first and a later one must be strictly nearer). Lines are rounded to a
/// thousandth of a point before they are compared with the page.
fn nearest_line(value: f64, limit: f64, pitch: f64, offset: f64, tolerance: Option<f64>) -> GridSnap {
    let unmoved = GridSnap { position: value, snapped: false };
    let base = offset.rem_euclid(pitch);
    let k = ((value - base) / pitch).floor();
    if !k.is_finite() {
        return unmoved;
    }
    let lo = base + k * pitch;
    let round = |v: f64| (v * 1000.0).round() / 1000.0 + 0.0;
    let mut best: Option<(f64, f64)> = None;
    for line in [round(lo), round(lo + pitch)] {
        if !(0.0..=limit).contains(&line) {
            continue;
        }
        let d = (line - value).abs();
        if tolerance.is_some_and(|t| d > t) {
            continue;
        }
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, line));
        }
    }
    match best {
        Some((_, line)) => GridSnap { position: line, snapped: true },
        None => unmoved,
    }
}

impl Session {
    /// Preferences ▸ Units & Guides ▸ Grid as set for this session: the grid a snap to a grid
    /// uses for what it is not given. `None` until [`Session::set_grid_preference`] stores one;
    /// it is held by the session only and is not kept between runs.
    pub fn grid_preference(&self) -> Option<Grid> {
        self.grid_preference
    }

    /// Stores `grid` as the grid preference, or forgets it with `None`. A grid that
    /// [`Grid::check`] refuses is refused, and the preference is then as it was.
    pub fn set_grid_preference(&mut self, grid: Option<Grid>) -> Result<(), EditError> {
        if let Some(g) = &grid {
            g.check()?;
        }
        self.grid_preference = grid;
        Ok(())
    }

    /// Where a point on `page` (0-based) lands when it snaps to `grid`: `x` to the nearest vertical
    /// line and `y` to the nearest horizontal one, each only when a line of the grid that lies on
    /// the page is within `tolerance` points (0 to [`MAX_SNAP_TOLERANCE`]; `None` snaps to the
    /// nearest line however far). The point is in the guides' own frame (see [`Grid`]) and must
    /// lie on the page. Read-only: no guide is added and the document is not touched. Refused for
    /// a page the document does not have, a grid [`Grid::check`] refuses, a point that is not
    /// finite or off the page, and a tolerance out of range.
    pub fn snap_to_grid(
        &self,
        id: DocId,
        page: usize,
        x: Option<f64>,
        y: Option<f64>,
        grid: &Grid,
        tolerance: Option<f64>,
    ) -> Result<(Option<GridSnap>, Option<GridSnap>), EditError> {
        let doc = self.get(id).ok_or(EditError::NoDocument)?;
        let Some(info) = doc.info.pages.get(page) else {
            return Err(EditError::Invalid(format!("page {} is out of range: the document has {} pages", page + 1, doc.info.pages.len())));
        };
        grid.check()?;
        if let Some(t) = tolerance
            && (!t.is_finite() || !(0.0..=MAX_SNAP_TOLERANCE).contains(&t))
        {
            return Err(EditError::Invalid(format!("a snap tolerance must be between 0 and {MAX_SNAP_TOLERANCE} points (got {t})")));
        }
        let (width, height) = (f64::from(info.width), f64::from(info.height));
        for (v, limit, what) in [(x, width, "x"), (y, height, "y")] {
            if let Some(v) = v
                && (!v.is_finite() || v < 0.0 || v > limit)
            {
                return Err(EditError::Invalid(format!("{what} must lie on the page, between 0 and {limit} points (got {v})")));
            }
        }
        let pitch = grid.pitch();
        Ok((x.map(|v| nearest_line(v, width, pitch, grid.offset_x, tolerance)), y.map(|v| nearest_line(v, height, pitch, grid.offset_y, tolerance))))
    }
}

#[cfg(test)]
mod grid_tests {
    use super::*;

    #[test]
    fn nearest_line_takes_the_closest_line_on_the_page_and_breaks_ties_low() {
        let snap = |v, limit, pitch, offset, tol| nearest_line(v, limit, pitch, offset, tol);
        let at = |position, snapped| GridSnap { position, snapped };
        assert_eq!(snap(100.0, 200.0, 72.0, 0.0, None), at(72.0, true));
        assert_eq!(snap(110.0, 200.0, 72.0, 0.0, None), at(144.0, true));
        // A tie goes to the lower line.
        assert_eq!(snap(36.0, 200.0, 72.0, 0.0, None), at(0.0, true));
        // A shifted grid, and a negative offset is the same grid as the positive one a pitch on.
        assert_eq!(snap(100.0, 200.0, 72.0, 10.0, None), at(82.0, true));
        assert_eq!(snap(100.0, 200.0, 72.0, -62.0, None), at(82.0, true));
        // A line past the page does not count, though it is nearer: 200 is 45 away from 155, 100 is 55.
        assert_eq!(snap(155.0, 160.0, 100.0, 0.0, None), at(100.0, true));
        // Both ends of the page are on it.
        assert_eq!(snap(200.0, 200.0, 100.0, 0.0, None), at(200.0, true));
        assert_eq!(snap(0.0, 200.0, 100.0, 0.0, None), at(0.0, true));
        // No line on the page: the offset is past it and the pitch is wide.
        assert_eq!(snap(50.0, 200.0, 400.0, 300.0, None), at(50.0, false));
        // Tolerance: edge included, beyond it nothing moves.
        assert_eq!(snap(100.0, 200.0, 72.0, 0.0, Some(28.0)), at(72.0, true));
        assert_eq!(snap(100.0, 200.0, 72.0, 0.0, Some(27.9)), at(100.0, false));
        // A pitch that does not divide the page evenly still lands on a line, to a thousandth.
        assert_eq!(snap(0.29, 0.3, 0.1, 0.0, None), at(0.3, true));
        // Huge values do not panic.
        assert!(!snap(f64::MAX, 200.0, 0.01, 0.0, None).snapped);
    }

    #[test]
    fn a_grid_is_checked() {
        let ok = Grid { spacing: 72.0, subdivisions: 8, offset_x: -5.0, offset_y: 1e6 };
        assert!(ok.check().is_ok());
        assert_eq!(ok.pitch(), 9.0);
        for bad in [
            Grid::new(0.0),
            Grid::new(0.009),
            Grid::new(14_400.1),
            Grid::new(f64::NAN),
            Grid::new(f64::INFINITY),
            Grid { subdivisions: 0, ..Grid::new(72.0) },
            Grid { subdivisions: 101, ..Grid::new(72.0) },
            Grid { subdivisions: 100, ..Grid::new(0.5) },
            Grid { offset_x: 1e6 + 1.0, ..Grid::new(72.0) },
            Grid { offset_y: f64::NAN, ..Grid::new(72.0) },
        ] {
            assert!(bad.check().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_grid_preference_is_stored_checked_and_forgotten() {
        let mut s = Session::new();
        assert_eq!(s.grid_preference(), None);
        let g = Grid { spacing: 36.0, subdivisions: 2, offset_x: 3.0, offset_y: -4.0 };
        s.set_grid_preference(Some(g)).unwrap();
        assert_eq!(s.grid_preference(), Some(g));
        // A refused grid leaves the stored one as it was.
        assert!(s.set_grid_preference(Some(Grid::new(0.0))).is_err());
        assert!(s.set_grid_preference(Some(Grid { subdivisions: 101, ..g })).is_err());
        assert_eq!(s.grid_preference(), Some(g));
        s.set_grid_preference(None).unwrap();
        assert_eq!(s.grid_preference(), None);
    }
}

#[cfg(test)]
mod snap_tests {
    use super::*;

    fn c(v: &[(u64, f64)]) -> impl Iterator<Item = (u64, f64)> + '_ {
        v.iter().copied()
    }

    #[test]
    fn nearest_takes_the_closest_within_reach_and_breaks_ties_low() {
        assert_eq!(nearest(c(&[(1, 10.0), (2, 20.0)]), 14.0, 5.0), AxisSnap { position: 10.0, guide: Some(1) });
        assert_eq!(nearest(c(&[(1, 10.0), (2, 20.0)]), 15.0, 5.0), AxisSnap { position: 10.0, guide: Some(1) });
        assert_eq!(nearest(c(&[(2, 20.0), (1, 10.0)]), 15.0, 5.0), AxisSnap { position: 10.0, guide: Some(1) });
        assert_eq!(nearest(c(&[(1, 10.0)]), 15.1, 5.0), AxisSnap { position: 15.1, guide: None });
        assert_eq!(nearest(c(&[]), 3.0, 5.0), AxisSnap { position: 3.0, guide: None });
        assert_eq!(nearest(c(&[(1, 5.0)]), 5.0, 0.0).guide, Some(1));
        // Huge values do not overflow or panic.
        assert_eq!(nearest(c(&[(1, 5.0)]), f64::MAX, 72.0).guide, None);
    }
}
