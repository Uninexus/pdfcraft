// PLAN: the `craft` dependency in crates/ui-egui/Cargo.toml needs the `text` feature for `craft::text` (owner: pdf-hub-w1).
use craft::behavior::kurbo::Point;
use craft::behavior::overlay::Align;
use craft::pages::ui::TextMark;
use craft::text::find::{FindMatches, FindOptions, FindPattern};
use craft::text::ui::find::{FindBar, FindBarState, FindRequest};
use craft::ui::canvas_controls::TaskBar;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::layout_components::Row;

use std::collections::HashMap;

use pdfcraft_render::{DestView, OutlineItem};

use super::{DocView, Rect, ViewAction};

/// The first number a comment hit takes as its "page" in [`FindMatches`]: past every real page, so the
/// comments' hits follow the text hits in the count and in Next and Previous. Hit `i` is page `BASE + i`.
const COMMENT_PAGE_BASE: usize = usize::MAX / 2;
/// The most comments one search lists.
const MAX_COMMENT_HITS: usize = 2000;
/// The same for bookmark hits, past every comment's number, so that they come last.
const BOOKMARK_PAGE_BASE: usize = COMMENT_PAGE_BASE + usize::MAX / 4;
/// The most bookmarks one search lists, and the deepest it reads.
const MAX_BOOKMARK_HITS: usize = 2000;
const MAX_BOOKMARK_DEPTH: usize = 256;
/// The most characters of a found bookmark's title or comment's line shown.
const SHOWN_CHARS: usize = 80;

/// A comment the search found: its `(page, index)` and where it sits on its page (for scrolling to it).
#[derive(Clone, Debug, PartialEq)]
struct CommentHit {
    key: (usize, usize),
    rect: [f32; 4],
    /// The line of the thread that holds the query, shortened: the Search panel lists it.
    excerpt: String,
}

/// What a match that is not page text stands for, as the Search panel lists it.
pub(crate) enum Listed<'a> {
    /// A comment on page `page` (from 0).
    Comment {
        page: usize,
        excerpt: &'a str,
    },
    Bookmark {
        title: String,
    },
}

/// A bookmark the search found: its title and where it goes.
#[derive(Clone, Debug, PartialEq)]
struct BookmarkHit {
    title: String,
    page: usize,
    view: DestView,
}

/// What the reader chose in the find bar and Advanced Search: kept for the next document and the next run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindChoices {
    pub match_case: bool,
    pub whole_word: bool,
    pub include_comments: bool,
    pub include_bookmarks: bool,
}

/// The find bar (⌘F): query, matches across the document, current match.
#[derive(Default)]
pub struct Find {
    /// The choices as last handed to the app, so a change made here is told apart from one made elsewhere.
    seen: FindChoices,
    pub query: String,
    /// Find options (Acrobat's: case-sensitive, whole words only).
    pub options: FindOptions,
    /// The matches in document order and the current one; it owns following the search's start page.
    pub matches: FindMatches,
    /// Whether the bar was just asked for (⌘F, Advanced Search), so its field takes the keyboard.
    pub bar: FindBarState,
    /// The query the matches belong to.
    pub case_query: String,
    /// Shown in the Search panel (Advanced Search) instead of the find bar.
    pub in_panel: bool,
    /// Find and replace (in Edit text mode): the Replace row shows, with what replaces a match.
    pub replacing: bool,
    pub replacement: String,
    /// Replace also where the paragraph's font lacks a letter of the new text, in a substitute
    /// font (off: such a match is left alone and counted).
    pub substitute_font: bool,
    /// Also search the text of the document's comments and replies ("Include comments").
    pub include_comments: bool,
    /// The comments found, in page order; hit `i` stands in `matches` as page `COMMENT_PAGE_BASE + i`.
    comment_hits: Vec<CommentHit>,
    /// A comment the search just moved to, which the bar scrolls to once it has the document.
    reveal_comment: Option<CommentHit>,
    /// Also search the titles of the document's bookmarks ("Include bookmarks").
    pub include_bookmarks: bool,
    /// The bookmarks found, in outline order; hit `i` stands in `matches` as page `BOOKMARK_PAGE_BASE + i`.
    bookmark_hits: Vec<BookmarkHit>,
    /// A bookmark the search just moved to, which the bar follows once it has the document.
    reveal_bookmark: Option<BookmarkHit>,
}

