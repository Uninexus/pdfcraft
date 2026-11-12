//! The Search panel's Folder scope: every PDF in a chosen folder is searched on a worker thread
//! with the engine's `folder_search` (the matcher the find bar uses), and the matches are listed
//! by file. A double-click opens the file at that page.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use egui::text::LayoutJob;
use egui::{Color32, FontId, TextFormat};
use pdfcraft_engine::folder_search::{self, FileOutcome, Match, Options, Summary};

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
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The folder scope of one document's Search panel.
#[derive(Default)]
pub struct FolderScope {
    /// Search the folder instead of this document.
    pub on: bool,
    pub folder: Option<PathBuf>,
    pub recursive: bool,
    pub results: Vec<FileHits>,
    pub skipped: Vec<(PathBuf, String)>,
    pub summary: Option<Summary>,
    pub error: Option<String>,
    files_done: usize,
    job: Option<Job>,
}

impl FolderScope {
    pub fn running(&self) -> bool {
        self.job.is_some()
    }

    /// Stop a running search (what it found stays listed).
    pub fn stop(&mut self) {
        self.job = None;
    }

    /// Search `folder` for `query` on a worker thread, replacing the listed results.
    pub fn start(&mut self, ctx: &egui::Context, query: &str, case_sensitive: bool, whole_words: bool) {
        let Some(folder) = self.folder.clone() else { return };
        self.job = None;
        (self.results, self.skipped, self.summary, self.error, self.files_done) = (Vec::new(), Vec::new(), None, None, 0);
        let options = Options { case_sensitive, whole_words, recursive: self.recursive, max_results: MAX_LISTED, ..Options::new(query) };
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (stop, ctx) = (cancel.clone(), ctx.clone());
        let work = move || {
            let result = folder_search::search_folder(&folder, &options, &|| stop.load(Ordering::Relaxed), &mut |outcome| {
                let _ = tx.send(Msg::File(outcome));
                ctx.request_repaint();
            });
            let _ = tx.send(Msg::Done(result));
            ctx.request_repaint();
        };
        match std::thread::Builder::new().name("pdfcraft-folder-search".into()).spawn(work) {
            Ok(_) => self.job = Some(Job { rx, cancel }),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Take what the worker has sent since the last frame.
    pub fn poll(&mut self) {
        let Some(job) = &self.job else { return };
        let mut done = false;
        for _ in 0..256 {
            match job.rx.try_recv() {
                Ok(Msg::File(FileOutcome::Searched { path, matches, .. })) => {
                    self.files_done += 1;
                    if !matches.is_empty() {
                        self.results.push(FileHits { path, matches });
                    }
                }
                Ok(Msg::File(FileOutcome::Skipped { path, reason })) => self.skipped.push((path, reason)),
                Ok(Msg::Done(result)) => {
                    match result {
                        Ok(summary) => self.summary = Some(summary),
                        Err(e) => self.error = Some(e),
                    }
                    done = true;
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.error = Some(tl!("The search stopped unexpectedly").to_owned());
                    done = true;
                    break;
                }
            }
        }
        if done {
            self.job = None;
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())
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
    scope.poll();
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
        } else if ui.add_enabled(ready, egui::Button::new(tl!("Search"))).clicked() {
            scope.start(ui.ctx(), query, case_sensitive, whole_words);
        }
    });
    if let Some(error) = &scope.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    let found: usize = scope.results.iter().map(|f| f.matches.len()).sum();
    let status = if scope.running() {
        crate::i18n::fmt(tl!("Searching, {n} files done"), &[("n", &scope.files_done.to_string())])
    } else if scope.summary.is_some() {
        let count =
            if found == 1 { crate::i18n::fmt(tl!("1 instance"), &[]) } else { crate::i18n::fmt(tl!("{n} instances"), &[("n", &found.to_string())]) };
        let files = scope.summary.as_ref().map_or(0, |s| s.files_searched);
        let files =
            if files == 1 { tl!("1 file searched").to_owned() } else { crate::i18n::fmt(tl!("{n} files searched"), &[("n", &files.to_string())]) };
        format!("{count} - {files}")
    } else {
        String::new()
    };
    ui.add_space(6.0);
    ui.label(egui::RichText::new(status).font(theme::semibold(12.5)));
    if scope.summary.as_ref().is_some_and(|s| s.results_capped || s.files_capped || s.listing_capped) {
        ui.label(egui::RichText::new(tl!("Not every file or match is listed")).small().color(t.text_faint));
    }
    if !scope.skipped.is_empty() {
        let title = match scope.skipped.len() {
            1 => tl!("1 file was skipped").to_owned(),
            n => crate::i18n::fmt(tl!("{n} files were skipped"), &[("n", &n.to_string())]),
        };
        egui::CollapsingHeader::new(title).id_salt("folder-search-skipped").show(ui, |ui| {
            for (path, reason) in scope.skipped.iter().take(MAX_SKIPPED_LISTED) {
                ui.label(egui::RichText::new(format!("{}: {reason}", file_name(path))).small().color(t.text_muted));
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
    /// Ask for the folder the active document's Search panel searches.
    pub(crate) fn choose_search_folder(&mut self) {
        let dialog = rfd::AsyncFileDialog::new();
        self.ask_one(crate::pickers::Ask::Folder(dialog), None, |app, dir| {
            if let Some(find) = app.active.and_then(|i| app.views.get_mut(i)).and_then(|v| v.find.as_mut()) {
                find.folder.folder = Some(dir);
            }
        });
    }

    /// Open `path` at the 0-based `page`, as `file.pdf#page=N` does.
    pub(crate) fn open_search_result(&mut self, path: &Path, page: usize) {
        let target = format!("{}#page={}", path.to_string_lossy(), page.saturating_add(1));
        let _ = self.open_path(&target);
    }
}
