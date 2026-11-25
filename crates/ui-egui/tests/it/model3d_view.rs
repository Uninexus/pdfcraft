//! View > 3D model: a page's PRC model in a window the reader turns, zooms and pans. The model is a synthetic
//! one-triangle PRC file in a synthetic PDF (the one `model_render`'s tests draw).

use std::sync::Arc;

use crate::support::AppHarness as _;
use crate::support::input::one_frame;
use craft::host::ScriptedFiles;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

/// One triangle at z = 0 with corners (-1,-1), (1,-1) and (0,1), as a PRC file of one file structure with no
/// product tree, in hexadecimal for the stream's `/ASCIIHexDecode`.
const TRIANGLE_PRC_HEX: &str = "505243c91f0000c91f00000100000002000000030000000400000000000000000000000000000000000000010000000500000006000000070000000800000000000000060000006b0000009a000000a3000000ab000000cd000000d5000000dd000000e500000000000000505243c91f0000c91f0000050000000600000007000000080000000000000000000000000000000000000000000000789c63000000010001789c030000000001789c9b71c087758da2422b43084fe3140e038713464cd759381a162800006ed70828789c030000000001789c030000000001789c030000000001";

/// A one-page PDF whose page holds a 3D annotation over a stream of `/Subtype /{subtype}`.
fn pdf(subtype: &str, hex: &str) -> Vec<u8> {
    let stream = format!("<< /Type /3D /Subtype /{subtype} /Filter /ASCIIHexDecode /Length {} >>\nstream\n{hex}>\nendstream", hex.len() + 1);
    let objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Annots [4 0 R] >>".into(),
        "<< /Type /Annot /Subtype /3D /Rect [20 20 180 180] /3DD 5 0 R >>".into(),
        stream,
    ];
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

fn harness(bytes: Vec<u8>) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1400.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::app_with_files(Arc::new(ScriptedFiles::new()));
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "model.pdf", None, bytes).unwrap();
            app
        }
    });
    crate::support::settle(&mut h);
    h
}

fn open_window(bytes: Vec<u8>) -> Harness<'static, PdfCraftApp> {
    let mut h = harness(bytes);
    assert!(h.execute("view.model3d"));
    h.run_steps(4);
    h
}

fn hash(h: &Harness<'static, PdfCraftApp>) -> u64 {
    h.state().model3d.as_ref().and_then(|w| w.picture_hash()).expect("a picture is drawn")
}

fn picture_rect(h: &Harness<'static, PdfCraftApp>) -> egui::Rect {
    h.get_by_role_and_label(Role::Image, "3D model, page 1").rect()
}

#[test]
fn the_window_shows_the_pages_model_and_a_named_view_changes_the_picture() {
    let mut h = open_window(pdf("PRC", TRIANGLE_PRC_HEX));
    let size = h.state().model3d.as_ref().and_then(|w| w.picture_size()).expect("a picture is drawn");
    assert!(size[0] > 0 && size[1] > 0, "{size:?}");
    let surface = h.get_by_role_and_label(Role::Image, "3D model, page 1");
    assert_eq!(surface.value().as_deref(), Some("Isometric"), "the surface reports its view");
    let before = hash(&h);
    h.query_all_by_role_and_label(Role::Button, "Front").last().expect("a Front button").click();
    h.run_steps(3);
    assert_ne!(hash(&h), before, "the Front view is another picture");
    assert_eq!(h.state().model3d.as_ref().and_then(|w| w.named_view()), Some("front"));
    h.query_all_by_role_and_label(Role::Button, "Reset view").last().expect("a Reset view button").click();
    h.run_steps(3);
    assert_eq!(hash(&h), before, "Reset view returns to the opening picture");
    h.query_all_by_role_and_label(Role::Button, "Close").last().expect("the window's Close").click();
    h.run_steps(3);
    assert!(h.state().model3d.is_none());
}

#[test]
fn dragging_scrolling_and_the_arrow_keys_move_the_view() {
    let mut h = open_window(pdf("PRC", TRIANGLE_PRC_HEX));
    let centre = picture_rect(&h).center();
    let start = hash(&h);
    let primary = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    one_frame(&mut h, [egui::Event::PointerMoved(centre), primary(centre, true)]);
    for step in [1.0_f32, 2.0, 3.0, 4.0] {
        one_frame(&mut h, [egui::Event::PointerMoved(centre + egui::vec2(15.0 * step, 5.0))]);
    }
    one_frame(&mut h, [primary(centre + egui::vec2(60.0, 5.0), false)]);
    h.run_steps(3);
    let turned = hash(&h);
    assert_ne!(turned, start, "dragging turns the model");
    assert_eq!(h.state().model3d.as_ref().and_then(|w| w.named_view()), None, "a turned view is no named view");
    assert_eq!(h.get_by_role_and_label(Role::Image, "3D model, page 1").value().as_deref(), Some("Custom view"));

    h.hover_at(centre);
    h.run_steps(1);
    one_frame(
        &mut h,
        [egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 120.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
            source: egui::MouseWheelSource::Unknown,
        }],
    );
    h.run_steps(4);
    let zoomed = hash(&h);
    assert_ne!(zoomed, turned, "scrolling zooms");

    h.get_by_role_and_label(Role::Image, "3D model, page 1").click();
    h.run_steps(2);
    h.key_press(egui::Key::ArrowLeft);
    h.run_steps(3);
    assert_ne!(hash(&h), zoomed, "an arrow key turns the focused picture");
}

#[test]
fn a_page_without_a_model_or_with_a_u3d_one_says_so_plainly() {
    let session = pdfcraft_engine::Session::new();
    let blank = session.create_blank(200.0, 200.0, 1).unwrap();
    let h = open_window(blank.as_ref().clone());
    let window = h.state().model3d.as_ref().expect("the window opens");
    assert!(!window.has_model());
    assert_eq!(window.message(), Some("This page has no 3D model."));
    assert!(h.query_all_by_label_contains("This page has no 3D model.").count() > 0, "the message is on screen");

    let h = open_window(pdf("U3D", "553344"));
    let message = h.state().model3d.as_ref().and_then(|w| w.message()).expect("a message").to_string();
    assert!(message.contains("U3D"), "{message}");
    assert!(h.state().model3d.as_ref().is_some_and(|w| w.picture_size().is_none()), "no picture of a U3D model");
}
