//! Named destinations: the names a link or an action can go to (ISO 32000-2 §12.3.2.4), stored in
//! the `/Dests` name tree under the catalog's `/Names` (§7.9.6) and, in files written for PDF 1.1,
//! in the catalog's `/Dests` dictionary.
//!
//! This module lists the names and takes one away by its position, and does nothing else: it
//! adds no name and changes no target. A removal takes the name's entry out of the leaf or the
//! dictionary that holds it, and leaves the page it went to, the links that use the name, and
//! every `/Limits` as they are (a leaf that has lost its first or last name still claims it, and
//! a lookup of it then finds nothing, as it does for any name the tree lacks).
//!
//! The name tree is read through a node that is a reference (one hop; a reference to a
//! reference is not a node) or a direct dictionary, for at most [`MAX_TREE_NODES`] nodes,
//! [`MAX_TREE_DEPTH`] levels and [`MAX_ITEMS_SCANNED`] kids and pairs, and a cut ends the walk of
//! the tree. The catalog's dictionary has its own allowance of [`MAX_ITEMS_SCANNED`] entries and
//! is read after a cut tree. A node object, and a `/Names` or `/Kids` array that is an object of
//! its own, is read once however many nodes name it, judged by the first reference only (not
//! after following a chain of them); a dictionary object that is both a tree node and the
//! catalog's `/Dests` is listed under both sources.
//!
//! A tree root or `/Dests` dictionary that is the catalog, the catalog's `/Names` dictionary
//! (by its first reference), the `/Pages` node, a node under it or a page is named unreadable and
//! never written. Any other dictionary is not guarded, the catalog's `/AcroForm`, `/Outlines` or
//! `/Info` among them, or the tree root object itself as the catalog's `/Dests`. Only a leaf that
//! is an object of its own, with its `/Names` array written in it, can have a name removed.

use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString};
use std::collections::HashSet;
use std::ops::ControlFlow;

use crate::OrganizeError;
use crate::print_presets::{Held, held};

/// Most names listed.
pub const MAX_LISTED_DESTINATIONS: usize = 256;
/// Most characters of a name listed; a longer name is cut and not `name_exact`.
pub const MAX_NAME_CHARS: usize = 127;
/// Most name tree nodes entered: this module's limit.
pub const MAX_TREE_NODES: usize = 1024;
/// Most kids and pairs of the name tree scanned in one walk, and separately the most entries of
/// the catalog's `/Dests` dictionary, whether they are names or not: this module's limit.
pub const MAX_ITEMS_SCANNED: usize = 100_000;
/// Deepest level of the name tree entered, the root being level 0: this module's limit.
pub const MAX_TREE_DEPTH: usize = 16;
/// Longest view name listed, in bytes (the longest the standard defines is 5).
const MAX_VIEW_BYTES: usize = 16;

/// Where a listed name is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The `/Dests` name tree under the catalog's `/Names`.
    Names,
    /// The catalog's `/Dests` dictionary.
    Dests,
}

/// One named destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destination {
    /// Position in the listing, counting from 0, the name tree first and then the dictionary
    /// (what [`remove_destination`] takes).
    pub index: usize,
    pub source: Source,
    /// The name, cut at [`MAX_NAME_CHARS`] characters. A tree key is decoded as a PDF text
    /// string; a dictionary key is read as UTF-8 when it is, and otherwise as one character for
    /// each byte.
    pub name: String,
    /// Whether `name` is whole.
    pub name_exact: bool,
    /// The 1-based page the destination goes to, when its first element is a reference to a
    /// page of this document.
    pub page: Option<usize>,
    /// The view's name (`XYZ`, `Fit` and so on), when the second element is a name of at most
    /// 16 bytes.
    pub view: Option<String>,
    /// Whether [`remove_destination`] can take it away.
    pub removable: bool,
}

/// What [`read_destinations`] read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Destinations {
    /// The first [`MAX_LISTED_DESTINATIONS`] names, in order.
    pub destinations: Vec<Destination>,
    /// More names exist than were listed.
    pub truncated: bool,
    /// The walk was cut at [`MAX_TREE_NODES`] nodes, [`MAX_TREE_DEPTH`] levels or
    /// [`MAX_ITEMS_SCANNED`] items.
    pub tree_capped: bool,
    /// `names` when the catalog's `/Names`, or its `/Dests`, is there but is neither a dictionary
    /// nor null, or the tree root is a structural object (see the module); `dests` the same for
    /// the catalog's `/Dests`. Those are not listed and not changed.
    pub unreadable: Vec<&'static str>,
}

