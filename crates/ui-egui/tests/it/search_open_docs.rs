//! The Search panel's All open documents scope in the real shell (egui_kittest): the query is searched in every open
//! tab, the matches are listed by document, and a click on one switches to that tab and goes to its page.

use crate::support::{AppHarness as _, step_until};
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

/// `first.pdf` (active) says "apple" once; `second.pdf` says "apple" on its second page and "banana" nowhere else.
fn harness() -> Harness<'static, PdfCraftApp> {
    let app = crate::support::test_app(crate::support::host());
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1400.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        let mut app = app;
        let ctx = egui::Context::default();
        app.set_option(&ctx, "language", "en").unwrap();
        app.open_bytes(&ctx, "first.pdf", None, pdf(&["red apple"])).unwrap();
        app.open_bytes(&ctx, "second.pdf", None, pdf(&["plain page", "green apple banana"])).unwrap();
        app
    });
    h.state_mut().active = Some(0);
    h.run_steps(4);
    h
}

#[test]
fn matches_from_every_open_document_are_listed_and_a_click_switches_tab() {
    let mut h = harness();
    assert!(h.execute("edit.advanced_search"));
    h.run_steps(3);
    h.state_mut().views[0].find.as_mut().unwrap().query = "apple".into();
    // The current document alone has one match.
    assert!(step_until(&mut h, |h| h.state().views[0].find.as_ref().is_some_and(|f| f.matches.len() == 1)), "the current document is searched");
    let in_current = h.state().views[0].find.as_ref().unwrap().matches.len();
    h.get_by_role_and_label(Role::Button, "Search all open documents").click();
    h.run_steps(3);
    assert!(h.state().open_docs_search.on);
    assert!(
        step_until(&mut h, |h| h.state().open_docs_search.done() && h.state().open_docs_search.found() > in_current),
        "both documents are searched"
    );
    let state = &h.state().open_docs_search;
    assert!(state.docs.iter().any(|d| d.name == "second.pdf" && d.found() == 1), "a result names the other document");
    h.run_steps(2);
    h.get_by_label("second.pdf (1)");
    // Clicking the result in the other document makes its tab current, at the match's page.
    h.get_by_label_contains("green apple").click();
    h.run_steps(4);
    let s = h.state();
    let i = s.active.unwrap();
    assert_ne!(i, 0, "the other tab is current");
    assert_eq!(s.views[i].current(), 1, "at the page of the match");
}

#[test]
fn a_word_only_the_second_document_has_is_found_only_there() {
    let mut h = harness();
    assert!(h.execute("edit.advanced_search"));
    h.run_steps(3);
    h.state_mut().views[0].find.as_mut().unwrap().query = "banana".into();
    h.get_by_role_and_label(Role::Button, "Search all open documents").click();
    h.run_steps(3);
    assert!(step_until(&mut h, |h| h.state().open_docs_search.done() && h.state().open_docs_search.found() > 0), "the word is found");
    let state = &h.state().open_docs_search;
    assert!(state.docs.iter().all(|d| (d.name == "second.pdf") == (d.found() > 0)), "only the second document lists it");
}
