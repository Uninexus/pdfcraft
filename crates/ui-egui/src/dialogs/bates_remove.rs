//! Edit ▸ Bates numbering ▸ Remove Bates numbering from pages: a page range, then the same removal the menu's plain
//! command makes on every page, on those pages only (`doc_bates_remove` with `pages`).

use craft::pages::ui::{PageRangeChoices, PageRangeOptions};
use craft::pages::{PageRange, PageScope};
use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, ParamDialog};
use pdfcraft_engine::{Edit, MarkKind};

use crate::PdfCraftApp;

/// The dialog's id, and the name its answer is read by.
pub(crate) const KEY: &str = "edit.bates_remove_pages";

/// Opens the page range, on all pages.
pub(crate) fn open(app: &mut PdfCraftApp, ctx: &egui::Context) {
    let Some((index, id)) = app.active_ids() else { return };
    let Some(doc) = app.session.get(id) else { return };
    let count = doc.info.pages.len();
    let current = app.views.get(index).map_or(0, |view| view.current());
    let range = PageRange { scope: PageScope::All, ..PageRange::default() };
    let fields = DialogFields::from_pairs([("pages", PageRangeOptions::field_value(&range))]);
    let dialog = HostedParamDialog::new(ctx, KEY, fields, move |fields| {
        let rows = vec![PageRangeOptions::param("pages", "", count, current, PageRangeChoices::CurrentFromTo)];
        ParamDialog::new(KEY, tl!("Remove Bates numbering"), rows).values(Bind::bound(fields))
    });
    crate::dialogs::host::open(app, dialog);
}

/// The range was confirmed: the numbers on those pages go, in one undo step. A range with none of them says so.
pub(crate) fn finish(ctx: &egui::Context, app: &mut PdfCraftApp, fields: &DialogFields) {
    let Some(range) = PageRangeOptions::value(fields, "pages") else { return };
    let Some((index, id)) = app.active_ids() else { return };
    let Some(doc) = app.session.get(id) else { return };
    let current = app.views.get(index).map_or(0, |view| view.current());
    let pages = range.pages(doc.info.pages.len(), current, &[]);
    app.apply_edit(ctx, Edit::RemoveMarks { kind: MarkKind::Bates, pages: Some(pages) });
}
