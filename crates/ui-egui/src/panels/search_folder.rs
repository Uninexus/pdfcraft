//! The Search panel's Folder scope: every PDF in a chosen folder is searched on a worker thread
//! with the engine's `folder_search` (the matcher the find bar uses), and the matches are listed
//! by file. A double-click opens the file at that page.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use egui::text::LayoutJob;
use egui::{Color32, FontId, TextFormat};
use pdfcraft_engine::folder_search::{self, FileOutcome, Match, Options, SkipReason, Summary};

use crate::theme::{self, Tokens};
use crate::{PdfCraftApp, widgets};

/// Matches one folder search lists; more are reported as capped.
const MAX_LISTED: usize = 1000;

/// Skipped files listed with their reasons.
const MAX_SKIPPED_LISTED: usize = 50;

/// One file with at least one match.
pub struct FileHits {
    pub path: PathBuf,
    pub matches: Vec<Match>,
}

/// What the panel asks the app to do (it can't open files or show pickers itself).
pub(crate) enum PanelAction {
    ChooseFolder,
    /// Open `path` at the 0-based `page`.
    Open {
        path: PathBuf,
        page: usize,
    },
}

enum Msg {
    File(FileOutcome),
    Done(Result<Summary, String>),
}

struct Job {
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    /// Set by the worker as its last act.
    finished: Arc<AtomicBool>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Messages taken from the worker in one frame; more wait for the next (a repaint is asked for).
const POLL_BATCH: usize = 512;

/// The folder scope of one document's Search panel. It lives on the document's view, so closing
/// the find bar or pressing Escape does not end a running search.
#[derive(Default)]
pub struct FolderScope {
    /// Search the folder instead of this document.
    pub on: bool,
    pub folder: Option<PathBuf>,
    pub recursive: bool,
    pub results: Vec<FileHits>,
    pub skipped: Vec<(PathBuf, SkipReason)>,
    pub unlistable: Vec<(PathBuf, String)>,
    pub summary: Option<Summary>,
    /// The search was stopped before it finished; what it found stays listed.
    pub stopped: bool,
    pub error: Option<String>,
    files_done: usize,
    pages_failed: usize,
    job: Option<Job>,
    /// A stopped worker's flag: it may still be finishing a file, and a new search waits for it.
    winding_down: Option<Arc<AtomicBool>>,
}

impl FolderScope {
    pub fn running(&self) -> bool {
        self.job.is_some()
    }

    /// A stopped worker has not finished yet.
    pub fn winding_down(&self) -> bool {
        self.winding_down.as_ref().is_some_and(|f| !f.load(Ordering::Acquire))
    }

    /// Stop a running search. What it found stays listed, marked as stopped.
    pub fn stop(&mut self) {
        if let Some(job) = self.job.take() {
            self.winding_down = Some(job.finished.clone());
            self.stopped = true;
        }
    }

