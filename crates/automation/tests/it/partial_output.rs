//! A tool that writes a series of files into a folder leaves nothing behind when it fails:
//! arguments are checked before the folder is made, and a file that cannot be written takes back
//! the files this call wrote to a free name, and the folder if the call made it.
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

/// An automation rooted at `dir`, with `<name>.pdf` of `pages` pages written there and opened.
fn open(dir: &std::path::Path, name: &str, pages: usize) -> (Automation, u64) {
    std::fs::write(dir.join(format!("{name}.pdf")), pdf(pages)).unwrap();
    let mut a = Automation::new().with_root(dir).unwrap();
    let id = call(&mut a, "doc_open", json!({ "path": format!("{name}.pdf") }))["doc"].as_u64().unwrap();
    (a, id)
}

fn names(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

fn refused(a: &mut Automation, tool: &str, args: Value) -> String {
    a.call(tool, &args).expect_err("the call is refused").to_string()
}

#[test]
fn an_image_export_with_a_bad_argument_makes_no_folder() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), "in", 2);
    let message = refused(&mut a, "doc_export_images", json!({ "doc": id, "folder": "out", "format": "webp" }));
    assert_eq!(message, "unknown format \"webp\" (png, jpeg, tiff)");
    assert!(!dir.path().join("out").exists(), "the folder was made before format was checked");
    let message = refused(&mut a, "doc_export_images", json!({ "doc": id, "folder": "out", "dpi": "fast" }));
    assert_eq!(message, "dpi must be a number");
    assert!(!dir.path().join("out").exists(), "the folder was made before dpi was checked");
    let message = refused(&mut a, "doc_export_all_images", json!({ "doc": id, "folder": "out", "min_size": "big" }));
    assert_eq!(message, "min_size must be an integer");
    assert!(!dir.path().join("out").exists(), "the folder was made before min_size was checked");
}

/// A folder where the file for page 2 goes: page 1 is written, then page 2 is refused.
#[test]
fn an_image_export_that_cannot_write_a_later_page_removes_the_pages_it_wrote() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, id) = open(dir.path(), "in", 3);
    let out = dir.path().join("out");
    std::fs::create_dir_all(out.join("in_page_2.png")).unwrap();
    std::fs::write(out.join("keep.txt"), "mine").unwrap();
    std::fs::write(out.join("in_page_3.png"), "old").unwrap();
    let message = refused(&mut a, "doc_export_images", json!({ "doc": id, "folder": "out", "dpi": 18 }));
    assert!(message.ends_with("in_page_2.png: is a folder, not a file"), "{message}");
    assert_eq!(names(&out), ["in_page_2.png", "in_page_3.png", "keep.txt"], "page 1 is removed again");
    assert_eq!(std::fs::read_to_string(out.join("keep.txt")).unwrap(), "mine");
    assert_eq!(std::fs::read_to_string(out.join("in_page_3.png")).unwrap(), "old");

    // A page file that was already there is replaced, as before, and is not removed.
    std::fs::write(out.join("in_page_1.png"), "old").unwrap();
    let message = refused(&mut a, "doc_export_images", json!({ "doc": id, "folder": "out", "dpi": 18 }));
    assert!(message.ends_with("in_page_2.png: is a folder, not a file"), "{message}");
    assert_eq!(names(&out), ["in_page_1.png", "in_page_2.png", "in_page_3.png", "keep.txt"]);
    assert!(std::fs::read(out.join("in_page_1.png")).unwrap().starts_with(b"\x89PNG"), "the replaced page holds the new image");
}

/// Names of 255 bytes are the most a file may have: page 10's is one byte over, so pages 1 to 9
/// are written and page 10 is refused (Unix: the limit's wording is the system's).
#[cfg(unix)]
#[test]
fn an_image_export_removes_the_folder_it_made() {
    let dir = tempfile::tempdir().unwrap();
    let stem = "x".repeat(244);
    let (mut a, id) = open(dir.path(), &stem, 10);
    let message = refused(&mut a, "doc_export_images", json!({ "doc": id, "folder": "out", "dpi": 18 }));
    assert!(message.ends_with("_page_10.png: File name too long (os error 36)"), "{message}");
    assert!(!dir.path().join("out").exists(), "pages 1 to 9 and the folder are removed again");
}

