//! The Attachments panel: the document's embedded files and file-attachment annotations, with Save,
//! Open and Delete on each row and the row's menu. A file that the document or a page also lists as an associated file
//! (`/AF`, PDF 2.0) shows its relationship, and its menu sets the relationship or takes it off the list.
// PLAN: assumed `super::right::PanelResult` and a `BodyCx` passed to `show` (pdf-chrome-w13 calls `show(app, ui)` today).

use crate::app::commands::{FocusIf as _, list_focus};
use std::sync::Arc;

use craft::behavior::{DisplayOrder, IconId, ItemSource};
use craft::ui::buttons::Button;
use craft::ui::contract::{BodyCx, CollectionEvent, Entry, SelectMode, Show as _, ShowCollection as _};
use craft::ui::list::{List, ListRow, RowEvent, TrailingFace, TrailingPart};
use craft::ui::menu::Menu;
use craft::ui::primitives::IconSpec;

use super::right::PanelResult;
use super::{empty_state, human_size};
use crate::PdfCraftApp;

const SAVE: TrailingPart = TrailingPart::new(0);
const OPEN: TrailingPart = TrailingPart::new(1);
const DELETE: TrailingPart = TrailingPart::new(2);

/// What the row menu's entries do.
const MENU_OPEN: u8 = 0;
const MENU_SAVE: u8 = 1;
const MENU_DESCRIBE: u8 = 2;
const MENU_DELETE: u8 = 3;
const MENU_RELATIONSHIP: u8 = 4;
const MENU_REMOVE_ASSOCIATED: u8 = 5;
const MENU_REMOVE_ASSOCIATED_KEEP: u8 = 6;

/// The associated files of the document read for one edit generation of one document, so the panel reads the `/AF`
/// arrays once per change and not once per frame.
#[derive(Clone, Default)]
struct AssociatedCache {
    key: Option<(pdfcraft_engine::DocId, u64)>,
    files: Arc<Vec<pdfcraft_engine::AssociatedFile>>,
}

/// A request to open Set Relationship…: the entry's 1-based index in the catalog's `/AF`, the file's name and its
/// relationship.
type RelationshipRequest = (usize, String, Option<String>);

/// What the row menu's associated file entry `picked` does for the file `name` listed at `listed` (index and
/// relationship in the catalog's `/AF`): an edit to apply, or a Set Relationship… dialog to open.
fn associated_pick(picked: u8, name: &str, listed: Option<(usize, Option<String>)>) -> (Option<pdfcraft_engine::Edit>, Option<RelationshipRequest>) {
    let Some((index, current)) = listed else { return (None, None) };
    if picked == MENU_RELATIONSHIP {
        return (None, Some((index, name.to_owned(), current)));
    }
    // The tool's own edit: the attachment goes with the entry unless asked to stay (or another entry lists the file).
    (Some(pdfcraft_engine::Edit::RemoveAssociatedFile { index, keep_attachment: picked == MENU_REMOVE_ASSOCIATED_KEEP }), None)
}

/// The row detail for a file that `f` lists as an associated file; `page_label` names the page when it is a page's.
fn associated_detail(f: &pdfcraft_engine::AssociatedFile, page_label: &str) -> String {
    let relationship = crate::dialogs::associated_files::relationship_label(f.relationship.as_deref());
    match f.page {
        None => crate::i18n::fmt(tl!("Associated file ({relationship}) of the document"), &[("relationship", &relationship)]),
        Some(_) => {
            crate::i18n::fmt(tl!("Associated file ({relationship}) of page {label}"), &[("relationship", &relationship), ("label", page_label)])
        }
    }
}

/// The Attachments panel's options menu rows.
pub(super) fn options_menu() -> Vec<craft::ui::menu::PanelRow> {
    vec![craft::ui::menu::PanelRow::Command(craft::keymap::CommandRef::new("attachment.associate"))]
}

/// The attachments, keyed by position.
struct Attached(usize);

