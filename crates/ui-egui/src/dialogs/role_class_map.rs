//! Role Map and Class Map (the Tags panel's options menu): the structure tree's `/RoleMap`, which says what standard
//! type each type a file made up stands for, and its `/ClassMap`. Role Map lists every role with the name it maps to
//! (type another name to change it, empty to remove it) and has rows to add roles; Class Map lists every class with
//! a check box to remove it. OK writes every change as one undo step, through the same edits the tools `role_map` and
//! `class_map` apply; a change the engine refuses writes nothing, and the dialog opens again with the typing and the
//! reason.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
use pdfcraft_engine::Edit;

use crate::PdfCraftApp;

/// The Role Map dialog's key.
pub(crate) const ROLE_KEY: &str = "a11y.role_map";
/// The Class Map dialog's key.
pub(crate) const CLASS_KEY: &str = "a11y.class_map";
/// How many new roles one Role Map pass can add.
const ADD_ROWS: usize = 3;

/// The Tags panel's options menu rows.
pub(crate) fn options_menu() -> Vec<craft::ui::menu::PanelRow> {
    [ROLE_KEY, CLASS_KEY].into_iter().map(|id| craft::ui::menu::PanelRow::Command(craft::keymap::CommandRef::new(id))).collect()
}

/// One role of the dialog: the role, what it maps to (none when the value is not a name) and whether the name is shown whole.
#[derive(Clone)]
struct RoleRow {
    role: String,
    maps_to: Option<String>,
    exact: bool,
}

/// One class of the dialog: its name, what its value is, how many attribute objects it holds and whether it can be removed.
#[derive(Clone)]
struct ClassRow {
    class: String,
    kind: &'static str,
    items: usize,
    removable: bool,
}

/// What the active document's structure tree offers a dialog.
enum Gate<T> {
    /// No document is open.
    Nothing,
    /// The document is read-only.
    ReadOnly,
    /// The document has no structure tree.
    NoTree,
    /// The tree root or the map cannot be read as one.
    Unreadable,
    /// The rows of the map.
    Rows(T),
}

/// Posts the notice a closed gate gives and returns the rows of an open one.
fn rows_or_notice<T>(app: &mut PdfCraftApp, gate: Gate<T>) -> Option<T> {
    match gate {
        Gate::Nothing => {}
        Gate::ReadOnly => app.notify_tr("This document is read-only, so its structure tree cannot be changed."),
        Gate::NoTree => app.notify_tr("This document has no structure tree, so there is no role map or class map. Tag the document first."),
        Gate::Unreadable => app.notify_tr("The structure tree's role map or class map cannot be read as one, so it was left as it is."),
        Gate::Rows(rows) => return Some(rows),
    }
    None
}

/// The role map of the active document, or why not.
fn role_rows(app: &PdfCraftApp) -> Gate<Vec<RoleRow>> {
    let Some(doc) = app.active_ids().and_then(|(_, id)| app.session.get(id)) else { return Gate::Nothing };
    if doc.read_only_reason.is_some() || !doc.allows_modification() {
        return Gate::ReadOnly;
    }
    let map = doc.role_map();
    if !map.has_structure_tree {
        Gate::NoTree
    } else if !map.unreadable.is_empty() {
        Gate::Unreadable
    } else {
        Gate::Rows(map.entries.iter().map(|e| RoleRow { role: e.role.clone(), maps_to: e.maps_to.clone(), exact: e.exact }).collect())
    }
}

/// The class map of the active document, or why not.
fn class_rows(app: &PdfCraftApp) -> Gate<Vec<ClassRow>> {
    let Some(doc) = app.active_ids().and_then(|(_, id)| app.session.get(id)) else { return Gate::Nothing };
    if doc.read_only_reason.is_some() || !doc.allows_modification() {
        return Gate::ReadOnly;
    }
    let map = doc.class_map();
    if !map.has_structure_tree {
        Gate::NoTree
    } else if !map.unreadable.is_empty() {
        Gate::Unreadable
    } else {
        Gate::Rows(
            map.entries
                .iter()
                .map(|e| ClassRow { class: e.class.clone(), kind: e.kind, items: e.items, removable: doc.class_map_removes(&e.class) })
                .collect(),
        )
    }
}

/// Opens Role Map for the active document.
pub(crate) fn open_role_map(app: &mut PdfCraftApp, ctx: &egui::Context) {
    let gate = role_rows(app);
    let Some(rows) = rows_or_notice(app, gate) else { return };
    let mut fields = DialogFields::new();
    for (i, row) in rows.iter().enumerate() {
        fields.set(format!("to.{i}"), row.maps_to.clone().unwrap_or_default());
    }
    for k in 0..ADD_ROWS {
        fields.set(format!("new_role.{k}"), String::new());
        fields.set(format!("new_to.{k}"), String::new());
    }
    show_role_map(app, ctx, fields, rows, None);
}

