//! Print production ▸ Print readiness: the report opens from the command, lists what the ink
//! check found, and changes nothing.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::{Dialog, PdfCraftApp};

fn with_ink() -> Vec<u8> {
    let mut doc = pdfcraft_cos::Document::open(std::sync::Arc::new(include_bytes!("../data/form.pdf").to_vec())).unwrap();
    let page = pdfcraft_model::pages(&doc).into_iter().next().unwrap();
    // 330% total ink.
    let ink = doc.add(pdfcraft_cos::Object::Stream(pdfcraft_cos::Stream::flate(pdfcraft_cos::Dict::new(), b"0.9 0.8 0.7 0.9 k 10 10 100 100 re f")));
    doc.update_dict(page.obj, |d| d.set(b"Contents".to_vec(), pdfcraft_cos::Object::Ref(ink))).unwrap();
    pdfcraft_cos::write_full(&doc, &Default::default()).unwrap()
}

fn harness(bytes: Vec<u8>) -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("ink.pdf", None, bytes.clone()).unwrap();
        app
    });
    h.run_steps(4);
    h
}

#[test]
fn the_report_lists_the_ink_findings_and_changes_nothing() {
    let mut h = harness(with_ink());
    let id = h.state().views[0].id;
    let generation = h.state().session.get(id).unwrap().edit_generation();
    assert!(h.state_mut().execute("prepress.readiness"));
    settle(&mut h);
    assert_eq!(h.state().dialog, Some(Dialog::PrintReadiness));
    h.get_by_label("Print readiness");
    assert!(h.state().readiness_report.as_ref().is_some_and(|r| r.findings.iter().any(|f| f.id == "ink-total")));
    // The finding is on screen, in words, with its severity.
    h.get_by_label_contains("total ink");
    h.get_by_label("Warning");
    h.get_by_label("Close").click();
    h.run_steps(2);
    assert_eq!(h.state().dialog, None);
    assert_eq!(h.state().session.get(id).unwrap().edit_generation(), generation, "nothing was edited");
    assert_eq!(h.state().session.get(id).unwrap().can_undo(), None);
}

/// Run frames until the dialog stops moving (Close has the same rectangle for three frames in a
/// row), so a click lands where the button is.
fn settle(h: &mut Harness<'static, PdfCraftApp>) {
    let mut last = None;
    let mut steady = 0;
    for _ in 0..60 {
        h.run_steps(1);
        let now = h.query_by_label("Close").map(|n| n.rect());
        steady = if now.is_some() && now == last { steady + 1 } else { 0 };
        if steady >= 3 {
            return;
        }
        last = now;
    }
    panic!("the dialog never settled");
}

/// A page with a TrimBox and a bleed past it, and nothing drawn: nothing for the report to find.
fn trimmed() -> Vec<u8> {
    let mut doc = pdfcraft_cos::Document::open(std::sync::Arc::new(include_bytes!("../data/form.pdf").to_vec())).unwrap();
    for page in pdfcraft_model::pages(&doc) {
        let [x0, y0, x1, y1] = page.crop(&doc);
        doc.update_dict(page.obj, |d| {
            let rect = |n: f64| pdfcraft_cos::Object::Array(vec![(x0 + n).into(), (y0 + n).into(), (x1 - n).into(), (y1 - n).into()]);
            d.set(b"TrimBox".to_vec(), rect(18.0));
            d.set(b"BleedBox".to_vec(), rect(0.0));
        })
        .unwrap();
    }
    pdfcraft_cos::write_full(&doc, &Default::default()).unwrap()
}

#[test]
fn a_document_with_nothing_to_report_says_so_and_still_shows_what_was_measured() {
    let mut h = harness(trimmed());
    assert!(h.state_mut().execute("prepress.readiness"));
    h.run_steps(3);
    assert_eq!(h.state().dialog, Some(Dialog::PrintReadiness));
    assert!(h.state().readiness_report.as_ref().is_some_and(|r| r.findings.is_empty()), "{:?}", h.state().readiness_report);
    h.get_by_label("No stop for image resolution, bleed or ink.");
    // What was measured is on screen too: the output intent and the page boxes, and what isn't checked.
    h.get_by_label_contains("Output intent");
    h.get_by_label_contains("Page 1: media");
    h.get_by_label_contains("annotation appearances");
}

