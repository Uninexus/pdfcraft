//! Compare files in the real shell (egui_kittest): the dialog, the Compare panel, marks and
//! the report.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::Session;
use pdfcraft_ui_egui::{PdfCraftApp, RightPanel};

#[test]
fn compare_two_versions() {
    let s = Session::new();
    let v1 = s.create_from_text("t", "Delivery within five days. Returns accepted.").unwrap().to_vec();
    let v2 = s.create_from_text("t", "Delivery within three days. Returns accepted for a week.").unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("v1.pdf", None, v1.clone()).unwrap();
        app.open_bytes("v2.pdf", None, v2.clone()).unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("doc.compare"));
    h.run_steps(2);
    h.get_by_label("Compare Files");
    h.get_by_label("Compare").click();
    h.run_steps(3);
    assert_eq!(h.state().right, Some(RightPanel::Compare));
    assert_eq!(h.state().toast.clone().unwrap().0, "2 differences found");
    h.get_by_label("2 Replaced");
    h.get_by_label("\"five\" → \"three\"").click();
    h.run_steps(2);
    assert_eq!(h.state().compare.as_ref().unwrap().selected, Some(0));
    let id = h.state().active_ids().unwrap().1;
    let i = h.state().active_ids().unwrap().0;
    assert_eq!(h.state().views[i].compare_marks.len(), 2);
    // Side by side: the old file's half shades its side of the changes and follows the new
    // file's zoom; "New file only" ends it.
    assert!(h.state().compare.as_ref().unwrap().side_by_side, "side by side on a wide window");
    let old_i = h.state().views.iter().position(|v| v.id == h.state().compare.as_ref().unwrap().old).unwrap();
    assert_eq!(h.state().views[old_i].compare_marks.len(), 2, "replaced words shaded on the old file too");
    assert!(h.state().views[old_i].flash.is_some(), "the old half jumped to the change");
    h.state_mut().views[i].zoom = 1.5;
    h.state_mut().views[i].fit = pdfcraft_ui_egui::canvas::Fit::None;
    h.run_steps(3);
    assert_eq!((h.state().views[old_i].zoom, h.state().views[old_i].fit), (1.5, pdfcraft_ui_egui::canvas::Fit::None), "the old half follows");
    h.get_by_label("New file only").click();
    h.run_steps(2);
    assert!(!h.state().compare.as_ref().unwrap().side_by_side);
    h.get_by_label("Side by side").click();
    h.run_steps(2);
    assert!(h.state().compare.as_ref().unwrap().side_by_side);
    h.get_by_label("New file only").click();
    h.run_steps(2);
    h.get_by_label("Mark as comments").click();
    h.run_steps(3);
    assert_eq!(h.state().session.get(id).unwrap().info.annotations.len(), 2);
    h.get_by_label("Report…").click();
    h.run_steps(3);
    assert_eq!(h.state().session.get(h.state().active_ids().unwrap().1).unwrap().name, "Compare Report.pdf");
}

/// Two versions compared side by side (old `v1.pdf` left, new `v2.pdf` right).
fn side_by_side() -> Harness<'static, PdfCraftApp> {
    let s = Session::new();
    let v1 = s.create_from_text("t", "Delivery within five days. Returns accepted.").unwrap().to_vec();
    let v2 = s.create_from_text("t", "Delivery within three days. Returns accepted for a week.").unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("v1.pdf", None, v1.clone()).unwrap();
        app.open_bytes("v2.pdf", None, v2.clone()).unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("doc.compare"));
    h.run_steps(2);
    h.get_by_label("Compare").click();
    h.run_steps(3);
    assert!(h.state().compare.as_ref().unwrap().side_by_side);
    h
}

/// #3915: a half that closes its own tab while it draws (its snapshot bar's Close sets
/// `close_requested`) must not leave the frame indexing the closed tab. Either half.
#[test]
fn closing_either_half_of_a_side_by_side_comparison_keeps_the_app_running() {
    for (half, name) in [(1, "v2.pdf"), (0, "v1.pdf")] {
        let mut h = side_by_side();
        let c = h.state().compare.as_ref().unwrap();
        let id = if half == 1 { c.new } else { c.old };
        let i = h.state().views.iter().position(|v| v.id == id).unwrap();
        h.state_mut().views[i].close_requested = true;
        h.run_steps(3);
        let s = h.state();
        assert_eq!(s.views.len(), 1, "the {name} tab closed");
        assert!(s.views.iter().all(|v| v.id != id));
    }
}

