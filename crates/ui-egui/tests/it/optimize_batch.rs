//! File ▸ Optimize multiple files in the real shell (egui_kittest): the dialog, the folder pick, the copies written, and
//! the cases that must say a file failed or never replace one.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::support::{AppHarness as _, fixtures, shown_notice};
use craft::host::{ManualJobs, Picked, ScriptedFiles};
use craft::ui::dialog::dialog_id;
use craft::ui::patterns::form_dialogs::{FieldValue, HostedParamDialog};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

fn input(name: &str) -> PathBuf {
    Path::new("/in").join(name)
}

fn output(name: &str) -> PathBuf {
    Path::new("/out").join(name)
}

/// An app whose jobs wait in the returned queue, whose folder picker answers `/out` and which can read `a.pdf` (two
/// pages), `b.pdf` (three) and `bad.pdf` (not a PDF) from `/in`.
fn harness(extra: impl FnOnce(ScriptedFiles) -> ScriptedFiles) -> (Harness<'static, PdfCraftApp>, Arc<ScriptedFiles>, Arc<ManualJobs>) {
    let files = ScriptedFiles::answering([Picked::Paths(vec![PathBuf::from("/out")])])
        .with_file(input("a.pdf"), fixtures::pdf_pages(2))
        .with_file(input("b.pdf"), fixtures::pdf_pages(3))
        .with_file(input("bad.pdf"), b"this is not a PDF".to_vec());
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

/// Opens the dialog and lists `names` from `/in`.
fn fill(h: &mut Harness<'static, PdfCraftApp>, names: &[&str]) {
    assert!(h.execute("optimize.batch"));
    h.run_steps(3);
    let dialog = h.state_mut().dialogs.get_mut_by_id::<HostedParamDialog>(dialog_id("optimize.batch")).expect("the dialog is open");
    let list = names.iter().map(|n| FieldValue::Text(input(n).to_string_lossy().into_owned())).collect();
    dialog.set_field("files", FieldValue::List(list));
    h.run_steps(3);
}

fn run_jobs(h: &mut Harness<'static, PdfCraftApp>, jobs: &ManualJobs) {
    h.run_steps(3);
    jobs.run_all();
    h.run_steps(4);
}

#[test]
fn every_file_is_written_to_the_chosen_folder_no_larger_than_it_was() {
    let (mut h, files, jobs) = harness(|f| f);
    fill(&mut h, &["a.pdf", "b.pdf"]);
    h.get_by_role_and_label(Role::Button, "Optimize files").click();
    run_jobs(&mut h, &jobs);
    for name in ["a.pdf", "b.pdf"] {
        let before = files.file(&input(name)).expect("the original is still there");
        let after = files.file(&output(name)).unwrap_or_else(|| panic!("{name} was written to the chosen folder"));
        assert!(after.len() <= before.len(), "{name}: {} bytes became {}", before.len(), after.len());
    }
    assert!(shown_notice(&h).is_some_and(|n| n.contains("Optimized 2 of 2") && !n.contains("failed")), "{:?}", shown_notice(&h));
}

#[test]
fn a_file_that_is_not_a_pdf_is_named_as_failed_and_the_others_still_run() {
    let (mut h, files, jobs) = harness(|f| f);
    fill(&mut h, &["a.pdf", "bad.pdf", "missing.pdf"]);
    h.get_by_role_and_label(Role::Button, "Optimize files").click();
    run_jobs(&mut h, &jobs);
    assert!(files.file(&output("a.pdf")).is_some(), "the good file is written");
    assert!(files.file(&output("bad.pdf")).is_none() && files.file(&output("missing.pdf")).is_none(), "{:?}", files.written());
    let notice = shown_notice(&h).unwrap_or_default();
    assert!(notice.contains("Optimized 1 of 3"), "{notice}");
    assert!(notice.contains("2 failed") && notice.contains("bad.pdf") && notice.contains("missing.pdf"), "{notice}");
}

#[test]
fn nothing_is_reported_as_optimized_when_every_file_fails() {
    let (mut h, files, jobs) = harness(|f| f);
    fill(&mut h, &["bad.pdf"]);
    h.get_by_role_and_label(Role::Button, "Optimize files").click();
    run_jobs(&mut h, &jobs);
    assert!(files.written().is_empty(), "{:?}", files.written());
    let notice = shown_notice(&h).unwrap_or_default();
    assert!(notice.contains("Nothing was optimized") && notice.contains("bad.pdf"), "{notice}");
}

#[test]
fn a_name_already_in_the_folder_is_never_replaced() {
    let (mut h, files, jobs) = harness(|f| f.with_file(output("a.pdf"), b"keep this file".to_vec()));
    fill(&mut h, &["a.pdf"]);
    h.get_by_role_and_label(Role::Button, "Optimize files").click();
    run_jobs(&mut h, &jobs);
    assert_eq!(files.file(&output("a.pdf")).as_deref(), Some(&b"keep this file"[..]));
    assert!(files.file(&output("a (2).pdf")).is_some(), "{:?}", files.written());
}

#[test]
fn the_file_menu_lists_it_and_the_dialog_asks_for_files_first() {
    let (mut h, _files, _jobs) = harness(|f| f);
    crate::support::open_menu(&mut h, "File");
    crate::support::menu_row(&h, "Optimize multiple files…").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Dialog, "Optimize multiple files");
    h.get_by_label_contains("Add the files to optimize");
}
