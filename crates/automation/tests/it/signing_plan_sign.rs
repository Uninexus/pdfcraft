//! `signing_plan_sign`: the next recipient's turn of a local signing plan in one step. The sign
//! crate's validator (`sign_list`, `signature_audit_report`) is the oracle for whether earlier
//! signatures survive a later one; the plan and the file on disk are compared before and after
//! every refusal. All people are made up and at example.org, and the digital IDs are self-signed
//! test keys made here.

use pdfcraft_automation::{Automation, Content, ToolError};
use serde_json::{Value, json};

fn tool(a: &mut Automation, name: &str, args: Value) -> Value {
    match a.call(name, &args).unwrap().remove(0) {
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
    fn file(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.dir.path().join(name)).unwrap()
    }

    fn plan_bytes(&self) -> Vec<u8> {
        self.file("form.pdf.pdfcraft-sign.json")
    }

    fn plan(&mut self) -> Value {
        tool(&mut self.a, "signing_plan_get", json!({ "path": "form.pdf" }))
    }

    fn sign(&mut self, email: &str, id: &str) -> Value {
        tool(&mut self.a, "signing_plan_sign", json!({ "doc": self.doc, "email": email, "id": id, "password": "secret12" }))
    }

    fn sign_refused(&mut self, args: Value) -> String {
        let mut args = args;
        args["doc"] = json!(self.doc);
        args["password"] = json!("secret12");
        refused(&mut self.a, "signing_plan_sign", args)
    }

    /// Status of each signature field as `sign_list` reads the open document.
    fn statuses(&mut self) -> Vec<(String, String)> {
        let list = tool(&mut self.a, "sign_list", json!({ "doc": self.doc }));
        list["signatures"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["field"].as_str().unwrap().to_string(), s["status"].as_str().unwrap().to_string()))
            .collect()
    }

    fn status_of(&mut self, field: &str) -> String {
        self.statuses().into_iter().find(|(f, _)| f == field).map(|(_, s)| s).unwrap()
    }

    /// The state that a refusal must leave alone: the saved file, the plan and the field statuses.
    fn state(&mut self) -> (Vec<u8>, Vec<u8>, Vec<(String, String)>) {
        (self.file("form.pdf"), self.plan_bytes(), self.statuses())
    }

    fn assert_refused_leaves_everything(&mut self, args: Value, why: &str) {
        let before = self.state();
        let m = self.sign_refused(args);
        assert!(m.contains(why), "{m}");
        assert!(self.state() == before, "a refused call changed the file, the plan or a status: {m}");
    }
}

