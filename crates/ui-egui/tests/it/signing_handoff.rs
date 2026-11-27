//! Sign > Save the hand-off package… and Open a hand-off package…: the saved PDF and the plan beside it travel as one zip.
//! The pickers are scripted and the files they name are kept in memory; the document's own folder is a real one.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use craft::host::{Picked, ScriptedFiles};
use craft_zip::{Archive, Compression, Limits, Writer};
use egui_kittest::Harness;
use pdfcraft_engine::Session;
use pdfcraft_engine::signing_handoff as handoff;
use pdfcraft_engine::signing_plan::{Plan, Role, plan_bytes, plan_from_bytes, sidecar_of};
use pdfcraft_ui_egui::PdfCraftApp;

use crate::support::{AppHarness as _, shown_notice};

/// The blank document every test here starts from. It is made once: a new file carries the time it was written, so two
/// made a second apart differ, and the tests compare bytes.
fn fixture() -> Vec<u8> {
    static BLANK: OnceLock<Vec<u8>> = OnceLock::new();
    BLANK.get_or_init(|| Session::new().create_blank(300.0, 400.0, 1).unwrap().as_ref().clone()).clone()
}

fn plan() -> Plan {
    let mut plan = Plan::empty(0);
    plan.add_recipient("Ann Author", "ann@example.org", Role::Signer, 0).unwrap();
    plan.add_recipient("Bob Boss", "bob@example.org", Role::Signer, 0).unwrap();
    plan
}

fn pick(path: &str) -> Picked {
    Picked::Paths(vec![PathBuf::from(path)])
}

/// An app over scripted files; `document` is a saved file (with a plan beside it when `with_plan`) opened in it.
fn app(files: &Arc<ScriptedFiles>, document: Option<&Path>) -> Harness<'static, PdfCraftApp> {
    let mut app = crate::support::test_app(crate::support::host_with_files(files.clone()));
    app.set_option(&egui::Context::default(), "language", "en").unwrap();
    if let Some(path) = document {
        app.open_bytes(&egui::Context::default(), "contract.pdf", Some(path.to_string_lossy().into_owned()), fixture()).unwrap();
    }
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1000.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        app
    });
    crate::support::settle(&mut h);
    h
}

/// A saved `contract.pdf` in a folder of its own, with a plan beside it when asked.
fn saved(with_plan: bool) -> (crate::support::TempDir, PathBuf) {
    let dir = crate::support::TempDir::new();
    let pdf = dir.path().join("contract.pdf");
    std::fs::write(&pdf, fixture()).unwrap();
    if with_plan {
        std::fs::write(sidecar_of(&pdf).unwrap(), plan_bytes(&plan()).unwrap()).unwrap();
    }
    (dir, pdf)
}

fn run(h: &mut Harness<'static, PdfCraftApp>, command: &str) {
    assert!(h.execute(command), "{command} is registered and enabled");
    h.run_steps(5);
}

fn package() -> Vec<u8> {
    handoff::build("contract.pdf", &fixture(), &plan()).unwrap().bytes
}

#[test]
fn a_saved_package_reads_back_to_the_document_and_the_plan() {
    let (_dir, pdf) = saved(true);
    let files = Arc::new(ScriptedFiles::answering([pick("/out/pkg.zip")]).with_file(&pdf, fixture()));
    let mut h = app(&files, Some(&pdf));
    run(&mut h, "sign.plan.handoff_export");
    let zip = files.file(Path::new("/out/pkg.zip")).expect("the package was written");
    let back = handoff::read(&zip, 1 << 28).expect("the package passes its own checks");
    assert_eq!(back.pdf, fixture());
    assert_eq!(back.plan.recipients.len(), plan().recipients.len());
}

