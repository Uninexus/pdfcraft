//! Create Multiple Copies (field menu while preparing a form): a field copied into a grid of rows and columns, each
//! copy a field of its own. The rows, columns, spacing (in the Preferences unit) and an optional name pattern; the
//! engine lays the grid out as the page is shown and adds every copy as one undo step, or refuses the whole grid with
//! the reason (a copy off the page, a repeated or existing name). The tool `form_copy_field_grid` does the same
//! headlessly.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
use pdfcraft_engine::field_copies::{FieldGrid, MAX_COLUMNS, MAX_ROWS};

use crate::PdfCraftApp;
use crate::pageboxes::unit_kind;

/// The dialog's key.
pub(crate) const KEY: &str = "form.copy_field_grid";
/// The key the dialog's values hold the field's name under; it has no row.
const FIELD: &str = "field";
/// The spacing a grid starts with, in points.
const START_GAP: f64 = 6.0;
/// The most spacing a grid takes, in points.
const MAX_GAP: f64 = 1000.0;

/// The rows of the dialog, the spacing in the Preferences unit.
fn params(unit: crate::units::Unit) -> Vec<Param> {
    let k = unit.per_point();
    let decimals = unit.decimals().max(2) as u32;
    let gap = |key: &str, label: &'static str| {
        Param::number(key, label, START_GAP * k, 0.5 * k, decimals).bounds(Some(0.0), Some(MAX_GAP * k)).unit(unit_kind(unit))
    };
    vec![
        Param::new("rows", tl!("Rows"), ParamKind::int(Some(1), Some(MAX_ROWS as i64), 2)),
        Param::new("columns", tl!("Columns"), ParamKind::int(Some(1), Some(MAX_COLUMNS as i64), 2)),
        gap("row_gap", tl!("Space between rows")),
        gap("column_gap", tl!("Space between columns")),
        Param::new("pattern", tl!("Name pattern"), ParamKind::text(String::new()))
            .hint(tl!("Name_1, Name_2, …"))
            .tooltip(tl!("Leave empty to name the copies after the field, with a number.")),
        Param::read_only_text(
            "note",
            "",
            tl!("The field stays where it is as the first cell; the copies fill the rest of the grid row by row. If any copy would run off the page or reuse a name, nothing is copied."),
        )
        .full_width(),
    ]
}

/// Opens Create Multiple Copies for the field `name`.
pub(crate) fn open(app: &mut PdfCraftApp, ctx: &egui::Context, name: String) {
    if app.active_ids().is_none() {
        return;
    }
    let unit = app.units;
    let mut fields = DialogFields::new();
    fields.set(FIELD, name.clone());
    fields.set("rows", 2i64);
    fields.set("columns", 2i64);
    fields.set("row_gap", START_GAP * unit.per_point());
    fields.set("column_gap", START_GAP * unit.per_point());
    fields.set("pattern", String::new());
    let line = crate::i18n::fmt(tl!("Copy \"{name}\" into a grid:"), &[("name", &name)]);
    let dialog = HostedParamDialog::new(ctx, KEY, fields, move |fields| {
        ParamDialog::new(KEY, tl!("Create Multiple Copies"), params(unit)).title_note(line.clone()).values(Bind::bound(fields))
    });
    crate::dialogs::host::open(app, dialog);
}

/// The dialog was confirmed with `fields`: copies the field into the grid, or says why not.
pub(crate) fn finish(ctx: &egui::Context, app: &mut PdfCraftApp, fields: &DialogFields) {
    let Some(name) = fields.text(FIELD).map(str::to_owned) else { return };
    let Some((_, id)) = app.active_ids() else { return };
    let count = |key: &str| fields.int(key).and_then(|v| usize::try_from(v).ok()).unwrap_or(1);
    let per_point = app.units.per_point();
    let gap = |key: &str| fields.number(key).map_or(0.0, |v| v / per_point);
    let grid = FieldGrid {
        rows: count("rows"),
        columns: count("columns"),
        row_gap: gap("row_gap"),
        column_gap: gap("column_gap"),
        pattern: fields.text("pattern").map(str::trim).filter(|p| !p.is_empty()).map(str::to_owned),
    };
    let copies = grid.rows.saturating_mul(grid.columns).saturating_sub(1);
    let Some(doc) = app.session.get(id) else { return };
    let edit = match pdfcraft_engine::field_copies::edit(&name, &grid, &doc.info.pages, &doc.form) {
        Ok(edit) => edit,
        Err(e) => return app.notify_error(e),
    };
    if app.apply_edit(ctx, edit) {
        app.notify_fmt("Made {n} copies of {name}", &[("n", &copies.to_string()), ("name", &name)]);
    }
}
