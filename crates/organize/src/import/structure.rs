//! Structure trees through Combine, Insert pages, Extract and Split (ISO 32000-2 §14.7): each
//! file's tags come along with its pages, under one tree. Elements that lead to no copied page
//! are left behind. The parent tree
//! (§14.7.5.4) is numbered afresh, so one file's keys never answer for another's pages,
//! annotations or forms.

use super::copy::Copier;
use super::structure_maps::{Maps, Renames};
use super::structure_order::Order;
use super::{Dict, Document, HashMap, HashSet, ObjRef, Object, OrganizeError};

/// How deep elements and number tree nodes are followed.
const MAX_DEPTH: usize = 256;
/// The most elements, and parent tree entries, one file brings.
const MAX_ITEMS: usize = 1_000_000;

/// The tree as sources are added: a new one, or a document's own ([`Tags::open`]).
#[derive(Default)]
pub(super) struct Tags {
    /// The combined `/StructTreeRoot`, made for the first tagged source.
    root: Option<ObjRef>,
    kids: Vec<Object>,
    /// Elements added after those, each to be put where its pages are.
    later: Vec<ObjRef>,
    /// Parent tree entries, in key order.
    nums: Vec<(i64, Object)>,
    next: i64,
    /// A source had no tags, so the whole is not a tagged document.
    untagged: bool,
    /// The first source's language, while every source says the same.
    lang: Option<Option<Object>>,
    /// Role and class maps, identifiers and namespaces.
    maps: Maps,
    /// The other entries of the document's own tree root.
    base: Dict,
    /// A source brought tags, so the tree has to be written.
    changed: bool,
}

/// One source's elements being copied.
struct Source<'a> {
    src: &'a Document,
    /// Source page → copy.
    pages: &'a HashMap<ObjRef, ObjRef>,
    /// Source object → copy.
    objects: &'a HashMap<ObjRef, ObjRef>,
    /// Whether all of its pages were copied: only then does an element holding nothing come too.
    whole: bool,
    copier: Copier<'a>,
    /// Source element → copy.
    elements: HashMap<ObjRef, ObjRef>,
    seen: HashSet<ObjRef>,
    left: usize,
    maps: &'a mut Maps,
    renames: Renames,
    /// The elements copied, and those among them that named others in `/Ref`.
    made: Vec<ObjRef>,
    refs: Vec<(ObjRef, Object)>,
}

fn kids_of(src: &Document, k: Option<&Object>) -> Vec<Object> {
    let Some(k) = k else { return Vec::new() };
    match src.resolve(k).as_array() {
        Some(list) => list.clone(),
        None => vec![k.clone()],
    }
}

/// The annotation `copy` stands for: a widget that was split from its field hangs below it.
fn annotation(out: &Document, copy: ObjRef) -> ObjRef {
    let held = out.get(copy);
    let Some(dict) = held.as_dict().filter(|d| !d.contains(b"Subtype")) else { return copy };
    let kids = dict.get(b"Kids").map(|k| out.resolve(k));
    match kids.as_deref().and_then(Object::as_array).map(Vec::as_slice) {
        Some([Object::Ref(only)]) => *only,
        _ => copy,
    }
}

