//! `signing_plan_audit`: the local signing plan set against what the saved file shows. The plan is
//! a record; the file is the evidence, read by the validator (`sign_list`, `signature_audit_report`).
//! The plans are made with `signing_plan_set` and `signing_plan_sign`, so the file really holds the
//! signatures the plan names. All people are made up and at example.org, and the digital IDs are
//! self-signed test keys made here.

use std::path::Path;

use pdfcraft_automation::file_access::AutomationRoots;
use pdfcraft_automation::{Automation, Content, ToolError};
use serde_json::{Value, json};

fn tool(a: &mut Automation, name: &str, args: Value) -> Value {
    match a.call(name, &args).unwrap_or_else(|e| panic!("{name} failed: {e:?}")).remove(0) {
        Content::Json(value) => value,
        other => panic!("expected JSON: {other:?}"),
    }
}

fn refused(a: &mut Automation, name: &str, args: Value) -> String {
    match a.call(name, &args) {
        Err(ToolError::InvalidArgs(m) | ToolError::Failed(m)) => m,
        other => panic!("not refused: {other:?}"),
    }
}

struct Rig {
    dir: tempfile::TempDir,
    a: Automation,
    doc: u64,
}

impl Rig {
    fn path(&self, name: &str) -> std::path::PathBuf {
        self.dir.path().join(name)
    }

    fn sign(&mut self, email: &str, id: &str) {
        tool(&mut self.a, "signing_plan_sign", json!({ "doc": self.doc, "email": email, "id": id, "password": "secret12" }));
    }

    /// The audit of `form.pdf` as it is on disk now, through a freshly opened document.
    fn audit(&mut self) -> Value {
        let doc = tool(&mut self.a, "doc_open", json!({ "path": "form.pdf" }))["doc"].as_u64().unwrap();
        tool(&mut self.a, "signing_plan_audit", json!({ "doc": doc }))
    }

    /// Sign `field` of `form.pdf` with `id` as a signature the plan knows nothing about (the file
    /// is signed to a new name, which then replaces `form.pdf`).
    fn sign_outside_the_plan(&mut self, field: &str, id: &str) {
        let doc = tool(&mut self.a, "doc_open", json!({ "path": "form.pdf" }))["doc"].as_u64().unwrap();
        tool(&mut self.a, "sign_document", json!({ "doc": doc, "id": id, "password": "secret12", "field": field, "out": "stray.pdf" }));
        std::fs::rename(self.path("stray.pdf"), self.path("form.pdf")).unwrap();
    }

    fn trust(&mut self) {
        tool(&mut self.a, "sign_trust", json!({ "paths": ["ana.p12", "bo.p12"], "password": "secret12" }));
    }
}

fn check<'v>(report: &'v Value, email: &str) -> &'v Value {
    report["recipients"].as_array().unwrap().iter().find(|r| r["recipient"]["email"] == email).unwrap_or_else(|| panic!("{email}: {report}"))
}

fn how(report: &Value, email: &str) -> String {
    check(report, email)["cross_check"].as_str().unwrap().to_string()
}

/// A saved form `form.pdf` with signature fields `SigAna` (labelled for Ana), `SigBo` (labelled for
/// Bo) and `Extra` (no label), a plan of Ana and Bo (signers) and Cy (an approver), and an ID file
/// for each of Ana and Bo.
fn rig() -> Rig {
    rig_with(None)
}

