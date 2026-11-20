//! PDF 2.0 document parts (ISO 32000-2 §14.12): the tree under the catalog's `/DPartRoot`,
//! each node a `/Type /DPart` dictionary whose `/DParts` array lists its children (an array of
//! arrays of references) and whose leaves name the first and last page they cover (`/Start`,
//! `/End`) and carry document part metadata (`/DPM`).
//!
//! Reading only: [`list_document_parts`] walks the tree and changes nothing. Nothing here writes,
//! validates the tree against the specification or reads the `/DPM` values as anything but a
//! short text.

use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::sync::Arc;

use pdfcraft_cos::{Document, ObjRef, Object};

/// Most nodes read in all.
pub const MAX_DPART_NODES: usize = 5000;
/// Deepest node read; the root node is depth 0.
pub const MAX_DPART_DEPTH: usize = 64;
/// Most `/DPM` entries listed for one node.
pub const MAX_DPM_ENTRIES: usize = 32;
/// Most characters of one `/DPM` value or key kept.
pub const MAX_DPM_CHARS: usize = 200;
/// Most pages looked at when turning `/Start` and `/End` into page numbers.
const MAX_PAGES_READ: usize = 100_000;
/// Most references read from the `/DParts` of one node.
const MAX_CHILDREN: usize = 5000;
/// What listing `/DPM` entries may read across the whole tree, in bytes: each entry costs its
/// key's length, its value's raw length when that is a string or a name, and
/// [`DPM_ENTRY_COST`]. An indirect value is resolved once to learn its length, so it is read
/// before it is charged; the charge is made before it is listed, and again for each node that
/// lists it, so a dictionary (or one long string) that many nodes share costs what it holds each
/// time.
pub const MAX_DPM_BYTES: usize = 4 << 20;
/// What each listed `/DPM` entry costs besides its key and value.
pub const DPM_ENTRY_COST: usize = 32;

/// One node of the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentPart {
    /// The node's object number.
    pub object: u32,
    /// The listed node this one was reached from (`None` for the root node).
    pub parent: Option<u32>,
    /// 0 for the root node.
    pub depth: usize,
    /// 1-based page of `/Start`, when it names a page of the document.
    pub start_page: Option<usize>,
    /// 1-based page of `/End`, when it names a page of the document.
    pub end_page: Option<usize>,
    /// Both `/Start` and `/End` name pages of the document and the end page comes before the
    /// start page, so the part covers no pages in order. Worked out from `start_page` and
    /// `end_page` alone; a missing `/End` is not inverted.
    pub range_inverted: bool,
    /// References read from `/DParts`, including any that were not read as nodes.
    pub children: usize,
    /// `/DPM` entries as `(key, value as short text)`, at most [`MAX_DPM_ENTRIES`].
    pub metadata: Vec<(String, String)>,
    /// `/DPM` has more entries than were listed: past [`MAX_DPM_ENTRIES`], or past what the tree
    /// as a whole may list ([`MAX_DPM_BYTES`]; once one entry does not fit, no later one is listed or
    /// read, and a node with a `/DPM` is flagged: one that is indirect and not read before without
    /// being read, so even if it is empty).
    pub metadata_truncated: bool,
}

/// What [`list_document_parts`] read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocumentPartList {
    /// Whether the catalog has a `/DPartRoot` dictionary.
    pub has_root: bool,
    /// The root's `/RecordLevel`, when it is an integer.
    pub record_level: Option<i64>,
    /// Nodes in document order (a node before its children).
    pub parts: Vec<DocumentPart>,
    /// References that were not read as nodes: not a dictionary, or a node already read (a cycle
    /// or a node listed twice).
    pub skipped: usize,
    /// How many of the listed nodes have `range_inverted`.
    pub inverted_ranges: usize,
    /// More nodes, levels or children exist than were read.
    pub truncated: bool,
}