impl Source<'_> {
    /// The copy of the element `o` under `parent`, or `None` when it leads to no copied page.
    fn element(&mut self, out: &mut Document, o: &Object, parent: ObjRef, depth: usize) -> Result<Option<ObjRef>, OrganizeError> {
        let from = o.as_ref();
        if depth >= MAX_DEPTH || self.left == 0 || from.is_some_and(|r| !self.seen.insert(r)) {
            return Ok(None);
        }
        let Some(dict) = self.src.resolve(o).as_dict().filter(|d| d.contains(b"S")).cloned() else { return Ok(None) };
        self.left -= 1;
        let new = out.add(Object::Null);
        let page = dict.reference(b"Pg");
        let here = page.and_then(|p| self.pages.get(&p).copied());
        let children = kids_of(self.src, dict.get(b"K"));
        let empty = children.is_empty();
        let mut kept = Vec::new();
        for child in children {
            if let Object::Int(mcid) = *self.src.resolve(&child) {
                if here.is_some() {
                    kept.push(Object::Int(mcid));
                }
                continue;
            }
            let held = self.src.resolve(&child);
            let Some(item) = held.as_dict() else { continue };
            let item_page = item.reference(b"Pg").or(page).and_then(|p| self.pages.get(&p).copied());
            match item.name(b"Type") {
                // Marked content on another page than the element's, or in a form.
                Some(b"MCR") => {
                    let (Some(on), Some(mcid)) = (item_page, item.get(b"MCID")) else { continue };
                    let mut reference = Dict::new();
                    reference.set(b"Type".to_vec(), Object::name("MCR"));
                    reference.set(b"Pg".to_vec(), Object::Ref(on));
                    reference.set(b"MCID".to_vec(), mcid.clone());
                    if let Some(stream) = item.reference(b"Stm") {
                        let Some(copy) = self.objects.get(&stream) else { continue };
                        reference.set(b"Stm".to_vec(), Object::Ref(*copy));
                    }
                    kept.push(Object::Dict(reference));
                }
                // An annotation or other whole object.
                Some(b"OBJR") => {
                    let Some(copy) = item.reference(b"Obj").and_then(|r| self.objects.get(&r).copied()) else { continue };
                    let mut reference = Dict::new();
                    reference.set(b"Type".to_vec(), Object::name("OBJR"));
                    if let Some(on) = item_page {
                        reference.set(b"Pg".to_vec(), Object::Ref(on));
                    }
                    reference.set(b"Obj".to_vec(), Object::Ref(annotation(out, copy)));
                    kept.push(Object::Dict(reference));
                }
                _ => {
                    if let Some(below) = self.element(out, &child, new, depth + 1)? {
                        kept.push(Object::Ref(below));
                    }
                }
            }
        }
        if kept.is_empty() && !(empty && self.whole) {
            return Ok(None);
        }
        let mut copy = Dict::new();
        for (key, value) in dict.iter() {
            // `/Ref` names other elements, which may not be copied yet: set once all are.
            if !matches!(key.as_slice(), b"K" | b"P" | b"Pg" | b"Ref") {
                copy.set(key.clone(), self.copier.copy_value(out, value, None)?);
            }
        }
        copy.set(b"P".to_vec(), Object::Ref(parent));
        self.maps.element(&mut self.renames, &mut copy, new);
        self.made.push(new);
        if let Some(named) = dict.get(b"Ref") {
            self.refs.push((new, named.clone()));
        }
        if let Some(here) = here {
            copy.set(b"Pg".to_vec(), Object::Ref(here));
        }
        match kept.len() {
            0 => {}
            1 => copy.set(b"K".to_vec(), kept.remove(0)),
            _ => copy.set(b"K".to_vec(), Object::Array(kept)),
        }
        out.set(new, Object::Dict(copy));
        if let Some(from) = from {
            self.elements.insert(from, new);
        }
        Ok(Some(new))
    }

    /// What can only be set once every element is copied: `/Ref`, and references to renamed
    /// identifiers.
    fn finish(&self, out: &mut Document) {
        for (element, named) in &self.refs {
            let kept: Vec<Object> = kids_of(self.src, Some(named))
                .iter()
                .filter_map(|o| o.as_ref().and_then(|r| self.elements.get(&r)))
                .map(|r| Object::Ref(*r))
                .collect();
            if !kept.is_empty()
                && let Some(mut dict) = out.get(*element).as_dict().cloned()
            {
                dict.set(b"Ref".to_vec(), Object::Array(kept));
                out.set(*element, Object::Dict(dict));
            }
        }
        let mut done = HashSet::new();
        for element in &self.made {
            let Some(mut dict) = out.get(*element).as_dict().cloned() else { continue };
            let Some(attributes) = dict.get(b"A").cloned() else { continue };
            if let Some(new) = Maps::headers(out, &self.renames, &attributes, &mut done, 0) {
                dict.set(b"A".to_vec(), new);
                out.set(*element, Object::Dict(dict));
            }
        }
    }

    /// A parent tree entry with its elements replaced by their copies; `None` when none is left.
    fn entry(&self, value: &Object) -> Option<Object> {
        let copy = |o: &Object| o.as_ref().and_then(|r| self.elements.get(&r)).map_or(Object::Null, |r| Object::Ref(*r));
        match self.src.resolve(value).as_array() {
            // Marked content, one element for each MCID.
            Some(list) => {
                let list: Vec<Object> = list.iter().map(copy).collect();
                list.iter().any(|o| !matches!(o, Object::Null)).then_some(Object::Array(list))
            }
            // An annotation's or form's one element.
            None => Some(copy(value)).filter(|o| !matches!(o, Object::Null)),
        }
    }
}

