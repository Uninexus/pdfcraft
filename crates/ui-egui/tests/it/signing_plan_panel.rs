//! Sign ▸ Signing plan: the dialog shows the local plan beside the saved document and the commands
//! `sign.plan.recipient.add`, `sign.plan.recipient.remove` and `sign.plan.message.set` change it.
//! The plan is a record only; these tests check what is written to the file beside the PDF, what is
//! refused, and that a closed dialog leaves the commands with nothing to act on.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use egui_kittest::{
    Harness,
    kittest::{NodeT as _, Queryable},
};
use pdfcraft_engine::Session;
use pdfcraft_engine::signing_plan::{Plan, Role, Status, plan_bytes, read_sidecar, sidecar_of};
use pdfcraft_ui_egui::{Dialog, PdfCraftApp};

fn pdf_bytes() -> Vec<u8> {
    let mut s = Session::new();
    let bytes = s.create_blank(200.0, 200.0, 1).unwrap();
    let id = s.open("a.pdf", None, bytes, None).unwrap();
    s.save_bytes(id).unwrap().to_vec()
}

/// A folder under the system temp directory that no other test uses.
fn folder(tag: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("pdfcraft-plan-panel-{tag}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A document opened from `<dir>/a.pdf` (the file is written there, as a saved document has one), or from no file.
fn harness(path: Option<&PathBuf>) -> Harness<'static, PdfCraftApp> {
    if let Some(p) = path {
        std::fs::write(p, pdf_bytes()).unwrap();
    }
    let path = path.map(|p| p.to_string_lossy().into_owned());
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("a.pdf", path.clone(), pdf_bytes()).unwrap();
        app
    });
    h.run_steps(4);
    h
}

fn sidecar(dir: &std::path::Path) -> PathBuf {
    sidecar_of(&dir.join("a.pdf")).unwrap()
}

fn toast(h: &Harness<'static, PdfCraftApp>) -> String {
    h.state().toast.as_ref().map(|t| t.0.clone()).unwrap_or_default()
}

fn open_dialog(h: &mut Harness<'static, PdfCraftApp>) {
    assert!(h.state_mut().execute("sign.plan.show"));
    h.run_steps(3);
}

fn type_recipient(h: &mut Harness<'static, PdfCraftApp>, name: &str, email: &str, role: Role) {
    let d = h.state_mut().signing_plan.as_mut().expect("the dialog is open");
    (d.name, d.email, d.role) = (name.into(), email.into(), role);
}

fn add(h: &mut Harness<'static, PdfCraftApp>, name: &str, email: &str, role: Role) {
    type_recipient(h, name, email, role);
    assert!(h.state_mut().execute("sign.plan.recipient.add"));
    h.run_steps(2);
}

fn write_plan(dir: &std::path::Path, plan: &Plan) {
    std::fs::write(sidecar(dir), plan_bytes(plan).unwrap()).unwrap();
}

fn two_people(started: bool) -> Plan {
    let mut plan = Plan::empty(100);
    plan.add_recipient("Ana", "ana@example.org", Role::Signer, 100).unwrap();
    plan.add_recipient("Bo", "bo@example.org", Role::Approver, 100).unwrap();
    if started {
        plan.record(0, Status::Signed, 200).unwrap();
    }
    plan
}

fn names_in(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.retain(|n| !n.ends_with(".pdf"));
    names.sort();
    names
}

