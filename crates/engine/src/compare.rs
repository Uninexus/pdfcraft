//! Compare files: the text differences between two open documents, a report, and the
//! differences marked as comments in the newer one.

use std::collections::HashMap;
use std::sync::Arc;

use pdfcraft_cos::ObjRef;

pub use pdfcraft_compare::{Change, Comparison, Kind, Options as WordOptions, Side};
// How far Office export reads into form XObjects; `OfficeExport::truncated_pages` lists the pages
// that draw more.
pub use pdfcraft_edit::{MAX_FORM_DEPTH, MAX_FORM_VISITS};

use crate::{DocId, Edit, EditError, Markup, NewAnnotation, NoteIcon, Session, Shape};

/// Most bytes of encoded pictures one Office export prepares, counting each image object once
/// (the Word and PowerPoint writers keep at most 512 MiB of them as well).
pub const MAX_EXPORT_IMAGE_BYTES: usize = 512 << 20;

/// Acrobat's compare colours: replaced blue, inserted green, deleted red.
pub fn colour(kind: Kind) -> crate::Rgb {
    match kind {
        Kind::Replaced => [0.2, 0.45, 0.95],
        Kind::Inserted => [0.2, 0.75, 0.3],
        Kind::Deleted => [0.9, 0.25, 0.25],
    }
}

/// What a visual difference is, as far as the comparison can tell: a region that overlaps a
/// comment (markup) annotation present in only one of the two files (added, removed, moved or edited), or any
/// other change in how the page looks (images, formatting, background and header/footer changes
/// are not told apart and stay `Other`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualCause {
    Annotation,
    Other,
}

impl VisualCause {
    pub fn label(self) -> &'static str {
        match self {
            Self::Annotation => "annotation",
            Self::Other => "other",
        }
    }
}

/// One visual difference: pages 0-based and each document's own, the box in the newer file's
/// user space, and what the region overlaps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisualRegion {
    pub old_page: usize,
    pub page: usize,
    pub rect: [f64; 4],
    pub cause: VisualCause,
}

/// Most comment annotations of one page that Compare files matches between the two files.
pub const MAX_COMPARED_ANNOTATIONS: usize = 2000;

/// A 1-based, inclusive range of pages of one document for Compare files ("Pages N to M").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageRange {
    pub from: usize,
    pub to: usize,
}

impl PageRange {
    /// The 0-based pages this range covers in a document of `count` pages, or why it can't.
    /// Nothing is clamped: a range that starts below 1, ends before it starts or reaches past
    /// the last page is refused (as neighbouring page arguments are), saying what is allowed.
    pub fn resolve(self, count: usize) -> Result<std::ops::Range<usize>, EditError> {
        let bad = |why: String| EditError::Compare(why);
        let pages = if count == 1 { "page" } else { "pages" };
        if self.from < 1 || self.to < 1 {
            return Err(bad(format!("page range {} to {} is invalid: pages start at 1", self.from, self.to)));
        }
        if self.from > self.to {
            return Err(bad(format!("page range {} to {} is invalid: the first page is after the last", self.from, self.to)));
        }
        if self.to > count {
            return Err(bad(format!("page range {} to {} is out of range: the document has {count} {pages}", self.from, self.to)));
        }
        Ok(self.from - 1..self.to)
    }
}

/// The kinds of change Compare files tells apart: the three kinds of text change, and visual
/// differences (regions that look different). The comparison reads words and pixels only, so
/// images, annotations, formatting and background or header/footer changes are not separate
/// kinds: each one is a visual difference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChangeKinds {
    pub replaced: bool,
    pub inserted: bool,
    pub deleted: bool,
    pub visual: bool,
}

impl Default for ChangeKinds {
    fn default() -> Self {
        Self::all()
    }
}

impl ChangeKinds {
    /// The names a filter accepts, as `kinds` lists them.
    pub const NAMES: [&'static str; 4] = ["replaced", "inserted", "deleted", "visual"];

    /// Every kind (the default: nothing is hidden).
    pub fn all() -> Self {
        Self { replaced: true, inserted: true, deleted: true, visual: true }
    }

    pub fn is_all(self) -> bool {
        self == Self::all()
    }

    pub fn is_empty(self) -> bool {
        !(self.replaced || self.inserted || self.deleted || self.visual)
    }

    /// Whether text changes of `kind` pass the filter.
    pub fn wants(self, kind: Kind) -> bool {
        match kind {
            Kind::Replaced => self.replaced,
            Kind::Inserted => self.inserted,
            Kind::Deleted => self.deleted,
        }
    }

    /// The names of the kinds that pass, in [`ChangeKinds::NAMES`] order.
    pub fn names(self) -> Vec<&'static str> {
        [self.replaced, self.inserted, self.deleted, self.visual].iter().zip(Self::NAMES).filter(|(on, _)| **on).map(|(_, n)| n).collect()
    }

