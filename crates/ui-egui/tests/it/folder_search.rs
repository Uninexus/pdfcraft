//! Edit ▸ Search all PDFs in a folder in the real shell (egui_kittest): the Search panel's Folder scope, the
//! background search with its Cancel, the results listed by file, and a click that opens a file at its page.

use std::path::Path;
use std::sync::Arc;

use crate::support::{AppHarness as _, TempDir, fixtures, shown_notice, step_until};
use craft::host::{ManualJobs, Picked, ScriptedFiles};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

/// A PDF with one line of text on each page.
fn pdf(pages: &[&str]) -> Vec<u8> {
    let n = pages.len();
    let mut objs = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {n} >>", (0..n).map(|i| format!("{} 0 R", 4 + 2 * i)).collect::<Vec<_>>().join(" ")),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    for (i, text) in pages.iter().enumerate() {
        let body = format!("BT /F1 14 Tf 20 150 Td ({text}) Tj ET");
        objs.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
            5 + 2 * i
        ));
        objs.push(format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()));
    }
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

/// A folder with two searchable files and one that is not a PDF.
fn folder() -> TempDir {
    let dir = TempDir::new();
    std::fs::write(dir.path().join("one.pdf"), pdf(&["alpha invoice"])).unwrap();
    std::fs::write(dir.path().join("two.pdf"), pdf(&["beta words", "gamma invoice"])).unwrap();
    std::fs::write(dir.path().join("broken.pdf"), b"not a pdf").unwrap();
    dir
}

/// A document open in an app whose jobs wait until the test runs them and whose folder picker answers `dir`.
fn harness(dir: &Path) -> (Harness<'static, PdfCraftApp>, Arc<ManualJobs>) {
    let jobs = Arc::new(ManualJobs::new());
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![dir.to_path_buf()])]));
    let app = crate::support::test_app(crate::support::host_with(jobs.clone(), files));
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        let mut app = app;
        app.set_option(&egui::Context::default(), "language", "en").unwrap();
        app.open_bytes(&egui::Context::default(), "open.pdf", None, fixtures::pdf_pages(1)).unwrap();
        app
    });
    h.run_steps(4);
    (h, jobs)
}

fn run_jobs(h: &mut Harness<'static, PdfCraftApp>, jobs: &ManualJobs) {
    jobs.run_all();
    h.run_steps(4);
}

/// Opens the folder scope on the picked folder and types the query.
fn ready(h: &mut Harness<'static, PdfCraftApp>, query: &str) {
    assert!(h.execute("edit.search_folder"));
    h.run_steps(4);
    assert!(h.state().folder_search.on, "the Folder scope is showing");
    assert!(h.state().folder_search.folder.is_some(), "the picked folder is kept");
    h.state_mut().views[0].find.as_mut().unwrap().query = query.into();
    h.run_steps(2);
}

#[test]
fn the_results_are_listed_by_file_and_a_click_opens_the_file_at_its_page() {
    let dir = folder();
    let (mut h, jobs) = harness(dir.path());
    ready(&mut h, "invoice");
    h.get_by_role_and_label(Role::Button, "Search folder").click();
    h.run_steps(3);
    assert!(h.state().folder_search.running(), "searching in the background");
    run_jobs(&mut h, &jobs);
    let fs = &h.state().folder_search;
    assert!(!fs.running() && fs.summary.is_some());
    let found: Vec<(String, Vec<usize>)> =
        fs.results.iter().map(|f| (f.path.file_name().unwrap().to_string_lossy().into_owned(), f.matches.iter().map(|m| m.page).collect())).collect();
    assert_eq!(found, [("one.pdf".to_owned(), vec![0]), ("two.pdf".to_owned(), vec![1])]);
    assert_eq!(fs.skipped.len(), 1, "the file that is not a PDF is skipped, not fatal");
    h.get_by_label("one.pdf (1)");
    h.get_by_label("two.pdf (1)");
    // Clicking the match on page 2 of two.pdf opens that file there.
    h.get_by_label_contains("gamma").click();
    let opened = step_until(&mut h, |h| {
        jobs.run_all();
        h.state().views.len() == 2 && h.state().views[1].current() == 1
    });
    assert!(opened, "two.pdf opens on its second page");
}

#[test]
fn stopping_keeps_what_was_found_and_says_the_list_is_incomplete() {
    let dir = folder();
    let (mut h, jobs) = harness(dir.path());
    ready(&mut h, "invoice");
    h.get_by_role_and_label(Role::Button, "Search folder").click();
    h.run_steps(3);
    h.state_mut().cancel_folder_search();
    run_jobs(&mut h, &jobs);
    let fs = &h.state().folder_search;
    assert!(!fs.running(), "the search ended");
    assert!(fs.stopped, "and says it was stopped");
    assert!(fs.found() < 2, "not every file was read");
    assert!(shown_notice(&h).is_some_and(|n| n.contains("stopped")), "{:?}", shown_notice(&h));
}

#[test]
fn a_search_without_text_is_refused_and_starts_nothing() {
    let dir = folder();
    let (mut h, jobs) = harness(dir.path());
    ready(&mut h, "  ");
    h.state_mut().start_folder_search();
    run_jobs(&mut h, &jobs);
    assert!(!h.state().folder_search.running() && h.state().folder_search.summary.is_none());
    assert!(shown_notice(&h).is_some_and(|n| n.contains("Type what to search for")), "{:?}", shown_notice(&h));
}

#[test]
fn the_edit_menu_lists_it_and_the_panel_leads_back_to_the_document() {
    let dir = folder();
    let (mut h, _jobs) = harness(dir.path());
    crate::support::open_menu(&mut h, "Edit");
    crate::support::menu_row(&h, "Search all PDFs in a folder…").click();
    h.run_steps(4);
    assert!(h.state().folder_search.on);
    assert_eq!(crate::support::right(h.state()), Some(pdfcraft_ui_egui::RightPanel::Search));
    h.get_by_role_and_label(Role::Button, "Search this document").click();
    h.run_steps(3);
    assert!(!h.state().folder_search.on);
    h.get_by_role_and_label(Role::Button, "Search all PDFs in a folder…");
}

#[test]
fn comments_and_bookmarks_of_the_files_are_listed_when_asked() {
    use pdfcraft_engine::{Edit, NewAnnotation, NoteIcon, Session, Shape};

    let dir = folder();
    // "cherry" is only in a comment of one file.
    let mut session = Session::new();
    let id = session.open("one.pdf", None, pdf(&["alpha invoice"]).into(), None).unwrap();
    let note = NewAnnotation {
        page: 0,
        shape: Shape::Note { at: [20.0, 100.0], icon: NoteIcon::Comment },
        style: Default::default(),
        contents: "ripe cherry".into(),
        author: "Tester".into(),
    };
    session.apply(id, Edit::AddAnnotation(note)).unwrap();
    std::fs::write(dir.path().join("one.pdf"), session.save_bytes(id).unwrap().as_slice()).unwrap();
    let (mut h, jobs) = harness(dir.path());
    ready(&mut h, "cherry");
    h.get_by_label("Include comments").click();
    h.run_steps(2);
    h.get_by_role_and_label(Role::Button, "Search folder").click();
    h.run_steps(2);
    run_jobs(&mut h, &jobs);
    let found: Vec<_> = h.state().folder_search.results.iter().flat_map(|f| f.matches.iter()).map(|m| m.snippet()).collect();
    assert_eq!(found, ["Comment: ripe cherry"]);
}
