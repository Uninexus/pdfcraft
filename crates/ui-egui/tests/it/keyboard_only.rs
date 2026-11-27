//! Keyboard-only operation: the main workflows driven by keys alone (parity a11y.keyboard-only).

use std::sync::Arc;

use craft::host::{Picked, ScriptedFiles};
use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use pdfcraft_ui_egui::PdfCraftApp;

use crate::support;

type H = Harness<'static, PdfCraftApp>;

/// Three pages and two bookmarks, the second to the last page.
fn book() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /Outlines 6 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 300 400] >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Type /Outlines /First 7 0 R /Last 8 0 R /Count 2 >>",
        "<< /Title (Start) /Parent 6 0 R /Next 8 0 R /Dest [3 0 R /Fit] >>",
        "<< /Title (Finish) /Parent 6 0 R /Prev 7 0 R /Dest [5 0 R /Fit] >>",
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

fn start(files: ScriptedFiles) -> H {
    let files = Arc::new(files);
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
        support::install_runtime(&cc.egui_ctx);
        let mut app = crate::control::app_with_files(files);
        app.set_option(&egui::Context::default(), "language", "en").unwrap();
        app
    });
    h.run_steps(4);
    h
}

fn open(bytes: Vec<u8>) -> H {
    let mut h = start(ScriptedFiles::new());
    let ctx = h.ctx.clone();
    h.state_mut().open_bytes(&ctx, "book.pdf", None, bytes).unwrap();
    h.run_steps(8);
    h
}

fn key(h: &mut H, modifiers: Modifiers, key: Key) {
    h.key_press_modifiers(modifiers, key);
    h.run_steps(4);
}

fn press(h: &mut H, pressed: Key) {
    key(h, Modifiers::NONE, pressed);
}

fn command(h: &mut H, pressed: Key) {
    key(h, Modifiers::COMMAND, pressed);
}

fn type_text(h: &mut H, text: &str) {
    h.event(egui::Event::Text(text.into()));
    h.run_steps(4);
}

/// The focused control's role and name.
fn focused(h: &H) -> Option<(Role, String)> {
    h.query_by(|n| n.is_focused()).map(|n| (n.accesskit_node().role(), n.accesskit_node().label().unwrap_or_default()))
}

/// Presses Tab until the control with this role and name has the focus.
fn tab_to(h: &mut H, role: Role, name: &str) {
    for _ in 0..150 {
        press(h, Key::Tab);
        if focused(h).is_some_and(|(r, n)| r == role && n == name) {
            return;
        }
    }
    panic!("Tab never reached {role:?} {name:?}");
}

/// Runs the command the palette lists first for `query`.
fn palette(h: &mut H, query: &str) {
    command(h, Key::K);
    type_text(h, query);
    press(h, Key::Enter);
    h.run_steps(8);
}

fn dirty(h: &H) -> bool {
    let app = h.state();
    app.session.get(app.views[0].id).unwrap().dirty
}

fn zoom(h: &H) -> f64 {
    h.state().views[0].canvas_view().unwrap().zoom
}

/// Whether a ring is painted round the page view.
fn ringed(h: &H) -> bool {
    let view = h.state().views[0].viewport_rect();
    h.output().shapes.iter().any(|clipped| match &clipped.shape {
        egui::Shape::Rect(shape) => {
            shape.stroke.width > 0.0
                && shape.fill.a() == 0
                && (shape.rect.min - view.min).length() < 4.0
                && (shape.rect.max - view.max).length() < 4.0
        }
        _ => false,
    })
}

/// The command palette runs what it lists; every row used to be disabled while it was open.
#[test]
fn the_command_palette_runs_the_command_it_lists() {
    let mut h = open(book());
    let before = zoom(&h);
    palette(&mut h, "view.zoom.in");
    assert!(zoom(&h) > before, "{before} then {}", zoom(&h));
    assert!(!h.state().palette_open);
}