/// A saved form `form.pdf` with a text field `name` and signature fields `SigAna`, `SigBo` and
/// `Extra`; `SigAna` is labelled for Ana and `SigBo` for Bo (before any signature exists); a plan
/// beside it of Ana, then Bo (signers), Cy (approver) and Dee (viewer); and ID files for each.
fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let mut session = pdfcraft_engine::Session::new();
    let bytes = session.create_blank(300.0, 400.0, 1).unwrap();
    let id = session.open("a.pdf", None, bytes, None).unwrap();
    std::fs::write(dir.path().join("a.pdf"), session.save_bytes(id).unwrap().as_slice()).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap().with_clock(|| 1_700_000_000);
    let doc = tool(&mut a, "doc_open", json!({ "path": "a.pdf" }))["doc"].as_u64().unwrap();
    for (ty, name, y) in [("text", "name", 20.0), ("signature", "SigAna", 60.0), ("signature", "SigBo", 120.0), ("signature", "Extra", 180.0)] {
        tool(&mut a, "form_add_field", json!({ "doc": doc, "page": 1, "type": ty, "name": name, "rect": [20.0, y, 180.0, y + 40.0] }));
    }
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "form.pdf" }));
    let recipients = json!([
        { "name": "Ana Example", "email": "Ana@example.org" },
        { "name": "Bo Example", "email": "bo@example.org" },
        { "name": "Cy Example", "email": "cy@example.org", "role": "approver" },
        { "name": "Dee Example", "email": "dee@example.org", "role": "viewer" },
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
fn two_recipients_sign_in_order_and_the_first_signature_stays_intact() {
    let mut r = rig();
    let start = r.file("form.pdf");
    // The label is stored in the plan's spelling (an upper-case A) and the call spells the
    // address in lower case and in capitals: both find the field.
    let label = tool(&mut r.a, "form_field_signer", json!({ "doc": r.doc, "field": "SigAna" }));
    assert_ne!(label["signer"].as_str().unwrap(), "ana@example.org", "{label}");
    assert_eq!(label["signer"].as_str().unwrap().to_lowercase(), "ana@example.org");
    let first = r.sign("ANA@EXAMPLE.ORG", "ana.p12");
    assert_eq!(
        (first["status"].as_str(), first["signed"].as_bool(), first["plan_advanced"].as_bool()),
        (Some("complete"), Some(true), Some(true)),
        "{first}"
    );
    assert_eq!(first["field"], "SigAna");
    assert_eq!(first["next"]["email"], "bo@example.org", "the plan moved on to Bo");
    let after_ana = r.file("form.pdf");
    assert!(after_ana.starts_with(&start) && after_ana.len() > start.len(), "an incremental update keeps the earlier bytes in front");
    let ana_alone = r.status_of("SigAna");
    assert_ne!(ana_alone, "invalid");

    let second = r.sign("bo@example.org", "bo.p12");
    assert_eq!(second["status"], "complete", "{second}");
    assert_eq!((second["signatures_before"].as_u64(), second["signatures_now"].as_u64()), (Some(1), Some(2)), "{second}");
    // The saved file is bigger by the second signature and keeps the first one's bytes in front.
    let after_bo = r.file("form.pdf");
    assert!(after_bo.starts_with(&after_ana) && after_bo.len() > after_ana.len());
    // The validator: Ana's signature is as good as it was alone, Bo's is not invalid, and the
    // file read again from disk says the same.
    assert_eq!(r.status_of("SigAna"), ana_alone);
    assert_ne!(r.status_of("SigBo"), "invalid");
    let fresh = tool(&mut r.a, "doc_open", json!({ "path": "form.pdf" }))["doc"].as_u64().unwrap();
    let list = tool(&mut r.a, "sign_list", json!({ "doc": fresh }));
    assert_eq!(list["signed"], 2, "{list}");
    let ana = list["signatures"].as_array().unwrap().iter().find(|s| s["field"] == "SigAna").unwrap().clone();
    assert_eq!(ana["status"], json!(ana_alone));
    let audit = tool(&mut r.a, "signature_audit_report", json!({ "doc": fresh }));
    let first_entry = audit["signatures"].as_array().unwrap().iter().find(|s| s["field"] == "SigAna").unwrap().clone();
    assert_eq!(first_entry["covers"], "changes_allowed", "later signing is a change Ana's signature allows: {first_entry}");

    // The plan lists both as signed, in order; Cy (an approver) is next and is not asked to sign.
    let plan = r.plan();
    let statuses: Vec<&str> = plan["plan"]["recipients"].as_array().unwrap().iter().map(|p| p["status"].as_str().unwrap()).collect();
    assert_eq!(statuses[..3], ["signed", "signed", "pending"], "{plan}");
    assert_eq!(plan["next"]["email"], "cy@example.org");
}

#[test]
fn the_plan_does_not_check_who_holds_the_id_and_the_result_says_so() {
    let mut r = rig();
    // Ana's turn, signed with Bo's ID: the plan does not know whose key it is.
    let out = r.sign("ana@example.org", "bo.p12");
    assert_eq!(
        (out["identity_checked"].as_bool(), out["sent"].as_bool(), out["notified"].as_bool(), out["local_record_only"].as_bool()),
        (Some(false), Some(false), Some(false), Some(true)),
        "{out}"
    );
    assert!(out["signature"]["signer"].as_str().unwrap().contains("Bo Example"), "{out}");
}

#[test]
fn a_turn_out_of_order_or_not_a_signers_is_refused_and_nothing_changes() {
    let mut r = rig();
    // An earlier edit leaves an undo step that no refusal may use up.
    tool(&mut r.a, "form_field_signer", json!({ "doc": r.doc, "field": "name", "signer": "ana@example.org" }));
    tool(&mut r.a, "doc_save", json!({ "doc": r.doc, "path": "form.pdf" }));
    for (email, why) in [
        ("bo@example.org", "not that recipient's turn"),
        ("cy@example.org", "not that recipient's turn"),
        ("dee@example.org", "viewer"),
        ("zed@example.org", "no recipient has that email address"),
    ] {
        r.assert_refused_leaves_everything(json!({ "email": email, "id": "bo.p12" }), why);
    }
    assert_eq!(r.plan()["plan"]["recipients"][0]["status"], "pending");
    // An approver's turn gets its own sentence pointing at the other tool; the plan has no signer left.
    r.sign("ana@example.org", "ana.p12");
    r.sign("bo@example.org", "bo.p12");
    r.assert_refused_leaves_everything(json!({ "email": "cy@example.org", "id": "ana.p12" }), "is an approver");
    // The same address again: it is already signed, so it is not its turn (and the plan has the approver next).
    r.assert_refused_leaves_everything(json!({ "email": "ana@example.org", "id": "ana.p12" }), "not that recipient's turn");
    // Signing drops the undo history (a refused call keeps it: see a_refused_call_keeps_the_undo_history_and_a_signing_drops_it).
    assert!(r.a.call("edit_undo", &json!({ "doc": r.doc })).is_err(), "signing drops the undo history, as sign_document does");
}

#[test]
fn a_refused_call_keeps_the_undo_history_and_a_signing_drops_it() {
    let mut r = rig();
    tool(&mut r.a, "form_field_signer", json!({ "doc": r.doc, "field": "name", "signer": "ana@example.org" }));
    tool(&mut r.a, "doc_save", json!({ "doc": r.doc, "path": "form.pdf" }));
    r.assert_refused_leaves_everything(json!({ "email": "bo@example.org", "id": "bo.p12" }), "not that recipient's turn");
    assert_eq!(tool(&mut r.a, "edit_undo", json!({ "doc": r.doc }))["undone"], "Set field signer", "the step is still there after a refusal");
    tool(&mut r.a, "edit_redo", json!({ "doc": r.doc }));
    tool(&mut r.a, "doc_save", json!({ "doc": r.doc, "path": "form.pdf" }));
    r.sign("ana@example.org", "ana.p12");
    assert!(r.a.call("edit_undo", &json!({ "doc": r.doc })).is_err());
}

#[test]
fn no_plan_unsaved_changes_extra_arguments_and_a_document_without_a_file_are_refused() {
    let mut r = rig();
    // Unsaved changes: the signed file replaces the saved one, so it must not swallow them.
    tool(&mut r.a, "form_field_signer", json!({ "doc": r.doc, "field": "name", "signer": "ana@example.org" }));
    r.assert_refused_leaves_everything(json!({ "email": "ana@example.org", "id": "ana.p12" }), "not saved");
    tool(&mut r.a, "doc_save", json!({ "doc": r.doc, "path": "form.pdf" }));
    // The arguments sign_document has that make no sense here are refused by the schema, not ignored.
    for key in ["certify", "out", "page", "rect", "run_form_scripts"] {
        let value = if key == "rect" {
            json!([0, 0, 10, 10])
        } else if key == "page" {
            json!(1)
        } else if key == "run_form_scripts" {
            json!(true)
        } else {
            json!("x")
        };
        let m = r.sign_refused(json!({ "email": "ana@example.org", "id": "ana.p12", key: value }));
        assert!(m.contains(&format!("unknown argument \"{key}\"")), "{m}");
    }
    // No plan.
    tool(&mut r.a, "signing_plan_clear", json!({ "path": "form.pdf" }));
    let before = r.file("form.pdf");
    let m = r.sign_refused(json!({ "email": "ana@example.org", "id": "ana.p12" }));
    assert!(m.contains("no signing plan"), "{m}");
    assert_eq!(r.file("form.pdf"), before);
    // A document that has no file.
    let blank = tool(&mut r.a, "doc_create", json!({ "from": "blank" }))["doc"].as_u64().unwrap();
    let m = refused(&mut r.a, "signing_plan_sign", json!({ "doc": blank, "email": "ana@example.org", "id": "ana.p12", "password": "secret12" }));
    assert!(m.contains("has no file"), "{m}");
}

#[test]
fn a_finished_plan_refuses_a_further_turn() {
    let mut r = rig();
    r.sign("ana@example.org", "ana.p12");
    r.sign("bo@example.org", "bo.p12");
    tool(&mut r.a, "signing_plan_advance", json!({ "path": "form.pdf", "email": "cy@example.org", "status": "approved" }));
    assert_eq!(r.plan()["state"], "completed");
    r.assert_refused_leaves_everything(json!({ "email": "ana@example.org", "id": "ana.p12" }), "the plan is finished");
}

#[test]
fn the_field_is_the_one_labelled_for_the_recipient_or_the_one_the_caller_names() {
    // Unlabelled: refused, and a named field is the way out.
    let mut r = rig();
    tool(&mut r.a, "form_field_signer", json!({ "doc": r.doc, "field": "SigAna", "signer": null }));
    tool(&mut r.a, "doc_save", json!({ "doc": r.doc, "path": "form.pdf" }));
    r.assert_refused_leaves_everything(json!({ "email": "ana@example.org", "id": "ana.p12" }), "no signature field is labelled for this recipient");
    let m = r.sign_refused(json!({ "email": "ana@example.org", "id": "ana.p12", "field": "SigBo" }));
    assert!(m.contains("labelled for another recipient"), "{m}");
    for (field, why) in [("name", "is not a signature field"), ("Nope", "there is no field named")] {
        let m = r.sign_refused(json!({ "email": "ana@example.org", "id": "ana.p12", "field": field }));
        assert!(m.contains(why), "{m}");
    }
    // Naming her own unlabelled field is the way out.
    let out = tool(
        &mut r.a,
        "signing_plan_sign",
        json!({ "doc": r.doc, "email": "ana@example.org", "id": "ana.p12", "password": "secret12", "field": "SigAna" }),
    );
    assert_eq!((out["status"].as_str(), out["field"].as_str()), (Some("complete"), Some("SigAna")), "{out}");
}

#[test]
fn a_named_unlabelled_field_is_signed_and_the_labelled_one_is_left_alone() {
    let mut r = rig();
    tool(&mut r.a, "form_field_signer", json!({ "doc": r.doc, "field": "SigAna", "signer": null }));
    tool(&mut r.a, "doc_save", json!({ "doc": r.doc, "path": "form.pdf" }));
    let out = tool(
        &mut r.a,
        "signing_plan_sign",
        json!({ "doc": r.doc, "email": "ANA@Example.ORG", "id": "ana.p12", "password": "secret12", "field": "Extra" }),
    );
    assert_eq!((out["status"].as_str(), out["field"].as_str()), (Some("complete"), Some("Extra")), "{out}");
    assert_eq!(r.status_of("SigAna"), "unsigned", "the labelled-before field was not the one signed");
}

#[test]
fn two_unsigned_fields_for_one_recipient_need_the_caller_to_choose() {
    let mut r = rig();
    tool(&mut r.a, "form_field_signer", json!({ "doc": r.doc, "field": "Extra", "signer": "ana@example.org" }));
    tool(&mut r.a, "doc_save", json!({ "doc": r.doc, "path": "form.pdf" }));
    let before = r.state();
    let m = r.sign_refused(json!({ "email": "ana@example.org", "id": "ana.p12" }));
    assert!(m.contains("2 unsigned signature fields") && m.contains("SigAna") && m.contains("Extra") && m.contains("`field`"), "{m}");
    assert!(r.state() == before);
    let out = tool(
        &mut r.a,
        "signing_plan_sign",
        json!({ "doc": r.doc, "email": "ana@example.org", "id": "ana.p12", "password": "secret12", "field": "Extra" }),
    );
    assert_eq!(out["field"], "Extra", "{out}");
    assert_eq!(r.status_of("SigAna"), "unsigned");
    assert_ne!(r.status_of("Extra"), "unsigned");
}

#[test]
fn a_recipient_whose_only_field_already_holds_a_signature_is_refused() {
    let mut r = rig();
    // Signed outside the plan, with sign_document: the plan still has Ana next.
    tool(&mut r.a, "sign_document", json!({ "doc": r.doc, "id": "ana.p12", "password": "secret12", "field": "SigAna", "out": "form.pdf" }));
    r.assert_refused_leaves_everything(json!({ "email": "ana@example.org", "id": "ana.p12" }), "already holds a signature");
    // The refusal says what to do if an earlier call was interrupted after it saved the file.
    assert!(r.sign_refused(json!({ "email": "ana@example.org", "id": "ana.p12" })).contains("signing_plan_advance"));
    let m = r.sign_refused(json!({ "email": "ana@example.org", "id": "ana.p12", "field": "SigAna" }));
    assert!(m.contains("already holds a signature") && m.contains("signing_plan_advance"), "{m}");
}

#[test]
fn a_field_locked_by_an_earlier_signature_is_refused_by_the_validator_and_the_plan_stays_put() {
    let mut r = rig();
    // Ana's signature locks Bo's field.
    tool(&mut r.a, "signature_field_lock", json!({ "doc": r.doc, "field": "SigAna", "action": "include", "fields": ["SigBo"] }));
    tool(&mut r.a, "doc_save", json!({ "doc": r.doc, "path": "form.pdf" }));
    r.sign("ana@example.org", "ana.p12");
    let ana = r.status_of("SigAna");
    let before = r.state();
    let m = r.sign_refused(json!({ "email": "bo@example.org", "id": "bo.p12" }));
    assert!(m.contains("covered by the lock of the signature in field \"SigAna\"") && m.contains("the document was not signed"), "{m}");
    assert!(r.state() == before, "file, plan and statuses are as they were");
    assert_eq!(r.status_of("SigAna"), ana);
    assert_eq!(r.plan()["next"]["email"], "bo@example.org", "the plan did not advance");
    // A field the lock does not name is still open to Bo.
    let out = tool(
        &mut r.a,
        "signing_plan_sign",
        json!({ "doc": r.doc, "email": "bo@example.org", "id": "bo.p12", "password": "secret12", "field": "Extra" }),
    );
    assert_eq!(out["status"], "complete", "{out}");
    assert_eq!(r.status_of("SigAna"), ana);
}

#[test]
fn a_certification_that_allows_no_changes_refuses_the_next_signer() {
    let mut r = rig();
    // Ana certifies with sign_document (this tool never certifies), and the plan is advanced by hand.
    tool(
        &mut r.a,
        "sign_document",
        json!({ "doc": r.doc, "id": "ana.p12", "password": "secret12", "field": "SigAna", "certify": "no_changes", "out": "form.pdf" }),
    );
    tool(&mut r.a, "signing_plan_advance", json!({ "path": "form.pdf", "email": "ana@example.org", "status": "signed" }));
    // File, plan and every status are as they were after the refusal.
    r.assert_refused_leaves_everything(json!({ "email": "bo@example.org", "id": "bo.p12" }), "certification that allows no changes");
    let m = r.sign_refused(json!({ "email": "bo@example.org", "id": "bo.p12" }));
    assert!(m.contains("the document was not signed"), "{m}");
    assert_eq!(r.status_of("SigBo"), "unsigned");
    assert_eq!(r.plan()["next"]["email"], "bo@example.org");
}

#[test]
fn a_plan_that_is_not_a_plan_or_an_id_that_does_not_open_refuses_before_any_write() {
    let mut r = rig();
    let before = r.state();
    let m = r.sign_refused(json!({ "email": "ana@example.org", "id": "missing.p12" }));
    assert!(m.contains("missing.p12"), "{m}");
    assert!(r.state() == before);
    let m =
        refused(&mut r.a, "signing_plan_sign", json!({ "doc": r.doc, "email": "ana@example.org", "id": "ana.p12", "password": "wrong-password" }));
    assert!(m.contains("password"), "{m}");
    assert!(r.state() == before);
    let file_before = r.file("form.pdf");
    std::fs::write(r.dir.path().join("form.pdf.pdfcraft-sign.json"), b"not json").unwrap();
    let m = r.sign_refused(json!({ "email": "ana@example.org", "id": "ana.p12" }));
    assert!(m.contains("is not a signing plan"), "{m}");
    assert_eq!(r.file("form.pdf"), file_before, "a bad plan is refused before the document is written");
    assert_eq!(r.status_of("SigAna"), "unsigned");
}

#[test]
fn a_stale_copy_of_the_document_cannot_overwrite_an_earlier_signature() {
    let mut r = rig();
    // A second copy of the document is opened from the same file before anyone signs.
    let stale = tool(&mut r.a, "doc_open", json!({ "path": "form.pdf" }))["doc"].as_u64().unwrap();
    assert_ne!(stale, r.doc);
    r.sign("ana@example.org", "ana.p12");
    let signed = r.file("form.pdf");
    let plan = r.plan_bytes();
    let m = refused(&mut r.a, "signing_plan_sign", json!({ "doc": stale, "email": "bo@example.org", "id": "bo.p12", "password": "secret12" }));
    assert!(m.contains("the file changed on disk since it was opened; reopen it"), "{m}");
    assert_eq!(r.file("form.pdf"), signed, "Ana's signature is still in the file, byte for byte");
    assert_eq!(r.plan_bytes(), plan);
    let fresh = tool(&mut r.a, "doc_open", json!({ "path": "form.pdf" }))["doc"].as_u64().unwrap();
    assert_eq!(tool(&mut r.a, "sign_list", json!({ "doc": fresh }))["signed"], 1);
    // A copy opened afterwards is current and may sign.
    let out = tool(&mut r.a, "signing_plan_sign", json!({ "doc": fresh, "email": "bo@example.org", "id": "bo.p12", "password": "secret12" }));
    assert_eq!(out["status"], "complete", "{out}");
    assert_eq!(out["signatures_now"], 2);
}

#[test]
fn a_field_with_signing_scripts_is_refused_and_nothing_changes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("form.pdf"), pdfcraft_xfa::fixtures::shell(&pdfcraft_xfa::fixtures::signing_events_template())).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap().with_clock(|| 1_700_000_000);
    tool(&mut a, "sign_id_create", json!({ "name": "Ana Example", "key": "p256", "password": "secret12", "path": "ana.p12" }));
    tool(&mut a, "signing_plan_set", json!({ "path": "form.pdf", "recipients": [{ "name": "Ana Example", "email": "ana@example.org" }] }));
    let doc = tool(&mut a, "doc_open", json!({ "path": "form.pdf" }))["doc"].as_u64().unwrap();
    let fields = tool(&mut a, "form_fields", json!({ "doc": doc }));
    let sign = fields["fields"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["name"].as_str())
        .find(|n| n.ends_with("sign") || n.ends_with(".sign[0]"))
        .unwrap()
        .to_string();
    let before = (std::fs::read(dir.path().join("form.pdf")).unwrap(), std::fs::read(dir.path().join("form.pdf.pdfcraft-sign.json")).unwrap());
    let m = refused(
        &mut a,
        "signing_plan_sign",
        json!({ "doc": doc, "email": "ana@example.org", "id": "ana.p12", "password": "secret12", "field": sign }),
    );
    assert!(m.contains("has signing scripts") && m.contains("sign_document"), "{m}");
    let after = (std::fs::read(dir.path().join("form.pdf")).unwrap(), std::fs::read(dir.path().join("form.pdf.pdfcraft-sign.json")).unwrap());
    assert!(before == after);
}

#[test]
fn a_field_named_with_a_dot_is_found_labelled_and_signed() {
    // A signature field below a parent field: the form lists it by its full name and the
    // validator reads the same name, so the new signature is found.
    let mut r = rig();
    tool(
        &mut r.a,
        "form_add_field",
        json!({ "doc": r.doc, "page": 1, "type": "signature", "name": "Party.Cy", "rect": [20.0, 240.0, 180.0, 280.0] }),
    );
    tool(&mut r.a, "form_field_signer", json!({ "doc": r.doc, "field": "SigAna", "signer": null }));
    tool(&mut r.a, "form_field_signer", json!({ "doc": r.doc, "field": "Party.Cy", "signer": "ana@example.org" }));
    tool(&mut r.a, "doc_save", json!({ "doc": r.doc, "path": "form.pdf" }));
    let out = r.sign("ana@example.org", "ana.p12");
    assert_eq!((out["status"].as_str(), out["field"].as_str()), (Some("complete"), Some("Party.Cy")), "{out}");
    assert_ne!(r.status_of("Party.Cy"), "unsigned");
}
