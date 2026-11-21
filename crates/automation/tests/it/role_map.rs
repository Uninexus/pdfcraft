//! `role_map`: the structure tree's role map (`/StructTreeRoot` `/RoleMap`), read back from the
//! saved file and through `bookmark_from_structure`, which follows it.

use pdfcraft_automation::{Automation, Content, ToolError};
use pdfcraft_cos::{Document, Object};
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

/// A tagged two-page file: a type `Heading` that only a role map makes a heading, and a real H2.
/// `tree_extra` goes into the tree root beside `/Type` and `/K`.
fn tagged_pdf(tree_extra: &str) -> Vec<u8> {
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 5 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R >>".to_string(),
        "<< /Type /Page /Parent 2 0 R >>".to_string(),
        format!("<< /Type /StructTreeRoot /K 6 0 R {tree_extra} >>"),
        "<< /S /Document /K [7 0 R << /S /Sect /K 8 0 R >>] >>".to_string(),
        "<< /S /Heading /Pg 3 0 R /ActualText (Report) >>".to_string(),
        "<< /S /H2 /Pg 4 0 R /Alt (Findings) >>".to_string(),
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

/// The saved file's `/RoleMap`, resolved, and whether the tree root has the key at all.
fn stored(dir: &tempfile::TempDir, name: &str) -> (Option<pdfcraft_cos::Dict>, bool) {
    let doc = Document::open(Arc::new(std::fs::read(dir.path().join(name)).unwrap())).unwrap();
    let c = doc.get(doc.root().unwrap()).as_dict().cloned().unwrap();
    let root = c.get(b"StructTreeRoot").map(|o| doc.resolve(o)).unwrap();
    let root = root.as_dict().unwrap();
    (root.get(b"RoleMap").map(|o| doc.resolve(o)).and_then(|o| o.as_dict().cloned()), root.get(b"RoleMap").is_some())
}

fn bad(a: &mut Automation, args: Value) -> String {
    match a.call("role_map", &args) {
        Err(ToolError::InvalidArgs(m) | ToolError::Failed(m)) => m,
        other => panic!("refused: {other:?}"),
    }
}

fn map(v: &Value) -> &Value {
    &v["role_map"]
}

fn bytes(dir: &tempfile::TempDir, a: &mut Automation, doc: u64, name: &str) -> Vec<u8> {
    tool(a, "doc_save", json!({ "doc": doc, "path": name }));
    std::fs::read(dir.path().join(name)).unwrap()
}

/// The titles of the bookmarks `bookmark_from_structure` makes, then taken back.
fn structure_titles(a: &mut Automation, doc: u64) -> Vec<String> {
    let made = a.call("bookmark_from_structure", &json!({ "doc": doc }));
    let titles = match made {
        Ok(mut c) => match c.remove(0) {
            Content::Json(v) => {
                fn walk(v: &Value, out: &mut Vec<String>) {
                    for b in v.as_array().into_iter().flatten() {
                        out.extend(b["title"].as_str().map(str::to_owned));
                        walk(&b["children"], out);
                    }
                }
                let mut out = Vec::new();
                walk(&v["bookmarks"], &mut out);
                out
            }
            other => panic!("{other:?}"),
        },
        Err(ToolError::Failed(m)) => vec![m],
        Err(e) => panic!("{e:?}"),
    };
    if titles.iter().all(|t| !t.contains("no tagged headings")) {
        assert_eq!(tool(a, "edit_undo", json!({ "doc": doc }))["undone"], "New bookmarks from structure");
    }
    titles
}

#[test]
fn an_entry_is_set_read_saved_followed_by_the_readers_undone_and_redone() {
    let (dir, mut a) = setup();
    std::fs::write(dir.path().join("tagged.pdf"), tagged_pdf("")).unwrap();
    let doc = open(&mut a, "tagged.pdf");
    let read = tool(&mut a, "role_map", json!({ "doc": doc }));
    assert_eq!(map(&read), &json!({ "has_structure_tree": true, "count": 0, "truncated": false, "entries": [], "unreadable": [] }), "{read}");
    let before = structure_titles(&mut a, doc);
    assert!(!before.iter().any(|t| t == "Report"), "without the entry Heading is not a heading: {before:?}");

    let set = tool(&mut a, "role_map", json!({ "doc": doc, "role": "Heading", "maps_to": "H1" }));
    assert_eq!(
        map(&set),
        &json!({
            "has_structure_tree": true, "count": 1, "truncated": false, "unreadable": [],
            "entries": [{ "role": "Heading", "maps_to": "H1", "exact": true, "loops": false }]
        }),
        "{set}"
    );
    // Asking for what is stored is not an edit: no undo step is added.
    tool(&mut a, "role_map", json!({ "doc": doc, "role": "Heading", "maps_to": "H1" }));
    let after = structure_titles(&mut a, doc);
    assert!(after.iter().any(|t| t == "Report"), "the reader follows the entry the tool wrote: {after:?}");
    let saved = bytes(&dir, &mut a, doc, "out.pdf");
    let (stored_map, _) = stored(&dir, "out.pdf");
    assert_eq!(stored_map.unwrap().get(b"Heading"), Some(&Object::name("H1")));

    // One undo step, labelled; the next undo has nothing to undo.
    assert_eq!(tool(&mut a, "edit_undo", json!({ "doc": doc }))["undone"], "Change role map");
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "the identical call added no step");
    assert_eq!(map(&tool(&mut a, "role_map", json!({ "doc": doc })))["entries"], json!([]));
    tool(&mut a, "edit_redo", json!({ "doc": doc }));
    assert_eq!(map(&tool(&mut a, "role_map", json!({ "doc": doc })))["count"], 1);
    assert_eq!(bytes(&dir, &mut a, doc, "again.pdf"), saved);
    // Undone, the readers no longer follow it (their own edit clears what could be redone).
    tool(&mut a, "edit_undo", json!({ "doc": doc }));
    assert_eq!(structure_titles(&mut a, doc), before);
    tool(&mut a, "role_map", json!({ "doc": doc, "role": "Heading", "maps_to": "H1" }));
    assert_eq!(bytes(&dir, &mut a, doc, "set-again.pdf"), saved);

    // An empty maps_to takes the entry away, and the emptied dictionary with it.
    let cleared = tool(&mut a, "role_map", json!({ "doc": doc, "role": "Heading", "maps_to": "" }));
    assert_eq!(map(&cleared)["entries"], json!([]));
    bytes(&dir, &mut a, doc, "cleared.pdf");
    assert_eq!(stored(&dir, "cleared.pdf"), (None, false));
}

