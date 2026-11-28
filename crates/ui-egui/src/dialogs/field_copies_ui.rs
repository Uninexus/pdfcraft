//! Create Multiple Copies (field menu while preparing a form): a field copied into a grid of rows and columns, each
//! copy a field of its own. The rows, columns, spacing (in the Preferences unit) and an optional name pattern; the
//! engine lays the grid out as the page is shown and adds every copy as one undo step, or refuses the whole grid with
//! the reason (a copy off the page, a repeated or existing name). While the dialog is open the page shows where the
//! copies would go, or that reason. With several fields selected they are copied as one block, each under its own name
//! (no pattern). The tool `form_copy_field_grid` does the same headlessly.

use craft::ui::contract::Bind;
use craft::ui::dialog::{DialogHost, dialog_id};
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
use pdfcraft_engine::field_copies::{FieldGrid, MAX_COLUMNS, MAX_ROWS};
use pdfcraft_engine::field_table::Cell;
use pdfcraft_engine::{Edit, FieldCopy, FormField};
use pdfcraft_render::PageInfo;

use crate::PdfCraftApp;
use crate::pageboxes::unit_kind;

/// The dialog's key.
pub(crate) const KEY: &str = "form.copy_field_grid";
/// The key the dialog's values hold the fields' names under (a JSON list); it has no row.
const FIELDS: &str = "fields";
/// The spacing a grid starts with, in points.
const START_GAP: f64 = 6.0;
/// The most spacing a grid takes, in points.
const MAX_GAP: f64 = 1000.0;

/// The rows of the dialog, the spacing in the Preferences unit.
fn params(unit: crate::units::Unit, several: bool) -> Vec<Param> {
    let k = unit.per_point();
    let decimals = unit.decimals().max(2) as u32;
    let gap = |key: &str, label: &'static str| {
        Param::number(key, label, START_GAP * k, 0.5 * k, decimals).bounds(Some(0.0), Some(MAX_GAP * k)).unit(unit_kind(unit))
    };
    let mut rows = vec![
        Param::new("rows", tl!("Rows"), ParamKind::int(Some(1), Some(MAX_ROWS as i64), 2)),
        Param::new("columns", tl!("Columns"), ParamKind::int(Some(1), Some(MAX_COLUMNS as i64), 2)),
        gap("row_gap", tl!("Space between rows")),
        gap("column_gap", tl!("Space between columns")),
        Param::new("pattern", tl!("Name pattern"), ParamKind::text(String::new()))
            .hint(tl!("Name_1, Name_2, …"))
            .tooltip(tl!("Leave empty to name the copies after the field, with a number.")),
    ];
    let note = if several {
        // Each field names its own copies.
        rows.pop();
        tl!(
            "The selected fields stay where they are as the first cell; copies of the block fill the rest of the grid row by row, each named after its field. If any copy would run off the page or reuse a name, nothing is copied."
        )
    } else {
        tl!(
            "The field stays where it is as the first cell; the copies fill the rest of the grid row by row. If any copy would run off the page or reuse a name, nothing is copied."
        )
    };
    rows.push(Param::read_only_text("note", "", note).full_width());
    rows
}

/// The names the dialog holds.
fn names_of(fields: &DialogFields) -> Vec<String> {
    fields.text(FIELDS).and_then(|list| serde_json::from_str(list).ok()).unwrap_or_default()
}

/// Opens Create Multiple Copies for the fields `names`.
pub(crate) fn open(app: &mut PdfCraftApp, ctx: &egui::Context, names: Vec<String>) {
    if app.active_ids().is_none() || names.is_empty() {
        return;
    }
    let Ok(list) = serde_json::to_string(&names) else { return };
    let unit = app.units;
    let mut fields = DialogFields::new();
    fields.set(FIELDS, list);
    fields.set("rows", 2i64);
    fields.set("columns", 2i64);
    fields.set("row_gap", START_GAP * unit.per_point());
    fields.set("column_gap", START_GAP * unit.per_point());
    fields.set("pattern", String::new());
    let several = names.len() > 1;
    let line = match names.as_slice() {
        [name] => crate::i18n::fmt(tl!("Copy \"{name}\" into a grid:"), &[("name", name)]),
        _ => crate::i18n::fmt(tl!("Copy {n} fields into a grid, as one block:"), &[("n", &names.len().to_string())]),
    };
    let dialog = HostedParamDialog::new(ctx, KEY, fields, move |fields| {
        ParamDialog::new(KEY, tl!("Create Multiple Copies"), params(unit, several)).title_note(line.clone()).values(Bind::bound(fields))
    });
    crate::dialogs::host::open(app, dialog);
}

