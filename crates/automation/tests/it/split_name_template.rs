//! `doc_split`'s `name_template`: the placeholders, the file-name sanitizer reached through the tool,
//! equal names getting -2/-3, nothing written outside `out_dir`, and a file already there being replaced.
use pdfcraft_automation::{Automation, Content};
use serde_json::{Value, json};
use std::path::Path;

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

/// `root/in.pdf` with `pages` pages, open, and an `Automation` confined to `root`.
fn open(root: &Path, pages: usize) -> (Automation, u64) {
    std::fs::write(root.join("in.pdf"), pdf(pages)).unwrap();
    let mut a = Automation::new().with_root(root).unwrap();
    let id = call(&mut a, "doc_open", json!({ "path": "in.pdf" }))["doc"].as_u64().unwrap();
    (a, id)
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

fn written(result: &Value) -> Vec<String> {
    result["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| Path::new(f["path"].as_str().unwrap()).file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn a_template_names_every_part_from_its_placeholders() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 5);
    let r = call(&mut a, "doc_split", json!({ "doc": id, "every": 2, "out_dir": "parts", "name_template": "{name}_{n}_pages {first}-{last}" }));
    assert_eq!(written(&r), ["in_1_pages 1-2.pdf", "in_2_pages 3-4.pdf", "in_3_pages 5-5.pdf"]);
    assert_eq!(names(&dir.path().join("parts")), ["in_1_pages 1-2.pdf", "in_2_pages 3-4.pdf", "in_3_pages 5-5.pdf"]);
}

#[test]
fn a_template_can_use_the_bookmark_a_part_starts_at() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 3);
    call(&mut a, "bookmark_add", json!({ "doc": id, "title": "Intro", "page": 1 }));
    call(&mut a, "bookmark_add", json!({ "doc": id, "title": "Body/Text", "page": 2 }));
    let r = call(&mut a, "doc_split", json!({ "doc": id, "bookmarks": true, "out_dir": "parts", "name_template": "{n} - {bookmark}" }));
    assert_eq!(written(&r), ["1 - Intro.pdf", "2 - Body_Text.pdf"]);
}

#[test]
fn without_a_template_the_default_names_are_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 2);
    let r = call(&mut a, "doc_split", json!({ "doc": id, "every": 1, "out_dir": "parts" }));
    assert_eq!(written(&r), ["in-part1.pdf", "in-part2.pdf"]);
}

#[test]
fn a_template_with_nothing_usable_falls_back_to_the_default_name() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 2);
    // No bookmark starts a part of an "every" split, so {bookmark} is empty; ".." leaves nothing either.
    let r = call(&mut a, "doc_split", json!({ "doc": id, "every": 1, "out_dir": "parts", "name_template": "{bookmark}" }));
    assert_eq!(written(&r), ["in-part1.pdf", "in-part2.pdf"]);
    let r = call(&mut a, "doc_split", json!({ "doc": id, "every": 1, "out_dir": "more", "name_template": " .. " }));
    assert_eq!(written(&r), ["in-part1.pdf", "in-part2.pdf"]);
}

#[test]
fn parts_that_get_the_same_name_stay_distinct_files() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 3);
    let r = call(&mut a, "doc_split", json!({ "doc": id, "every": 1, "out_dir": "parts", "name_template": "Same" }));
    assert_eq!(written(&r), ["Same.pdf", "Same-2.pdf", "Same-3.pdf"]);
    // Names that differ only in case, or become equal once the sanitizer has run, collide too.
    let r = call(&mut a, "doc_split", json!({ "doc": id, "every": 1, "out_dir": "folded", "name_template": "A:b" }));
    assert_eq!(written(&r), ["A_b.pdf", "A_b-2.pdf", "A_b-3.pdf"]);
    let r = call(&mut a, "doc_split", json!({ "doc": id, "every": 1, "out_dir": "cased", "name_template": "{bookmark}Ab{first}" }));
    assert_eq!(written(&r), ["Ab1.pdf", "Ab2.pdf", "Ab3.pdf"]);
}

#[test]
fn a_template_cannot_put_a_part_outside_out_dir() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 2);
    std::fs::create_dir(dir.path().join("work")).unwrap();
    let before = names(dir.path());
    for template in ["../../escape", "../{n}", "/abs/{n}", r"..\..\x{n}", "a/../../b", "..", "{name}/{n}", "C:\\evil", "x\u{0}y{n}"] {
        let r = call(&mut a, "doc_split", json!({ "doc": id, "every": 1, "out_dir": "work/parts", "name_template": template }));
        for f in r["files"].as_array().unwrap() {
            let p = Path::new(f["path"].as_str().unwrap());
            assert_eq!(p.parent().unwrap().canonicalize().unwrap(), dir.path().join("work/parts").canonicalize().unwrap(), "{template}: {p:?}");
        }
    }
    assert_eq!(names(dir.path()), before);
    assert_eq!(names(&dir.path().join("work")), ["parts"]);
}

#[test]
fn windows_device_names_and_trailing_dots_are_made_safe() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 1);
    for (template, want) in [
        ("CON", "_CON.pdf"),
        ("nul.txt", "_nul.txt.pdf"),
        ("lpt1", "_lpt1.pdf"),
        ("part. . ", "part.pdf"),
        ("COM10", "COM10.pdf"),
        ("com0", "_com0.pdf"),
        ("LPT\u{b2}", "_LPT\u{b2}.pdf"),
        ("x.pdf .", "x.pdf"),
    ] {
        let r = call(&mut a, "doc_split", json!({ "doc": id, "every": 1, "out_dir": "parts", "name_template": template }));
        assert_eq!(written(&r), [want], "{template}");
    }
}

