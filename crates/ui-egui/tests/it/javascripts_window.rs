//! Edit all JavaScripts, the window (egui_kittest, no GPU): every script listed with its kind and
//! place, filters, selecting one, editing and applying it, deleting it, undo and redo, a list
//! gone stale, a document whose security forbids changes and scripts too long to edit. Nothing
//! the window does runs a script (the Mouse Up script would fill a field in).

use std::sync::{Arc, Mutex};

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use pdfcraft_ui_egui::PdfCraftApp;
use pdfcraft_ui_egui::control::{ControlClient, Reply};
use serde_json::{Value, json};

fn pdf_of(objs: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

/// A named script, a Will Save action, a text field with a Mouse Up script, and, when `open` is
/// given, an open action holding that code.
fn fixture(open: Option<&str>) -> Vec<u8> {
    let open_action = open.map(|code| format!(" /OpenAction << /S /JavaScript /JS ({code}) >>")).unwrap_or_default();
    pdf_of(&[
        format!("<< /Type /Catalog /Pages 2 0 R /Names << /JavaScript << /Names [(lib) << /S /JavaScript /JS (var lib = 1;) >>] >> >> /AA << /WS << /S /JavaScript /JS (willSave\\(\\);) >> >>{open_action} /AcroForm << /Fields [4 0 R] >> >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Annots [4 0 R] >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (log) /V () /Rect [20 250 180 270] /P 3 0 R /F 4 /A << /S /JavaScript /JS (getField\\('log'\\).value = 'ran';) >> >>".into(),
    ])
}

/// `bytes` with the editing permission (bit 4) taken away, opened with no password.
fn locked(bytes: Vec<u8>) -> Vec<u8> {
    let mut doc = pdfcraft_cos::Document::open(Arc::new(bytes)).unwrap();
    doc.set_encryption(&pdfcraft_cos::NewEncryption {
        algorithm: pdfcraft_cos::Algorithm::Aes256,
        user_password: "",
        owner_password: "owner",
        permissions: -1 ^ 8,
        encrypt_metadata: true,
        seed: [6; 32],
    })
    .unwrap();
    pdfcraft_cos::write_full(&doc, &Default::default()).unwrap()
}

fn harness(bytes: Vec<u8>) -> (Harness<'static, PdfCraftApp>, ControlClient) {
    let slot: Arc<Mutex<Option<ControlClient>>> = Arc::default();
    let s = slot.clone();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 1000.0)).build_eframe(move |cc| {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        *s.lock().unwrap() = Some(app.attach_control(&cc.egui_ctx));
        app.open_bytes("scripts.pdf", None, bytes).unwrap();
        app
    });
    h.run_steps(4);
    let client = slot.lock().unwrap().take().unwrap();
    (h, client)
}

fn call(h: &mut Harness<'static, PdfCraftApp>, c: &ControlClient, method: &str, params: Value) -> Reply {
    let rx = c.send(method, params);
    let mut reply = None;
    let answered = crate::support::step_until(h, |_| {
        reply = rx.try_recv().ok();
        reply.is_some()
    });
    match reply {
        Some(r) if answered => r,
        _ => panic!("{method}: no reply in 30 s"),
    }
}

fn ok(h: &mut Harness<'static, PdfCraftApp>, c: &ControlClient, method: &str, params: Value) -> Value {
    call(h, c, method, params).unwrap_or_else(|e| panic!("{method}: {e}"))
}

/// The window's state as an agent sees it.
fn state(h: &mut Harness<'static, PdfCraftApp>, c: &ControlClient) -> Value {
    h.run_steps(2);
    ok(h, c, "ui.state", json!({}))["javascripts"].clone()
}

fn open_window(h: &mut Harness<'static, PdfCraftApp>, c: &ControlClient) {
    ok(h, c, "ui.set", json!({"key":"dialog","value":"javascripts"}));
    h.run_steps(4);
}

/// The listed code at the trigger, straight from the engine.
fn code(h: &Harness<'static, PdfCraftApp>, trigger: &str) -> Option<String> {
    let app = h.state();
    let doc = app.session.get(app.active_ids().unwrap().1).unwrap();
    doc.list_javascripts().entries.into_iter().find(|e| e.trigger == trigger).and_then(|e| e.code)
}

