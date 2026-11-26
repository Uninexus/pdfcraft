//! Preferences ▸ Identity ▸ Login name: the author of new comments when the box is ticked.

use crate::support::AppHarness as _;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

const PDF: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj
4 0 obj << /Length 55 >> stream
BT /F1 14 Tf 20 150 Td (The quick brown fox) Tj ET
endstream endobj
5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj
trailer << /Root 1 0 R >>
%%EOF";

fn type_into(h: &mut Harness<'static, PdfCraftApp>, label: &str, text: &str) {
    h.get_by_role_and_label(Role::TextInput, label).scroll_to_me();
    h.run_steps(4);
    h.get_by_role_and_label(Role::TextInput, label).click();
    h.run_steps(1);
    h.get_by_role_and_label(Role::TextInput, label).type_text(text);
    h.run_steps(2);
}

fn authors(h: &Harness<'static, PdfCraftApp>) -> Vec<Option<String>> {
    let doc = h.state().session.get(h.state().views[0].id).unwrap();
    doc.info.annotations.iter().map(|a| a.author.clone()).collect()
}

#[test]
fn the_login_name_authors_new_comments_when_ticked_and_is_remembered() {
    const BOX: &str = "Use the login name as the author of new comments";
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1400.0)).build_eframe(|cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "doc.pdf", None, PDF.to_vec()).expect("opens");
            app.set_option(&egui::Context::default(), "author", "Tester").unwrap();
            app.set_option(&egui::Context::default(), "dialog", "preferences").unwrap();
            app
        }
    });
    h.run_steps(4);
    type_into(&mut h, "Login name", "ghopper");
    h.get_by(|n| !n.is_hidden() && n.role() != Role::Label && n.label().as_deref() == Some(BOX)).scroll_to_me();
    h.run_steps(4);
    h.get_by(|n| !n.is_hidden() && n.role() != Role::Label && n.label().as_deref() == Some(BOX)).click();
    h.run_steps(3);
    assert_eq!(h.state().session.identity().login_name, "ghopper");
    assert!(h.state().session.identity().author_from_login_name, "a row applies as it changes");
    assert_eq!(h.state().comment_prefs.author, "Tester", "the name is left as it was");

    h.set_option("dialog", "none").unwrap();
    crate::support::wait_until(&mut h, "the page's text layer", |app| app.views[0].glyph_screen_pos(0, 8).is_some() && !app.render_pending());
    h.state_mut().views[0].select_text(0, 4, 8);
    assert!(h.execute("comment.highlight"));
    crate::support::wait_until(&mut h, "the first highlight", |app| app.session.get(app.views[0].id).is_some_and(|d| !d.info.annotations.is_empty()));
    assert_eq!(authors(&h), vec![Some("ghopper".to_string())]);

    // Remembered between runs.
    let mut restored = crate::control::new_app();
    restored.apply_settings(&h.state().settings_snapshot());
    assert_eq!(restored.session.identity().login_name, "ghopper");
    assert!(restored.session.identity().author_from_login_name);

    // Unticked, a new comment uses the name again.
    h.set_option("dialog", "preferences").unwrap();
    h.run_steps(4);
    h.get_by(|n| !n.is_hidden() && n.role() != Role::Label && n.label().as_deref() == Some(BOX)).scroll_to_me();
    h.run_steps(4);
    h.get_by(|n| !n.is_hidden() && n.role() != Role::Label && n.label().as_deref() == Some(BOX)).click();
    h.run_steps(3);
    assert!(!h.state().session.identity().author_from_login_name);
    h.set_option("dialog", "none").unwrap();
    h.run_steps(2);
    h.state_mut().views[0].select_text(0, 10, 15);
    assert!(h.execute("comment.highlight"));
    crate::support::wait_until(&mut h, "the second highlight", |app| app.session.get(app.views[0].id).is_some_and(|d| d.info.annotations.len() == 2));
    assert_eq!(authors(&h).last().cloned().flatten().as_deref(), Some("Tester"));
}
