//! Sign > Signing plan: the window shows the local plan beside the open document and adds or removes a recipient and sets
//! the message through registered commands. The plan is a file beside the PDF; the PDF is never touched.

use std::path::{Path, PathBuf};

use crate::support::{AppHarness as _, shown_notice};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::Session;
use pdfcraft_engine::signing_plan::{Plan, Role as PlanRole, Status, plan_bytes, read_sidecar, sidecar_of};
use pdfcraft_ui_egui::PdfCraftApp;

fn fixture() -> Vec<u8> {
    let session = Session::new();
    let bytes = session.create_blank(300.0, 400.0, 1).unwrap();
    bytes.as_ref().clone()
}

fn harness(path: Option<String>) -> Harness<'static, PdfCraftApp> {
    let bytes = fixture();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1000.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "contract.pdf", path, bytes).unwrap();
            app
        }
    });
    crate::support::settle(&mut h);
    h
}

/// A saved document in its own folder, and the window open on it.
fn saved() -> (crate::support::TempDir, PathBuf, Harness<'static, PdfCraftApp>) {
    let dir = crate::support::TempDir::new();
    let pdf = dir.path().join("contract.pdf");
    std::fs::write(&pdf, fixture()).unwrap();
    let h = harness(Some(pdf.to_string_lossy().into_owned()));
    (dir, pdf, h)
}

fn open_window(h: &mut Harness<'static, PdfCraftApp>) {
    assert!(h.execute("sign.plan.show"));
    h.run_steps(4);
    assert!(h.state().signing_plan.is_some());
}

fn plan_at(pdf: &Path) -> Option<Plan> {
    read_sidecar(&sidecar_of(pdf).unwrap()).unwrap()
}

fn type_into(h: &mut Harness<'static, PdfCraftApp>, name: &str, email: &str, role: PlanRole) {
    let d = h.state_mut().signing_plan.as_mut().expect("the window is open");
    d.name = name.into();
    d.email = email.into();
    d.role = role;
    h.run_steps(2);
}

fn run(h: &mut Harness<'static, PdfCraftApp>, command: &str) {
    assert!(h.execute(command), "{command} is registered and enabled with a document");
    h.run_steps(3);
}

fn result(h: &Harness<'static, PdfCraftApp>) -> String {
    h.state().signing_plan.as_ref().map(|d| d.result.clone()).unwrap_or_default()
}

/// Everything in the folder, by name.
fn files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

#[test]
fn recipients_and_a_message_are_written_beside_the_pdf_and_the_pdf_is_left_alone() {
    let (dir, pdf, mut h) = saved();
    let before = std::fs::read(&pdf).unwrap();
    open_window(&mut h);
    type_into(&mut h, "Ann Author", "ann@example.org", PlanRole::Signer);
    run(&mut h, "sign.plan.recipient.add");
    type_into(&mut h, "Bob Boss", "bob@example.org", PlanRole::Approver);
    run(&mut h, "sign.plan.recipient.add");
    h.state_mut().signing_plan.as_mut().unwrap().message = "Please sign by Friday".into();
    run(&mut h, "sign.plan.message.set");
    let plan = plan_at(&pdf).expect("the first recipient makes the plan");
    assert_eq!(plan.recipients.iter().map(|r| r.email.as_str()).collect::<Vec<_>>(), ["ann@example.org", "bob@example.org"]);
    assert_eq!(plan.recipients[1].role, PlanRole::Approver);
    assert_eq!(plan.message, "Please sign by Friday");
    assert_eq!(std::fs::read(&pdf).unwrap(), before, "the PDF is not touched");
    assert_eq!(files(dir.path()), ["contract.pdf", "contract.pdf.pdfcraft-sign.json"], "only the plan file was made");
    // The window shows the file as it is now.
    h.run_steps(3);
    assert!(h.query_all_by_label_contains("Bob Boss").count() > 0, "the recipient is listed");
    // Removing one renumbers the rest.
    h.state_mut().signing_plan.as_mut().unwrap().remove = Some("ann@example.org".into());
    run(&mut h, "sign.plan.recipient.remove");
    let plan = plan_at(&pdf).unwrap();
    assert_eq!(plan.recipients.len(), 1);
    assert_eq!(plan.recipients[0].order, 1, "the rest are numbered again");
}

