//! Signing plan > Check the plan: the plan set beside the signatures the saved file holds, shown in the window. It only reads.

use std::path::PathBuf;
use std::sync::Arc;

use craft::host::ScriptedFiles;
use egui_kittest::Harness;
use pdfcraft_engine::Session;
use pdfcraft_engine::signing_plan::{Plan, Role, Status, plan_bytes, sidecar_of};
use pdfcraft_ui_egui::PdfCraftApp;

use crate::support::AppHarness as _;

fn fixture() -> Vec<u8> {
    Session::new().create_blank(300.0, 400.0, 1).unwrap().as_ref().clone()
}

/// A saved `contract.pdf` with a plan of Ann and Bob (Ann recorded as signed when `ann_signed`), in a folder of its own,
/// open in an app over scripted files.
fn open(with_plan: bool, ann_signed: bool) -> (crate::support::TempDir, PathBuf, Harness<'static, PdfCraftApp>) {
    let dir = crate::support::TempDir::new();
    let pdf = dir.path().join("contract.pdf");
    std::fs::write(&pdf, fixture()).unwrap();
    if with_plan {
        let mut plan = Plan::empty(0);
        plan.add_recipient("Ann Author", "ann@example.org", Role::Signer, 0).unwrap();
        plan.add_recipient("Bob Boss", "bob@example.org", Role::Signer, 0).unwrap();
        if ann_signed {
            plan.record(0, Status::Signed, 1).unwrap();
        }
        std::fs::write(sidecar_of(&pdf).unwrap(), plan_bytes(&plan).unwrap()).unwrap();
    }
    let files = Arc::new(ScriptedFiles::new().with_file(&pdf, fixture()));
    let mut app = crate::support::test_app(crate::support::host_with_files(files));
    app.set_option(&egui::Context::default(), "language", "en").unwrap();
    app.open_bytes(&egui::Context::default(), "contract.pdf", Some(pdf.to_string_lossy().into_owned()), fixture()).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1000.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        app
    });
    crate::support::settle(&mut h);
    (dir, pdf, h)
}

fn check(h: &mut Harness<'static, PdfCraftApp>) -> (Option<String>, String, bool) {
    assert!(h.execute("sign.plan.audit"), "the check is registered and enabled with a document");
    h.run_steps(4);
    let d = h.state().signing_plan.as_ref().expect("the check opens the Signing plan window");
    (d.check.clone(), d.result.clone(), d.result_ok)
}

#[test]
fn a_plan_nobody_has_signed_agrees_with_a_file_that_holds_no_signature() {
    let (_dir, _pdf, mut h) = open(true, false);
    let (text, said, ok) = check(&mut h);
    assert!(ok, "{said}");
    let text = text.expect("the check text is kept for the window");
    assert!(text.contains("Ann Author"), "every recipient is listed");
    assert!(!text.contains("Problems"), "nothing to report: {text}");
}

#[test]
fn a_plan_that_says_signed_with_no_signature_in_the_file_is_called_a_problem_and_nothing_is_changed() {
    let (_dir, pdf, mut h) = open(true, true);
    let before = (std::fs::read(&pdf).unwrap(), std::fs::read(sidecar_of(&pdf).unwrap()).unwrap());
    let (text, said, ok) = check(&mut h);
    assert!(ok, "the check itself worked: {said}");
    let text = text.unwrap();
    assert!(text.contains("Problems") && text.contains("Ann Author"), "the disagreement is named: {text}");
    assert_eq!(before, (std::fs::read(&pdf).unwrap(), std::fs::read(sidecar_of(&pdf).unwrap()).unwrap()), "the check only reads");
}

#[test]
fn a_document_with_no_plan_gets_no_check_and_the_reason() {
    let (_dir, pdf, mut h) = open(false, false);
    let (text, said, ok) = check(&mut h);
    assert!(text.is_none() && !ok, "no check without a plan");
    assert!(said.contains("no signing plan"), "{said}");
    assert!(!sidecar_of(&pdf).unwrap().exists(), "no plan file was made by looking");
}
