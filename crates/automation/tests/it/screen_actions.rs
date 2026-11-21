//! `screen_actions`: the media action of the screen annotations of a document read, and taken out
//! by position, checked in the saved file.

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

/// Objects: 4 a screen with a rendition action (a script, a /Next) and an /AA, 5 a screen with a
/// sound action, 6 a screen with a URI action, 7 a link with a sound action (not a screen), 8 a
/// screen with a movie action behind a reference (9), 10 a screen with no action.
fn screens_pdf() -> Vec<u8> {
    screens_pdf_with_page("<< /Type /Page /Parent 2 0 R /Annots [4 0 R 5 0 R 6 0 R 7 0 R 8 0 R 10 0 R] >>")
}

/// The same file with this body for the page (object 3); the cross-reference table is rebuilt.
fn screens_pdf_with_page(page: &str) -> Vec<u8> {
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>",
        page,
        "<< /Type /Annot /Subtype /Screen /Rect [0 0 10 10] /A << /S /Rendition /OP 0 /JS (app.alert(1)) /Next << /S /URI /URI (http://a.example/) >> >> /AA << /E << /S /Named /N /NextPage >> >> >>",
        "<< /Type /Annot /Subtype /Screen /Rect [0 20 10 30] /A << /S /Sound >> >>",
        "<< /Type /Annot /Subtype /Screen /Rect [0 40 10 50] /A << /S /URI /URI (http://a.example/) >> >>",
        "<< /Type /Annot /Subtype /Link /Rect [0 60 10 70] /A << /S /Sound >> >>",
        "<< /Type /Annot /Subtype /Screen /Rect [0 80 10 90] /A 9 0 R >>",
        "<< /S /Movie /Operation /Play >>",
        "<< /Type /Annot /Subtype /Screen /Rect [0 100 10 110] >>",
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

fn setup() -> (tempfile::TempDir, Automation, u64) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("s.pdf"), screens_pdf()).unwrap();
    // Saves stamp /ModDate from the clock: pin it, or two saves a second apart differ.
    let mut a = Automation::new().with_root(dir.path()).unwrap().with_clock(|| 1_700_000_000);
    let doc = tool(&mut a, "doc_open", json!({ "path": "s.pdf" }))["doc"].as_u64().unwrap();
    (dir, a, doc)
}

fn call(a: &mut Automation, doc: u64, extra: Value) -> Value {
    let mut args = json!({ "doc": doc });
    for (k, v) in extra.as_object().unwrap() {
        args[k] = v.clone();
    }
    tool(a, "screen_actions", args)
}

fn bad(a: &mut Automation, args: Value) -> String {
    match a.call("screen_actions", &args) {
        Err(ToolError::InvalidArgs(m) | ToolError::Failed(m)) => m,
        other => panic!("refused: {other:?}"),
    }
}

fn bytes(dir: &tempfile::TempDir, a: &mut Automation, doc: u64, name: &str) -> Vec<u8> {
    tool(a, "doc_save", json!({ "doc": doc, "path": name }));
    std::fs::read(dir.path().join(name)).unwrap()
}

/// The `/S` name of the action of each screen annotation of a saved file, in the order of their
/// `/Rect` bottoms (the save numbers the objects again), with whether it has an `/AA`.
fn stored(dir: &tempfile::TempDir, name: &str) -> Vec<(i64, Option<String>, bool)> {
    let doc = Document::open(Arc::new(std::fs::read(dir.path().join(name)).unwrap())).unwrap();
    let mut found: Vec<(i64, Option<String>, bool)> = doc
        .scan_objects()
        .filter_map(|(_, o)| {
            let d = o.as_dict()?;
            if d.name(b"Subtype") != Some(b"Screen") {
                return None;
            }
            let y = match doc.resolve(d.get(b"Rect")?).as_array()?.get(1)? {
                Object::Int(y) => *y,
                _ => return None,
            };
            let action = d
                .get(b"A")
                .map(|a| doc.resolve(a))
                .and_then(|a| a.as_dict().and_then(|a| a.name(b"S")).map(|s| String::from_utf8_lossy(s).into_owned()));
            Some((y, action, d.contains(b"AA")))
        })
        .collect();
    found.sort();
    found
}

