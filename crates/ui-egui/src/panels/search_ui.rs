//! Edit ▸ Advanced Search (⇧⌘F): the search in a side panel that lists every match with its
//! context, page by page; click a result to go to it. It shares the find bar's search.
// PLAN: assumed `super::right::PanelResult` (pdf-chrome-w13).

use std::collections::HashMap;

use craft::behavior::{DisplayOrder, ItemSource, Selection, SelectionChange};
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, CollectionEvent, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::layout_components::Row;
use craft::ui::list::{List, ListRow};
use craft::ui::runtime::env::Label;
use craft::ui::runtime::focus::{self, FocusTarget};
use craft::ui::style::Span;
use craft::ui::text::TextField;

use super::right::PanelResult;
use crate::PdfCraftApp;
use crate::canvas::Listed;
use crate::folder_search_ui::Action;
use crate::open_docs_search_ui::Action as OpenDocsAction;

/// Glyphs of context shown on each side of a match.
const CONTEXT: usize = 36;
/// The most results listed.
const MAX_RESULTS: usize = 2000;

/// A row of the results: a page's heading, the comments' or bookmarks' heading, or a match by its index in the search's
/// matches.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Hit {
    Page(usize),
    Comments,
    Bookmarks,
    Match(usize),
}

/// The results in display order.
struct Results {
    rows: Vec<Hit>,
    at: HashMap<Hit, usize>,
}

impl DisplayOrder<Hit> for Results {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn key(&self, at: usize) -> Hit {
        self.rows.get(at).cloned().unwrap_or(Hit::Page(0))
    }

    fn index_of(&self, key: &Hit) -> Option<usize> {
        self.at.get(key).copied()
    }
}

impl ItemSource<Hit> for Results {}

