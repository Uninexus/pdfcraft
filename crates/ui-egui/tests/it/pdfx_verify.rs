//! Standards ▸ Verify PDF/X: the dialog runs the same check as the tool `pdfx_verify` and lists what it finds.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

use crate::support::AppHarness as _;

/// A page that declares PDF/X-3 in its document information but has no output intent.
fn declared_without_intent() -> Vec<u8> {
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /GTS_PDFXVersion (PDF/X-3:2002) /Trapped /False >>",
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R /Info 4 0 R >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).as_bytes());
    out
}

#[test]
fn verify_lists_the_missing_output_intent_and_claims_nothing() {
    let bytes = declared_without_intent();
    let mut h: Harness<'static, PdfCraftApp> = Harness::builder().with_size(egui::vec2(1400.0, 1400.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "x.pdf", None, bytes.clone()).expect("opens");
            app
        }
    });
    h.run_steps(4);
    assert!(h.execute("standards.pdfx_verify"));
    h.run_steps(4);
    assert!(h.query_all_by_label_contains("output intent").next().is_none(), "nothing is listed before Verify");
    h.get_by_label("Verify").click();
    h.run_steps(4);
    assert!(h.query_all_by_label_contains("output intent").next().is_some(), "the missing output intent is a finding");
    h.get_by_label_contains("Checked against PDF/X-3");
    assert!(h.query_all_by_label_contains("conforms").next().is_none(), "the report never claims conformance");
}
