//! Pages ▸ Remove white margins in the real shell (egui_kittest): the dialog, the background run with its progress
//! card, the one undo step, and the cases that must change nothing (cancelled, document edited meanwhile).

use std::sync::Arc;

use crate::support::{AppHarness as _, shown_notice};
use craft::host::ManualJobs;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::Edit;
use pdfcraft_ui_egui::PdfCraftApp;

/// Three 300 × 400 pages: a 50 × 50 square on the first, nothing on the second, and a square covering the whole third.
fn fixture() -> Vec<u8> {
    let streams = ["0 g 100 100 50 50 re f", "", "0 g 0 0 300 400 re f"];
    let mut pdf = String::from("%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n");
    pdf.push_str("2 0 obj << /Type /Pages /Kids [3 0 R 5 0 R 7 0 R] /Count 3 /MediaBox [0 0 300 400] >> endobj\n");
    for (n, content) in streams.iter().enumerate() {
        let (page, stream) = (3 + 2 * n, 4 + 2 * n);
        pdf.push_str(&format!("{page} 0 obj << /Type /Page /Parent 2 0 R /Contents {stream} 0 R >> endobj\n"));
        pdf.push_str(&format!("{stream} 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n", content.len() + 1));
    }
    pdf.push_str("trailer << /Root 1 0 R >>\n%%EOF");
    pdf.into_bytes()
}

/// The document open in an app whose jobs wait until the test runs them.
fn harness() -> (Harness<'static, PdfCraftApp>, Arc<ManualJobs>) {
    let (app, jobs) = crate::control::app_with_manual_jobs();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = app;
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "margins.pdf", None, fixture()).unwrap();
            app
        }
    });
    h.run_steps(4);
    (h, jobs)
}

fn widths(h: &Harness<'static, PdfCraftApp>) -> Vec<f32> {
    let app = h.state();
    app.session.get(app.views[0].id).unwrap().info.pages.iter().map(|p| p.width).collect()
}

fn undo_label(h: &Harness<'static, PdfCraftApp>) -> Option<String> {
    let app = h.state();
    app.session.get(app.views[0].id).unwrap().can_undo().map(str::to_owned)
}

/// Runs the host's queued jobs, then lets the app poll them.
fn run_jobs(h: &mut Harness<'static, PdfCraftApp>, jobs: &ManualJobs) {
    jobs.run_all();
    h.run_steps(4);
}

fn start(h: &mut Harness<'static, PdfCraftApp>) {
    let ctx = h.ctx.clone();
    h.state_mut().start_white_margins(&ctx);
    h.run_steps(2);
}

#[test]
fn the_dialog_crops_the_pages_to_their_content_as_one_undoable_step() {
    let (mut h, jobs) = harness();
    assert_eq!(widths(&h), [300.0; 3]);
    assert!(h.execute("page.remove_white_margins"));
    h.run_steps(3);
    h.get_by_role_and_label(Role::Dialog, "Remove white margins");
    h.get_by_role_and_label(Role::Button, "Remove").click();
    h.run_steps(3);
    assert!(h.state().white_margins_run.is_some(), "measuring in the background");
    assert!(shown_notice(&h).is_some_and(|n| n.contains("Measuring margins")), "a progress card says so");
    run_jobs(&mut h, &jobs);
    let after = widths(&h);
    assert!(after[0] < 100.0, "the first page is cut to its square: {after:?}");
    assert_eq!(&after[1..], [300.0; 2], "a blank page and a full page stay");
    assert!(shown_notice(&h).is_some_and(|n| n.starts_with("Cropped 1 page")), "{:?}", shown_notice(&h));
    assert!(h.state().white_margins_run.is_none());
    assert_eq!(undo_label(&h).as_deref(), Some("Remove white margins"));
    h.undo();
    h.run_steps(2);
    assert_eq!(widths(&h), [300.0; 3], "one undo puts every page back");
}

