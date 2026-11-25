//! Word, Excel, PowerPoint, OpenDocument and RTF files in Insert pages, Combine files, Create a PDF from multiple files
//! and Insert pages from the clipboard. Converting one takes seconds, so the files are converted one after another on
//! the host's job queue, with a progress card that names the file and offers Cancel, and the flow that asked carries on
//! with the converted PDFs when they are ready.
//!
//! The conversion is [`pdfcraft_engine::OfficeRunner`]'s: the same private temporary folder, fresh profile with macros
//! and link updates off, and time and size limits as File ▸ Open. What converts is the app's [`OfficeConverter`]; tests
//! give it a stand-in, so they never start LibreOffice.
//!
//! A file that can't be converted (LibreOffice isn't installed, or the file is damaged or protected) is named with the
//! reason and left out; the other files carry on. Cancel stops after the file being converted and adds nothing.

use std::sync::Arc;

use craft::host::Wake;
use pdfcraft_engine::{EditError, OfficeRunner, SourceKind, source_kind};

use super::job_count::JobCount;
use crate::PdfCraftApp;
use crate::notices::{self, ProgressKey, ProgressNotice};

/// A picked file as the flows take it: its name and bytes, or its name and why it couldn't be read.
pub(crate) type Item = Result<(String, Vec<u8>), (String, String)>;

/// What a conversion made: each document's PDF or why not, in order.
pub(crate) type Converted = Vec<Result<Arc<Vec<u8>>, String>>;

/// A file left out: its name and why.
pub(crate) type LeftOut = (String, String);

/// What a flow does with the PDFs of its batch.
type Done = Box<dyn FnOnce(&mut PdfCraftApp, Converted)>;

/// What a flow does with its files once the Office documents among them are converted: the files that can be used, in
/// order, and the positions among them of the documents that were converted (a flow that treats those differently
/// from PDFs it was given needs them).
pub(crate) type FilesDone = Box<dyn FnOnce(&mut PdfCraftApp, Vec<Item>, Vec<usize>)>;

/// Whether a converter can run now.
type Available = Arc<dyn Fn() -> bool + Send + Sync>;

/// Converts one Office document (its name and bytes) to a PDF, or says why not.
type Convert = Arc<dyn Fn(&str, &Arc<Vec<u8>>) -> Result<Arc<Vec<u8>>, String> + Send + Sync>;

/// What converts Office documents for the app: the installed LibreOffice, or a stand-in a test gives.
#[derive(Clone)]
pub struct OfficeConverter {
    available: Available,
    convert: Convert,
}

impl OfficeConverter {
    /// A converter that is `available` while that answers true and converts with `convert`.
    pub fn new(
        available: impl Fn() -> bool + Send + Sync + 'static,
        convert: impl Fn(&str, &Arc<Vec<u8>>) -> Result<Arc<Vec<u8>>, String> + Send + Sync + 'static,
    ) -> Self {
        Self { available: Arc::new(available), convert: Arc::new(convert) }
    }

    /// No converter installed: every Office file is left out with the reason. Tests start from this one.
    pub fn unavailable() -> Self {
        Self::new(|| false, |_, _| Err(missing()))
    }

    /// The LibreOffice `runner` finds.
    fn libreoffice(runner: OfficeRunner) -> Self {
        let probe = runner.clone();
        Self::new(move || probe.is_available(), move |name, bytes| runner.convert(name, bytes).map_err(|e| why(name, &e)))
    }

    /// Whether a conversion can run now.
    pub fn is_available(&self) -> bool {
        (self.available)()
    }
}

/// What a batch made on its worker.
pub struct OfficeOutcome {
    converted: Converted,
    cancelled: bool,
}

/// A batch converting on the job queue: the count it shares with the progress card, the files' names, and what the
/// flow does with the result.
pub struct OfficeRun {
    progress: Arc<JobCount>,
    names: Vec<String>,
    done: Done,
}

/// Why nothing can be converted, as the reason a file is left out.
fn missing() -> String {
    tl!("Word, Excel and PowerPoint files need LibreOffice, which isn't installed. Install it free from libreoffice.org, or set PDFCRAFT_SOFFICE to its soffice program.")
        .to_string()
}

