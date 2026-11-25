//! Edit all JavaScripts: every script of the active document listed in one window, with a text
//! filter and a kind filter, and a code editor for the selected one. Apply and Delete are the
//! engine's undoable `SetJavaScript` and `RemoveJavaScript` edits, each carrying the fingerprint of
//! the code the list showed, so an edit made from a stale list is refused and the list is read again.
//!
//! Scripts are untrusted document content: this window only reads and rewrites them. It never
//! runs one. It is a window beside the dialog host, like Print readiness, so it reads the app
//! directly and closes with the document it lists.

use craft::behavior::{Selection, keyed};
use craft::theme::variant::Tone;
use craft::ui::contract::{Bind, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::Dialog;
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::list::{List, ListRow};
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::pickers::Select;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::style::TypeRole;
use craft::ui::text::TextField;
use pdfcraft_engine::{DocId, Edit, JavaScriptEntry, JavaScriptInventory, JavaScriptKind};

use crate::PdfCraftApp;

/// Most code shown in the editor, in bytes. A longer script is cut here and cannot be edited, so
/// the editor never lays out (or hands back) a megabyte of text.
pub(crate) const SHOWN_LIMIT: usize = 64 * 1024;
/// Most characters of a script's first line shown in the list.
const FIRST_LINE_CHARS: usize = 80;
/// Most characters of a name, field name or title from the file shown anywhere in the window.
const NAME_CHARS: usize = 200;
/// Most bytes of a script's code the filter looks at (searched in lower case, kept per row).
const SEARCH_CODE: usize = 4096;
/// Longest filter text, in characters.
pub(crate) const FILTER_CHARS: usize = 200;
/// Rows of the list shown before it scrolls.
const LIST_ROWS: u16 = 8;
/// Lines the editor grows to before its text scrolls inside it.
const EDITOR_LINES: usize = 16;

/// What the user was doing when unsaved typing stood in the way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Leave {
    /// Open another script (by id).
    Select(String),
    /// Close the window.
    Close,
}

/// One list row, made once per listing.
struct ScriptRow {
    label: String,
    /// The label and the start of the code, in lower case, for the text filter.
    haystack: String,
}

/// How much of the selected script the editor holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cut {
    /// All of it.
    No,
    /// The stored script was longer than the engine lists (`truncated`).
    Listing,
    /// The script is longer than [`SHOWN_LIMIT`]; this many bytes are stored in the listing.
    Shown(usize),
}

/// The script the editor was filled from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Loaded {
    pub id: String,
    /// The listed fingerprint: what an edit must name as `expect`.
    pub fingerprint: String,
    /// What the editor held when it was filled.
    pub original: String,
    /// The action has code to read (otherwise Apply writes some).
    pub readable: bool,
    pub cut: Cut,
}

#[derive(Default)]
pub(crate) struct Draft {
    pub(crate) filter: String,
    pub(crate) kind: Option<JavaScriptKind>,
    inventory: JavaScriptInventory,
    /// One per entry of `inventory`.
    rows: Vec<ScriptRow>,
    /// The document and edit generation the inventory was read at.
    listed: Option<(DocId, u64)>,
    /// Indices into the inventory that the filters let through.
    visible: Vec<usize>,
    /// The filters `visible` was made with.
    visible_for: Option<(String, Option<JavaScriptKind>)>,
    pub(crate) loaded: Option<Loaded>,
    pub(crate) text: String,
    pub(crate) notice: Option<String>,
    pub(crate) confirm_delete: bool,
    /// Asked to leave this script (or close) with unsaved typing: waiting for Discard or Keep editing.
    pub(crate) leaving: Option<Leave>,
    /// Counts the times the editor was filled again, so its field starts from the new text.
    epoch: usize,
}

/// Why the window can't change anything, if it can't.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Locked {
    /// The editing parser did not open the file.
    Unreadable,
    /// The document's security settings forbid changing it.
    Permissions,
}

fn locked(doc: &pdfcraft_engine::Document) -> Option<Locked> {
    if !doc.editable() {
        Some(Locked::Unreadable)
    } else if !doc.allows_modification() {
        Some(Locked::Permissions)
    } else {
        None
    }
}

