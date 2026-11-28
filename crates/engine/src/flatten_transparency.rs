//! Flatten transparency: each page whose own content paints transparently becomes an opaque
//! picture of that content. Comments and form fields stay on top, as they were.
//!
//! The whole page turns into pixels: its text can no longer be selected or searched, content of
//! hidden layers is gone, and its tags no longer point at anything.

use std::sync::Arc;

use pdfcraft_cos::{Dict, ObjRef, Object, SaveOptions, Stream, write_full};
use pdfcraft_render::{PageRenderer, RenderConfig};

use crate::edit::check_permission;
use crate::spooler::{FLATTEN_DPI, MAX_FLATTENED_BYTES, Picture, sheet_picture};
use crate::{DocId, Edit, EditError, Session, guard};

/// The resolutions the pictures can be made at, in pixels per inch.
pub const DPI_RANGE: std::ops::RangeInclusive<f64> = 72.0..=600.0;

/// What flattening did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlattenReport {
    /// Pages (0-based) turned into pictures, in order.
    pub pages: Vec<usize>,
    /// Pages left alone because their content paints nothing transparently.
    pub opaque: Vec<usize>,
    /// Pages left alone because they could not be drawn, or their picture was too large.
    pub failed: Vec<usize>,
}

/// One page's picture: the content that draws it and its images.
#[derive(Clone, Debug, PartialEq)]
pub struct FlatPage {
    pub(crate) page: usize,
    pub(crate) object: ObjRef,
    pub(crate) content: Vec<u8>,
    pub(crate) tiles: Vec<(Vec<u8>, Stream)>,
}

/// What flattening needs, independent of the session.
pub struct FlattenJob {
    cos: pdfcraft_cos::Document,
    config: RenderConfig,
    /// The pages that paint transparently.
    targets: Vec<usize>,
    dpi: f64,
    /// The document's edit generation it was taken at: a result is for that state only.
    generation: u64,
    report: FlattenReport,
}

/// The pictures, ready for [`Session::apply_flatten_transparency`], and the report.
pub struct Flattened {
    pages: Vec<FlatPage>,
    generation: u64,
    pub report: FlattenReport,
}

impl Session {
    /// Capture what flattening transparency on `pages` (0-based; empty = all) of document `id`
    /// needs, with pictures at `dpi` pixels per inch ([`DPI_RANGE`]). Errors when the document
    /// can't be changed or a page doesn't exist.
    pub fn flatten_transparency_job(&self, id: DocId, pages: &[usize], dpi: f64) -> Result<FlattenJob, EditError> {
        let doc = self.get(id).ok_or(EditError::NoDocument)?;
        if let Some(why) = &doc.read_only_reason {
            return Err(EditError::ReadOnly(why.clone()));
        }
        let editor = doc.editor.as_ref().ok_or_else(|| EditError::ReadOnly(String::new()))?;
        if let Some(p) = editor.cos.permissions() {
            check_permission(&Edit::FlattenTransparency { pages: Vec::new() }, &p)?;
        }
        if !DPI_RANGE.contains(&dpi) {
            return Err(EditError::Invalid(format!("the resolution must be from {} to {} pixels per inch", DPI_RANGE.start(), DPI_RANGE.end())));
        }
        let list = pdfcraft_model::pages(&editor.cos);
        let all = list.len();
        let mut wanted: Vec<usize> = if pages.is_empty() { (0..all).collect() } else { pages.to_vec() };
        if let Some(p) = wanted.iter().find(|p| **p >= all) {
            return Err(EditError::Invalid(format!("there is no page {}: the document has {all}", p.saturating_add(1))));
        }
        wanted.sort_unstable();
        wanted.dedup();
        let mut report = FlattenReport::default();
        let mut targets = Vec::new();
        for i in wanted {
            let Some(page) = list.get(i) else { continue };
            // What annotations paint stays with them, so only the page's own content counts.
            let mut own = page.dict.clone();
            own.remove(b"Annots");
            if pdfcraft_preflight::page_uses_transparency(&editor.cos, &own) {
                targets.push(i);
            } else {
                report.opaque.push(i);
            }
        }
        // Drawn as shown on screen (the layers as they are), without display-only settings.
        let config = RenderConfig { password: doc.config.password.clone(), layers: doc.config.layers.clone(), ..RenderConfig::default() };
        Ok(FlattenJob { cos: editor.cos.clone(), config, targets, dpi, generation: doc.edit_generation(), report })
    }

