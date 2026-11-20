//! PDF 2.0 associated files (ISO 32000-2 §14.13): the `/AF` array of the catalog (the document)
//! or of a page, each entry a file specification that says how the file relates to what it is
//! associated with (`/AFRelationship`).
//!
//! Reading covers the document and every page. Writing is document level only: the file goes in
//! through the same path as an attachment (an embedded stream in a `/Type /Filespec`, so it also
//! shows in the Attachments panel) and its specification is appended to the catalog's `/AF`.

use std::collections::HashSet;

use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString};

use crate::OrganizeError;
use crate::attachments::{add_attachment_spec, delete_attachment_by_ref};

/// The relationships ISO 32000-2 Table 43 names, in the order it lists them.
pub const AF_RELATIONSHIPS: [&str; 8] = ["Source", "Data", "Alternative", "Supplement", "EncryptedPayload", "FormData", "Schema", "Unspecified"];

/// Most entries read from one `/AF` array, and most the catalog's array may hold after an add.
pub const MAX_AF_ENTRIES: usize = 256;
/// Largest file [`add_associated_file`] embeds.
pub const MAX_AF_BYTES: usize = 32 * 1024 * 1024;
/// Most rows [`list_associated_files`] returns in all (document and pages together).
pub const MAX_AF_ROWS: usize = 2000;
/// Most pages whose `/AF` is read, so a page tree of millions of pages cannot stall a listing.
const MAX_PAGES_READ: usize = 100_000;

/// One entry of an `/AF` array.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssociatedFile {
    /// `None` for the document (catalog); the 0-based page otherwise.
    pub page: Option<usize>,
    /// 1-based position in its `/AF` array.
    pub index: usize,
    /// The `/AFRelationship` name as written (`None` when missing: readers treat it as Unspecified).
    pub relationship: Option<String>,
    /// The file specification's `/UF`, else its `/F`.
    pub name: Option<String>,
    pub description: Option<String>,
    /// The embedded file's `/Subtype`.
    pub mime: Option<String>,
    /// The embedded file's `/Params /Size`, when it states one.
    pub size: Option<u64>,
    /// Whether the specification carries an embedded stream (`/EF`); false for a link to an outside file.
    pub embedded: bool,
}

/// The rows [`list_associated_files`] read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssociatedList {
    pub files: Vec<AssociatedFile>,
    /// More entries (or pages) exist than were read: [`MAX_AF_ROWS`] rows or [`MAX_PAGES_READ`] pages.
    pub truncated: bool,
    /// How many of `files` have no `/AFRelationship` that is a name. ISO 32000-2 requires one on a
    /// file specification that an `/AF` array lists; a reader treats the entry as Unspecified.
    pub without_relationship: usize,
    /// How many of `files` carry no embedded file stream (a link to a file outside the document,
    /// which a PDF/A-3 file may not rely on).
    pub not_embedded: usize,
}

/// The `/AF` entries of the document and of every page, document first, at most
/// [`MAX_AF_ROWS`] rows in all. An indirect `/AF` array shared by several holders is read once.
pub fn list_associated_files(doc: &Document) -> AssociatedList {
    let mut out = AssociatedList::default();
    let mut seen = HashSet::new();
    if let Some(root) = doc.root()
        && let Some(catalog) = doc.get(root).as_dict()
    {
        read_array(doc, catalog, None, &mut out, &mut seen);
    }
    if let Ok(pages) = crate::pages(doc) {
        if pages.len() > MAX_PAGES_READ {
            out.truncated = true;
        }
        for (n, page) in pages.iter().take(MAX_PAGES_READ).enumerate() {
            if out.truncated && out.files.len() >= MAX_AF_ROWS {
                break;
            }
            if let Some(dict) = doc.get(page.obj).as_dict() {
                read_array(doc, dict, Some(n), &mut out, &mut seen);
            }
        }
    }
    // Counted from the rows already read (at most [`MAX_AF_ROWS`]): nothing more is read or copied.
    out.without_relationship = out.files.iter().filter(|f| f.relationship.is_none()).count();
    out.not_embedded = out.files.iter().filter(|f| !f.embedded).count();
    out
}

