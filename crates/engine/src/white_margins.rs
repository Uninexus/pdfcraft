//! Organize ▸ Crop Pages ▸ Remove White Margins: crop each page to where its content is.
//!
//! The content bounds come from a raster of the page (what a viewer would show, so anything
//! that paints counts: text, images, vector art, visible annotations), at a modest resolution:
//! [`SCALE`] pixels per point, lowered so that no side passes [`MAX_SIDE`] pixels. The bounds are
//! the box around every pixel that is not blank, then widened by the padding and clamped to what
//! the page showed. They are applied through the ordinary crop edit ([`Edit::SetPageBox`]), one
//! per page inside a single [`Edit::Batch`], so Crop pages' rules (the crop box is clipped to the
//! media box, permissions, undo) are the ones that apply here.
//!
//! The crop box is rounded outward to hundredths of a point, so it never cuts into the content.
//!
//! A pixel is blank when it is transparent (alpha below [`ALPHA_BLANK`]) or when every colour
//! channel is at least [`WHITE_FROM`] (so faint off-white paper and anti-aliasing fringes below
//! 2% don't count as content). The raster resolves bounds to about `1 / SCALE` point, and to
//! `max side / MAX_SIDE` points on very large pages.
//!
//! The raster is of the page as it is cropped now, so the new crop box is always inside the old
//! one: this never uncrops a page. `/Rotate` and `/UserUnit` are handled by mapping the bounds
//! through [`pdfcraft_render::PageInfo::view_to_user`], which goes from the displayed
//! page to user space.

use pdfcraft_organize::{BoxSpec, PageBox};
use std::sync::Arc;

use pdfcraft_render::{PageInfo, PageRenderer, RenderRequest, RequestKind};

use crate::edit::check_permission;
use crate::{DocId, Edit, EditError, Session, guard};

/// Pixels per point of the raster the bounds come from (144 dpi).
pub const SCALE: f32 = 2.0;
/// The longest side of that raster in pixels, which bounds its memory (about 16 MB of pixels at
/// most, whatever the page size).
pub const MAX_SIDE: f32 = 2000.0;
/// A colour channel at or above this is white paper.
pub const WHITE_FROM: u8 = 250;
/// A pixel with less alpha than this is transparent.
pub const ALPHA_BLANK: u8 = 8;
/// The padding cap in points: past it the request is refused rather than clamped.
pub const MAX_PADDING: f64 = 1000.0;
/// A crop box is never made smaller than this many user units on either side (Crop pages refuses
/// less than 1, and a user unit is a point unless the page has a `/UserUnit`).
const MIN_SIDE: f64 = 1.0;

/// What [`Session::remove_white_margins`] did, with 0-based page numbers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WhiteMarginsReport {
    /// Pages given a new crop box, with it in user space `[x0, y0, x1, y1]`.
    pub cropped: Vec<(usize, [f64; 4])>,
    /// Pages with nothing to crop, left as they were: the content already fills the crop box.
    pub unchanged: Vec<usize>,
    /// Blank pages (nothing but white or transparent), left as they were.
    pub blank: Vec<usize>,
    /// Pages that could not be measured, left as they were, with why.
    pub failed: Vec<(usize, String)>,
}

/// The pixel bounds `[x0, y0, x1, y1)` of the non-blank pixels of a `width` × `height`
/// premultiplied RGBA8 raster, or `None` when it is blank (or `rgba` is too short for its size).
pub fn content_bounds(rgba: &[u8], width: usize, height: usize) -> Option<[usize; 4]> {
    let row = width.checked_mul(4)?;
    if rgba.len() < row.checked_mul(height)? {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for (y, line) in rgba.chunks_exact(row.max(4)).take(height).enumerate() {
        let (px_chunks, _) = line.as_chunks::<4>();
        for (x, px) in px_chunks.iter().enumerate() {
            let content = match px {
                [r, g, b, a] => *a >= ALPHA_BLANK && (*r < WHITE_FROM || *g < WHITE_FROM || *b < WHITE_FROM),
            };
            if content {
                x0 = x0.min(x);
                x1 = x1.max(x + 1);
                y0 = y0.min(y);
                y1 = y1.max(y + 1);
            }
        }
    }
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1, y1])
}