    /// The kinds named in `names` (case and surrounding spaces ignored). An empty list or a name
    /// that is not a kind is refused, and the error lists the names allowed.
    pub fn parse<S: AsRef<str>>(names: &[S]) -> Result<Self, EditError> {
        let allowed = Self::NAMES.join(", ");
        if names.is_empty() {
            return Err(Self::empty_error());
        }
        let mut k = Self { replaced: false, inserted: false, deleted: false, visual: false };
        for n in names {
            match n.as_ref().trim().to_ascii_lowercase().as_str() {
                "replaced" => k.replaced = true,
                "inserted" => k.inserted = true,
                "deleted" => k.deleted = true,
                "visual" => k.visual = true,
                _ => return Err(EditError::Compare(format!("unknown kind of change \"{}\": the kinds are {allowed}", n.as_ref().trim()))),
            }
        }
        Ok(k)
    }

    /// The error for a filter that passes nothing.
    pub fn empty_error() -> EditError {
        EditError::Compare(format!("kinds is empty: name at least one of {}", Self::NAMES.join(", ")))
    }

    /// `c` with only the text changes this filter passes.
    pub fn filter(self, mut c: Comparison) -> Comparison {
        c.changes.retain(|ch| self.wants(ch.kind));
        c
    }
}

impl crate::Document {
    /// Standards ▸ Verify PDF/A: the rules the document breaks for `level`.
    pub fn pdfa_verify(&self, level: pdfcraft_preflight::Level) -> Vec<pdfcraft_preflight::Issue> {
        self.editor.as_ref().map(|e| pdfcraft_preflight::verify(&e.cos, level)).unwrap_or_default()
    }

    /// Standards ▸ Verify PDF/X: what the document declares and what stands between it and the
    /// PDF/X family `family` (the one it declares when `None`). Reads the file; claims nothing.
    pub fn pdfx_verify(&self, family: Option<pdfcraft_preflight::pdfx::Family>) -> pdfcraft_preflight::pdfx::Report {
        match &self.editor {
            Some(e) => pdfcraft_preflight::pdfx::verify(&e.cos, family),
            None => pdfcraft_preflight::pdfx::Report {
                version: None,
                conformance: None,
                declared: None,
                checked: family,
                findings: vec![pdfcraft_preflight::pdfx::Finding {
                    rule: "document",
                    message: "The document has no object graph to check.".into(),
                    page: None,
                }],
            },
        }
    }

    /// The standards the document declares (Standards panel).
    pub fn standards(&self) -> pdfcraft_preflight::Declared {
        self.editor.as_ref().map(|e| pdfcraft_preflight::declared(&e.cos)).unwrap_or_default()
    }

    /// Every word of the document in reading order, with page and box.
    pub fn words(&self) -> Vec<pdfcraft_compare::Word> {
        let config = pdfcraft_render::RenderConfig { password: self.password.as_deref().map(Arc::from), ..Default::default() };
        let mut r = pdfcraft_render::PageRenderer::new(self.bytes.clone(), config);
        let mut out = Vec::new();
        for (page, info) in self.info.pages.iter().enumerate() {
            let res = r.render(pdfcraft_render::RenderRequest { page, kind: pdfcraft_render::RequestKind::Text, scale: 1.0, ..Default::default() });
            if let Some(t) = res.text {
                out.extend(crate::js::page_words(&t, info).into_iter().map(|(text, rect)| pdfcraft_compare::Word { text, page, rect }));
            }
        }
        out
    }
}

impl Session {
    /// Compare files: the text differences from document `old` to document `new`. The result
    /// carries both documents' text, so both must allow copying it.
    pub fn compare(&self, old: DocId, new: DocId) -> Result<Comparison, EditError> {
        self.compare_in(old, new, None, None)
    }

    /// [`Session::compare`] of only the pages in `old_range` of `old` and `new_range` of `new`
    /// (`None`: every page). Only the words on those pages are compared; the pages in the result
    /// are the documents' own (page 7 is page 7, not renumbered from the range), and the word
    /// counts are of the compared pages. An invalid range is an error ([`PageRange::resolve`]).
    pub fn compare_in(&self, old: DocId, new: DocId, old_range: Option<PageRange>, new_range: Option<PageRange>) -> Result<Comparison, EditError> {
        self.compare_in_with(old, new, old_range, new_range, false)
    }