/// [`rig`], with one more unlabelled signature field added before the others when `first` is given, so
/// that a field of that name comes first in the form.
fn rig_with(first: Option<&str>) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let mut session = pdfcraft_engine::Session::new();
    let bytes = session.create_blank(300.0, 400.0, 1).unwrap();
    let id = session.open("a.pdf", None, bytes, None).unwrap();
    std::fs::write(dir.path().join("a.pdf"), session.save_bytes(id).unwrap().as_slice()).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap().with_clock(|| 1_700_000_000);
    let doc = tool(&mut a, "doc_open", json!({ "path": "a.pdf" }))["doc"].as_u64().unwrap();
    if let Some(name) = first {
        tool(&mut a, "form_add_field", json!({ "doc": doc, "page": 1, "type": "signature", "name": name, "rect": [20.0, 240.0, 180.0, 280.0] }));
    }
    for (name, y) in [("SigAna", 60.0), ("SigBo", 120.0), ("Extra", 180.0)] {
        tool(&mut a, "form_add_field", json!({ "doc": doc, "page": 1, "type": "signature", "name": name, "rect": [20.0, y, 180.0, y + 40.0] }));
    }
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "form.pdf" }));
    let recipients = json!([
        { "name": "Ana Example", "email": "ana@example.org" },
        { "name": "Bo Example", "email": "bo@example.org" },
        { "name": "Cy Example", "email": "cy@example.org", "role": "approver" },
    ]);
    tool(&mut a, "signing_plan_set", json!({ "path": "form.pdf", "recipients": recipients }));
    let doc = tool(&mut a, "doc_open", json!({ "path": "form.pdf" }))["doc"].as_u64().unwrap();
    tool(&mut a, "form_field_signer", json!({ "doc": doc, "field": "SigAna", "signer": "ana@example.org" }));
    tool(&mut a, "form_field_signer", json!({ "doc": doc, "field": "SigBo", "signer": "bo@example.org" }));
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "form.pdf" }));
    for (name, file) in [("Ana Example", "ana.p12"), ("Bo Example", "bo.p12")] {
        tool(&mut a, "sign_id_create", json!({ "name": name, "key": "p256", "password": "secret12", "path": file }));
    }
    Rig { dir, a, doc }
}

#[test]
fn a_fully_signed_two_recipient_flow_agrees_with_its_file() {
    let mut r = rig();
    r.trust();
    r.sign("ana@example.org", "ana.p12");
    r.sign("bo@example.org", "bo.p12");
    let report = r.audit();
    assert_eq!((how(&report, "ana@example.org"), how(&report, "bo@example.org")), ("agrees".into(), "agrees".into()), "{report}");
    // The approver signs nothing, and nothing in the file is left unexplained.
    assert_eq!(how(&report, "cy@example.org"), "not_applicable");
    assert_eq!(report["consistent"], true, "{report}");
    assert!(report["problems"].as_array().unwrap().is_empty() && report["open_questions"].as_array().unwrap().is_empty(), "{report}");
    assert!(report["unaccounted_signatures"].as_array().unwrap().is_empty(), "{report}");
    // Each recipient's row names the field the file shows for them, and the file part is the audit's.
    let ana = check(&report, "ana@example.org")["signatures_labelled_for_recipient"].as_array().unwrap().clone();
    assert_eq!((ana.len(), &ana[0]["field"], &ana[0]["status"]), (1, &json!("SigAna"), &json!("valid")), "{report}");
    assert_eq!((report["file"]["signed"].as_u64(), report["file"]["valid"].as_u64()), (Some(2), Some(2)), "{report}");
    // The plan part is the plan: the same recipients, in order, with their history.
    assert_eq!(report["plan"]["state"], "in_progress");
    let history = check(&report, "ana@example.org")["recipient"]["history"].as_array().unwrap().clone();
    assert_eq!(
        (history.first().map(|e| &e["status"]), history.last().map(|e| &e["status"])),
        (Some(&json!("pending")), Some(&json!("signed"))),
        "{report}"
    );
}

#[test]
fn a_signature_that_is_present_but_not_shown_valid_is_not_called_agreement() {
    // Nobody trusts the test IDs, so the validator says unknown: the plan and the file agree that a
    // signature exists, and the report does not say it validates.
    let mut r = rig();
    r.sign("ana@example.org", "ana.p12");
    let report = r.audit();
    assert_eq!(how(&report, "ana@example.org"), "signature_present_not_shown_valid", "{report}");
    assert_eq!(how(&report, "bo@example.org"), "agrees", "Bo is not signed in the plan or in the file");
    assert_eq!(report["consistent"], Value::Null, "{report}");
    assert!(report["problems"].as_array().unwrap().is_empty());
    assert!(!report["open_questions"].as_array().unwrap().is_empty());
}

