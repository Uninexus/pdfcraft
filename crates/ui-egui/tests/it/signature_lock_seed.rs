//! The "When signed" part of a digital signature field's Field Properties: Set the lock, Set seed values and No seed values
//! run the registered commands `form.field.lock.set`, `form.field.seed.set` and `form.field.seed.clear`, each one undo step,
//! refused where the `signature_field_lock` and `signature_seed_values` tools are.

use crate::support::{AppHarness as _, shown_notice};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::{Edit, NewField, SeedEntry, SeedSpec, Session};
use pdfcraft_ui_egui::PdfCraftApp;
use pdfcraft_ui_egui::prepare::FieldPropsDialog;
use pdfcraft_ui_egui::prepare::signed::{LockChoice, SignedDraft};

/// A blank page with a signature field `Sig1` that already has seed values (one reason and a format) and a text field `Name`.
fn fixture() -> Vec<u8> {
    let mut session = Session::new();
    let bytes = session.create_blank(300.0, 400.0, 1).unwrap();
    let id = session.open("lock.pdf", None, bytes, None).unwrap();
    for (name, rect, kind) in
        [("Sig1", [40.0, 290.0, 220.0, 326.0], NewField::Signature), ("Name", [40.0, 230.0, 230.0, 254.0], NewField::Text { multiline: false })]
    {
        session.apply(id, Edit::AddField { name: Some(name.into()), page: 0, rect, kind }).unwrap();
    }
    let field = session.get(id).unwrap().form.iter().find(|f| f.name == "Sig1").unwrap().obj;
    let seed = SeedSpec::new(vec!["Old reason".into()], vec!["adbe.pkcs7.detached".into()], Vec::new(), None, Vec::new()).unwrap();
    session.apply(id, Edit::SetSignatureSeed { field, seed: Some(seed) }).unwrap();
    session.save_bytes(id).unwrap().as_ref().clone()
}

fn harness() -> Harness<'static, PdfCraftApp> {
    let bytes = fixture();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1000.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "lock.pdf", None, bytes).unwrap();
            app
        }
    });
    crate::support::settle(&mut h);
    h
}

fn open_props(h: &mut Harness<'static, PdfCraftApp>, name: &str) {
    h.set_option("dialog", &format!("field-properties:{name}")).unwrap();
    h.run_steps(3);
}

/// Runs a registered command, as the dialog's button does after the pass (the buttons lie low in a tall tab, so a test does
/// not depend on where a click would land).
fn run(h: &mut Harness<'static, PdfCraftApp>, command: &str) {
    assert!(h.execute(command), "{command} is registered and enabled");
    h.run_steps(3);
}

fn undo_depth(h: &Harness<'static, PdfCraftApp>) -> usize {
    let s = h.state();
    s.session.get(s.views[0].id).unwrap().undo_depth()
}

/// What the document holds for `Sig1`: its lock and its seed values.
struct Stored {
    lock_entry: bool,
    lock_names: Vec<String>,
    seed_entry: bool,
    reasons: Vec<String>,
    reason_required: bool,
    formats: usize,
}

fn stored(h: &Harness<'static, PdfCraftApp>) -> Stored {
    let s = h.state();
    let doc = s.session.get(s.views[0].id).unwrap();
    let field = doc.form.iter().find(|f| f.name == "Sig1").unwrap().obj;
    let (lock, seed) = (doc.signature_lock(field), doc.signature_seed(field));
    Stored {
        lock_entry: lock.has_entry,
        lock_names: lock.names,
        seed_entry: seed.has_entry,
        reasons: seed.reasons.items,
        reason_required: seed.required.contains(&SeedEntry::Reasons),
        formats: seed.sub_filters.items.len(),
    }
}

/// What is typed in the dialog, replaced by `typed`.
fn type_signed(h: &mut Harness<'static, PdfCraftApp>, lock: LockChoice, fields: &str, reasons: &str, required: bool) {
    let dialog = h.state_mut().dialogs.get_mut::<FieldPropsDialog>().expect("the dialog is open");
    let view = dialog.signed().view.clone();
    dialog.set_signed(SignedDraft { lock, lock_fields: fields.into(), reasons: reasons.into(), reason_required: required, view });
    h.run_steps(2);
}

