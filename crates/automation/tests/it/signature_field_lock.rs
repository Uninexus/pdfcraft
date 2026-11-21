//! `signature_field_lock`: the `/Lock` of a signature field, read, set and removed, checked in the
//! saved file.

use pdfcraft_automation::{Automation, Content, ToolError};
use pdfcraft_cos::{Document, ObjRef, Object};
use pdfcraft_engine::Session;
use serde_json::{Value, json};
use std::sync::Arc;

fn tool(a: &mut Automation, name: &str, args: Value) -> Value {
    match a.call(name, &args).unwrap().remove(0) {
        Content::Json(value) => value,
        other => panic!("expected JSON: {other:?}"),
    }
}

/// A one-page file with text fields `name` and `address.city`, and signature fields `Sig1` and
/// `Sig2`, saved as a.pdf and reopened, so that no undo step is left.
fn setup() -> (tempfile::TempDir, Automation, u64) {
    let dir = tempfile::tempdir().unwrap();
    let mut session = Session::new();
    let bytes = session.create_blank(300.0, 400.0, 1).unwrap();
    let id = session.open("a.pdf", None, bytes, None).unwrap();
    std::fs::write(dir.path().join("a.pdf"), session.save_bytes(id).unwrap().as_slice()).unwrap();
    // Saves stamp /ModDate from the clock: pin it, or two saves a second apart differ.
    let mut a = Automation::new().with_root(dir.path()).unwrap().with_clock(|| 1_700_000_000);
    let doc = tool(&mut a, "doc_open", json!({ "path": "a.pdf" }))["doc"].as_u64().unwrap();
    for (ty, name, y) in [("text", "name", 20.0), ("text", "address.city", 60.0), ("signature", "Sig1", 100.0), ("signature", "Sig2", 160.0)] {
        tool(&mut a, "form_add_field", json!({ "doc": doc, "page": 1, "type": ty, "name": name, "rect": [20.0, y, 180.0, y + 30.0] }));
    }
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "form.pdf" }));
    let doc = tool(&mut a, "doc_open", json!({ "path": "form.pdf" }))["doc"].as_u64().unwrap();
    (dir, a, doc)
}

fn lock(a: &mut Automation, doc: u64, field: &str, rest: Value) -> Value {
    let mut args = json!({ "doc": doc, "field": field });
    for (k, v) in rest.as_object().unwrap() {
        args[k] = v.clone();
    }
    tool(a, "signature_field_lock", args)
}

fn refused(a: &mut Automation, args: Value) -> String {
    match a.call("signature_field_lock", &args) {
        Err(ToolError::InvalidArgs(m) | ToolError::Failed(m)) => m,
        other => panic!("not refused: {other:?}"),
    }
}

fn bytes(dir: &tempfile::TempDir, a: &mut Automation, doc: u64, name: &str) -> Vec<u8> {
    tool(a, "doc_save", json!({ "doc": doc, "path": name }));
    std::fs::read(dir.path().join(name)).unwrap()
}

/// The `/Lock` entries of the saved file's signature fields: (title, action, names).
fn saved_locks(bytes: Vec<u8>) -> Vec<(String, String, Vec<String>)> {
    let doc = Document::open(Arc::new(bytes)).unwrap();
    let mut out = Vec::new();
    for num in 1..200 {
        let o = doc.get(ObjRef { num, generation: 0 });
        let Some(d) = o.as_dict() else { continue };
        if d.name(b"FT") != Some(b"Sig".as_slice()) {
            continue;
        }
        let title = d.get(b"T").and_then(Object::as_string).map(|s| s.to_text()).unwrap_or_default();
        let Some(l) = d.get(b"Lock").map(|l| doc.resolve(l)) else { continue };
        let l = l.as_dict().unwrap();
        assert_eq!(l.name(b"Type"), Some(b"SigFieldLock".as_slice()));
        let names = l
            .get(b"Fields")
            .and_then(Object::as_array)
            .map(|a| a.iter().filter_map(|n| n.as_string().map(|s| s.to_text())).collect())
            .unwrap_or_default();
        out.push((title, String::from_utf8_lossy(l.name(b"Action").unwrap()).into_owned(), names));
    }
    out
}

