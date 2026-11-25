//! File ▸ Optimize multiple files (`optimize.batch`): a list of PDFs and, if asked, a folder whose PDFs are added (with
//! its subfolders when that is on). After OK the app asks for the folder to write to, then reduces each file's size with
//! Reduce File Size's settings on the host's job queue, with a progress card and Cancel. This is the optimize step of
//! the batch tool (`doc_batch`), which is [`Session::optimize_smaller`]: a copy that is not smaller leaves the file's
//! own bytes, which are still written.
//!
//! Every file is read, optimized and written on its own; one that fails is kept apart with its reason and the others
//! carry on. The originals are never changed and no file is replaced (a taken name gets a number). The message that
//! ends the run says how many files were written and names each one that was not, with why.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use craft::docpath::names::{CopyNumbering, unique_file_name};
use craft::host::{Files, PickRequest, Wake, WriteIntent};
use craft::ui::contract::Bind;
use craft::ui::dialog::dialog_id;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::Label;
use pdfcraft_engine::Session;
use pdfcraft_engine::optimize::{ImageSettings, MonoSettings};

use super::job_count::JobCount;
use crate::PdfCraftApp;
use crate::notices::{self, ProgressKey, ProgressNotice};

/// The dialog's key.
const KEY: &str = "optimize.batch";

/// Most files one run optimizes.
const MAX_FILES: usize = 1000;

/// Most folder entries one listing looks at, and most folder levels it goes down.
const MAX_ENTRIES: usize = 10_000;
const MAX_DEPTH: usize = 16;

/// The largest file one run reads.
const MAX_INPUT_BYTES: usize = 512 * 1024 * 1024;

/// Most failed files a message names.
const SHOWN: usize = 5;

/// What the dialog asked for.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct OptimizeBatchRequest {
    /// The listed files, in order.
    pub files: Vec<PathBuf>,
    /// A folder whose PDFs are added.
    pub folder: Option<PathBuf>,
    /// Whether the folder's subfolders are searched too.
    pub subfolders: bool,
}

/// What became of one file: its sizes in bytes before and after, or why it was not written.
pub struct FileResult {
    pub name: String,
    pub result: Result<(u64, u64), String>,
}

/// How a run ended: each file's result in the order run, whether Cancel stopped it, and the words for the user.
pub struct OptimizeBatchOutcome {
    pub results: Vec<FileResult>,
    pub cancelled: bool,
    pub message: String,
}

/// A running job and its count (with the cancel request).
pub struct OptimizeBatchRun {
    pub progress: Arc<JobCount>,
}

/// The files listed in the dialog, in order.
fn chosen(fields: &DialogFields) -> Vec<PathBuf> {
    fields
        .list("files")
        .iter()
        .flat_map(|items| items.iter())
        .filter_map(|item| match item {
            FieldValue::Text(text) => Some(PathBuf::from(text)),
            _ => None,
        })
        .collect()
}

/// The rows of the dialog.
fn params() -> Vec<Param> {
    vec![
        Param::files("files", tl!("PDF files to optimize")),
        Param::new("use_folder", tl!("Also optimize the PDFs in a folder (you choose it next)"), ParamKind::bool(false)),
        Param::new("subfolders", tl!("Include subfolders"), ParamKind::bool(true)).visible_when("use_folder", true),
    ]
}

/// What the dialog's note says, and whether OK may run.
fn readiness(fields: &DialogFields) -> (String, bool) {
    let files = chosen(fields);
    let folder = fields.bool("use_folder").unwrap_or(false);
    if files.is_empty() && !folder {
        (tl!("Add the files to optimize, or choose a folder.").to_string(), false)
    } else if files.len() > MAX_FILES {
        (crate::i18n::fmt(tl!("At most {n} files can be optimized at once."), &[("n", &MAX_FILES.to_string())]), false)
    } else {
        (
            tl!("Each file is reduced like Reduce File Size. You choose the folder for the optimized copies next; the originals are not changed.")
                .to_string(),
            true,
        )
    }
}

impl PdfCraftApp {
    /// `optimize.batch`: opens the dialog with no files listed.
    pub(crate) fn open_optimize_batch(&mut self, ctx: &egui::Context) {
        if cfg!(target_arch = "wasm32") {
            self.notify_tr("Optimizing several files is not available in the browser");
            return;
        }
        let dialog = HostedParamDialog::new(ctx, KEY, DialogFields::new(), |fields| {
            let (note, ready) = readiness(fields);
            ParamDialog::new(KEY, tl!("Optimize multiple files"), params())
                .values(Bind::bound(fields))
                .ok_label(Label::from(tl!("Optimize files")))
                .ok_enabled(ready)
                .footer_note(note)
        });
        crate::dialogs::host::open(self, dialog);
    }

    /// Takes the dialog's answer and asks for the folders.
    pub(crate) fn optimize_batch_pass(&mut self, _ctx: &egui::Context) {
        let Some(answer) = self.dialogs.take_by_id::<HostedParamDialog>(dialog_id(KEY)) else { return };
        let Some(result) = answer.ok() else { return };
        let f = &result.fields;
        let request = OptimizeBatchRequest { files: chosen(f), folder: None, subfolders: f.bool("subfolders").unwrap_or(true) };
        self.start_optimize_batch(request, f.bool("use_folder").unwrap_or(false));
    }

