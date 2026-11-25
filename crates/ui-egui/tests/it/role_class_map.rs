//! Role Map and Class Map (the Tags panel's options menu): each OK is one undo step through the tools' own edits, a
//! change the engine refuses writes nothing and says why, and a document with no structure tree gets a notice.

use craft::ui::dialog::dialog_id;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;
use serde_json::json;

use crate::support::AppHarness as _;

/// A tagged page with two custom roles (`Heading` and `Callout`) and two classes (`Boxed`, a dictionary, and `Wide`,
/// an array of two).
fn tagged_pdf() -> Vec<u8> {
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 4 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Type /StructTreeRoot /K 5 0 R /RoleMap << /Heading /H1 /Callout /P >> /ClassMap << /Boxed 6 0 R /Wide [6 0 R 7 0 R] >> >>",
        "<< /S /Document /K 8 0 R >>",
        "<< /O /Layout /Placement /Block >>",
        "<< /O /Table /Summary (x) >>",
        "<< /S /Heading /Pg 3 0 R /ActualText (Report) >>",
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

/// A page with no structure tree.
const UNTAGGED: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 100] >> endobj
3 0 obj << /Type /Page /Parent 2 0 R >> endobj
trailer << /Root 1 0 R >>
%%EOF";

fn harness(bytes: Vec<u8>) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "tagged.pdf", None, bytes.clone()).expect("opens");
            app
        }
    });
    h.run_steps(4);
    h
}

/// The role map as (role, maps to) pairs, sorted.
fn roles(h: &Harness<'static, PdfCraftApp>) -> Vec<(String, Option<String>)> {
    let id = h.state().views[0].id;
    let mut pairs: Vec<_> = h.state().session.get(id).unwrap().role_map().entries.iter().map(|e| (e.role.clone(), e.maps_to.clone())).collect();
    pairs.sort();
    pairs
}

/// The class names, sorted.
fn classes(h: &Harness<'static, PdfCraftApp>) -> Vec<String> {
    let id = h.state().views[0].id;
    let mut names: Vec<_> = h.state().session.get(id).unwrap().class_map().entries.iter().map(|e| e.class.clone()).collect();
    names.sort();
    names
}

fn can_undo(h: &Harness<'static, PdfCraftApp>) -> Option<String> {
    let id = h.state().views[0].id;
    h.state().session.get(id).unwrap().can_undo().map(str::to_owned)
}

/// The index the dialog gives the entry whose name is `name`.
fn role_index(h: &Harness<'static, PdfCraftApp>, name: &str) -> usize {
    let id = h.state().views[0].id;
    h.state().session.get(id).unwrap().role_map().entries.iter().position(|e| e.role == name).expect("the role is listed")
}

fn class_index(h: &Harness<'static, PdfCraftApp>, name: &str) -> usize {
    let id = h.state().views[0].id;
    h.state().session.get(id).unwrap().class_map().entries.iter().position(|e| e.class == name).expect("the class is listed")
}

fn set(h: &mut Harness<'static, PdfCraftApp>, key: &str, field: &str, value: serde_json::Value) {
    let ctx = h.ctx.clone();
    h.state_mut().dialogs.set_json_field(&ctx, dialog_id(key), field, value).unwrap();
    h.run_steps(1);
}

#[test]
fn adding_a_mapping_and_removing_another_is_one_undo_step() {
    let mut h = harness(tagged_pdf());
    assert!(h.execute("a11y.role_map"));
    h.run_steps(2);
    h.get_by_role_and_label(Role::Dialog, "Role Map");
    let callout = role_index(&h, "Callout");
    let heading = role_index(&h, "Heading");
    set(&mut h, "a11y.role_map", &format!("to.{callout}"), json!(""));
    set(&mut h, "a11y.role_map", &format!("to.{heading}"), json!("H2"));
    set(&mut h, "a11y.role_map", "new_role.0", json!("Note"));
    set(&mut h, "a11y.role_map", "new_to.0", json!("Sect"));
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(
        roles(&h),
        [("Heading".to_owned(), Some("H2".to_owned())), ("Note".to_owned(), Some("Sect".to_owned()))],
        "one role changed, one added and one removed"
    );
    assert_eq!(can_undo(&h).as_deref(), Some("Change role map"));
    h.undo();
    h.run_steps(2);
    assert_eq!(
        roles(&h),
        [("Callout".to_owned(), Some("P".to_owned())), ("Heading".to_owned(), Some("H1".to_owned()))],
        "one undo takes every change back"
    );
    assert_eq!(can_undo(&h), None, "the dialog made a single step");
}

#[test]
fn a_refused_role_name_shows_its_reason_and_writes_nothing() {
    let mut h = harness(tagged_pdf());
    assert!(h.execute("a11y.role_map"));
    h.run_steps(2);
    set(&mut h, "a11y.role_map", "new_role.0", json!("Loop"));
    set(&mut h, "a11y.role_map", "new_to.0", json!("Loop"));
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(roles(&h).len(), 2, "the role map is as it was");
    assert_eq!(can_undo(&h), None, "no undo step was added");
    let notice = crate::support::shown_notice(&h).unwrap_or_default();
    assert!(notice.contains("itself"), "the reason is shown: {notice:?}");
    h.get_by_role_and_label(Role::Dialog, "Role Map");
}

#[test]
fn removing_a_class_is_one_undo_step() {
    let mut h = harness(tagged_pdf());
    assert_eq!(classes(&h), ["Boxed", "Wide"]);
    assert!(h.execute("a11y.class_map"));
    h.run_steps(2);
    h.get_by_role_and_label(Role::Dialog, "Class Map");
    let boxed = class_index(&h, "Boxed");
    set(&mut h, "a11y.class_map", &format!("rm.{boxed}"), json!(true));
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(classes(&h), ["Wide"]);
    assert_eq!(can_undo(&h).as_deref(), Some("Remove class map entry"));
    h.undo();
    h.run_steps(2);
    assert_eq!(classes(&h), ["Boxed", "Wide"], "one undo brings the class back");
    assert_eq!(can_undo(&h), None);
}

#[test]
fn a_document_with_no_structure_tree_gets_a_notice_instead_of_a_dialog() {
    let mut h = harness(UNTAGGED.to_vec());
    for command in ["a11y.role_map", "a11y.class_map"] {
        h.execute(command);
        h.run_steps(2);
        let notice = crate::support::shown_notice(&h).unwrap_or_default();
        assert!(notice.contains("no structure tree"), "{command}: {notice:?}");
        assert!(h.query_by_role_and_label(Role::Dialog, "Role Map").is_none());
        assert!(h.query_by_role_and_label(Role::Dialog, "Class Map").is_none());
    }
    assert_eq!(can_undo(&h), None);
}