/// Draw the Search panel with its query, options, progress and clickable result excerpts.
pub(crate) fn show(app: &mut PdfCraftApp, ui: &mut egui::Ui) -> PanelResult {
    let mut result = PanelResult::default();
    let Some((index, id)) = app.active_ids() else { return result };
    let pages = app.session.get(id).map_or(0, |d| d.info.pages.len());
    let names: Vec<(u64, String)> = if app.open_docs_search.on {
        (0..app.views.len()).filter_map(|i| Some((app.views.get(i)?.view_key(), crate::canvas::tab_label(app, i)?))).collect()
    } else {
        Vec::new()
    };
    let folder = &mut app.folder_search;
    let open_docs = &mut app.open_docs_search;
    let Some(view) = app.views.get_mut(index) else { return result };
    let searched = view.session.texts.len() + view.session.text_failed.len();
    let Some(find) = view.find.as_mut() else { return result };
    find.in_panel = true;
    Text::new(tl!("What word or phrase would you like to search for?").to_string()).show(ui);
    let field =
        TextField::search(tl!("Search").to_string(), Bind::bound(&mut find.query)).placeholder(tl!("Search")).select_on_focus().submits().show(ui);
    // The bar was asked for (⇧⌘F, also with the panel open): the field takes the focus once.
    if find.bar.taken() {
        focus::request(ui.ctx(), FocusTarget::Id(field.response.id));
    }
    let before = (find.options.whole_word, find.options.match_case);
    Row::new().show(ui, |ui, _| {
        Checkbox::new(Bind::bound(&mut find.options.whole_word)).label(tl!("Whole words only").to_string()).show(ui);
        Checkbox::new(Bind::bound(&mut find.options.match_case)).label(tl!("Case-sensitive").to_string()).show(ui);
    });
    if before != (find.options.whole_word, find.options.match_case) {
        find.case_query.clear();
    }
    // Comments and bookmarks are not searched in a folder's files.
    if !folder.on {
        let before = (find.include_comments, find.include_bookmarks);
        Row::new().show(ui, |ui, _| {
            Checkbox::new(Bind::bound(&mut find.include_comments)).label(tl!("Include comments").to_string()).show(ui);
            Checkbox::new(Bind::bound(&mut find.include_bookmarks)).label(tl!("Include bookmarks").to_string()).show(ui);
        });
        // The page view lists them on its next pass.
        if before != (find.include_comments, find.include_bookmarks) {
            ui.ctx().request_repaint();
        }
    }
    if open_docs.on {
        let (query, case, whole) = (find.query.clone(), find.options.match_case, find.options.whole_word);
        let can_search = !query.trim().is_empty();
        let wanted = crate::open_docs_search_ui::Wanted {
            query,
            match_case: case,
            whole_word: whole,
            comments: find.include_comments,
            bookmarks: find.include_bookmarks,
        };
        open_docs.pass(&mut app.views, &app.session, &names, &wanted);
        let action = crate::open_docs_search_ui::section(ui, open_docs, can_search);
        return apply_open_docs(app, action, result);
    }
    if folder.on {
        let can_search = !find.query.trim().is_empty();
        let submitted = field.submitted && can_search && folder.folder.is_some() && !folder.running();
        let mut action = crate::folder_search_ui::section(ui, folder, can_search);
        if submitted && action.is_none() {
            action = Some(Action::Search);
        }
        return apply(app, action, result);
    }
    if Button::new(tl!("Search all PDFs in a folder…").to_string()).show(ui).clicked() {
        result.command = Some("edit.search_folder");
    }
    if Button::new(tl!("Search all open documents").to_string()).show(ui).clicked() {
        open_docs.on = true;
    }
    if find.query != find.case_query {
        view.rerun_find();
    }
    let Some(find) = view.find.as_ref() else { return result };
    let matches = find.matches.matches();
    let status = if find.query.trim().is_empty() {
        String::new()
    } else if matches.is_empty() && searched < pages {
        crate::i18n::fmt(tl!("Searching… {s} of {p} pages"), &[("s", &searched.to_string()), ("p", &pages.to_string())])
    } else {
        let n = matches.len();
        let more = if searched < pages {
            crate::i18n::fmt(tl!(" (searching… {s} of {p} pages)"), &[("s", &searched.to_string()), ("p", &pages.to_string())])
        } else {
            String::new()
        };
        // English keeps its singular; Chinese has no plural.
        let count = if n == 1 { crate::i18n::fmt(tl!("1 instance"), &[]) } else { crate::i18n::fmt(tl!("{n} instances"), &[("n", &n.to_string())]) };
        format!("{count}{more}")
    };
    if !status.is_empty() {
        Text::new(status).show(ui);
    }
    let mut rows = Vec::new();
    let mut last_heading = None;
    for (i, m) in matches.iter().enumerate().take(MAX_RESULTS) {
        let heading = match find.listed(m.page) {
            Some(Listed::Comment { .. }) => Hit::Comments,
            Some(Listed::Bookmark { .. }) => Hit::Bookmarks,
            None if view.session.texts.contains_key(&m.page) => Hit::Page(m.page),
            None => continue,
        };
        if last_heading.as_ref() != Some(&heading) {
            rows.push(heading.clone());
            last_heading = Some(heading);
        }
        rows.push(Hit::Match(i));
    }
    let at = rows.iter().cloned().enumerate().map(|(i, h)| (h, i)).collect();
    let results = Results { rows, at };
    let current = find.matches.current().map(|i| Selection::single(Hit::Match(i))).unwrap_or_default();
    let flat = |s: String| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let out = List::<Hit>::new(tl!("Search results").to_string())
        .select(SelectMode::Single)
        .selection(Bind::given(current))
        .natural_height(true)
        .show_items(ui, &results, &mut |key, _| match key {
            Hit::Page(p) => ListRow::new(crate::i18n::fmt(tl!("Page {label}"), &[("label", &(p + 1).to_string())])).section_heading(),
            Hit::Comments => ListRow::new(tl!("Comments").to_string()).section_heading(),
            Hit::Bookmarks => ListRow::new(tl!("Bookmarks").to_string()).section_heading(),
            Hit::Match(i) => {
                let Some(m) = matches.get(*i) else { return ListRow::new(Label::Empty) };
                let tip = tl!("Go to this result").to_string();
                match find.listed(m.page) {
                    Some(Listed::Comment { page, excerpt }) => {
                        let on = crate::i18n::fmt(tl!("Page {label}"), &[("label", &(page + 1).to_string())]);
                        return ListRow::new(format!("{on}: {excerpt}")).tooltip(tip);
                    }
                    Some(Listed::Bookmark { title }) => return ListRow::new(title).tooltip(tip),
                    None => {}
                }
                let Some(text) = view.session.texts.get(&m.page) else { return ListRow::new(Label::Empty) };
                let n = text.glyphs.len();
                let range = &m.range;
                let before = flat(text.text_of(range.start.saturating_sub(CONTEXT)..range.start.min(n)));
                let hit = flat(text.text_of(range.start.min(n)..range.end.min(n)));
                let after = flat(text.text_of(range.end.min(n)..range.end.saturating_add(CONTEXT).min(n)));
                let name = format!("…{before} {hit} {after}…");
                ListRow::new(Label::Empty)
                    .lines(vec![
                        Span::new(format!("…{before} ")).muted(),
                        Span::new(hit).highlight().strong(),
                        Span::new(format!(" {after}…")).muted(),
                    ])
                    .accessible_name(name)
                    .tooltip(tl!("Go to this result").to_string())
            }
        });
    let mut go = None;
    for event in &out.events {
        match event {
            CollectionEvent::Selection(SelectionChange::Set(keys)) => {
                if let Some(Hit::Match(i)) = keys.last() {
                    go = Some(*i);
                }
            }
            CollectionEvent::Activate(Hit::Match(i)) => go = Some(*i),
            _ => {}
        }
    }
    if let Some(i) = go {
        view.go_to_match(i);
    }
    result
}

/// What the folder scope's buttons asked for, done. The panel can't open files itself, and choosing a folder goes
/// through the registered command.
fn apply(app: &mut PdfCraftApp, action: Option<Action>, mut result: PanelResult) -> PanelResult {
    match action {
        Some(Action::Choose) => result.command = Some("edit.search_folder"),
        Some(Action::Search) => app.start_folder_search(),
        Some(Action::Stop) => app.cancel_folder_search(),
        Some(Action::ThisDocument) => app.folder_search.on = false,
        Some(Action::OpenDocuments) => {
            app.folder_search.on = false;
            app.open_docs_search.on = true;
        }
        Some(Action::Open(path, page)) => app.open_search_result(&path, page),
        None => {}
    }
    result
}

/// What the All open documents scope's buttons and results asked for, done.
fn apply_open_docs(app: &mut PdfCraftApp, action: Option<OpenDocsAction>, result: PanelResult) -> PanelResult {
    match action {
        Some(OpenDocsAction::ThisDocument) => {
            app.open_docs_search.on = false;
            for view in &mut app.views {
                view.session.hold_text = false;
            }
        }
        Some(OpenDocsAction::Go(key, page)) => app.go_to_open_doc_result(key, page),
        None => {}
    }
    result
}