#[test]
fn a_lock_is_read_set_saved_removed_undone_and_redone() {
    let (dir, mut a, doc) = setup();
    let none = lock(&mut a, doc, "Sig1", json!({}));
    assert!(none.get("changed").is_none(), "a read says nothing about changing: {none}");
    assert_eq!(none["field"], "Sig1");
    assert_eq!(
        none["lock"],
        json!({ "has_entry": false, "present": false, "action": null, "fields": [], "count": 0, "truncated": false, "not_names": 0, "exact": true, "signed": false, "settable": true, "capped": false })
    );
    let before = bytes(&dir, &mut a, doc, "before.pdf");

    let set = lock(&mut a, doc, "Sig1", json!({ "action": "include", "fields": ["name", "address"] }));
    assert_eq!(set["changed"], true);
    assert_eq!(set["lock"]["action"], "Include");
    assert_eq!(set["lock"]["fields"], json!(["name", "address"]));
    assert_eq!((set["lock"]["count"].as_u64(), set["lock"]["exact"].as_bool()), (Some(2), Some(true)));
    let included = bytes(&dir, &mut a, doc, "included.pdf");
    assert_eq!(saved_locks(included.clone()), [("Sig1".to_string(), "Include".to_string(), vec!["name".to_string(), "address".to_string()])]);
    // The other signature field and the text fields are not touched.
    assert_eq!(lock(&mut a, doc, "Sig2", json!({}))["lock"]["has_entry"], false);

    // A second set replaces the first: an All lock lists no name.
    let all = lock(&mut a, doc, "Sig1", json!({ "action": "all" }));
    assert_eq!((all["lock"]["action"].as_str(), all["lock"]["count"].as_u64()), (Some("All"), Some(0)));
    assert_eq!(saved_locks(bytes(&dir, &mut a, doc, "all.pdf")), [("Sig1".to_string(), "All".to_string(), vec![])]);

    // A removal takes /Lock out and is one undo step; a second changes nothing.
    let gone = lock(&mut a, doc, "Sig1", json!({ "remove": true }));
    assert_eq!((gone["changed"].as_bool(), gone["lock"]["has_entry"].as_bool()), (Some(true), Some(false)));
    let removed = bytes(&dir, &mut a, doc, "removed.pdf");
    assert!(saved_locks(removed.clone()).is_empty());
    let again = lock(&mut a, doc, "Sig1", json!({ "remove": true }));
    assert_eq!(again["changed"], false);
    assert_eq!(bytes(&dir, &mut a, doc, "again.pdf"), removed);

    // Each write was one labelled undo step: three, and a fourth finds nothing.
    for _ in 0..3 {
        assert_eq!(tool(&mut a, "edit_undo", json!({ "doc": doc }))["undone"], "Set signature field lock");
    }
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err());
    assert_eq!(bytes(&dir, &mut a, doc, "undone.pdf"), before);
    tool(&mut a, "edit_redo", json!({ "doc": doc }));
    assert_eq!(bytes(&dir, &mut a, doc, "redone.pdf"), included);
}

