//! Redact ▸ Redact files in a folder: the dialog (what to look for), the two folders, and the
//! background run with its progress card. The redaction is the engine's
//! ([`pdfcraft_engine::redact_folder`]); what happened to every file is written to a report in the
//! output folder, since a file that was not redacted, or has pages that could not be searched,
//! must not be missed.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use craft::host::PickRequest;
use craft::ui::contract::Bind;
use craft::ui::dialog::{DialogResult, dialog_id};
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::Label;
use pdfcraft_engine::redact_folder::{self, FileResult, Options, Outcome, Summary, Target};
use pdfcraft_engine::{REDACT_PATTERNS, RedactPattern, redact_word_list};

use super::job_count::JobCount;
use crate::PdfCraftApp;
use crate::notices::{self, ProgressKey, ProgressNotice};

const KEY: &str = "redact.folder";

/// What the dialog asked for.
#[derive(Clone, Debug)]
pub struct RedactFolderRequest {
    pub target: Target,
    pub recursive: bool,
}

/// A running redaction: the count the job and the progress card share.
pub struct RedactFolderRun {
    pub progress: Arc<JobCount>,
}

fn params() -> Vec<Param> {
    let modes = vec![("words".to_string(), tl!("Words or phrases").into()), ("pattern".to_string(), tl!("Pattern").into())];
    let patterns = REDACT_PATTERNS.iter().map(|p| (p.id().to_string(), tl!(p.label()).into())).collect();
    vec![
        Param::new("mode", tl!("Redact"), ParamKind::radio(modes, Some("words".to_string()))),
        Param::new("words", tl!("Words or phrases, one per line"), ParamKind::text("")).multiline(4).visible_when("mode", "words"),
        Param::new("case", tl!("Case-sensitive"), ParamKind::bool(false)).visible_when("mode", "words"),
        Param::new("whole", tl!("Whole words only"), ParamKind::bool(false)).visible_when("mode", "words"),
        Param::new("pattern", tl!("Pattern"), ParamKind::choice(patterns, Some(RedactPattern::Ssn.id().to_string()))).visible_when("mode", "pattern"),
        Param::new("subfolders", tl!("Include subfolders"), ParamKind::bool(true)),
    ]
}

/// The request the fields describe, if they name something to look for.
fn request(fields: &DialogFields) -> Option<RedactFolderRequest> {
    let target = if fields.text("mode") == Some("pattern") {
        Target::Pattern(fields.text("pattern").and_then(RedactPattern::from_id).unwrap_or(RedactPattern::Ssn))
    } else {
        let words = redact_word_list(fields.text("words").unwrap_or_default());
        if words.is_empty() {
            return None;
        }
        Target::Words { words, case_sensitive: fields.bool("case").unwrap_or(false), whole_words: fields.bool("whole").unwrap_or(false) }
    };
    Some(RedactFolderRequest { target, recursive: fields.bool("subfolders").unwrap_or(true) })
}

/// Pages (0-based, ascending) as "1-3, 5".
fn page_list(pages: &[usize]) -> String {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for &page in pages {
        match runs.last_mut() {
            Some((_, last)) if *last + 1 == page => *last = page,
            _ => runs.push((page, page)),
        }
    }
    let shown = |(a, b): (usize, usize)| if a == b { (a + 1).to_string() } else { format!("{}-{}", a + 1, b + 1) };
    runs.into_iter().map(shown).collect::<Vec<_>>().join(", ")
}

