//! Edit ▸ Search all PDFs in a folder (`edit.search_folder`): the Search panel's Folder scope. Every PDF in a chosen
//! folder is searched on the host's job queue with the engine's [`folder_search`] (the matcher the find bar uses), the
//! matches are listed by file, and a click on one opens that file at its page. The search runs in the background with a
//! progress card and Cancel (also the panel's Stop); what it found before it was stopped stays listed and is marked.
//!
//! The query and the two options are the Search panel's own (the active document's find state), so a folder search and
//! a search of the document agree on what a match is.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use craft::behavior::{DisplayOrder, ItemSource, Selection, SelectionChange};
use craft::host::PickRequest;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, CollectionEvent, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::list::{List, ListRow};
use craft::ui::runtime::env::Label;
use craft::ui::style::{Span, TypeRole};
use pdfcraft_engine::folder_search::{self, FileOutcome, Match, Options, SkipReason, Summary};

use super::job_count::JobCount;
use crate::PdfCraftApp;
use crate::notices::{self, ProgressKey, ProgressNotice};

/// Matches one folder search lists; more are reported as capped.
const MAX_LISTED: usize = 1000;

/// Skipped files and unreadable folders listed with their reasons.
const MAX_SKIPPED_LISTED: usize = 20;

/// One file with at least one match.
pub struct FileHits {
    pub path: PathBuf,
    pub matches: Vec<Match>,
}

/// What a running search shares with its job: the count (with the cancel request) and the files done since the frame
/// last took them.
struct Run {
    progress: Arc<JobCount>,
    inbox: Arc<Mutex<Vec<FileOutcome>>>,
}

/// The folder scope of the Search panel and what its last search found.
#[derive(Default)]
pub struct FolderSearch {
    /// Search the folder instead of the document.
    pub on: bool,
    pub folder: Option<PathBuf>,
    /// Search the sub-folders too.
    pub recursive: bool,
    pub results: Vec<FileHits>,
    pub skipped: Vec<(PathBuf, SkipReason)>,
    pub unlistable: Vec<(PathBuf, String)>,
    pub summary: Option<Summary>,
    /// The search was stopped before it finished; what it found stays listed.
    pub stopped: bool,
    pub error: Option<String>,
    /// Files searched so far in a running search.
    pub files_done: usize,
    pages_failed: usize,
    run: Option<Run>,
}

impl FolderSearch {
    /// A search is running (or stopping).
    pub fn running(&self) -> bool {
        self.run.is_some()
    }

    /// Matches listed.
    pub fn found(&self) -> usize {
        self.results.iter().map(|f| f.matches.len()).sum()
    }

    /// The results of the last search are forgotten.
    fn clear(&mut self) {
        self.results.clear();
        self.skipped.clear();
        self.unlistable.clear();
        (self.summary, self.error, self.stopped, self.files_done, self.pages_failed) = (None, None, false, 0, 0);
    }

    /// Files the worker finished.
    fn absorb(&mut self, outcome: FileOutcome) {
        match outcome {
            FileOutcome::Searched { path, matches, pages_failed, .. } => {
                self.files_done += 1;
                self.pages_failed += pages_failed;
                if !matches.is_empty() {
                    self.results.push(FileHits { path, matches });
                }
            }
            FileOutcome::Skipped { path, reason } => self.skipped.push((path, reason)),
            FileOutcome::FolderUnlistable { path, error } => self.unlistable.push((path, error)),
        }
    }
}

/// What the panel asks for.
pub(crate) enum Action {
    /// Pick another folder.
    Choose,
    Search,
    Stop,
    /// Back to searching the document.
    ThisDocument,
    /// Open this file at the 0-based page.
    Open(PathBuf, usize),
}

