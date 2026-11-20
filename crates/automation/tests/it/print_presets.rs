//! `doc_print_presets`: Document Properties ▸ Advanced ▸ Print Dialog Presets, read back from the
//! saved file's `/ViewerPreferences`.

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
    let bytes = session.create_blank(300.0, 400.0, 3).unwrap();
    let id = session.open("a.pdf", None, bytes, None).unwrap();
    std::fs::write(dir.path().join("a.pdf"), session.save_bytes(id).unwrap().as_slice()).unwrap();
    let auto = Automation::new().with_root(dir.path()).unwrap();
    (dir, auto)
}

fn open(a: &mut Automation, path: &str) -> u64 {
    tool(a, "doc_open", json!({ "path": path }))["doc"].as_u64().unwrap()
}

/// The saved file's `/ViewerPreferences`, as the file holds it.
fn stored(dir: &tempfile::TempDir, name: &str) -> Option<pdfcraft_cos::Dict> {
    let doc = Document::open(Arc::new(std::fs::read(dir.path().join(name)).unwrap())).unwrap();
    let c = doc.get(doc.root().unwrap()).as_dict().cloned().unwrap();
    c.get(b"ViewerPreferences").map(|o| doc.resolve(o)).and_then(|o| o.as_dict().cloned())
}

/// A one-page PDF whose catalog is `catalog_extra` beside `/Type /Catalog /Pages 2 0 R`.
fn raw_pdf(catalog_extra: &str) -> Vec<u8> {
    let bodies = [
        format!("<< /Type /Catalog /Pages 2 0 R {catalog_extra} >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 10 10] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R >>".to_string(),
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
    match a.call("doc_print_presets", &args) {
        Err(ToolError::InvalidArgs(m) | ToolError::Failed(m)) => m,
        other => panic!("refused: {other:?}"),
    }
}

const DEFAULTS: fn() -> Value =
    || json!({ "scaling": "default", "duplex": "default", "pick_tray_by_size": "default", "copies": "default", "page_ranges": [], "unreadable": [] });

#[test]
fn presets_are_set_read_saved_undone_and_redone() {
    let (dir, mut a) = setup();
    let doc = open(&mut a, "a.pdf");
    assert_eq!(tool(&mut a, "doc_print_presets", json!({ "doc": doc }))["print_presets"], DEFAULTS());

    let set = tool(
        &mut a,
        "doc_print_presets",
        json!({ "doc": doc, "scaling": "none", "duplex": "flip_long_edge", "pick_tray_by_size": true, "copies": 3, "page_ranges": [[1, 2], [3, 3]] }),
    );
    let want = json!({
        "scaling": "none", "duplex": "flip_long_edge", "pick_tray_by_size": true, "copies": 3, "page_ranges": [[1, 2], [3, 3]], "unreadable": [],
    });
    assert_eq!(set["print_presets"], want, "{set}");
    // Only the given ones change: a second call with one setting keeps the rest.
    let one = tool(&mut a, "doc_print_presets", json!({ "doc": doc, "copies": 5 }));
    assert_eq!(one["print_presets"]["copies"], 5);
    assert_eq!(one["print_presets"]["duplex"], "flip_long_edge");
    // A call that asks for what is there is not an edit, so it adds no undo step.
    tool(&mut a, "doc_print_presets", json!({ "doc": doc, "copies": 5, "duplex": "flip_long_edge" }));
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "out.pdf" }));
    let vp = stored(&dir, "out.pdf").unwrap();
    assert_eq!(vp.name(b"PrintScaling"), Some(&b"None"[..]));
    assert_eq!(vp.name(b"Duplex"), Some(&b"DuplexFlipLongEdge"[..]));
    assert_eq!(vp.get(b"PickTrayByPDFSize"), Some(&Object::Bool(true)));
    assert_eq!(vp.get(b"NumCopies"), Some(&Object::Int(5)));
    assert_eq!(vp.get(b"PrintPageRange"), Some(&Object::Array([1, 2, 3, 3].map(Object::Int).to_vec())));

    // One undo step per change: two edits, then back to the defaults.
    assert_eq!(tool(&mut a, "edit_undo", json!({ "doc": doc }))["undone"], "Change print dialog presets");
    assert_eq!(tool(&mut a, "doc_print_presets", json!({ "doc": doc }))["print_presets"], want);
    tool(&mut a, "edit_undo", json!({ "doc": doc }));
    assert_eq!(tool(&mut a, "doc_print_presets", json!({ "doc": doc }))["print_presets"], DEFAULTS());
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "only two steps were made");
    tool(&mut a, "edit_redo", json!({ "doc": doc }));
    assert_eq!(tool(&mut a, "doc_print_presets", json!({ "doc": doc }))["print_presets"], want);

    // Defaults remove the entries, and the empty dictionary goes with them.
    let cleared = tool(
        &mut a,
        "doc_print_presets",
        json!({ "doc": doc, "scaling": "default", "duplex": "default", "pick_tray_by_size": "default", "copies": "default", "page_ranges": [] }),
    );
    assert_eq!(cleared["print_presets"], DEFAULTS());
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "cleared.pdf" }));
    assert!(stored(&dir, "cleared.pdf").is_none());
}

