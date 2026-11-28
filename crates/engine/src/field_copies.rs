//! Prepare a form ▸ Create multiple copies of a field: a field copied into a grid of rows and
//! columns, each copy a field of its own with the same type, flags and look. One undoable edit.
//!
//! The grid is laid out in view space (points from the top-left of the page as displayed, after
//! `/Rotate`) starting at the field itself, which stays where it is as the grid's first cell; the
//! copies fill the rest, row by row. Everything is checked before anything is added, so either all
//! of the copies are made or none. Several fields on one page are copied as one block ([`edit_block`]): the first cell
//! is the box around them, and each keeps its place in it.

use std::collections::HashSet;

use pdfcraft_forms::table::cell_name;
use pdfcraft_forms::{FieldCopy, FieldKind, FormError};
use pdfcraft_render::PageInfo;

use crate::{Edit, EditError, FormField};

/// The edit's name in the history ("Undo Copy field into a grid"), as [`Edit::label`] words it.
pub const LABEL: &str = "Copy field into a grid";
/// The same for several fields at once.
pub const LABEL_FIELDS: &str = "Copy fields into a grid";
/// The most rows a grid can have.
pub const MAX_ROWS: usize = 100;
/// The most columns a grid can have.
pub const MAX_COLUMNS: usize = 50;
/// The most copies one grid can make.
pub const MAX_COPIES: usize = 1000;

/// How a field is copied into a grid.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldGrid {
    /// Rows down, the field's own row included.
    pub rows: usize,
    /// Columns across, the field's own column included.
    pub columns: usize,
    /// Points between two rows.
    pub row_gap: f64,
    /// Points between two columns.
    pub column_gap: f64,
    /// How the copies are named; `None` for the field's name, `_`, and the copy's number (the
    /// field's own cell is 0, so the copies count 1, 2, 3, … along the rows). A pattern is read as
    /// a table column's name pattern is (`item_N`, `{row}`, `{col}`, `{seq}`; see
    /// [`pdfcraft_forms::table::cell_name`]) with the row counted from 1 and `{seq}` as above.
    pub pattern: Option<String>,
}

fn refuse(why: impl Into<String>) -> EditError {
    EditError::Form(FormError::Invalid(why.into()))
}

/// The edit that copies the field `source` of a document whose pages are `pages` and fields are
/// `form` into `grid`. Refused, with nothing copied, for a field that is not there or is a radio
/// button, a grid of no copies or more than [`MAX_COPIES`], spacing that is not a number or is
/// negative, copies that would run off the page, and names that are repeated, hold a period, or
/// are a field's already.
pub fn edit(source: &str, grid: &FieldGrid, pages: &[PageInfo], form: &[FormField]) -> Result<Edit, EditError> {
    edit_block(&[source], grid, pages, form)
}

/// A field to copy: its name and where the page shows it (view space).
struct Source<'a> {
    name: &'a str,
    view: [f64; 4],
}

/// `name`'s page and place, or why it can't be copied.
fn source<'a>(name: &'a str, pages: &[PageInfo], form: &[FormField]) -> Result<(usize, Source<'a>), EditError> {
    let field = form.iter().find(|f| f.name == name).ok_or_else(|| EditError::Form(FormError::NoSuchField(name.into())))?;
    if field.kind == FieldKind::Radio {
        return Err(refuse("a radio button belongs to its group: add another button to the group instead of copying it"));
    }
    let widget = field.widgets.first().ok_or_else(|| refuse(format!("{name} has no widget")))?;
    let page = widget.page.ok_or_else(|| refuse(format!("{name} is not on a page")))?;
    let info = pages.get(page).ok_or_else(|| refuse(format!("page {} does not exist", page + 1)))?;
    // The field as the page shows it.
    let [ux0, uy0, ux1, uy1] = widget.rect;
    let (a, b) = (info.user_to_view(ux0 as f32, uy0 as f32), info.user_to_view(ux1 as f32, uy1 as f32));
    let view = [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])].map(f64::from);
    Ok((page, Source { name, view }))
}

