//! `form_field_signer` and `form_signer_report`: a label on a form field naming the signing-plan
//! recipient it is meant for, checked in the saved file. The label restricts nobody; these tests
//! check that it is stored, read back, undone, kept by saves, checked against the plan beside the
//! document, and refused where the document's permissions or a signature forbid a change.

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

/// A form with text fields `name` and `city` and signature fields `Sig1` and `Sig2`, saved as
/// `form.pdf` and reopened, so that no undo step is left.
fn setup() -> (tempfile::TempDir, Automation, u64) {
    let dir = tempfile::tempdir().unwrap();
    let mut session = pdfcraft_engine::Session::new();
    let bytes = session.create_blank(300.0, 400.0, 1).unwrap();
    let id = session.open("a.pdf", None, bytes, None).unwrap();
    std::fs::write(dir.path().join("a.pdf"), session.save_bytes(id).unwrap().as_slice()).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap().with_clock(|| 1_700_000_000);
    let doc = tool(&mut a, "doc_open", json!({ "path": "a.pdf" }))["doc"].as_u64().unwrap();
    for (ty, name, y) in [("text", "name", 20.0), ("text", "city", 60.0), ("signature", "Sig1", 100.0), ("signature", "Sig2", 160.0)] {
        tool(&mut a, "form_add_field", json!({ "doc": doc, "page": 1, "type": ty, "name": name, "rect": [20.0, y, 180.0, y + 30.0] }));
    }
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "form.pdf" }));
    let doc = tool(&mut a, "doc_open", json!({ "path": "form.pdf" }))["doc"].as_u64().unwrap();
    (dir, a, doc)
}

fn plan(a: &mut Automation) {
    let recipients = json!([
        { "name": "Ana Example", "email": "Ana@example.org" },
        { "name": "Bo Example", "email": "bo@example.org", "role": "approver" },
        { "name": "Cy Example", "email": "cy@example.org", "role": "viewer" },
    ]);
    tool(a, "signing_plan_set", json!({ "path": "form.pdf", "recipients": recipients }));
}

fn set(a: &mut Automation, doc: u64, field: &str, signer: Value) -> Value {
    tool(a, "form_field_signer", json!({ "doc": doc, "field": field, "signer": signer }))
}

fn read(a: &mut Automation, doc: u64, field: &str) -> Value {
    tool(a, "form_field_signer", json!({ "doc": doc, "field": field }))
}

fn saved(dir: &tempfile::TempDir, a: &mut Automation, doc: u64, name: &str) -> Vec<u8> {
    tool(a, "doc_save", json!({ "doc": doc, "path": name }));
    std::fs::read(dir.path().join(name)).unwrap()
}

/// The labels in the saved file, read with the object parser (a saved file may keep objects in
/// compressed streams, where a byte search would not find the key).
fn labels(bytes: &[u8]) -> Vec<String> {
    let doc = pdfcraft_cos::Document::open(std::sync::Arc::new(bytes.to_vec())).unwrap();
    (1..200)
        .filter_map(|num| {
            doc.get(pdfcraft_cos::ObjRef { num, generation: 0 })
                .as_dict()
                .and_then(|d| d.get(b"PdfCraftSigner").and_then(|o| o.as_string().map(|s| s.to_text())))
        })
        .collect()
}

fn has_label(bytes: &[u8]) -> bool {
    !labels(bytes).is_empty()
}

fn no_undo_step(a: &mut Automation, doc: u64) {
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "an undo step was left");
}

