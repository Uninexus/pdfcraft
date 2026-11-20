//! A rectangle, margin or point argument that holds anything but numbers is refused, not read
//! with the non-numbers left out.
//!
//! `redact_mark`, `link_add`, `link_edit`, `form_add_field`, `form_set_props`, `page_add_text`
//! (`rect` and `at`), `image_edit` move and crop, `doc_header_footer` and `page_set_box` used to
//! keep only the numbers of the list and then check the count, so `[10, "x", 20, 30, 40]` was read
//! as `[10, 20, 30, 40]`: a redaction mark, a link, a field or a crop box landed somewhere the
//! caller never asked for. Each case here carries a list of the right length plus one string.

use pdfcraft_automation::{Automation, Content, ToolError};
use serde_json::{Value, json};

/// A one-page 200 x 300 PDF with an empty content stream.
fn pdf() -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_string(),
        "<< /Length 0 >>\nstream\n\nendstream".to_string(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn json_of(content: Vec<Content>) -> Value {
    match content.into_iter().next() {
        Some(Content::Json(v)) => v,
        _ => panic!("expected JSON"),
    }
}

fn open(test: &str) -> (Automation, u64) {
    let dir = std::env::temp_dir().join(format!("pdfcraft-rect-args-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("in.pdf"), pdf()).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap();
    let doc = json_of(a.call("doc_open", &json!({ "path": "in.pdf" })).unwrap())["doc"].as_u64().unwrap();
    (a, doc)
}

#[test]
fn a_list_with_a_string_in_it_is_refused_and_nothing_changes() {
    let (mut a, doc) = open("refused");
    let stray = json!([10, "x", 20, 30, 40]);
    let url = "https://example.com";
    let cases = [
        ("page_set_box", json!({ "doc": doc, "pages": [1], "margins": stray }), "margins must be an array of 4 numbers"),
        ("page_set_box", json!({ "doc": doc, "pages": [1], "rect": stray }), "rect must be an array of 4 numbers"),
        ("doc_header_footer", json!({ "doc": doc, "pages": [1], "header_center": "hi", "margins": stray }), "margins must be an array of 4 numbers"),
        ("redact_mark", json!({ "doc": doc, "page": 1, "rect": stray }), "rect must be an array of 4 numbers"),
        ("link_add", json!({ "doc": doc, "page": 1, "rect": stray, "url": url }), "rect must be an array of 4 numbers"),
        ("form_add_field", json!({ "doc": doc, "page": 1, "type": "text", "name": "f", "rect": stray }), "rect must be an array of 4 numbers"),
        ("page_add_text", json!({ "doc": doc, "page": 1, "text": "hi", "rect": stray }), "rect must be an array of 4 numbers"),
        ("page_add_text", json!({ "doc": doc, "page": 1, "text": "hi", "at": [10, "x", 20] }), "at must be an array of 2 numbers"),
        // A long list of numbers is refused by its length.
        ("page_add_text", json!({ "doc": doc, "page": 1, "text": "hi", "rect": vec![1; 1000] }), "rect must be an array of 4 numbers"),
    ];
    let mut accepted = Vec::new();
    for (tool, args, want) in cases {
        match a.call(tool, &args) {
            Err(ToolError::InvalidArgs(m)) if m == want => {}
            other => accepted.push(format!("{tool} {args}: {other:?}")),
        }
    }
    assert!(accepted.is_empty(), "these calls were not refused with the expected message: {accepted:#?}");
    let info = json_of(a.call("doc_info", &json!({ "doc": doc })).unwrap());
    assert_eq!(info["document"]["dirty"], json!(false), "a refused call leaves the document as it was: {info}");
    assert_eq!(info["document"]["undo"], Value::Null, "and nothing to undo: {info}");
}

#[test]
fn numbers_in_the_same_places_still_work() {
    let (mut a, doc) = open("accepted");
    json_of(a.call("page_set_box", &json!({ "doc": doc, "pages": [1], "margins": [10, 20, 30, 40] })).unwrap());
    json_of(a.call("page_add_text", &json!({ "doc": doc, "page": 1, "text": "hi", "rect": [10, 10, 100, 40] })).unwrap());
    json_of(a.call("page_add_text", &json!({ "doc": doc, "page": 1, "text": "hi", "at": [10, 100] })).unwrap());
}