impl PdfCraftApp {
    /// `edit.search_folder`: the Search panel opens on the Folder scope and asks for the folder to search.
    pub(crate) fn open_folder_search(&mut self) {
        if cfg!(target_arch = "wasm32") {
            self.notify_tr("Searching a folder is not available in the browser");
            return;
        }
        let Some(i) = self.active else { return };
        if let Some(view) = self.views.get_mut(i) {
            let find = view.find.get_or_insert_with(Default::default);
            find.in_panel = true;
            find.bar.open();
        }
        self.choose_right_panel(Some(crate::RightPanel::Search));
        self.folder_search.on = true;
        self.pick_first(PickRequest::folder("search.folder").title(tl!("Choose a folder to search").to_string()), None, |app, _, dir| {
            app.folder_search.folder = Some(dir);
            app.folder_search.on = true;
        });
    }

    /// Search the chosen folder for the Search panel's query, on a worker. Nothing starts while another search runs.
    pub fn start_folder_search(&mut self) {
        if self.folder_search.running() {
            self.notify_tr("A folder search is already running");
            return;
        }
        let Some(folder) = self.folder_search.folder.clone() else {
            self.notify_tr("Choose a folder to search first");
            return;
        };
        let Some(find) = self.active.and_then(|i| self.views.get(i)).and_then(|v| v.find.as_ref()) else { return };
        let query = find.query.clone();
        if query.trim().is_empty() {
            self.notify_tr("Type what to search for first");
            return;
        }
        let (case_sensitive, whole_words) = (find.options.match_case, find.options.whole_word);
        // A result nobody took (an earlier search that was dropped) is not this search's.
        while self.folder_search_jobs.try_recv().is_some() {}
        self.folder_search.clear();
        let options =
            Options { case_sensitive, whole_words, recursive: self.folder_search.recursive, max_results: MAX_LISTED, ..Options::new(query) };
        let progress = Arc::new(JobCount::default());
        let inbox: Arc<Mutex<Vec<FileOutcome>>> = Arc::default();
        let (count, sink, wake) = (progress.clone(), inbox.clone(), self.wake.clone());
        self.folder_search_jobs.submit((), Default::default(), move |_| {
            let mut done = 0usize;
            folder_search::search_folder(&folder, &options, &|| count.cancelled(), &mut |outcome| {
                if matches!(outcome, FileOutcome::Searched { .. }) {
                    done += 1;
                }
                sink.lock().unwrap_or_else(PoisonError::into_inner).push(outcome);
                count.set(done, 0);
                wake.wake();
            })
        });
        self.folder_search.run = Some(Run { progress, inbox });
        self.post_folder_search_progress();
    }

    /// Stop a running search; what it found stays listed.
    pub fn cancel_folder_search(&mut self) {
        let Some(run) = &self.folder_search.run else { return };
        run.progress.cancel();
        self.post_folder_search_progress();
    }

    /// Shows the running search's progress card. The number of files is not known until the listing is done, so the
    /// card counts files and its bar stays empty.
    fn post_folder_search_progress(&mut self) {
        let Some(run) = &self.folder_search.run else { return };
        let (done, _) = run.progress.read();
        let cancelling = run.progress.cancelled();
        let label = if cancelling {
            tl!("Cancelling…").to_string()
        } else {
            crate::i18n::fmt(tl!("Searching the folder… {n} files done"), &[("n", &done.to_string())])
        };
        notices::set_progress(self, ProgressKey::FolderSearch, ProgressNotice { label, fraction: 0.0, cancellable: !cancelling });
    }

    /// Takes the files the worker finished and, when it is done, ends the search and says how it went.
    pub(crate) fn poll_folder_search(&mut self) {
        let Some(run) = self.folder_search.run.as_ref() else { return };
        let (progress, inbox) = (run.progress.clone(), run.inbox.clone());
        // Taken before the files, so that every file of a finished search is in the box.
        let finished = self.folder_search_jobs.try_recv();
        let batch = std::mem::take(&mut *inbox.lock().unwrap_or_else(PoisonError::into_inner));
        for outcome in batch {
            self.folder_search.absorb(outcome);
        }
        let Some(finished) = finished else {
            if progress.take_change().is_some() {
                self.post_folder_search_progress();
            }
            return;
        };
        self.folder_search.run = None;
        notices::end_progress(self, ProgressKey::FolderSearch);
        match finished.result {
            Ok(summary) => {
                self.folder_search.stopped = summary.cancelled;
                let line = if summary.cancelled {
                    tl!("The folder search was stopped. What it found is listed.").to_string()
                } else {
                    count_line(self.folder_search.found(), summary.files_searched)
                };
                self.folder_search.summary = Some(summary);
                self.notify(line);
            }
            Err(e) => {
                self.folder_search.error = Some(e.clone());
                self.notify_fmt("The folder search failed: {e}", &[("e", &e)]);
            }
        }
    }

