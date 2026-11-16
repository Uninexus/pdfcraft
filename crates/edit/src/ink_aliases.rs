//! Print production ▸ Ink Manager ▸ Ink aliases: spot colorants renamed so that two inks print
//! as one. The names in every Separation and DeviceN colour space of the document are
//! rewritten (colour spaces in page and form resources, in images, patterns and shadings, and
//! those kept as their own objects); nothing else changes. Each space keeps its own alternate
//! space and tint transform, so what the page looks like on screen stays as it is.
//!
//! A DeviceN space is rewritten whole: its names, its colorants dictionary and its mixing hints
//! (solidities, dot gain, printing order), wherever they are stored, direct or as objects of their
//! own. Everything is worked out before anything is written, so a refusal changes nothing.
//!
//! [`list_inks`] reads the spot colorants a document uses. Colorants named `/All` and `/None`,
//! and DeviceCMYK's own (`Cyan`, `Magenta`, `Yellow`, `Black`), are not inks to alias, and are
//! not allowed as the name an ink is given: `/None` paints nothing and `/All` paints on every
//! plate, so the page would no longer look the same.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use pdfcraft_cos::{Dict, Document, ObjRef, Object};

use crate::EditError;

/// Colour space arrays nested deeper than this are not read (an Indexed space holds a base space
/// and so on; real files stop at two or three).
const MAX_DEPTH: usize = 16;
/// The most aliases one call may give.
pub const MAX_ALIASES: usize = 64;
/// The longest colorant name PDF allows (ISO 32000-2 Annex C).
const MAX_NAME_BYTES: usize = 127;
/// References followed to reach a part of a colour space.
const MAX_HOPS: usize = 8;

/// Which colorant is renamed to which.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InkAliases {
    /// `(from, to)` pairs: every spot colorant named `from` is named `to`. A name is looked up
    /// once, so `a → b` with `b → c` renames `a` to `b` and `b` to `c`, not `a` to `c`.
    pub aliases: Vec<(String, String)>,
}

impl InkAliases {
    /// Whether the aliases can be applied: at most [`MAX_ALIASES`], names of 1 to 127 bytes with
    /// no NUL, no name aliased twice, and no ink given the name of a process colorant, `All` or
    /// `None`. `Err` says what is wrong.
    pub fn check(&self) -> Result<(), String> {
        if self.aliases.len() > MAX_ALIASES {
            return Err(format!("at most {MAX_ALIASES} aliases"));
        }
        let mut seen = std::collections::HashSet::new();
        for (from, to) in &self.aliases {
            for name in [from, to] {
                if name.is_empty() || name.len() > MAX_NAME_BYTES || name.contains('\0') {
                    return Err(format!("an ink name is 1 to {MAX_NAME_BYTES} bytes with no NUL: {name:?}"));
                }
            }
            if !is_spot(to.as_bytes()) {
                return Err(format!("{to:?} is a process colorant, All or None, not a name an ink can be given"));
            }
            if !seen.insert(from.as_bytes().to_vec()) {
                return Err(format!("{from:?} is aliased twice"));
            }
        }
        Ok(())
    }

    fn lookup(&self, name: &[u8]) -> Option<&[u8]> {
        if !is_spot(name) {
            return None;
        }
        self.aliases.iter().find(|(from, to)| from.as_bytes() == name && from != to).map(|(_, to)| to.as_bytes())
    }
}

/// What [`alias_inks`] changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Aliased {
    /// Colour spaces (Separation and DeviceN) that had a colorant renamed.
    pub spaces: usize,
    /// Objects of the file rewritten.
    pub objects: usize,
    /// Names given to inks whose Separation spaces do not all share one alternate space and tint
    /// transform, so one name now prints two ways. Not an error, but PDF/X wants one definition
    /// per name.
    pub different_alternates: Vec<String>,
    /// Arrays, dictionaries and streams nested deeper than colour spaces are read to, left alone.
    pub skipped_nested: usize,
}

/// A spot colorant the document uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ink {
    /// The name; a name that is not UTF-8 is shown as `0x` and its bytes in hex.
    pub name: String,
    /// Colour spaces that name it.
    pub spaces: usize,
    /// Whether an alias can name it (a name that is not UTF-8 cannot be given).
    pub aliasable: bool,
}

fn is_spot(name: &[u8]) -> bool {
    name != b"None" && name != b"All" && !matches!(name, b"Cyan" | b"Magenta" | b"Yellow" | b"Black")
}

/// `o`, following references, with the object it finally lives in when it was a reference.
fn target(doc: &Document, o: &Object) -> (Option<ObjRef>, Arc<Object>) {
    let Object::Ref(first) = o else { return (None, Arc::new(o.clone())) };
    let (mut at, mut held) = (*first, doc.get(*first));
    for _ in 0..MAX_HOPS {
        let Object::Ref(next) = &*held else { break };
        at = *next;
        held = doc.get(*next);
    }
    (Some(at), held)
}

/// The spot colorants the document's Separation and DeviceN spaces name, with how many spaces
/// name each, in name order.
pub fn list_inks(doc: &Document) -> Vec<Ink> {
    let mut counts: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
    for num in doc.object_numbers() {
        let object = doc.get(ObjRef::new(num, doc.generation(num)));
        collect(doc, &object, 0, &mut counts);
    }
    counts
        .into_iter()
        .map(|(name, spaces)| match String::from_utf8(name) {
            Ok(name) => Ink { name, spaces, aliasable: true },
            Err(e) => {
                let hex: String = e.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
                Ink { name: format!("0x{hex}"), spaces, aliasable: false }
            }
        })
        .collect()
}