    /// Search `folder` for `query` on a worker thread, replacing the listed results. Nothing
    /// starts while a stopped worker is still finishing.
    pub fn start(&mut self, ctx: &egui::Context, query: &str, case_sensitive: bool, whole_words: bool) {
        let Some(folder) = self.folder.clone() else { return };
        if self.running() || self.winding_down() {
            return;
        }
        self.winding_down = None;
        self.results.clear();
        self.skipped.clear();
        self.unlistable.clear();
        (self.summary, self.error, self.stopped, self.files_done, self.pages_failed) = (None, None, false, 0, 0);
        let options = Options { case_sensitive, whole_words, recursive: self.recursive, max_results: MAX_LISTED, ..Options::new(query) };
        let (tx, rx) = channel();
        let (cancel, finished) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let (stop, done, ctx) = (cancel.clone(), finished.clone(), ctx.clone());
        let work = move || {
            let result = folder_search::search_folder(&folder, &options, &|| stop.load(Ordering::Relaxed), &mut |outcome| {
                let _ = tx.send(Msg::File(outcome));
                ctx.request_repaint();
            });
            let _ = tx.send(Msg::Done(result));
            done.store(true, Ordering::Release);
            ctx.request_repaint();
        };
        match std::thread::Builder::new().name("pdfcraft-folder-search".into()).spawn(work) {
            Ok(_) => self.job = Some(Job { rx, cancel, finished }),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Take what the worker has sent since the last frame (at most [`POLL_BATCH`] messages; a
    /// repaint is asked for when more may wait).
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.job else { return };
        let mut done = false;
        let mut drained = false;
        for _ in 0..POLL_BATCH {
            match job.rx.try_recv() {
                Ok(Msg::File(FileOutcome::Searched { path, matches, pages_failed, .. })) => {
                    self.files_done += 1;
                    self.pages_failed += pages_failed;
                    if !matches.is_empty() {
                        self.results.push(FileHits { path, matches });
                    }
                }
                Ok(Msg::File(FileOutcome::Skipped { path, reason })) => self.skipped.push((path, reason)),
                Ok(Msg::File(FileOutcome::FolderUnlistable { path, error })) => self.unlistable.push((path, error)),
                Ok(Msg::Done(result)) => {
                    match result {
                        Ok(summary) => self.summary = Some(summary),
                        Err(e) => self.error = Some(e),
                    }
                    done = true;
                    break;
                }
                Err(TryRecvError::Empty) => {
                    drained = true;
                    break;
                }
                Err(TryRecvError::Disconnected) => {
                    self.error = Some(tl!("The search stopped unexpectedly").to_owned());
                    done = true;
                    break;
                }
            }
        }
        if done {
            self.job = None;
        } else if !drained {
            ctx.request_repaint();
        }
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

/// The count line: "{n} instances - {f} files searched".
fn count_line(found: usize, files: usize) -> String {
    let count =
        if found == 1 { crate::i18n::fmt(tl!("1 instance"), &[]) } else { crate::i18n::fmt(tl!("{n} instances"), &[("n", &found.to_string())]) };
    let files =
        if files == 1 { tl!("1 file searched").to_owned() } else { crate::i18n::fmt(tl!("{n} files searched"), &[("n", &files.to_string())]) };
    format!("{count} - {files}")
}

/// The Folder scope's controls and results, below the query and options.
pub(crate) fn section(
    ui: &mut egui::Ui,
    t: &Tokens,
    scope: &mut FolderScope,
    query: &str,
    case_sensitive: bool,
    whole_words: bool,
) -> Option<PanelAction> {
    let mut action = None;
    scope.poll(ui.ctx());
    if scope.running() {
        // A search that is quiet for a while still shows its progress.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
    }
    ui.horizontal(|ui| {
        if ui.button(tl!("Choose folder")).clicked() {
            action = Some(PanelAction::ChooseFolder);
        }
        ui.add(widgets::checkbox(&mut scope.recursive, tl!("Include sub-folders")));
    });
    match &scope.folder {
        Some(folder) => ui.label(egui::RichText::new(folder.to_string_lossy()).small().color(t.text_muted)),
        None => ui.label(egui::RichText::new(tl!("No folder chosen")).small().color(t.text_faint)),
    };
    ui.horizontal(|ui| {
        let ready = scope.folder.is_some() && !query.trim().is_empty();
        if scope.running() {
            if ui.button(tl!("Stop")).clicked() {
                scope.stop();
            }
        } else if scope.winding_down() {
            ui.add_enabled(false, egui::Button::new(tl!("Stopping")));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        } else if ui.add_enabled(ready, egui::Button::new(tl!("Search"))).clicked() {
            scope.start(ui.ctx(), query, case_sensitive, whole_words);
        }
    });
    if let Some(error) = &scope.error {
        ui.colored_label(ui.visuals().error_fg_color, format!("{}: {error}", tl!("The search failed")));
    }
    let found: usize = scope.results.iter().map(|f| f.matches.len()).sum();
    let status = if scope.running() {
        crate::i18n::fmt(tl!("Searching, {n} files done"), &[("n", &scope.files_done.to_string())])
    } else if scope.stopped {
        crate::i18n::fmt(tl!("Stopped, {n} files searched"), &[("n", &scope.files_done.to_string())])
    } else if let Some(s) = &scope.summary {
        count_line(found, s.files_searched)
    } else {
        String::new()
    };
    ui.add_space(6.0);
    ui.label(egui::RichText::new(status).font(theme::semibold(12.5)));
    // Anything that stopped the search short of every page of every file.
    let cut = scope.stopped || scope.pages_failed > 0 || scope.summary.as_ref().is_some_and(|s| !s.complete());
    if cut {
        ui.label(egui::RichText::new(tl!("Not every file or match is listed")).small().color(t.text_faint));
    }
    if !scope.skipped.is_empty() {
        let title = match scope.skipped.len() {
            1 => tl!("1 file was skipped").to_owned(),
            n => crate::i18n::fmt(tl!("{n} files were skipped"), &[("n", &n.to_string())]),
        };
        egui::CollapsingHeader::new(title).id_salt("folder-search-skipped").show(ui, |ui| {
            for (path, reason) in scope.skipped.iter().take(MAX_SKIPPED_LISTED) {
                ui.label(egui::RichText::new(format!("{}: {}", file_name(path), reason_text(reason))).small().color(t.text_muted));
            }
        });
    }
    if !scope.unlistable.is_empty() {
        let title = match scope.unlistable.len() {
            1 => tl!("1 folder could not be listed").to_owned(),
            n => crate::i18n::fmt(tl!("{n} folders could not be listed"), &[("n", &n.to_string())]),
        };
        egui::CollapsingHeader::new(title).id_salt("folder-search-unlistable").show(ui, |ui| {
            for (path, error) in scope.unlistable.iter().take(MAX_SKIPPED_LISTED) {
                ui.label(egui::RichText::new(format!("{}: {error}", file_name(path))).small().color(t.text_muted));
            }
        });
    }
    ui.add_space(4.0);
    for (k, file) in scope.results.iter().enumerate() {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(format!("{} ({})", file_name(&file.path), file.matches.len())).font(theme::semibold(12.5)))
            .on_hover_text(file.path.to_string_lossy());
        for (j, m) in file.matches.iter().enumerate() {
            let mut job = LayoutJob::default();
            let plain = TextFormat { font_id: FontId::new(12.5, egui::FontFamily::Proportional), color: t.text_muted, ..Default::default() };
            let page = crate::i18n::fmt(tl!("Page {label}"), &[("label", &m.page.saturating_add(1).to_string())]);
            job.append(&format!("{page}  ...{} ", m.before), 0.0, plain.clone());
            job.append(
                &m.text,
                0.0,
                TextFormat {
                    font_id: theme::semibold(12.5),
                    color: t.text,
                    background: Color32::from_rgba_unmultiplied(255, 214, 0, 90),
                    ..Default::default()
                },
            );
            job.append(&format!(" {}...", m.after), 0.0, plain);
            job.wrap.max_width = ui.available_width();
            let resp = ui.push_id((k, j), |ui| ui.add(egui::Button::selectable(false, job).wrap())).inner;
            let resp = resp.on_hover_text(tl!("Double-click to open the file at this page"));
            if resp.double_clicked() {
                action = Some(PanelAction::Open { path: file.path.clone(), page: m.page });
            }
        }
    }
    action
}

impl PdfCraftApp {
    /// Ask for the folder the active document's Search panel searches. The answer goes to the
    /// document that was in front when the picker opened, whatever is in front when it closes.
    pub(crate) fn choose_search_folder(&mut self) {
        let Some((_, id)) = self.active_ids() else { return };
        let dialog = rfd::AsyncFileDialog::new();
        self.ask_one(crate::pickers::Ask::Folder(dialog), None, move |app, dir| {
            if let Some(view) = app.views.iter_mut().find(|v| v.id == id) {
                view.folder_search.folder = Some(dir);
            }
        });
    }

    /// Open `path` at the 0-based `page`. A name that is not UTF-8 can't be handed to the opener.
    pub(crate) fn open_search_result(&mut self, path: &Path, page: usize) {
        match path.to_str() {
            Some(path) => {
                let _ = self.open_path_at_page(path, page);
            }
            None => self.notify_tr("This file's name can't be opened from here"),
        }
    }
}