fn bytes(h: &Harness<'static, PdfCraftApp>) -> Arc<Vec<u8>> {
    let app = h.state();
    app.session.get(app.active_ids().unwrap().1).unwrap().bytes.clone()
}

/// Nothing ran: the field the Mouse Up script would fill in is as it was.
fn nothing_ran(h: &mut Harness<'static, PdfCraftApp>) {
    let id = h.state().active_ids().unwrap().1;
    assert_eq!(h.state().session.get(id).unwrap().form.iter().find(|f| f.name == "log").unwrap().value.join(""), "");
}

/// Press a button the way a pointer does: it is hovered for a frame first.
fn press(h: &mut Harness<'static, PdfCraftApp>, label: &str) {
    h.get_by_label(label).hover();
    h.run_steps(2);
    h.get_by_label(label).click();
    h.run_steps(3);
}

fn type_into_editor(h: &mut Harness<'static, PdfCraftApp>, text: &str) {
    let editor = h.get_by_role(egui::accesskit::Role::MultilineTextInput);
    editor.focus();
    editor.type_text(text);
    h.run_steps(2);
}

#[test]
fn the_window_lists_every_script_with_its_kind_and_place() {
    let (mut h, c) = harness(fixture(Some("app.beep();")));
    assert_eq!(state(&mut h, &c)["open"], false);
    open_window(&mut h, &c);
    let s = state(&mut h, &c);
    assert_eq!(s["open"], true);
    assert_eq!(s["total"], 4);
    let entries = s["entries"].as_array().unwrap();
    let kinds: Vec<_> = entries.iter().map(|e| e["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["document", "open_action", "document_action", "field"], "{entries:?}");
    assert_eq!(entries[3]["first_line"], "getField('log').value = 'ran';");
    assert!(entries[3]["location"].as_str().unwrap().contains("log"));
    h.get_by_label_contains("Named script \"lib\"");
    h.get_by_label_contains("Field \"log\", page 1");
    h.get_by_label_contains("The document's open action");
    assert!(h.query_by_label("This document has no JavaScripts.").is_none());
    nothing_ran(&mut h);
}

#[test]
fn selecting_editing_and_applying_changes_only_that_script_and_undo_and_redo_follow() {
    let (mut h, c) = harness(fixture(None));
    open_window(&mut h, &c);
    h.get_by_label_contains("Mouse Up").click();
    h.run_steps(3);
    let s = state(&mut h, &c);
    assert_eq!(s["selected"]["trigger"], "Mouse Up");
    assert_eq!(s["dirty"], false);
    assert_eq!(s["can_apply"], false, "nothing to apply yet");
    assert!(h.get_by_label("Apply").accesskit_node().is_disabled());
    let original = bytes(&h);
    let will_save = code(&h, "Will Save");

    type_into_editor(&mut h, "CHANGED");
    let s = state(&mut h, &c);
    assert_eq!(s["dirty"], true);
    assert_eq!(s["can_apply"], true);
    assert_eq!(bytes(&h), original, "typing changes nothing until Apply");
    press(&mut h, "Apply");

    let changed = code(&h, "Mouse Up").unwrap();
    assert!(changed.contains("CHANGED") && changed.contains("getField"), "{changed}");
    assert_eq!(code(&h, "Will Save"), will_save, "no other script changed");
    let app = h.state();
    assert_eq!(app.session.get(app.active_ids().unwrap().1).unwrap().can_undo(), Some("Edit JavaScript"));
    let s = state(&mut h, &c);
    assert_eq!(s["dirty"], false, "the editor holds the applied code");
    assert_eq!(s["selected"]["trigger"], "Mouse Up");
    assert!(s["notice"].as_str().unwrap().starts_with("Applied"), "{s}");
    nothing_ran(&mut h);

    // Undo puts the script back, and the window follows without being told.
    h.state_mut().undo();
    h.run_steps(3);
    assert_eq!(bytes(&h), original, "undo restores the exact bytes");
    let s = state(&mut h, &c);
    assert_eq!(s["editor_bytes"], "getField('log').value = 'ran';".len());
    assert!(s["notice"].as_str().unwrap().contains("undo"), "{s}");
    h.state_mut().redo();
    h.run_steps(3);
    assert_eq!(code(&h, "Mouse Up").unwrap(), changed);
    assert_eq!(state(&mut h, &c)["editor_bytes"], changed.len());
    nothing_ran(&mut h);
}

#[test]
fn the_text_and_kind_filters_narrow_the_list() {
    let (mut h, c) = harness(fixture(Some("app.beep();")));
    open_window(&mut h, &c);
    ok(&mut h, &c, "ui.set", json!({"key":"javascripts-filter","value":"WILLSAVE"}));
    let s = state(&mut h, &c);
    assert_eq!((s["shown"].as_u64(), s["total"].as_u64()), (Some(1), Some(4)), "matches the code, ignoring case");
    assert_eq!(s["entries"][0]["kind"], "document_action");
    h.get_by_label_contains("Showing 1 of 4 scripts");
    ok(&mut h, &c, "ui.set", json!({"key":"javascripts-filter","value":"no such code"}));
    assert_eq!(state(&mut h, &c)["shown"], 0);
    h.get_by_label("No script matches the filter.");
    ok(&mut h, &c, "ui.set", json!({"key":"javascripts-filter","value":""}));
    ok(&mut h, &c, "ui.set", json!({"key":"javascripts-kind","value":"field"}));
    let s = state(&mut h, &c);
    assert_eq!(s["shown"], 1);
    assert_eq!(s["entries"][0]["kind"], "field");
    assert!(call(&mut h, &c, "ui.set", json!({"key":"javascripts-kind","value":"nonsense"})).is_err());
    ok(&mut h, &c, "ui.set", json!({"key":"javascripts-kind","value":"all"}));
    assert_eq!(state(&mut h, &c)["shown"], 4);
}

#[test]
fn delete_asks_first_removes_one_script_and_undo_brings_it_back() {
    let (mut h, c) = harness(fixture(None));
    open_window(&mut h, &c);
    h.get_by_label_contains("Will Save").click();
    h.run_steps(3);
    let original = bytes(&h);
    press(&mut h, "Delete");
    assert_eq!(state(&mut h, &c)["confirm_delete"], true);
    assert_eq!(bytes(&h), original, "asking changes nothing");
    press(&mut h, "Keep");
    assert_eq!(state(&mut h, &c)["confirm_delete"], false);
    assert_eq!(bytes(&h), original);
    press(&mut h, "Delete");
    press(&mut h, "Delete script");
    assert_eq!(code(&h, "Will Save"), None, "the script is gone");
    let s = state(&mut h, &c);
    assert_eq!((s["total"].as_u64(), s["shown"].as_u64()), (Some(2), Some(2)), "the list and its rows are current at once");
    assert!(s["selected"].is_null(), "nothing selected after a delete");
    let app = h.state();
    assert_eq!(app.session.get(app.active_ids().unwrap().1).unwrap().can_undo(), Some("Delete JavaScript"));
    h.state_mut().undo();
    h.run_steps(3);
    assert_eq!(bytes(&h), original);
    assert_eq!(state(&mut h, &c)["total"], 3, "the list follows the undo");
    nothing_ran(&mut h);
}

#[test]
fn a_script_changed_behind_the_windows_back_is_read_again_not_overwritten() {
    let (mut h, c) = harness(fixture(None));
    open_window(&mut h, &c);
    h.get_by_label_contains("Mouse Up").click();
    h.run_steps(3);
    type_into_editor(&mut h, "UNSAVED");
    // Another edit (the headless tools, say) changes the same script.
    let (id, fp) = {
        let app = h.state();
        let e =
            app.session.get(app.active_ids().unwrap().1).unwrap().list_javascripts().entries.into_iter().find(|e| e.trigger == "Mouse Up").unwrap();
        (e.id, e.fingerprint)
    };
    let doc = h.state().active_ids().unwrap().1;
    h.state_mut().session.apply(doc, pdfcraft_engine::Edit::SetJavaScript { id, expect: fp, code: "other();".into() }).unwrap();
    h.run_steps(3);
    let s = state(&mut h, &c);
    assert_eq!(s["editor_bytes"], "other();".len(), "the editor shows what the document holds now");
    assert_eq!(s["dirty"], false);
    let notice = s["notice"].as_str().unwrap();
    assert!(notice.contains("changed") && notice.contains("unsaved changes"), "{notice}");
    assert_eq!(code(&h, "Mouse Up").as_deref(), Some("other();"));
}

#[test]
fn a_document_that_forbids_changes_is_shown_read_only_with_the_reason() {
    let (mut h, c) = harness(locked(fixture(None)));
    open_window(&mut h, &c);
    let s = state(&mut h, &c);
    assert_eq!(s["total"], 3, "the scripts are still listed and readable");
    assert_eq!(s["read_only"], true);
    h.get_by_label_contains("security settings don't allow changes");
    h.get_by_label_contains("Mouse Up").click();
    h.run_steps(3);
    assert_eq!(state(&mut h, &c)["selected"]["trigger"], "Mouse Up");
    assert!(h.get_by_label("Apply").accesskit_node().is_disabled());
    assert!(h.get_by_label("Delete").accesskit_node().is_disabled());
    let before = bytes(&h);
    // The editor refuses typing, so there is nothing to apply.
    type_into_editor(&mut h, "NOPE");
    let s = state(&mut h, &c);
    assert_eq!((s["dirty"].as_bool(), s["can_apply"].as_bool()), (Some(false), Some(false)), "{s}");
    assert_eq!(bytes(&h), before);
}

#[test]
fn a_script_too_long_to_edit_is_cut_cannot_be_applied_and_can_still_be_deleted() {
    // Past the editor's limit but within what is listed, and past what is listed.
    for (size, listing_cut) in [(100 * 1024, false), (2 << 20, true)] {
        let (mut h, c) = harness(fixture(Some(&"x".repeat(size))));
        open_window(&mut h, &c);
        let s = state(&mut h, &c);
        assert_eq!(s["total"], 4);
        assert_eq!(s["truncated"], u64::from(listing_cut));
        h.get_by_label_contains("The document's open action").click();
        h.run_steps(3);
        let s = state(&mut h, &c);
        assert_eq!(s["selected"]["kind"], "open_action");
        assert_eq!(s["editor_cut"], true, "{size}");
        assert!(s["editor_bytes"].as_u64().unwrap() <= 64 * 1024, "the editor never holds more than 64 KiB");
        assert_eq!(s["can_apply"], false);
        assert!(h.get_by_label("Apply").accesskit_node().is_disabled());
        h.get_by_label_contains("It can still be deleted");
        // Typing is refused too.
        let before = bytes(&h);
        type_into_editor(&mut h, "NOPE");
        let s = state(&mut h, &c);
        assert_eq!((s["dirty"].as_bool(), s["can_apply"].as_bool()), (Some(false), Some(false)), "{s}");
        assert_eq!(bytes(&h), before);
        // Deleting needs no editing: it goes after the confirmation.
        press(&mut h, "Delete");
        press(&mut h, "Delete script");
        assert_eq!(state(&mut h, &c)["total"], 3, "{size}");
        nothing_ran(&mut h);
    }
}

#[test]
fn an_unreadable_open_action_is_listed_and_can_be_given_code() {
    let bytes = pdf_of(&[
        "<< /Type /Catalog /Pages 2 0 R /OpenAction << /S /JavaScript >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
    ]);
    let (mut h, c) = harness(bytes);
    open_window(&mut h, &c);
    h.get_by_label_contains("(no readable code)").click();
    h.run_steps(3);
    h.get_by_label_contains("no readable code. Applying writes");
    type_into_editor(&mut h, "app.beep();");
    press(&mut h, "Apply");
    let app = h.state();
    let entries = app.session.get(app.active_ids().unwrap().1).unwrap().list_javascripts().entries;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].code.as_deref(), Some("app.beep();"));
}