/// The page view shows a ring when keys bring the focus to it, and none after a click.
#[test]
fn the_page_view_shows_where_the_keyboard_is() {
    let mut h = open(book());
    assert!(!ringed(&h));
    tab_to(&mut h, Role::Canvas, "Pages");
    assert!(ringed(&h), "no ring round the focused page view");
    let at = h.state().views[0].viewport_rect().center();
    h.hover_at(at);
    h.drag_at(at);
    h.run_steps(2);
    h.drop_at(at);
    h.run_steps(4);
    assert!(!ringed(&h), "a click shows no ring");
}

#[test]
fn open_read_zoom_use_the_menu_and_a_tool_then_switch_tabs_and_save_by_keys() {
    let path = std::env::temp_dir().join(format!("pdfcraft-keys-{}.pdf", std::process::id()));
    std::fs::write(&path, book()).unwrap();
    let mut h = start(ScriptedFiles::answering([Picked::Paths(vec![path.clone()])]).with_file(&path, book()));

    // Open from Home.
    tab_to(&mut h, Role::Button, "Open…");
    press(&mut h, Key::Enter);
    assert!(support::step_until(&mut h, |h| h.state().active.is_some()), "nothing opened");
    h.run_steps(8);

    // Pages and zoom.
    tab_to(&mut h, Role::Canvas, "Pages");
    press(&mut h, Key::PageDown);
    assert_eq!(h.state().views[0].current(), 1);
    let before = zoom(&h);
    command(&mut h, Key::Plus);
    assert!(zoom(&h) > before);
    command(&mut h, Key::Minus);

    // The menu bar, and back to the page.
    press(&mut h, Key::F10);
    assert_eq!(focused(&h), Some((Role::MenuItem, "File".into())));
    press(&mut h, Key::ArrowRight);
    assert_eq!(focused(&h), Some((Role::MenuItem, "Edit".into())));
    press(&mut h, Key::ArrowDown);
    assert!(focused(&h).is_some_and(|(_, name)| name != "Edit"), "the menu opened on a row");
    press(&mut h, Key::Escape);
    press(&mut h, Key::Escape);

    // The Tools pane.
    tab_to(&mut h, Role::List, "All tools");
    press(&mut h, Key::ArrowDown);
    press(&mut h, Key::ArrowDown);
    press(&mut h, Key::ArrowDown);
    assert_eq!(focused(&h), Some((Role::ListItem, "Edit a PDF".into())));
    press(&mut h, Key::Enter);
    assert!(matches!(&h.state().left, pdfcraft_ui_egui::LeftPanel::Tool(id) if *id == "edit"));

    // Switch tabs and back, then save.
    let ctx = h.ctx.clone();
    h.state_mut().open_bytes(&ctx, "two.pdf", None, crate::control::fixture(2)).unwrap();
    h.run_steps(6);
    assert_eq!(h.state().active, Some(1));
    key(&mut h, Modifiers::CTRL, Key::Tab);
    assert_eq!(h.state().active, Some(0));
    command(&mut h, Key::S);
    h.run_steps(8);
    assert!(support::shown_notice(&h).is_some_and(|n| n.starts_with("Saved")), "{:?}", support::shown_notice(&h));
    let _ = std::fs::remove_file(path);
}

