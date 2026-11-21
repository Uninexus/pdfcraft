//! The portfolio collection: the catalog's `/Collection` dictionary, which makes a PDF open as a
//! portfolio of its embedded files (ISO 32000-2 §7.11.6 and §12.3.5). It has `/View` (the name
//! `D`, `T`, `H` or `C`: details, tile, hidden or custom), `/D` (the name-tree key of the file shown first),
//! and `/Schema`, a dictionary whose entries each describe one column of the file list: `/N` (the
//! column's name), `/Subtype` (the name `S`, `D`, `N`, `F`, `Desc`, `ModDate`, `CreationDate`,
//! `Size`, `CompressedSize`), `/O` (its order), `/V` (visible) and `/E` (editable).
//!
//! This module reads that and takes the `/Collection` entry out of the catalog, which makes the
//! file an ordinary PDF whose embedded files are the attachments the attachment tools handle. It
//! adds no collection, changes no column, and reads no `/Sort`, `/Folders` or `/CI` (a file
//! specification's collection item).
//!
//! A read has one allowance of [`MAX_GETS`] reads, against which every read is charged before it
//! is made (references are followed as `Document::resolve` follows them, the first object and up
//! to 32 more, each hop charged) and each `/Schema` entry looked at costs one more; when it is
//! spent the read stops and says so, and a column whose reads were not all paid for is not listed. The one read not charged is the catalog's. `/View`, `/D`,
//! `/Subtype`, `/O`, `/V` and `/E` are read as written, not through references.

use std::sync::Arc;

use pdfcraft_cos::{Document, Object, PdfString};

use crate::OrganizeError;
use crate::print_presets::Held;
use crate::role_map::shown;

/// Most columns listed (all are counted).
pub const MAX_LISTED_FIELDS: usize = 256;
/// Most characters of a column name or of `/D` listed; a longer one is cut and not `exact`.
pub const MAX_TEXT_CHARS: usize = 127;
/// Most reads one call of [`read_collection`] makes: its allowance, shared by everything it reads.
pub const MAX_GETS: usize = 100_000;
/// References followed after the first object, as `Document::resolve` follows them.
const MORE_HOPS: usize = 32;

/// One entry of `/Schema`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// The key of the entry (the field's name in a file specification's collection item), cut
    /// at [`MAX_TEXT_CHARS`] bytes.
    pub key: String,
    /// The entry is a dictionary, after references; when false the rest is empty or default.
    pub is_field: bool,
    /// `/N`, the column's name, a string after references, cut at [`MAX_TEXT_CHARS`] characters.
    pub name: Option<String>,
    /// `/Subtype` when it is a name written in the field, as written.
    pub subtype: Option<String>,
    /// `/O` when it is an integer written in the field.
    pub order: Option<i64>,
    /// `/V` when it is a boolean written in the field; a column is visible when it is absent.
    pub visible: bool,
    /// `/E` when it is a boolean written in the field; a column is not editable when it is absent.
    pub editable: bool,
    /// `key` and `name` are whole: false when either was cut, and when the key is not valid UTF-8
    /// (it is shown with replacement characters).
    pub exact: bool,
}

/// What [`read_collection`] read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Collection {
    /// The catalog has a `/Collection` entry that is not null: what a removal takes out.
    pub has_entry: bool,
    /// The entry is a dictionary after references: the file is a portfolio.
    pub present: bool,
    /// `/View` when it is a name written in the dictionary, as written.
    pub view: Option<String>,
    /// `/D` when it is a string written in the dictionary, cut at [`MAX_TEXT_CHARS`] characters.
    pub initial_document: Option<String>,
    /// `/D` was longer than [`MAX_TEXT_CHARS`] characters and `initial_document` is cut.
    pub initial_document_cut: bool,
    /// The first [`MAX_LISTED_FIELDS`] entries of `/Schema` other than `/Type`, in file order.
    pub fields: Vec<Field>,
    /// How many entries `/Schema` has other than `/Type`.
    pub count: usize,
    /// More entries exist than were listed.
    pub truncated: bool,
    /// The [`MAX_GETS`] allowance was spent, so what follows the stop was not read.
    pub capped: bool,
}

struct Spent;

/// The reads left.
struct Gets(usize);

impl Gets {
    fn charge(&mut self) -> Result<(), Spent> {
        self.0 = self.0.checked_sub(1).ok_or(Spent)?;
        Ok(())
    }