#[test]
fn a_lock_is_one_undo_step_and_lock_nothing_takes_it_out() {
    let mut h = harness();
    open_props(&mut h, "Sig1");
    assert!(!stored(&h).lock_entry, "the field starts without a lock");
    let before = undo_depth(&h);
    type_signed(&mut h, LockChoice::All, "", "Old reason", false);
    run(&mut h, "form.field.lock.set");
    assert!(stored(&h).lock_entry, "the lock is in the file");
    assert!(undo_depth(&h) > before, "and it is an undo step");
    assert!(h.query_all_by_label_contains("When signed: all fields are locked").count() > 0, "the window says what the file holds");
    let locked = undo_depth(&h);
    type_signed(&mut h, LockChoice::Nothing, "", "Old reason", false);
    run(&mut h, "form.field.lock.set");
    assert!(!stored(&h).lock_entry, "Lock nothing takes it out");
    assert!(undo_depth(&h) > locked, "as one more step");
    h.undo();
    h.run_steps(3);
    assert!(stored(&h).lock_entry, "undo brings the lock back");
}

#[test]
fn a_lock_on_listed_fields_names_only_fields_the_form_has() {
    let mut h = harness();
    open_props(&mut h, "Sig1");
    let before = undo_depth(&h);
    type_signed(&mut h, LockChoice::Include, "Nowhere", "Old reason", false);
    run(&mut h, "form.field.lock.set");
    assert!(!stored(&h).lock_entry, "a name the form does not have locks nothing");
    assert_eq!(undo_depth(&h), before, "and adds no undo step");
    assert!(shown_notice(&h).is_some_and(|n| n.contains("no field named")), "with the reason");
    type_signed(&mut h, LockChoice::Include, "Name", "Old reason", false);
    run(&mut h, "form.field.lock.set");
    assert_eq!(stored(&h).lock_names, ["Name"]);
    type_signed(&mut h, LockChoice::Include, "", "Old reason", false);
    run(&mut h, "form.field.lock.set");
    assert_eq!(stored(&h).lock_names, ["Name"], "an empty list is refused and the lock stays");
}

#[test]
fn reasons_are_set_and_kept_beside_the_formats_already_there_and_taken_out_with_the_rest() {
    let mut h = harness();
    open_props(&mut h, "Sig1");
    assert!(h.query_all_by_label_contains("Seed values: reasons Old reason").count() > 0, "the window shows what the field holds");
    let before = undo_depth(&h);
    type_signed(&mut h, LockChoice::Nothing, "", "Approved\nI am the author", true);
    run(&mut h, "form.field.seed.set");
    let seed = stored(&h);
    assert_eq!(seed.reasons.len(), 2, "both reasons are in the file");
    assert!(seed.reason_required, "and the signer must pick one");
    assert!(seed.formats > 0, "the format the field already allowed is kept");
    assert!(undo_depth(&h) > before, "one undo step");
    run(&mut h, "form.field.seed.clear");
    assert!(!stored(&h).seed_entry, "No seed values takes them out");
    h.undo();
    h.run_steps(3);
    assert_eq!(stored(&h).reasons.len(), 2, "undo brings them back");
}

#[test]
fn a_text_field_has_no_when_signed_entries_and_the_commands_refuse_it() {
    let mut h = harness();
    open_props(&mut h, "Name");
    assert!(h.query_all_by_role_and_label(Role::Button, "Set the lock").next().is_none());
    let before = undo_depth(&h);
    for command in ["form.field.lock.set", "form.field.seed.set", "form.field.seed.clear"] {
        assert!(h.execute(command));
        h.run_steps(2);
    }
    assert_eq!(undo_depth(&h), before, "a text field takes neither");
}

#[test]
fn the_commands_need_the_open_dialog() {
    let mut h = harness();
    let before = undo_depth(&h);
    assert!(h.execute("form.field.lock.set"));
    h.run_steps(2);
    assert!(shown_notice(&h).is_some_and(|n| n.contains("Open the signature field")), "the refusal says where to go");
    assert_eq!(undo_depth(&h), before);
}
