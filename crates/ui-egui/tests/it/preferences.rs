//! Preferences rows apply as they change and persist when the window closes.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
// PLAN: assumed public `pdfcraft_ui_egui::{Mode, units::Unit}` and `craft::theme::selection::MotionChoice` for tests
use craft::theme::selection::MotionChoice;
use pdfcraft_ui_egui::units::Unit;
use pdfcraft_ui_egui::{Mode, PdfCraftApp, side_panels};

const PDF: &[u8] = include_bytes!("../data/form.pdf");

type H = Harness<'static, PdfCraftApp>;

/// One row edit and the app state it must produce immediately.
struct Case {
    name: &'static str,
    page: &'static str,
    edit: fn(&mut H),
    applied: fn(&PdfCraftApp) -> bool,
}

fn harness() -> H {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "form.pdf", None, PDF.to_vec()).unwrap();
            app.recent.extend((0..5).map(|n| pdfcraft_ui_egui::RecentFile {
                name: format!("{n}.pdf"),
                path: format!("/nowhere/{n}.pdf"),
                pages: 1,
                size: 0,
            }));
            app
        }
    });
    h.run_steps(4);
    h
}

/// Whether the Preferences window is on screen: its search field shows while it is open.
fn preferences_shown(h: &H) -> bool {
    h.query_by_role_and_label(egui::accesskit::Role::TextInput, "Search settings").is_some()
}

fn click(h: &mut H, label: &str) {
    // Preferences is one scrolling page: bring the row into view first.
    h.get_by(|node| !node.is_hidden() && node.role() != egui::accesskit::Role::Label && node.label().as_deref() == Some(label)).scroll_to_me();
    h.run_steps(3);
    h.get_by(|node| !node.is_hidden() && node.role() != egui::accesskit::Role::Label && node.label().as_deref() == Some(label)).click();
    h.run_steps(3);
}

/// Picks `option` of the choice row `row`: a short choice is a row of segments (a radio group), a long one a
/// drop-down whose options show once it opens.
fn choose(h: &mut H, row: &str, option: &str) {
    use egui::accesskit::Role;
    if h.query_by_role_and_label(Role::RadioGroup, row).is_some() {
        h.get_by_role_and_label(Role::RadioGroup, row).scroll_to_me();
        h.run_steps(3);
        h.get_by_role_and_label(Role::RadioGroup, row).get_by_label(option).click();
        h.run_steps(3);
    } else {
        click(h, row);
        click(h, option);
    }
}

/// Replaces the text of the field named `label` through the keyboard, as a user would.
fn type_into(h: &mut H, label: &str, text: &str) {
    let role = if label == "Recent files to keep" { egui::accesskit::Role::SpinButton } else { egui::accesskit::Role::TextInput };
    h.get_by_role_and_label(role, label).scroll_to_me();
    h.run_steps(3);
    h.get_by_role_and_label(role, label).click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.get_by_role_and_label(role, label).type_text(text);
    h.run_steps(3);
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "default mode",
            page: "Documents and view",
            edit: |h| {
                // A choice row is a drop-down: its options show once it opens.
                choose(h, "Default workspace mode", "E-Sign");
            },
            applied: |a| a.default_mode == Mode::Sign,
        },
        Case {
            name: "layout and continuous on the active view",
            page: "Documents and view",
            edit: |h| {
                // A choice row is a drop-down: its options show once it opens.
                choose(h, "Default page display", "Two-page view");
                click(h, "Continuous scrolling");
            },
            applied: |a| {
                let v = &a.views[0].view;
                v.display == craft::pages::PageDisplay::TwoUp && v.continuous != crate::control::new_app().views.first().is_none_or(|_| true)
            },
        },
        Case {
            name: "units",
            page: "Documents and view",
            edit: |h| {
                choose(h, "Units", Unit::Millimetres.label());
            },
            applied: |a| a.units == Unit::Millimetres,
        },
        Case {
            name: "paper for new blank pages",
            page: "Documents and view",
            edit: |h| choose(h, "Paper for new blank pages", "A4"),
            applied: |a| a.blank_paper == pdfcraft_ui_egui::paper::Paper::A4,
        },
        Case {
            name: "keep the comment tool selected",
            page: "Comment",
            edit: |h| click(h, "Keep the selected comment tool selected after use"),
            applied: |a| a.comment_prefs.pinned,
        },
        Case {
            name: "tools side and rail side",
            page: "General & Interface",
            edit: |h| {
                // Each side row is a drop-down of Left and Right.
                choose(h, "Tools panel side", "Right");
                choose(h, "Navigation rail side", "Left");
            },
            applied: |a| {
                let dock = side_panels::state(a);
                dock["tools_side"] == "right" && dock["rail_side"] == "left"
            },
        },
        Case {
            name: "animation effects",
            page: "General & Interface",
            edit: |h| {
                choose(h, "Animation effects", "Off");
            },
            applied: |a| a.animation_effects == MotionChoice::Off,
        },
        Case {
            name: "recent limit truncates",
            page: "General & Interface",
            edit: |h| type_into(h, "Recent files to keep", "2"),
            applied: |a| a.recent_limit == 2 && a.recent.len() == 2,
        },
        Case {
            name: "comment author",
            page: "Identity",
            edit: |h| type_into(h, "Name on new comments", "Ada"),
            applied: |a| a.comment_prefs.author == "Ada",
        },
        Case {
            name: "identity email",
            page: "Identity",
            edit: |h| type_into(h, "Email", "ada@example.com"),
            applied: |a| a.identity_email == "ada@example.com",
        },
        Case {
            name: "field tooltips",
            page: "Forms",
            edit: |h| click(h, "Show a field's tooltip when the pointer rests on it"),
            applied: |a| !a.form_tooltips,
        },
        Case { name: "auto-complete", page: "Forms", edit: |h| choose(h, "Auto-Complete", "Basic"), applied: |a| a.session.form_autocomplete() },
        Case {
            name: "date format",
            page: "Fill & Sign",
            edit: |h| type_into(h, "Pattern", "dd.mm.yyyy"),
            applied: |a| a.session.date_format() == "dd.mm.yyyy",
        },
        Case { name: "javascript", page: "JavaScript", edit: |h| click(h, "Enable Acrobat JavaScript"), applied: |a| !a.session.javascript() },
    ]
}

