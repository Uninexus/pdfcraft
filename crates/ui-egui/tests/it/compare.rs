//! Compare files in the real shell (egui_kittest): the dialog, the Compare panel, marks and
//! the report.

use crate::support::AppHarness as _;
use craft::ui::dialog::dialog_id;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use pdfcraft_engine::Session;
use pdfcraft_ui_egui::{PdfCraftApp, RightPanel};

#[test]
fn compare_two_versions() {
    let s = Session::new();
    let v1 = s.create_from_text("t", "Delivery within five days. Returns accepted.").unwrap().to_vec();
    let v2 = s.create_from_text("t", "Delivery within three days. Returns accepted for a week.").unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "v1.pdf", None, v1.clone()).unwrap();
            app.open_bytes(&egui::Context::default(), "v2.pdf", None, v2.clone()).unwrap();
            app
        }
    });
    h.run_steps(4);
    assert!(h.execute("doc.compare"));
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::Dialog, "Compare Files");
    h.get_by_label("Compare").click();
    h.run_steps(3);
    assert_eq!(crate::support::right(h.state()), Some(RightPanel::Compare));
    assert_eq!(crate::support::shown_notice(&h).as_deref(), Some("2 differences found"));
    h.get_by_label("2 Replaced");
    h.get_by_label_contains("\"five\" → \"three\"").click();
    h.run_steps(2);
    assert_eq!(h.state().compare.as_ref().unwrap().selected, Some(0));
    let id = h.state().active_ids().unwrap().1;
    let i = h.state().active_ids().unwrap().0;
    assert_eq!(h.state().views[i].session.compare_marks.len(), 2);
    // Side by side: the old file's half shades its side of the changes and follows the new
    // file's zoom; "New file only" ends it.
    assert!(h.state().compare.as_ref().unwrap().side_by_side, "side by side on a wide window");
    let old_i = h.state().views.iter().position(|v| v.id == h.state().compare.as_ref().unwrap().old).unwrap();
    assert_eq!(h.state().views[old_i].session.compare_marks.len(), 2, "replaced words shaded on the old file too");
    assert!(h.state().views[old_i].session.flash.is_some(), "the old half jumped to the change");
    h.state_mut().views[i].view.zoom = 1.5;
    h.state_mut().views[i].view.fit = craft::pages::Fit::None;
    h.run_steps(3);
    assert_eq!((h.state().views[old_i].view.zoom, h.state().views[old_i].view.fit), (1.5, craft::pages::Fit::None), "the old half follows");
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
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "v1.pdf", None, v1.clone()).unwrap();
            app.open_bytes(&egui::Context::default(), "v2.pdf", None, v2.clone()).unwrap();
            app
        }
    });
    h.run_steps(4);
    assert!(h.execute("doc.compare"));
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
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "v1.pdf", None, v1.clone()).unwrap();
            app.open_bytes(&egui::Context::default(), "v2.pdf", None, v2.clone()).unwrap();
            app
        }
    });
    h.run_steps(4);
    h.state_mut().compare_old = h.state().views.first().map(|v| v.id);
    let ctx = h.ctx.clone();
    h.state_mut().run_compare(&ctx);
    h.run_steps(3);
    let new_i = h.state().active_ids().unwrap().0;
    let old_i = h.state().views.iter().position(|v| v.id == h.state().compare.as_ref().unwrap().old).unwrap();
    // Visual differences are shaded orange (compare_ui::VISUAL), text changes in their kind's colour.
    let visual = egui::Color32::from_rgb(0xF0, 0x8C, 0x1A);
    let kinds = |h: &Harness<'_, PdfCraftApp>, i: usize| {
        let marks = &h.state().views[i].session.compare_marks;
        (marks.iter().filter(|m| m.2 != visual).count(), marks.iter().filter(|m| m.2 == visual).count())
    };
    let (text_new, visual_new) = kinds(&h, new_i);
    let (text_old, visual_old) = kinds(&h, old_i);
    assert!(text_new > 0 && visual_new > 0, "the fixture has both kinds on the new file: {text_new} text, {visual_new} visual");
    assert!(text_old > 0 && visual_old > 0, "and on the old file: {text_old} text, {visual_old} visual");
    h.get_by_label_contains("\"five\" → \"three\"");

    h.get_by_label("Replaced").click();
    h.run_steps(2);
    assert!(h.query_by_label_contains("\"five\" → \"three\"").is_none(), "replaced changes leave the list");
    assert_eq!(kinds(&h, new_i), (0, visual_new), "and the new file's shading");
    assert_eq!(kinds(&h, old_i), (0, visual_old), "and the old file's");

    h.get_by_label("Visual").click();
    h.run_steps(2);
    assert_eq!(kinds(&h, new_i), (0, 0));
    assert_eq!(kinds(&h, old_i), (0, 0));

    h.get_by_label("Replaced").click();
    h.run_steps(2);
    h.get_by_label_contains("\"five\" → \"three\"");
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
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "v1.pdf", None, v1.clone()).unwrap();
            app.open_bytes(&egui::Context::default(), "v2.pdf", None, v2.clone()).unwrap();
            app
        }
    });
    h.run_steps(4);
    assert!(h.execute("doc.compare"));
    h.run_steps(2);
    assert!(shows(&h, "Kinds of change"));
    // Nothing ticked: Compare is off, so no empty-kinds error is possible.
    for key in ["kind_replaced", "kind_inserted", "kind_deleted", "kind_visual"] {
        set_field(&mut h, key, serde_json::json!(false));
    }
    assert!(compare_is_off(&h), "Compare waits for a kind of change");
    // Without Inserted and Deleted: only the replacement is compared and listed.
    set_field(&mut h, "kind_replaced", serde_json::json!(true));
    set_field(&mut h, "kind_visual", serde_json::json!(true));
    assert!(!compare_is_off(&h));
    h.get_by_label("Compare").click();
    h.run_steps(3);
    let k = h.state().compare.as_ref().unwrap().kinds;
    assert_eq!((k.replaced, k.inserted, k.deleted, k.visual), (true, false, false, true));
    assert_eq!(h.state().compare_kinds, k, "the next dialog opens with the same kinds");
    h.get_by_label_contains("\"900\" → \"950\"");
    assert!(h.query_by_label_contains("+ \"Parking included").is_none(), "the insertion is not listed");
    let id = h.state().active_ids().unwrap().1;
    h.get_by_label("Mark as comments").click();
    h.run_steps(3);
    assert_eq!(h.state().session.get(id).unwrap().info.annotations.len(), 1, "only the replacement is marked");
    // Nothing ticked in the panel: Mark as comments and Report… are off.
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
    assert_eq!(crate::support::shown_notice(&h).as_deref(), Some("No comments added: visual differences are shaded, not marked as comments"));
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