#[test]
fn a_label_is_set_read_saved_reopened_undone_redone_and_cleared() {
    let (dir, mut a, doc) = setup();
    let none = read(&mut a, doc, "name");
    assert_eq!((none["signer"].clone(), none["has_entry"].as_bool()), (Value::Null, Some(false)));
    assert_eq!(none["label_only"], true);
    assert!(none["note"].as_str().unwrap().contains("does not restrict who can fill or sign"), "{none}");

    let out = set(&mut a, doc, "name", json!("ana@example.org"));
    assert_eq!((out["changed"].as_bool(), out["signer"].as_str()), (Some(true), Some("ana@example.org")));
    // No plan beside the file: the result says so and does not call the address checked.
    assert_eq!(out["plan"]["state"], "none", "{out}");
    assert!(out["plan"]["why"].as_str().unwrap().contains("not checked against one"), "{out}");
    assert_eq!(out["in_plan"], Value::Null);
    // Another field is not touched.
    assert_eq!(read(&mut a, doc, "city")["signer"], Value::Null);

    // The label is in the saved file and survives a reopen.
    let bytes = saved(&dir, &mut a, doc, "out.pdf");
    assert!(has_label(&bytes));
    let again = tool(&mut a, "doc_open", json!({ "path": "out.pdf" }))["doc"].as_u64().unwrap();
    assert_eq!(read(&mut a, again, "name")["signer"], "ana@example.org");
    assert_eq!(read(&mut a, again, "city")["signer"], Value::Null);

    // One undo step each way.
    assert_eq!(tool(&mut a, "edit_undo", json!({ "doc": doc }))["undone"], "Set field signer");
    assert_eq!(read(&mut a, doc, "name")["signer"], Value::Null);
    assert!(!has_label(&saved(&dir, &mut a, doc, "undone.pdf")));
    tool(&mut a, "edit_redo", json!({ "doc": doc }));
    assert_eq!(read(&mut a, doc, "name")["signer"], "ana@example.org");

    // An empty string and a null both clear, each one step; clearing again changes nothing.
    assert_eq!(set(&mut a, doc, "name", json!(""))["changed"], true);
    assert_eq!(read(&mut a, doc, "name")["signer"], Value::Null);
    set(&mut a, doc, "name", json!("ana@example.org"));
    assert_eq!(set(&mut a, doc, "name", Value::Null)["changed"], true);
    assert_eq!(set(&mut a, doc, "name", Value::Null)["changed"], false);
    assert!(!has_label(&saved(&dir, &mut a, doc, "cleared.pdf")));
}

#[test]
fn a_call_that_changes_nothing_adds_no_undo_step() {
    let (_dir, mut a, doc) = setup();
    assert_eq!(set(&mut a, doc, "name", json!(""))["changed"], false);
    no_undo_step(&mut a, doc);
    set(&mut a, doc, "name", json!("ana@example.org"));
    assert_eq!(set(&mut a, doc, "name", json!(" ana@example.org "))["changed"], false, "the same label, trimmed, is no change");
    read(&mut a, doc, "name");
    tool(&mut a, "edit_undo", json!({ "doc": doc }));
    no_undo_step(&mut a, doc);
}

#[test]
fn the_label_survives_an_incremental_save_over_a_signed_file() {
    let (dir, mut a, doc) = setup();
    tool(&mut a, "sign_id_create", json!({ "name": "Ana Example", "key": "p256", "password": "secret12", "path": "ana.p12" }));
    tool(&mut a, "sign_document", json!({ "doc": doc, "id": "ana.p12", "password": "secret12", "field": "Sig2", "out": "signed.pdf" }));
    let signed = tool(&mut a, "doc_open", json!({ "path": "signed.pdf" }))["doc"].as_u64().unwrap();
    let before = std::fs::read(dir.path().join("signed.pdf")).unwrap();

    // The signed field is read but not labelled; an unsigned one in the same file is.
    let sig = read(&mut a, signed, "Sig2");
    assert_eq!((sig["signed"].as_bool(), sig["settable"].as_bool(), sig["signature_field"].as_bool()), (Some(true), Some(false), Some(true)));
    let m = refused(&mut a, "form_field_signer", json!({ "doc": signed, "field": "Sig2", "signer": "ana@example.org" }));
    assert!(m.contains("holds a signature"), "{m}");
    no_undo_step(&mut a, signed);
    assert_eq!(read(&mut a, signed, "Sig1")["settable"], true);

    set(&mut a, signed, "Sig1", json!("ana@example.org"));
    tool(&mut a, "doc_save", json!({ "doc": signed, "path": "signed.pdf" }));
    let after = std::fs::read(dir.path().join("signed.pdf")).unwrap();
    assert!(after.starts_with(&before), "an incremental save keeps the signed bytes in front");
    assert!(has_label(&after) && !has_label(&before));
    let reopened = tool(&mut a, "doc_open", json!({ "path": "signed.pdf" }))["doc"].as_u64().unwrap();
    assert_eq!(read(&mut a, reopened, "Sig1")["signer"], "ana@example.org");
    let audit = tool(&mut a, "signature_audit_report", json!({ "doc": reopened }));
    assert_ne!(audit["signatures"][0]["covers"], "changes_disallowed", "{audit}");
}