#[test]
fn the_dialog_shows_fonts_and_pdfa_results_and_clears_its_report_when_closed() {
    let mut h = harness(include_bytes!("../data/form.pdf").to_vec());
    assert!(h.state_mut().execute("prepress.readiness"));
    settle(&mut h);
    let report = h.state().readiness_report.clone().unwrap();
    // The PDF/A-2b result is a line of the report, shown whether or not it found a problem.
    h.get_by_label_contains("PDF/A-2b");
    assert!(report.details().contains("PDF/A-2b"));
    h.get_by_label("Close").click();
    h.run_steps(2);
    assert_eq!(h.state().dialog, None);
    assert!(h.state().readiness_report.is_none(), "the report is dropped with the dialog");
}

#[test]
fn print_readiness_needs_a_document() {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app
    });
    h.run_steps(3);
    assert!(!h.state_mut().execute("prepress.readiness"), "disabled without a document");
    assert_eq!(h.state().dialog, None);
}

/// One page with a 200 ppi image, 330% total ink, and a TrimBox 10 pt (3.5 mm) inside the bleed:
/// by default only the ink and the resolution are findings.
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

fn open_report() -> PdfCraftApp {
    let mut app = PdfCraftApp::new();
    app.set_option("language", "en").unwrap();
    app.open_bytes("all.pdf", None, all_three()).unwrap();
    assert!(app.execute("prepress.readiness"));
    assert_eq!(app.dialog, Some(Dialog::PrintReadiness));
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
    app.set_option("readiness-ink-limit", "350").unwrap();
    assert!(ids(&app, "ink-total").is_empty(), "330% is under 350");
    assert!(app.readiness_report.as_ref() == Some(&engine(&app, Some(350.0), None, None)));
    app.set_option("readiness-ink-limit", "250").unwrap();
    assert!(ids(&app, "ink-total")[0].1.contains("250"), "{:?}", ids(&app, "ink-total"));
    app.set_option("readiness-min-bleed-mm", "5").unwrap();
    assert!(ids(&app, "page-bleed")[0].1.contains("5 mm was asked for"), "{:?}", ids(&app, "page-bleed"));
    app.set_option("readiness-min-ppi", "600").unwrap();
    assert!(ids(&app, "image-ppi")[0].1.contains("600 ppi was asked for"), "{:?}", ids(&app, "image-ppi"));
    assert!(app.readiness_report.as_ref() == Some(&engine(&app, Some(250.0), Some(5.0), Some(600.0))));
    let text = app.readiness_report_text().unwrap();
    assert!(text.contains("total ink limit 250%") && text.contains("least bleed 5 mm") && text.contains("least image resolution 600 ppi"), "{text}");
    assert!(text.contains("600 ppi was asked for"), "Copy report carries the findings of the changed thresholds: {text}");
    app.set_option("readiness-min-ppi", "150").unwrap();
    assert!(ids(&app, "image-ppi").is_empty(), "200 ppi passes 150");
    // Back to the defaults: the same report as before anything was set.
    for key in ["readiness-ink-limit", "readiness-min-bleed-mm", "readiness-min-ppi"] {
        app.set_option(key, "default").unwrap();
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
            let e = app.set_option(key, value).expect_err(&format!("{key}={value:?} is refused"));
            assert!(!e.is_empty() && (e.contains(top) || value.is_empty()), "{key}={value:?}: {e}");
        }
    }
    assert!(app.readiness_options.is_default(), "{:?}", app.readiness_options);
    assert!(app.readiness_report == before, "a refused value does not re-measure");
    // The ends of each range are accepted.
    for (key, ok) in [("readiness-ink-limit", ["100", "400"]), ("readiness-min-bleed-mm", ["0", "50"]), ("readiness-min-ppi", ["36", "1200"])] {
        for value in ok {
            app.set_option(key, value).unwrap_or_else(|e| panic!("{key}={value}: {e}"));
        }
    }
}