fn collect(doc: &Document, object: &Object, depth: usize, counts: &mut BTreeMap<Vec<u8>, usize>) {
    if depth > MAX_DEPTH {
        return;
    }
    match object {
        Object::Array(items) => {
            for name in colorants(doc, items).into_iter().flatten().filter(|n| is_spot(n)) {
                *counts.entry(name).or_default() += 1;
            }
            for item in items {
                collect(doc, item, depth + 1, counts);
            }
        }
        Object::Dict(dict) => dict.iter().for_each(|(_, v)| collect(doc, v, depth + 1, counts)),
        Object::Stream(stream) => stream.dict.iter().for_each(|(_, v)| collect(doc, v, depth + 1, counts)),
        _ => {}
    }
}

/// The colorant names a Separation or DeviceN array holds, or `None` when it is not one.
fn colorants(doc: &Document, items: &[Object]) -> Option<Vec<Vec<u8>>> {
    match items.first()?.as_name()? {
        b"Separation" => doc.resolve(items.get(1)?).as_name().map(|n| vec![n.to_vec()]),
        b"DeviceN" => {
            let list = doc.resolve(items.get(1)?);
            Some(list.as_array()?.iter().filter_map(|n| doc.resolve(n).as_name().map(<[u8]>::to_vec)).collect())
        }
        _ => None,
    }
}

/// Renames the colorants of every Separation and DeviceN space as `aliases` says. Does nothing,
/// and writes nothing, when no space names a colorant that is aliased.
///
/// # Errors
/// [`EditError::Invalid`] when the aliases fail [`InkAliases::check`], or when they would give one
/// DeviceN space the same colorant twice (`Gold` and `Gold2` both aliased to `Gold`) or one
/// dictionary of it two entries of one name. Nothing is written then.
pub fn alias_inks(doc: &mut Document, aliases: &InkAliases) -> Result<Aliased, EditError> {
    let (done, writes) = plan(doc, aliases)?;
    for (r, object) in writes {
        doc.set(r, object);
    }
    Ok(done)
}

/// What [`alias_inks`] would do, without doing it.
///
/// # Errors
/// As [`alias_inks`].
pub fn preview_aliases(doc: &Document, aliases: &InkAliases) -> Result<Aliased, EditError> {
    plan(doc, aliases).map(|(done, _)| done)
}

fn plan(doc: &Document, aliases: &InkAliases) -> Result<(Aliased, Vec<(ObjRef, Object)>), EditError> {
    aliases.check().map_err(EditError::Invalid)?;
    if aliases.aliases.is_empty() {
        return Ok((Aliased::default(), Vec::new()));
    }
    let mut walk =
        Walk { doc, aliases, extras: BTreeMap::new(), spaces: 0, skipped: 0, recording: false, sigs: BTreeMap::new(), renamed_to: BTreeSet::new() };
    let nums = doc.object_numbers();
    // First every space is read, which fills in the objects its parts live in (a names array or
    // an attributes dictionary of its own), so that an object is rewritten once whatever order
    // the file keeps them in.
    for &num in &nums {
        walk.rename(&doc.get(ObjRef::new(num, doc.generation(num))), 0).map_err(EditError::Invalid)?;
    }
    walk.spaces = 0;
    walk.skipped = 0;
    walk.recording = true;
    let mut writes = Vec::new();
    for num in nums {
        let r = ObjRef::new(num, doc.generation(num));
        let from_parts = walk.extras.get(&num).cloned();
        let object = doc.get(r);
        let base: &Object = from_parts.as_ref().unwrap_or(&object);
        let changed = walk.rename(base, 0).map_err(EditError::Invalid)?;
        if let Some(new) = changed.or(from_parts) {
            writes.push((r, new));
        }
    }
    if walk.spaces == 0 {
        return Ok((Aliased { skipped_nested: walk.skipped, ..Aliased::default() }, Vec::new()));
    }
    let different_alternates = walk
        .renamed_to
        .iter()
        .filter(|name| walk.sigs.get(*name).is_some_and(|s| s.len() > 1))
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .collect();
    let done = Aliased { spaces: walk.spaces, objects: writes.len(), different_alternates, skipped_nested: walk.skipped };
    Ok((done, writes))
}

struct Walk<'a> {
    doc: &'a Document,
    aliases: &'a InkAliases,
    /// Objects that hold a part of a colour space (names, attributes, colorants, hints), as they
    /// are to be written, by object number (the generation a file records for it may disagree
    /// with a reference to it).
    extras: BTreeMap<u32, Object>,
    spaces: usize,
    skipped: usize,
    /// Whether to note how each Separation is defined (the second pass only).
    recording: bool,
    /// For each Separation colorant name after renaming, how its spaces are defined.
    sigs: BTreeMap<Vec<u8>, BTreeSet<String>>,
    /// Names some ink was given.
    renamed_to: BTreeSet<Vec<u8>>,
}

type Res<T> = Result<T, String>;

/// How a part of a Separation (its alternate space or tint transform) is defined, for telling
/// two definitions apart: a reference stands for its object, a direct value for itself. Never the
/// bytes of a stream.
fn part_signature(part: Option<&Object>) -> String {
    match part {
        Some(Object::Ref(r)) => format!("{} {}", r.num, r.generation),
        Some(other) => format!("{other:?}"),
        None => String::new(),
    }
}