fn read(v: &Value) -> &Value {
    &v["screen_actions"]
}

#[test]
fn the_screen_annotations_are_read_removed_saved_undone_and_redone() {
    let (dir, mut a, doc) = setup();
    let first = call(&mut a, doc, json!({}));
    assert!(first.get("changed").is_none(), "a read says nothing about removing: {first}");
    let r = read(&first);
    assert_eq!((&r["count"], &r["truncated"], &r["capped"]), (&json!(5), &json!(false), &json!(false)), "{first}");
    let kinds: Vec<(&Value, &Value, &Value)> = r["items"].as_array().unwrap().iter().map(|i| (&i["index"], &i["object"], &i["kind"])).collect();
    assert_eq!(
        kinds,
        [
            (&json!(0), &json!("4 0"), &json!("rendition")),
            (&json!(1), &json!("5 0"), &json!("sound")),
            (&json!(2), &json!("6 0"), &Value::Null),
            (&json!(3), &json!("8 0"), &json!("movie")),
            (&json!(4), &json!("10 0"), &Value::Null),
        ],
        "the link with a sound action is not a screen annotation: {first}"
    );
    let rendition = &r["items"][0];
    assert_eq!(
        rendition,
        &json!({ "index": 0, "object": "4 0", "has_entry": true, "present": true, "action": "Rendition", "exact": true, "kind": "rendition",
            "has_script": true, "has_next": true, "has_additional_actions": true, "removable": true })
    );
    assert_eq!(r["items"][2]["action"], "URI");
    assert_eq!((&r["items"][4]["has_entry"], &r["items"][4]["present"]), (&json!(false), &json!(false)));
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    assert_eq!(
        stored(&dir, "before.pdf"),
        [
            (0, Some("Rendition".into()), true),
            (20, Some("Sound".into()), false),
            (40, Some("URI".into()), false),
            (80, Some("Movie".into()), false),
            (100, None, false)
        ]
    );

    let gone = call(&mut a, doc, json!({ "remove": 0 }));
    assert_eq!((&gone["changed"], &gone["removed"]), (&json!(true), &json!({ "action": "Rendition", "kind": "rendition", "next": true })), "{gone}");
    assert_eq!(read(&gone)["items"][0]["has_entry"], false);
    assert_eq!(read(&gone)["items"][0]["has_additional_actions"], true, "the /AA stays");
    assert_eq!(call(&mut a, doc, json!({ "remove": 3 }))["changed"], true);
    assert_eq!(call(&mut a, doc, json!({ "remove": 1 }))["changed"], true);
    let saved = bytes(&dir, &mut a, doc, "removed.pdf");
    assert_eq!(
        stored(&dir, "removed.pdf"),
        [(0, None, true), (20, None, false), (40, Some("URI".into()), false), (80, None, false), (100, None, false)]
    );

    // Another kind of action, no action and a position with nothing listed change nothing.
    for index in [2, 4, 5, 6, 256, 1_000_000] {
        let r = call(&mut a, doc, json!({ "remove": index }));
        assert_eq!(r["changed"], false, "{index}");
        assert!(r.get("removed").is_none(), "{index}");
    }
    assert_eq!(bytes(&dir, &mut a, doc, "again.pdf"), saved, "removals that change nothing write nothing");
    assert_eq!(call(&mut a, doc, json!({ "remove": 0 }))["changed"], false, "an action already out is no change");

    // Three removals, undone one by one: the calls that changed nothing added no step.
    let mut undone = Vec::new();
    while let Ok(mut r) = a.call("edit_undo", &json!({ "doc": doc })) {
        let Content::Json(v) = r.remove(0) else { panic!() };
        undone.push(v["undone"].as_str().unwrap().to_string());
    }
    assert_eq!(undone, ["Remove screen annotation media action"; 3]);
    assert_eq!(bytes(&dir, &mut a, doc, "undone.pdf"), before);
    for _ in 0..3 {
        tool(&mut a, "edit_redo", json!({ "doc": doc }));
    }
    assert_eq!(bytes(&dir, &mut a, doc, "redone.pdf"), saved);
}

