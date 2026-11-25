//! Edit ▸ Bates numbering on several files (`edit.bates_multi`): a list of files (add, remove, reorder), the number to
//! start from and how it is written. The files are numbered in the order listed, the numbers running on from one file
//! to the next, and the numbered copies are written to a folder chosen after OK; the originals are never changed and no
//! file is replaced (a name already taken gets a number). The work runs on the host's job queue with a progress card and
//! Cancel, each file in a session of its own, so no open document is touched.
//!
//! Every file is read and counted before anything is written, so a file that can't be used stops the run with nothing
//! written, and the numbers each file gets are known before the first one is numbered.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use craft::docpath::names::{CopyNumbering, unique_file_name};
use craft::host::{Files, PickRequest, Wake, WriteIntent};
use craft::ui::contract::Bind;
use craft::ui::dialog::dialog_id;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::Label;
use pdfcraft_engine::bates_batch::{self, IntervalError, MAX_AFFIX_CHARS, MAX_DIGITS};
use pdfcraft_engine::{Edit, HeaderFooter, Session};

use super::job_count::JobCount;
use crate::PdfCraftApp;
use crate::notices::{self, ProgressKey, ProgressNotice};

/// The dialog's key.
const KEY: &str = "edit.bates_multi";

/// Most files one run numbers.
const MAX_FILES: usize = 1000;

/// Most bytes of input one run holds at once.
const MAX_INPUT_BYTES: u64 = 512 * 1024 * 1024;

/// The largest number the first-number row takes: twelve digits.
const MAX_START: i64 = 999_999_999_999;

/// What the dialog asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct BatesRequest {
    pub files: Vec<PathBuf>,
    pub start: u64,
    pub digits: u8,
    pub prefix: String,
    pub suffix: String,
}

/// How a run ended, in words.
pub struct BatesOutcome {
    pub message: String,
}

/// A running job and its count (with the cancel request).
pub struct BatesRun {
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
        Param::files("files", tl!("Files, in the order they are numbered")),
        Param::new("start", tl!("First number"), ParamKind::int(Some(1), Some(MAX_START), 1)),
        Param::new("digits", tl!("Digits"), ParamKind::int(Some(1), Some(i64::from(MAX_DIGITS)), 6)),
        Param::new("prefix", tl!("Text before the number"), ParamKind::text("")),
        Param::new("suffix", tl!("Text after the number"), ParamKind::text("")),
    ]
}

/// What the dialog's note says, and whether OK may run.
fn readiness(fields: &DialogFields) -> (String, bool) {
    let files = chosen(fields);
    let (prefix, suffix) = (fields.text("prefix").unwrap_or_default(), fields.text("suffix").unwrap_or_default());
    if files.is_empty() {
        (tl!("Add the files to number.").to_string(), false)
    } else if files.len() > MAX_FILES {
        (crate::i18n::fmt(tl!("At most {n} files can be numbered at once."), &[("n", &MAX_FILES.to_string())]), false)
    } else if !bates_batch::affixes_ok(prefix, suffix) {
        (
            crate::i18n::fmt(
                tl!("The text before and after the number can have at most {n} characters and cannot contain # or >>."),
                &[("n", &MAX_AFFIX_CHARS.to_string())],
            ),
            false,
        )
    } else {
        (
            tl!("The number goes at the bottom right of every page and runs on from one file to the next. You choose the folder for the numbered copies next; the originals are not changed.").to_string(),
            true,
        )
    }
}

impl PdfCraftApp {
    /// `edit.bates_multi`: opens the dialog with no files listed.
    pub(crate) fn open_bates_multi(&mut self, ctx: &egui::Context) {
        if cfg!(target_arch = "wasm32") {
            self.notify_tr("Numbering several files is not available in the browser");
            return;
        }
        let dialog = HostedParamDialog::new(ctx, KEY, DialogFields::new(), |fields| {
            let (note, ready) = readiness(fields);
            ParamDialog::new(KEY, tl!("Bates numbering on several files"), params())
                .values(Bind::bound(fields))
                .ok_label(Label::from(tl!("Number files")))
                .ok_enabled(ready)
                .footer_note(note)
        });
        crate::dialogs::host::open(self, dialog);
    }

