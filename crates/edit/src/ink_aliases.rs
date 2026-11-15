//! Print production ▸ Ink Manager ▸ Ink aliases: spot colorants renamed so that two inks print
//! as one. The names in every Separation and DeviceN colour space of the document are
//! rewritten (colour spaces in page and form resources, in images, patterns and shadings, and
//! those kept as their own objects); nothing else changes. Each space keeps its own alternate
//! space and tint transform, so what the page looks like on screen stays as it is.
//!
//! [`list_inks`] reads the spot colorants a document uses. Colorants named `/All` and `/None`,
//! and DeviceCMYK's own (`Cyan`, `Magenta`, `Yellow`, `Black`), are not inks to alias.

use std::collections::BTreeMap;

use pdfcraft_cos::{Dict, Document, ObjRef, Object};

use crate::EditError;

/// Colour space arrays nested deeper than this are not read (an Indexed space holds a base space
/// and so on; real files stop at two or three).
const MAX_DEPTH: usize = 16;
/// The most aliases one call may give.
pub const MAX_ALIASES: usize = 64;
/// The longest colorant name PDF allows (ISO 32000-2 Annex C).
const MAX_NAME_BYTES: usize = 127;

/// Which colorant is renamed to which.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InkAliases {
    /// `(from, to)` pairs: every spot colorant named `from` is named `to`. A name is looked up
    /// once, so `a → b` with `b → c` renames `a` to `b` and `b` to `c`, not `a` to `c`.
    pub aliases: Vec<(String, String)>,
}

impl InkAliases {
    /// Whether the aliases can be applied: at most [`MAX_ALIASES`], names of 1 to 127 bytes with
    /// no NUL, no name aliased twice. `Err` says what is wrong.
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
            if !seen.insert(from.as_bytes().to_vec()) {
                return Err(format!("{from:?} is aliased twice"));
            }
        }
        Ok(())
    }

    fn lookup(&self, name: &[u8]) -> Option<&[u8]> {
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
}

fn is_spot(name: &[u8]) -> bool {
    name != b"None" && name != b"All" && !matches!(name, b"Cyan" | b"Magenta" | b"Yellow" | b"Black")
}

/// The spot colorants the document's Separation and DeviceN spaces name, with how many spaces
/// name each, in name order.
pub fn list_inks(doc: &Document) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
    for num in doc.object_numbers() {
        let object = doc.get(ObjRef::new(num, doc.generation(num)));
        collect(&object, 0, &mut counts);
    }
    counts.into_iter().map(|(name, n)| (String::from_utf8_lossy(&name).into_owned(), n)).collect()
}

fn collect(object: &Object, depth: usize, counts: &mut BTreeMap<Vec<u8>, usize>) {
    if depth > MAX_DEPTH {
        return;
    }
    match object {
        Object::Array(items) => {
            if let Some(names) = colorants(items) {
                for name in names.into_iter().filter(|n| is_spot(n)) {
                    *counts.entry(name.to_vec()).or_default() += 1;
                }
            }
            for item in items {
                collect(item, depth + 1, counts);
            }
        }
        Object::Dict(dict) => dict.iter().for_each(|(_, v)| collect(v, depth + 1, counts)),
        Object::Stream(stream) => stream.dict.iter().for_each(|(_, v)| collect(v, depth + 1, counts)),
        _ => {}
    }
}

/// The colorant names a Separation or DeviceN array holds, or `None` when it is not one.
fn colorants(items: &[Object]) -> Option<Vec<&[u8]>> {
    match items.first()?.as_name()? {
        b"Separation" => items.get(1)?.as_name().map(|n| vec![n]),
        b"DeviceN" => Some(items.get(1)?.as_array()?.iter().filter_map(Object::as_name).collect()),
        _ => None,
    }
}

