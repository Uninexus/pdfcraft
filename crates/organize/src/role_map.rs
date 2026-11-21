//! The role map of the structure tree: the `/RoleMap` dictionary of the catalog's
//! `/StructTreeRoot`, which maps the name of a structure type a file made up to the name of
//! another type (ISO 32000-2 §14.7.4.2). The accessibility checks, the reading order and the
//! tag lists of this application follow it with `Document::resolve`.
//!
//! This module reads and changes one entry at a time, and does nothing else: it does not check
//! that a role is a type the tree uses, that the type it maps to is a standard one, or that a
//! role is not itself a standard type. Entries read as they are stored, so a value that is not a
//! name is listed with no target.
//!
//! References are followed as `Document::resolve` follows them (the first object and up to 32
//! more), for `/StructTreeRoot`, for `/RoleMap` and for the value of an entry. A stream's
//! dictionary counts as a dictionary for `/StructTreeRoot` only: a `/RoleMap` that is a stream is
//! unreadable (its keys are the stream's, not roles), as is a `/RoleMap` that is the catalog or
//! the tree root itself, and a `/StructTreeRoot` that is the catalog. Reading borrows what is
//! direct; a name is cut at [`MAX_ROLE_BYTES`] bytes before it is copied.
//!
//! The refusal of a role that maps to itself or leads back to itself within [`MAX_LOOP_STEPS`]
//! steps is this module's own rule, not the file format's.
//!
//! A `/RoleMap` or a `/StructTreeRoot` that is one object with the catalog, the `/Pages` node, a
//! node under it (an inner `/Pages` node or a page, found by walking `/Kids` from the `/Pages` node)
//! or, for the maps, the tree root's other map (`/RoleMap` and `/ClassMap`) is unreadable and never
//! changed, because a change to it would be a change to that object (see [`with_map`]). Any other
//! dictionary a map or a tree root might be, such as a `/ParentTree`, an attribute object, the
//! document information dictionary, an annotation or a font, is not guarded: it is read and changed
//! as a role map.

use pdfcraft_cos::{Dict, Document, ObjRef, Object};

use crate::OrganizeError;
use crate::print_presets::{Held, held};

/// Longest role or target this module writes, and longest it reads as a whole: this module's
/// limit, not one of the file format's. A longer stored name is listed cut and not `exact`.
pub const MAX_ROLE_BYTES: usize = 127;
/// Most entries listed (all are counted).
pub const MAX_LISTED_ROLES: usize = 256;
/// Most steps followed along a chain of roles to see whether it leads back to where it began.
pub const MAX_LOOP_STEPS: usize = 64;
/// References `Document::resolve` follows after the first object it reads.
const MORE_HOPS: usize = 32;

/// One entry of the role map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleEntry {
    /// The structure type the file made up (the key), cut at [`MAX_ROLE_BYTES`] bytes.
    pub role: String,
    /// What it maps to, when the value is a name (after references), cut the same way.
    pub maps_to: Option<String>,
    /// Whether `role` and `maps_to` are shown whole: valid UTF-8 of at most [`MAX_ROLE_BYTES`]
    /// bytes. Only such a name can be asked for by this module.
    pub exact: bool,
    /// Whether following the names from `maps_to` gets back to `role` within
    /// [`MAX_LOOP_STEPS`] steps.
    pub loops: bool,
}

/// What [`read_role_map`] read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoleMap {
    /// Whether the catalog's `/StructTreeRoot` is a dictionary.
    pub has_structure_tree: bool,
    /// The first [`MAX_LISTED_ROLES`] entries, in the order the dictionary holds them.
    pub entries: Vec<RoleEntry>,
    /// How many entries the dictionary has.
    pub count: usize,
    /// More entries exist than were listed.
    pub truncated: bool,
    /// `struct_tree_root` when the catalog's `/StructTreeRoot` is neither a dictionary nor null,
    /// is behind too many references, or is one object with the catalog, the `/Pages` node or a
    /// node under it; `role_map` the same for the tree root's `/RoleMap`, which is also
    /// unreadable when it is a stream, or is one object with the tree root or its `/ClassMap`
    /// (a page tree of over `MAX_PAGE_TREE_NODES` nodes or `MAX_KIDS_SCANNED` `/Kids` entries, which
    /// cannot be checked, also makes either unreadable).
    pub unreadable: Vec<&'static str>,
}

