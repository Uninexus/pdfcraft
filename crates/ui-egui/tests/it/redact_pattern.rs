//! Find text and redact by pattern (egui_kittest): a regular expression with a check-digit test,
//! and a refused expression that says why and marks nothing.

use crate::support::AppHarness as _;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::{PdfCraftApp, RedactSearchMode};

/// A page with two card-shaped numbers: the first passes the Luhn check, the second does not.
fn harness() -> Harness<'static, PdfCraftApp> {
    let memo = pdfcraft_engine::Session::new()
        .create_from_text("memo", "Paid with 4111 1111 1111 1111 today\nRetry with 4111 1111 1111 1112 later")
        .unwrap()
        .to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "memo.pdf", None, memo.clone()).unwrap();
            app
        }
    });
    h.run_steps(4);
    h
}

fn marks(h: &Harness<'static, PdfCraftApp>) -> usize {
    let s = h.state();
    s.session.get(s.views[0].id).unwrap().redaction_marks()
}

fn last_step(h: &Harness<'static, PdfCraftApp>) -> Option<String> {
    let s = h.state();
    s.session.get(s.views[0].id).unwrap().can_undo().map(str::to_owned)
}

#[test]
fn a_pattern_with_the_card_check_marks_only_the_valid_number_in_one_step() {
    let mut h = harness();
    assert!(h.execute("redact.search"));
    h.run_steps(2);
    h.get_by_label("Pattern (regular expression)");
    assert!(h.get_all_by_label("Check digits").count() > 0, "the check-digit choice is offered");
    {
        let d = &mut h.state_mut().redact_search;
        d.mode = RedactSearchMode::Regex;
        d.regex = r"\d{4} \d{4} \d{4} \d{4}".into();
        d.validator = Some(pdfcraft_engine::RedactValidator::Luhn);
    }
    h.run_steps(2);
    let before = last_step(&h);
    h.get_by_label("Mark all").click();
    h.run_steps(3);
    assert_eq!((marks(&h), h.state().redact_search.found), (1, Some(1)), "only the number that passes the check is marked");
    assert!(h.state().redact_search.error.is_none());
    assert_ne!(last_step(&h), before, "marking is a step of its own");
    assert!(h.execute("edit.undo"));
    h.run_steps(3);
    assert_eq!(marks(&h), 0, "one undo removes the mark");
    assert_eq!(last_step(&h), before);
}

#[test]
fn a_refused_expression_says_why_and_marks_nothing() {
    let mut h = harness();
    assert!(h.execute("redact.search"));
    h.run_steps(2);
    let before = last_step(&h);
    for expression in ["(", "a*", ""] {
        {
            let d = &mut h.state_mut().redact_search;
            d.mode = RedactSearchMode::Regex;
            d.regex = expression.into();
            d.validator = None;
        }
        h.run_steps(2);
        h.get_by_label("Mark all").click();
        h.run_steps(3);
        let reason = h.state().redact_search.error.clone();
        assert!(reason.as_deref().is_some_and(|r| !r.is_empty()), "{expression:?} is refused with a reason");
        h.get_by_label_contains("nothing was marked");
        assert_eq!((marks(&h), last_step(&h)), (0, before.clone()), "{expression:?} marks nothing");
    }
}