impl DisplayOrder<usize> for Attached {
    fn len(&self) -> usize {
        self.0
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0).then_some(*key)
    }
}

impl ItemSource<usize> for Attached {}

/// Whether an attachment is a PDF by its name; whether its bytes are is decided where it opens.
fn is_pdf(a: &pdfcraft_render::Attachment) -> bool {
    a.name.to_lowercase().ends_with(".pdf")
}

/// Show the Attachments panel for the active document.
pub(crate) fn show(app: &mut PdfCraftApp, ui: &mut egui::Ui, cx: &mut BodyCx<'_>) -> PanelResult {
    let mut result = PanelResult::default();
    let Some((_, id)) = app.active_ids() else { return result };
    let Some(doc) = app.session.get(id) else { return result };
    let info = &doc.info;
    let attachable = doc.allows_modification() && doc.read_only_reason.is_none();
    if attachable && Button::new(tl!("Attach a file…").to_string()).show(ui).clicked() {
        result.command = Some("edit.attach");
    }
    if info.attachments.is_empty() {
        empty_state(ui, "paperclip", tl!("This document has no attachments."));
        return result;
    }
    let cache_id = cx.id.with("associated-files");
    let associated = {
        let cache = cx.memory.shown::<AssociatedCache>(cache_id);
        let key = (id, doc.edit_generation());
        if cache.key != Some(key) {
            cache.files = Arc::new(doc.associated_files());
            cache.key = Some(key);
        }
        Arc::clone(&cache.files)
    };
    // The associated file entry an attachment is, by its name; the document's own entries come first.
    let associated_as = |a: &pdfcraft_render::Attachment| associated.iter().find(|f| f.name.as_deref() == Some(a.name.as_str()));
    // Only an attachment of the document itself can be deleted here.
    let deletable = |a: &pdfcraft_render::Attachment| match &a.source {
        pdfcraft_render::AttachmentSource::Document { key } if attachable => Some(key.clone()),
        _ => None,
    };
    let detail = |a: &pdfcraft_render::Attachment| {
        let mut parts = vec![a.size.map(human_size).unwrap_or_default()];
        if let pdfcraft_render::AttachmentSource::Annotation { page, .. } = a.source {
            parts.push(crate::i18n::fmt(tl!("on page {label}"), &[("label", info.pages.get(page).map(|p| p.label.as_str()).unwrap_or("?"))]));
        }
        if let Some(f) = associated_as(a) {
            parts.push(associated_detail(f, f.page.and_then(|p| info.pages.get(p)).map_or("?", |p| p.label.as_str())));
        }
        if let Some(d) = &a.description {
            parts.push(d.clone());
        }
        parts.join("  ·  ")
    };
    let take = list_focus(&mut app.panel_focus, crate::RightPanel::Attachments, !doc.info.attachments.is_empty());
    let out = List::<usize>::new(tl!("Attachments").to_string()).focus_if(take).select(SelectMode::Single).natural_height(true).show_items(
        ui,
        &Attached(info.attachments.len()),
        &mut |key, _| {
            let Some(a) = info.attachments.get(*key) else { return ListRow::new("") };
            let mut row = ListRow::new(a.name.clone()).icon(IconSpec::svg(IconId::new("paperclip"))).detail(detail(a)).trailing_part(
                SAVE,
                TrailingFace::Icon(IconSpec::svg(IconId::new("file-down"))),
                tl!("Save attachment…").to_string(),
            );
            if is_pdf(a) {
                row = row.trailing_part(OPEN, TrailingFace::Icon(IconSpec::svg(IconId::new("file-input"))), tl!("Open in a new tab").to_string());
            }
            if deletable(a).is_some() {
                row = row.trailing_part(DELETE, TrailingFace::Icon(IconSpec::svg(IconId::new("trash-2"))), tl!("Delete attachment").to_string());
            }
            row
        },
    );
    // A double click opens a PDF in a new tab, as Acrobat opens an attachment; any other file is saved,
    // since PdfCraft opens only PDFs.
    // The row the open context menu belongs to lives in the panel body's memory, not in egui's temp data.
    let menu_slot = cx.id.with("attachment-menu-row");
    for event in &out.events {
        match event {
            CollectionEvent::Activate(ai) => {
                if let Some(a) = info.attachments.get(*ai) {
                    result.attachment = Some((*ai, is_pdf(a)));
                }
            }
            CollectionEvent::Item(ai, RowEvent::Trailing { part, .. }) => {
                let Some(a) = info.attachments.get(*ai) else { continue };
                if *part == SAVE {
                    result.attachment = Some((*ai, false));
                } else if *part == OPEN {
                    result.attachment = Some((*ai, true));
                } else if *part == DELETE
                    && let Some(key) = deletable(a)
                {
                    result.edit = Some(pdfcraft_engine::Edit::DeleteAttachment { key });
                }
            }
            CollectionEvent::ContextMenu(request) => {
                *cx.memory.shown::<Option<usize>>(menu_slot) = Some(request.key);
            }
            _ => {}
        }
    }
    let row: Option<usize> = *cx.memory.shown::<Option<usize>>(menu_slot);
    let mut describe = None;
    let mut relationship = None;
    if let Some(ai) = row
        && let Some(a) = info.attachments.get(ai)
    {
        let key = deletable(a);
        // Only an entry of the catalog's /AF can be changed here, as the tools af_set_relationship and af_remove do.
        let listed = associated_as(a).filter(|f| f.page.is_none() && attachable).map(|f| (f.index, f.relationship.clone()));
        let mut entries = Vec::new();
        if is_pdf(a) {
            entries.push(Entry::new(MENU_OPEN, tl!("Open in a new tab").to_string()));
        }
        entries.push(Entry::new(MENU_SAVE, tl!("Save attachment…").to_string()));
        if key.is_some() {
            entries.push(Entry::new(MENU_DESCRIBE, tl!("Edit description").to_string()));
            entries.push(Entry::new(MENU_DELETE, tl!("Delete attachment").to_string()));
        }
        if listed.is_some() {
            entries.extend(
                [
                    (MENU_RELATIONSHIP, tl!("Set Relationship…")),
                    (MENU_REMOVE_ASSOCIATED, tl!("Remove from Associated Files and Delete Attachment")),
                    (MENU_REMOVE_ASSOCIATED_KEEP, tl!("Remove from Associated Files, Keep Attachment")),
                ]
                .map(|(id, label)| Entry::new(id, label.to_string())),
            );
        }
        match Menu::context(tl!("Attachment").to_string(), &out.response).pick(ui, &entries).picked {
            Some(MENU_OPEN) => result.attachment = Some((ai, true)),
            Some(MENU_SAVE) => result.attachment = Some((ai, false)),
            Some(MENU_DESCRIBE) => {
                if let Some(key) = key {
                    describe = Some((doc.edit_generation(), key, a.name.clone(), a.description.clone().unwrap_or_default(), a.has_description));
                }
            }
            Some(MENU_DELETE) => {
                if let Some(key) = key {
                    result.edit = Some(pdfcraft_engine::Edit::DeleteAttachment { key });
                }
            }
            Some(picked @ MENU_RELATIONSHIP..=MENU_REMOVE_ASSOCIATED_KEEP) => {
                let (edit, open) = associated_pick(picked, &a.name, listed);
                result.edit = edit;
                relationship = open;
            }
            _ => {}
        }
    }
    // Opened once the panel's borrows of the document end: the dialog is the app's.
    if let Some((generation, key, name, description, has_description)) = describe {
        let ctx = ui.ctx().clone();
        app.open_attachment_description(&ctx, id, generation, key, name, description, has_description);
    }
    if let Some((index, name, current)) = relationship {
        let ctx = ui.ctx().clone();
        crate::dialogs::associated_files::open_relationship(app, &ctx, index, name, current);
    }
    result
}