/// Sign `Sig2` of the setup form (certifying at `certify`, with `lock` as the field's lock first)
/// and reopen the result.
fn signed_form(certify: Option<&str>, lock: Option<Value>) -> (tempfile::TempDir, Automation, u64) {
    let (dir, mut a, doc) = setup();
    if let Some(mut lock) = lock {
        lock["doc"] = json!(doc);
        lock["field"] = json!("Sig2");
        tool(&mut a, "signature_field_lock", lock);
    }
    tool(&mut a, "sign_id_create", json!({ "name": "Ana Example", "key": "p256", "password": "secret12", "path": "ana.p12" }));
    let mut args = json!({ "doc": doc, "id": "ana.p12", "password": "secret12", "field": "Sig2", "out": "signed.pdf" });
    if let Some(c) = certify {
        args["certify"] = json!(c);
    }
    tool(&mut a, "sign_document", args);
    let signed = tool(&mut a, "doc_open", json!({ "path": "signed.pdf" }))["doc"].as_u64().unwrap();
    (dir, a, signed)
}

fn audit(a: &mut Automation, doc: u64) -> Value {
    let report = tool(a, "signature_audit_report", json!({ "doc": doc }));
    report["signatures"].as_array().unwrap().iter().find(|s| s["kind"] == "signature").unwrap().clone()
}

#[test]
fn a_label_that_the_signature_forbids_is_refused_and_leaves_it_whole() {
    // A certification that allows no changes: a label on another field is a change after it.
    let (dir, mut a, doc) = signed_form(Some("no_changes"), None);
    let before = audit(&mut a, doc);
    assert_eq!((before["covers"].as_str(), before["certification_level"].as_u64()), (Some("whole_file"), Some(1)), "{before}");
    let m = refused(&mut a, "form_field_signer", json!({ "doc": doc, "field": "name", "signer": "ana@example.org" }));
    assert!(m.contains("certification that allows no changes"), "{m}");
    no_undo_step(&mut a, doc);
    assert_eq!(read(&mut a, doc, "name")["signer"], Value::Null, "nothing was written");
    let after = audit(&mut a, doc);
    assert_eq!((after["covers"].clone(), after["status"].clone()), (before["covers"].clone(), before["status"].clone()), "{after}");
    assert!(after["changes_after_signing"].as_array().unwrap().is_empty(), "{after}");
    assert!(!has_label(&std::fs::read(dir.path().join("signed.pdf")).unwrap()));
}

#[test]
fn a_label_on_a_field_a_signatures_lock_names_is_refused_and_one_it_does_not_name_is_not() {
    let (_dir, mut a, doc) = signed_form(None, Some(json!({ "action": "include", "fields": ["name"] })));
    let before = audit(&mut a, doc);
    let m = refused(&mut a, "form_field_signer", json!({ "doc": doc, "field": "name", "signer": "ana@example.org" }));
    assert!(m.contains("covered by the lock"), "{m}");
    no_undo_step(&mut a, doc);
    assert_eq!(audit(&mut a, doc)["status"], before["status"]);
    // `city` is not named by the lock: written, and the signature is as it was. A refused label
    // afterwards leaves an undone step redoable.
    assert_eq!(set(&mut a, doc, "city", json!("ana@example.org"))["changed"], true);
    tool(&mut a, "edit_undo", json!({ "doc": doc }));
    refused(&mut a, "form_field_signer", json!({ "doc": doc, "field": "name", "signer": "ana@example.org" }));
    tool(&mut a, "edit_redo", json!({ "doc": doc }));
    assert_eq!(read(&mut a, doc, "city")["signer"], "ana@example.org");
    let after = audit(&mut a, doc);
    assert_eq!(after["status"], before["status"], "{after}");
    assert_eq!(after["covers"], "changes_allowed", "{after}");
    // A lock on every field refuses a label on any of them.
    let (_dir, mut a, doc) = signed_form(None, Some(json!({ "action": "all" })));
    let m = refused(&mut a, "form_field_signer", json!({ "doc": doc, "field": "city", "signer": "ana@example.org" }));
    assert!(m.contains("covered by the lock"), "{m}");
}

