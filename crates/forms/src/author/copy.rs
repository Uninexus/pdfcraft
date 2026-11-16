//! Prepare a form ▸ copy an existing field: a new field with the same type, flags, look, options
//! and actions as `source`, at another place, with a name of its own (Acrobat's "Create Multiple
//! Copies" makes a grid of these; the grid is laid out by the engine).
//!
//! The copy is a field in its own right, not another widget of the same field (that is
//! [`duplicate_field`](super::duplicate_field)), so it has its own value: the value is not copied,
//! a check box starts off and a signature field unsigned.

use super::look::redraw_field;
use super::tree::{add_to_page, attach, clash, ensure_form, name_parts};
use super::{Dict, Document, FieldKind, FormError, MIN_FIELD_SIDE, Object, fields, invalid, page_refs};

/// Keys that belong to this field's place or value, not to a copy of it. The appearance is drawn
/// again for the copy.
const NOT_COPIED: [&[u8]; 10] = [b"Parent", b"Kids", b"T", b"V", b"P", b"Rect", b"StructParent", b"AP", b"AS", b"RV"];

/// The keys a field takes from the nodes above it, copied from them when the widget lacks them.
const FROM_ANCESTORS: [&[u8]; 14] = [b"FT", b"Ff", b"DV", b"DA", b"Q", b"MaxLen", b"Opt", b"TU", b"TM", b"AA", b"I", b"TI", b"Lock", b"DS"];

/// The most nodes above a field that are read for what it inherits.
const MAX_ANCESTORS: usize = 32;

/// Add a field on `page` (0-based) in `rect` (user space) that has the same type, flags, look,
/// options and actions as the field `source`, under the full name `name`. Returns the name.
///
/// # Errors
/// [`FormError::NoSuchField`] when there is no field `source`; [`FormError::Invalid`] for a radio
/// button (it belongs to its group), a field with no widget, a box under [`MIN_FIELD_SIDE`], a page
/// that does not exist, or a name that is empty, has an empty part or is, or sits inside, another
/// field's.
pub fn copy_field(doc: &mut Document, source: &str, page: usize, rect: [f64; 4], name: &str) -> Result<String, FormError> {
    let existing = fields(doc);
    let f = existing.iter().find(|f| f.name == source).ok_or_else(|| FormError::NoSuchField(source.into()))?;
    if f.kind == FieldKind::Radio {
        return invalid("a radio button belongs to its group: add another button to the group instead of copying it");
    }
    let model = f.widgets.first().ok_or_else(|| FormError::Invalid(format!("{source} has no widget")))?;
    let pages = page_refs(doc);
    let page_ref = *pages.get(page).ok_or_else(|| FormError::Invalid(format!("page {} does not exist", page + 1)))?;
    let rect = [rect[0].min(rect[2]), rect[1].min(rect[3]), rect[0].max(rect[2]), rect[1].max(rect[3])];
    if !rect.iter().all(|v| v.is_finite()) || rect[2] - rect[0] < MIN_FIELD_SIDE || rect[3] - rect[1] < MIN_FIELD_SIDE {
        return invalid("the copy is too small");
    }
    let name = name.trim();
    let parts = name_parts(name)?;
    if let Some(why) = existing.iter().find_map(|other| clash(name, &other.name)) {
        return invalid(why);
    }
    let mut d: Dict = doc.get(model.obj).as_dict().cloned().unwrap_or_default();
    // The nodes above the widget hold what it inherits.
    let mut above = d.reference(b"Parent");
    for _ in 0..MAX_ANCESTORS {
        let Some(r) = above else { break };
        let Some(node) = doc.get(r).as_dict().cloned() else { break };
        for key in FROM_ANCESTORS {
            if d.get(key).is_none()
                && let Some(v) = node.get(key)
            {
                d.set(key.to_vec(), v.clone());
            }
        }
        above = node.reference(b"Parent");
    }
    for key in NOT_COPIED {
        d.remove(key);
    }
    d.set(b"Type".to_vec(), Object::name("Annot"));
    d.set(b"Subtype".to_vec(), Object::name("Widget"));
    d.set(b"Rect".to_vec(), Object::Array(rect.iter().map(|v| Object::Real(*v)).collect()));
    d.set(b"P".to_vec(), Object::Ref(page_ref));
    let af = ensure_form(doc)?;
    let widget = doc.add(Object::Dict(d));
    attach(doc, af, widget, &parts)?;
    add_to_page(doc, page_ref, widget)?;
    redraw_field(doc, name)?;
    Ok(name.to_owned())
}