    /// [`Session::compare_in`]; with `ignore_case`, words that differ only in letter case are not
    /// changes (the words in the changes keep their own text). The visual comparison is unaffected.
    pub fn compare_in_with(
        &self,
        old: DocId,
        new: DocId,
        old_range: Option<PageRange>,
        new_range: Option<PageRange>,
        ignore_case: bool,
    ) -> Result<Comparison, EditError> {
        self.compare_in_options(old, new, old_range, new_range, WordOptions { ignore_case, ignore_punctuation: false })
    }

    /// [`Session::compare_in`] with the word differences `options` names left out (case, sentence
    /// punctuation). The words in the changes keep their own text; the visual comparison is unaffected.
    pub fn compare_in_options(
        &self,
        old: DocId,
        new: DocId,
        old_range: Option<PageRange>,
        new_range: Option<PageRange>,
        options: WordOptions,
    ) -> Result<Comparison, EditError> {
        let a = self.get(old).ok_or(EditError::NoDocument)?;
        let b = self.get(new).ok_or(EditError::NoDocument)?;
        let old_pages = old_range.map(|r| r.resolve(a.info.pages.len())).transpose()?;
        let new_pages = new_range.map(|r| r.resolve(b.info.pages.len())).transpose()?;
        a.check_copying()?;
        b.check_copying()?;
        let within = |mut words: Vec<pdfcraft_compare::Word>, pages: Option<std::ops::Range<usize>>| {
            if let Some(p) = pages {
                words.retain(|w| p.contains(&w.page));
            }
            words
        };
        let (old_words, new_words) = (within(a.words(), old_pages.clone()), within(b.words(), new_pages.clone()));
        let mut c = pdfcraft_compare::compare_options(&old_words, &new_words, options);
        // A range without words has no neighbour to place a change by: put it on the range's
        // first page rather than page 1.
        for (words, pages, side) in [(&old_words, &old_pages, 0), (&new_words, &new_pages, 1)] {
            if let (true, Some(p)) = (words.is_empty(), pages) {
                for ch in &mut c.changes {
                    let s = if side == 0 { &mut ch.old } else { &mut ch.new };
                    (s.page, s.last_page) = (p.start, p.start);
                }
            }
        }
        Ok(c)
    }

    /// [`Session::compare`], keeping the changes on `pages` of `new` (0-based; empty: every page).
    pub fn compare_pages(&self, old: DocId, new: DocId, pages: &[usize]) -> Result<Comparison, EditError> {
        let mut c = self.compare(old, new)?;
        if !pages.is_empty() {
            c.changes.retain(|ch| pages.iter().any(|page| (ch.new.page..=ch.new.last_page).contains(page)));
        }
        Ok(c)
    }

    /// Visual compare: regions where page n of `new` looks different from page n of `old`
    /// (rendered at `dpi`), as (page, user-space box in `new`).
    pub fn compare_visual(&self, old: DocId, new: DocId, dpi: f32) -> Result<Vec<(usize, [f64; 4])>, EditError> {
        Ok(self.compare_visual_in(old, new, dpi, None, None)?.into_iter().map(|(_, page, rect)| (page, rect)).collect())
    }

    /// [`Session::compare_visual`] of page ranges: the pages of the ranges are paired by position
    /// (`old_range.from + k` with `new_range.from + k`, for as many pairs as the shorter range
    /// has; no range: the whole document, so page n with page n; a range for one file only: the
    /// other file's same page numbers). Returns (old page, new page,
    /// user-space box in `new`), pages 0-based and the documents' own.
    pub fn compare_visual_in(
        &self,
        old: DocId,
        new: DocId,
        dpi: f32,
        old_range: Option<PageRange>,
        new_range: Option<PageRange>,
    ) -> Result<Vec<(usize, usize, [f64; 4])>, EditError> {
        let a = self.get(old).ok_or(EditError::NoDocument)?;
        let b = self.get(new).ok_or(EditError::NoDocument)?;
        // With a range for one file only, the other file's pages are paired by page number: the
        // same pages (as far as it has them), not its first pages.
        let pages_of = |range: Option<PageRange>, other: Option<PageRange>, count: usize| -> Result<std::ops::Range<usize>, EditError> {
            match (range, other) {
                (Some(r), _) => r.resolve(count),
                (None, Some(o)) => o.resolve(usize::MAX).map(|p| p.start.min(count)..p.end.min(count)),
                (None, None) => Ok(0..count),
            }
        };
        let old_pages = pages_of(old_range, new_range, a.info.pages.len())?;
        let new_pages = pages_of(new_range, old_range, b.info.pages.len())?;
        let renderer = |d: &crate::Document| {
            pdfcraft_render::PageRenderer::new(
                d.bytes.clone(),
                pdfcraft_render::RenderConfig { password: d.password.as_deref().map(Arc::from), ..Default::default() },
            )
        };
        let (mut ra, mut rb) = (renderer(a), renderer(b));
        let scale = dpi.clamp(18.0, 150.0) / 72.0;
        let mut out = Vec::new();
        for (old_page, page) in old_pages.zip(new_pages) {
            let Some(info) = b.info.pages.get(page) else { continue };
            let x = ra.render(pdfcraft_render::RenderRequest { page: old_page, scale, ..Default::default() });
            let y = rb.render(pdfcraft_render::RenderRequest { page, scale, ..Default::default() });
            if x.error.is_some() || y.error.is_some() {
                continue;
            }
            let s = y.width as f32 / info.width.max(1e-3);
            for r in pdfcraft_compare::visual_regions((&y.rgba, y.width, y.height), (&x.rgba, x.width, x.height), 24) {
                let p = info.view_to_user(r[0] as f32 / s, r[1] as f32 / s);
                let q = info.view_to_user(r[2] as f32 / s, r[3] as f32 / s);
                out.push((old_page, page, [p[0].min(q[0]) as f64, p[1].min(q[1]) as f64, p[0].max(q[0]) as f64, p[1].max(q[1]) as f64]));
            }
        }
        Ok(out)
    }