/// [`list_associated_files`]'s rows alone.
pub fn associated_files(doc: &Document) -> Vec<AssociatedFile> {
    list_associated_files(doc).files
}

fn read_array(doc: &Document, holder: &Dict, page: Option<usize>, out: &mut AssociatedList, seen: &mut HashSet<ObjRef>) {
    let Some(af) = holder.get(b"AF") else { return };
    // An array shared through a reference is read for the first holder only.
    if let Some(r) = af.as_ref()
        && !seen.insert(r)
    {
        return;
    }
    let list = doc.resolve(af);
    let Some(list) = list.as_array() else { return };
    if list.len() > MAX_AF_ENTRIES {
        out.truncated = true;
    }
    for (i, entry) in list.iter().take(MAX_AF_ENTRIES).enumerate() {
        if out.files.len() >= MAX_AF_ROWS {
            out.truncated = true;
            return;
        }
        let spec = doc.resolve(entry);
        let Some(spec) = spec.as_dict() else { continue };
        let text = |key: &[u8]| spec.get(key).map(|o| doc.resolve(o)).and_then(|o| o.as_string().map(PdfString::to_text));
        let embedded_stream = spec
            .get(b"EF")
            .map(|o| doc.resolve(o))
            .and_then(|ef| ef.as_dict().and_then(|d| d.get(b"UF").or_else(|| d.get(b"F")).cloned()))
            .map(|o| doc.resolve(&o));
        let stream_dict = embedded_stream.as_deref().and_then(Object::as_dict);
        let size = stream_dict
            .and_then(|d| d.get(b"Params").map(|p| doc.resolve(p)))
            .and_then(|p| p.as_dict().and_then(|p| p.get(b"Size").and_then(Object::as_int)))
            .and_then(|n| u64::try_from(n).ok());
        out.files.push(AssociatedFile {
            page,
            index: i + 1,
            relationship: spec
                .get(b"AFRelationship")
                .map(|o| doc.resolve(o))
                .and_then(|o| o.as_name().map(|n| String::from_utf8_lossy(n).into_owned())),
            name: text(b"UF").or_else(|| text(b"F")),
            description: text(b"Desc"),
            mime: stream_dict.and_then(|d| d.get(b"Subtype")).and_then(Object::as_name).map(|n| String::from_utf8_lossy(n).into_owned()),
            size,
            embedded: stream_dict.is_some(),
        });
    }
}

/// The catalog's `/AF` as written: its array (a copy, so that a shared indirect array is never
/// changed in place).
fn catalog_af(doc: &Document) -> Result<(ObjRef, Vec<Object>), OrganizeError> {
    let root = doc.root().ok_or(OrganizeError::NoPageTree)?;
    let catalog = doc.get(root).as_dict().cloned().ok_or_else(|| OrganizeError::Invalid("the document has no catalog".into()))?;
    let Some(af) = catalog.get(b"AF") else { return Ok((root, Vec::new())) };
    match doc.resolve(af).as_array() {
        Some(list) => Ok((root, list.clone())),
        None => Err(OrganizeError::Invalid("the catalog's /AF is not an array; it is left as it is".into())),
    }
}

/// Write the catalog's `/AF` as a new direct array. A page or another holder that shares the
/// old indirect array keeps it exactly as it was (copy on write).
fn write_catalog_af(doc: &mut Document, root: ObjRef, list: Vec<Object>) -> Result<(), OrganizeError> {
    doc.update_dict(root, |c| {
        if list.is_empty() {
            c.remove(b"AF");
        } else {
            c.set(b"AF".to_vec(), Object::Array(list));
        }
    })?;
    Ok(())
}

