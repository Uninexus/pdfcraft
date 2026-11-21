//! `signature_seed_values`: the `/SV` of a signature field, read, set and removed, checked in the
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

/// A one-page file with a text field `name` and signature fields `Sig1` and `Sig2`, saved as
/// a.pdf and reopened, so that no undo step is left.
fn setup() -> (tempfile::TempDir, Automation, u64) {
    let dir = tempfile::tempdir().unwrap();
    let mut session = Session::new();
    let bytes = session.create_blank(300.0, 400.0, 1).unwrap();
    let id = session.open("a.pdf", None, bytes, None).unwrap();
    std::fs::write(dir.path().join("a.pdf"), session.save_bytes(id).unwrap().as_slice()).unwrap();
    // Saves stamp /ModDate from the clock: pin it, or two saves a second apart differ.
    let mut a = Automation::new().with_root(dir.path()).unwrap().with_clock(|| 1_700_000_000);
    let doc = tool(&mut a, "doc_open", json!({ "path": "a.pdf" }))["doc"].as_u64().unwrap();
    for (ty, name, y) in [("text", "name", 20.0), ("signature", "Sig1", 100.0), ("signature", "Sig2", 160.0)] {
        tool(&mut a, "form_add_field", json!({ "doc": doc, "page": 1, "type": ty, "name": name, "rect": [20.0, y, 180.0, y + 30.0] }));
    }
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "form.pdf" }));
    let doc = tool(&mut a, "doc_open", json!({ "path": "form.pdf" }))["doc"].as_u64().unwrap();
    (dir, a, doc)
}

fn seed(a: &mut Automation, doc: u64, field: &str, rest: Value) -> Value {
    let mut args = json!({ "doc": doc, "field": field });
    for (k, v) in rest.as_object().unwrap() {
        args[k] = v.clone();
    }
    tool(a, "signature_seed_values", args)
}

fn refused(a: &mut Automation, args: Value) -> String {
    match a.call("signature_seed_values", &args) {
        Err(ToolError::InvalidArgs(m) | ToolError::Failed(m)) => m,
        other => panic!("not refused: {other:?}"),
    }
}

fn bytes(dir: &tempfile::TempDir, a: &mut Automation, doc: u64, name: &str) -> Vec<u8> {
    tool(a, "doc_save", json!({ "doc": doc, "path": name }));
    std::fs::read(dir.path().join(name)).unwrap()
}

/// The `/SV` entries of the saved file's signature fields: (title, /Ff, reasons, sub-filters,
/// digest methods, /AddRevInfo).
type Saved = (String, Option<i64>, Vec<String>, Vec<String>, Vec<String>, Option<bool>);

