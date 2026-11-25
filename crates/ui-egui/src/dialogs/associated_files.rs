//! PDF 2.0 associated files (`/AF`) in the Attachments panel: the panel options menu's Attach as Associated File…
//! (choose a file, then its relationship and an optional description) and the row menu's Set Relationship…. Each
//! confirmed dialog applies the edit the tools `af_add` and `af_set_relationship` apply, as one undo step; a document
//! that cannot be changed gets a notice instead of a dialog.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
use pdfcraft_engine::{AF_RELATIONSHIPS, DocId, Edit, MAX_AF_BYTES};

use crate::PdfCraftApp;

/// The Attach as Associated File dialog's key, which is also its command's id.
pub(crate) const ATTACH_KEY: &str = "attachment.associate";
/// The Set Relationship dialog's key.
pub(crate) const RELATIONSHIP_KEY: &str = "attachment.relationship";

/// The relationship a new associated file starts with.
const DEFAULT_RELATIONSHIP: &str = "Unspecified";

/// A relationship name as the user reads it. A name this app has no text for is shown as the file has it.
pub(crate) fn relationship_label(name: Option<&str>) -> String {
    match name {
        Some("Source") => tl!("Source").to_string(),
        Some("Data") => tl!("Data").to_string(),
        Some("Alternative") => tl!("Alternative").to_string(),
        Some("Supplement") => tl!("Supplement").to_string(),
        Some("EncryptedPayload") => tl!("Encrypted payload").to_string(),
        Some("FormData") => tl!("Form data").to_string(),
        Some("Schema") => tl!("Schema").to_string(),
        Some("Unspecified") | None => tl!("Unspecified").to_string(),
        Some(other) => other.to_string(),
    }
}

/// The relationships a dialog offers: the ones the tools accept (not EncryptedPayload, which needs an `/EP` dictionary).
fn relationship_param(default: &str) -> Param {
    let options: Vec<(String, craft::ui::runtime::env::Label)> = AF_RELATIONSHIPS
        .iter()
        .filter(|name| **name != "EncryptedPayload")
        .map(|name| (name.to_string(), relationship_label(Some(*name)).into()))
        .collect();
    let default = if options.iter().any(|(id, _)| id == default) { default } else { DEFAULT_RELATIONSHIP };
    Param::new("relationship", tl!("Relationship"), ParamKind::choice(options, Some(default.to_string())))
}

/// The active document's view and edit generation, kept in a dialog's fields to know the document is as it was.
fn remember_document(app: &PdfCraftApp, fields: &mut DialogFields) {
    if let Some((view, id)) = app.active_ids() {
        fields.set("view", view.to_string());
        fields.set("generation", app.session.get(id).map(|d| d.edit_generation()).unwrap_or_default().to_string());
    }
}

/// The active document when it is the one `fields` were opened for, unchanged; else a notice and `None`.
fn unchanged_since_open(app: &mut PdfCraftApp, fields: &DialogFields) -> Option<DocId> {
    let current = app.active_ids().and_then(|(view, id)| app.session.get(id).map(|d| (view, id, d.edit_generation())));
    let asked_view = fields.text("view").and_then(|v| v.parse::<usize>().ok());
    let asked_generation = fields.text("generation").and_then(|v| v.parse::<u64>().ok());
    match current {
        Some((view, id, generation)) if asked_view == Some(view) && asked_generation == Some(generation) => Some(id),
        _ => {
            app.notify_tr("The document changed while the dialog was open, so nothing was changed. Open the dialog again.");
            None
        }
    }
}

/// The active document when it may be changed; else a notice and `None`.
fn writable(app: &mut PdfCraftApp) -> Option<DocId> {
    let (_, id) = app.active_ids()?;
    let read_only = app.session.get(id).is_none_or(|doc| doc.read_only_reason.is_some() || !doc.allows_modification());
    if read_only {
        app.notify_tr("This document is read-only, so associated files cannot be changed.");
        return None;
    }
    Some(id)
}

/// Attach as Associated File…: asks for the file first, then for its relationship and description.
pub(crate) fn start(app: &mut PdfCraftApp) {
    let Some(doc) = writable(app) else { return };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let target = app.pick_target(Some(doc));
        let request = craft::host::PickRequest::open("attachment").title(tl!("Attach as Associated File").to_string());
        app.pick_first(request, target, |app, ctx, path| open_attach(app, ctx, &path));
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = doc;
        app.notify_tr("Attaching files is not available on the web yet");
    }
}

