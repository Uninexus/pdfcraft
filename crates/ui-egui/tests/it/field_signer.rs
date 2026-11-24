//! The Signer entry of a digital signature field's Field Properties: Set signer and No one run the registered
//! commands `form.field.signer.set` and `form.field.signer.clear`, each one undo step, with the address rule and the
//! signing plan beside the file applied as the `form_field_signer` tool does.

use crate::support::{AppHarness as _, shown_notice};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::signing_plan::{Plan, Role as PlanRole, plan_bytes};
use pdfcraft_engine::{Edit, NewField, Session};
use pdfcraft_ui_egui::PdfCraftApp;
use pdfcraft_ui_egui::prepare::FieldPropsDialog;

/// A blank page with a signature field `Sig1` and a text field `Name`.
fn fixture() -> Vec<u8> {
    let mut session = Session::new();
    let bytes = session.create_blank(300.0, 400.0, 1).unwrap();
    let id = session.open("signer.pdf", None, bytes, None).unwrap();
    for (name, rect, kind) in
        [("Sig1", [40.0, 290.0, 220.0, 326.0], NewField::Signature), ("Name", [40.0, 230.0, 230.0, 254.0], NewField::Text { multiline: false })]
    {
        session.apply(id, Edit::AddField { name: Some(name.into()), page: 0, rect, kind }).unwrap();
    }
    session.save_bytes(id).unwrap().as_ref().clone()
}

fn harness(path: Option<String>) -> Harness<'static, PdfCraftApp> {
    let bytes = fixture();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "signer.pdf", path, bytes).unwrap();
            app
        }
    });
    crate::support::settle(&mut h);
    h
}

/// The document with a signing plan of Ann (signer), Bob (viewer) beside its file.
fn with_plan() -> (crate::support::TempDir, Harness<'static, PdfCraftApp>) {
    let dir = crate::support::TempDir::new().unwrap();
    let pdf = dir.path().join("signer.pdf");
    std::fs::write(&pdf, fixture()).unwrap();
    let mut plan = Plan::empty(1);
    plan.add_recipient("Ann", "ann@example.org", PlanRole::Signer, 1).unwrap();
    plan.add_recipient("Bob", "bob@example.org", PlanRole::Viewer, 1).unwrap();
    std::fs::write(dir.path().join("signer.pdf.pdfcraft-sign.json"), plan_bytes(&plan).unwrap()).unwrap();
    let h = harness(Some(pdf.to_string_lossy().into_owned()));
    (dir, h)
}

fn open_props(h: &mut Harness<'static, PdfCraftApp>, name: &str) {
    h.set_option("dialog", &format!("field-properties:{name}")).unwrap();
    h.run_steps(3);
}

/// The signer label the engine holds for `name`.
fn label(h: &Harness<'static, PdfCraftApp>, name: &str) -> Option<String> {
    let s = h.state();
    let doc = s.session.get(s.views[0].id).unwrap();
    let field = doc.form.iter().find(|f| f.name == name).unwrap();
    doc.field_signer(field.obj).email
}

fn can_undo(h: &Harness<'static, PdfCraftApp>) -> bool {
    let s = h.state();
    s.session.get(s.views[0].id).unwrap().can_undo().is_some()
}

fn type_email(h: &mut Harness<'static, PdfCraftApp>, text: &str) {
    h.get_by_role_and_label(Role::TextInput, "Signer email").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_role_and_label(Role::TextInput, "Signer email").type_text(text);
    h.run_steps(2);
}

fn press(h: &mut Harness<'static, PdfCraftApp>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).click();
    h.run_steps(3);
}

#[test]
fn set_signer_and_no_one_are_each_one_undo_step() {
    let mut h = harness(None);
    open_props(&mut h, "Sig1");
    assert!(h.state().dialogs.is_open::<FieldPropsDialog>());
    assert!(h.query_all_by_label_contains("Signer: No one").count() > 0, "the entry says who the field is for now");
    type_email(&mut h, "ann@example.org");
    press(&mut h, "Set signer");
    assert_eq!(label(&h, "Sig1").as_deref(), Some("ann@example.org"));
    assert!(can_undo(&h), "the label is one undo step");
    assert!(shown_notice(&h).is_some_and(|n| n.contains("Signer set to")), "the command says what it did");
    assert!(h.query_all_by_label_contains("Signer: ann@example.org").count() > 0, "the entry shows the new label");
    press(&mut h, "No one (clear signer)");
    assert_eq!(label(&h, "Sig1"), None);
    assert!(h.query_all_by_label_contains("Signer: No one").count() > 0);
    h.undo();
    h.run_steps(3);
    assert_eq!(label(&h, "Sig1").as_deref(), Some("ann@example.org"), "undo brings the label back");
    assert!(h.query_all_by_label_contains("Signer: ann@example.org").count() > 0, "and the entry follows the undo");
}

#[test]
fn a_malformed_address_changes_nothing() {
    let mut h = harness(None);
    open_props(&mut h, "Sig1");
    for bad in ["not-an-address", "a b@example.org", "a@b"] {
        type_email(&mut h, bad);
        press(&mut h, "Set signer");
        assert_eq!(label(&h, "Sig1"), None, "{bad:?} is refused");
        assert!(!can_undo(&h), "and adds no undo step");
        assert!(shown_notice(&h).is_some_and(|n| n.contains("must look like")), "with the reason");
    }
}

#[test]
fn a_text_field_has_no_signer_entry_and_the_command_refuses_it() {
    let mut h = harness(None);
    open_props(&mut h, "Name");
    assert!(h.query_all_by_role_and_label(Role::Button, "Set signer").next().is_none());
    assert!(h.execute("form.field.signer.clear"));
    h.run_steps(2);
    assert_eq!(label(&h, "Name"), None);
    assert!(!can_undo(&h));
}

#[test]
fn the_commands_need_the_open_dialog() {
    let mut h = harness(None);
    assert!(h.execute("form.field.signer.set"));
    h.run_steps(2);
    assert!(shown_notice(&h).is_some_and(|n| n.contains("Open the signature field")), "set without the dialog says where to type");
    open_props(&mut h, "Sig1");
    type_email(&mut h, "ann@example.org");
    press(&mut h, "Cancel");
    assert!(!h.state().dialogs.is_open::<FieldPropsDialog>());
    assert!(h.execute("form.field.signer.set"));
    h.run_steps(2);
    assert_eq!(label(&h, "Sig1"), None, "a cancelled dialog leaves no draft for a later command to write");
}

#[test]
fn with_a_plan_beside_the_file_only_a_signer_of_it_can_be_named() {
    let (_dir, mut h) = with_plan();
    open_props(&mut h, "Sig1");
    type_email(&mut h, "carol@example.org");
    press(&mut h, "Set signer");
    assert_eq!(label(&h, "Sig1"), None, "someone who is not in the plan is refused");
    assert!(shown_notice(&h).is_some_and(|n| n.contains("not a signer or an approver")));
    type_email(&mut h, "bob@example.org");
    press(&mut h, "Set signer");
    assert_eq!(label(&h, "Sig1"), None, "a viewer has no turn");
    type_email(&mut h, "ANN@Example.org");
    press(&mut h, "Set signer");
    assert_eq!(label(&h, "Sig1").as_deref(), Some("ann@example.org"), "the label takes the plan's spelling");
}