impl Find {
    /// A new search that starts with the reader's `choices`.
    pub(crate) fn with(choices: FindChoices) -> Self {
        let mut find =
            Find { seen: choices, include_comments: choices.include_comments, include_bookmarks: choices.include_bookmarks, ..Default::default() };
        find.options.match_case = choices.match_case;
        find.options.whole_word = choices.whole_word;
        find
    }

    /// The choices as they stand, when they changed since the last call.
    pub(crate) fn changed_choices(&mut self) -> Option<FindChoices> {
        let now = FindChoices {
            match_case: self.options.match_case,
            whole_word: self.options.whole_word,
            include_comments: self.include_comments,
            include_bookmarks: self.include_bookmarks,
        };
        (now != self.seen).then(|| {
            self.seen = now;
            now
        })
    }

    /// The matches as highlights for the page view's text marks, the current one stronger.
    pub fn marks(&self) -> Vec<TextMark> {
        let current = self.matches.current();
        let marks = self.matches.matches().iter().enumerate().filter(|(_, m)| m.page < COMMENT_PAGE_BASE);
        marks.map(|(i, m)| TextMark::new(m.page, m.range.clone(), current == Some(i))).collect()
    }

    /// Forget the comments and bookmarks listed among the matches, after the matches were started over.
    pub(super) fn forget_listed_hits(&mut self) {
        self.comment_hits.clear();
        self.bookmark_hits.clear();
    }

    /// The bookmark a match stands for, when it is a bookmark's.
    fn bookmark_at(&self, page: usize) -> Option<&BookmarkHit> {
        self.bookmark_hits.get(page.checked_sub(BOOKMARK_PAGE_BASE)?)
    }

    /// The title of the bookmark that is the current match, shortened to fit the bar.
    fn current_bookmark(&self) -> Option<String> {
        Some(shortened(&self.bookmark_at(self.matches.current_match()?.page)?.title))
    }

    /// The comment or bookmark a match's `page` stands for; `None` for a real page.
    pub(crate) fn listed(&self, page: usize) -> Option<Listed<'_>> {
        if let Some(hit) = self.comment_at(page) {
            return Some(Listed::Comment { page: hit.key.0, excerpt: &hit.excerpt });
        }
        self.bookmark_at(page).map(|hit| Listed::Bookmark { title: shortened(&hit.title) })
    }

    /// The comment a match stands for, when it is a comment's.
    fn comment_at(&self, page: usize) -> Option<&CommentHit> {
        self.comment_hits.get(page.checked_sub(COMMENT_PAGE_BASE)?)
    }
}

impl DocView {
    /// Open the find bar (or focus it if already open).
    pub fn open_find(&mut self) {
        let choices = self.find_choices;
        let f = self.find.get_or_insert_with(|| Find::with(choices));
        f.bar.open();
    }

    /// Open the find bar with its Replace row (Edit text mode shows it): the Find field takes the
    /// keyboard while it is empty, else the Replace field.
    pub fn open_replace(&mut self) {
        let choices = self.find_choices;
        let f = self.find.get_or_insert_with(|| Find::with(choices));
        f.in_panel = false;
        f.replacing = true;
        f.bar.open_replace();
    }

    /// Re-run the search on all pages whose text is known (after the query changed).
    pub fn rerun_find(&mut self) {
        let current = self.view.current;
        let Some(f) = self.find.as_mut() else { return };
        f.case_query = f.query.clone();
        // Refining the query keeps the page the reader started searching from, even when an
        // automatic choice has already scrolled them to a match.
        let from = f.matches.from().unwrap_or(current);
        f.matches = FindMatches::new(from);
        // The new matches hold no comments or bookmarks: the next sync adds them again.
        f.forget_listed_hits();
        if f.case_query.trim().is_empty() {
            return;
        }
        let mut pages: Vec<usize> = self.session.texts.keys().copied().collect();
        pages.sort();
        let mut reveal = false;
        for &p in &pages {
            if let Some(text) = self.session.texts.get(&p) {
                reveal |= f.matches.set_page(p, text.find_opts(&f.case_query, f.options.match_case, f.options.whole_word));
            }
        }
        let target = f.matches.current_match().map(|m| m.page).filter(|_| reveal);
        if let Some(p) = target {
            self.flash_match(p);
        }
    }

