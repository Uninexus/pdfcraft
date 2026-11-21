//! `link_named_action`: the menu-item (named) action of a link, read, set and removed, checked in
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

/// A page with five links: 4 has a web action, 5 a destination, 6 nothing, 7 a named action with a
/// /Next chain, and the fifth is written in the page (an inline link, which has no object).
fn linked_pdf() -> Vec<u8> {
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Annots [4 0 R 5 0 R 6 0 R 7 0 R << /Type /Annot /Subtype /Link /Rect [0 0 5 5] /A << /S /URI /URI (http://z.test) >> >>] >>".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 10 50 50] /A << /S /URI /URI (http://x.test) >> >>".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 60 50 90] /Dest [3 0 R /Fit] >>".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 100 50 130] >>".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 140 50 170] /A << /S /Named /N /Print /Next << /S /Named /N /NextPage >> >> >>".to_string(),
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
    match a.call("link_named_action", &args) {
        Err(ToolError::InvalidArgs(m) | ToolError::Failed(m)) => m,
        other => panic!("refused: {other:?}"),
    }
}

fn bytes(dir: &tempfile::TempDir, a: &mut Automation, doc: u64, name: &str) -> Vec<u8> {
    tool(a, "doc_save", json!({ "doc": doc, "path": name }));
    std::fs::read(dir.path().join(name)).unwrap()
}

/// The action of the link whose rectangle has this lower y in a saved file (a full save
/// renumbers objects): its `/S`, its `/N`, whether it has a `/Next`, and whether the link still
/// has a `/Dest`.
fn stored(dir: &tempfile::TempDir, name: &str, y: i64) -> (Option<String>, Option<String>, bool, bool) {
    let doc = Document::open(Arc::new(std::fs::read(dir.path().join(name)).unwrap())).unwrap();
    let (_, link) = doc
        .scan_objects()
        .filter_map(|(_, o)| o.as_dict().cloned().map(|d| ((), d)))
        .find(|(_, d)| d.name(b"Subtype") == Some(b"Link".as_slice()) && rect_y(&doc, d) == Some(y))
        .expect("the link is in the file");
    let action = link.get(b"A").map(|a| doc.resolve(a));
    let action = action.as_deref().and_then(Object::as_dict);
    let word = |key: &[u8]| action.and_then(|a| a.name(key)).map(|s| String::from_utf8_lossy(s).into_owned());
    (word(b"S"), word(b"N"), action.is_some_and(|a| a.contains(b"Next")), link.contains(b"Dest"))
}

fn rect_y(doc: &Document, d: &pdfcraft_cos::Dict) -> Option<i64> {
    let r = doc.resolve(d.get(b"Rect")?);
    Some(r.as_array()?.get(1)?.as_f64()? as i64)
}

fn call(a: &mut Automation, doc: u64, index: u32, extra: Value) -> Value {
    let mut args = json!({ "doc": doc, "page": 1, "index": index });
    for (k, v) in extra.as_object().unwrap() {
        args[k] = v.clone();
    }
    tool(a, "link_named_action", args)
}

#[test]
fn a_named_action_is_read_set_saved_removed_undone_and_redone() {
    let (dir, mut a, doc) = setup();
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    let read = call(&mut a, doc, 1, json!({}));
    assert!(read.get("changed").is_none(), "a read says nothing about changing: {read}");
    assert_eq!(
        (&read["named_action"]["is_named"], &read["named_action"]["action"], &read["named_action"]["settable"]),
        (&json!(false), &json!("URI"), &json!(true))
    );
    let read = call(&mut a, doc, 4, json!({}));
    assert_eq!(
        (&read["named_action"]["name"], &read["named_action"]["safe"], &read["named_action"]["has_next"]),
        (&json!("Print"), &json!(true), &json!(true))
    );

    // Link 1 had a web action: the next-page item replaces it.
    let set = call(&mut a, doc, 1, json!({ "name": "NextPage" }));
    assert_eq!(set["changed"], true);
    assert_eq!(set["replaced"], json!({ "action": "URI", "has_entry": true, "destination": false, "next": false }));
    assert_eq!((&set["named_action"]["name"], &set["named_action"]["safe"]), (&json!("NextPage"), &json!(true)));
    bytes(&dir, &mut a, doc, "one.pdf");
    assert_eq!(stored(&dir, "one.pdf", 10), (Some("Named".into()), Some("NextPage".into()), false, false));
    // The link list sees an action it does not edit.
    let listed = tool(&mut a, "link_list", json!({ "doc": doc }));
    assert_eq!(listed["links"][0]["other"], "Named", "{listed}");

    // A link with a destination loses it; a link with nothing gets the action too.
    let set = call(&mut a, doc, 2, json!({ "name": "LastPage" }));
    assert_eq!(set["replaced"], json!({ "action": null, "has_entry": false, "destination": true, "next": false }));
    assert_eq!(set["named_action"]["destination"], false);
    assert_eq!(call(&mut a, doc, 3, json!({ "name": "FirstPage" }))["changed"], true);
    // A named action with a /Next chain is replaced and the chain goes.
    let set = call(&mut a, doc, 4, json!({ "name": "PrevPage" }));
    assert_eq!(set["replaced"], json!({ "action": "Named", "has_entry": true, "destination": false, "next": true }));
    bytes(&dir, &mut a, doc, "two.pdf");
    assert_eq!(stored(&dir, "two.pdf", 60), (Some("Named".into()), Some("LastPage".into()), false, false));
    assert_eq!(stored(&dir, "two.pdf", 100), (Some("Named".into()), Some("FirstPage".into()), false, false));
    assert_eq!(stored(&dir, "two.pdf", 140), (Some("Named".into()), Some("PrevPage".into()), false, false));

    // Removal takes only a named action out, and a second one changes nothing.
    let gone = call(&mut a, doc, 1, json!({ "remove": true }));
    assert_eq!((&gone["changed"], &gone["named_action"]["has_entry"]), (&json!(true), &json!(false)));
    let saved = bytes(&dir, &mut a, doc, "removed.pdf");
    assert_eq!(stored(&dir, "removed.pdf", 10), (None, None, false, false));
    let again = call(&mut a, doc, 1, json!({ "remove": true }));
    assert_eq!(again["changed"], false);
    assert_eq!(bytes(&dir, &mut a, doc, "again.pdf"), saved);

    // Four sets and the removal, undone one by one: the unchanged call added no step.
    let mut undone = Vec::new();
    while let Ok(mut r) = a.call("edit_undo", &json!({ "doc": doc })) {
        let Content::Json(v) = r.remove(0) else { panic!() };
        undone.push(v["undone"].as_str().unwrap().to_string());
    }
    assert_eq!(undone, ["Set link menu action"; 5]);
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
        (c(json!({ "remove": true, "name": "Print" })), "remove cannot be combined"),
        (c(json!({ "name": "Quit" })), "a menu item is one of NextPage, PrevPage, FirstPage, LastPage, Print"),
        (c(json!({ "name": "nextpage" })), "a menu item is one of"),
        (c(json!({ "name": "" })), "a menu item is one of"),
        (c(json!({ "name": 1 })), "name must be a string"),
        (c(json!({ "remove": "yes" })), "remove must be true or false"),
        (json!({ "doc": doc, "page": 1, "index": 9, "name": "Print" }), "has no link 9"),
        (json!({ "doc": doc, "page": 9, "index": 1, "name": "Print" }), "page"),
        (json!({ "doc": doc, "page": 1, "index": 5, "name": "Print" }), "inline link"),
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
    match a.call("link_named_action", &json!({ "doc": locked, "page": 1, "index": 1, "name": "Print" })) {
        Err(ToolError::Failed(m)) => assert!(m.contains("allow") || m.contains("permission"), "{m}"),
        other => panic!("refused: {other:?}"),
    }
    assert_eq!(call(&mut a, locked, 1, json!({}))["named_action"]["action"], "URI");
    // Nothing to remove: nothing to refuse.
    assert_eq!(call(&mut a, locked, 1, json!({ "remove": true }))["changed"], false);
}
