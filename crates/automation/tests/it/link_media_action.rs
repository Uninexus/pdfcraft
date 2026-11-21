//! `link_media_action`: the sound, movie, rendition and rich-media action of a link, read and
//! removed (never set), checked in the saved file.

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

/// A page with eight links, by the lower y of their rectangles: 10 a sound action, 60 a movie
/// action with a /Next chain, 100 a rendition action with a script, 140 a rich-media action with a
/// /Dest, 180 a JavaScript action, 220 a web action with a sound action as its /Next, 260 nothing,
/// and the eighth is written in the page (an inline link, which has no object).
fn linked_pdf() -> Vec<u8> {
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 300] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Annots [4 0 R 5 0 R 6 0 R 7 0 R 8 0 R 9 0 R 10 0 R << /Type /Annot /Subtype /Link /Rect [0 0 5 5] /A << /S /Sound >> >>] >>".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 10 50 50] /A << /S /Sound /Volume 1 >> >>".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 60 50 90] /A << /S /Movie /T (clip) /Operation /Pause /Next << /S /URI /URI (http://x.test) >> >> >>".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 100 50 130] /A << /S /Rendition /OP 4 /JS (app.alert\\(1\\)) /AN 3 0 R >> >>".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 140 50 170] /A << /S /RichMediaExecute /TA 3 0 R >> /Dest [3 0 R /Fit] >>".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 180 50 210] /A << /S /JavaScript /JS (this.print\\(\\)) >> >>".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 220 50 250] /A << /S /URI /URI (http://y.test) /Next << /S /Sound >> >> >>".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 260 50 290] >>".to_string(),
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
    match a.call("link_media_action", &args) {
        Err(ToolError::InvalidArgs(m) | ToolError::Failed(m)) => m,
        other => panic!("refused: {other:?}"),
    }
}

fn bytes(dir: &tempfile::TempDir, a: &mut Automation, doc: u64, name: &str) -> Vec<u8> {
    tool(a, "doc_save", json!({ "doc": doc, "path": name }));
    std::fs::read(dir.path().join(name)).unwrap()
}

/// The action of the link whose rectangle has this lower y in a saved file (a full save
/// renumbers objects): its `/S`, whether it has a `/Next`, and whether the link still has a
/// `/Dest`.
fn stored(dir: &tempfile::TempDir, name: &str, y: i64) -> (Option<String>, bool, bool) {
    let doc = Document::open(Arc::new(std::fs::read(dir.path().join(name)).unwrap())).unwrap();
    let (_, link) = doc
        .scan_objects()
        .filter_map(|(_, o)| o.as_dict().cloned().map(|d| ((), d)))
        .find(|(_, d)| d.name(b"Subtype") == Some(b"Link".as_slice()) && rect_y(&doc, d) == Some(y))
        .expect("the link is in the file");
    let action = link.get(b"A").map(|a| doc.resolve(a));
    let action = action.as_deref().and_then(Object::as_dict);
    let s = action.and_then(|a| a.name(b"S")).map(|s| String::from_utf8_lossy(s).into_owned());
    (s, action.is_some_and(|a| a.contains(b"Next")), link.contains(b"Dest"))
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
    tool(a, "link_media_action", args)
}

