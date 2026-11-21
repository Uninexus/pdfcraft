//! `portfolio_collection`: the document's portfolio collection (the catalog's `/Collection`),
//! read, and taken out of the catalog, checked in the saved file.

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

/// A file with an embedded file, `collection` written into its catalog as `/Collection ...`.
/// Object 4 is a collection (details view, a.txt first) whose schema is object 5: two columns,
/// the second with its name behind a reference (6), and an entry that is not a column. Object 7 is
/// the file specification of the embedded file (8).
fn portfolio_pdf(collection: &str) -> Vec<u8> {
    portfolio_pdf_with(collection, "")
}

/// [`portfolio_pdf`] with `extra` written into the schema after its three entries.
fn portfolio_pdf_with(collection: &str, extra: &str) -> Vec<u8> {
    let bodies = [
        format!("<< /Type /Catalog /Pages 2 0 R {collection} /Names << /EmbeddedFiles << /Names [(a.txt) 7 0 R] >> >> >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R >>".to_string(),
        "<< /Type /Collection /View /T /D (a.txt) /Schema 5 0 R >>".to_string(),
        "<< /Type /CollectionSchema /Name << /Type /CollectionField /Subtype /S /N (Name) /O 0 >> \
         /Size << /Type /CollectionField /Subtype /Size /N 6 0 R /O 2 /V false /E true >> /Junk 7 {extra} >>"
            .replace("{extra}", extra),
        "(File size)".to_string(),
        "<< /Type /Filespec /F (a.txt) /EF << /F 8 0 R >> >>".to_string(),
        "<< /Type /EmbeddedFile /Length 3 >>\nstream\nhi\n\nendstream".to_string(),
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

const COLLECTION: &str = "/Collection 4 0 R";

fn read(v: &Value) -> &Value {
    &v["collection"]
}

fn bad(a: &mut Automation, args: Value) -> String {
    match a.call("portfolio_collection", &args) {
        Err(ToolError::InvalidArgs(m) | ToolError::Failed(m)) => m,
        other => panic!("refused: {other:?}"),
    }
}

fn bytes(dir: &tempfile::TempDir, a: &mut Automation, doc: u64, name: &str) -> Vec<u8> {
    tool(a, "doc_save", json!({ "doc": doc, "path": name }));
    std::fs::read(dir.path().join(name)).unwrap()
}

/// Whether the saved file's catalog has `/Collection`, and whether it still has its embedded file
/// (`/Names`).
fn catalog_has(bytes: Vec<u8>) -> (bool, bool) {
    let doc = Document::open(Arc::new(bytes)).unwrap();
    let catalog = doc.get(doc.root().unwrap());
    let catalog = catalog.as_dict().unwrap();
    (catalog.contains(b"Collection"), catalog.contains(b"Names"))
}

#[test]
fn the_collection_is_read_removed_saved_undone_and_redone() {
    let (dir, mut a) = setup();
    let plain = open(&mut a, "a.pdf");
    let none = tool(&mut a, "portfolio_collection", json!({ "doc": plain }));
    assert!(none.get("removed").is_none(), "a read says nothing about removing: {none}");
    assert_eq!(
        read(&none),
        &json!({ "present": false, "has_entry": false, "view": null, "initial_document": null, "initial_document_cut": false, "count": 0, "truncated": false, "capped": false, "fields": [] })
    );

    std::fs::write(dir.path().join("p.pdf"), portfolio_pdf(COLLECTION)).unwrap();
    let doc = open(&mut a, "p.pdf");
    let first = tool(&mut a, "portfolio_collection", json!({ "doc": doc }));
    let fields = json!([
        { "key": "Name", "is_field": true, "name": "Name", "subtype": "S", "order": 0, "visible": true, "editable": false, "exact": true },
        { "key": "Size", "is_field": true, "name": "File size", "subtype": "Size", "order": 2, "visible": false, "editable": true, "exact": true },
        { "key": "Junk", "is_field": false, "name": null, "subtype": null, "order": null, "visible": true, "editable": false, "exact": true },
    ]);
    let expected = json!({ "present": true, "has_entry": true, "view": "T", "initial_document": "a.txt", "initial_document_cut": false, "count": 3, "truncated": false, "capped": false, "fields": fields });
    assert_eq!(read(&first), &expected, "{first}");
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    assert_eq!(catalog_has(before.clone()), (true, true));

    // `remove: false` only reads.
    let kept = tool(&mut a, "portfolio_collection", json!({ "doc": doc, "remove": false }));
    assert!(kept.get("removed").is_none() && read(&kept) == &expected);
    assert_eq!(bytes(&dir, &mut a, doc, "kept.pdf"), before);

    let gone = tool(&mut a, "portfolio_collection", json!({ "doc": doc, "remove": true }));
    assert_eq!(gone["removed"], true);
    assert_eq!(read(&gone)["present"], false);
    assert_eq!(read(&gone)["has_entry"], false);
    assert_eq!(read(&gone)["count"], 0);
    let saved = bytes(&dir, &mut a, doc, "removed.pdf");
    assert_eq!(catalog_has(saved.clone()), (false, true), "the embedded files stay");

    // A second removal has nothing to take: no change, no undo step.
    let again = tool(&mut a, "portfolio_collection", json!({ "doc": doc, "remove": true }));
    assert_eq!(again["removed"], false);
    assert_eq!(bytes(&dir, &mut a, doc, "again.pdf"), saved);

    // One undo step, labelled; the next undo has nothing to undo.
    assert_eq!(tool(&mut a, "edit_undo", json!({ "doc": doc }))["undone"], "Remove portfolio collection");
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err());
    assert_eq!(read(&tool(&mut a, "portfolio_collection", json!({ "doc": doc }))), &expected);
    assert_eq!(bytes(&dir, &mut a, doc, "undone.pdf"), before);
    tool(&mut a, "edit_redo", json!({ "doc": doc }));
    assert_eq!(bytes(&dir, &mut a, doc, "redone.pdf"), saved);
}

#[test]
fn a_schema_of_many_columns_lists_the_first_256_and_counts_the_rest() {
    let (dir, mut a) = setup();
    // The three columns of the fixture and 300 more.
    let extra: String = (0..300).map(|i| format!("/c{i} << /Subtype /S /O {i} >> ")).collect();
    std::fs::write(dir.path().join("many.pdf"), portfolio_pdf_with(COLLECTION, &extra)).unwrap();
    let doc = open(&mut a, "many.pdf");
    let r = tool(&mut a, "portfolio_collection", json!({ "doc": doc }));
    let (fields, count) = (read(&r)["fields"].as_array().unwrap(), &read(&r)["count"]);
    assert_eq!((fields.len(), count, &read(&r)["truncated"], &read(&r)["capped"]), (256, &json!(303), &json!(true), &json!(false)), "{r}");
    assert_eq!(fields[255]["order"], 252, "the first ones in file order: Name, Size, Junk, then c0");

    // A `/D` of 128 characters is cut at 127 and says so; one of 127 does not.
    for (len, cut) in [(128, true), (127, false)] {
        let direct = format!("/Collection << /View /C /D ({}) >>", "d".repeat(len));
        std::fs::write(dir.path().join("long.pdf"), portfolio_pdf(&direct)).unwrap();
        let long = open(&mut a, "long.pdf");
        let r = tool(&mut a, "portfolio_collection", json!({ "doc": long }));
        assert_eq!(
            (read(&r)["initial_document_cut"].as_bool(), read(&r)["initial_document"].as_str().map(str::len), read(&r)["view"].as_str()),
            (Some(cut), Some(127), Some("C"))
        );
    }
}

#[test]
fn a_call_that_changes_nothing_writes_nothing_and_a_bad_argument_is_named() {
    let (dir, mut a) = setup();
    let plain = open(&mut a, "a.pdf");
    let before = bytes(&dir, &mut a, plain, "plain-before.pdf");
    assert_eq!(tool(&mut a, "portfolio_collection", json!({ "doc": plain, "remove": true }))["removed"], false);
    assert_eq!(bytes(&dir, &mut a, plain, "plain-after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": plain })).is_err(), "no step was added");

    // A null `/Collection` is no entry; one that is a number is an entry that is no portfolio,
    // which a removal takes out all the same.
    std::fs::write(dir.path().join("null.pdf"), portfolio_pdf("/Collection null")).unwrap();
    let null_doc = open(&mut a, "null.pdf");
    assert_eq!(read(&tool(&mut a, "portfolio_collection", json!({ "doc": null_doc })))["has_entry"], false);
    let null_before = bytes(&dir, &mut a, null_doc, "null-before.pdf");
    assert_eq!(tool(&mut a, "portfolio_collection", json!({ "doc": null_doc, "remove": true }))["removed"], false);
    assert_eq!(bytes(&dir, &mut a, null_doc, "null-after.pdf"), null_before);
    assert!(a.call("edit_undo", &json!({ "doc": null_doc })).is_err(), "no step was added");
    std::fs::write(dir.path().join("num.pdf"), portfolio_pdf("/Collection 5")).unwrap();
    let num = open(&mut a, "num.pdf");
    let r = tool(&mut a, "portfolio_collection", json!({ "doc": num }));
    assert!(read(&r)["has_entry"] == true && read(&r)["present"] == false, "{r}");
    assert_eq!(tool(&mut a, "portfolio_collection", json!({ "doc": num, "remove": true }))["removed"], true);

    std::fs::write(dir.path().join("p.pdf"), portfolio_pdf(COLLECTION)).unwrap();
    let doc = open(&mut a, "p.pdf");
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    assert_eq!(bad(&mut a, json!({ "doc": doc, "remove": "true" })), "remove must be true or false");
    assert_eq!(bad(&mut a, json!({ "doc": doc, "remove": 1 })), "remove must be true or false");
    // A null is left out like any other argument: it reads.
    assert_eq!(read(&tool(&mut a, "portfolio_collection", json!({ "doc": doc, "remove": null })))["present"], true);
    assert_eq!(bytes(&dir, &mut a, doc, "after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "no edit was made");
}

#[test]
fn a_collection_that_is_a_page_tree_node_is_read_but_only_the_catalog_changes() {
    let (dir, mut a) = setup();
    // `/Collection` is the `/Pages` node itself (object 2): read as written, and a removal takes
    // the catalog's entry and leaves the node, so the pages stay.
    std::fs::write(dir.path().join("t.pdf"), portfolio_pdf("/Collection 2 0 R")).unwrap();
    let doc = open(&mut a, "t.pdf");
    let r = tool(&mut a, "portfolio_collection", json!({ "doc": doc }));
    assert!(read(&r)["present"] == true && read(&r)["count"] == 0, "{r}");
    assert_eq!(tool(&mut a, "portfolio_collection", json!({ "doc": doc, "remove": true }))["removed"], true);
    let saved = bytes(&dir, &mut a, doc, "t-removed.pdf");
    assert_eq!(catalog_has(saved.clone()), (false, true));
    let reopened = Document::open(Arc::new(saved)).unwrap();
    let catalog = reopened.get(reopened.root().unwrap());
    let node = reopened.get(catalog.as_dict().unwrap().reference(b"Pages").unwrap());
    assert_eq!(node.as_dict().unwrap().get(b"Kids").and_then(|k| k.as_array()).map(Vec::len), Some(1), "the /Pages node keeps its /Kids");
}

#[test]
fn a_modify_restriction_refuses_a_removal_and_still_reads() {
    let (dir, mut a) = setup();
    std::fs::write(dir.path().join("locked-src.pdf"), portfolio_pdf(COLLECTION)).unwrap();
    let src = open(&mut a, "locked-src.pdf");
    tool(&mut a, "doc_protect", json!({ "doc": src, "open_password": "open", "permissions_password": "boss", "changes": "none" }));
    tool(&mut a, "doc_save", json!({ "doc": src, "path": "locked.pdf" }));
    let locked = tool(&mut a, "doc_open", json!({ "path": "locked.pdf", "password": "open" }))["doc"].as_u64().unwrap();
    match a.call("portfolio_collection", &json!({ "doc": locked, "remove": true })) {
        Err(ToolError::Failed(m)) => assert!(m.contains("allow") || m.contains("permission"), "{m}"),
        other => panic!("refused: {other:?}"),
    }
    assert_eq!(read(&tool(&mut a, "portfolio_collection", json!({ "doc": locked })))["present"], true, "a protected copy still reads");
}
