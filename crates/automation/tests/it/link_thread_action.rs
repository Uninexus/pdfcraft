//! `link_thread_action`: the article (thread) action of a link, read, set and removed, checked in
//! the saved file.

use pdfcraft_automation::{Automation, Content, ToolError};
use pdfcraft_cos::{Document, Object};
use serde_json::{Value, json};
use std::sync::Arc;

fn tool(a: &mut Automation, name: &str, args: Value) -> Value {
    match a.call(name, &args).unwrap().remove(0) {
        Content::Json(value) => value,
        other => panic!("expected JSON: {other:?}"),
    }
}

fn setup() -> (tempfile::TempDir, Automation, u64) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.pdf"), linked_pdf()).unwrap();
    // Saves stamp /ModDate from the clock: pin it, or two saves a second apart differ.
    let mut auto = Automation::new().with_root(dir.path()).unwrap().with_clock(|| 1_700_000_000);
    let doc = tool(&mut auto, "doc_open", json!({ "path": "a.pdf" }))["doc"].as_u64().unwrap();
    (dir, auto, doc)
}

/// Two threads (objects 4 and 5; the first has the beads 6 and 7, the second the bead 8) and a page
/// with five links: 9 has a web action, 10 a destination, 11 nothing, 12 another web action, and
/// the fifth is written in the page (an inline link, which has no object).
fn linked_pdf() -> Vec<u8> {
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R /Threads [4 0 R 5 0 R] >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Annots [9 0 R 10 0 R 11 0 R 12 0 R << /Type /Annot /Subtype /Link /Rect [0 0 5 5] /A << /S /URI /URI (http://z.test) >> >>] >>".to_string(),
        "<< /F 6 0 R /I << /Title (Intro) >> >>".to_string(),
        "<< /F 8 0 R /I << /Title (Second) >> >>".to_string(),
        "<< /T 4 0 R /P 3 0 R /R [0 0 10 10] /N 7 0 R /V 7 0 R >>".to_string(),
        "<< /T 4 0 R /P 3 0 R /R [0 0 20 20] /N 6 0 R /V 6 0 R >>".to_string(),
        "<< /T 5 0 R /P 3 0 R /R [0 0 30 30] /N 8 0 R /V 8 0 R >>".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 10 50 50] /A << /S /URI /URI (http://x.test) >> >>".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 60 50 90] /Dest [3 0 R /Fit] >>".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 100 50 130] >>".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 140 50 170] /A << /S /URI /URI (http://y.test) >> >>".to_string(),
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

fn bad(a: &mut Automation, args: Value) -> String {
    match a.call("link_thread_action", &args) {
        Err(ToolError::InvalidArgs(m) | ToolError::Failed(m)) => m,
        other => panic!("refused: {other:?}"),
    }
}

fn bytes(dir: &tempfile::TempDir, a: &mut Automation, doc: u64, name: &str) -> Vec<u8> {
    tool(a, "doc_save", json!({ "doc": doc, "path": name }));
    std::fs::read(dir.path().join(name)).unwrap()
}

/// The action of the link with this rectangle's top-left y in a saved file (a full save renumbers
/// objects): its `/S`, the title of the thread `/D` names, the width of the rectangle of the bead
/// `/B` names (20 for the second bead of the first thread), and whether the link still has a
/// `/Dest`.
fn stored(dir: &tempfile::TempDir, name: &str, y: i64) -> (Option<String>, Option<String>, Option<i64>, bool) {
    let doc = Document::open(Arc::new(std::fs::read(dir.path().join(name)).unwrap())).unwrap();
    let (_, link) = doc
        .scan_objects()
        .filter_map(|(_, o)| o.as_dict().cloned().map(|d| ((), d)))
        .find(|(_, d)| d.name(b"Subtype") == Some(b"Link".as_slice()) && doc_rect_y(&doc, d) == Some(y))
        .expect("the link is in the file");
    let action = link.get(b"A").map(|a| doc.resolve(a));
    let action = action.as_deref().and_then(Object::as_dict);
    let target = |key: &[u8]| action.and_then(|a| a.get(key)).map(|o| doc.resolve(o));
    let title = target(b"D").as_deref().and_then(Object::as_dict).and_then(|t| t.get(b"I")).map(|i| doc.resolve(i)).and_then(|i| {
        match i.as_dict()?.get(b"Title")? {
            Object::String(s) => Some(s.to_text()),
            _ => None,
        }
    });
    let width = target(b"B")
        .as_deref()
        .and_then(Object::as_dict)
        .and_then(|b| b.get(b"R"))
        .map(|r| doc.resolve(r))
        .and_then(|r| r.as_array()?.get(2)?.as_f64());
    (action.and_then(|a| a.name(b"S")).map(|s| String::from_utf8_lossy(s).into_owned()), title, width.map(|w| w as i64), link.contains(b"Dest"))
}

fn doc_rect_y(doc: &Document, d: &pdfcraft_cos::Dict) -> Option<i64> {
    let r = doc.resolve(d.get(b"Rect")?);
    Some(r.as_array()?.get(1)?.as_f64()? as i64)
}

fn call(a: &mut Automation, doc: u64, index: u32, extra: Value) -> Value {
    let mut args = json!({ "doc": doc, "page": 1, "index": index });
    for (k, v) in extra.as_object().unwrap() {
        args[k] = v.clone();
    }
    tool(a, "link_thread_action", args)
}

#[test]
fn a_thread_action_is_read_set_saved_removed_undone_and_redone() {
    let (dir, mut a, doc) = setup();
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    let read = call(&mut a, doc, 1, json!({}));
    assert!(read.get("changed").is_none(), "a read says nothing about changing: {read}");
    assert_eq!(read["thread_action"]["is_thread"], false);
    assert_eq!(read["thread_action"]["action"], "URI");
    assert_eq!(read["thread_action"]["settable"], true);

    // Link 1 had a web action: the second thread replaces it.
    let set = call(&mut a, doc, 1, json!({ "thread": 2 }));
    assert_eq!(set["changed"], true);
    assert_eq!(set["replaced"], json!({ "action": "URI", "has_entry": true, "destination": false, "next": false }));
    assert_eq!(set["thread_action"]["thread"], 2);
    assert_eq!(set["thread_action"]["bead"], Value::Null);
    bytes(&dir, &mut a, doc, "one.pdf");
    assert_eq!(stored(&dir, "one.pdf", 10), (Some("Thread".into()), Some("Second".into()), None, false));
    // The link list sees an action it does not edit.
    let listed = tool(&mut a, "link_list", json!({ "doc": doc }));
    assert_eq!(listed["links"][0]["other"], "Thread", "{listed}");

    // A bead of the first thread, in place of the first action.
    let set = call(&mut a, doc, 1, json!({ "thread": 1, "bead": 2 }));
    assert_eq!((&set["thread_action"]["thread"], &set["thread_action"]["bead"]), (&json!(1), &json!(2)));
    assert_eq!(set["replaced"]["action"], "Thread");
    bytes(&dir, &mut a, doc, "two.pdf");
    assert_eq!(stored(&dir, "two.pdf", 10), (Some("Thread".into()), Some("Intro".into()), Some(20), false));

    // A link with a destination loses it.
    let set = call(&mut a, doc, 2, json!({ "thread": 1 }));
    assert_eq!(set["replaced"], json!({ "action": null, "has_entry": false, "destination": true, "next": false }));
    assert_eq!(set["thread_action"]["destination"], false);
    bytes(&dir, &mut a, doc, "three.pdf");
    assert_eq!(stored(&dir, "three.pdf", 60), (Some("Thread".into()), Some("Intro".into()), None, false));
    // A link with nothing gets the action too.
    assert_eq!(call(&mut a, doc, 3, json!({ "thread": 2 }))["changed"], true);

    // Removal takes only a thread action out, and a second one changes nothing.
    let gone = call(&mut a, doc, 1, json!({ "remove": true }));
    assert_eq!((&gone["changed"], &gone["thread_action"]["has_entry"]), (&json!(true), &json!(false)));
    let saved = bytes(&dir, &mut a, doc, "removed.pdf");
    assert_eq!(stored(&dir, "removed.pdf", 10), (None, None, None, false));
    for index in [1, 4] {
        let again = call(&mut a, doc, index, json!({ "remove": true }));
        assert_eq!(again["changed"], false, "{index}");
    }
    assert_eq!(call(&mut a, doc, 4, json!({}))["thread_action"]["action"], "URI", "another kind of action is kept");
    assert_eq!(bytes(&dir, &mut a, doc, "again.pdf"), saved);

    // Five steps (four sets and the removal), undone one by one.
    let mut undone = Vec::new();
    while let Ok(mut r) = a.call("edit_undo", &json!({ "doc": doc })) {
        let Content::Json(v) = r.remove(0) else { panic!() };
        undone.push(v["undone"].as_str().unwrap().to_string());
    }
    assert_eq!(undone, ["Set link article action"; 5], "the refused and unchanged calls added no step");
    assert_eq!(bytes(&dir, &mut a, doc, "undone.pdf"), before);
    for _ in 0..5 {
        tool(&mut a, "edit_redo", json!({ "doc": doc }));
    }
    assert_eq!(bytes(&dir, &mut a, doc, "redone.pdf"), saved);
}

#[test]
fn a_call_that_cannot_be_applied_is_named_and_changes_nothing() {
    let (dir, mut a, doc) = setup();
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    let c = |extra: Value| {
        let mut args = json!({ "doc": doc, "page": 1, "index": 1 });
        for (k, v) in extra.as_object().unwrap() {
            args[k] = v.clone();
        }
        args
    };
    let cases = [
        (c(json!({ "remove": true, "thread": 1 })), "remove cannot be combined"),
        (c(json!({ "remove": true, "bead": 1 })), "remove cannot be combined"),
        (c(json!({ "bead": 1 })), "bead needs a thread"),
        (c(json!({ "thread": 0 })), "thread must be 1 to 256"),
        (c(json!({ "thread": 257 })), "thread must be 1 to 256"),
        (c(json!({ "thread": 1, "bead": 0 })), "bead must be 1 to 256"),
        (c(json!({ "thread": 1, "bead": 257 })), "bead must be 1 to 256"),
        (c(json!({ "thread": i64::MIN })), "thread must be 1 to 256"),
        (c(json!({ "thread": "1" })), "thread must be an integer"),
        (c(json!({ "remove": "yes" })), "remove must be true or false"),
        (c(json!({ "thread": 3 })), "no article thread at that position"),
        (c(json!({ "thread": 2, "bead": 2 })), "no bead at that position"),
        (c(json!({ "thread": 1, "bead": 3 })), "no bead at that position"),
        (json!({ "doc": doc, "page": 1, "index": 9, "thread": 1 }), "has no link 9"),
        (json!({ "doc": doc, "page": 9, "index": 1, "thread": 1 }), "page"),
        (json!({ "doc": doc, "page": 1, "index": 5, "thread": 1 }), "inline link"),
    ];
    for (args, expected) in cases {
        let m = bad(&mut a, args.clone());
        assert!(m.contains(expected), "{args}: {m}");
    }
    assert_eq!(bytes(&dir, &mut a, doc, "after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "no step was added");
}

#[test]
fn a_modify_restriction_refuses_a_write_and_still_reads() {
    let (_dir, mut a, src) = setup();
    tool(&mut a, "doc_protect", json!({ "doc": src, "open_password": "open", "permissions_password": "boss", "changes": "none" }));
    tool(&mut a, "doc_save", json!({ "doc": src, "path": "locked.pdf" }));
    let locked = tool(&mut a, "doc_open", json!({ "path": "locked.pdf", "password": "open" }))["doc"].as_u64().unwrap();
    match a.call("link_thread_action", &json!({ "doc": locked, "page": 1, "index": 1, "thread": 1 })) {
        Err(ToolError::Failed(m)) => assert!(m.contains("allow") || m.contains("permission"), "{m}"),
        other => panic!("refused: {other:?}"),
    }
    assert_eq!(call(&mut a, locked, 1, json!({}))["thread_action"]["action"], "URI");
    // Nothing to remove: nothing to refuse.
    assert_eq!(call(&mut a, locked, 1, json!({ "remove": true }))["changed"], false);
}
