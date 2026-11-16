//! The logical reading order of a tagged document: every marked-content reference of the
//! structure tree in tree order, with the element that owns it. Read-only.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString};
use pdfcraft_model::Page;

use crate::alt;

/// Work and output limits of one walk. Every one reports itself as `truncated`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    /// Structure nodes (elements, arrays, references) visited.
    pub nodes: usize,
    /// Nesting of structure elements and `/K` arrays followed. The walk recurses this deep, so it
    /// stays small.
    pub depth: usize,
    /// Characters kept of one item's text.
    pub text_chars: usize,
    /// Characters kept of one `/Alt`, `/ActualText` or `/Lang` string.
    pub string_chars: usize,
    /// Bytes of text, alt, actual text, language and type names the items hold in all.
    pub output_bytes: usize,
    /// Raw bytes of `/Alt`, `/ActualText` and `/Lang` strings decoded in all (as headings do).
    pub source_bytes: usize,
    /// Distinct page contents parsed for text.
    pub parsed_pages: usize,
}

const LIMITS: Limits = Limits {
    nodes: 1_000_000,
    depth: 256,
    text_chars: 2_000,
    string_chars: 1_000,
    output_bytes: 8 << 20,
    source_bytes: 32 << 20,
    parsed_pages: 1_000,
};

/// The element that owns an item: its type, and the text properties its content inherits. The
/// strings are shared, so a deep or wide tree never copies an inherited string.
#[derive(Debug, Default)]
struct Owner {
    /// Element depth below the root (the top element is 0).
    depth: usize,
    object: Option<ObjRef>,
    kind: Arc<str>,
    standard: Arc<str>,
    alt: Option<Arc<str>>,
    actual_text: Option<Arc<str>>,
    language: Option<Arc<str>>,
}

/// One piece of content in reading order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadingItem {
    /// Position in reading order, from 0.
    pub index: usize,
    /// Nesting of the owning element (the top element is 0).
    pub depth: usize,
    /// The owning element's object when it is indirect.
    pub element: Option<ObjRef>,
    /// The owning element's structure type as written, and after the role map.
    pub kind: Arc<str>,
    pub standard: Arc<str>,
    /// Zero-based page when known.
    pub page: Option<usize>,
    /// The marked-content id; `None` for an object reference (an annotation or XObject).
    pub mcid: Option<i64>,
    /// The referenced object of an object reference.
    pub object: Option<ObjRef>,
    /// The page text of the marked content, cut to a bounded length (empty when it draws none, is
    /// not found, or `in_stream`).
    pub text: String,
    /// The reference names a marked-content id in another content stream (`/Stm`), not the
    /// page's: its text is not read.
    pub in_stream: bool,
    /// `/Alt`, `/ActualText` and `/Lang` of the owning element or its nearest ancestor with one,
    /// each cut to a bounded length.
    pub alt: Option<Arc<str>>,
    pub actual_text: Option<Arc<str>>,
    pub language: Option<Arc<str>>,
}

/// The items kept, and whether the walk stopped at a limit before the end of the tree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadingOrder {
    pub items: Vec<ReadingItem>,
    /// More content follows than `max_items` or a work or output limit allowed, or some text was
    /// not read or was cut short.
    pub truncated: bool,
}

fn is_type(doc: &Document, d: &Dict, name: &[u8]) -> bool {
    d.get(b"Type").map(|t| doc.resolve(t)).as_deref().and_then(Object::as_name) == Some(name)
}

struct Walk<'a> {
    doc: &'a Document,
    limits: Limits,
    max_items: usize,
    role_map: Dict,
    pages: Vec<Page>,
    page_index: HashMap<ObjRef, usize>,
    /// Page texts by the contents and resources they come from, so pages sharing them parse once.
    texts: HashMap<String, Arc<HashMap<i64, String>>>,
    seen: HashSet<ObjRef>,
    /// Decoded strings of indirect objects, which many elements may name.
    strings: HashMap<ObjRef, Option<Arc<str>>>,
    source_left: usize,
    output_left: usize,
    nodes: usize,
    out: ReadingOrder,
}