/// What a removal of one position would do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Removal {
    /// There is no name at that position.
    NoEntry,
    /// The name is stored in an object this module does not change (a direct node, or a
    /// `/Names` array that is an object of its own).
    Unsupported,
    /// The name can be removed.
    Yes,
}

/// Where an entry sits.
#[derive(Clone, Copy)]
enum Place {
    /// Pair `position` of a leaf's `/Names`; the node's object when it has its own and holds the
    /// array directly, which is the only case that can be changed.
    Leaf(Option<ObjRef>, usize),
    /// The catalog's `/Dests` dictionary: its object, or `None` when written in the catalog.
    Dict(Option<ObjRef>),
}

enum Key<'a> {
    Text(&'a PdfString),
    Name(&'a [u8]),
}

struct Entry<'a> {
    source: Source,
    key: Key<'a>,
    value: &'a Object,
    place: Place,
}

#[derive(Default)]
struct Walked {
    capped: bool,
    unreadable: Vec<&'static str>,
}

type Visit<'v> = dyn FnMut(usize, &Entry<'_>) -> ControlFlow<()> + 'v;

/// An object a node is read from: its own object when `o` is a reference.
fn node_of<'a>(doc: &Document, o: &'a Object) -> (Option<ObjRef>, Held<'a>) {
    match o {
        Object::Ref(r) => (Some(*r), Held::Resolved(doc.get(*r))),
        o => (None, Held::Direct(o)),
    }
}

struct Tree<'d, 'v, 'f> {
    doc: &'d Document,
    nodes: usize,
    /// Kids and pairs scanned, against [`MAX_ITEMS_SCANNED`].
    scanned: usize,
    /// Dictionary entries scanned, against [`MAX_ITEMS_SCANNED`].
    dict_scanned: usize,
    index: usize,
    capped: bool,
    /// `visit` asked to stop.
    broke: bool,
    seen_nodes: HashSet<ObjRef>,
    seen_names: HashSet<ObjRef>,
    seen_kids: HashSet<ObjRef>,
    visit: &'f mut Visit<'v>,
}

impl Tree<'_, '_, '_> {
    /// Count one kid or pair of the tree; past the allowance the walk of the tree is cut.
    fn charge(&mut self) -> ControlFlow<()> {
        if self.scanned >= MAX_ITEMS_SCANNED {
            self.capped = true;
            return ControlFlow::Break(());
        }
        self.scanned += 1;
        #[cfg(test)]
        crate::work(1);
        ControlFlow::Continue(())
    }

    /// [`Self::charge`] for the catalog's dictionary, which has an allowance of its own.
    fn charge_dict(&mut self) -> ControlFlow<()> {
        if self.dict_scanned >= MAX_ITEMS_SCANNED {
            return ControlFlow::Break(());
        }
        self.dict_scanned += 1;
        #[cfg(test)]
        crate::work(1);
        ControlFlow::Continue(())
    }

    fn emit(&mut self, entry: &Entry<'_>) -> ControlFlow<()> {
        let flow = (self.visit)(self.index, entry);
        self.index += 1;
        if flow.is_break() {
            self.broke = true;
        }
        flow
    }

    fn node(&mut self, at: Option<ObjRef>, node: &Dict, depth: usize) -> ControlFlow<()> {
        if at.is_some_and(|r| !self.seen_nodes.insert(r)) {
            return ControlFlow::Continue(());
        }
        if self.nodes >= MAX_TREE_NODES {
            self.capped = true;
            return ControlFlow::Break(());
        }
        self.nodes += 1;
        let entry = node.get(b"Names");
        let shared = entry.and_then(Object::as_ref).is_some_and(|r| !self.seen_names.insert(r));
        let names = entry.map(|o| held(self.doc, o));
        if let (false, Some(Object::Array(items))) = (shared, names.as_deref()) {
            let own = if matches!(entry, Some(Object::Array(_))) { at } else { None };
            for (pos, [name, value]) in items.as_chunks::<2>().0.iter().enumerate() {
                self.charge()?;
                let Object::String(key) = name else { continue };
                let entry = Entry { source: Source::Names, key: Key::Text(key), value, place: Place::Leaf(own, pos) };
                self.emit(&entry)?;
            }
        }
        let entry = node.get(b"Kids");
        let shared = entry.and_then(Object::as_ref).is_some_and(|r| !self.seen_kids.insert(r));
        let kids = entry.map(|o| held(self.doc, o));
        if let (false, Some(Object::Array(kids))) = (shared, kids.as_deref()) {
            if depth >= MAX_TREE_DEPTH && !kids.is_empty() {
                self.capped = true;
                return ControlFlow::Break(());
            }
            for kid in kids {
                self.charge()?;
                let (kid_at, held_kid) = node_of(self.doc, kid);
                if let Object::Dict(d) = &*held_kid {
                    self.node(kid_at, d, depth + 1)?;
                }
            }
        }
        ControlFlow::Continue(())
    }
}