    /// Asks for the folder to take PDFs from when `use_folder`, then for the folder the copies go to, then runs.
    pub fn start_optimize_batch(&mut self, request: OptimizeBatchRequest, use_folder: bool) {
        if self.optimize_batch_run.is_some() {
            self.notify_tr("Optimizing files is already running");
            return;
        }
        if (request.files.is_empty() && !use_folder) || request.files.len() > MAX_FILES {
            self.notify_tr("Those files can't be optimized");
            return;
        }
        if use_folder {
            let ask = PickRequest::folder("optimize.batch.input").title(tl!("Choose the folder with the PDFs to optimize").to_string());
            self.pick_first(ask, None, move |app, _, folder| app.ask_optimize_batch_output(OptimizeBatchRequest { folder: Some(folder), ..request }));
        } else {
            self.ask_optimize_batch_output(request);
        }
    }

    /// Asks for the folder the optimized copies go to.
    fn ask_optimize_batch_output(&mut self, request: OptimizeBatchRequest) {
        let ask = PickRequest::folder("optimize.batch.output").title(tl!("Choose a folder for the optimized files").to_string());
        self.pick_first(ask, None, move |app, _, dir| app.run_optimize_batch(request, dir));
    }

    /// Optimizes the files into `dir` on the job queue.
    fn run_optimize_batch(&mut self, request: OptimizeBatchRequest, dir: PathBuf) {
        if self.optimize_batch_run.is_some() {
            self.notify_tr("Optimizing files is already running");
            return;
        }
        // A result nobody took is not this run's.
        while self.optimize_batch_jobs.try_recv().is_some() {}
        let progress = Arc::new(JobCount::default());
        progress.set(0, request.files.len());
        let (count, wake, host) = (progress.clone(), self.wake.clone(), self.host.clone());
        let javascript = self.session.javascript();
        // The message is written on a worker thread: in the UI's language.
        let lang = crate::i18n::current();
        self.optimize_batch_jobs.submit((), Default::default(), move |_| {
            crate::i18n::set_current(lang);
            optimize_files(host.files(), &request, &dir, javascript, &count, &wake)
        });
        self.optimize_batch_run = Some(OptimizeBatchRun { progress });
        self.post_optimize_batch_progress();
    }

    /// Stops the run after the file it is on. Files already written stay.
    pub fn cancel_optimize_batch(&mut self) {
        let Some(run) = &self.optimize_batch_run else { return };
        run.progress.cancel();
        self.post_optimize_batch_progress();
    }

    fn post_optimize_batch_progress(&mut self) {
        let Some(run) = &self.optimize_batch_run else { return };
        let (done, total) = run.progress.read();
        let cancelling = run.progress.cancelled();
        let label = if cancelling {
            tl!("Cancelling…").to_string()
        } else if total == 0 {
            tl!("Looking for PDFs…").to_string()
        } else {
            crate::i18n::fmt(tl!("Optimizing files… file {d} of {t}"), &[("d", &(done + 1).min(total).to_string()), ("t", &total.to_string())])
        };
        let fraction = done.min(total) as f32 / total.max(1) as f32;
        notices::set_progress(self, ProgressKey::OptimizeBatch, ProgressNotice { label, fraction, cancellable: !cancelling });
    }

    /// Shows the run's progress, and how it ended when it is done.
    pub(crate) fn poll_optimize_batch(&mut self) {
        let Some(run) = self.optimize_batch_run.as_ref() else { return };
        let progress = run.progress.clone();
        if let Some(finished) = self.optimize_batch_jobs.try_recv() {
            self.optimize_batch_run = None;
            notices::end_progress(self, ProgressKey::OptimizeBatch);
            self.notify(finished.result.message);
            return;
        }
        // A worker that ended without a result must not leave the card up for ever.
        let counts = self.optimize_batch_jobs.counts();
        if counts.queued + counts.running + counts.finished == 0 {
            self.optimize_batch_run = None;
            notices::end_progress(self, ProgressKey::OptimizeBatch);
            self.notify_tr("Optimizing stopped unexpectedly. Check the output folder.");
            return;
        }
        if progress.take_change().is_some() {
            self.post_optimize_batch_progress();
        }
    }
}

fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(|| path.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())
}

/// The PDFs in `folder` (and below it when `recursive`), sorted by name within each folder, links skipped, and whether
/// the listing stopped at a limit. The folder `skip` (where the copies go) is not entered.
fn pdfs_in(folder: &Path, recursive: bool, skip: &Path) -> Result<(Vec<PathBuf>, bool), String> {
    let mut found = Vec::new();
    let mut pending = vec![(folder.to_path_buf(), 0usize)];
    let mut seen = 0usize;
    while let Some((dir, depth)) = pending.pop() {
        let listing = std::fs::read_dir(&dir);
        let mut entries: Vec<_> = match listing {
            Ok(entries) => entries.flatten().collect(),
            Err(e) if depth == 0 => return Err(e.to_string()),
            Err(_) => continue,
        };
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            seen += 1;
            if seen > MAX_ENTRIES || found.len() >= MAX_FILES {
                return Ok((found, true));
            }
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() {
                if recursive && depth < MAX_DEPTH && !path.starts_with(skip) {
                    pending.push((path, depth + 1));
                }
            } else if kind.is_file() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
                found.push(path);
            }
        }
    }
    Ok((found, false))
}

