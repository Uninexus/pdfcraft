//! Word, Excel, PowerPoint, OpenDocument and RTF files in Insert pages, Combine files and
//! Create ▸ Multiple files. LibreOffice takes seconds per document, so the conversions run on a
//! worker thread, one batch at a time per request, and the flow that asked carries on with the
//! converted PDFs when they are ready, as File ▸ Open does for one document. The conversion is
//! [`pdfcraft_engine::OfficeRunner`]'s: the same private temporary folder, fresh profile with
//! macros and link updates off, and time and size limits as everywhere else.
//!
//! A batch is all or nothing about what is missing: with no LibreOffice installed, nothing is
//! converted and nothing is added, and the one message says what to install. A document that
//! fails to convert (damaged, password-protected) is left to the flow, which names it as it
//! names any file it can't read.

use std::sync::Arc;
use std::sync::mpsc;

use pdfcraft_engine::{OfficeRunner, SourceKind, source_kind};

use crate::PdfCraftApp;

/// A picked file as the flows take it: its name and bytes, or its name and why it couldn't be read.
pub(crate) type Item = Result<(String, Vec<u8>), (String, String)>;

/// What a conversion batch made: a PDF or the reason for each document, in order; or the one
/// reason nothing could be converted.
type Outcome = Result<Vec<Result<Arc<Vec<u8>>, String>>, String>;

/// What a flow does with its batch's results (each document's PDF or why not, in order).
type Done = Box<dyn FnOnce(&mut PdfCraftApp, Vec<Result<Arc<Vec<u8>>, String>>)>;

/// What a flow does with its files once the Office documents among them are converted.
pub(crate) type FilesDone = Box<dyn FnOnce(&mut PdfCraftApp, Vec<Item>, Vec<usize>)>;

/// A batch converting on its worker thread.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct OfficeJob {
    receiver: mpsc::Receiver<Outcome>,
    done: Done,
    /// Said after the reason when nothing could be converted.
    note: String,
}

/// Whether the name has an Office document's extension. Only the name: used before a file is
/// read, to decide how to take a batch. What a file is is decided by [`source_kind`] on its bytes.
pub(crate) fn has_office_name(name: &str) -> bool {
    // The web build has no LibreOffice: Office files are not special there, and are refused as
    // any file of a kind the flow doesn't take.
    !cfg!(target_arch = "wasm32")
        && name.rsplit_once('.').is_some_and(|(_, ext)| pdfcraft_engine::OFFICE_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
}

/// Whether `bytes` named `name` is an Office document to convert (a PDF with an Office name is
/// not: it is a PDF).
pub(crate) fn is_office(name: &str, bytes: &[u8]) -> bool {
    !cfg!(target_arch = "wasm32") && source_kind(name, bytes) == Some(SourceKind::Office)
}

/// The text of an engine error without the file's name, which the caller already shows.
fn why(name: &str, error: &pdfcraft_engine::EditError) -> String {
    let text = error.to_string();
    text.strip_prefix(name).and_then(|rest| rest.strip_prefix(": ")).map_or(text.clone(), str::to_string)
}

/// Convert `docs` in order with `runner`: all of them or, when LibreOffice can't be found, none.
fn convert(runner: &OfficeRunner, docs: &[(String, Arc<Vec<u8>>)]) -> Outcome {
    let converted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runner.convert_all(docs)))
        .map_err(|_| "the conversion stopped unexpectedly".to_string())?
        .map_err(|e| e.to_string())?;
    Ok(converted.into_iter().zip(docs).map(|(result, (name, _))| result.map_err(|e| why(name, &e))).collect())
}

