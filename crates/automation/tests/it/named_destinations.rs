//! `named_destinations`: the document's named destinations (the `/Dests` name tree and the
//! catalog's `/Dests` dictionary), read, and one taken away by its position, checked in the
//! saved file.

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

/// A file with the names a and b in a leaf object (4, under the root 5), `direct` in a leaf
/// written inside the root, and `old` in the catalog's dictionary. `names_array` is object 6,
/// a /Names array of its own that the root's second kid holds.
fn named_pdf() -> Vec<u8> {
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R /Names << /Dests 5 0 R >> /Dests << /old [3 0 R /FitH 10] >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Names [(a) [3 0 R /XYZ null null null] (b) << /D [3 0 R /Fit] >>] /Limits [(a) (b)] >>",
        "<< /Kids [4 0 R << /Names [(direct) [3 0 R /Fit]] >> << /Names 6 0 R >>] >>",
        "[(own) [3 0 R /Fit]]",
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

fn read(v: &Value) -> &Value {
    &v["named_destinations"]
}

fn names(v: &Value) -> Vec<String> {
    read(v)["destinations"].as_array().unwrap().iter().map(|d| d["name"].as_str().unwrap().to_string()).collect()
}

fn bad(a: &mut Automation, args: Value) -> String {
    match a.call("named_destinations", &args) {
        Err(ToolError::InvalidArgs(m) | ToolError::Failed(m)) => m,
        other => panic!("refused: {other:?}"),
    }
}

fn bytes(dir: &tempfile::TempDir, a: &mut Automation, doc: u64, name: &str) -> Vec<u8> {
    tool(a, "doc_save", json!({ "doc": doc, "path": name }));
    std::fs::read(dir.path().join(name)).unwrap()
}

/// The saved file's names, read by the tool.
fn stored(dir: &tempfile::TempDir, a: &mut Automation, name: &str) -> Vec<String> {
    let doc = open(a, name);
    let _ = dir;
    names(&tool(a, "named_destinations", json!({ "doc": doc })))
}

#[test]
fn the_names_are_read_one_is_removed_saved_undone_and_redone() {
    let (dir, mut a) = setup();
    let plain = open(&mut a, "a.pdf");
    assert_eq!(
        read(&tool(&mut a, "named_destinations", json!({ "doc": plain }))),
        &json!({ "truncated": false, "tree_capped": false, "destinations": [], "unreadable": [] })
    );
    std::fs::write(dir.path().join("n.pdf"), named_pdf()).unwrap();
    let doc = open(&mut a, "n.pdf");
    let first = tool(&mut a, "named_destinations", json!({ "doc": doc }));
    assert!(first.get("removed").is_none(), "a read says nothing about removing: {first}");
    assert_eq!(
        read(&first),
        &json!({
            "truncated": false, "tree_capped": false, "unreadable": [],
            "destinations": [
                { "index": 0, "source": "names", "name": "a", "name_exact": true, "page": 1, "view": "XYZ", "removable": true },
                { "index": 1, "source": "names", "name": "b", "name_exact": true, "page": 1, "view": "Fit", "removable": true },
                { "index": 2, "source": "names", "name": "direct", "name_exact": true, "page": 1, "view": "Fit", "removable": false },
                { "index": 3, "source": "names", "name": "own", "name_exact": true, "page": 1, "view": "Fit", "removable": false },
                { "index": 4, "source": "dests", "name": "old", "name_exact": true, "page": 1, "view": "FitH", "removable": true },
            ]
        }),
        "{first}"
    );
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    let gone = tool(&mut a, "named_destinations", json!({ "doc": doc, "remove": 1 }));
    assert_eq!(gone["removed"], true);
    assert_eq!(names(&gone), ["a", "direct", "own", "old"]);
    let saved = bytes(&dir, &mut a, doc, "removed.pdf");
    assert_eq!(stored(&dir, &mut a, "removed.pdf"), ["a", "direct", "own", "old"]);

    // One undo step, labelled; the next undo has nothing to undo.
    assert_eq!(tool(&mut a, "edit_undo", json!({ "doc": doc }))["undone"], "Remove named destination");
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err());
    assert_eq!(names(&tool(&mut a, "named_destinations", json!({ "doc": doc }))).len(), 5);
    assert_eq!(bytes(&dir, &mut a, doc, "undone.pdf"), before);
    tool(&mut a, "edit_redo", json!({ "doc": doc }));
    assert_eq!(bytes(&dir, &mut a, doc, "redone.pdf"), saved);

    // A position with no entry changes nothing and adds no undo step.
    for index in [4, 5, 1_000_000] {
        let again = tool(&mut a, "named_destinations", json!({ "doc": doc, "remove": index }));
        assert_eq!(again["removed"], false, "{index}");
        assert_eq!(names(&again).len(), 4, "{index}");
    }
    assert_eq!(bytes(&dir, &mut a, doc, "again.pdf"), saved);
    assert_eq!(tool(&mut a, "edit_undo", json!({ "doc": doc }))["undone"], "Remove named destination");
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "the missing positions added no step");
}

