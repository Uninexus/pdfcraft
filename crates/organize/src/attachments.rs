//! Document-level file attachments (Acrobat: Edit ▸ Attach file; the Attachments panel): the
//! `/Names /EmbeddedFiles` name tree of file specifications, each with its embedded stream.
//!
//! A FileAttachment annotation (a paper clip on a page) is a comment and lives in the annot
//! crate; this is the document level only.

use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString, Stream};

use crate::OrganizeError;
use crate::import::{name_tree_entries, set_attachments, set_attachments_exact};

/// The keys of the document-level attachments, in name-tree order.
pub fn attachment_keys(doc: &Document) -> Vec<String> {
    // A listing: a tree past the key allowance shows what was read (changing it refuses).
    entries(doc).0.into_iter().map(|(k, _)| PdfString::literal(k).to_text()).collect()
}

/// Attach `bytes` to the document under `name` (a file name; any folder part is dropped). The
/// name tree's key is the name, made unique with " (2)", " (3)"… when it is already taken, and
/// is returned so the caller can find the attachment again. `date` is a PDF date string for the
/// embedded file's `/ModDate`; `mime` its `/Subtype` when known.
pub fn add_attachment(
    doc: &mut Document,
    name: &str,
    bytes: &[u8],
    description: Option<&str>,
    mime: Option<&str>,
    date: Option<&str>,
) -> Result<String, OrganizeError> {
    add_attachment_spec(doc, name, bytes, description, mime, date).map(|(key, _)| key)
}

/// [`add_attachment`], also returning the new file specification (what `/AF` arrays point at).
pub(crate) fn add_attachment_spec(
    doc: &mut Document,
    name: &str,
    bytes: &[u8],
    description: Option<&str>,
    mime: Option<&str>,
    date: Option<&str>,
) -> Result<(String, ObjRef), OrganizeError> {
    let name = file_name(name);
    if name.is_empty() {
        return Err(OrganizeError::Invalid("the attachment needs a file name".into()));
    }
    let mut all = entries_to_rewrite(doc)?;
    let taken: Vec<String> = all.iter().map(|(k, _)| PdfString::literal(k.clone()).to_text()).collect();
    let mut key = name.clone();
    let mut n = 2;
    while taken.contains(&key) {
        key = format!("{name} ({n})");
        n += 1;
    }

    let mut params = Dict::new();
    params.set(b"Size".to_vec(), Object::Int(bytes.len() as i64));
    if let Some(d) = date {
        params.set(b"ModDate".to_vec(), Object::String(PdfString::literal(d.as_bytes().to_vec())));
    }
    let mut ef = Dict::new();
    ef.set(b"Type".to_vec(), Object::name("EmbeddedFile"));
    if let Some(m) = mime.map(str::trim).filter(|m| !m.is_empty()) {
        ef.set(b"Subtype".to_vec(), Object::Name(m.as_bytes().to_vec()));
    }
    ef.set(b"Params".to_vec(), Object::Dict(params));
    let stream = doc.add(Object::Stream(Stream::flate(ef, bytes)));

    let mut spec = Dict::new();
    spec.set(b"Type".to_vec(), Object::name("Filespec"));
    spec.set(b"F".to_vec(), Object::String(PdfString::text(&key)));
    spec.set(b"UF".to_vec(), Object::String(PdfString::text(&key)));
    if let Some(d) = description.map(str::trim).filter(|d| !d.is_empty()) {
        spec.set(b"Desc".to_vec(), Object::String(PdfString::text(d)));
    }
    let mut files = Dict::new();
    files.set(b"F".to_vec(), Object::Ref(stream));
    files.set(b"UF".to_vec(), Object::Ref(stream));
    spec.set(b"EF".to_vec(), Object::Dict(files));
    let spec = doc.add(spec);

    all.push((PdfString::text(&key).bytes, Object::Ref(spec)));
    set_attachments(doc, all)?;
    Ok((key, spec))
}

/// Remove the document-level attachment whose name-tree key is `key`. The embedded stream is
/// left for the writer's garbage collection, as with every other removal.
pub fn delete_attachment(doc: &mut Document, key: &str) -> Result<(), OrganizeError> {
    let mut all = entries_to_rewrite(doc)?;
    let before = all.len();
    all.retain(|(k, _)| PdfString::literal(k.clone()).to_text() != key);
    if all.len() == before {
        return Err(OrganizeError::Invalid(format!("there is no attachment named {key}")));
    }
    if all.is_empty() { clear_embedded_files(doc) } else { set_attachments(doc, all) }
}

/// Remove the document-level attachment entries whose value is the file specification `spec`
/// itself, whatever their keys read as. Returns whether any was removed.
pub(crate) fn delete_attachment_by_ref(doc: &mut Document, spec: ObjRef) -> Result<bool, OrganizeError> {
    let mut all = entries_to_rewrite(doc)?;
    let before = all.len();
    all.retain(|(_, v)| *v != Object::Ref(spec));
    if all.len() == before {
        return Ok(false);
    }
    if all.is_empty() {
        clear_embedded_files(doc)?
    } else {
        set_attachments_exact(doc, all)?
    }
    Ok(true)
}