impl Walk<'_> {
    /// A capped copy of the text string `key` holds. A string that would overspend the decoding
    /// allowance is left out, and says so.
    fn string(&mut self, d: &Dict, key: &[u8]) -> Option<Arc<str>> {
        let value = d.get(key)?;
        let indirect = value.as_ref();
        if let Some(r) = indirect
            && let Some(done) = self.strings.get(&r)
        {
            return done.clone();
        }
        let resolved = self.doc.resolve(value);
        let s: &PdfString = resolved.as_string()?;
        let decoded = if s.bytes.len() > self.source_left {
            self.out.truncated = true;
            None
        } else {
            self.source_left -= s.bytes.len();
            if s.bytes.len() / 2 > self.limits.string_chars {
                self.out.truncated = true;
            }
            Some(Arc::<str>::from(s.text_chars().take(self.limits.string_chars).collect::<String>()))
        };
        if let Some(r) = indirect {
            self.strings.insert(r, decoded.clone());
        }
        decoded
    }

    /// The text of marked content `mcid` on `page`, parsing a page's contents once.
    fn text(&mut self, page: usize, mcid: i64) -> String {
        let Some(p) = self.pages.get(page) else { return String::new() };
        // Pages that share their contents and resources by reference share one parse.
        let shared = |o: Option<&Object>| match o {
            None => Some(String::new()),
            Some(Object::Ref(r)) => Some(format!("{} {}", r.num, r.generation)),
            Some(Object::Array(a)) => {
                a.iter().map(|x| x.as_ref().map(|r| format!("{} {}", r.num, r.generation))).collect::<Option<Vec<_>>>().map(|v| v.join(","))
            }
            _ => None,
        };
        let key = match (shared(p.dict.get(b"Contents")), shared(p.dict.get(b"Resources"))) {
            (Some(c), Some(r)) => format!("{c}|{r}"),
            _ => format!("page {page}"),
        };
        if !self.texts.contains_key(&key) {
            if self.texts.len() >= self.limits.parsed_pages {
                self.out.truncated = true;
                return String::new();
            }
            let parsed = crate::headings::mcid_texts(self.doc, &p.dict);
            self.texts.insert(key.clone(), Arc::new(parsed));
        }
        self.texts.get(&key).and_then(|t| t.get(&mcid)).map(|t| t.chars().take(self.limits.text_chars).collect()).unwrap_or_default()
    }

    /// Keep one item. `false` when the walk must stop: `max_items` or the output allowance is spent.
    fn push(&mut self, owner: Option<&Arc<Owner>>, page: Option<ObjRef>, mcid: Option<i64>, object: Option<ObjRef>, in_stream: bool) -> bool {
        if self.out.items.len() >= self.max_items {
            self.out.truncated = true;
            return false;
        }
        let page = page.and_then(|p| self.page_index.get(&p).copied());
        let text = match (page, mcid, in_stream) {
            (Some(i), Some(mcid), false) => self.text(i, mcid),
            _ => String::new(),
        };
        let o = owner.cloned().unwrap_or_default();
        let cost = [&o.kind, &o.standard].iter().map(|s| s.len()).sum::<usize>()
            + text.len()
            + [&o.alt, &o.actual_text, &o.language].iter().map(|s| s.as_ref().map_or(0, |s| s.len())).sum::<usize>()
            + 64;
        match self.output_left.checked_sub(cost) {
            Some(left) => self.output_left = left,
            None => {
                self.out.truncated = true;
                return false;
            }
        }
        self.out.items.push(ReadingItem {
            index: self.out.items.len(),
            depth: o.depth,
            element: o.object,
            kind: o.kind.clone(),
            standard: o.standard.clone(),
            page,
            mcid,
            object,
            text,
            in_stream,
            alt: o.alt.clone(),
            actual_text: o.actual_text.clone(),
            language: o.language.clone(),
        });
        true
    }

    /// Walk `node` (a `/K` value, array entry or element). `false` stops the whole walk.
    fn visit(&mut self, node: &Object, page: Option<ObjRef>, owner: Option<&Arc<Owner>>, level: usize) -> bool {
        self.nodes += 1;
        if self.nodes > self.limits.nodes || level > self.limits.depth {
            self.out.truncated = true;
            return true;
        }
        match node {
            Object::Ref(r) => {
                if !self.seen.insert(*r) {
                    return true;
                }
                let target = self.doc.get(*r);
                match &*target {
                    Object::Dict(d) => self.dict(d, Some(*r), page, owner, level),
                    other => self.visit(other, page, owner, level + 1),
                }
            }
            Object::Array(items) => {
                // Never queue or walk more entries than the node allowance has left.
                let room = self.limits.nodes.saturating_sub(self.nodes);
                if items.len() > room {
                    self.out.truncated = true;
                }
                for item in items.iter().take(room) {
                    if !self.visit(item, page, owner, level + 1) {
                        return false;
                    }
                }
                true
            }
            Object::Int(mcid) => self.push(owner, page, Some(*mcid), None, false),
            Object::Dict(d) => self.dict(d, None, page, owner, level),
            _ => true,
        }
    }

    fn dict(&mut self, d: &Dict, element: Option<ObjRef>, page: Option<ObjRef>, owner: Option<&Arc<Owner>>, level: usize) -> bool {
        let pg = d.get(b"Pg").and_then(Object::as_ref).or(page);
        if is_type(self.doc, d, b"MCR") {
            let in_stream = d.contains(b"Stm");
            return match d.get(b"MCID").map(|m| self.doc.resolve(m)).and_then(|m| m.as_int()) {
                Some(mcid) => {
                    if in_stream {
                        self.out.truncated = true;
                    }
                    self.push(owner, pg, Some(mcid), None, in_stream)
                }
                None => true,
            };
        }
        if is_type(self.doc, d, b"OBJR") {
            return self.push(owner, pg, None, d.get(b"Obj").and_then(Object::as_ref), false);
        }
        let Some(kind) = d.get(b"S").map(|s| self.doc.resolve(s)).and_then(|s| s.as_name().map(<[u8]>::to_vec)) else { return true };
        let standard = alt::standard(self.doc, &self.role_map, &kind);
        let me = Arc::new(Owner {
            depth: owner.map_or(0, |o| o.depth + 1),
            object: element,
            kind: Arc::from(String::from_utf8_lossy(&kind).as_ref()),
            standard: Arc::from(String::from_utf8_lossy(&standard).as_ref()),
            alt: self.string(d, b"Alt").or_else(|| owner.and_then(|o| o.alt.clone())),
            actual_text: self.string(d, b"ActualText").or_else(|| owner.and_then(|o| o.actual_text.clone())),
            language: self.string(d, b"Lang").or_else(|| owner.and_then(|o| o.language.clone())),
        });
        match d.get(b"K") {
            Some(kids) => self.visit(kids, pg, Some(&me), level + 1),
            None => true,
        }
    }
}