impl PdfCraftApp {
    /// Convert the Office documents `docs` (name and bytes) off the frame thread, then call `done`
    /// with each one's PDF or why it failed, in order. When LibreOffice can't be found nothing is
    /// converted, `done` is not called and the user is told what to install. Under
    /// [`PdfCraftApp::run_inline`], without a window to wake (tests) and on the web, which has no LibreOffice, it runs here.
    ///
    /// `note` is added to the message when nothing could be converted (what the flow had already
    /// left out, say). Batches are applied as their workers finish, which is not always the order
    /// they were asked for.
    pub(crate) fn convert_office(&mut self, docs: Vec<(String, Arc<Vec<u8>>)>, done: Done, note: String) {
        if docs.is_empty() {
            return done(self, Vec::new());
        }
        let runner = self.session.office_runner();
        #[cfg(not(target_arch = "wasm32"))]
        let docs_count = docs.len();
        let (sender, receiver) = mpsc::channel();
        let ctx = self.ctx.clone();
        let work = move || {
            let _ = sender.send(convert(&runner, &docs));
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        if !self.run_inline {
            // Without a window to wake (startup, before the first frame) the frame loop still
            // polls the job once it runs.
            self.notify(crate::i18n::trn(
                crate::i18n::current(),
                docs_count as u64,
                "Converting {n} Office document with LibreOffice…",
                "Converting {n} Office documents with LibreOffice…",
            ));
            match std::thread::Builder::new().name("pdfcraft-office".into()).spawn(work) {
                Ok(_) => self.office_jobs.push(OfficeJob { receiver, done, note }),
                Err(e) => self.notify_error(e),
            }
            return;
        }
        work();
        self.finish_office(receiver.try_recv().unwrap_or_else(|_| Err("the conversion stopped unexpectedly".into())), done, &note);
    }

    /// Take in the batches whose worker has finished (called every frame).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn poll_office_jobs(&mut self) {
        for job in std::mem::take(&mut self.office_jobs) {
            match job.receiver.try_recv() {
                Ok(outcome) => self.finish_office(outcome, job.done, &job.note),
                Err(mpsc::TryRecvError::Empty) => self.office_jobs.push(job),
                Err(mpsc::TryRecvError::Disconnected) => self.finish_office(Err("the conversion stopped unexpectedly".into()), job.done, &job.note),
            }
        }
    }

    fn finish_office(&mut self, outcome: Outcome, done: Done, note: &str) {
        match outcome {
            Ok(results) => done(self, results),
            Err(message) if note.is_empty() => self.notify_error(message),
            Err(message) => self.notify_error(format!("{message} {note}")),
        }
    }

