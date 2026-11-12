//! Preferences ▸ Units and paper and ▸ Comment: each control changes the setting it
//! names at once, the setting survives a restart, and settings that are missing, mistyped or
//! corrupt leave the current choice alone.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;
use pdfcraft_ui_egui::comments::CommentTool;
use pdfcraft_ui_egui::paper::Paper;
use pdfcraft_ui_egui::units::Unit;

fn preferences() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.set_option("dialog", "preferences").unwrap();
        app
    });
    h.run_steps(6);
    h
}

fn restarted(h: &Harness<'static, PdfCraftApp>) -> PdfCraftApp {
    let mut app = PdfCraftApp::new();
    app.set_option("language", "en").unwrap();
    app.restore(&h.state().persist());
    app
}

#[test]
fn the_paper_for_new_blank_pages_is_chosen_applied_and_remembered() {
    let mut h = preferences();
    h.state_mut().blank_paper = Paper::Letter;
    h.get_by_label("A4").scroll_to_me();
    h.run_steps(3);
    h.get_by_label("A4").click();
    h.run_steps(3);
    assert_eq!(h.state().blank_paper, Paper::A4);
    // Applied at once: the next blank page is on A4.
    h.state_mut().set_option("dialog", "none").unwrap();
    assert!(h.state_mut().execute("create.blank"));
    let docs = h.state().session.docs();
    let crop = docs.last().and_then(|d| d.info.pages.first()).map(|p| p.crop).unwrap();
    let (w, hgt) = Paper::A4.points();
    assert!(((crop[2] - crop[0]) as f64 - w).abs() < 0.01 && ((crop[3] - crop[1]) as f64 - hgt).abs() < 0.01, "{crop:?}");
    // Remembered: a fresh app that starts on the other paper takes the saved one, whichever the
    // region's default is, and the choice back to Letter is remembered too.
    let restarted_from = |h: &Harness<'static, PdfCraftApp>, start: Paper| {
        let mut app = PdfCraftApp::new();
        app.blank_paper = start;
        app.restore(&h.state().persist());
        app.blank_paper
    };
    assert_eq!(restarted_from(&h, Paper::Letter), Paper::A4);
    h.state_mut().blank_paper = Paper::Letter;
    assert_eq!(restarted_from(&h, Paper::A4), Paper::Letter);
}

#[test]
fn the_unit_is_chosen_in_units_and_paper_and_remembered() {
    let mut h = preferences();
    // The pill lives in the Units and paper card, between its title and the paper row.
    let card = h.get_by_label("Units and paper").rect();
    let next = h.get_by_label("Paper for new blank pages").rect();
    let mm = h.get_by_label("mm").rect();
    assert!(card.min.y <= mm.min.y && mm.max.y <= next.min.y, "{card:?} {mm:?} {next:?}");
    h.state_mut().units = Unit::Inches;
    h.get_by_label("mm").scroll_to_me();
    h.run_steps(3);
    h.get_by_label("mm").click();
    h.run_steps(3);
    assert_eq!(h.state().units, Unit::Millimetres);
    assert_eq!(restarted(&h).units, Unit::Millimetres);
}

#[test]
fn corrupt_or_unknown_saved_paper_and_units_keep_the_current_choice() {
    let mut app = PdfCraftApp::new();
    app.blank_paper = Paper::A4;
    app.units = Unit::Points;
    for json in [
        "",
        "not json",
        "[]",
        "null",
        r#"{"blank_paper": null, "units": null}"#,
        r#"{"blank_paper": 7, "units": 7}"#,
        r#"{"blank_paper": "legal", "units": "furlongs"}"#,
        r#"{"blank_paper": ["a4"], "units": {"in": 1}}"#,
        r#"{"blank_paper": "letter\u0000", "units": ""}"#,
    ] {
        app.restore(json);
        assert_eq!((app.blank_paper, app.units), (Paper::A4, Unit::Points), "{json:?}");
    }
    app.restore(r#"{"blank_paper": "letter", "units": "centimetres"}"#);
    assert_eq!((app.blank_paper, app.units), (Paper::Letter, Unit::Centimetres));
}

#[test]
fn the_control_channel_sets_and_refuses_units_and_paper() {
    let mut app = PdfCraftApp::new();
    app.set_option("blank-paper", "A4").unwrap();
    app.set_option("units", "mm").unwrap();
    assert_eq!((app.blank_paper, app.units), (Paper::A4, Unit::Millimetres));
    for (key, bad) in [("blank-paper", "legal"), ("blank-paper", ""), ("units", "feet"), ("units", ""), ("reset-comment-defaults", "false")] {
        assert!(app.set_option(key, bad).is_err(), "{key}={bad}");
    }
    assert_eq!((app.blank_paper, app.units), (Paper::A4, Unit::Millimetres), "a refused value changes nothing");
}

#[test]
fn the_comment_tool_pin_is_set_in_preferences_and_shared_with_the_toolbar_setting() {
    let mut h = preferences();
    assert!(!h.state().comment_prefs.pinned);
    h.get_by_label("Keep the selected comment tool selected after use").scroll_to_me();
    h.run_steps(3);
    h.get_by_label("Keep the selected comment tool selected after use").click();
    h.run_steps(3);
    assert!(h.state().comment_prefs.pinned);
    assert!(restarted(&h).comment_prefs.pinned);
    // The control channel's pin and the checkbox are one setting.
    h.state_mut().set_option("comment-tool-pinned", "false").unwrap();
    h.run_steps(3);
    assert!(!h.state().comment_prefs.pinned);
}

#[test]
fn reset_comment_tool_defaults_forgets_saved_colours_and_keeps_the_name_and_pin() {
    let mut h = preferences();
    let original = h.state().comment_prefs.style(CommentTool::Highlight).color;
    h.state_mut().comment_prefs.set_color(CommentTool::Highlight, [0.0, 0.0, 1.0]);
    h.state_mut().comment_prefs.set_opacity(CommentTool::Highlight, 0.25);
    h.state_mut().comment_prefs.author = "Grace Hopper".into();
    h.state_mut().comment_prefs.pinned = true;
    assert_eq!(restarted(&h).comment_prefs.style(CommentTool::Highlight).color, [0.0, 0.0, 1.0], "the saved default survives a restart");
    h.get_by_label("Reset comment tool colours and sizes").scroll_to_me();
    h.run_steps(3);
    h.get_by_label("Reset comment tool colours and sizes").click();
    h.run_steps(3);
    let prefs = &h.state().comment_prefs;
    assert_eq!(prefs.style(CommentTool::Highlight).color, original);
    assert_ne!(prefs.style(CommentTool::Highlight).opacity, 0.25);
    assert_eq!((prefs.author.as_str(), prefs.pinned), ("Grace Hopper", true));
    assert_eq!(restarted(&h).comment_prefs.style(CommentTool::Highlight).color, original);
}
