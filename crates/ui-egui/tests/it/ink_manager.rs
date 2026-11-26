//! Print production ▸ Ink Manager: the window lists the spot inks the colour spaces name, and giving one ink another
//! ink's name merges them as one undoable edit, as the tool `doc_ink_aliases` does.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

use crate::support::AppHarness as _;

/// A one-page file whose page paints with two Separation spaces, `PANTONE 185` and `Red`.
fn inks_pdf() -> Vec<u8> {
    let content = "/CS1 cs 1 scn 0 0 50 50 re f /CS2 cs 1 scn 60 0 50 50 re f";
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /ColorSpace << /CS1 5 0 R /CS2 6 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "[/Separation /PANTONE#20185 /DeviceCMYK 7 0 R]".to_string(),
        "[/Separation /Red /DeviceCMYK 7 0 R]".to_string(),
        "<< /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 1 1 0] /N 1 >>".to_string(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).as_bytes());
    out
}

fn harness(bytes: Vec<u8>) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1400.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "inks.pdf", None, bytes.clone()).expect("opens");
            app
        }
    });
    h.run_steps(4);
    h
}

/// The document's spot inks as (name, colour spaces naming it), read from the engine.
fn inks(h: &Harness<'static, PdfCraftApp>) -> Vec<(String, usize)> {
    let id = h.state().views[0].id;
    h.state().session.list_inks(id).unwrap().inks.into_iter().map(|i| (i.name, i.spaces)).collect()
}

#[test]
fn an_ink_given_another_inks_name_merges_with_it_as_one_edit() {
    let mut h = harness(inks_pdf());
    let before = inks(&h);
    assert_eq!(before.len(), 2, "the file has two spot inks");
    assert!(h.execute("print.ink_manager"));
    h.run_steps(4);
    h.get_by_role_and_label(Role::Group, "PANTONE 185");
    h.get_by_role_and_label(Role::Group, "Red");
    assert_eq!(h.query_all_by_label_contains("Alias ").count(), before.len(), "each ink can be given an alias");
    let id = h.state().views[0].id;
    assert!(!h.state().session.get(id).unwrap().dirty, "listing changes nothing");

    h.get_by_label("Alias PANTONE 185").click();
    h.run_steps(3);
    let name = h.get_by_role_and_label(Role::TextInput, "New ink name");
    name.focus();
    name.type_text("Red");
    h.run_steps(2);
    h.get_by_label("Apply").click();
    h.run_steps(4);

    let after = inks(&h);
    assert!(after.len() < before.len(), "the two inks are one now");
    assert!(h.query_by_role_and_label(Role::Group, "PANTONE 185").is_none(), "the renamed ink is gone from the list");
    h.get_by_role_and_label(Role::Group, "Red");
    assert_eq!(h.query_all_by_label_contains("Alias ").count(), after.len(), "the list shows what the document holds");
    let doc = h.state().session.get(id).unwrap();
    assert!(doc.dirty, "the document is marked changed");
    assert!(doc.can_undo().is_some(), "the rename is an undo step");

    h.undo();
    h.run_steps(4);
    assert_eq!(inks(&h).len(), before.len(), "undo brings the ink back");
    h.get_by_role_and_label(Role::Group, "PANTONE 185");
}
