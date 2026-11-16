//! `doc_linearization` reports what a file declares about its linearization (Fast Web View) and
//! whether that still holds after an incremental save.

use pdfcraft_automation::{Automation, Content};
use serde_json::{Value, json};

/// A one-page PDF whose first object is a linearization parameter dictionary (object 5), `/L`
/// being its own length. Its offsets are real; the hint stream (object 6) is a stub.
fn linearized_pdf() -> Vec<u8> {
    let build = |l: usize| {
        let mut out = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
        let mut offsets = [0usize; 7];
        let bodies = [
            (5, format!("<< /Linearized 1 /L {l:08} /H [ 0000000000 0000000000 ] /O 3 /E 0000000000 /N 1 /T 0000000000 >>")),
            (1, "<< /Type /Catalog /Pages 2 0 R >>".to_string()),
            (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string()),
            (3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>".to_string()),
        ];
        for (num, body) in bodies {
            if let Some(o) = offsets.get_mut(num) {
                *o = out.len();
            }
            out.extend_from_slice(format!("{num} 0 obj\n{body}\nendobj\n").as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
        for o in offsets.iter().skip(1).take(5) {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
        out
    };
    let len = build(0).len();
    let mut out = build(len);
    // Offsets that must lie inside the file: patch the fixed-width zeros.
    let patch = |out: &mut Vec<u8>, key: &str, at: usize| {
        let k = format!("{key} ");
        let p = out.windows(k.len()).position(|w| w == k.as_bytes()).unwrap() + k.len();
        out[p..p + 10].copy_from_slice(format!("{at:010}").as_bytes());
    };
    let end = out.len() / 2;
    patch(&mut out, "/E", end);
    patch(&mut out, "/T", end);
    out
}

fn call(a: &mut Automation, tool: &str, args: Value) -> Value {
    match a.call(tool, &args).unwrap().remove(0) {
        Content::Json(v) => v,
        other => panic!("expected JSON: {other:?}"),
    }
}

#[test]
fn a_linearized_file_reads_valid_until_an_incremental_save_appends_to_it() {
    let dir = std::env::temp_dir().join(format!("pdfcraft-linearization-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let bytes = linearized_pdf();
    std::fs::write(dir.join("fast.pdf"), &bytes).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap();
    let doc = call(&mut a, "doc_open", json!({ "path": "fast.pdf" }))["doc"].as_u64().unwrap();

    let r = call(&mut a, "doc_linearization", json!({ "doc": doc }));
    assert_eq!(r["linearized"], true, "{r}");
    assert_eq!(r["valid"], true, "{r}");
    assert_eq!((r["version"].as_f64(), r["first_page_object"].as_u64(), r["page_count"].as_u64()), (Some(1.0), Some(3), Some(1)));
    assert_eq!(
        (r["declared_length"].as_u64(), r["file_length"].as_u64(), r["appended_bytes"].as_u64()),
        (Some(bytes.len() as u64), Some(bytes.len() as u64), Some(0))
    );
    assert_eq!(r["problems"], json!([]));

    // An incremental save appends a revision after /L: the file still opens but is no longer
    // a valid linearized file.
    call(&mut a, "comment_add", json!({ "doc": doc, "page": 1, "type": "note", "at": [20, 20], "contents": "hi" }));
    let dirty = call(&mut a, "doc_linearization", json!({ "doc": doc }));
    assert_eq!((dirty["valid"].as_bool(), dirty["unsaved_changes"].as_bool()), (Some(true), Some(true)), "unsaved edits are not in the report");
    call(&mut a, "doc_save", json!({ "doc": doc }));
    let saved = std::fs::read(dir.join("fast.pdf")).unwrap();
    assert!(saved.starts_with(&bytes), "the save was incremental");
    let r = call(&mut a, "doc_linearization", json!({ "doc": doc }));
    assert_eq!((r["linearized"].as_bool(), r["valid"].as_bool()), (Some(true), Some(false)), "{r}");
    assert_eq!(r["appended_bytes"].as_u64(), Some((saved.len() - bytes.len()) as u64));
    assert!(r["problems"][0].as_str().unwrap().contains("incremental update"), "{r}");
    assert_eq!(r["unsaved_changes"], false);

    // The file reopened from disk reports the same.
    let again = call(&mut a, "doc_open", json!({ "path": "fast.pdf" }))["doc"].as_u64().unwrap();
    assert_eq!(call(&mut a, "doc_linearization", json!({ "doc": again }))["appended_bytes"], r["appended_bytes"]);
}

#[test]
fn an_ordinary_file_is_not_linearized() {
    let dir = std::env::temp_dir().join(format!("pdfcraft-linearization-plain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.txt"), "Plain").unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap();
    let doc = call(&mut a, "doc_create", json!({ "from": "text", "path": "notes.txt" }))["doc"].as_u64().unwrap();
    let r = call(&mut a, "doc_linearization", json!({ "doc": doc }));
    assert_eq!((r["linearized"].as_bool(), r["valid"].as_bool()), (Some(false), Some(false)), "{r}");
    assert_eq!(r["problems"], json!([]));
    assert!(r["declared_length"].is_null());
}
