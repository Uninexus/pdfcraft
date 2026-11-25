//! The Comments panel's "…" menu shows or hides the comments of one type on the page.

use egui_kittest::{
    Harness,
    kittest::{NodeT as _, Queryable},
};
use pdfcraft_ui_egui::PdfCraftApp;

/// A page with a rectangle comment and an oval comment, built with a real cross-reference table.
fn fixture() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Annots [4 0 R 5 0 R] >>",
        "<< /Type /Annot /Subtype /Square /Rect [20 100 80 160] /C [1 0 0] /Contents (Box) /T (Ann) >>",
        "<< /Type /Annot /Subtype /Circle /Rect [120 100 180 160] /C [0 0 1] /Contents (Round) /T (Ann) >>",
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

fn harness() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1400.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::support::app_with_clipboard(crate::support::RecordingClipboard::default());
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.set_option(&egui::Context::default(), "author", "Tester").unwrap();
            app.open_bytes(&egui::Context::default(), "types.pdf", None, fixture()).expect("opens");
            app.set_option(&egui::Context::default(), "panel", "comments").unwrap();
            app
        }
    });
    h.run_steps(4);
    crate::support::settle(&mut h);
    h
}

/// Open the "…" menu and click the row labelled `label` that is a check (the page's cards may carry
/// the same word).
fn pick_type(h: &mut Harness<'static, PdfCraftApp>, label: &str) {
    h.get_by_label("More options").click();
    h.run_steps(2);
    let rows: Vec<_> = h.query_all_by_label(label).collect();
    let row = rows.iter().find(|row| row.accesskit_node().toggled().is_some()).or(rows.last()).expect("a row for the type");
    row.click();
    h.run_steps(3);
}

fn hidden(h: &Harness<'static, PdfCraftApp>) -> Vec<String> {
    let app = h.state();
    app.session.get(app.views[0].id).unwrap().hidden_comment_types().iter().cloned().collect()
}

#[test]
fn the_panel_menu_hides_and_shows_one_comment_type() {
    let mut h = harness();
    assert!(hidden(&h).is_empty());

    pick_type(&mut h, "Oval");
    assert_eq!(hidden(&h), ["Circle"], "ovals are hidden, rectangles are not");
    let app = h.state();
    let id = app.views[0].id;
    assert!(!app.session.get(id).unwrap().comments_hidden(), "Hide all comments is a setting of its own");
    let config = app.session.render_config(id).unwrap();
    assert_eq!(config.annotations.iter().filter(|(_, _, shown)| !*shown).count(), 1, "the page stops drawing the oval and only it");

    pick_type(&mut h, "Rectangle");
    assert_eq!(hidden(&h), ["Circle", "Square"]);

    h.get_by_label("More options").click();
    h.run_steps(2);
    h.get_by_label("Show all comment types").click();
    h.run_steps(3);
    assert!(hidden(&h).is_empty(), "Show all brings every type back");
}