    /// Takes the dialog's answer and asks for the folder.
    pub(crate) fn bates_multi_pass(&mut self, _ctx: &egui::Context) {
        let Some(answer) = self.dialogs.take_by_id::<HostedParamDialog>(dialog_id(KEY)) else { return };
        let Some(result) = answer.ok() else { return };
        let f = &result.fields;
        let request = BatesRequest {
            files: chosen(f),
            start: f.int("start").map_or(1, |n| n.clamp(1, MAX_START) as u64),
            digits: f.int("digits").map_or(6, |n| n.clamp(1, i64::from(MAX_DIGITS)) as u8),
            prefix: f.text("prefix").unwrap_or_default().to_string(),
            suffix: f.text("suffix").unwrap_or_default().to_string(),
        };
        self.start_bates_multi(request);
    }

    /// Asks for the folder the numbered copies go to, then runs.
    pub fn start_bates_multi(&mut self, request: BatesRequest) {
        if self.bates_multi_run.is_some() {
            self.notify_tr("Bates numbering is already running");
            return;
        }
        if request.files.is_empty() || request.files.len() > MAX_FILES || !bates_batch::affixes_ok(&request.prefix, &request.suffix) {
            self.notify_tr("Those files can't be numbered with these settings");
            return;
        }
        let ask = PickRequest::folder("bates.output").title(tl!("Choose a folder for the numbered files").to_string());
        self.pick_first(ask, None, move |app, _, dir| app.run_bates_multi(request, dir));
    }

    /// Numbers the files into `dir` on the job queue.
    fn run_bates_multi(&mut self, request: BatesRequest, dir: PathBuf) {
        if self.bates_multi_run.is_some() {
            self.notify_tr("Bates numbering is already running");
            return;
        }
        // A result nobody took is not this run's.
        while self.bates_multi_jobs.try_recv().is_some() {}
        let progress = Arc::new(JobCount::default());
        progress.set(0, request.files.len().saturating_mul(2));
        let (count, wake, host) = (progress.clone(), self.wake.clone(), self.host.clone());
        // The message is written on a worker thread: in the UI's language.
        let lang = crate::i18n::current();
        self.bates_multi_jobs.submit((), Default::default(), move |_| {
            crate::i18n::set_current(lang);
            number_files(host.files(), &request, &dir, &count, &wake)
        });
        self.bates_multi_run = Some(BatesRun { progress });
        self.post_bates_multi_progress();
    }

    /// Stops the run after the file it is on. Files already written stay.
    pub fn cancel_bates_multi(&mut self) {
        let Some(run) = &self.bates_multi_run else { return };
        run.progress.cancel();
        self.post_bates_multi_progress();
    }

    fn post_bates_multi_progress(&mut self) {
        let Some(run) = &self.bates_multi_run else { return };
        let (done, total) = run.progress.read();
        let total = total.max(1);
        let cancelling = run.progress.cancelled();
        let label = if cancelling {
            tl!("Cancelling…").to_string()
        } else {
            crate::i18n::fmt(tl!("Numbering files… step {d} of {t}"), &[("d", &(done + 1).min(total).to_string()), ("t", &total.to_string())])
        };
        let fraction = done.min(total) as f32 / total as f32;
        notices::set_progress(self, ProgressKey::BatesMulti, ProgressNotice { label, fraction, cancellable: !cancelling });
    }

    /// Shows the run's progress, and how it ended when it is done.
    pub(crate) fn poll_bates_multi(&mut self) {
        let Some(run) = self.bates_multi_run.as_ref() else { return };
        let progress = run.progress.clone();
        if let Some(finished) = self.bates_multi_jobs.try_recv() {
            self.bates_multi_run = None;
            notices::end_progress(self, ProgressKey::BatesMulti);
            self.notify(finished.result.message);
            return;
        }
        if progress.take_change().is_some() {
            self.post_bates_multi_progress();
        }
    }
}

fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(|| path.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())
}

fn say(message: String) -> BatesOutcome {
    BatesOutcome { message }
}

