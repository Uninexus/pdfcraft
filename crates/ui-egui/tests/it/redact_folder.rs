//! Redact ▸ Redact files in a folder in the real shell: the dialog, the two folder picks, the
//! redacted copies and the report that names what was not redacted.

use std::sync::Arc;

use crate::support::{AppHarness as _, fixtures, shown_notice};
use craft::host::{ManualJobs, Picked, ScriptedFiles};
use craft::ui::dialog::dialog_id;
use craft::ui::patterns::form_dialogs::{FieldValue, HostedParamDialog};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

#[test]
fn a_folder_is_redacted_into_copies_and_the_report_names_what_was_not() {
    let root = std::env::temp_dir().join(format!("pdfcraft-ui-redact-folder-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (input, output) = (root.join("in"), root.join("out"));
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(input.join("a.pdf"), fixtures::pdf_pages(2)).unwrap();
    std::fs::write(input.join("bad.pdf"), b"not a PDF").unwrap();
    // The engine reads and writes the disk; only the folder picker is scripted.
    let files = Arc::new(ScriptedFiles::answering([Picked::Paths(vec![input.clone()]), Picked::Paths(vec![output.clone()])]));
    let jobs = Arc::new(ManualJobs::new());
    let app = crate::support::test_app(crate::support::host_with(jobs.clone(), files));
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        let mut app = app;
        app.set_option(&egui::Context::default(), "language", "en").unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.execute("redact.folder"), "needs no open document");
    h.run_steps(3);
    let dialog = h.state_mut().dialogs.get_mut_by_id::<HostedParamDialog>(dialog_id("redact.folder")).expect("the dialog is open");
    dialog.set_field("words", FieldValue::Text("quick".into()));
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Choose folders").click();
    for _ in 0..4 {
        h.run_steps(3);
        jobs.run_all();
    }
    h.run_steps(4);
    assert!(output.join("a_Redacted.pdf").is_file(), "{:?}", shown_notice(&h));
    assert!(!output.join("bad_Redacted.pdf").exists());
    let report = std::fs::read_to_string(output.join("redaction-report.txt")).unwrap();
    assert!(report.contains("NOT REDACTED") && report.contains("bad.pdf") && report.contains("a.pdf"), "{report}");
    assert!(report.find("bad.pdf") < report.find("a.pdf: "), "what needs a look comes first: {report}");
    let said = shown_notice(&h).unwrap_or_default();
    assert!(said.contains("Redacted 1 file(s)") && said.contains("Not redacted: 1"), "{said}");
    let _ = std::fs::remove_dir_all(&root);
}
