//! `class_map`: the structure tree's class map (`/StructTreeRoot` `/ClassMap`), read, and an
//! entry taken away by its class name, checked in the saved file.

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
    let auto = Automation::new().with_root(dir.path()).unwrap();
    (dir, auto)
}

fn open(a: &mut Automation, path: &str) -> u64 {
    tool(a, "doc_open", json!({ "path": path }))["doc"].as_u64().unwrap()
}

/// A tagged file whose tree root has `tree_extra` beside `/Type` and `/K`; object 9 is an
/// attribute dictionary and object 10 an attribute stream.
fn tagged_pdf(tree_extra: &str) -> Vec<u8> {
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 5 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R >>".to_string(),
        "<< /Type /Page /Parent 2 0 R >>".to_string(),
        format!("<< /Type /StructTreeRoot /K 6 0 R {tree_extra} >>"),
        "<< /S /Document /K 7 0 R >>".to_string(),
        "<< /S /P /Pg 3 0 R /C /Boxed /ActualText (Report) >>".to_string(),
        "<< /Type /Placeholder >>".to_string(),
        "<< /O /Layout /Placement /Block >>".to_string(),
        "<< /O /Table /Length 1 >>\nstream\nx\nendstream".to_string(),
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

const MAP: &str =
    "/ClassMap << /Boxed 9 0 R /Grid 10 0 R /Both [9 0 R << /O /List >> 4] /Odd 7 /Plain << /Placement /Inline >> /Direct << /O /Layout >> >>";

/// The saved file's `/ClassMap`, resolved, and whether the tree root has the key at all.
fn stored(dir: &tempfile::TempDir, name: &str) -> (Option<pdfcraft_cos::Dict>, bool) {
    let doc = Document::open(Arc::new(std::fs::read(dir.path().join(name)).unwrap())).unwrap();
    let c = doc.get(doc.root().unwrap()).as_dict().cloned().unwrap();
    let root = c.get(b"StructTreeRoot").map(|o| doc.resolve(o)).unwrap();
    let root = root.as_dict().unwrap();
    (root.get(b"ClassMap").map(|o| doc.resolve(o)).and_then(|o| o.as_dict().cloned()), root.get(b"ClassMap").is_some())
}

fn bad(a: &mut Automation, args: Value) -> String {
    match a.call("class_map", &args) {
        Err(ToolError::InvalidArgs(m) | ToolError::Failed(m)) => m,
        other => panic!("refused: {other:?}"),
    }
}

fn map(v: &Value) -> &Value {
    &v["class_map"]
}

fn bytes(dir: &tempfile::TempDir, a: &mut Automation, doc: u64, name: &str) -> Vec<u8> {
    tool(a, "doc_save", json!({ "doc": doc, "path": name }));
    std::fs::read(dir.path().join(name)).unwrap()
}

#[test]
fn the_class_map_is_read_an_entry_is_removed_saved_undone_and_redone() {
    let (dir, mut a) = setup();
    let plain = open(&mut a, "a.pdf");
    assert_eq!(
        map(&tool(&mut a, "class_map", json!({ "doc": plain }))),
        &json!({ "has_structure_tree": false, "count": 0, "truncated": false, "entries": [], "unreadable": [] })
    );
    std::fs::write(dir.path().join("tagged.pdf"), tagged_pdf(MAP)).unwrap();
    let doc = open(&mut a, "tagged.pdf");
    let read = tool(&mut a, "class_map", json!({ "doc": doc }));
    assert!(read.get("removed").is_none(), "a read says nothing about removing: {read}");
    assert_eq!(
        map(&read),
        &json!({
            "has_structure_tree": true, "count": 6, "truncated": false, "unreadable": [],
            "entries": [
                { "class": "Boxed", "kind": "dictionary", "items": 1, "owners": ["Layout"], "exact": true },
                { "class": "Grid", "kind": "stream", "items": 1, "owners": ["Table"], "exact": true },
                { "class": "Both", "kind": "array", "items": 3, "owners": ["Layout", "List", null], "exact": true },
                { "class": "Odd", "kind": "other", "items": 0, "owners": [], "exact": true },
                { "class": "Plain", "kind": "dictionary", "items": 1, "owners": [null], "exact": true },
                { "class": "Direct", "kind": "dictionary", "items": 1, "owners": ["Layout"], "exact": true },
            ]
        }),
        "{read}"
    );
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    let gone = tool(&mut a, "class_map", json!({ "doc": doc, "remove": "Boxed" }));
    assert_eq!(gone["removed"], true);
    assert_eq!(map(&gone)["count"], 5);
    assert_eq!(map(&gone)["entries"][0]["class"], "Grid");
    let saved = bytes(&dir, &mut a, doc, "removed.pdf");
    let (stored_map, _) = stored(&dir, "removed.pdf");
    let stored_map = stored_map.unwrap();
    assert!(stored_map.get(b"Boxed").is_none() && stored_map.get(b"Grid").is_some() && stored_map.len() == 5);

    // One undo step, labelled; the next undo has nothing to undo.
    assert_eq!(tool(&mut a, "edit_undo", json!({ "doc": doc }))["undone"], "Remove class map entry");
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err());
    assert_eq!(map(&tool(&mut a, "class_map", json!({ "doc": doc })))["count"], 6);
    assert_eq!(bytes(&dir, &mut a, doc, "undone.pdf"), before);
    tool(&mut a, "edit_redo", json!({ "doc": doc }));
    assert_eq!(bytes(&dir, &mut a, doc, "redone.pdf"), saved);

    // Asking again for what is gone, or for a name that was never stored (even a different case
    // or an empty one), changes nothing and adds no undo step.
    for class in ["Boxed", "boxed", "Nothing", ""] {
        let again = tool(&mut a, "class_map", json!({ "doc": doc, "remove": class }));
        assert_eq!(again["removed"], false, "{class}");
        assert_eq!(map(&again)["count"], 5, "{class}");
    }
    assert_eq!(bytes(&dir, &mut a, doc, "again.pdf"), saved);
    // Removing the rest, one step each, takes the dictionary away and keeps the tree root.
    for class in ["Grid", "Both", "Odd", "Plain", "Direct"] {
        assert_eq!(tool(&mut a, "class_map", json!({ "doc": doc, "remove": class }))["removed"], true, "{class}");
    }
    let emptied = tool(&mut a, "class_map", json!({ "doc": doc }));
    assert_eq!(map(&emptied)["entries"], json!([]));
    bytes(&dir, &mut a, doc, "emptied.pdf");
    assert_eq!(stored(&dir, "emptied.pdf"), (None, false));
    for _ in 0..5 {
        assert_eq!(tool(&mut a, "edit_undo", json!({ "doc": doc }))["undone"], "Remove class map entry");
    }
}

