//! Reference XObjects: form XObjects with a `/Ref` dictionary, whose content stream is a proxy for
//! a page of another PDF file (ISO 32000-2 §8.10.4, which PDF/X-5g and PDF/X-5pg use). `/Ref`
//! has `/F` (the file specification), `/Page` (a page index from 0, or a page label as a text
//! string) and, optionally, `/ID` (an array of two byte strings).
//!
//! This module lists the reference XObjects of a file and takes the `/Ref` away from one, which
//! makes the form an ordinary form that draws its own content. It does not open the file that
//! `/Ref` names, draw its page in place of the proxy, add a reference or change one.
//!
//! The file is scanned by object number, not through the pages, so a reference XObject that no
//! page uses is listed as well. The scan uses `Document::scan_objects`, which keeps its own cache
//! and leaves the editing cache alone; only the objects a reference leads through are read with
//! `Document::get`. A form is a reference XObject when it is a stream whose `/Subtype` is the
//! name `Form`, read as written and not through a reference, and whose `/Ref` is a dictionary,
//! after references. Each scan has its own allowance of [`MAX_GETS`] reads, and each walk of the
//! page tree (see `structural`) has another of the same size: a scan pays one read for each
//! object scanned (charged before the scan reads it) and one for each object a reference leads
//! through, so it stops as soon as its allowance is spent and what it found stays. A call can
//! scan and walk several times (a read makes one of each, a removal three of each), so the
//! allowances are per scan and per walk, not per call. The one read not charged is the
//! catalog's, at the start of a walk. Objects are compared by number, not by generation.

use std::collections::HashSet;
use std::sync::Arc;

use pdfcraft_cos::{Document, ObjRef, Object, PdfString};

use crate::OrganizeError;
use crate::print_presets::Held;
use crate::role_map::{MAX_KIDS_SCANNED, MAX_PAGE_TREE_NODES};

/// Most reference XObjects listed (all that were found are counted).
pub const MAX_LISTED: usize = 256;
/// Most characters of a file name or page label listed; a longer one is cut and not `exact`.
pub const MAX_TEXT_CHARS: usize = 127;
/// Most reads one scan makes, and most one walk of the page tree makes: this module's allowance
/// for each, against which a read is charged before it is made (a walk's first read, of the
/// catalog, is not).
pub const MAX_GETS: usize = 2_000_000;
/// References followed after the first object, as `Document::resolve` follows them.
const MORE_HOPS: usize = 32;

/// One reference XObject.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceXObject {
    /// Position in the listing, counting from 0 (what [`remove_reference_xobject`] takes).
    pub index: usize,
    /// The form XObject's object.
    pub object: ObjRef,
    /// The name of the file `/Ref` points at: the `/UF` string of its file specification, else
    /// the `/F` string, or the file specification when it is a string; cut at
    /// [`MAX_TEXT_CHARS`] characters. `None` when there is none.
    pub file: Option<String>,
    /// `/Page` when it is an integer.
    pub page_index: Option<i64>,
    /// `/Page` when it is a string, as a text string cut at [`MAX_TEXT_CHARS`] characters.
    pub page_label: Option<String>,
    /// `/ID` is an array (after references) of exactly two direct strings.
    pub has_id: bool,
    /// [`remove_reference_xobject`] can take its `/Ref` away: the form is not the catalog, the
    /// `/Pages` node or a node under it (found by number), and that walk was not cut short.
    pub removable: bool,
    /// `file` and `page_label` are whole.
    pub exact: bool,
}

/// What a scan found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReferenceXObjects {
    /// The first [`MAX_LISTED`] reference XObjects, in object-number order.
    pub items: Vec<ReferenceXObject>,
    /// How many were found, whether listed or not.
    pub count: usize,
    /// More were found than listed.
    pub truncated: bool,
    /// The scan stopped because the [`MAX_GETS`] allowance was spent, so `count` is a lower bound
    /// and objects after the stop were not looked at.
    pub capped: bool,
}

pub(crate) struct Spent;

/// The reads left.
pub(crate) struct Gets(pub(crate) usize);

impl Gets {
    /// Charge one read that the caller makes itself.
    pub(crate) fn charge(&mut self) -> Result<(), Spent> {
        self.0 = self.0.checked_sub(1).ok_or(Spent)?;
        Ok(())
    }