#[test]
fn a_refused_call_changes_nothing_not_even_the_valid_arguments_beside_it() {
    let (dir, mut a) = setup();
    let doc = open(&mut a, "a.pdf");
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "before.pdf" }));
    let cases = [
        (json!({ "scaling": "shrink" }), "unknown scaling \"shrink\" (default, none)"),
        (json!({ "duplex": "both" }), "unknown duplex \"both\" (default, simplex, flip_short_edge, flip_long_edge)"),
        (json!({ "scaling": 1 }), "scaling must be a string"),
        (json!({ "pick_tray_by_size": "yes" }), "pick_tray_by_size must be true, false or \"default\" (no entry)"),
        (json!({ "copies": 0 }), "copies must be a whole number from 2 to 5 or \"default\", not 0"),
        (json!({ "copies": 1 }), "copies must be a whole number from 2 to 5 or \"default\", not 1"),
        (json!({ "copies": 6 }), "copies must be a whole number from 2 to 5 or \"default\", not 6"),
        (json!({ "copies": 2.5 }), "copies must be a whole number or \"default\" (no entry)"),
        (json!({ "copies": "3" }), "copies must be a whole number or \"default\" (no entry)"),
        (
            json!({ "page_ranges": "1-3" }),
            "page_ranges must be an array of [first, last] page pairs, 1-based, such as [[1, 3], [7, 7]], or [] for every page",
        ),
        (
            json!({ "page_ranges": [[1]] }),
            "page_ranges must be an array of [first, last] page pairs, 1-based, such as [[1, 3], [7, 7]], or [] for every page",
        ),
        (
            json!({ "page_ranges": [[1, 2, 3]] }),
            "page_ranges must be an array of [first, last] page pairs, 1-based, such as [[1, 3], [7, 7]], or [] for every page",
        ),
        (
            json!({ "page_ranges": [[1, -2]] }),
            "page_ranges must be an array of [first, last] page pairs, 1-based, such as [[1, 3], [7, 7]], or [] for every page",
        ),
        (
            json!({ "page_ranges": [[1.5, 2]] }),
            "page_ranges must be an array of [first, last] page pairs, 1-based, such as [[1, 3], [7, 7]], or [] for every page",
        ),
        (
            json!({ "page_ranges": [["1", 2]] }),
            "page_ranges must be an array of [first, last] page pairs, 1-based, such as [[1, 3], [7, 7]], or [] for every page",
        ),
        (json!({ "page_ranges": vec![json!([1, 1]); 257] }), "page_ranges holds at most 256 ranges, not 257"),
        (json!({ "page_ranges": [[0, 2]] }), "print page range 0-2 must start at page 1 or later and not end before it starts"),
        (json!({ "page_ranges": [[3, 2]] }), "print page range 3-2 must start at page 1 or later and not end before it starts"),
        (json!({ "page_ranges": [[2, 4]] }), "print page range 2-4 goes past the last page (3)"),
        (json!({ "page_ranges": [[1, u64::MAX]] }), "print page range 1-18446744073709551615 goes past the last page (3)"),
    ];
    for (args, message) in cases {
        // A valid argument beside the bad one is not applied either.
        let mut full = args.clone();
        full["doc"] = json!(doc);
        full["duplex"] = full.get("duplex").cloned().unwrap_or(json!("simplex"));
        let got = bad(&mut a, full);
        assert_eq!(got, message, "{args}");
        assert_eq!(tool(&mut a, "doc_print_presets", json!({ "doc": doc }))["print_presets"], DEFAULTS(), "{args}");
    }
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "after.pdf" }));
    assert_eq!(std::fs::read(dir.path().join("before.pdf")).unwrap(), std::fs::read(dir.path().join("after.pdf")).unwrap());
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "no edit was made");
}