/// The content of the structure tree in logical order, at most `max_items` items. Cycles, shared
/// elements and pathologically deep, wide or text-heavy trees are bounded: each indirect object
/// is visited once, text and strings are cut, and a walk that hits a limit says so in `truncated`.
pub fn reading_order(doc: &Document, max_items: usize) -> ReadingOrder {
    reading_order_within(doc, max_items, LIMITS)
}

pub(crate) fn reading_order_within(doc: &Document, max_items: usize, limits: Limits) -> ReadingOrder {
    let Some(root) = alt::tree_root(doc) else { return ReadingOrder::default() };
    let pages = pdfcraft_model::pages(doc);
    let mut walk = Walk {
        doc,
        limits,
        max_items,
        role_map: root.get(b"RoleMap").map(|o| doc.resolve(o)).and_then(|o| o.as_dict().cloned()).unwrap_or_default(),
        page_index: pages.iter().enumerate().map(|(i, p)| (p.obj, i)).collect(),
        pages,
        texts: HashMap::new(),
        seen: HashSet::new(),
        strings: HashMap::new(),
        source_left: limits.source_bytes,
        output_left: limits.output_bytes,
        nodes: 0,
        out: ReadingOrder::default(),
    };
    if let Some(first) = root.get(b"K") {
        walk.visit(first, None, None, 0);
    }
    walk.out
}