    /// [`Session::compare_visual_in`] telling which regions are comment annotation changes: a
    /// region is [`VisualCause::Annotation`] when it overlaps (by `/Rect`, one point of slack) a
    /// comment (markup) annotation of the old or the new page that the other page lacks, matched
    /// on subtype, contents and rectangle rounded to a whole point. Only the annotations
    /// `DocInfo::annotations` lists count (not form fields, links, popups or media), and at most
    /// [`MAX_COMPARED_ANNOTATIONS`] per page are matched (the rest are ignored, so their
    /// regions stay `Other`). Each page pair is matched once, in time linear in its annotations.
    pub fn compare_visual_regions_in(
        &self,
        old: DocId,
        new: DocId,
        dpi: f32,
        old_range: Option<PageRange>,
        new_range: Option<PageRange>,
    ) -> Result<Vec<VisualRegion>, EditError> {
        let regions = self.compare_visual_in(old, new, dpi, old_range, new_range)?;
        let a = self.get(old).ok_or(EditError::NoDocument)?;
        let b = self.get(new).ok_or(EditError::NoDocument)?;
        type Key<'a> = (&'a str, Option<&'a str>, [i64; 4]);
        fn keyed(d: &crate::Document, page: usize) -> Vec<(Key<'_>, [f64; 4])> {
            d.info
                .annotations
                .iter()
                .filter(|x| x.page == page)
                .take(MAX_COMPARED_ANNOTATIONS)
                .map(|x| ((x.subtype.as_str(), x.contents.as_deref(), x.rect.map(|v| v.round() as i64)), x.rect.map(f64::from)))
                .collect()
        }
        fn unmatched<'a>(mine: &[(Key<'a>, [f64; 4])], other: &[(Key<'a>, [f64; 4])]) -> Vec<[f64; 4]> {
            let seen: std::collections::HashSet<&Key<'a>> = other.iter().map(|(k, _)| k).collect();
            mine.iter().filter(|(k, _)| !seen.contains(k)).map(|(_, r)| *r).collect()
        }
        let mut changed: HashMap<(usize, usize), Vec<[f64; 4]>> = HashMap::new();
        Ok(regions
            .into_iter()
            .map(|(old_page, page, rect)| {
                let rects = changed.entry((old_page, page)).or_insert_with(|| {
                    let (olds, news) = (keyed(a, old_page), keyed(b, page));
                    let mut u = unmatched(&olds, &news);
                    u.extend(unmatched(&news, &olds));
                    u
                });
                let hit = rects.iter().any(|r| {
                    r[0].min(r[2]) - 1.0 <= rect[2]
                        && r[0].max(r[2]) + 1.0 >= rect[0]
                        && r[1].min(r[3]) - 1.0 <= rect[3]
                        && r[1].max(r[3]) + 1.0 >= rect[1]
                });
                VisualRegion { old_page, page, rect, cause: if hit { VisualCause::Annotation } else { VisualCause::Other } }
            })
            .collect())
    }

    /// The compare report as a new PDF (not opened).
    pub fn compare_report(&self, old: DocId, new: DocId) -> Result<Arc<Vec<u8>>, EditError> {
        self.compare_report_of(old, new, &[])
    }

    /// [`Session::compare_report`] of the changes on `pages` ([`Session::compare_pages`]).
    pub fn compare_report_of(&self, old: DocId, new: DocId, pages: &[usize]) -> Result<Arc<Vec<u8>>, EditError> {
        let c = self.compare_pages(old, new, pages)?;
        self.compare_report_for(&c, old, new, (None, None), ChangeKinds::all(), WordOptions::default())
    }

    /// [`Session::compare_report`] of [`Session::compare_in`]: only the pages in each range. The
    /// report names each range beside its file, and every change lists the old and the new
    /// document's own page numbers.
    pub fn compare_report_in(
        &self,
        old: DocId,
        new: DocId,
        old_range: Option<PageRange>,
        new_range: Option<PageRange>,
    ) -> Result<Arc<Vec<u8>>, EditError> {
        self.compare_report_in_kinds(old, new, old_range, new_range, ChangeKinds::all())
    }

    /// [`Session::compare_report_in`] listing only the kinds of change in `kinds`. A report of a
    /// filter says which kinds it shows and how many changes it hid, counts the visual
    /// differences when `kinds.visual` is on (they are never listed), and says that images,
    /// annotations, formatting and background changes are not told apart from them.
    pub fn compare_report_in_kinds(
        &self,
        old: DocId,
        new: DocId,
        old_range: Option<PageRange>,
        new_range: Option<PageRange>,
        kinds: ChangeKinds,
    ) -> Result<Arc<Vec<u8>>, EditError> {
        self.compare_report_in_options(old, new, old_range, new_range, kinds, WordOptions::default())
    }

    /// [`Session::compare_report_in_kinds`] of a comparison that leaves out the word differences
    /// `options` names (case, sentence punctuation). The report says which were ignored.
    pub fn compare_report_in_options(
        &self,
        old: DocId,
        new: DocId,
        old_range: Option<PageRange>,
        new_range: Option<PageRange>,
        kinds: ChangeKinds,
        options: WordOptions,
    ) -> Result<Arc<Vec<u8>>, EditError> {
        if kinds.is_empty() {
            return Err(ChangeKinds::empty_error());
        }
        let c = self.compare_in_options(old, new, old_range, new_range, options)?;
        self.compare_report_for(&c, old, new, (old_range, new_range), kinds, options)
    }

    fn compare_report_for(
        &self,
        c: &Comparison,
        old: DocId,
        new: DocId,
        (old_range, new_range): (Option<PageRange>, Option<PageRange>),
        kinds: ChangeKinds,
        options: WordOptions,
    ) -> Result<Arc<Vec<u8>>, EditError> {
        let name = |id, range: Option<PageRange>| {
            let n = self.get(id).map(|d| d.name.clone()).unwrap_or_default();
            match range {
                Some(r) if r.from == r.to => format!("{n} (page {})", r.from),
                Some(r) => format!("{n} (pages {} to {})", r.from, r.to),
                None => n,
            }
        };
        let mut notes = Vec::new();
        if options != WordOptions::default() {
            let mut ignored = Vec::new();
            if options.ignore_case {
                ignored.push("letter case");
            }
            if options.ignore_punctuation {
                ignored.push("sentence punctuation");
            }
            notes.push(format!("Ignored in the text comparison: {}.", ignored.join(", ")));
        }
        let shown = if kinds.is_all() { c.clone() } else { kinds.filter(c.clone()) };
        if !kinds.is_all() {
            notes.push(format!("Kinds of change shown: {}.", kinds.names().join(", ")));
            let hidden: Vec<String> = [Kind::Replaced, Kind::Inserted, Kind::Deleted]
                .into_iter()
                .filter(|k| !kinds.wants(*k) && c.count(*k) > 0)
                .map(|k| format!("{} {}", c.count(k), k.label().to_lowercase()))
                .collect();
            if !hidden.is_empty() {
                notes.push(format!("Hidden by the filter: {}.", hidden.join(", ")));
            }
            if kinds.visual {
                // A page that fails to render is left out of the count, as the panel does.
                let n = self.compare_visual_in(old, new, 72.0, old_range, new_range).unwrap_or_default().len();
                notes.push(format!("Visual differences (counted here, shaded in the viewer, never listed): {n}."));
            }
            notes.push(
                "Images, annotations, formatting and background or header/footer changes are not told apart: the comparison finds text changes and pixel differences only, so all of them count as visual."
                    .to_string(),
            );
        }
        let text = pdfcraft_compare::report_with_notes(&shown, &name(old, old_range), &name(new, new_range), &notes, c.identical());
        self.create_report("Compare Report", &text)
    }

    /// Mark the differences in `new` as comments: highlights over replaced and inserted text
    /// (blue, green) and a note where text was deleted (red), authored "Compare". One undoable
    /// step; returns how many comments were added.
    pub fn mark_differences(&mut self, old: DocId, new: DocId) -> Result<usize, EditError> {
        self.mark_differences_of(old, new, &[])
    }

    /// [`Session::mark_differences`] of the changes on `pages` ([`Session::compare_pages`]).
    pub fn mark_differences_of(&mut self, old: DocId, new: DocId, pages: &[usize]) -> Result<usize, EditError> {
        let c = self.compare_pages(old, new, pages)?;
        self.mark_comparison(new, &c)
    }

    /// [`Session::mark_differences`] of [`Session::compare_in`]: only the changes between the
    /// pages in each range, placed on the newer document's own pages.
    pub fn mark_differences_in(
        &mut self,
        old: DocId,
        new: DocId,
        old_range: Option<PageRange>,
        new_range: Option<PageRange>,
    ) -> Result<usize, EditError> {
        self.mark_differences_in_kinds(old, new, old_range, new_range, ChangeKinds::all())
    }

    /// [`Session::mark_differences_in`] of the kinds of text change in `kinds`. Visual
    /// differences are never marked (they are regions, with no text to quote), so `kinds.visual`
    /// adds nothing here; a filter of only `visual` marks nothing.
    pub fn mark_differences_in_kinds(
        &mut self,
        old: DocId,
        new: DocId,
        old_range: Option<PageRange>,
        new_range: Option<PageRange>,
        kinds: ChangeKinds,
    ) -> Result<usize, EditError> {
        self.mark_differences_in_options(old, new, old_range, new_range, kinds, WordOptions::default())
    }

    /// [`Session::mark_differences_in_kinds`] of a comparison that leaves out the word
    /// differences `options` names (case, sentence punctuation).
    pub fn mark_differences_in_options(
        &mut self,
        old: DocId,
        new: DocId,
        old_range: Option<PageRange>,
        new_range: Option<PageRange>,
        kinds: ChangeKinds,
        options: WordOptions,
    ) -> Result<usize, EditError> {
        if kinds.is_empty() {
            return Err(ChangeKinds::empty_error());
        }
        let c = kinds.filter(self.compare_in_options(old, new, old_range, new_range, options)?);
        self.mark_comparison(new, &c)
    }

    fn mark_comparison(&mut self, new: DocId, c: &Comparison) -> Result<usize, EditError> {
        let mut edits = Vec::new();
        for ch in &c.changes {
            let color = colour(ch.kind);
            let contents = match ch.kind {
                Kind::Replaced => format!("Replaced: \"{}\" with \"{}\"", ch.old.text, ch.new.text),
                Kind::Inserted => format!("Inserted: \"{}\"", ch.new.text),
                Kind::Deleted => format!("Deleted: \"{}\"", ch.old.text),
            };
            let shape = if ch.new.rects.is_empty() {
                let Some(r) = ch.new.near else { continue };
                Shape::Note { at: [r[0], r[3]], icon: NoteIcon::Note }
            } else {
                Shape::TextMarkup {
                    kind: Markup::Highlight,
                    quads: ch.new.rects.iter().map(|r| [r[0], r[3], r[2], r[3], r[0], r[1], r[2], r[1]]).collect(),
                }
            };
            let mut style = crate::Style::default_for(&shape);
            style.color = color;
            edits.push(Edit::AddAnnotation(NewAnnotation { page: ch.new.page, shape, style, contents, author: "Compare".into() }));
        }
        let n = edits.len();
        if n > 0 {
            self.apply(new, Edit::Batch { label: "Mark differences".into(), edits })?;
        }
        Ok(n)
    }
}

