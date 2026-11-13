//! Pages ▸ Remove white margins in the real shell (egui_kittest): the command, its dialog and
//! the one undoable step it makes.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::{Dialog, PdfCraftApp};

fn fixture() -> Vec<u8> {
    let streams = ["0 g 100 100 50 50 re f", "", "0 g 0 0 300 400 re f"];
    let mut pdf = String::from("%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n");
    pdf.push_str("2 0 obj << /Type /Pages /Kids [3 0 R 5 0 R 7 0 R] /Count 3 /MediaBox [0 0 300 400] >> endobj\n");
    for (n, content) in streams.iter().enumerate() {
        let (page, stream) = (3 + 2 * n, 4 + 2 * n);
        pdf.push_str(&format!("{page} 0 obj << /Type /Page /Parent 2 0 R /Contents {stream} 0 R >> endobj\n"));
        pdf.push_str(&format!("{stream} 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n", content.len() + 1));
    }
    pdf.push_str("trailer << /Root 1 0 R >>\n%%EOF");
    pdf.into_bytes()
}

fn harness() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.run_inline = true;
        app.open_bytes("margins.pdf", None, fixture()).unwrap();
        app.set_option("left", "closed").unwrap();
        app
    });
    h.run_steps(6);
    h
}

fn sizes(h: &Harness<'static, PdfCraftApp>) -> Vec<(f32, f32)> {
    let s = h.state();
    s.session.get(s.views[0].id).unwrap().info.pages.iter().map(|p| (p.width, p.height)).collect()
}

#[test]
fn the_command_is_registered_and_in_the_pages_menu() {
    let command = pdfcraft_engine::commands::command("page.remove_white_margins").expect("registered");
    assert_eq!(command.label, "Remove white margins…");
    assert_eq!(command.menu, Some("Pages"));
}

#[test]
fn the_dialog_removes_the_margins_and_undo_restores_them() {
    let mut h = harness();
    assert!(h.state_mut().execute("page.remove_white_margins"));
    h.run_steps(3);
    assert_eq!(h.state().dialog, Some(Dialog::WhiteMargins));
    h.get_by_label("Remove White Margins");
    h.get_by_label("Keep around the content");
    h.get_by_label("Remove").click();
    h.run_steps(4);
    assert_eq!(h.state().dialog, None);
    assert!(h.state().white_margins_run.is_none());
    let after = sizes(&h);
    assert!((after[0].0 - 50.0).abs() < 2.0 && (after[0].1 - 50.0).abs() < 2.0, "{after:?}");
    assert_eq!(&after[1..], [(300.0, 400.0); 2]);
    assert!(h.state().toast.as_ref().is_some_and(|t| t.0.starts_with("Cropped 1 page to its content.")));
    assert!(h.state_mut().execute("edit.undo"));
    h.run_steps(3);
    assert_eq!(sizes(&h), [(300.0, 400.0); 3]);
}

#[test]
fn cancel_closes_the_dialog_and_changes_nothing() {
    let mut h = harness();
    h.state_mut().execute("page.remove_white_margins");
    h.run_steps(3);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(h.state().dialog, None);
    assert_eq!(sizes(&h), [(300.0, 400.0); 3]);
}
