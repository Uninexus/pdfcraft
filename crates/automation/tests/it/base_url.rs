//! `doc_base_url`: the base URL for relative links (catalog `/URI` `/Base`), read back from the
//! saved file and through `link_list`.

use pdfcraft_automation::{Automation, Content, ToolError};
use pdfcraft_cos::{Document, Object, PdfString};
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

/// The saved file's catalog `/URI`, as the file holds it.
fn stored(dir: &tempfile::TempDir, name: &str) -> Option<pdfcraft_cos::Dict> {
    let doc = Document::open(Arc::new(std::fs::read(dir.path().join(name)).unwrap())).unwrap();
    let c = doc.get(doc.root().unwrap()).as_dict().cloned().unwrap();
    c.get(b"URI").map(|o| doc.resolve(o)).and_then(|o| o.as_dict().cloned())
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
    match a.call("doc_base_url", &args) {
        Err(ToolError::InvalidArgs(m) | ToolError::Failed(m)) => m,
        other => panic!("refused: {other:?}"),
    }
}

fn none() -> Value {
    json!({ "base": null, "links_use_it": false, "unreadable": [] })
}

#[test]
fn a_base_is_set_read_saved_used_by_links_undone_and_redone() {
    let (dir, mut a) = setup();
    let doc = open(&mut a, "a.pdf");
    assert_eq!(tool(&mut a, "doc_base_url", json!({ "doc": doc }))["base_url"], none());
    tool(&mut a, "link_add", json!({ "doc": doc, "page": 1, "rect": [10, 10, 100, 30], "url": "page.html" }));
    let link = |a: &mut Automation| tool(a, "link_list", json!({ "doc": doc }))["links"][0].clone();
    assert_eq!(link(&mut a).get("resolved_url"), None, "no base, nothing to resolve against");

    let set = tool(&mut a, "doc_base_url", json!({ "doc": doc, "base": "https://example.com/docs/" }));
    assert_eq!(set["base_url"], json!({ "base": "https://example.com/docs/", "links_use_it": true, "unreadable": [] }), "{set}");
    assert_eq!(link(&mut a)["resolved_url"], "https://example.com/docs/page.html");
    // Asking for what is stored is not an edit: no undo step is added.
    tool(&mut a, "doc_base_url", json!({ "doc": doc, "base": "https://example.com/docs/" }));
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "out.pdf" }));
    let uri = stored(&dir, "out.pdf").unwrap();
    assert_eq!(uri.get(b"Base"), Some(&Object::String(PdfString::literal(b"https://example.com/docs/".to_vec()))));

    // One undo step, labelled; a second undo has nothing to undo (the link was the first edit).
    assert_eq!(tool(&mut a, "edit_undo", json!({ "doc": doc }))["undone"], "Change base URL");
    assert_eq!(tool(&mut a, "doc_base_url", json!({ "doc": doc }))["base_url"], none());
    assert_eq!(link(&mut a).get("resolved_url"), None);
    tool(&mut a, "edit_redo", json!({ "doc": doc }));
    assert_eq!(link(&mut a)["resolved_url"], "https://example.com/docs/page.html");

    // An empty string takes the base away, with the dictionary.
    assert_eq!(tool(&mut a, "doc_base_url", json!({ "doc": doc, "base": "" }))["base_url"], none());
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "cleared.pdf" }));
    assert!(stored(&dir, "cleared.pdf").is_none());
}