#[test]
fn a_typed_field_applies_when_valid_and_is_refused_visibly_when_not() {
    use pdfcraft_ui_egui::ReadinessField::{InkLimit, MinBleed, MinPpi};
    let mut app = open_report();
    app.edit_readiness_field(InkLimit, "350").unwrap();
    assert!(ids(&app, "ink-total").is_empty());
    // Half-typed and out of range: refused with the reason, the last good value and report stay,
    // and what was typed stays in the field so the dialog can show why.
    let kept = app.readiness_report.clone();
    for bad in ["4", "4000", "-", "NaN", "inf", "3e2x"] {
        let why = app.edit_readiness_field(InkLimit, bad).expect_err(bad);
        assert!(why.contains("100") && why.contains("400"), "{bad}: {why}");
        assert_eq!(app.readiness_edit.text[0], bad);
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
    let mut app = PdfCraftApp::new();
    app.set_option("readiness-min-ppi", "225.5").unwrap();
    app.set_option("readiness-ink-limit", "280").unwrap();
    let saved = app.persist();
    let mut next = PdfCraftApp::new();
    next.restore(&saved);
    assert_eq!(
        (next.readiness_options.ink_limit, next.readiness_options.min_bleed_mm, next.readiness_options.min_ppi),
        (Some(280.0), None, Some(225.5))
    );
    // A default threshold is not written, so it does not overwrite a choice made since.
    assert!(!saved.contains("min_bleed_mm"), "{saved}");
    let mut app = PdfCraftApp::new();
    app.set_option("readiness-min-bleed-mm", "3").unwrap();
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
    ] {
        app.restore(json);
        assert_eq!(
            (app.readiness_options.ink_limit, app.readiness_options.min_bleed_mm, app.readiness_options.min_ppi),
            (None, Some(3.0), None),
            "{json:?}"
        );
    }
    app.restore(r#"{"print_readiness": {"min_ppi": 150}}"#);
    assert_eq!(
        (app.readiness_options.min_bleed_mm, app.readiness_options.min_ppi),
        (Some(3.0), Some(150.0)),
        "a missing key leaves the others alone"
    );
    // A restored value is what the next report is measured with.
    let mut again = PdfCraftApp::new();
    again.restore(&app.persist());
    again.open_bytes("all.pdf", None, all_three()).unwrap();
    assert!(again.execute("prepress.readiness"));
    assert!(again.readiness_report.as_ref() == Some(&engine(&again, None, Some(3.0), Some(150.0))));
}

#[test]
fn typing_into_the_dialog_fields_applies_on_enter_or_leaving_and_a_bad_value_is_refused_on_screen() {
    let mut h = harness(with_ink());
    assert!(h.state_mut().execute("prepress.readiness"));
    settle(&mut h);
    let first = h.state().readiness_report.clone();
    assert!(first.as_ref().is_some_and(|r| r.findings.iter().any(|f| f.id == "ink-total")));
    // Typing changes nothing yet: no setting, no measuring, and the line under the field says what to do.
    h.get_by_label("Total ink limit (%)").focus();
    h.run_steps(2);
    h.get_by_label("Total ink limit (%)").type_text("350");
    h.run_steps(3);
    assert_eq!(h.state().readiness_options.ink_limit, None);
    assert!(h.state().readiness_report == first, "a keystroke does not measure");
    h.get_by_label_contains("Press Enter to apply");
    // Enter applies it: 330% is under a 350% limit, so the finding leaves the list, and the text stays.
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().readiness_options.ink_limit, Some(350.0));
    assert_eq!(h.state().readiness_edit.text[0], "350");
    assert!(h.state().readiness_report.as_ref().is_some_and(|r| r.findings.iter().all(|f| f.id != "ink-total")));
    // Leaving a field applies it too (focus moves to another field).
    h.get_by_label("Least bleed (mm)").focus();
    h.run_steps(2);
    h.get_by_label("Least bleed (mm)").type_text("2,5");
    h.run_steps(2);
    assert_eq!(h.state().readiness_options.min_bleed_mm, None);
    h.get_by_label("Least resolution (ppi)").focus();
    h.run_steps(3);
    assert_eq!(h.state().readiness_options.min_bleed_mm, Some(2.5), "a comma is a decimal mark");
    // One more digit is out of range: nothing is said while typing, the reason shows on Enter, and the setting is kept.
    h.get_by_label("Total ink limit (%)").focus();
    h.run_steps(2);
    h.get_by_label("Total ink limit (%)").type_text("5");
    h.run_steps(3);
    assert!(h.query_by_label_contains("from 100 to 400 percent").is_none(), "no refusal while typing");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    h.get_by_label_contains("from 100 to 400 percent");
    assert_eq!(h.state().readiness_options.ink_limit, Some(350.0));
    h.get_by_label("Reset to defaults").click();
    settle(&mut h);
    assert!(h.state().readiness_options.is_default());
    assert!(h.query_by_label_contains("from 100 to 400 percent").is_none(), "Reset clears the refusal");
    assert!(h.state().readiness_report.as_ref().is_some_and(|r| r.findings.iter().any(|f| f.id == "ink-total")));
    h.get_by_label("Close").click();
    h.run_steps(2);
    assert_eq!(h.state().dialog, None);
}

