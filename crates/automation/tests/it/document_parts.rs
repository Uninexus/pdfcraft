//! `dpart_list` reads the PDF 2.0 document part tree (`/DPartRoot`) of a file the tool opens, and
//! the tree survives another edit and a save.

use pdfcraft_automation::{Automation, Content};
use serde_json::{Value, json};

fn call(a: &mut Automation, tool: &str, args: Value) -> Value {
    match a.call(tool, &args).unwrap().remove(0) {
        Content::Json(v) => v,
        other => panic!("expected JSON: {other:?}"),
    }
}

/// A two-page PDF 2.0 file with a root part (`/RecordLevel 1`) over two parts, one per page.
fn pdf_with_parts() -> Vec<u8> {
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R /DPartRoot 5 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Type /DPartRoot /DPartRootNode 6 0 R /RecordLevel 1 >>",
        "<< /Type /DPart /DParts [[7 0 R 8 0 R]] >>",
        "<< /Type /DPart /Parent 6 0 R /Start 3 0 R /End 3 0 R /DPM << /Kind (cover) /Run 2 >> >>",
        "<< /Type /DPart /Parent 6 0 R /Start 4 0 R /End 4 0 R /DPM << /Kind (body) >> >>",
    ];
    let mut out = b"%PDF-2.0\n".to_vec();
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

#[test]
fn document_parts_are_listed_through_the_tool_and_survive_a_save() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("parts.pdf"), pdf_with_parts()).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    let doc = call(&mut a, "doc_open", json!({ "path": "parts.pdf" }))["doc"].as_u64().unwrap();

    let listed = call(&mut a, "dpart_list", json!({ "doc": doc }));
    assert_eq!((listed["has_root"].as_bool(), listed["record_level"].as_i64(), listed["count"].as_u64()), (Some(true), Some(1), Some(3)), "{listed}");
    let parts = listed["parts"].as_array().unwrap();
    let row = |i: usize| {
        let p = &parts[i];
        (p["object"].as_u64().unwrap(), p["parent"].as_u64(), p["depth"].as_u64().unwrap(), p["start_page"].as_u64(), p["end_page"].as_u64())
    };
    assert_eq!([row(0), row(1), row(2)], [(6, None, 0, None, None), (7, Some(6), 1, Some(1), Some(1)), (8, Some(6), 1, Some(2), Some(2))]);
    assert_eq!(parts[1]["metadata"], json!([{ "key": "Kind", "value": "cover" }, { "key": "Run", "value": "2" }]));
    assert_eq!(listed["truncated"], false);

    // Reading is not an edit; the tree is still there after another edit and a save.
    assert_eq!(listed["unsaved_changes"], false);
    call(&mut a, "page_rotate", json!({ "doc": doc, "pages": [1], "degrees": 90 }));
    call(&mut a, "doc_save", json!({ "doc": doc, "path": "out.pdf" }));
    let reopened = call(&mut a, "doc_open", json!({ "path": "out.pdf" }))["doc"].as_u64().unwrap();
    let again = call(&mut a, "dpart_list", json!({ "doc": reopened }));
    // A save may renumber objects, so compare the tree without object numbers.
    let shape = |v: &Value| -> Vec<Value> {
        v["parts"].as_array().unwrap().iter().map(|p| json!([p["depth"], p["start_page"], p["end_page"], p["children"], p["metadata"]])).collect()
    };
    assert_eq!(shape(&again), shape(&listed), "{again}");

    // A document without parts reads as having no root.
    let blank = call(&mut a, "doc_create", json!({ "from": "blank" }));
    let none = call(&mut a, "dpart_list", json!({ "doc": blank["doc"] }));
    assert_eq!((none["has_root"].as_bool(), none["count"].as_u64()), (Some(false), Some(0)), "{none}");
}