/// Shows the Role Map dialog over `rows` with `fields`, and `note` (why the last try was refused) over the rows.
fn show_role_map(app: &mut PdfCraftApp, ctx: &egui::Context, fields: DialogFields, rows: Vec<RoleRow>, note: Option<String>) {
    let dialog = HostedParamDialog::new(ctx, ROLE_KEY, fields, move |fields| {
        let mut params = vec![Param::section(tl!("Custom roles"))];
        if rows.is_empty() {
            params.push(Param::read_only_text("none", "", tl!("No role is mapped yet.")).full_width());
        }
        for (i, row) in rows.iter().enumerate() {
            let key = format!("to.{i}");
            match (&row.maps_to, row.exact) {
                (Some(_), true) => params.push(
                    Param::new(key, row.role.clone(), ParamKind::text(String::new()))
                        .hint(tl!("Maps to (empty removes the role)"))
                        .tooltip(tl!("The standard type this role stands for. Leave it empty to remove the role.")),
                ),
                (Some(to), false) => params.push(Param::read_only_text(key, row.role.clone(), to.clone())),
                (None, _) => params.push(Param::read_only_text(key, row.role.clone(), tl!("(not a name)"))),
            }
        }
        params.push(Param::section(tl!("Add a role")));
        for k in 0..ADD_ROWS {
            let n = (k + 1).to_string();
            params.push(
                Param::new(format!("new_role.{k}"), crate::i18n::fmt(tl!("New role {n}"), &[("n", &n)]), ParamKind::text(String::new()))
                    .hint(tl!("Role name")),
            );
            params.push(
                Param::new(format!("new_to.{k}"), crate::i18n::fmt(tl!("New role {n} maps to"), &[("n", &n)]), ParamKind::text(String::new()))
                    .hint(tl!("Maps to")),
            );
        }
        let line = match &note {
            Some(why) => why.clone(),
            None => tl!("Names are 1 to 127 bytes. A role cannot map to itself or back to itself through other roles.").to_string(),
        };
        ParamDialog::new(ROLE_KEY, tl!("Role Map"), params).title_note(line).values(Bind::bound(fields))
    });
    crate::dialogs::host::open(app, dialog);
}

/// Role Map was confirmed with `fields`: writes every changed or added mapping as one undo step, or says why not and
/// opens the dialog again with what was typed.
pub(crate) fn finish_role_map(ctx: &egui::Context, app: &mut PdfCraftApp, fields: &DialogFields) {
    let gate = role_rows(app);
    let Some(rows) = rows_or_notice(app, gate) else { return };
    let mut edits = Vec::new();
    for (i, entry) in rows.iter().enumerate() {
        if !entry.exact || entry.maps_to.is_none() {
            continue;
        }
        let typed = fields.text(&format!("to.{i}")).map_or("", str::trim);
        let maps_to = (!typed.is_empty()).then(|| typed.to_owned());
        if maps_to != entry.maps_to {
            edits.push(Edit::SetRoleMap { role: entry.role.clone(), maps_to });
        }
    }
    let mut refused = None;
    for k in 0..ADD_ROWS {
        let role = fields.text(&format!("new_role.{k}")).map_or("", str::trim);
        let to = fields.text(&format!("new_to.{k}")).map_or("", str::trim);
        if role.is_empty() && to.is_empty() {
            continue;
        }
        if role.is_empty() || to.is_empty() {
            let why = tl!("A new role needs both its name and the name it maps to.").to_string();
            app.notify(why.clone());
            refused = Some(why);
            break;
        }
        edits.push(Edit::SetRoleMap { role: role.to_owned(), maps_to: Some(to.to_owned()) });
    }
    if refused.is_none() && !edits.is_empty() {
        let edit = if edits.len() == 1 { edits.remove(0) } else { Edit::Batch { label: tl!("Change role map").to_string(), edits } };
        if app.apply_edit(ctx, edit) {
            return;
        }
        refused = app.last_edit_refusal.clone();
    }
    // An edit the engine refused has already posted its reason (the history's notice); it is also shown in the dialog.
    if let Some(why) = refused {
        show_role_map(app, ctx, fields.clone(), rows, Some(why));
    }
}

/// Opens Class Map for the active document.
pub(crate) fn open_class_map(app: &mut PdfCraftApp, ctx: &egui::Context) {
    let gate = class_rows(app);
    let Some(rows) = rows_or_notice(app, gate) else { return };
    let mut fields = DialogFields::new();
    for i in 0..rows.len() {
        fields.set(format!("rm.{i}"), false);
    }
    let dialog = HostedParamDialog::new(ctx, CLASS_KEY, fields, move |fields| {
        let mut params = vec![Param::section(tl!("Classes"))];
        if rows.is_empty() {
            params.push(Param::read_only_text("none", "", tl!("The class map is empty.")).full_width());
        }
        for (i, row) in rows.iter().enumerate() {
            let n = row.items.to_string();
            let label = crate::i18n::fmt(tl!("{class}: {kind}, {n} attribute objects"), &[("class", &row.class), ("kind", row.kind), ("n", &n)]);
            if row.removable {
                params.push(Param::new(format!("rm.{i}"), label, ParamKind::bool(false)).tooltip(tl!("Check to remove this class.")));
            } else {
                params.push(Param::read_only_text(format!("rm.{i}"), label, tl!("(cannot be removed)")));
            }
        }
        let line = tl!("Check the classes to remove. The attribute objects they point at stay in the file.").to_string();
        ParamDialog::new(CLASS_KEY, tl!("Class Map"), params).title_note(line).values(Bind::bound(fields))
    });
    crate::dialogs::host::open(app, dialog);
}

/// Class Map was confirmed with `fields`: removes the checked classes as one undo step.
pub(crate) fn finish_class_map(ctx: &egui::Context, app: &mut PdfCraftApp, fields: &DialogFields) {
    let gate = class_rows(app);
    let Some(rows) = rows_or_notice(app, gate) else { return };
    let mut edits: Vec<Edit> = rows
        .iter()
        .enumerate()
        .filter(|(i, row)| row.removable && fields.bool(&format!("rm.{i}")) == Some(true))
        .map(|(_, row)| Edit::RemoveClassMapEntry(row.class.clone()))
        .collect();
    let edit = match edits.len() {
        0 => return,
        1 => edits.remove(0),
        _ => Edit::Batch { label: tl!("Remove class map entries").to_string(), edits },
    };
    app.apply_edit(ctx, edit);
}