fn image_document(dir: &std::path::Path) -> (Automation, u64) {
    let (mut a, id) = open(dir, "a", 2);
    call(&mut a, "doc_export_images", json!({ "doc": id, "folder": "src", "dpi": 36, "pages": [1] }));
    call(&mut a, "doc_export_images", json!({ "doc": id, "folder": "src", "dpi": 36, "pages": [2], "format": "jpeg" }));
    let made = call(&mut a, "doc_create", json!({ "from": "images", "paths": ["src/a_page_1.png", "src/a_page_2.jpg"] }))["doc"].as_u64().unwrap();
    call(&mut a, "doc_save", json!({ "doc": made, "path": "pics.pdf" }));
    (a, made)
}

#[test]
fn an_all_images_export_that_cannot_write_a_later_image_removes_the_images_it_wrote() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, made) = image_document(dir.path());
    let ok = call(&mut a, "doc_export_all_images", json!({ "doc": made, "folder": "probe" }));
    let second = std::path::Path::new(ok["files"][1]["path"].as_str().unwrap()).file_name().unwrap().to_string_lossy().into_owned();
    let out = dir.path().join("out");
    std::fs::create_dir_all(out.join(&second)).unwrap();
    std::fs::write(out.join("keep.txt"), "mine").unwrap();
    let message = refused(&mut a, "doc_export_all_images", json!({ "doc": made, "folder": "out" }));
    assert!(message.ends_with(&format!("{second}: is a folder, not a file")), "{message}");
    let mut left = vec![second, "keep.txt".to_string()];
    left.sort();
    assert_eq!(names(&out), left, "image 1 is removed again");
    assert_eq!(std::fs::read_to_string(out.join("keep.txt")).unwrap(), "mine");
}

/// The first image's name is too long, so the folder this call made is taken back.
#[cfg(unix)]
#[test]
fn an_all_images_export_removes_the_folder_it_made() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, made) = image_document(dir.path());
    let long = "y".repeat(240);
    call(&mut a, "doc_save", json!({ "doc": made, "path": format!("{long}.pdf") }));
    let message = refused(&mut a, "doc_export_all_images", json!({ "doc": made, "folder": "out" }));
    assert!(message.ends_with("_Page_1_Image_0001.png: File name too long (os error 36)"), "{message}");
    assert!(!dir.path().join("out").exists());
}

/// Three inputs: the third's output would be `<long> (2).pdf`, over the 255-byte limit, after the
/// first two are written.
#[cfg(unix)]
#[test]
fn a_separate_create_that_cannot_write_a_later_file_removes_the_files_it_wrote() {
    let dir = tempfile::tempdir().unwrap();
    let long = "z".repeat(251);
    for sub in ["one", "two"] {
        std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        std::fs::write(dir.path().join(sub).join(format!("{long}.txt")), "text").unwrap();
    }
    std::fs::write(dir.path().join("short.txt"), "text").unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    let paths = ["short.txt".to_string(), format!("one/{long}.txt"), format!("two/{long}.txt")];
    let args = json!({ "paths": paths, "mode": "separate", "out_dir": "out" });
    let message = refused(&mut a, "doc_create_multiple", args.clone());
    assert!(message.ends_with(&format!("{long} (2).pdf: File name too long (os error 36)")), "{message}");
    assert!(!dir.path().join("out").exists(), "the files and the folder this call made are removed again");

    // In a folder that was there, only what this call wrote goes.
    std::fs::create_dir_all(dir.path().join("out")).unwrap();
    std::fs::write(dir.path().join("out/keep.txt"), "mine").unwrap();
    let message = refused(&mut a, "doc_create_multiple", args);
    assert!(message.ends_with(&format!("{long} (2).pdf: File name too long (os error 36)")), "{message}");
    assert_eq!(names(&dir.path().join("out")), ["keep.txt"]);
}

#[test]
fn a_separate_create_that_works_still_lists_what_it_wrote() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "one").unwrap();
    std::fs::write(dir.path().join("b.txt"), "two").unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    let r = call(&mut a, "doc_create_multiple", json!({ "paths": ["a.txt", "b.txt"], "mode": "separate", "out_dir": "out" }));
    assert_eq!(r["files"].as_array().unwrap().len(), 2);
    assert_eq!(names(&dir.path().join("out")), ["a.pdf", "b.pdf"]);
}