/// Two versions whose differences are of both kinds: a replaced word (text) and a rectangle
/// drawn far below the text (visual only, no text change explains it).
fn text_and_visual_versions() -> (Vec<u8>, Vec<u8>) {
    let mut s = Session::new();
    let v1 = s.create_from_text("t", "Delivery within five days. Returns accepted.").unwrap().to_vec();
    let text = s.create_from_text("t", "Delivery within three days. Returns accepted.").unwrap();
    let id = s.open("v2.pdf", None, text, None).unwrap();
    let square = pdfcraft_engine::Shape::Rectangle { rect: [100.0, 80.0, 300.0, 200.0] };
    let style = pdfcraft_engine::Style::default_for(&square);
    let new = pdfcraft_engine::NewAnnotation { page: 0, shape: square, style, contents: String::new(), author: "Ada".into() };
    s.apply(id, pdfcraft_engine::Edit::AddAnnotation(new)).unwrap();
    (v1, s.save_bytes(id).unwrap().to_vec())
}

/// The Compare panel's Show checkboxes (Replaced, Inserted, Deleted, Visual), on both halves: a
/// kind of text change leaves the list and the shading, Visual takes the visual differences
/// (counted, never listed) out of the shading, and each brings them back.
#[test]
fn compare_filters_hide_text_or_visual_changes() {
    let (v1, v2) = text_and_visual_versions();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("v1.pdf", None, v1.clone()).unwrap();
        app.open_bytes("v2.pdf", None, v2.clone()).unwrap();
        app
    });
    h.run_steps(4);
    h.state_mut().compare_old = h.state().views.first().map(|v| v.id);
    h.state_mut().run_compare();
    h.run_steps(3);
    let new_i = h.state().active_ids().unwrap().0;
    let old_i = h.state().views.iter().position(|v| v.id == h.state().compare.as_ref().unwrap().old).unwrap();
    // Visual differences are shaded orange (compare_ui::VISUAL), text changes in their kind's colour.
    let visual = egui::Color32::from_rgb(0xF0, 0x8C, 0x1A);
    let kinds = |h: &Harness<'_, PdfCraftApp>, i: usize| {
        let marks = &h.state().views[i].compare_marks;
        (marks.iter().filter(|m| m.2 != visual).count(), marks.iter().filter(|m| m.2 == visual).count())
    };
    let (text_new, visual_new) = kinds(&h, new_i);
    let (text_old, visual_old) = kinds(&h, old_i);
    assert!(text_new > 0 && visual_new > 0, "the fixture has both kinds on the new file: {text_new} text, {visual_new} visual");
    assert!(text_old > 0 && visual_old > 0, "and on the old file: {text_old} text, {visual_old} visual");
    h.get_by_label("\"five\" → \"three\"");

    h.get_by_label("Replaced").click();
    h.run_steps(2);
    assert!(h.query_by_label("\"five\" → \"three\"").is_none(), "replaced changes leave the list");
    assert_eq!(kinds(&h, new_i), (0, visual_new), "and the new file's shading");
    assert_eq!(kinds(&h, old_i), (0, visual_old), "and the old file's");

    h.get_by_label("Visual").click();
    h.run_steps(2);
    assert_eq!(kinds(&h, new_i), (0, 0));
    assert_eq!(kinds(&h, old_i), (0, 0));

    h.get_by_label("Replaced").click();
    h.run_steps(2);
    h.get_by_label("\"five\" → \"three\"");
    assert_eq!(kinds(&h, new_i), (text_new, 0), "replaced back, visual still hidden");
    assert_eq!(kinds(&h, old_i), (text_old, 0));
}