/// The document part tree, at most [`MAX_DPART_NODES`] nodes, [`MAX_DPART_DEPTH`] levels deep.
/// Each node is read once, so a cycle ends. Only indirect references are followed to nodes, as
/// the specification requires.
pub fn list_document_parts(doc: &Document) -> DocumentPartList {
    let mut out = DocumentPartList::default();
    let Some(catalog) = doc.root().map(|r| doc.get(r)) else { return out };
    let Some(root) = catalog.as_dict().and_then(|c| c.get(b"DPartRoot")).map(|o| doc.resolve(o)) else { return out };
    let Some(root) = root.as_dict() else { return out };
    out.has_root = true;
    out.record_level = root.get(b"RecordLevel").map(|o| doc.resolve(o)).and_then(|o| o.as_int());
    let Some(first) = root.get(b"DPartRootNode").and_then(Object::as_ref) else { return out };

    let mut page_numbers: HashMap<ObjRef, usize> = HashMap::new();
    if let Ok(pages) = crate::pages(doc) {
        if pages.len() > MAX_PAGES_READ {
            out.truncated = true;
        }
        for (n, page) in pages.iter().take(MAX_PAGES_READ).enumerate() {
            page_numbers.entry(page.obj).or_insert(n + 1);
        }
    }

    let mut text_left = MAX_DPM_BYTES;
    // Set once an entry did not fit; from then on no /DPM is listed or read.
    let mut spent = false;
    let mut seen: HashSet<ObjRef> = HashSet::new();
    // Preorder: a stack of (node, depth, parent), children pushed last-first.
    let mut stack = vec![(first, 0usize, None::<u32>)];
    while let Some((node, depth, parent)) = stack.pop() {
        if !seen.insert(node) {
            out.skipped = out.skipped.saturating_add(1);
            continue;
        }
        if out.parts.len() >= MAX_DPART_NODES {
            out.truncated = true;
            break;
        }
        let object = doc.get(node);
        let Some(dict) = object.as_dict() else {
            out.skipped = out.skipped.saturating_add(1);
            continue;
        };
        let page_of = |key: &[u8]| dict.get(key).and_then(Object::as_ref).and_then(|r| page_numbers.get(&r).copied());

        let mut children = Vec::new();
        let mut counted = 0usize;
        // Direct values are borrowed; only references are followed (a direct one would be cloned).
        if let Some(groups) = dict.get(b"DParts").map(|o| follow(doc, o))
            && let Some(groups) = groups.as_array()
        {
            'groups: for group in groups {
                let group = follow(doc, group);
                let Some(group) = group.as_array() else { continue };
                for child in group {
                    if counted >= MAX_CHILDREN {
                        out.truncated = true;
                        break 'groups;
                    }
                    counted += 1;
                    match child.as_ref() {
                        Some(r) => children.push(r),
                        None => out.skipped = out.skipped.saturating_add(1),
                    }
                }
            }
        }

        let mut metadata = Vec::new();
        let mut metadata_truncated = false;
        // Once an entry has not fitted, no /DPM is read at all: a node that has one is flagged, and
        // an indirect one is flagged without being resolved (it may be empty).
        let dpm = match dict.get(b"DPM") {
            Some(Object::Ref(_)) if spent => {
                metadata_truncated = true;
                None
            }
            Some(direct) if spent => {
                metadata_truncated = direct.as_dict().is_some_and(|d| !d.is_empty());
                None
            }
            Some(o) => prepare_dpm(doc, &follow(doc, o)),
            None => None,
        };
        if let Some(dpm) = dpm {
            for entry in &dpm.entries {
                if spent || entry.cost > text_left {
                    spent = true;
                    metadata_truncated = true;
                    break;
                }
                text_left -= entry.cost;
                metadata.push((entry.key.clone(), entry.value.clone()));
            }
            metadata_truncated |= dpm.more;
        }

        let (start_page, end_page) = (page_of(b"Start"), page_of(b"End"));
        let range_inverted = matches!((start_page, end_page), (Some(s), Some(e)) if e < s);
        out.inverted_ranges = out.inverted_ranges.saturating_add(usize::from(range_inverted));
        out.parts.push(DocumentPart {
            object: node.num,
            parent,
            depth,
            start_page,
            end_page,
            range_inverted,
            children: counted,
            metadata,
            metadata_truncated,
        });
        if depth >= MAX_DPART_DEPTH {
            if !children.is_empty() {
                out.truncated = true;
            }
            continue;
        }
        for child in children.into_iter().rev() {
            stack.push((child, depth + 1, Some(node.num)));
        }
    }
    out
}

