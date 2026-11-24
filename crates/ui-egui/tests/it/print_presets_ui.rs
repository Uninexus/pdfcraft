//! Document Properties ▸ Advanced ▸ Print Dialog Presets: set with OK as one undo step, shown when the
//! document already has them, and an entry that cannot be read is shown as such and kept.

use crate::support::AppHarness as _;
use craft::ui::dialog::dialog_id;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::{Duplex, PrintPresets, PrintScaling};
use pdfcraft_ui_egui::PdfCraftApp;
use serde_json::{Value, json};

/// Two pages, with this `/ViewerPreferences` entry in the catalog.
fn pdf_with(prefs: &str) -> Vec<u8> {
    format!(
        "%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R {prefs} >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 300] >> endobj
3 0 obj << /Type /Page /Parent 2 0 R >> endobj
4 0 obj << /Type /Page /Parent 2 0 R >> endobj
trailer << /Root 1 0 R >>
%%EOF"
    )
    .into_bytes()
}

/// Open Document Properties on `pdf`.
fn harness(pdf: Vec<u8>) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "presets.pdf", None, pdf.clone()).unwrap();
            app.set_option(&egui::Context::default(), "dialog", "properties").unwrap();
            app
        }
    });
    h.run_steps(4);
    h
}

/// Set one row of the open dialog, as automation does.
fn set_field(h: &mut Harness<'static, PdfCraftApp>, field: &str, value: Value) {
    let ctx = h.ctx.clone();
    h.state_mut().dialogs.set_json_field(&ctx, dialog_id("file.properties"), field, value).unwrap();
    h.run_steps(2);
}

/// A copy of the open dialog's fields.
fn fields(h: &Harness<'static, PdfCraftApp>) -> DialogFields {
    h.state().dialogs.get_by_id::<HostedParamDialog>(dialog_id("file.properties")).expect("Document Properties is open").fields().clone()
}

fn press_ok(h: &mut Harness<'static, PdfCraftApp>) {
    h.get_by_label("OK").click();
    h.run_steps(3);
}

/// The presets and the unreadable settings of the first document, and whether it has an undo step.
fn stored(h: &Harness<'static, PdfCraftApp>) -> (PrintPresets, Vec<&'static str>, bool) {
    let id = h.state().views[0].id;
    let doc = h.state().session.get(id).unwrap();
    let (presets, unreadable) = doc.print_presets();
    (presets, unreadable, doc.can_undo().is_some())
}

#[test]
fn print_presets_set_in_the_dialog_are_written_by_ok_as_one_undo_step() {
    let mut h = harness(pdf_with(""));
    assert_eq!(stored(&h), (PrintPresets::default(), vec![], false));
    set_field(&mut h, "print_scaling", json!("none"));
    set_field(&mut h, "print_duplex", json!("flip_short_edge"));
    set_field(&mut h, "print_pick_tray", json!("no"));
    set_field(&mut h, "print_copies", json!("4"));
    set_field(&mut h, "print_ranges", json!("1-2"));
    press_ok(&mut h);
    let (presets, unreadable, undoable) = stored(&h);
    assert_eq!(
        presets,
        PrintPresets {
            scaling: PrintScaling::None,
            duplex: Some(Duplex::FlipShortEdge),
            pick_tray_by_size: Some(false),
            page_ranges: vec![(1, 2)],
            copies: Some(4),
        }
    );
    assert!(unreadable.is_empty() && undoable);
    h.undo();
    h.run_steps(3);
    assert_eq!(stored(&h), (PrintPresets::default(), vec![], false), "one undo takes every preset back");
}

#[test]
fn the_dialog_shows_the_presets_the_document_already_has() {
    let h = harness(pdf_with(
        "/ViewerPreferences << /PrintScaling /None /Duplex /DuplexFlipLongEdge /PickTrayByPDFSize true /NumCopies 3 /PrintPageRange [1 1 2 2] >>",
    ));
    let shown = fields(&h);
    assert_eq!(shown.text("print_scaling"), Some("none"));
    assert_eq!(shown.text("print_duplex"), Some("flip_long_edge"));
    assert_eq!(shown.text("print_pick_tray"), Some("yes"));
    assert_eq!(shown.text("print_copies"), Some("3"));
    assert_eq!(shown.text("print_ranges"), Some("1, 2"));
}

#[test]
fn a_preset_that_cannot_be_read_is_shown_as_such_and_kept_until_a_real_value_is_picked() {
    let mut h = harness(pdf_with("/ViewerPreferences << /NumCopies 1 >>"));
    assert_eq!(fields(&h).text("print_copies"), Some("unreadable"));
    press_ok(&mut h);
    let (presets, unreadable, undoable) = stored(&h);
    assert_eq!((presets.copies, unreadable, undoable), (None, vec!["copies"], false), "OK with no change leaves the entry alone");

    let mut h = harness(pdf_with("/ViewerPreferences << /NumCopies 1 >>"));
    set_field(&mut h, "print_copies", json!("2"));
    press_ok(&mut h);
    let (presets, unreadable, _) = stored(&h);
    assert_eq!((presets.copies, unreadable), (Some(2), vec![]));
}
