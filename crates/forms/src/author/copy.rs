//! Prepare a form ▸ copy an existing field: new fields with the same type, flags, look, options
//! and actions as `source`, at other places, each with a name of its own (Acrobat's "Create
//! Multiple Copies" makes a grid of these; the grid is laid out by the engine).
//!
//! A copy is a field in its own right, not another widget of the same field (that is
//! [`duplicate_field`](super::duplicate_field)), so it has its own value: the value is not copied,
//! a check box starts off (keeping its on state) and a signature field unsigned. A field with
//! several widgets is copied from its first widget.

use std::collections::HashSet;

use super::look::redraw_fields;
use super::tree::{add_to_page, attach, clash, ensure_form, inherited, name_parts};
use super::{Dict, Document, FieldKind, FormError, MIN_FIELD_SIDE, Object, fields, invalid, page_refs};

/// Keys that belong to the source's place, value or identity, not to a copy of it. The appearance
/// of everything but a check box is drawn again for the copy; a check box keeps its own so its on
/// state keeps its name.
const NOT_COPIED: [&[u8]; 12] = [b"Parent", b"Kids", b"T", b"V", b"I", b"P", b"Rect", b"StructParent", b"AP", b"AS", b"RV", b"NM"];
/// Also the last-modified date, which is the copy's own.
const ALSO_NOT_COPIED: [&[u8]; 1] = [b"M"];

/// Where one copy goes and what it is called.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldCopy {
    /// The page, 0-based.
    pub page: usize,
    /// The box, in user space.
    pub rect: [f64; 4],
    /// The full name of the new field.
    pub name: String,
}

/// Add a field on `page` (0-based) in `rect` (user space) that has the same type, flags, look,
/// options and actions as the field `source`, under the full name `name`. Returns the name. See
/// [`copy_field_to`].
///
/// # Errors
/// As [`copy_field_to`].
pub fn copy_field(doc: &mut Document, source: &str, page: usize, rect: [f64; 4], name: &str) -> Result<String, FormError> {
    let mut made = copy_field_to(doc, source, &[FieldCopy { page, rect, name: name.to_owned() }])?;
    made.pop().ok_or_else(|| FormError::Invalid("nothing was copied".into()))
}

/// Add every copy of `copies` of the field `source`, reading the form once for all of them. The
/// copies are worked out and checked before anything is written, so a refusal changes nothing.
/// Returns their names, trimmed, in order.
///
/// # Errors
/// [`FormError::NoSuchField`] when there is no field `source`; [`FormError::Invalid`] for a radio
/// button (it belongs to its group), a field with no widget, a box under [`MIN_FIELD_SIDE`], a
/// page that does not exist, or a name that is empty, has an empty part, or is, or sits inside, a
/// field's or another copy's.
pub fn copy_field_to(doc: &mut Document, source: &str, copies: &[FieldCopy]) -> Result<Vec<String>, FormError> {
    let existing = fields(doc);
    let f = existing.iter().find(|f| f.name == source).ok_or_else(|| FormError::NoSuchField(source.into()))?;
    if f.kind == FieldKind::Radio {
        return invalid("a radio button belongs to its group: add another button to the group instead of copying it");
    }
    let model = f.widgets.first().ok_or_else(|| FormError::Invalid(format!("{source} has no widget")))?;
    let pages = page_refs(doc);
    // Checked first: nothing is written until every copy is known to be good.
    let mut taken: Vec<String> = Vec::with_capacity(copies.len());
    let mut places = Vec::with_capacity(copies.len());
    for copy in copies {
        let page_ref = *pages.get(copy.page).ok_or_else(|| FormError::Invalid(format!("page {} does not exist", copy.page + 1)))?;
        let r = copy.rect;
        let rect = [r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3])];
        if !rect.iter().all(|v| v.is_finite()) || rect[2] - rect[0] < MIN_FIELD_SIDE || rect[3] - rect[1] < MIN_FIELD_SIDE {
            return invalid("the copy is too small");
        }
        let name = copy.name.trim();
        let parts = name_parts(name)?;
        if let Some(why) = existing.iter().map(|o| o.name.as_str()).chain(taken.iter().map(String::as_str)).find_map(|other| clash(name, other)) {
            return invalid(why);
        }
        taken.push(name.to_owned());
        places.push((page_ref, rect, name.to_owned(), parts.len()));
    }
    let template = template(doc, f, model);
    let calculates = template.get(b"AA").map(|aa| doc.resolve(aa)).and_then(|aa| aa.as_dict().map(|d| d.contains(b"C"))).unwrap_or(false);
    let af = ensure_form(doc)?;
    for (page_ref, rect, name, _) in &places {
        let mut d = template.clone();
        d.set(b"Rect".to_vec(), Object::Array(rect.iter().map(|v| Object::Real(*v)).collect()));
        d.set(b"P".to_vec(), Object::Ref(*page_ref));
        let widget = doc.add(Object::Dict(d));
        let parts = name_parts(name)?;
        attach(doc, af, widget, &parts)?;
        add_to_page(doc, *page_ref, widget)?;
        // A copy that calculates joins the end of the calculation order, as a new calculated
        // field does.
        if calculates {
            crate::calc_order::keep_listed(doc, widget, true)?;
        }
    }
    // Every appearance in one pass over a form read once, not a read of the form per copy.
    let new: HashSet<&str> = taken.iter().map(String::as_str).collect();
    redraw_fields(doc, |_, field| new.contains(field.name.as_str()))?;
    Ok(taken)
}

/// What every copy starts from: the source's own keys (its widget's over its field's, with their
/// two `/AA` dictionaries merged), then what the nodes above it hand down.
fn template(doc: &Document, f: &crate::Field, model: &crate::Widget) -> Dict {
    let widget = doc.get(model.obj).as_dict().cloned().unwrap_or_default();
    let mut d = if model.obj == f.obj {
        widget
    } else {
        // A field with widget kids: the field holds the type, flags, value and field actions, the
        // widget the look and its own actions.
        let mut d = doc.get(f.obj).as_dict().cloned().unwrap_or_default();
        let field_aa = d.get(b"AA").map(|o| doc.resolve(o)).and_then(|o| o.as_dict().cloned());
        let widget_aa = widget.get(b"AA").map(|o| doc.resolve(o)).and_then(|o| o.as_dict().cloned());
        for (key, value) in widget.iter() {
            d.set(key.clone(), value.clone());
        }
        if let (Some(mut merged), Some(own)) = (field_aa, widget_aa) {
            for (key, value) in own.iter() {
                merged.set(key.clone(), value.clone());
            }
            d.set(b"AA".to_vec(), Object::Dict(merged));
        }
        d
    };
    for (key, value) in inherited(doc, f.obj).iter() {
        if !d.contains(key) {
            d.set(key.clone(), value.clone());
        }
    }
    let check_box = f.kind == FieldKind::CheckBox;
    for key in NOT_COPIED.iter().chain(ALSO_NOT_COPIED.iter()) {
        if check_box && *key == b"AP" {
            continue;
        }
        d.remove(key);
    }
    if check_box {
        d.set(b"AS".to_vec(), Object::name("Off"));
    }
    d.set(b"Type".to_vec(), Object::name("Annot"));
    d.set(b"Subtype".to_vec(), Object::name("Widget"));
    d
}
