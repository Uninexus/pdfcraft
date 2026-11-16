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
}
