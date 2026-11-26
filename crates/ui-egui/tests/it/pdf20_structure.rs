//! PDF 2.0 structure: the dialog lists the same namespaces and document parts as the tools `ns_list` and `dpart_list`.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

use crate::support::AppHarness as _;

/// A one-page PDF 2.0 file with one structure namespace and one document part covering page 1.
fn with_namespace_and_part() -> Vec<u8> {
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 4 0 R /DPartRoot 6 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Type /StructTreeRoot /Namespaces [5 0 R] >>",
        "<< /Type /Namespace /NS (urn:example:tags) >>",
        "<< /Type /DPartRoot /DPartRootNode 7 0 R /RecordLevel 0 >>",
        "<< /Type /DPart /DParts [[8 0 R]] >>",
        "<< /Type /DPart /Parent 7 0 R /Start 3 0 R /End 3 0 R >>",
    ];
    let mut out = b"%PDF-2.0\n".to_vec();
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
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).as_bytes());
    out
}

#[test]
fn the_dialog_lists_the_namespace_and_the_part_with_its_pages_and_claims_nothing() {
    let bytes = with_namespace_and_part();
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
    assert!(h.execute("standards.pdf20_structure"));
    h.run_steps(6);
    h.get_by_label_contains("urn:example:tags");
    h.get_by_label_contains("page 1");
    assert!(h.query_all_by_label_contains("This document has no namespaces").next().is_none(), "the namespace is listed, not the empty state");
    assert!(h.query_all_by_label_contains("This document has no document parts").next().is_none());
    assert!(h.query_all_by_label_contains("is valid").next().is_none(), "the dialog never claims validity");
}
