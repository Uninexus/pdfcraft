//! Word, Excel and PowerPoint files in Insert pages, Combine files, Create a PDF from multiple files and Insert pages
//! from the clipboard, in the real shell (egui_kittest). The app converts with a stand-in the test gives it, never
//! LibreOffice: the last byte of a document says how many pages its PDF has (`1` to `3`), and `X` fails.

use std::sync::{Arc, Mutex};

use crate::support::{AppHarness as _, RecordingClipboard, shown_notice, step_until};
use craft::host::{ManualJobs, ScriptedFiles};
use egui_kittest::Harness;
use pdfcraft_engine::Session;
use pdfcraft_ui_egui::{FilePurpose, OfficeConverter, PdfCraftApp};

type Log = Arc<Mutex<Vec<String>>>;

fn blank(pages: usize) -> Vec<u8> {
    Session::new().create_blank(200.0, 200.0, pages).unwrap().as_ref().clone()
}

/// A Word-like file whose stand-in PDF has `marker` pages (`b'1'`), or fails (`b'X'`).
fn office(marker: u8) -> Vec<u8> {
    let mut bytes = b"PK\x03\x04\x14\x00".to_vec();
    bytes.push(marker);
    bytes
}

/// A converter that writes the names it converts to `log`.
fn stand_in(log: &Log) -> OfficeConverter {
    let log = log.clone();
    OfficeConverter::new(
        || true,
        move |name, bytes| {
            log.lock().unwrap().push(name.to_string());
            match bytes.last() {
                Some(n @ b'1'..=b'3') => Session::new().create_blank(200.0, 200.0, usize::from(n - b'0')).map_err(|e| e.to_string()),
                _ => Err("the file is damaged".to_string()),
            }
        },
    )
}

/// An app with a two-page document open, whose background work waits in the returned jobs until the test runs it.
fn harness(converter: OfficeConverter, clipboard: RecordingClipboard) -> (Harness<'static, PdfCraftApp>, Arc<ManualJobs>) {
    let jobs = Arc::new(ManualJobs::new());
    let host = crate::support::host_with(jobs.clone(), Arc::new(ScriptedFiles::new()));
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        let mut app = PdfCraftApp::with_host(host, crate::support::services(clipboard));
        app.office_converter = Some(converter);
        app.set_option(&egui::Context::default(), "language", "en").unwrap();
        app.open_bytes(&egui::Context::default(), "target.pdf", None, blank(2)).unwrap();
        app
    });
    let opened = step_until(&mut h, |h| {
        jobs.run_all();
        !h.state().views.is_empty()
    });
    assert!(opened, "the document opened");
    run(&mut h, &jobs);
    (h, jobs)
}

fn run(h: &mut Harness<'static, PdfCraftApp>, jobs: &ManualJobs) {
    h.run_steps(3);
    jobs.run_all();
    h.run_steps(4);
}

fn pages(h: &Harness<'static, PdfCraftApp>) -> usize {
    let app = h.state();
    app.session.get(app.views[0].id).unwrap().info.pages.len()
}

fn insert(h: &mut Harness<'static, PdfCraftApp>, files: Vec<(String, Vec<u8>)>) {
    let ctx = h.ctx.clone();
    h.state_mut().insert_files(&ctx, files);
}

#[test]
fn insert_converts_office_files_in_the_background_and_adds_their_pages_in_order() {
    let log = Log::default();
    let (mut h, jobs) = harness(stand_in(&log), RecordingClipboard::default());
    insert(
        &mut h,
        vec![
            ("one.pdf".into(), blank(2)),
            ("two.docx".into(), office(b'3')),
            ("three.txt".into(), b"hello".to_vec()),
            ("four.xlsx".into(), office(b'1')),
        ],
    );
    h.run_steps(2);
    assert_eq!(pages(&h), 2, "nothing is inserted until the conversion is done");
    assert!(h.state().office_run.is_some(), "the conversion is running");
    assert!(log.lock().unwrap().is_empty(), "the frame thread converts nothing");
    run(&mut h, &jobs);
    assert!(h.state().office_run.is_none());
    assert_eq!(*log.lock().unwrap(), ["two.docx", "four.xlsx"], "converted in the order given");
    assert_eq!(pages(&h), 2 + 2 + 3 + 1 + 1);
    assert_eq!(h.state().views[0].target_pages().len(), 7, "the inserted pages are selected");
}