fn saved_seeds(bytes: Vec<u8>) -> Vec<Saved> {
    let doc = Document::open(Arc::new(bytes)).unwrap();
    let mut out = Vec::new();
    for num in 1..200 {
        let o = doc.get(ObjRef { num, generation: 0 });
        let Some(d) = o.as_dict() else { continue };
        if d.name(b"FT") != Some(b"Sig".as_slice()) {
            continue;
        }
        let title = d.get(b"T").and_then(Object::as_string).map(|s| s.to_text()).unwrap_or_default();
        let Some(sv) = d.get(b"SV").map(|l| doc.resolve(l)) else { continue };
        let sv = sv.as_dict().unwrap();
        assert_eq!(sv.name(b"Type"), Some(b"SV".as_slice()));
        let texts = |key: &[u8]| -> Vec<String> {
            sv.get(key)
                .and_then(Object::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|n| match n {
                            Object::String(s) => Some(s.to_text()),
                            Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let rev = match sv.get(b"AddRevInfo") {
            Some(Object::Bool(b)) => Some(*b),
            _ => None,
        };
        out.push((title, sv.int(b"Ff"), texts(b"Reasons"), texts(b"SubFilter"), texts(b"DigestMethod"), rev));
    }
    out
}

#[test]
fn seed_values_are_read_set_saved_removed_undone_and_redone() {
    let (dir, mut a, doc) = setup();
    let none = seed(&mut a, doc, "Sig1", json!({}));
    assert!(none.get("changed").is_none(), "a read says nothing about changing: {none}");
    assert_eq!(none["field"], "Sig1");
    let empty = json!({ "items": [], "count": 0, "truncated": false, "skipped": 0 });
    assert_eq!(
        none["seed_values"],
        json!({
            "has_entry": false, "present": false, "flags": null, "required": [], "reasons": empty, "sub_filters": empty, "digest_methods": empty,
            "add_rev_info": null, "other_entries": 0, "exact": true, "signed": false, "settable": true, "capped": false
        })
    );
    let before = bytes(&dir, &mut a, doc, "before.pdf");

    let set = seed(
        &mut a,
        doc,
        "Sig1",
        json!({ "reasons": ["I approve", "I wrote it"], "sub_filters": ["adbe.pkcs7.detached"], "digest_methods": ["SHA256", "SHA512"], "add_rev_info": true, "required": ["reasons", "digest_method"] }),
    );
    assert_eq!(set["changed"], true);
    let read = &set["seed_values"];
    assert_eq!((read["flags"].as_i64(), read["required"].clone()), (Some(72), json!(["reasons", "digest_method"])));
    assert_eq!(read["reasons"]["items"], json!(["I approve", "I wrote it"]));
    assert_eq!(read["sub_filters"]["items"], json!(["adbe.pkcs7.detached"]));
    assert_eq!(
        (read["digest_methods"]["count"].as_u64(), read["add_rev_info"].as_bool(), read["exact"].as_bool()),
        (Some(2), Some(true), Some(true))
    );
    let first = bytes(&dir, &mut a, doc, "first.pdf");
    assert_eq!(
        saved_seeds(first.clone()),
        [(
            "Sig1".to_string(),
            Some(72),
            vec!["I approve".to_string(), "I wrote it".to_string()],
            vec!["adbe.pkcs7.detached".to_string()],
            vec!["SHA256".to_string(), "SHA512".to_string()],
            Some(true)
        )]
    );
    // The other signature field is not touched.
    assert_eq!(seed(&mut a, doc, "Sig2", json!({}))["seed_values"]["has_entry"], false);

    // A second set replaces the first: nothing required writes no /Ff, and what it does not give is gone.
    let second = seed(&mut a, doc, "Sig1", json!({ "reasons": ["Reviewed"] }));
    assert_eq!((second["seed_values"]["flags"].clone(), second["seed_values"]["sub_filters"]["count"].as_u64()), (Value::Null, Some(0)));
    assert_eq!(saved_seeds(bytes(&dir, &mut a, doc, "second.pdf")), [("Sig1".to_string(), None, vec!["Reviewed".to_string()], vec![], vec![], None)]);

    // A removal takes /SV out and is one undo step; a second changes nothing.
    let gone = seed(&mut a, doc, "Sig1", json!({ "remove": true }));
    assert_eq!((gone["changed"].as_bool(), gone["seed_values"]["has_entry"].as_bool()), (Some(true), Some(false)));
    let removed = bytes(&dir, &mut a, doc, "removed.pdf");
    assert!(saved_seeds(removed.clone()).is_empty());
    let again = seed(&mut a, doc, "Sig1", json!({ "remove": true }));
    assert_eq!(again["changed"], false);
    assert_eq!(bytes(&dir, &mut a, doc, "again.pdf"), removed);

    // Each write was one labelled undo step: three, and a fourth finds nothing.
    for _ in 0..3 {
        assert_eq!(tool(&mut a, "edit_undo", json!({ "doc": doc }))["undone"], "Set signature field seed values");
    }
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err());
    assert_eq!(bytes(&dir, &mut a, doc, "undone.pdf"), before);
    tool(&mut a, "edit_redo", json!({ "doc": doc }));
    assert_eq!(bytes(&dir, &mut a, doc, "redone.pdf"), first);
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
        (d(json!({ "remove": true, "reasons": ["a"] })), "remove cannot be combined"),
        (d(json!({ "remove": true, "required": ["reasons"] })), "remove cannot be combined"),
        (d(json!({ "remove": true, "add_rev_info": false })), "remove cannot be combined"),
        (d(json!({ "required": ["reasons"] })), "seed values with no entry"),
        (d(json!({ "reasons": ["a"], "required": ["filter"] })), "unknown entry \"filter\""),
        (d(json!({ "reasons": ["a"], "required": ["digest_method"] })), "cannot be required without being given"),
        (d(json!({ "reasons": ["a"], "required": ["reasons", "reasons"] })), "required twice"),
        (d(json!({ "reasons": ["a", "a"] })), "listed twice"),
        (d(json!({ "reasons": [""] })), "a reason is 1 to 127 characters"),
        (d(json!({ "reasons": ["r".repeat(128)] })), "a reason is 1 to 127 characters"),
        (d(json!({ "sub_filters": ["adbe.pkcs7.rsa"] })), "unknown sub-filter"),
        (d(json!({ "sub_filters": ["ADBE.PKCS7.DETACHED"] })), "unknown sub-filter"),
        (d(json!({ "digest_methods": ["SHA1"] })), "unknown digest method"),
        (d(json!({ "digest_methods": ["SHA256", "SHA256"] })), "listed twice"),
        (d(json!({ "reasons": [] })), "reasons is empty"),
        (d(json!({ "sub_filters": [] })), "sub_filters is empty"),
        (d(json!({ "digest_methods": [] })), "digest_methods is empty"),
        (d(json!({ "reasons": ["a"], "required": [] })), "required is empty"),
        (d(json!({ "sub_filters": ["adbe.x509.rsa_sha1"], "add_rev_info": true })), "revocation information needs"),
        (d(json!({ "reasons": "a" })), "reasons"),
        (d(json!({ "add_rev_info": "yes" })), "add_rev_info"),
        (d(json!({ "remove": "yes" })), "remove"),
        (json!({ "doc": doc, "field": "Nobody" }), "no field named \"Nobody\""),
        (json!({ "doc": doc, "field": "name" }), "not a signature field"),
    ];
    for (args, what) in cases {
        let m = refused(&mut a, args.clone());
        assert!(m.contains(what), "{args}: {m}");
    }
    let long: Vec<String> = (0..17).map(|i| format!("r{i}")).collect();
    assert!(refused(&mut a, d(json!({ "reasons": long }))).contains("at most 16 reasons"));
    assert_eq!(bytes(&dir, &mut a, doc, "after.pdf"), before, "nothing was written");
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "and no undo step was added");
}

