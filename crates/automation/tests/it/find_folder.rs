//! `text_find_folder`: the text search of every PDF in a folder, end to end on synthetic files.

use std::path::{Path, PathBuf};

use pdfcraft_automation::{Automation, Content, ToolError};
use serde_json::{Value, json};

/// A fresh directory for one test.
fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pdfcraft-find-folder-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap()
}

fn ok(a: &mut Automation, args: Value) -> Value {
    match a.call("text_find_folder", &args) {
        Ok(mut c) => match c.remove(0) {
            Content::Json(v) => v,
            other => panic!("expected JSON, got {other:?}"),
        },
        Err(e) => panic!("text_find_folder {args}: {e}"),
    }
}

fn err(a: &mut Automation, args: Value) -> String {
    match a.call("text_find_folder", &args) {
        Err(ToolError::InvalidArgs(m) | ToolError::Failed(m)) => m,
        other => panic!("expected an error for {args}, got {other:?}"),
    }
}

/// A PDF with one line of text on each of `pages` (Helvetica).
fn pdf(pages: &[&str]) -> Vec<u8> {
    let n = pages.len();
    let mut objs = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {n} >>", (0..n).map(|i| format!("{} 0 R", 4 + 2 * i)).collect::<Vec<_>>().join(" ")),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
    ];
    for (i, text) in pages.iter().enumerate() {
        let body = format!("BT /F1 14 Tf 72 700 Td ({text}) Tj ET");
        objs.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
            5 + 2 * i
        ));
        objs.push(format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()));
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

fn write(dir: &Path, name: &str, bytes: &[u8]) {
    if let Some(parent) = dir.join(name).parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(dir.join(name), bytes).unwrap();
}

/// A folder of three good PDFs, a corrupt one, a text file named .txt and a sub-folder with one more.
fn folder(name: &str) -> PathBuf {
    let d = tmp(name);
    write(&d, "a.pdf", &pdf(&["Cover page", "The quarterly invoice is due"]));
    write(&d, "b.PDF", &pdf(&["Invoice number 42", "Nothing here", "Another Invoice"]));
    write(&d, "c.pdf", &pdf(&["Only unrelated words"]));
    write(&d, "broken.pdf", b"this is not a pdf at all");
    write(&d, "notes.txt", b"invoice in a text file is never searched");
    write(&d, "sub/deep.pdf", &pdf(&["A deep invoice"]));
    d
}

fn names(v: &Value, key: &str) -> Vec<String> {
    v[key].as_array().unwrap().iter().map(|f| f["relative"].as_str().unwrap().to_owned()).collect()
}

#[test]
fn matches_across_two_files_with_pages_and_snippets() {
    let dir = folder("two");
    let mut a = Automation::new().with_root(&dir).unwrap();
    let r = ok(&mut a, json!({ "folder": ".", "query": "invoice" }));
    assert_eq!(names(&r, "files"), ["a.pdf", "b.PDF"], "path order; the sub-folder is not entered by default: {r}");
    assert_eq!(r["count"], 3, "{r}");
    assert_eq!(r["files_found"], 4, "four .pdf files at the top level: {r}");
    assert_eq!(r["files_searched"], 3);
    assert_eq!(r["files_with_matches"], 2);
    let a_pdf = &r["files"][0];
    assert_eq!(a_pdf["pages"], 2);
    assert_eq!(a_pdf["matches"][0]["page"], 2, "1-based: {a_pdf}");
    assert_eq!(a_pdf["matches"][0]["text"], "invoice");
    assert!(a_pdf["matches"][0]["before"].as_str().unwrap().ends_with("quarterly"), "{a_pdf}");
    assert!(a_pdf["matches"][0]["snippet"].as_str().unwrap().contains("invoice is due"), "{a_pdf}");
    let b_pages: Vec<i64> = r["files"][1]["matches"].as_array().unwrap().iter().map(|m| m["page"].as_i64().unwrap()).collect();
    assert_eq!(b_pages, [1, 3]);
    // The path is one doc_open accepts.
    let path = a_pdf["path"].as_str().unwrap();
    assert!(Path::new(path).is_absolute());
    let doc = a.call("doc_open", &json!({ "path": path })).unwrap();
    assert!(matches!(&doc[0], Content::Json(v) if v["pages"] == 2));
}

