//! Advanced search's Folder scope: choose a folder, search every PDF in it on a worker thread,
//! list the matches by file, and open a file at the page of a result by double-click.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

use crate::support::{fixtures, step_until};

/// A PDF with one line of text on each page.
fn pdf(pages: &[&str]) -> Vec<u8> {
    let n = pages.len();
    let mut objs = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {n} >>", (0..n).map(|i| format!("{} 0 R", 4 + 2 * i)).collect::<Vec<_>>().join(" ")),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    for (i, text) in pages.iter().enumerate() {
        let body = format!("BT /F1 14 Tf 20 150 Td ({text}) Tj ET");
        objs.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
            5 + 2 * i
        ));
        objs.push(format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()));
    }
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

struct Dir(std::path::PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn harness() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("open.pdf", None, fixtures::pdf_pages(1)).unwrap();
        app
    });
    crate::support::settle(&mut h);
    h
}

#[test]
fn the_folder_scope_lists_matches_by_file_and_opens_one_at_its_page() {
    let dir = Dir(std::env::temp_dir().join(format!("pdfcraft-search-folder-ui-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();
    std::fs::write(dir.0.join("one.pdf"), pdf(&["alpha invoice"])).unwrap();
    std::fs::write(dir.0.join("two.pdf"), pdf(&["beta words", "gamma invoice"])).unwrap();
    std::fs::write(dir.0.join("broken.pdf"), b"not a pdf").unwrap();

    let mut h = harness();
    let i = h.state().active.unwrap();
    assert!(h.state_mut().execute("edit.advanced_search"));
    h.run_steps(2);
    h.get_by_label("This document");
    h.get_by_role_and_label(Role::Button, "Folder").click();
    h.run_steps(2);
    assert!(h.state().views[i].find.as_ref().unwrap().folder.on, "the Folder scope is chosen");

    // The folder comes from the picker.
    h.state_mut().pick_override = Some(vec![dir.0.to_string_lossy().into_owned()]);
    h.get_by_role_and_label(Role::Button, "Choose folder").click();
    assert!(step_until(&mut h, |h| h.state().views[i].find.as_ref().unwrap().folder.folder.is_some()), "the picked folder is kept");
    h.state_mut().views[i].find.as_mut().unwrap().query = "invoice".into();
    h.run_steps(2);
    h.get_by_role_and_label(Role::Button, "Search").click();
    assert!(step_until(&mut h, |h| h.state().views[i].find.as_ref().unwrap().folder.summary.is_some()), "the worker thread finished");
    h.run_steps(2);

    let scope = &h.state().views[i].find.as_ref().unwrap().folder;
    let found: Vec<(String, Vec<usize>)> = scope
        .results
        .iter()
        .map(|f| (f.path.file_name().unwrap().to_string_lossy().into_owned(), f.matches.iter().map(|m| m.page).collect()))
        .collect();
    assert_eq!(found, [("one.pdf".to_owned(), vec![0]), ("two.pdf".to_owned(), vec![1])]);
    assert_eq!(scope.skipped.len(), 1, "the corrupt file is skipped, not fatal");
    h.get_by_label("one.pdf (1)");
    h.get_by_label("two.pdf (1)");
    h.get_by_label("2 instances - 2 files searched");
    h.get_by_label("1 file was skipped");

    // Double-clicking the result on page 2 of two.pdf opens it there.
    let node = h.get_by_label_contains("Page 2  ...");
    let at = node.rect().center();
    crate::support::input::double_click(&mut h, at);
    let ok = step_until(&mut h, |h| h.state().views.len() == 2 && h.state().views[1].current == 1);
    let state = h.state();
    assert!(
        ok,
        "two.pdf opens on its second page: views {:?}, toast {:?}",
        state.views.iter().map(|v| (v.current, v.id)).collect::<Vec<_>>(),
        state.toast
    );
    let opened = h.state().active.unwrap();
    assert_eq!(h.state().views[opened].current, 1);
}