#[test]
fn the_padding_keeps_room_around_the_content_and_the_range_limits_the_pages() {
    let (mut h, jobs) = harness();
    h.state_mut().white_margins_draft.padding = 0.0;
    start(&mut h);
    run_jobs(&mut h, &jobs);
    let tight = widths(&h)[0];
    h.undo();
    h.run_steps(2);
    h.state_mut().white_margins_draft.padding = 10.0;
    start(&mut h);
    run_jobs(&mut h, &jobs);
    assert!(widths(&h)[0] > tight, "10 points more on each side");
    h.undo();
    h.run_steps(2);
    // Only the pages in the range are measured.
    h.state_mut().white_margins_draft.padding = 0.0;
    h.state_mut().white_margins_draft.range =
        craft::pages::PageRange { scope: craft::pages::PageScope::Range, from: 2, to: Some(3), ..Default::default() };
    start(&mut h);
    run_jobs(&mut h, &jobs);
    assert_eq!(widths(&h), [300.0; 3], "page 1 was not chosen");
}

#[test]
fn a_padding_that_is_not_allowed_is_refused_with_the_reason() {
    let (mut h, jobs) = harness();
    h.state_mut().white_margins_draft.padding = -3.0;
    start(&mut h);
    run_jobs(&mut h, &jobs);
    assert!(h.state().white_margins_run.is_none(), "nothing was started");
    assert!(shown_notice(&h).is_some_and(|n| n.contains("The padding must be between 0 and")));
    assert_eq!(undo_label(&h), None);
}

#[test]
fn cancelling_changes_nothing_and_says_so() {
    let (mut h, jobs) = harness();
    start(&mut h);
    h.state_mut().cancel_white_margins();
    run_jobs(&mut h, &jobs);
    assert!(h.state().white_margins_run.is_none());
    assert!(shown_notice(&h).is_some_and(|n| n.contains("cancelled")), "{:?}", shown_notice(&h));
    assert_eq!(widths(&h), [300.0; 3]);
    assert_eq!(undo_label(&h), None);
}

#[test]
fn a_result_is_refused_when_the_document_was_edited_while_it_ran() {
    let (mut h, jobs) = harness();
    start(&mut h);
    let ctx = h.ctx.clone();
    assert!(h.state_mut().apply_edit(&ctx, Edit::RotatePages { pages: vec![1], degrees: 90 }));
    run_jobs(&mut h, &jobs);
    assert!(h.state().white_margins_run.is_none());
    assert_eq!(undo_label(&h).as_deref(), Some("Rotate page"), "no crop step was added after the edit");
    assert!(shown_notice(&h).is_some_and(|n| n.contains("changed while its margins were being measured")));
}

#[test]
fn the_pages_menu_lists_it_and_opens_the_dialog() {
    let (mut h, _jobs) = harness();
    crate::support::open_menu(&mut h, "Edit");
    crate::support::menu_row(&h, "Pages").hover();
    h.run_steps(3);
    crate::support::menu_row(&h, "Remove white margins…").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Dialog, "Remove white margins");
}

#[test]
fn the_subset_trims_only_odd_or_even_pages() {
    use craft::pages::{PageRange, Subset};

    let (mut h, jobs) = harness();
    // Even pages: the second is blank, so nothing changes, though the first has margins to trim.
    h.state_mut().white_margins_draft.range = PageRange { subset: Subset::Even, ..PageRange::default() };
    start(&mut h);
    run_jobs(&mut h, &jobs);
    assert_eq!(widths(&h), [300.0; 3], "the odd pages were left alone");
    h.state_mut().white_margins_draft.range = PageRange { subset: Subset::Odd, ..PageRange::default() };
    start(&mut h);
    run_jobs(&mut h, &jobs);
    assert!(widths(&h)[0] < 100.0, "the first page is odd");
    // The dialog offers the list.
    assert!(h.execute("page.remove_white_margins"));
    h.run_steps(3);
    h.get_by_role_and_label(Role::Dialog, "Remove white margins").get_by_role_and_label(Role::ComboBox, "Subset");
}
