//! The Signer control in a signature field's properties: it sets or clears the label
//! `/PdfCraftSigner` through the registered commands `form.field.signer.set` and
//! `form.field.signer.clear` (the edit `form_field_signer` makes), one undo step each. The label
//! restricts nobody; these tests check that it is stored, undone, and refused where it must be.

use egui_kittest::{Harness, kittest::Queryable};
use pdfcraft_engine::{Edit, NewField, Session};
use pdfcraft_ui_egui::PdfCraftApp;

/// A page with a text field `name` and a signature field `Sig1`.
fn fixture() -> Vec<u8> {
    let mut s = Session::new();
    let bytes = s.create_blank(300.0, 400.0, 1).unwrap();
    let id = s.open("form.pdf", None, bytes, None).unwrap();
    for (name, rect, kind) in
        [("name", [40.0, 290.0, 230.0, 314.0], NewField::Text { multiline: false }), ("Sig1", [40.0, 200.0, 230.0, 240.0], NewField::Signature)]
    {
        s.apply(id, Edit::AddField { name: Some(name.into()), page: 0, rect, kind }).unwrap();
    }
    s.save_bytes(id).unwrap().to_vec()
}

fn harness() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("form.pdf", None, fixture()).unwrap();
        app
    });
    h.run_steps(4);
    h
}

/// The signer label of the named field as the document holds it now.
fn label(h: &Harness<'static, PdfCraftApp>, name: &str) -> Option<String> {
    let s = h.state();
    let doc = s.session.get(s.active_ids().unwrap().1).unwrap();
    let obj = doc.form.iter().find(|f| f.name == name).unwrap().obj;
    doc.field_signer(obj).email
}

fn undo_label(h: &Harness<'static, PdfCraftApp>) -> Option<String> {
    let s = h.state();
    s.session.get(s.active_ids().unwrap().1).unwrap().can_undo().map(str::to_string)
}

fn undo_depth(h: &Harness<'static, PdfCraftApp>) -> usize {
    let s = h.state();
    s.session.get(s.active_ids().unwrap().1).unwrap().undo_depth()
}

fn open_properties(h: &mut Harness<'static, PdfCraftApp>, field: &str) {
    h.state_mut().open_field_props(field, 0);
    h.run_steps(3);
}

fn type_signer(h: &mut Harness<'static, PdfCraftApp>, text: &str) {
    h.state_mut().field_props.as_mut().expect("properties are open").signer = text.into();
}

fn toast(h: &Harness<'static, PdfCraftApp>) -> String {
    h.state().toast.as_ref().map(|t| t.0.clone()).unwrap_or_default()
}

#[test]
fn the_set_command_labels_the_field_in_one_undoable_step() {
    let mut h = harness();
    open_properties(&mut h, "Sig1");
    h.get_by_label("Digital Signature Properties");
    assert_eq!(label(&h, "Sig1"), None);
    type_signer(&mut h, "  Ana@Example.org ");
    assert!(h.state_mut().execute("form.field.signer.set"));
    h.run_steps(2);
    assert_eq!(label(&h, "Sig1").as_deref(), Some("Ana@Example.org"), "stored trimmed, as typed otherwise");
    assert_eq!(undo_label(&h).as_deref(), Some("Set field signer"));
    assert!(toast(&h).contains("does not restrict who can sign"), "{:?}", toast(&h));
    // The dialog now says who the field is for.
    h.get_by_label("Signer: Ana@Example.org");
    h.state_mut().undo();
    h.run_steps(2);
    assert_eq!(label(&h, "Sig1"), None, "one undo takes the label out");
    h.get_by_label("Signer: No one");
    h.state_mut().redo();
    h.run_steps(2);
    h.get_by_label("Signer: Ana@Example.org");
}

#[test]
fn the_clear_command_takes_the_label_out_and_undo_brings_it_back() {
    let mut h = harness();
    open_properties(&mut h, "Sig1");
    type_signer(&mut h, "bo@example.org");
    assert!(h.state_mut().execute("form.field.signer.set"));
    assert_eq!(label(&h, "Sig1").as_deref(), Some("bo@example.org"));
    assert!(h.state_mut().execute("form.field.signer.clear"));
    h.run_steps(2);
    assert_eq!(label(&h, "Sig1"), None);
    assert_eq!(h.state().field_props.as_ref().unwrap().signer_view.label, None);
    h.state_mut().undo();
    assert_eq!(label(&h, "Sig1").as_deref(), Some("bo@example.org"));
}

#[test]
fn the_dialog_buttons_run_the_commands() {
    let mut h = harness();
    open_properties(&mut h, "Sig1");
    type_signer(&mut h, "cy@example.org");
    h.get_by_label("Set signer").click();
    h.run_steps(3);
    assert_eq!(label(&h, "Sig1").as_deref(), Some("cy@example.org"));
    // The notice is the command's own: a button that wrote the edit itself would not give it.
    assert!(toast(&h).starts_with("Signer set to cy@example.org"), "{:?}", toast(&h));
    h.get_by_label("No one (clear signer)").click();
    h.run_steps(3);
    assert_eq!(label(&h, "Sig1"), None);
}

