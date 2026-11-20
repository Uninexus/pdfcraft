//! A tool that is going to refuse writes nothing: a mistyped `open` is refused before `out` is
//! written (`page_extract`, `doc_combine`, `comments_summarize`), and a `doc_split` whose later
//! part cannot be written removes the parts it wrote to a free name.
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
    std::fs::write(dir.join("two.pdf"), pdf(1)).unwrap();
    let mut a = Automation::new().with_root(dir).unwrap();
    let id = call(&mut a, "doc_open", json!({ "path": "in.pdf" }))["doc"].as_u64().unwrap();
    (a, id)
}

fn documents(a: &mut Automation) -> usize {
    call(a, "doc_list", json!({}))["documents"].as_array().unwrap().len()
}

fn names(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

#[test]
fn a_mistyped_open_is_refused_before_out_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 2);
    let cases = [
        ("page_extract", json!({ "doc": id, "pages": [1], "out": "x.pdf", "open": "yes" })),
        ("doc_combine", json!({ "paths": ["in.pdf", "two.pdf"], "out": "x.pdf", "open": "yes" })),
        ("comments_summarize", json!({ "doc": id, "out": "x.pdf", "open": "yes" })),
    ];
    for (tool, args) in cases {
        let error = a.call(tool, &args).expect_err(tool);
        assert_eq!(error.to_string(), "open must be true or false", "{tool}");
        assert!(!dir.path().join("x.pdf").exists(), "{tool}: the file must not be written");
        assert_eq!(documents(&mut a), 1, "{tool}: nothing may be opened");
    }
    assert_eq!(names(dir.path()), ["in.pdf", "two.pdf"]);
    // With a correct `open` the same calls write the file (so the refusal above is the argument's).
    call(&mut a, "page_extract", json!({ "doc": id, "pages": [1], "out": "x.pdf", "open": false }));
    assert!(dir.path().join("x.pdf").exists());
    assert_eq!(documents(&mut a), 1);
}

#[test]
fn a_split_that_cannot_write_a_later_part_removes_the_parts_it_wrote() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 3);
    // A folder where part 2 would go: part 1 is written, then part 2 is refused.
    let out = dir.path().join("out");
    std::fs::create_dir_all(out.join("in-part2.pdf")).unwrap();
    std::fs::write(out.join("keep.txt"), "mine").unwrap();
    std::fs::write(out.join("in-part3.pdf"), "old").unwrap();
    let error = a.call("doc_split", &json!({ "doc": id, "every": 1, "out_dir": "out" })).expect_err("part 2 is a folder");
    let message = error.to_string();
    assert!(message.ends_with("in-part2.pdf: is a folder, not a file"), "{message}");
    assert_eq!(names(&out), ["in-part2.pdf", "in-part3.pdf", "keep.txt"], "part 1 is removed again");
    assert_eq!(std::fs::read_to_string(out.join("keep.txt")).unwrap(), "mine");
    assert_eq!(std::fs::read_to_string(out.join("in-part3.pdf")).unwrap(), "old");

    // A part that was already there is replaced by the write, as before, and is not removed.
    std::fs::write(out.join("in-part1.pdf"), "old").unwrap();
    let error = a.call("doc_split", &json!({ "doc": id, "every": 1, "out_dir": "out" })).expect_err("part 2 is a folder");
    assert!(error.to_string().ends_with("in-part2.pdf: is a folder, not a file"));
    assert_eq!(names(&out), ["in-part1.pdf", "in-part2.pdf", "in-part3.pdf", "keep.txt"]);
    let replaced = std::fs::read(out.join("in-part1.pdf")).unwrap();
    assert!(replaced.starts_with(b"%PDF-"), "the replaced part holds the new PDF, not the old text");
}

/// A dangling link where a part goes counts as a file that was there: the write replaces it and a
/// later failure does not remove what replaced it.
#[cfg(unix)]
#[test]
fn a_dangling_link_in_a_part_name_is_not_removed_after_a_failed_split() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 3);
    let out = dir.path().join("out");
    std::fs::create_dir_all(out.join("in-part2.pdf")).unwrap();
    std::os::unix::fs::symlink("missing-target", out.join("in-part1.pdf")).unwrap();
    let error = a.call("doc_split", &json!({ "doc": id, "every": 1, "out_dir": "out" })).expect_err("part 2 is a folder");
    assert!(error.to_string().ends_with("in-part2.pdf: is a folder, not a file"), "{error}");
    assert_eq!(names(&out), ["in-part1.pdf", "in-part2.pdf"]);
    assert!(out.join("in-part1.pdf").symlink_metadata().unwrap().is_file(), "the link was replaced by the part");
}

/// The same for `page_extract` with `separate`.
#[cfg(unix)]
#[test]
fn a_dangling_link_in_a_page_name_is_not_removed_after_a_failed_extract() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 3);
    let out = dir.path().join("out");
    std::fs::create_dir_all(out.join("in-page2.pdf")).unwrap();
    std::os::unix::fs::symlink("missing-target", out.join("in-page1.pdf")).unwrap();
    let error = a.call("page_extract", &json!({ "doc": id, "pages": [1, 2], "separate": true, "out_dir": "out" })).expect_err("page 2 is a folder");
    assert!(error.to_string().ends_with("in-page2.pdf: is a folder, not a file"), "{error}");
    assert_eq!(names(&out), ["in-page1.pdf", "in-page2.pdf"]);
    assert!(out.join("in-page1.pdf").symlink_metadata().unwrap().is_file());
}

#[test]
fn a_split_that_works_still_lists_what_it_wrote() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 3);
    let r = call(&mut a, "doc_split", json!({ "doc": id, "every": 2, "out_dir": "parts" }));
    let files = r["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!((files[0]["first_page"].clone(), files[0]["last_page"].clone()), (json!(1), json!(2)));
    assert_eq!((files[1]["first_page"].clone(), files[1]["last_page"].clone()), (json!(3), json!(3)));
    assert_eq!(names(&dir.path().join("parts")), ["in-part1.pdf", "in-part2.pdf"]);
}