#[test]
fn a_label_is_written_where_the_signature_allows_form_filling() {
    for certify in [None, Some("form_fill")] {
        let (dir, mut a, doc) = signed_form(certify, None);
        let before = audit(&mut a, doc);
        assert_eq!(set(&mut a, doc, "name", json!("ana@example.org"))["changed"], true, "{certify:?}");
        let after = audit(&mut a, doc);
        assert_eq!(after["status"], before["status"], "{certify:?}: {after}");
        assert_eq!(after["covers"], "changes_allowed", "{certify:?}: {after}");
        assert!(has_label(&saved(&dir, &mut a, doc, "signed.pdf")));
        // A clear is the same kind of change and is allowed here too, and is one undo step.
        assert_eq!(set(&mut a, doc, "name", Value::Null)["changed"], true);
        tool(&mut a, "edit_undo", json!({ "doc": doc }));
        assert_eq!(read(&mut a, doc, "name")["signer"], "ana@example.org");
    }
}

#[test]
fn with_a_plan_only_a_signer_or_an_approver_of_it_can_be_named() {
    let (dir, mut a, doc) = setup();
    plan(&mut a);
    let before = saved(&dir, &mut a, doc, "form.pdf");
    for (who, what) in [
        ("zed@example.org", "is not a recipient of the signing plan"),
        ("cy@example.org", "is a viewer in the signing plan"),
        ("not-an-address", "must look like name@example.org"),
        ("ana @example.org", "must look like name@example.org"),
    ] {
        let m = refused(&mut a, "form_field_signer", json!({ "doc": doc, "field": "name", "signer": who }));
        assert!(m.contains(what), "{who}: {m}");
    }
    assert_eq!(saved(&dir, &mut a, doc, "form.pdf"), before, "refused calls wrote nothing");
    no_undo_step(&mut a, doc);

    // The plan's spelling is what is stored, however the caller spelled the address.
    let out = set(&mut a, doc, "name", json!("ANA@EXAMPLE.ORG"));
    assert_eq!(
        (out["signer"].as_str(), out["in_plan"].as_bool(), out["plan"]["state"].as_str()),
        (Some("Ana@example.org"), Some(true), Some("found"))
    );
    let approver = set(&mut a, doc, "city", json!("bo@example.org"));
    assert_eq!(approver["signer"], "bo@example.org");

    // A viewer a label already names (written before the plan said so) is reported, not hidden.
    tool(&mut a, "signing_plan_clear", json!({ "path": "form.pdf" }));
    set(&mut a, doc, "Sig1", json!("cy@example.org"));
    plan(&mut a);
    let report = tool(&mut a, "form_signer_report", json!({ "doc": doc }));
    let row = report["assigned"].as_array().unwrap().iter().find(|r| r["field"] == "Sig1").unwrap().clone();
    assert_eq!(row["plan_recipient"]["role"], "viewer", "{report}");
}

#[test]
fn without_a_plan_a_well_formed_address_is_accepted_and_says_it_is_unchecked() {
    let (_dir, mut a, doc) = setup();
    let out = set(&mut a, doc, "Sig1", json!("anyone@example.org"));
    assert_eq!(out["changed"], true);
    assert_eq!(out["plan"]["state"], "none");
    assert_eq!(out["signature_field"], true);
    for bad in ["anyone", "a@b", "@example.org", "a@@example.org", "a b@example.org"] {
        let m = refused(&mut a, "form_field_signer", json!({ "doc": doc, "field": "name", "signer": bad }));
        assert!(m.contains("must look like name@example.org"), "{bad}: {m}");
    }
    let long = format!("{}@example.org", "a".repeat(65));
    assert!(refused(&mut a, "form_field_signer", json!({ "doc": doc, "field": "name", "signer": long })).contains("at most 254 bytes"));
    for wrong in [json!(7), json!(true), json!(["a@example.org"])] {
        let m = refused(&mut a, "form_field_signer", json!({ "doc": doc, "field": "name", "signer": wrong }));
        assert!(m.contains("signer must be"), "{m}");
    }
    assert_eq!(read(&mut a, doc, "name")["signer"], Value::Null);
}

