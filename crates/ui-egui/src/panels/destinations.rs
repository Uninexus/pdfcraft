//! The Destinations panel: the document's named destinations (the `/Dests` name tree and the catalog's `/Dests`
//! dictionary, ISO 32000-2 12.3.2.4) with the page each goes to. A double click or Enter goes to the page; Delete
//! takes the name out as one undoable edit. No name is added here: the engine only reads and removes them.

use crate::app::commands::{FocusIf as _, list_focus};
use craft::behavior::{DisplayOrder, IconId, ItemSource, Mixed};
use craft::ui::contract::{Bind, BodyCx, CollectionEvent, Entry, SelectMode, Show as _, ShowCollection as _};
use craft::ui::list::{List, ListRow, RowEvent, TrailingFace, TrailingPart};
use craft::ui::menu::Menu;
use craft::ui::notices::Notice;
use craft::ui::primitives::IconSpec;
use craft::ui::text::TextField;

use super::right::PanelResult;
use super::{Nav, empty_state};
use crate::PdfCraftApp;

const DELETE: TrailingPart = TrailingPart::new(0);

/// What the row menu's entries do.
const MENU_GO: u8 = 0;
const MENU_DELETE: u8 = 1;

/// One listed name, as the panel keeps it between edits.
#[derive(Clone)]
struct Name {
    /// Its position in the engine's listing, which is what removing it takes.
    index: usize,
    name: String,
    /// The 1-based page it goes to.
    page: Option<usize>,
    view: Option<String>,
    removable: bool,
}

/// The names, keyed by position in the panel's list.
struct Names(usize);

impl DisplayOrder<usize> for Names {
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

impl ItemSource<usize> for Names {}

/// The names of the document, read again only when its edit generation changes: the name tree walk is bounded but not
/// free, and a panel draws every frame.
fn read_names(ui: &egui::Ui, cx: &BodyCx<'_>, doc: &pdfcraft_engine::Document) -> (Vec<Name>, bool) {
    let slot = cx.id.with("destinations-read");
    let generation = doc.edit_generation();
    if let Some((seen, names, truncated)) = ui.data(|d| d.get_temp::<(u64, Vec<Name>, bool)>(slot))
        && seen == generation
    {
        return (names, truncated);
    }
    let read = doc.named_destinations();
    let names: Vec<Name> = read
        .destinations
        .iter()
        .map(|d| Name { index: d.index, name: d.name.clone(), page: d.page, view: d.view.clone(), removable: d.removable })
        .collect();
    ui.data_mut(|d| d.insert_temp(slot, (generation, names.clone(), read.truncated)));
    (names, read.truncated)
}

/// Show the Destinations panel for the active document.
pub(crate) fn show(app: &mut PdfCraftApp, ui: &mut egui::Ui, cx: &mut BodyCx<'_>) -> PanelResult {
    let mut result = PanelResult::default();
    let Some((_, id)) = app.active_ids() else { return result };
    let Some(doc) = app.session.get(id) else { return result };
    let editable = doc.allows_modification() && doc.read_only_reason.is_none();
    let (names, truncated) = read_names(ui, cx, doc);
    if names.is_empty() {
        empty_state(ui, "pin", tl!("This document has no named destinations."));
        return result;
    }
    let pages = &doc.info.pages;
    // The query is the search field's kept value, inside the document's id scope so each document keeps its own.
    let query = ui
        .push_id(id.0, |ui| match TextField::search(tl!("Search"), Bind::<String>::kept()).show(ui).shown {
            Some(Mixed::Same(text)) => text.as_str().to_owned(),
            _ => String::new(),
        })
        .inner;
    let page_label = |name: &Name| name.page.and_then(|p| pages.get(p.checked_sub(1)?)).map(|p| p.label.clone());
    let detail = |name: &Name| match (page_label(name), &name.view) {
        (Some(label), Some(view)) => crate::i18n::fmt(tl!("Page {label}, {view}"), &[("label", &label), ("view", view)]),
        (Some(label), None) => crate::i18n::fmt(tl!("Page {label}"), &[("label", &label)]),
        (None, _) => tl!("No page").to_string(),
    };
    let row_key = |key: &usize| names.get(*key).map(|n| vec![n.name.clone().into()]).unwrap_or_default();
    let take = list_focus(&mut app.panel_focus, crate::RightPanel::Destinations, !names.is_empty());
    let out = List::<usize>::new(tl!("Named destinations").to_string())
        .focus_if(take)
        .select(SelectMode::Single)
        .empty(tl!("No matches."))
        .filter(&query, row_key)
        .show_items(ui, &Names(names.len()), &mut |key, _| {
            let Some(name) = names.get(*key) else { return ListRow::new("") };
            let row = ListRow::new(name.name.clone()).icon(IconSpec::svg(IconId::new("pin"))).detail(detail(name));
            if name.removable {
                row.trailing_part(DELETE, TrailingFace::Icon(IconSpec::svg(IconId::new("trash-2"))), tl!("Delete destination").to_string())
            } else {
                row
            }
        });
    // The row the open context menu belongs to lives in the panel body's memory, not in egui's temp data.
    let menu_slot = cx.id.with("destination-menu-row");
    let delete = |name: &Name, result: &mut PanelResult| {
        if editable {
            result.edit = Some(pdfcraft_engine::Edit::RemoveNamedDestination(name.index));
        } else {
            result.notice = Some("This document is read-only, so the name can't be removed.");
        }
    };
    for event in &out.events {
        match event {
            CollectionEvent::Activate(key) => {
                if let Some(page) = names.get(*key).and_then(|n| n.page).filter(|p| *p >= 1 && *p <= pages.len()) {
                    result.nav = Some(Nav::Page(page - 1));
                }
            }
            CollectionEvent::Item(key, RowEvent::Trailing { part, .. }) if *part == DELETE => {
                if let Some(name) = names.get(*key) {
                    delete(name, &mut result);
                }
            }
            CollectionEvent::ContextMenu(request) => {
                *cx.memory.shown::<Option<usize>>(menu_slot) = Some(request.key);
            }
            _ => {}
        }
    }
    let row: Option<usize> = *cx.memory.shown::<Option<usize>>(menu_slot);
    if let Some(name) = row.and_then(|key| names.get(key)) {
        let mut entries = Vec::new();
        if name.page.is_some() {
            entries.push(Entry::new(MENU_GO, tl!("Go to destination").to_string()));
        }
        if name.removable {
            entries.push(Entry::new(MENU_DELETE, tl!("Delete").to_string()));
        }
        match Menu::context(tl!("Named destination").to_string(), &out.response).pick(ui, &entries).picked {
            Some(MENU_GO) => {
                if let Some(page) = name.page.filter(|p| *p >= 1 && *p <= pages.len()) {
                    result.nav = Some(Nav::Page(page - 1));
                }
            }
            Some(MENU_DELETE) => delete(name, &mut result),
            _ => {}
        }
    }
    if truncated {
        Notice::new(craft::theme::variant::Tone::Neutral, tl!("Some named destinations are not shown here: the panel lists a limited number."))
            .show(ui);
    }
    result
}
