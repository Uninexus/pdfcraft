//! Sign > Signature audit report and Sign > Signer labels report: two read-only reports of the open document in a window,
//! with Copy report and, for the audit, Save as PDF. Neither changes the document.

use std::sync::Arc;

use crate::support::AppHarness as _;
use craft::host::{Picked, ScriptedFiles};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable};
use pdfcraft_engine::signing_plan::{Plan, Role as PlanRole, plan_bytes};
use pdfcraft_engine::{Edit, NewField, Session};
use pdfcraft_ui_egui::{PdfCraftApp, SignatureReportKind};

/// A blank page with signature fields `Sig1` and `Sig2` and a text field `Name`.
fn fixture() -> Vec<u8> {
    let mut session = Session::new();
    let bytes = session.create_blank(300.0, 400.0, 1).unwrap();
    let id = session.open("doc.pdf", None, bytes, None).unwrap();
    for (name, rect, kind) in [
        ("Sig1", [40.0, 290.0, 220.0, 326.0], NewField::Signature),
        ("Sig2", [40.0, 250.0, 220.0, 286.0], NewField::Signature),
        ("Name", [40.0, 200.0, 230.0, 224.0], NewField::Text { multiline: false }),
    ] {
        session.apply(id, Edit::AddField { name: Some(name.into()), page: 0, rect, kind }).unwrap();
    }
    session.save_bytes(id).unwrap().as_ref().clone()
}

fn harness(files: Arc<ScriptedFiles>, path: Option<String>, open: bool) -> Harness<'static, PdfCraftApp> {
    let bytes = fixture();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1000.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::app_with_files(files);
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            if open {
                app.open_bytes(&egui::Context::default(), "doc.pdf", path, bytes).unwrap();
            }
            app
        }
    });
    crate::support::settle(&mut h);
    h
}

fn label(h: &mut Harness<'static, PdfCraftApp>, field: &str, email: &str) {
    let id = h.state().views[0].id;
    let obj = h.state().session.get(id).unwrap().form.iter().find(|f| f.name == field).unwrap().obj;
    h.state_mut().session.apply(id, Edit::SetFieldSigner { field: obj, email: Some(email.into()) }).unwrap();
}

#[test]
fn the_audit_opens_from_the_command_says_what_was_not_checked_and_changes_nothing() {
    let mut h = harness(Arc::new(ScriptedFiles::new()), None, true);
    let id = h.state().views[0].id;
    let generation = h.state().session.get(id).unwrap().edit_generation();
    assert!(h.execute("sign.audit_report"));
    h.run_steps(4);
    let report = h.state().signature_report.clone().expect("the report is open");
    assert_eq!(report.kind, SignatureReportKind::Audit);
    assert!(report.text.contains("Signature audit report") && report.text.contains("No online revocation check"));
    assert!(h.query_all_by_label_contains("No online revocation check").count() > 0, "what was not checked is on screen");
    let save = h.query_all_by_role_and_label(Role::Button, "Save as PDF…").last().expect("a Save as PDF button");
    assert!(save.accesskit_node().is_disabled(), "an audit of a document with no signature has nothing to save");
    h.query_all_by_role_and_label(Role::Button, "Close").last().expect("the window's Close").click();
    h.run_steps(3);
    assert!(h.state().signature_report.is_none());
    assert_eq!(h.state().session.get(id).unwrap().edit_generation(), generation, "nothing was edited");
}

#[test]
fn save_as_pdf_writes_the_report_as_a_pdf() {
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec!["/reports".into()])]));
    let mut h = harness(files.clone(), None, true);
    assert!(h.execute("sign.audit_report"));
    h.run_steps(4);
    // The fixture holds no signature: pretend the audit found one, so the save is offered.
    if let Some(a) = h.state_mut().signature_report.as_mut().and_then(|r| r.audit.as_mut()) {
        a.signed = 1;
    }
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Save as PDF…").click();
    h.run_steps(4);
    let written = files.written();
    assert_eq!(written.len(), 1, "one report file was written");
    assert_eq!(written[0].parent(), Some(std::path::Path::new("/reports")), "in the folder that was picked");
    let name = written[0].file_name().and_then(|n| n.to_str()).unwrap_or_default();
    assert!(name.starts_with("doc") && name.ends_with(".pdf"), "named after the document: {name}");
    let bytes = files.file(&written[0]).expect("the report was written");
    assert!(bytes.starts_with(b"%PDF"), "as a PDF");
    assert!(h.state().signature_report.is_some(), "the window stays open");
}

#[test]
fn the_signer_labels_report_lists_labels_gaps_and_names_the_plan_does_not_have() {
    let dir = crate::support::TempDir::new();
    let pdf = dir.path().join("doc.pdf");
    std::fs::write(&pdf, fixture()).unwrap();
    let mut plan = Plan::empty(1);
    plan.add_recipient("Ann", "ann@example.org", PlanRole::Signer, 1).unwrap();
    plan.add_recipient("Bob", "bob@example.org", PlanRole::Signer, 1).unwrap();
    std::fs::write(dir.path().join("doc.pdf.pdfcraft-sign.json"), plan_bytes(&plan).unwrap()).unwrap();
    let mut h = harness(Arc::new(ScriptedFiles::new()), Some(pdf.to_string_lossy().into_owned()), true);
    label(&mut h, "Sig1", "zed@example.org");
    assert!(h.execute("form.signer_report"));
    h.run_steps(4);
    let report = h.state().signature_report.clone().expect("the report is open");
    assert_eq!(report.kind, SignatureReportKind::Signers);
    let text = report.text;
    assert!(text.contains("Sig1: zed@example.org"), "{text}");
    let at = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("{needle:?} in {text}"));
    assert!(at("Signature fields with no signer") < text.rfind("Sig2").unwrap(), "Sig2 has no signer");
    assert!(
        at("Labels that name someone who is not a signer or approver in the plan") < text.rfind("zed@example.org").unwrap(),
        "zed is not in the plan"
    );
    assert!(at("Plan recipients with no field") < text.rfind("Ann (ann@example.org)").unwrap(), "Ann has no field");
    assert!(text.contains("Bob (bob@example.org)"));
}

#[test]
fn the_reports_need_a_document() {
    let mut h = harness(Arc::new(ScriptedFiles::new()), None, false);
    assert!(!h.execute("sign.audit_report"));
    assert!(!h.execute("form.signer_report"));
    assert!(h.state().signature_report.is_none());
}
