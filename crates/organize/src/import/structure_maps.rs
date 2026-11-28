//! What a structure tree's root holds beside its elements, brought together by Combine: the role
//! map and class map (ISO 32000-2 §14.7.3, §14.7.6.2), element identifiers (the ID tree) and
//! namespaces (§14.7.4). A name or identifier one file already used for something else is
//! renamed for the later file, with everything of its that refers to it.

use std::collections::BTreeMap;

use pdfcraft_cos::PdfString;

use super::copy::Copier;
use super::{Dict, Document, HashMap, HashSet, ObjRef, Object, OrganizeError};

/// The most entries one file's role map, class map or namespace list brings.
const MAX_ENTRIES: usize = 10_000;
/// How deep two values are compared.
const MAX_DEPTH: usize = 32;

/// The combined root entries.
#[derive(Default)]
pub(super) struct Maps {
    roles: Dict,
    classes: Dict,
    /// Identifier → element, sorted as the name tree must be.
    ids: BTreeMap<Vec<u8>, ObjRef>,
    namespaces: Vec<ObjRef>,
    /// Namespace name → the namespace used for it, when it is nothing but its name.
    plain: HashMap<Vec<u8>, ObjRef>,
}

/// What one file's names are called in the combined tree, when not by their own name.
#[derive(Default)]
pub(super) struct Renames {
    roles: HashMap<Vec<u8>, Vec<u8>>,
    classes: HashMap<Vec<u8>, Vec<u8>>,
    ids: HashMap<Vec<u8>, Vec<u8>>,
}

fn renamed<'a>(map: &'a HashMap<Vec<u8>, Vec<u8>>, name: &'a [u8]) -> &'a [u8] {
    map.get(name).map_or(name, Vec::as_slice)
}

/// `name` with the first number that makes it a name `taken` doesn't hold.
fn free(name: &[u8], taken: impl Fn(&[u8]) -> bool) -> Vec<u8> {
    let mut number = 2u32;
    loop {
        let mut candidate = name.to_vec();
        candidate.extend_from_slice(format!("_{number}").as_bytes());
        if !taken(&candidate) || number == u32::MAX {
            return candidate;
        }
        number += 1;
    }
}

/// Whether `a` of one document says what `b` of another says.
fn same(da: &Document, a: &Object, db: &Document, b: &Object, depth: usize) -> bool {
    if depth >= MAX_DEPTH {
        return false;
    }
    let (a, b) = (da.resolve(a), db.resolve(b));
    match (&*a, &*b) {
        (Object::Dict(x), Object::Dict(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(da, v, db, w, depth + 1))),
        (Object::Array(x), Object::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(v, w)| same(da, v, db, w, depth + 1)),
        (x, y) => x == y,
    }
}

/// The dictionary `tree` holds under `key`.
fn held(src: &Document, tree: &Dict, key: &[u8]) -> Dict {
    tree.get(key).and_then(|o| src.dict(o)).unwrap_or_default()
}

impl Maps {
    /// Take in the root entries of `tree`, one file's structure tree.
    pub(super) fn add(&mut self, out: &mut Document, src: &Document, tree: &Dict, copier: &mut Copier<'_>) -> Result<Renames, OrganizeError> {
        self.namespaces(out, src, tree, copier)?;
        let mut renames = Renames::default();

        let roles: Vec<(Vec<u8>, Vec<u8>)> =
            held(src, tree, b"RoleMap").iter().take(MAX_ENTRIES).filter_map(|(k, v)| Some((k.clone(), src.resolve(v).as_name()?.to_vec()))).collect();
        let used = |name: &[u8]| roles.iter().any(|(k, v)| k == name || v == name);
        // A rename changes what other entries map to, which can make them differ too.
        loop {
            let clash = roles.iter().find(|(from, to)| {
                let there = self.roles.get(renamed(&renames.roles, from)).and_then(Object::as_name);
                there.is_some_and(|there| there != renamed(&renames.roles, to))
            });
            let Some((from, _)) = clash else { break };
            if renames.roles.contains_key(from) {
                break;
            }
            let name = free(from, |c| self.roles.contains(c) || used(c));
            renames.roles.insert(from.clone(), name);
        }
        for (from, to) in &roles {
            let name = renamed(&renames.roles, from).to_vec();
            if !self.roles.contains(&name) {
                self.roles.set(name, Object::Name(renamed(&renames.roles, to).to_vec()));
            }
        }

        let classes = held(src, tree, b"ClassMap");
        for (name, value) in classes.iter().take(MAX_ENTRIES) {
            let mut target = name.clone();
            if let Some(there) = self.classes.get(name) {
                // What is there is a copy in `out`.
                if same(src, value, out, there, 0) {
                    continue;
                }
                target = free(name, |c| self.classes.contains(c) || classes.contains(c));
                renames.classes.insert(name.clone(), target.clone());
            }
            let copy = copier.copy_value(out, value, None)?;
            self.classes.set(target, copy);
        }
        Ok(renames)
    }

