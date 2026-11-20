//! What a Submit a Form action asks for, read without sending anything (the PDF specification:
//! `/F` the target, `/Fields` the names, `/Flags` the choices).

use pdfcraft_cos::{Document, Object};

use crate::actions::Trigger;
use crate::read::{action_fields_counted, held};
use crate::{FormError, fields};

/// A target address longer than this many bytes is not copied (the flag `url_too_long` says so).
pub const MAX_SUBMIT_URL_BYTES: usize = 64 * 1024;

/// How the action sends the form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmitFormat {
    /// Forms Data Format (no format flag set).
    Fdf,
    /// HTML form encoding (`ExportFormat`, bit 3).
    Html,
    /// XML Forms Data Format (`XFDF`, bit 6).
    Xfdf,
    /// The whole document as a PDF (`SubmitPDF`, bit 9).
    Pdf,
}

impl SubmitFormat {
    /// Snake-case id (agents).
    pub fn id(self) -> &'static str {
        match self {
            SubmitFormat::Fdf => "fdf",
            SubmitFormat::Html => "html",
            SubmitFormat::Xfdf => "xfdf",
            SubmitFormat::Pdf => "pdf",
        }
    }
}

/// Which fields the action names.
#[derive(Clone, Debug, PartialEq)]
pub enum SubmitFields {
    /// No `/Fields` entry: every field.
    All,
    /// `/Fields` with the `Include/Exclude` flag clear: only these. A reference is listed by the
    /// field's fully qualified name (the partial names up its `/Parent` chain joined by dots), a
    /// string as written; entries that name nothing are left out ([`SubmitAction::fields_left_out`]),
    /// so an empty list can come from a non-empty `/Fields`.
    Only(Vec<String>),
    /// `/Fields` with the flag set: every field but these.
    Except(Vec<String>),
    /// `/Fields` is longer than one action may list (the caps on a field list): not listed.
    TooMany,
}

/// A Submit a Form action of one field, as written. Nothing here has been sent anywhere.
#[derive(Clone, Debug, PartialEq)]
pub struct SubmitAction {
    pub trigger: Trigger,
    /// The `/F` address (a string, or a file specification's `/F`); empty when absent or when `/F` is anything else (a name, a number, a dictionary with no `/F` string).
    pub url: String,
    /// `/F` was longer than [`MAX_SUBMIT_URL_BYTES`] and is not copied.
    pub url_too_long: bool,
    /// The format. When several bits are set, the specification's own rules decide: SubmitPDF
    /// makes every other flag ignored except GetMethod, and ExportFormat (HTML) is meaningful
    /// only when SubmitPDF and XFDF are clear; so the order is PDF, XFDF, HTML, else FDF.
    pub format: SubmitFormat,
    pub fields: SubmitFields,
    /// Entries of `/Fields` that name nothing (a reference to nothing, a dictionary with no
    /// `/T` up its parents, a number, `null`) and so are not in `fields`.
    pub fields_left_out: usize,
    /// `/Flags` (the low 32 bits).
    pub flags: u32,
    // The booleans are reported as set, whether or not the format uses them: with `Pdf` the whole
    // document is sent and only `get_method` applies; `get_method` and `submit_coordinates` are
    // for `Html` only; `include_append_saves`, `include_annotations`, `excl_f_key` and
    // `embed_form` are for `Fdf` only; `excl_non_user_annots` only with `include_annotations`.
    pub get_method: bool,
    pub include_no_value_fields: bool,
    pub submit_coordinates: bool,
    pub include_append_saves: bool,
    pub include_annotations: bool,
    pub canonical_format: bool,
    pub excl_non_user_annots: bool,
    pub excl_f_key: bool,
    pub embed_form: bool,
}

fn bit(flags: u32, n: u32) -> bool {
    flags & (1 << (n - 1)) != 0
}

/// The Submit a Form actions that field `name` (its first widget) runs on each trigger, in the
/// Actions tab's order. Only the action a trigger starts is read, not the ones chained after it
/// with `/Next`, and nothing is sent. Triggers whose action is anything else are left out.
pub fn submit_actions(doc: &Document, name: &str) -> Result<Vec<SubmitAction>, FormError> {
    let all = fields(doc);
    let f = all.iter().find(|f| f.name == name).ok_or_else(|| FormError::NoSuchField(name.into()))?;
    let Some(w) = f.widgets.first() else { return Ok(Vec::new()) };
    let widget = doc.get(w.obj);
    let Some(wd) = widget.as_dict() else { return Ok(Vec::new()) };
    let aa = wd.get(b"AA").map(|a| held(doc, a));
    let aa = aa.as_ref().and_then(|a| a.get().as_dict());
    let mut out = Vec::new();
    for t in Trigger::ALL {
        let o = match t.key() {
            None => wd.get(b"A").or_else(|| aa.and_then(|aa| aa.get(b"U"))),
            Some(k) => aa.and_then(|aa| aa.get(k)),
        };
        let Some(o) = o else { continue };
        let action = held(doc, o);
        let Some(d) = action.get().as_dict().filter(|d| d.name(b"S") == Some(b"SubmitForm")) else { continue };
        let flags = d.get(b"Flags").and_then(|v| held(doc, v).get().as_int()).map_or(0, |v| (v & 0xFFFF_FFFF) as u32);
        let (mut url, mut url_too_long) = (String::new(), false);
        if let Some(target) = d.get(b"F") {
            let target = held(doc, target);
            let text = match target.get() {
                Object::Dict(fd) => fd.get(b"F").map(|x| held(doc, x)),
                _ => None,
            };
            let string = match &text {
                Some(x) => x.get().as_string(),
                None => target.get().as_string(),
            };
            if let Some(s) = string {
                // The cap is checked on the encoded length, before any text is built.
                if s.bytes.len() > MAX_SUBMIT_URL_BYTES {
                    url_too_long = true;
                } else {
                    url = s.to_text();
                }
            }
        }
        let listed = d.get(b"Fields").map(|o| action_fields_counted(doc, o));
        let mut fields_left_out = 0;
        let fields = match listed {
            None => SubmitFields::All,
            Some(None) => SubmitFields::TooMany,
            Some(Some((names, left_out))) => {
                fields_left_out = left_out;
                if bit(flags, 1) { SubmitFields::Except(names) } else { SubmitFields::Only(names) }
            }
        };
        let format = if bit(flags, 9) {
            SubmitFormat::Pdf
        } else if bit(flags, 6) {
            SubmitFormat::Xfdf
        } else if bit(flags, 3) {
            SubmitFormat::Html
        } else {
            SubmitFormat::Fdf
        };
        out.push(SubmitAction {
            trigger: t,
            url,
            url_too_long,
            format,
            fields,
            fields_left_out,
            flags,
            get_method: bit(flags, 4),
            include_no_value_fields: bit(flags, 2),
            submit_coordinates: bit(flags, 5),
            include_append_saves: bit(flags, 7),
            include_annotations: bit(flags, 8),
            canonical_format: bit(flags, 10),
            excl_non_user_annots: bit(flags, 11),
            excl_f_key: bit(flags, 12),
            embed_form: bit(flags, 14),
        });
    }
    Ok(out)
}
