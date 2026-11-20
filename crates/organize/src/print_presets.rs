//! Document Properties ▸ Advanced ▸ Print Dialog Presets: what the Print dialog starts with for
//! this document (ISO 32000-2 §12.2 Table 147: `/PrintScaling`, `/Duplex`, `/PickTrayByPDFSize`,
//! `/PrintPageRange`, `/NumCopies`, all in the catalog's `/ViewerPreferences`).
//!
//! Only `/PrintScaling` has a default the specification states (`/AppDefault`); for the others,
//! no entry means the viewer decides, which is not the same as an explicit value, so `None` here
//! is "no entry" and a stored `false` is kept as `Some(false)`.
//!
//! An entry that is there but is not a value this module accepts is read as no entry and named by
//! [`read_print_presets`]; [`set_print_presets`] leaves it as it is unless the caller asks for a
//! value other than what is read for that setting. Reading copies nothing from the file: it
//! borrows the catalog and the dictionary, and looks at the length of an array before it looks at
//! what is in it.

use pdfcraft_cos::{Dict, Document, ObjRef, Object};
use std::ops::Deref;
use std::sync::Arc;

use crate::OrganizeError;

/// Most page ranges read from, or written to, `/PrintPageRange` (a longer array is read as
/// unreadable without looking at its items, and is never copied).
pub const MAX_PRINT_RANGES: usize = 256;
/// Least and most copies `/NumCopies` is read or written with: Table 147 lists the integers 2
/// through 5 as the supported values. A stored 1, or anything else, is read as unreadable.
pub const MIN_PRINT_COPIES: u8 = 2;
pub const MAX_PRINT_COPIES: u8 = 5;

/// `/PrintScaling`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrintScaling {
    /// The viewer's own choice (`/AppDefault`, also what no entry means).
    #[default]
    AppDefault,
    /// No scaling (`/None`).
    None,
}

/// `/Duplex`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Duplex {
    Simplex,
    FlipShortEdge,
    FlipLongEdge,
}

impl Duplex {
    fn name(self) -> &'static str {
        match self {
            Duplex::Simplex => "Simplex",
            Duplex::FlipShortEdge => "DuplexFlipShortEdge",
            Duplex::FlipLongEdge => "DuplexFlipLongEdge",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrintPresets {
    pub scaling: PrintScaling,
    /// `None` when the document has no preference.
    pub duplex: Option<Duplex>,
    /// `/PickTrayByPDFSize`: whether the paper source is chosen by the size of the pages. `None`
    /// when there is no entry (the viewer decides); `Some(false)` is an explicit no.
    pub pick_tray_by_size: Option<bool>,
    /// 1-based inclusive page ranges `(first, last)` in the order stored; empty means no entry
    /// (the viewer decides, usually all pages).
    pub page_ranges: Vec<(usize, usize)>,
    /// `/NumCopies`, [`MIN_PRINT_COPIES`] to [`MAX_PRINT_COPIES`]; `None` when there is no entry.
    pub copies: Option<u8>,
}

/// An object that is borrowed when it is direct and shared when it was a reference, so that
/// neither is copied.
enum Held<'a> {
    Direct(&'a Object),
    Resolved(Arc<Object>),
}

impl Deref for Held<'_> {
    type Target = Object;
    fn deref(&self) -> &Object {
        match self {
            Held::Direct(o) => o,
            Held::Resolved(o) => o,
        }
    }
}

fn held<'a>(doc: &Document, o: &'a Object) -> Held<'a> {
    match o {
        Object::Ref(_) => Held::Resolved(doc.resolve(o)),
        o => Held::Direct(o),
    }
}

/// What `/ViewerPreferences` is in the catalog.
enum Prefs {
    /// No entry, a null, or a reference to nothing: a writer starts an empty dictionary.
    Absent,
    /// A dictionary written in the catalog.
    Direct,
    /// A reference to a dictionary object.
    Indirect(ObjRef),
    /// Anything else, which is read as no preferences and never replaced.
    Other,
}

fn prefs_kind(doc: &Document, catalog: &Dict) -> Prefs {
    match catalog.get(b"ViewerPreferences") {
        None | Some(Object::Null) => Prefs::Absent,
        Some(Object::Dict(_)) => Prefs::Direct,
        Some(Object::Ref(r)) => match &*doc.get(*r) {
            Object::Dict(_) => Prefs::Indirect(*r),
            Object::Null => Prefs::Absent,
            _ => Prefs::Other,
        },
        Some(_) => Prefs::Other,
    }
}