#[test]
fn signing_plan_panel_shows_the_recipients_and_state_read_from_the_file() {
    let dir = folder("show");
    write_plan(&dir, &two_people(true));
    let mut h = harness(Some(&dir.join("a.pdf")));
    assert_eq!(h.state().dialog, None);
    open_dialog(&mut h);
    assert_eq!(h.state().dialog, Some(Dialog::SigningPlan));
    h.get_by_label("Signing plan");
    h.get_by_label("State: In progress");
    h.get_by_label("Next: Bo (bo@example.org)");
    h.get_by_label("ana@example.org");
    h.get_by_label("Signed");
    // Each control's name starts with its visible label.
    h.get_by_label("Remove: Ana (ana@example.org)");
    h.get_by_label("Add recipient");
    h.get_by_label("Set message");
    h.get_by_label("Close");
    // It started, so it says why the recipients cannot be changed here.
    h.get_by_label_contains("can no longer be changed here");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn signing_plan_panel_add_remove_and_message_write_the_file_beside_the_pdf_and_nothing_else() {
    let dir = folder("write");
    let mut h = harness(Some(&dir.join("a.pdf")));
    open_dialog(&mut h);
    h.get_by_label("There is no signing plan for this document yet. Add a recipient to make one.");
    // A viewer cannot be the first recipient; nothing is written.
    add(&mut h, "Vi", "vi@example.org", Role::Viewer);
    assert!(toast(&h).contains("first recipient must be a signer or an approver"), "{:?}", toast(&h));
    assert!(names_in(&dir).is_empty());
    add(&mut h, " Ana ", "ana@example.org", Role::Signer);
    add(&mut h, "Bo", "bo@example.org", Role::Approver);
    add(&mut h, "Vi", "vi@example.org", Role::Viewer);
    let plan = read_sidecar(&sidecar(&dir)).unwrap().expect("a plan file");
    assert_eq!(plan.recipients.iter().map(|r| (r.name.as_str(), r.order)).collect::<Vec<_>>(), [("Ana", 1), ("Bo", 2), ("Vi", 3)]);
    assert_eq!(names_in(&dir), ["a.pdf.pdfcraft-sign.json"], "only the sidecar beside the PDF: no staging file left");
    // The dialog shows the file, and says what the command did, in its live line.
    h.get_by_label("Remove: Vi (vi@example.org)");
    assert!(h.state().signing_plan.as_ref().unwrap().result.contains("Vi was added"));
    // The same address in another case, and an address that is not one, change nothing.
    let before = std::fs::read(sidecar(&dir)).unwrap();
    add(&mut h, "Again", "ANA@example.org", Role::Signer);
    assert!(toast(&h).contains("already in the plan"), "{:?}", toast(&h));
    add(&mut h, "Bad", "not an address", Role::Signer);
    assert_eq!(std::fs::read(sidecar(&dir)).unwrap(), before);
    // Remove takes the recipient a Remove button chose, and numbers the rest again.
    h.state_mut().signing_plan.as_mut().unwrap().remove = Some("BO@example.org".into());
    assert!(h.state_mut().execute("sign.plan.recipient.remove"));
    h.run_steps(2);
    let plan = read_sidecar(&sidecar(&dir)).unwrap().unwrap();
    assert_eq!(plan.recipients.iter().map(|r| (r.name.as_str(), r.order)).collect::<Vec<_>>(), [("Ana", 1), ("Vi", 2)]);
    // Remove with nothing chosen is refused.
    assert!(h.state_mut().execute("sign.plan.recipient.remove"));
    assert!(toast(&h).contains("Choose a recipient to remove first"), "{:?}", toast(&h));
    // The last signer cannot be removed, which would leave a plan nobody can act on.
    h.state_mut().signing_plan.as_mut().unwrap().remove = Some("ana@example.org".into());
    h.state_mut().execute("sign.plan.recipient.remove");
    assert_eq!(read_sidecar(&sidecar(&dir)).unwrap().unwrap().recipients.len(), 2);
    assert!(toast(&h).contains("at least one signer or approver"), "{:?}", toast(&h));
    // The message.
    h.state_mut().signing_plan.as_mut().unwrap().message = "Please sign by Friday".into();
    assert!(h.state_mut().execute("sign.plan.message.set"));
    assert_eq!(read_sidecar(&sidecar(&dir)).unwrap().unwrap().message, "Please sign by Friday");
    h.state_mut().execute("sign.plan.message.set");
    assert!(toast(&h).contains("already has that message"), "{:?}", toast(&h));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn signing_plan_panel_buttons_run_the_commands() {
    let dir = folder("buttons");
    let mut h = harness(Some(&dir.join("a.pdf")));
    open_dialog(&mut h);
    type_recipient(&mut h, "Ana", "ana@example.org", Role::Signer);
    h.run_steps(2);
    h.get_by_label("Add recipient").click();
    h.run_steps(4);
    assert_eq!(read_sidecar(&sidecar(&dir)).unwrap().unwrap().recipients.len(), 1);
    h.get_by_label("Remove: Ana (ana@example.org)");
    // Taking the only signer out is refused by the rules, through the Remove button.
    h.get_by_label("Remove: Ana (ana@example.org)").click();
    h.run_steps(4);
    assert_eq!(read_sidecar(&sidecar(&dir)).unwrap().unwrap().recipients.len(), 1);
    assert!(toast(&h).contains("at least one signer or approver"), "{:?}", toast(&h));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn signing_plan_panel_refuses_every_edit_once_a_recipient_has_a_status() {
    let dir = folder("started");
    write_plan(&dir, &two_people(true));
    let before = std::fs::read(sidecar(&dir)).unwrap();
    let mut h = harness(Some(&dir.join("a.pdf")));
    open_dialog(&mut h);
    type_recipient(&mut h, "Cy", "cy@example.org", Role::Signer);
    h.state_mut().signing_plan.as_mut().unwrap().message = "changed".into();
    for command in ["sign.plan.recipient.add", "sign.plan.message.set"] {
        assert!(h.state_mut().execute(command));
        assert!(toast(&h).contains("can no longer be edited"), "{command}: {:?}", toast(&h));
        assert!(h.state().signing_plan.as_ref().unwrap().result.contains("can no longer be edited"));
    }
    h.state_mut().signing_plan.as_mut().unwrap().remove = Some("bo@example.org".into());
    assert!(h.state_mut().execute("sign.plan.recipient.remove"));
    assert!(toast(&h).contains("can no longer be edited"), "{:?}", toast(&h));
    assert_eq!(std::fs::read(sidecar(&dir)).unwrap(), before, "the file is as it was");
    // The plan still shows, and a started plan has its Remove buttons switched off.
    assert!(h.get_by_label("Remove: Bo (bo@example.org)").accesskit_node().is_disabled());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn signing_plan_panel_refuses_a_document_with_no_saved_file() {
    let mut h = harness(None);
    open_dialog(&mut h);
    h.get_by_label_contains("The document has no saved file");
    type_recipient(&mut h, "Ana", "ana@example.org", Role::Signer);
    h.state_mut().signing_plan.as_mut().unwrap().message = "hello".into();
    for command in ["sign.plan.recipient.add", "sign.plan.message.set", "sign.plan.recipient.remove"] {
        assert!(h.state_mut().execute(command));
        assert!(toast(&h).contains("has no saved file"), "{command}: {:?}", toast(&h));
    }
    assert!(h.query_by_label("Add recipient").is_none(), "no controls are offered when nothing can be written");
}

#[test]
fn signing_plan_panel_a_closed_dialog_leaves_the_commands_refusing() {
    let dir = folder("closed");
    let mut h = harness(Some(&dir.join("a.pdf")));
    open_dialog(&mut h);
    type_recipient(&mut h, "Ana", "ana@example.org", Role::Signer);
    // Escape closes the dialog and discards what was typed.
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().dialog, None);
    assert!(h.state().signing_plan.is_none(), "the draft goes with the dialog");
    for command in ["sign.plan.recipient.add", "sign.plan.message.set", "sign.plan.recipient.remove"] {
        assert!(h.state_mut().execute(command));
        assert!(toast(&h).contains("Open the signing plan first"), "{command}: {:?}", toast(&h));
    }
    assert!(names_in(&dir).is_empty());
    // The Close button does the same.
    open_dialog(&mut h);
    h.get_by_label("Close").click();
    h.run_steps(3);
    assert!(h.state().signing_plan.is_none() && h.state().dialog.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn signing_plan_panel_a_draft_of_another_document_is_not_acted_on() {
    let dir = folder("other");
    let mut h = harness(Some(&dir.join("a.pdf")));
    open_dialog(&mut h);
    type_recipient(&mut h, "Ana", "ana@example.org", Role::Signer);
    // Another document comes to the front while the draft is still there.
    let other = folder("other-b");
    h.state_mut().open_bytes("b.pdf", Some(other.join("b.pdf").to_string_lossy().into_owned()), pdf_bytes()).unwrap();
    h.state_mut().dialog = Some(Dialog::SigningPlan);
    assert!(h.state_mut().execute("sign.plan.recipient.add"));
    assert!(toast(&h).contains("Open the signing plan first"), "{:?}", toast(&h));
    assert!(names_in(&dir).is_empty() && names_in(&other).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&other);
}

#[test]
fn signing_plan_panel_a_file_that_is_not_a_plan_is_left_alone_and_its_text_is_cleaned() {
    let dir = folder("unreadable");
    // An unknown field name with a direction-changing character comes back in the parser's refusal.
    let bad = "{\"version\":1,\"message\":\"\",\"created\":0,\"recipients\":[],\"ev\u{202E}il\":1}";
    std::fs::write(sidecar(&dir), bad).unwrap();
    let mut h = harness(Some(&dir.join("a.pdf")));
    open_dialog(&mut h);
    let shown = match &h.state().signing_plan.as_ref().unwrap().view {
        pdfcraft_ui_egui::SigningPlanView::Unreadable(why) => why.clone(),
        other => panic!("{other:?}"),
    };
    assert!(shown.contains("not a signing plan") && !shown.contains('\u{202E}'), "{shown:?}");
    h.get_by_label("The plan file beside this document cannot be used, and was left alone:");
    // Nothing can be added over it.
    type_recipient(&mut h, "Ana", "ana@example.org", Role::Signer);
    assert!(h.state_mut().execute("sign.plan.recipient.add"));
    assert_eq!(std::fs::read_to_string(sidecar(&dir)).unwrap(), bad);
    assert!(!toast(&h).contains('\u{202E}'));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn signing_plan_panel_a_plan_started_elsewhere_is_refused_and_then_shown_as_started() {
    let dir = folder("elsewhere");
    write_plan(&dir, &two_people(false));
    let mut h = harness(Some(&dir.join("a.pdf")));
    open_dialog(&mut h);
    h.run_steps(2);
    assert!(!h.get_by_label("Remove: Bo (bo@example.org)").accesskit_node().is_disabled(), "an unstarted plan can be edited");
    // Another program records a signature while the dialog is open.
    write_plan(&dir, &two_people(true));
    let before = std::fs::read(sidecar(&dir)).unwrap();
    type_recipient(&mut h, "Cy", "cy@example.org", Role::Signer);
    assert!(h.state_mut().execute("sign.plan.recipient.add"));
    h.run_steps(3);
    assert!(toast(&h).contains("can no longer be edited"), "{:?}", toast(&h));
    assert_eq!(std::fs::read(sidecar(&dir)).unwrap(), before, "nothing was written");
    assert!(h.get_by_label("Remove: Bo (bo@example.org)").accesskit_node().is_disabled(), "the dialog now shows the started plan");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn signing_plan_panel_refuses_when_the_pdf_itself_is_gone() {
    let dir = folder("gone");
    let mut h = harness(Some(&dir.join("a.pdf")));
    std::fs::remove_file(dir.join("a.pdf")).unwrap();
    open_dialog(&mut h);
    type_recipient(&mut h, "Ana", "ana@example.org", Role::Signer);
    assert!(h.state_mut().execute("sign.plan.recipient.add"));
    assert!(toast(&h).contains("not there"), "{:?}", toast(&h));
    assert!(names_in(&dir).is_empty(), "no plan file was made for a document that is not there");
    let _ = std::fs::remove_dir_all(&dir);
}
