//! A name tree whose nodes all name one `/Names` array lists each entry once. The file holds
//! `kids + pairs` objects, so a reader that lists the array for every node that names it would
//! return `kids x pairs` attachments from a file of a few kilobytes.

use pdfcraft_automation::{Automation, Content};
use serde_json::{Value, json};

fn call(a: &mut Automation, tool: &str, args: Value) -> Value {
    match a.call(tool, &args).unwrap().remove(0) {
        Content::Json(v) => v,
        other => panic!("expected JSON: {other:?}"),
    }
}

/// A one-page file whose `/EmbeddedFiles` tree has `kids` leaf nodes, every one `/Names 5 0 R`,
/// where object 5 lists `pairs` file specifications.
fn pdf_with_shared_names(kids: usize, pairs: usize) -> Vec<u8> {
    let entries: String = (0..pairs).map(|i| format!("(f{i:04}.txt) << /Type /Filespec /F (f{i:04}.txt) >> ")).collect();
    let refs: String = (0..kids).map(|i| format!("{} 0 R ", i + 6)).collect();
    let mut bodies = vec![
        "<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles 4 0 R >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        format!("<< /Kids [{refs}] >>"),
        format!("[{entries}]"),
    ];
    bodies.extend((0..kids).map(|_| "<< /Names 5 0 R >>".to_string()));
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

#[test]
fn attachment_add_keeps_each_entry_of_a_shared_names_array_once() {
    let (kids, pairs) = (300, 300);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("shared.pdf"), pdf_with_shared_names(kids, pairs)).unwrap();
    std::fs::write(dir.path().join("new.txt"), b"hello").unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    let doc = call(&mut a, "doc_open", json!({ "path": "shared.pdf" }))["doc"].as_u64().unwrap();
    let before = call(&mut a, "doc_info", json!({ "doc": doc }))["attachments"].as_array().map(Vec::len);
    assert_eq!(before, Some(pairs), "doc_info lists each file specification once");

    let added = call(&mut a, "attachment_add", json!({ "doc": doc, "path": "new.txt" }));
    assert_eq!(added["name"], "new.txt", "{added}");
    let after = call(&mut a, "doc_info", json!({ "doc": doc }))["attachments"].as_array().map(Vec::len).unwrap();
    assert_eq!(after, pairs + 1, "one entry per file specification, plus the one added");
}