/// The whole job: count every file, then number and write each one into `dir` under a name that is free.
pub(crate) fn number_files(files: &dyn Files, request: &BatesRequest, dir: &Path, count: &JobCount, wake: &Wake) -> BatesOutcome {
    let total = request.files.len();
    let steps = total.saturating_mul(2);
    let mut sources: Vec<(String, Arc<Vec<u8>>)> = Vec::with_capacity(total);
    let mut pages = Vec::with_capacity(total);
    let mut held = 0u64;
    for (i, path) in request.files.iter().enumerate() {
        if count.cancelled() {
            return say(tl!("Bates numbering was cancelled. Nothing was written.").to_string());
        }
        count.set(i, steps);
        wake.wake();
        let name = name_of(path);
        let refuse = |why: String| say(crate::i18n::fmt(tl!("Nothing was numbered. {name} can't be used: {why}"), &[("name", &name), ("why", &why)]));
        let bytes = match files.read(path) {
            Ok(bytes) => bytes,
            Err(e) => return refuse(e.to_string()),
        };
        held = held.saturating_add(bytes.len() as u64);
        if held > MAX_INPUT_BYTES {
            return refuse(tl!("the files together are larger than 512 MB").to_string());
        }
        let bytes = Arc::new(bytes);
        let mut session = Session::new();
        let id = match session.open(name.clone(), None, bytes.clone(), None) {
            Ok(id) => id,
            Err(e) => return refuse(e.to_string()),
        };
        pages.push(session.get(id).map_or(0, |d| d.info.pages.len()));
        sources.push((name, bytes));
    }
    let intervals = match bates_batch::intervals(request.start, &pages) {
        Ok(intervals) => intervals,
        Err(IntervalError::NoPages(i) | IntervalError::Overflow(i)) if i >= sources.len() => {
            return say(tl!("Those files can't be numbered with these settings").to_string());
        }
        Err(IntervalError::NoPages(i)) => {
            let name = &sources[i].0;
            return say(crate::i18n::fmt(tl!("Nothing was numbered. {name} has no pages to number."), &[("name", name)]));
        }
        Err(IntervalError::Overflow(i)) => {
            let name = &sources[i].0;
            return say(crate::i18n::fmt(tl!("Nothing was numbered. The numbers would run past the largest there is at {name}."), &[("name", name)]));
        }
    };
    let (mut written, mut failed): (usize, Vec<String>) = (0, Vec::new());
    let mut used: HashSet<String> = HashSet::new();
    for (i, ((name, bytes), (first, _))) in sources.iter().zip(&intervals).enumerate() {
        if count.cancelled() {
            return say(crate::i18n::fmt(
                tl!("Bates numbering was cancelled after {n} files. Those are written; the rest were not."),
                &[("n", &written.to_string())],
            ));
        }
        count.set(total + i, steps);
        wake.wake();
        let numbered = (|| -> Result<Arc<Vec<u8>>, String> {
            let mut session = Session::new();
            let id = session.open(name.clone(), None, bytes.clone(), None).map_err(|e| e.to_string())?;
            let mut settings = HeaderFooter::default();
            settings.text[5] = bates_batch::template(request.digits, *first, &request.prefix, &request.suffix);
            let pages: Vec<usize> = (0..session.get(id).map_or(0, |d| d.info.pages.len())).collect();
            session.apply(id, Edit::AddHeaderFooter { pages, settings, replace: false }).map_err(|e| e.to_string())?;
            session.save_full_bytes(id).map_err(|e| e.to_string())
        })();
        let result = numbered.and_then(|numbered| {
            let unique = unique_file_name(name, CopyNumbering::Parenthesized, |n| used.contains(n) || files.read(&dir.join(n)).is_ok());
            files.write(&dir.join(&unique), numbered.as_slice(), WriteIntent::SaveAs).map_err(|e| e.to_string())?;
            used.insert(unique);
            Ok(())
        });
        match result {
            Ok(()) => written += 1,
            Err(why) => failed.push(format!("{name}: {why}")),
        }
    }
    let last = intervals.last().map_or(request.start, |(_, last)| *last);
    let mut message = crate::i18n::fmt(
        &crate::i18n::trn(crate::i18n::current(), written as u64, "Numbered {n} file, {first} to {last}.", "Numbered {n} files, {first} to {last}."),
        &[("first", &request.start.to_string()), ("last", &last.to_string())],
    );
    if !failed.is_empty() {
        message.push(' ');
        message.push_str(&crate::i18n::fmt(
            tl!("{k} could not be written, and their numbers stay unused: {list}"),
            &[("k", &failed.len().to_string()), ("list", &failed.join("; "))],
        ));
    }
    say(message)
}
