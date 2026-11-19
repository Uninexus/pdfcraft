//! `ns_list` reads the PDF 2.0 structure namespaces of a file the tool opens, and they survive
//! another edit and a save.

use pdfcraft_automation::{Automation, Content};
use serde_json::{Value, json};

fn call(a: &mut Automation, tool: &str, args: Value) -> Value {
    match a.call(tool, &args).unwrap().remove(0) {
        Content::Json(v) => v,
        other => panic!("expected JSON: {other:?}"),
    }
}

/// A one-page PDF 2.0 file whose structure tree root lists two namespaces.
fn pdf_with_namespaces() -> Vec<u8> {
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 4 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Type /StructTreeRoot /Namespaces [5 0 R 6 0 R] >>",
        "<< /Type /Namespace /NS (http://iso.org/pdf2/ssn) >>",
        "<< /Type /Namespace /NS (urn:example:tags) /Schema << /Type /Filespec /F (tags.rnc) >> /RoleMapNS << /Note [/P 5 0 R] /Plain /Span >> >>",
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
fn structure_namespaces_are_listed_through_the_tool_and_survive_a_save() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ns.pdf"), pdf_with_namespaces()).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    let doc = call(&mut a, "doc_open", json!({ "path": "ns.pdf" }))["doc"].as_u64().unwrap();

    let listed = call(&mut a, "ns_list", json!({ "doc": doc }));
    assert_eq!(
        (listed["has_structure_tree"].as_bool(), listed["count"].as_u64(), listed["truncated"].as_bool()),
        (Some(true), Some(2), Some(false)),
        "{listed}"
    );
    let rows = listed["namespaces"].as_array().unwrap();
    assert_eq!(
        (rows[0]["object"].as_u64(), rows[0]["ns"].as_str(), rows[0]["has_schema"].as_bool()),
        (Some(5), Some("http://iso.org/pdf2/ssn"), Some(false))
    );
    assert_eq!(
        (rows[1]["ns"].as_str(), rows[1]["has_schema"].as_bool(), rows[1]["role_count"].as_u64()),
        (Some("urn:example:tags"), Some(true), Some(2))
    );
    assert_eq!(
        rows[1]["roles"],
        json!([
            { "type": "Note", "role": "P", "role_namespace": "http://iso.org/pdf2/ssn" },
            { "type": "Plain", "role": "Span", "role_namespace": null }
        ])
    );

    assert_eq!(listed["duplicate_ns"], json!([]), "{listed}");

    // Reading is not an edit; the namespaces are still there after another edit and a save.
    assert_eq!(listed["unsaved_changes"], false);
    call(&mut a, "page_rotate", json!({ "doc": doc, "pages": [1], "degrees": 90 }));
    call(&mut a, "doc_save", json!({ "doc": doc, "path": "out.pdf" }));
    let reopened = call(&mut a, "doc_open", json!({ "path": "out.pdf" }))["doc"].as_u64().unwrap();
    let again = call(&mut a, "ns_list", json!({ "doc": reopened }));
    let shape =
        |v: &Value| -> Vec<Value> { v["namespaces"].as_array().unwrap().iter().map(|n| json!([n["ns"], n["has_schema"], n["roles"]])).collect() };
    assert_eq!(shape(&again), shape(&listed), "{again}");

    // A document without a structure tree has no namespaces.
    let blank = call(&mut a, "doc_create", json!({ "from": "blank" }));
    let none = call(&mut a, "ns_list", json!({ "doc": blank["doc"] }));
    assert_eq!((none["has_structure_tree"].as_bool(), none["count"].as_u64()), (Some(false), Some(0)), "{none}");
}

#[test]
fn repeated_namespace_identifiers_are_reported_through_the_tool() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = String::from_utf8(pdf_with_namespaces()).unwrap();
    // The second namespace now repeats the first one's identifier. Offsets in the xref shift, but
    // the reader rebuilds a damaged table.
    let bytes = bytes.replace("(urn:example:tags)", "(http://iso.org/pdf2/ssn)");
    std::fs::write(dir.path().join("dup.pdf"), bytes).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    let doc = call(&mut a, "doc_open", json!({ "path": "dup.pdf" }))["doc"].as_u64().unwrap();
    let listed = call(&mut a, "ns_list", json!({ "doc": doc }));
    assert_eq!(listed["count"], 2, "{listed}");
    assert_eq!(listed["duplicate_ns"], json!([{ "ns": "http://iso.org/pdf2/ssn", "count": 2, "objects": [5, 6] }]), "{listed}");
}