/// What an entry of a dictionary is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Where {
    /// No entry, a null, or a reference that ends in nothing.
    Absent,
    /// A dictionary written in place.
    Direct,
    /// A dictionary (or a stream, where the caller accepts one), reached through one reference or
    /// a chain of them (the last one).
    Indirect(ObjRef),
    /// Anything else, including a chain longer than `Document::resolve` follows.
    Other,
}

pub(crate) fn locate(doc: &Document, entry: Option<&Object>, streams: bool) -> Where {
    let mut at = match entry {
        None | Some(Object::Null) => return Where::Absent,
        Some(Object::Dict(_)) => return Where::Direct,
        Some(Object::Ref(r)) => *r,
        Some(_) => return Where::Other,
    };
    for _ in 0..=MORE_HOPS {
        match &*doc.get(at) {
            Object::Dict(_) => return Where::Indirect(at),
            Object::Stream(_) if streams => return Where::Indirect(at),
            Object::Stream(_) => return Where::Other,
            Object::Null => return Where::Absent,
            Object::Ref(next) => at = *next,
            _ => return Where::Other,
        }
    }
    Where::Other
}

/// Most nodes [`reaches_page_tree_node`] lets the walk visit.
pub(crate) const MAX_PAGE_TREE_NODES: usize = 1_000_000;
/// Most `/Kids` entries [`reaches_page_tree_node`] lets the walk scan in all (a shared `/Kids`
/// array is scanned once for each node that lists it, so this bounds time as the node cap bounds
/// memory).
pub(crate) const MAX_KIDS_SCANNED: usize = 2_000_000;

/// Whether `target` is the `/Pages` node or a node under it (an inner `/Pages` node or a page),
/// found by walking `/Kids` from the `/Pages` node. A node is put on the stack once, when it is
/// first seen, so the stack and the seen set hold at most `max_nodes` + 1 nodes; and every entry
/// of every `/Kids` array scanned is counted, at most `max_items` + 1 in all. A walk that sees
/// more than `max_nodes` nodes, or scans more than `max_items` entries, before it finds `target`
/// answers true, because it could not show that `target` is not one (exactly `max_nodes` nodes
/// and exactly `max_items` entries still answer by what was found). A document with no `/Pages`
/// node answers false.
pub(crate) fn reaches_page_tree_node(doc: &Document, target: ObjRef, max_nodes: usize, max_items: usize) -> bool {
    walk_page_tree(doc, target, max_nodes, max_items).0
}

/// [`reaches_page_tree_node`] and the most nodes the stack held (for the tests).
pub(crate) fn walk_page_tree(doc: &Document, target: ObjRef, max_nodes: usize, max_items: usize) -> (bool, usize) {
    let Some(top) = doc.root().and_then(|r| doc.get(r).as_dict().and_then(|c| c.reference(b"Pages"))) else {
        return (false, 0);
    };
    if top == target {
        return (true, 1);
    }
    let mut seen = std::collections::HashSet::from([top]);
    let mut todo = vec![top];
    let (mut scanned, mut peak) = (0usize, 1usize);
    while let Some(at) = todo.pop() {
        let node = doc.get(at);
        let Some(kids) = node.as_dict().and_then(|d| d.get(b"Kids")) else { continue };
        let kids = doc.resolve(kids);
        let Some(items) = kids.as_array() else { continue };
        for item in items {
            scanned += 1;
            if scanned > max_items {
                return (true, peak);
            }
            let Some(kid) = item.as_ref() else { continue };
            if kid == target {
                return (true, peak);
            }
            if seen.insert(kid) {
                if seen.len() > max_nodes {
                    return (true, peak);
                }
                todo.push(kid);
                peak = peak.max(todo.len());
            }
        }
    }
    (false, peak)
}