#[test]
fn a_refused_call_writes_nothing_and_adds_no_undo_step() {
    let (dir, mut a) = setup();
    // No structure tree at all.
    let plain = open(&mut a, "a.pdf");
    let before = bytes(&dir, &mut a, plain, "plain-before.pdf");
    assert_eq!(
        bad(&mut a, json!({ "doc": plain, "role": "A", "maps_to": "P" })),
        "the document has no structure tree, so there is no role map to change"
    );
    assert_eq!(
        map(&tool(&mut a, "role_map", json!({ "doc": plain }))),
        &json!({ "has_structure_tree": false, "count": 0, "truncated": false, "entries": [], "unreadable": [] })
    );
    // Taking away what is not there is not a change.
    tool(&mut a, "role_map", json!({ "doc": plain, "role": "A", "maps_to": "" }));
    assert_eq!(bytes(&dir, &mut a, plain, "plain-after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": plain })).is_err());

    std::fs::write(dir.path().join("tagged.pdf"), tagged_pdf("/RoleMap << /B /C /C /D >>")).unwrap();
    let doc = open(&mut a, "tagged.pdf");
    let before = bytes(&dir, &mut a, doc, "before.pdf");
    let long = "a".repeat(128);
    let role_bad = "a role must be 1 to 127 bytes, not ";
    let to_bad = "what a role maps to must be 1 to 127 bytes, not ";
    for (args, message) in [
        (json!({ "role": "", "maps_to": "P" }), format!("{role_bad}0 bytes")),
        (json!({ "role": long, "maps_to": "P" }), format!("{role_bad}128 bytes")),
        (json!({ "role": "A\u{0}", "maps_to": "P" }), "a role must not contain a NUL byte".to_owned()),
        (json!({ "role": "A\u{0}", "maps_to": "" }), "a role must not contain a NUL byte".to_owned()),
        (json!({ "role": long, "maps_to": "" }), format!("{role_bad}128 bytes")),
        (json!({ "role": "", "maps_to": "" }), format!("{role_bad}0 bytes")),
        (json!({ "role": "A", "maps_to": long }), format!("{to_bad}128 bytes")),
        (json!({ "role": "A", "maps_to": "A" }), "the role A cannot map to itself".to_owned()),
        (json!({ "role": "D", "maps_to": "B" }), "B already leads back to D, so mapping D to it would make a loop".to_owned()),
        (json!({ "role": "A" }), "role needs maps_to (an empty string takes the entry away)".to_owned()),
        (json!({ "maps_to": "P" }), "maps_to needs role".to_owned()),
        (json!({ "role": 7, "maps_to": "P" }), "role must be a string".to_owned()),
        (json!({ "role": "A", "maps_to": ["P"] }), "maps_to must be a string".to_owned()),
    ] {
        let mut args = args;
        args["doc"] = json!(doc);
        assert_eq!(bad(&mut a, args.clone()), message, "{args}");
    }
    assert_eq!(map(&tool(&mut a, "role_map", json!({ "doc": doc })))["count"], 2);
    assert_eq!(bytes(&dir, &mut a, doc, "after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "no edit was made");
    // A null is left out like any other argument: both null read.
    let read = tool(&mut a, "role_map", json!({ "doc": doc, "role": null, "maps_to": null }));
    assert_eq!(map(&read)["count"], 2);
}

#[test]
fn a_stored_loop_is_listed_and_sent_back_unchanged_and_a_value_that_is_not_a_name_is_listed() {
    let (dir, mut a) = setup();
    std::fs::write(dir.path().join("loop.pdf"), tagged_pdf("/RoleMap << /A /B /B /A /N 5 /Free /P >>")).unwrap();
    let doc = open(&mut a, "loop.pdf");
    let read = tool(&mut a, "role_map", json!({ "doc": doc }));
    assert_eq!(
        map(&read)["entries"],
        json!([
            { "role": "A", "maps_to": "B", "exact": true, "loops": true },
            { "role": "B", "maps_to": "A", "exact": true, "loops": true },
            { "role": "N", "maps_to": null, "exact": true, "loops": false },
            { "role": "Free", "maps_to": "P", "exact": true, "loops": false },
        ]),
        "{read}"
    );
    let before = bytes(&dir, &mut a, doc, "loop-before.pdf");
    // The stored loop sent back is accepted and adds no undo step; a new one is refused.
    tool(&mut a, "role_map", json!({ "doc": doc, "role": "A", "maps_to": "B" }));
    assert_eq!(
        bad(&mut a, json!({ "doc": doc, "role": "P", "maps_to": "Free" })),
        "Free already leads back to P, so mapping P to it would make a loop"
    );
    assert_eq!(bytes(&dir, &mut a, doc, "loop-after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err());
    // A value that is not a name is replaced by a name, one step, and the loop is broken by A.
    tool(&mut a, "role_map", json!({ "doc": doc, "role": "N", "maps_to": "Span" }));
    let broken = tool(&mut a, "role_map", json!({ "doc": doc, "role": "A", "maps_to": "P" }));
    assert_eq!(map(&broken)["entries"][0], json!({ "role": "A", "maps_to": "P", "exact": true, "loops": false }));
    assert_eq!(map(&broken)["entries"][1]["loops"], false);
    assert_eq!(map(&broken)["entries"][2]["maps_to"], "Span");
}

#[test]
fn a_tree_root_or_role_map_that_is_not_a_dictionary_is_named_left_and_a_modify_restriction_holds() {
    let (dir, mut a) = setup();
    let refused = "the structure tree's /StructTreeRoot or /RoleMap cannot be changed (it is not a dictionary, is behind more than 33 references, is a stream role map, or is the same object as the catalog, the /Pages node, a page or inner /Pages node, or (for a map) the tree root or its other map, or the page tree has over 1000000 nodes or 2000000 /Kids entries and could not be checked), so it was left as it is";
    std::fs::write(dir.path().join("odd.pdf"), tagged_pdf("/RoleMap 7")).unwrap();
    let doc = open(&mut a, "odd.pdf");
    let before = bytes(&dir, &mut a, doc, "odd-before.pdf");
    assert_eq!(bad(&mut a, json!({ "doc": doc, "role": "A", "maps_to": "P" })), refused);
    assert_eq!(
        map(&tool(&mut a, "role_map", json!({ "doc": doc }))),
        &json!({ "has_structure_tree": true, "count": 0, "truncated": false, "entries": [], "unreadable": ["role_map"] })
    );
    tool(&mut a, "role_map", json!({ "doc": doc, "role": "A", "maps_to": "" }));
    assert_eq!(bytes(&dir, &mut a, doc, "odd-after.pdf"), before);
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err());

    // A /RoleMap that is the /Pages node, a page, or the /ClassMap dictionary: named, refused, and
    // the object is not written (a role named Kids would break the page tree).
    for (name, extra) in [("pages", "/RoleMap 2 0 R"), ("page", "/RoleMap 3 0 R"), ("sibling", "/RoleMap 8 0 R /ClassMap 8 0 R")] {
        std::fs::write(dir.path().join(format!("{name}.pdf")), tagged_pdf(extra)).unwrap();
        let doc = open(&mut a, &format!("{name}.pdf"));
        let before = bytes(&dir, &mut a, doc, &format!("{name}-before.pdf"));
        assert_eq!(bad(&mut a, json!({ "doc": doc, "role": "Kids", "maps_to": "P" })), refused, "{name}");
        assert_eq!(map(&tool(&mut a, "role_map", json!({ "doc": doc })))["unreadable"], json!(["role_map"]), "{name}");
        tool(&mut a, "role_map", json!({ "doc": doc, "role": "Kids", "maps_to": "" }));
        assert_eq!(bytes(&dir, &mut a, doc, &format!("{name}-after.pdf")), before, "{name}");
        assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "{name}");
    }

    let plain = open(&mut a, "a.pdf");
    tool(&mut a, "doc_protect", json!({ "doc": plain, "open_password": "open", "permissions_password": "boss", "changes": "none" }));
    tool(&mut a, "doc_save", json!({ "doc": plain, "path": "locked.pdf" }));
    let locked = tool(&mut a, "doc_open", json!({ "path": "locked.pdf", "password": "open" }))["doc"].as_u64().unwrap();
    match a.call("role_map", &json!({ "doc": locked, "role": "A", "maps_to": "P" })) {
        Err(ToolError::Failed(m)) => assert!(m.contains("allow") || m.contains("permission"), "{m}"),
        other => panic!("refused: {other:?}"),
    }
    assert_eq!(map(&tool(&mut a, "role_map", json!({ "doc": locked })))["has_structure_tree"], false, "a protected copy still reads");
}