#[test]
fn a_call_that_cannot_be_applied_is_named_and_changes_nothing() {
    let (dir, mut a, doc) = setup();
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    let d = |extra: Value| {
        let mut args = json!({ "doc": doc, "field": "Sig1" });
        for (k, v) in extra.as_object().unwrap() {
            args[k] = v.clone();
        }
        args
    };
    let cases = [
        (d(json!({ "remove": true, "action": "all" })), "remove cannot be combined with action"),
        (d(json!({ "remove": true, "fields": ["name"] })), "remove cannot be combined with fields"),
        (d(json!({ "fields": ["name"] })), "fields needs an action"),
        (d(json!({ "action": "everything" })), "unknown action"),
        (d(json!({ "action": "all", "fields": ["name"] })), "lists no field names"),
        (d(json!({ "action": "include" })), "lists 1 to 256 names"),
        (d(json!({ "action": "exclude", "fields": [] })), "lists 1 to 256 names"),
        (d(json!({ "action": "include", "fields": ["name", "name"] })), "listed twice"),
        (d(json!({ "action": "include", "fields": ["nobody"] })), "no field named \"nobody\" to lock"),
        (d(json!({ "action": "include", "fields": ["addr"] })), "no field named \"addr\" to lock"),
        (d(json!({ "action": "include", "fields": "name" })), "fields"),
        (d(json!({ "remove": "yes" })), "remove"),
        (json!({ "doc": doc, "field": "Nobody" }), "no field named \"Nobody\""),
        (json!({ "doc": doc, "field": "name" }), "not a signature field"),
        (json!({ "doc": doc, "field": "address" }), "no field named \"address\""),
    ];
    for (args, what) in cases {
        let m = refused(&mut a, args.clone());
        assert!(m.contains(what), "{args}: {m}");
    }
    let long: Vec<String> = (0..257).map(|i| format!("f{i}")).collect();
    assert!(refused(&mut a, d(json!({ "action": "include", "fields": long }))).contains("lists 1 to 256 names"));
    assert_eq!(bytes(&dir, &mut a, doc, "after.pdf"), before, "nothing was written");
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "and no undo step was added");
}

/// Sig2 locks every field but `except` and is signed; then Sig1 is locked in the signed file and
/// the file saved incrementally. Returns the status the signature has afterwards.
fn sign_then_lock_the_other(except: &[&str]) -> String {
    let (dir, mut a, doc) = setup();
    tool(&mut a, "sign_id_create", json!({ "name": "Ada Lovelace", "key": "p256", "password": "secret12", "path": "ada.p12" }));
    lock(&mut a, doc, "Sig2", json!({ "action": "exclude", "fields": except }));
    tool(&mut a, "sign_document", json!({ "doc": doc, "id": "ada.p12", "password": "secret12", "field": "Sig2", "out": "signed.pdf" }));
    let signed = tool(&mut a, "doc_open", json!({ "path": "signed.pdf" }))["doc"].as_u64().unwrap();

    let read = lock(&mut a, signed, "Sig2", json!({}));
    assert_eq!((read["lock"]["signed"].as_bool(), read["lock"]["settable"].as_bool()), (Some(true), Some(false)), "{read}");
    assert_eq!(
        (read["lock"]["action"].as_str(), read["lock"]["fields"].clone()),
        (Some("Exclude"), json!(except)),
        "a signed field's lock is still read"
    );
    for rest in [json!({ "action": "all" }), json!({ "remove": true })] {
        let m = refused(&mut a, json!({ "doc": signed, "field": "Sig2", "action": rest["action"], "remove": rest["remove"] }));
        assert!(m.contains("holds a signature"), "{m}");
    }
    assert!(a.call("edit_undo", &json!({ "doc": signed })).is_err(), "no undo step was added");

    // The other field of the signed file is unsigned and can be locked: an incremental save on
    // top of the signature, with both locks in the file.
    let set = lock(&mut a, signed, "Sig1", json!({ "action": "all" }));
    assert_eq!((set["changed"].as_bool(), set["lock"]["settable"].as_bool()), (Some(true), Some(true)));
    tool(&mut a, "doc_save", json!({ "doc": signed, "path": "signed.pdf" }));
    assert_eq!(saved_locks(std::fs::read(dir.path().join("signed.pdf")).unwrap()).len(), 2);
    let listed = tool(&mut a, "sign_list", json!({ "doc": signed }));
    let sig2 = listed["signatures"].as_array().unwrap().iter().find(|s| s["field"] == "Sig2").unwrap().clone();
    sig2["status"].as_str().unwrap().to_owned()
}

#[test]
fn a_signed_field_is_read_but_not_edited_and_an_unsigned_one_still_is() {
    // Sig2 leaves Sig1 free, so locking Sig1 afterwards keeps the signature intact (the signer
    // is not trusted, which is "unknown"); when Sig2 locks Sig1, the change breaks it.
    assert_eq!(sign_then_lock_the_other(&["address.city", "Sig1"]), "unknown");
    assert_eq!(sign_then_lock_the_other(&["address.city"]), "invalid", "a lock written by the tool is the one the verifier enforces");
}