/// What removing the white margins from some pages needs, independent of the session: the
/// displayed file and the pages' sizes. Measuring is the slow part (a raster per page), so it
/// runs anywhere, such as a worker thread; [`Session::apply_white_margins`] then makes the result
/// one undoable step.
pub struct WhiteMarginsJob {
    display: Arc<Vec<u8>>,
    config: pdfcraft_render::RenderConfig,
    pages: Vec<(usize, PageInfo)>,
    padding: f64,
    /// The document's edit generation it was taken at: a result is for that state only.
    generation: u64,
}

/// The measured crop boxes, ready for [`Session::apply_white_margins`], and the report.
pub struct WhiteMargins {
    edits: Vec<Edit>,
    generation: u64,
    pub report: WhiteMarginsReport,
}

impl WhiteMargins {
    /// The edit-generation of the document the boxes were measured on.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The one edit that applies the new crop boxes, or `None` when no page changed. An
    /// interface that applies edits itself (to keep its views in step) uses this, after checking
    /// that the document is still at [`Self::generation`].
    pub fn edit(&self) -> Option<Edit> {
        (!self.edits.is_empty()).then(|| Edit::Batch { label: "Remove white margins".into(), edits: self.edits.clone() })
    }
}

impl Session {
    /// Capture what removing white margins from `pages` (0-based; empty = all) of document `id`
    /// needs, with `padding` points kept on every side (zero or more, at most [`MAX_PADDING`]).
    /// Errors, changing nothing, when the document can't be changed, a page doesn't exist or the
    /// padding is not allowed.
    pub fn white_margins_job(&self, id: DocId, pages: &[usize], padding: f64) -> Result<WhiteMarginsJob, EditError> {
        if !padding.is_finite() || !(0.0..=MAX_PADDING).contains(&padding) {
            return Err(EditError::Invalid(format!("the padding must be between 0 and {MAX_PADDING} points")));
        }
        let doc = self.get(id).ok_or(EditError::NoDocument)?;
        if let Some(why) = &doc.read_only_reason {
            return Err(EditError::ReadOnly(why.clone()));
        }
        let editor = doc.editor.as_ref().ok_or_else(|| EditError::ReadOnly(String::new()))?;
        if let Some(p) = editor.cos.permissions() {
            check_permission(&Edit::SetPageBox { pages: Vec::new(), which: PageBox::Crop, spec: BoxSpec::Remove }, &p)?;
        }
        let all = doc.info.pages.len();
        let mut wanted: Vec<usize> = if pages.is_empty() { (0..all).collect() } else { pages.to_vec() };
        if let Some(p) = wanted.iter().find(|p| **p >= all) {
            return Err(EditError::Invalid(format!("there is no page {}: the document has {all}", p.saturating_add(1))));
        }
        wanted.sort_unstable();
        wanted.dedup();
        // The page as the file draws it: on white, in its true colours, whatever the viewing
        // settings (reading colours would change the paper), with no installed fonts, with the
        // layers as the file sets them (not as the view has it) and with comments shown, so
        // what is hidden in the view isn't cropped away for good.
        let config = pdfcraft_render::RenderConfig {
            reading_colors: Default::default(),
            transparent: false,
            system_fonts: false,
            layers: Default::default(),
            print_layers: None,
            hide_comments: false,
            ..doc.config.clone()
        };
        let pages = wanted.into_iter().filter_map(|p| doc.info.pages.get(p).map(|i| (p, i.clone()))).collect();
        Ok(WhiteMarginsJob { display: doc.display.clone(), config, pages, padding, generation: doc.edit_generation() })
    }