/// `s` cut at most `limit` bytes long, on a character boundary.
fn cut_str(s: &str, limit: usize) -> &str {
    if s.len() <= limit {
        return s;
    }
    let mut end = limit;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or("")
}

/// `s` cut to `NAME_CHARS` characters, with an ellipsis when cut: names come from the file.
fn short(s: &str) -> String {
    let mut out: String = s.chars().take(NAME_CHARS).collect();
    if out.len() < s.len() {
        out.push('…');
    }
    out
}

fn kib(bytes: usize) -> String {
    format!("{}", bytes.div_ceil(1024))
}

fn kind_label(kind: JavaScriptKind) -> String {
    match kind {
        JavaScriptKind::Document => tl!("Document script"),
        JavaScriptKind::OpenAction => tl!("Open action"),
        JavaScriptKind::DocumentAction => tl!("Document action"),
        JavaScriptKind::Field => tl!("Field"),
        JavaScriptKind::Link => tl!("Link"),
        JavaScriptKind::Annotation => tl!("Annotation"),
        JavaScriptKind::Page => tl!("Page"),
        JavaScriptKind::Bookmark => tl!("Bookmark"),
    }
    .to_string()
}

/// Where the script lives, for the list and the editor's heading.
fn location(e: &JavaScriptEntry) -> String {
    let page = e.page.map_or_else(|| "?".to_string(), |p| p.to_string());
    let fill =
        |template: &str, name: Option<&String>| crate::i18n::fmt(template, &[("name", &name.map_or_else(String::new, |n| short(n))), ("n", &page)]);
    match e.kind {
        JavaScriptKind::Document => fill(tl!("Named script \"{name}\""), e.name.as_ref()),
        JavaScriptKind::OpenAction => tl!("The document's open action").to_string(),
        JavaScriptKind::DocumentAction => tl!("The document").to_string(),
        JavaScriptKind::Field if e.page.is_some() => fill(tl!("Field \"{name}\", page {n}"), e.field.as_ref().or(e.name.as_ref())),
        JavaScriptKind::Field => fill(tl!("Field \"{name}\""), e.field.as_ref().or(e.name.as_ref())),
        JavaScriptKind::Link => fill(tl!("Link on page {n}"), None),
        JavaScriptKind::Annotation => fill(tl!("Annotation on page {n}"), None),
        JavaScriptKind::Page => fill(tl!("Page {n}"), None),
        JavaScriptKind::Bookmark => fill(tl!("Bookmark \"{name}\""), e.name.as_ref()),
    }
}

/// The first non-blank line of the code, cut short.
fn first_line(code: Option<&str>) -> String {
    let line = code.and_then(|c| c.lines().map(str::trim).find(|l| !l.is_empty())).unwrap_or("");
    let mut out: String = line.chars().take(FIRST_LINE_CHARS).collect();
    if out.len() < line.len() {
        out.push('…');
    }
    out
}

/// One list row: kind, location, trigger, the code's first line and any marker.
fn row_label(e: &JavaScriptEntry) -> String {
    let mut s = format!("{}  ·  {}  ·  {}", kind_label(e.kind), location(e), short(&e.trigger));
    let line = first_line(e.code.as_deref());
    if !line.is_empty() {
        s.push_str("  ·  ");
        s.push_str(&line);
    }
    if e.truncated {
        s.push_str("  ");
        s.push_str(tl!("(cut)"));
    } else if e.code.is_none() {
        s.push_str("  ");
        s.push_str(tl!("(no readable code)"));
    }
    s
}

fn make_row(e: &JavaScriptEntry) -> ScriptRow {
    let label = row_label(e);
    let mut haystack = label.to_lowercase();
    if let Some(code) = e.code.as_deref() {
        haystack.push('\n');
        haystack.push_str(&cut_str(code, SEARCH_CODE).to_lowercase());
    }
    ScriptRow { label, haystack }
}