#[test]
fn options_agree_with_the_find_bar() {
    let dir = folder("opts");
    let mut a = Automation::new().with_root(&dir).unwrap();
    let r = ok(&mut a, json!({ "folder": ".", "query": "Invoice", "case_sensitive": true }));
    assert_eq!(r["count"], 2, "`Invoice number 42` and `Another Invoice`, not the lower-case ones: {r}");
    let r = ok(&mut a, json!({ "folder": ".", "query": "invoic", "whole_words": true }));
    assert_eq!(r["count"], 0, "a prefix is not a whole word: {r}");
    let r = ok(&mut a, json!({ "folder": ".", "query": "invoice", "recursive": true }));
    assert_eq!(names(&r, "files"), ["a.pdf", "b.PDF", "sub/deep.pdf"], "{r}");
}

#[test]
fn no_match_is_an_empty_complete_answer() {
    let dir = folder("none");
    let mut a = Automation::new().with_root(&dir).unwrap();
    let r = ok(&mut a, json!({ "folder": ".", "query": "zebra" }));
    assert_eq!((r["count"].as_i64(), r["files"].as_array().map(Vec::len)), (Some(0), Some(0)), "{r}");
    assert_eq!(r["files_searched"], 3);
    assert_eq!(r["complete"], true);
}

#[test]
fn a_corrupt_file_is_reported_as_skipped_not_fatal() {
    let dir = folder("corrupt");
    let mut a = Automation::new().with_root(&dir).unwrap();
    let r = ok(&mut a, json!({ "folder": ".", "query": "invoice" }));
    assert_eq!(r["files_skipped"], 1, "{r}");
    let skipped = &r["skipped"][0];
    assert_eq!(skipped["relative"], "broken.pdf");
    assert!(skipped["reason"].as_str().unwrap().contains("not a readable PDF"), "{skipped}");
    assert_eq!(r["count"], 3, "the readable files were still searched");
}

#[test]
fn a_password_protected_file_is_skipped_with_its_reason() {
    let dir = folder("locked");
    let mut a = Automation::new().with_root(&dir).unwrap();
    let id = match &a.call("doc_open", &json!({ "path": "c.pdf" })).unwrap()[0] {
        Content::Json(v) => v["doc"].as_u64().unwrap(),
        other => panic!("{other:?}"),
    };
    a.call("doc_protect", &json!({ "doc": id, "open_password": "secret" })).unwrap();
    a.call("doc_save", &json!({ "doc": id, "path": "locked.pdf" })).unwrap();
    let r = ok(&mut a, json!({ "folder": ".", "query": "invoice" }));
    let locked = r["skipped"].as_array().unwrap().iter().find(|s| s["relative"] == "locked.pdf").unwrap_or_else(|| panic!("{r}"));
    assert_eq!(locked["reason"], "password protected");
    assert_eq!(r["count"], 3);
}

#[test]
fn caps_on_files_and_results_say_so() {
    let dir = folder("caps");
    let mut a = Automation::new().with_root(&dir).unwrap();
    let r = ok(&mut a, json!({ "folder": ".", "query": "invoice", "limit": 2 }));
    assert_eq!(r["count"], 2, "{r}");
    assert_eq!(r["results_capped"], true);
    assert_eq!(r["complete"], false);
    let r = ok(&mut a, json!({ "folder": ".", "query": "invoice", "limit": 3 }));
    assert_eq!((r["count"].as_i64(), r["results_capped"].as_bool()), (Some(3), Some(false)), "exactly the limit is not a cap: {r}");
    let r = ok(&mut a, json!({ "folder": ".", "query": "invoice", "max_files": 1 }));
    assert_eq!(r["files_found"], 4);
    assert_eq!(r["files_searched"], 1, "{r}");
    assert_eq!(r["files_capped"], true);
    assert_eq!(names(&r, "files"), ["a.pdf"]);
}

