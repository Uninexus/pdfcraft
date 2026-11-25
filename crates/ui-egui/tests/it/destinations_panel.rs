//! The Destinations panel lists the named destinations, goes to one's page, and deletes one as a single undo step.

use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

/// Three pages and three names in a leaf that holds its own `/Names` array (so each can be removed).
const NAMED: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R /Names << /Dests 6 0 R >> >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 300 400] >> endobj
3 0 obj << /Type /Page /Parent 2 0 R >> endobj
4 0 obj << /Type /Page /Parent 2 0 R >> endobj
5 0 obj << /Type /Page /Parent 2 0 R >> endobj
6 0 obj << /Names [(alpha) [3 0 R /Fit] (beta) [5 0 R /Fit] (gamma) [4 0 R /Fit]] >> endobj
trailer << /Root 1 0 R >>
%%EOF";

fn harness() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_step_dt(1.0 / 60.0).build_eframe(|cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "named.pdf", None, NAMED.to_vec()).expect("opens");
            app.set_option(&egui::Context::default(), "left", "closed").unwrap();
            app.set_option(&egui::Context::default(), "panel", "destinations").unwrap();
            app
        }
    });
    h.run_steps(6);
    h
}

fn names(h: &Harness<'static, PdfCraftApp>) -> Vec<String> {
    let id = h.state().views[0].id;
    let read = h.state().session.get(id).unwrap().named_destinations();
    read.destinations.iter().map(|d| d.name.clone()).collect()
}

/// The point on the row whose label starts with `label`.
fn row_at(h: &Harness<'static, PdfCraftApp>, label: &str) -> egui::Pos2 {
    let row = h.get_by(|node| {
        node.role() == egui::accesskit::Role::ListItem && node.label().is_some_and(|text| text.starts_with(label)) && !node.is_hidden()
    });
    let rect = row.rect();
    egui::pos2(rect.left() + rect.width() * 0.25, rect.center().y)
}

fn double_click(h: &mut Harness<'static, PdfCraftApp>, label: &str) {
    let at = row_at(h, label);
    h.hover_at(at);
    h.run_steps(1);
    for _ in 0..2 {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.run_steps(1);
    }
    h.run_steps(3);
}

fn right_click(h: &mut Harness<'static, PdfCraftApp>, label: &str) {
    let at = row_at(h, label);
    h.hover_at(at);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() });
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
}

#[test]
fn the_panel_lists_every_name_of_the_document() {
    let h = harness();
    for name in ["alpha", "beta", "gamma"] {
        row_at(&h, name);
    }
    assert_eq!(names(&h), ["alpha", "beta", "gamma"]);
}

#[test]
fn activating_a_name_moves_the_view_to_its_page() {
    let mut h = harness();
    assert_eq!(h.state().views[0].current(), 0);
    double_click(&mut h, "beta");
    assert_eq!(h.state().views[0].current(), 2, "beta names the third page");
    double_click(&mut h, "gamma");
    assert_eq!(h.state().views[0].current(), 1, "gamma names the second page");
}

#[test]
fn deleting_a_name_is_one_undo_step() {
    let mut h = harness();
    right_click(&mut h, "beta");
    h.get_by_role(egui::accesskit::Role::Menu).get_by_label("Delete").click();
    h.run_steps(3);
    assert_eq!(names(&h), ["alpha", "gamma"]);
    let id = h.state().views[0].id;
    assert_eq!(h.state().session.get(id).unwrap().can_undo(), Some("Remove named destination"));
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(3);
    assert_eq!(names(&h), ["alpha", "beta", "gamma"], "one undo brings the name back");
    assert_eq!(h.state().session.get(id).unwrap().can_undo(), None, "the removal was the only step");
}
