//! Who a form field is meant for: a private label on the field's dictionary, `/PdfCraftSigner`,
//! holding a recipient's email address as a text string.
//!
//! This is PdfCraft's own key, not part of the PDF specification, and the label is nothing more
//! than that: no other reader looks at it (a reader that does not know a key ignores it), and
//! nothing here, or in any reader, makes the field fillable or signable by only that person. It
//! says who the form's author meant the field for, so a plan for several signers can show which
//! fields are whose; it does not restrict anyone. The address is written into the PDF and travels
//! with every copy of the file.
//!
//! Two departures from `docs/esign-design.md` (Local design, item 2): the label is on the field's
//! own dictionary, not on a widget kid, and the assignment lives in the PDF while the signing plan
//! lives in its sidecar, so the two are kept in step by hand (the report lists where they differ).
//! The key is unlikely to collide with another producer's, not guaranteed not to.
//!
//! The label sits on the field's own dictionary (the dictionary with the field's `/T`, which also
//! holds the widget when the field and its widget are one object) and is not inherited from a
//! parent or read from a widget kid. A read has an allowance of [`MAX_GETS`] reads, charged before
//! each is made. A signature field that holds a signature (its `/V` is not null, with `/FT` found
//! on the field or a parent) is not edited: a change after the signature would be a change the
//! signature does not cover. A read still says what is there. Whether a label on any other field
//! would break a signature already in the file (a certification that allows no changes, or a
//! signature whose lock names the field) is decided by the validator, in the engine, which refuses
//! such a write.

use pdfcraft_cos::{Document, ObjRef, Object, PdfString};

use crate::OrganizeError;
use crate::reference_xobjects::{Gets, Spent};

/// The key written on the field's dictionary.
pub const KEY: &[u8] = b"PdfCraftSigner";
/// Most bytes of an address written, which is the longest an address may be (RFC 5321 path).
pub const MAX_EMAIL_BYTES: usize = 254;
/// Most characters of a label listed from a file; a longer one is cut and not `exact`.
pub const MAX_READ_CHARS: usize = 254;
/// Most reads one call of [`read_field_signer`] makes.
pub const MAX_GETS: usize = 1_000;
/// Most `/Parent` links followed to find a field's type.
const MAX_PARENTS: usize = 32;

/// What [`read_field_signer`] read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldSigner {
    /// The label as text, when the key holds a string (after references), cut at
    /// [`MAX_READ_CHARS`] characters.
    pub email: Option<String>,
    /// The key is there and is not null: what a clear takes out.
    pub has_entry: bool,
    /// The key is there but is not a string, so it is not a label this reads.
    pub not_text: bool,
    /// The key holds a string with no text in it (empty or only white space): there is no label,
    /// and a clear takes the key out. `email` is `None` then.
    pub empty: bool,
    /// `email` is whole (false when it was cut).
    pub exact: bool,
    /// The field is a signature field that holds a signature, so it is not edited.
    pub signed: bool,
    /// The object is a dictionary with a `/T`, so it is a field this can edit.
    pub settable: bool,
    /// The [`MAX_GETS`] allowance was spent, so what follows the stop was not read.
    pub capped: bool,
}

/// What [`set_field_signer`] would do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// It would write the field's dictionary.
    Yes,
    /// It would write nothing: the label is already as asked, or there is none to clear.
    NoChange,
    /// It is refused, for this reason.
    Refused(&'static str),
}

fn walk(doc: &Document, gets: &mut Gets, field: ObjRef, out: &mut FieldSigner) -> Result<(), Spent> {
    out.exact = true;
    let held = gets.get(doc, field)?;
    let Some(d) = held.as_dict() else { return Ok(()) };
    out.settable = d.get(b"T").is_some_and(|t| !matches!(t, Object::Null));
    let own_value = d.get(b"V").is_some_and(|v| !matches!(v, Object::Null));
    if let Some(entry) = d.get(KEY).filter(|e| !matches!(e, Object::Null)) {
        out.has_entry = true;
        match &*gets.follow(doc, entry)? {
            Object::String(s) => {
                let mut chars = s.text_chars();
                let text: String = chars.by_ref().take(MAX_READ_CHARS).collect();
                out.exact = chars.next().is_none();
                if text.trim().is_empty() {
                    out.empty = true;
                } else {
                    out.email = Some(text);
                }
            }
            _ => out.not_text = true,
        }
    }
    if own_value {
        // The type is on the field or a parent of it.
        let mut at = held.clone();
        for _ in 0..MAX_PARENTS {
            let Some(dict) = at.as_dict() else { break };
            if let Some(ft) = dict.name(b"FT") {
                out.signed = ft == b"Sig";
                break;
            }
            let Some(parent) = dict.reference(b"Parent") else { break };
            at = gets.get(doc, parent)?;
        }
    }
    Ok(())
}

/// The label of `field` as stored, with whether it can be edited.
pub fn read_field_signer(doc: &Document, field: ObjRef) -> FieldSigner {
    let mut out = FieldSigner::default();
    if walk(doc, &mut Gets(MAX_GETS), field, &mut out).is_err() {
        out.capped = true;
        out.settable = false;
    }
    out
}

/// What setting the label of `field` to `email` (a clear when `None`) would do.
pub fn field_signer_change(doc: &Document, field: ObjRef, email: Option<&str>) -> Change {
    let read = read_field_signer(doc, field);
    if read.capped {
        return Change::Refused("the field could not be read within the allowance of reads");
    }
    if !read.settable {
        return Change::Refused("the object is not a form field this can edit");
    }
    if read.signed {
        return Change::Refused("the field holds a signature, and a signed field is not changed");
    }
    match email {
        None if !read.has_entry => Change::NoChange,
        Some(e) if read.exact && read.email.as_deref() == Some(e) => Change::NoChange,
        Some(e) if e.is_empty() || e.len() > MAX_EMAIL_BYTES => Change::Refused("an address is 1 to 254 bytes"),
        _ => Change::Yes,
    }
}

/// Set the label of `field` to `email` as a text string, or take the key out when `None`.
pub fn set_field_signer(doc: &mut Document, field: ObjRef, email: Option<&str>) -> Result<(), OrganizeError> {
    match field_signer_change(doc, field, email) {
        Change::NoChange => return Ok(()),
        Change::Refused(why) => return Err(OrganizeError::Invalid(why.into())),
        Change::Yes => {}
    }
    doc.update_dict(field, |d| match email {
        Some(e) => d.set(KEY.to_vec(), Object::String(PdfString::text(e))),
        None => {
            d.remove(KEY);
        }
    })?;
    Ok(())
}