#[test]
fn a_name_in_an_object_the_tool_does_not_change_is_refused_and_a_bad_argument_is_named() {
    let (dir, mut a) = setup();
    let plain = open(&mut a, "a.pdf");
    let before = bytes(&dir, &mut a, plain, "plain-before.pdf");
    assert_eq!(tool(&mut a, "named_destinations", json!({ "doc": plain, "remove": 0 }))["removed"], false);
    assert_eq!(bytes(&dir, &mut a, plain, "plain-after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": plain })).is_err(), "no step was added");

    std::fs::write(dir.path().join("n.pdf"), named_pdf()).unwrap();
    let doc = open(&mut a, "n.pdf");
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    for index in [2, 3] {
        assert_eq!(bad(&mut a, json!({ "doc": doc, "remove": index })), "that name is stored in an object this tool does not change (see removable)");
    }
    assert_eq!(bad(&mut a, json!({ "doc": doc, "remove": -1 })), "remove must be 0 or more");
    assert_eq!(bad(&mut a, json!({ "doc": doc, "remove": "0" })), "remove must be an integer");
    assert_eq!(bad(&mut a, json!({ "doc": doc, "remove": 0.5 })), "remove must be an integer");
    assert_eq!(bad(&mut a, json!({ "doc": doc, "remove": 18446744073709551615u64 })), "remove must be an integer");
    assert_eq!(tool(&mut a, "named_destinations", json!({ "doc": doc, "remove": i64::MAX }))["removed"], false);
    // A null is left out like any other argument: it reads.
    assert_eq!(names(&tool(&mut a, "named_destinations", json!({ "doc": doc, "remove": null }))).len(), 5);
    assert_eq!(bytes(&dir, &mut a, doc, "after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "no edit was made");
}

#[test]
fn the_dictionary_and_a_leaf_are_changed_where_they_are_and_a_protected_copy_is_left() {
    let (dir, mut a) = setup();
    std::fs::write(dir.path().join("n.pdf"), named_pdf()).unwrap();
    let doc = open(&mut a, "n.pdf");
    // The catalog's dictionary: the name goes, the dictionary stays in the catalog.
    assert_eq!(tool(&mut a, "named_destinations", json!({ "doc": doc, "remove": 4 }))["removed"], true);
    bytes(&dir, &mut a, doc, "dict.pdf");
    let reopened = Document::open(Arc::new(std::fs::read(dir.path().join("dict.pdf")).unwrap())).unwrap();
    let catalog = reopened.get(reopened.root().unwrap()).as_dict().cloned().unwrap();
    assert!(catalog.get(b"Dests").and_then(|o| o.as_dict()).is_some_and(|d| d.is_empty()));
    // Both names of the first leaf go, and its /Limits stay as they were written.
    for _ in 0..2 {
        assert_eq!(tool(&mut a, "named_destinations", json!({ "doc": doc, "remove": 0 }))["removed"], true);
    }
    bytes(&dir, &mut a, doc, "leaf.pdf");
    let reopened = Document::open(Arc::new(std::fs::read(dir.path().join("leaf.pdf")).unwrap())).unwrap();
    let leaf = reopened.scan_objects().filter_map(|(_, o)| o.as_dict().cloned()).find(|d| d.get(b"Limits").is_some()).unwrap();
    assert!(leaf.get(b"Names").and_then(|o| o.as_array()).is_some_and(|n| n.is_empty()));
    assert_eq!(stored(&dir, &mut a, "leaf.pdf"), ["direct", "own"]);

    std::fs::write(dir.path().join("locked-src.pdf"), named_pdf()).unwrap();
    let src = open(&mut a, "locked-src.pdf");
    tool(&mut a, "doc_protect", json!({ "doc": src, "open_password": "open", "permissions_password": "boss", "changes": "none" }));
    tool(&mut a, "doc_save", json!({ "doc": src, "path": "locked.pdf" }));
    let locked = tool(&mut a, "doc_open", json!({ "path": "locked.pdf", "password": "open" }))["doc"].as_u64().unwrap();
    match a.call("named_destinations", &json!({ "doc": locked, "remove": 0 })) {
        Err(ToolError::Failed(m)) => assert!(m.contains("allow") || m.contains("permission"), "{m}"),
        other => panic!("refused: {other:?}"),
    }
    assert_eq!(names(&tool(&mut a, "named_destinations", json!({ "doc": locked }))).len(), 5, "a protected copy still reads");
    assert_eq!(tool(&mut a, "named_destinations", json!({ "doc": locked, "remove": 9 }))["removed"], false);
}

#[test]
fn a_dictionary_that_is_the_page_tree_is_named_unreadable_and_never_written() {
    let (dir, mut a) = setup();
    let text = String::from_utf8(named_pdf()).unwrap().replace("/Dests << /old [3 0 R /FitH 10] >>", "/Dests 2 0 R");
    std::fs::write(dir.path().join("p.pdf"), text).unwrap();
    let doc = open(&mut a, "p.pdf");
    let before = bytes(&dir, &mut a, doc, "p-before.pdf");
    let r = tool(&mut a, "named_destinations", json!({ "doc": doc }));
    assert_eq!(names(&r), ["a", "b", "direct", "own"], "{r}");
    assert_eq!(read(&r)["unreadable"], json!(["dests"]));
    // Position 4 would be the page tree's first key: there is none, and nothing is written.
    let r = tool(&mut a, "named_destinations", json!({ "doc": doc, "remove": 4 }));
    assert_eq!(r["removed"], false);
    assert_eq!(bytes(&dir, &mut a, doc, "p-after.pdf"), before);
}
