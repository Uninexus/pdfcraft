//! Bookmarks panel ▸ Set web address…: the bookmark opens a web or email address instead of going to a page. The
//! address is checked as a link's is; OK waits for one that PdfCraft would offer to open. One undo step. The tool
//! `bookmark_set_url` does the same headlessly.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
use pdfcraft_engine::Edit;
use pdfcraft_render::{LinkTarget, OutlineItem};

use crate::PdfCraftApp;

/// The dialog's key.
pub(crate) const KEY: &str = "bookmark.web_address";
/// The key the dialog's values hold the bookmark's path under (a JSON list); it has no row.
const PATH: &str = "path";

fn item_at<'a>(items: &'a [OutlineItem], path: &[usize]) -> Option<&'a OutlineItem> {
    let (&first, rest) = path.split_first()?;
    let item = items.get(first)?;
    if rest.is_empty() { Some(item) } else { item_at(&item.children, rest) }
}

/// Opens the dialog for the bookmark at `path`, holding the address it opens now, if it opens one.
pub(crate) fn open(app: &mut PdfCraftApp, ctx: &egui::Context, path: Vec<usize>) {
    let Some((_, id)) = app.active_ids() else { return };
    let Some(item) = app.session.get(id).and_then(|d| item_at(&d.info.outline, &path)) else { return };
    let now = match &item.action {
        Some(LinkTarget::Uri(url)) => url.clone(),
        _ => String::new(),
    };
    let line = item.title.clone();
    let Ok(at) = serde_json::to_string(&path) else { return };
    let mut fields = DialogFields::new();
    fields.set(PATH, at);
    fields.set("url", now);
    let dialog = HostedParamDialog::new(ctx, KEY, fields, move |fields| {
        let ready = pdfcraft_engine::links::document_url(fields.text("url").unwrap_or_default()).is_ok();
        let rows = vec![Param::new("url", tl!("Web address"), ParamKind::text(String::new())).hint("https://")];
        ParamDialog::new(KEY, tl!("Set web address"), rows).title_note(line.clone()).ok_enabled(ready).values(Bind::bound(fields))
    });
    crate::dialogs::host::open(app, dialog);
}

/// The dialog was confirmed with `fields`.
pub(crate) fn finish(ctx: &egui::Context, app: &mut PdfCraftApp, fields: &DialogFields) {
    let Some(path) = fields.text(PATH).and_then(|at| serde_json::from_str(at).ok()) else { return };
    let url = fields.text("url").unwrap_or_default().to_owned();
    app.apply_edit(ctx, Edit::SetBookmarkUrl { path, url });
}