/// The report's lines: one per file, the ones that need a look first.
fn report(results: &[FileResult], summary: &Summary) -> String {
    let unsearched = |p: &[usize]| {
        if p.is_empty() {
            String::new()
        } else {
            let list = page_list(p);
            format!(" {}", crate::i18n::fmt(tl!("NOT SEARCHED, no text on page(s) {p}: check these by eye."), &[("p", &list)]))
        }
    };
    let mut skipped = Vec::new();
    let mut checked = Vec::new();
    for r in results {
        let name = r.path.display().to_string();
        match &r.outcome {
            Outcome::Skipped(why) => skipped.push(format!("{name}: {} {why}", tl!("NOT REDACTED:"))),
            Outcome::NoMatch { unsearched: p } => checked.push(format!("{name}: {}{}", tl!("no match, no copy written."), unsearched(p))),
            Outcome::Redacted { output, matches, still_holding, unsearched: p } => {
                let mut line = format!(
                    "{name}: {}",
                    crate::i18n::fmt(tl!("{n} match(es) redacted into {o}."), &[("n", &matches.to_string()), ("o", &output.display().to_string())])
                );
                if *still_holding > 0 {
                    line.push(' ');
                    line.push_str(&crate::i18n::fmt(
                        tl!("{n} bookmark(s), comment(s), attachment(s) or metadata may still hold the text."),
                        &[("n", &still_holding.to_string())],
                    ));
                }
                line.push_str(&unsearched(p));
                checked.push(line);
            }
        }
    }
    let mut out = Vec::new();
    if summary.cancelled {
        out.push(tl!("The run was cancelled: files not listed here were not looked at.").to_string());
    }
    if summary.capped {
        out.push(tl!("The folder holds more than one run takes: files not listed here were not looked at.").to_string());
    }
    out.extend(skipped);
    out.extend(checked);
    out.join("\n") + "\n"
}

/// What the notice says when a run ends.
fn message(results: &[FileResult], summary: &Summary, report: Option<&Path>) -> String {
    let unsearched = results
        .iter()
        .filter(|r| matches!(&r.outcome, Outcome::Redacted { unsearched, .. } | Outcome::NoMatch { unsearched } if !unsearched.is_empty()))
        .count();
    let counts =
        [("r", summary.redacted.to_string()), ("m", summary.no_match.to_string()), ("s", summary.skipped.to_string()), ("u", unsearched.to_string())];
    let counts: Vec<(&str, &str)> = counts.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let mut out =
        crate::i18n::fmt(tl!("Redacted {r} file(s). No match: {m}. Not redacted: {s}. With pages that could not be searched: {u}."), &counts);
    if summary.cancelled {
        out = format!("{} {out}", tl!("Cancelled."));
    }
    out.push(' ');
    match report {
        Some(path) => out.push_str(&crate::i18n::fmt(tl!("Details: {path}"), &[("path", &path.display().to_string())])),
        None => out.push_str(tl!("The report could not be written.")),
    }
    out
}

/// Write the report into `dir` under a name nothing there has yet.
fn write_report(dir: &Path, text: &str) -> Option<PathBuf> {
    for n in 1..=999 {
        let path = dir.join(if n == 1 { "redaction-report.txt".to_string() } else { format!("redaction-report ({n}).txt") });
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => return file.write_all(text.as_bytes()).ok().map(|()| path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return None,
        }
    }
    None
}

/// The whole run, on a worker thread; the result is what the notice says.
fn run(folder: &Path, request: RedactFolderRequest, output: PathBuf, count: &JobCount, wake: &dyn Fn()) -> String {
    let options = Options { target: request.target, recursive: request.recursive, output };
    let mut results = Vec::new();
    let done = redact_folder::redact_folder(folder, &options, &|| count.cancelled(), &mut |total, result| {
        results.push(result);
        count.set(results.len(), total.max(results.len()));
        wake();
    });
    match done {
        Err(e) => e,
        Ok(summary) if summary.files_found == 0 && results.is_empty() => tl!("There were no PDFs to redact.").to_string(),
        Ok(summary) => {
            let written = write_report(&options.output, &report(&results, &summary));
            message(&results, &summary, written.as_deref())
        }
    }
}

impl PdfCraftApp {
    pub(crate) fn open_redact_folder(&mut self, ctx: &egui::Context) {
        if cfg!(target_arch = "wasm32") {
            self.notify_tr("Redacting a folder is not available in the browser");
            return;
        }
        let dialog = HostedParamDialog::new(ctx, KEY, DialogFields::new(), |fields| {
            let ready = request(fields).is_some();
            ParamDialog::new(KEY, tl!("Redact files in a folder"), params())
                .values(Bind::bound(fields))
                .ok_label(Label::from(tl!("Choose folders")))
                .ok_enabled(ready)
                .footer_note(tl!(
                    "Next you choose the folder with the PDFs, then a folder for the redacted copies. Originals are not changed. Redaction cannot be undone in the copies."
                ))
        });
        crate::dialogs::host::open(self, dialog);
    }

