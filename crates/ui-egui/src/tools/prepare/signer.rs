//! The Signer entry of a signature field's Field Properties and the commands behind it,
//! `form.field.signer.set` and `form.field.signer.clear`.
//!
//! The commands make the edit `form_field_signer` makes: the engine's `field_signer_change`, which also applies the
//! shared address rule, then `Edit::SetFieldSigner`, one undo step. Like the tool they look at the signing plan beside
//! the document's file: with a plan there, only a signer or an approver of it can be named, and the label is written
//! with the plan's spelling of the address; with none, a well-formed address is accepted and the notice says it was not
//! checked against a plan. The label is stored in the file and restricts nobody.
//!
//! The commands act only on a Field Properties dialog that is open on the active document, and, for a clear, on the
//! selected field when none is open.

use std::path::Path;

use pdfcraft_engine::signing_plan as plan_rules;
use pdfcraft_engine::{DocId, Edit, FieldSignerChange, SignerAddressError, check_signer_address, is_hidden_char};

use super::FormFieldKind;
use super::dialog::FieldPropsDialog;
use crate::PdfCraftApp;

/// What a signature field holds as its signer label, as the dialog shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignerView {
    /// The label with hidden, control and direction-changing characters taken out, as `form_field_signer` echoes it;
    /// `None` when there is no address-like text.
    pub label: Option<String>,
    /// The key is there (so a clear has something to take out), even when it holds no text.
    pub has_entry: bool,
    /// Hidden characters were taken out of `label` for display.
    pub hidden_removed: bool,
    /// The stored label was longer than is read, and `label` is its first part.
    pub cut: bool,
}

/// What the signing plan beside a document says about who may be named.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PlanNote {
    /// The document has no file path (or not a .pdf one): no plan could be looked for.
    #[default]
    NotChecked,
    /// There is no plan file beside the document.
    None,
    /// A plan is there: the recipients who have a turn (name, address), in plan order.
    Found(Vec<(String, String)>),
    /// A plan file is there and cannot be used, for this reason (hidden characters taken out).
    Unreadable(String),
}

/// The longest piece of a refusal read from a file that is shown (characters).
const SHOWN_CHARS: usize = 300;

/// Text from a file or a refusal as it is shown: control, hidden and direction-changing characters taken out, and cut
/// at `max` characters.
pub(crate) fn clean(text: &str, max: usize) -> String {
    let clean: String = text.chars().filter(|c| !is_hidden_char(*c)).collect();
    if clean.chars().count() > max { format!("{}…", clean.chars().take(max).collect::<String>()) } else { clean }
}