    pub(super) fn refresh_find_page(&mut self, page: usize) {
        let (Some(f), Some(text)) = (self.find.as_mut(), self.session.texts.get(&page)) else { return };
        if f.case_query.trim().is_empty() {
            return;
        }
        let reveal = f.matches.set_page(page, text.find_opts(&f.case_query, f.options.match_case, f.options.whole_word));
        let target = f.matches.current_match().map(|m| m.page).filter(|_| reveal);
        if let Some(p) = target {
            self.flash_match(p);
        }
    }

    /// Keep the comments among the matches in step with the search and the document: with "Include comments" on,
    /// the comments whose text (or a reply's) holds the query, none when comments are hidden or of a hidden type.
    pub(super) fn sync_find_comments(&mut self, doc: &pdfcraft_engine::Document, editing: bool) {
        let Some(f) = self.find.as_mut() else { return };
        // The Search panel lists them also in Edit text mode, where only the find bar (with Replace) leaves them out.
        let on = f.include_comments && (f.in_panel || !editing) && !f.case_query.trim().is_empty();
        let hits = if on { find_comment_hits(doc, &f.case_query, &f.options) } else { Vec::new() };
        if hits == f.comment_hits {
            return;
        }
        for i in 0..f.comment_hits.len() {
            f.matches.set_page(COMMENT_PAGE_BASE + i, Vec::<std::ops::Range<usize>>::new());
        }
        for i in 0..hits.len() {
            f.matches.set_page(COMMENT_PAGE_BASE + i, std::iter::once(0usize..1).collect::<Vec<_>>());
        }
        f.comment_hits = hits;
    }

    /// The same for bookmarks: with "Include bookmarks" on, those whose title holds the query.
    pub(super) fn sync_find_bookmarks(&mut self, doc: &pdfcraft_engine::Document, editing: bool) {
        let Some(f) = self.find.as_mut() else { return };
        let on = f.include_bookmarks && (f.in_panel || !editing) && !f.case_query.trim().is_empty();
        let hits = if on { find_bookmark_hits(&doc.info.outline, &f.case_query, &f.options) } else { Vec::new() };
        if hits == f.bookmark_hits {
            return;
        }
        for i in 0..f.bookmark_hits.len() {
            f.matches.set_page(BOOKMARK_PAGE_BASE + i, Vec::<std::ops::Range<usize>>::new());
        }
        for i in 0..hits.len() {
            f.matches.set_page(BOOKMARK_PAGE_BASE + i, std::iter::once(0usize..1).collect::<Vec<_>>());
        }
        f.bookmark_hits = hits;
    }

    /// Go where the bookmark the search moved to goes, now that the document is at hand.
    fn reveal_found_bookmark(&mut self, info: &pdfcraft_render::DocInfo) {
        let Some(hit) = self.find.as_mut().and_then(|f| f.reveal_bookmark.take()) else { return };
        self.go_to_dest(hit.page, hit.view, info);
    }

    /// Scroll to the comment the search moved to, now that the document is at hand.
    fn reveal_found_comment(&mut self, info: &pdfcraft_render::DocInfo) {
        let Some(hit) = self.find.as_mut().and_then(|f| f.reveal_comment.take()) else { return };
        self.reveal_rect(hit.key.0, hit.rect, info);
        self.session.flash = Some((hit.key.0, hit.rect, 0.0));
    }

    /// Go to match `i` (the Search panel's results).
    pub fn go_to_match(&mut self, i: usize) {
        let Some(f) = self.find.as_mut() else { return };
        if i >= f.matches.len() {
            return;
        }
        f.matches.pick(i);
        self.show_current_match();
    }

    /// Move to the next/previous match.
    pub fn find_step(&mut self, forward: bool) {
        let Some(f) = self.find.as_mut() else { return };
        if f.matches.step(!forward).is_none() {
            return;
        }
        self.show_current_match();
    }

