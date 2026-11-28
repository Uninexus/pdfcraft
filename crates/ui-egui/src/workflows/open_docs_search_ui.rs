//! The Search panel's All open documents scope: the query and options of the panel are searched in every open
//! document, with the matcher the find bar uses (`PageText::find_opts_limited`), and the matches are listed by document
//! with their page and context. A click on one switches to that document's tab and goes to its page.
//!
//! Each document's pages are read in the background the way the find bar reads them. A page's matches are kept when it
//! is read, so a long document is searched whole without holding on to more than the matches. The list is bounded per
//! document and in all, and says when it was cut short.

use std::collections::{BTreeMap, HashMap, HashSet};

use craft::behavior::{DisplayOrder, ItemSource, Selection, SelectionChange};
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, CollectionEvent, Props as _, SelectMode, Show as _, ShowCollection as _};
use craft::ui::display::{Shade, Text};
use craft::ui::list::{List, ListRow};
use craft::ui::runtime::env::Label;
use craft::ui::style::{Span, TypeRole};
use pdfcraft_engine::folder_search::{CONTEXT, Match};

use crate::PdfCraftApp;
use crate::canvas::DocView;

/// Matches listed for one document; more are reported as capped.
pub const MAX_PER_DOCUMENT: usize = 200;

/// Matches listed in all.
pub const MAX_TOTAL: usize = 1000;

/// Pages asked for at a time from one document, so a long one does not queue every page at once.
const PAGES_AHEAD: usize = 16;

/// What one open document's search found so far.
pub struct DocHits {
    /// The tab ([`DocView::view_key`]).
    pub key: u64,
    /// The tab's name.
    pub name: String,
    pages: usize,
    scanned: HashSet<usize>,
    failed: usize,
    by_page: BTreeMap<usize, Vec<Match>>,
    /// The document's own cap was reached.
    pub capped: bool,
}

impl DocHits {
    fn new(key: u64, name: String, pages: usize) -> Self {
        Self { key, name, pages, scanned: HashSet::new(), failed: 0, by_page: BTreeMap::new(), capped: false }
    }

    /// Matches found, in page order.
    pub fn matches(&self) -> Vec<&Match> {
        self.by_page.values().flatten().collect()
    }

    /// Matches found.
    pub fn found(&self) -> usize {
        self.by_page.values().map(Vec::len).sum()
    }

    /// Every page was read (or the cap ended the search of this document).
    pub fn done(&self) -> bool {
        self.capped || self.scanned.len() >= self.pages
    }
}

/// The All open documents scope of the Search panel and what its last search found.
#[derive(Default)]
pub struct OpenDocsSearch {
    /// Search every open document instead of the active one.
    pub on: bool,
    /// The documents with matches, or still being read, in tab order.
    pub docs: Vec<DocHits>,
    /// The total cap was reached.
    pub total_capped: bool,
    /// What the results are for: query, case, whole words.
    signature: Option<(String, bool, bool)>,
}

impl OpenDocsSearch {
    /// Matches listed in all.
    pub fn found(&self) -> usize {
        self.docs.iter().map(DocHits::found).sum()
    }

    /// Pages that could not be read.
    fn pages_failed(&self) -> usize {
        self.docs.iter().map(|d| d.failed).sum()
    }

    /// Every document is searched to the end (or capped).
    pub fn done(&self) -> bool {
        self.total_capped || self.docs.iter().all(DocHits::done)
    }

    /// Pages read of all pages, for the status line.
    fn progress(&self) -> (usize, usize) {
        (self.docs.iter().map(|d| d.scanned.len().min(d.pages)).sum(), self.docs.iter().map(|d| d.pages).sum())
    }
}

/// What the panel asks for.
pub(crate) enum Action {
    /// Back to searching the active document.
    ThisDocument,
    /// Go to the 0-based page of the tab with this key.
    Go(u64, usize),
}

/// A snippet of the context around a match, whitespace folded as the other result lists do.
fn snippet(text: &pdfcraft_render::PageText, page: usize, range: std::ops::Range<usize>) -> Match {
    let n = text.glyphs.len();
    let (start, end) = (range.start.min(n), range.end.min(n));
    let flat = |s: String| s.split_whitespace().collect::<Vec<_>>().join(" ");
    Match {
        page,
        before: flat(text.text_of(start.saturating_sub(CONTEXT)..start)),
        text: flat(text.text_of(start..end)),
        after: flat(text.text_of(end..end.saturating_add(CONTEXT).min(n))),
    }
}

