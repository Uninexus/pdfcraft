//! Print production ▸ Print readiness: the report opens from the command, lists what the ink check found, shows what
//! was measured, and changes nothing.

use crate::support::AppHarness as _;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

fn with_ink() -> Vec<u8> {
    let mut doc = pdfcraft_cos::Document::open(std::sync::Arc::new(include_bytes!("../data/form.pdf").to_vec())).unwrap();
    let page = pdfcraft_model::pages(&doc).into_iter().next().unwrap();
    // 330% total ink.
    let ink = doc.add(pdfcraft_cos::Object::Stream(pdfcraft_cos::Stream::flate(pdfcraft_cos::Dict::new(), b"0.9 0.8 0.7 0.9 k 10 10 100 100 re f")));
    doc.update_dict(page.obj, |d| d.set(b"Contents".to_vec(), pdfcraft_cos::Object::Ref(ink))).unwrap();
    pdfcraft_cos::write_full(&doc, &Default::default()).unwrap()
}

/// The app with `bytes` open, or with no document for `None`.
fn harness(bytes: Option<Vec<u8>>) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            if let Some(bytes) = &bytes {
                app.open_bytes(&egui::Context::default(), "ink.pdf", None, bytes.clone()).unwrap();
            }
            app
        }
    });
    h.run_steps(4);
    h
}

/// Presses the dialog's Close.
fn close(h: &mut Harness<'static, PdfCraftApp>) {
    h.query_all_by_role_and_label(Role::Button, "Close").last().expect("the dialog's Close").click();
    h.run_steps(3);
}

#[test]
fn the_report_lists_the_ink_findings_and_changes_nothing() {
    let mut h = harness(Some(with_ink()));
    let id = h.state().views[0].id;
    let generation = h.state().session.get(id).unwrap().edit_generation();
    assert!(h.execute("prepress.readiness"));
    h.run_steps(4);
    assert!(h.state().readiness_open);
    assert!(h.state().readiness_report.as_ref().is_some_and(|r| r.findings.iter().any(|f| f.id == "ink-total")));
    // The finding is on screen, in words, with its severity.
    assert!(h.query_all_by_label_contains("total ink").count() > 0, "the finding is shown");
    assert!(h.query_all_by_label_contains("Warning").count() > 0, "with its severity as a word");
    close(&mut h);
    assert!(!h.state().readiness_open);
    assert!(h.state().readiness_report.is_none(), "the report is dropped with the dialog");
    assert_eq!(h.state().session.get(id).unwrap().edit_generation(), generation, "nothing was edited");
    assert_eq!(h.state().session.get(id).unwrap().can_undo(), None);
}

#[test]
fn the_dialog_shows_what_was_measured_and_the_pdfa_result() {
    let mut h = harness(Some(include_bytes!("../data/form.pdf").to_vec()));
    assert!(h.execute("prepress.readiness"));
    h.run_steps(4);
    let report = h.state().readiness_report.clone().expect("a report");
    // The PDF/A-2b result is a line of the report, shown whether or not it found a problem.
    assert!(report.details().contains("PDF/A-2b"));
    assert!(h.state().readiness_report_text().is_some_and(|t| t.contains("PDF/A-2b") && t.contains("Not checked")));
    assert!(h.query_all_by_label_contains("Not checked").count() > 0, "what isn't checked is shown");
}

#[test]
fn print_readiness_needs_a_document() {
    let mut h = harness(None);
    assert!(!h.execute("prepress.readiness"), "disabled without a document");
    assert!(!h.state().readiness_open);
}

