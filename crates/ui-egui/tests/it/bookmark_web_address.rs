//! Bookmarks panel ▸ Set web address…: the bookmark opens a web address, checked before OK is offered.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use pdfcraft_engine::Edit;
use pdfcraft_ui_egui::PdfCraftApp;

fn harness() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "pages.pdf", None, crate::support::fixtures::pdf_pages(2)).expect("opens");
            app.set_option(&egui::Context::default(), "panel", "bookmarks").unwrap();
            app
        }
    });
    h.run_steps(6);
    h
}

#[test]
fn the_bookmark_menu_sets_a_web_address() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    let add = Edit::AddBookmark { parent: Vec::new(), index: 0, title: "Manual".into(), page: 1, left: None, top: None };
    assert!(h.state_mut().apply_edit(&ctx, add));
    h.run_steps(3);
    h.get_by_label("Manual").click_secondary();
    h.run_steps(2);
    h.get_by_label("Set web address…").click();
    h.run_steps(3);
    let dialog = h.get_by_role_and_label(Role::Dialog, "Set web address");
    assert!(dialog.get_by_label("OK").accesskit_node().is_disabled(), "OK waits for an address");
    dialog.get_by_role_and_label(Role::TextInput, "Web address").focus();
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, "Web address").type_text("https://example.org/manual");
    h.run_steps(3);
    h.get_by_role_and_label(Role::Dialog, "Set web address").get_by_label("OK").click();
    h.run_steps(4);
    let state = h.state();
    let item = &state.session.get(state.views[0].id).unwrap().info.outline[0];
    assert_eq!(item.page, None);
    assert!(format!("{:?}", item.action).contains("https://example.org/manual"), "{:?}", item.action);
}