#[test]
fn a_media_action_is_read_removed_saved_undone_and_redone() {
    let (dir, mut a, doc) = setup();
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    let read = |a: &mut Automation, i: u32| call(a, doc, i, json!({}));
    let one = read(&mut a, 1);
    assert!(one.get("changed").is_none() && one.get("removed").is_none(), "a read says nothing about changing: {one}");
    assert_eq!(
        (&one["media_action"]["is_media"], &one["media_action"]["kind"], &one["media_action"]["has_next"]),
        (&json!(true), &json!("sound"), &json!(false))
    );
    let two = read(&mut a, 2);
    assert_eq!(
        (&two["media_action"]["kind"], &two["media_action"]["operation"], &two["media_action"]["has_target"], &two["media_action"]["has_next"]),
        (&json!("movie"), &json!("Pause"), &json!(true), &json!(true))
    );
    let three = read(&mut a, 3);
    assert_eq!(
        (
            &three["media_action"]["kind"],
            &three["media_action"]["rendition_op"],
            &three["media_action"]["has_script"],
            &three["media_action"]["has_target"]
        ),
        (&json!("rendition"), &json!(4), &json!(true), &json!(true))
    );
    let four = read(&mut a, 4);
    assert_eq!((&four["media_action"]["kind"], &four["media_action"]["destination"]), (&json!("rich_media_execute"), &json!(true)));
    let five = read(&mut a, 5);
    assert_eq!(
        (&five["media_action"]["is_media"], &five["media_action"]["action"], &five["media_action"]["kind"]),
        (&json!(false), &json!("JavaScript"), &json!(null))
    );
    // The link list sees these as actions it does not edit.
    let listed = tool(&mut a, "link_list", json!({ "doc": doc }));
    for (i, kind) in ["Sound", "Movie", "Rendition"].into_iter().enumerate() {
        assert_eq!(listed["links"][i]["other"], kind, "{listed}");
    }
    // The fourth has a /Dest as well, which the list shows in preference to its action.
    assert!(listed["links"][3].get("other").is_none() && listed["links"][3]["to_page"] == 1, "{listed}");

    // A read changed nothing; removals take the action and its /Next chain out of the link only.
    assert_eq!(bytes(&dir, &mut a, doc, "read.pdf"), before);
    let gone = call(&mut a, doc, 2, json!({ "remove": true }));
    assert_eq!(gone["changed"], true);
    assert_eq!(gone["removed"], json!({ "action": "Movie", "kind": "movie", "next": true }));
    assert_eq!((&gone["media_action"]["has_entry"], &gone["media_action"]["present"]), (&json!(false), &json!(false)));
    assert_eq!(call(&mut a, doc, 3, json!({ "remove": true }))["changed"], true);
    let kept_dest = call(&mut a, doc, 4, json!({ "remove": true }));
    assert_eq!(kept_dest["media_action"]["destination"], true, "a removal leaves the link's /Dest");
    assert_eq!(call(&mut a, doc, 1, json!({ "remove": true }))["changed"], true);
    let saved = bytes(&dir, &mut a, doc, "removed.pdf");
    for (y, dest) in [(10, false), (60, false), (100, false), (140, true)] {
        assert_eq!(stored(&dir, "removed.pdf", y), (None, false, dest), "{y}");
    }
    // Other actions are untouched, by a removal and in the saved file.
    for i in [5, 6, 7] {
        assert_eq!(call(&mut a, doc, i, json!({ "remove": true }))["changed"], false, "link {i}");
    }
    assert_eq!(stored(&dir, "removed.pdf", 180), (Some("JavaScript".into()), false, false));
    assert_eq!(stored(&dir, "removed.pdf", 220), (Some("URI".into()), true, false), "a media action behind a /Next stays");
    assert_eq!(bytes(&dir, &mut a, doc, "again.pdf"), saved, "removals that change nothing write nothing");
    assert_eq!(call(&mut a, doc, 2, json!({ "remove": true }))["changed"], false);

    // Four removals, undone one by one: the calls that changed nothing added no step.
    let mut undone = Vec::new();
    while let Ok(mut r) = a.call("edit_undo", &json!({ "doc": doc })) {
        let Content::Json(v) = r.remove(0) else { panic!() };
        undone.push(v["undone"].as_str().unwrap().to_string());
    }
    assert_eq!(undone, ["Remove link media action"; 4]);
    assert_eq!(bytes(&dir, &mut a, doc, "undone.pdf"), before);
    for _ in 0..4 {
        tool(&mut a, "edit_redo", json!({ "doc": doc }));
    }
    assert_eq!(bytes(&dir, &mut a, doc, "redone.pdf"), saved);
}

#[test]
fn there_is_no_way_to_set_a_media_action() {
    let (dir, mut a, doc) = setup();
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    // An argument that would set one is not part of the tool: it is not applied.
    for extra in [json!({ "kind": "sound" }), json!({ "operation": "Play" }), json!({ "script": "app.alert(1)" })] {
        let mut args = json!({ "doc": doc, "page": 1, "index": 7 });
        for (k, v) in extra.as_object().unwrap() {
            args[k] = v.clone();
        }
        let _ = a.call("link_media_action", &args);
    }
    assert_eq!(bytes(&dir, &mut a, doc, "after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "no step was added");
    let def = pdfcraft_automation::tools().into_iter().find(|t| t.name == "link_media_action").unwrap();
    let mut names: Vec<&str> = def.input_schema["properties"].as_object().unwrap().keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(names, ["doc", "index", "page", "remove"], "the only arguments name a link and a removal");
}

#[test]
fn a_call_that_cannot_be_applied_is_named_and_changes_nothing() {
    let (dir, mut a, doc) = setup();
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    let cases = [
        (json!({ "doc": doc, "page": 1, "index": 1, "remove": "yes" }), "remove must be true or false"),
        (json!({ "doc": doc, "page": 1, "index": 99, "remove": true }), "has no link 99"),
        (json!({ "doc": doc, "page": 9, "index": 1, "remove": true }), "page"),
        (json!({ "doc": doc, "page": 1, "index": 8, "remove": true }), "inline link"),
        (json!({ "doc": doc, "page": 1, "index": 8 }), "inline link"),
    ];
    for (args, expected) in cases {
        let m = bad(&mut a, args.clone());
        assert!(m.contains(expected), "{args}: {m}");
    }
    assert_eq!(bytes(&dir, &mut a, doc, "after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "no step was added");
}

#[test]
fn a_modify_restriction_refuses_a_removal_and_still_reads() {
    let (_dir, mut a, src) = setup();
    tool(&mut a, "doc_protect", json!({ "doc": src, "open_password": "open", "permissions_password": "boss", "changes": "none" }));
    tool(&mut a, "doc_save", json!({ "doc": src, "path": "locked.pdf" }));
    let locked = tool(&mut a, "doc_open", json!({ "path": "locked.pdf", "password": "open" }))["doc"].as_u64().unwrap();
    match a.call("link_media_action", &json!({ "doc": locked, "page": 1, "index": 1, "remove": true })) {
        Err(ToolError::Failed(m)) => assert!(m.contains("allow") || m.contains("permission"), "{m}"),
        other => panic!("refused: {other:?}"),
    }
    assert_eq!(call(&mut a, locked, 1, json!({}))["media_action"]["kind"], "sound");
    // Nothing to remove: nothing to refuse.
    assert_eq!(call(&mut a, locked, 5, json!({ "remove": true }))["changed"], false);
}