fn four_page_versions() -> (Vec<u8>, Vec<u8>) {
    let s = Session::new();
    let v1 = s.create_from_text("t", "a one\u{c}b two\u{c}c three\u{c}d four").unwrap().to_vec();
    let v2 = s.create_from_text("t", "a one\u{c}b TWO\u{c}c three\u{c}d FOUR").unwrap().to_vec();
    (v1, v2)
}

fn four_page_harness() -> Harness<'static, PdfCraftApp> {
    let (v1, v2) = four_page_versions();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "v1.pdf", None, v1.clone()).unwrap();
            app.open_bytes(&egui::Context::default(), "v2.pdf", None, v2.clone()).unwrap();
            app
        }
    });
    h.run_steps(4);
    h
}

/// Sets a row of the open Compare Files dialog as `ui.dialog.set` does.
fn set_field(h: &mut Harness<'_, PdfCraftApp>, field: &str, value: serde_json::Value) {
    let ctx = h.ctx.clone();
    h.state_mut().dialogs.set_json_field(&ctx, dialog_id("compare"), field, value).unwrap_or_else(|e| panic!("{field}: {e}"));
    h.run_steps(2);
}

/// What a row of the open Compare Files dialog holds.
fn field_text(h: &Harness<'_, PdfCraftApp>, field: &str) -> String {
    let dialog = h.state().dialogs.get_by_id::<HostedParamDialog>(dialog_id("compare")).expect("the Compare Files dialog is open");
    dialog.fields().text(field).unwrap_or_default().to_owned()
}

/// What a checkbox row of the open Compare Files dialog holds.
fn field_flag(h: &Harness<'_, PdfCraftApp>, field: &str) -> Option<bool> {
    let dialog = h.state().dialogs.get_by_id::<HostedParamDialog>(dialog_id("compare")).expect("the Compare Files dialog is open");
    dialog.fields().bool(field)
}

/// Whether a label with this text is on screen.
fn shows(h: &Harness<'_, PdfCraftApp>, text: &str) -> bool {
    h.query_all_by_label_contains(text).next().is_some()
}

/// Whether the dialog's Compare button is off.
fn compare_is_off(h: &Harness<'_, PdfCraftApp>) -> bool {
    h.query_by(|n| n.label().as_deref() == Some("Compare") && n.is_disabled()).is_some()
}