/// Opens the relationship dialog for the file chosen at `path`.
#[cfg(not(target_arch = "wasm32"))]
fn open_attach(app: &mut PdfCraftApp, ctx: &egui::Context, path: &std::path::Path) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_AF_BYTES as u64) {
        app.notify_fmt("{name} is larger than 32 MiB, the most an associated file can be.", &[("name", &name)]);
        return;
    }
    let mut fields = DialogFields::new();
    fields.set("path", path.to_string_lossy().into_owned());
    fields.set("relationship", DEFAULT_RELATIONSHIP.to_owned());
    fields.set("description", String::new());
    remember_document(app, &mut fields);
    let dialog = HostedParamDialog::new(ctx, ATTACH_KEY, fields, move |fields| {
        ParamDialog::new(
            ATTACH_KEY,
            tl!("Attach as Associated File"),
            vec![
                Param::read_only_text("file", tl!("File"), name.clone()),
                relationship_param(DEFAULT_RELATIONSHIP),
                Param::new("description", tl!("Description"), ParamKind::text(String::new())),
            ],
        )
        .values(Bind::bound(fields))
    });
    crate::dialogs::host::open(app, dialog);
}

/// Attach as Associated File was confirmed with `fields`: embeds the file and lists it in the catalog's `/AF`, as one
/// undo step.
pub(crate) fn finish_attach(ctx: &egui::Context, app: &mut PdfCraftApp, fields: &DialogFields) {
    if writable(app).is_none() || unchanged_since_open(app, fields).is_none() {
        return;
    }
    let path = std::path::PathBuf::from(fields.text("path").unwrap_or_default());
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) => {
            app.notify_fmt("Couldn't read {name}: {e}", &[("name", &name), ("e", &e.to_string())]);
            return;
        }
    };
    let relationship =
        fields.text("relationship").filter(|r| AF_RELATIONSHIPS.contains(r) && *r != "EncryptedPayload").unwrap_or(DEFAULT_RELATIONSHIP).to_owned();
    let description = fields.text("description").map(str::trim).filter(|d| !d.is_empty()).map(str::to_owned);
    // A file of unknown type is stored as application/octet-stream, as the tool does.
    let mime = Some(pdfcraft_engine::mime_for_name(&name).unwrap_or("application/octet-stream").to_string());
    if app.apply_edit(ctx, Edit::AddAssociatedFile { name, bytes, relationship, description, mime }) {
        app.set_right(Some(crate::RightPanel::Attachments));
    }
}

/// Opens Set Relationship… for the document-level associated file at 1-based `index` of the catalog's `/AF`, called
/// `name`, whose relationship is `current`.
pub(crate) fn open_relationship(app: &mut PdfCraftApp, ctx: &egui::Context, index: usize, name: String, current: Option<String>) {
    if writable(app).is_none() {
        return;
    }
    let current = current.unwrap_or_else(|| DEFAULT_RELATIONSHIP.to_owned());
    let mut fields = DialogFields::new();
    fields.set("index", index.to_string());
    fields.set("name", name.clone());
    fields.set("relationship", current.clone());
    remember_document(app, &mut fields);
    let dialog = HostedParamDialog::new(ctx, RELATIONSHIP_KEY, fields, move |fields| {
        ParamDialog::new(
            RELATIONSHIP_KEY,
            tl!("Set Relationship"),
            vec![Param::read_only_text("file", tl!("File"), name.clone()), relationship_param(&current)],
        )
        .values(Bind::bound(fields))
    });
    crate::dialogs::host::open(app, dialog);
}

/// Set Relationship was confirmed with `fields`: sets `/AFRelationship` as one undo step, when the entry is still the
/// one the dialog was opened for and the relationship differs.
pub(crate) fn finish_relationship(ctx: &egui::Context, app: &mut PdfCraftApp, fields: &DialogFields) {
    if writable(app).is_none() {
        return;
    }
    let Some(id) = unchanged_since_open(app, fields) else { return };
    let index = fields.text("index").and_then(|v| v.parse::<usize>().ok());
    let name = fields.text("name").unwrap_or_default();
    let entry = app
        .session
        .get(id)
        .and_then(|doc| doc.associated_files().into_iter().find(|f| f.page.is_none() && Some(f.index) == index && f.name.as_deref() == Some(name)));
    let Some(entry) = entry else {
        app.notify_tr("That associated file is no longer listed, so nothing was changed.");
        return;
    };
    let Some(relationship) = fields.text("relationship").filter(|r| AF_RELATIONSHIPS.contains(r) && *r != "EncryptedPayload") else { return };
    if entry.relationship.as_deref() != Some(relationship) {
        app.apply_edit(ctx, Edit::SetAssociatedRelationship { index: entry.index, relationship: relationship.to_owned() });
    }
}