#[test]
fn a_call_that_changes_nothing_writes_nothing_and_a_bad_argument_is_named() {
    let (dir, mut a) = setup();
    let plain = open(&mut a, "a.pdf");
    let before = bytes(&dir, &mut a, plain, "plain-before.pdf");
    let r = tool(&mut a, "class_map", json!({ "doc": plain, "remove": "A" }));
    assert_eq!(r["removed"], false);
    assert_eq!(bytes(&dir, &mut a, plain, "plain-after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": plain })).is_err(), "no step was added");

    std::fs::write(dir.path().join("tagged.pdf"), tagged_pdf(MAP)).unwrap();
    let doc = open(&mut a, "tagged.pdf");
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    assert_eq!(bad(&mut a, json!({ "doc": doc, "remove": 7 })), "remove must be a string");
    assert_eq!(bad(&mut a, json!({ "doc": doc, "remove": ["Boxed"] })), "remove must be a string");
    // A null is left out like any other argument: it reads.
    assert_eq!(map(&tool(&mut a, "class_map", json!({ "doc": doc, "remove": null })))["count"], 6);
    assert_eq!(bytes(&dir, &mut a, doc, "after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "no edit was made");
}

#[test]
fn an_unreadable_class_map_is_named_and_left_and_a_modify_restriction_holds() {
    let (dir, mut a) = setup();
    // A class map that is not a dictionary, and one that is a stream: named, never replaced, and
    // taking an entry from it is no change.
    for (name, extra) in [("odd", "/ClassMap 7"), ("stream", "/ClassMap 10 0 R"), ("tree", "/ClassMap 5 0 R"), ("catalog", "/ClassMap 1 0 R")] {
        std::fs::write(dir.path().join(format!("{name}.pdf")), tagged_pdf(extra)).unwrap();
        let doc = open(&mut a, &format!("{name}.pdf"));
        let before = bytes(&dir, &mut a, doc, &format!("{name}-before.pdf"));
        let r = tool(&mut a, "class_map", json!({ "doc": doc, "remove": "O" }));
        assert_eq!(
            map(&r),
            &json!({ "has_structure_tree": true, "count": 0, "truncated": false, "entries": [], "unreadable": ["class_map"] }),
            "{name}"
        );
        assert_eq!(r["removed"], false, "{name}");
        for key in ["Length", "K", "Pages", "Type"] {
            assert_eq!(tool(&mut a, "class_map", json!({ "doc": doc, "remove": key }))["removed"], false, "{name} {key}");
        }
        assert_eq!(bytes(&dir, &mut a, doc, &format!("{name}-after.pdf")), before, "{name}");
        assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "{name}");
    }

    std::fs::write(dir.path().join("locked-src.pdf"), tagged_pdf(MAP)).unwrap();
    let src = open(&mut a, "locked-src.pdf");
    tool(&mut a, "doc_protect", json!({ "doc": src, "open_password": "open", "permissions_password": "boss", "changes": "none" }));
    tool(&mut a, "doc_save", json!({ "doc": src, "path": "locked.pdf" }));
    let locked = tool(&mut a, "doc_open", json!({ "path": "locked.pdf", "password": "open" }))["doc"].as_u64().unwrap();
    match a.call("class_map", &json!({ "doc": locked, "remove": "Boxed" })) {
        Err(ToolError::Failed(m)) => assert!(m.contains("allow") || m.contains("permission"), "{m}"),
        other => panic!("refused: {other:?}"),
    }
    let read = tool(&mut a, "class_map", json!({ "doc": locked }));
    assert_eq!(map(&read)["count"], 6, "a protected copy still reads");
    // Nothing stored under the name: nothing to change, so nothing to refuse.
    assert_eq!(tool(&mut a, "class_map", json!({ "doc": locked, "remove": "Nothing" }))["removed"], false);
}