/// Runs `f` with where the tree root and the map `/<map_key>` of the tree root (`RoleMap`,
/// `ClassMap`) are, and that map's dictionary.
///
/// Why the guards: the tree root and a map are changed where they are stored, so one that is the
/// same object as the catalog, the `/Pages` node, a node under it, or (for a map) the tree root or
/// the tree root's other map would change that object (a role named `Kids` in the `/Pages` node
/// breaks the page tree). Each is reported as `Where::Other`, which the callers name as
/// unreadable and never write. The page tree is walked only for a tree root or a map that is
/// indirect.
pub(crate) fn with_map<R>(doc: &Document, map_key: &[u8], f: impl FnOnce(Where, Where, Option<&Dict>) -> R) -> R {
    let Some(catalog) = doc.root().map(|r| doc.get(r)) else { return f(Where::Absent, Where::Absent, None) };
    let Some(catalog) = catalog.as_dict() else { return f(Where::Absent, Where::Absent, None) };
    let entry = catalog.get(b"StructTreeRoot");
    let catalog_ref = doc.root();
    let mut tree = locate(doc, entry, true);
    if matches!(tree, Where::Indirect(r) if Some(r) == catalog_ref || reaches_page_tree_node(doc, r, MAX_PAGE_TREE_NODES, MAX_KIDS_SCANNED)) {
        tree = Where::Other;
    }
    let root: Option<Held> = match (tree, entry) {
        (Where::Direct, Some(o)) => Some(Held::Direct(o)),
        (Where::Indirect(r), _) => Some(Held::Resolved(doc.get(r))),
        _ => None,
    };
    let root_dict = root.as_deref().and_then(Object::as_dict);
    let entry = root_dict.and_then(|d| d.get(map_key));
    let mut map = if root_dict.is_some() { locate(doc, entry, false) } else { Where::Absent };
    // A map that is the catalog, the tree root, the tree root's other map, the `/Pages` node or a
    // node under it would be edited as a map.
    if let Where::Indirect(r) = map {
        let other_key: &[u8] = if map_key == b"RoleMap" { b"ClassMap" } else { b"RoleMap" };
        let sibling = if root_dict.is_some() { locate(doc, root_dict.and_then(|d| d.get(other_key)), false) } else { Where::Absent };
        if Some(r) == catalog_ref
            || tree == Where::Indirect(r)
            || sibling == Where::Indirect(r)
            || reaches_page_tree_node(doc, r, MAX_PAGE_TREE_NODES, MAX_KIDS_SCANNED)
        {
            map = Where::Other;
        }
    }
    let map_held: Option<Held> = match (map, entry) {
        (Where::Direct, Some(o)) => Some(Held::Direct(o)),
        (Where::Indirect(r), _) => Some(Held::Resolved(doc.get(r))),
        _ => None,
    };
    f(tree, map, map_held.as_deref().and_then(Object::as_dict))
}

/// What the entry `key` of `map` holds: its value after references, when there is an entry.
fn value<'m>(doc: &Document, map: &'m Dict, key: &[u8]) -> Option<Held<'m>> {
    map.get(key).map(|o| held(doc, o))
}

/// Whether following the names from `first`, what `role` maps to, gets back to `role` within
/// [`MAX_LOOP_STEPS`] steps.
fn leads_back(doc: &Document, map: &Dict, role: &[u8], first: &[u8]) -> bool {
    if first == role {
        return true;
    }
    let Some(mut cur) = value(doc, map, first) else { return false };
    for _ in 0..MAX_LOOP_STEPS {
        let Some(name) = cur.as_name() else { return false };
        if name == role {
            return true;
        }
        match value(doc, map, name) {
            Some(next) => cur = next,
            None => return false,
        }
    }
    false
}

/// A name as text: cut at [`MAX_ROLE_BYTES`] bytes before it is copied, and whether it is shown
/// whole.
pub(crate) fn shown(name: &[u8]) -> (String, bool) {
    let cut = name.get(..MAX_ROLE_BYTES).unwrap_or(name);
    (String::from_utf8_lossy(cut).into_owned(), name.len() <= MAX_ROLE_BYTES && std::str::from_utf8(name).is_ok())
}