#[test]
fn the_buttons_run_the_commands() {
    let (_dir, pdf, mut h) = saved();
    open_window(&mut h);
    type_into(&mut h, "Ann Author", "ann@example.org", PlanRole::Signer);
    h.get_by_role_and_label(Role::Button, "Add recipient").click();
    h.run_steps(4);
    assert_eq!(plan_at(&pdf).map(|p| p.recipients.len()), Some(1), "Add recipient wrote the plan");
    assert!(shown_notice(&h).is_some_and(|n| n.contains("was added to the signing plan")));
    type_into(&mut h, "Bob Boss", "bob@example.org", PlanRole::Signer);
    h.get_by_role_and_label(Role::Button, "Add recipient").click();
    h.run_steps(4);
    h.query_all_by_label_contains("Remove Ann").next().expect("a Remove button names the recipient").click();
    h.run_steps(4);
    let plan = plan_at(&pdf).unwrap();
    assert_eq!(plan.recipients.iter().map(|r| r.email.as_str()).collect::<Vec<_>>(), ["bob@example.org"], "Remove took Ann out");
}

#[test]
fn bad_input_changes_nothing_and_says_why() {
    let (_dir, pdf, mut h) = saved();
    open_window(&mut h);
    type_into(&mut h, "Vic Viewer", "vic@example.org", PlanRole::Viewer);
    run(&mut h, "sign.plan.recipient.add");
    assert!(plan_at(&pdf).is_none(), "a viewer cannot be the first recipient");
    assert!(!result(&h).is_empty() && !h.state().signing_plan.as_ref().unwrap().result_ok);
    type_into(&mut h, "Ann Author", "ann@example.org", PlanRole::Signer);
    run(&mut h, "sign.plan.recipient.add");
    let written = std::fs::read(sidecar_of(&pdf).unwrap()).unwrap();
    for (name, email) in [("Ann Again", "ANN@example.org"), ("Bad", "not-an-address"), ("", "x@example.org")] {
        type_into(&mut h, name, email, PlanRole::Signer);
        run(&mut h, "sign.plan.recipient.add");
        assert_eq!(std::fs::read(sidecar_of(&pdf).unwrap()).unwrap(), written, "{email:?} changed nothing");
    }
    h.state_mut().signing_plan.as_mut().unwrap().remove = None;
    run(&mut h, "sign.plan.recipient.remove");
    h.state_mut().signing_plan.as_mut().unwrap().remove = Some("ann@example.org".into());
    run(&mut h, "sign.plan.recipient.remove");
    assert_eq!(std::fs::read(sidecar_of(&pdf).unwrap()).unwrap(), written, "the last signer cannot be taken out");
}

#[test]
fn a_plan_that_has_started_cannot_be_edited_and_offers_no_remove() {
    let (_dir, pdf, mut h) = saved();
    let mut plan = Plan::empty(1);
    plan.add_recipient("Ann Author", "ann@example.org", PlanRole::Signer, 1).unwrap();
    plan.add_recipient("Bob Boss", "bob@example.org", PlanRole::Signer, 1).unwrap();
    plan.record(0, Status::Signed, 2).unwrap();
    std::fs::write(sidecar_of(&pdf).unwrap(), plan_bytes(&plan).unwrap()).unwrap();
    let written = std::fs::read(sidecar_of(&pdf).unwrap()).unwrap();
    open_window(&mut h);
    assert!(h.query_all_by_label_contains("Remove Ann").next().is_none(), "no Remove once a recipient has a status");
    type_into(&mut h, "Cy", "cy@example.org", PlanRole::Signer);
    run(&mut h, "sign.plan.recipient.add");
    h.state_mut().signing_plan.as_mut().unwrap().remove = Some("bob@example.org".into());
    run(&mut h, "sign.plan.recipient.remove");
    h.state_mut().signing_plan.as_mut().unwrap().message = "later".into();
    run(&mut h, "sign.plan.message.set");
    assert_eq!(std::fs::read(sidecar_of(&pdf).unwrap()).unwrap(), written, "every edit is refused and the file is as it was");
}

#[test]
fn a_document_with_no_saved_file_is_refused_and_a_closed_window_discards_the_draft() {
    let mut h = harness(None);
    open_window(&mut h);
    type_into(&mut h, "Ann Author", "ann@example.org", PlanRole::Signer);
    run(&mut h, "sign.plan.recipient.add");
    assert!(result(&h).contains("no saved file"), "{}", result(&h));
    h.query_all_by_role_and_label(Role::Button, "Close").last().expect("the window's Close").click();
    h.run_steps(3);
    assert!(h.state().signing_plan.is_none(), "Close discards the draft");
    run(&mut h, "sign.plan.recipient.add");
    assert!(shown_notice(&h).is_some_and(|n| n.contains("Open the signing plan first")), "with no window the commands do nothing");
}