/// The dialog has a page range for each file; the comparison, the panel, Mark as comments and
/// the report all use them, and the pages shown are each file's own.
#[test]
fn compare_dialog_takes_a_page_range_for_each_file() {
    let mut h = four_page_harness();
    assert!(h.execute("doc.compare"));
    h.run_steps(2);
    assert!(shows(&h, "Old file pages:"));
    assert!(shows(&h, "New file pages:"));
    // Old page 4 against new pages 3 to 4.
    set_field(&mut h, "old_pages", serde_json::json!("4"));
    set_field(&mut h, "pages", serde_json::json!("3-4"));
    h.get_by_label("Compare").click();
    h.run_steps(3);
    assert_eq!(crate::support::right(h.state()), Some(RightPanel::Compare));
    let c = h.state().compare.as_ref().unwrap();
    assert_eq!(c.old_range.map(|r| (r.from, r.to)), Some((4, 4)));
    assert_eq!(c.new_range.map(|r| (r.from, r.to)), Some((3, 4)));
    let pages: Vec<(usize, usize)> = c.result.changes.iter().map(|x| (x.old.page, x.new.page)).collect();
    assert!(!pages.is_empty() && pages.iter().all(|(o, n)| *o == 3 && (2..=3).contains(n)), "{pages:?}");
    assert!(c.result.changes.iter().all(|x| !x.new.text.contains("TWO")), "page 2 is outside the new range");
    assert!(shows(&h, "Compared pages: old file 4, new file 3-4"));
    // Mark as comments: only the new file's pages 3 to 4 get comments.
    let id = h.state().active_ids().unwrap().1;
    h.get_by_label("Mark as comments").click();
    h.run_steps(3);
    let marked = h.state().session.get(id).unwrap().info.annotations.len();
    assert_eq!(marked, h.state().compare.as_ref().unwrap().result.changes.len());
    // The report is a new document.
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
    assert!(h.execute("doc.compare"));
    h.run_steps(2);
    assert!(!compare_is_off(&h), "empty fields compare the whole files");
    set_field(&mut h, "old_pages", serde_json::json!("5"));
    assert!(shows(&h, "Old file: pages must be within 1-4"));
    assert!(compare_is_off(&h), "an out-of-range old page turns Compare off");
    assert!(h.state().compare.is_none(), "and the comparison does not run");
    set_field(&mut h, "pages", serde_json::json!("1,3"));
    assert!(shows(&h, "New file: enter one page or one range, such as 3 or 2-5"));
    // A range for the new file alone: the same page of the old file is used, so one change.
    set_field(&mut h, "old_pages", serde_json::json!(""));
    set_field(&mut h, "pages", serde_json::json!("2"));
    assert!(!compare_is_off(&h));
    h.get_by_label("Compare").click();
    h.run_steps(3);
    let c = h.state().compare.as_ref().unwrap();
    assert_eq!(c.old_range.map(|r| (r.from, r.to)), Some((2, 2)));
    assert_eq!(c.new_range.map(|r| (r.from, r.to)), Some((2, 2)));
    assert_eq!(c.result.changes.len(), 1, "only page 2's change");
    assert!(c.result.changes.iter().all(|x| x.new.page == 1 && x.old.page == 1));
    // Both empty: the whole files.
    h.get_by_label("Clear").click();
    h.run_steps(2);
    h.state_mut().compare_pages.clear();
    h.state_mut().compare_old_pages.clear();
    let ctx = h.ctx.clone();
    h.state_mut().run_compare(&ctx);
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
    let ctx = h.ctx.clone();
    for (old, new) in [("", "2"), ("2", ""), ("4", ""), ("", "4")] {
        h.state_mut().compare_old_pages = old.into();
        h.state_mut().compare_pages = new.into();
        h.state_mut().run_compare(&ctx);
        h.run_steps(2);
        let c = h.state().compare.as_ref().unwrap();
        let page = old.parse::<usize>().or(new.parse()).unwrap();
        assert_eq!(c.old_range, c.new_range, "{old:?} {new:?}");
        assert_eq!(c.result.changes.len(), 1, "{old:?} {new:?}");
        assert!(c.result.changes.iter().all(|x| x.old.page == page - 1 && x.new.page == page - 1), "{old:?} {new:?}");
    }
}