/// Renames the colorants of every Separation and DeviceN space as `aliases` says. Does nothing,
/// and writes nothing, when no space names a colorant that is aliased.
///
/// # Errors
/// [`EditError::Invalid`] when the aliases fail [`InkAliases::check`].
pub fn alias_inks(doc: &mut Document, aliases: &InkAliases) -> Result<Aliased, EditError> {
    aliases.check().map_err(EditError::Invalid)?;
    let mut out = Aliased::default();
    if aliases.aliases.is_empty() {
        return Ok(out);
    }
    for num in doc.object_numbers() {
        let r = ObjRef::new(num, doc.generation(num));
        let object = doc.get(r);
        let mut spaces = 0;
        if let Some(changed) = rename(&object, aliases, 0, &mut spaces) {
            doc.set(r, changed);
            out.spaces += spaces;
            out.objects += 1;
        }
    }
    Ok(out)
}

/// `object` with the colorants of the spaces inside it renamed, or `None` when none is. Only an
/// object that changes is cloned.
fn rename(object: &Object, aliases: &InkAliases, depth: usize, spaces: &mut usize) -> Option<Object> {
    if depth > MAX_DEPTH {
        return None;
    }
    match object {
        Object::Array(items) => {
            let mut changed: Option<Vec<Object>> = None;
            if colorants(items).is_some_and(|names| names.iter().any(|n| is_spot(n) && aliases.lookup(n).is_some())) {
                let mut copy = items.clone();
                let separation = items.first().and_then(Object::as_name) == Some(b"Separation");
                if separation {
                    if let (Some(to), Some(slot)) = (items.get(1).and_then(Object::as_name).and_then(|n| aliases.lookup(n)), copy.get_mut(1)) {
                        *slot = Object::Name(to.to_vec());
                    }
                } else if let Some(Object::Array(list)) = copy.get_mut(1) {
                    for name in list.iter_mut() {
                        if let Some(to) = name.as_name().filter(|n| is_spot(n)).and_then(|n| aliases.lookup(n)) {
                            *name = Object::Name(to.to_vec());
                        }
                    }
                    // The attributes of a DeviceN space name the same colorants.
                    if let Some(Object::Dict(attributes)) = copy.get_mut(4) {
                        rename_attributes(attributes, aliases);
                    }
                }
                *spaces += 1;
                changed = Some(copy);
            }
            // Spaces inside this array (an Indexed space's base, say).
            let base = changed.as_deref().unwrap_or(items);
            let mut nested: Option<Vec<Object>> = None;
            for (i, item) in base.iter().enumerate() {
                if let Some(new) = rename(item, aliases, depth + 1, spaces)
                    && let Some(slot) = nested.get_or_insert_with(|| base.to_vec()).get_mut(i)
                {
                    *slot = new;
                }
            }
            nested.or(changed).map(Object::Array)
        }
        Object::Dict(dict) => rename_dict(dict, aliases, depth, spaces).map(Object::Dict),
        Object::Stream(stream) => rename_dict(&stream.dict, aliases, depth, spaces).map(|dict| {
            let mut s = stream.clone();
            s.dict = dict;
            Object::Stream(s)
        }),
        _ => None,
    }
}

fn rename_dict(dict: &Dict, aliases: &InkAliases, depth: usize, spaces: &mut usize) -> Option<Dict> {
    let mut changed: Option<Dict> = None;
    for (key, value) in dict.iter() {
        if let Some(new) = rename(value, aliases, depth + 1, spaces) {
            changed.get_or_insert_with(|| dict.clone()).set(key.clone(), new);
        }
    }
    changed
}