#[test]
fn a_plan_that_says_signed_with_no_signature_in_the_file_is_flagged() {
    let mut r = rig();
    // Only a record: the plan is told Ana signed and nothing was signed.
    tool(&mut r.a, "signing_plan_advance", json!({ "path": "form.pdf", "email": "ana@example.org", "status": "signed" }));
    let report = r.audit();
    assert_eq!(how(&report, "ana@example.org"), "plan_says_signed_but_no_signature", "{report}");
    assert_eq!(report["consistent"], false);
    assert!(report["problems"].as_array().unwrap().iter().any(|p| p.as_str().unwrap().contains("ana@example.org")), "{report}");
    assert_eq!(report["file"]["signed"], 0);
    assert_eq!(how(&report, "bo@example.org"), "agrees");
}

#[test]
fn a_signature_the_plan_does_not_account_for_is_flagged() {
    let mut r = rig();
    r.trust();
    r.sign("ana@example.org", "ana.p12");
    // A third party signs the unlabelled field with an ID of their own, outside the plan.
    r.sign_outside_the_plan("Extra", "bo.p12");
    let report = r.audit();
    let extra = report["unaccounted_signatures"].as_array().unwrap();
    assert_eq!(extra.len(), 1, "{report}");
    assert_eq!(extra[0]["signature"]["field"], "Extra");
    assert!(extra[0]["why"].as_str().unwrap().contains("no signer label"), "{report}");
    assert_eq!(report["consistent"], false);
    // Ana is still shown to agree: the stray signature does not hide or replace hers.
    assert_eq!(how(&report, "ana@example.org"), "agrees", "{report}");
    assert_eq!(report["file"]["signed"], 2);
}

#[test]
fn a_signature_labelled_for_an_address_outside_the_plan_is_flagged() {
    let mut r = rig();
    // Zed is a recipient for a moment, so the field can be labelled for Zed; then the plan is made again without Zed.
    let with_zed = json!([{ "name": "Ana Example", "email": "ana@example.org" }, { "name": "Zed Example", "email": "zed@example.org" }]);
    tool(&mut r.a, "signing_plan_set", json!({ "path": "form.pdf", "recipients": with_zed, "replace": true }));
    tool(&mut r.a, "form_field_signer", json!({ "doc": r.doc, "field": "Extra", "signer": "zed@example.org" }));
    tool(&mut r.a, "doc_save", json!({ "doc": r.doc, "path": "form.pdf" }));
    let without_zed = json!([{ "name": "Ana Example", "email": "ana@example.org" }]);
    tool(&mut r.a, "signing_plan_set", json!({ "path": "form.pdf", "recipients": without_zed, "replace": true }));
    r.sign_outside_the_plan("Extra", "bo.p12");
    let report = r.audit();
    let extra = report["unaccounted_signatures"].as_array().unwrap();
    assert_eq!(extra.len(), 1, "{report}");
    assert!(extra[0]["why"].as_str().unwrap().contains("not a recipient"), "{report}");
}

#[test]
fn a_file_changed_after_signing_is_surfaced() {
    let mut r = rig();
    r.trust();
    r.sign("ana@example.org", "ana.p12");
    assert_eq!(how(&r.audit(), "ana@example.org"), "agrees");
    // One byte inside the signed range changes on disk, with the length kept; the plan is untouched.
    let mut bytes = std::fs::read(r.path("form.pdf")).unwrap();
    let at = bytes.windows(3).position(|w| w == b"300").expect("the page size is written in the first revision");
    bytes[at + 2] = b'1';
    std::fs::write(r.path("form.pdf"), &bytes).unwrap();
    let plan_before = std::fs::read(r.path("form.pdf.pdfcraft-sign.json")).unwrap();
    let report = r.audit();
    assert_eq!(how(&report, "ana@example.org"), "signature_invalid", "{report}");
    assert_eq!(report["file"]["invalid"], 1);
    assert_eq!(report["consistent"], false);
    assert!(report["problems"].as_array().unwrap().iter().any(|p| p.as_str().unwrap().contains("invalid")), "{report}");
    assert_eq!(plan_before, std::fs::read(r.path("form.pdf.pdfcraft-sign.json")).unwrap(), "the audit writes nothing");
}

