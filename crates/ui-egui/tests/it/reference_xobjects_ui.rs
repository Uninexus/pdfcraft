//! Standards ▸ Reference XObjects: the window lists the forms that carry a `/Ref`, Remove Reference takes one out as one
//! undo step, and a file without any gets the empty text.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

use crate::support::AppHarness as _;

/// A file with two forms that have a `/Ref` (objects 4 and 6, to `a.pdf` page 0 and `d.pdf` page 3) and an ordinary form.
fn forms_pdf(with_refs: bool) -> Vec<u8> {
    let form = |extra: &str| format!("<< /Subtype /Form /BBox [0 0 10 10] {extra} /Length 5 >>\nstream\n0 0 m\nendstream");
    let (first, second) = if with_refs { ("/Ref << /F (a.pdf) /Page 0 >>", "/Ref << /F (d.pdf) /Page 3 >>") } else { ("", "") };
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Resources << /XObject << /A 4 0 R /B 5 0 R /C 6 0 R >> >> >>".to_string(),
        form(first),
        form(""),
        form(second),
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
            app.open_bytes(&egui::Context::default(), "refs.pdf", None, bytes.clone()).expect("opens");
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
    assert!(h.execute("standards.reference_xobjects"));
    h.run_steps(4);
}

#[test]
fn each_reference_is_listed_with_its_file_and_page() {
    let mut h = harness(forms_pdf(true));
    open_window(&mut h);
    let first = h.get_by_role_and_label(Role::Group, "Reference to a.pdf");
    let second = h.get_by_role_and_label(Role::Group, "Reference to d.pdf");
    assert!(first.value().is_some_and(|v| v.contains("page 1")), "the first names its page");
    assert!(second.value().is_some_and(|v| v.contains("page 4")), "the second names its page");
    assert_eq!(h.query_all_by_label("Remove Reference").count(), 2, "each one can be removed");
    assert!(!can_undo(&h), "listing changes nothing");
}

#[test]
fn removing_a_reference_is_one_undo_step_and_undo_lists_it_again() {
    let mut h = harness(forms_pdf(true));
    open_window(&mut h);
    h.query_all_by_label("Remove Reference").next().expect("a Remove Reference button").click();
    h.run_steps(4);
    assert!(h.query_by_role_and_label(Role::Group, "Reference to a.pdf").is_none(), "the removed row is gone");
    h.get_by_role_and_label(Role::Group, "Reference to d.pdf");
    assert!(can_undo(&h));
    h.undo();
    h.run_steps(4);
    h.get_by_role_and_label(Role::Group, "Reference to a.pdf");
    h.get_by_role_and_label(Role::Group, "Reference to d.pdf");
    assert!(!can_undo(&h), "one undo took back the whole removal");
}

#[test]
fn a_file_without_references_says_so() {
    let mut h = harness(forms_pdf(false));
    open_window(&mut h);
    h.get_by_label("This document has no reference XObjects.");
    assert!(h.query_all_by_label("Remove Reference").next().is_none());
}
