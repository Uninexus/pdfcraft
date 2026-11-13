//! Office documents through `page_insert_image`, `doc_combine` and `doc_create_multiple` with a
//! stand-in for LibreOffice (a shell script, handed to the tools as their `OfficeLocator`), so
//! the whole path runs without LibreOffice: ordering, all-or-nothing results, the missing
//! LibreOffice message, `--root`, and the temporary folders going away. The stand-in converts by
//! the last byte of the document: `1`, `2` or `3` give a one-page PDF saying "Converted one",
//! "Converted two" or "Converted three"; `X` fails.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pdfcraft_automation::{Automation, Content, ToolError};
use pdfcraft_engine::{OfficeLocator, Session};
use serde_json::{Value, json};

struct Fixture {
    dir: tempfile::TempDir,
    program: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let tools = dir.path().join("tools");
        std::fs::create_dir(&tools).unwrap();
        let maker = Session::new();
        for (n, word) in [(1, "one"), (2, "two"), (3, "three")] {
            std::fs::write(tools.join(format!("p{n}.pdf")), maker.create_from_text("t", &format!("Converted {word}")).unwrap().as_slice()).unwrap();
        }
        let script = format!(
            "#!/bin/sh\n[ $# -eq 0 ] && exit 0\nfor input; do :; done\nwhile [ $# -gt 0 ]; do case \"$1\" in --outdir) out=\"$2\";; esac; shift; done\n\
             n=$(tail -c 1 \"$input\")\necho \"$n $input\" >> '{t}/log'\n[ \"$n\" = X ] && exit 3\ncp '{t}/p'\"$n\".pdf \"$out/document.pdf\"\n",
            t = tools.display()
        );
        let program = tools.join("soffice");
        let mut file = std::fs::File::create(&program).unwrap();
        file.write_all(script.as_bytes()).unwrap();
        file.set_permissions(std::fs::Permissions::from_mode(0o755)).unwrap();
        file.sync_all().unwrap();
        drop(file);
        // Linux refuses to run a script another thread's fork may still hold open for writing
        // ("Text file busy"): run it once, with no arguments, until it starts.
        for _ in 0..100 {
            match std::process::Command::new(&program).status() {
                Err(e) if e.to_string().contains("Text file busy") => std::thread::sleep(std::time::Duration::from_millis(50)),
                _ => break,
            }
        }
        // The workspace the tools are rooted in is a folder of its own, apart from the stand-in.
        std::fs::create_dir(dir.path().join("work")).unwrap();
        Self { dir, program }
    }

    fn work(&self) -> PathBuf {
        self.dir.path().join("work")
    }

    fn file(&self, name: &str, bytes: &[u8]) {
        std::fs::write(self.work().join(name), bytes).unwrap();
    }

    /// A document whose last byte is `marker`.
    fn office(&self, name: &str, marker: char) {
        let mut bytes = b"PK\x03\x04\x14\x00".to_vec();
        bytes.push(marker as u8);
        self.file(name, &bytes);
    }

    fn tools(&self) -> Automation {
        let program = self.program.clone();
        let locator: OfficeLocator = Arc::new(move || Some(program.clone()));
        Automation::new().with_root(self.work()).unwrap().with_clock(|| 1_700_000_000).with_office_locator(locator)
    }

    /// The tools with no LibreOffice to be found.
    fn tools_without_libreoffice(&self) -> Automation {
        let locator: OfficeLocator = Arc::new(|| None);
        Automation::new().with_root(self.work()).unwrap().with_clock(|| 1_700_000_000).with_office_locator(locator)
    }

    fn converted(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("tools").join("log")).unwrap_or_default().lines().filter_map(|l| l.split(' ').next()).collect()
    }

    fn all_work_dirs_removed(&self) -> bool {
        let log = std::fs::read_to_string(self.dir.path().join("tools").join("log")).unwrap_or_default();
        log.lines().filter_map(|l| l.split_once(' ')).all(|(_, input)| Path::new(input).parent().is_some_and(|d| !d.exists()))
    }
}

fn ok(a: &mut Automation, tool: &str, args: Value) -> Value {
    match a.call(tool, &args) {
        Ok(mut c) => match c.remove(0) {
            Content::Json(v) => v,
            other => panic!("{tool}: expected JSON, got {other:?}"),
        },
        Err(e) => panic!("{tool} {args}: {e}"),
    }
}

