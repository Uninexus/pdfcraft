//! Print production ▸ Print readiness: the report opens from the command, lists what the ink
//! check found, and changes nothing.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use pdfcraft_ui_egui::{Dialog, PdfCraftApp};

fn with_ink() -> Vec<u8> {
    let mut doc = pdfcraft_cos::Document::open(std::sync::Arc::new(include_bytes!("../data/form.pdf").to_vec())).unwrap();
    let page = pdfcraft_model::pages(&doc).into_iter().next().unwrap();
    // 330% total ink.
    let ink = doc.add(pdfcraft_cos::Object::Stream(pdfcraft_cos::Stream::flate(pdfcraft_cos::Dict::new(), b"0.9 0.8 0.7 0.9 k 10 10 100 100 re f")));
    doc.update_dict(page.obj, |d| d.set(b"Contents".to_vec(), pdfcraft_cos::Object::Ref(ink))).unwrap();
    pdfcraft_cos::write_full(&doc, &Default::default()).unwrap()
}

fn harness(bytes: Vec<u8>) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("ink.pdf", None, bytes.clone()).unwrap();
        app
    });
    h.run_steps(4);
    h
}

#[test]
fn the_report_lists_the_ink_findings_and_changes_nothing() {
    let mut h = harness(with_ink());
    let id = h.state().views[0].id;
    let generation = h.state().session.get(id).unwrap().edit_generation();
    assert!(h.state_mut().execute("prepress.readiness"));
    h.run_steps(3);
    assert_eq!(h.state().dialog, Some(Dialog::PrintReadiness));
    h.get_by_label("Print readiness");
    assert!(h.state().readiness_report.as_ref().is_some_and(|r| r.findings.iter().any(|f| f.id == "ink-total")));
    // The finding is on screen, in words, with its severity.
    h.get_by_label_contains("total ink");
    h.get_by_label("Warning");
    h.get_by_label("Close").click();
    h.run_steps(2);
    assert_eq!(h.state().dialog, None);
    assert_eq!(h.state().session.get(id).unwrap().edit_generation(), generation, "nothing was edited");
    assert_eq!(h.state().session.get(id).unwrap().can_undo(), None);
}

#[test]
fn a_document_with_nothing_to_report_says_so() {
    let mut h = harness(include_bytes!("../data/form.pdf").to_vec());
    assert!(h.state_mut().execute("prepress.readiness"));
    h.run_steps(3);
    assert_eq!(h.state().dialog, Some(Dialog::PrintReadiness));
    assert!(h.state().readiness_report.as_ref().is_some_and(|r| !r.findings.iter().any(|f| f.id.starts_with("ink-"))));
}