/// The plan beside the file at `path`, read from disk.
pub(crate) fn plan_beside(path: Option<&str>) -> PlanNote {
    let Some(path) = path else { return PlanNote::NotChecked };
    let Ok(sidecar) = plan_rules::sidecar_of(Path::new(path)) else { return PlanNote::NotChecked };
    match plan_rules::read_sidecar(&sidecar) {
        Ok(None) => PlanNote::None,
        Ok(Some(plan)) => PlanNote::Found(plan.recipients.iter().filter(|r| r.role.acts()).map(|r| (r.name.clone(), r.email.clone())).collect()),
        Err(e) => PlanNote::Unreadable(clean(&e.0, SHOWN_CHARS)),
    }
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

    /// The saved path of document `id`.
    fn document_path(&self, id: DocId) -> Option<String> {
        self.session.get(id).and_then(|d| d.path.clone())
    }

    /// Fills the Signer entry of the open Field Properties from the document: the label the field holds now, and the
    /// plan beside the file. Asked when the dialog opens and after each command.
    pub(crate) fn load_signer_entry(&mut self, id: DocId) {
        let plan = plan_beside(self.document_path(id).as_deref());
        let Some((field, signature)) = self.dialogs.get::<FieldPropsDialog>().map(|d| (d.signer_field().to_owned(), d.signer_field_is_signature()))
        else {
            return;
        };
        if !signature {
            return;
        }
        let view = self.signer_view(id, &field);
        if let Some(dialog) = self.dialogs.get_mut::<FieldPropsDialog>() {
            dialog.set_signer_plan(plan);
            dialog.set_signer_view(view);
        }
    }

    /// Keeps the label the open Field Properties shows current, so an undo or a redo is seen. Cheap: it reads the
    /// document, not the plan file.
    pub(crate) fn refresh_signer_view(&mut self) {
        let Some((_, id)) = self.active_ids() else { return };
        let Some((field, signature)) = self.dialogs.get::<FieldPropsDialog>().map(|d| (d.signer_field().to_owned(), d.signer_field_is_signature()))
        else {
            return;
        };
        if !signature {
            return;
        }
        let view = self.signer_view(id, &field);
        if let Some(dialog) = self.dialogs.get_mut::<FieldPropsDialog>() {
            dialog.set_signer_view(view);
        }
    }

    /// `form.field.signer.set` and `form.field.signer.clear`: write or take out the signer label (`/PdfCraftSigner`) of
    /// the signature field whose Field Properties are open on the active document, or, for a clear with none open, of
    /// the selected field. Set takes the address typed in the open dialog and is refused with none. One undo step;
    /// refused where `form_field_signer` is.
    pub fn set_field_signer(&mut self, ctx: &egui::Context, clear: bool) {
        let Some((index, id)) = self.active_ids() else { return };
        // A dialog that was closed (Cancel, Escape) or opened on another document is not open for this command.
        let open = self
            .dialogs
            .get::<FieldPropsDialog>()
            .filter(|d| d.signer_doc() == Some(id))
            .map(|d| (d.signer_field().to_owned(), d.signer_typed().to_owned()));
        let selected = self.views.get(index).and_then(|v| v.session.prepare.selected.as_ref()).map(|(name, _)| name.clone());
        let (name, typed) = match (open, selected) {
            (Some(open), _) => open,
            (None, _) if !clear => return self.notify_tr("Open the signature field's properties and type the signer's email address first"),
            (None, Some(name)) => (name, String::new()),
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
        let mut checked_against_plan = false;
        let email = if clear {
            None
        } else {
            let address = match check_signer_address(&typed) {
                Ok(address) => address,
                Err(SignerAddressError::TooLong) => {
                    return self.notify_tr("An address may have at most 254 bytes, and at most 64 bytes before the @");
                }
                Err(SignerAddressError::Shape) => {
                    return self.notify_tr("An address must look like name@example.org (one @, no spaces or hidden characters, a dot in the domain)");
                }
            };
            match plan_beside(self.document_path(id).as_deref()) {
                PlanNote::Found(people) => {
                    let key = address.to_lowercase();
                    match people.iter().find(|(_, email)| email.to_lowercase() == key) {
                        // The plan's own spelling, so the label matches the plan exactly.
                        Some((_, email)) => {
                            checked_against_plan = true;
                            Some(email.clone())
                        }
                        None => {
                            return self.notify_fmt(
                                "{who} is not a signer or an approver of the signing plan beside the document, so it was not set",
                                &[("who", &clean(&address, 254))],
                            );
                        }
                    }
                }
                PlanNote::Unreadable(why) => {
                    return self
                        .notify_fmt("The signing plan beside the document could not be read, so the signer was not set ({why})", &[("why", &why)]);
                }
                PlanNote::NotChecked | PlanNote::None => Some(address),
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
        if !self.apply_edit(ctx, Edit::SetFieldSigner { field: obj, email: email.clone() }) {
            return;
        }
        // The dialog shows the new label, and the typed text is the label's spelling.
        let view = self.signer_view(id, &name);
        let shown = view.label.clone().unwrap_or_default();
        if let Some(dialog) = self.dialogs.get_mut::<FieldPropsDialog>().filter(|d| d.signer_doc() == Some(id) && d.signer_field() == name) {
            dialog.set_signer_text(shown);
            dialog.set_signer_view(view);
        }
        let mut message = match email {
            Some(who) => crate::i18n::fmt(
                tl!("Signer set to {who}. This is a label in the file: it does not restrict who can sign, and nothing is sent"),
                &[("who", &clean(&who, 254))],
            ),
            None => tl!("Signer cleared").to_string(),
        };
        if !clear && !checked_against_plan {
            message = format!("{message}. {}", tl!("It was not checked against a signing plan"));
        }
        self.notify(message);
    }
}