/// One page with a 200 ppi image, 330% total ink, and a TrimBox 10 pt (3.5 mm) inside the bleed: by default only the
/// ink and the resolution are findings.
fn all_three() -> Vec<u8> {
    let content = "0.9 0.8 0.7 0.9 k 20 20 50 50 re f q 0.72 0 0 0.72 100 100 cm /Im Do Q";
    format!(
        "%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /BleedBox [0 0 200 200] /TrimBox [10 10 190 190] /Resources << /XObject << /Im 5 0 R >> >> /Contents 4 0 R >> endobj
4 0 obj << /Length {} >> stream
{content}
endstream endobj
5 0 obj << /Type /XObject /Subtype /Image /Width 2 /Height 2 /BitsPerComponent 8 /ColorSpace /DeviceGray /Length 4 >> stream
AAAA
endstream endobj
trailer << /Root 1 0 R >>
%%EOF",
        content.len()
    )
    .into_bytes()
}

/// An app with the report open on that document, without a window.
fn open_report() -> PdfCraftApp {
    let ctx = egui::Context::default();
    let mut app = crate::control::new_app();
    app.set_option(&ctx, "language", "en").unwrap();
    app.open_bytes(&ctx, "all.pdf", None, all_three()).unwrap();
    assert!(app.execute(&ctx, "prepress.readiness"));
    assert!(app.readiness_open);
    app
}

fn ids(app: &PdfCraftApp, id: &str) -> Vec<(String, String)> {
    let report = app.readiness_report.as_ref().expect("a report");
    report.findings.iter().filter(|f| f.id == id).map(|f| (f.severity.to_string(), f.message.clone())).collect()
}

/// What the engine says for the same thresholds: the dialog must show exactly this.
fn engine(app: &PdfCraftApp, ink: Option<f64>, bleed: Option<f64>, ppi: Option<f64>) -> pdfcraft_engine::printcheck::PrintReadiness {
    let id = app.views[0].id;
    app.session.get(id).unwrap().print_readiness_with_options(ink, bleed, ppi).unwrap()
}

fn set(app: &mut PdfCraftApp, key: &str, value: &str) -> Result<(), String> {
    app.set_option(&egui::Context::default(), key, value)
}

#[test]
fn the_dialog_measures_with_the_default_thresholds_until_one_is_set() {
    let app = open_report();
    let id = app.views[0].id;
    let plain = app.session.get(id).unwrap().print_readiness();
    assert!(app.readiness_report.as_ref() == Some(&plain), "the default report is the tool's report without options");
    assert!(ids(&app, "ink-total").len() == 1 && ids(&app, "page-bleed").is_empty(), "{:?}", app.readiness_report);
    assert!(ids(&app, "image-ppi")[0].1.contains("close-up"), "{:?}", ids(&app, "image-ppi"));
    assert!(app.readiness_report_text().unwrap() == plain.text(), "Copy report is the engine's text with no options");
}

#[test]
fn every_threshold_changes_the_findings_the_measured_lines_and_copy_report() {
    let mut app = open_report();
    set(&mut app, "readiness-ink-limit", "350").unwrap();
    assert!(ids(&app, "ink-total").is_empty(), "330% is under 350");
    assert!(app.readiness_report.as_ref() == Some(&engine(&app, Some(350.0), None, None)));
    set(&mut app, "readiness-ink-limit", "250").unwrap();
    assert!(ids(&app, "ink-total")[0].1.contains("250"), "{:?}", ids(&app, "ink-total"));
    set(&mut app, "readiness-min-bleed-mm", "5").unwrap();
    assert!(ids(&app, "page-bleed")[0].1.contains("5 mm was asked for"), "{:?}", ids(&app, "page-bleed"));
    set(&mut app, "readiness-min-ppi", "600").unwrap();
    assert!(ids(&app, "image-ppi")[0].1.contains("600 ppi was asked for"), "{:?}", ids(&app, "image-ppi"));
    assert!(app.readiness_report.as_ref() == Some(&engine(&app, Some(250.0), Some(5.0), Some(600.0))));
    let text = app.readiness_report_text().unwrap();
    assert!(text.contains("total ink limit 250%") && text.contains("least bleed 5 mm") && text.contains("least image resolution 600 ppi"), "{text}");
    assert!(text.contains("600 ppi was asked for"), "Copy report carries the findings of the changed thresholds: {text}");
    set(&mut app, "readiness-min-ppi", "150").unwrap();
    assert!(ids(&app, "image-ppi").is_empty(), "200 ppi passes 150");
    // Back to the defaults: the same report as before anything was set.
    for key in ["readiness-ink-limit", "readiness-min-bleed-mm", "readiness-min-ppi"] {
        set(&mut app, key, "default").unwrap();
    }
    assert!(app.readiness_report.as_ref() == Some(&engine(&app, None, None, None)));
    assert!(app.readiness_options.is_default());
}

