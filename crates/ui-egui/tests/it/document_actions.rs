//! A form's Did Save and Will Close document actions in the real shell (egui_kittest): the same
//! actions `doc_save` and `doc_close` run through the tool table. Did Save runs once the file is
//! written and what it changes stays an unsaved change; Will Close runs on a close that goes ahead.

use std::sync::Arc;

use craft::host::ScriptedFiles;
use egui_kittest::Harness;
use pdfcraft_ui_egui::{PdfCraftApp, SaveTarget};

fn pdf_of(objs: &[String]) -> Vec<u8> {
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

/// A form whose Will Close, Did Save and Did Print actions each add a word to `log` and say
/// something on the console.
fn closing_form() -> Vec<u8> {
    let act = |tag: &str| format!("<< /S /JavaScript /JS (getField\\('log'\\).value += '{tag};'; console.println\\('{tag}'\\);) >>");
    pdf_of(&[
        format!(
            "<< /Type /Catalog /Pages 2 0 R /AA << /WC {} /DS {} /DP {} >> /AcroForm << /Fields [4 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 5 0 R >> >> >> >>",
            act("wc"),
            act("ds"),
            act("dp")
        ),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Annots [4 0 R] >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (log) /V () /Rect [20 250 180 270] /P 3 0 R /F 4 >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into(),
    ])
}

fn harness(files: &Arc<ScriptedFiles>, path: &str) -> Harness<'static, PdfCraftApp> {
    let files = files.clone();
    let path = path.to_owned();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1400.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        let mut app = crate::control::app_with_files(files.clone());
        app.set_option(&egui::Context::default(), "language", "en").unwrap();
        app.open_bytes(&egui::Context::default(), "form.pdf", Some(path.clone()), closing_form()).unwrap();
        app
    });
    h.run_steps(3);
    h
}

fn log_value(app: &PdfCraftApp) -> String {
    let doc = app.session.get(app.views[0].id).unwrap();
    doc.form.iter().find(|f| f.name == "log").map(|f| f.value.concat()).unwrap_or_default()
}

#[test]
fn did_save_changes_the_open_document_but_not_the_file_just_written() {
    let path = std::env::temp_dir().join(format!("pdfcraft-did-save-{}.pdf", std::process::id()));
    let files = Arc::new(ScriptedFiles::new());
    let mut h = harness(&files, &path.to_string_lossy());
    assert!(!log_value(h.state()).contains("ds;"), "nothing ran on opening");

    assert!(h.state_mut().save_active(&egui::Context::default(), SaveTarget::InPlace));
    assert!(log_value(h.state()).contains("ds;"), "Did Save ran after the write and changed the open document");
    let id = h.state().views[0].id;
    assert!(h.state().session.get(id).unwrap().dirty, "what Did Save changed is an unsaved change");
    assert!(h.state().js_console.log.iter().any(|l| l == "ds"), "its console line was shown: {:?}", h.state().js_console.log);

    let written = files.file(&path).expect("the save wrote the file");
    let mut reopened = crate::control::new_app();
    reopened.open_bytes(&egui::Context::default(), "saved.pdf", None, written).unwrap();
    assert!(!log_value(&reopened).contains("ds;"), "Did Save's change is not in the written file");
}

#[test]
fn will_close_runs_before_a_close_that_goes_ahead() {
    let files = Arc::new(ScriptedFiles::new());
    let mut h = harness(&files, "/tmp/pdfcraft-will-close.pdf");
    assert!(!h.state().js_console.log.iter().any(|l| l == "wc"), "nothing ran on opening");
    // A clean document closes at once, and Will Close ran first.
    h.state_mut().request_close_tab(&egui::Context::default(), 0);
    assert!(h.state().views.is_empty());
    assert!(h.state().js_console.log.iter().any(|l| l == "wc"), "Will Close ran: {:?}", h.state().js_console.log);
}

#[test]
fn close_all_runs_will_close_for_each_clean_document() {
    let files = Arc::new(ScriptedFiles::new());
    let mut h = harness(&files, "/tmp/pdfcraft-will-close-all.pdf");
    h.state_mut().open_bytes(&egui::Context::default(), "second.pdf", None, closing_form()).unwrap();
    assert_eq!(h.state().views.len(), 2);
    h.state_mut().close_all(&egui::Context::default());
    assert!(h.state().views.is_empty());
    let ran = h.state().js_console.log.iter().filter(|l| *l == "wc").count();
    assert_eq!(ran, 2, "one Will Close per document: {:?}", h.state().js_console.log);
}
