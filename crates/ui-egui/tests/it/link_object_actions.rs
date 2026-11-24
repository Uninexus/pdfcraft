//! Link Properties' article, menu item and file actions: chosen in the dialog, one undo step each,
//! read back when the dialog opens again, and offered only for a link that exists.

use crate::support::AppHarness as _;
use egui::Pos2;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_engine::LinkAction;
use pdfcraft_ui_egui::PdfCraftApp;

/// Two threads (the first has two beads, the second one) and a page with a web link in its lower
/// left corner (object 9, `/Rect [10 10 50 50]`) on a 200 pt square page.
fn linked_pdf() -> Vec<u8> {
    linked_pdf_with("/A << /S /URI /URI (http://x.test) >>")
}

/// [`linked_pdf`] with this entry as the link's action.
fn linked_pdf_with(action: &str) -> Vec<u8> {
    let link = format!("<< /Type /Annot /Subtype /Link /Rect [10 10 50 50] /Border [0 0 0] {action} >>");
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R /Threads [4 0 R 5 0 R] >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>",
        "<< /Type /Page /Parent 2 0 R /Annots [9 0 R] >>",
        "<< /F 6 0 R /I << /Title (Intro) >> >>",
        "<< /F 8 0 R /I << /Title (Second) >> >>",
        "<< /T 4 0 R /P 3 0 R /R [0 0 10 10] /N 7 0 R /V 7 0 R >>",
        "<< /T 4 0 R /P 3 0 R /R [0 0 20 20] /N 6 0 R /V 6 0 R >>",
        "<< /T 5 0 R /P 3 0 R /R [0 0 30 30] /N 8 0 R /V 8 0 R >>",
        link.as_str(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).as_bytes());
    out
}

fn harness() -> Harness<'static, PdfCraftApp> {
    harness_on(linked_pdf())
}

fn harness_on(pdf: Vec<u8>) -> Harness<'static, PdfCraftApp> {
    // 60 fps steps, so two clicks fall inside egui's double-click window.
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "linked.pdf", None, pdf.clone()).unwrap();
            app.set_option(&egui::Context::default(), "zoom", "150").unwrap();
            app
        }
    });
    h.run_steps(6);
    h
}

/// A point on the page, in points from its top left.
fn at(h: &Harness<'static, PdfCraftApp>, x: f32, y: f32) -> Pos2 {
    let r = h.state().views[0].page_screen_rect(0).unwrap();
    let k = r.width() / 200.0;
    r.min + egui::vec2(x * k, y * k)
}

fn double_click(h: &mut Harness<'static, PdfCraftApp>, c: Pos2) {
    h.hover_at(c);
    h.run_steps(1);
    for _ in 0..2 {
        h.drag_at(c);
        h.step();
        h.drop_at(c);
        h.step();
    }
    h.run_steps(2);
}

fn drag(h: &mut Harness<'static, PdfCraftApp>, a: Pos2, b: Pos2) {
    h.hover_at(a);
    h.run_steps(1);
    h.drag_at(a);
    h.run_steps(1);
    for k in 1..=4 {
        h.hover_at(a + (b - a) * (k as f32 / 4.0));
        h.run_steps(1);
    }
    h.drop_at(b);
    h.run_steps(3);
}

/// Opens Link Properties of the page's one link, with the Link tool.
fn open_properties(h: &mut Harness<'static, PdfCraftApp>) {
    assert!(h.execute("edit.link"));
    h.run_steps(2);
    let c = at(h, 30.0, 170.0);
    double_click(h, c);
    h.get_by_role_and_label(Role::Dialog, "Link Properties");
}

/// Picks `option` in the drop-down named `combo`.
fn choose(h: &mut Harness<'static, PdfCraftApp>, combo: &str, option: &str) {
    h.query_all_by_role_and_label(Role::ComboBox, combo).last().unwrap_or_else(|| panic!("a {combo} drop-down")).click();
    h.run_steps(3);
    h.query_all_by_role_and_label(Role::ListBoxOption, option).last().unwrap_or_else(|| panic!("a {option} choice")).scroll_to_me();
    h.run_steps(3);
    h.query_all_by_role_and_label(Role::ListBoxOption, option).last().unwrap_or_else(|| panic!("a {option} choice")).click();
    h.run_steps(2);
}

fn ok(h: &mut Harness<'static, PdfCraftApp>) {
    h.get_by_label("OK").click();
    h.run_steps(3);
}

fn link(h: &Harness<'static, PdfCraftApp>) -> pdfcraft_engine::LinkItem {
    let s = h.state();
    s.session.get(s.views[0].id).unwrap().links()[0].clone()
}

fn with_doc<T>(h: &Harness<'static, PdfCraftApp>, read: impl FnOnce(&pdfcraft_engine::Document, pdfcraft_cos::ObjRef) -> T) -> T {
    let s = h.state();
    let doc = s.session.get(s.views[0].id).unwrap();
    let obj = doc.links()[0].obj.expect("a link with an object");
    read(doc, obj)
}

fn disabled(h: &Harness<'static, PdfCraftApp>, role: Role, label: &str) -> bool {
    h.query_all_by_role_and_label(role, label).last().unwrap_or_else(|| panic!("a {label}")).accesskit_node().is_disabled()
}