/// The Compare Files dialog's Kinds of change checkboxes choose what the comparison shows, and
/// Mark as comments and the report follow them (the panel's Show checkboxes change them after).
#[test]
fn compare_dialog_kinds_filter_the_panel_marks_and_report() {
    let s = Session::new();
    let v1 = s.create_from_text("t", "Rent is 900 per month. Pets are not allowed.").unwrap().to_vec();
    let v2 = s.create_from_text("t", "Rent is 950 per month. Pets are allowed. Parking included.").unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("v1.pdf", None, v1.clone()).unwrap();
        app.open_bytes("v2.pdf", None, v2.clone()).unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("doc.compare"));
    h.run_steps(2);
    h.get_by_label("Kinds of change:");
    // Without Inserted and Deleted: only the replacement is compared and listed.
    h.get_by_label("Inserted").click();
    h.get_by_label("Deleted").click();
    h.run_steps(2);
    h.get_by_label("Compare").click();
    h.run_steps(3);
    let k = h.state().compare.as_ref().unwrap().kinds;
    assert_eq!((k.replaced, k.inserted, k.deleted, k.visual), (true, false, false, true));
    h.get_by_label("\"900\" → \"950\"");
    assert!(h.query_by_label("+ \"Parking included.\"").is_none() && h.query_by_label("+ \"Parking included\"").is_none());
    let id = h.state().active_ids().unwrap().1;
    h.get_by_label("Mark as comments").click();
    h.run_steps(3);
    assert_eq!(h.state().session.get(id).unwrap().info.annotations.len(), 1, "only the replacement is marked");
    // Nothing ticked: Mark as comments and Report… are off, so no empty-kinds error is possible.
    for k in ["Replaced", "Visual"] {
        h.get_by_label(k).click();
        h.run_steps(2);
    }
    assert!(h.state().compare.as_ref().unwrap().kinds.is_empty());
    assert!(h.query_by(|n| n.label().as_deref() == Some("Mark as comments") && n.is_disabled()).is_some(), "Mark as comments is off");
    assert!(h.query_by(|n| n.label().as_deref() == Some("Report…") && n.is_disabled()).is_some(), "Report… is off");
    // Visual only: Mark as comments says why it adds nothing.
    h.get_by_label("Visual").click();
    h.run_steps(2);
    h.get_by_label("Mark as comments").click();
    h.run_steps(3);
    assert_eq!(h.state().toast.clone().unwrap().0, "No comments added: visual differences are shaded, not marked as comments");
    assert_eq!(h.state().session.get(id).unwrap().info.annotations.len(), 1, "still only the first mark");
    h.get_by_label("Replaced").click();
    h.run_steps(2);
    // Bringing Inserted back through the panel marks the insertion too.
    h.get_by_label("Inserted").click();
    h.run_steps(2);
    assert!(h.state().compare.as_ref().unwrap().kinds.inserted);
    h.get_by_label("Mark as comments").click();
    h.run_steps(3);
    assert_eq!(h.state().session.get(id).unwrap().info.annotations.len(), 3, "1 + replaced and inserted again");
    // Deleted back on: the report opens as a new document.
    h.get_by_label("Deleted").click();
    h.run_steps(2);
    h.get_by_label("Report…").click();
    h.run_steps(3);
    let report = h.state().active_ids().unwrap().1;
    assert_eq!(h.state().session.get(report).unwrap().name, "Compare Report.pdf");
}

/// The Compare Files dialog's New file pages field rejects an out-of-range page: the
/// comparison does not run and the dialog stays open with the typed range kept.
#[test]
fn compare_dialog_pages_field_rejects_out_of_range() {
    let s = Session::new();
    let v1 = s.create_from_text("t", "Delivery within five days. Returns accepted.").unwrap().to_vec();
    let v2 = s.create_from_text("t", "Delivery within three days. Returns accepted for a week.").unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("v1.pdf", None, v1.clone()).unwrap();
        app.open_bytes("v2.pdf", None, v2.clone()).unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("doc.compare"));
    h.run_steps(2);
    h.get_by_label("Compare Files");
    let pages = h.query_by_role_and_label(egui::accesskit::Role::TextInput, "New file pages");
    assert!(pages.is_some(), "the dialog has a New file pages text input");
    assert!(h.query_by_label("New file pages:").is_some(), "the dialog has a New file pages field");
    assert!(h.query_by_role_and_label(egui::accesskit::Role::TextInput, "Old file pages").is_some(), "and an Old file pages one");
    let Some(pages) = pages else { return };
    pages.focus();
    pages.type_text("99");
    h.run_steps(2);
    assert_eq!(
        h.query_by_role_and_label(egui::accesskit::Role::TextInput, "New file pages").and_then(|n| n.value()),
        Some("99".to_string()),
        "the typed range is kept"
    );
    h.get_by_label("New file: pages must be within 1-1");
    h.get_by_label("Compare").click();
    h.run_steps(3);
    assert!(h.query_by_label("Compare Files").is_some(), "the dialog stays open for an out-of-range page");
    assert!(h.state().compare.is_none(), "the comparison did not run");
    assert_ne!(h.state().right, Some(RightPanel::Compare));
}