    fn namespaces(&mut self, out: &mut Document, src: &Document, tree: &Dict, copier: &mut Copier<'_>) -> Result<(), OrganizeError> {
        let list = tree.get(b"Namespaces").map(|n| src.resolve(n));
        for item in list.as_deref().and_then(Object::as_array).into_iter().flatten().take(MAX_ENTRIES) {
            let Some(from) = item.as_ref() else { continue };
            let Some(dict) = src.dict(item) else { continue };
            let name = dict.get(b"NS").and_then(|n| src.resolve(n).as_string().map(|s| s.bytes.clone()));
            // One that is only a name is one namespace however many files declare it.
            let bare = dict.iter().all(|(k, _)| matches!(k.as_slice(), b"Type" | b"NS"));
            if let Some(shared) = name.as_ref().filter(|_| bare).and_then(|n| self.plain.get(n)) {
                copier.map.insert(from, *shared);
                continue;
            }
            let Object::Ref(copy) = copier.copy_value(out, item, None)? else { continue };
            self.namespaces.push(copy);
            if let Some(name) = name.filter(|_| bare) {
                self.plain.insert(name, copy);
            }
        }
        Ok(())
    }

    /// Give the element `copy`, to be stored as `at`, the combined tree's names.
    pub(super) fn element(&mut self, renames: &mut Renames, copy: &mut Dict, at: ObjRef) {
        // A type in a namespace of its own is not looked up in the role map.
        if !copy.contains(b"NS")
            && let Some(name) = copy.name(b"S").and_then(|s| renames.roles.get(s))
        {
            copy.set(b"S".to_vec(), Object::Name(name.clone()));
        }
        if let Some(classes) = copy.get(b"C") {
            let rename = |o: &Object| match o {
                Object::Name(n) => Object::Name(renamed(&renames.classes, n).to_vec()),
                other => other.clone(),
            };
            let classes = match classes {
                // Class names, each perhaps followed by its revision number.
                Object::Array(list) => Object::Array(list.iter().map(rename).collect()),
                one => rename(one),
            };
            copy.set(b"C".to_vec(), classes);
        }
        if let Some(Object::String(id)) = copy.get(b"ID") {
            let (mut bytes, hex) = (id.bytes.clone(), id.hex);
            if self.ids.contains_key(&bytes) {
                let name = free(&bytes, |c| self.ids.contains_key(c));
                renames.ids.insert(bytes, name.clone());
                copy.set(b"ID".to_vec(), Object::String(PdfString { bytes: name.clone(), hex }));
                bytes = name;
            }
            self.ids.insert(bytes, at);
        }
    }

    /// Point the `/Headers` of table cell attributes (§14.8.5.7), in the attributes `value`, at
    /// renamed identifiers. `done` holds the objects already seen: one attribute object can serve
    /// many elements, and is changed once.
    pub(super) fn headers(out: &mut Document, renames: &Renames, value: &Object, done: &mut HashSet<ObjRef>, depth: usize) -> Option<Object> {
        if renames.ids.is_empty() || depth >= MAX_DEPTH {
            return None;
        }
        match value {
            Object::Ref(r) => {
                if done.insert(*r) {
                    let held = (*out.get(*r)).clone();
                    if let Some(new) = Self::headers(out, renames, &held, done, depth + 1) {
                        out.set(*r, new);
                    }
                }
                None
            }
            Object::Array(list) => {
                let new: Vec<Option<Object>> = list.iter().map(|o| Self::headers(out, renames, o, done, depth + 1)).collect();
                let changed = new.iter().any(Option::is_some);
                changed.then(|| Object::Array(new.into_iter().zip(list).map(|(n, o)| n.unwrap_or_else(|| o.clone())).collect()))
            }
            Object::Dict(d) => {
                let ids = d.get(b"Headers").map(|h| out.resolve(h))?;
                let ids: Vec<Object> = ids
                    .as_array()?
                    .iter()
                    .map(|id| match id {
                        Object::String(s) => Object::String(PdfString { bytes: renamed(&renames.ids, &s.bytes).to_vec(), hex: s.hex }),
                        other => other.clone(),
                    })
                    .collect();
                let mut d = d.clone();
                d.set(b"Headers".to_vec(), Object::Array(ids));
                Some(Object::Dict(d))
            }
            _ => None,
        }
    }

    /// Write the entries into the combined tree root.
    pub(super) fn write(self, out: &mut Document, tree: &mut Dict) {
        if !self.roles.is_empty() {
            tree.set(b"RoleMap".to_vec(), Object::Dict(self.roles));
        }
        if !self.classes.is_empty() {
            tree.set(b"ClassMap".to_vec(), Object::Dict(self.classes));
        }
        if !self.ids.is_empty() {
            let names = self.ids.into_iter().flat_map(|(id, r)| [Object::String(PdfString::literal(id)), Object::Ref(r)]).collect();
            let mut leaf = Dict::new();
            leaf.set(b"Names".to_vec(), Object::Array(names));
            let leaf = out.add(Object::Dict(leaf));
            tree.set(b"IDTree".to_vec(), Object::Ref(leaf));
        }
        if !self.namespaces.is_empty() {
            tree.set(b"Namespaces".to_vec(), Object::Array(self.namespaces.into_iter().map(Object::Ref).collect()));
        }
    }
}