    /// Show the current match, which the matches already chose (the find bar and the Search panel
    /// step or pick it). Only explicit search navigation joins history; background refresh uses
    /// `flash_match` alone.
    pub(super) fn show_current_match(&mut self) {
        let Some(p) = self.find.as_ref().and_then(|f| f.matches.current_match()).map(|m| m.page) else { return };
        // A comment: select it as a click in the Comments panel does, and scroll to it once the bar has the document.
        if let Some(hit) = self.find.as_ref().and_then(|f| f.comment_at(p)).cloned() {
            self.session.comments.select_keys(&[hit.key]);
            self.session.comments.reveal = true;
            self.go_to_page(hit.key.0);
            if let Some(f) = self.find.as_mut() {
                f.reveal_comment = Some(hit);
            }
            return;
        }
        // A bookmark: go where it goes once the bar has the document.
        if let Some(hit) = self.find.as_ref().and_then(|f| f.bookmark_at(p)).cloned() {
            if let Some(f) = self.find.as_mut() {
                f.reveal_bookmark = Some(hit);
            }
            return;
        }
        self.go_to_page(p);
        self.flash_match(p);
    }

    /// The centre of the current match in page points on its displayed page (from the top-left
    /// corner, after the view rotation), where the page view's go-to point takes it.
    fn match_centre(&self) -> Option<Point> {
        let m = self.find.as_ref()?.matches.current_match()?;
        let glyphs = self.session.texts.get(&m.page)?.glyphs.get(m.range.clone())?;
        let [x0, y0, x1, y1] = glyphs.iter().map(|g| g.rect).reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])?;
        let &(w, h) = self.page_sizes.get(m.page).filter(|(w, h)| *w > 0.0 && *h > 0.0)?;
        let (w, h) = (f64::from(w), f64::from(h));
        let cx = (f64::from(x0 + x1) / 2.0).clamp(0.0, w);
        let cy = (f64::from(y0 + y1) / 2.0).clamp(0.0, h);
        // Turned clockwise by the view rotation.
        Some(match self.view.rotation % 360 {
            90 => Point::new(h - cy, cx),
            180 => Point::new(w - cx, h - cy),
            270 => Point::new(cy, w - cx),
            _ => Point::new(cx, cy),
        })
    }

    fn flash_match(&mut self, page: usize) {
        if page >= COMMENT_PAGE_BASE {
            return;
        }
        // Put the match's centre at the window's centre, as Acrobat does (#3530) and as a /FitR
        // destination's rectangle is: a page wider or taller than the window no longer leaves it
        // off to one side or near an edge.
        // Finite only: a hostile page's glyphs can span -inf..inf, and NaN survives `clamp`.
        if let Some(centre) = self.match_centre().filter(|c| c.x.is_finite() && c.y.is_finite()) {
            self.view.go_to_point(page, centre, (0.5, 0.5));
        } else {
            // The match line's top, centred on that axis alone (a NaN axis keeps its offset).
            let top = self
                .find
                .as_ref()
                .and_then(|f| f.matches.current_match())
                .and_then(|m| self.session.texts.get(&m.page).and_then(|t| t.glyphs.get(m.range.start)))
                .map(|g| f64::from(g.rect[1]))
                .zip(self.page_sizes.get(page).map(|&(_, h)| f64::from(h)))
                .filter(|(y, h)| y.is_finite() && h.is_finite() && *h > 0.0);
            match top {
                Some((y, h)) => self.view.go_to_point(page, Point::new(f64::NAN, y.clamp(0.0, h)), (0.5, 0.5)),
                None => self.view.go_to(page),
            }
        }
    }

    /// Ends automatic search following after the reader navigated or scrolled the view. A
    /// programmatic search flash never comes through here, so that its own scroll cannot be
    /// mistaken for the reader's.
    pub(super) fn end_find_following(&mut self) {
        if let Some(f) = self.find.as_mut() {
            f.matches.stop_following();
        }
    }
}