    /// Make the measured crop boxes one undoable step, "Remove white margins" (none when no page
    /// changed), and return the report. Refused, changing nothing, when the document changed
    /// since the job was taken.
    pub fn apply_white_margins(&mut self, id: DocId, measured: WhiteMargins) -> Result<WhiteMarginsReport, EditError> {
        let doc = self.get(id).ok_or(EditError::NoDocument)?;
        if doc.edit_generation() != measured.generation {
            return Err(EditError::Invalid(
                "the document changed while its white margins were being measured; nothing was changed, try again".into(),
            ));
        }
        if !measured.edits.is_empty() {
            self.apply(id, Edit::Batch { label: "Remove white margins".into(), edits: measured.edits })?;
        }
        Ok(measured.report)
    }

    /// Remove white margins from `pages` (0-based; empty = all) of document `id`: each page's
    /// crop box becomes the bounds of its content plus `padding` points on every side (zero or
    /// more, at most [`MAX_PADDING`]), within what the page showed. One undoable step,
    /// "Remove white margins", and none when no page changed. Blank pages are left as they are
    /// and reported. Errors, changing nothing, when the document can't be changed, a page
    /// doesn't exist or the padding is not allowed. This is [`Self::white_margins_job`], run, then
    /// [`Self::apply_white_margins`] (the CLI and agents use it; the app runs the job on a worker).
    pub fn remove_white_margins(&mut self, id: DocId, pages: &[usize], padding: f64) -> Result<WhiteMarginsReport, EditError> {
        let measured = self.white_margins_job(id, pages, padding)?.run(|_, _| true)?;
        self.apply_white_margins(id, measured)
    }
}

impl WhiteMarginsJob {
    /// The pages to measure.
    pub fn total(&self) -> usize {
        self.pages.len()
    }

    /// Measure every page, one raster at a time. `progress(done, total)` is called before each
    /// page; returning `false` stops with [`EditError::Cancelled`]. A panic is returned as an
    /// error, so a worker thread always reports back.
    pub fn run(self, mut progress: impl FnMut(usize, usize) -> bool) -> Result<WhiteMargins, EditError> {
        guard(move || self.work(&mut progress)).map_err(|m| EditError::Invalid(format!("removing the white margins failed unexpectedly ({m})")))?
    }