/// The role map as stored. One read of the catalog, the tree root and the role map.
pub fn read_role_map(doc: &Document) -> RoleMap {
    with_map(doc, b"RoleMap", |tree, map, dict| {
        let mut out = RoleMap { has_structure_tree: matches!(tree, Where::Direct | Where::Indirect(_)), ..RoleMap::default() };
        match (tree, map) {
            (Where::Other, _) => out.unreadable.push("struct_tree_root"),
            (_, Where::Other) => out.unreadable.push("role_map"),
            _ => {}
        }
        let Some(dict) = dict else { return out };
        out.count = dict.len();
        out.truncated = dict.len() > MAX_LISTED_ROLES;
        for (key, v) in dict.iter().take(MAX_LISTED_ROLES) {
            let target = held(doc, v);
            let target = target.as_name();
            let (role, role_exact) = shown(key);
            let (maps_to, to_exact) = match target {
                Some(n) => {
                    let (text, exact) = shown(n);
                    (Some(text), exact)
                }
                None => (None, true),
            };
            out.entries.push(RoleEntry {
                role,
                maps_to,
                exact: role_exact && to_exact,
                loops: target.is_some_and(|n| leads_back(doc, dict, key, n)),
            });
        }
        out
    })
}

/// Whether a role name or a target is acceptable: 1 to [`MAX_ROLE_BYTES`] bytes with no NUL.
fn check_name(what: &str, name: &str) -> Result<(), String> {
    if name.bytes().any(|b| b == 0) {
        return Err(format!("{what} must not contain a NUL byte"));
    }
    if name.is_empty() || name.len() > MAX_ROLE_BYTES {
        return Err(format!("{what} must be 1 to {MAX_ROLE_BYTES} bytes, not {} bytes", name.len()));
    }
    Ok(())
}

/// Whether [`set_role_map`] with these arguments would write: false when the value of `role` is
/// already the name asked for, or when `role` has no entry and none is asked for. A stored value
/// that is not a name is a change to any name. Taking away an entry that is stored is a change
/// whatever its key, so that a stored key this module could not write can be cleaned up; taking
/// away an entry that is not stored is refused when `role` is not a name this module writes, and is
/// no change otherwise.
pub fn role_map_changes(doc: &Document, role: &str, maps_to: Option<&str>) -> Result<bool, OrganizeError> {
    let (stored, name_matches) = with_map(doc, b"RoleMap", |_, _, dict| {
        let stored = dict.and_then(|d| value(doc, d, role.as_bytes()));
        let matches = match (stored.as_deref(), maps_to) {
            (Some(o), Some(m)) => o.as_name() == Some(m.as_bytes()),
            _ => false,
        };
        (stored.is_some(), matches)
    });
    match (stored, maps_to) {
        (false, None) => check_name("a role", role).map(|()| false).map_err(OrganizeError::Invalid),
        (true, Some(_)) => Ok(!name_matches),
        _ => Ok(true),
    }
}