#[test]
fn comment_fill_a_form_and_use_the_side_panels_and_dialogs_by_keys() {
    // Bookmarks: the panel from the palette, a row by arrows and Enter.
    let mut h = open(book());
    palette(&mut h, "Bookmarks");
    tab_to(&mut h, Role::List, "Bookmarks");
    press(&mut h, Key::ArrowDown);
    press(&mut h, Key::ArrowDown);
    press(&mut h, Key::Enter);
    h.run_steps(20);
    assert_eq!(h.state().views[0].current(), 2);

    // A sticky note, written in the Comments panel.
    let mut h = open(book());
    palette(&mut h, "Comments");
    tab_to(&mut h, Role::TextInput, "Add a comment");
    type_text(&mut h, "By keyboard");
    press(&mut h, Key::Enter);
    h.run_steps(8);
    assert!(dirty(&h), "the note was added");
    press(&mut h, Key::Tab);
    assert!(focused(&h).is_some_and(|(role, name)| role == Role::TreeItem && name.contains("By keyboard")), "{:?}", focused(&h));

    // Highlight the selected text of a one-page document.
    let mut h = open(crate::control::fixture(1));
    support::settle(&mut h);
    command(&mut h, Key::A);
    support::settle(&mut h);
    palette(&mut h, "comment.highlight");
    assert!(dirty(&h), "the selection was highlighted");

    // A form: type in a field, Tab on, tick the box.
    let mut h = open(include_bytes!("../data/form.pdf").to_vec());
    tab_to(&mut h, Role::MultilineTextInput, "name");
    type_text(&mut h, "Ada");
    press(&mut h, Key::Tab);
    press(&mut h, Key::Space);
    let doc = h.state().session.get(h.state().views[0].id).unwrap();
    let value = |name: &str| doc.form.iter().find(|f| f.name == name).unwrap().value.clone();
    assert_eq!(value("name"), ["Ada"]);
    assert_eq!(value("agree"), ["Yes"]);

    // Preferences and Print open, take Tab, and close back to where the keyboard was.
    let mut h = open(book());
    tab_to(&mut h, Role::Canvas, "Pages");
    command(&mut h, Key::Comma);
    h.run_steps(8);
    assert_eq!(focused(&h), Some((Role::TextInput, "Search settings".into())));
    press(&mut h, Key::Tab);
    assert!(focused(&h).is_some());
    press(&mut h, Key::Escape);
    assert_eq!(focused(&h), Some((Role::Canvas, "Pages".into())));
    command(&mut h, Key::P);
    h.run_steps(8);
    assert_eq!(focused(&h), Some((Role::SpinButton, "Copies".into())));
    press(&mut h, Key::Escape);
    assert_eq!(focused(&h), Some((Role::Canvas, "Pages".into())));
}

/// Preferences opened with the title bar's button gives the keyboard back to that button.
#[test]
fn closing_preferences_returns_the_focus_to_its_opener() {
    let mut h = open(book());
    tab_to(&mut h, Role::Button, "Preferences…");
    press(&mut h, Key::Enter);
    h.run_steps(8);
    assert_eq!(focused(&h), Some((Role::TextInput, "Search settings".into())));
    press(&mut h, Key::Escape);
    h.run_steps(8);
    assert_eq!(focused(&h), Some((Role::Button, "Preferences…".into())));
}

/// Select All of a continuous view of several pages, then a text markup tool, marks the text of every page.
#[test]
fn markup_after_select_all_marks_every_page() {
    for command_id in ["comment.highlight", "comment.underline", "comment.strikeout"] {
        let mut h = open(crate::control::fixture(3));
        support::settle(&mut h);
        command(&mut h, Key::A);
        support::settle(&mut h);
        palette(&mut h, command_id);
        support::settle(&mut h);
        let app = h.state();
        let mut pages: Vec<usize> = app.session.get(app.views[0].id).unwrap().info.annotations.iter().map(|a| a.page).collect();
        pages.sort_unstable();
        pages.dedup();
        assert_eq!(pages, [0, 1, 2], "{command_id}");
    }
}

/// A side panel opened by keys takes the keyboard: its list where it has one, else the start of the panel.
#[test]
fn opening_a_side_panel_by_keys_moves_the_focus_into_it() {
    // By the palette.
    let mut h = open(book());
    palette(&mut h, "Bookmarks");
    h.run_steps(8);
    assert_eq!(focused(&h), Some((Role::List, "Bookmarks".into())), "palette");
    // F4 closes it and brings it back.
    press(&mut h, Key::F4);
    press(&mut h, Key::F4);
    h.run_steps(8);
    assert_eq!(focused(&h), Some((Role::List, "Bookmarks".into())), "F4");
    // By the View menu's command.
    let mut h = open(book());
    palette(&mut h, "Comments");
    h.run_steps(8);
    assert_eq!(focused(&h).map(|(_, name)| name), Some("Comments panel".into()), "comments");
    press(&mut h, Key::Tab);
    assert!(focused(&h).is_some_and(|(_, name)| name != "Comments panel"));
}