/// What `o` points at: a reference is followed, and anything else is borrowed, not cloned.
enum Followed<'a> {
    Direct(&'a Object),
    Held(Arc<Object>),
}

impl Deref for Followed<'_> {
    type Target = Object;
    fn deref(&self) -> &Object {
        match self {
            Self::Direct(o) => o,
            Self::Held(o) => o,
        }
    }
}

fn follow<'a>(doc: &Document, o: &'a Object) -> Followed<'a> {
    match o {
        Object::Ref(_) => Followed::Held(doc.resolve(o)),
        direct => Followed::Direct(direct),
    }
}

#[cfg(test)]
thread_local! {
    /// `/DPM` dictionaries [`prepare_dpm`] read on this thread.
    pub(crate) static PREPARES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// One `/DPM` entry as listed, and what listing it costs against [`MAX_DPM_BYTES`].
struct Entry {
    cost: usize,
    key: String,
    value: String,
}

/// The first [`MAX_DPM_ENTRIES`] entries of a `/DPM` dictionary, and whether it has more.
struct Dpm {
    entries: Vec<Entry>,
    more: bool,
}

/// `dpm` as a listing, or `None` when it is not a dictionary. Direct values are borrowed; a
/// reference is followed, which has to happen before the value's length is known and charged.
fn prepare_dpm(doc: &Document, dpm: &Object) -> Option<Dpm> {
    #[cfg(test)]
    PREPARES.with(|n| n.set(n.get().saturating_add(1)));
    let dpm = dpm.as_dict()?;
    let entries = dpm
        .iter()
        .take(MAX_DPM_ENTRIES)
        .map(|(key, value)| {
            let value = follow(doc, value);
            Entry { cost: key.len().saturating_add(raw_len(&value)).saturating_add(DPM_ENTRY_COST), key: short_bytes(key), value: value_text(&value) }
        })
        .collect();
    Some(Dpm { entries, more: dpm.iter().nth(MAX_DPM_ENTRIES).is_some() })
}

/// The bytes reading `value` as text looks at.
fn raw_len(value: &Object) -> usize {
    match value {
        Object::String(s) => s.bytes.len(),
        Object::Name(n) => n.len(),
        _ => 0,
    }
}

fn short(text: &str) -> String {
    text.chars().take(MAX_DPM_CHARS).collect()
}

/// The first [`MAX_DPM_CHARS`] characters of `bytes` read as UTF-8 with replacement characters,
/// looking at no more bytes than that many characters can take (four each).
fn short_bytes(bytes: &[u8]) -> String {
    short(&String::from_utf8_lossy(bytes.get(..MAX_DPM_CHARS * 4).unwrap_or(bytes)))
}

fn value_text(value: &Object) -> String {
    match value {
        Object::Null => "null".to_string(),
        Object::Bool(b) => b.to_string(),
        Object::Int(n) => n.to_string(),
        Object::Real(x) => x.to_string(),
        Object::String(s) => s.text_chars().take(MAX_DPM_CHARS).collect(),
        Object::Name(n) => format!("/{}", short_bytes(n)).chars().take(MAX_DPM_CHARS).collect(),
        Object::Array(_) => "(array)".to_string(),
        Object::Dict(_) => "(dictionary)".to_string(),
        Object::Stream(_) => "(stream)".to_string(),
        Object::Ref(_) => "(reference)".to_string(),
    }
}