/// The dialog opens with the Pages fields as they were typed for the same pair of files, and empty
/// for another pair.
#[test]
fn compare_dialog_clears_the_pages_fields_for_another_pair_of_files() {
    let mut h = four_page_harness();
    assert!(h.execute("doc.compare"));
    h.run_steps(2);
    set_field(&mut h, "pages", serde_json::json!("2"));
    set_field(&mut h, "old_pages", serde_json::json!("3"));
    h.get_by_label("Compare").click();
    h.run_steps(3);
    let (new, old) = (h.state().active_ids().unwrap().1, h.state().compare_old);
    assert_eq!(h.state().compare_fields_for, Some((new, old)));
    // The same pair keeps what was typed.
    h.get_by_label("Clear").click();
    h.run_steps(2);
    assert!(h.execute("doc.compare"));
    h.run_steps(2);
    assert_eq!((field_text(&h, "pages"), field_text(&h, "old_pages")), ("2".to_string(), "3".to_string()));
    // Another old file starts over.
    let (v1, _) = four_page_versions();
    h.state_mut().open_bytes(&egui::Context::default(), "v3.pdf", None, v1).unwrap();
    h.run_steps(3);
    assert!(h.execute("doc.compare"));
    h.run_steps(2);
    assert_ne!(h.state().active_ids().unwrap().1, new, "v3 is the new file now");
    assert!(field_text(&h, "pages").is_empty() && field_text(&h, "old_pages").is_empty());
}

/// With ranges starting at different pages, visual differences sit on each file's own page: the
/// old half is shaded on the old range's page, not the new one's.
#[test]
fn compare_with_different_ranges_shades_each_file_on_its_own_page() {
    let mut h = four_page_harness();
    h.state_mut().compare_old = h.state().views.first().map(|v| v.id);
    let ctx = h.ctx.clone();
    h.state_mut().compare_old_pages = "4".into();
    h.state_mut().compare_pages = "4".into();
    h.state_mut().run_compare(&ctx);
    h.run_steps(3);
    let c = h.state().compare.as_ref().unwrap();
    assert!(c.visual.iter().all(|(o, n, _)| (*o, *n) == (3, 3)), "{:?}", c.visual);
    h.state_mut().compare_old_pages = "3".into();
    h.state_mut().run_compare(&ctx);
    h.run_steps(3);
    let c = h.state().compare.as_ref().unwrap();
    assert!(!c.visual.is_empty() && c.visual.iter().all(|(o, n, _)| (*o, *n) == (2, 3)), "{:?}", c.visual);
    let (old, new) = (c.old, c.new);
    let on = |h: &Harness<'_, PdfCraftApp>, id| {
        let v = h.state().views.iter().find(|v| v.id == id).unwrap();
        v.session.compare_marks.iter().map(|m| m.0).collect::<std::collections::BTreeSet<_>>()
    };
    assert!(on(&h, old).iter().all(|p| *p == 2 || *p == 3), "{:?}", on(&h, old));
    assert!(on(&h, new).iter().all(|p| *p == 3), "{:?}", on(&h, new));
    assert!(on(&h, old).contains(&2), "the old half is shaded on its own page 3");
}

