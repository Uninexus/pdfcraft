//! Align and distribute objects selected together in Edit a PDF, through the registered commands.
use egui::{Key, Modifiers, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::{AddedText, Edit, EditableObject};
use pdfcraft_ui_egui::PdfCraftApp;

use crate::editing::{Copies, clipboard_app};
use crate::support::AppHarness as _;

/// One empty 400 x 500 pt page.
fn blank_page() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 500] /Contents 4 0 R /Resources << >> >>".to_owned(),
        "<< /Length 0 >>\nstream\n\nendstream".to_owned(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    out
}

/// A page with three added labels of different sizes and places, in Edit text & images.
fn harness() -> Harness<'static, PdfCraftApp> {
    let copies = Copies::default();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_step_dt(0.25).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = clipboard_app(&cc.egui_ctx, &copies);
            app.session = pdfcraft_engine::Session::new().with_clock(|| 1_700_000_000);
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "labels.pdf", None, blank_page()).unwrap();
            let id = app.views[0].id;
            for rect in [[40.0, 380.0, 140.0, 410.0], [200.0, 250.0, 290.0, 280.0], [90.0, 60.0, 150.0, 90.0]] {
                app.session.apply(id, Edit::AddText { page: 0, text: AddedText { text: "Label".into(), rect, ..Default::default() } }).unwrap();
            }
            let saved = app.session.save_bytes(id).unwrap();
            app.session.mark_saved(id, saved, None).unwrap();
            app.set_option(&egui::Context::default(), "zoom", "100").unwrap();
            assert!(app.execute(&egui::Context::default(), "edit.edit_text"));
            app
        }
    });
    crate::support::settle(&mut h);
    h
}

fn at(h: &Harness<'static, PdfCraftApp>, x: f32, y: f32) -> Pos2 {
    let r = h.state().views[0].page_screen_rect(0).unwrap();
    r.min + egui::vec2(x, y) * (r.width() / 400.0)
}

fn objects(h: &Harness<'static, PdfCraftApp>) -> Vec<EditableObject> {
    let app = h.state();
    app.session.get(app.views[0].id).unwrap().editable_objects(0).unwrap()
}

/// The labels' rectangles (user space, y up), one per label in a fixed order.
fn rects(h: &Harness<'static, PdfCraftApp>) -> Vec<[f64; 4]> {
    let mut found = objects(h);
    found.sort_by_key(|o| o.target);
    found.into_iter().map(|o| o.rect).collect()
}

fn select_all_by_marquee(h: &mut Harness<'static, PdfCraftApp>) {
    let (a, b) = (at(h, 4.0, 4.0), at(h, 396.0, 496.0));
    h.hover_at(a);
    h.run_steps(2);
    h.drag_at(a);
    h.run_steps(2);
    h.hover_at(b);
    h.run_steps(2);
    h.drop_at(b);
    h.run_steps(4);
    h.get_by_label("3 objects selected");
}

fn undo(h: &mut Harness<'static, PdfCraftApp>) {
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(4);
}

#[test]
fn align_left_then_distribute_vertically_are_each_one_undo_step() {
    let mut h = harness();
    let start = rects(&h);
    assert_eq!(start.len(), 3);
    assert!(!h.execute("arrange.align_left"), "nothing is selected yet");
    select_all_by_marquee(&mut h);

    assert!(h.execute("arrange.align_left"));
    h.run_steps(4);
    let aligned = rects(&h);
    let least = start.iter().map(|r| r[0]).fold(f64::INFINITY, f64::min);
    assert!(aligned.iter().all(|r| (r[0] - least).abs() < 0.01), "{aligned:?}");
    assert!(aligned.iter().zip(&start).all(|(a, s)| (a[1] - s[1]).abs() < 0.01), "only the left edges move");
    h.get_by_label("3 objects selected");

    assert!(h.execute("arrange.distribute_vertical"));
    h.run_steps(4);
    let spread = rects(&h);
    let mut order: Vec<usize> = (0..3).collect();
    order.sort_by(|&a, &b| spread[a][1].total_cmp(&spread[b][1]));
    let (low, mid, high) = (spread[order[0]], spread[order[1]], spread[order[2]]);
    assert!(((mid[1] - low[3]) - (high[1] - mid[3])).abs() < 0.01, "equal gaps: {spread:?}");
    assert!(order.iter().any(|&i| (spread[i][1] - aligned[i][1]).abs() > 0.01), "the middle label moved");
    assert!(spread.iter().all(|r| (r[0] - least).abs() < 0.01), "distributing vertically keeps the left edges");

    undo(&mut h);
    assert_eq!(rects(&h), aligned, "one undo takes back the distribution only");
    undo(&mut h);
    assert_eq!(rects(&h), start, "one more takes back the alignment");
}