#[test]
fn a_file_that_is_not_a_plan_is_shown_as_unusable_and_left_alone() {
    let (_dir, pdf, mut h) = saved();
    let junk = b"this is not a plan".to_vec();
    std::fs::write(sidecar_of(&pdf).unwrap(), &junk).unwrap();
    open_window(&mut h);
    type_into(&mut h, "Ann Author", "ann@example.org", PlanRole::Signer);
    run(&mut h, "sign.plan.recipient.add");
    assert_eq!(std::fs::read(sidecar_of(&pdf).unwrap()).unwrap(), junk, "it is not written over");
    assert!(h.query_all_by_label_contains("cannot be used").count() > 0, "and the window says so");
}

#[test]
fn a_recorded_step_is_asked_for_first_and_then_locks_the_plan() {
    let (_dir, pdf, mut h) = saved();
    open_window(&mut h);
    type_into(&mut h, "Ann Author", "ann@example.org", PlanRole::Signer);
    run(&mut h, "sign.plan.recipient.add");
    type_into(&mut h, "Bob Boss", "bob@example.org", PlanRole::Approver);
    run(&mut h, "sign.plan.recipient.add");
    h.get_by_role_and_label(Role::Button, "Record Ann Author as signed").click();
    h.run_steps(3);
    assert_eq!(plan_at(&pdf).unwrap().recipients[0].status, Status::Pending, "asking is not recording");
    assert!(h.query_all_by_label_contains("cannot be taken back").count() > 0, "the window warns first");
    h.get_by_role_and_label(Role::Button, "Keep the plan as it is").click();
    h.run_steps(3);
    assert_eq!(plan_at(&pdf).unwrap().recipients[0].status, Status::Pending);
    h.get_by_role_and_label(Role::Button, "Record Ann Author as signed").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Record it").click();
    h.run_steps(4);
    let plan = plan_at(&pdf).unwrap();
    assert_eq!(plan.recipients[0].status, Status::Signed);
    assert_eq!(plan.next().map(|r| r.email.as_str()), Some("bob@example.org"), "the approver is next");
    // The plan is a record that has started: it can no longer be edited.
    let written = std::fs::read(sidecar_of(&pdf).unwrap()).unwrap();
    type_into(&mut h, "Cy", "cy@example.org", PlanRole::Signer);
    run(&mut h, "sign.plan.recipient.add");
    assert_eq!(std::fs::read(sidecar_of(&pdf).unwrap()).unwrap(), written);
    // The approver's turn is approved, not signed, and a decline finishes the plan.
    run(&mut h, "sign.plan.decline");
    let plan = plan_at(&pdf).unwrap();
    assert_eq!(plan.recipients[1].status, Status::Declined);
    assert_eq!(plan.state(), "declined");
    run(&mut h, "sign.plan.record");
    assert!(!h.state().signing_plan.as_ref().unwrap().result_ok, "nothing is left to record once the plan is declined");
}

#[test]
fn signing_as_the_next_recipient_opens_signing_on_the_field_labelled_for_them() {
    use pdfcraft_engine::{Edit, NewField};
    let dir = crate::support::TempDir::new();
    let pdf = dir.path().join("contract.pdf");
    let mut session = Session::new();
    let bytes = session.create_blank(300.0, 400.0, 1).unwrap();
    let id = session.open("contract.pdf", None, bytes, None).unwrap();
    session.apply(id, Edit::AddField { name: Some("Sig1".into()), page: 0, rect: [40.0, 290.0, 220.0, 326.0], kind: NewField::Signature }).unwrap();
    let obj = session.get(id).unwrap().form[0].obj;
    session.apply(id, Edit::SetFieldSigner { field: obj, email: Some("ann@example.org".into()) }).unwrap();
    std::fs::write(&pdf, session.save_bytes(id).unwrap().as_ref()).unwrap();
    let mut plan = Plan::empty(1);
    plan.add_recipient("Ann Author", "ann@example.org", PlanRole::Signer, 1).unwrap();
    std::fs::write(sidecar_of(&pdf).unwrap(), plan_bytes(&plan).unwrap()).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1000.0)).build_eframe({
        let (path, bytes) = (pdf.to_string_lossy().into_owned(), std::fs::read(&pdf).unwrap());
        move |cc| {
            crate::support::install_runtime(&cc.egui_ctx);
            {
                let mut app = crate::control::new_app();
                app.set_option(&egui::Context::default(), "language", "en").unwrap();
                app.open_bytes(&egui::Context::default(), "contract.pdf", Some(path), bytes).unwrap();
                app
            }
        }
    });
    crate::support::settle(&mut h);
    assert!(h.execute("sign.plan.sign_next"));
    h.run_steps(3);
    let draft = h.state().sign_draft.as_ref().expect("Sign with a Digital ID is open");
    assert_eq!(draft.field.as_deref(), Some("Sig1"), "on the field labelled for the recipient");
}