/// [`edit`] for one field or several: one `CopyField` for one, a batch (one undo step) for several. Several
/// must be different fields on one page, and take no name pattern, which would give them all the same names.
pub fn edit_block(names: &[&str], grid: &FieldGrid, pages: &[PageInfo], form: &[FormField]) -> Result<Edit, EditError> {
    let mut sources = Vec::with_capacity(names.len());
    let mut on = None;
    let mut asked: HashSet<&str> = HashSet::new();
    for &name in names {
        if !asked.insert(name) {
            return Err(refuse(format!("{name} is listed twice")));
        }
        let (page, one) = source(name, pages, form)?;
        if on.is_some_and(|p| p != page) {
            return Err(refuse("the fields are on different pages: copy those of one page at a time"));
        }
        on = Some(page);
        sources.push(one);
    }
    let page = on.ok_or_else(|| refuse("no field to copy"))?;
    let info = pages.get(page).ok_or_else(|| refuse(format!("page {} does not exist", page + 1)))?;
    if sources.len() > 1 && grid.pattern.is_some() {
        return Err(refuse("a name pattern names one field's copies: leave it empty for several fields"));
    }
    if !(1..=MAX_ROWS).contains(&grid.rows) || !(1..=MAX_COLUMNS).contains(&grid.columns) {
        return Err(refuse(format!("a grid has 1 to {MAX_ROWS} rows and 1 to {MAX_COLUMNS} columns")));
    }
    // Both counts are small from here on.
    let copies = (grid.rows * grid.columns).saturating_sub(1);
    if copies == 0 {
        return Err(refuse("a grid of one field has no copies: give more than one row or column"));
    }
    if copies.saturating_mul(sources.len()) > MAX_COPIES {
        return Err(refuse(format!("a grid makes at most {MAX_COPIES} copies")));
    }
    if ![grid.row_gap, grid.column_gap].iter().all(|g| g.is_finite() && *g >= 0.0) {
        return Err(refuse("the spacing must be a number, 0 or more"));
    }
    // The block: the box around the fields.
    let block = sources.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, s| {
        [b[0].min(s.view[0]), b[1].min(s.view[1]), b[2].max(s.view[2]), b[3].max(s.view[3])]
    });
    let (width, height) = (block[2] - block[0], block[3] - block[1]);
    let (page_w, page_h) = (f64::from(info.width), f64::from(info.height));
    // "a.b" also takes "a": a new field of that name would sit beside the parent of "a.b".
    let mut taken: HashSet<&str> = HashSet::new();
    for f in form {
        taken.insert(f.name.as_str());
        taken.extend(f.name.match_indices('.').filter_map(|(at, _)| f.name.get(..at)));
    }
    let mut seen: HashSet<String> = HashSet::with_capacity(copies);
    let mut made = Vec::with_capacity(sources.len());
    for one in &sources {
        let source = one.name;
        let mut places = Vec::with_capacity(copies);
        for row in 0..grid.rows {
            for column in 0..grid.columns {
                if row == 0 && column == 0 {
                    continue;
                }
                let seq = row * grid.columns + column;
                // As the form stores it: a name's own spaces at either end are not part of it.
                let name = match &grid.pattern {
                    None => format!("{source}_{seq}"),
                    Some(pattern) => cell_name(pattern, row + 1, column + 1, seq),
                }
                .trim()
                .to_owned();
                if name.is_empty() {
                    return Err(refuse("a copy would have no name"));
                }
                // A field that sits under a parent (a.b) copies to a sibling (a.b_1), and the pattern
                // is not allowed to make one.
                if grid.pattern.is_some() && name.contains('.') {
                    return Err(refuse(format!("field names can't contain a period: {name:?}")));
                }
                if !seen.insert(name.clone()) {
                    return Err(refuse(format!("two copies would both be named {name:?}")));
                }
                if taken.contains(name.as_str()) {
                    return Err(refuse(format!("a field named {name:?} already exists")));
                }
                // Counts of at most 100 and 50, so exact as floats.
                let (dx, dy) = (column as f64 * (width + grid.column_gap), row as f64 * (height + grid.row_gap));
                let view = [one.view[0] + dx, one.view[1] + dy, one.view[2] + dx, one.view[3] + dy];
                if view[0] < -0.01 || view[1] < -0.01 || view[2] > page_w + 0.01 || view[3] > page_h + 0.01 {
                    return Err(refuse("the copies would run off the page: use fewer rows or columns, or less spacing"));
                }
                let rect = info.view_rect_to_user(view.map(|v| v as f32)).map(f64::from);
                if !rect.iter().all(|v| v.is_finite()) {
                    return Err(refuse("the copies are outside the numbers a page can hold"));
                }
                places.push(FieldCopy { page, rect, name });
            }
        }
        made.push((source.to_owned(), places));
    }
    if made.len() == 1
        && let Some((name, copies)) = made.pop()
    {
        return Ok(Edit::CopyField { name, copies });
    }
    // Cell by cell, so the new fields' tab order runs along each copy of the block, as it does along the original.
    let mut queues: Vec<_> = made.into_iter().map(|(name, copies)| (name, copies.into_iter())).collect();
    let mut edits = Vec::new();
    for _ in 0..copies {
        for (name, rest) in &mut queues {
            edits.extend(rest.next().map(|copy| Edit::CopyField { name: name.clone(), copies: vec![copy] }));
        }
    }
    Ok(Edit::Batch { label: LABEL_FIELDS.into(), edits })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NewField, Session};

    fn grid(rows: usize, columns: usize) -> FieldGrid {
        FieldGrid { rows, columns, row_gap: 10.0, column_gap: 20.0, pattern: None }
    }

    fn open_with_field() -> (Session, crate::DocId) {
        let mut s = Session::new();
        let bytes = s.create_blank(300.0, 400.0, 1).unwrap();
        let id = s.open("t.pdf", None, bytes, None).unwrap();
        // 60 × 20 points at (50, 40) from the top-left as displayed.
        let info = s.get(id).unwrap().info.pages[0].clone();
        let rect = info.view_rect_to_user([50.0, 40.0, 110.0, 60.0]).map(f64::from);
        s.apply(id, Edit::AddField { page: 0, rect, kind: NewField::Text { multiline: false }, name: Some("item".into()) }).unwrap();
        (s, id)
    }

    fn plan(s: &Session, id: crate::DocId, grid: &FieldGrid) -> Result<Edit, EditError> {
        let doc = s.get(id).unwrap();
        edit("item", grid, &doc.info.pages, &doc.form)
    }

    #[test]
    fn a_grid_of_copies_is_added_and_undone_as_one_step() {
        let (mut s, id) = open_with_field();
        s.apply(id, plan(&s, id, &grid(2, 3)).unwrap()).unwrap();
        let doc = s.get(id).unwrap();
        assert_eq!(doc.form.len(), 6, "the field and five copies");
        let shown = |name: &str| {
            let r = doc.form.iter().find(|f| f.name == name).unwrap().widgets[0].rect;
            let info = &doc.info.pages[0];
            let (a, b) = (info.user_to_view(r[0] as f32, r[1] as f32), info.user_to_view(r[2] as f32, r[3] as f32));
            [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])]
        };
        // Along the row 80 points apart (60 wide and 20 between), down 30 (20 high and 10 between).
        let near = |name: &str, want: [f32; 4]| {
            let got = shown(name);
            assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() < 0.01), "{name}: {got:?} vs {want:?}");
        };
        near("item", [50.0, 40.0, 110.0, 60.0]);
        near("item_1", [130.0, 40.0, 190.0, 60.0]);
        near("item_2", [210.0, 40.0, 270.0, 60.0]);
        near("item_3", [50.0, 70.0, 110.0, 90.0]);
        near("item_5", [210.0, 70.0, 270.0, 90.0]);
        assert_eq!(s.undo(id).unwrap().to_string(), LABEL);
        assert_eq!(s.get(id).unwrap().form.len(), 1, "one undo takes every copy back");
    }

    #[test]
    fn a_pattern_names_the_copies_and_a_refused_grid_adds_nothing() {
        let (mut s, id) = open_with_field();
        let mut by_pattern = grid(2, 2);
        by_pattern.pattern = Some("cell_r{row}c{col}".into());
        s.apply(id, plan(&s, id, &by_pattern).unwrap()).unwrap();
        let names: Vec<_> = s.get(id).unwrap().form.iter().map(|f| f.name.clone()).collect();
        for n in ["item", "cell_r1c2", "cell_r2c1", "cell_r2c2"] {
            assert!(names.iter().any(|x| x == n), "{n} in {names:?}");
        }
        // The same names again are taken; a period, a repeat, too many, off the page: all refused.
        let why = |g: &FieldGrid| plan(&s, id, g).unwrap_err().to_string();
        assert!(why(&by_pattern).contains("already exists"));
        let mut period = grid(2, 2);
        period.pattern = Some("a.b{seq}".into());
        assert!(why(&period).contains("period"));
        let mut twice = grid(1, 3);
        twice.pattern = Some("same".into());
        assert!(why(&twice).contains("both be named"), "{}", why(&twice));
        assert!(why(&grid(1, 1)).contains("no copies"));
        assert!(why(&grid(0, 3)).contains("1 to 100 rows"));
        assert!(why(&grid(100, 50)).contains("at most 1000"));
        // 60 + 20 across eleven columns is past the 300 points of the page.
        assert!(why(&grid(1, 11)).contains("off the page"));
        let mut negative = grid(2, 2);
        negative.row_gap = -1.0;
        assert!(why(&negative).contains("spacing"));
        let mut nan = grid(2, 2);
        nan.column_gap = f64::NAN;
        assert!(why(&nan).contains("spacing"));
        assert_eq!(s.get(id).unwrap().form.len(), 4, "nothing was added by a refused grid");
    }

    #[test]
    fn a_radio_button_and_a_missing_field_are_refused() {
        let (mut s, id) = open_with_field();
        let info = s.get(id).unwrap().info.pages[0].clone();
        let rect = info.view_rect_to_user([50.0, 200.0, 70.0, 220.0]).map(f64::from);
        s.apply(id, Edit::AddField { page: 0, rect, kind: NewField::Radio { group: Some("size".into()), export: "S".into() }, name: None }).unwrap();
        let doc = s.get(id).unwrap();
        assert!(edit("size", &grid(2, 2), &doc.info.pages, &doc.form).unwrap_err().to_string().contains("radio button"));
        assert!(edit("nope", &grid(2, 2), &doc.info.pages, &doc.form).unwrap_err().to_string().contains("no field named"));
    }

    /// Rows run down a turned page as it is shown, whatever the page's own axes are.
    #[test]
    fn the_grid_runs_down_a_turned_page_as_it_is_shown() {
        let (mut s, id) = open_with_field();
        s.apply(id, Edit::RotatePages { pages: vec![0], degrees: 90 }).unwrap();
        let doc = s.get(id).unwrap();
        let info = doc.info.pages[0].clone();
        s.apply(id, edit("item", &grid(2, 2), &doc.info.pages, &doc.form).unwrap()).unwrap();
        let doc = s.get(id).unwrap();
        let shown = |name: &str| {
            let r = doc.form.iter().find(|f| f.name == name).unwrap().widgets[0].rect;
            let (a, b) = (info.user_to_view(r[0] as f32, r[1] as f32), info.user_to_view(r[2] as f32, r[3] as f32));
            [a[0].min(b[0]), a[1].min(b[1])]
        };
        let origin = shown("item");
        // The field is 20 wide and 60 tall as shown on the turned page: columns step 20 + 20, rows 60 + 10.
        let near = |got: [f32; 2], want: [f32; 2]| got.iter().zip(want).all(|(g, w)| (g - w).abs() < 0.01);
        assert!(near(shown("item_1"), [origin[0] + 40.0, origin[1]]), "the next column is to the right as shown: {:?}", shown("item_1"));
        assert!(near(shown("item_2"), [origin[0], origin[1] + 70.0]), "the next row is below as shown: {:?}", shown("item_2"));
    }

    /// What the copies hold survives a save and reopen: a check box keeps its on state and starts
    /// off, a list box has no selection, and a name with spaces round the pattern is stored, and
    /// reported, trimmed.
    #[test]
    fn copies_survive_a_save_and_reopen() {
        let (mut s, id) = open_with_field();
        let info = s.get(id).unwrap().info.pages[0].clone();
        let at = |view: [f32; 4]| info.view_rect_to_user(view).map(f64::from);
        s.apply(id, Edit::AddField { page: 0, rect: at([50.0, 100.0, 70.0, 120.0]), kind: NewField::CheckBox, name: Some("agree".into()) }).unwrap();
        s.apply(id, Edit::SetFieldValue { name: "agree".into(), value: pdfcraft_forms::FieldValue::Check(true) }).unwrap();
        let options = vec!["a".to_owned(), "b".to_owned()];
        s.apply(
            id,
            Edit::AddField {
                page: 0,
                rect: at([100.0, 100.0, 160.0, 150.0]),
                kind: NewField::List { options, multi: false },
                name: Some("pick".into()),
            },
        )
        .unwrap();
        s.apply(id, Edit::SetFieldValue { name: "pick".into(), value: pdfcraft_forms::FieldValue::Choice(vec!["b".into()]) }).unwrap();
        let spaced = FieldGrid { rows: 1, columns: 2, row_gap: 0.0, column_gap: 10.0, pattern: Some(" copy {seq} ".into()) };
        for source in ["agree", "pick"] {
            let doc = s.get(id).unwrap();
            let Edit::CopyField { copies, .. } = edit(source, &spaced, &doc.info.pages, &doc.form).unwrap() else { panic!("a copy edit") };
            assert_eq!(copies.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["copy 1"], "trimmed in the edit, so reported as stored");
            let mut again = spaced.clone();
            again.pattern = Some(format!(" {source}_{{seq}} "));
            s.apply(id, plan_for(&s, id, source, &again).unwrap()).unwrap();
        }
        let bytes = s.save_bytes(id).unwrap();
        let mut reopened = Session::new();
        let id2 = reopened.open("t.pdf", None, bytes, None).unwrap();
        let form = &reopened.get(id2).unwrap().form;
        let get = |name: &str| {
            form.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("{name} in {:?}", form.iter().map(|f| &f.name).collect::<Vec<_>>()))
        };
        assert_eq!(get("agree").widgets[0].state.as_deref(), Some("Yes"), "the source stays on");
        let copy = get("agree_1");
        assert_eq!((copy.widgets[0].on_state.as_deref(), copy.widgets[0].state.as_deref()), (Some("Yes"), Some("Off")));
        assert!(copy.value.is_empty(), "{:?}", copy.value);
        assert!(!get("pick").selected.is_empty());
        assert!(get("pick_1").selected.is_empty() && get("pick_1").value.is_empty());
        assert_eq!(get("pick_1").options.len(), 2);
    }

    fn plan_for(s: &Session, id: crate::DocId, source: &str, grid: &FieldGrid) -> Result<Edit, EditError> {
        let doc = s.get(id).unwrap();
        edit(source, grid, &doc.info.pages, &doc.form)
    }

    #[test]
    fn several_fields_are_copied_as_one_block_and_undone_as_one_step() {
        let (mut s, id) = open_with_field();
        let info = s.get(id).unwrap().info.pages[0].clone();
        // A second field beside the first: the block is 50..170 across and 40..60 down.
        let rect = info.view_rect_to_user([130.0, 40.0, 170.0, 60.0]).map(f64::from);
        s.apply(id, Edit::AddField { page: 0, rect, kind: NewField::Text { multiline: false }, name: Some("qty".into()) }).unwrap();
        let doc = s.get(id).unwrap();
        let down = FieldGrid { rows: 3, columns: 1, row_gap: 10.0, column_gap: 0.0, pattern: None };
        let mut named = down.clone();
        named.pattern = Some("n{seq}".into());
        assert!(edit_block(&["item", "qty"], &named, &doc.info.pages, &doc.form).unwrap_err().to_string().contains("pattern"));
        assert!(edit_block(&["item", "item"], &down, &doc.info.pages, &doc.form).unwrap_err().to_string().contains("twice"));
        let copies = edit_block(&["item", "qty"], &down, &doc.info.pages, &doc.form).unwrap();
        s.apply(id, copies).unwrap();
        let doc = s.get(id).unwrap();
        let order: Vec<_> = doc.form.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(order, ["item", "qty", "item_1", "qty_1", "item_2", "qty_2"], "row by row");
        let shown = |name: &str| {
            let r = doc.form.iter().find(|f| f.name == name).unwrap().widgets[0].rect;
            let info = &doc.info.pages[0];
            let (a, b) = (info.user_to_view(r[0] as f32, r[1] as f32), info.user_to_view(r[2] as f32, r[3] as f32));
            [a[0].min(b[0]), a[1].min(b[1])]
        };
        for (name, want) in [("item_1", [50.0, 70.0]), ("qty_1", [130.0, 70.0]), ("item_2", [50.0, 100.0]), ("qty_2", [130.0, 100.0])] {
            let got = shown(name);
            assert!((got[0] - want[0]).abs() < 0.01 && (got[1] - want[1]).abs() < 0.01, "{name}: {got:?}");
        }
        assert_eq!(s.undo(id).unwrap().to_string(), LABEL_FIELDS);
        assert_eq!(s.get(id).unwrap().form.len(), 2, "one undo takes every copy back");
    }
}
