//! Base URL for relative links: the document's `/URI` dictionary in the catalog and its `/Base`
//! string, which the viewer resolves a relative address of a URI action against (ISO 32000-2
//! Table 211; `pdfcraft_render::resolve_uri` is what reads it).
//!
//! An entry that is there but is not a string this module reads (not a string, over
//! [`MAX_BASE_URL_BYTES`] bytes, or not UTF-8) is read as no base and named by [`read_base_url`];
//! [`set_base_url`] leaves it as it is unless a base other than "none" is asked for. Reading
//! borrows the catalog and the dictionary, and measures a string before it copies it.
//!
//! References are followed as far as the viewer's own reader goes, for `/URI` and for `/Base`: up
//! to [`MAX_HOPS`] of them, and a stream's dictionary counts as a dictionary.

use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString};
use std::sync::Arc;

use crate::OrganizeError;
use crate::print_presets::Held;

/// Longest base read or written, in bytes: the length the viewer reads a `/Base` up to, so a
/// longer one would be stored and then not used.
pub const MAX_BASE_URL_BYTES: usize = 8192;

/// Most references followed from the catalog's `/URI` to its dictionary, or from `/Base` to its
/// string: the length of chain the viewer's reader (`link_list`) follows. A longer one is read
/// here as not reachable.
const MAX_HOPS: usize = 128;

/// What `/URI` is in the catalog.
enum Uri {
    /// No entry, a null, or a reference that ends in nothing: a writer starts a dictionary.
    Absent,
    /// A dictionary written in the catalog.
    Direct,
    /// A dictionary object (or a stream, whose dictionary counts), reached through one reference
    /// or a chain of them (the last one).
    Indirect(ObjRef),
    /// Anything else, including a chain of more than [`MAX_HOPS`] references: read as no base
    /// and never replaced.
    Other,
}

fn uri_kind(doc: &Document, catalog: &Dict) -> Uri {
    let mut at = match catalog.get(b"URI") {
        None | Some(Object::Null) => return Uri::Absent,
        Some(Object::Dict(_)) => return Uri::Direct,
        Some(Object::Ref(r)) => *r,
        Some(_) => return Uri::Other,
    };
    for _ in 0..MAX_HOPS {
        match &*doc.get(at) {
            Object::Dict(_) | Object::Stream(_) => return Uri::Indirect(at),
            Object::Null => return Uri::Absent,
            Object::Ref(next) => at = *next,
            _ => return Uri::Other,
        }
    }
    Uri::Other
}

/// What `r` refers to after up to [`MAX_HOPS`] more references (the last reference's object when
/// the chain is longer, which no reader above takes for a string).
fn follow(doc: &Document, r: ObjRef) -> Arc<Object> {
    let mut held = doc.get(r);
    for _ in 0..MAX_HOPS {
        let Object::Ref(next) = &*held else { break };
        held = doc.get(*next);
    }
    held
}

/// The base as stored, and what is unreadable: `uri` when the catalog's `/URI` is neither a
/// dictionary nor null, `base` when the `/Base` entry is there but is not a string of at most
/// [`MAX_BASE_URL_BYTES`] bytes of UTF-8. One read of the catalog.
pub fn read_base_url(doc: &Document) -> (Option<String>, Vec<&'static str>) {
    let mut bad = Vec::new();
    let Some(root) = doc.root() else { return (None, bad) };
    let catalog_object = doc.get(root);
    let Some(catalog) = catalog_object.as_dict() else { return (None, bad) };
    let target;
    let uri: &Dict = match (uri_kind(doc, catalog), catalog.get(b"URI")) {
        (Uri::Absent, _) => return (None, bad),
        (Uri::Other, _) => {
            bad.push("uri");
            return (None, bad);
        }
        (Uri::Direct, Some(Object::Dict(d))) => d,
        (Uri::Indirect(r), _) => {
            target = doc.get(r);
            match target.as_dict() {
                Some(d) => d,
                None => return (None, bad),
            }
        }
        _ => return (None, bad),
    };
    let value = uri.get(b"Base").map(|o| match o {
        Object::Ref(r) => Held::Resolved(follow(doc, *r)),
        o => Held::Direct(o),
    });
    let base = match value.as_deref() {
        None | Some(Object::Null) => None,
        Some(Object::String(s)) if s.bytes.len() <= MAX_BASE_URL_BYTES => match std::str::from_utf8(&s.bytes) {
            Ok(text) => Some(text.to_owned()),
            Err(_) => {
                bad.push("base");
                None
            }
        },
        Some(_) => {
            bad.push("base");
            None
        }
    };
    (base, bad)
}

