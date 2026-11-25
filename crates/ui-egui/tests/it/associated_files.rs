//! PDF 2.0 associated files (`/AF`) in the Attachments panel: Attach as Associated File… in the panel's options menu,
//! and Set Relationship… and Remove from Associated Files in a row's menu. Each is one undo step, through the edits the
//! tools `af_add`, `af_set_relationship` and `af_remove` apply.

use std::path::PathBuf;
use std::sync::Arc;

use craft::host::{Picked, ScriptedFiles};
use craft::ui::dialog::dialog_id;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;
use serde_json::json;

use crate::support::AppHarness as _;

const ATTACH: &str = "attachment.associate";
const RELATIONSHIP: &str = "attachment.relationship";

/// A one-page PDF with one embedded file, `notes.txt`, so the Attachments panel lists something to start with.
fn base_pdf() -> Vec<u8> {
    let objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles << /Names [(notes.txt) 5 0 R] >> >> >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << >> >>".to_vec(),
        b"<< /Type /EmbeddedFile /Length 11 >>\nstream\nhello notes\nendstream".to_vec(),
        b"<< /Type /Filespec /F (notes.txt) /UF (notes.txt) /EF << /F 4 0 R >> >>".to_vec(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

/// A file on disk to attach, in a directory of the test's own.
fn file_to_attach(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pdfcraft-assoc-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("orders.csv");
    std::fs::write(&path, b"id,total\n1,10\n").unwrap();
    path
}

/// The app with the base document open and the Attachments panel showing; the file picker answers with `pick`.
fn harness(pick: PathBuf) -> Harness<'static, PdfCraftApp> {
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![pick])]));
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::support::test_app(crate::support::host_with_files(files.clone()));
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "doc.pdf", None, base_pdf()).unwrap();
            app.set_option(&egui::Context::default(), "panel", "attachments").unwrap();
            app
        }
    });
    h.run_steps(4);
    h
}

/// The document's associated files as (name, relationship, description).
fn associated(h: &Harness<'static, PdfCraftApp>) -> Vec<(String, Option<String>, Option<String>)> {
    let id = h.state().views[0].id;
    let files = h.state().session.get(id).unwrap().associated_files();
    files.into_iter().map(|f| (f.name.unwrap_or_default(), f.relationship, f.description)).collect()
}

/// The names of the document's attachments.
fn attachments(h: &Harness<'static, PdfCraftApp>) -> Vec<String> {
    let id = h.state().views[0].id;
    h.state().session.get(id).unwrap().info.attachments.iter().map(|a| a.name.clone()).collect()
}

fn can_undo(h: &Harness<'static, PdfCraftApp>) -> Option<String> {
    let id = h.state().views[0].id;
    h.state().session.get(id).unwrap().can_undo().map(str::to_owned)
}

fn set(h: &mut Harness<'static, PdfCraftApp>, key: &str, field: &str, value: serde_json::Value) {
    let ctx = h.ctx.clone();
    h.state_mut().dialogs.set_json_field(&ctx, dialog_id(key), field, value).unwrap();
    h.run_steps(1);
}

/// Confirms the open dialog and checks that it closed.
fn confirm(h: &mut Harness<'static, PdfCraftApp>, title: &str) {
    h.get_by_label("OK").click();
    h.run_steps(4);
    assert!(h.query_by_role_and_label(Role::Dialog, title).is_none(), "{title} closed");
}

/// Attaches `orders.csv` as an associated file with `relationship` and `description`, through the options menu.
fn attach(h: &mut Harness<'static, PdfCraftApp>, relationship: &str, description: &str) {
    h.get_by_role_and_label(Role::Button, "Panel Menu").click();
    h.run_steps(2);
    h.get_by_label("Attach as Associated File…").click();
    h.run_steps(4);
    h.get_by_role_and_label(Role::Dialog, "Attach as Associated File");
    set(h, ATTACH, "relationship", json!(relationship));
    set(h, ATTACH, "description", json!(description));
    confirm(h, "Attach as Associated File");
}