fn err(a: &mut Automation, tool: &str, args: Value) -> String {
    match a.call(tool, &args) {
        Ok(c) => panic!("{tool} {args}: expected an error, got {c:?}"),
        Err(e) => e.to_string(),
    }
}

fn page_text(a: &mut Automation, doc: u64) -> Vec<String> {
    ok(a, "text_extract", json!({ "doc": doc }))["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["text"].as_str().unwrap().split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
}

fn open(f: &Fixture, a: &mut Automation, pages: usize) -> u64 {
    let pdf = Session::new().create_blank(200.0, 200.0, pages).unwrap();
    f.file("target.pdf", &pdf);
    ok(a, "doc_open", json!({ "path": "target.pdf" }))["doc"].as_u64().unwrap()
}

#[test]
fn insert_pages_takes_office_documents_in_order_as_one_undoable_step() {
    let f = Fixture::new();
    f.office("Letter.DOCX", '2');
    f.office("sheet.xlsx", '1');
    f.file("notes.txt", b"Plain notes");
    let mut a = f.tools();
    let doc = open(&f, &mut a, 2);
    let done = ok(&mut a, "page_insert_image", json!({ "doc": doc, "paths": ["Letter.DOCX", "notes.txt", "sheet.xlsx"], "at": 2 }));
    assert_eq!(done["inserted"], json!({ "first_page": 2, "pages": 3 }), "{done}");
    let text = page_text(&mut a, doc);
    assert_eq!(text.len(), 5, "{text:?}");
    assert_eq!((text[1].as_str(), text[2].as_str(), text[3].as_str()), ("Converted two", "Plain notes", "Converted one"));
    assert_eq!(f.converted(), "21", "converted in order");
    assert!(f.all_work_dirs_removed());
    ok(&mut a, "edit_undo", json!({ "doc": doc }));
    assert_eq!(page_text(&mut a, doc).len(), 2, "one undo step");
}

#[test]
fn insert_pages_without_libreoffice_fails_clearly_and_changes_nothing() {
    let f = Fixture::new();
    f.office("a.docx", '1');
    f.file("notes.txt", b"Plain notes");
    let mut a = f.tools_without_libreoffice();
    let doc = open(&f, &mut a, 2);
    let said = err(&mut a, "page_insert_image", json!({ "doc": doc, "paths": ["notes.txt", "a.docx"], "at": 2 }));
    assert!(said.contains("a.docx") && said.contains("LibreOffice, which isn't installed") && said.contains("libreoffice.org"), "{said}");
    assert!(said.contains("the document was not changed"), "{said}");
    assert_eq!(page_text(&mut a, doc).len(), 2);
    assert_eq!(f.converted(), "");
}

#[test]
fn insert_pages_stops_at_a_document_that_fails_and_changes_nothing() {
    let f = Fixture::new();
    f.office("a.docx", '1');
    f.office("b.docx", 'X');
    f.office("c.docx", '3');
    let mut a = f.tools();
    let doc = open(&f, &mut a, 2);
    let said = err(&mut a, "page_insert_image", json!({ "doc": doc, "paths": ["a.docx", "b.docx", "c.docx"], "at": 2 }));
    assert!(said.starts_with("b.docx: ") || said.contains("b.docx"), "{said}");
    assert_eq!(page_text(&mut a, doc).len(), 2, "no partial result");
    assert_eq!(f.converted(), "1X", "the document after the failure was not converted");
    assert!(f.all_work_dirs_removed());
}

#[test]
fn a_pdf_with_an_office_name_and_an_office_name_with_other_bytes_are_handled_by_their_bytes() {
    let f = Fixture::new();
    f.file("really-a-pdf.docx", &Session::new().create_from_text("t", "Genuine PDF").unwrap());
    f.file("not-office.docx", b"plain text, not a package");
    let mut a = f.tools();
    let doc = open(&f, &mut a, 1);
    ok(&mut a, "page_insert_image", json!({ "doc": doc, "paths": ["really-a-pdf.docx"], "at": 2 }));
    assert_eq!(page_text(&mut a, doc)[1], "Genuine PDF");
    assert_eq!(f.converted(), "", "LibreOffice was not run for a PDF");
    let said = err(&mut a, "page_insert_image", json!({ "doc": doc, "paths": ["not-office.docx"], "at": 1 }));
    assert!(said.contains("can't be inserted"), "{said}");
    assert_eq!(f.converted(), "");
}

#[test]
fn files_outside_the_root_are_not_converted() {
    let f = Fixture::new();
    let mut a = f.tools();
    let doc = open(&f, &mut a, 1);
    let outside = f.dir.path().join("outside.docx");
    std::fs::write(&outside, b"PK\x03\x04\x14\x001").unwrap();
    for path in [outside.to_string_lossy().into_owned(), "../outside.docx".to_string()] {
        let said = err(&mut a, "page_insert_image", json!({ "doc": doc, "paths": [path], "at": 1 }));
        assert!(!said.is_empty());
    }
    assert_eq!(f.converted(), "");
    assert_eq!(page_text(&mut a, doc).len(), 1);
}

#[test]
fn combine_files_joins_office_documents_in_order_with_a_bookmark_each() {
    let f = Fixture::new();
    f.office("deck.PPTX", '3');
    f.office("letter.docx", '1');
    f.file("plain.pdf", &Session::new().create_from_text("t", "Made here").unwrap());
    let mut a = f.tools();
    let made = ok(&mut a, "doc_combine", json!({ "paths": ["letter.docx", "plain.pdf", "deck.PPTX"], "out": "all.pdf", "open": true }));
    let doc = made["document"]["doc"].as_u64().unwrap();
    assert_eq!(page_text(&mut a, doc), ["Converted one", "Made here", "Converted three"]);
    let titles: Vec<String> = ok(&mut a, "bookmark_list", json!({ "doc": doc }))["bookmarks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["title"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(titles, ["letter", "plain", "deck"], "{titles:?}");
    assert!(f.work().join("all.pdf").is_file());
    assert_eq!(f.converted(), "13");
    assert!(f.all_work_dirs_removed());
}

#[test]
fn combine_files_without_libreoffice_or_with_a_failing_document_writes_nothing() {
    let f = Fixture::new();
    f.office("a.docx", '1');
    f.office("b.xlsx", 'X');
    f.file("plain.pdf", &Session::new().create_from_text("t", "Made here").unwrap());
    let mut missing = f.tools_without_libreoffice();
    let said = err(&mut missing, "doc_combine", json!({ "paths": ["plain.pdf", "a.docx"], "out": "none.pdf" }));
    assert!(said.contains("a.docx") && said.contains("LibreOffice, which isn't installed") && said.contains("libreoffice.org"), "{said}");
    let mut a = f.tools();
    let said = err(&mut a, "doc_combine", json!({ "paths": ["a.docx", "b.xlsx", "plain.pdf"], "out": "none.pdf" }));
    assert!(said.contains("b.xlsx"), "{said}");
    assert!(!f.work().join("none.pdf").exists(), "no partial result is written");
    assert!(f.all_work_dirs_removed());
}

#[test]
fn create_multiple_files_takes_office_documents_combined_or_separate() {
    let f = Fixture::new();
    f.office("one.docx", '1');
    f.office("two.odt", '2');
    let mut a = f.tools();
    let made = ok(&mut a, "doc_create_multiple", json!({ "paths": ["two.odt", "one.docx"], "out": "both.pdf", "open": true }));
    let doc = made["document"]["doc"].as_u64().unwrap();
    assert_eq!(page_text(&mut a, doc), ["Converted two", "Converted one"]);
    let made = ok(&mut a, "doc_create_multiple", json!({ "paths": ["one.docx", "two.odt"], "mode": "separate", "out_dir": "out" }));
    let files = made["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "{made}");
    assert!(f.work().join("out").join("one.pdf").is_file() && f.work().join("out").join("two.pdf").is_file(), "{made}");
    let mut missing = f.tools_without_libreoffice();
    let said = err(&mut missing, "doc_create_multiple", json!({ "paths": ["one.docx"], "out": "never.pdf" }));
    assert!(said.contains("LibreOffice, which isn't installed"), "{said}");
    assert!(!f.work().join("never.pdf").exists());
    assert!(matches!(a.call("doc_create_multiple", &json!({ "paths": [] })), Err(ToolError::InvalidArgs(_))));
}