    /// Open `path` at the 0-based `page`. A name that is not UTF-8 can't be handed to the opener.
    pub(crate) fn open_search_result(&mut self, path: &Path, page: usize) {
        #[cfg(not(target_arch = "wasm32"))]
        match path.to_str() {
            Some(path) => {
                let _ = self.open_path_at_page(path, page);
            }
            None => self.notify_tr("This file's name can't be opened from here"),
        }
        #[cfg(target_arch = "wasm32")]
        let _ = (path, page);
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())
}

/// A skipped file's reason in the interface language.
fn reason_text(reason: &SkipReason) -> String {
    match reason {
        SkipReason::Link => tl!("A link is not followed").to_owned(),
        SkipReason::SpecialFile => tl!("Not a regular file").to_owned(),
        SkipReason::TooLarge(mib) => crate::i18n::fmt(tl!("Larger than {n} MiB"), &[("n", &mib.to_string())]),
        SkipReason::PasswordProtected => tl!("Password protected").to_owned(),
        SkipReason::Unreadable(detail) => format!("{}: {detail}", tl!("Could not be read")),
        SkipReason::Invalid(_) => tl!("Not a readable PDF").to_owned(),
        SkipReason::Unsupported(_) => tl!("Uses a feature that is not supported").to_owned(),
        SkipReason::NoPages => tl!("Has no pages").to_owned(),
        SkipReason::Failed(_) => tl!("Reading it failed unexpectedly").to_owned(),
    }
}

/// "{n} instances, {f} files searched".
fn count_line(found: usize, files: usize) -> String {
    let count =
        if found == 1 { crate::i18n::fmt(tl!("1 instance"), &[]) } else { crate::i18n::fmt(tl!("{n} instances"), &[("n", &found.to_string())]) };
    let files = crate::i18n::trn(crate::i18n::current(), files as u64, "{n} file searched", "{n} files searched");
    format!("{count}, {files}")
}

/// A row of the results: a file's heading, or a match by its file and its place in the file's matches.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Hit {
    File(usize),
    Match(usize, usize),
}

/// The results in display order.
struct Rows {
    rows: Vec<Hit>,
    at: HashMap<Hit, usize>,
}

