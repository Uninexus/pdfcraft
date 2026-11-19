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
        let position = (position * 1000.0).round() / 1000.0 + 0.0;
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