    /// Write what [`FlattenJob::run`] drew into document `id` as one undoable step, "Flatten
    /// transparency" (none when no page was drawn), and return the report. Refused when the
    /// document changed since the job was taken.
    pub fn apply_flatten_transparency(&mut self, id: DocId, flat: Flattened) -> Result<FlattenReport, EditError> {
        let doc = self.get(id).ok_or(EditError::NoDocument)?;
        if doc.edit_generation() != flat.generation {
            return Err(EditError::Invalid("the document changed while it was being flattened; nothing was changed, try again".into()));
        }
        if !flat.pages.is_empty() {
            self.apply(id, Edit::FlattenTransparency { pages: flat.pages })?;
        }
        Ok(flat.report)
    }

    /// [`Self::flatten_transparency_job`], run, then [`Self::apply_flatten_transparency`] (the
    /// CLI and agents use this).
    pub fn flatten_transparency(&mut self, id: DocId, pages: &[usize], dpi: f64) -> Result<FlattenReport, EditError> {
        let job = self.flatten_transparency_job(id, pages, dpi)?;
        let flat = job.run(|_, _| true)?;
        self.apply_flatten_transparency(id, flat)
    }
}

impl FlattenJob {
    /// The pages to draw.
    pub fn total(&self) -> usize {
        self.targets.len()
    }

    /// Draw every page that needs it. `progress(done, total)` is called before each page;
    /// returning `false` stops with [`EditError::Cancelled`]. A panic is returned as an error,
    /// so a worker thread always reports back.
    pub fn run(self, mut progress: impl FnMut(usize, usize) -> bool) -> Result<Flattened, EditError> {
        guard(move || self.work(&mut progress)).map_err(|m| EditError::Invalid(format!("flattening failed unexpectedly ({m})")))?
    }

    fn work(self, progress: &mut dyn FnMut(usize, usize) -> bool) -> Result<Flattened, EditError> {
        let FlattenJob { cos, config, targets, dpi, generation, mut report } = self;
        let mut done = Vec::new();
        if targets.is_empty() {
            return Ok(Flattened { pages: done, generation, report });
        }
        let list = pdfcraft_model::pages(&cos);
        // The pictures are of the content alone: draw a copy with the annotations taken off.
        let mut bare = cos.clone();
        for page in targets.iter().filter_map(|i| list.get(*i)) {
            bare.update_dict(page.obj, |d| {
                d.remove(b"Annots");
            })
            .map_err(|e| EditError::Invalid(e.to_string()))?;
        }
        let bytes = write_full(&bare, &SaveOptions::default()).map_err(|e| EditError::Invalid(e.to_string()))?;
        let mut renderer = PageRenderer::new(Arc::new(bytes), config);
        if renderer.page_count() != list.len() {
            return Err(EditError::Invalid("the pages could not be drawn".into()));
        }
        let total = targets.len();
        let mut budget = MAX_FLATTENED_BYTES;
        for (n, i) in targets.into_iter().enumerate() {
            if !progress(n, total) {
                return Err(EditError::Cancelled);
            }
            let Some(page) = list.get(i) else { continue };
            match sheet_picture(&bare, &mut renderer, page, i, dpi, &mut budget) {
                Some(Picture { content, tiles }) => {
                    report.pages.push(i);
                    done.push(FlatPage { page: i, object: page.obj, content, tiles });
                }
                None => report.failed.push(i),
            }
        }
        Ok(Flattened { pages: done, generation, report })
    }
}

/// [`Edit::FlattenTransparency`]: each page draws its picture and nothing else.
pub(crate) fn write_back(doc: &mut pdfcraft_cos::Document, pages: &[FlatPage]) -> Result<(), EditError> {
    let list = pdfcraft_model::pages(doc);
    for flat in pages {
        if list.get(flat.page).map(|p| p.obj) != Some(flat.object) {
            return Err(EditError::Invalid(format!("page {} is no longer the one that was drawn", flat.page.saturating_add(1))));
        }
        let mut xobjects = Dict::new();
        for (name, image) in &flat.tiles {
            xobjects.set(name.clone(), Object::Ref(doc.add(Object::Stream(image.clone()))));
        }
        let mut resources = Dict::new();
        resources.set("XObject", Object::Dict(xobjects));
        let contents = doc.add(Object::Stream(Stream::flate(Dict::new(), &flat.content)));
        doc.update_dict(flat.object, |d| {
            d.set("Contents", Object::Ref(contents));
            d.set("Resources", Object::Dict(resources));
            d.remove(b"Group");
            d.remove(b"StructParents");
        })
        .map_err(|e| EditError::Invalid(e.to_string()))?;
    }
    Ok(())
}

/// The resolution offered first.
pub const DEFAULT_DPI: f64 = FLATTEN_DPI;
