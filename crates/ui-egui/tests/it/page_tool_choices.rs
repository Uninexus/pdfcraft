//! Split's file name template and Remove white margins' padding are kept between runs.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

const FIXTURE: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 300 400] >> endobj
3 0 obj << /Type /Page /Parent 2 0 R >> endobj
4 0 obj << /Type /Page /Parent 2 0 R >> endobj
5 0 obj << /Type /Page /Parent 2 0 R >> endobj
trailer << /Root 1 0 R >>
%%EOF";

fn harness(app: PdfCraftApp) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        let mut app = app;
        app.set_option(&egui::Context::default(), "language", "en").unwrap();
        app.open_bytes(&egui::Context::default(), "doc.pdf", None, FIXTURE.to_vec()).unwrap();
        app
    });
    h.run_steps(4);
    h
}

#[test]
fn split_and_white_margin_choices_come_back_in_the_next_run() {
    let mut first = crate::support::test_app(crate::support::host());
    first.split_template = "{name} part {n}".into();
    first.white_margins_draft.padding = 18.0;
    let kept = first.settings_snapshot();

    let mut next = crate::support::test_app(crate::support::host());
    next.apply_settings(&kept);
    assert_eq!(next.split_template, "{name} part {n}");
    assert!((next.white_margins_draft.padding - 18.0).abs() < 1e-9);

    // The dialog opens with the kept template.
    let mut h = harness(next);
    let ctx = h.ctx.clone();
    assert!(h.state_mut().execute(&ctx, "page.split"));
    h.run_steps(3);
    let field = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "File name template");
    assert_eq!(field.value().as_deref(), Some("{name} part {n}"));
}

#[test]
fn a_kept_value_neither_dialog_takes_is_left_out() {
    let mut first = crate::support::test_app(crate::support::host());
    first.split_template = "{nonsense}".into();
    first.white_margins_draft.padding = -5.0;
    let kept = first.settings_snapshot();
    let mut next = crate::support::test_app(crate::support::host());
    next.apply_settings(&kept);
    assert_eq!(next.split_template, "");
    assert!(next.white_margins_draft.padding.abs() < 1e-9);
}
