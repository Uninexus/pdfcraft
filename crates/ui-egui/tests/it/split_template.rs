//! The Split dialog's file name template in the real shell (egui_kittest): the placeholders' hint, the validation, the
//! preview of the first names, and the names the parts are written under.

use std::path::PathBuf;
use std::sync::Arc;

use crate::support::{AppHarness as _, fixtures};
use craft::host::{Picked, ScriptedFiles};
use craft::ui::dialog::dialog_id;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

/// `report.pdf` of three pages open in an app whose folder picker answers `out`.
fn harness(out: &str) -> (Harness<'static, PdfCraftApp>, Arc<ScriptedFiles>) {
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![PathBuf::from(out)])]));
    let app = crate::support::test_app(crate::support::host_with_files(files.clone()));
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        let mut app = app;
        app.set_option(&egui::Context::default(), "language", "en").unwrap();
        app.open_bytes(&egui::Context::default(), "report.pdf", None, fixtures::pdf_pages(3)).unwrap();
        app
    });
    h.run_steps(4);
    (h, files)
}

/// Types `text` into the open Split dialog's template row.
fn type_template(h: &mut Harness<'static, PdfCraftApp>, text: &str) {
    let dialog = h.state_mut().dialogs.get_mut_by_id::<HostedParamDialog>(dialog_id("page.split")).expect("the Split dialog is open");
    dialog.set_field("template", text.into());
    h.run_steps(3);
}

#[test]
fn the_preview_shows_the_first_names_the_template_gives() {
    let (mut h, _files) = harness("/out");
    assert!(h.execute("page.split"));
    h.run_steps(3);
    h.get_by_label_contains("Placeholders:");
    // Without a template the usual names show.
    h.get_by_label_contains("First files: report (page 1).pdf");
    type_template(&mut h, "{name}-{n}-p{first}to{last}");
    h.get_by_label_contains("First files: report-1-p1to1.pdf, report-2-p2to2.pdf");
}

#[test]
fn a_template_that_cannot_be_used_says_why_and_blocks_the_split() {
    let (mut h, files) = harness("/out");
    assert!(h.execute("page.split"));
    h.run_steps(3);
    type_template(&mut h, "{oops}");
    h.get_by_label_contains("is not a placeholder");
    h.get_all_by_role_and_label(Role::Button, "Split").last().unwrap().click();
    h.run_steps(3);
    assert!(files.written().is_empty(), "nothing was written");
}

#[test]
fn the_parts_are_written_under_the_template_names() {
    let (mut h, files) = harness("/out");
    assert!(h.execute("page.split"));
    h.run_steps(3);
    type_template(&mut h, "{name} part {n}");
    h.get_all_by_role_and_label(Role::Button, "Split").last().unwrap().click();
    h.run_steps(5);
    let names: Vec<String> = files.written().iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
    let mut names = names;
    names.sort();
    assert_eq!(names, ["report part 1.pdf", "report part 2.pdf", "report part 3.pdf"]);
}