    pub(crate) fn redact_folder_pass(&mut self, _ctx: &egui::Context) {
        let Some(DialogResult::Ok(answer)) = self.dialogs.take_by_id::<HostedParamDialog>(dialog_id(KEY)) else { return };
        let Some(request) = request(&answer.fields) else { return };
        if self.redact_folder_run.is_some() {
            self.notify_tr("A folder is already being redacted");
            return;
        }
        let ask = PickRequest::folder("redact.folder.input").title(tl!("Choose the folder with the PDFs to redact").to_string());
        self.pick_first(ask, None, move |app, _, folder| {
            let ask = PickRequest::folder("redact.folder.output").title(tl!("Choose a folder for the redacted copies").to_string());
            app.pick_first(ask, None, move |app, _, output| app.run_redact_folder(folder, request, output));
        });
    }

    fn run_redact_folder(&mut self, folder: PathBuf, request: RedactFolderRequest, output: PathBuf) {
        if self.redact_folder_run.is_some() {
            self.notify_tr("A folder is already being redacted");
            return;
        }
        // A result nobody took is not this run's.
        while self.redact_folder_jobs.try_recv().is_some() {}
        let progress = Arc::new(JobCount::default());
        progress.set(0, 0);
        let (count, wake) = (progress.clone(), self.wake.clone());
        // The report and the message are written on a worker thread: in the UI's language.
        let lang = crate::i18n::current();
        self.redact_folder_jobs.submit((), Default::default(), move |_| {
            crate::i18n::set_current(lang);
            run(&folder, request, output, &count, &|| wake.wake())
        });
        self.redact_folder_run = Some(RedactFolderRun { progress });
        self.post_redact_folder_progress();
    }

    pub(crate) fn cancel_redact_folder(&mut self) {
        let Some(run) = &self.redact_folder_run else { return };
        run.progress.cancel();
        self.post_redact_folder_progress();
    }

    fn post_redact_folder_progress(&mut self) {
        let Some(run) = &self.redact_folder_run else { return };
        let (done, total) = run.progress.read();
        let cancelling = run.progress.cancelled();
        let label = if cancelling {
            tl!("Cancelling…").to_string()
        } else if total == 0 {
            tl!("Looking for PDFs…").to_string()
        } else {
            crate::i18n::fmt(tl!("Redacting files… {d} of {t} done"), &[("d", &done.min(total).to_string()), ("t", &total.to_string())])
        };
        let fraction = done.min(total) as f32 / total.max(1) as f32;
        notices::set_progress(self, ProgressKey::RedactFolder, ProgressNotice { label, fraction, cancellable: !cancelling });
    }

    pub(crate) fn poll_redact_folder(&mut self) {
        let Some(run) = self.redact_folder_run.as_ref() else { return };
        let progress = run.progress.clone();
        if let Some(finished) = self.redact_folder_jobs.try_recv() {
            self.redact_folder_run = None;
            notices::end_progress(self, ProgressKey::RedactFolder);
            self.notify(finished.result);
            return;
        }
        // A worker that ended without a result must not leave the card up for ever.
        let counts = self.redact_folder_jobs.counts();
        if counts.queued + counts.running + counts.finished == 0 {
            self.redact_folder_run = None;
            notices::end_progress(self, ProgressKey::RedactFolder);
            self.notify_tr("Redacting stopped unexpectedly. Check the output folder.");
            return;
        }
        if progress.take_change().is_some() {
            self.post_redact_folder_progress();
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn page_runs_are_written_as_ranges() {
        assert_eq!(super::page_list(&[0, 1, 2, 4, 6, 7]), "1-3, 5, 7-8");
        assert_eq!(super::page_list(&[]), "");
    }
}
