//! The Signer entry of a signature field's Field Properties and the commands behind it,
//! `form.field.signer.set` and `form.field.signer.clear`.
//!
//! The commands make the edit `form_field_signer` makes (the engine's `field_signer_change`, which
//! also applies the shared address rule `pdfcraft_organize::field_signer::check_address`, then
//! `Edit::SetFieldSigner`). They are not the whole tool: the signing plan is read by the automation
//! crate, which this layer does not depend on, so an address typed here is not checked against the
//! plan (the dialog and the notice say so when a plan file sits beside the document).

use pdfcraft_engine::{DocId, Edit, FieldSignerChange, SignerAddressError, check_signer_address, is_hidden_char};

use super::FormFieldKind;
use crate::{Dialog, PdfCraftApp};

/// What a signature field holds as its signer label, as the dialog shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignerView {
    /// The label with hidden, control and direction-changing characters taken out, as
    /// `form_field_signer` echoes it; `None` when there is no address-like text.
    pub label: Option<String>,
    /// The key is there (so a clear has something to take out), even when it holds no text.
    pub has_entry: bool,
    /// Hidden characters were taken out of `label` for display.
    pub hidden_removed: bool,
    /// The stored label was longer than is read, and `label` is its first part.
    pub cut: bool,
}

impl PdfCraftApp {
    /// The signer label of the one field called `name` in document `id`.
    pub(crate) fn signer_view(&self, id: DocId, name: &str) -> SignerView {
        let Some(doc) = self.session.get(id) else { return SignerView::default() };
        let mut named = doc.form.iter().filter(|f| f.name == name);
        let (Some(field), None) = (named.next(), named.next()) else { return SignerView::default() };
        let read = doc.field_signer(field.obj);
        let (label, hidden_removed) = match &read.email {
            Some(e) => {
                let clean: String = e.chars().filter(|c| !is_hidden_char(*c)).collect();
                let removed = clean != *e;
                (Some(clean), removed)
            }
            None => (None, false),
        };
        SignerView { label, has_entry: read.has_entry, hidden_removed, cut: !read.exact }
    }

    /// A signing plan file (`<file>.pdfcraft-sign.json`) sits beside document `id`'s file.
    pub(crate) fn plan_sidecar_beside(&self, id: DocId) -> bool {
        self.session.get(id).and_then(|d| d.path.as_ref()).is_some_and(|p| std::fs::symlink_metadata(format!("{p}.pdfcraft-sign.json")).is_ok())
    }

    /// Re-read the label the open Field Properties shows, so an undo or redo is seen.
    pub(crate) fn refresh_signer_view(&mut self) {
        let Some((_, id)) = self.active_ids() else { return };
        let Some(d) = self.field_props.as_ref().filter(|d| d.kind == FormFieldKind::Signature && d.doc == Some(id)) else { return };
        let view = self.signer_view(id, &d.field);
        if let Some(d) = self.field_props.as_mut() {
            d.signer_view = view;
        }
    }

    /// `form.field.signer.set` and `form.field.signer.clear`: write or take out the signer label
    /// (`/PdfCraftSigner`, see `pdfcraft_organize::field_signer`) of the signature field whose
    /// Field Properties are open on the active document, or, for a clear with none open, of the
    /// selected field. Set takes the address typed in the open dialog and is refused with none.
    /// One undo step; refused where `form_field_signer` is.
    pub fn set_field_signer(&mut self, clear: bool) {
        let Some((index, id)) = self.active_ids() else { return };
        // A dialog that was closed (Cancel, Escape) or opened on another document is not open.
        if self.field_props.as_ref().is_some_and(|d| self.dialog != Some(Dialog::FieldProps) || d.doc != Some(id)) {
            self.field_props = None;
        }
        let name = match (&self.field_props, self.views[index].prepare.selected.as_ref()) {
            (Some(d), _) => d.field.clone(),
            (None, _) if !clear => return self.notify_tr("Open the signature field's properties and type the signer's email address first"),
            (None, Some((name, _))) => name.clone(),
            (None, None) => return self.notify_tr("Select a signature field first"),
        };
        let found = self.session.get(id).map(|doc| doc.form.iter().filter(|f| f.name == name).map(|f| (f.obj, f.kind)).collect::<Vec<_>>());
        let (obj, kind) = match found.as_deref() {
            Some([one]) => *one,
            _ => return self.notify_tr("That name does not pick out one field"),
        };
        if kind != FormFieldKind::Signature {
            return self.notify_tr("A signer can only be set on a signature field");
        }
        let email = if clear {
            None
        } else {
            let Some(d) = self.field_props.as_ref() else { return };
            match check_signer_address(&d.signer) {
                Ok(address) => Some(address),
                Err(SignerAddressError::TooLong) => {
                    return self.notify_tr("An address may have at most 254 bytes, and at most 64 bytes before the @");
                }
                Err(SignerAddressError::Shape) => {
                    return self.notify_tr("An address must look like name@example.org (one @, no spaces or hidden characters, a dot in the domain)");
                }
            }
        };
        let Some(doc) = self.session.get(id) else { return };
        match doc.field_signer_change(obj, email.as_deref()) {
            FieldSignerChange::Refused(why) => return self.notify(why),
            FieldSignerChange::NoChange => {
                return self.notify_tr(if clear { "This field has no signer" } else { "This field already has that signer" });
            }
            FieldSignerChange::Yes => {}
        }
        if !self.apply_edit(Edit::SetFieldSigner { field: obj, email: email.clone() }) {
            return;
        }
        let view = self.signer_view(id, &name);
        let plan = self.plan_sidecar_beside(id);
        if let Some(d) = self.field_props.as_mut().filter(|d| d.field == name) {
            d.signer = view.label.clone().unwrap_or_default();
            d.signer_view = view;
            d.signer_plan_beside = plan;
        }
        let mut message = match email {
            Some(who) => crate::i18n::fmt(
                tl!("Signer set to {who}. This is a label in the file: it does not restrict who can sign, and nothing is sent"),
                &[("who", &who)],
            ),
            None => tl!("Signer cleared").to_string(),
        };
        if plan {
            message = format!("{message}. {}", tl!("It was not checked against the signing plan beside this file"));
        }
        self.notify(message);
    }
}
