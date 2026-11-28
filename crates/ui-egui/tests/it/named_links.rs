//! Links whose action runs a menu item (`/S /Named`) move through the pages or open Print when clicked.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

/// Three pages; page 1 has a link for each of Next page, Last page, Print and SaveAs, side by side.
fn fixture() -> Vec<u8> {
    let link =
        |x: u32, item: &str| format!("<< /Type /Annot /Subtype /Link /Rect [{x} 20 {} 50] /Border [0 0 0] /A << /S /Named /N /{item} >> >>", x + 60);
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 300 400] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Annots [6 0 R 7 0 R 8 0 R 9 0 R] >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        link(10, "NextPage"),
        link(80, "LastPage"),
        link(150, "Print"),
        link(220, "SaveAs"),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let x = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn harness() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::support::test_app(crate::support::host());
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "links.pdf", None, fixture()).unwrap();
            app.set_option(&egui::Context::default(), "zoom", "100").unwrap();
            app
        }
    });
    h.run_steps(6);
    h
}

/// Goes to page 1 and clicks the link whose left edge is `x` points from the page's.
fn click_link(h: &mut Harness<'static, PdfCraftApp>, x: f32) {
    h.state_mut().views[0].go_to_page(0);
    h.run_steps(6);
    let r = h.state().views[0].page_screen_rect(0).expect("page 1 is on screen");
    let k = r.width() / 300.0;
    let p = r.min + egui::vec2((x + 30.0) * k, (400.0 - 35.0) * k);
    h.hover_at(p);
    h.run_steps(2);
    h.drag_at(p);
    h.run_steps(1);
    h.drop_at(p);
    h.run_steps(4);
}

#[test]
fn links_that_name_a_page_move_there_and_print_asks_first() {
    let mut h = harness();
    click_link(&mut h, 10.0);
    assert_eq!(h.state().views[0].current(), 1, "Next page");
    click_link(&mut h, 80.0);
    assert_eq!(h.state().views[0].current(), 2, "Last page");
    // Only the five safe names run: SaveAs opens nothing.
    click_link(&mut h, 220.0);
    assert!(h.state().dialogs.is_empty(), "no dialog for SaveAs");
    assert_eq!(h.state().views[0].current(), 0);
    click_link(&mut h, 150.0);
    h.get_by_role_and_label(egui::accesskit::Role::Dialog, "Print");
}
