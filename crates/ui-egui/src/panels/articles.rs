//! The Articles panel: the document's article threads (the catalog's `/Threads`, ISO 32000-2 12.4.3) with how many
//! beads each has and the page of its first bead. A double click or Enter goes to that page; Delete takes the thread
//! out as one undoable edit. The engine reads no bead's rectangle and adds no thread, so the panel does not scroll to
//! a bead, step from bead to bead or add an article.

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

/// One listed thread, as the panel keeps it between edits.
#[derive(Clone)]
struct Article {
    /// Its position in `/Threads`, which is what removing it takes.
    index: usize,
    title: String,
    beads: usize,
    /// The 1-based page of its first bead.
    page: Option<usize>,
}

/// The threads, keyed by position in the panel's list.
struct Articles(usize);

impl DisplayOrder<usize> for Articles {
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

impl ItemSource<usize> for Articles {}

/// The threads of the document, read again only when its edit generation changes: a panel draws every frame and the
/// bead walk is bounded but not free.
fn read_articles(ui: &egui::Ui, cx: &BodyCx<'_>, doc: &pdfcraft_engine::Document) -> (Vec<Article>, bool) {
    let slot = cx.id.with("articles-read");
    let generation = doc.edit_generation();
    if let Some((seen, articles, truncated)) = ui.data(|d| d.get_temp::<(u64, Vec<Article>, bool)>(slot))
        && seen == generation
    {
        return (articles, truncated);
    }
    let read = doc.article_threads();
    let articles: Vec<Article> = read
        .threads
        .iter()
        .map(|t| {
            let title = match t.title.as_deref().map(str::trim).filter(|title| !title.is_empty()) {
                Some(title) => title.to_owned(),
                None => crate::i18n::fmt(tl!("Untitled article {number}"), &[("number", &(t.index + 1).to_string())]),
            };
            Article { index: t.index, title, beads: t.beads, page: t.first_page }
        })
        .collect();
    ui.data_mut(|d| d.insert_temp(slot, (generation, articles.clone(), read.truncated)));
    (articles, read.truncated)
}

/// Show the Articles panel for the active document.
pub(crate) fn show(app: &mut PdfCraftApp, ui: &mut egui::Ui, cx: &mut BodyCx<'_>) -> PanelResult {
    let mut result = PanelResult::default();
    let Some((_, id)) = app.active_ids() else { return result };
    let Some(doc) = app.session.get(id) else { return result };
    let editable = doc.allows_modification() && doc.read_only_reason.is_none();
    let (articles, truncated) = read_articles(ui, cx, doc);
    if articles.is_empty() {
        empty_state(ui, "columns-3", tl!("This document has no articles."));
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
    let page_label = |article: &Article| article.page.and_then(|p| pages.get(p.checked_sub(1)?)).map(|p| p.label.clone());
    let detail = |article: &Article| {
        let beads = match article.beads {
            1 => tl!("1 bead").to_string(),
            n => crate::i18n::fmt(tl!("{count} beads"), &[("count", &n.to_string())]),
        };
        match page_label(article) {
            Some(label) => crate::i18n::fmt(tl!("{beads}, page {label}"), &[("beads", &beads), ("label", &label)]),
            None => beads,
        }
    };
    let row_key = |key: &usize| articles.get(*key).map(|a| vec![a.title.clone().into()]).unwrap_or_default();
    let out = List::<usize>::new(tl!("Articles").to_string())
        .select(SelectMode::Single)
        .empty(tl!("No matches."))
        .filter(&query, row_key)
        .show_items(ui, &Articles(articles.len()), &mut |key, _| {
            let Some(article) = articles.get(*key) else { return ListRow::new("") };
            ListRow::new(article.title.clone()).icon(IconSpec::svg(IconId::new("columns-3"))).detail(detail(article)).trailing_part(
                DELETE,
                TrailingFace::Icon(IconSpec::svg(IconId::new("trash-2"))),
                tl!("Delete article").to_string(),
            )
        });
    // The row the open context menu belongs to lives in the panel body's memory, not in egui's temp data.
    let menu_slot = cx.id.with("article-menu-row");
    let delete = |article: &Article, result: &mut PanelResult| {
        if editable {
            result.edit = Some(pdfcraft_engine::Edit::RemoveArticleThread(article.index));
        } else {
            result.notice = Some("This document is read-only, so the article can't be removed.");
        }
    };
    let go = |article: &Article, result: &mut PanelResult| {
        if let Some(page) = article.page.filter(|p| *p >= 1 && *p <= pages.len()) {
            result.nav = Some(Nav::Page(page - 1));
        }
    };
    for event in &out.events {
        match event {
            CollectionEvent::Activate(key) => {
                if let Some(article) = articles.get(*key) {
                    go(article, &mut result);
                }
            }
            CollectionEvent::Item(key, RowEvent::Trailing { part, .. }) if *part == DELETE => {
                if let Some(article) = articles.get(*key) {
                    delete(article, &mut result);
                }
            }
            CollectionEvent::ContextMenu(request) => {
                *cx.memory.shown::<Option<usize>>(menu_slot) = Some(request.key);
            }
            _ => {}
        }
    }
    let row: Option<usize> = *cx.memory.shown::<Option<usize>>(menu_slot);
    if let Some(article) = row.and_then(|key| articles.get(key)) {
        let mut entries = Vec::new();
        if article.page.is_some() {
            entries.push(Entry::new(MENU_GO, tl!("Go to article").to_string()));
        }
        entries.push(Entry::new(MENU_DELETE, tl!("Delete").to_string()));
        match Menu::context(tl!("Article").to_string(), &out.response).pick(ui, &entries).picked {
            Some(MENU_GO) => go(article, &mut result),
            Some(MENU_DELETE) => delete(article, &mut result),
            _ => {}
        }
    }
    if truncated {
        Notice::new(craft::theme::variant::Tone::Neutral, tl!("Some articles are not shown here: the panel lists a limited number.")).show(ui);
    }
    result
}