fn four_page_versions() -> (Vec<u8>, Vec<u8>) {
    let s = Session::new();
    let v1 = s.create_from_text("t", "a one\u{c}b two\u{c}c three\u{c}d four").unwrap().to_vec();
    let v2 = s.create_from_text("t", "a one\u{c}b TWO\u{c}c three\u{c}d FOUR").unwrap().to_vec();
    (v1, v2)
}

fn four_page_harness() -> Harness<'static, PdfCraftApp> {
    let (v1, v2) = four_page_versions();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("v1.pdf", None, v1.clone()).unwrap();
        app.open_bytes("v2.pdf", None, v2.clone()).unwrap();
        app
    });
    h.run_steps(4);
    h
}

fn type_into(h: &mut Harness<'_, PdfCraftApp>, label: &str, text: &str) {
    let Some(field) = h.query_by_role_and_label(egui::accesskit::Role::TextInput, label) else { panic!("no {label} field") };
    field.focus();
    field.type_text(text);
    h.run_steps(2);
}

/// The dialog has a page range for each file; the comparison, the panel, Mark as comments and
/// the report all use them, and the pages shown are each file's own.
#[test]
fn compare_dialog_takes_a_page_range_for_each_file() {
    let mut h = four_page_harness();
    assert!(h.state_mut().execute("doc.compare"));
    h.run_steps(2);
    h.get_by_label("Old file pages:");
    h.get_by_label("New file pages:");
    // Old page 4 against new pages 3 to 4.
    type_into(&mut h, "Old file pages", "4");
    type_into(&mut h, "New file pages", "3-4");
    h.get_by_label("Compare").click();
    h.run_steps(3);
    assert_eq!(h.state().right, Some(RightPanel::Compare));
    let c = h.state().compare.as_ref().unwrap();
    assert_eq!(c.old_range.map(|r| (r.from, r.to)), Some((4, 4)));
    assert_eq!(c.new_range.map(|r| (r.from, r.to)), Some((3, 4)));
    let pages: Vec<(usize, usize)> = c.result.changes.iter().map(|x| (x.old.page, x.new.page)).collect();
    assert!(!pages.is_empty() && pages.iter().all(|(o, n)| *o == 3 && (2..=3).contains(n)), "{pages:?}");
    assert!(c.result.changes.iter().all(|x| !x.new.text.contains("TWO")), "page 2 is outside the new range");
    h.get_by_label("Compared pages: old file 4, new file 3-4");
    // Mark as comments: only the new file's pages 3 to 4 get comments.
    let id = h.state().active_ids().unwrap().1;
    h.get_by_label("Mark as comments").click();
    h.run_steps(3);
    let marked = h.state().session.get(id).unwrap().info.annotations.len();
    assert_eq!(marked, h.state().compare.as_ref().unwrap().result.changes.len());
    // The report names the ranges.
    h.get_by_label("Report…").click();
    h.run_steps(3);
    let report = h.state().active_ids().unwrap().1;
    assert_eq!(h.state().session.get(report).unwrap().name, "Compare Report.pdf");
}

/// Ranges that don't fit their own file are refused, naming the file and its page count, and the
/// comparison doesn't run; a range for one file alone and empty fields (the whole files) work.
#[test]
fn compare_dialog_page_range_errors_and_defaults() {
    let mut h = four_page_harness();
    assert!(h.state_mut().execute("doc.compare"));
    h.run_steps(2);
    type_into(&mut h, "Old file pages", "5");
    h.get_by_label("Old file: pages must be within 1-4");
    h.get_by_label("Compare").click();
    h.run_steps(3);
    assert!(h.state().compare.is_none(), "an out-of-range old page does not run the comparison");
    assert!(h.query_by_label("Compare Files").is_some());
    type_into(&mut h, "New file pages", "1,3");
    h.get_by_label("New file: enter one page or one range, such as 3 or 2-5");
    // A range for the new file alone: the same page of the old file is used, so one change.
    h.state_mut().compare_old_pages.clear();
    h.state_mut().compare_pages = "2".into();
    h.run_steps(2);
    h.get_by_label("Compare").click();
    h.run_steps(3);
    let c = h.state().compare.as_ref().unwrap();
    assert_eq!(c.old_range.map(|r| (r.from, r.to)), Some((2, 2)));
    assert_eq!(c.new_range.map(|r| (r.from, r.to)), Some((2, 2)));
    assert_eq!(c.result.changes.len(), 1, "only page 2's change");
    assert!(c.result.changes.iter().all(|x| x.new.page == 1 && x.old.page == 1));
    // Both empty: the whole files, as before.
    h.get_by_label("Clear").click();
    h.run_steps(2);
    h.state_mut().compare_pages.clear();
    h.state_mut().run_compare();
    h.run_steps(2);
    let c = h.state().compare.as_ref().unwrap();
    assert!((c.old_range, c.new_range) == (None, None));
    assert_eq!(c.result.changes.len(), 2);
}