impl Draft {
    pub(crate) fn dirty(&self) -> bool {
        self.loaded.as_ref().is_some_and(|l| l.original != self.text)
    }

    pub(crate) fn total(&self) -> usize {
        self.inventory.entries.len()
    }

    pub(crate) fn shown(&self) -> usize {
        self.visible.len()
    }

    pub(crate) fn capped(&self) -> bool {
        self.inventory.capped
    }

    pub(crate) fn selected_entry(&self) -> Option<&JavaScriptEntry> {
        let l = self.loaded.as_ref()?;
        self.inventory.entries.iter().find(|e| e.id == l.id)
    }

    /// The editor can change the script: the document allows it and nothing is cut.
    pub(crate) fn can_edit(&self, locked: bool) -> bool {
        !locked && self.loaded.as_ref().is_some_and(|l| l.cut == Cut::No)
    }

    fn deselect(&mut self) {
        self.loaded = None;
        self.text.clear();
        self.confirm_delete = false;
        self.leaving = None;
        self.epoch = self.epoch.wrapping_add(1);
    }

    /// Set the text filter (at most [`FILTER_CHARS`] characters).
    pub(crate) fn set_filter(&mut self, text: &str) {
        self.filter = text.chars().take(FILTER_CHARS).collect();
        self.refresh_visible();
    }

    pub(crate) fn set_kind(&mut self, kind: Option<JavaScriptKind>) {
        self.kind = kind;
        self.refresh_visible();
    }

    /// Fill the editor from entry `i` of the listing.
    pub(crate) fn select(&mut self, i: usize) {
        let Some(e) = self.inventory.entries.get(i) else { return };
        let code = e.code.as_deref().unwrap_or("");
        let shown = cut_str(code, SHOWN_LIMIT);
        let cut = if e.truncated {
            Cut::Listing
        } else if shown.len() < code.len() {
            Cut::Shown(code.len())
        } else {
            Cut::No
        };
        self.text = shown.to_string();
        self.loaded =
            Some(Loaded { id: e.id.clone(), fingerprint: e.fingerprint.clone(), original: self.text.clone(), readable: e.code.is_some(), cut });
        self.confirm_delete = false;
        self.leaving = None;
        self.notice = None;
        self.epoch = self.epoch.wrapping_add(1);
    }

    /// Select the script with this id, if listed.
    pub(crate) fn select_id(&mut self, id: &str) -> bool {
        match self.inventory.entries.iter().position(|e| e.id == id) {
            Some(i) => {
                self.select(i);
                true
            }
            None => false,
        }
    }

    /// Take a fresh listing of `doc` at `generation`. The selected script is looked up again by
    /// id: if it is gone or its code is another one now (an edit, undo or redo moved things), the
    /// selection follows what the list holds and the window says so. `quiet` after this window's
    /// own edit, `failed` after one the engine refused.
    fn relisted(&mut self, doc: DocId, generation: u64, inventory: JavaScriptInventory, quiet: bool, failed: bool) {
        if self.listed.is_some_and(|(d, _)| d != doc) {
            self.deselect();
        }
        self.rows = inventory.entries.iter().map(make_row).collect();
        self.inventory = inventory;
        self.listed = Some((doc, generation));
        self.follow_selection(quiet, failed);
        self.visible_for = None;
        self.refresh_visible();
    }