#[test]
fn refused_thresholds_change_nothing_and_say_what_is_allowed() {
    let mut app = open_report();
    let before = app.readiness_report.clone();
    let cases = [
        ("readiness-ink-limit", ["99.9", "400.1", "NaN", "inf", "-1", "abc", "", "1e400"].as_slice(), "400"),
        ("readiness-min-bleed-mm", ["-0.1", "50.1", "NaN", "-inf", "two", ""].as_slice(), "50"),
        ("readiness-min-ppi", ["35.9", "1200.1", "NaN", "0", "x", ""].as_slice(), "1200"),
    ];
    for (key, bad, top) in cases {
        for value in bad {
            let e = set(&mut app, key, value).expect_err(&format!("{key}={value:?} is refused"));
            assert!(!e.is_empty() && (e.contains(top) || value.is_empty()), "{key}={value:?}: {e}");
        }
    }
    assert!(app.readiness_options.is_default(), "{:?}", app.readiness_options);
    assert!(app.readiness_report == before, "a refused value does not measure again");
    // The ends of each range are accepted.
    for (key, ok) in [("readiness-ink-limit", ["100", "400"]), ("readiness-min-bleed-mm", ["0", "50"]), ("readiness-min-ppi", ["36", "1200"])] {
        for value in ok {
            set(&mut app, key, value).unwrap_or_else(|e| panic!("{key}={value}: {e}"));
        }
    }
}

#[test]
fn a_typed_field_applies_when_valid_and_is_refused_visibly_when_not() {
    use pdfcraft_ui_egui::ReadinessField::{InkLimit, MinBleed, MinPpi};
    let mut app = open_report();
    app.edit_readiness_field(InkLimit, "350").unwrap();
    assert!(ids(&app, "ink-total").is_empty());
    // Half-typed and out of range: refused with the reason, the last good value and report stay, and what was typed
    // stays in the field so the dialog can show why.
    let kept = app.readiness_report.clone();
    for bad in ["4", "4000", "-", "NaN", "inf", "3e2x"] {
        let why = app.edit_readiness_field(InkLimit, bad).expect_err(bad);
        assert!(why.contains("100") && why.contains("400"), "{bad}: {why}");
        assert_eq!(app.readiness_edit.text[0], bad);
        assert_eq!(app.readiness_edit.refused[0].as_deref(), Some(why.as_str()));
        assert_eq!(app.readiness_options.ink_limit, Some(350.0));
    }
    assert!(app.readiness_report == kept);
    // Emptying a field goes back to the default.
    app.edit_readiness_field(InkLimit, "").unwrap();
    assert_eq!(app.readiness_options.ink_limit, None);
    assert!(ids(&app, "ink-total").len() == 1);
    assert!(app.edit_readiness_field(MinBleed, "51").is_err() && app.edit_readiness_field(MinPpi, "35").is_err());
    assert!(app.readiness_options.is_default());
}

