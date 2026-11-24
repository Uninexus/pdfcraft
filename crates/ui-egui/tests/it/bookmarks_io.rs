//! Exporting and importing bookmarks from the Bookmarks panel, through scripted file pickers.

use std::path::Path;
use std::sync::Arc;

use craft::host::{Picked, ScriptedFiles};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::Edit;
use pdfcraft_ui_egui::PdfCraftApp;

const PAGES: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 300 400] >> endobj
3 0 obj << /Type /Page /Parent 2 0 R >> endobj
4 0 obj << /Type /Page /Parent 2 0 R >> endobj
5 0 obj << /Type /Page /Parent 2 0 R >> endobj
trailer << /Root 1 0 R >>
%%EOF";

type Flat = Vec<(String, Option<usize>, usize)>;

/// The app with `a.pdf` (holding Intro > Detail, then Results) and `b.pdf` (no bookmarks) open, `b.pdf` in front,
/// and the Bookmarks panel showing. File picks answer from `files`.
fn harness(files: &Arc<ScriptedFiles>) -> Harness<'static, PdfCraftApp> {
    let files = files.clone();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        let ctx = egui::Context::default();
        let mut app = crate::support::test_app(crate::support::host_with_files(files));
        app.set_option(&ctx, "language", "en").unwrap();
        app.open_bytes(&ctx, "a.pdf", None, PAGES.to_vec()).expect("opens");
        for (parent, index, title, page) in [(vec![], 0, "Intro", 0), (vec![0], 0, "Detail", 1), (vec![], 1, "Results", 2)] {
            app.apply_edit(&ctx, Edit::AddBookmark { parent, index, title: title.into(), page, left: None, top: None });
        }
        app.open_bytes(&ctx, "b.pdf", None, PAGES.to_vec()).expect("opens");
        app.set_option(&ctx, "left", "closed").unwrap();
        app.set_option(&ctx, "panel", "bookmarks").unwrap();
        app
    });
    h.run_steps(6);
    h
}

/// The bookmarks of the document in tab `tab`, in order, as (title, 0-based page, depth).
fn outline(h: &Harness<'static, PdfCraftApp>, tab: usize) -> Flat {
    fn flat(items: &[pdfcraft_render::OutlineItem], depth: usize, out: &mut Flat) {
        for i in items {
            out.push((i.title.clone(), i.page, depth));
            flat(&i.children, depth + 1, out);
        }
    }
    let mut out = Vec::new();
    flat(&h.state().session.get(h.state().views[tab].id).unwrap().info.outline, 0, &mut out);
    out
}

fn show_tab(h: &mut Harness<'static, PdfCraftApp>, tab: usize) {
    h.state_mut().active = Some(tab);
    h.run_steps(3);
}

/// Choose `item` of the Bookmarks panel's options menu.
fn options(h: &mut Harness<'static, PdfCraftApp>, item: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Panel Menu").click();
    h.run_steps(2);
    h.get_by_label(item).click();
    h.run_steps(4);
}

/// Choose `item` of the context menu of the bookmark called `bookmark`.
fn context(h: &mut Harness<'static, PdfCraftApp>, bookmark: &str, item: &str) {
    h.get_by_label(bookmark).click_secondary();
    h.run_steps(2);
    h.get_by_label(item).click();
    h.run_steps(4);
}

#[test]
fn exported_bookmarks_import_into_another_document_as_one_undo_step() {
    let out = Path::new("/out/marks.json");
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![out.to_path_buf()]), Picked::Paths(vec![out.to_path_buf()])]));
    let mut h = harness(&files);
    show_tab(&mut h, 0);
    let source = outline(&h, 0);
    assert_eq!(source.len(), 3);
    options(&mut h, "Export Bookmarks…");
    assert!(files.file(out).is_some_and(|f| !f.is_empty()), "the file was written");

    show_tab(&mut h, 1);
    assert!(outline(&h, 1).is_empty());
    options(&mut h, "Import Bookmarks…");
    assert_eq!(outline(&h, 1), source, "the same bookmarks, nesting and pages");
    let id = h.state().views[1].id;
    assert!(h.state().session.get(id).unwrap().can_undo().is_some());

    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(3);
    assert!(outline(&h, 1).is_empty(), "one undo takes all of them back");
    assert_eq!(h.state().session.get(id).unwrap().can_undo(), None);
}

#[test]
fn one_bookmark_exports_with_its_subtree_and_imports_under_another() {
    let out = Path::new("/out/intro.json");
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![out.to_path_buf()]), Picked::Paths(vec![out.to_path_buf()])]));
    let mut h = harness(&files);
    show_tab(&mut h, 0);
    context(&mut h, "Intro", "Export This Bookmark…");
    let text = String::from_utf8(files.file(out).expect("written")).unwrap();
    assert!(text.contains("Detail") && !text.contains("Results"), "{text}");

    show_tab(&mut h, 1);
    let ctx = h.ctx.clone();
    h.state_mut().apply_edit(&ctx, Edit::AddBookmark { parent: vec![], index: 0, title: "Host".into(), page: 2, left: None, top: None });
    h.run_steps(3);
    context(&mut h, "Host", "Import Under This Bookmark…");
    assert_eq!(outline(&h, 1), [("Host".into(), Some(2), 0), ("Intro".into(), Some(0), 1), ("Detail".into(), Some(1), 2)]);
}

#[test]
fn a_file_that_is_not_bookmark_json_is_refused_with_its_reason_and_changes_nothing() {
    let (garbled, other) = (Path::new("/in/garbled.json"), Path::new("/in/other.json"));
    let files = Arc::new(
        ScriptedFiles::answering([Picked::Paths(vec![garbled.to_path_buf()]), Picked::Paths(vec![other.to_path_buf()])])
            .with_file(garbled, b"this is not json".to_vec())
            .with_file(other, br#"{"pages": []}"#.to_vec()),
    );
    let mut h = harness(&files);
    let id = h.state().views[1].id;
    for _ in 0..2 {
        options(&mut h, "Import Bookmarks…");
        let notice = crate::support::shown_notice(&h).expect("a notice");
        assert!(notice.contains("not a bookmark file"), "{notice}");
        assert!(outline(&h, 1).is_empty());
        assert_eq!(h.state().session.get(id).unwrap().can_undo(), None, "nothing changed");
    }
}