#[test]
fn a_package_is_never_written_over_a_file_that_is_there() {
    let (_dir, pdf) = saved(true);
    let files = Arc::new(ScriptedFiles::answering([pick("/out/pkg.zip")]).with_file(&pdf, fixture()).with_file("/out/pkg.zip", b"mine".to_vec()));
    let mut h = app(&files, Some(&pdf));
    run(&mut h, "sign.plan.handoff_export");
    assert_eq!(files.file(Path::new("/out/pkg.zip")).unwrap(), b"mine");
    assert!(shown_notice(&h).is_some_and(|n| n.contains("already exists")), "the refusal says why");
}

#[test]
fn a_document_with_no_plan_is_refused_before_anything_is_asked_or_written() {
    let (_dir, pdf) = saved(false);
    let files = Arc::new(ScriptedFiles::answering([pick("/out/pkg.zip")]).with_file(&pdf, fixture()));
    let mut h = app(&files, Some(&pdf));
    run(&mut h, "sign.plan.handoff_export");
    assert!(files.file(Path::new("/out/pkg.zip")).is_none());
    assert!(shown_notice(&h).is_some_and(|n| n.contains("no signing plan")), "the refusal says why");
}

#[test]
fn an_imported_package_puts_the_document_and_its_plan_side_by_side_and_opens_the_document() {
    let files = Arc::new(ScriptedFiles::answering([pick("/in/pkg.zip"), pick("/dest")]).with_file("/in/pkg.zip", package()));
    let mut h = app(&files, None);
    assert!(h.state().active_ids().is_none());
    run(&mut h, "sign.plan.handoff_import");
    let placed = files.file(Path::new("/dest/contract.pdf")).expect("the document was written");
    assert_eq!(placed, fixture());
    let sidecar = sidecar_of(Path::new("/dest/contract.pdf")).unwrap();
    let plan_file = files.file(&sidecar).expect("the plan was written beside it");
    assert_eq!(plan_from_bytes(&plan_file, "plan", "x").unwrap().recipients.len(), plan().recipients.len());
    assert!(h.state().active_ids().is_some(), "the imported document is open");
}

#[test]
fn a_package_whose_document_was_changed_is_refused_with_the_reason_and_nothing_is_written() {
    let package = package();
    let mut zip = Archive::open(&package, Limits { max_entries: 3, max_total_bytes: 1 << 26 }).unwrap();
    let manifest = zip.read("manifest.json", 1 << 16).unwrap().unwrap();
    let plan_file = zip.read("plan.json", 1 << 16).unwrap().unwrap();
    let mut forged = fixture();
    forged.extend_from_slice(b"\n% added after the hash was made\n");
    let mut w = Writer::new();
    w.add("manifest.json", &manifest, Compression::Deflated).unwrap();
    w.add("document.pdf", &forged, Compression::Deflated).unwrap();
    w.add("plan.json", &plan_file, Compression::Deflated).unwrap();
    let files = Arc::new(ScriptedFiles::answering([pick("/in/pkg.zip"), pick("/dest")]).with_file("/in/pkg.zip", w.finish().unwrap()));
    let mut h = app(&files, None);
    run(&mut h, "sign.plan.handoff_import");
    assert!(files.file(Path::new("/dest/contract.pdf")).is_none(), "nothing was written");
    assert!(h.state().active_ids().is_none(), "nothing was opened");
    assert!(shown_notice(&h).is_some_and(|n| n.contains("hash")), "the refusal names the hash");
}

#[test]
fn an_import_does_not_overwrite_a_file_that_is_there() {
    let files = Arc::new(
        ScriptedFiles::answering([pick("/in/pkg.zip"), pick("/dest")])
            .with_file("/in/pkg.zip", package())
            .with_file("/dest/contract.pdf", b"mine".to_vec()),
    );
    let mut h = app(&files, None);
    run(&mut h, "sign.plan.handoff_import");
    assert_eq!(files.file(Path::new("/dest/contract.pdf")).unwrap(), b"mine");
    assert!(files.file(&sidecar_of(Path::new("/dest/contract.pdf")).unwrap()).is_none(), "the plan was not written either");
    assert!(shown_notice(&h).is_some_and(|n| n.contains("already exists")));
}