    /// [`Self::convert_office`] for files in a flow's own form: each Office document among
    /// `files` keeps its name and gets its PDF as bytes (the flows go by what the bytes are, so
    /// they take it as the PDF it now is) or, when it couldn't be converted, is replaced by the
    /// reason; the rest are as they were. `then` gets the list in the same order and the
    /// positions of the documents that were converted, for a flow that treats them differently
    /// from PDFs it was given. When LibreOffice can't be found `then` is not called.
    pub(crate) fn convert_office_files(&mut self, files: Vec<Item>, then: FilesDone, note: String) {
        let mut files: Vec<Option<Item>> = files.into_iter().map(Some).collect();
        let mut slots = Vec::new();
        let mut docs = Vec::new();
        for (slot, file) in files.iter_mut().enumerate() {
            if file.as_ref().is_some_and(|f| f.as_ref().is_ok_and(|(name, bytes)| is_office(name, bytes)))
                && let Some(Ok((name, bytes))) = file.take()
            {
                slots.push((slot, name.clone()));
                docs.push((name, Arc::new(bytes)));
            }
        }
        self.convert_office(
            docs,
            Box::new(move |app, results| {
                let mut converted = Vec::new();
                for ((slot, name), result) in slots.into_iter().zip(results) {
                    let item = match result {
                        Ok(pdf) => {
                            converted.push(slot);
                            Ok((name, Arc::try_unwrap(pdf).unwrap_or_else(|shared| shared.to_vec())))
                        }
                        Err(why) => Err((name, why)),
                    };
                    if let Some(file) = files.get_mut(slot) {
                        *file = Some(item);
                    }
                }
                then(app, files.into_iter().flatten().collect(), converted);
            }),
            note,
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

    #[cfg(unix)]
    mod stand_in {
        use super::*;
        use pdfcraft_engine::{DocId, OfficeLocator, Session};
        use std::path::PathBuf;
        use std::sync::Arc;

        /// A folder of the test's own, removed when dropped.
        struct TempDir(PathBuf);

        impl TempDir {
            fn new() -> Self {
                use std::sync::atomic::{AtomicU64, Ordering};
                static NEXT: AtomicU64 = AtomicU64::new(0);
                let dir = std::env::temp_dir().join(format!("pdfcraft-office-batch-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
                std::fs::create_dir_all(&dir).unwrap();
                Self(dir)
            }

            fn path(&self) -> &std::path::Path {
                &self.0
            }
        }

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        /// A shell script standing in for LibreOffice: the document's last byte `1`, `2` or `3` makes a
        /// PDF of that many pages, `X` fails.
        struct Fake {
            dir: TempDir,
            program: PathBuf,
        }

        impl Fake {
            fn new() -> Self {
                use std::io::Write;
                use std::os::unix::fs::PermissionsExt;
                let dir = TempDir::new();
                let maker = Session::new();
                for n in 1..=3 {
                    std::fs::write(dir.path().join(format!("p{n}.pdf")), maker.create_blank(200.0, 200.0, n).unwrap().as_slice()).unwrap();
                }
                let script = format!(
                    "#!/bin/sh\n[ $# -eq 0 ] && exit 0\nfor input; do :; done\nwhile [ $# -gt 0 ]; do case \"$1\" in --outdir) out=\"$2\";; esac; shift; done\n\
                     n=$(tail -c 1 \"$input\")\necho \"$n $input\" >> '{d}/log'\n[ \"$n\" = X ] && exit 3\ncp '{d}/p'\"$n\".pdf \"$out/document.pdf\"\n",
                    d = dir.path().display()
                );
                let program = dir.path().join("soffice");
                let mut file = std::fs::File::create(&program).unwrap();
                file.write_all(script.as_bytes()).unwrap();
                file.set_permissions(std::fs::Permissions::from_mode(0o755)).unwrap();
                file.sync_all().unwrap();
                drop(file);
                // Linux refuses to run a script another thread's fork may still hold open for
                // writing ("Text file busy"): run it once, with no arguments, until it starts.
                for _ in 0..100 {
                    match std::process::Command::new(&program).status() {
                        Err(e) if e.to_string().contains("Text file busy") => std::thread::sleep(std::time::Duration::from_millis(50)),
                        _ => break,
                    }
                }
                Self { dir, program }
            }

            fn install(&self, app: &mut PdfCraftApp) {
                let program = self.program.clone();
                let locator: OfficeLocator = Arc::new(move || Some(program.clone()));
                app.session.set_office_locator(locator);
            }

            fn converted(&self) -> String {
                std::fs::read_to_string(self.dir.path().join("log")).unwrap_or_default().lines().filter_map(|l| l.split(' ').next()).collect()
            }

            fn work_dirs_removed(&self) -> bool {
                let log = std::fs::read_to_string(self.dir.path().join("log")).unwrap_or_default();
                log.lines().filter_map(|l| l.split_once(' ')).all(|(_, input)| std::path::Path::new(input).parent().is_some_and(|d| !d.exists()))
            }
        }

        fn office(marker: char) -> Vec<u8> {
            let mut bytes = b"PK\x03\x04\x14\x00".to_vec();
            bytes.push(marker as u8);
            bytes
        }

        fn app_with_document(pages: usize) -> (PdfCraftApp, DocId) {
            let mut app = PdfCraftApp::new();
            app.set_option("language", "en").unwrap();
            app.run_inline = true;
            let pdf = app.session.create_blank(300.0, 300.0, pages).unwrap();
            app.open_bytes("target.pdf", None, pdf.to_vec()).unwrap();
            let id = app.active_ids().unwrap().1;
            (app, id)
        }

        fn pages(app: &PdfCraftApp, id: DocId) -> usize {
            app.session.get(id).unwrap().info.pages.len()
        }

        fn toast(app: &PdfCraftApp) -> String {
            app.toast.as_ref().map(|(text, _)| text.clone()).unwrap_or_default()
        }

        #[test]
        fn insert_pages_converts_office_documents_in_order_one_file_after_another_with_the_new_pages_selected() {
            let fake = Fake::new();
            let (mut app, id) = app_with_document(2);
            fake.install(&mut app);
            let pdf = app.session.create_blank(100.0, 100.0, 2).unwrap().to_vec();
            app.insert_files(vec![
                ("one.PDF".into(), pdf),
                ("two.DOCX".into(), office('3')),
                ("three.txt".into(), b"hello".to_vec()),
                ("four.xlsx".into(), office('1')),
            ]);
            assert_eq!(pages(&app, id), 2 + 2 + 3 + 1 + 1);
            assert_eq!(fake.converted(), "31", "converted in the order given");
            assert!(fake.work_dirs_removed());
            let selected = app.views[0].target_pages();
            assert_eq!(selected.len(), 7, "the inserted pages are selected: {selected:?}");
            // As for any picked files, each file is its own undo step.
            app.execute("edit.undo");
            assert_eq!(pages(&app, id), 2 + 2 + 3 + 1, "the last file is gone");
            for _ in 0..3 {
                app.execute("edit.undo");
            }
            assert_eq!(pages(&app, id), 2);
        }

        #[test]
        fn insert_pages_without_libreoffice_inserts_nothing_and_says_what_to_install() {
            let (mut app, id) = app_with_document(2);
            let locator: OfficeLocator = Arc::new(|| None);
            app.session.set_office_locator(locator);
            let pdf = app.session.create_blank(100.0, 100.0, 1).unwrap().to_vec();
            app.insert_files(vec![("one.pdf".into(), pdf), ("two.docx".into(), office('1'))]);
            assert_eq!(pages(&app, id), 2, "no partial result");
            let said = toast(&app);
            assert!(said.contains("two.docx") && said.contains("LibreOffice, which isn't installed") && said.contains("libreoffice.org"), "{said}");
        }

        #[test]
        fn a_document_that_fails_is_named_and_the_rest_still_go_in() {
            let fake = Fake::new();
            let (mut app, id) = app_with_document(1);
            fake.install(&mut app);
            app.insert_files(vec![("a.docx".into(), office('1')), ("b.docx".into(), office('X')), ("c.docx".into(), office('2'))]);
            assert_eq!(pages(&app, id), 1 + 1 + 2, "a and c are inserted");
            assert!(toast(&app).contains("b.docx"), "the document that failed is named: {}", toast(&app));
            assert!(fake.work_dirs_removed());
        }

        #[test]
        fn the_conversion_runs_on_a_worker_and_the_pages_arrive_when_it_is_done() {
            let fake = Fake::new();
            let (mut app, id) = app_with_document(2);
            fake.install(&mut app);
            app.run_inline = false;
            app.ctx = Some(egui::Context::default());
            app.insert_files(vec![("a.docx".into(), office('2'))]);
            assert_eq!(app.office_jobs.len(), 1, "handed to a worker");
            assert!(toast(&app).contains("Converting 1 Office document "), "{}", toast(&app));
            let started = std::time::Instant::now();
            while !app.office_jobs.is_empty() {
                assert!(started.elapsed() < std::time::Duration::from_secs(60), "the worker finished");
                app.poll_office_jobs();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            assert_eq!(pages(&app, id), 4);
        }

        #[test]
        fn pages_are_not_inserted_into_another_document_than_the_one_asked() {
            let fake = Fake::new();
            let (mut app, id) = app_with_document(2);
            fake.install(&mut app);
            app.run_inline = false;
            app.ctx = Some(egui::Context::default());
            app.insert_files(vec![("a.docx".into(), office('2'))]);
            // Another document comes to the front while LibreOffice works.
            app.run_inline = true;
            let other = app.session.create_blank(300.0, 300.0, 1).unwrap().to_vec();
            app.open_bytes("other.pdf", None, other).unwrap();
            let other_id = app.active_ids().unwrap().1;
            assert_ne!(id, other_id);
            let started = std::time::Instant::now();
            while !app.office_jobs.is_empty() {
                assert!(started.elapsed() < std::time::Duration::from_secs(60));
                app.poll_office_jobs();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            assert_eq!((pages(&app, id), pages(&app, other_id)), (2, 1), "nothing was inserted anywhere");
            assert!(toast(&app).contains("nothing was inserted"), "{}", toast(&app));
        }

        #[test]
        fn combine_adds_office_documents_in_their_place_among_the_other_files() {
            let fake = Fake::new();
            let mut app = PdfCraftApp::new();
            app.set_option("language", "en").unwrap();
            app.run_inline = true;
            fake.install(&mut app);
            let pdf = app.session.create_from_text("t", "Made here").unwrap().to_vec();
            app.stage_combine(vec![("Quarter.DOCX".into(), office('2')), ("plain.pdf".into(), pdf), ("deck.pptx".into(), office('3'))]);
            let names: Vec<&str> = app.combine_draft.iter().map(|f| f.name.as_str()).collect();
            assert_eq!(names, ["Quarter.DOCX", "plain.pdf", "deck.pptx"]);
            assert_eq!(app.combine_draft.iter().map(|f| f.pages).collect::<Vec<_>>(), [2, 1, 3]);
            assert!(app.combine_draft.iter().all(|f| f.problem.is_none()));
            assert_eq!(fake.converted(), "23");
        }

        #[test]
        fn combine_keeps_a_document_that_fails_to_convert_in_the_list_with_its_problem() {
            let fake = Fake::new();
            let mut app = PdfCraftApp::new();
            app.set_option("language", "en").unwrap();
            app.run_inline = true;
            fake.install(&mut app);
            app.stage_combine(vec![("good.docx".into(), office('1')), ("bad.docx".into(), office('X'))]);
            let names: Vec<&str> = app.combine_draft.iter().map(|f| f.name.as_str()).collect();
            assert_eq!(names, ["good.docx", "bad.docx"]);
            assert!(app.combine_draft[0].problem.is_none());
            let problem = app.combine_draft[1].problem.clone().unwrap_or_default();
            assert!(problem.contains("LibreOffice couldn't convert it"), "{problem}");
        }

        #[test]
        fn a_failed_conversion_batch_still_names_what_the_clipboard_left_out() {
            let (mut app, id) = app_with_document(1);
            let locator: OfficeLocator = Arc::new(|| None);
            app.session.set_office_locator(locator);
            let dir = TempDir::new();
            let doc = dir.path().join("Copied.docx");
            std::fs::write(&doc, office('2')).unwrap();
            // A folder can't be inserted: it is left out before anything is converted.
            app.clipboard_files_override = Some(vec![doc, dir.path().to_path_buf()]);
            app.insert_from_clipboard();
            assert_eq!(pages(&app, id), 1);
            let said = toast(&app);
            assert!(said.contains("libreoffice.org") && said.contains("Left out"), "{said}");
        }

        #[test]
        fn combine_without_libreoffice_adds_none_of_the_batch() {
            let mut app = PdfCraftApp::new();
            app.set_option("language", "en").unwrap();
            app.run_inline = true;
            let locator: OfficeLocator = Arc::new(|| None);
            app.session.set_office_locator(locator);
            let pdf = app.session.create_from_text("t", "Made here").unwrap().to_vec();
            app.stage_combine(vec![("plain.pdf".into(), pdf), ("Quarter.docx".into(), office('2'))]);
            assert!(app.combine_draft.is_empty(), "no partial result");
            assert!(toast(&app).contains("LibreOffice, which isn't installed"), "{}", toast(&app));
        }

        #[test]
        fn create_multiple_files_combines_office_documents_and_separate_writes_each() {
            use crate::FilePurpose;
            let fake = Fake::new();
            let mut app = PdfCraftApp::new();
            app.set_option("language", "en").unwrap();
            app.run_inline = true;
            fake.install(&mut app);
            let files = vec![("notes.txt".to_string(), b"from text".to_vec()), ("Report.docx".to_string(), office('3'))];
            app.use_files(FilePurpose::CreateMultiple, files.clone());
            let id = app.active_ids().unwrap().1;
            assert_eq!(pages(&app, id), 1 + 3);
            let out = TempDir::new();
            app.export_dir_override = Some(out.path().to_string_lossy().into_owned());
            app.use_files(FilePurpose::CreateMultipleSeparate, files);
            assert!(out.path().join("notes.pdf").is_file());
            assert!(out.path().join("Report.pdf").is_file(), "a converted document is written, not skipped as a PDF");
            assert!(fake.work_dirs_removed());
        }

        #[test]
        fn create_multiple_without_libreoffice_makes_nothing() {
            use crate::FilePurpose;
            let mut app = PdfCraftApp::new();
            app.set_option("language", "en").unwrap();
            app.run_inline = true;
            let locator: OfficeLocator = Arc::new(|| None);
            app.session.set_office_locator(locator);
            app.use_files(FilePurpose::CreateMultiple, vec![("notes.txt".into(), b"from text".to_vec()), ("Report.docx".into(), office('3'))]);
            assert!(app.views.is_empty(), "no partial result");
            assert!(toast(&app).contains("LibreOffice, which isn't installed"), "{}", toast(&app));
        }

        #[test]
        fn copied_office_files_are_inserted_from_the_clipboard() {
            let fake = Fake::new();
            let (mut app, id) = app_with_document(1);
            fake.install(&mut app);
            let doc = fake.dir.path().join("Copied.docx");
            std::fs::write(&doc, office('2')).unwrap();
            app.clipboard_files_override = Some(vec![doc]);
            app.insert_from_clipboard();
            assert_eq!(pages(&app, id), 3);
        }
    }
}
