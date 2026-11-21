//! `reference_xobjects`: the document's form XObjects with a `/Ref` read, and the `/Ref` of one
//! taken away by its position, checked in the saved file.

use pdfcraft_automation::{Automation, Content, ToolError};
use pdfcraft_cos::Document;
use pdfcraft_engine::Session;
use serde_json::{Value, json};
use std::sync::Arc;

fn tool(a: &mut Automation, name: &str, args: Value) -> Value {
    match a.call(name, &args).unwrap().remove(0) {
        Content::Json(value) => value,
        other => panic!("expected JSON: {other:?}"),
    }
}

fn setup() -> (tempfile::TempDir, Automation) {
    let dir = tempfile::tempdir().unwrap();
    let mut session = Session::new();
    let bytes = session.create_blank(300.0, 400.0, 2).unwrap();
    let id = session.open("a.pdf", None, bytes, None).unwrap();
    std::fs::write(dir.path().join("a.pdf"), session.save_bytes(id).unwrap().as_slice()).unwrap();
    // Saves stamp /ModDate from the clock: pin it, or two saves a second apart differ.
    let auto = Automation::new().with_root(dir.path()).unwrap().with_clock(|| 1_700_000_000);
    (dir, auto)
}

fn open(a: &mut Automation, path: &str) -> u64 {
    tool(a, "doc_open", json!({ "path": path }))["doc"].as_u64().unwrap()
}

/// A file whose page draws four forms: 4 and 7 have a direct `/Ref` (to a.pdf, page 0, and to
/// d.pdf, page 3), 5 has one behind a reference (a file specification with /UF, a page label and
/// an /ID), and 6 is an ordinary form. `page` is the body of the page object (object 3).
fn forms_pdf(page: &str) -> Vec<u8> {
    let form = |extra: &str| format!("<< /Subtype /Form /BBox [0 0 10 10] {extra} /Length 5 >>\nstream\n0 0 m\nendstream");
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_string(),
        page.to_string(),
        form("/Ref << /F (a.pdf) /Page 0 >>"),
        form("/Ref 8 0 R"),
        form(""),
        form("/Ref << /F (d.pdf) /Page 3 >>"),
        "<< /F << /Type /Filespec /F (b.pdf) /UF (b-uf.pdf) >> /Page (iv) /ID [(a) (b)] >>".to_string(),
    ];
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

const PAGE: &str = "<< /Type /Page /Parent 2 0 R /Resources << /XObject << /A 4 0 R /B 5 0 R /C 6 0 R /D 7 0 R >> >> >>";

fn read(v: &Value) -> &Value {
    &v["reference_xobjects"]
}

fn bad(a: &mut Automation, args: Value) -> String {
    match a.call("reference_xobjects", &args) {
        Err(ToolError::InvalidArgs(m) | ToolError::Failed(m)) => m,
        other => panic!("refused: {other:?}"),
    }
}

fn bytes(dir: &tempfile::TempDir, a: &mut Automation, doc: u64, name: &str) -> Vec<u8> {
    tool(a, "doc_save", json!({ "doc": doc, "path": name }));
    std::fs::read(dir.path().join(name)).unwrap()
}

/// The file names (the /UF, else the /F) of the /Ref of each form XObject of the saved file that
/// still has one, sorted. The save numbers the objects again, so objects are not named.
fn with_ref(dir: &tempfile::TempDir, name: &str) -> Vec<String> {
    let doc = Document::open(Arc::new(std::fs::read(dir.path().join(name)).unwrap())).unwrap();
    let text = |o: &pdfcraft_cos::Object| match &*doc.resolve(o) {
        pdfcraft_cos::Object::String(s) => Some(s.to_text()),
        _ => None,
    };
    let mut found: Vec<String> = doc
        .scan_objects()
        .filter_map(|(_, o)| {
            let reference = doc.resolve(o.as_dict()?.get(b"Ref")?);
            let reference = reference.as_dict()?;
            let file = doc.resolve(reference.get(b"F")?);
            match &*file {
                pdfcraft_cos::Object::Dict(spec) => spec.get(b"UF").and_then(text),
                other => text(other),
            }
        })
        .collect();
    found.sort();
    found
}