#[test]
fn on_a_short_window_the_report_scrolls_and_close_stays_in_reach() {
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 500.0)).build_eframe(|_cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.open_bytes("ink.pdf", None, with_ink()).unwrap();
        app
    });
    h.run_steps(4);
    assert!(h.state_mut().execute("prepress.readiness"));
    settle(&mut h);
    for label in ["Close", "Copy report", "Reset to defaults"] {
        let r = h.get_by_label(label).rect();
        assert!(r.min.y >= 0.0 && r.max.y <= 500.0, "{label} at {r:?} is off a 500 pt window");
    }
    h.get_by_label("Close").click();
    h.run_steps(2);
    assert_eq!(h.state().dialog, None);
}

/// The typed forms of a number: a comma is the decimal mark, full-width digits are digits, -0 is 0.
#[test]
fn typed_numbers_accept_a_comma_and_full_width_digits_and_read_minus_zero_as_zero() {
    use pdfcraft_ui_egui::ReadinessField::{InkLimit, MinBleed, MinPpi};
    assert_eq!(MinBleed.parse("2,5"), Ok(Some(2.5)));
    assert_eq!(MinBleed.parse(" 2.5 "), Ok(Some(2.5)));
    assert_eq!(InkLimit.parse("２５０"), Ok(Some(250.0)));
    assert_eq!(MinPpi.parse("２２５，５"), Ok(Some(225.5)));
    let zero = MinBleed.parse("-0").unwrap().unwrap();
    assert!(zero == 0.0 && zero.is_sign_positive(), "{zero:?}");
    assert_eq!(MinBleed.parse("−0.0").unwrap(), Some(0.0));
    assert_eq!(InkLimit.parse("Default"), Ok(None));
    for bad in ["1,000,5", "2,", "٣٠٠", "three", "NaN", "inf", "99,9", "400,1", "1e999"] {
        let why = InkLimit.parse(bad).expect_err(bad);
        assert!(why.contains("100") && why.contains("400") && why.contains("digits 0-9"), "{bad}: {why}");
    }
    let mut app = PdfCraftApp::new();
    app.set_option("readiness-min-bleed-mm", "-0").unwrap();
    let saved = app.readiness_options.min_bleed_mm.unwrap();
    assert!(saved == 0.0 && saved.is_sign_positive(), "{saved:?}");
    let mut next = PdfCraftApp::new();
    next.restore(r#"{"print_readiness": {"min_bleed_mm": -0.0}}"#);
    assert!(next.readiness_options.min_bleed_mm.is_some_and(|n| n.is_sign_positive()));
}

/// ui.set says which key, what is allowed and how to go back, in English; and setting the value a
/// field already has still puts its text right again.
#[test]
fn control_refusals_name_the_key_and_an_unchanged_set_clears_a_refused_text() {
    use pdfcraft_ui_egui::ReadinessField::InkLimit;
    let mut app = PdfCraftApp::new();
    for (key, bad) in [("readiness-ink-limit", "401"), ("readiness-ink-limit", ""), ("readiness-ink-limit", "abc"), ("readiness-ink-limit", "NaN")] {
        let e = app.set_option(key, bad).expect_err(bad);
        assert_eq!(e, "readiness-ink-limit must be a number from 100 to 400, or default", "{bad:?}");
    }
    assert_eq!(app.set_option("readiness-min-bleed-mm", "51").unwrap_err(), "readiness-min-bleed-mm must be a number from 0 to 50, or default");
    assert_eq!(app.set_option("readiness-min-ppi", "x").unwrap_err(), "readiness-min-ppi must be a number from 36 to 1200, or default");
    assert!(app.edit_readiness_field(InkLimit, "4000").is_err());
    assert!(app.readiness_edit.refused[0].is_some() && app.readiness_edit.text[0] == "4000");
    // The value is already the default, so nothing changes, but the field no longer shows 4000.
    app.set_option("readiness-ink-limit", "default").unwrap();
    assert!(app.readiness_edit.refused[0].is_none() && app.readiness_edit.text[0].is_empty(), "{:?}", app.readiness_edit);
}
