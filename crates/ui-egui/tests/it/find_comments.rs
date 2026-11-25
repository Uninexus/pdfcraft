//! The find bar's "Include comments" option: a word only a comment holds is found when the option is on, counted with
//! the page's hits, and moving to it selects the comment; a comment of a hidden type is not found.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

use crate::support;

/// One 300×200 page whose text has no "zebra", with a note whose text has.
const FIXTURE: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> /Annots [6 0 R] >> endobj
4 0 obj << /Length 55 >> stream
BT /F1 14 Tf 20 150 Td (The quick brown fox) Tj ET
endstream endobj
5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj
6 0 obj << /Type /Annot /Subtype /Text /Rect [20 100 40 120] /NM (note-1) /T (Alice) /Contents (A zebra crossed here) >> endobj
trailer << /Root 1 0 R >>
%%EOF";

fn harness() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1400.0)).build_eframe(|cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "zebra.pdf", None, FIXTURE.to_vec()).expect("opens");
            app.set_option(&egui::Context::default(), "left", "closed").unwrap();
            // Opens the find bar with the query, as ⌘F and typing do.
            app.set_option(&egui::Context::default(), "find", "zebra").unwrap();
            app
        }
    });
    h.run_steps(4);
    support::settle(&mut h);
    h.run_steps(3);
    h
}

fn found_nothing(h: &Harness<'static, PdfCraftApp>) -> bool {
    h.query_all_by_label("No matches").count() == 1
}

fn matches_found(h: &Harness<'static, PdfCraftApp>) -> usize {
    h.state().views[0].find.as_ref().map_or(0, |f| f.matches.len())
}

#[test]
fn the_find_bar_finds_a_comment_only_when_asked_to() {
    let mut h = harness();
    assert!(found_nothing(&h), "the page text has no such word");
    assert_eq!(matches_found(&h), 0);

    h.get_by_label("Include comments").click();
    h.run_steps(3);
    assert!(matches_found(&h) > 0, "the comment's text is found");
    assert!(!found_nothing(&h));

    // Next moves to the comment and selects it.
    h.state_mut().views[0].find_step(true);
    h.run_steps(3);
    assert!(!h.state().views[0].session.comments.selection().is_empty(), "the comment is selected");

    // Off again: back to page text only.
    h.get_by_label("Include comments").click();
    h.run_steps(3);
    assert_eq!(matches_found(&h), 0);
}

#[test]
fn a_comment_of_a_hidden_type_is_not_found() {
    let mut h = harness();
    h.get_by_label("Include comments").click();
    h.run_steps(3);
    assert!(matches_found(&h) > 0);

    let id = h.state().views[0].id;
    let hidden = ["Text".to_owned()].into_iter().collect();
    assert!(h.state_mut().session.set_hidden_comment_types(id, hidden), "the type is now hidden");
    h.run_steps(3);
    assert_eq!(matches_found(&h), 0, "a hidden comment is not matched");
}