impl Walk<'_> {
    /// `object` with the colorants of the spaces inside it renamed, or `None` when none is. Only
    /// an object that changes is cloned.
    fn rename(&mut self, object: &Object, depth: usize) -> Res<Option<Object>> {
        if depth > MAX_DEPTH {
            if matches!(object, Object::Array(_) | Object::Dict(_) | Object::Stream(_)) {
                self.skipped += 1;
            }
            return Ok(None);
        }
        match object {
            Object::Array(items) => {
                let space = self.rename_space(items)?;
                // Spaces inside this array (an Indexed space's base, say).
                let current: &[Object] = space.as_deref().unwrap_or(items);
                let mut nested: Option<Vec<Object>> = None;
                for (i, item) in current.iter().enumerate() {
                    if let Some(new) = self.rename(item, depth + 1)?
                        && let Some(slot) = nested.get_or_insert_with(|| current.to_vec()).get_mut(i)
                    {
                        *slot = new;
                    }
                }
                Ok(nested.or(space).map(Object::Array))
            }
            Object::Dict(dict) => Ok(self.rename_dict(dict, depth)?.map(Object::Dict)),
            Object::Stream(stream) => Ok(self.rename_dict(&stream.dict, depth)?.map(|dict| {
                let mut s = stream.clone();
                s.dict = dict;
                Object::Stream(s)
            })),
            _ => Ok(None),
        }
    }

    fn rename_dict(&mut self, dict: &Dict, depth: usize) -> Res<Option<Dict>> {
        let mut changed: Option<Dict> = None;
        for (key, value) in dict.iter() {
            if let Some(new) = self.rename(value, depth + 1)? {
                changed.get_or_insert_with(|| dict.clone()).set(key.clone(), new);
            }
        }
        Ok(changed)
    }

    /// The array as a copy with its colorants renamed, when it is a Separation or DeviceN space
    /// that names an aliased ink. Parts stored as objects of their own go to `extras`.
    fn rename_space(&mut self, items: &[Object]) -> Res<Option<Vec<Object>>> {
        match items.first().and_then(Object::as_name) {
            Some(b"Separation") => {
                let Some(name_object) = items.get(1) else { return Ok(None) };
                let name = self.doc.resolve(name_object);
                let Some(name) = name.as_name() else { return Ok(None) };
                let to = self.aliases.lookup(name);
                if self.recording && is_spot(name) {
                    let kept = to.unwrap_or(name).to_vec();
                    let sig = format!("{}|{}", part_signature(items.get(2)), part_signature(items.get(3)));
                    self.sigs.entry(kept.clone()).or_default().insert(sig);
                    if to.is_some() {
                        self.renamed_to.insert(kept);
                    }
                }
                let Some(to) = to else { return Ok(None) };
                self.spaces += 1;
                let mut copy = items.to_vec();
                if let Some(slot) = copy.get_mut(1) {
                    *slot = Object::Name(to.to_vec());
                }
                Ok(Some(copy))
            }
            Some(b"DeviceN") => self.rename_device_n(items),
            _ => Ok(None),
        }
    }

    fn rename_device_n(&mut self, items: &[Object]) -> Res<Option<Vec<Object>>> {
        let Some(names_object) = items.get(1) else { return Ok(None) };
        let (names_at, names) = target(self.doc, names_object);
        let Some(list) = names.as_array() else { return Ok(None) };
        let mut new_names = Vec::with_capacity(list.len());
        let mut any = false;
        let mut seen: BTreeSet<Vec<u8>> = BTreeSet::new();
        let mut given: BTreeSet<Vec<u8>> = BTreeSet::new();
        for entry in list {
            let resolved = self.doc.resolve(entry);
            let (kept, renamed) = match resolved.as_name().and_then(|name| self.aliases.lookup(name)) {
                Some(to) => {
                    any = true;
                    given.insert(to.to_vec());
                    (Object::Name(to.to_vec()), true)
                }
                None => (entry.clone(), false),
            };
            // A repeat the file already had (a malformed space) is not ours to refuse: only one
            // the aliases make.
            if let Some(name) = self.doc.resolve(&kept).as_name()
                && name != b"None"
                && !seen.insert(name.to_vec())
                && (renamed || given.contains(name))
            {
                return Err(format!(
                    "aliasing would give one DeviceN colour space the colorant {:?} twice; a DeviceN space names each colorant once, so those inks cannot be merged",
                    String::from_utf8_lossy(name)
                ));
            }
            new_names.push(kept);
        }
        let mut copy = items.to_vec();
        if any {
            self.store(names_at, &mut copy, 1, Object::Array(new_names));
        }
        // The attributes name colorants too, and are kept in step even when the names do not
        // have the ink (a colorants entry for an ink the space does not list).
        let mut attributes_changed = false;
        if let Some(attributes) = items.get(4) {
            let (at, resolved) = target(self.doc, attributes);
            if let Object::Dict(dict) = &*resolved
                && let Some(new) = self.rename_attributes(dict)?
            {
                self.store(at, &mut copy, 4, Object::Dict(new));
                attributes_changed = true;
            }
        }
        if !any && !attributes_changed {
            return Ok(None);
        }
        self.spaces += 1;
        // A space whose parts all live in objects of their own is itself unchanged.
        Ok((copy != items).then_some(copy))
    }

    /// Puts `new` where a part lives: its own object, or slot `i` of the array being rewritten.
    fn store(&mut self, at: Option<ObjRef>, parent: &mut [Object], i: usize, new: Object) {
        match at {
            Some(r) => {
                self.extras.insert(r.num, new);
            }
            None => {
                if let Some(slot) = parent.get_mut(i) {
                    *slot = new;
                }
            }
        }
    }

    fn store_in_dict(&mut self, at: Option<ObjRef>, parent: &mut Dict, key: &[u8], new: Object) {
        match at {
            Some(r) => {
                self.extras.insert(r.num, new);
            }
            None => parent.set(key.to_vec(), new),
        }
    }

    /// A DeviceN space's attributes. The colorants dictionary is keyed by colorant name, and the
    /// mixing hints name colorants in their solidities, dot gains and printing order. The process
    /// dictionary's components are DeviceCMYK's own and are never renamed.
    fn rename_attributes(&mut self, attributes: &Dict) -> Res<Option<Dict>> {
        let mut out = attributes.clone();
        let mut changed = false;
        if let Some(value) = attributes.get(b"Colorants") {
            let (at, resolved) = target(self.doc, value);
            if let Object::Dict(dict) = &*resolved
                && let Some(new) = self.rekey(dict)?
            {
                self.store_in_dict(at, &mut out, b"Colorants", Object::Dict(new));
                changed = true;
            }
        }
        if let Some(value) = attributes.get(b"MixingHints") {
            let (at, resolved) = target(self.doc, value);
            if let Object::Dict(hints) = &*resolved {
                let mut new_hints = hints.clone();
                let mut hints_changed = false;
                for key in [&b"Solidities"[..], b"DotGain"] {
                    if let Some(part) = hints.get(key) {
                        let (part_at, part) = target(self.doc, part);
                        if let Object::Dict(dict) = &*part
                            && let Some(new) = self.rekey(dict)?
                        {
                            self.store_in_dict(part_at, &mut new_hints, key, Object::Dict(new));
                            hints_changed = true;
                        }
                    }
                }
                if let Some(part) = hints.get(b"PrintingOrder") {
                    let (part_at, part) = target(self.doc, part);
                    if let Object::Array(list) = &*part {
                        let mut renamed = false;
                        let new: Vec<Object> = list
                            .iter()
                            .map(|entry| match self.doc.resolve(entry).as_name().and_then(|n| self.aliases.lookup(n)) {
                                Some(to) => {
                                    renamed = true;
                                    Object::Name(to.to_vec())
                                }
                                None => entry.clone(),
                            })
                            .collect();
                        if renamed {
                            let mut names = BTreeSet::new();
                            for entry in &new {
                                if let Some(name) = self.doc.resolve(entry).as_name()
                                    && !names.insert(name.to_vec())
                                {
                                    return Err(format!("aliasing would name {:?} twice in a DeviceN printing order", String::from_utf8_lossy(name)));
                                }
                            }
                            self.store_in_dict(part_at, &mut new_hints, b"PrintingOrder", Object::Array(new));
                            hints_changed = true;
                        }
                    }
                }
                if hints_changed {
                    self.store_in_dict(at, &mut out, b"MixingHints", Object::Dict(new_hints));
                    changed = true;
                }
            }
        }
        Ok(changed.then_some(out))
    }

    /// A dictionary keyed by colorant name with its keys renamed, built in one pass from the
    /// original so that a swap or a chain loses nothing; a key that comes out twice is refused.
    fn rekey(&self, dict: &Dict) -> Res<Option<Dict>> {
        let mut out = Dict::new();
        let mut changed = false;
        for (key, value) in dict.iter() {
            let kept = match self.aliases.lookup(key) {
                Some(to) => {
                    changed = true;
                    to.to_vec()
                }
                None => key.clone(),
            };
            if out.get(&kept).is_some() {
                return Err(format!("aliasing would give a DeviceN dictionary two entries named {:?}", String::from_utf8_lossy(&kept)));
            }
            out.set(kept, value.clone());
        }
        Ok(changed.then_some(out))
    }
}

