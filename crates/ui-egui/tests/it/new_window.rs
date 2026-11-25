//! Moving a tab to a window of its own (`view.move_to_new_window`, also the tab's context menu): the document opens in a
//! second PdfCraft process and the tab closes here. The process launch goes through the app's `window_spawner`, which
//! these tests replace, so none starts. A document that isn't saved is refused with the reason and its tab stays.

use std::sync::Mutex;

use crate::support::{AppHarness as _, shown_notice};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::Session;
use pdfcraft_ui_egui::PdfCraftApp;

/// Every launch asked for: the file and the page.
static LAUNCHES: Mutex<Vec<(String, usize)>> = Mutex::new(Vec::new());

fn record(path: &str, page: usize) -> Result<(), String> {
    LAUNCHES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((path.to_string(), page));
    Ok(())
}

fn fail(_: &str, _: usize) -> Result<(), String> {
    Err("no window".to_string())
}

/// The launches of `path`.
fn launched(path: &str) -> Vec<usize> {
    LAUNCHES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().filter(|(p, _)| p == path).map(|(_, page)| *page).collect()
}

/// An app with the saved file `name` in `dir` open, launching through `spawner`.
fn harness(dir: &crate::support::TempDir, name: &str, spawner: pdfcraft_ui_egui::WindowSpawner) -> (Harness<'static, PdfCraftApp>, String) {
    let path = dir.path().join(name);
    let bytes = Session::new().create_blank(300.0, 400.0, 3).unwrap().as_ref().clone();
    std::fs::write(&path, &bytes).unwrap();
    let text = path.to_string_lossy().into_owned();
    let opened = text.clone();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        let mut app = crate::support::app_with_clipboard(Default::default());
        app.window_spawner = spawner;
        app.set_option(&egui::Context::default(), "language", "en").unwrap();
        app.open_bytes(&egui::Context::default(), name, Some(opened), bytes).unwrap();
        app
    });
    crate::support::settle(&mut h);
    (h, text)
}

#[test]
fn the_command_opens_the_saved_document_at_its_page_in_a_new_window_and_closes_the_tab() {
    let dir = crate::support::TempDir::new();
    let (mut h, path) = harness(&dir, "moved.pdf", record);
    h.run_steps(2);
    let page = h.state().views[0].current();
    assert!(h.execute("view.move_to_new_window"));
    h.run_steps(3);
    assert_eq!(launched(&path), [page], "one launch, at the page shown");
    assert!(h.state().views.is_empty(), "the tab closed here");
    assert!(h.state().closed_tabs.is_empty(), "it is not offered by Reopen Closed Tab: it lives on in the other window");
}

#[test]
fn the_tabs_context_menu_moves_it_too() {
    let dir = crate::support::TempDir::new();
    let (mut h, path) = harness(&dir, "menu.pdf", record);
    h.get_by_role_and_label(Role::Tab, "menu.pdf").click_secondary();
    h.run_steps(2);
    h.get_by_label("Move to new window").click();
    h.run_steps(3);
    assert_eq!(launched(&path).len(), 1);
    assert!(h.state().views.is_empty());
}

#[test]
fn a_document_with_unsaved_changes_is_refused_with_the_reason_and_stays() {
    let dir = crate::support::TempDir::new();
    let (mut h, path) = harness(&dir, "edited.pdf", record);
    let ctx = h.ctx.clone();
    assert!(h.state_mut().apply_edit(&ctx, pdfcraft_engine::Edit::RotatePages { pages: vec![0], degrees: 90 }));
    assert!(h.execute("view.move_to_new_window"));
    h.run_steps(3);
    assert!(launched(&path).is_empty(), "no process for a document the file doesn't match");
    assert_eq!(h.state().views.len(), 1, "the tab stays");
    assert!(shown_notice(&h).is_some_and(|n| n.contains("Save the document")), "{:?}", shown_notice(&h));
}

#[test]
fn a_launch_that_fails_keeps_the_tab_and_says_so() {
    let dir = crate::support::TempDir::new();
    let (mut h, _) = harness(&dir, "stays.pdf", fail);
    assert!(h.execute("view.move_to_new_window"));
    h.run_steps(3);
    assert_eq!(h.state().views.len(), 1);
    assert!(shown_notice(&h).is_some_and(|n| n.contains("no window")), "{:?}", shown_notice(&h));
}