/// The comments whose text, or a reply's, holds `query` under the find `options` (the find bar's own matcher, so match case
/// and whole words apply), in page order. Nothing while comments are hidden; a comment of a hidden type is skipped.
fn find_comment_hits(doc: &pdfcraft_engine::Document, query: &str, options: &FindOptions) -> Vec<CommentHit> {
    if doc.comments_hidden() {
        return Vec::new();
    }
    let Ok(pattern) = FindPattern::compile(query, options) else { return Vec::new() };
    let info = &doc.info;
    let hidden = doc.hidden_comment_types();
    // Each thread's text: the comment's and its replies' (not status changes).
    let tops = pdfcraft_render::thread_tops(&info.annotations);
    let mut texts: HashMap<(usize, usize), String> = HashMap::new();
    for a in info.annotations.iter().filter(|a| a.state.is_none()) {
        let key = (a.page, a.index);
        if let Some(text) = a.contents.as_deref().filter(|t| !t.is_empty()) {
            let all = texts.entry(tops.get(&key).copied().unwrap_or(key)).or_default();
            if !all.is_empty() {
                all.push('\n');
            }
            all.push_str(text);
        }
    }
    let mut hits: Vec<CommentHit> = info
        .annotations
        .iter()
        .filter(|a| a.is_comment() && !a.is_group_member() && !hidden.contains(&a.subtype))
        .filter_map(|a| {
            let text = texts.get(&(a.page, a.index))?;
            if pattern.find_all(text, 1).is_empty() {
                return None;
            }
            // A match across two lines has no single line to show: the thread's first stands for it.
            let line = text.lines().find(|line| !pattern.find_all(line, 1).is_empty()).or_else(|| text.lines().next());
            let rect = a.shown_rect(info.pages.get(a.page).map_or(0, |p| p.rotation));
            Some(CommentHit { key: (a.page, a.index), rect, excerpt: shortened(line.unwrap_or_default()) })
        })
        .collect();
    hits.sort_by_key(|h| h.key);
    hits.truncate(MAX_COMMENT_HITS);
    hits
}

/// `text` as one line the bar or the panel can show.
fn shortened(text: &str) -> String {
    let mut shown: String = text.chars().take(SHOWN_CHARS).collect();
    if shown.len() < text.len() {
        shown.push('…');
    }
    shown
}

/// The bookmarks whose title holds `query` under the find `options`, in outline order. A bookmark that goes to no
/// page of this document (a web address, another file) is left out: finding it must not run it.
fn find_bookmark_hits(outline: &[OutlineItem], query: &str, options: &FindOptions) -> Vec<BookmarkHit> {
    fn walk(items: &[OutlineItem], depth: usize, pattern: &FindPattern, out: &mut Vec<BookmarkHit>) {
        if depth >= MAX_BOOKMARK_DEPTH {
            return;
        }
        for item in items {
            if out.len() >= MAX_BOOKMARK_HITS {
                return;
            }
            if let Some(page) = item.page
                && !pattern.find_all(&item.title, 1).is_empty()
            {
                out.push(BookmarkHit { title: item.title.clone(), page, view: item.view });
            }
            walk(&item.children, depth + 1, pattern, out);
        }
    }
    let mut hits = Vec::new();
    if let Ok(pattern) = FindPattern::compile(query, options) {
        walk(outline, 0, &pattern, &mut hits);
    }
    hits
}

/// The comments and bookmarks of `doc` that hold `query`, for a list of results: where each is and what it says,
/// labelled as the Search panel labels them.
pub(crate) fn listed_matches(
    doc: &pdfcraft_engine::Document,
    query: &str,
    options: &FindOptions,
    comments: bool,
    bookmarks: bool,
) -> Vec<pdfcraft_engine::folder_search::Match> {
    let listed = |page, kind: &str, text| {
        let from = pdfcraft_engine::folder_search::Found::Page;
        pdfcraft_engine::folder_search::Match { from, page, before: format!("{kind}:"), text, after: String::new() }
    };
    let mut found = Vec::new();
    if comments {
        found.extend(find_comment_hits(doc, query, options).into_iter().map(|hit| listed(hit.key.0, tl!("Comment"), hit.excerpt)));
    }
    if bookmarks {
        let hits = find_bookmark_hits(&doc.info.outline, query, options);
        found.extend(hits.into_iter().map(|hit| listed(hit.page, tl!("Bookmark"), shortened(&hit.title))));
    }
    found
}