#[test]
fn a_signed_field_is_read_but_not_edited_and_an_unsigned_one_still_is() {
    let (dir, mut a, doc) = setup();
    tool(&mut a, "sign_id_create", json!({ "name": "Ada Lovelace", "key": "p256", "password": "secret12", "path": "ada.p12" }));
    seed(&mut a, doc, "Sig2", json!({ "reasons": ["I approve"], "required": ["reasons"] }));
    tool(&mut a, "sign_document", json!({ "doc": doc, "id": "ada.p12", "password": "secret12", "field": "Sig2", "out": "signed.pdf" }));
    let signed = tool(&mut a, "doc_open", json!({ "path": "signed.pdf" }))["doc"].as_u64().unwrap();
    let read = seed(&mut a, signed, "Sig2", json!({}));
    assert_eq!((read["seed_values"]["signed"].as_bool(), read["seed_values"]["settable"].as_bool()), (Some(true), Some(false)), "{read}");
    assert_eq!(read["seed_values"]["reasons"]["items"], json!(["I approve"]), "a signed field's seed values are still read");
    for rest in [json!({ "reasons": ["other"] }), json!({ "remove": true })] {
        let mut args = json!({ "doc": signed, "field": "Sig2" });
        for (k, v) in rest.as_object().unwrap() {
            args[k] = v.clone();
        }
        assert!(refused(&mut a, args).contains("holds a signature"));
    }
    assert!(a.call("edit_undo", &json!({ "doc": signed })).is_err(), "no undo step was added");
    // The other field of the signed file is unsigned and can be set, and the file saved with both fields'
    // seed values. Whether the signature is still valid after that is not asserted (sign_list is not read).
    let set = seed(&mut a, signed, "Sig1", json!({ "digest_methods": ["SHA384"] }));
    assert_eq!((set["changed"].as_bool(), set["seed_values"]["settable"].as_bool()), (Some(true), Some(true)));
    tool(&mut a, "doc_save", json!({ "doc": signed, "path": "signed.pdf" }));
    assert_eq!(saved_seeds(std::fs::read(dir.path().join("signed.pdf")).unwrap()).len(), 2);
}