fn edited(case: &Case) -> H {
    let mut h = harness();
    h.state_mut().open_preferences();
    h.run_steps(4);
    (case.edit)(&mut h);
    h
}

#[test]
fn preference_rows_apply_live_and_survive_closing() {
    for case in cases() {
        let mut h = edited(&case);
        assert!((case.applied)(h.state()), "{} ({}): a row applies immediately", case.name, case.page);
        click(&mut h, "OK");
        h.run_steps(3);
        assert!(!preferences_shown(&h), "{}: OK closes the window", case.name);
        assert!((case.applied)(h.state()), "{}: OK keeps the change", case.name);

        let mut h = edited(&case);
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        assert!(!preferences_shown(&h), "{}: Escape closes the window", case.name);
        assert!((case.applied)(h.state()), "{}: Escape keeps the change", case.name);
    }
}

#[test]
fn remove_all_remembered_entries_asks_first_forgets_them_and_leaves_the_window_open() {
    let mut h = harness();
    h.state_mut().session.set_form_entries(pdfcraft_engine::form_entries::FormEntries::restored(["Maplewood", "Oakdale"]));
    h.state_mut().open_preferences();
    h.run_steps(4);
    click(&mut h, "Remove all remembered entries");
    assert_eq!(h.state().session.form_entries().len(), 2, "asking forgets nothing");
    assert!(h.query_all_by_label_contains("cannot be taken back").count() > 0, "the window asks first");
    click(&mut h, "Keep the entries");
    assert_eq!(h.state().session.form_entries().len(), 2, "keeping them forgets nothing");
    click(&mut h, "Remove all remembered entries");
    click(&mut h, "Remove them");
    assert!(h.state().session.form_entries().is_empty(), "the answer forgets what was remembered");
    assert!(preferences_shown(&h), "and leaves the window open");
}

#[test]
fn a_portable_copy_keeps_remembered_entries_out_of_its_settings() {
    let mut h = harness();
    h.state_mut().session.set_form_entries(pdfcraft_engine::form_entries::FormEntries::restored(["Maplewood"]));
    h.state_mut().remember_entries_on_disk = true;
    let kept = h.state().settings_snapshot().behavior.form_entries;
    assert!(kept.is_some_and(|text| text.contains("Maplewood")), "an installed copy writes them");
    h.state_mut().remember_entries_on_disk = false;
    let saved = h.state().settings_snapshot();
    assert!(saved.behavior.form_entries.is_none(), "a portable copy writes none");
    // Entries found in a settings file are not read back either.
    let mut from_file = saved.clone();
    from_file.behavior.form_entries = Some("[\"Oakdale\"]".into());
    h.state_mut().session.clear_form_entries();
    h.state_mut().apply_settings(&from_file);
    assert!(h.state().session.form_entries().is_empty(), "nothing is restored from the file in a portable copy");
}

#[test]
fn follow_theme_takes_one_colour_back_to_the_theme_and_leaves_the_other() {
    let mut h = harness();
    let ctx = egui::Context::default();
    h.state_mut().set_option(&ctx, "field-highlight-color", "#102030").unwrap();
    h.state_mut().set_option(&ctx, "required-field-color", "#203040").unwrap();
    h.state_mut().open_preferences();
    h.run_steps(4);
    click(&mut h, "Follow theme: highlight colour");
    assert!(h.state().field_highlight_color.is_none(), "the highlight follows the theme again");
    assert!(h.state().required_field_color.is_some(), "the other colour was not reset");
}

/// A check box row is one line high, so a label that wraps is cut off. Every English check box label fits on one.
#[test]
fn no_check_box_label_is_cut_off() {
    use egui_kittest::kittest::NodeT as _;
    fn lines(shape: &egui::Shape, out: &mut Vec<(String, usize)>) {
        match shape {
            egui::Shape::Text(t) => out.push((t.galley.text().to_owned(), t.galley.rows.len())),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| lines(s, out)),
            _ => {}
        }
    }
    // Tall enough that the whole page is painted.
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 6000.0)).build_eframe(|cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        let mut app = crate::control::new_app();
        app.set_option(&egui::Context::default(), "language", "en").unwrap();
        app
    });
    h.run_steps(3);
    h.state_mut().open_preferences();
    h.run_steps(8);
    let boxes: Vec<String> = h.query_all_by_role(egui::accesskit::Role::CheckBox).filter_map(|n| n.accesskit_node().label()).collect();
    let mut painted = Vec::new();
    h.output().shapes.iter().for_each(|c| lines(&c.shape, &mut painted));
    let seen: Vec<&(String, usize)> = painted.iter().filter(|(text, _)| boxes.contains(text)).collect();
    assert!(seen.len() > 10, "the labels were painted: {seen:?}");
    let wrapped: Vec<_> = seen.iter().filter(|(_, rows)| *rows > 1).collect();
    assert!(wrapped.is_empty(), "{wrapped:?}");
}