/// A DeviceN space's attributes: the colorants dictionary is keyed by name, and the process
/// components are names.
fn rename_attributes(attributes: &mut Dict, aliases: &InkAliases) {
    let keys: Vec<(Vec<u8>, Vec<u8>)> = match attributes.get(b"Colorants") {
        Some(Object::Dict(colorants)) => {
            colorants.iter().filter_map(|(k, _)| aliases.lookup(k).filter(|_| is_spot(k)).map(|to| (k.clone(), to.to_vec()))).collect()
        }
        _ => Vec::new(),
    };
    if let Some(Object::Dict(colorants)) = attributes.get_mut(b"Colorants") {
        for (from, to) in keys {
            if let Some(value) = colorants.remove(&from) {
                colorants.set(to, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use pdfcraft_cos::Stream;

    use super::*;

    fn sep(name: &str) -> Object {
        Object::Array(vec![Object::name("Separation"), Object::name(name), Object::name("DeviceCMYK"), Object::Dict(Dict::new())])
    }

    fn device_n(names: &[&str], colorants: &[&str]) -> Object {
        let mut attrs = Dict::new();
        let mut cs = Dict::new();
        for c in colorants {
            cs.set(c.as_bytes().to_vec(), sep(c));
        }
        attrs.set(b"Colorants".to_vec(), Object::Dict(cs));
        Object::Array(vec![
            Object::name("DeviceN"),
            Object::Array(names.iter().map(|n| Object::name(n)).collect()),
            Object::name("DeviceCMYK"),
            Object::Dict(Dict::new()),
            Object::Dict(attrs),
        ])
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

    #[test]
    fn every_space_naming_the_ink_is_renamed_and_nothing_else() {
        let (mut doc, objs) = fixture();
        let before = list_inks(&doc);
        assert_eq!(before, vec![("PANTONE 185 C".to_owned(), 6), ("Varnish".to_owned(), 1)], "{before:?}");
        let done = alias_inks(&mut doc, &aliases(&[("PANTONE 185 C", "PANTONE 186 C")])).unwrap();
        // The indirect space, the direct one, the DeviceN and its colorants entry, the Indexed
        // base and the image's space: 6 spaces in 3 objects.
        assert_eq!((done.spaces, done.objects), (6, 3), "{done:?}");
        let after = list_inks(&doc);
        assert_eq!(after, vec![("PANTONE 186 C".to_owned(), 6), ("Varnish".to_owned(), 1)], "{after:?}");
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
    }

    #[test]
    fn an_alias_nothing_uses_writes_nothing_and_aliases_are_not_chained() {
        let (mut doc, _) = fixture();
        let touched = doc.modified_objects();
        let done = alias_inks(&mut doc, &aliases(&[("Gold", "Silver")])).unwrap();
        assert_eq!(done, Aliased::default());
        assert_eq!(doc.modified_objects(), touched, "no object was written");
        // a to b and b to c: each name is looked up once.
        let mut doc = Document::new_empty();
        let a = doc.add(sep("A"));
        let b = doc.add(sep("B"));
        alias_inks(&mut doc, &aliases(&[("A", "B"), ("B", "C")])).unwrap();
        let name = |r: ObjRef| doc.get(r).as_array().and_then(|x| x.get(1)).and_then(Object::as_name).map(<[u8]>::to_vec);
        assert_eq!((name(a), name(b)), (Some(b"B".to_vec()), Some(b"C".to_vec())));
    }

    #[test]
    fn process_and_special_colorants_are_not_inks_and_cannot_be_renamed() {
        let mut doc = Document::new_empty();
        let all = doc.add(sep("All"));
        let black = doc.add(sep("Black"));
        assert!(list_inks(&doc).is_empty());
        let done = alias_inks(&mut doc, &aliases(&[("All", "X"), ("Black", "Y")])).unwrap();
        assert_eq!(done, Aliased::default());
        assert_eq!(doc.get(all).as_array().and_then(|a| a.get(1)).and_then(Object::as_name), Some(&b"All"[..]));
        assert_eq!(doc.get(black).as_array().and_then(|a| a.get(1)).and_then(Object::as_name), Some(&b"Black"[..]));
    }

    #[test]
    fn bad_aliases_are_refused_before_anything_is_written() {
        let mut doc = Document::new_empty();
        doc.add(sep("A"));
        let touched = doc.modified_objects();
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
        assert_eq!(doc.modified_objects(), touched);
        assert!(aliases(&[("A", "A")]).check().is_ok(), "an alias to itself is allowed and does nothing");
    }

    #[test]
    fn colour_spaces_nested_past_the_limit_are_left_alone() {
        let mut doc = Document::new_empty();
        let mut space = sep("Deep");
        for _ in 0..40 {
            space = Object::Array(vec![Object::name("Indexed"), space, Object::Int(0), Object::String(pdfcraft_cos::PdfString::literal(vec![0]))]);
        }
        let r = doc.add(space);
        let done = alias_inks(&mut doc, &aliases(&[("Deep", "Shallow")])).unwrap();
        assert_eq!(done, Aliased::default(), "past the depth limit nothing is read, and nothing overflows");
        assert!(list_inks(&doc).is_empty());
        let _ = r;
    }
}