#[test]
fn a_modify_restriction_refuses_a_write_and_still_reads() {
    let (_dir, mut a, doc) = setup();
    seed(&mut a, doc, "Sig1", json!({ "reasons": ["a"] }));
    tool(&mut a, "doc_protect", json!({ "doc": doc, "open_password": "open", "permissions_password": "boss", "changes": "none" }));
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "locked.pdf" }));
    let locked = tool(&mut a, "doc_open", json!({ "path": "locked.pdf", "password": "open" }))["doc"].as_u64().unwrap();
    let m = refused(&mut a, json!({ "doc": locked, "field": "Sig1", "remove": true }));
    assert!(m.contains("allow") || m.contains("permission"), "{m}");
    assert_eq!(seed(&mut a, locked, "Sig1", json!({}))["seed_values"]["reasons"]["items"], json!(["a"]), "a protected copy still reads");
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
fn a_file_made_elsewhere_is_listed_by_what_it_holds_and_a_set_over_unmodelled_entries_is_refused() {
    let (dir, mut a, _) = setup();
    // 20 reasons, of which one is a number; a name that a set would refuse; a /Cert; a flag.
    let mut reasons = vec!["7".to_string()];
    reasons.extend((0..19).map(|i| format!("(r{i})")));
    let sv = format!(
        "/T (Big) /SV << /Type /SV /Ff 66 /Reasons [{}] /SubFilter [/adbe.pkcs7.rsa /ETSI.CAdES.detached] /Cert << /Type /SVCert >> /Filter /Adobe.PPKLite >>",
        reasons.join(" ")
    );
    std::fs::write(dir.path().join("big.pdf"), form_pdf(&[sv])).unwrap();
    let doc = tool(&mut a, "doc_open", json!({ "path": "big.pdf" }))["doc"].as_u64().unwrap();
    let read = &seed(&mut a, doc, "Big", json!({}))["seed_values"];
    assert_eq!(read["reasons"]["items"].as_array().unwrap().len(), 15, "16 entries looked at, one of them a number");
    assert_eq!(read["reasons"]["items"][14], "r14", "the 16th entry in file order is the last looked at");
    assert_eq!(
        (read["reasons"]["count"].as_u64(), read["reasons"]["truncated"].as_bool(), read["reasons"]["skipped"].as_u64()),
        (Some(20), Some(true), Some(1))
    );
    assert_eq!(read["sub_filters"]["items"], json!(["adbe.pkcs7.rsa", "ETSI.CAdES.detached"]), "a name a set would refuse is listed");
    assert_eq!(
        (read["flags"].as_i64(), read["required"].clone(), read["other_entries"].as_u64()),
        (Some(66), json!(["sub_filter", "digest_method"]), Some(2))
    );
    let m = refused(&mut a, json!({ "doc": doc, "field": "Big", "reasons": ["x"] }));
    assert!(m.contains("entries this does not write"), "{m}");
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "nothing was written");
    let gone = seed(&mut a, doc, "Big", json!({ "remove": true }));
    assert_eq!((gone["changed"].as_bool(), gone["seed_values"]["has_entry"].as_bool()), (Some(true), Some(false)));
}

#[test]
fn a_name_two_fields_share_is_refused_rather_than_guessed() {
    let (dir, mut a, _) = setup();
    std::fs::write(dir.path().join("dup.pdf"), form_pdf(&["/T (Dup)".to_string(), "/T (Dup)".to_string()])).unwrap();
    let doc = tool(&mut a, "doc_open", json!({ "path": "dup.pdf" }))["doc"].as_u64().unwrap();
    for rest in [json!({}), json!({ "reasons": ["a"] }), json!({ "remove": true })] {
        let mut args = json!({ "doc": doc, "field": "Dup" });
        for (k, v) in rest.as_object().unwrap() {
            args[k] = v.clone();
        }
        let m = refused(&mut a, args);
        assert!(m.contains("names more than one field"), "{m}");
    }
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "nothing was written");
}