    /// The object, after charging one read; nothing is read when the allowance is spent.
    pub(crate) fn get(&mut self, doc: &Document, r: ObjRef) -> Result<Arc<Object>, Spent> {
        self.0 = self.0.checked_sub(1).ok_or(Spent)?;
        Ok(doc.get(r))
    }

    /// `o` after references, as `Document::resolve` follows them (the first object and up to 32
    /// more), each read charged before it is made.
    pub(crate) fn follow<'a>(&mut self, doc: &Document, o: &'a Object) -> Result<Held<'a>, Spent> {
        let Object::Ref(first) = o else { return Ok(Held::Direct(o)) };
        let mut held = self.get(doc, *first)?;
        for _ in 0..MORE_HOPS {
            let Object::Ref(next) = &*held else { break };
            held = self.get(doc, *next)?;
        }
        Ok(Held::Resolved(held))
    }

    /// A string value after references: its text cut at [`MAX_TEXT_CHARS`] and whether it is whole.
    fn text(&mut self, doc: &Document, o: &Object) -> Result<Option<(String, bool)>, Spent> {
        Ok(match &*self.follow(doc, o)? {
            Object::String(s) => Some(cut(s)),
            _ => None,
        })
    }
}

fn cut(s: &PdfString) -> (String, bool) {
    let mut chars = s.text_chars();
    let shown: String = chars.by_ref().take(MAX_TEXT_CHARS).collect();
    (shown, chars.next().is_none())
}

/// The reference XObject `/Ref` describes, or `None` when it is not a dictionary.
fn read_ref(doc: &Document, gets: &mut Gets, entry: &Object, object: ObjRef) -> Result<Option<ReferenceXObject>, Spent> {
    let held = gets.follow(doc, entry)?;
    let Object::Dict(d) = &*held else { return Ok(None) };
    let mut item = ReferenceXObject { index: 0, object, file: None, page_index: None, page_label: None, has_id: false, removable: true, exact: true };
    if let Some(f) = d.get(b"F") {
        let spec = gets.follow(doc, f)?;
        let name = match &*spec {
            Object::String(s) => Some(cut(s)),
            Object::Dict(fs) => {
                let mut found = None;
                for key in [&b"UF"[..], b"F"] {
                    if let Some(v) = fs.get(key) {
                        found = gets.text(doc, v)?;
                        if found.is_some() {
                            break;
                        }
                    }
                }
                found
            }
            _ => None,
        };
        if let Some((text, whole)) = name {
            item.file = Some(text);
            item.exact &= whole;
        }
    }
    if let Some(p) = d.get(b"Page") {
        match &*gets.follow(doc, p)? {
            Object::Int(i) => item.page_index = Some(*i),
            Object::String(s) => {
                let (text, whole) = cut(s);
                item.page_label = Some(text);
                item.exact &= whole;
            }
            _ => {}
        }
    }
    if let Some(id) = d.get(b"ID") {
        let id = gets.follow(doc, id)?;
        item.has_id = matches!(id.as_array().map(Vec::as_slice), Some([Object::String(_), Object::String(_)]));
    }
    Ok(Some(item))
}

/// The scan, with `allowance` reads ([`MAX_GETS`] in use). With `stop_after` the walk ends as soon as the item at that position is listed,
/// which is enough to find it.
pub(crate) fn scan(doc: &Document, stop_after: Option<usize>, allowance: usize) -> ReferenceXObjects {
    let mut out = ReferenceXObjects::default();
    let mut gets = Gets(allowance);
    let mut objects = doc.scan_objects();
    // `scan_objects` knows how many objects are left, so the last one is paid for only when it
    // is there to read.
    while objects.size_hint().1 != Some(0) {
        if gets.0 == 0 {
            out.capped = true;
            break;
        }
        gets.0 -= 1;
        let Some((at, object)) = objects.next() else { break };
        let Object::Stream(s) = &*object else { continue };
        if s.dict.name(b"Subtype") != Some(b"Form") {
            continue;
        }
        let Some(entry) = s.dict.get(b"Ref") else { continue };
        match read_ref(doc, &mut gets, entry, at) {
            Err(Spent) => {
                out.capped = true;
                break;
            }
            Ok(None) => {}
            Ok(Some(mut item)) => {
                out.count += 1;
                if out.items.len() < MAX_LISTED {
                    item.index = out.items.len();
                    out.items.push(item);
                    if stop_after.is_some_and(|i| out.items.len() > i) {
                        break;
                    }
                }
            }
        }
    }
    out.truncated = out.count > out.items.len();
    out
}