#[test]
fn a_plan_that_cannot_be_read_refuses_a_set_and_still_allows_a_read() {
    let (dir, mut a, doc) = setup();
    std::fs::write(dir.path().join("form.pdf.pdfcraft-sign.json"), b"not a plan").unwrap();
    let m = refused(&mut a, "form_field_signer", json!({ "doc": doc, "field": "name", "signer": "ana@example.org" }));
    assert!(m.contains("could not be read"), "{m}");
    no_undo_step(&mut a, doc);
    let r = read(&mut a, doc, "name");
    assert_eq!(r["plan"]["state"], "error", "{r}");
    // The report does not fail either: the plan parts are empty and the state says error.
    let report = tool(&mut a, "form_signer_report", json!({ "doc": doc }));
    assert_eq!(report["plan"]["state"], "error", "{report}");
    assert_eq!((report["signers_not_in_plan"].clone(), report["plan_recipients_without_fields"].clone()), (json!([]), json!([])));
    assert_eq!(report["fields"], 4);
    assert_eq!(std::fs::read(dir.path().join("form.pdf.pdfcraft-sign.json")).unwrap(), b"not a plan", "the file was left alone");
}

#[test]
fn an_unknown_or_shared_field_name_is_refused_and_nothing_is_written() {
    let (dir, mut a, doc) = setup();
    for (field, what) in [("Nobody", "no field named"), ("", "no field named"), ("nam", "no field named")] {
        let m = refused(&mut a, "form_field_signer", json!({ "doc": doc, "field": field, "signer": "ana@example.org" }));
        assert!(m.contains(what), "{field}: {m}");
    }
    no_undo_step(&mut a, doc);
    // Two fields with one name: refused for a read and a write, and the report marks them.
    let two = (0..2)
        .map(|i| format!("<< /Type /Annot /Subtype /Widget /FT /Tx /T (Dup) /F 4 /P 3 0 R /Rect [20 {} 120 {}] >>", 20 + 40 * i, 50 + 40 * i))
        .collect::<Vec<_>>();
    std::fs::write(dir.path().join("dup.pdf"), two_field_pdf(&two)).unwrap();
    let dup = tool(&mut a, "doc_open", json!({ "path": "dup.pdf" }))["doc"].as_u64().unwrap();
    for args in [json!({ "doc": dup, "field": "Dup" }), json!({ "doc": dup, "field": "Dup", "signer": "ana@example.org" })] {
        assert!(refused(&mut a, "form_field_signer", args).contains("names more than one field"));
    }
    no_undo_step(&mut a, dup);
    let report = tool(&mut a, "form_signer_report", json!({ "doc": dup }));
    assert!(report["unassigned"].as_array().unwrap().iter().all(|r| r["name_shared_by_fields"] == true), "{report}");
}

fn two_field_pdf(widgets: &[String]) -> Vec<u8> {
    let refs = (0..widgets.len()).map(|i| format!("{} 0 R", i + 4)).collect::<Vec<_>>().join(" ");
    let mut bodies = vec![
        format!("<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [{refs}] >> >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /Annots [{refs}] >>"),
    ];
    bodies.extend(widgets.iter().cloned());
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

#[test]
fn a_modify_restriction_refuses_a_write_and_still_reads_and_reports() {
    let (_dir, mut a, doc) = setup();
    set(&mut a, doc, "name", json!("ana@example.org"));
    tool(&mut a, "doc_protect", json!({ "doc": doc, "open_password": "open", "permissions_password": "boss", "changes": "none" }));
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "locked.pdf" }));
    let locked = tool(&mut a, "doc_open", json!({ "path": "locked.pdf", "password": "open" }))["doc"].as_u64().unwrap();
    for signer in [json!("bo@example.org"), Value::Null] {
        let m = refused(&mut a, "form_field_signer", json!({ "doc": locked, "field": "name", "signer": signer }));
        assert!(m.contains("allow") || m.contains("permission"), "{m}");
    }
    no_undo_step(&mut a, locked);
    assert_eq!(read(&mut a, locked, "name")["signer"], "ana@example.org", "a protected copy still reads");
    let report = tool(&mut a, "form_signer_report", json!({ "doc": locked }));
    assert_eq!(report["assigned_count"], 1);
}