    /// Look the selected script up again after a new listing.
    fn follow_selection(&mut self, quiet: bool, failed: bool) {
        let Some((id, fingerprint)) = self.loaded.as_ref().map(|l| (l.id.clone(), l.fingerprint.clone())) else { return };
        match self.inventory.entries.iter().position(|e| e.id == id) {
            None => {
                self.deselect();
                if !quiet {
                    self.notice = Some(
                        if failed {
                            tl!("That script is no longer in the document, so nothing was changed. The list was read again.")
                        } else {
                            tl!("The selected script is no longer in the document (an edit, undo or redo removed it), so the list was read again.")
                        }
                        .to_string(),
                    );
                }
            }
            Some(i) if self.inventory.entries.get(i).is_some_and(|e| e.fingerprint != fingerprint) => {
                let lost = self.dirty();
                let typed = std::mem::take(&mut self.text);
                self.select(i);
                if failed && lost && self.can_edit(false) {
                    // Nothing was applied, and the typing is not thrown away: it stays in the editor,
                    // and Apply (now against the current code) writes it over what the script holds.
                    self.text = typed;
                    self.notice = Some(
                        tl!("That script had changed since the list was made, so nothing was changed. Your text is kept; Apply writes it over the script's current code.")
                            .to_string(),
                    );
                } else if !quiet {
                    let base = if failed {
                        tl!(
                            "That script had changed since the list was made, so nothing was changed. The list was read again and the editor shows its current code."
                        )
                    } else {
                        tl!("The selected script was changed by another edit, undo or redo; the editor shows its current code.")
                    };
                    self.notice = Some(if lost { format!("{base} {}", tl!("Your unsaved changes to it were dropped.")) } else { base.to_string() });
                }
            }
            Some(_) => {}
        }
    }

    /// Recompute the rows the filters let through, when they or the listing changed.
    fn refresh_visible(&mut self) {
        let key = (self.filter.clone(), self.kind);
        if self.visible_for.as_ref() == Some(&key) {
            return;
        }
        let query = self.filter.trim().to_lowercase();
        self.visible = self
            .inventory
            .entries
            .iter()
            .zip(&self.rows)
            .enumerate()
            .filter(|(_, (e, row))| self.kind.is_none_or(|k| k == e.kind) && (query.is_empty() || row.haystack.contains(&query)))
            .map(|(i, _)| i)
            .collect();
        self.visible_for = Some(key);
    }

    /// The window's state for the control channel.
    pub(crate) fn state(&self, open: bool, locked: bool) -> serde_json::Value {
        use serde_json::json;
        let selected = self.selected_entry().map(|e| {
            json!({
                "id": e.id, "kind": e.kind.id(), "trigger": short(&e.trigger), "location": location(e),
                "truncated": e.truncated, "has_code": e.code.is_some(), "code_bytes": e.code.as_ref().map_or(0, String::len),
                "fingerprint": e.fingerprint,
            })
        });
        json!({
            "open": open,
            "total": self.total(),
            "shown": self.shown(),
            "capped": self.capped(),
            "truncated": self.inventory.truncated,
            "names_left_out": self.inventory.names_left_out,
            "filter": self.filter,
            "kind": self.kind.map(JavaScriptKind::id),
            "selected": selected,
            "editor_bytes": self.text.len(),
            "editor_cut": self.loaded.as_ref().is_some_and(|l| l.cut != Cut::No),
            "dirty": self.dirty(),
            "read_only": locked,
            "can_apply": open && self.can_edit(locked) && self.dirty(),
            "confirm_delete": self.confirm_delete,
            "confirm_discard": self.leaving.is_some(),
            "notice": self.notice,
            "entries": self.visible.iter().take(100).filter_map(|i| self.inventory.entries.get(*i)).map(|e| json!({
                "id": e.id, "kind": e.kind.id(), "trigger": short(&e.trigger), "location": location(e), "first_line": first_line(e.code.as_deref()),
                "truncated": e.truncated, "has_code": e.code.is_some(),
            })).collect::<Vec<_>>(),
        })
    }
}

/// The line shown when the listing left some names out.
pub(crate) fn names_left_out_note() -> String {
    tl!("Some scripts are listed without the field name, script name or title they belong to: the document's names are too long to read in full.")
        .to_string()
}

/// Read the document's scripts again into the draft.
fn relist(app: &mut PdfCraftApp, id: DocId, quiet: bool, failed: bool) {
    let Some(doc) = app.session.get(id) else { return };
    let (generation, inventory) = (doc.edit_generation(), doc.list_javascripts());
    app.javascripts_draft.relisted(id, generation, inventory, quiet, failed);
}

/// Whether the control channel and the window see the document as read-only.
pub(crate) fn read_only(app: &PdfCraftApp) -> bool {
    app.active_ids().and_then(|(_, id)| app.session.get(id)).is_some_and(|d| locked(d).is_some())
}