#[test]
fn a_refused_call_writes_nothing_and_adds_no_undo_step() {
    let (dir, mut a) = setup();
    let doc = open(&mut a, "a.pdf");
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "before.pdf" }));
    let web = "a base URL must be a web address that names its host, such as https://example.com/docs/";
    let ascii = "a base URL must be printable ASCII with no space";
    let too_long = format!("https://e.example/{}", "a".repeat(8192 - 17));
    for (base, message) in [
        (json!("ftp://example.com/"), web.to_owned()),
        (json!("example.com"), web.to_owned()),
        (json!("https://"), web.to_owned()),
        (json!("https://example.com/a b"), ascii.to_owned()),
        (json!("https://example.com/\u{e9}"), ascii.to_owned()),
        (json!(too_long), "a base URL holds at most 8192 bytes, not 8193".to_owned()),
        (json!(7), "base must be a string".to_owned()),
        (json!(["https://example.com/"]), "base must be a string".to_owned()),
    ] {
        assert_eq!(bad(&mut a, json!({ "doc": doc, "base": base })), message, "{base}");
    }
    assert_eq!(tool(&mut a, "doc_base_url", json!({ "doc": doc }))["base_url"], none());
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "after.pdf" }));
    assert_eq!(std::fs::read(dir.path().join("before.pdf")).unwrap(), std::fs::read(dir.path().join("after.pdf")).unwrap());
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "no edit was made");
    // A null is left out like any other argument: it reads.
    assert_eq!(tool(&mut a, "doc_base_url", json!({ "doc": doc, "base": null }))["base_url"], none());
}

#[test]
fn a_stored_base_that_cannot_be_read_is_named_and_kept_through_none() {
    let (dir, mut a) = setup();
    std::fs::write(dir.path().join("odd.pdf"), raw_pdf("/URI << /Base 42 /IsMap true >>")).unwrap();
    let doc = open(&mut a, "odd.pdf");
    let read = tool(&mut a, "doc_base_url", json!({ "doc": doc }));
    assert_eq!(read["base_url"], json!({ "base": null, "links_use_it": false, "unreadable": ["base"] }), "{read}");
    // Asking for none is asking for what is read: nothing changes, and no undo step is added.
    tool(&mut a, "doc_base_url", json!({ "doc": doc, "base": "" }));
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err());
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "kept.pdf" }));
    assert_eq!(stored(&dir, "kept.pdf").unwrap().get(b"Base"), Some(&Object::Int(42)));
    // A real base replaces it and keeps the other entry of /URI.
    let set = tool(&mut a, "doc_base_url", json!({ "doc": doc, "base": "http://example.org/a/" }));
    assert_eq!(set["base_url"], json!({ "base": "http://example.org/a/", "links_use_it": true, "unreadable": [] }), "{set}");
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "set.pdf" }));
    let uri = stored(&dir, "set.pdf").unwrap();
    assert_eq!(uri.get(b"IsMap"), Some(&Object::Bool(true)));
    assert_eq!(uri.get(b"Base"), Some(&Object::String(PdfString::literal(b"http://example.org/a/".to_vec()))));
}

#[test]
fn a_uri_that_is_not_a_dictionary_is_refused_and_left_and_a_modify_restriction_holds() {
    let (dir, mut a) = setup();
    std::fs::write(dir.path().join("weird.pdf"), raw_pdf("/URI 7")).unwrap();
    let weird = open(&mut a, "weird.pdf");
    tool(&mut a, "doc_save", json!({ "doc": weird, "path": "weird-before.pdf" }));
    assert_eq!(
        bad(&mut a, json!({ "doc": weird, "base": "https://example.com/" })),
        "the document's /URI is not a dictionary, or is behind more than 128 references, so it was left as it is"
    );
    assert_eq!(
        tool(&mut a, "doc_base_url", json!({ "doc": weird }))["base_url"],
        json!({ "base": null, "links_use_it": false, "unreadable": ["uri"] })
    );
    tool(&mut a, "doc_save", json!({ "doc": weird, "path": "weird-after.pdf" }));
    assert_eq!(std::fs::read(dir.path().join("weird-before.pdf")).unwrap(), std::fs::read(dir.path().join("weird-after.pdf")).unwrap());

    let plain = open(&mut a, "a.pdf");
    tool(&mut a, "doc_protect", json!({ "doc": plain, "open_password": "open", "permissions_password": "boss", "changes": "none" }));
    tool(&mut a, "doc_save", json!({ "doc": plain, "path": "locked.pdf" }));
    let locked = tool(&mut a, "doc_open", json!({ "path": "locked.pdf", "password": "open" }))["doc"].as_u64().unwrap();
    match a.call("doc_base_url", &json!({ "doc": locked, "base": "https://example.com/" })) {
        Err(ToolError::Failed(m)) => assert!(m.contains("allow") || m.contains("permission"), "{m}"),
        other => panic!("refused: {other:?}"),
    }
    assert_eq!(tool(&mut a, "doc_base_url", json!({ "doc": locked }))["base_url"], none(), "reading is allowed");
}