#[test]
fn a_read_only_field_and_a_field_a_lock_names_can_be_labelled_when_the_document_allows() {
    let (dir, mut a, doc) = setup();
    tool(&mut a, "form_set_props", json!({ "doc": doc, "field": "name", "read_only": true }));
    tool(&mut a, "signature_field_lock", json!({ "doc": doc, "field": "Sig1", "action": "include", "fields": ["name", "city"] }));
    let out = set(&mut a, doc, "name", json!("ana@example.org"));
    assert_eq!(out["changed"], true, "a read-only flag stops filling, not the label: {out}");
    assert_eq!(set(&mut a, doc, "city", json!("ana@example.org"))["changed"], true, "a name a lock lists is not made read-only here");
    assert!(has_label(&saved(&dir, &mut a, doc, "out.pdf")));
}

#[test]
fn the_report_lists_fields_signers_unassigned_signature_fields_and_the_plans_gaps() {
    let (_dir, mut a, doc) = setup();
    // With no plan: assigned and unassigned only, and the plan parts empty.
    set(&mut a, doc, "name", json!("ana@example.org"));
    set(&mut a, doc, "Sig1", json!("zed@example.org"));
    let r = tool(&mut a, "form_signer_report", json!({ "doc": doc }));
    assert_eq!((r["fields"].as_u64(), r["assigned_count"].as_u64(), r["unassigned_count"].as_u64()), (Some(4), Some(2), Some(2)));
    assert_eq!(r["unassigned_signature_fields"], 1, "Sig2 has no signer");
    assert_eq!(r["plan"]["state"], "none");
    assert_eq!((r["signers_not_in_plan"].clone(), r["plan_recipients_without_fields"].clone()), (json!([]), json!([])));
    let sig1 = r["assigned"].as_array().unwrap().iter().find(|x| x["field"] == "Sig1").unwrap().clone();
    assert_eq!(sig1["signature_field"], true);
    assert_eq!(r["unassigned"].as_array().unwrap().iter().map(|x| x["field"].as_str().unwrap()).collect::<Vec<_>>().len(), 2);
    assert_eq!(r["label_only"], true);

    // With a plan: zed is not in it; bo (an approver) and cy (a viewer) have no field.
    plan(&mut a);
    let r = tool(&mut a, "form_signer_report", json!({ "doc": doc }));
    assert_eq!(r["plan"]["state"], "found");
    assert_eq!(r["signers_not_in_plan"], json!([{ "signer": "zed@example.org", "signer_cleaned": false, "fields": 1 }]));
    let gaps = r["plan_recipients_without_fields"].as_array().unwrap();
    assert_eq!(gaps.iter().map(|g| g["email"].as_str().unwrap()).collect::<Vec<_>>(), ["bo@example.org", "cy@example.org"]);
    assert_eq!(gaps[0]["has_a_turn"], true);
    assert_eq!(gaps[1]["has_a_turn"], false);
    let name_row = r["assigned"].as_array().unwrap().iter().find(|x| x["field"] == "name").unwrap().clone();
    assert_eq!(name_row["plan_recipient"]["name"], "Ana Example");
    // The report only reads.
    tool(&mut a, "edit_undo", json!({ "doc": doc }));
    tool(&mut a, "edit_undo", json!({ "doc": doc }));
    no_undo_step(&mut a, doc);
}

