//! PDF 2.0 document parts (ISO 32000-2 §14.12): the tree under the catalog's `/DPartRoot`,
//! each node a `/Type /DPart` dictionary whose `/DParts` array lists its children (an array of
//! arrays of references) and whose leaves name the first and last page they cover (`/Start`,
//! `/End`) and carry document part metadata (`/DPM`).
//!
//! Reading only: [`list_document_parts`] walks the tree and changes nothing. Nothing here writes,
//! validates the tree against the specification or reads the `/DPM` values as anything but a
//! short text.

use std::collections::{HashMap, HashSet};

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
    /// References read from `/DParts`, including any that were not read as nodes.
    pub children: usize,
    /// `/DPM` entries as `(key, value as short text)`, at most [`MAX_DPM_ENTRIES`].
    pub metadata: Vec<(String, String)>,
    /// `/DPM` has more entries than were listed.
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
        if let Some(groups) = dict.get(b"DParts").map(|o| doc.resolve(o))
            && let Some(groups) = groups.as_array()
        {
            'groups: for group in groups {
                let group = doc.resolve(group);
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
        if let Some(dpm) = dict.get(b"DPM").map(|o| doc.resolve(o))
            && let Some(dpm) = dpm.as_dict()
        {
            for (key, value) in dpm.iter() {
                if metadata.len() >= MAX_DPM_ENTRIES {
                    metadata_truncated = true;
                    break;
                }
                metadata.push((short(&String::from_utf8_lossy(key)), value_text(doc, value)));
            }
        }

        out.parts.push(DocumentPart {
            object: node.num,
            parent,
            depth,
            start_page: page_of(b"Start"),
            end_page: page_of(b"End"),
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

fn short(text: &str) -> String {
    text.chars().take(MAX_DPM_CHARS).collect()
}

fn value_text(doc: &Document, value: &Object) -> String {
    match &*doc.resolve(value) {
        Object::Null => "null".to_string(),
        Object::Bool(b) => b.to_string(),
        Object::Int(n) => n.to_string(),
        Object::Real(x) => x.to_string(),
        Object::String(s) => short(&s.text_chars().take(MAX_DPM_CHARS).collect::<String>()),
        Object::Name(n) => short(&format!("/{}", String::from_utf8_lossy(n))),
        Object::Array(_) => "(array)".to_string(),
        Object::Dict(_) => "(dictionary)".to_string(),
        Object::Stream(_) => "(stream)".to_string(),
        Object::Ref(_) => "(reference)".to_string(),
    }
}