#[test]
fn what_could_not_be_determined_is_not_read_as_none() {
    // A signature sits in a field whose name two fields share, so it cannot be matched to a label,
    // and Ana's field is one of the two: the report does not say Ana has no signature.
    let mut r = rig();
    tool(&mut r.a, "form_add_field", json!({ "doc": r.doc, "page": 1, "type": "signature", "name": "SigAnb", "rect": [20.0, 240.0, 180.0, 280.0] }));
    tool(&mut r.a, "doc_save", json!({ "doc": r.doc, "path": "form.pdf" }));
    r.sign_outside_the_plan("SigAnb", "bo.p12");
    // The second field takes the first's name, with its length kept.
    let mut bytes = std::fs::read(r.path("form.pdf")).unwrap();
    let places: Vec<usize> = (0..bytes.len() - 8).filter(|&i| &bytes[i..i + 8] == b"(SigAnb)").collect();
    assert!(!places.is_empty(), "the field name is written plainly");
    for at in places {
        bytes[at + 6] = b'a';
    }
    std::fs::write(r.path("form.pdf"), &bytes).unwrap();
    let report = r.audit();
    assert_eq!(how(&report, "ana@example.org"), "not_determined", "{report}");
    assert_eq!(report["not_determined_signatures"].as_array().unwrap().len(), 1, "{report}");
    assert_eq!(report["consistent"], false, "the changed bytes also make the signature invalid, which is a problem: {report}");
    assert!(report["unaccounted_signatures"].as_array().unwrap().is_empty(), "undetermined is not listed as unaccounted");
    assert!(!report["open_questions"].as_array().unwrap().is_empty());
}

#[test]
fn a_signature_in_a_recipients_field_while_the_plan_does_not_say_signed_is_flagged() {
    let mut r = rig();
    // The field labelled for Ana is signed by hand, and nobody told the plan.
    r.sign_outside_the_plan("SigAna", "ana.p12");
    let report = r.audit();
    assert_eq!(how(&report, "ana@example.org"), "signature_present_but_plan_not_signed", "{report}");
    assert_eq!(report["consistent"], false);
    assert_eq!(check(&report, "ana@example.org")["plan_says"], "pending");
}

#[test]
fn a_signature_labelled_for_an_approver_is_not_taken_for_a_signers() {
    let mut r = rig();
    tool(&mut r.a, "form_field_signer", json!({ "doc": r.doc, "field": "Extra", "signer": "cy@example.org" }));
    tool(&mut r.a, "doc_save", json!({ "doc": r.doc, "path": "form.pdf" }));
    r.sign_outside_the_plan("Extra", "bo.p12");
    let report = r.audit();
    assert_eq!(how(&report, "cy@example.org"), "signature_labelled_for_a_recipient_who_does_not_sign", "{report}");
    assert_eq!(report["consistent"], false);
}

#[test]
fn a_plan_address_in_capitals_still_matches_its_field_label() {
    // The plan keeps an address as typed and compares without regard to case; so does the audit.
    let mut r = rig();
    let people = json!([{ "name": "Ana Example", "email": "Ana@Example.org" }]);
    tool(&mut r.a, "signing_plan_set", json!({ "path": "form.pdf", "recipients": people, "replace": true }));
    // The label is written in the plan's spelling, capitals included.
    tool(&mut r.a, "form_field_signer", json!({ "doc": r.doc, "field": "SigAna", "signer": "ana@example.org" }));
    assert_eq!(tool(&mut r.a, "form_field_signer", json!({ "doc": r.doc, "field": "SigAna" }))["signer"], "Ana@Example.org");
    tool(&mut r.a, "doc_save", json!({ "doc": r.doc, "path": "form.pdf" }));
    r.trust();
    r.sign("ana@example.org", "ana.p12");
    let report = r.audit();
    assert_eq!(how(&report, "Ana@Example.org"), "agrees", "{report}");
    assert!(report["unaccounted_signatures"].as_array().unwrap().is_empty(), "{report}");
}