/// The files to optimize: the listed ones, then the folder's, each once; and the results of what could not be listed.
fn gather(request: &OptimizeBatchRequest, out: &Path) -> (Vec<PathBuf>, Vec<FileResult>) {
    let mut seen = HashSet::new();
    let mut inputs: Vec<PathBuf> = request.files.iter().filter(|p| seen.insert((*p).clone())).cloned().collect();
    let mut problems = Vec::new();
    if let Some(folder) = &request.folder {
        match pdfs_in(folder, request.subfolders, out) {
            Ok((found, truncated)) => {
                for path in found {
                    if inputs.len() < MAX_FILES && seen.insert(path.clone()) {
                        inputs.push(path);
                    }
                }
                if truncated {
                    let why = crate::i18n::fmt(tl!("more than {n} files, so the rest were left out"), &[("n", &MAX_FILES.to_string())]);
                    problems.push(FileResult { name: name_of(folder), result: Err(why) });
                }
            }
            Err(why) => problems.push(FileResult { name: name_of(folder), result: Err(why) }),
        }
    }
    (inputs, problems)
}

/// Optimizes the file at `path` and writes the copy into `dir` under a name that is free; the sizes before and after.
fn optimize_one(files: &dyn Files, path: &Path, name: &str, dir: &Path, javascript: bool, used: &mut HashSet<String>) -> Result<(u64, u64), String> {
    let bytes = files.read(path).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(tl!("the file is larger than 512 MB").to_string());
    }
    let before = bytes.len() as u64;
    let mut session = Session::new();
    session.set_javascript(javascript);
    let optimized = session.optimize_smaller(name, Arc::new(bytes), ImageSettings::REDUCE, MonoSettings::REDUCE)?;
    let unique = unique_file_name(name, CopyNumbering::Parenthesized, |n| used.contains(n) || files.exists(&dir.join(n)));
    files.write(&dir.join(&unique), optimized.as_slice(), WriteIntent::SaveAs).map_err(|e| e.to_string())?;
    used.insert(unique);
    Ok((before, optimized.len() as u64))
}

/// The whole job: list the files, then optimize and write each one into `dir`, going on past a file that fails.
pub(crate) fn optimize_files(
    files: &dyn Files,
    request: &OptimizeBatchRequest,
    dir: &Path,
    javascript: bool,
    count: &JobCount,
    wake: &Wake,
) -> OptimizeBatchOutcome {
    let (inputs, problems) = gather(request, dir);
    let total = inputs.len();
    count.set(0, total);
    wake.wake();
    let mut results = Vec::with_capacity(total);
    let mut used = HashSet::new();
    let mut cancelled = false;
    for (i, path) in inputs.iter().enumerate() {
        if count.cancelled() {
            cancelled = true;
            break;
        }
        count.set(i, total);
        wake.wake();
        let name = name_of(path);
        let result = optimize_one(files, path, &name, dir, javascript, &mut used);
        results.push(FileResult { name, result });
    }
    count.set(total, total);
    results.extend(problems);
    let message = summary(&results, total, cancelled, dir);
    OptimizeBatchOutcome { results, cancelled, message }
}

/// How a run ended, in words: what was written, and each file that was not, with why.
fn summary(results: &[FileResult], total: usize, cancelled: bool, dir: &Path) -> String {
    let written = results.iter().filter(|r| r.result.is_ok()).count();
    let failed: Vec<String> = results.iter().filter_map(|r| r.result.as_ref().err().map(|why| format!("{}: {why}", r.name))).collect();
    let (n, t) = (written.to_string(), total.to_string());
    let mut message = if cancelled {
        crate::i18n::fmt(tl!("Optimizing was cancelled after {n} of {t} files. Those are written; the rest were not."), &[("n", &n), ("t", &t)])
    } else if written > 0 {
        crate::i18n::fmt(tl!("Optimized {n} of {t} files into {dir}."), &[("n", &n), ("t", &t), ("dir", &dir.display().to_string())])
    } else if failed.is_empty() {
        tl!("There were no PDFs to optimize.").to_string()
    } else {
        tl!("Nothing was optimized.").to_string()
    };
    if !failed.is_empty() {
        let more = failed.len().saturating_sub(SHOWN);
        let mut list = failed.iter().take(SHOWN).cloned().collect::<Vec<_>>().join("; ");
        if more > 0 {
            list = crate::i18n::fmt(tl!("{list}, and {more} more"), &[("list", &list), ("more", &more.to_string())]);
        }
        message.push(' ');
        message.push_str(&crate::i18n::fmt(tl!("{k} failed: {list}"), &[("k", &failed.len().to_string()), ("list", &list)]));
    }
    message
}