/// Takes the page texts a document has read since the last pass.
fn absorb(doc: &mut DocHits, view: &DocView, query: &str, case: bool, whole: bool, room_total: usize) -> usize {
    let mut taken = 0usize;
    let mut pages: Vec<usize> = view.session.texts.keys().copied().filter(|p| !doc.scanned.contains(p)).collect();
    pages.sort_unstable();
    for page in pages {
        let room = MAX_PER_DOCUMENT.saturating_sub(doc.found()).min(room_total.saturating_sub(taken));
        if room == 0 {
            doc.capped = true;
            break;
        }
        let Some(text) = view.session.texts.get(&page) else { continue };
        // One past the room proves that more exist.
        let found = text.find_opts_limited(query, case, whole, room.saturating_add(1));
        let more = found.len() > room;
        let hits: Vec<Match> = found.into_iter().take(room).map(|range| snippet(text, page, range)).collect();
        taken += hits.len();
        doc.scanned.insert(page);
        if !hits.is_empty() {
            doc.by_page.insert(page, hits);
        }
        if more {
            doc.capped = true;
            break;
        }
    }
    for page in &view.session.text_failed {
        if doc.scanned.insert(*page) {
            doc.failed += 1;
        }
    }
    taken
}

impl OpenDocsSearch {
    /// One pass: starts over when the query or an option changed, forgets closed tabs, takes what the documents read
    /// since the last pass and asks for the pages not read yet. `names` are the tab names by key.
    pub(crate) fn pass(
        &mut self,
        views: &mut [DocView],
        session: &pdfcraft_engine::Session,
        names: &[(u64, String)],
        query: &str,
        case: bool,
        whole: bool,
    ) {
        let signature = (query.to_owned(), case, whole);
        if self.signature.as_ref() != Some(&signature) {
            self.docs.clear();
            self.total_capped = false;
            self.signature = Some(signature);
        }
        self.docs.retain(|d| names.iter().any(|(key, _)| *key == d.key));
        if query.trim().is_empty() {
            self.docs.clear();
            for view in views.iter_mut() {
                view.session.hold_text = false;
            }
            return;
        }
        // Tab order.
        let order: HashMap<u64, usize> = names.iter().enumerate().map(|(i, (key, _))| (*key, i)).collect();
        let mut total = self.found();
        for view in views.iter_mut() {
            let key = view.view_key();
            let Some((_, name)) = names.iter().find(|(k, _)| *k == key) else { continue };
            let at = match self.docs.iter().position(|d| d.key == key) {
                Some(at) => at,
                None => {
                    self.docs.push(DocHits::new(key, name.clone(), view.page_count()));
                    self.docs.len() - 1
                }
            };
            let Some(doc) = self.docs.get_mut(at) else { continue };
            doc.name.clone_from(name);
            doc.pages = view.page_count();
            total += absorb(doc, view, query, case, whole, MAX_TOTAL.saturating_sub(total));
            if total >= MAX_TOTAL {
                self.total_capped = true;
            }
            let wanted = !doc.done() && !self.total_capped;
            view.session.hold_text = wanted;
            if wanted {
                let ahead: Vec<usize> = (0..doc.pages).filter(|p| !doc.scanned.contains(p)).take(PAGES_AHEAD).collect();
                view.want_text(session, &[], &ahead);
            }
        }
        self.docs.sort_by_key(|d| order.get(&d.key).copied().unwrap_or(usize::MAX));
    }
}

/// A row of the results: a document's heading, or a match by its document and its place among the document's matches.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Hit {
    Doc(usize),
    Match(usize, usize),
}

/// The results in display order.
struct Rows {
    rows: Vec<Hit>,
    at: HashMap<Hit, usize>,
}

impl Rows {
    fn of(docs: &[Vec<&Match>]) -> Self {
        let mut rows = Vec::new();
        for (d, found) in docs.iter().enumerate() {
            if found.is_empty() {
                continue;
            }
            rows.push(Hit::Doc(d));
            rows.extend((0..found.len()).map(|m| Hit::Match(d, m)));
        }
        let at = rows.iter().cloned().enumerate().map(|(i, h)| (h, i)).collect();
        Self { rows, at }
    }
}

impl DisplayOrder<Hit> for Rows {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn key(&self, at: usize) -> Hit {
        self.rows.get(at).cloned().unwrap_or(Hit::Doc(0))
    }

    fn index_of(&self, key: &Hit) -> Option<usize> {
        self.at.get(key).copied()
    }
}