#[test]
fn a_folder_outside_the_root_is_refused_and_arguments_are_checked() {
    let base = tmp("root");
    let inside = base.join("inside");
    std::fs::create_dir_all(&inside).unwrap();
    write(&base, "outside.pdf", &pdf(&["invoice"]));
    write(&inside, "in.pdf", &pdf(&["invoice"]));
    let mut a = Automation::new().with_root(&inside).unwrap();
    for folder in ["..", base.to_str().unwrap(), "../inside/..", "/"] {
        let m = err(&mut a, json!({ "folder": folder, "query": "invoice" }));
        assert!(m.contains("outside the allowed directory"), "{folder}: {m}");
    }
    assert_eq!(ok(&mut a, json!({ "folder": ".", "query": "invoice" }))["count"], 1);
    assert!(err(&mut a, json!({ "folder": "in.pdf", "query": "invoice" })).contains("not a folder"));
    assert!(err(&mut a, json!({ "folder": "missing", "query": "invoice" })).contains("not found"));
    assert!(err(&mut a, json!({ "folder": ".", "query": "  " })).contains("query"));
    assert!(err(&mut a, json!({ "folder": ".", "query": "x", "limit": 0 })).contains("between 1 and 5000"));
    assert!(err(&mut a, json!({ "folder": ".", "query": "x", "max_files": 2001 })).contains("between 1 and 2000"));
    assert!(err(&mut a, json!({ "folder": ".", "query": "x", "recursive": "yes" })).contains("true or false"));
    assert!(err(&mut a, json!({ "folder": ".", "query": "x", "bogus": 1 })).contains("unknown argument"));
    assert!(err(&mut a, json!({ "query": "x" })).contains("missing argument folder"));
}

/// A link to a PDF outside the root is listed as skipped, never read.
#[cfg(unix)]
#[test]
fn a_link_out_of_the_root_is_not_followed() {
    let base = tmp("links");
    let inside = base.join("inside");
    std::fs::create_dir_all(&inside).unwrap();
    write(&base, "secret.pdf", &pdf(&["invoice secret"]));
    std::os::unix::fs::symlink(base.join("secret.pdf"), inside.join("link.pdf")).unwrap();
    std::os::unix::fs::symlink(&base, inside.join("up")).unwrap();
    write(&inside, "in.pdf", &pdf(&["invoice"]));
    let mut a = Automation::new().with_root(&inside).unwrap();
    let r = ok(&mut a, json!({ "folder": ".", "query": "invoice", "recursive": true }));
    assert_eq!(names(&r, "files"), ["in.pdf"], "{r}");
    assert_eq!(names(&r, "skipped"), ["link.pdf"], "{r}");
}

/// A file name that is not UTF-8 is marked, not hidden.
#[cfg(unix)]
#[test]
fn a_name_that_is_not_utf8_is_flagged() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tmp("lossy");
    let name = std::ffi::OsStr::from_bytes(b"caf\xe9.pdf");
    std::fs::write(dir.join(name), pdf(&["invoice"])).unwrap();
    write(&dir, "plain.pdf", &pdf(&["invoice"]));
    let mut a = Automation::new().with_root(&dir).unwrap();
    let r = ok(&mut a, json!({ "folder": ".", "query": "invoice" }));
    let lossy: Vec<bool> = r["files"].as_array().unwrap().iter().map(|f| f["path_lossy"].as_bool().unwrap()).collect();
    assert_eq!(lossy.iter().filter(|l| **l).count(), 1, "{r}");
    assert_eq!(r["count"], 2);
}
