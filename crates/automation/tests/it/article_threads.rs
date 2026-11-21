//! `article_threads`: the document's article threads (the catalog's `/Threads`), read, and one
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

/// A file whose catalog has `threads` as its `/Threads`. Objects 4 and 5 are two threads
/// ("Intro", with two beads, and an untitled one with one bead), 6 to 8 their beads.
fn threaded_pdf(threads: &str) -> Vec<u8> {
    let bodies = [
        format!("<< /Type /Catalog /Pages 2 0 R {threads} >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /B [6 0 R 7 0 R 8 0 R] >>".to_string(),
        "<< /F 6 0 R /I << /Title (Intro) >> >>".to_string(),
        "<< /F 8 0 R >>".to_string(),
        "<< /T 4 0 R /P 3 0 R /R [0 0 10 10] /N 7 0 R /V 7 0 R >>".to_string(),
        "<< /T 4 0 R /P 3 0 R /R [0 0 10 10] /N 6 0 R /V 6 0 R >>".to_string(),
        "<< /T 5 0 R /P 3 0 R /R [0 0 10 10] /N 8 0 R /V 8 0 R >>".to_string(),
        "[4 0 R 5 0 R]".to_string(),
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

const THREADS: &str = "/Threads [4 0 R 5 0 R]";

fn read(v: &Value) -> &Value {
    &v["article_threads"]
}

fn bad(a: &mut Automation, args: Value) -> String {
    match a.call("article_threads", &args) {
        Err(ToolError::InvalidArgs(m) | ToolError::Failed(m)) => m,
        other => panic!("refused: {other:?}"),
    }
}

fn bytes(dir: &tempfile::TempDir, a: &mut Automation, doc: u64, name: &str) -> Vec<u8> {
    tool(a, "doc_save", json!({ "doc": doc, "path": name }));
    std::fs::read(dir.path().join(name)).unwrap()
}

/// The saved file's `/Threads`, resolved, as the title of each thread ("-" for none), and the
/// reopened document.
fn stored(dir: &tempfile::TempDir, name: &str) -> (Option<Vec<String>>, Document) {
    let doc = Document::open(Arc::new(std::fs::read(dir.path().join(name)).unwrap())).unwrap();
    let c = doc.get(doc.root().unwrap()).as_dict().cloned().unwrap();
    let title = |item: &pdfcraft_cos::Object| {
        let t = doc.resolve(item);
        let info = t.as_dict().and_then(|d| d.get(b"I")).map(|i| doc.resolve(i));
        let text = info.as_deref().and_then(|i| i.as_dict()).and_then(|d| d.get(b"Title"));
        match text {
            Some(pdfcraft_cos::Object::String(s)) => s.to_text(),
            _ => "-".to_string(),
        }
    };
    let list = c.get(b"Threads").map(|o| doc.resolve(o)).and_then(|o| o.as_array().map(|a| a.iter().map(title).collect()));
    (list, doc)
}

/// How many objects of the file are beads (dictionaries with a `/T`) and threads (with an `/F`).
fn beads_and_threads(doc: &Document) -> (usize, usize) {
    let dicts: Vec<_> = doc.scan_objects().filter_map(|(_, o)| o.as_dict().cloned()).collect();
    (dicts.iter().filter(|d| d.get(b"T").is_some()).count(), dicts.iter().filter(|d| d.get(b"F").is_some()).count())
}

#[test]
fn the_threads_are_read_one_is_removed_saved_undone_and_redone() {
    let (dir, mut a) = setup();
    let plain = open(&mut a, "a.pdf");
    assert_eq!(
        read(&tool(&mut a, "article_threads", json!({ "doc": plain }))),
        &json!({ "count": 0, "truncated": false, "threads": [], "unreadable": [] })
    );
    std::fs::write(dir.path().join("t.pdf"), threaded_pdf(THREADS)).unwrap();
    let doc = open(&mut a, "t.pdf");
    let first = tool(&mut a, "article_threads", json!({ "doc": doc }));
    assert!(first.get("removed").is_none(), "a read says nothing about removing: {first}");
    assert_eq!(
        read(&first),
        &json!({
            "count": 2, "truncated": false, "unreadable": [],
            "threads": [
                { "index": 0, "title": "Intro", "title_exact": true, "beads": 2, "beads_capped": false, "closed": true },
                { "index": 1, "title": null, "title_exact": true, "beads": 1, "beads_capped": false, "closed": true },
            ]
        }),
        "{first}"
    );
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    let gone = tool(&mut a, "article_threads", json!({ "doc": doc, "remove": 0 }));
    assert_eq!(gone["removed"], true);
    assert_eq!(read(&gone)["count"], 1);
    assert_eq!(
        read(&gone)["threads"][0],
        json!({ "index": 0, "title": null, "title_exact": true, "beads": 1, "beads_capped": false, "closed": true })
    );
    let saved = bytes(&dir, &mut a, doc, "removed.pdf");
    let (list, reopened) = stored(&dir, "removed.pdf");
    assert_eq!(list, Some(vec!["-".to_string()]));
    // The removed thread and the beads are still in the file.
    assert_eq!(beads_and_threads(&reopened), (3, 2));

    // One undo step, labelled; the next undo has nothing to undo.
    assert_eq!(tool(&mut a, "edit_undo", json!({ "doc": doc }))["undone"], "Remove article thread");
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err());
    assert_eq!(read(&tool(&mut a, "article_threads", json!({ "doc": doc })))["count"], 2);
    assert_eq!(bytes(&dir, &mut a, doc, "undone.pdf"), before);
    tool(&mut a, "edit_redo", json!({ "doc": doc }));
    assert_eq!(bytes(&dir, &mut a, doc, "redone.pdf"), saved);

    // A position with no entry changes nothing and adds no undo step.
    for index in [1, 2, 1_000_000] {
        let again = tool(&mut a, "article_threads", json!({ "doc": doc, "remove": index }));
        assert_eq!(again["removed"], false, "{index}");
        assert_eq!(read(&again)["count"], 1, "{index}");
    }
    assert_eq!(bytes(&dir, &mut a, doc, "again.pdf"), saved);
    // The last one takes `/Threads` out of the catalog.
    assert_eq!(tool(&mut a, "article_threads", json!({ "doc": doc, "remove": 0 }))["removed"], true);
    bytes(&dir, &mut a, doc, "emptied.pdf");
    assert_eq!(stored(&dir, "emptied.pdf").0, None);
    for _ in 0..2 {
        assert_eq!(tool(&mut a, "edit_undo", json!({ "doc": doc }))["undone"], "Remove article thread");
    }
}

#[test]
fn a_call_that_changes_nothing_writes_nothing_and_a_bad_argument_is_named() {
    let (dir, mut a) = setup();
    let plain = open(&mut a, "a.pdf");
    let before = bytes(&dir, &mut a, plain, "plain-before.pdf");
    assert_eq!(tool(&mut a, "article_threads", json!({ "doc": plain, "remove": 0 }))["removed"], false);
    assert_eq!(bytes(&dir, &mut a, plain, "plain-after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": plain })).is_err(), "no step was added");

    std::fs::write(dir.path().join("t.pdf"), threaded_pdf(THREADS)).unwrap();
    let doc = open(&mut a, "t.pdf");
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    assert_eq!(bad(&mut a, json!({ "doc": doc, "remove": -1 })), "remove must be 0 or more");
    assert_eq!(bad(&mut a, json!({ "doc": doc, "remove": "0" })), "remove must be an integer");
    assert_eq!(bad(&mut a, json!({ "doc": doc, "remove": 0.5 })), "remove must be an integer");
    assert_eq!(bad(&mut a, json!({ "doc": doc, "remove": 18446744073709551615u64 })), "remove must be an integer");
    assert_eq!(tool(&mut a, "article_threads", json!({ "doc": doc, "remove": i64::MAX }))["removed"], false);
    // A null is left out like any other argument: it reads.
    assert_eq!(read(&tool(&mut a, "article_threads", json!({ "doc": doc, "remove": null })))["count"], 2);
    assert_eq!(bytes(&dir, &mut a, doc, "after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "no edit was made");
}

#[test]
fn an_array_behind_a_reference_is_changed_where_it_is_and_an_unreadable_entry_is_left() {
    let (dir, mut a) = setup();
    std::fs::write(dir.path().join("via.pdf"), threaded_pdf("/Threads 9 0 R")).unwrap();
    let doc = open(&mut a, "via.pdf");
    assert_eq!(read(&tool(&mut a, "article_threads", json!({ "doc": doc })))["count"], 2);
    assert_eq!(tool(&mut a, "article_threads", json!({ "doc": doc, "remove": 1 }))["removed"], true);
    bytes(&dir, &mut a, doc, "via-removed.pdf");
    let (list, saved) = stored(&dir, "via-removed.pdf");
    assert_eq!(list, Some(vec!["Intro".to_string()]));
    let catalog = saved.get(saved.root().unwrap()).as_dict().cloned().unwrap();
    assert!(catalog.get(b"Threads").is_some_and(|o| o.as_ref().is_some()), "the catalog still refers to the array object");

    for (name, extra) in [("number", "/Threads 7"), ("dict", "/Threads << /F 6 0 R >>"), ("page", "/Threads 6 0 R")] {
        std::fs::write(dir.path().join(format!("{name}.pdf")), threaded_pdf(extra)).unwrap();
        let doc = open(&mut a, &format!("{name}.pdf"));
        let before = bytes(&dir, &mut a, doc, &format!("{name}-before.pdf"));
        let r = tool(&mut a, "article_threads", json!({ "doc": doc, "remove": 0 }));
        assert_eq!(read(&r), &json!({ "count": 0, "truncated": false, "threads": [], "unreadable": ["threads"] }), "{name}");
        assert_eq!(r["removed"], false, "{name}");
        assert_eq!(bytes(&dir, &mut a, doc, &format!("{name}-after.pdf")), before, "{name}");
        assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "{name}");
    }

    std::fs::write(dir.path().join("locked-src.pdf"), threaded_pdf(THREADS)).unwrap();
    let src = open(&mut a, "locked-src.pdf");
    tool(&mut a, "doc_protect", json!({ "doc": src, "open_password": "open", "permissions_password": "boss", "changes": "none" }));
    tool(&mut a, "doc_save", json!({ "doc": src, "path": "locked.pdf" }));
    let locked = tool(&mut a, "doc_open", json!({ "path": "locked.pdf", "password": "open" }))["doc"].as_u64().unwrap();
    match a.call("article_threads", &json!({ "doc": locked, "remove": 0 })) {
        Err(ToolError::Failed(m)) => assert!(m.contains("allow") || m.contains("permission"), "{m}"),
        other => panic!("refused: {other:?}"),
    }
    assert_eq!(read(&tool(&mut a, "article_threads", json!({ "doc": locked })))["count"], 2, "a protected copy still reads");
    // Nothing at that position: nothing to change, so nothing to refuse.
    assert_eq!(tool(&mut a, "article_threads", json!({ "doc": locked, "remove": 5 }))["removed"], false);
}