/// The numbers of the catalog, the `/Pages` node and every node under it (an inner `/Pages`
/// node or a page), found by walking `/Kids` from the `/Pages` node, and whether the walk was cut
/// short (more than [`MAX_PAGE_TREE_NODES`] nodes, more than [`MAX_KIDS_SCANNED`] `/Kids`
/// entries, or [`MAX_GETS`] reads spent), in which case the set is not all of them. A node is put
/// on the stack once, and every read after the catalog's is charged before it is made, against the
/// walk's own allowance of `reads`.
pub(crate) fn structural(doc: &Document) -> (HashSet<u32>, bool) {
    structural_with(doc, MAX_GETS, MAX_PAGE_TREE_NODES, MAX_KIDS_SCANNED)
}

/// [`structural`] with the three allowances given.
pub(crate) fn structural_with(doc: &Document, reads: usize, max_nodes: usize, max_kids: usize) -> (HashSet<u32>, bool) {
    let mut seen: HashSet<u32> = doc.root().into_iter().map(|r| r.num).collect();
    let mut gets = Gets(reads);
    let Some(top) = doc.root().and_then(|r| doc.get(r).as_dict().and_then(|c| c.reference(b"Pages"))) else {
        return (seen, false);
    };
    seen.insert(top.num);
    let mut todo = vec![top];
    let (mut nodes, mut scanned) = (1usize, 0usize);
    while let Some(at) = todo.pop() {
        let Ok(node) = gets.get(doc, at) else { return (seen, true) };
        let Some(kids) = node.as_dict().and_then(|d| d.get(b"Kids")) else { continue };
        let Ok(kids) = gets.follow(doc, kids) else { return (seen, true) };
        let Some(items) = kids.as_array() else { continue };
        for item in items {
            scanned += 1;
            if scanned > max_kids {
                return (seen, true);
            }
            let Some(kid) = item.as_ref() else { continue };
            if seen.insert(kid.num) {
                nodes += 1;
                if nodes > max_nodes {
                    return (seen, true);
                }
                todo.push(kid);
            }
        }
    }
    (seen, false)
}

/// The reference XObjects of the file, from one scan.
pub fn read_reference_xobjects(doc: &Document) -> ReferenceXObjects {
    let mut found = scan(doc, None, MAX_GETS);
    if !found.items.is_empty() {
        let (nodes, cut) = structural(doc);
        for item in &mut found.items {
            item.removable = !cut && !nodes.contains(&item.object.num);
        }
    }
    found
}

/// What [`remove_reference_xobject`] would do for a position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Removal {
    /// The `/Ref` of the form listed there would be taken away.
    Yes,
    /// Nothing is listed there (a position of [`MAX_LISTED`] or more never is).
    NoEntry,
    /// A form is listed there that is not removable: see [`ReferenceXObject::removable`].
    Refused,
}

/// The object to take the `/Ref` from, or why not.
fn target(doc: &Document, index: usize) -> Result<ObjRef, Removal> {
    if index >= MAX_LISTED {
        return Err(Removal::NoEntry);
    }
    let found = scan(doc, Some(index), MAX_GETS);
    let at = found.items.get(index).ok_or(Removal::NoEntry)?.object;
    let (nodes, cut) = structural(doc);
    if cut || nodes.contains(&at.num) {
        return Err(Removal::Refused);
    }
    Ok(at)
}

/// What [`remove_reference_xobject`] with this position would do.
pub fn reference_xobject_removal(doc: &Document, index: usize) -> Removal {
    target(doc, index).map_or_else(|e| e, |_| Removal::Yes)
}

/// Take the `/Ref` out of the reference XObject listed at `index`. Its content, its other entries
/// and the object `/Ref` pointed at (when it was indirect) are left as they are. A position with
/// nothing listed, and a form that is not removable, are no change.
pub fn remove_reference_xobject(doc: &mut Document, index: usize) -> Result<(), OrganizeError> {
    let Ok(at) = target(doc, index) else { return Ok(()) };
    doc.update_dict(at, |d| {
        d.remove(b"Ref");
    })?;
    Ok(())
}