/// Bring the listing up to date with the active document, if the window is open.
pub(crate) fn sync(app: &mut PdfCraftApp) {
    let Some((_, id)) = app.active_ids() else { return };
    let Some(generation) = app.session.get(id).map(|d| d.edit_generation()) else { return };
    if app.javascripts_draft.listed != Some((id, generation)) {
        relist(app, id, false, false);
    }
}

/// Opens the window on the active document with nothing selected.
pub(crate) fn open(app: &mut PdfCraftApp) {
    app.javascripts_draft = Draft::default();
    app.javascripts_open = true;
}

/// Closes the window and forgets the list.
fn close(app: &mut PdfCraftApp) {
    app.javascripts_open = false;
    app.javascripts_draft = Draft::default();
}

/// Apply the editor's code to the selected script.
pub(crate) fn apply(app: &mut PdfCraftApp, ctx: &egui::Context) -> bool {
    let Some((_, id)) = app.active_ids() else { return false };
    let Some(doc) = app.session.get(id) else { return false };
    let locked = locked(doc).is_some();
    let Some(l) = app.javascripts_draft.loaded.clone().filter(|_| app.javascripts_draft.can_edit(locked)) else { return false };
    let code = app.javascripts_draft.text.clone();
    let done = app.apply_edit(ctx, Edit::SetJavaScript { id: l.id.clone(), expect: l.fingerprint.clone(), code });
    finish(app, id, done, tl!("Applied. Undo in the Edit menu takes it back."));
    done
}

/// Delete the selected script.
pub(crate) fn delete(app: &mut PdfCraftApp, ctx: &egui::Context) -> bool {
    let Some((_, id)) = app.active_ids() else { return false };
    let Some(doc) = app.session.get(id) else { return false };
    let locked = locked(doc).is_some();
    let Some(l) = app.javascripts_draft.loaded.clone().filter(|_| !locked) else { return false };
    app.javascripts_draft.confirm_delete = false;
    let done = app.apply_edit(ctx, Edit::RemoveJavaScript { id: l.id.clone(), expect: l.fingerprint.clone() });
    if done {
        // The id may lead to an action chained after the removed one now: not the script that was selected.
        app.javascripts_draft.deselect();
    }
    finish(app, id, done, tl!("Deleted. Undo in the Edit menu takes it back."));
    done
}

/// After an edit: read the list again, and say what happened. A refusal is already a notice of
/// the app; the window adds one only when the script it holds changed or went.
fn finish(app: &mut PdfCraftApp, id: DocId, done: bool, message: &str) {
    relist(app, id, done, !done);
    if done {
        app.javascripts_draft.notice = Some(message.to_string());
    }
}

/// What a footer button asks for.
#[derive(Clone, Copy)]
enum Action {
    Apply,
    Delete,
    Close,
    DeleteNow,
    KeepScript,
    Discard,
    KeepEditing,
}