#[test]
fn a_label_that_is_not_a_string_is_reported_as_unreadable_and_can_be_replaced() {
    let (dir, mut a, _) = setup();
    let widgets = vec!["<< /Type /Annot /Subtype /Widget /FT /Tx /T (Odd) /PdfCraftSigner 7 /F 4 /P 3 0 R /Rect [20 20 120 50] >>".to_string()];
    std::fs::write(dir.path().join("odd.pdf"), two_field_pdf(&widgets)).unwrap();
    let doc = tool(&mut a, "doc_open", json!({ "path": "odd.pdf" }))["doc"].as_u64().unwrap();
    let r = read(&mut a, doc, "Odd");
    assert_eq!((r["signer"].clone(), r["not_text"].as_bool(), r["has_entry"].as_bool()), (Value::Null, Some(true), Some(true)));
    let report = tool(&mut a, "form_signer_report", json!({ "doc": doc }));
    assert_eq!(
        (report["unreadable_count"].as_u64(), report["assigned_count"].as_u64(), report["unassigned_count"].as_u64()),
        (Some(1), Some(0), Some(0))
    );
    assert_eq!(set(&mut a, doc, "Odd", json!("ana@example.org"))["signer"], "ana@example.org");
    assert_eq!(set(&mut a, doc, "Odd", json!(""))["changed"], true);
}

#[test]
fn an_empty_label_is_no_label_in_both_tools_and_a_clear_takes_it_out() {
    let (dir, mut a, _) = setup();
    let widgets = vec!["<< /Type /Annot /Subtype /Widget /FT /Tx /T (Blank) /PdfCraftSigner (  ) /F 4 /P 3 0 R /Rect [20 20 120 50] >>".to_string()];
    std::fs::write(dir.path().join("blank.pdf"), two_field_pdf(&widgets)).unwrap();
    let doc = tool(&mut a, "doc_open", json!({ "path": "blank.pdf" }))["doc"].as_u64().unwrap();
    let r = read(&mut a, doc, "Blank");
    assert_eq!((r["signer"].clone(), r["empty_label"].as_bool(), r["not_text"].as_bool()), (Value::Null, Some(true), Some(false)), "{r}");
    let report = tool(&mut a, "form_signer_report", json!({ "doc": doc }));
    assert_eq!(
        (report["assigned_count"].as_u64(), report["unassigned_count"].as_u64(), report["unreadable_count"].as_u64()),
        (Some(0), Some(1), Some(0)),
        "{report}"
    );
    // A clear takes the empty key out, one undo step; a set replaces it.
    assert_eq!(set(&mut a, doc, "Blank", Value::Null)["changed"], true);
    assert!(!has_label(&saved(&dir, &mut a, doc, "cleared.pdf")));
    assert_eq!(read(&mut a, doc, "Blank")["has_entry"], false);
    tool(&mut a, "edit_undo", json!({ "doc": doc }));
    assert_eq!(set(&mut a, doc, "Blank", json!("ana@example.org"))["changed"], true);
}