#[test]
fn entries_that_are_not_allowed_values_are_named_and_kept_through_a_change_to_another() {
    let (dir, mut a) = setup();
    let bytes = raw_pdf("/ViewerPreferences << /PrintScaling /Shrink /NumCopies 9 /PrintPageRange [2 1] /Duplex 4 >>");
    std::fs::write(dir.path().join("odd.pdf"), bytes).unwrap();
    let doc = open(&mut a, "odd.pdf");
    let read = tool(&mut a, "doc_print_presets", json!({ "doc": doc }));
    assert_eq!(
        read["print_presets"],
        json!({ "scaling": "default", "duplex": "default", "pick_tray_by_size": "default", "copies": "default", "page_ranges": [], "unreadable": ["scaling", "duplex", "copies", "page_ranges"] }),
        "{read}"
    );
    // Asking for the defaults it reads changes nothing and adds no undo step.
    tool(&mut a, "doc_print_presets", json!({ "doc": doc, "copies": "default", "scaling": "default" }));
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err());
    let set = tool(&mut a, "doc_print_presets", json!({ "doc": doc, "pick_tray_by_size": true }));
    assert_eq!(set["print_presets"]["unreadable"], json!(["scaling", "duplex", "copies", "page_ranges"]), "{set}");
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "out.pdf" }));
    let vp = stored(&dir, "out.pdf").unwrap();
    assert_eq!(vp.get(b"NumCopies"), Some(&Object::Int(9)));
    assert_eq!(vp.get(b"Duplex"), Some(&Object::Int(4)));
    assert_eq!(vp.name(b"PrintScaling"), Some(&b"Shrink"[..]));
    assert_eq!(vp.get(b"PickTrayByPDFSize"), Some(&Object::Bool(true)));
    // A real value for one of them replaces it.
    let set = tool(&mut a, "doc_print_presets", json!({ "doc": doc, "copies": 2 }));
    assert_eq!(set["print_presets"]["copies"], 2);
    assert_eq!(set["print_presets"]["unreadable"], json!(["scaling", "duplex", "page_ranges"]), "{set}");
}

#[test]
fn a_viewer_preferences_that_is_not_a_dictionary_is_refused_and_left_and_a_modify_restriction_holds() {
    let (dir, mut a) = setup();
    let out = raw_pdf("/ViewerPreferences 7");
    std::fs::write(dir.path().join("weird.pdf"), out).unwrap();
    let weird = open(&mut a, "weird.pdf");
    assert_eq!(
        bad(&mut a, json!({ "doc": weird, "duplex": "simplex" })),
        "the document's /ViewerPreferences is not a dictionary, so it was left as it is"
    );
    let mut named = DEFAULTS();
    named["unreadable"] = json!(["viewer_preferences"]);
    assert_eq!(tool(&mut a, "doc_print_presets", json!({ "doc": weird }))["print_presets"], named);

    let plain = open(&mut a, "a.pdf");
    tool(&mut a, "doc_protect", json!({ "doc": plain, "open_password": "open", "permissions_password": "boss", "changes": "none" }));
    tool(&mut a, "doc_save", json!({ "doc": plain, "path": "locked.pdf" }));
    let locked = tool(&mut a, "doc_open", json!({ "path": "locked.pdf", "password": "open" }))["doc"].as_u64().unwrap();
    match a.call("doc_print_presets", &json!({ "doc": locked, "copies": 2 })) {
        Err(ToolError::Failed(m)) => assert!(m.contains("allow") || m.contains("permission"), "{m}"),
        other => panic!("refused: {other:?}"),
    }
    // Reading is allowed.
    assert_eq!(tool(&mut a, "doc_print_presets", json!({ "doc": locked }))["print_presets"], DEFAULTS());
}

#[test]
fn an_explicit_false_is_kept_and_a_stored_range_past_the_last_page_is_accepted_when_sent_back_unchanged() {
    let (dir, mut a) = setup();
    let doc = open(&mut a, "a.pdf");
    let r = tool(&mut a, "doc_print_presets", json!({ "doc": doc, "pick_tray_by_size": false }));
    assert_eq!(r["print_presets"]["pick_tray_by_size"], false);
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "false.pdf" }));
    assert_eq!(stored(&dir, "false.pdf").unwrap().get(b"PickTrayByPDFSize"), Some(&Object::Bool(false)));
    let r = tool(&mut a, "doc_print_presets", json!({ "doc": doc, "pick_tray_by_size": "default" }));
    assert_eq!(r["print_presets"], DEFAULTS());

    std::fs::write(dir.path().join("past.pdf"), raw_pdf("/ViewerPreferences << /PrintPageRange [1 1] >>")).unwrap();
    let past = open(&mut a, "past.pdf");
    // The file has one page: a range inside it, sent back, and one past it, refused.
    let r = tool(&mut a, "doc_print_presets", json!({ "doc": past, "page_ranges": [[1, 1]], "duplex": "simplex" }));
    assert_eq!(r["print_presets"]["page_ranges"], json!([[1, 1]]));
    assert_eq!(bad(&mut a, json!({ "doc": past, "page_ranges": [[1, 2]] })), "print page range 1-2 goes past the last page (1)");
    std::fs::write(dir.path().join("far.pdf"), raw_pdf("/ViewerPreferences << /PrintPageRange [2 40] >>")).unwrap();
    let far = open(&mut a, "far.pdf");
    let r = tool(&mut a, "doc_print_presets", json!({ "doc": far, "page_ranges": [[2, 40]], "duplex": "simplex" }));
    assert_eq!((r["print_presets"]["page_ranges"].clone(), r["print_presets"]["duplex"].clone()), (json!([[2, 40]]), json!("simplex")));
    assert_eq!(bad(&mut a, json!({ "doc": far, "page_ranges": [[2, 41]] })), "print page range 2-41 goes past the last page (1)");
}