    /// `o` after references, each read charged before it is made.
    fn follow<'a>(&mut self, doc: &Document, o: &'a Object) -> Result<Held<'a>, Spent> {
        let Object::Ref(first) = o else { return Ok(Held::Direct(o)) };
        self.charge()?;
        let mut held: Arc<Object> = doc.get(*first);
        for _ in 0..MORE_HOPS {
            let Object::Ref(next) = &*held else { break };
            self.charge()?;
            held = doc.get(*next);
        }
        Ok(Held::Resolved(held))
    }
}

fn cut(s: &PdfString) -> (String, bool) {
    let mut chars = s.text_chars();
    let shown: String = chars.by_ref().take(MAX_TEXT_CHARS).collect();
    (shown, chars.next().is_none())
}

fn field(doc: &Document, gets: &mut Gets, key: &[u8], value: &Object) -> Result<Field, Spent> {
    let (key, key_whole) = shown(key);
    let mut out = Field { key, is_field: false, name: None, subtype: None, order: None, visible: true, editable: false, exact: key_whole };
    let held = gets.follow(doc, value)?;
    let Object::Dict(d) = &*held else { return Ok(out) };
    out.is_field = true;
    out.subtype = d.name(b"Subtype").map(|n| shown(n).0);
    out.order = d.int(b"O");
    if let Some(Object::Bool(v)) = d.get(b"V") {
        out.visible = *v;
    }
    if let Some(Object::Bool(e)) = d.get(b"E") {
        out.editable = *e;
    }
    if let Some(n) = d.get(b"N")
        && let Object::String(s) = &*gets.follow(doc, n)?
    {
        let (text, whole) = cut(s);
        out.name = Some(text);
        out.exact &= whole;
    }
    Ok(out)
}

fn read(doc: &Document, gets: &mut Gets, out: &mut Collection) -> Result<(), Spent> {
    let Some(catalog) = doc.root().map(|r| doc.get(r)) else { return Ok(()) };
    let Some(entry) = catalog.as_dict().and_then(|c| c.get(b"Collection")) else { return Ok(()) };
    out.has_entry = !matches!(entry, Object::Null);
    let held = gets.follow(doc, entry)?;
    let Object::Dict(collection) = &*held else { return Ok(()) };
    out.present = true;
    out.view = collection.name(b"View").map(|n| shown(n).0);
    if let Some(Object::String(s)) = collection.get(b"D") {
        let (text, whole) = cut(s);
        out.initial_document = Some(text);
        out.initial_document_cut = !whole;
    }
    let Some(schema) = collection.get(b"Schema") else { return Ok(()) };
    let schema = gets.follow(doc, schema)?;
    let Object::Dict(schema) = &*schema else { return Ok(()) };
    out.count = schema.iter().filter(|(k, _)| k.as_slice() != b"Type").count();
    out.truncated = out.count > MAX_LISTED_FIELDS;
    for (key, value) in schema.iter().filter(|(k, _)| k.as_slice() != b"Type").take(MAX_LISTED_FIELDS) {
        gets.charge()?;
        out.fields.push(field(doc, gets, key, value)?);
    }
    Ok(())
}

/// The collection as stored. One uncharged read of the catalog, then at most [`MAX_GETS`] reads.
pub fn read_collection(doc: &Document) -> Collection {
    read_with(doc, MAX_GETS)
}

pub(crate) fn read_with(doc: &Document, gets: usize) -> Collection {
    let mut out = Collection::default();
    if read(doc, &mut Gets(gets), &mut out).is_err() {
        out.capped = true;
    }
    out
}

/// Whether [`remove_collection`] would write: the catalog has a `/Collection` entry that is not
/// null. One read of the catalog.
pub fn collection_removes(doc: &Document) -> bool {
    let Some(catalog) = doc.root().map(|r| doc.get(r)) else { return false };
    catalog.as_dict().and_then(|c| c.get(b"Collection")).is_some_and(|o| !matches!(o, Object::Null))
}

/// Take `/Collection` out of the catalog. An indirect dictionary is left in the file unreferenced,
/// and the embedded files and the rest of the catalog are not touched. A catalog without the
/// entry is no change.
pub fn remove_collection(doc: &mut Document) -> Result<(), OrganizeError> {
    let root = doc.root().ok_or(OrganizeError::NoPageTree)?;
    if !collection_removes(doc) {
        return Ok(());
    }
    doc.update_dict(root, |c| {
        c.remove(b"Collection");
    })?;
    Ok(())
}