#[cfg(test)]
mod tests {
    use pdfcraft_cos::Stream;

    use super::*;

    fn sep_with(name: &str, alternate: &str) -> Object {
        Object::Array(vec![Object::name("Separation"), Object::name(name), Object::name(alternate), Object::Dict(Dict::new())])
    }

    fn sep(name: &str) -> Object {
        sep_with(name, "DeviceCMYK")
    }

    fn names_array(names: &[&str]) -> Object {
        Object::Array(names.iter().map(|n| Object::name(n)).collect())
    }

    fn colorants_dict(colorants: &[&str]) -> Dict {
        let mut cs = Dict::new();
        for c in colorants {
            cs.set(c.as_bytes().to_vec(), sep(c));
        }
        cs
    }

    fn device_n(names: &[&str], colorants: &[&str]) -> Object {
        let mut attrs = Dict::new();
        attrs.set(b"Colorants".to_vec(), Object::Dict(colorants_dict(colorants)));
        Object::Array(vec![Object::name("DeviceN"), names_array(names), Object::name("DeviceCMYK"), Object::Dict(Dict::new()), Object::Dict(attrs)])
    }

    /// A page-less document holding: an indirect Separation, one direct in a resource dictionary,
    /// a DeviceN with its colorants dictionary, an Indexed space over a Separation, and a stream
    /// whose dictionary names a Separation.
    fn fixture() -> (Document, Vec<ObjRef>) {
        let mut doc = Document::new_empty();
        let indirect = doc.add(sep("PANTONE 185 C"));
        let mut spaces = Dict::new();
        spaces.set(b"Direct".to_vec(), sep("PANTONE 185 C"));
        spaces.set(b"Other".to_vec(), sep("Varnish"));
        spaces.set(b"Process".to_vec(), device_n(&["Cyan", "PANTONE 185 C", "None"], &["PANTONE 185 C"]));
        spaces.set(
            b"Idx".to_vec(),
            Object::Array(vec![
                Object::name("Indexed"),
                sep("PANTONE 185 C"),
                Object::Int(1),
                Object::String(pdfcraft_cos::PdfString::literal(vec![0, 255])),
            ]),
        );
        let mut res = Dict::new();
        res.set(b"ColorSpace".to_vec(), Object::Dict(spaces));
        let resources = doc.add(Object::Dict(res));
        let mut image = Dict::new();
        image.set(b"ColorSpace".to_vec(), sep("PANTONE 185 C"));
        let stream = doc.add(Object::Stream(Stream::from_raw(image, vec![1, 2, 3])));
        (doc, vec![indirect, resources, stream])
    }

    fn aliases(pairs: &[(&str, &str)]) -> InkAliases {
        InkAliases { aliases: pairs.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect() }
    }