/// The entries of the number tree at `node`.
fn numbers(src: &Document, node: &Object, depth: usize, seen: &mut HashSet<ObjRef>, out: &mut HashMap<i64, Object>) {
    if depth >= MAX_DEPTH || node.as_ref().is_some_and(|r| !seen.insert(r)) {
        return;
    }
    let held = src.resolve(node);
    let Some(dict) = held.as_dict() else { return };
    if let Some(nums) = dict.get(b"Nums").map(|n| src.resolve(n)).as_deref().and_then(Object::as_array) {
        for pair in nums.as_chunks::<2>().0 {
            if out.len() >= MAX_ITEMS {
                return;
            }
            if let [Object::Int(key), value] = pair {
                out.insert(*key, value.clone());
            }
        }
    }
    if let Some(kids) = dict.get(b"Kids").map(|k| src.resolve(k)).as_deref().and_then(Object::as_array) {
        for kid in kids {
            numbers(src, kid, depth + 1, seen, out);
        }
    }
}

/// Set or take away `key` in the dictionary, or stream dictionary, of `r`.
fn put(out: &mut Document, r: ObjRef, key: &[u8], value: Option<i64>) {
    let mut object = (*out.get(r)).clone();
    let dict = match &mut object {
        Object::Dict(d) => d,
        Object::Stream(s) => &mut s.dict,
        _ => return,
    };
    match value {
        Some(value) => dict.set(key.to_vec(), Object::Int(value)),
        None if dict.contains(key) => {
            dict.remove(key);
        }
        None => return,
    }
    out.set(r, object);
}

/// The parent tree key `r` of `src` holds under `key`.
fn key_of(src: &Document, r: ObjRef, key: &[u8]) -> Option<i64> {
    let held = src.get(r);
    let dict = match &*held {
        Object::Dict(d) => d,
        Object::Stream(s) => &s.dict,
        _ => return None,
    };
    match dict.get(key).map(|v| src.resolve(v)).as_deref() {
        Some(Object::Int(n)) => Some(*n),
        _ => None,
    }
}

/// Whether `doc` says it is a tagged document.
fn marked(doc: &Document) -> bool {
    let catalog = doc.root().map(|r| doc.get(r));
    let info = catalog.as_deref().and_then(Object::as_dict).and_then(|c| c.get(b"MarkInfo")).and_then(|m| doc.dict(m));
    info.is_some_and(|i| matches!(i.get(b"Marked").map(|v| doc.resolve(v)).as_deref(), Some(Object::Bool(true))))
}

impl Tags {
    /// The tree `doc` already has, for pages inserted into it; its language is its own. `None` when
    /// its tree root is not an object of its own, so no element can be added below it.
    pub(super) fn open(doc: &Document) -> Option<Self> {
        let mut tags = Tags { lang: Some(None), untagged: !marked(doc), ..Tags::default() };
        let catalog = doc.root().map(|r| doc.get(r));
        let entry = catalog.as_deref().and_then(Object::as_dict).and_then(|c| c.get(b"StructTreeRoot"));
        let Some(entry) = entry else { return Some(tags) };
        let (root, tree) = (entry.as_ref()?, doc.dict(entry)?);
        tags.root = Some(root);
        tags.kids = kids_of(doc, tree.get(b"K"));
        let mut entries = HashMap::new();
        if let Some(parents) = tree.get(b"ParentTree") {
            numbers(doc, parents, 0, &mut HashSet::new(), &mut entries);
        }
        tags.nums = entries.into_iter().collect();
        tags.nums.sort_unstable_by_key(|(k, _)| *k);
        let past = tags.nums.last().map_or(0, |(k, _)| k.saturating_add(1));
        tags.next = tree.int(b"ParentTreeNextKey").unwrap_or(0).max(past).max(0);
        tags.maps = Maps::open(doc, &tree);
        tags.base = tree;
        Some(tags)
    }