#[test]
fn the_command_is_in_the_edit_menu_and_the_palette_and_needs_a_document() {
    let spec = pdfcraft_engine::commands::command("tools.javascripts").unwrap();
    assert_eq!(spec.menu, Some("Edit"));
    let mut app = PdfCraftApp::new();
    app.set_option("language", "en").unwrap();
    assert!(!app.execute("tools.javascripts"), "no document, no window");
    assert!(app.dialog.is_none());
    app.open_bytes("scripts.pdf", None, fixture(None)).unwrap();
    assert!(app.execute("tools.javascripts"));
    assert_eq!(app.dialog, Some(pdfcraft_ui_egui::Dialog::JavaScripts));
}

#[test]
fn the_window_closes_and_forgets_the_list() {
    let (mut h, c) = harness(fixture(None));
    open_window(&mut h, &c);
    h.get_by_label_contains("Mouse Up").click();
    h.run_steps(3);
    press(&mut h, "Close");
    let s = state(&mut h, &c);
    assert_eq!((s["open"].as_bool(), s["total"].as_u64(), s["selected"].is_null()), (Some(false), Some(0), true));
}

fn key(h: &mut Harness<'static, PdfCraftApp>, k: egui::Key) {
    h.key_press(k);
    h.run_steps(3);
}