/// The presets as stored, and the names of the settings whose entry is there but unusable
/// (`viewer_preferences` when the dictionary itself is not a dictionary).
fn read(doc: &Document) -> (PrintPresets, Vec<&'static str>) {
    let mut p = PrintPresets::default();
    let mut bad = Vec::new();
    let Some(root) = doc.root() else { return (p, bad) };
    let catalog_object = doc.get(root);
    let Some(catalog) = catalog_object.as_dict() else { return (p, bad) };
    let target;
    let vp: &Dict = match (prefs_kind(doc, catalog), catalog.get(b"ViewerPreferences")) {
        (Prefs::Absent, _) => return (p, bad),
        (Prefs::Other, _) => {
            bad.push("viewer_preferences");
            return (p, bad);
        }
        (Prefs::Direct, Some(Object::Dict(d))) => d,
        (Prefs::Indirect(r), _) => {
            target = doc.get(r);
            match target.as_dict() {
                Some(d) => d,
                None => return (p, bad),
            }
        }
        _ => return (p, bad),
    };
    let value = |k: &[u8]| vp.get(k).map(|o| held(doc, o));
    match value(b"PrintScaling").as_deref() {
        None => {}
        Some(Object::Name(n)) if n.as_slice() == b"AppDefault" => {}
        Some(Object::Name(n)) if n.as_slice() == b"None" => p.scaling = PrintScaling::None,
        Some(_) => bad.push("scaling"),
    }
    match value(b"Duplex").as_deref() {
        None => {}
        Some(Object::Name(n)) if n.as_slice() == b"Simplex" => p.duplex = Some(Duplex::Simplex),
        Some(Object::Name(n)) if n.as_slice() == b"DuplexFlipShortEdge" => p.duplex = Some(Duplex::FlipShortEdge),
        Some(Object::Name(n)) if n.as_slice() == b"DuplexFlipLongEdge" => p.duplex = Some(Duplex::FlipLongEdge),
        Some(_) => bad.push("duplex"),
    }
    match value(b"PickTrayByPDFSize").as_deref() {
        None => {}
        Some(Object::Bool(b)) => p.pick_tray_by_size = Some(*b),
        Some(_) => bad.push("pick_tray_by_size"),
    }
    match value(b"NumCopies").as_deref() {
        None => {}
        Some(Object::Int(n)) if (i64::from(MIN_PRINT_COPIES)..=i64::from(MAX_PRINT_COPIES)).contains(n) => p.copies = u8::try_from(*n).ok(),
        Some(_) => bad.push("copies"),
    }
    match value(b"PrintPageRange").as_deref() {
        None => {}
        Some(Object::Array(items)) => match ranges(doc, items) {
            Some(r) => p.page_ranges = r,
            None => bad.push("page_ranges"),
        },
        Some(_) => bad.push("page_ranges"),
    }
    (p, bad)
}

/// The pairs of a `/PrintPageRange` array: whole numbers, `1 <= first <= last`, at most
/// [`MAX_PRINT_RANGES`] of them. Anything else is `None`.
fn ranges(doc: &Document, items: &[Object]) -> Option<Vec<(usize, usize)>> {
    if !items.len().is_multiple_of(2) || items.len() / 2 > MAX_PRINT_RANGES {
        return None;
    }
    let mut out = Vec::with_capacity(items.len() / 2);
    for pair in items.chunks(2) {
        let n = |o: &Object| match &*held(doc, o) {
            Object::Int(i) => usize::try_from(*i).ok().filter(|i| *i >= 1),
            _ => None,
        };
        let (first, last) = (n(pair.first()?)?, n(pair.get(1)?)?);
        if first > last {
            return None;
        }
        out.push((first, last));
    }
    Some(out)
}

/// The presets as stored and the settings (`viewer_preferences`, `scaling`, `duplex`,
/// `pick_tray_by_size`, `copies`, `page_ranges`) whose entry is there but is not a value this
/// module accepts, which is read as no entry. One read of the catalog.
pub fn read_print_presets(doc: &Document) -> (PrintPresets, Vec<&'static str>) {
    read(doc)
}

/// The document's print dialog presets as stored.
pub fn print_presets(doc: &Document) -> PrintPresets {
    read(doc).0
}

/// The names [`read_print_presets`] gives as unreadable.
pub fn unreadable_print_presets(doc: &Document) -> Vec<&'static str> {
    read(doc).1
}