/// Export a PDF ▸ Word, PowerPoint, HTML or RTF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OfficeFormat {
    Docx,
    Pptx,
    Html,
    /// CSS-free HTML that preserves text and inline images but drops colours and layout styling.
    HtmlSimple,
    Rtf,
    Markdown,
}

impl OfficeFormat {
    pub fn extension(self) -> &'static str {
        match self {
            OfficeFormat::Docx => "docx",
            OfficeFormat::Pptx => "pptx",
            OfficeFormat::Html => "html",
            OfficeFormat::HtmlSimple => "html",
            OfficeFormat::Rtf => "rtf",
            OfficeFormat::Markdown => "md",
        }
    }

    pub fn from_extension(ext: &str) -> Option<OfficeFormat> {
        match ext.to_ascii_lowercase().as_str() {
            "docx" => Some(OfficeFormat::Docx),
            "pptx" => Some(OfficeFormat::Pptx),
            "html" | "htm" => Some(OfficeFormat::Html),
            "rtf" => Some(OfficeFormat::Rtf),
            "md" | "markdown" => Some(OfficeFormat::Markdown),
            _ => None,
        }
    }
}

/// A Word, PowerPoint, HTML, RTF or Markdown export ([`crate::Document::export_office_checked`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OfficeExport {
    /// The exported file.
    pub bytes: Vec<u8>,
    /// Pages (0-based) that draw forms nested deeper than [`MAX_FORM_DEPTH`] or more of them
    /// than [`MAX_FORM_VISITS`], or pictures past [`MAX_EXPORT_IMAGE_BYTES`] or past what one export
    /// may decode: export stops reading there to stay bounded on hostile files, so the text and images of those forms, or
    /// those pictures, are missing from `bytes`. RTF also lists the pages that lost a picture to
    /// its own limits (it repeats a picture's data at every placement) or one whose rotation it
    /// couldn't bake in; their other content is kept. A picture no format can encode (a JPX image,
    /// say) is left out of every format without being listed.
    pub truncated_pages: Vec<usize>,
}

