//! The class map of the structure tree: the `/ClassMap` dictionary of the catalog's
//! `/StructTreeRoot`, which maps a class name to an attribute object or to an array of them
//! (ISO 32000-2 §14.7.5.4). A structure element names classes in its `/C`, and takes the
//! attributes of the classes it names.
//!
//! This module reads the class map and takes one entry away by its name, and does nothing else:
//! it writes no attribute object, adds no class and checks no `/C`. Where the tree root and the
//! map are, and what is unreadable, is decided as for the role map (see [`crate::role_map`]):
//! references are followed as `Document::resolve` follows them, a `/ClassMap` that is a stream, the
//! catalog or the tree root itself is unreadable and never changed, and a `/StructTreeRoot` that is
//! the catalog is too. Reading borrows what is direct; a name is cut at
//! [`MAX_CLASS_NAME_BYTES`] bytes before it is copied, and an array is measured before any of its
//! items is looked at.
//!
//! No guard refuses a `/ClassMap` that is another live dictionary (the `/Pages` node, a page, the
//! `/RoleMap` dictionary): it is read and changed as a class map, so removing `Kids` from one that
//! is the `/Pages` node would break the page tree.

use pdfcraft_cos::{Document, Object};

use crate::OrganizeError;
use crate::print_presets::held;
use crate::role_map::{Where, apply_to_map, shown, with_map};

/// Longest class name or owner listed whole: this module's limit for listing, not the file
/// format's. A longer name is listed cut and not `exact`.
pub const MAX_CLASS_NAME_BYTES: usize = crate::role_map::MAX_ROLE_BYTES;
/// Most entries listed (all are counted).
pub const MAX_LISTED_CLASSES: usize = 256;
/// Most attribute objects of one entry whose owner is listed (all are counted).
pub const MAX_LISTED_OWNERS: usize = 16;

/// One entry of the class map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassEntry {
    /// The class name (the key), cut at [`MAX_CLASS_NAME_BYTES`] bytes.
    pub class: String,
    /// What the value is after references: `dictionary` or `stream` (one attribute object),
    /// `array` (a list of them), or `other` (anything else, including a null or a reference that
    /// ends in nothing).
    pub kind: &'static str,
    /// How many attribute objects the value holds: 1 for a dictionary or a stream, the length of
    /// an array (whatever its items are), 0 for `other`.
    pub items: usize,
    /// The `/O` (owner) name of the first [`MAX_LISTED_OWNERS`] attribute objects, null for an
    /// item that is not a dictionary or a stream, or has no `/O` that is a name; cut like `class`.
    pub owners: Vec<Option<String>>,
    /// Whether `class` and every listed owner are shown whole: valid UTF-8 of at most
    /// [`MAX_CLASS_NAME_BYTES`] bytes. A class that is listed cut and longer than that can be
    /// removed only by its full name, and one that is not valid UTF-8 cannot be removed at all.
    pub exact: bool,
}

/// What [`read_class_map`] read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClassMap {
    /// Whether the catalog's `/StructTreeRoot` is a dictionary.
    pub has_structure_tree: bool,
    /// The first [`MAX_LISTED_CLASSES`] entries, in the order the dictionary holds them.
    pub entries: Vec<ClassEntry>,
    /// How many entries the dictionary has.
    pub count: usize,
    /// More entries exist than were listed.
    pub truncated: bool,
    /// `struct_tree_root` when the catalog's `/StructTreeRoot` is neither a dictionary nor null,
    /// is behind too many references, or is the catalog itself; `class_map` the same for the tree
    /// root's `/ClassMap`, which is also unreadable when it is a stream or is the catalog or the
    /// tree root.
    pub unreadable: Vec<&'static str>,
}

/// The `/O` of an attribute object (a dictionary, or a stream's dictionary), when it is a name.
fn owner(doc: &Document, object: &Object) -> Option<(String, bool)> {
    let dict = match object {
        Object::Dict(d) => d,
        Object::Stream(s) => &s.dict,
        _ => return None,
    };
    let o = held(doc, dict.get(b"O")?);
    o.as_name().map(shown)
}

/// The class map as stored. One read of the catalog, the tree root and the class map.
pub fn read_class_map(doc: &Document) -> ClassMap {
    with_map(doc, b"ClassMap", |tree, map, dict| {
        let mut out = ClassMap { has_structure_tree: matches!(tree, Where::Direct | Where::Indirect(_)), ..ClassMap::default() };
        match (tree, map) {
            (Where::Other, _) => out.unreadable.push("struct_tree_root"),
            (_, Where::Other) => out.unreadable.push("class_map"),
            _ => {}
        }
        let Some(dict) = dict else { return out };
        out.count = dict.len();
        out.truncated = dict.len() > MAX_LISTED_CLASSES;
        for (key, v) in dict.iter().take(MAX_LISTED_CLASSES) {
            let value = held(doc, v);
            let (class, mut exact) = shown(key);
            let (kind, items, owners) = match &*value {
                Object::Dict(_) => ("dictionary", 1, vec![owner(doc, &value)]),
                Object::Stream(_) => ("stream", 1, vec![owner(doc, &value)]),
                Object::Array(items) => ("array", items.len(), items.iter().take(MAX_LISTED_OWNERS).map(|i| owner(doc, &held(doc, i))).collect()),
                _ => ("other", 0, Vec::new()),
            };
            exact &= owners.iter().flatten().all(|(_, whole)| *whole);
            out.entries.push(ClassEntry { class, kind, items, owners: owners.into_iter().map(|o| o.map(|(n, _)| n)).collect(), exact });
        }
        out
    })
}

/// Whether [`remove_class_map_entry`] with this class would write: whether the class map has an
/// entry with exactly this key, whatever the key is and whatever the value is. A class map that
/// is unreadable has no entries here.
pub fn class_map_removes(doc: &Document, class: &str) -> bool {
    with_map(doc, b"ClassMap", |_, _, dict| dict.is_some_and(|d| d.get(class.as_bytes()).is_some()))
}

/// Take the entry `class` away from the class map, and the class map with it when it was the
/// last entry (an indirect one is taken out of the tree root and its object is left
/// unreferenced). Other entries are kept, and an indirect attribute object the entry pointed at is
/// left where it is (a direct one goes with the entry). A class that is not stored is no change, and so is a class map that is absent,
/// null, dangling or unreadable (see [`ClassMap`]): such a value is never replaced.
pub fn remove_class_map_entry(doc: &mut Document, class: &str) -> Result<(), OrganizeError> {
    // Not only a saving of a copy: this also keeps an unreadable map (or tree root) out of
    // `apply_to_map`, which would otherwise be handed a `Where::Other` it is not meant to see.
    if !class_map_removes(doc, class) {
        return Ok(());
    }
    let catalog_ref = doc.root().ok_or(OrganizeError::NoPageTree)?;
    let (tree, map) = with_map(doc, b"ClassMap", |tree, map, _| (tree, map));
    let key = class.as_bytes().to_vec();
    apply_to_map(doc, catalog_ref, tree, map, b"ClassMap", &|d| {
        d.remove(&key);
    })
}
