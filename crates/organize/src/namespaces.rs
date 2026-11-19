//! PDF 2.0 structure namespaces (ISO 32000-2 §14.7.4.3): the array `/Namespaces` of the
//! structure tree root lists namespace dictionaries, each with a `/NS` identifier (usually a URI),
//! an optional `/Schema` file specification and an optional `/RoleMapNS` dictionary that maps the
//! structure types the namespace defines to a role in the same or another namespace.
//!
//! Reading only: [`list_structure_namespaces`] changes nothing and does not check that the
//! namespaces are used or that the role maps are consistent. It reports `/NS` values that more
//! than one listed dictionary carries (a namespace is identified by its `/NS`, so two dictionaries with the same `/NS` describe one namespace twice (ISO 32000-2 14.7.4)),
//! compared as the listed text (cut at [`MAX_NS_CHARS`]).

use std::collections::{HashMap, HashSet};

use pdfcraft_cos::{Document, ObjRef, Object};

/// Most namespace dictionaries read.
pub const MAX_NAMESPACES: usize = 1000;
/// Most `/RoleMapNS` entries listed for one namespace (all are counted).
pub const MAX_ROLE_ENTRIES: usize = 64;
/// Most characters of one identifier, type or role kept.
pub const MAX_NS_CHARS: usize = 200;

/// One `/RoleMapNS` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamespaceRole {
    /// The structure type the namespace defines.
    pub structure_type: String,
    /// The role it maps to: a name, or the name of an `[name namespace]` pair; `None` otherwise.
    pub role: Option<String>,
    /// The `/NS` of the namespace a pair points at, when it is a namespace dictionary with one.
    pub role_namespace: Option<String>,
}

/// One namespace dictionary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructureNamespace {
    /// Object number, when the dictionary is indirect (it should be).
    pub object: Option<u32>,
    /// `/NS` as short text; `None` when missing or not a string.
    pub ns: Option<String>,
    /// Whether `/Schema` is present.
    pub has_schema: bool,
    /// Number of `/RoleMapNS` entries.
    pub role_count: usize,
    /// The first [`MAX_ROLE_ENTRIES`] entries, in the order the dictionary holds them.
    pub roles: Vec<NamespaceRole>,
}

/// Most objects named for one repeated `/NS`.
pub const MAX_DUPLICATE_OBJECTS: usize = 16;

/// An `/NS` identifier that more than one listed namespace dictionary has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplicateNamespace {
    /// The identifier as listed.
    pub ns: String,
    /// How many listed dictionaries carry it (at least 2).
    pub count: usize,
    /// Their object numbers (`None` for a direct dictionary), the first [`MAX_DUPLICATE_OBJECTS`].
    pub objects: Vec<Option<u32>>,
}

/// What [`list_structure_namespaces`] read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NamespaceList {
    /// Whether the catalog has a `/StructTreeRoot` dictionary.
    pub has_structure_tree: bool,
    pub namespaces: Vec<StructureNamespace>,
    /// `/Namespaces` items that are not dictionaries.
    pub skipped: usize,
    /// More namespaces exist than were read.
    pub truncated: bool,
    /// `/NS` values carried by more than one of the namespaces read, in order of first appearance.
    pub duplicates: Vec<DuplicateNamespace>,
}

fn short(text: &str) -> String {
    text.chars().take(MAX_NS_CHARS).collect()
}

fn ns_text(doc: &Document, dict: &pdfcraft_cos::Dict) -> Option<String> {
    match &*doc.resolve(dict.get(b"NS")?) {
        Object::String(s) => Some(s.text_chars().take(MAX_NS_CHARS).collect::<String>()),
        _ => None,
    }
}

/// The structure namespaces of the document, at most [`MAX_NAMESPACES`]. A dictionary listed
/// twice by the same reference is read once.
pub fn list_structure_namespaces(doc: &Document) -> NamespaceList {
    let mut out = NamespaceList::default();
    let Some(catalog) = doc.root().map(|r| doc.get(r)) else { return out };
    let Some(root) = catalog.as_dict().and_then(|c| c.get(b"StructTreeRoot")).map(|o| doc.resolve(o)) else { return out };
    let Some(root) = root.as_dict() else { return out };
    out.has_structure_tree = true;
    let Some(list) = root.get(b"Namespaces").map(|o| doc.resolve(o)) else { return out };
    let Some(list) = list.as_array() else { return out };
    let mut seen: HashSet<ObjRef> = HashSet::new();
    for item in list {
        if out.namespaces.len() >= MAX_NAMESPACES {
            out.truncated = true;
            break;
        }
        let object = item.as_ref();
        if let Some(r) = object
            && !seen.insert(r)
        {
            // The same reference listed twice is listed once.
            continue;
        }
        let resolved = doc.resolve(item);
        let Some(dict) = resolved.as_dict() else {
            out.skipped = out.skipped.saturating_add(1);
            continue;
        };
        let mut roles = Vec::new();
        let mut role_count = 0usize;
        if let Some(map) = dict.get(b"RoleMapNS").map(|o| doc.resolve(o))
            && let Some(map) = map.as_dict()
        {
            role_count = map.len();
            for (key, value) in map.iter().take(MAX_ROLE_ENTRIES) {
                let (role, role_namespace) = match &*doc.resolve(value) {
                    Object::Name(n) => (Some(short(&String::from_utf8_lossy(n))), None),
                    Object::Array(pair) => {
                        let role = pair.first().map(|o| doc.resolve(o)).and_then(|o| match &*o {
                            Object::Name(n) => Some(short(&String::from_utf8_lossy(n))),
                            _ => None,
                        });
                        let target = pair.get(1).map(|o| doc.resolve(o)).and_then(|o| o.as_dict().and_then(|d| ns_text(doc, d)));
                        (role, target)
                    }
                    _ => (None, None),
                };
                roles.push(NamespaceRole { structure_type: short(&String::from_utf8_lossy(key)), role, role_namespace });
            }
        }
        out.namespaces.push(StructureNamespace {
            object: object.map(|r| r.num),
            ns: ns_text(doc, dict),
            has_schema: dict.get(b"Schema").is_some(),
            role_count,
            roles,
        });
    }
    out.duplicates = duplicate_namespaces(&out.namespaces);
    out
}

/// The `/NS` values that more than one of `namespaces` has. At most [`MAX_NAMESPACES`] rows are
/// given, so the work and the result are bounded.
fn duplicate_namespaces(namespaces: &[StructureNamespace]) -> Vec<DuplicateNamespace> {
    let mut index: HashMap<&str, usize> = HashMap::new();
    let mut groups: Vec<(&str, Vec<Option<u32>>, usize)> = Vec::new();
    for n in namespaces {
        let Some(ns) = n.ns.as_deref() else { continue };
        let slot = *index.entry(ns).or_insert_with(|| {
            groups.push((ns, Vec::new(), 0));
            groups.len().saturating_sub(1)
        });
        if let Some(g) = groups.get_mut(slot) {
            g.2 = g.2.saturating_add(1);
            if g.1.len() < MAX_DUPLICATE_OBJECTS {
                g.1.push(n.object);
            }
        }
    }
    groups.into_iter().filter(|g| g.2 > 1).map(|(ns, objects, count)| DuplicateNamespace { ns: ns.to_string(), count, objects }).collect()
}