/// Shows the window while it is open and carries out the button it was answered with. It closes
/// with the document it listed, and asks before it drops unsaved typing (Escape and Close too).
pub(crate) fn show(app: &mut PdfCraftApp, ui: &mut egui::Ui) {
    if !app.javascripts_open {
        return;
    }
    let Some((_, id)) = app.active_ids() else { return close(app) };
    let Some(lock) = app.session.get(id).map(locked) else { return close(app) };
    sync(app);
    let read_only = lock.is_some();
    let mut d = std::mem::take(&mut app.javascripts_draft);
    d.refresh_visible();
    if d.leaving.is_some() && !d.dirty() {
        d.leaving = None;
    }
    let has = d.loaded.is_some();
    let can_apply = d.can_edit(read_only) && d.dirty();
    let actions: Vec<(Action, FooterButton)> = if d.leaving.is_some() {
        vec![
            (Action::Discard, FooterButton::new(tl!("Discard"), DialogRole::Destructive).stays_open(true)),
            (Action::KeepEditing, FooterButton::new(tl!("Keep editing"), DialogRole::Other).stays_open(true)),
        ]
    } else if d.confirm_delete && has {
        vec![
            (Action::DeleteNow, FooterButton::new(tl!("Delete script"), DialogRole::Destructive).stays_open(true)),
            (Action::KeepScript, FooterButton::new(tl!("Keep"), DialogRole::Other).stays_open(true)),
        ]
    } else {
        vec![
            (Action::Apply, FooterButton::new(tl!("Apply"), DialogRole::Accept).stays_open(true).enabled(can_apply)),
            (Action::Delete, FooterButton::new(tl!("Delete"), DialogRole::Other).stays_open(true).enabled(has && !read_only)),
            (Action::Close, FooterButton::new(tl!("Close"), DialogRole::Cancel)),
        ]
    };
    let buttons: Vec<FooterButton> = actions.iter().map(|(_, b)| b.clone()).collect();
    let mut open = true;
    let mut pick = None;
    let shown = Dialog::new(tl!("JavaScripts")).opened(Bind::bound(&mut open)).escape_keeps_typing(true).buttons(buttons).show(ui, |ui, _cx| {
        pick = body(ui, &mut d, lock);
    });
    let picked = shown.output.picked.and_then(|at| actions.get(at)).map(|(a, _)| *a);
    if let Some(i) = pick {
        // Another script than the one open: not while typing would be lost without asking.
        let same = d.inventory.entries.get(i).is_some_and(|e| d.loaded.as_ref().is_some_and(|l| l.id == e.id));
        if !same {
            if d.dirty() {
                d.leaving = d.inventory.entries.get(i).map(|e| Leave::Select(e.id.clone()));
            } else {
                d.select(i);
            }
        }
    }
    app.javascripts_draft = d;
    let ctx = ui.ctx().clone();
    match picked {
        Some(Action::Apply) => {
            apply(app, &ctx);
        }
        Some(Action::Delete) => app.javascripts_draft.confirm_delete = true,
        Some(Action::DeleteNow) => {
            delete(app, &ctx);
        }
        Some(Action::KeepScript) => app.javascripts_draft.confirm_delete = false,
        Some(Action::KeepEditing) => app.javascripts_draft.leaving = None,
        Some(Action::Discard) => match app.javascripts_draft.leaving.take() {
            Some(Leave::Close) => {
                close(app);
                return;
            }
            Some(Leave::Select(id)) => {
                app.javascripts_draft.select_id(&id);
            }
            None => {}
        },
        Some(Action::Close) | None => {}
    }
    if !open {
        // Escape, the close action or Close: not while typing would be lost without asking.
        if app.javascripts_draft.dirty() {
            app.javascripts_draft.leaving = Some(Leave::Close);
        } else {
            close(app);
        }
    }
}

