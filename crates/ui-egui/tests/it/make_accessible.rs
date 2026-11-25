//! Prepare for accessibility ▸ Make Accessible in the real shell (egui_kittest): the dialog asks
//! for the document language, fixes the failing rules in one undo step, refreshes the report and
//! says what was fixed and what still needs attention.

use crate::support::AppHarness as _;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::a11y::{Rule, Status};
use pdfcraft_ui_egui::{PdfCraftApp, RightPanel};

/// No title, no language and no tags.
const UNTITLED: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 100] >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj
4 0 obj << /Length 37 >> stream
BT /F1 12 Tf 20 50 Td (Hello) Tj ET
endstream endobj
5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj
trailer << /Root 1 0 R >>
%%EOF";

/// Titled (shown in the title bar) and in a language: only the review rules fail.
const READY: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R /Lang (en-US) /ViewerPreferences << /DisplayDocTitle true >> >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 100] >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj
4 0 obj << /Length 37 >> stream
BT /F1 12 Tf 20 50 Td (Hello) Tj ET
endstream endobj
5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj
6 0 obj << /Title (Notes) >> endobj
trailer << /Root 1 0 R /Info 6 0 R >>
%%EOF";

fn harness(name: &'static str, bytes: &'static [u8]) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), name, None, bytes.to_vec()).unwrap();
            app
        }
    });
    h.run_steps(4);
    h
}

fn undo_label(h: &Harness<'static, PdfCraftApp>) -> Option<String> {
    let s = h.state();
    s.session.get(s.views[0].id).and_then(|d| d.can_undo().map(str::to_owned))
}

fn status(h: &Harness<'static, PdfCraftApp>, rule: Rule) -> Status {
    h.state().a11y.report.as_ref().unwrap().1.result(rule).unwrap().status
}

#[test]
fn make_accessible_fixes_title_and_language_in_one_undo_step() {
    let mut h = harness("notes.pdf", UNTITLED);
    assert!(h.execute("a11y.make_accessible"));
    h.run_steps(2);
    h.get_by_role_and_label(Role::Dialog, "Make Accessible");
    let field = h.get_by_role_and_label(Role::TextInput, "Document language");
    field.focus();
    h.run_steps(1);
    let field = h.get_by_role_and_label(Role::TextInput, "Document language");
    field.type_text("en-US");
    h.run_steps(2);
    h.get_by_role_and_label(Role::Button, "Make accessible").click();
    h.run_steps(4);
    assert!(h.state().dialogs.is_empty(), "OK closes the dialog");
    assert_eq!(undo_label(&h).as_deref(), Some("Make accessible"));
    assert_eq!(crate::support::right(h.state()), Some(RightPanel::Accessibility));
    assert_eq!(status(&h, Rule::Title), Status::Passed);
    assert_eq!(status(&h, Rule::PrimaryLanguage), Status::Passed);
    assert_eq!(status(&h, Rule::TaggedPdf), Status::Failed, "tags are left for review");
    let notice = crate::support::shown_notice(&h).expect("the result is said");
    assert!(notice.starts_with("Fixed 2 issues; ") && notice.ends_with("still need attention"), "{notice}");
    // One step undoes both fixes.
    assert!(h.execute("edit.undo"));
    h.run_steps(2);
    assert_eq!(undo_label(&h), None, "nothing else was recorded");
    h.state_mut().run_accessibility_check();
    h.run_steps(2);
    assert_eq!(status(&h, Rule::Title), Status::Failed);
    assert_eq!(status(&h, Rule::PrimaryLanguage), Status::Failed);
}

#[test]
fn make_accessible_with_nothing_to_fix_adds_no_undo_step() {
    let mut h = harness("ready.pdf", READY);
    assert!(h.execute("a11y.make_accessible"));
    h.run_steps(2);
    let language = h.get_by_role_and_label(Role::TextInput, "Document language").value();
    assert_eq!(language.as_deref(), Some("en-US"), "the document's own language is offered");
    h.get_by_role_and_label(Role::Button, "Make accessible").click();
    h.run_steps(4);
    assert!(h.state().dialogs.is_empty());
    assert_eq!(undo_label(&h), None, "no undo step");
    assert_eq!(crate::support::right(h.state()), Some(RightPanel::Accessibility));
    let notice = crate::support::shown_notice(&h).expect("the result is said");
    assert!(notice.starts_with("Nothing needed fixing"), "{notice}");
}