#[test]
fn there_is_no_way_to_set_a_media_action() {
    let (dir, mut a, doc) = setup();
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    // An argument that would set one is not part of the tool: it is not applied.
    for extra in [json!({ "kind": "sound" }), json!({ "action": "Sound" }), json!({ "script": "app.alert(1)" }), json!({ "index": 2, "set": true })] {
        let mut args = json!({ "doc": doc });
        for (k, v) in extra.as_object().unwrap() {
            args[k] = v.clone();
        }
        let _ = a.call("screen_actions", &args);
    }
    assert_eq!(bytes(&dir, &mut a, doc, "after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "no step was added");
    let def = pdfcraft_automation::tools().into_iter().find(|t| t.name == "screen_actions").unwrap();
    let mut names: Vec<&str> = def.input_schema["properties"].as_object().unwrap().keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(names, ["doc", "remove"], "the only arguments name a document and a removal");
    assert!(def.description.contains("There is no set"));
}

#[test]
fn a_bad_argument_is_named_and_changes_nothing() {
    let (dir, mut a, doc) = setup();
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    assert_eq!(bad(&mut a, json!({ "doc": doc, "remove": -1 })), "remove must be 0 or more");
    assert_eq!(bad(&mut a, json!({ "doc": doc, "remove": "0" })), "remove must be an integer");
    assert_eq!(bad(&mut a, json!({ "doc": doc, "remove": 0.5 })), "remove must be an integer");
    assert_eq!(call(&mut a, doc, json!({ "remove": i64::MAX }))["changed"], false);
    // A null is left out like any other argument: it reads.
    assert_eq!(read(&call(&mut a, doc, json!({ "remove": null })))["count"], 5);
    assert_eq!(bytes(&dir, &mut a, doc, "after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "no edit was made");
}

#[test]
fn a_screen_annotation_that_is_a_page_keeps_its_action_and_a_modify_restriction_holds() {
    let (dir, mut a, doc) = setup();
    // A modify restriction refuses a removal that would change something, and still reads; a
    // removal that finds nothing to remove is not refused.
    tool(&mut a, "doc_protect", json!({ "doc": doc, "open_password": "open", "permissions_password": "boss", "changes": "none" }));
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "locked.pdf" }));
    let locked = tool(&mut a, "doc_open", json!({ "path": "locked.pdf", "password": "open" }))["doc"].as_u64().unwrap();
    match a.call("screen_actions", &json!({ "doc": locked, "remove": 1 })) {
        Err(ToolError::Failed(m)) => assert!(m.contains("allow") || m.contains("permission"), "{m}"),
        other => panic!("refused: {other:?}"),
    }
    assert_eq!(read(&call(&mut a, locked, json!({})))["items"][1]["kind"], "sound");
    assert_eq!(call(&mut a, locked, json!({ "remove": 2 }))["changed"], false);

    // The page object itself marked as a screen annotation with a media action: listed, not
    // removable, refused by name, and left as it was.
    let text =
        screens_pdf_with_page("<< /Type /Page /Subtype /Screen /A << /S /Sound >> /Parent 2 0 R /Annots [4 0 R 5 0 R 6 0 R 7 0 R 8 0 R 10 0 R] >>");
    std::fs::write(dir.path().join("page.pdf"), text).unwrap();
    let page = tool(&mut a, "doc_open", json!({ "path": "page.pdf" }))["doc"].as_u64().unwrap();
    let listed = call(&mut a, page, json!({}));
    assert_eq!((&read(&listed)["items"][0]["object"], &read(&listed)["items"][0]["removable"]), (&json!("3 0"), &json!(false)), "{listed}");
    assert_eq!(read(&listed)["items"][1]["removable"], true);
    let before = bytes(&dir, &mut a, page, "page-before.pdf");
    let m = bad(&mut a, json!({ "doc": page, "remove": 0 }));
    assert!(m.contains("page tree"), "{m}");
    assert_eq!(bytes(&dir, &mut a, page, "page-after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": page })).is_err(), "no step was added");
}