#[test]
fn a_modify_restriction_refuses_a_write_and_still_reads() {
    let (_dir, mut a, doc) = setup();
    lock(&mut a, doc, "Sig1", json!({ "action": "all" }));
    tool(&mut a, "doc_protect", json!({ "doc": doc, "open_password": "open", "permissions_password": "boss", "changes": "none" }));
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "locked.pdf" }));
    let locked = tool(&mut a, "doc_open", json!({ "path": "locked.pdf", "password": "open" }))["doc"].as_u64().unwrap();
    let m = refused(&mut a, json!({ "doc": locked, "field": "Sig1", "remove": true }));
    assert!(m.contains("allow") || m.contains("permission"), "{m}");
    assert_eq!(lock(&mut a, locked, "Sig1", json!({}))["lock"]["action"], "All", "a protected copy still reads");
}

/// A file with one signature widget field for each of `fields` (the entries of its dictionary
/// after `/FT /Sig`), objects 4 and up.
fn form_pdf(fields: &[String]) -> Vec<u8> {
    let refs: Vec<String> = (0..fields.len()).map(|i| format!("{} 0 R", i + 4)).collect();
    let refs = refs.join(" ");
    let mut bodies = vec![
        format!("<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [{refs}] >> >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /Annots [{refs}] >>"),
    ];
    for (i, f) in fields.iter().enumerate() {
        let y = 20 + 40 * i;
        bodies.push(format!("<< /Type /Annot /Subtype /Widget /FT /Sig {f} /F 4 /P 3 0 R /Rect [20 {y} 120 {}] >>", y + 30));
    }
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
fn a_lock_of_many_names_lists_the_first_256_and_counts_the_rest() {
    let (dir, mut a, _) = setup();
    // 300 entries: a number, a name of 128 characters, and 298 short names.
    let mut entries = vec!["7".to_string(), format!("({})", "n".repeat(128))];
    entries.extend((0..298).map(|i| format!("(f{i})")));
    let lock = format!("/T (Big) /Lock << /Type /SigFieldLock /Action /Include /Fields [{}] >>", entries.join(" "));
    std::fs::write(dir.path().join("big.pdf"), form_pdf(&[lock])).unwrap();
    let doc = tool(&mut a, "doc_open", json!({ "path": "big.pdf" }))["doc"].as_u64().unwrap();
    let read = &lock_of(&mut a, doc, "Big")["lock"];
    let names = read["fields"].as_array().unwrap();
    assert_eq!(names.len(), 255, "256 entries looked at, one of them a number");
    assert_eq!(names[0], "n".repeat(127), "a name of 128 characters is cut at 127");
    assert_eq!(names[254], "f253", "the 256th entry in file order is the last looked at");
    assert_eq!((read["count"].as_u64(), read["truncated"].as_bool(), read["not_names"].as_u64()), (Some(300), Some(true), Some(1)));
    assert_eq!((read["exact"].as_bool(), read["capped"].as_bool(), read["settable"].as_bool()), (Some(false), Some(false), Some(true)));
}

/// A file whose two signature fields are both named Dup.
fn duplicate_names_pdf() -> Vec<u8> {
    form_pdf(&["/T (Dup)".to_string(), "/T (Dup)".to_string()])
}

fn lock_of(a: &mut Automation, doc: u64, field: &str) -> Value {
    lock(a, doc, field, json!({}))
}

#[test]
fn a_name_two_fields_share_is_refused_rather_than_guessed() {
    let (dir, mut a, _) = setup();
    std::fs::write(dir.path().join("dup.pdf"), duplicate_names_pdf()).unwrap();
    let doc = tool(&mut a, "doc_open", json!({ "path": "dup.pdf" }))["doc"].as_u64().unwrap();
    for rest in [json!({}), json!({ "action": "all" }), json!({ "remove": true })] {
        let mut args = json!({ "doc": doc, "field": "Dup" });
        for (k, v) in rest.as_object().unwrap() {
            args[k] = v.clone();
        }
        let m = refused(&mut a, args);
        assert!(m.contains("names more than one field"), "{m}");
    }
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "nothing was written");
}
