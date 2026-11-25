//! Standards ▸ Reference XObjects: the form XObjects of the document that point at a page of another file (`/Ref`, ISO
//! 32000-2 8.10.4, used by PDF/X-5g and PDF/X-5pg), listed as the tool `reference_xobjects` lists them, with a Remove
//! Reference button on each one that can be changed. A removal is the engine's `RemoveReferenceXObject` edit, which
//! the tool applies too: one undo step. The referenced file is never opened and its page is not drawn.

use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Outcome as _, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::style::TypeRole;
use egui::Ui;
use pdfcraft_engine::{DocId, Edit};

use crate::PdfCraftApp;

/// The dialog has no fields of its own; the form is only what the dialog shell binds.
#[derive(Clone, Debug, Default, PartialEq)]
struct NoFields;

/// One reference XObject as the dialog shows it.
#[derive(Clone, Debug)]
struct Row {
    /// Its position in the tool's listing: what the removal takes.
    index: usize,
    file: Option<String>,
    object: String,
    page_index: Option<i64>,
    page_label: Option<String>,
    has_id: bool,
    removable: bool,
    exact: bool,
}

/// What the active document holds, read when the document or its edit count changed.
#[derive(Clone, Debug, Default)]
struct Listing {
    rows: Vec<Row>,
    /// How many were found, listed or not.
    count: usize,
    /// More were found than listed.
    truncated: bool,
    /// The scan stopped at its allowance, so `count` is a lower bound.
    capped: bool,
    /// The document's security allows changes.
    modifiable: bool,
}

/// Reference XObjects, owned by the dialog host.
pub(crate) struct ReferenceXObjectsDialog {
    id: DialogId,
    form: NoFields,
    shown: Listing,
    /// The document and edit count `shown` was read at.
    key: Option<(DocId, u64)>,
    /// The position whose Remove Reference was pressed; `reference_xobjects_pass` carries it out.
    action: Option<usize>,
}

impl PdfCraftApp {
    /// Opens Reference XObjects for the active document; the list is read on the first pass.
    pub(crate) fn open_reference_xobjects(&mut self) {
        if self.active_ids().is_none() {
            return;
        }
        let dialog =
            ReferenceXObjectsDialog { id: dialog_id("reference-xobjects"), form: NoFields, shown: Listing::default(), key: None, action: None };
        crate::dialogs::host::open(self, dialog);
    }

    /// The reference XObjects of document `id`, and whether it may be changed.
    fn reference_xobject_listing(&self, id: DocId) -> Listing {
        let Some(doc) = self.session.get(id) else { return Listing::default() };
        let read = doc.reference_xobjects();
        let rows = read
            .items
            .iter()
            .map(|r| Row {
                index: r.index,
                file: r.file.clone(),
                object: format!("{} {}", r.object.num, r.object.generation),
                page_index: r.page_index,
                page_label: r.page_label.clone(),
                has_id: r.has_id,
                removable: r.removable,
                exact: r.exact,
            })
            .collect();
        let modifiable = doc.read_only_reason.is_none() && doc.allows_modification();
        Listing { rows, count: read.count, truncated: read.truncated, capped: read.capped, modifiable }
    }

    /// Carries out the Remove Reference the open dialog was asked for and reads the list again when the document changed.
    pub(crate) fn reference_xobjects_pass(&mut self, ctx: &egui::Context) {
        let Some(asked) = self.dialogs.get_mut::<ReferenceXObjectsDialog>().map(|d| d.action.take()) else { return };
        let Some((_, id)) = self.active_ids() else {
            self.dialogs.cancel::<ReferenceXObjectsDialog>(ctx);
            return;
        };
        let generation = self.session.get(id).map(|d| d.edit_generation());
        let listed = self.dialogs.get::<ReferenceXObjectsDialog>().and_then(|d| d.key);
        // A press made on a list the document has since left is dropped: the position may now name another form.
        let current = listed.is_some_and(|(doc, at)| doc == id && Some(at) == generation);
        if let Some(index) = asked.filter(|_| current) {
            let modifiable = self.reference_xobject_listing(id).modifiable;
            if modifiable {
                self.apply_edit(ctx, Edit::RemoveReferenceXObject(index));
            } else {
                self.notify_tr("This document is read-only, so reference XObjects cannot be changed.");
            }
        }
        let generation = self.session.get(id).map(|d| d.edit_generation());
        let now = generation.map(|g| (id, g));
        if self.dialogs.get::<ReferenceXObjectsDialog>().is_some_and(|d| d.key != now) {
            let listing = self.reference_xobject_listing(id);
            if let Some(d) = self.dialogs.get_mut::<ReferenceXObjectsDialog>() {
                d.shown = listing;
                d.key = now;
            }
        }
    }
}