#[test]
fn the_thresholds_are_remembered_each_alone_and_bad_saved_values_keep_the_current_choice() {
    let mut app = crate::control::new_app();
    set(&mut app, "readiness-min-ppi", "225.5").unwrap();
    set(&mut app, "readiness-ink-limit", "280").unwrap();
    let saved = app.settings_snapshot();
    let mut next = crate::control::new_app();
    next.apply_settings(&saved);
    assert_eq!(
        (next.readiness_options.ink_limit, next.readiness_options.min_bleed_mm, next.readiness_options.min_ppi),
        (Some(280.0), None, Some(225.5))
    );
    // A restored value is what the next report is measured with.
    let ctx = egui::Context::default();
    next.open_bytes(&ctx, "all.pdf", None, all_three()).unwrap();
    assert!(next.execute(&ctx, "prepress.readiness"));
    assert!(next.readiness_report.as_ref() == Some(&engine(&next, Some(280.0), None, Some(225.5))));
    // Anything that is not a number the engine accepts keeps the current choice.
    let mut app = crate::control::new_app();
    set(&mut app, "readiness-min-bleed-mm", "3").unwrap();
    for json in [
        "",
        "not json",
        "[]",
        r#"{"print_readiness": null}"#,
        r#"{"print_readiness": 7}"#,
        r#"{"print_readiness": [300, 2, 300]}"#,
        r#"{"print_readiness": {"min_bleed_mm": "5", "ink_limit": 1000, "min_ppi": -4}}"#,
        r#"{"print_readiness": {"min_bleed_mm": null, "ink_limit": 99.9, "min_ppi": 1200.5}}"#,
        r#"{"print_readiness": {"min_bleed_mm": 50.5}}"#,
        r#"{"readiness_ink_limit": "no", "readiness_min_ppi": 4000}"#,
    ] {
        app.apply_settings(&pdfcraft_ui_egui::settings::import_json(json).0);
        assert_eq!(
            (app.readiness_options.ink_limit, app.readiness_options.min_bleed_mm, app.readiness_options.min_ppi),
            (None, Some(3.0), None),
            "{json:?}"
        );
    }
    // The fork's releases kept them as `print_readiness: { ... }`; a missing key leaves the others alone.
    app.apply_settings(&pdfcraft_ui_egui::settings::import_json(r#"{"print_readiness": {"min_ppi": 150}}"#).0);
    assert_eq!((app.readiness_options.min_bleed_mm, app.readiness_options.min_ppi), (Some(3.0), Some(150.0)));
}

/// The typed forms of a number: a comma is the decimal mark, full-width digits are digits, -0 is 0.
#[test]
fn typed_numbers_accept_a_comma_and_full_width_digits_and_read_minus_zero_as_zero() {
    use pdfcraft_ui_egui::ReadinessField::{InkLimit, MinBleed, MinPpi};
    assert_eq!(MinBleed.parse("2,5"), Ok(Some(2.5)));
    assert_eq!(MinBleed.parse(" 2.5 "), Ok(Some(2.5)));
    assert_eq!(InkLimit.parse("\u{FF12}\u{FF15}\u{FF10}"), Ok(Some(250.0)));
    assert_eq!(MinPpi.parse("\u{FF12}\u{FF12}\u{FF15}\u{FF0C}\u{FF15}"), Ok(Some(225.5)));
    let zero = MinBleed.parse("-0").unwrap().unwrap();
    assert!(zero == 0.0 && zero.is_sign_positive(), "{zero:?}");
    assert_eq!(MinBleed.parse("\u{2212}0.0").unwrap(), Some(0.0));
    assert_eq!(InkLimit.parse("Default"), Ok(None));
    for bad in ["1,000,5", "2,", "three", "NaN", "inf", "99,9", "400,1", "1e999"] {
        let why = InkLimit.parse(bad).expect_err(bad);
        assert!(why.contains("100") && why.contains("400") && why.contains("digits 0-9"), "{bad}: {why}");
    }
    let mut app = crate::control::new_app();
    set(&mut app, "readiness-min-bleed-mm", "-0").unwrap();
    let kept = app.readiness_options.min_bleed_mm.unwrap();
    assert!(kept == 0.0 && kept.is_sign_positive(), "{kept:?}");
}

/// ui.set says which key, what is allowed and how to go back, in English; and setting the value a field already has
/// still puts its text right again.
#[test]
fn control_refusals_name_the_key_and_an_unchanged_set_clears_a_refused_text() {
    use pdfcraft_ui_egui::ReadinessField::InkLimit;
    let mut app = crate::control::new_app();
    for bad in ["401", "", "abc", "NaN"] {
        let e = set(&mut app, "readiness-ink-limit", bad).expect_err(bad);
        assert_eq!(e, "readiness-ink-limit must be a number from 100 to 400, or default", "{bad:?}");
    }
    assert_eq!(set(&mut app, "readiness-min-bleed-mm", "51").unwrap_err(), "readiness-min-bleed-mm must be a number from 0 to 50, or default");
    assert_eq!(set(&mut app, "readiness-min-ppi", "x").unwrap_err(), "readiness-min-ppi must be a number from 36 to 1200, or default");
    assert!(app.edit_readiness_field(InkLimit, "4000").is_err());
    assert!(app.readiness_edit.refused[0].is_some() && app.readiness_edit.text[0] == "4000");
    // The value is already the default, so nothing changes, but the field no longer shows 4000.
    set(&mut app, "readiness-ink-limit", "default").unwrap();
    assert!(app.readiness_edit.refused[0].is_none() && app.readiness_edit.text[0].is_empty(), "{:?}", app.readiness_edit);
}

/// Typing changes nothing until Enter or leaving the field; a refused text says why on screen.
#[test]
fn typing_into_the_dialog_fields_applies_on_enter_or_leaving_and_a_bad_value_is_refused_on_screen() {
    let mut h = harness(Some(with_ink()));
    assert!(h.execute("prepress.readiness"));
    h.run_steps(4);
    let first = h.state().readiness_report.clone();
    assert!(first.as_ref().is_some_and(|r| r.findings.iter().any(|f| f.id == "ink-total")));
    // Typing changes nothing yet: no setting, no measuring, and the line under the field says what to do.
    let ink = h.get_by_role_and_label(Role::TextInput, "Total ink limit (%)");
    ink.focus();
    ink.type_text("350");
    h.run_steps(3);
    assert_eq!(h.state().readiness_options.ink_limit, None);
    assert!(h.state().readiness_report == first, "a keystroke does not measure");
    assert!(h.query_all_by_label_contains("Press Enter to apply").count() > 0);
    // Enter applies it: 330% is under a 350% limit, so the finding leaves the list, and the text stays.
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().readiness_options.ink_limit, Some(350.0));
    assert_eq!(h.state().readiness_edit.text[0], "350");
    assert!(h.state().readiness_report.as_ref().is_some_and(|r| r.findings.iter().all(|f| f.id != "ink-total")));
    // Leaving a field applies it too (focus moves to another field).
    let bleed = h.get_by_role_and_label(Role::TextInput, "Least bleed (mm)");
    bleed.focus();
    bleed.type_text("2,5");
    h.run_steps(2);
    assert_eq!(h.state().readiness_options.min_bleed_mm, None);
    h.get_by_role_and_label(Role::TextInput, "Least resolution (ppi)").focus();
    h.run_steps(3);
    assert_eq!(h.state().readiness_options.min_bleed_mm, Some(2.5), "a comma is a decimal mark");
    // One more digit is out of range: the reason shows on Enter and the setting is kept.
    let ink = h.get_by_role_and_label(Role::TextInput, "Total ink limit (%)");
    ink.focus();
    ink.type_text("5");
    h.run_steps(3);
    assert!(h.query_all_by_label_contains("from 100 to 400 percent").count() == 0, "no refusal while typing");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.query_all_by_label_contains("from 100 to 400 percent").count() > 0, "the reason is on screen");
    assert_eq!(h.state().readiness_options.ink_limit, Some(350.0));
    h.query_all_by_role_and_label(Role::Button, "Reset to defaults").last().expect("Reset to defaults").click();
    h.run_steps(3);
    assert!(h.state().readiness_options.is_default());
    assert!(h.query_all_by_label_contains("from 100 to 400 percent").count() == 0, "Reset clears the refusal");
    assert!(h.state().readiness_report.as_ref().is_some_and(|r| r.findings.iter().any(|f| f.id == "ink-total")));
    close(&mut h);
    assert!(!h.state().readiness_open);
}