#[test]
fn a_cut_signature_list_does_not_prove_that_no_signature_exists() {
    // More signature fields than the audit lists: the plan says Ana signed and the first fifty
    // fields hold no signature, but the rest were not looked at.
    let mut r = rig();
    for n in 0..52 {
        let y = 10.0 + f64::from(n);
        tool(
            &mut r.a,
            "form_add_field",
            json!({ "doc": r.doc, "page": 1, "type": "signature", "name": format!("S{n:02}"), "rect": [10.0, y, 20.0, y + 5.0] }),
        );
    }
    tool(&mut r.a, "doc_save", json!({ "doc": r.doc, "path": "form.pdf" }));
    tool(&mut r.a, "signing_plan_advance", json!({ "path": "form.pdf", "email": "ana@example.org", "status": "signed" }));
    let report = r.audit();
    assert_eq!(report["file"]["truncated"], true, "{}", report["file"]["listed"]);
    assert_eq!((how(&report, "ana@example.org"), report["all_signature_fields_read"].clone()), ("not_determined".into(), json!(false)), "{report}");
    assert_eq!(report["consistent"], Value::Null);
}

/// A stranger signs the field that comes first in the form, whose name reads as `SigAna` once the audit
/// has cleaned it (a zero-width character is dropped) and is not the labelled field `SigAna`.
fn stray_in_a_look_alike_field(look_alike: &str) -> Rig {
    let mut r = rig_with(Some(look_alike));
    r.trust();
    let doc = tool(&mut r.a, "doc_open", json!({ "path": "form.pdf" }))["doc"].as_u64().unwrap();
    tool(&mut r.a, "sign_document", json!({ "doc": doc, "id": "bo.p12", "password": "secret12", "field": look_alike, "out": "stray.pdf" }));
    std::fs::rename(r.path("stray.pdf"), r.path("form.pdf")).unwrap();
    r
}

#[test]
fn a_look_alike_field_name_is_never_taken_for_the_labelled_field() {
    for (what, name) in [("a zero-width character at the end", "SigAna\u{200B}"), ("a zero-width character inside", "Sig\u{200B}Ana")] {
        let mut r = stray_in_a_look_alike_field(name);
        // The plan has been told that Ana signed; the only signature in the file is the stranger's.
        tool(&mut r.a, "signing_plan_advance", json!({ "path": "form.pdf", "email": "ana@example.org", "status": "signed" }));
        let report = r.audit();
        assert_eq!(report["file"]["signed"], 1, "{what}: {report}");
        assert_ne!(how(&report, "ana@example.org"), "agrees", "{what}: {report}");
        assert!(check(&report, "ana@example.org")["signatures_labelled_for_recipient"].as_array().unwrap().is_empty(), "{what}: {report}");
        assert_eq!(how(&report, "ana@example.org"), "not_determined", "{what}: {report}");
        assert_ne!(report["consistent"], true, "{what}: {report}");
        assert_eq!(report["not_determined_signatures"].as_array().unwrap().len(), 1, "{what}: {report}");
    }
}

#[test]
fn a_signature_that_could_not_be_matched_is_an_open_question_and_not_a_problem() {
    // A valid signature in a look-alike field, with a plan that says nothing is signed: nothing is wrong
    // that can be shown, but something could not be determined, so consistent is null and not false.
    for name in ["SigAna\u{200B}", "Sig\u{200B}Ana"] {
        let mut r = stray_in_a_look_alike_field(name);
        let report = r.audit();
        assert_eq!(report["file"]["valid"], 1, "{report}");
        assert_eq!(report["consistent"], Value::Null, "{report}");
        assert!(report["problems"].as_array().unwrap().is_empty(), "{report}");
        assert_eq!(report["not_determined_signatures"].as_array().unwrap().len(), 1, "{report}");
        assert!(report["unaccounted_signatures"].as_array().unwrap().is_empty(), "{report}");
    }
}