/// The line under a row's name: its object, page and whether `/ID` is whole.
fn detail(row: &Row) -> String {
    let page = match (&row.page_label, row.page_index) {
        (Some(label), _) => crate::i18n::fmt(tl!("page {label}"), &[("label", label)]),
        (None, Some(index)) => crate::i18n::fmt(tl!("page {n}"), &[("n", &index.saturating_add(1).to_string())]),
        (None, None) => tl!("no page named").to_string(),
    };
    let id = if row.has_id { tl!("ID is valid") } else { tl!("ID is missing or not valid") };
    let cut = if row.exact { String::new() } else { format!(", {}", tl!("name cut short")) };
    crate::i18n::fmt(tl!("object {object}, {page}, {id}{cut}"), &[("object", &row.object), ("page", &page), ("id", id), ("cut", &cut)])
}

/// The name a row is announced by.
fn row_name(row: &Row) -> String {
    match &row.file {
        Some(file) => crate::i18n::fmt(tl!("Reference to {file}"), &[("file", file)]),
        None => tl!("Reference to a file with no name").to_string(),
    }
}

impl StackDialog for ReferenceXObjectsDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let ReferenceXObjectsDialog { form, shown, action, .. } = self;
        let close = FooterButton::new(tl!("Close").to_string(), DialogRole::Cancel);
        let out = FormDialog::new(tl!("Reference XObjects").to_string(), Bind::bound(form)).shaped(|d| d.buttons([close])).show(ui, |ui, _, _| {
            Text::new(
                tl!("Forms that stand in for a page of another file (PDF/X-5g and PDF/X-5pg). Removing the reference keeps the form's own content. The other file is not opened and its page is not drawn.")
                    .to_string(),
            )
            .role(TypeRole::Caption)
            .shade(Shade::Muted)
            .show(ui);
            if !shown.modifiable {
                Notice::new(Tone::Warn, tl!("This document does not allow changes, so references cannot be removed.").to_string())
                    .layout(NoticeLayout::Inline)
                    .show(ui);
            }
            if shown.rows.is_empty() {
                Text::new(tl!("This document has no reference XObjects.").to_string()).show(ui);
            }
            for row in &shown.rows {
                let line = detail(row);
                Field::new(row_name(row)).value(line.clone()).show(ui, |ui, _| {
                    Text::new(line.clone()).show(ui);
                    if !row.removable {
                        Text::new(tl!("This form is part of the page tree, so its reference cannot be removed here.").to_string())
                            .role(TypeRole::Caption)
                            .shade(Shade::Muted)
                            .show(ui);
                    } else if Button::new(tl!("Remove Reference").to_string()).enabled(shown.modifiable).id_salt(row.index).show(ui).clicked() {
                        *action = Some(row.index);
                    }
                });
            }
            if shown.truncated || shown.capped {
                let listed = shown.rows.len();
                let more = shown.count.saturating_sub(listed);
                let line = if shown.capped {
                    crate::i18n::fmt(
                        tl!("The scan stopped early: at least {count} were found and {listed} are listed."),
                        &[("count", &shown.count.to_string()), ("listed", &listed.to_string())],
                    )
                } else {
                    crate::i18n::fmt(tl!("The list was cut short: {more} more are not listed."), &[("more", &more.to_string())])
                };
                Notice::new(Tone::Warn, line).layout(NoticeLayout::Inline).show(ui);
            }
            true
        });
        out.closed.map(|_| DialogResult::Cancel)
    }
}