/// The third file's output would be `<long> (2).pdf`, over the 255-byte limit, after the first
/// two are published.
#[cfg(unix)]
#[test]
fn an_action_run_that_cannot_publish_a_later_file_removes_the_files_it_wrote() {
    let dir = tempfile::tempdir().unwrap();
    let long = "z".repeat(251);
    for sub in ["one", "two"] {
        std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        std::fs::write(dir.path().join(sub).join(format!("{long}.pdf")), pdf(1)).unwrap();
    }
    std::fs::write(dir.path().join("short.pdf"), pdf(1)).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    let paths = ["short.pdf".to_string(), format!("one/{long}.pdf"), format!("two/{long}.pdf")];
    let args = json!({ "paths": paths, "folder": "out", "steps": [{ "step": "flatten_comments" }] });
    let message = refused(&mut a, "action_run", args.clone());
    let expected = "File name too long (os error 36)";
    assert_eq!(message, expected);
    assert!(!dir.path().join("out").exists(), "the files and the folder this call made are removed again");

    std::fs::create_dir_all(dir.path().join("out")).unwrap();
    std::fs::write(dir.path().join("out/keep.txt"), "mine").unwrap();
    let message = refused(&mut a, "action_run", args);
    assert_eq!(message, expected);
    assert_eq!(names(&dir.path().join("out")), ["keep.txt"]);
}

#[test]
fn an_action_run_that_works_still_lists_what_it_wrote() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.pdf"), pdf(1)).unwrap();
    std::fs::write(dir.path().join("b.pdf"), pdf(1)).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    let r = call(&mut a, "action_run", json!({ "paths": ["a.pdf", "b.pdf"], "folder": "out", "steps": [{ "step": "flatten_comments" }] }));
    assert_eq!(r["files"].as_array().unwrap().len(), 2);
    assert_eq!(names(&dir.path().join("out")), ["a.pdf", "b.pdf"]);
}

/// The engines are built before the folder is made. Without the OCR models the build fails, so
/// this runs only there; with them it cannot reach that error.
#[test]
fn an_ocr_batch_whose_engines_cannot_be_built_makes_no_folder() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.pdf"), pdf(1)).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    if call(&mut a, "ocr_status", json!({}))["available"] == true {
        eprintln!("skipped: OCR models installed");
        return;
    }
    let message = refused(&mut a, "ocr_recognize_files", json!({ "paths": ["a.pdf"], "folder": "out" }));
    assert_eq!(message, "the text recognition models are not installed (reinstall PdfCraft, or set PDFCRAFT_MODELS to the folder that holds them)");
    assert!(!dir.path().join("out").exists(), "the folder was made before the engines were built");
}

/// An output that cannot be written, after an earlier one was. The output has its input's file
/// name, and its staging file is longer, so a 254-byte name fails there. Runs only with the OCR
/// models installed, which the batch needs before it writes anything.
#[cfg(unix)]
#[test]
fn an_ocr_batch_that_cannot_write_a_later_output_removes_the_outputs_it_wrote() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    if call(&mut a, "ocr_status", json!({}))["available"] != true {
        eprintln!("skipped: OCR models not installed");
        return;
    }
    let long = format!("{}.pdf", "z".repeat(250));
    std::fs::write(dir.path().join("a.pdf"), pdf(1)).unwrap();
    std::fs::write(dir.path().join(&long), pdf(1)).unwrap();
    let args = json!({ "paths": ["a.pdf", long], "folder": "out" });
    let message = refused(&mut a, "ocr_recognize_files", args.clone());
    assert!(message.contains("File name too long"), "{message}");
    assert!(!dir.path().join("out").exists(), "the outputs and the folder this call made are removed again");

    std::fs::create_dir_all(dir.path().join("out")).unwrap();
    std::fs::write(dir.path().join("out/keep.txt"), "mine").unwrap();
    refused(&mut a, "ocr_recognize_files", args);
    assert_eq!(names(&dir.path().join("out")), ["keep.txt"]);
}

#[test]
fn an_action_run_whose_folder_cannot_be_made_names_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.pdf"), pdf(1)).unwrap();
    std::fs::write(dir.path().join("out"), "a file").unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    let message = refused(&mut a, "action_run", json!({ "paths": ["a.pdf"], "folder": "out", "steps": [{ "step": "flatten_comments" }] }));
    assert!(message.starts_with(&dir.path().join("out").display().to_string()), "{message}");
}