#[test]
fn unsaved_typing_is_never_thrown_away_without_asking() {
    let (mut h, c) = harness(fixture(None));
    open_window(&mut h, &c);
    h.get_by_label_contains("Mouse Up").click();
    h.run_steps(3);
    type_into_editor(&mut h, "TYPED");
    let typed = state(&mut h, &c)["editor_bytes"].clone();
    // Another row: asks first, keeps the typing and the script.
    h.get_by_label_contains("Will Save").click();
    h.run_steps(3);
    let s = state(&mut h, &c);
    assert_eq!((s["confirm_discard"].as_bool(), s["dirty"].as_bool()), (Some(true), Some(true)), "{s}");
    assert_eq!(s["selected"]["trigger"], "Mouse Up");
    assert_eq!(s["editor_bytes"], typed);
    // The control channel refuses to move the selection under typing.
    let will_save = s["entries"].as_array().unwrap().iter().find(|e| e["trigger"] == "Will Save").unwrap()["id"].clone();
    assert!(call(&mut h, &c, "ui.set", json!({"key":"javascripts-select","value":will_save})).is_err());
    press(&mut h, "Keep editing");
    let s = state(&mut h, &c);
    assert_eq!((s["confirm_discard"].as_bool(), s["dirty"].as_bool()), (Some(false), Some(true)));
    // Escape and Close ask too, and the window stays.
    key(&mut h, egui::Key::Escape);
    let s = state(&mut h, &c);
    assert_eq!((s["open"].as_bool(), s["confirm_discard"].as_bool(), s["dirty"].as_bool()), (Some(true), Some(true), Some(true)), "{s}");
    press(&mut h, "Keep editing");
    press(&mut h, "Close");
    let s = state(&mut h, &c);
    assert_eq!((s["open"].as_bool(), s["confirm_discard"].as_bool()), (Some(true), Some(true)), "{s}");
    // Discard closes it, and the document was never touched.
    let before = bytes(&h);
    press(&mut h, "Discard");
    assert_eq!(state(&mut h, &c)["open"], false);
    assert_eq!(bytes(&h), before);
    // Discard on a row change moves to the other script with its own code.
    open_window(&mut h, &c);
    h.get_by_label_contains("Mouse Up").click();
    h.run_steps(3);
    type_into_editor(&mut h, "TYPED");
    h.get_by_label_contains("Will Save").click();
    h.run_steps(3);
    press(&mut h, "Discard");
    let s = state(&mut h, &c);
    assert_eq!((s["selected"]["trigger"].as_str(), s["dirty"].as_bool()), (Some("Will Save"), Some(false)), "{s}");
}

#[test]
fn names_from_the_file_are_cut_everywhere_they_are_shown() {
    let long = "n".repeat(400);
    let bytes = pdf_of(&[
        format!("<< /Type /Catalog /Pages 2 0 R /Names << /JavaScript << /Names [({long}) << /S /JavaScript /JS (var a;) >>] >> >> >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
    ]);
    let (mut h, c) = harness(bytes);
    open_window(&mut h, &c);
    let s = state(&mut h, &c);
    let location = s["entries"][0]["location"].as_str().unwrap().to_string();
    assert!(location.contains(&"n".repeat(200)) && !location.contains(&"n".repeat(201)) && location.contains('…'), "{location}");
    h.get_by_label_contains("Named script");
    h.get_by_label_contains("Named script").click();
    h.run_steps(3);
    let s = state(&mut h, &c);
    assert!(!s["selected"]["location"].as_str().unwrap().contains(&"n".repeat(201)));
    // The filter is bounded too.
    ok(&mut h, &c, "ui.set", json!({"key":"javascripts-filter","value":"x".repeat(1000)}));
    assert_eq!(state(&mut h, &c)["filter"].as_str().unwrap().chars().count(), 200);
}