/// A range in one field only applies to both files, so the other file is not compared whole
/// against a part: typing 2 in the New field keeps the changes of page 2, and 4 the changes of
/// page 4, and the same in the Old field.
#[test]
fn compare_with_a_range_in_one_field_uses_the_same_pages_of_the_other_file() {
    let mut h = four_page_harness();
    h.state_mut().compare_old = h.state().views.first().map(|v| v.id);
    for (old, new) in [("", "2"), ("2", ""), ("4", ""), ("", "4")] {
        h.state_mut().compare_old_pages = old.into();
        h.state_mut().compare_pages = new.into();
        h.state_mut().run_compare();
        h.run_steps(2);
        let c = h.state().compare.as_ref().unwrap();
        let page = old.parse::<usize>().or(new.parse()).unwrap();
        assert_eq!(c.old_range, c.new_range, "{old:?} {new:?}");
        assert_eq!(c.result.changes.len(), 1, "{old:?} {new:?}");
        assert!(c.result.changes.iter().all(|x| x.old.page == page - 1 && x.new.page == page - 1), "{old:?} {new:?}");
    }
}

/// The Pages fields start empty for another pair of files.
#[test]
fn compare_dialog_clears_the_pages_fields_for_another_pair_of_files() {
    let mut h = four_page_harness();
    assert!(h.state_mut().execute("doc.compare"));
    h.run_steps(2);
    type_into(&mut h, "New file pages", "2");
    assert_eq!(h.state().compare_pages, "2");
    // The same pair keeps what was typed; another old file starts over.
    h.run_steps(2);
    assert_eq!(h.state().compare_pages, "2");
    h.state_mut().compare_old_pages = "3".into();
    let (v1, _) = four_page_versions();
    h.state_mut().open_bytes("v3.pdf", None, v1).unwrap();
    h.run_steps(3);
    let other =
        h.state().views.iter().map(|v| v.id).find(|id| Some(*id) != h.state().active_ids().map(|a| a.1) && Some(*id) != h.state().compare_old);
    h.state_mut().compare_old = other;
    h.run_steps(3);
    assert!(
        h.state().compare_pages.is_empty() && h.state().compare_old_pages.is_empty(),
        "{:?} {:?}",
        h.state().compare_pages,
        h.state().compare_old_pages
    );
}

/// With ranges starting at different pages, visual differences sit on each file's own page: the
/// old half is shaded on the old range's page, not the new one's.
#[test]
fn compare_with_different_ranges_shades_each_file_on_its_own_page() {
    let mut h = four_page_harness();
    h.state_mut().compare_old = h.state().views.first().map(|v| v.id);
    h.state_mut().compare_old_pages = "4".into();
    h.state_mut().compare_pages = "4".into();
    h.state_mut().run_compare();
    h.run_steps(3);
    let c = h.state().compare.as_ref().unwrap();
    assert!(c.visual.iter().all(|(o, n, _)| (*o, *n) == (3, 3)), "{:?}", c.visual);
    h.state_mut().compare_old_pages = "3".into();
    h.state_mut().run_compare();
    h.run_steps(3);
    let c = h.state().compare.as_ref().unwrap();
    assert!(!c.visual.is_empty() && c.visual.iter().all(|(o, n, _)| (*o, *n) == (2, 3)), "{:?}", c.visual);
    let (old, new) = (c.old, c.new);
    let on = |h: &Harness<'_, PdfCraftApp>, id| {
        let v = h.state().views.iter().find(|v| v.id == id).unwrap();
        v.compare_marks.iter().map(|m| m.0).collect::<std::collections::BTreeSet<_>>()
    };
    assert!(on(&h, old).iter().all(|p| *p == 2 || *p == 3), "{:?}", on(&h, old));
    assert!(on(&h, new).iter().all(|p| *p == 3), "{:?}", on(&h, new));
    assert!(on(&h, old).contains(&2), "the old half is shaded on its own page 3");
}