#[test]
fn two_signatures_labelled_for_one_recipient_are_an_open_question() {
    let mut r = rig();
    r.trust();
    tool(&mut r.a, "form_field_signer", json!({ "doc": r.doc, "field": "Extra", "signer": "ana@example.org" }));
    tool(&mut r.a, "doc_save", json!({ "doc": r.doc, "path": "form.pdf" }));
    r.sign_outside_the_plan("SigAna", "ana.p12");
    r.sign_outside_the_plan("Extra", "bo.p12");
    let report = r.audit();
    assert_eq!(check(&report, "ana@example.org")["labelled_signature_count"], 2, "{report}");
    assert!(report["open_questions"].as_array().unwrap().iter().any(|q| q.as_str().unwrap().contains("2 signatures are labelled")), "{report}");
    assert_ne!(report["consistent"], true);
}

#[test]
fn the_report_says_it_is_a_record_and_not_proof_of_identity_or_legal_effect() {
    let mut r = rig();
    let report = r.audit();
    assert_eq!(
        (report["identity_checked"].as_bool(), report["legal_effect_assessed"].as_bool(), report["local_record_only"].as_bool()),
        (Some(false), Some(false), Some(true)),
        "{report}"
    );
    assert_eq!((report["sent"].as_bool(), report["notified"].as_bool()), (Some(false), Some(false)));
    let note = report["note"].as_str().unwrap();
    assert!(note.contains("local record") && note.contains("evidence") && note.contains("legal effect") && note.contains("not_determined"), "{note}");
    let words = report.to_string().to_lowercase();
    assert!(!words.contains("certificate of completion"));
}

#[test]
fn a_document_without_a_plan_a_file_or_a_saved_state_is_refused() {
    let mut r = rig();
    // No plan.
    std::fs::remove_file(r.path("form.pdf.pdfcraft-sign.json")).unwrap();
    let doc = tool(&mut r.a, "doc_open", json!({ "path": "form.pdf" }))["doc"].as_u64().unwrap();
    let m = refused(&mut r.a, "signing_plan_audit", json!({ "doc": doc }));
    assert!(m.contains("no signing plan"), "{m}");
    assert!(!r.path("form.pdf.pdfcraft-sign.json").exists(), "a refused audit made no plan");
    // A plan that is not a plan.
    std::fs::write(r.path("form.pdf.pdfcraft-sign.json"), b"{ \"not\": \"a plan\" }").unwrap();
    let m = refused(&mut r.a, "signing_plan_audit", json!({ "doc": doc }));
    assert!(m.contains("left alone") || m.contains("not a signing plan"), "{m}");
    // No file, and unsaved changes.
    let blank = tool(&mut r.a, "doc_create", json!({ "from": "blank", "pages": 1 }))["doc"].as_u64().unwrap();
    assert!(refused(&mut r.a, "signing_plan_audit", json!({ "doc": blank })).contains("no file"));
    tool(&mut r.a, "form_add_field", json!({ "doc": doc, "page": 1, "type": "text", "name": "t", "rect": [20.0, 300.0, 100.0, 320.0] }));
    assert!(refused(&mut r.a, "signing_plan_audit", json!({ "doc": doc })).contains("not saved"));
}

#[test]
fn a_file_changed_on_disk_since_it_was_opened_is_refused() {
    let mut r = rig();
    let doc = tool(&mut r.a, "doc_open", json!({ "path": "form.pdf" }))["doc"].as_u64().unwrap();
    let mut bytes = std::fs::read(r.path("form.pdf")).unwrap();
    bytes.extend_from_slice(b"\n% another program wrote here\n");
    std::fs::write(r.path("form.pdf"), bytes).unwrap();
    assert!(refused(&mut r.a, "signing_plan_audit", json!({ "doc": doc })).contains("changed on disk"));
}

fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut all = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        all.push((entry.file_name().to_string_lossy().into_owned(), std::fs::read(entry.path()).unwrap_or_default()));
    }
    all.sort();
    all
}

#[test]
fn the_roots_are_respected_and_nothing_is_written() {
    let base = tempfile::tempdir().unwrap();
    let (read, write, outside) = (base.path().join("read"), base.path().join("write"), base.path().join("outside"));
    for d in [&read, &write, &outside] {
        std::fs::create_dir_all(d).unwrap();
    }
    let mut setup = Automation::new().with_root(&read).unwrap();
    let mut session = pdfcraft_engine::Session::new();
    let bytes = session.create_blank(300.0, 400.0, 1).unwrap();
    let id = session.open("a.pdf", None, bytes, None).unwrap();
    std::fs::write(read.join("form.pdf"), session.save_bytes(id).unwrap().as_slice()).unwrap();
    tool(&mut setup, "signing_plan_set", json!({ "path": "form.pdf", "recipients": [{ "name": "Ana", "email": "ana@example.org" }] }));

    let roots = AutomationRoots::new(Some(&read), Some(&write)).unwrap();
    let mut a = Automation::new().with_automation_roots(roots);
    let file = read.join("form.pdf").to_string_lossy().into_owned();
    let doc = tool(&mut a, "doc_open", json!({ "path": file }))["doc"].as_u64().unwrap();
    let before = (snapshot(&read), snapshot(&write), snapshot(&outside));
    let report = tool(&mut a, "signing_plan_audit", json!({ "doc": doc }));
    assert_eq!(report["plan"]["recipients"].as_array().unwrap().len(), 1);
    assert!(before == (snapshot(&read), snapshot(&write), snapshot(&outside)), "the audit wrote a file");
    // The tool has no path argument to point elsewhere: an extra one is refused.
    refused(&mut a, "signing_plan_audit", json!({ "doc": doc, "path": outside.join("x.json").to_string_lossy() }));
    refused(&mut a, "signing_plan_audit", json!({ "doc": doc, "output": write.join("x.pdf").to_string_lossy() }));
    assert!(before == (snapshot(&read), snapshot(&write), snapshot(&outside)));
}

#[cfg(unix)]
#[test]
fn a_plan_that_is_a_link_to_a_file_outside_the_root_is_refused_and_not_read() {
    let base = tempfile::tempdir().unwrap();
    let (read, outside) = (base.path().join("read"), base.path().join("outside"));
    for d in [&read, &outside] {
        std::fs::create_dir_all(d).unwrap();
    }
    let mut setup = Automation::new().with_root(base.path()).unwrap();
    let mut session = pdfcraft_engine::Session::new();
    let bytes = session.create_blank(300.0, 400.0, 1).unwrap();
    let id = session.open("a.pdf", None, bytes, None).unwrap();
    std::fs::write(read.join("form.pdf"), session.save_bytes(id).unwrap().as_slice()).unwrap();
    tool(&mut setup, "signing_plan_set", json!({ "path": "read/form.pdf", "recipients": [{ "name": "Ana", "email": "ana@example.org" }] }));
    std::fs::rename(read.join("form.pdf.pdfcraft-sign.json"), outside.join("plan.json")).unwrap();
    std::os::unix::fs::symlink(outside.join("plan.json"), read.join("form.pdf.pdfcraft-sign.json")).unwrap();
    let roots = AutomationRoots::new(Some(&read), Some(&read)).unwrap();
    let mut a = Automation::new().with_automation_roots(roots);
    let doc = tool(&mut a, "doc_open", json!({ "path": read.join("form.pdf").to_string_lossy() }))["doc"].as_u64().unwrap();
    let m = refused(&mut a, "signing_plan_audit", json!({ "doc": doc }));
    assert!(m.contains("not a plain file") || m.contains("outside"), "{m}");
}