/// An image object prepared for Office export: its file extension and encoded bytes, or why it is
/// left out (`Err(true)`: past [`MAX_EXPORT_IMAGE_BYTES`], or what the export may decode;
/// `Err(false)`: it can't be encoded).
type PreparedImage = Result<(&'static str, Arc<[u8]>), bool>;

/// Displayed points per user-space unit on `info`'s page (`/UserUnit`), 1 when the page has no
/// usable size.
pub(crate) fn display_unit(info: &pdfcraft_render::PageInfo) -> f64 {
    let crop_width = f64::from(info.crop[2] - info.crop[0]);
    let display_width = f64::from(if info.rotation.is_multiple_of(180) { info.width } else { info.height });
    if crop_width > 0.0 && crop_width.is_finite() && display_width.is_finite() { display_width / crop_width } else { 1.0 }
}

/// Bold and italic as a font's name says them (`Arial-BoldItalicMT`).
pub(crate) fn weight_and_slant(base_font: &str) -> (bool, bool) {
    let f = base_font.to_ascii_lowercase();
    (f.contains("bold") || f.contains("black") || f.contains("heavy"), f.contains("italic") || f.contains("oblique"))
}

impl crate::Document {
    /// The pages as paragraphs and images (for Word, HTML and RTF export), including what
    /// form XObjects draw.
    pub fn export_pages(&self) -> Vec<pdfcraft_export::Page> {
        self.export_pages_checked(true, MAX_EXPORT_IMAGE_BYTES).0
    }

    /// [`Self::export_pages`], and the pages whose forms were left partly unread or whose
    /// pictures went past `image_budget` bytes ([`OfficeExport::truncated_pages`]). Without
    /// `images` no picture is prepared.
    pub(crate) fn export_pages_checked(&self, images: bool, image_budget: usize) -> (Vec<pdfcraft_export::Page>, Vec<usize>) {
        let Some(editor) = self.editor.as_ref() else { return (Vec::new(), Vec::new()) };
        self.export_pages_within(images, image_budget, &mut pdfcraft_cos::ImageBudget::new(&editor.cos))
    }

    /// [`Self::export_pages_checked`], its pictures decoded within `image_work`, which is shared
    /// by the whole export: a picture past it is left out like one past `image_budget`.
    pub(crate) fn export_pages_within(
        &self,
        images: bool,
        image_budget: usize,
        image_work: &mut pdfcraft_cos::ImageBudget,
    ) -> (Vec<pdfcraft_export::Page>, Vec<usize>) {
        let Some(cos) = self.editor.as_ref().map(|e| &e.cos) else { return (Vec::new(), Vec::new()) };
        let mut truncated = Vec::new();
        // Each image object is encoded once and shared by every place that draws it.
        let mut encoded: HashMap<ObjRef, PreparedImage> = HashMap::new();
        let mut image_bytes = image_budget;
        let pages = self
            .info
            .pages
            .iter()
            .enumerate()
            .map(|(i, info)| {
                // Export uses the visible page in points, y up. Raw PDF coordinates include
                // CropBox offsets and precede /Rotate and /UserUnit.
                let visible_rect = |r: [f64; 4]| {
                    let a = info.user_to_view(r[0] as f32, r[1] as f32);
                    let b = info.user_to_view(r[2] as f32, r[3] as f32);
                    [a[0].min(b[0]) as f64, (info.height - a[1].max(b[1])) as f64, a[0].max(b[0]) as f64, (info.height - a[1].min(b[1])) as f64]
                };
                let unit = display_unit(info);
                let reading = pdfcraft_edit::reading_blocks_checked(cos, i).unwrap_or_default();
                let forms_unread = reading.truncated;
                let blocks = reading
                    .blocks
                    .into_iter()
                    .filter(|b| !b.text.trim().is_empty())
                    .map(|b| {
                        let (bold, italic) = weight_and_slant(&b.base_font);
                        // `text` is trimmed; the editor text keeps the source's own spaces.
                        let space_around = [b.editor_text.starts_with(char::is_whitespace), b.editor_text.ends_with(char::is_whitespace)];
                        pdfcraft_export::Block {
                            space_around,
                            text: b.text,
                            rect: visible_rect(b.rect),
                            size: b.size * unit,
                            bold,
                            italic,
                            color: b.color,
                            rotation: b.rotation - f64::from(info.rotation),
                        }
                    })
                    .collect();
                let placed = if images { pdfcraft_edit::reading_images(cos, i).unwrap_or_default() } else { Vec::new() };
                let mut over_budget = false;
                let images = placed
                    .iter()
                    .filter_map(|im| {
                        let object = im.object?;
                        let prepared = encoded.entry(object).or_insert_with(|| {
                            let refused = image_work.refused();
                            let (ext, bytes) =
                                pdfcraft_create::image_file_within(cos, object, &mut *image_work).map_err(|_| image_work.refused() > refused)?;
                            image_bytes = image_bytes.checked_sub(bytes.len()).ok_or(true)?;
                            Ok((if ext == "jpg" { "jpg" } else { "png" }, Arc::from(bytes)))
                        });
                        over_budget |= matches!(prepared, Err(true));
                        let (ext, bytes) = prepared.clone().ok()?;
                        Some(pdfcraft_export::Image {
                            ext,
                            bytes,
                            rect: visible_rect(im.rect),
                            size: [im.matrix[0].hypot(im.matrix[1]) * unit, im.matrix[2].hypot(im.matrix[3]) * unit],
                            rotation: im.matrix[1].atan2(im.matrix[0]).to_degrees() - f64::from(info.rotation),
                        })
                    })
                    .collect();
                if forms_unread || over_budget {
                    truncated.push(i);
                }
                // The page colour (Edit a PDF ▸ Background, a producer's page fill) for formats that keep one.
                let background = pdfcraft_edit::page_background(cos, i).ok().flatten();
                // The ruled lines tell a drawn table from text that merely lines up (#5149).
                let rules = pdfcraft_edit::page_rules(cos, i).unwrap_or_default().into_iter().map(visible_rect).collect();
                pdfcraft_export::Page { width: info.width as f64, height: info.height as f64, blocks, images, background, rules }
            })
            .collect();
        (pages, truncated)
    }

    /// The document as a Word, PowerPoint, HTML or RTF file.
    pub fn export_office(&self, format: OfficeFormat) -> Vec<u8> {
        self.export_office_checked(format).bytes
    }

    /// [`Self::export_office`], and the pages whose text it had to leave out
    /// ([`OfficeExport::truncated_pages`]).
    pub fn export_office_checked(&self, format: OfficeFormat) -> OfficeExport {
        // Markdown leaves pictures out.
        let (pages, mut truncated_pages) = self.export_pages_checked(!matches!(format, OfficeFormat::Markdown), MAX_EXPORT_IMAGE_BYTES);
        let title = self.info.title.clone().unwrap_or_else(|| self.name.trim_end_matches(".pdf").to_string());
        // Word and HTML name the document's language (`/Lang`), so spelling is checked in it.
        let language = self.initial_view().language;
        let bytes = match format {
            OfficeFormat::Docx => pdfcraft_export::docx_with_language(&pages, &title, language.as_deref()),
            OfficeFormat::Pptx => pdfcraft_export::pptx(&pages, &title),
            OfficeFormat::Html => pdfcraft_export::html_with_language(&pages, &title, language.as_deref()).into_bytes(),
            OfficeFormat::HtmlSimple => pdfcraft_export::html_simple_with_language(&pages, &title, language.as_deref()).into_bytes(),
            OfficeFormat::Rtf => {
                let (rtf, omitted) = pdfcraft_export::rtf_with_title_checked(&pages, &title);
                truncated_pages.extend(omitted);
                truncated_pages.sort_unstable();
                truncated_pages.dedup();
                rtf.into_bytes()
            }
            OfficeFormat::Markdown => pdfcraft_export::markdown(&pages).into_bytes(),
        };
        OfficeExport { bytes, truncated_pages }
    }
}