/// Set or clear the description of a document-level attachment without replacing its file
/// specification or embedded stream.
pub fn set_attachment_description(doc: &mut Document, key: &str, description: Option<&str>) -> Result<(), OrganizeError> {
    let mut all = entries_to_rewrite(doc)?;
    let Some((_, value)) = all.iter_mut().find(|(name, _)| PdfString::literal(name.clone()).to_text() == key) else {
        return Err(OrganizeError::Invalid(format!("there is no attachment named {key}")));
    };
    let description = description.map(str::trim).filter(|s| !s.is_empty()).map(PdfString::text).map(Object::String);
    match value {
        Object::Ref(reference) => {
            doc.update_dict(*reference, |spec| match description {
                Some(description) => spec.set(b"Desc".to_vec(), description),
                None => {
                    spec.remove(b"Desc");
                }
            })?;
        }
        Object::Dict(spec) => {
            match description {
                Some(description) => spec.set(b"Desc".to_vec(), description),
                None => {
                    spec.remove(b"Desc");
                }
            }
            set_attachments_exact(doc, all)?;
        }
        _ => return Err(OrganizeError::Invalid(format!("attachment {key} has an invalid file specification"))),
    }
    Ok(())
}

/// The `/EmbeddedFiles` entries (key, file specification), in tree order.
/// A tree past the key allowance (see `name_tree_entries`) gives what was read and `false`.
pub(crate) fn entries(doc: &Document) -> (Vec<(Vec<u8>, Object)>, bool) {
    let Some(tree) = doc
        .root()
        .and_then(|r| doc.get(r).as_dict().cloned())
        .and_then(|c| c.get(b"Names").map(|n| doc.resolve(n)))
        .and_then(|n| n.as_dict().and_then(|d| d.get(b"EmbeddedFiles").cloned()))
    else {
        return (Vec::new(), true);
    };
    let mut out = Vec::new();
    let complete = name_tree_entries(doc, &tree, 0, &mut Default::default(), &mut out).is_ok();
    (out, complete)
}

/// Every entry, for a change that writes the tree back flat: an error, with the document
/// untouched, when the tree is past the key allowance, since writing back what was read would
/// silently delete the rest.
pub(crate) fn entries_to_rewrite(doc: &Document) -> Result<Vec<(Vec<u8>, Object)>, OrganizeError> {
    match entries(doc) {
        (all, true) => Ok(all),
        _ => Err(OrganizeError::Invalid(format!(
            "the attachments (EmbeddedFiles) name tree's keys total more than {} MiB (a key shared by many pairs counts once per pair), so the attachments are not edited; nothing was changed",
            crate::import::MAX_NAME_TREE_KEY_BYTES / (1024 * 1024)
        ))),
    }
}

/// Drop `/EmbeddedFiles` from the catalog's `/Names` (and `/Names` itself when nothing is left).
fn clear_embedded_files(doc: &mut Document) -> Result<(), OrganizeError> {
    let root = doc.root().ok_or(OrganizeError::NoPageTree)?;
    let names = doc.get(root).as_dict().and_then(|c| c.get(b"Names").cloned());
    match names {
        Some(Object::Ref(r)) => {
            let mut empty = false;
            doc.update_dict(r, |n| {
                n.remove(b"EmbeddedFiles");
                empty = n.is_empty();
            })?;
            if empty {
                doc.update_dict(root, |c| {
                    c.remove(b"Names");
                })?;
            }
        }
        Some(Object::Dict(mut n)) => {
            n.remove(b"EmbeddedFiles");
            doc.update_dict(root, |c| {
                if n.is_empty() {
                    c.remove(b"Names");
                } else {
                    c.set(b"Names".to_vec(), Object::Dict(n));
                }
            })?;
        }
        _ => {}
    }
    Ok(())
}

/// The last path component of `name`, trimmed: a dragged-in path or a folder part never becomes
/// the attachment's name.
fn file_name(name: &str) -> String {
    name.rsplit(['/', '\\']).next().unwrap_or(name).trim().to_string()
}

/// A media type from the file's extension, for the embedded file's `/Subtype`; `None` when it
/// is not one of the common kinds (the attachment is still fine without it).
pub fn mime_for_name(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
    Some(match ext.as_str() {
        "pdf" => "application/pdf",
        "txt" | "log" | "md" => "text/plain",
        "csv" => "text/csv",
        "htm" | "html" => "text/html",
        "xml" => "text/xml",
        "json" => "application/json",
        "zip" => "application/zip",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "rtf" => "application/rtf",
        "eml" => "message/rfc822",
        "msg" => "application/vnd.ms-outlook",
        _ => return None,
    })
}