#[test]
fn an_address_that_is_not_one_changes_nothing() {
    let mut h = harness();
    open_properties(&mut h, "Sig1");
    let before = undo_label(&h);
    // The engine refuses a bad address for every caller, not only this entry.
    let obj = {
        let s = h.state();
        s.session.get(s.active_ids().unwrap().1).unwrap().form.iter().find(|f| f.name == "Sig1").unwrap().obj
    };
    assert!(!h.state_mut().apply_edit(Edit::SetFieldSigner { field: obj, email: Some("ana\u{00ad}@example.org".into()) }));
    assert_eq!(label(&h, "Sig1"), None);
    let long_local = format!("{}@example.org", "a".repeat(65));
    type_signer(&mut h, &long_local);
    h.state_mut().execute("form.field.signer.set");
    assert_eq!(label(&h, "Sig1"), None, "a 65-byte local part");
    assert!(toast(&h).contains("at most 254 bytes"), "{:?}", toast(&h));
    for bad in [
        "",
        "no-at-sign",
        "a@b",
        "a b@example.org",
        "a@@example.org",
        "a@example..org",
        "a@example.org\u{202e}",
        "ana\u{00ad}@example.org",
        "ana\u{e0041}@example.org",
    ] {
        type_signer(&mut h, bad);
        h.state_mut().execute("form.field.signer.set");
        assert_eq!(label(&h, "Sig1"), None, "{bad:?}");
        assert_eq!(undo_label(&h), before, "{bad:?} made no edit");
        assert!(toast(&h).contains("must look like"), "{bad:?}: {:?}", toast(&h));
    }
}

#[test]
fn a_text_field_gets_no_signer() {
    let mut h = harness();
    open_properties(&mut h, "name");
    assert!(h.query_by_label("Set signer").is_none(), "no Signer control on a text field");
    type_signer(&mut h, "ana@example.org");
    h.state_mut().execute("form.field.signer.set");
    assert_eq!(label(&h, "name"), None);
    assert!(toast(&h).contains("only be set on a signature field"), "{:?}", toast(&h));
}

#[test]
fn setting_the_same_signer_again_is_said_and_adds_no_step() {
    let mut h = harness();
    open_properties(&mut h, "Sig1");
    type_signer(&mut h, "ana@example.org");
    h.state_mut().execute("form.field.signer.set");
    let depth = {
        let s = h.state();
        s.session.get(s.active_ids().unwrap().1).unwrap().undo_depth()
    };
    h.state_mut().execute("form.field.signer.set");
    assert!(toast(&h).contains("already has that signer"), "{:?}", toast(&h));
    let s = h.state();
    assert_eq!(s.session.get(s.active_ids().unwrap().1).unwrap().undo_depth(), depth);
}

/// A dialog that was cancelled is not open: the commands neither write what was typed in it nor
/// act on the field it was open on.
#[test]
fn a_cancelled_dialog_leaves_the_commands_with_nothing_to_act_on() {
    let mut h = harness();
    open_properties(&mut h, "Sig1");
    type_signer(&mut h, "ana@example.org");
    assert!(h.state_mut().execute("form.field.signer.set"));
    let steps = undo_depth(&h);
    type_signer(&mut h, "bo@example.org");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().field_props.is_none(), "the draft goes with the dialog");
    h.state_mut().execute("form.field.signer.set");
    assert!(toast(&h).contains("Open the signature field's properties"), "{:?}", toast(&h));
    assert_eq!(label(&h, "Sig1").as_deref(), Some("ana@example.org"), "the typed address was not written");
    // Clear acts on the selection, and nothing is selected: the old field keeps its label.
    h.state_mut().execute("form.field.signer.clear");
    assert!(toast(&h).contains("Select a signature field first"), "{:?}", toast(&h));
    assert_eq!(label(&h, "Sig1").as_deref(), Some("ana@example.org"));
    assert_eq!(undo_depth(&h), steps);
}

/// A dialog opened on one document does not label a field of the same name in another.
#[test]
fn a_dialog_of_another_document_is_not_acted_on() {
    let mut h = harness();
    open_properties(&mut h, "Sig1");
    type_signer(&mut h, "ana@example.org");
    h.state_mut().open_bytes("other.pdf", None, fixture()).unwrap();
    h.run_steps(3);
    h.state_mut().execute("form.field.signer.set");
    assert_eq!(label(&h, "Sig1"), None, "the other document's Sig1 is untouched");
    assert!(h.state().field_props.is_none());
    assert!(toast(&h).contains("Open the signature field's properties"), "{:?}", toast(&h));
}

/// A plan file beside the document is said in the dialog and in the notice: the entry is not
/// checked against it.
#[test]
fn a_plan_file_beside_the_document_is_said() {
    let dir = std::env::temp_dir().join(format!("pdfcraft-signer-plan-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("form.pdf");
    std::fs::write(&path, fixture()).unwrap();
    std::fs::write(dir.join("form.pdf.pdfcraft-sign.json"), b"{}").unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("form.pdf", Some(path.to_string_lossy().into_owned()), fixture()).unwrap();
        app
    });
    h.run_steps(4);
    open_properties(&mut h, "Sig1");
    h.get_by_label_contains("not checked against it");
    type_signer(&mut h, "ana@example.org");
    h.state_mut().execute("form.field.signer.set");
    assert!(toast(&h).contains("not checked against the signing plan beside this file"), "{:?}", toast(&h));
    drop(h);
    let _ = std::fs::remove_dir_all(&dir);
}
