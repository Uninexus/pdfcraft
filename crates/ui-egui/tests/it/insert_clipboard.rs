//! Pages ▸ Insert pages from clipboard in the real shell (egui_kittest): a copied picture or copied files become
//! pages after the selection as one undo step, what can't be used is named, and an empty clipboard says what may be
//! copied. The app reads a stand-in clipboard, never the system's.

use crate::support::{AppHarness as _, RecordingClipboard, shown_notice};
use craft::platform::clipboard::ClipboardImage;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::Session;
use pdfcraft_ui_egui::PdfCraftApp;

fn blank(pages: usize, width: f64, height: f64) -> Vec<u8> {
    Session::new().create_blank(width, height, pages).unwrap().as_ref().clone()
}

fn harness(clipboard: RecordingClipboard) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::support::app_with_clipboard(clipboard);
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "doc.pdf", None, blank(2, 300.0, 400.0)).unwrap();
            app
        }
    });
    crate::support::settle(&mut h);
    h
}

/// The page sizes of the open document, `(width, height)` each.
fn sizes(h: &Harness<'static, PdfCraftApp>) -> Vec<(f32, f32)> {
    let app = h.state();
    app.session.get(app.views[0].id).unwrap().info.pages.iter().map(|p| (p.width, p.height)).collect()
}

fn can_undo(h: &Harness<'static, PdfCraftApp>) -> bool {
    let app = h.state();
    app.session.get(app.views[0].id).unwrap().can_undo().is_some()
}

#[test]
fn a_copied_picture_becomes_one_page_after_the_current_page_and_undoes_in_one_step() {
    let image = ClipboardImage { width: 40, height: 20, rgba: [200, 30, 30, 255].repeat(40 * 20) };
    let mut h = harness(RecordingClipboard::holding_image(image));
    assert!(h.execute("page.insert_clipboard"));
    h.run_steps(3);
    let after = sizes(&h);
    assert_eq!(after.len(), 3, "one page was added");
    assert!(after[1].0 > after[1].1, "the new page is the picture's own shape, after page 1: {after:?}");
    assert_eq!(h.state().views[0].target_pages(), [1], "the new page is selected");
    h.undo();
    h.run_steps(2);
    assert_eq!(sizes(&h).len(), 2, "one undo takes the whole insert back");
}

#[test]
fn copied_files_go_in_order_and_what_cannot_be_used_is_named() {
    let dir = crate::support::TempDir::new();
    let pdf = dir.path().join("three.pdf");
    std::fs::write(&pdf, blank(3, 200.0, 200.0)).unwrap();
    let junk = dir.path().join("junk.bin");
    std::fs::write(&junk, [0u8, 1, 2, 3, 4, 5]).unwrap();
    let mut h = harness(RecordingClipboard::holding_files(vec![junk, pdf]));
    assert!(h.execute("page.insert_clipboard"));
    h.run_steps(3);
    assert_eq!(sizes(&h).len(), 5, "the PDF's three pages went in");
    assert_eq!(h.state().views[0].target_pages(), [1, 2, 3]);
    let said = shown_notice(&h).expect("the left-out file is announced");
    assert!(said.contains("junk.bin"), "{said}");
}

#[test]
fn nothing_usable_on_the_clipboard_says_what_to_copy_and_changes_nothing() {
    let mut h = harness(RecordingClipboard::default());
    assert!(h.execute("page.insert_clipboard"));
    h.run_steps(3);
    assert_eq!(sizes(&h).len(), 2);
    assert!(!can_undo(&h), "no edit was made");
    assert!(shown_notice(&h).is_some_and(|n| n.contains("no picture")));
    // Files that are all unusable are named and also change nothing.
    let dir = crate::support::TempDir::new();
    let junk = dir.path().join("junk.bin");
    std::fs::write(&junk, [9u8, 9, 9]).unwrap();
    let mut h = harness(RecordingClipboard::holding_files(vec![junk]));
    assert!(h.execute("page.insert_clipboard"));
    h.run_steps(3);
    assert_eq!(sizes(&h).len(), 2);
    assert!(shown_notice(&h).is_some_and(|n| n.contains("junk.bin") && n.contains("Nothing on the clipboard could be inserted")));
}

#[test]
fn the_pages_menu_lists_it() {
    let image = ClipboardImage { width: 8, height: 8, rgba: [0, 0, 0, 255].repeat(64) };
    let mut h = harness(RecordingClipboard::holding_image(image));
    crate::support::open_menu(&mut h, "Edit");
    crate::support::menu_row(&h, "Pages").hover();
    h.run_steps(3);
    crate::support::menu_row(&h, "Insert pages from clipboard").click();
    h.run_steps(3);
    assert_eq!(sizes(&h).len(), 3, "the menu row ran the command");
}

/// The page grid's menu puts the clipboard before the page, the first one included.
#[test]
fn the_page_menu_inserts_before_a_page() {
    let image = ClipboardImage { width: 40, height: 20, rgba: [200, 30, 30, 255].repeat(40 * 20) };
    let mut h = harness(RecordingClipboard::holding_image(image));
    h.state_mut().views[0].organize = true;
    h.run_steps(4);
    crate::support::settle(&mut h);
    h.get_by_label("Page 1").click_secondary();
    h.run_steps(4);
    h.get_by_label("Insert pages from clipboard before").click();
    h.run_steps(4);
    let after = sizes(&h);
    assert_eq!(after.len(), 3);
    assert!(after[0].0 > after[0].1, "the picture is the first page: {after:?}");
    assert_eq!(h.state().views[0].target_pages(), [0]);
}