/// Map `role` to `maps_to`, or take its entry away (`None`). Only a change writes: see
/// [`role_map_changes`]; a stored entry that this module would refuse as a new one may be sent
/// back as it is, and a stored key may be taken away whatever it is. Nothing is written when a
/// name is not 1 to [`MAX_ROLE_BYTES`] bytes without a NUL byte (this includes the role of a
/// removal that matches no entry), when `role` would map to itself or to a name whose entries lead
/// back to `role` within [`MAX_LOOP_STEPS`] steps (this module's rule), when the document has no
/// structure tree, or when the tree root or its `/RoleMap` is unreadable (see [`RoleMap`]): such
/// a value is never replaced. Other entries are kept and an emptied role map goes. A null or
/// dangling `/RoleMap` counts as absent and an entry starts a dictionary; a dictionary behind
/// references is changed where it is. A value that is not a name is replaced by a name asked for.
pub fn set_role_map(doc: &mut Document, role: &str, maps_to: Option<&str>) -> Result<(), OrganizeError> {
    if !role_map_changes(doc, role, maps_to)? {
        return Ok(());
    }
    if let Some(target) = maps_to {
        check_name("a role", role).map_err(OrganizeError::Invalid)?;
        check_name("what a role maps to", target).map_err(OrganizeError::Invalid)?;
        if target == role {
            return Err(OrganizeError::Invalid(format!("the role {role} cannot map to itself")));
        }
    }
    let catalog_ref = doc.root().ok_or(OrganizeError::NoPageTree)?;
    let (tree, map, loops) = with_map(doc, b"RoleMap", |tree, map, dict| {
        let loops = match (maps_to, dict) {
            (Some(t), Some(d)) => leads_back(doc, d, role.as_bytes(), t.as_bytes()),
            _ => false,
        };
        (tree, map, loops)
    });
    if maps_to.is_some() {
        match (tree, map) {
            (Where::Absent, _) => {
                return Err(OrganizeError::Invalid("the document has no structure tree, so there is no role map to change".into()));
            }
            (Where::Other, _) | (_, Where::Other) => {
                return Err(OrganizeError::Invalid(format!(
                    "the structure tree's /StructTreeRoot or /RoleMap cannot be changed (it is not a dictionary, is behind more than {} references, is a stream role map, or is the same object as the catalog, the /Pages node, a page or inner /Pages node, or (for a map) the tree root or its other map, or the page tree has over {MAX_PAGE_TREE_NODES} nodes or {MAX_KIDS_SCANNED} /Kids entries and could not be checked), so it was left as it is",
                    MORE_HOPS + 1
                )));
            }
            _ => {}
        }
        if loops {
            return Err(OrganizeError::Invalid(format!(
                "{} already leads back to {role}, so mapping {role} to it would make a loop",
                maps_to.unwrap_or_default()
            )));
        }
    }
    let key = role.as_bytes().to_vec();
    let apply = |d: &mut Dict| match maps_to {
        Some(t) => d.set(key.clone(), Object::name(t)),
        None => {
            d.remove(&key);
        }
    };
    apply_to_map(doc, catalog_ref, tree, map, b"RoleMap", &apply)
}

/// Applies `apply` to the map `/<map_key>` of the tree root where it is stored: in place when it
/// is a dictionary (behind references or not), or in a new direct dictionary when it is absent.
/// A map that `apply` leaves empty is taken out of the tree root (an indirect one is left
/// unreferenced). `tree` and `map` are what [`with_map`] found, and neither is unreadable.
pub(crate) fn apply_to_map(
    doc: &mut Document,
    catalog_ref: ObjRef,
    tree: Where,
    map: Where,
    map_key: &[u8],
    apply: &dyn Fn(&mut Dict),
) -> Result<(), OrganizeError> {
    let root_ref = match tree {
        Where::Indirect(r) => Some(r),
        _ => None,
    };
    let change_root = |doc: &mut Document, f: &dyn Fn(&mut Dict)| -> Result<(), OrganizeError> {
        match root_ref {
            Some(r) => doc.update_dict(r, |d| f(d))?,
            None => doc.update_dict(catalog_ref, |c| {
                if let Some(Object::Dict(d)) = c.get_mut(b"StructTreeRoot") {
                    f(d);
                }
            })?,
        }
        Ok(())
    };
    match map {
        Where::Indirect(r) => {
            doc.update_dict(r, apply)?;
            if matches!(&*doc.get(r), Object::Dict(d) if d.is_empty()) {
                change_root(doc, &|d| {
                    d.remove(map_key);
                })?;
            }
        }
        _ => change_root(doc, &|root| {
            let emptied = match root.get_mut(map_key) {
                Some(Object::Dict(d)) => {
                    apply(d);
                    d.is_empty()
                }
                _ => {
                    let mut d = Dict::new();
                    apply(&mut d);
                    if !d.is_empty() {
                        root.set(map_key.to_vec(), Object::Dict(d));
                    }
                    false
                }
            };
            if emptied {
                root.remove(map_key);
            }
        })?,
    }
    Ok(())
}