/// Writes a screenshot of the Compare panel when PDFCRAFT_SHOT is set (for review).
#[test]
fn compare_panel_screenshot() {
    let Ok(out) = std::env::var("PDFCRAFT_SHOT") else { return };
    let _gpu = crate::gpu();
    let s = Session::new();
    let v1 = s.create_from_text("t", "Delivery within five days. Returns accepted.").unwrap().to_vec();
    let v2 = s.create_from_text("t", "Delivery within three days. Returns accepted for a week.").unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("v1.pdf", None, v1.clone()).unwrap();
        app.open_bytes("v2.pdf", None, v2.clone()).unwrap();
        app
    });
    h.run_steps(4);
    h.state_mut().compare_old = h.state().views.first().map(|v| v.id);
    h.state_mut().run_compare();
    for _ in 0..20 {
        h.run_steps(2);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    h.render().unwrap().save(out).unwrap();
}

#[test]
fn pdfa_dialog_verifies_and_converts() {
    let doc = Session::new().create_from_text("t", "Keep forever").unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("keep.pdf", None, doc.clone()).unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("standards.pdfa"));
    h.run_steps(2);
    h.get_by_label("Declared conformance: none");
    assert!(h.query_by_label("Remove PDF/A identification").is_none(), "nothing to remove yet");
    h.get_by_label("Verify").click();
    h.run_steps(3);
    h.get_by_label("The document has no XMP metadata");
    h.get_by_label("Save as PDF/A").click();
    h.run_steps(3);
    h.get_by_label("Declared conformance: PDF/A-2b");
    let issues = h.state().pdfa.issues.clone().unwrap();
    assert!(issues.iter().all(|i| !i.fixable), "{issues:?}");
    let id = h.state().active_ids().unwrap().1;
    assert_eq!(h.state().session.get(id).unwrap().can_undo(), Some("Save as PDF/A-2b"));
    h.get_by_label("Remove PDF/A identification").click();
    h.run_steps(3);
    h.get_by_label("Declared conformance: none");
    assert_eq!(h.state().session.get(id).unwrap().can_undo(), Some("Remove PDF/A identification"));
}

/// A one-page PDF whose XMP declares the given `pdfaid`/`pdfuaid` attributes.
fn declaring(ids: &str) -> Vec<u8> {
    let xmp = format!(
        "<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" \
         xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" xmlns:pdfuaid=\"http://www.aiim.org/pdfua/ns/id/\" {ids}/></rdf:RDF>"
    );
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /Metadata 4 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Resources << >> >>".to_string(),
        format!("<< /Type /Metadata /Subtype /XML /Length {} >>\nstream\n{xmp}\nendstream", xmp.len()),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for o in offsets {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    pdf
}

#[test]
fn pdfa_dialog_shows_a_declared_pdfua() {
    // The dialog read only the PDF/A identification, so a PDF/UA-1 document showed "none" (#1309).
    for (ids, shown) in [
        ("pdfuaid:part=\"1\"", "Declared conformance: PDF/UA-1"),
        ("pdfaid:part=\"2\" pdfaid:conformance=\"B\" pdfuaid:part=\"1\"", "Declared conformance: PDF/A-2b, PDF/UA-1"),
    ] {
        let doc = declaring(ids);
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
            let mut app = PdfCraftApp::new();
            app.set_option("language", "en").unwrap();
            app.open_bytes("ua.pdf", None, doc.clone()).unwrap();
            app
        });
        h.run_steps(4);
        assert!(h.state_mut().execute("standards.pdfa"));
        h.run_steps(2);
        h.get_by_label(shown);
    }
}

#[test]
fn export_to_word_html_and_rtf() {
    let doc = Session::new().create_from_text("t", "Exported words").unwrap().to_vec();
    let dir = std::env::temp_dir().join(format!("pdfcraft-office-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = PdfCraftApp::new();
    app.set_option("language", "en").unwrap();
    app.open_bytes("e.pdf", None, doc).unwrap();
    for ext in ["docx", "pptx", "html", "rtf"] {
        let out = dir.join(format!("e.{ext}"));
        app.save_override = Some(out.to_string_lossy().into_owned());
        assert!(app.execute(&format!("export.{ext}")));
        assert!(std::fs::metadata(&out).unwrap().len() > 40, "{ext}");
    }
    assert!(std::fs::read_to_string(dir.join("e.html")).unwrap().contains("Exported words"));
}