/// The grid the dialog's values ask for; the spacing there is in the Preferences unit, `per_point` of it to a point.
fn grid_of(fields: &DialogFields, per_point: f64) -> FieldGrid {
    let count = |key: &str| fields.int(key).and_then(|v| usize::try_from(v).ok()).unwrap_or(1);
    let gap = |key: &str| fields.number(key).map_or(0.0, |v| v / per_point);
    FieldGrid {
        rows: count("rows"),
        columns: count("columns"),
        row_gap: gap("row_gap"),
        column_gap: gap("column_gap"),
        pattern: fields.text("pattern").map(str::trim).filter(|p| !p.is_empty()).map(str::to_owned),
    }
}

/// What the page shows while the dialog is open: where each copy would go, or why none can be made. Rectangles are as
/// the page is shown (from its top-left corner), which is what the page overlay draws in.
pub(crate) struct CopiesPreview {
    /// The page the field is on, 0-based.
    pub page: usize,
    /// The field being copied.
    pub field: [f64; 4],
    pub cells: Result<Vec<Cell>, String>,
}

/// The preview for the open dialog; `pages` and `form` are the active document's, which the modal dialog keeps active.
pub(crate) fn preview(dialogs: &DialogHost, per_point: f64, pages: &[PageInfo], form: &[FormField]) -> Option<CopiesPreview> {
    let dialog = dialogs.get_by_id::<HostedParamDialog>(dialog_id(KEY))?;
    let fields = dialog.fields();
    let names = names_of(fields);
    let widgets: Vec<_> = names.iter().filter_map(|name| form.iter().find(|f| f.name == *name)?.widgets.first()).collect();
    let page = widgets.first()?.page?;
    let info = pages.get(page)?;
    let shown = |[x0, y0, x1, y1]: [f64; 4]| {
        let (a, b) = (info.user_to_view(x0 as f32, y0 as f32), info.user_to_view(x1 as f32, y1 as f32));
        [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])].map(f64::from)
    };
    // The block being copied: the box around the fields.
    let field = widgets.iter().map(|w| shown(w.rect)).reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])?;
    let sources: Vec<&str> = names.iter().map(String::as_str).collect();
    let cells = match pdfcraft_engine::field_copies::edit_block(&sources, &grid_of(fields, per_point), pages, form) {
        Ok(edit) => Ok(copies_of(&edit).into_iter().map(|c| Cell { rect: shown(c.rect), name: c.name.clone() }).collect()),
        Err(e) => Err(e.to_string()),
    };
    Some(CopiesPreview { page, field, cells })
}

/// Every copy an edit of [`pdfcraft_engine::field_copies::edit_block`] makes.
fn copies_of(edit: &Edit) -> Vec<&FieldCopy> {
    match edit {
        Edit::CopyField { copies, .. } => copies.iter().collect(),
        Edit::Batch { edits, .. } => edits.iter().flat_map(copies_of).collect(),
        _ => Vec::new(),
    }
}

/// The dialog was confirmed with `fields`: copies the fields into the grid, or says why not.
pub(crate) fn finish(ctx: &egui::Context, app: &mut PdfCraftApp, fields: &DialogFields) {
    let names = names_of(fields);
    let Some((_, id)) = app.active_ids() else { return };
    let grid = grid_of(fields, app.units.per_point());
    let Some(doc) = app.session.get(id) else { return };
    let sources: Vec<&str> = names.iter().map(String::as_str).collect();
    let edit = match pdfcraft_engine::field_copies::edit_block(&sources, &grid, &doc.info.pages, &doc.form) {
        Ok(edit) => edit,
        Err(e) => return app.notify_error(e),
    };
    let made = copies_of(&edit).len().to_string();
    if app.apply_edit(ctx, edit) {
        match names.as_slice() {
            [name] => app.notify_fmt("Made {n} copies of {name}", &[("n", &made), ("name", name)]),
            _ => app.notify_fmt("Made {n} copies of {count} fields", &[("n", &made), ("count", &names.len().to_string())]),
        }
    }
}
