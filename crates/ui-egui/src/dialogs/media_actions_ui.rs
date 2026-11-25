//! Edit ▸ Media Actions: the screen annotations of the document and what their action holds, listed as the tool
//! `screen_actions` lists them, with a Remove Action button on each sound, movie, rendition or rich-media action that
//! can be taken out. A removal is the engine's `RemoveScreenAction` edit, which the tool applies too: one undo step.
//! Nothing is played and no clip or script is read.

use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
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

/// One screen annotation as the dialog shows it.
#[derive(Clone, Debug)]
struct Row {
    /// Its position in the tool's listing: what the removal takes.
    index: usize,
    object: String,
    /// The action's `/S` name when it is a direct name.
    action: Option<String>,
    /// `"sound"`, `"movie"`, `"rendition"` or `"rich_media_execute"` for the actions a removal takes out.
    kind: Option<&'static str>,
    has_entry: bool,
    has_script: bool,
    has_next: bool,
    has_additional: bool,
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

/// Media Actions, owned by the dialog host.
pub(crate) struct MediaActionsDialog {
    id: DialogId,
    form: NoFields,
    shown: Listing,
    /// The document and edit count `shown` was read at.
    key: Option<(DocId, u64)>,
    /// The position whose Remove Action was pressed; `media_actions_pass` carries it out.
    action: Option<usize>,
}

impl PdfCraftApp {
    /// Opens Media Actions for the active document; the list is read on the first pass.
    pub(crate) fn open_media_actions(&mut self) {
        if self.active_ids().is_none() {
            return;
        }
        let dialog = MediaActionsDialog { id: dialog_id("media-actions"), form: NoFields, shown: Listing::default(), key: None, action: None };
        crate::dialogs::host::open(self, dialog);
    }

    /// The screen annotations of document `id`, and whether it may be changed.
    fn media_action_listing(&self, id: DocId) -> Listing {
        let Some(doc) = self.session.get(id) else { return Listing::default() };
        let read = doc.screen_actions();
        let rows = read
            .items
            .iter()
            .map(|r| Row {
                index: r.index,
                object: format!("{} {}", r.object.num, r.object.generation),
                action: r.action.clone(),
                kind: r.kind,
                has_entry: r.has_entry,
                has_script: r.has_script,
                has_next: r.has_next,
                has_additional: r.has_additional,
                removable: r.removable,
                exact: r.exact,
            })
            .collect();
        let modifiable = doc.read_only_reason.is_none() && doc.allows_modification();
        Listing { rows, count: read.count, truncated: read.truncated, capped: read.capped, modifiable }
    }

    /// Carries out the Remove Action the open dialog was asked for and reads the list again when the document changed.
    pub(crate) fn media_actions_pass(&mut self, ctx: &egui::Context) {
        let Some(asked) = self.dialogs.get_mut::<MediaActionsDialog>().map(|d| d.action.take()) else { return };
        let Some((_, id)) = self.active_ids() else {
            self.dialogs.cancel::<MediaActionsDialog>(ctx);
            return;
        };
        let generation = self.session.get(id).map(|d| d.edit_generation());
        let listed = self.dialogs.get::<MediaActionsDialog>().and_then(|d| d.key);
        // A press made on a list the document has since left is dropped: the position may now name another annotation.
        let current = listed.is_some_and(|(doc, at)| doc == id && Some(at) == generation);
        if let Some(index) = asked.filter(|_| current) {
            let modifiable = self.media_action_listing(id).modifiable;
            if modifiable {
                self.apply_edit(ctx, Edit::RemoveScreenAction(index));
            } else {
                self.notify_tr("This document is read-only, so media actions cannot be changed.");
            }
        }
        let generation = self.session.get(id).map(|d| d.edit_generation());
        let now = generation.map(|g| (id, g));
        if self.dialogs.get::<MediaActionsDialog>().is_some_and(|d| d.key != now) {
            let listing = self.media_action_listing(id);
            if let Some(d) = self.dialogs.get_mut::<MediaActionsDialog>() {
                d.shown = listing;
                d.key = now;
            }
        }
    }
}

/// The line under a row's name: what its action is and what goes with it.
fn detail(row: &Row) -> String {
    let action = match (&row.action, row.has_entry) {
        (Some(name), _) => {
            let cut = if row.exact { String::new() } else { format!(" ({})", tl!("name cut short")) };
            let label = match row.kind {
                Some("sound") => tl!("Sound action").to_string(),
                Some("movie") => tl!("Movie action").to_string(),
                Some("rendition") => tl!("Rendition action").to_string(),
                Some("rich_media_execute") => tl!("RichMediaExecute action").to_string(),
                _ => crate::i18n::fmt(tl!("Other action ({name})"), &[("name", name)]),
            };
            format!("{label}{cut}")
        }
        (None, true) => tl!("An action with no readable name").to_string(),
        (None, false) => tl!("No action").to_string(),
    };
    let mut parts = vec![action];
    if row.has_script {
        parts.push(tl!("has a stored script").to_string());
    }
    if row.has_next {
        parts.push(tl!("has a chain of next actions").to_string());
    }
    if row.has_additional {
        parts.push(tl!("has additional actions").to_string());
    }
    crate::i18n::fmt(tl!("object {object}: {details}"), &[("object", &row.object), ("details", &parts.join(", "))])
}

/// The name a row is announced by.
fn row_name(row: &Row) -> String {
    crate::i18n::fmt(tl!("Screen annotation {object}"), &[("object", &row.object)])
}

impl StackDialog for MediaActionsDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let MediaActionsDialog { form, shown, action, .. } = self;
        let close = FooterButton::new(tl!("Close").to_string(), DialogRole::Cancel);
        let out = FormDialog::new(tl!("Media Actions").to_string(), Bind::bound(form)).shaped(|d| d.buttons([close])).show(ui, |ui, _, _| {
            Text::new(
                tl!("The sound, movie, rendition and rich-media actions of screen annotations. Playing media is not supported: nothing here plays, opens or runs a clip or script. Removing an action takes it out with its chain of next actions and keeps the annotation.")
                    .to_string(),
            )
            .role(TypeRole::Caption)
            .shade(Shade::Muted)
            .show(ui);
            if !shown.modifiable {
                Notice::new(Tone::Warn, tl!("This document does not allow changes, so media actions cannot be removed.").to_string())
                    .layout(NoticeLayout::Inline)
                    .show(ui);
            }
            if shown.rows.is_empty() {
                Text::new(tl!("This document has no screen annotations.").to_string()).show(ui);
            }
            for row in &shown.rows {
                let line = detail(row);
                Field::new(row_name(row)).value(line.clone()).show(ui, |ui, _| {
                    Text::new(line.clone()).show(ui);
                    if !row.removable {
                        Text::new(tl!("This annotation is part of the page tree, so its action cannot be removed here.").to_string())
                            .role(TypeRole::Caption)
                            .shade(Shade::Muted)
                            .show(ui);
                    }
                    if row.removable && row.kind.is_none() {
                        Text::new(tl!("This is not a sound, movie, rendition or rich-media action, so it is left as it is.").to_string())
                            .role(TypeRole::Caption)
                            .shade(Shade::Muted)
                            .show(ui);
                    }
                });
                // Outside the field, so the button keeps its own name: inside, the field names its control after the row.
                if row.removable && row.kind.is_some() && Button::new(tl!("Remove Action").to_string()).enabled(shown.modifiable).id_salt(row.index).show(ui).clicked() {
                    *action = Some(row.index);
                }
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