#[test]
fn without_a_converter_the_office_file_is_named_with_the_reason_and_the_others_still_go_in() {
    let (mut h, jobs) = harness(OfficeConverter::unavailable(), RecordingClipboard::default());
    insert(&mut h, vec![("report.docx".into(), office(b'2')), ("one.pdf".into(), blank(2))]);
    run(&mut h, &jobs);
    assert_eq!(pages(&h), 4, "the PDF went in");
    let said = shown_notice(&h).expect("the user is told");
    assert!(said.contains("report.docx") && said.contains("LibreOffice"), "{said}");
}

#[test]
fn a_file_that_fails_to_convert_is_named_and_left_out() {
    let log = Log::default();
    let (mut h, jobs) = harness(stand_in(&log), RecordingClipboard::default());
    insert(&mut h, vec![("bad.docx".into(), office(b'X')), ("good.pptx".into(), office(b'2'))]);
    run(&mut h, &jobs);
    assert_eq!(pages(&h), 4, "the good file's two pages went in");
    let said = shown_notice(&h).expect("the user is told");
    assert!(said.contains("bad.docx") && said.contains("damaged"), "{said}");
}

#[test]
fn cancelling_stops_the_conversion_and_adds_nothing() {
    let log = Log::default();
    let (mut h, jobs) = harness(stand_in(&log), RecordingClipboard::default());
    insert(&mut h, vec![("a.docx".into(), office(b'2')), ("b.docx".into(), office(b'2'))]);
    h.run_steps(3);
    assert!(h.state().office_run.is_some());
    h.state_mut().cancel_office();
    run(&mut h, &jobs);
    assert!(h.state().office_run.is_none(), "the run ended");
    assert!(log.lock().unwrap().is_empty(), "nothing was converted");
    assert_eq!(pages(&h), 2);
    assert!(shown_notice(&h).is_some_and(|n| n.contains("cancelled")), "{:?}", shown_notice(&h));
}

#[test]
fn pages_are_not_inserted_into_a_document_that_changed_while_converting() {
    let log = Log::default();
    let (mut h, jobs) = harness(stand_in(&log), RecordingClipboard::default());
    insert(&mut h, vec![("a.docx".into(), office(b'2'))]);
    h.run_steps(2);
    let ctx = h.ctx.clone();
    assert!(h.state_mut().apply_edit(&ctx, pdfcraft_engine::Edit::DeletePages { pages: vec![0] }));
    run(&mut h, &jobs);
    assert_eq!(pages(&h), 1, "only the user's own edit happened");
}

#[test]
fn copied_office_files_are_converted_and_inserted_with_the_pdfs() {
    let dir = crate::support::TempDir::new();
    let (word, pdf) = (dir.path().join("memo.docx"), dir.path().join("two.pdf"));
    std::fs::write(&word, office(b'3')).unwrap();
    std::fs::write(&pdf, blank(2)).unwrap();
    let log = Log::default();
    let (mut h, jobs) = harness(stand_in(&log), RecordingClipboard::holding_files(vec![word, pdf]));
    assert!(h.execute("page.insert_clipboard"));
    run(&mut h, &jobs);
    assert_eq!(*log.lock().unwrap(), ["memo.docx"]);
    assert_eq!(pages(&h), 2 + 3 + 2);
    assert_eq!(h.state().views[0].target_pages().len(), 5);
}

#[test]
fn combine_converts_office_files_and_they_join_the_list_in_their_place() {
    let log = Log::default();
    let (mut h, jobs) = harness(stand_in(&log), RecordingClipboard::default());
    let ctx = h.ctx.clone();
    let files = vec![("first.pdf".into(), blank(1)), ("memo.docx".into(), office(b'3')), ("bad.docx".into(), office(b'X'))];
    h.state_mut().use_files(&ctx, FilePurpose::Combine, files);
    run(&mut h, &jobs);
    let listed: Vec<(String, usize)> = h.state().combine_draft.iter().map(|f| (f.name.clone(), f.pages)).collect();
    assert_eq!(listed, [("first.pdf".to_string(), 1), ("memo.docx".to_string(), 3)], "the failed file is left out");
    assert!(shown_notice(&h).is_some_and(|n| n.contains("bad.docx")), "{:?}", shown_notice(&h));
}

#[test]
fn create_from_multiple_files_converts_office_files_before_combining() {
    let log = Log::default();
    let (mut h, jobs) = harness(stand_in(&log), RecordingClipboard::default());
    let ctx = h.ctx.clone();
    let files = vec![("notes.txt".into(), b"text".to_vec()), ("deck.pptx".into(), office(b'2')), ("a.pdf".into(), blank(1))];
    h.state_mut().use_files(&ctx, FilePurpose::CreateMultiple, files);
    run(&mut h, &jobs);
    let made = h.state().session.docs().iter().find(|d| d.name == "Combined.pdf").map(|d| d.info.pages.len());
    assert_eq!(made, Some(1 + 2 + 1));
}