/// Whether a container this module would read or change is one that must not be: the catalog,
/// the catalog's `/Names` dictionary, the `/Pages` node, a node under it or a page. Such a
/// dictionary is named unreadable and never written (a name `Kids` taken out of the `/Pages`
/// node breaks the page tree). Only a container that is an object of its own is checked, by the
/// walk of the page tree that `role_map` and `class_map` use.
fn structural(doc: &Document, at: Option<ObjRef>, names_dict: Option<ObjRef>) -> bool {
    at.is_some_and(|r| {
        Some(r) == doc.root()
            || Some(r) == names_dict
            || crate::role_map::reaches_page_tree_node(doc, r, crate::role_map::MAX_PAGE_TREE_NODES, crate::role_map::MAX_KIDS_SCANNED)
    })
}

/// Walk the names in order, calling `visit` with each one's position until it breaks.
fn walk(doc: &Document, visit: &mut Visit<'_>) -> Walked {
    let mut out = Walked::default();
    let Some(catalog) = doc.root().map(|r| doc.get(r)) else { return out };
    let Some(catalog) = catalog.as_dict() else { return out };
    let mut tree = Tree {
        doc,
        nodes: 0,
        scanned: 0,
        dict_scanned: 0,
        index: 0,
        capped: false,
        broke: false,
        seen_nodes: HashSet::new(),
        seen_names: HashSet::new(),
        seen_kids: HashSet::new(),
        visit,
    };
    let names_dict = catalog.get(b"Names").and_then(Object::as_ref);
    let names = catalog.get(b"Names").map(|o| held(doc, o));
    match names.as_deref() {
        None | Some(Object::Null) => {}
        Some(Object::Dict(d)) => {
            if let Some(root) = d.get(b"Dests") {
                let (at, node) = node_of(doc, root);
                match &*node {
                    Object::Dict(_) if structural(doc, at, names_dict) => out.unreadable.push("names"),
                    Object::Dict(root) => {
                        let _ = tree.node(at, root, 0);
                    }
                    Object::Null => {}
                    _ => out.unreadable.push("names"),
                }
            }
        }
        Some(_) => out.unreadable.push("names"),
    }
    out.capped = tree.capped;
    if tree.broke {
        return out;
    }
    if let Some(o) = catalog.get(b"Dests") {
        let (at, node) = node_of(doc, o);
        match &*node {
            Object::Dict(_) if structural(doc, at, names_dict) => out.unreadable.push("dests"),
            Object::Dict(d) => {
                for (key, value) in d.iter() {
                    if tree.charge_dict().is_break() {
                        break;
                    }
                    let entry = Entry { source: Source::Dests, key: Key::Name(key), value, place: Place::Dict(at) };
                    if tree.emit(&entry).is_break() {
                        break;
                    }
                }
            }
            Object::Null => {}
            _ => out.unreadable.push("dests"),
        }
    }
    out
}

fn cut(chars: impl Iterator<Item = char>) -> (String, bool) {
    let mut chars = chars;
    let shown: String = chars.by_ref().take(MAX_NAME_CHARS).collect();
    (shown, chars.next().is_none())
}