    fn work(self, progress: &mut dyn FnMut(usize, usize) -> bool) -> Result<WhiteMargins, EditError> {
        let WhiteMarginsJob { display, config, pages, padding, generation } = self;
        let total = pages.len();
        let mut renderer = PageRenderer::new(display, config);
        let mut report = WhiteMarginsReport::default();
        let mut edits = Vec::new();
        for (done, (page, info)) in pages.iter().enumerate() {
            let page = *page;
            if !progress(done, total) {
                return Err(EditError::Cancelled);
            }
            let side = info.width.max(info.height);
            if !(side.is_finite() && side > 0.0 && info.width > 0.0 && info.height > 0.0) {
                report.failed.push((page, "the page has no size".into()));
                continue;
            }
            let scale = SCALE.min(MAX_SIDE / side);
            let out = renderer.render(RenderRequest { page, kind: RequestKind::Pixels, tile: None, scale, tag: 0, preview: false });
            if let Some(e) = out.error {
                report.failed.push((page, e));
                continue;
            }
            let (w, h) = (out.width as usize, out.height as usize);
            let Some([px0, py0, px1, py1]) = content_bounds(&out.rgba, w, h) else {
                report.blank.push(page);
                continue;
            };
            // Pixel bounds → the displayed page in points. The raster is `scale` pixels per point
            // and its size is rounded up, so a pixel is 1 / scale points, not width / pixels. If the
            // renderer drew at another scale (its own caps), use the ratio and a pixel of slack.
            let (vw, vh) = (f64::from(info.width), f64::from(info.height));
            let drawn = |pt: f32| f64::from(pt * scale - 0.01).ceil().max(1.0);
            let exact = drawn(info.width) == w as f64 && drawn(info.height) == h as f64;
            let (sx, sy, slack) = if exact { (f64::from(scale), f64::from(scale), 0.0) } else { (w as f64 / vw, h as f64 / vh, 1.0) };
            // Widened by the padding, kept on the page.
            let view = [
                ((px0 as f64 - slack) / sx - padding).max(0.0),
                ((py0 as f64 - slack) / sy - padding).max(0.0),
                ((px1 as f64 + slack) / sx + padding).min(vw),
                ((py1 as f64 + slack) / sy + padding).min(vh),
            ];
            // To user space (through /Rotate and /UserUnit).
            let a = info.view_to_user(view[0] as f32, view[1] as f32);
            let b = info.view_to_user(view[2] as f32, view[3] as f32);
            let old = info.crop.map(f64::from);
            let mut rect = [
                f64::from(a[0].min(b[0])).max(old[0]),
                f64::from(a[1].min(b[1])).max(old[1]),
                f64::from(a[0].max(b[0])).min(old[2]),
                f64::from(a[1].max(b[1])).min(old[3]),
            ];
            // No smaller than MIN_SIDE user units (what Crop pages requires), around the content.
            for axis in 0..2 {
                if rect[axis + 2] - rect[axis] < MIN_SIDE {
                    let mid = (rect[axis] + rect[axis + 2]) / 2.0;
                    let (mut lo, mut hi) = (mid - MIN_SIDE / 2.0, mid + MIN_SIDE / 2.0);
                    if lo < old[axis] {
                        (lo, hi) = (old[axis], old[axis] + MIN_SIDE);
                    }
                    if hi > old[axis + 2] {
                        (lo, hi) = (old[axis + 2] - MIN_SIDE, old[axis + 2]);
                    }
                    (rect[axis], rect[axis + 2]) = (lo, hi);
                }
            }
            // Hundredths of a point, rounded outward so the box never cuts into the content (a
            // thousandth of a hundredth of slack absorbs float noise), and still inside the old box.
            let down = |v: f64| (v * 100.0 + 1e-3).floor() / 100.0;
            let up = |v: f64| (v * 100.0 - 1e-3).ceil() / 100.0;
            let rect = [down(rect[0]).max(old[0]), down(rect[1]).max(old[1]), up(rect[2]).min(old[2]), up(rect[3]).min(old[3])];
            if !rect.iter().all(|v| v.is_finite())
                || rect[2] - rect[0] < MIN_SIDE - 1e-9
                || rect[3] - rect[1] < MIN_SIDE - 1e-9
                || rect[0] < old[0] - 1e-9
                || rect[1] < old[1] - 1e-9
            {
                report.failed.push((page, "the page is too small to crop".into()));
                continue;
            }
            if rect.iter().zip(old).all(|(n, o)| (n - o).abs() < 0.01) {
                report.unchanged.push(page);
                continue;
            }
            edits.push(Edit::SetPageBox { pages: vec![page], which: PageBox::Crop, spec: BoxSpec::Rect(rect) });
            report.cropped.push((page, rect));
        }
        Ok(WhiteMargins { edits, generation, report })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A white `w` × `h` raster with `(x, y, [r, g, b, a])` pixels set.
    fn raster(w: usize, h: usize, set: &[(usize, usize, [u8; 4])]) -> Vec<u8> {
        let mut v = vec![255u8; w * h * 4];
        for (x, y, px) in set {
            v[(y * w + x) * 4..][..4].copy_from_slice(px);
        }
        v
    }

    #[test]
    fn bounds_of_non_blank_pixels() {
        let img = raster(10, 8, &[(2, 3, [0, 0, 0, 255]), (6, 5, [200, 255, 255, 255])]);
        assert_eq!(content_bounds(&img, 10, 8), Some([2, 3, 7, 6]));
        // Faint paper noise and transparent pixels are not content; a pixel just under the cut is.
        let img = raster(4, 4, &[(0, 0, [250, 250, 250, 255]), (1, 1, [0, 0, 0, 0]), (3, 3, [249, 255, 255, 255])]);
        assert_eq!(content_bounds(&img, 4, 4), Some([3, 3, 4, 4]));
        assert_eq!(content_bounds(&raster(5, 5, &[]), 5, 5), None, "a white raster is blank");
    }

    #[test]
    fn hostile_sizes_give_none_and_never_panic() {
        assert_eq!(content_bounds(&[], 0, 0), None);
        assert_eq!(content_bounds(&[0; 16], 4, 4), None, "too short for its size");
        assert_eq!(content_bounds(&[0; 16], usize::MAX, 2), None, "overflowing size");
        assert_eq!(content_bounds(&[0; 16], 0, 7), None);
    }

    fn open(session: &mut Session, n: usize) -> DocId {
        session.open("t.pdf", None, Arc::new(crate::tests::fixture(n)), None).unwrap()
    }

    #[test]
    fn crops_the_text_pages_then_undoes_as_one_step() {
        let mut s = Session::new();
        let id = open(&mut s, 3);
        let before: Vec<[f32; 2]> = s.get(id).unwrap().info.pages.iter().map(|p| [p.width, p.height]).collect();
        let r = s.remove_white_margins(id, &[], 0.0).unwrap();
        assert_eq!(r.cropped.len(), 3);
        assert!(r.blank.is_empty() && r.failed.is_empty());
        for p in &s.get(id).unwrap().info.pages {
            // "Page n" at 24 pt is a line of text on a 200 × 300 page: a small box.
            assert!(p.width < 120.0 && p.height < 40.0, "{} x {}", p.width, p.height);
        }
        s.undo(id).unwrap();
        let after: Vec<[f32; 2]> = s.get(id).unwrap().info.pages.iter().map(|p| [p.width, p.height]).collect();
        assert_eq!(after, before, "one undo step put all three pages back");
    }

    #[test]
    fn a_measured_job_is_refused_when_the_document_changed_and_can_be_cancelled() {
        let mut s = Session::new();
        let id = open(&mut s, 2);
        let job = s.white_margins_job(id, &[], 0.0).unwrap();
        assert_eq!(job.total(), 2);
        let mut seen = Vec::new();
        let measured = job
            .run(|done, total| {
                seen.push((done, total));
                true
            })
            .unwrap();
        assert_eq!(seen, [(0, 2), (1, 2)]);
        s.apply(id, Edit::RotatePages { pages: vec![0], degrees: 90 }).unwrap();
        let before: Vec<[f32; 2]> = s.get(id).unwrap().info.pages.iter().map(|p| [p.width, p.height]).collect();
        assert!(matches!(s.apply_white_margins(id, measured), Err(EditError::Invalid(_))));
        let after: Vec<[f32; 2]> = s.get(id).unwrap().info.pages.iter().map(|p| [p.width, p.height]).collect();
        assert_eq!(before, after, "nothing was changed");

        let job = s.white_margins_job(id, &[], 0.0).unwrap();
        assert!(matches!(job.run(|done, _| done == 0), Err(EditError::Cancelled)), "stops before the second page");
    }

    #[test]
    fn refuses_bad_requests_without_changing_anything() {
        let mut s = Session::new();
        let id = open(&mut s, 2);
        for bad in [f64::NAN, -1.0, f64::INFINITY, MAX_PADDING + 1.0] {
            assert!(matches!(s.remove_white_margins(id, &[], bad), Err(EditError::Invalid(_))));
        }
        assert!(matches!(s.remove_white_margins(id, &[2], 0.0), Err(EditError::Invalid(_))));
        assert!(matches!(s.remove_white_margins(DocId(u64::MAX), &[], 0.0), Err(EditError::NoDocument)));
        assert!(s.undo(id).is_err(), "nothing was applied");
    }
}