    /// Every object of the document, as text: what "nothing was written" is checked against.
    fn snapshot(doc: &Document) -> Vec<String> {
        doc.object_numbers().into_iter().map(|n| format!("{n}: {:?}", doc.get(ObjRef::new(n, doc.generation(n))))).collect()
    }

    fn name_at(doc: &Document, r: ObjRef, i: usize) -> Option<Vec<u8>> {
        doc.get(r).as_array().and_then(|x| x.get(i)).and_then(Object::as_name).map(<[u8]>::to_vec)
    }

    fn names_of(object: &Object) -> Vec<Vec<u8>> {
        object.as_array().unwrap().iter().filter_map(Object::as_name).map(<[u8]>::to_vec).collect()
    }

    fn keys_of(dict: &Dict) -> Vec<Vec<u8>> {
        let mut keys: Vec<_> = dict.iter().map(|(k, _)| k.clone()).collect();
        keys.sort();
        keys
    }

    #[test]
    fn every_space_naming_the_ink_is_renamed_and_nothing_else() {
        let (mut doc, objs) = fixture();
        let before = list_inks(&doc);
        let pairs = |inks: &[Ink]| inks.iter().map(|i| (i.name.clone(), i.spaces)).collect::<Vec<_>>();
        assert_eq!(pairs(&before), vec![("PANTONE 185 C".to_owned(), 6), ("Varnish".to_owned(), 1)], "{before:?}");
        let preview = preview_aliases(&doc, &aliases(&[("PANTONE 185 C", "PANTONE 186 C")])).unwrap();
        let touched = snapshot(&doc);
        let done = alias_inks(&mut doc, &aliases(&[("PANTONE 185 C", "PANTONE 186 C")])).unwrap();
        assert_eq!(done, preview, "the preview says what the edit does");
        // The indirect space, the direct one, the DeviceN and its colorants entry, the Indexed
        // base and the image's space: 6 spaces in 3 objects.
        assert_eq!((done.spaces, done.objects), (6, 3), "{done:?}");
        assert_ne!(snapshot(&doc), touched);
        let after = list_inks(&doc);
        assert_eq!(pairs(&after), vec![("PANTONE 186 C".to_owned(), 6), ("Varnish".to_owned(), 1)], "{after:?}");
        // The image's data is untouched, only its dictionary changed.
        match &*doc.get(objs[2]) {
            Object::Stream(s) => {
                assert_eq!(s.raw.as_ref(), &[1, 2, 3]);
                assert_eq!(
                    s.dict.get(b"ColorSpace").and_then(Object::as_array).and_then(|a| a.get(1)).and_then(Object::as_name),
                    Some(&b"PANTONE 186 C"[..])
                );
            }
            other => panic!("{other:?}"),
        }
        // The DeviceN's process colorants and its None stay; its colorants dictionary is keyed by the new name.
        let objects = doc.get(objs[1]);
        let n = objects
            .as_dict()
            .and_then(|r| r.get(b"ColorSpace"))
            .and_then(Object::as_dict)
            .and_then(|c| c.get(b"Process"))
            .and_then(Object::as_array)
            .unwrap();
        let names: Vec<_> = n.get(1).and_then(Object::as_array).unwrap().iter().filter_map(Object::as_name).collect();
        assert_eq!(names, vec![&b"Cyan"[..], b"PANTONE 186 C", b"None"]);
        let attrs = n.get(4).and_then(Object::as_dict).and_then(|d| d.get(b"Colorants")).and_then(Object::as_dict).unwrap();
        assert!(attrs.get(b"PANTONE 186 C").is_some() && attrs.get(b"PANTONE 185 C").is_none());
        assert!(done.different_alternates.is_empty(), "{done:?}");
    }

    #[test]
    fn an_alias_nothing_uses_writes_nothing_and_aliases_are_not_chained() {
        let (mut doc, _) = fixture();
        let touched = snapshot(&doc);
        let done = alias_inks(&mut doc, &aliases(&[("Gold", "Silver")])).unwrap();
        assert_eq!(done, Aliased::default());
        assert_eq!(snapshot(&doc), touched, "no object was written");
        // a to b and b to c: each name is looked up once.
        let mut doc = Document::new_empty();
        let a = doc.add(sep("A"));
        let b = doc.add(sep("B"));
        alias_inks(&mut doc, &aliases(&[("A", "B"), ("B", "C")])).unwrap();
        assert_eq!((name_at(&doc, a, 1), name_at(&doc, b, 1)), (Some(b"B".to_vec()), Some(b"C".to_vec())));
    }

    #[test]
    fn process_and_special_colorants_are_not_inks_and_cannot_be_renamed() {
        let mut doc = Document::new_empty();
        let all = doc.add(sep("All"));
        let black = doc.add(sep("Black"));
        assert!(list_inks(&doc).is_empty());
        let done = alias_inks(&mut doc, &aliases(&[("All", "X"), ("Black", "Y")])).unwrap();
        assert_eq!(done, Aliased::default());
        assert_eq!(name_at(&doc, all, 1), Some(b"All".to_vec()));
        assert_eq!(name_at(&doc, black, 1), Some(b"Black".to_vec()));
    }

    #[test]
    fn bad_aliases_are_refused_before_anything_is_written() {
        let mut doc = Document::new_empty();
        doc.add(sep("A"));
        let touched = snapshot(&doc);
        for bad in [
            aliases(&[("", "B")]),
            aliases(&[("A", "")]),
            aliases(&[("A", "B"), ("A", "C")]),
            aliases(&[("A", &"x".repeat(128))]),
            aliases(&[("A", "B\0")]),
            InkAliases { aliases: (0..65).map(|i| (format!("A{i}"), "B".to_owned())).collect() },
        ] {
            assert!(matches!(alias_inks(&mut doc, &bad), Err(EditError::Invalid(_))), "{bad:?}");
        }
        assert_eq!(snapshot(&doc), touched);
        assert!(aliases(&[("A", "A")]).check().is_ok(), "an alias to itself is allowed and does nothing");
    }