/// The page and the view a destination value goes to: it is an array, or a dictionary whose `/D`
/// is one.
fn target(doc: &Document, value: &Object) -> (Option<ObjRef>, Option<String>) {
    let v = held(doc, value);
    let inner;
    let array = match &*v {
        Object::Array(a) => a,
        Object::Dict(d) => {
            let Some(o) = d.get(b"D") else { return (None, None) };
            inner = held(doc, o);
            match &*inner {
                Object::Array(a) => a,
                _ => return (None, None),
            }
        }
        _ => return (None, None),
    };
    let view = match array.get(1) {
        Some(Object::Name(n)) if n.len() <= MAX_VIEW_BYTES => Some(String::from_utf8_lossy(n).into_owned()),
        _ => None,
    };
    (array.first().and_then(Object::as_ref), view)
}

/// The names as stored. One read of the catalog and of each node entered; the pages are listed
/// (one walk of the page tree) only when a name goes to a page.
pub fn read_destinations(doc: &Document) -> Destinations {
    let mut out = Destinations::default();
    let mut pages: Option<Vec<ObjRef>> = None;
    let mut truncated = false;
    let list = &mut out.destinations;
    let walked = walk(doc, &mut |index, e| {
        if index >= MAX_LISTED_DESTINATIONS {
            truncated = true;
            return ControlFlow::Break(());
        }
        let (name, name_exact) = match e.key {
            Key::Text(s) => cut(s.text_chars()),
            Key::Name(n) => match std::str::from_utf8(n) {
                Ok(s) => cut(s.chars()),
                Err(_) => cut(n.iter().map(|&b| char::from(b))),
            },
        };
        let (page_ref, view) = target(doc, e.value);
        let page = page_ref.and_then(|r| {
            let all = pages.get_or_insert_with(|| crate::pages(doc).map(|p| p.into_iter().map(|p| p.obj).collect()).unwrap_or_default());
            all.iter().position(|p| *p == r).map(|i| i + 1)
        });
        let removable = matches!(e.place, Place::Leaf(Some(_), _) | Place::Dict(_));
        list.push(Destination { index, source: e.source, name, name_exact, page, view, removable });
        ControlFlow::Continue(())
    });
    out.truncated = truncated;
    out.tree_capped = walked.capped;
    out.unreadable = walked.unreadable;
    out
}

/// The place of the name at `index`, and its key bytes when it is in the dictionary.
fn find(doc: &Document, index: usize) -> Option<(Place, Vec<u8>)> {
    let mut found = None;
    walk(doc, &mut |i, e| {
        if i < index {
            return ControlFlow::Continue(());
        }
        let key = match e.key {
            Key::Name(n) => n.to_vec(),
            Key::Text(_) => Vec::new(),
        };
        found = Some((e.place, key));
        ControlFlow::Break(())
    });
    found
}

/// What [`remove_destination`] would do with this position.
pub fn destination_removal(doc: &Document, index: usize) -> Removal {
    match find(doc, index) {
        None => Removal::NoEntry,
        Some((Place::Leaf(None, _), _)) => Removal::Unsupported,
        Some(_) => Removal::Yes,
    }
}

/// Take the name at `index` of the listing out of the leaf or the dictionary that holds it. A
/// position with no entry is no change; a name that [`destination_removal`] calls unsupported
/// is an error. The page, the links that use the name and every `/Limits` are not touched.
pub fn remove_destination(doc: &mut Document, index: usize) -> Result<(), OrganizeError> {
    let root = doc.root().ok_or(OrganizeError::NoPageTree)?;
    let Some((place, key)) = find(doc, index) else { return Ok(()) };
    match place {
        Place::Leaf(Some(node), pos) => doc.update_dict(node, |d| {
            if let Some(Object::Array(a)) = d.get_mut(b"Names")
                && let Some(end) = pos.checked_mul(2).and_then(|s| s.checked_add(2)).filter(|end| *end <= a.len())
            {
                a.drain(end - 2..end);
            }
        })?,
        Place::Dict(Some(r)) => doc.update_dict(r, |d| {
            d.remove(&key);
        })?,
        Place::Dict(None) => doc.update_dict(root, |c| {
            if let Some(Object::Dict(d)) = c.get_mut(b"Dests") {
                d.remove(&key);
            }
        })?,
        Place::Leaf(None, _) => {
            return Err(OrganizeError::Invalid("that name is stored in an object this tool does not change".into()));
        }
    }
    Ok(())
}