/// Embed `bytes` as `name` (made unique, as for any attachment) and associate it with the
/// document: the file specification gets `/AFRelationship` and joins the catalog's `/AF`.
/// Returns the attachment key and the 1-based position in `/AF`. Refuses before changing
/// anything: an unknown relationship, `EncryptedPayload` (its file specification needs an
/// `/EP` encrypted-payload dictionary, which this does not write), an empty name, a file over [`MAX_AF_BYTES`], an `/AF`
/// that is not an array, or one already holding [`MAX_AF_ENTRIES`].
pub fn add_associated_file(
    doc: &mut Document,
    name: &str,
    bytes: &[u8],
    relationship: &str,
    description: Option<&str>,
    mime: Option<&str>,
    date: Option<&str>,
) -> Result<(String, usize), OrganizeError> {
    if !AF_RELATIONSHIPS.contains(&relationship) {
        return Err(OrganizeError::Invalid(format!("unknown relationship {relationship:?}; use one of {}", AF_RELATIONSHIPS.join(", "))));
    }
    if relationship == "EncryptedPayload" {
        return Err(OrganizeError::Invalid(
            "EncryptedPayload is not supported: it marks an encrypted file, and its file specification then needs an /EP encrypted-payload dictionary, which is not written; use Data, Source, Alternative, Supplement, FormData, Schema or Unspecified".into(),
        ));
    }
    if bytes.len() > MAX_AF_BYTES {
        return Err(OrganizeError::Invalid(format!("the file is {} bytes; an associated file may be at most {MAX_AF_BYTES}", bytes.len())));
    }
    let (root, mut list) = catalog_af(doc)?;
    if list.len() >= MAX_AF_ENTRIES {
        return Err(OrganizeError::Invalid(format!("the document already has {} associated files; at most {MAX_AF_ENTRIES}", list.len())));
    }
    if name.rsplit(['/', '\\']).next().is_none_or(|n| n.trim().is_empty()) {
        return Err(OrganizeError::Invalid("the associated file needs a file name".into()));
    }
    // PDF/A-3 wants every embedded file to state its media type.
    let mime = mime.map(str::trim).filter(|m| !m.is_empty()).unwrap_or("application/octet-stream");
    let (key, spec) = add_attachment_spec(doc, name, bytes, description, Some(mime), date)?;
    doc.update_dict(spec, |s| s.set(b"AFRelationship".to_vec(), Object::name(relationship)))?;
    list.push(Object::Ref(spec));
    let index = list.len();
    write_catalog_af(doc, root, list)?;
    Ok((key, index))
}