#[test]
fn hidden_characters_in_a_stored_label_are_not_echoed_and_the_label_is_flagged() {
    let (dir, mut a, _) = setup();
    // "ana", U+202E (right-to-left override), U+200B (zero width space), "@example.org", as UTF-16.
    let hex = "FEFF0061006E0061202E200B0040006500780061006D0070006C0065002E006F00720067";
    let widgets = vec![
        format!("<< /Type /Annot /Subtype /Widget /FT /Tx /T (Odd) /PdfCraftSigner <{hex}> /F 4 /P 3 0 R /Rect [20 20 120 50] >>"),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (Pad) /PdfCraftSigner ( ana@example.org ) /F 4 /P 3 0 R /Rect [20 140 120 170] >>".to_string(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (Plain) /PdfCraftSigner (plain) /F 4 /P 3 0 R /Rect [20 60 120 90] >>".to_string(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (Good) /PdfCraftSigner (good@example.org) /F 4 /P 3 0 R /Rect [20 100 120 130] >>".to_string(),
    ];
    std::fs::write(dir.path().join("odd.pdf"), two_field_pdf(&widgets)).unwrap();
    let doc = tool(&mut a, "doc_open", json!({ "path": "odd.pdf" }))["doc"].as_u64().unwrap();
    let r = read(&mut a, doc, "Odd");
    assert_eq!(
        (r["signer"].as_str(), r["signer_cleaned"].as_bool(), r["well_formed"].as_bool()),
        (Some("ana@example.org"), Some(true), Some(false)),
        "{r}"
    );
    assert!(!r.to_string().contains('\u{202E}') && !r.to_string().contains("\\u202e"), "{r}");
    let good = read(&mut a, doc, "Good");
    assert_eq!((good["signer_cleaned"].as_bool(), good["well_formed"].as_bool()), (Some(false), Some(true)), "{good}");
    assert_eq!(read(&mut a, doc, "Plain")["well_formed"], false);
    // Spaces round an address: the plan comparisons do not trim, so it is not called well formed.
    assert_eq!(read(&mut a, doc, "Pad")["well_formed"], false);
    let report = tool(&mut a, "form_signer_report", json!({ "doc": doc }));
    let rows = report["assigned"].as_array().unwrap();
    let row = |f: &str| rows.iter().find(|x| x["field"] == f).unwrap().clone();
    assert_eq!(
        (row("Odd")["signer"].as_str(), row("Odd")["signer_cleaned"].as_bool(), row("Odd")["well_formed"].as_bool()),
        (Some("ana@example.org"), Some(true), Some(false))
    );
    assert_eq!((row("Good")["signer_cleaned"].as_bool(), row("Good")["well_formed"].as_bool()), (Some(false), Some(true)));
}

#[test]
fn the_labels_not_in_the_plan_are_capped_and_the_report_says_it_was_cut() {
    let (dir, mut a, _) = setup();
    let widgets: Vec<String> = (0..505)
        .map(|i| {
            format!("<< /Type /Annot /Subtype /Widget /FT /Tx /T (F{i}) /PdfCraftSigner (u{i}@example.org) /F 4 /P 3 0 R /Rect [20 20 120 50] >>")
        })
        .collect();
    std::fs::write(dir.path().join("many.pdf"), two_field_pdf(&widgets)).unwrap();
    tool(&mut a, "signing_plan_set", json!({ "path": "many.pdf", "recipients": [{ "name": "Ana Example", "email": "ana@example.org" }] }));
    let doc = tool(&mut a, "doc_open", json!({ "path": "many.pdf" }))["doc"].as_u64().unwrap();
    let report = tool(&mut a, "form_signer_report", json!({ "doc": doc }));
    assert_eq!(report["signers_not_in_plan_count"], 505, "{}", report["plan"]);
    assert_eq!(report["signers_not_in_plan"].as_array().unwrap().len(), 500);
    assert_eq!(report["assigned"].as_array().unwrap().len(), 500);
    assert_eq!(report["truncated"], true);
}

#[test]
fn a_refused_label_keeps_the_oldest_undo_step_of_a_full_history() {
    let (_dir, mut a, doc) = signed_form(None, Some(json!({ "action": "include", "fields": ["name"] })));
    // 101 steps on `city` (which the lock does not name): the history holds the last 100.
    for i in 0..101 {
        set(&mut a, doc, "city", json!(format!("e{i}@example.org")));
    }
    let m = refused(&mut a, "form_field_signer", json!({ "doc": doc, "field": "name", "signer": "ana@example.org" }));
    assert!(m.contains("covered by the lock"), "{m}");
    let mut undone = 0;
    while a.call("edit_undo", &json!({ "doc": doc })).is_ok() {
        undone += 1;
    }
    assert_eq!(undone, 100, "the refusal took no step from the history");
    assert_eq!(read(&mut a, doc, "city")["signer"], "e0@example.org", "the oldest step kept is the one after the dropped first");
}

#[test]
fn the_refusal_names_its_cause() {
    let (_d, mut a, doc) = signed_form(Some("no_changes"), None);
    let m = refused(&mut a, "form_field_signer", json!({ "doc": doc, "field": "name", "signer": "ana@example.org" }));
    assert!(m.contains("certification that allows no changes") && m.contains("any field"), "{m}");
    let (_d, mut a, doc) = signed_form(None, Some(json!({ "action": "include", "fields": ["name"] })));
    let m = refused(&mut a, "form_field_signer", json!({ "doc": doc, "field": "name", "signer": "ana@example.org" }));
    assert!(m.contains("covered by the lock") && !m.contains("any field"), "{m}");
}