#[test]
fn a_base_that_links_cannot_use_is_reported_as_not_used() {
    let (dir, mut a) = setup();
    // Stored by a file, not written by the tool: a UTF-8 base that is not a web address.
    std::fs::write(dir.path().join("ftp.pdf"), raw_pdf("/URI << /Base (ftp://example.com/x/) >>")).unwrap();
    let doc = open(&mut a, "ftp.pdf");
    assert_eq!(
        tool(&mut a, "doc_base_url", json!({ "doc": doc }))["base_url"],
        json!({ "base": "ftp://example.com/x/", "links_use_it": false, "unreadable": [] })
    );
}

#[test]
fn a_stored_base_sent_back_is_accepted_and_the_limit_is_the_viewers() {
    let (dir, mut a) = setup();
    assert_eq!(pdfcraft_engine::MAX_BASE_URL_BYTES, pdfcraft_render::MAX_URI_BASE_BYTES, "a base is written up to the length the viewer reads");
    std::fs::write(dir.path().join("ftp.pdf"), raw_pdf("/URI << /Base (ftp://example.com/x/) >>")).unwrap();
    let doc = open(&mut a, "ftp.pdf");
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "before.pdf" }));
    // The stored value sent back is no change: accepted, no undo step; a different one is refused.
    let same = tool(&mut a, "doc_base_url", json!({ "doc": doc, "base": "ftp://example.com/x/" }));
    assert_eq!(same["base_url"]["base"], "ftp://example.com/x/");
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err());
    assert_eq!(
        bad(&mut a, json!({ "doc": doc, "base": "ftp://example.com/y/" })),
        "a base URL must be a web address that names its host, such as https://example.com/docs/"
    );
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "after.pdf" }));
    assert_eq!(std::fs::read(dir.path().join("before.pdf")).unwrap(), std::fs::read(dir.path().join("after.pdf")).unwrap());
    // The address is not parsed: a base with a port that no URL has is written, and says it is not used.
    let odd = tool(&mut a, "doc_base_url", json!({ "doc": doc, "base": "https://a:99999/" }));
    assert_eq!(odd["base_url"], json!({ "base": "https://a:99999/", "links_use_it": false, "unreadable": [] }), "{odd}");
}

#[test]
fn a_uri_that_is_a_stream_is_read_like_link_list_reads_it() {
    let (dir, mut a) = setup();
    // A page with a relative link; /URI is a stream, and /Base is reached through a chain.
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R /URI 6 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Annots [4 0 R] >>".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [0 0 50 50] /Border [0 0 0] /A << /S /URI /URI (p.html) >> >>".to_string(),
        "(https://stream.example/d/)".to_string(),
        "<< /Length 1 /Base 5 0 R >>\nstream\nx\nendstream".to_string(),
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
    std::fs::write(dir.path().join("stream.pdf"), out).unwrap();
    let doc = open(&mut a, "stream.pdf");
    let links = tool(&mut a, "link_list", json!({ "doc": doc }));
    assert_eq!(links["links"][0]["resolved_url"], "https://stream.example/d/p.html", "{links}");
    let read = tool(&mut a, "doc_base_url", json!({ "doc": doc }));
    assert_eq!(read["base_url"], json!({ "base": "https://stream.example/d/", "links_use_it": true, "unreadable": [] }), "{read}");
}
