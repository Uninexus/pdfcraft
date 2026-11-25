//! Edit ▸ Media Actions: the window lists the screen annotations and what their action holds, Remove Action takes a
//! media action out as one undo step, and a file without screen annotations gets the empty text.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

use crate::support::AppHarness as _;

/// A file with three screen annotations: object 4 has a rendition action with a script and a `/Next`, object 5 a sound
/// action, object 6 a URI action (not a media action). Without `with_screens` the page has no annotation.
fn screens_pdf(with_screens: bool) -> Vec<u8> {
    let annots = if with_screens { "/Annots [4 0 R 5 0 R 6 0 R]" } else { "" };
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R {annots} >>"),
        "<< /Type /Annot /Subtype /Screen /Rect [0 0 10 10] /A << /S /Rendition /OP 0 /JS (x) /Next << /S /URI /URI (http://a.example/) >> >> >>"
            .to_string(),
        "<< /Type /Annot /Subtype /Screen /Rect [0 20 10 30] /A << /S /Sound >> >>".to_string(),
        "<< /Type /Annot /Subtype /Screen /Rect [0 40 10 50] /A << /S /URI /URI (http://a.example/) >> >>".to_string(),
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
            app.open_bytes(&egui::Context::default(), "screens.pdf", None, bytes.clone()).expect("opens");
            app
        }
    });
    h.run_steps(4);
    h
}

fn can_undo(h: &Harness<'static, PdfCraftApp>) -> bool {
    let id = h.state().views[0].id;
    h.state().session.get(id).unwrap().can_undo().is_some()
}

fn open_window(h: &mut Harness<'static, PdfCraftApp>) {
    assert!(h.execute("edit.media_actions"));
    h.run_steps(4);
}

#[test]
fn each_screen_annotation_is_listed_with_what_its_action_holds() {
    let mut h = harness(screens_pdf(true));
    open_window(&mut h);
    let rendition = h.get_by_role_and_label(Role::Group, "Screen annotation 4 0");
    let value = rendition.value().unwrap_or_default();
    assert!(value.contains("Rendition action"), "the kind is named");
    assert!(value.contains("stored script") && value.contains("next actions"), "the script and the chain are named");
    let sound = h.get_by_role_and_label(Role::Group, "Screen annotation 5 0");
    assert!(sound.value().is_some_and(|v| v.contains("Sound action")));
    let other = h.get_by_role_and_label(Role::Group, "Screen annotation 6 0");
    assert!(other.value().is_some_and(|v| v.contains("Other action (URI)")), "another action is shown by its name");
    assert_eq!(h.query_all_by_label("Remove Action").count(), 2, "only the two media actions can be removed");
    assert!(!can_undo(&h), "listing changes nothing");
}

#[test]
fn removing_an_action_is_one_undo_step_and_undo_lists_it_again() {
    let mut h = harness(screens_pdf(true));
    open_window(&mut h);
    h.query_all_by_label("Remove Action").next().expect("a Remove Action button").click();
    h.run_steps(4);
    let first = h.get_by_role_and_label(Role::Group, "Screen annotation 4 0");
    assert!(first.value().is_some_and(|v| !v.contains("Rendition action")), "the action is gone and the annotation stays");
    assert_eq!(h.query_all_by_label("Remove Action").count(), 1);
    assert!(can_undo(&h));
    h.undo();
    h.run_steps(4);
    let first = h.get_by_role_and_label(Role::Group, "Screen annotation 4 0");
    assert!(first.value().is_some_and(|v| v.contains("Rendition action")), "undo brings the action back");
    assert_eq!(h.query_all_by_label("Remove Action").count(), 2);
    assert!(!can_undo(&h), "one undo took back the whole removal");
}

#[test]
fn a_file_without_screen_annotations_says_so() {
    let mut h = harness(screens_pdf(false));
    open_window(&mut h);
    h.get_by_label("This document has no screen annotations.");
    assert!(h.query_all_by_label("Remove Action").next().is_none());
}