/// The text of the row of the attachment called `name`: its name followed by its details.
fn row_text(h: &Harness<'static, PdfCraftApp>, name: &str) -> String {
    use egui_kittest::kittest::NodeT as _;
    h.get_by(|node| node.role() == Role::ListItem && node.label().is_some_and(|text| text.starts_with(name)) && !node.is_hidden())
        .accesskit_node()
        .label()
        .unwrap_or_default()
        .to_string()
}

/// Chooses `item` of the context menu of the row called `name`.
fn row_menu(h: &mut Harness<'static, PdfCraftApp>, name: &str, item: &str) {
    h.get_by(|node| node.role() == Role::ListItem && node.label().is_some_and(|text| text.starts_with(name)) && !node.is_hidden()).click_secondary();
    h.run_steps(2);
    h.get_by_role(Role::Menu).get_by_label(item).click();
    h.run_steps(4);
}

#[test]
fn attaching_an_associated_file_lists_it_with_its_relationship_in_one_undo_step() {
    let mut h = harness(file_to_attach("attach"));
    assert!(associated(&h).is_empty());
    attach(&mut h, "Source", "Order data");
    assert_eq!(associated(&h), [("orders.csv".to_owned(), Some("Source".to_owned()), Some("Order data".to_owned()))]);
    assert!(attachments(&h).contains(&"orders.csv".to_owned()), "it is also an attachment");
    let row = row_text(&h, "orders.csv");
    assert!(row.contains("Source") && row.contains("document"), "the row says how it is associated: {row:?}");
    assert!(!row_text(&h, "notes.txt").contains("Associated"), "an attachment that is not listed says nothing");
    assert!(can_undo(&h).is_some_and(|label| label.starts_with("Associate")));
    h.undo();
    h.run_steps(3);
    assert!(associated(&h).is_empty() && !attachments(&h).contains(&"orders.csv".to_owned()), "one undo takes the file and its entry back");
    assert_eq!(can_undo(&h), None, "the dialog made a single step");
}

#[test]
fn changing_the_relationship_is_one_undo_step() {
    let mut h = harness(file_to_attach("relationship"));
    attach(&mut h, "Source", "");
    row_menu(&mut h, "orders.csv", "Set Relationship…");
    h.get_by_role_and_label(Role::Dialog, "Set Relationship");
    set(&mut h, RELATIONSHIP, "relationship", json!("Data"));
    confirm(&mut h, "Set Relationship");
    assert_eq!(associated(&h).first().map(|f| f.1.clone()), Some(Some("Data".to_owned())));
    assert!(row_text(&h, "orders.csv").contains("Data"));
    assert_eq!(can_undo(&h).as_deref(), Some("Set associated file relationship"));
    h.undo();
    h.run_steps(3);
    assert_eq!(associated(&h).first().map(|f| f.1.clone()), Some(Some("Source".to_owned())), "one undo restores it");
    assert!(can_undo(&h).is_some_and(|label| label.starts_with("Associate")), "the attach is the step before");
}

#[test]
fn removing_it_from_the_associated_files_is_one_undo_step() {
    let mut h = harness(file_to_attach("remove"));
    attach(&mut h, "Source", "");
    row_menu(&mut h, "orders.csv", "Remove from Associated Files and Delete Attachment");
    assert!(associated(&h).is_empty(), "no longer listed");
    assert!(!attachments(&h).contains(&"orders.csv".to_owned()), "the attachment went with it");
    h.undo();
    h.run_steps(3);
    assert_eq!(associated(&h).first().map(|f| f.1.clone()), Some(Some("Source".to_owned())), "one undo lists it again");
    assert!(attachments(&h).contains(&"orders.csv".to_owned()), "and brings the attachment back");
    assert!(can_undo(&h).is_some_and(|label| label.starts_with("Associate")), "the attach is the step before");
}

#[test]
fn keeping_the_attachment_removes_only_the_entry() {
    let mut h = harness(file_to_attach("keep"));
    attach(&mut h, "Data", "");
    row_menu(&mut h, "orders.csv", "Remove from Associated Files, Keep Attachment");
    assert!(associated(&h).is_empty());
    assert!(attachments(&h).contains(&"orders.csv".to_owned()), "the embedded file stays");
    assert!(!row_text(&h, "orders.csv").contains("Associated"), "and no longer says it is associated");
}