/// Whether `base` can be written: it starts with `http://` or `https://` (in any case) followed
/// by a letter, a digit or `[`, is printable ASCII with no space, and is at most
/// [`MAX_BASE_URL_BYTES`] bytes. This is this module's limit, narrower than what a PDF may hold,
/// and it does not parse the address (this crate has no URL parser): it accepts some a link
/// cannot be resolved against (`https://a:99999/`, `http://[`), which the viewer then leaves as
/// written. The tool reports that as `links_use_it`.
pub fn check_base_url(base: &str) -> Result<(), String> {
    if base.len() > MAX_BASE_URL_BYTES {
        return Err(format!("a base URL holds at most {MAX_BASE_URL_BYTES} bytes, not {}", base.len()));
    }
    if !base.bytes().all(|b| (0x21..=0x7E).contains(&b)) {
        return Err("a base URL must be printable ASCII with no space".into());
    }
    let rest = match (base.get(..7), base.get(..8)) {
        (Some(s), _) if s.eq_ignore_ascii_case("http://") => base.get(7..),
        (_, Some(s)) if s.eq_ignore_ascii_case("https://") => base.get(8..),
        _ => None,
    };
    if !rest.and_then(|r| r.bytes().next()).is_some_and(|b| b.is_ascii_alphanumeric() || b == b'[') {
        return Err("a base URL must be a web address that names its host, such as https://example.com/docs/".into());
    }
    Ok(())
}

/// Set the base (`Some`) or take it away (`None`). Only a change writes: asking for what is read
/// changes nothing, so a `/Base` this module cannot read stays unless a base is asked for, and a
/// stored base that [`check_base_url`] would refuse (`ftp://…`) may be sent back as it is. A base
/// that differs from the stored one and is refused by [`check_base_url`], or a catalog `/URI`
/// that is not a dictionary (or is behind more than [`MAX_HOPS`] references), writes nothing.
/// Other entries of the `/URI` dictionary are kept, and an emptied one goes. A null or dangling
/// `/URI` counts as absent and gets a new dictionary; a dictionary behind references is changed
/// where it is.
pub fn set_base_url(doc: &mut Document, base: Option<&str>) -> Result<(), OrganizeError> {
    if base == read_base_url(doc).0.as_deref() {
        return Ok(());
    }
    if let Some(b) = base {
        check_base_url(b).map_err(OrganizeError::Invalid)?;
    }
    let root = doc.root().ok_or(OrganizeError::NoPageTree)?;
    let kind = {
        let catalog = doc.get(root);
        uri_kind(doc, catalog.as_dict().ok_or(OrganizeError::NoPageTree)?)
    };
    if matches!(kind, Uri::Other) {
        return Err(OrganizeError::Invalid(format!(
            "the document's /URI is not a dictionary, or is behind more than {MAX_HOPS} references, so it was left as it is"
        )));
    }
    let apply = |d: &mut Dict| match base {
        Some(b) => d.set(b"Base".to_vec(), Object::String(PdfString::literal(b.as_bytes()))),
        None => {
            d.remove(b"Base");
        }
    };
    match kind {
        Uri::Indirect(r) => {
            doc.update_dict(r, apply)?;
            if matches!(&*doc.get(r), Object::Dict(d) if d.is_empty()) {
                doc.update_dict(root, |c| {
                    c.remove(b"URI");
                })?;
            }
        }
        Uri::Direct => doc.update_dict(root, |c| {
            let emptied = match c.get_mut(b"URI") {
                Some(Object::Dict(d)) => {
                    apply(d);
                    d.is_empty()
                }
                _ => false,
            };
            if emptied {
                c.remove(b"URI");
            }
        })?,
        // `Other` was refused above; it is named here only so that the match is exhaustive.
        Uri::Absent | Uri::Other => {
            let mut d = Dict::new();
            apply(&mut d);
            doc.update_dict(root, |c| {
                if d.is_empty() {
                    c.remove(b"URI");
                } else {
                    c.set(b"URI".to_vec(), Object::Dict(d.clone()));
                }
            })?;
        }
    }
    Ok(())
}