/// The find bar at the page view's top edge, end-aligned. In Edit text mode (`editing`) it offers a
/// Replace row: Replace changes the highlighted match, Replace all every match, each inside its own
/// paragraph.
pub(super) fn find_bar(view: &mut DocView, doc: &pdfcraft_engine::Document, area: Rect, editing: bool, ui: &mut egui::Ui) {
    let pages = doc.info.pages.len();
    view.sync_find_comments(doc, editing);
    view.reveal_found_comment(&doc.info);
    view.sync_find_bookmarks(doc, editing);
    view.reveal_found_bookmark(&doc.info);
    let searched = view.session.texts.len() + view.session.text_failed.len();
    let key = view.pane.key;
    let Some(find) = view.find.as_mut() else { return };
    // The Search panel shows this search.
    if find.in_panel {
        return;
    }
    // One id per pane, so two panes' bars do not share a field id (and so the focus).
    let mut included_changed = false;
    let bookmark = find.current_bookmark();
    let shown = ui
        .push_id(("find-bar", key), |ui| {
            TaskBar::at_edge(tl!("Find"), Align::End).show(ui, area.shrink2(egui::vec2(6.0, 12.0)), |ui, _cx| {
                let Find { query, options, matches, bar, replacement, replacing, substitute_font, include_comments, include_bookmarks, .. } =
                    &mut *find;
                // Read before the bar borrows it: the bar may switch the row on in this pass, and the
                // checkbox then follows on the repaint asked for below.
                let replacing_before = *replacing;
                let mut find_bar = FindBar::new(query, options, matches).state(bar).progress(searched, pages);
                if editing {
                    find_bar = find_bar.replace(replacement, replacing);
                }
                let out = ui
                    .vertical(|ui| {
                        let out = find_bar.show(ui);
                        // The bar's options menu is the framework's, so the Replace-only option sits under
                        // the Replace row: off, a paragraph whose font lacks a letter of the new text is
                        // left alone and counted.
                        if editing && replacing_before {
                            Row::new().show(ui, |ui, _| {
                                Checkbox::new(Bind::bound(&mut *substitute_font))
                                    .label(tl!("Use a substitute font if the font lacks letters").to_string())
                                    .show(ui);
                            });
                        }
                        // Off, a search finds page text only; on, it also finds comments (and replies) or bookmark titles
                        // holding the query.
                        if !editing {
                            let before = (*include_comments, *include_bookmarks);
                            Row::new().show(ui, |ui, _| {
                                Checkbox::new(Bind::bound(&mut *include_comments)).label(tl!("Include comments").to_string()).show(ui);
                            });
                            Row::new().show(ui, |ui, _| {
                                Checkbox::new(Bind::bound(&mut *include_bookmarks)).label(tl!("Include bookmarks").to_string()).show(ui);
                            });
                            included_changed = before != (*include_comments, *include_bookmarks);
                            // A bookmark has no highlight on the page: say which one the current match is.
                            if let Some(title) = &bookmark {
                                Text::new(crate::i18n::fmt(tl!("Bookmark: {title}"), &[("title", title)])).show(ui);
                            }
                        }
                        out
                    })
                    .inner;
                if *replacing != replacing_before {
                    ui.ctx().request_repaint();
                }
                out
            })
        })
        .inner;
    let Some(out) = shown.inner else { return };
    if out.closed {
        view.find = None;
        return;
    }
    match out.request {
        Some(FindRequest::Replace { .. }) => view.pending_action = Some(ViewAction::Replace { all: false }),
        Some(FindRequest::ReplaceAll { .. }) => view.pending_action = Some(ViewAction::Replace { all: true }),
        _ => {}
    }
    if out.search || included_changed {
        view.rerun_find();
        view.sync_find_comments(doc, editing);
        view.sync_find_bookmarks(doc, editing);
    }
    if out.stepped {
        // The bar already stepped the matches.
        view.show_current_match();
        // The bookmark line was laid out before this step.
        ui.ctx().request_repaint();
    }
}