/// Write the presets. Only a setting that differs from what [`read_print_presets`] reads changes,
/// so an entry this module cannot read stays unless a value other than "no entry" is asked for.
/// "No entry" is written by removing the entry (an explicit `false` for `pick_tray_by_size` is
/// kept as `false`); an empty `/ViewerPreferences` goes. A null or dangling `/ViewerPreferences`
/// counts as absent and gets a new dictionary; a dictionary behind a reference is changed where
/// it is, and one written in the catalog is changed there. Nothing is written when a value is
/// refused: more than [`MAX_PRINT_RANGES`] ranges, a range that is not `1 <= first <= last <=
/// pages` (checked only for ranges that differ from what is stored), copies outside
/// [`MIN_PRINT_COPIES`] to [`MAX_PRINT_COPIES`], or a `/ViewerPreferences` that is not a dictionary.
pub fn set_print_presets(doc: &mut Document, p: &PrintPresets) -> Result<(), OrganizeError> {
    let invalid = |m: String| OrganizeError::Invalid(m);
    if let Some(c) = p.copies.filter(|c| !(MIN_PRINT_COPIES..=MAX_PRINT_COPIES).contains(c)) {
        return Err(invalid(format!("copies must be between {MIN_PRINT_COPIES} and {MAX_PRINT_COPIES}, not {c}")));
    }
    if p.page_ranges.len() > MAX_PRINT_RANGES {
        return Err(invalid(format!("a print page range list holds at most {MAX_PRINT_RANGES} ranges, not {}", p.page_ranges.len())));
    }
    let (now, _) = read(doc);
    if *p == now {
        return Ok(());
    }
    // Ranges already stored are not checked against the page count again: only a change is.
    if p.page_ranges != now.page_ranges {
        let pages = crate::walk(doc)?.len();
        for (first, last) in &p.page_ranges {
            if *first < 1 || first > last {
                return Err(invalid(format!("print page range {first}-{last} must start at page 1 or later and not end before it starts")));
            }
            if *last > pages {
                return Err(invalid(format!("print page range {first}-{last} goes past the last page ({pages})")));
            }
        }
    }
    let root = doc.root().ok_or(OrganizeError::NoPageTree)?;
    let kind = {
        let catalog = doc.get(root);
        prefs_kind(doc, catalog.as_dict().ok_or(OrganizeError::NoPageTree)?)
    };
    if matches!(kind, Prefs::Other) {
        return Err(invalid("the document's /ViewerPreferences is not a dictionary, so it was left as it is".into()));
    }
    let mut ops: Vec<(&'static [u8], Option<Object>)> = Vec::new();
    if p.scaling != now.scaling {
        ops.push((b"PrintScaling", (p.scaling == PrintScaling::None).then(|| Object::name("None"))));
    }
    if p.duplex != now.duplex {
        ops.push((b"Duplex", p.duplex.map(|d| Object::name(d.name()))));
    }
    if p.pick_tray_by_size != now.pick_tray_by_size {
        ops.push((b"PickTrayByPDFSize", p.pick_tray_by_size.map(Object::Bool)));
    }
    if p.copies != now.copies {
        ops.push((b"NumCopies", p.copies.map(|c| Object::Int(i64::from(c)))));
    }
    if p.page_ranges != now.page_ranges {
        let flat = |(a, b): &(usize, usize)| [i64::try_from(*a).unwrap_or(i64::MAX), i64::try_from(*b).unwrap_or(i64::MAX)];
        ops.push((
            b"PrintPageRange",
            (!p.page_ranges.is_empty()).then(|| Object::Array(p.page_ranges.iter().flat_map(flat).map(Object::Int).collect())),
        ));
    }
    let apply = |d: &mut Dict| {
        for (key, value) in &ops {
            match value {
                Some(v) => d.set(key.to_vec(), v.clone()),
                None => {
                    d.remove(key);
                }
            }
        }
    };
    match kind {
        Prefs::Indirect(r) => {
            doc.update_dict(r, apply)?;
            if matches!(&*doc.get(r), Object::Dict(d) if d.is_empty()) {
                doc.update_dict(root, |c| {
                    c.remove(b"ViewerPreferences");
                })?;
            }
        }
        Prefs::Direct => doc.update_dict(root, |c| {
            let emptied = match c.get_mut(b"ViewerPreferences") {
                Some(Object::Dict(d)) => {
                    apply(d);
                    d.is_empty()
                }
                _ => false,
            };
            if emptied {
                c.remove(b"ViewerPreferences");
            }
        })?,
        Prefs::Absent | Prefs::Other => {
            let mut d = Dict::new();
            apply(&mut d);
            doc.update_dict(root, |c| {
                if d.is_empty() {
                    c.remove(b"ViewerPreferences");
                } else {
                    c.set(b"ViewerPreferences".to_vec(), Object::Dict(d.clone()));
                }
            })?;
        }
    }
    Ok(())
}