fn item(index: usize, object: &str, file: Value, page_index: Value, page_label: Value, has_id: bool) -> Value {
    json!({ "index": index, "object": object, "file": file, "page_index": page_index, "page_label": page_label, "has_id": has_id, "removable": true, "exact": true })
}

#[test]
fn the_reference_xobjects_are_read_one_is_removed_saved_undone_and_redone() {
    let (dir, mut a) = setup();
    let plain = open(&mut a, "a.pdf");
    assert_eq!(
        read(&tool(&mut a, "reference_xobjects", json!({ "doc": plain }))),
        &json!({ "count": 0, "truncated": false, "capped": false, "items": [] })
    );
    std::fs::write(dir.path().join("f.pdf"), forms_pdf(PAGE)).unwrap();
    let doc = open(&mut a, "f.pdf");
    let first = tool(&mut a, "reference_xobjects", json!({ "doc": doc }));
    assert!(first.get("removed").is_none(), "a read says nothing about removing: {first}");
    let all = json!([
        item(0, "4 0", json!("a.pdf"), json!(0), Value::Null, false),
        item(1, "5 0", json!("b-uf.pdf"), Value::Null, json!("iv"), true),
        item(2, "7 0", json!("d.pdf"), json!(3), Value::Null, false),
    ]);
    assert_eq!(read(&first), &json!({ "count": 3, "truncated": false, "capped": false, "items": all }), "{first}");
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    assert_eq!(with_ref(&dir, "before.pdf"), ["a.pdf", "b-uf.pdf", "d.pdf"]);

    let gone = tool(&mut a, "reference_xobjects", json!({ "doc": doc, "remove": 1 }));
    assert_eq!(gone["removed"], true);
    assert_eq!(read(&gone)["count"], 2);
    // The one after it takes its position (a save numbers the objects again, so the object is
    // not compared).
    let next = &read(&gone)["items"][1];
    assert!(next["index"] == 1 && next["file"] == "d.pdf" && next["page_index"] == 3, "{gone}");
    let saved = bytes(&dir, &mut a, doc, "removed.pdf");
    assert_eq!(with_ref(&dir, "removed.pdf"), ["a.pdf", "d.pdf"]);

    // One undo step, labelled; the next undo has nothing to undo.
    assert_eq!(tool(&mut a, "edit_undo", json!({ "doc": doc }))["undone"], "Remove reference XObject link");
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err());
    assert_eq!(read(&tool(&mut a, "reference_xobjects", json!({ "doc": doc })))["count"], 3);
    assert_eq!(bytes(&dir, &mut a, doc, "undone.pdf"), before);
    tool(&mut a, "edit_redo", json!({ "doc": doc }));
    assert_eq!(bytes(&dir, &mut a, doc, "redone.pdf"), saved);

    // A position with nothing listed changes nothing and adds no undo step.
    for index in [2, 3, 256, 1_000_000] {
        let again = tool(&mut a, "reference_xobjects", json!({ "doc": doc, "remove": index }));
        assert_eq!(again["removed"], false, "{index}");
        assert_eq!(read(&again)["count"], 2, "{index}");
    }
    assert_eq!(bytes(&dir, &mut a, doc, "again.pdf"), saved);
    // The first, then the last one left.
    for _ in 0..2 {
        assert_eq!(tool(&mut a, "reference_xobjects", json!({ "doc": doc, "remove": 0 }))["removed"], true);
    }
    bytes(&dir, &mut a, doc, "emptied.pdf");
    assert_eq!(with_ref(&dir, "emptied.pdf"), Vec::<String>::new());
    for _ in 0..3 {
        assert_eq!(tool(&mut a, "edit_undo", json!({ "doc": doc }))["undone"], "Remove reference XObject link");
    }
}