/// Whether the name has an Office document's extension. Only the name: used before a file is read, to decide how to
/// take a batch. What a file is is decided by [`source_kind`] on its bytes.
pub(crate) fn has_office_name(name: &str) -> bool {
    // The web build has no LibreOffice: Office files are not special there, and are refused as any file of a kind the
    // flow doesn't take.
    !cfg!(target_arch = "wasm32")
        && name.rsplit_once('.').is_some_and(|(_, ext)| pdfcraft_engine::OFFICE_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
}

/// Whether `bytes` named `name` is an Office document to convert (a PDF with an Office name is a PDF).
pub(crate) fn is_office(name: &str, bytes: &[u8]) -> bool {
    !cfg!(target_arch = "wasm32") && source_kind(name, bytes) == Some(SourceKind::Office)
}

/// Whether any of the files is an Office document to convert.
pub(crate) fn any_office(files: &[Item]) -> bool {
    files.iter().any(|f| matches!(f, Ok((name, bytes)) if is_office(name, bytes)))
}

/// The text of an engine error without the file's name, which the caller already shows.
fn why(name: &str, error: &EditError) -> String {
    let text = error.to_string();
    text.strip_prefix(name).and_then(|rest| rest.strip_prefix(": ")).map_or(text.clone(), str::to_string)
}

/// The whole job: convert `docs` in order, stopping when asked.
fn convert_batch(converter: &OfficeConverter, docs: &[(String, Arc<Vec<u8>>)], count: &JobCount, wake: &Wake) -> OfficeOutcome {
    let mut converted = Vec::with_capacity(docs.len());
    for (i, (name, bytes)) in docs.iter().enumerate() {
        if count.cancelled() {
            return OfficeOutcome { converted, cancelled: true };
        }
        count.set(i, docs.len());
        wake.wake();
        converted.push((converter.convert)(name, bytes));
    }
    count.set(docs.len(), docs.len());
    OfficeOutcome { converted, cancelled: count.cancelled() }
}

/// Most files named in a "left out" message.
const SHOWN: usize = 3;

impl PdfCraftApp {
    /// What converts Office documents now: the test's stand-in, else the installed LibreOffice.
    fn office_converter(&self) -> OfficeConverter {
        self.office_converter.clone().unwrap_or_else(|| OfficeConverter::libreoffice(self.session.office_runner()))
    }

    /// Convert the Office documents `docs` (name and bytes) off the frame thread, then call `done` with each one's PDF
    /// or why it failed, in order. Without a converter `done` is called at once with the reason for each. A batch is
    /// refused while another converts (`done` is not called, and the user is told); Cancel ends the batch without
    /// calling `done`.
    pub(crate) fn convert_office(&mut self, docs: Vec<(String, Arc<Vec<u8>>)>, done: Done) {
        if docs.is_empty() {
            return done(self, Vec::new());
        }
        let converter = self.office_converter();
        if !converter.is_available() {
            let reason = missing();
            return done(self, docs.iter().map(|_| Err(reason.clone())).collect());
        }
        if self.office_run.is_some() {
            self.notify_tr("Office documents are already being converted");
            return;
        }
        // A result nobody took is not this run's.
        while self.office_jobs.try_recv().is_some() {}
        let progress = Arc::new(JobCount::default());
        progress.set(0, docs.len());
        let names = docs.iter().map(|(name, _)| name.clone()).collect();
        let (count, wake) = (progress.clone(), self.wake.clone());
        self.office_jobs.submit((), Default::default(), move |_| convert_batch(&converter, &docs, &count, &wake));
        self.office_run = Some(OfficeRun { progress, names, done });
        self.post_office_progress();
    }

    /// Stop the conversion after the file it is on. Nothing is added.
    pub fn cancel_office(&mut self) {
        let Some(run) = &self.office_run else { return };
        run.progress.cancel();
        self.post_office_progress();
    }

    /// Shows the run's progress card.
    fn post_office_progress(&mut self) {
        let Some(run) = &self.office_run else { return };
        let (done, total) = run.progress.read();
        let total = total.max(1);
        let cancelling = run.progress.cancelled();
        let label = if cancelling {
            tl!("Cancelling…").to_string()
        } else {
            let name = run.names.get(done).map_or("", String::as_str);
            crate::i18n::fmt(
                tl!("Converting {name} to PDF… file {d} of {t}"),
                &[("name", name), ("d", &(done + 1).min(total).to_string()), ("t", &total.to_string())],
            )
        };
        let fraction = done.min(total) as f32 / total as f32;
        notices::set_progress(self, ProgressKey::Office, ProgressNotice { label, fraction, cancellable: !cancelling });
    }

    /// Shows the run's progress, and carries the flow on when the batch is done.
    pub(crate) fn poll_office(&mut self) {
        let Some(run) = self.office_run.as_ref() else { return };
        let progress = run.progress.clone();
        if let Some(finished) = self.office_jobs.try_recv() {
            let Some(run) = self.office_run.take() else { return };
            notices::end_progress(self, ProgressKey::Office);
            let OfficeOutcome { converted, cancelled } = finished.result;
            if cancelled {
                self.notify_tr("Converting was cancelled. Nothing was added.");
                return;
            }
            (run.done)(self, converted);
            return;
        }
        // A worker that ended without a result (it stopped unexpectedly) must not leave the card up for ever.
        let counts = self.office_jobs.counts();
        if counts.queued + counts.running + counts.finished == 0 {
            self.office_run = None;
            notices::end_progress(self, ProgressKey::Office);
            self.notify_tr("The conversion stopped unexpectedly. Nothing was added.");
            return;
        }
        if progress.take_change().is_some() {
            self.post_office_progress();
        }
    }

    /// Says which files were left out and why: the first few, and how many more.
    pub(crate) fn say_office_left_out(&mut self, left_out: &[LeftOut]) {
        let Some((_, first)) = left_out.first() else { return };
        let more = left_out.len().saturating_sub(SHOWN);
        let more_of = |list: String| {
            if more > 0 { crate::i18n::fmt(tl!("{list}, and {more} more"), &[("list", &list), ("more", &more.to_string())]) } else { list }
        };
        if left_out.iter().all(|(_, why)| why == first) {
            let names = more_of(left_out.iter().take(SHOWN).map(|(name, _)| name.as_str()).collect::<Vec<_>>().join(", "));
            self.notify_fmt("Left out: {names}. {why}", &[("names", &names), ("why", first)]);
        } else {
            let list = more_of(left_out.iter().take(SHOWN).map(|(name, why)| format!("{name} ({why})")).collect::<Vec<_>>().join(", "));
            self.notify_fmt("Left out: {names}", &[("names", &list)]);
        }
    }

    /// [`Self::convert_office`] for files in a flow's own form: each Office document among `files` keeps its name and
    /// gets its PDF as bytes (the flows go by what the bytes are, so they take it as the PDF it now is); one that can't
    /// be converted is named and left out; the rest are as they were. `then` gets the files in the same order and the
    /// positions among them of the documents that were converted.
    pub(crate) fn convert_office_files(&mut self, files: Vec<Item>, then: FilesDone) {
        let mut files: Vec<Option<Item>> = files.into_iter().map(Some).collect();
        let mut slots = Vec::new();
        let mut docs = Vec::new();
        for (slot, file) in files.iter_mut().enumerate() {
            if file.as_ref().is_some_and(|f| matches!(f, Ok((name, bytes)) if is_office(name, bytes)))
                && let Some(Ok((name, bytes))) = file.take()
            {
                slots.push((slot, name.clone()));
                docs.push((name, Arc::new(bytes)));
            }
        }
        self.convert_office(
            docs,
            Box::new(move |app, results| {
                let (mut left_out, mut made) = (Vec::new(), Vec::new());
                for ((slot, name), result) in slots.into_iter().zip(results) {
                    match result {
                        Ok(pdf) => {
                            made.push(slot);
                            if let Some(file) = files.get_mut(slot) {
                                *file = Some(Ok((name, Arc::try_unwrap(pdf).unwrap_or_else(|shared| shared.to_vec()))));
                            }
                        }
                        Err(why) => left_out.push((name, why)),
                    }
                }
                app.say_office_left_out(&left_out);
                let (mut items, mut converted) = (Vec::new(), Vec::new());
                for (slot, file) in files.into_iter().enumerate() {
                    let Some(file) = file else { continue };
                    if made.contains(&slot) {
                        converted.push(items.len());
                    }
                    items.push(file);
                }
                then(app, items, converted);
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn office_names_are_matched_in_any_case_and_bytes_decide_what_a_file_is() {
        for name in ["a.docx", "A.DOCX", "Sheet.XlSx", "deck.PPTX", "old.doc", "x.odt", "notes.RTF"] {
            assert!(has_office_name(name), "{name}");
        }
        for name in ["a.pdf", "docx", "a.docx.txt", "", "photo.png"] {
            assert!(!has_office_name(name), "{name}");
        }
        let package = b"PK\x03\x04\x14\x00";
        assert!(is_office("Report.DOCX", package));
        // The name alone is not enough, and a PDF stays a PDF whatever it is called.
        assert!(!is_office("Report.docx", b"plain text"));
        assert!(!is_office("Report.docx", b"%PDF-1.7\n"));
        assert!(!is_office("archive.zip", package));
    }
}