impl ItemSource<Hit> for Rows {}

/// The scope's controls and results, below the query and the options.
pub(crate) fn section(ui: &mut egui::Ui, state: &OpenDocsSearch, can_search: bool) -> Option<Action> {
    let mut action = None;
    if Button::new(tl!("Search this document").to_string()).show(ui).clicked() {
        action = Some(Action::ThisDocument);
    }
    if !can_search {
        return action;
    }
    let found = state.found();
    let count =
        if found == 1 { crate::i18n::fmt(tl!("1 instance"), &[]) } else { crate::i18n::fmt(tl!("{n} instances"), &[("n", &found.to_string())]) };
    let status = if state.done() {
        let docs = state.docs.iter().filter(|d| d.found() > 0).count();
        let docs = crate::i18n::trn(crate::i18n::current(), docs as u64, "{n} document", "{n} documents");
        format!("{count}, {docs}")
    } else {
        let (read, all) = state.progress();
        crate::i18n::fmt(tl!("{count}, searching… {s} of {p} pages"), &[("count", &count), ("s", &read.to_string()), ("p", &all.to_string())])
    };
    Text::new(status).show(ui);
    if state.total_capped || state.docs.iter().any(|d| d.capped) || state.pages_failed() > 0 {
        Text::new(tl!("Not every match is listed").to_string()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    }
    let lists: Vec<Vec<&Match>> = state.docs.iter().map(DocHits::matches).collect();
    if lists.iter().all(Vec::is_empty) {
        return action;
    }
    let rows = Rows::of(&lists);
    let none: Selection<Hit> = Selection::default();
    let docs = &state.docs;
    let out = List::<Hit>::new(tl!("Open document search results").to_string())
        .select(SelectMode::Single)
        .selection(Bind::given(none))
        .natural_height(true)
        .show_items(ui, &rows, &mut |key, _| match key {
            Hit::Doc(d) => {
                let Some(doc) = docs.get(*d) else { return ListRow::new(Label::Empty) };
                ListRow::new(format!("{} ({})", doc.name, doc.found())).section_heading()
            }
            Hit::Match(d, m) => {
                let Some(hit) = lists.get(*d).and_then(|l| l.get(*m)) else { return ListRow::new(Label::Empty) };
                let page = crate::i18n::fmt(tl!("Page {label}"), &[("label", &hit.page.saturating_add(1).to_string())]);
                let name = format!("{page} …{} {} {}…", hit.before, hit.text, hit.after);
                ListRow::new(Label::Empty)
                    .lines(vec![
                        Span::new(format!("{page} …{} ", hit.before)).muted(),
                        Span::new(hit.text.clone()).highlight().strong(),
                        Span::new(format!(" {}…", hit.after)).muted(),
                    ])
                    .accessible_name(name)
                    .tooltip(tl!("Go to this result").to_string())
            }
        });
    for event in &out.events {
        let hit = match event {
            CollectionEvent::Selection(SelectionChange::Set(keys)) => keys.last(),
            CollectionEvent::Activate(key) => Some(key),
            _ => None,
        };
        if let Some(Hit::Match(d, m)) = hit
            && let Some(found) = lists.get(*d).and_then(|l| l.get(*m))
            && let Some(doc) = docs.get(*d)
        {
            action = Some(Action::Go(doc.key, found.page));
        }
    }
    action
}

impl PdfCraftApp {
    /// The tab with the view key `key` becomes the active one, at the 0-based `page`.
    pub(crate) fn go_to_open_doc_result(&mut self, key: u64, page: usize) {
        let Some(i) = self.tab_with_key(key) else {
            self.notify_tr("That document is no longer open");
            return;
        };
        // The panel shows the active tab's search, so the tab takes over the query and options being searched.
        let carried = self
            .active
            .and_then(|a| self.views.get(a))
            .and_then(|v| v.find.as_ref())
            .map(|f| (f.query.clone(), f.options.match_case, f.options.whole_word));
        self.active = Some(i);
        self.combine_tab.focused = false;
        if let Some(view) = self.views.get_mut(i) {
            if let Some((query, match_case, whole_word)) = carried {
                let find = view.find.get_or_insert_with(|| crate::canvas::Find::with(view.find_choices));
                (find.query, find.in_panel) = (query, true);
                (find.options.match_case, find.options.whole_word) = (match_case, whole_word);
                find.case_query.clear();
            }
            view.go_to_page(page);
        }
    }
}
