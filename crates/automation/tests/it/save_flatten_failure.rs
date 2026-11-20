//! `doc_save` with `flatten_fill_sign`: the flatten is an undoable edit made only to be saved, so a
//! save that is refused leaves the document, its unsaved-changes flag and its undo steps as they were.

use pdfcraft_automation::{Automation, Content};
use serde_json::{Value, json};
use std::path::PathBuf;

fn pdf() -> Vec<u8> {
    let content = "0 g 10 10 20 20 re f";
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
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

fn call(a: &mut Automation, tool: &str, args: Value) -> Value {
    match a.call(tool, &args).unwrap_or_else(|e| panic!("{tool}: {e}")).remove(0) {
        Content::Json(v) => v,
        _ => panic!("{tool}: expected JSON"),
    }
}

fn workdir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pdfcraft-save-flatten-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("in.pdf"), pdf()).unwrap();
    // A folder where the file would go: the write is refused after the bytes are made.
    std::fs::create_dir(dir.join("taken.pdf")).unwrap();
    dir
}

fn contents(a: &mut Automation, doc: u64) -> Vec<String> {
    let list = call(a, "comment_list", json!({ "doc": doc }));
    list["comments"].as_array().unwrap().iter().map(|c| c["contents"].as_str().unwrap_or_default().to_string()).collect()
}

fn state(a: &mut Automation, doc: u64) -> (Value, Vec<String>) {
    (call(a, "doc_info", json!({ "doc": doc }))["document"].clone(), contents(a, doc))
}

fn refused(a: &mut Automation, dir: &std::path::Path, doc: u64) {
    let error =
        a.call("doc_save", &json!({ "doc": doc, "path": "taken.pdf", "flatten_fill_sign": true })).expect_err("a folder cannot be written as a file");
    assert_eq!(error.to_string(), format!("{}: is a folder, not a file", dir.join("taken.pdf").display()));
}

#[test]
fn a_refused_save_leaves_the_flatten_untaken_on_a_document_with_unsaved_changes() {
    let dir = workdir("dirty");
    let mut a = Automation::new().with_root(&dir).unwrap().with_clock(|| 1_700_000_000);
    let doc = call(&mut a, "doc_open", json!({ "path": "in.pdf" }))["doc"].as_u64().unwrap();
    call(&mut a, "page_rotate", json!({ "doc": doc, "pages": [1], "degrees": 90 }));
    call(&mut a, "fill_sign_add", json!({ "doc": doc, "page": 1, "type": "text", "at": [40.0, 80.0], "text": "Hello" }));
    let before = state(&mut a, doc);
    assert_eq!(before.0["dirty"], true);
    assert_eq!(before.1, ["Hello"]);

    refused(&mut a, &dir, doc);
    assert_eq!(state(&mut a, doc), before, "a refused save changed the document, its flag or its undo step");
    // The step the refused call would have added is not there: the next undo is the typewriter.
    assert_eq!(call(&mut a, "edit_undo", json!({ "doc": doc }))["undone"], "Add text");
    call(&mut a, "edit_redo", json!({ "doc": doc }));

    // The same save to a place that accepts it still flattens, as one undo step.
    call(&mut a, "doc_save", json!({ "doc": doc, "path": "flat.pdf", "flatten_fill_sign": true }));
    assert_eq!(contents(&mut a, doc), Vec::<String>::new());
    assert_eq!(call(&mut a, "edit_undo", json!({ "doc": doc }))["undone"], "Flatten Fill & Sign");
    assert_eq!(contents(&mut a, doc), ["Hello"]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_refused_save_leaves_a_saved_document_saved() {
    let dir = workdir("clean");
    let mut a = Automation::new().with_root(&dir).unwrap().with_clock(|| 1_700_000_000);
    let doc = call(&mut a, "doc_open", json!({ "path": "in.pdf" }))["doc"].as_u64().unwrap();
    call(&mut a, "fill_sign_add", json!({ "doc": doc, "page": 1, "type": "text", "at": [40.0, 80.0], "text": "Hello" }));
    call(&mut a, "doc_save", json!({ "doc": doc, "path": "kept.pdf" }));
    let before = state(&mut a, doc);
    assert_eq!(before.0["dirty"], false);

    refused(&mut a, &dir, doc);
    assert_eq!(state(&mut a, doc), before, "a refused save changed the document, its flag or its undo step");
    std::fs::remove_dir_all(dir).unwrap();
}

/// A form with a text field `log` and a Will Save action that sets it.
fn scripted_pdf() -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R /AA << /WS 6 0 R >> /AcroForm << /Fields [4 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 5 0 R >> >> >> >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Annots [4 0 R] >>".to_string(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (log) /V () /Rect [20 250 180 270] /P 3 0 R /F 4 >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
        "<< /S /JavaScript /JS (getField\\('log'\\).value = 'saving';) >>".to_string(),
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

#[test]
fn a_refused_save_keeps_the_flatten_when_a_will_save_script_changed_the_document_after_it() {
    let dir = workdir("script");
    std::fs::write(dir.join("form.pdf"), scripted_pdf()).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap().with_clock(|| 1_700_000_000);
    let doc = call(&mut a, "doc_open", json!({ "path": "form.pdf" }))["doc"].as_u64().unwrap();
    call(&mut a, "fill_sign_add", json!({ "doc": doc, "page": 1, "type": "text", "at": [40.0, 80.0], "text": "Hello" }));

    let error =
        a.call("doc_save", &json!({ "doc": doc, "path": "taken.pdf", "flatten_fill_sign": true })).expect_err("a folder cannot be written as a file");
    assert_eq!(
        error.to_string(),
        format!(
            "{}: is a folder, not a file; the Fill & Sign flatten was kept, because the document changed after it (a Will Save script): undo it to take it back",
            dir.join("taken.pdf").display()
        )
    );
    // The script ran and its change is the last step; the flatten is the one under it, not lost.
    assert_eq!(contents(&mut a, doc), Vec::<String>::new());
    let info = call(&mut a, "doc_info", json!({ "doc": doc }))["document"].clone();
    assert_eq!(info["dirty"], true);
    assert_eq!(info["undo"], "Run JavaScript");
    assert_eq!(log_value(&mut a, doc), "saving");
    call(&mut a, "edit_undo", json!({ "doc": doc }));
    assert_eq!(call(&mut a, "doc_info", json!({ "doc": doc }))["document"]["undo"], "Flatten Fill & Sign");
    assert_eq!(call(&mut a, "edit_undo", json!({ "doc": doc }))["undone"], "Flatten Fill & Sign");
    assert_eq!(contents(&mut a, doc), ["Hello"]);
    std::fs::remove_dir_all(dir).unwrap();
}

fn log_value(a: &mut Automation, doc: u64) -> Value {
    let f = call(a, "form_fields", json!({ "doc": doc }));
    f["fields"].as_array().unwrap().iter().find(|f| f["name"] == "log").map(|f| f["value"].clone()).unwrap()
}