    /// None paints nothing, All paints on every plate and the process colorants are DeviceCMYK's:
    /// giving an ink one of those names would change the page, so it is refused.
    #[test]
    fn an_ink_cannot_be_given_the_name_of_a_process_colorant_all_or_none() {
        let mut doc = Document::new_empty();
        let gold = doc.add(sep("Gold"));
        let touched = snapshot(&doc);
        for to in ["None", "All", "Cyan", "Magenta", "Yellow", "Black"] {
            let Err(EditError::Invalid(why)) = alias_inks(&mut doc, &aliases(&[("Gold", to)])) else { panic!("{to} was accepted") };
            assert!(why.contains(to), "{why}");
        }
        assert_eq!(snapshot(&doc), touched);
        assert_eq!(name_at(&doc, gold, 1), Some(b"Gold".to_vec()));
    }

    /// Two inks in one DeviceN space cannot become one: a DeviceN space names each colorant once.
    /// Nothing is written, not even the spaces that could be renamed.
    #[test]
    fn merging_two_inks_of_one_devicen_space_is_refused_and_writes_nothing() {
        let mut doc = Document::new_empty();
        let single = doc.add(sep("Gold2"));
        let both = doc.add(device_n(&["Gold", "Gold2"], &["Gold", "Gold2"]));
        let touched = snapshot(&doc);
        let Err(EditError::Invalid(why)) = alias_inks(&mut doc, &aliases(&[("Gold2", "Gold")])) else { panic!("merged") };
        assert!(why.contains("Gold") && why.contains("twice"), "{why}");
        assert_eq!(snapshot(&doc), touched, "nothing was written");
        assert_eq!(name_at(&doc, single, 1), Some(b"Gold2".to_vec()));
        assert_eq!(names_of(&doc.get(both).as_array().unwrap()[1]), vec![b"Gold".to_vec(), b"Gold2".to_vec()]);
        // The same two inks in different spaces merge without trouble, and None may repeat.
        let mut doc = Document::new_empty();
        let a = doc.add(device_n(&["Gold", "None", "None"], &["Gold"]));
        let b = doc.add(device_n(&["Gold2"], &["Gold2"]));
        assert_eq!(alias_inks(&mut doc, &aliases(&[("Gold2", "Gold")])).unwrap().spaces, 2);
        assert_eq!(names_of(&doc.get(b).as_array().unwrap()[1]), vec![b"Gold".to_vec()]);
        assert_eq!(names_of(&doc.get(a).as_array().unwrap()[1]).len(), 3);
    }

    /// A swap and a chain keep every colorant entry: the dictionary is rebuilt in one pass from the
    /// original, not renamed key by key.
    #[test]
    fn a_swap_keeps_every_entry_of_the_colorants_dictionary() {
        let mut doc = Document::new_empty();
        let r = doc.add(device_n(&["A", "B", "C"], &["A", "B", "C"]));
        let done = alias_inks(&mut doc, &aliases(&[("A", "B"), ("B", "A"), ("C", "D")]));
        // A and B both appear in the space, so the names array holds B, A, D: a swap is fine.
        let done = done.unwrap();
        assert!(done.spaces >= 1, "{done:?}");
        let object = doc.get(r);
        let items = object.as_array().unwrap();
        assert_eq!(names_of(&items[1]), vec![b"B".to_vec(), b"A".to_vec(), b"D".to_vec()]);
        let colorants = items[4].as_dict().and_then(|d| d.get(b"Colorants")).and_then(Object::as_dict).unwrap();
        assert_eq!(keys_of(colorants), vec![b"A".to_vec(), b"B".to_vec(), b"D".to_vec()], "no entry was lost");
        // Each entry is the Separation space that carries its new name.
        for key in [b"A", b"B", b"D"] {
            let space = colorants.get(key).and_then(Object::as_array).unwrap();
            assert_eq!(space.get(1).and_then(Object::as_name), Some(&key[..]));
        }
    }

    /// A colorants dictionary holding both inks of a merge would get two entries of one name.
    #[test]
    fn a_colorants_dictionary_that_would_get_two_entries_of_one_name_is_refused() {
        let mut doc = Document::new_empty();
        // The names list holds only Gold, but the dictionary also keeps Gold2.
        doc.add(device_n(&["Gold"], &["Gold", "Gold2"]));
        let touched = snapshot(&doc);
        // Aliasing the name that is in the list to the one only the dictionary has clashes.
        let Err(EditError::Invalid(why)) = alias_inks(&mut doc, &aliases(&[("Gold", "Gold2")])) else { panic!("accepted") };
        assert!(why.contains("two entries"), "{why}");
        assert_eq!(snapshot(&doc), touched);
    }