impl Rows {
    fn of(results: &[FileHits]) -> Self {
        let mut rows = Vec::new();
        for (f, file) in results.iter().enumerate() {
            rows.push(Hit::File(f));
            rows.extend((0..file.matches.len()).map(|m| Hit::Match(f, m)));
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
        self.rows.get(at).cloned().unwrap_or(Hit::File(0))
    }

    fn index_of(&self, key: &Hit) -> Option<usize> {
        self.at.get(key).copied()
    }
}

impl ItemSource<Hit> for Rows {}

/// The Folder scope's controls and results, below the query and the options. `can_search` is whether the query has
/// text.
pub(crate) fn section(ui: &mut egui::Ui, fs: &mut FolderSearch, can_search: bool) -> Option<Action> {
    let mut action = None;
    Row::new().wrap(true).show(ui, |ui, _| {
        if Button::new(tl!("Choose folder…").to_string()).show(ui).clicked() {
            action = Some(Action::Choose);
        }
        Checkbox::new(Bind::bound(&mut fs.recursive)).label(tl!("Include sub-folders").to_string()).show(ui);
    });
    let place = fs.folder.as_ref().map_or_else(|| tl!("No folder chosen").to_string(), |p| p.to_string_lossy().into_owned());
    Text::new(place).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    let (running, ready) = (fs.running(), fs.folder.is_some() && can_search);
    Row::new().wrap(true).show(ui, |ui, _| {
        if running {
            if Button::new(tl!("Stop").to_string()).show(ui).clicked() {
                action = Some(Action::Stop);
            }
        } else if Button::new(tl!("Search folder").to_string()).enabled(ready).show(ui).clicked() {
            action = Some(Action::Search);
        }
        if Button::new(tl!("Search this document").to_string()).show(ui).clicked() {
            action = Some(Action::ThisDocument);
        }
    });
    if let Some(error) = &fs.error {
        Text::new(format!("{}: {error}", tl!("The search failed"))).show(ui);
    }
    let status = if running {
        crate::i18n::fmt(tl!("Searching, {n} files done"), &[("n", &fs.files_done.to_string())])
    } else if fs.stopped {
        crate::i18n::fmt(tl!("Stopped, {n} files searched"), &[("n", &fs.files_done.to_string())])
    } else if let Some(s) = &fs.summary {
        count_line(fs.found(), s.files_searched)
    } else {
        String::new()
    };
    if !status.is_empty() {
        Text::new(status).show(ui);
    }
    // Anything that stopped the search short of every page of every file.
    if fs.stopped || fs.pages_failed > 0 || fs.summary.as_ref().is_some_and(|s| !s.complete()) {
        Text::new(tl!("Not every file or match is listed").to_string()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    }
    if !fs.skipped.is_empty() {
        let title = match fs.skipped.len() {
            1 => tl!("1 file was skipped").to_owned(),
            n => crate::i18n::fmt(tl!("{n} files were skipped"), &[("n", &n.to_string())]),
        };
        Text::new(title).show(ui);
        for (path, reason) in fs.skipped.iter().take(MAX_SKIPPED_LISTED) {
            Text::new(format!("{}: {}", file_name(path), reason_text(reason))).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
        }
    }
    if !fs.unlistable.is_empty() {
        let title = match fs.unlistable.len() {
            1 => tl!("1 folder could not be listed").to_owned(),
            n => crate::i18n::fmt(tl!("{n} folders could not be listed"), &[("n", &n.to_string())]),
        };
        Text::new(title).show(ui);
        for (path, error) in fs.unlistable.iter().take(MAX_SKIPPED_LISTED) {
            Text::new(format!("{}: {error}", file_name(path))).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
        }
    }
    if fs.results.is_empty() {
        return action;
    }
    let rows = Rows::of(&fs.results);
    let results = &fs.results;
    let none: Selection<Hit> = Selection::default();
    let out = List::<Hit>::new(tl!("Folder search results").to_string())
        .select(SelectMode::Single)
        .selection(Bind::given(none))
        .natural_height(true)
        .show_items(ui, &rows, &mut |key, _| match key {
            Hit::File(f) => {
                let Some(file) = results.get(*f) else { return ListRow::new(Label::Empty) };
                ListRow::new(format!("{} ({})", file_name(&file.path), file.matches.len()))
                    .section_heading()
                    .tooltip(file.path.to_string_lossy().into_owned())
            }
            Hit::Match(f, m) => {
                let Some(hit) = results.get(*f).and_then(|file| file.matches.get(*m)) else { return ListRow::new(Label::Empty) };
                let page = crate::i18n::fmt(tl!("Page {label}"), &[("label", &hit.page.saturating_add(1).to_string())]);
                let name = format!("{page} …{} {} {}…", hit.before, hit.text, hit.after);
                ListRow::new(Label::Empty)
                    .lines(vec![
                        Span::new(format!("{page} …{} ", hit.before)).muted(),
                        Span::new(hit.text.clone()).highlight().strong(),
                        Span::new(format!(" {}…", hit.after)).muted(),
                    ])
                    .accessible_name(name)
                    .tooltip(tl!("Open the file at this result").to_string())
            }
        });
    for event in &out.events {
        let hit = match event {
            CollectionEvent::Selection(SelectionChange::Set(keys)) => keys.last(),
            CollectionEvent::Activate(key) => Some(key),
            _ => None,
        };
        if let Some(Hit::Match(f, m)) = hit
            && let Some((file, found)) = fs.results.get(*f).and_then(|file| file.matches.get(*m).map(|found| (file, found)))
        {
            action = Some(Action::Open(file.path.clone(), found.page));
        }
    }
    action
}