#[test]
fn a_link_is_set_to_go_to_an_article_thread_in_one_undo_step() {
    let mut h = harness();
    assert_eq!(link(&h).action, LinkAction::Uri("http://x.test".into()));
    open_properties(&mut h);
    h.get_by_label("Go to an article thread").click();
    h.run_steps(3);
    choose(&mut h, "Article", "Second");
    ok(&mut h);
    let read = with_doc(&h, |d, o| d.link_thread_action(o));
    assert!(read.is_thread, "the link now has an article action");
    assert_eq!(read.thread, Some(1), "the thread picked, counting from 0");
    h.undo();
    h.run_steps(3);
    assert_eq!(link(&h).action, LinkAction::Uri("http://x.test".into()), "one undo puts the web address back");
    assert!(!with_doc(&h, |d, o| d.link_thread_action(o)).is_thread);
}

#[test]
fn a_link_is_set_to_run_a_menu_item_from_the_safe_list_in_one_undo_step() {
    let mut h = harness();
    open_properties(&mut h);
    h.get_by_label("Run a menu item").click();
    h.run_steps(3);
    choose(&mut h, "Menu item", "Last page");
    ok(&mut h);
    let read = with_doc(&h, |d, o| d.link_named_action(o));
    assert!(read.is_named && read.safe);
    assert_eq!(read.name.as_deref(), Some("LastPage"));
    h.undo();
    h.run_steps(3);
    assert_eq!(link(&h).action, LinkAction::Uri("http://x.test".into()));
    assert!(!with_doc(&h, |d, o| d.link_named_action(o)).is_named);
}

#[test]
fn the_dialog_shows_the_article_action_a_link_already_has() {
    let mut h = harness();
    open_properties(&mut h);
    assert!(disabled(&h, Role::ComboBox, "Article"), "a web link leaves the article choice off");
    h.get_by_label("Go to an article thread").click();
    h.run_steps(3);
    assert!(!disabled(&h, Role::ComboBox, "Article"));
    choose(&mut h, "Article", "Second");
    ok(&mut h);
    open_properties(&mut h);
    assert!(!disabled(&h, Role::ComboBox, "Article"), "the link's own article action is the one chosen");
    assert!(disabled(&h, Role::ComboBox, "Zoom"), "and the page view is not");
    assert_eq!(h.state().dialogs.open_dialogs().len(), 1);
}

#[test]
fn a_file_to_open_is_a_relative_path_to_a_pdf_and_is_set_in_one_undo_step() {
    let mut h = harness();
    open_properties(&mut h);
    h.get_by_label("Open a PDF file").click();
    h.run_steps(3);
    // Nothing typed yet: OK changes nothing and the dialog stays.
    ok(&mut h);
    assert_eq!(h.state().dialogs.open_dialogs().len(), 1, "an empty path is not accepted");
    assert_eq!(link(&h).action, LinkAction::Uri("http://x.test".into()));
    h.query_all_by_role_and_label(Role::TextInput, "File to open").last().expect("the file field").click();
    h.run_steps(2);
    h.query_all_by_role_and_label(Role::TextInput, "File to open").last().expect("the file field").type_text("other.pdf");
    h.run_steps(2);
    h.query_all_by_role_and_label(Role::CheckBox, "Open in a new window").last().expect("the window choice").click();
    h.run_steps(2);
    ok(&mut h);
    let read = with_doc(&h, |d, o| d.link_launch_action(o));
    assert!(read.is_launch);
    assert_eq!(read.file.as_deref(), Some("other.pdf"));
    assert_eq!(read.new_window, Some(true));
    h.undo();
    h.run_steps(3);
    assert!(!with_doc(&h, |d, o| d.link_launch_action(o)).is_launch);
    assert_eq!(link(&h).action, LinkAction::Uri("http://x.test".into()));
}

#[test]
fn a_new_link_offers_the_web_and_page_actions_and_says_why_the_others_wait() {
    let mut h = harness();
    assert!(h.execute("edit.link"));
    h.run_steps(2);
    drag(&mut h, at(&h, 100.0, 40.0), at(&h, 170.0, 70.0));
    h.get_by_role_and_label(Role::Dialog, "Create Link");
    for radio in ["Go to an article thread", "Run a menu item", "Open a PDF file"] {
        assert!(disabled(&h, Role::RadioButton, radio), "{radio} needs a link that exists");
    }
    assert!(!disabled(&h, Role::RadioButton, "Go to a page view"));
    h.get_by_label("An article, menu item or file action can be set once the link exists: create it, then open its properties.");
}

#[test]
fn a_media_action_is_kept_unless_the_dialog_is_asked_to_take_it_out() {
    let mut h = harness_on(linked_pdf_with("/A << /S /Sound >>"));
    let media = |h: &Harness<'static, PdfCraftApp>| with_doc(h, |d, o| d.link_media_action(o).is_media);
    assert!(media(&h));
    open_properties(&mut h);
    ok(&mut h);
    assert!(media(&h), "OK alone keeps the action as it is");
    open_properties(&mut h);
    h.query_all_by_role_and_label(Role::CheckBox, "Remove this action").last().expect("the removal choice").click();
    h.run_steps(2);
    ok(&mut h);
    assert!(!media(&h), "the action was taken out");
    h.undo();
    h.run_steps(3);
    assert!(media(&h), "one undo puts it back");
}