    /// The names array, the attributes, the colorants dictionary and the mixing hints may each be
    /// an object of their own, and the file may keep them in any order.
    #[test]
    fn parts_stored_as_objects_of_their_own_are_rewritten_too() {
        let mut doc = Document::new_empty();
        let mut hints = Dict::new();
        let mut solidities = Dict::new();
        solidities.set(b"Gold".to_vec(), Object::Real(1.0));
        let solidities = doc.add(Object::Dict(solidities));
        hints.set(b"Solidities".to_vec(), Object::Ref(solidities));
        let mut dot_gain = Dict::new();
        dot_gain.set(b"Gold".to_vec(), Object::Real(0.2));
        hints.set(b"DotGain".to_vec(), Object::Dict(dot_gain));
        hints.set(b"PrintingOrder".to_vec(), names_array(&["Gold", "Cyan"]));
        let hints = doc.add(Object::Dict(hints));
        let colorants = doc.add(Object::Dict(colorants_dict(&["Gold"])));
        let mut attrs = Dict::new();
        attrs.set(b"Colorants".to_vec(), Object::Ref(colorants));
        attrs.set(b"MixingHints".to_vec(), Object::Ref(hints));
        let attrs = doc.add(Object::Dict(attrs));
        let names = doc.add(names_array(&["Gold", "Cyan"]));
        let space = doc.add(Object::Array(vec![
            Object::name("DeviceN"),
            Object::Ref(names),
            Object::name("DeviceCMYK"),
            Object::Dict(Dict::new()),
            Object::Ref(attrs),
        ]));
        assert_eq!(list_inks(&doc).iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), vec!["Gold"], "an indirect names array is read");
        let done = alias_inks(&mut doc, &aliases(&[("Gold", "Foil")])).unwrap();
        assert_eq!(done.spaces, 2, "the DeviceN and the Separation in its colorants: {done:?}");
        assert_eq!(names_of(&doc.get(names)), vec![b"Foil".to_vec(), b"Cyan".to_vec()]);
        assert_eq!(list_inks(&doc).iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), vec!["Foil"]);
        let renamed = doc.get(colorants);
        let renamed = renamed.as_dict().unwrap();
        assert_eq!(keys_of(renamed), vec![b"Foil".to_vec()]);
        assert_eq!(renamed.get(b"Foil").and_then(Object::as_array).and_then(|a| a.get(1)).and_then(Object::as_name), Some(&b"Foil"[..]));
        assert_eq!(keys_of(doc.get(solidities).as_dict().unwrap()), vec![b"Foil".to_vec()]);
        let hints = doc.get(hints);
        let hints = hints.as_dict().unwrap();
        assert_eq!(keys_of(hints.get(b"DotGain").and_then(Object::as_dict).unwrap()), vec![b"Foil".to_vec()]);
        assert_eq!(names_of(hints.get(b"PrintingOrder").unwrap()), vec![b"Foil".to_vec(), b"Cyan".to_vec()]);
        // The space itself still points at its parts.
        assert!(matches!(doc.get(space).as_array().map(|a| a[1].clone()), Some(Object::Ref(r)) if r == names));
    }

    /// One names array shared by two spaces is rewritten once, and the spaces both count.
    #[test]
    fn a_names_array_shared_by_two_spaces_is_rewritten_once() {
        let mut doc = Document::new_empty();
        let names = doc.add(names_array(&["Gold"]));
        let space =
            |names: ObjRef| Object::Array(vec![Object::name("DeviceN"), Object::Ref(names), Object::name("DeviceCMYK"), Object::Dict(Dict::new())]);
        doc.add(space(names));
        doc.add(space(names));
        let done = alias_inks(&mut doc, &aliases(&[("Gold", "Foil")])).unwrap();
        assert_eq!((done.spaces, done.objects), (2, 1), "only the shared names array is written: {done:?}");
        assert_eq!(names_of(&doc.get(names)), vec![b"Foil".to_vec()]);
    }

    /// Two inks given one name keep their own alternate spaces; the result says so.
    #[test]
    fn inks_that_end_up_with_different_alternates_are_reported() {
        let mut doc = Document::new_empty();
        doc.add(sep_with("Gold", "DeviceCMYK"));
        doc.add(sep_with("Gold2", "DeviceRGB"));
        let done = alias_inks(&mut doc, &aliases(&[("Gold2", "Gold")])).unwrap();
        assert_eq!(done.different_alternates, vec!["Gold".to_owned()], "{done:?}");
        // Same definitions: nothing to report.
        let mut doc = Document::new_empty();
        doc.add(sep("Gold"));
        doc.add(sep("Gold2"));
        let done = alias_inks(&mut doc, &aliases(&[("Gold2", "Gold")])).unwrap();
        assert!(done.different_alternates.is_empty(), "{done:?}");
    }

    #[test]
    fn a_name_that_is_not_utf8_is_listed_by_its_bytes_and_cannot_be_aliased() {
        let mut doc = Document::new_empty();
        doc.add(Object::Array(vec![
            Object::name("Separation"),
            Object::Name(vec![b'G', 0xE9, b'l']),
            Object::name("DeviceCMYK"),
            Object::Dict(Dict::new()),
        ]));
        let inks = list_inks(&doc);
        assert_eq!(inks, vec![Ink { name: "0x47e96c".to_owned(), spaces: 1, aliasable: false }]);
        // No alias a caller can write matches those bytes.
        assert_eq!(alias_inks(&mut doc, &aliases(&[("G\u{fffd}l", "Foil")])).unwrap(), Aliased::default());
    }

    /// A DeviceN that already repeats a colorant (malformed, but real) does not stop other
    /// inks being aliased; only a repeat the aliases make is refused.
    #[test]
    fn a_repeat_the_file_already_had_does_not_block_aliasing() {
        let mut doc = Document::new_empty();
        let broken = doc.add(device_n(&["Spot1", "Spot1"], &[]));
        let other = doc.add(sep("Gold"));
        let done = alias_inks(&mut doc, &aliases(&[("Gold", "Foil")])).unwrap();
        assert_eq!(done.spaces, 1, "{done:?}");
        assert_eq!(name_at(&doc, other, 1), Some(b"Foil".to_vec()));
        assert_eq!(names_of(&doc.get(broken).as_array().unwrap()[1]), vec![b"Spot1".to_vec(), b"Spot1".to_vec()]);
        // A repeat beside an aliased ink in the same space stays as it was too.
        let mut doc = Document::new_empty();
        let space = doc.add(device_n(&["Spot1", "Spot1", "Gold"], &[]));
        alias_inks(&mut doc, &aliases(&[("Gold", "Foil")])).unwrap();
        assert_eq!(names_of(&doc.get(space).as_array().unwrap()[1]), vec![b"Spot1".to_vec(), b"Spot1".to_vec(), b"Foil".to_vec()]);
    }

    /// Objects with a generation other than 0 are rewritten like any other, and a reference whose
    /// generation the file does not have for the object is dangling: not read, so not half done.
    #[test]
    fn objects_with_a_generation_are_rewritten_and_dangling_references_are_left_alone() {
        let mut doc = Document::new_empty();
        let names = ObjRef::new(40, 3);
        doc.set(names, names_array(&["Gold"]));
        let space = |reference: ObjRef| {
            Object::Array(vec![Object::name("DeviceN"), Object::Ref(reference), Object::name("DeviceCMYK"), Object::Dict(Dict::new())])
        };
        doc.add(space(names));
        let done = alias_inks(&mut doc, &aliases(&[("Gold", "Foil")])).unwrap();
        assert_eq!((done.spaces, done.objects), (1, 1), "{done:?}");
        assert_eq!(names_of(&doc.get(names)), vec![b"Foil".to_vec()]);
        assert_eq!(doc.generation(40), 3, "the object keeps its generation");
        let mut doc = Document::new_empty();
        let names = doc.add(names_array(&["Gold"]));
        let touched = snapshot(&doc);
        doc.add(space(ObjRef::new(names.num, names.generation.wrapping_add(5))));
        let before = snapshot(&doc);
        assert_ne!(before, touched);
        assert_eq!(alias_inks(&mut doc, &aliases(&[("Gold", "Foil")])).unwrap(), Aliased::default());
        assert_eq!(snapshot(&doc), before);
    }

    /// An ink only the colorants dictionary has is renamed there, key and space together.
    #[test]
    fn a_colorants_entry_for_an_ink_the_names_lack_stays_consistent() {
        let mut doc = Document::new_empty();
        let space = doc.add(device_n(&["Gold"], &["Gold", "Gold2"]));
        alias_inks(&mut doc, &aliases(&[("Gold2", "Silver")])).unwrap();
        let object = doc.get(space);
        let items = object.as_array().unwrap();
        assert_eq!(names_of(&items[1]), vec![b"Gold".to_vec()]);
        let colorants = items[4].as_dict().and_then(|d| d.get(b"Colorants")).and_then(Object::as_dict).unwrap();
        assert_eq!(keys_of(colorants), vec![b"Gold".to_vec(), b"Silver".to_vec()]);
        let silver = colorants.get(b"Silver").and_then(Object::as_array).unwrap();
        assert_eq!(silver.get(1).and_then(Object::as_name), Some(&b"Silver"[..]));
    }

    /// A printing order that would name an ink twice is refused.
    #[test]
    fn a_printing_order_that_would_repeat_an_ink_is_refused() {
        let mut doc = Document::new_empty();
        let mut hints = Dict::new();
        hints.set(b"PrintingOrder".to_vec(), names_array(&["Gold", "Foil"]));
        let mut attrs = Dict::new();
        attrs.set(b"MixingHints".to_vec(), Object::Dict(hints));
        doc.add(Object::Array(vec![
            Object::name("DeviceN"),
            names_array(&["Gold"]),
            Object::name("DeviceCMYK"),
            Object::Dict(Dict::new()),
            Object::Dict(attrs),
        ]));
        let touched = snapshot(&doc);
        let Err(EditError::Invalid(why)) = alias_inks(&mut doc, &aliases(&[("Gold", "Foil")])) else { panic!("accepted") };
        assert!(why.contains("printing order"), "{why}");
        assert_eq!(snapshot(&doc), touched);
    }

    /// Tint transforms that are different objects are different definitions, however big their
    /// streams are; the same object is one.
    #[test]
    fn definitions_are_told_apart_by_the_object_not_its_bytes() {
        let mut doc = Document::new_empty();
        let f1 = doc.add(Object::Stream(Stream::from_raw(Dict::new(), vec![1; 1000])));
        let with =
            |name: &str, f: ObjRef| Object::Array(vec![Object::name("Separation"), Object::name(name), Object::name("DeviceCMYK"), Object::Ref(f)]);
        doc.add(with("Gold", f1));
        doc.add(with("Gold2", f1));
        assert!(alias_inks(&mut doc, &aliases(&[("Gold2", "Gold")])).unwrap().different_alternates.is_empty());
        let mut doc = Document::new_empty();
        let f1 = doc.add(Object::Stream(Stream::from_raw(Dict::new(), vec![1; 1000])));
        let f2 = doc.add(Object::Stream(Stream::from_raw(Dict::new(), vec![2; 1000])));
        doc.add(with("Gold", f1));
        doc.add(with("Gold2", f2));
        assert_eq!(alias_inks(&mut doc, &aliases(&[("Gold2", "Gold")])).unwrap().different_alternates, vec!["Gold".to_owned()]);
    }

    #[test]
    fn colour_spaces_nested_past_the_limit_are_left_alone_and_counted() {
        let mut doc = Document::new_empty();
        let mut space = sep("Deep");
        for _ in 0..40 {
            space = Object::Array(vec![Object::name("Indexed"), space, Object::Int(0), Object::String(pdfcraft_cos::PdfString::literal(vec![0]))]);
        }
        doc.add(space);
        let done = alias_inks(&mut doc, &aliases(&[("Deep", "Shallow")])).unwrap();
        assert_eq!((done.spaces, done.objects), (0, 0), "past the depth limit nothing is read, and nothing overflows");
        assert!(done.skipped_nested > 0, "{done:?}");
        assert!(list_inks(&doc).is_empty());
    }
}
