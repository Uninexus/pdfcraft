//! The AI Assistant window in the real shell: it opens from its commands, and a proposed edit is shown, made as
//! ordinary undo steps and taken back. No model is asked and nothing leaves the test.

use crate::support::AppHarness as _;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;
use serde_json::json;

fn harness(pages: usize) -> Harness<'static, PdfCraftApp> {
    let text = vec!["Page text"; pages].join("\n\u{c}");
    let bytes = pdfcraft_engine::Session::new().create_from_text("t", &text).unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "doc.pdf", None, bytes.clone()).unwrap();
            app
        }
    });
    h.run_steps(4);
    h
}

fn pages(h: &Harness<'static, PdfCraftApp>) -> usize {
    let (_, id) = h.state().active_ids().unwrap();
    h.state().session.get(id).unwrap().info.pages.len()
}

#[test]
fn each_ai_command_opens_the_window() {
    for id in ["ai.ask", "ai.summary", "ai.translate", "ai.edit"] {
        let mut h = harness(1);
        assert!(!h.state().ai_open);
        assert!(h.execute(id), "{id}");
        h.run_steps(3);
        assert!(h.state().ai_open, "{id}");
        h.get_by_role_and_label(egui::accesskit::Role::Dialog, "AI Assistant");
    }
}

#[test]
fn a_proposed_edit_is_made_as_undo_steps_and_taken_back() {
    let mut h = harness(3);
    assert!(h.execute("ai.edit"));
    h.run_steps(2);
    let ctx = h.ctx.clone();
    let before = pages(&h);
    h.state_mut().ai_propose_for_test("tidy up", &[("insert_blank_page", json!({ "at": 1 })), ("delete_pages", json!({ "pages": [3] }))]).unwrap();
    h.run_steps(2);
    h.get_by_label_contains("Make these changes");
    assert_eq!(pages(&h), before, "a proposal changes nothing");
    assert_eq!(h.state_mut().ai_apply_for_test(&ctx), None);
    h.run_steps(2);
    assert_eq!(pages(&h), before, "one page added, one removed");
    h.get_by_label("Undo these changes");
    h.state_mut().ai_undo_for_test(&ctx);
    h.run_steps(2);
    assert_eq!(pages(&h), before);
    let (_, id) = h.state().active_ids().unwrap();
    let depth = h.state().session.get(id).unwrap().undo_depth();
    assert_eq!(depth, 0, "every step was undone");
}

#[test]
fn a_proposal_the_document_no_longer_fits_is_refused_and_changes_nothing() {
    let mut h = harness(3);
    let ctx = h.ctx.clone();
    h.state_mut().ai_propose_for_test("rotate the last", &[("rotate_pages", json!({ "pages": [3], "degrees": 90 }))]).unwrap();
    // The document shrinks before the user presses the button.
    let (_, id) = h.state().active_ids().unwrap();
    assert!(h.state_mut().apply_edit(&ctx, pdfcraft_engine::Edit::DeletePages { pages: vec![2] }));
    let depth = h.state().session.get(id).unwrap().undo_depth();
    let error = h.state_mut().ai_apply_for_test(&ctx);
    assert!(error.is_some(), "refused");
    assert_eq!(h.state().session.get(id).unwrap().undo_depth(), depth, "nothing was made");
}

#[test]
fn a_proposal_naming_a_missing_page_is_not_shown() {
    let mut h = harness(2);
    assert!(h.state_mut().ai_propose_for_test("x", &[("rotate_pages", json!({ "pages": [9], "degrees": 90 }))]).is_err());
}
