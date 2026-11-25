//! Document Properties ▸ Advanced ▸ Fast web view: Yes for a linearized file whose dictionary
//! still holds, No for an ordinary one.

use crate::support::AppHarness as _;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

/// A one-page PDF whose first object is a linearization parameter dictionary, `/L` being its own
/// length and the offsets real; the hint stream is a stub.
fn linearized_pdf() -> Vec<u8> {
    let build = |l: usize| {
        let mut out = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
        let mut offsets = [0usize; 7];
        let bodies = [
            (5, format!("<< /Linearized 1 /L {l:08} /H [ 0000000000 0000000000 ] /O 3 /E 0000000000 /N 1 /T 0000000000 >>")),
            (1, "<< /Type /Catalog /Pages 2 0 R >>".to_string()),
            (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string()),
            (3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>".to_string()),
            (6, "<< /Length 0 >>".to_string()),
        ];
        for (num, body) in bodies {
            if let Some(o) = offsets.get_mut(num) {
                *o = out.len();
            }
            out.extend_from_slice(format!("{num} 0 obj\n{body}\nendobj\n").as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
        for o in offsets.iter().skip(1).take(5) {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
        out
    };
    let len = build(0).len();
    let mut out = build(len);
    let end = out.len() / 2;
    let hint = out.windows(7).position(|w| w == b"6 0 obj").unwrap_or(0);
    for (key, at) in [("/E ", end), ("/T ", end), ("/H [ ", hint)] {
        if let Some(p) = out.windows(key.len()).position(|w| w == key.as_bytes()) {
            let p = p + key.len();
            out[p..p + 10].copy_from_slice(format!("{at:010}").as_bytes());
        }
    }
    out
}

/// An ordinary one-page PDF.
fn plain_pdf() -> Vec<u8> {
    b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >> endobj
3 0 obj << /Type /Page /Parent 2 0 R >> endobj
trailer << /Root 1 0 R >>
%%EOF"
        .to_vec()
}

/// Opens Document Properties on its Advanced tab for `pdf`.
fn harness(pdf: Vec<u8>) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1400.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "fast.pdf", None, pdf.clone()).unwrap();
            app.set_option(&egui::Context::default(), "dialog", "advanced").unwrap();
            app
        }
    });
    h.run_steps(4);
    h
}

/// What the Fast web view row says: its label and value as the accessibility tree has them.
fn row_text(h: &Harness<'static, PdfCraftApp>) -> String {
    h.query_all_by(|n| n.label().is_some_and(|l| l.starts_with("Fast web view")))
        .map(|n| format!("{} {}", n.label().unwrap_or_default(), n.value().unwrap_or_default()))
        .collect::<Vec<_>>()
        .join(" | ")
}

#[test]
fn document_properties_says_whether_the_file_is_set_up_for_fast_web_view() {
    let fast = row_text(&harness(linearized_pdf()));
    let plain = row_text(&harness(plain_pdf()));
    assert!(fast.contains("Yes"), "{fast}");
    assert!(plain.contains("No"), "{plain}");
    assert_ne!(fast, plain);
}