#[test]
fn a_long_template_is_cut_to_a_name_the_system_accepts() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 2);
    let long = "\u{e9}".repeat(400);
    let r = call(&mut a, "doc_split", json!({ "doc": id, "every": 1, "out_dir": "parts", "name_template": long }));
    let files = written(&r);
    assert_eq!(files.len(), 2);
    assert!(files.iter().all(|f| f.len() <= 255), "{files:?}");
    assert_ne!(files[0], files[1]);
}

#[test]
fn a_bad_template_is_refused_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 2);
    let before = names(dir.path());
    let many = "{".repeat(10_001);
    for template in ["{oops}", "{n", "}", "{", many.as_str()] {
        let e = a.call("doc_split", &json!({ "doc": id, "every": 1, "out_dir": "parts", "name_template": template })).expect_err(template);
        assert!(e.to_string().contains("name_template"), "{e}");
    }
    assert_eq!(names(dir.path()), before);
    // Wrong types come back as a result or an error, never a panic (the sweep in hostile_args.rs sends them too).
    for hostile in [json!(1), json!(null), json!([]), json!({}), json!(true)] {
        let _ = a.call("doc_split", &json!({ "doc": id, "every": 1, "out_dir": "parts", "name_template": hostile }));
    }
}

#[test]
fn a_file_with_a_parts_name_is_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 2);
    std::fs::create_dir(dir.path().join("parts")).unwrap();
    std::fs::write(dir.path().join("parts/keep.pdf"), b"old").unwrap();
    std::fs::write(dir.path().join("parts/other.pdf"), b"untouched").unwrap();
    call(&mut a, "doc_split", json!({ "doc": id, "every": 1, "out_dir": "parts", "name_template": "keep" }));
    // The first part took the name of the file that was there; the second got -2 beside it.
    assert!(std::fs::read(dir.path().join("parts/keep.pdf")).unwrap().starts_with(b"%PDF"));
    assert!(dir.path().join("parts/keep-2.pdf").exists());
    assert_eq!(std::fs::read(dir.path().join("parts/other.pdf")).unwrap(), b"untouched");
}

#[test]
fn bookmark_placeholder_works_with_before_and_without_bookmarks_true() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 4);
    call(&mut a, "bookmark_add", json!({ "doc": id, "title": "Intro", "page": 1 }));
    call(&mut a, "bookmark_add", json!({ "doc": id, "title": "Body", "page": 3 }));
    // `before` splits at pages 2 and 3: the part that starts at page 2 has no bookmark, the one at 3 has.
    let r = call(&mut a, "doc_split", json!({ "doc": id, "before": [2, 3], "out_dir": "parts", "name_template": "{n}-{first}-{bookmark}" }));
    assert_eq!(written(&r), ["1-1-Intro.pdf", "2-2-.pdf", "3-3-Body.pdf"]);
    // Without a template the default names stay what they were.
    let r = call(&mut a, "doc_split", json!({ "doc": id, "before": [3], "out_dir": "plain" }));
    assert_eq!(written(&r), ["in-part1.pdf", "in-part2.pdf"]);
}

#[test]
fn bookmark_titles_that_differ_only_in_case_get_distinct_files() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 2);
    call(&mut a, "bookmark_add", json!({ "doc": id, "title": "Intro", "page": 1 }));
    call(&mut a, "bookmark_add", json!({ "doc": id, "title": "INTRO", "page": 2 }));
    let r = call(&mut a, "doc_split", json!({ "doc": id, "bookmarks": true, "out_dir": "parts", "name_template": "{bookmark}" }));
    assert_eq!(written(&r), ["Intro.pdf", "INTRO-2.pdf"]);
}

#[test]
fn a_device_name_stem_is_guarded_in_the_default_names_too() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("nul.x.pdf"), pdf(2)).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    let id = call(&mut a, "doc_open", json!({ "path": "nul.x.pdf" }))["doc"].as_u64().unwrap();
    let r = call(&mut a, "doc_split", json!({ "doc": id, "every": 1, "out_dir": "parts" }));
    assert_eq!(written(&r), ["_nul.x-part1.pdf", "_nul.x-part2.pdf"]);
    // The template's fallback is the same guarded name, and {name} is the file name without its extension.
    let r = call(&mut a, "doc_split", json!({ "doc": id, "every": 1, "out_dir": "more", "name_template": "{bookmark}" }));
    assert_eq!(written(&r), ["_nul.x-part1.pdf", "_nul.x-part2.pdf"]);
    let r = call(&mut a, "doc_split", json!({ "doc": id, "every": 1, "out_dir": "named", "name_template": "{name}_{n}" }));
    assert_eq!(written(&r), ["_nul.x_1.pdf", "_nul.x_2.pdf"]);
}

#[test]
fn a_template_over_the_length_cap_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), 1);
    let e = a.call("doc_split", &json!({ "doc": id, "every": 1, "out_dir": "parts", "name_template": "a".repeat(1001) })).expect_err("too long");
    assert!(e.to_string().contains("name_template"), "{e}");
    assert!(!dir.path().join("parts").exists());
}
