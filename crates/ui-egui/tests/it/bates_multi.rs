//! Edit ▸ Bates numbering on several files in the real shell (egui_kittest): the dialog, the numbers that run on from
//! one file to the next, the folder pick, and the cases that must write nothing or never replace a file.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::support::{AppHarness as _, fixtures, shown_notice};
use craft::host::{ManualJobs, Picked, ScriptedFiles};
use craft::ui::dialog::dialog_id;
use craft::ui::patterns::form_dialogs::{FieldValue, HostedParamDialog};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_render::{PageRenderer, RenderRequest, RequestKind};
use pdfcraft_ui_egui::PdfCraftApp;

fn input(name: &str) -> PathBuf {
    Path::new("/in").join(name)
}

fn output(name: &str) -> PathBuf {
    Path::new("/out").join(name)
}

/// An app whose jobs wait in the returned queue, whose folder picker answers `/out` and which can read `a.pdf` (two
/// pages) and `b.pdf` (three) from `/in`.
fn harness(extra: impl FnOnce(ScriptedFiles) -> ScriptedFiles) -> (Harness<'static, PdfCraftApp>, Arc<ScriptedFiles>, Arc<ManualJobs>) {
    let files = ScriptedFiles::answering([Picked::Paths(vec![PathBuf::from("/out")])])
        .with_file(input("a.pdf"), fixtures::pdf_pages(2))
        .with_file(input("b.pdf"), fixtures::pdf_pages(3));
    let files = Arc::new(extra(files));
    let jobs = Arc::new(ManualJobs::new());
    let app = crate::support::test_app(crate::support::host_with(jobs.clone(), files.clone()));
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        let mut app = app;
        app.set_option(&egui::Context::default(), "language", "en").unwrap();
        app
    });
    h.run_steps(4);
    (h, files, jobs)
}

/// Opens the dialog and lists `names` from `/in`, numbering from `start` with the text `BX-` in front.
fn fill(h: &mut Harness<'static, PdfCraftApp>, names: &[&str], start: i64) {
    assert!(h.execute("edit.bates_multi"));
    h.run_steps(3);
    let dialog = h.state_mut().dialogs.get_mut_by_id::<HostedParamDialog>(dialog_id("edit.bates_multi")).expect("the dialog is open");
    let list = names.iter().map(|n| FieldValue::Text(input(n).to_string_lossy().into_owned())).collect();
    dialog.set_field("files", FieldValue::List(list));
    dialog.set_field("start", start.into());
    dialog.set_field("prefix", "BX-".into());
    h.run_steps(3);
}

fn run_jobs(h: &mut Harness<'static, PdfCraftApp>, jobs: &ManualJobs) {
    h.run_steps(3);
    jobs.run_all();
    h.run_steps(4);
}

fn texts(bytes: Vec<u8>) -> Vec<String> {
    let mut r = PageRenderer::new(Arc::new(bytes), Default::default());
    (0..r.page_count())
        .map(|p| {
            r.render(RenderRequest { page: p, kind: RequestKind::Text, scale: 1.0, ..Default::default() })
                .text
                .map(|t| t.plain_text())
                .unwrap_or_default()
        })
        .collect()
}

#[test]
fn the_numbers_run_on_from_one_file_to_the_next_and_the_copies_go_to_the_chosen_folder() {
    let (mut h, files, jobs) = harness(|f| f);
    fill(&mut h, &["a.pdf", "b.pdf"], 10);
    h.get_by_role_and_label(Role::Button, "Number files").click();
    run_jobs(&mut h, &jobs);
    let a = texts(files.file(&output("a.pdf")).expect("a.pdf was written to the chosen folder"));
    let b = texts(files.file(&output("b.pdf")).expect("b.pdf was written"));
    assert!(a[0].contains("BX-000010") && a[1].contains("BX-000011"), "{a:?}");
    assert!(b[0].contains("BX-000012") && b[2].contains("BX-000014"), "{b:?}");
    assert!(files.file(&input("a.pdf")).is_some_and(|bytes| texts(bytes).iter().all(|t| !t.contains("BX-"))), "the original is unchanged");
    assert!(shown_notice(&h).is_some_and(|n| n.contains("Numbered 2 files")), "{:?}", shown_notice(&h));
}

#[test]
fn a_file_that_cannot_be_read_stops_the_run_with_nothing_written() {
    let (mut h, files, jobs) = harness(|f| f);
    fill(&mut h, &["a.pdf", "missing.pdf"], 1);
    h.get_by_role_and_label(Role::Button, "Number files").click();
    run_jobs(&mut h, &jobs);
    assert!(files.written().is_empty(), "{:?}", files.written());
    assert!(shown_notice(&h).is_some_and(|n| n.contains("Nothing was numbered") && n.contains("missing.pdf")), "{:?}", shown_notice(&h));
}

#[test]
fn a_name_already_in_the_folder_is_never_replaced() {
    let (mut h, files, jobs) = harness(|f| f.with_file(output("a.pdf"), b"keep this file".to_vec()));
    fill(&mut h, &["a.pdf"], 1);
    h.get_by_role_and_label(Role::Button, "Number files").click();
    run_jobs(&mut h, &jobs);
    assert_eq!(files.file(&output("a.pdf")).as_deref(), Some(&b"keep this file"[..]));
    assert!(files.file(&output("a (2).pdf")).is_some(), "{:?}", files.written());
}

#[test]
fn cancelling_before_the_run_starts_writes_nothing_and_says_so() {
    let (mut h, files, jobs) = harness(|f| f);
    fill(&mut h, &["a.pdf", "b.pdf"], 1);
    h.get_by_role_and_label(Role::Button, "Number files").click();
    h.run_steps(4);
    assert!(h.state().bates_multi_run.is_some(), "numbering in the background");
    h.state_mut().cancel_bates_multi();
    run_jobs(&mut h, &jobs);
    assert!(h.state().bates_multi_run.is_none());
    assert!(files.written().is_empty());
    assert!(shown_notice(&h).is_some_and(|n| n.contains("cancelled")), "{:?}", shown_notice(&h));
}

#[test]
fn the_edit_menu_lists_it_and_the_dialog_asks_for_files_first() {
    let (mut h, _files, _jobs) = harness(|f| f);
    crate::support::open_menu(&mut h, "Edit");
    crate::support::menu_row(&h, "Bates numbering on several files…").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Dialog, "Bates numbering on several files");
    h.get_by_label_contains("Add the files to number.");
}