    /// Add the tags of `src`, whose pages `pages` and objects `objects` have been copied into `out`.
    pub(super) fn add(
        &mut self,
        out: &mut Document,
        src: &Document,
        pages: &HashMap<ObjRef, ObjRef>,
        objects: &HashMap<ObjRef, ObjRef>,
        whole: bool,
    ) -> Result<(), OrganizeError> {
        let catalog = src.root().map(|r| src.get(r));
        let catalog = catalog.as_deref().and_then(Object::as_dict);
        let lang = catalog.and_then(|c| c.get(b"Lang")).map(|l| (*src.resolve(l)).clone());
        match &self.lang {
            None => self.lang = Some(lang.clone()),
            Some(first) if *first != lang => self.lang = Some(None),
            Some(_) => {}
        }
        let tree = catalog.and_then(|c| c.get(b"StructTreeRoot")).map(|t| src.resolve(t));
        let tree = tree.as_deref().and_then(Object::as_dict);
        self.untagged |= tree.is_none() || !marked(src);

        let mut source = Source {
            src,
            pages,
            objects,
            whole,
            copier: Copier::new(src, objects.clone(), pages.clone()),
            elements: HashMap::new(),
            seen: HashSet::new(),
            left: MAX_ITEMS,
            maps: &mut self.maps,
            renames: Renames::default(),
            made: Vec::new(),
            refs: Vec::new(),
        };
        let mut parents = HashMap::new();
        if let Some(tree) = tree {
            self.changed = true;
            let root = match self.root {
                Some(root) => root,
                None => *self.root.insert(out.add(Object::Null)),
            };
            source.renames = source.maps.add(out, src, tree, &mut source.copier)?;
            let first = self.kids.is_empty();
            for top in kids_of(src, tree.get(b"K")) {
                let Some(new) = source.element(out, &top, root, 0)? else { continue };
                // The file's language stays with its content.
                if let Some(lang) = &lang
                    && let Some(mut d) = out.get(new).as_dict().cloned()
                    && !d.contains(b"Lang")
                {
                    d.set(b"Lang".to_vec(), lang.clone());
                    out.set(new, Object::Dict(d));
                }
                if first {
                    self.kids.push(Object::Ref(new));
                } else {
                    self.later.push(new);
                }
            }
            source.finish(out);
            if let Some(parent_tree) = tree.get(b"ParentTree") {
                numbers(src, parent_tree, 0, &mut HashSet::new(), &mut parents);
            }
        }
        // Pages, annotations and forms get new keys; a key that answers nothing now is dropped,
        // or it would answer for another file's content.
        let mut pages: Vec<(ObjRef, ObjRef)> = pages.iter().map(|(a, b)| (*a, *b)).collect();
        pages.sort_unstable_by_key(|(_, copy)| copy.num);
        let mut objects: Vec<(ObjRef, ObjRef)> = objects.iter().map(|(a, b)| (*a, *b)).collect();
        objects.sort_unstable_by_key(|(_, copy)| copy.num);
        for (from, copy) in pages.into_iter().chain(objects) {
            let target = annotation(out, copy);
            for key in [&b"StructParents"[..], b"StructParent"] {
                let entry = key_of(src, from, key).and_then(|k| parents.get(&k)).and_then(|v| source.entry(v));
                let number = entry.map(|entry| {
                    let number = self.next;
                    self.next = self.next.saturating_add(1);
                    self.nums.push((number, entry));
                    number
                });
                put(out, target, key, number);
            }
        }
        Ok(())
    }

    /// Write the tree into `out`'s catalog, when any source had one.
    pub(super) fn finish(self, out: &mut Document) -> Result<(), OrganizeError> {
        let Some(root) = self.root else { return Ok(()) };
        let catalog = out.root().ok_or_else(|| OrganizeError::Invalid("the document has no catalog".into()))?;
        let mut info = out.get(catalog).as_dict().and_then(|c| c.get(b"MarkInfo")).and_then(|m| out.dict(m)).unwrap_or_default();
        // Only a document tagged throughout says it is tagged.
        let said = marked(out);
        let mark = (said == self.untagged).then(|| {
            info.set(b"Marked".to_vec(), Object::Bool(!self.untagged));
            info
        });
        if !self.changed {
            // Nothing new in the tree; pages without tags can still take the claim away.
            if let Some(info) = mark {
                out.update_dict(catalog, |c| c.set(b"MarkInfo".to_vec(), Object::Dict(info)))?;
            }
            return Ok(());
        }
        let mut kids = self.kids;
        let mut order = Order::new(crate::pages(out)?.iter().map(|p| p.obj));
        for element in self.later {
            order.place(out, &mut kids, root, element, 0);
        }
        let mut parent_tree = Dict::new();
        let nums = self.nums.into_iter().flat_map(|(k, v)| [Object::Int(k), v]).collect();
        parent_tree.set(b"Nums".to_vec(), Object::Array(nums));
        let parent_tree = out.add(Object::Dict(parent_tree));
        let mut tree = self.base;
        tree.set(b"Type".to_vec(), Object::name("StructTreeRoot"));
        tree.set(b"K".to_vec(), Object::Array(kids));
        tree.set(b"ParentTree".to_vec(), Object::Ref(parent_tree));
        tree.set(b"ParentTreeNextKey".to_vec(), Object::Int(self.next));
        self.maps.write(out, &mut tree);
        out.set(root, Object::Dict(tree));
        let lang = self.lang.flatten();
        out.update_dict(catalog, |c| {
            c.set(b"StructTreeRoot".to_vec(), Object::Ref(root));
            if let Some(info) = mark {
                c.set(b"MarkInfo".to_vec(), Object::Dict(info));
            }
            if let Some(lang) = lang {
                c.set(b"Lang".to_vec(), lang);
            }
        })?;
        Ok(())
    }
}