#[test]
fn a_call_that_changes_nothing_writes_nothing_and_a_bad_argument_is_named() {
    let (dir, mut a) = setup();
    let plain = open(&mut a, "a.pdf");
    let before = bytes(&dir, &mut a, plain, "plain-before.pdf");
    assert_eq!(tool(&mut a, "reference_xobjects", json!({ "doc": plain, "remove": 0 }))["removed"], false);
    assert_eq!(bytes(&dir, &mut a, plain, "plain-after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": plain })).is_err(), "no step was added");

    std::fs::write(dir.path().join("f.pdf"), forms_pdf(PAGE)).unwrap();
    let doc = open(&mut a, "f.pdf");
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    assert_eq!(bad(&mut a, json!({ "doc": doc, "remove": -1 })), "remove must be 0 or more");
    assert_eq!(bad(&mut a, json!({ "doc": doc, "remove": "0" })), "remove must be an integer");
    assert_eq!(bad(&mut a, json!({ "doc": doc, "remove": 0.5 })), "remove must be an integer");
    assert_eq!(bad(&mut a, json!({ "doc": doc, "remove": 18446744073709551615u64 })), "remove must be an integer");
    assert_eq!(tool(&mut a, "reference_xobjects", json!({ "doc": doc, "remove": i64::MAX }))["removed"], false);
    // A null is left out like any other argument: it reads.
    assert_eq!(read(&tool(&mut a, "reference_xobjects", json!({ "doc": doc, "remove": null })))["count"], 3);
    assert_eq!(bytes(&dir, &mut a, doc, "after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "no edit was made");
}

#[test]
fn a_form_that_is_a_page_keeps_its_ref_and_a_modify_restriction_holds() {
    let (dir, mut a) = setup();
    // The page object is itself a form XObject with a /Ref: listed as not removable, never changed.
    let page_stream =
        "<< /Type /Page /Parent 2 0 R /Subtype /Form /BBox [0 0 1 1] /Ref << /F (p.pdf) /Page 0 >> /Length 5 >>\nstream\n0 0 m\nendstream";
    std::fs::write(dir.path().join("page.pdf"), forms_pdf(page_stream)).unwrap();
    let doc = open(&mut a, "page.pdf");
    // The page is object 3, so it is listed first, with the three forms the file also holds
    // (unused: a save writes what the trailer reaches, so they are read before one).
    let read_only = tool(&mut a, "reference_xobjects", json!({ "doc": doc }));
    assert_eq!(read(&read_only)["count"], 4, "{read_only}");
    assert_eq!(read(&read_only)["items"][0]["file"], "p.pdf");
    let before = bytes(&dir, &mut a, doc, "page-before.pdf");
    assert_eq!(read(&read_only)["items"][0]["removable"], false);
    assert!(read(&read_only)["items"].as_array().unwrap()[1..].iter().all(|i| i["removable"] == true));
    // A refused position is an error, not "removed: false" like an empty one.
    let m = bad(&mut a, json!({ "doc": doc, "remove": 0 }));
    assert!(m.contains("see removable"), "{m}");
    assert_eq!(bytes(&dir, &mut a, doc, "page-after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err());

    std::fs::write(dir.path().join("locked-src.pdf"), forms_pdf(PAGE)).unwrap();
    let src = open(&mut a, "locked-src.pdf");
    tool(&mut a, "doc_protect", json!({ "doc": src, "open_password": "open", "permissions_password": "boss", "changes": "none" }));
    tool(&mut a, "doc_save", json!({ "doc": src, "path": "locked.pdf" }));
    let locked = tool(&mut a, "doc_open", json!({ "path": "locked.pdf", "password": "open" }))["doc"].as_u64().unwrap();
    match a.call("reference_xobjects", &json!({ "doc": locked, "remove": 0 })) {
        Err(ToolError::Failed(m)) => assert!(m.contains("allow") || m.contains("permission"), "{m}"),
        other => panic!("refused: {other:?}"),
    }
    assert_eq!(read(&tool(&mut a, "reference_xobjects", json!({ "doc": locked })))["count"], 3, "a protected copy still reads");
    // Nothing at that position: nothing to change, so nothing to refuse.
    assert_eq!(tool(&mut a, "reference_xobjects", json!({ "doc": locked, "remove": 5 }))["removed"], false);
}