/// The window's content. Returns the list row the user picked, if any (an index of the listing).
fn body(ui: &mut egui::Ui, d: &mut Draft, lock: Option<Locked>) -> Option<usize> {
    let read_only = lock.is_some();
    Text::new(tl!("Every script in this document, in one list. Nothing here runs a script.")).shade(Shade::Muted).show(ui);
    match lock {
        Some(Locked::Unreadable) => {
            Notice::new(Tone::Warn, tl!("This document can't be edited, so its scripts are shown read-only.").to_string())
                .layout(NoticeLayout::Inline)
                .id_salt(0usize)
                .show(ui);
        }
        Some(Locked::Permissions) => {
            Notice::new(
                Tone::Warn,
                tl!("This document's security settings don't allow changes, so its scripts are shown read-only. Enter its permissions password to edit them.")
                    .to_string(),
            )
            .layout(NoticeLayout::Inline)
            .id_salt(0usize)
            .show(ui);
        }
        None => {}
    }

    let kinds: Vec<Entry<usize>> = std::iter::once(Entry::new(0usize, tl!("All kinds").to_string()))
        .chain(JavaScriptKind::ALL.iter().enumerate().map(|(i, k)| Entry::new(i + 1, kind_label(*k))))
        .collect();
    let kind_at = d.kind.and_then(|k| JavaScriptKind::ALL.iter().position(|a| *a == k)).map_or(0, |i| i + 1);
    let mut kind_pick = None;
    Row::new().show(ui, |ui, _| {
        TextField::new(tl!("Filter"), Bind::bound(&mut d.filter)).placeholder(tl!("Name, field, trigger or code")).limit(FILTER_CHARS).show(ui);
        kind_pick = Select::new(tl!("Kind"), Bind::given(kind_at), &kinds).show(ui).value().copied();
    });
    if let Some(at) = kind_pick.filter(|at| *at != kind_at) {
        d.set_kind(at.checked_sub(1).and_then(|i| JavaScriptKind::ALL.get(i).copied()));
    }
    d.refresh_visible();

    let summary = if d.shown() == d.total() {
        crate::i18n::fmt(tl!("{n} scripts"), &[("n", &d.total().to_string())])
    } else {
        crate::i18n::fmt(tl!("Showing {shown} of {n} scripts"), &[("shown", &d.shown().to_string()), ("n", &d.total().to_string())])
    };
    Text::new(summary).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    if d.capped() {
        Text::new(tl!("The document holds more scripts than can be listed; some are not shown."))
            .role(TypeRole::Caption)
            .shade(Shade::Muted)
            .show(ui);
    }
    if d.inventory.truncated > 0 {
        Text::new(tl!("Some scripts are too long to be listed in full and are marked (cut); they can't be edited here."))
            .role(TypeRole::Caption)
            .shade(Shade::Muted)
            .show(ui);
    }
    if d.inventory.names_left_out > 0 {
        Text::new(names_left_out_note()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    }

    let mut picked = None;
    if d.visible.is_empty() {
        let message = if d.total() == 0 { tl!("This document has no JavaScripts.") } else { tl!("No script matches the filter.") };
        Text::new(message).shade(Shade::Muted).show(ui);
    } else {
        let current = d.loaded.as_ref().and_then(|l| d.inventory.entries.iter().position(|e| e.id == l.id));
        let mut selection = Selection::new(current.into_iter().collect(), current, current);
        let order = d.visible.clone();
        let rows = keyed(&order, |i| *i);
        let labels = &d.rows;
        List::new(tl!("Scripts").to_string()).select(SelectMode::Single).selection(Bind::bound(&mut selection)).max_rows(LIST_ROWS).show_items(
            ui,
            &rows,
            &mut |key, item| match labels.get(*key) {
                Some(r) => ListRow::new(r.label.clone()).state(item.row.clone()),
                None => ListRow::new(String::new()),
            },
        );
        picked = selection.items.first().copied().filter(|p| Some(*p) != current);
    }

    if let Some((cut, readable)) = d.loaded.as_ref().map(|l| (l.cut, l.readable)) {
        let heading = d.selected_entry().map(|e| format!("{}: {}  ({})", kind_label(e.kind), location(e), short(&e.trigger))).unwrap_or_default();
        Text::new(heading).role(TypeRole::Heading).show(ui);
        let editable = d.can_edit(read_only);
        let epoch = d.epoch;
        TextField::new(tl!("Script code"), Bind::bound(&mut d.text))
            .lines(EDITOR_LINES)
            .max_lines(EDITOR_LINES)
            .mono()
            .tab_inserts(true)
            .limit(SHOWN_LIMIT)
            .read_only(!editable)
            .id_salt(epoch)
            .show(ui);
        let note = match cut {
            Cut::Listing => Some(
                tl!("This script is too long to be listed in full, so only its start is shown and it can't be edited here. It can still be deleted.")
                    .to_string(),
            ),
            Cut::Shown(bytes) => Some(crate::i18n::fmt(
                tl!("This script is {size} KiB, so only the first {limit} KiB are shown and it can't be edited here. It can still be deleted."),
                &[("size", &kib(bytes)), ("limit", &kib(SHOWN_LIMIT))],
            )),
            Cut::No if !readable => Some(tl!("This action has no readable code. Applying writes the code above in its place.").to_string()),
            Cut::No => None,
        };
        if let Some(note) = note {
            Text::new(note).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
        }
    } else if d.total() > 0 {
        Text::new(tl!("Select a script to read or change its code.")).shade(Shade::Muted).show(ui);
    }
    if let Some(n) = &d.notice {
        Notice::new(Tone::Info, n.clone()).layout(NoticeLayout::Inline).id_salt(1usize).show(ui);
    }
    if d.leaving.is_some() {
        Notice::new(Tone::Warn, tl!("You have unsaved changes to this script. Discard them?").to_string())
            .layout(NoticeLayout::Inline)
            .id_salt(2usize)
            .show(ui);
    } else if d.confirm_delete && d.loaded.is_some() {
        Notice::new(Tone::Warn, tl!("Delete this script? Undo brings it back.").to_string()).layout(NoticeLayout::Inline).id_salt(3usize).show(ui);
    }
    picked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R /OpenAction << /S /JavaScript /JS (one\\(\\);) >> >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>",
            "<< /Type /Page /Parent 2 0 R >>",
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
        out
    }

    #[test]
    fn a_new_listing_refreshes_the_rows_at_once() {
        let mut sess = pdfcraft_engine::Session::new();
        let id = sess.open("one.pdf", None, std::sync::Arc::new(fixture()), None).unwrap();
        let inventory = sess.get(id).unwrap().list_javascripts();
        let mut d = Draft::default();
        d.relisted(id, 0, JavaScriptInventory::default(), true, false);
        assert_eq!((d.total(), d.shown()), (0, 0));
        d.relisted(id, 1, inventory, true, false);
        assert_eq!((d.total(), d.shown()), (1, 1), "no frame has run, and the state is current");
        d.set_filter("one");
        assert_eq!(d.shown(), 1);
        d.set_filter("zzz");
        assert_eq!(d.shown(), 0, "a filter set from outside takes effect at once");
    }

    #[test]
    fn names_left_out_are_reported_in_the_state_and_the_note_names_all_three() {
        let mut sess = pdfcraft_engine::Session::new();
        let id = sess.open("one.pdf", None, std::sync::Arc::new(fixture()), None).unwrap();
        let mut inventory = sess.get(id).unwrap().list_javascripts();
        assert_eq!(Draft::default().state(true, false)["names_left_out"], 0);
        inventory.names_left_out = 3;
        let mut d = Draft::default();
        d.relisted(id, 1, inventory, true, false);
        assert_eq!(d.state(true, false)["names_left_out"], 3);
        let note = names_left_out_note();
        assert!(note.contains("field name") && note.contains("script name") && note.contains("title"), "{note}");
    }

    #[test]
    fn a_long_script_is_cut_on_a_character_boundary() {
        let s = "é".repeat(SHOWN_LIMIT);
        let cut = cut_str(&s, SHOWN_LIMIT);
        assert!(cut.len() <= SHOWN_LIMIT && cut.len() >= SHOWN_LIMIT - 1);
        assert_eq!(cut_str("abc", 10), "abc");
        assert_eq!(cut_str("é", 1), "");
    }

    #[test]
    fn a_stale_selection_keeps_the_typing_after_a_refused_apply() {
        let mut sess = pdfcraft_engine::Session::new();
        let id = sess.open("one.pdf", None, std::sync::Arc::new(fixture()), None).unwrap();
        let mut d = Draft::default();
        d.relisted(id, 0, sess.get(id).unwrap().list_javascripts(), true, false);
        d.select(0);
        // The window names a fingerprint the document does not hold, as a list from before another edit would.
        if let Some(l) = d.loaded.as_mut() {
            l.fingerprint = "0000000000000000".into();
        }
        d.text = "two();".into();
        d.relisted(id, 1, sess.get(id).unwrap().list_javascripts(), false, true);
        assert_eq!(d.text, "two();", "the typing is kept, not reloaded");
        assert!(d.dirty(), "and it can be applied against the current code");
        assert!(d.notice.as_deref().is_some_and(|n| n.contains("nothing was changed") && n.contains("Your text is kept")), "{:?}", d.notice);
        assert_ne!(d.loaded.as_ref().map(|l| l.fingerprint.as_str()), Some("0000000000000000"));
    }
}
