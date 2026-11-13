//! Office documents in Insert pages, Combine files and Create ▸ Multiple files, converted by a
//! stand-in for LibreOffice (a shell script found through the session's `OfficeLocator`), so
//! they run without it. The stand-in converts by the last byte of the document: `1`, `2` or `3`
//! give a PDF of that many pages; `X` fails.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pdfcraft_engine::{Edit, EditError, OfficeLocator, Session, SourceKind, source_kind};

struct Fake {
    dir: PathBuf,
    program: PathBuf,
}

impl Fake {
    fn new(tag: &str) -> Self {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("pdfcraft-office-assemble-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let maker = Session::new();
        for n in 1..=3 {
            std::fs::write(dir.join(format!("p{n}.pdf")), maker.create_blank(200.0, 200.0, n).unwrap().as_slice()).unwrap();
        }
        let script = format!(
            "#!/bin/sh\n[ $# -eq 0 ] && exit 0\nfor input; do :; done\nwhile [ $# -gt 0 ]; do case \"$1\" in --outdir) out=\"$2\";; esac; shift; done\n\
             n=$(tail -c 1 \"$input\")\necho \"$n $input\" >> '{dir}/log'\n[ \"$n\" = X ] && exit 3\ncp '{dir}/p'\"$n\".pdf \"$out/document.pdf\"\n",
            dir = dir.display()
        );
        let program = dir.join("soffice");
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
        Self { dir, program }
    }

    fn session(&self) -> Session {
        let program = self.program.clone();
        let locator: OfficeLocator = Arc::new(move || Some(program.clone()));
        Session::new().with_office_locator(locator)
    }

    /// The markers of the documents converted so far, in order.
    fn converted(&self) -> String {
        std::fs::read_to_string(self.dir.join("log")).unwrap_or_default().lines().filter_map(|l| l.split(' ').next()).collect()
    }

    /// The input folders LibreOffice was given, which must be gone.
    fn work_dirs(&self) -> Vec<PathBuf> {
        let log = std::fs::read_to_string(self.dir.join("log")).unwrap_or_default();
        log.lines().filter_map(|l| l.split_once(' ')).filter_map(|(_, input)| Path::new(input).parent().map(Path::to_path_buf)).collect()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A document in the given format with `marker` as its last byte.
fn office(marker: char) -> Arc<Vec<u8>> {
    let mut bytes = b"PK\x03\x04\x14\x00".to_vec();
    bytes.push(marker as u8);
    Arc::new(bytes)
}

fn no_locator() -> Session {
    let locator: OfficeLocator = Arc::new(|| None);
    Session::new().with_office_locator(locator)
}

fn open_target(s: &mut Session) -> pdfcraft_engine::DocId {
    let bytes = s.create_blank(300.0, 300.0, 2).unwrap();
    s.open_new("target.pdf", bytes).unwrap()
}

fn inserted_names(edit: &Edit) -> Vec<String> {
    let Edit::Batch { edits, .. } = edit else { panic!("a batch") };
    edits
        .iter()
        .map(|e| match e {
            Edit::InsertPagesFrom { name, .. } => name.clone(),
            other => panic!("{other:?}"),
        })
        .collect()
}

#[test]
fn the_extension_is_matched_in_any_case_and_the_bytes_decide_the_rest() {
    let head = office('1');
    for name in ["a.docx", "A.DOCX", "Sheet.XlSx", "deck.PPTX", "old.Doc", "x.odt", "x.ods", "x.odp"] {
        assert_eq!(source_kind(name, &head), Some(SourceKind::Office), "{name}");
    }
    // The name alone is not enough, and the right bytes under another name are not Office.
    assert_eq!(source_kind("a.docx", b"plain text"), None);
    assert_eq!(source_kind("a.zip", &head), None);
    // A PDF stays a PDF whatever it is called.
    assert_eq!(source_kind("report.docx", b"%PDF-1.7\n"), Some(SourceKind::Pdf));
}

#[test]
fn insert_converts_office_documents_in_order_between_other_files() {
    let fake = Fake::new("order");
    let mut s = fake.session();
    let id = open_target(&mut s);
    let pdf = s.create_blank(100.0, 100.0, 2).unwrap();
    let sources = vec![
        ("one.PDF".to_string(), pdf.clone()),
        ("two.DOCX".to_string(), office('3')),
        ("three.txt".to_string(), Arc::new(b"hello".to_vec())),
        ("four.xlsx".to_string(), office('1')),
    ];
    let plan = s.plan_insert_converting(id, 1, sources);
    assert!(plan.skipped.is_empty(), "{:?}", plan.skipped);
    assert_eq!((plan.start, plan.pages), (1, 2 + 3 + 1 + 1));
    let edit = plan.edit.unwrap();
    assert_eq!(inserted_names(&edit), ["one.PDF", "two.DOCX", "three.txt", "four.xlsx"]);
    assert_eq!(fake.converted(), "31", "converted in the order given, once each");
    s.apply(id, edit).unwrap();
    assert_eq!(s.get(id).unwrap().info.pages.len(), 2 + 7);
    // Every temporary folder LibreOffice was given is gone.
    assert!(fake.work_dirs().iter().all(|d| !d.exists()), "{:?}", fake.work_dirs());
}

#[test]
fn insert_without_libreoffice_converts_nothing_and_says_what_to_install() {
    let mut s = no_locator();
    let id = open_target(&mut s);
    let pdf = s.create_blank(100.0, 100.0, 1).unwrap();
    let plan = s.plan_insert_converting(id, 0, vec![("a.pdf".to_string(), pdf), ("b.docx".to_string(), office('1'))]);
    assert!(plan.edit.is_none(), "no partial result");
    let [(name, why)] = plan.skipped.as_slice() else { panic!("{:?}", plan.skipped) };
    assert_eq!(name, "b.docx");
    assert!(why.contains("LibreOffice, which isn't installed") && why.contains("libreoffice.org"), "{why}");
    // Files that need no LibreOffice are unaffected by its absence.
    let plan = s.plan_insert_converting(id, 0, vec![("a.txt".to_string(), Arc::new(b"hi".to_vec()))]);
    assert!(plan.edit.is_some() && plan.skipped.is_empty());
}

#[test]
fn a_failing_document_ends_the_plan_and_later_documents_are_not_converted() {
    let fake = Fake::new("failing");
    let mut s = fake.session();
    let id = open_target(&mut s);
    let plan = s.plan_insert_converting(id, 0, vec![("a.docx".into(), office('1')), ("b.docx".into(), office('X')), ("c.docx".into(), office('3'))]);
    let [(name, why)] = plan.skipped.as_slice() else { panic!("{:?}", plan.skipped) };
    assert_eq!(name, "b.docx");
    assert!(why.contains("LibreOffice couldn't convert it"), "{why}");
    assert_eq!(fake.converted(), "1X", "the third was never converted");
    assert!(fake.work_dirs().iter().all(|d| !d.exists()), "temporary folders are removed after a failure too");
}

#[test]
fn a_pdf_with_an_office_name_is_inserted_as_a_pdf_without_running_libreoffice() {
    let fake = Fake::new("pdfnamed");
    let mut s = fake.session();
    let id = open_target(&mut s);
    let pdf = s.create_blank(100.0, 100.0, 2).unwrap();
    let plan = s.plan_insert_converting(id, 0, vec![("Report.docx".to_string(), pdf.clone())]);
    assert!(plan.skipped.is_empty(), "{:?}", plan.skipped);
    assert_eq!(plan.pages, 2);
    assert_eq!(fake.converted(), "", "LibreOffice was not run");
    // Combine's and Create's conversions agree.
    assert_eq!(s.as_pdf("Report.docx", &pdf).unwrap(), pdf);
    assert_eq!(s.convert_to_pdf("Report.docx", &pdf).unwrap().0, SourceKind::Pdf);
    assert_eq!(fake.converted(), "");
}

#[test]
fn the_frame_thread_calls_still_refuse_office_documents() {
    let fake = Fake::new("inline");
    let mut s = fake.session();
    let id = open_target(&mut s);
    let plan = s.plan_insert(id, 0, vec![("a.docx".to_string(), office('1'))]);
    assert!(plan.edit.is_none() && plan.skipped.len() == 1);
    assert_eq!(fake.converted(), "", "LibreOffice was not run on the frame thread");
}

#[test]
fn combine_and_create_convert_through_the_same_locator() {
    let fake = Fake::new("combine");
    let s = fake.session();
    let docx = office('2');
    let pdf = s.as_pdf("Quarter.docx", &docx).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    assert_eq!(s.page_count_of("Quarter.pdf", &pdf).unwrap(), 2);
    let (kind, pdf) = s.convert_to_pdf("Deck.PPTX", &office('3')).unwrap();
    assert_eq!(kind, SourceKind::Office);
    assert_eq!(s.page_count_of("Deck.pdf", &pdf).unwrap(), 3);
    assert_eq!(fake.converted(), "23");
    // Without LibreOffice both say so, naming the file.
    for err in [
        no_locator().as_pdf("Quarter.docx", &docx).map(|_| ()).unwrap_err(),
        no_locator().convert_to_pdf("Quarter.docx", &docx).map(|_| ()).unwrap_err(),
    ] {
        let EditError::Create(e) = err else { panic!("{err:?}") };
        let message = e.to_string();
        assert!(message.starts_with("Quarter.docx: ") && message.contains("LibreOffice, which isn't installed"), "{message}");
    }
}

#[test]
fn a_batch_without_libreoffice_is_one_error_and_converts_nothing() {
    let fake = Fake::new("batch");
    let files = vec![("a.docx".to_string(), office('1')), ("b.docx".to_string(), office('2'))];
    let runner = no_locator().office_runner();
    assert!(!runner.is_available());
    let error = runner.convert_all(&files).map(|_| ()).unwrap_err().to_string();
    assert!(error.starts_with("a.docx: ") && error.contains("libreoffice.org"), "{error}");
    let results = fake.session().office_runner().convert_all(&files).unwrap();
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(Result::is_ok));
    assert_eq!(fake.converted(), "12");
    assert!(fake.session().office_runner().convert_all(&[]).unwrap().is_empty());
}
