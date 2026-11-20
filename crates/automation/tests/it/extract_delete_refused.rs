//! `page_extract` with `delete: true`: a removal that would be refused is refused before the copy
//! is written or opened, so the call either does both or neither.
use pdfcraft_automation::{Automation, Content};
use serde_json::{Value, json};

fn pdf(pages: usize) -> Vec<u8> {
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 3 + i)).collect();
    let mut objs = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {pages} /MediaBox [0 0 200 300] >>", kids.join(" ")),
    ];
    for _ in 0..pages {
        objs.push("<< /Type /Page /Parent 2 0 R >>".to_string());
    }
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

fn call(a: &mut Automation, tool: &str, args: Value) -> Value {
    match a.call(tool, &args).unwrap_or_else(|e| panic!("{tool}: {e}")).remove(0) {
        Content::Json(v) => v,
        _ => panic!("{tool}: expected JSON"),
    }
}

fn open(dir: &std::path::Path, pages: usize) -> (Automation, u64) {
    std::fs::write(dir.join("in.pdf"), pdf(pages)).unwrap();
    let mut a = Automation::new().with_root(dir).unwrap();
    let id = call(&mut a, "doc_open", json!({ "path": "in.pdf" }))["doc"].as_u64().unwrap();
    (a, id)
}

fn documents(a: &mut Automation) -> Vec<Value> {
    call(a, "doc_list", json!({}))["documents"].as_array().unwrap().clone()
}

const REFUSAL: &str = "a document must keep at least one page";

#[test]
fn extracting_every_page_with_delete_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 2);
    let before = call(&mut a, "doc_info", json!({ "doc": id }))["document"].clone();
    let error = a.call("page_extract", &json!({ "doc": id, "pages": [1, 2], "out": "all.pdf", "delete": true })).expect_err("every page cannot go");
    assert_eq!(error.to_string(), REFUSAL);
    assert!(!dir.path().join("all.pdf").exists(), "the copy must not be written");
    assert_eq!(documents(&mut a).len(), 1, "no copy may be opened (out is given, so open is off)");
    assert_eq!(call(&mut a, "doc_info", json!({ "doc": id }))["document"], before);
    // Page 2 given twice and page 1 once is still every page.
    let error =
        a.call("page_extract", &json!({ "doc": id, "pages": [2, 1, 2], "out": "all.pdf", "delete": true })).expect_err("every page cannot go");
    assert_eq!(error.to_string(), REFUSAL);
    assert!(!dir.path().join("all.pdf").exists());
}

#[test]
fn extracting_every_page_with_delete_opens_no_copy() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 2);
    let error = a.call("page_extract", &json!({ "doc": id, "pages": [1, 2], "open": true, "delete": true })).expect_err("every page cannot go");
    assert_eq!(error.to_string(), REFUSAL);
    assert_eq!(documents(&mut a).len(), 1, "the copy must not be left open");
}

#[test]
fn extracting_every_page_separately_with_delete_makes_no_folder() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 2);
    let error = a
        .call("page_extract", &json!({ "doc": id, "pages": [1, 2], "separate": true, "out_dir": "parts", "delete": true }))
        .expect_err("every page cannot go");
    assert_eq!(error.to_string(), REFUSAL);
    assert!(!dir.path().join("parts").exists(), "no folder and no page files");
}

#[test]
fn extracting_some_pages_with_delete_still_does_both() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 2);
    let out = call(&mut a, "page_extract", json!({ "doc": id, "pages": [2], "out": "one.pdf", "delete": true }));
    assert!(dir.path().join("one.pdf").exists());
    assert_eq!(out["original"]["pages"], 1, "{out}");
    assert_eq!(documents(&mut a).len(), 1);
    assert_eq!(call(&mut a, "doc_info", json!({ "doc": id }))["document"]["undo"], "Delete page");
}

#[test]
fn a_mistyped_delete_is_refused_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 2);
    let error = a.call("page_extract", &json!({ "doc": id, "pages": [2], "out": "one.pdf", "delete": "yes" })).expect_err("delete must be a boolean");
    assert_eq!(error.to_string(), "delete must be true or false");
    assert!(!dir.path().join("one.pdf").exists(), "the copy must not be written");
    assert_eq!(documents(&mut a).len(), 1);
}