/// Whether the catalog's `/AF` entry at 1-based `index` is a file specification that anything
/// else also lists in an `/AF` array (another catalog entry or any page's). A page tree too big
/// to read in full counts as shared: the answer errs towards keeping files.
pub fn associated_file_is_shared(doc: &Document, index: usize) -> Result<bool, OrganizeError> {
    let (_, list) = catalog_af(doc)?;
    let Some(Object::Ref(spec)) = index.checked_sub(1).and_then(|i| list.get(i)) else { return Ok(false) };
    let own = list.iter().enumerate().filter(|(i, o)| *i + 1 != index && **o == Object::Ref(*spec)).count();
    if own > 0 {
        return Ok(true);
    }
    let pages = crate::pages(doc)?;
    if pages.len() > MAX_PAGES_READ {
        return Ok(true);
    }
    let mut seen = HashSet::new();
    for page in &pages {
        let Some(dict) = doc.get(page.obj).as_dict().cloned() else { continue };
        let Some(af) = dict.get(b"AF") else { continue };
        if let Some(r) = af.as_ref()
            && !seen.insert(r)
        {
            continue;
        }
        if doc.resolve(af).as_array().is_some_and(|l| l.iter().take(MAX_AF_ENTRIES).any(|o| *o == Object::Ref(*spec))) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Remove the document-level associated file at 1-based `index` of the catalog's `/AF`. When the
/// same file specification is also an attachment (as every file [`add_associated_file`] embeds
/// is) that attachment, found by the specification itself and not by its name, is deleted too
/// unless `keep_attachment` or something else (another catalog entry or a page's `/AF`) still
/// lists the same specification. Returns whether an attachment was deleted.
pub fn remove_associated_file(doc: &mut Document, index: usize, keep_attachment: bool) -> Result<bool, OrganizeError> {
    let (root, mut list) = catalog_af(doc)?;
    if index == 0 || index > list.len() {
        return Err(OrganizeError::Invalid(format!("there is no associated file {index}; the document has {}", list.len())));
    }
    let shared = associated_file_is_shared(doc, index)?;
    let removed = list.remove(index - 1);
    if matches!(removed, Object::Ref(_)) && !keep_attachment && !shared {
        // The attachment is deleted below; find out now whether its tree can be rewritten, so a
        // refusal leaves /AF as it was.
        crate::attachments::entries_to_rewrite(doc)?;
    }
    write_catalog_af(doc, root, list)?;
    match removed {
        Object::Ref(spec) if !keep_attachment && !shared => delete_attachment_by_ref(doc, spec),
        _ => Ok(false),
    }
}

/// Whether `dict` can be a file specification: its `/Type`, when it has one, is `Filespec`, and at
/// least one of its entries has the type a file specification gives it: `/F` or `/UF` a string,
/// `/EF` a dictionary, `/FS` a name (references are followed). An annotation, whose `/F` is a
/// number of flags, and a file attachment annotation, whose `/FS` is a dictionary, are not.
fn looks_like_filespec(doc: &Document, dict: &Dict) -> bool {
    let has = |key: &[u8], ok: fn(&Object) -> bool| match dict.get(key) {
        None => false,
        Some(o @ Object::Ref(_)) => ok(&doc.resolve(o)),
        Some(o) => ok(o),
    };
    let type_ok = match dict.get(b"Type") {
        None => true,
        Some(o @ Object::Ref(_)) => doc.resolve(o).as_name() == Some(&b"Filespec"[..]),
        Some(o) => o.as_name() == Some(&b"Filespec"[..]),
    };
    type_ok
        && (has(b"F", |o| o.as_string().is_some())
            || has(b"UF", |o| o.as_string().is_some())
            || has(b"EF", |o| matches!(o, Object::Dict(_)))
            || has(b"FS", |o| o.as_name().is_some()))
}

/// Whether the catalog's `/AF` is written as an indirect object (so it may be shared with a page),
/// as opposed to a direct array or none at all.
pub fn catalog_af_is_indirect(doc: &Document) -> bool {
    doc.root().and_then(|r| doc.get(r).as_dict().and_then(|c| c.get(b"AF")).map(|o| matches!(o, Object::Ref(_)))).unwrap_or(false)
}

/// Give the file specification at 1-based `index` of the catalog's `/AF` the `/AFRelationship`
/// `relationship`, replacing the name it had (or adding one where it had none or a value that was
/// not a name: ISO 32000-2 requires one on every file specification an `/AF` array lists).
/// Returns the name that was there, `None` when it was missing or not a name.
///
/// Refuses before changing anything: an unknown relationship, `EncryptedPayload` (its file
/// specification needs an `/EP` encrypted-payload dictionary, which this does not write), an `/AF` that is
/// not an array or holds more than [`MAX_AF_ENTRIES`] entries (the listing reads no further), an
/// `index` outside it, an entry that is not a file specification dictionary (a number, a
/// string, a stream, or a dictionary that has a `/Type` other than `Filespec`, or has none
/// of `/F` or `/UF` as a string, `/EF` as a dictionary and `/FS` as a name: the catalog, a page,
/// an annotation, with its `/F` flags number, and a file attachment annotation, with its `/FS`
/// dictionary, are refused),
/// and a specification that already has this relationship (nothing would change). A
/// specification given by reference that a page's `/AF` (or another catalog entry) also lists is
/// the same object, so it changes for them too; see [`associated_file_is_shared`]. A specification
/// written in the array itself is changed in a new direct array for the catalog; if that array
/// was indirect, the old array object is left as it was, so another holder of it keeps the old
/// entry (see [`catalog_af_is_indirect`]).
pub fn set_associated_relationship(doc: &mut Document, index: usize, relationship: &str) -> Result<Option<String>, OrganizeError> {
    if !AF_RELATIONSHIPS.contains(&relationship) {
        return Err(OrganizeError::Invalid(format!("unknown relationship {relationship:?}; use one of {}", AF_RELATIONSHIPS.join(", "))));
    }
    if relationship == "EncryptedPayload" {
        return Err(OrganizeError::Invalid(
            "EncryptedPayload is not supported: it marks an encrypted file, and its file specification then needs an /EP encrypted-payload dictionary, which is not written; use Data, Source, Alternative, Supplement, FormData, Schema or Unspecified".into(),
        ));
    }
    let root = doc.root().ok_or(OrganizeError::NoPageTree)?;
    let name_of =
        |spec: &Dict| spec.get(b"AFRelationship").map(|o| doc.resolve(o)).and_then(|o| o.as_name().map(|n| String::from_utf8_lossy(n).into_owned()));
    // Read in place: only the one entry is copied, after the array's length has been checked.
    let entry = {
        let catalog = doc.get(root);
        let Some(af) = catalog.as_dict().and_then(|c| c.get(b"AF")) else {
            return Err(OrganizeError::Invalid("the document has no associated files".into()));
        };
        let resolved;
        let list = match af {
            Object::Array(list) => list,
            other => {
                resolved = doc.resolve(other);
                match resolved.as_array() {
                    Some(list) => list,
                    None => return Err(OrganizeError::Invalid("the catalog's /AF is not an array; it is left as it is".into())),
                }
            }
        };
        if list.len() > MAX_AF_ENTRIES {
            return Err(OrganizeError::Invalid(format!(
                "the catalog's /AF holds {} entries; only the first {MAX_AF_ENTRIES} are listed, so it is left as it is",
                list.len()
            )));
        }
        match index.checked_sub(1).and_then(|i| list.get(i)) {
            Some(entry) => entry.clone(),
            None => return Err(OrganizeError::Invalid(format!("there is no associated file {index}; the document has {}", list.len()))),
        }
    };
    match entry {
        Object::Ref(spec) => {
            let previous = {
                let obj = doc.get(spec);
                let dict = match &*obj {
                    Object::Dict(d) if looks_like_filespec(doc, d) => d,
                    _ => {
                        return Err(OrganizeError::Invalid(format!(
                            "associated file {index} is not a file specification dictionary; it is left as it is"
                        )));
                    }
                };
                name_of(dict)
            };
            if previous.as_deref() == Some(relationship) {
                return Err(OrganizeError::Invalid(format!("associated file {index} already has the relationship {relationship}")));
            }
            doc.update_dict(spec, |s| s.set(b"AFRelationship".to_vec(), Object::name(relationship)))?;
            Ok(previous)
        }
        Object::Dict(mut spec) => {
            if !looks_like_filespec(doc, &spec) {
                return Err(OrganizeError::Invalid(format!("associated file {index} is not a file specification dictionary; it is left as it is")));
            }
            let previous = name_of(&spec);
            if previous.as_deref() == Some(relationship) {
                return Err(OrganizeError::Invalid(format!("associated file {index} already has the relationship {relationship}")));
            }
            spec.set(b"AFRelationship".to_vec(), Object::name(relationship));
            // The specification is written in the array itself: write the array again, as a new
            // direct one, so an indirect array a page also uses keeps what it said.
            let (root, mut list) = catalog_af(doc)?;
            if let Some(slot) = index.checked_sub(1).and_then(|i| list.get_mut(i)) {
                *slot = Object::Dict(spec);
            }
            write_catalog_af(doc, root, list)?;
            Ok(previous)
        }
        _ => Err(OrganizeError::Invalid(format!("associated file {index} is not a file specification dictionary; it is left as it is"))),
    }
}