/// The dialog's Ignore checkboxes are the `ignore_case` and `ignore_punctuation` options of the
/// `doc_compare` tool: with neither, a changed capital and a changed full stop are two text
/// differences; each box leaves out its own kind and not the other's, and both leave the real
/// word change (and nothing else) in the list. Mark as comments and the report use the same ones.
#[test]
fn compare_dialog_ignore_checkboxes_leave_out_case_and_punctuation_changes() {
    let s = Session::new();
    let v1 = s.create_from_text("t", "Delivery within five days. Returns accepted").unwrap().to_vec();
    let v2 = s.create_from_text("t", "delivery within three days. Returns, accepted").unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "v1.pdf", None, v1.clone()).unwrap();
            app.open_bytes(&egui::Context::default(), "v2.pdf", None, v2.clone()).unwrap();
            app
        }
    });
    h.run_steps(4);
    h.state_mut().compare_old = h.state().views.first().map(|v| v.id);
    let ctx = h.ctx.clone();
    let count = |h: &mut Harness<'_, PdfCraftApp>| {
        h.state_mut().run_compare(&ctx);
        h.run_steps(2);
        h.state().compare.as_ref().unwrap().result.changes.len()
    };
    assert_eq!(count(&mut h), 3, "case, word and punctuation");
    h.get_by_label("Clear").click();
    h.run_steps(2);
    // Through the dialog: Letter case ignored, punctuation kept.
    assert!(h.execute("doc.compare"));
    h.run_steps(2);
    assert!(shows(&h, "Letter case") && shows(&h, "Punctuation"));
    assert_eq!(field_flag(&h, "ignore_case"), Some(false), "both start off");
    set_field(&mut h, "ignore_case", serde_json::json!(true));
    h.get_by_label("Compare").click();
    h.run_steps(3);
    assert!(h.state().compare_ignore_case && !h.state().compare_ignore_punctuation);
    assert_eq!(h.state().compare.as_ref().unwrap().result.changes.len(), 2, "capital ignored, punctuation kept");
    // The next dialog opens with the same choice.
    h.get_by_label("Clear").click();
    h.run_steps(2);
    assert!(h.execute("doc.compare"));
    h.run_steps(2);
    assert_eq!((field_flag(&h, "ignore_case"), field_flag(&h, "ignore_punctuation")), (Some(true), Some(false)));
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    h.state_mut().compare_ignore_case = false;
    h.state_mut().compare_ignore_punctuation = true;
    assert_eq!(count(&mut h), 2, "punctuation ignored, capital kept");
    h.state_mut().compare_ignore_case = true;
    assert_eq!(count(&mut h), 1, "only five -> three");
    // Mark as comments and Report... use the same options as the list: one comment, and a report
    // that names what was ignored and lists only the real change.
    assert!(h.state().compare.as_ref().unwrap().options.ignore_case);
    let new = h.state().active_ids().unwrap().1;
    h.get_by_label("Mark as comments").click();
    h.run_steps(3);
    assert_eq!(h.state().session.get(new).unwrap().info.annotations.len(), 1, "only five -> three is marked");
    h.get_by_label("Report…").click();
    h.run_steps(3);
    let report = h.state().active_ids().unwrap().1;
    let text: Vec<String> = h.state().session.get(report).unwrap().words().into_iter().map(|w| w.text).collect();
    let text = text.join(" ");
    assert!(text.contains("Ignored in the text comparison: letter case, sentence punctuation"), "{text}");
    assert!(text.contains("five") && !text.contains("Returns,"), "{text}");
}

#[test]
fn pdfa_dialog_verifies_and_converts() {
    let doc = Session::new().create_from_text("t", "Keep forever").unwrap().to_vec();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "keep.pdf", None, doc.clone()).unwrap();
            app
        }
    });
    h.run_steps(4);
    assert!(h.execute("standards.pdfa"));
    h.run_steps(2);
    h.get_by_label("Declared conformance: none");
    assert!(h.query_by_label("Remove PDF/A identification").is_none(), "nothing to remove yet");
    h.get_by_label("Verify").click();
    h.run_steps(3);
    // A finding's row is named by its text, then its clause and severity.
    assert!(h.query_all_by_label_contains("The document has no XMP metadata").next().is_some(), "the missing XMP is a finding");
    h.get_by_label("Save as PDF/A").click();
    h.run_steps(3);
    h.get_by_label("Declared conformance: PDF/A-2b");
    // The issues are the dialog's findings: the one left over is shown as not fixable here.
    assert!(!h.query_all_by_label_contains("not fixable here").collect::<Vec<_>>().is_empty(), "the remaining finding says it is not fixable");
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
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
            crate::support::install_runtime(&cc.egui_ctx);
            {
                let mut app = crate::control::new_app();
                app.set_option(&egui::Context::default(), "language", "en").unwrap();
                app.open_bytes(&egui::Context::default(), "ua.pdf", None, doc.clone()).unwrap();
                app
            }
        });
        h.run_steps(4);
        assert!(h.execute("standards.pdfa"));
        h.run_steps(2);
        h.get_by_label(shown);
    }
}

#[test]
fn export_to_word_html_and_rtf() {
    let doc = Session::new().create_from_text("t", "Exported words").unwrap().to_vec();
    for ext in ["docx", "pptx", "html", "rtf"] {
        // Each save is answered with the suggested name in the suggested folder.
        let files = std::sync::Arc::new(craft::host::ScriptedFiles::new().saving_suggested());
        let (app_files, doc) = (files.clone(), doc.clone());
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
            crate::support::install_runtime(&cc.egui_ctx);
            let mut app = crate::control::app_with_files(app_files);
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "e.pdf", None, doc).unwrap();
            app
        });
        h.run_steps(4);
        assert!(h.execute(&format!("export.{ext}")));
        // The picker's answer is taken on a later frame.
        h.run_steps(3);
        let request = files.requests().pop().expect("the export asked where to save");
        let path = request.directory.or(request.default_directory).unwrap_or_default().join(request.file_name.expect("a suggested name"));
        let bytes = files.file(&path).unwrap_or_else(|| panic!("{ext} was written"));
        assert!(bytes.len() > 40, "{ext}");
        if ext == "html" {
            assert!(String::from_utf8_lossy(&bytes).contains("Exported words"));
        }
    }
}
