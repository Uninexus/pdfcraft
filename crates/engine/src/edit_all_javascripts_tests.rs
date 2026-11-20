//! Edit all JavaScripts: every script listed in one place and edited by id, as one undoable
//! edit that never runs anything.

use std::sync::Arc;

use crate::{DocId, Edit, EditError, Session};

fn pdf_of(objs: &[&str]) -> Vec<u8> {
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

/// A document with a named script, a Will Save action and a field with a Mouse Up script.
fn session() -> (Session, DocId) {
    let bytes = pdf_of(&[
        "<< /Type /Catalog /Pages 2 0 R /Names << /JavaScript << /Names [(lib) << /S /JavaScript /JS (var lib = 1;) >>] >> >> /AA << /WS << /S /JavaScript /JS (willSave\\(\\);) >> >> /AcroForm << /Fields [4 0 R] >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>",
        "<< /Type /Page /Parent 2 0 R /Annots [4 0 R] >>",
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (log) /V () /Rect [20 250 180 270] /P 3 0 R /F 4 /A << /S /JavaScript /JS (getField\\('log'\\).value = 'ran';) >> >>",
    ]);
    let mut s = Session::new();
    let id = s.open("scripts.pdf", None, Arc::new(bytes), None).unwrap();
    (s, id)
}

/// The id, code and fingerprint of the script run on `trigger`.
fn entry(s: &Session, id: DocId, trigger: &str) -> (String, Option<String>, String) {
    let inv = s.get(id).unwrap().list_javascripts();
    let e = inv.entries.into_iter().find(|e| e.trigger == trigger).unwrap_or_else(|| panic!("no {trigger} script"));
    (e.id, e.code, e.fingerprint)
}

#[test]
fn a_script_is_edited_in_one_undoable_step_and_nothing_runs() {
    let (mut s, id) = session();
    let (sid, code, fp) = entry(&s, id, "Mouse Up");
    assert_eq!(code.as_deref(), Some("getField('log').value = 'ran';"));
    let original = s.get(id).unwrap().bytes.clone();
    s.apply(id, Edit::SetJavaScript { id: sid.clone(), expect: fp, code: "app.alert('changed');".into() }).unwrap();
    assert_eq!(entry(&s, id, "Mouse Up").1.as_deref(), Some("app.alert('changed');"));
    assert_eq!(s.get(id).unwrap().can_undo(), Some("Edit JavaScript"));
    // Editing a script does not run it: the field is as it was and the console is empty.
    let doc = s.get(id).unwrap();
    assert_eq!(doc.form.iter().find(|f| f.name == "log").unwrap().value.join(""), "");
    let out = s.take_js_output(id);
    assert!(out.errors.is_empty() && out.alerts.is_empty() && out.console.is_empty());
    assert_eq!(s.undo(id).unwrap().as_str(), "Edit JavaScript");
    assert_eq!(s.get(id).unwrap().bytes, original, "undo restores the exact bytes");
}

#[test]
fn a_script_is_removed_in_one_undoable_step() {
    let (mut s, id) = session();
    let (sid, _, fp) = entry(&s, id, "Will Save");
    s.apply(id, Edit::RemoveJavaScript { id: sid, expect: fp }).unwrap();
    assert!(s.get(id).unwrap().list_javascripts().entries.iter().all(|e| e.trigger != "Will Save"));
    assert_eq!(s.get(id).unwrap().can_undo(), Some("Delete JavaScript"));
    s.undo(id).unwrap();
    assert!(entry(&s, id, "Will Save").1.is_some());
}

#[test]
fn an_id_that_is_gone_is_refused_and_changes_nothing() {
    let (mut s, id) = session();
    let (sid, _, fp) = entry(&s, id, "Will Save");
    s.apply(id, Edit::RemoveJavaScript { id: sid.clone(), expect: fp.clone() }).unwrap();
    let depth = s.get(id).unwrap().undo_depth();
    for edit in [Edit::SetJavaScript { id: sid.clone(), expect: fp.clone(), code: "x".into() }, Edit::RemoveJavaScript { id: sid, expect: fp }] {
        let e = s.apply(id, edit).unwrap_err();
        assert!(matches!(e, EditError::Form(_)) || e.to_string().contains("list the JavaScripts again"), "{e}");
        assert!(e.to_string().contains("list the JavaScripts again"), "{e}");
    }
    assert_eq!(s.get(id).unwrap().undo_depth(), depth);
}

/// An id kept across an edit and its undo or redo can lead to another script; the fingerprint
/// stops an edit from landing on it.
#[test]
fn undo_and_redo_shifting_what_an_id_leads_to_is_caught() {
    let bytes = pdf_of(&[
        "<< /Type /Catalog /Pages 2 0 R /OpenAction << /S /JavaScript /JS (var first = 1;) /Next << /S /JavaScript /JS (var second = 2;) >> >> >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>",
        "<< /Type /Page /Parent 2 0 R >>",
    ]);
    let mut s = Session::new();
    let id = s.open("open.pdf", None, Arc::new(bytes), None).unwrap();
    let (oid, _, first) = entry(&s, id, "Document open");
    s.apply(id, Edit::RemoveJavaScript { id: oid.clone(), expect: first.clone() }).unwrap();
    // Redo state: the id now leads to the second script, so the old fingerprint is refused.
    let e = s.apply(id, Edit::SetJavaScript { id: oid.clone(), expect: first.clone(), code: "x".into() }).unwrap_err();
    assert!(e.to_string().contains("changed; list the JavaScripts again"), "{e}");
    s.undo(id).unwrap();
    // Undone, the id leads to the first again, so now the second one's fingerprint is refused.
    let second = {
        let inv = s.get(id).unwrap().list_javascripts();
        inv.entries.into_iter().find(|e| e.code.as_deref() == Some("var second = 2;")).unwrap().fingerprint
    };
    assert!(s.apply(id, Edit::SetJavaScript { id: oid.clone(), expect: second, code: "x".into() }).is_err());
    s.apply(id, Edit::SetJavaScript { id: oid, expect: first, code: "var first = 3;".into() }).unwrap();
}

/// Find and replace over every script is one undoable edit that runs nothing, needs the
/// permission to modify, and writes nothing when it is refused.
#[test]
fn replace_in_scripts_is_one_undoable_step_needing_modify_permission() {
    let (mut s, id) = session();
    let original = s.get(id).unwrap().bytes.clone();
    let edit = Edit::ReplaceInJavaScripts { find: "ran".into(), replace: "walked".into(), match_case: true };
    s.apply(id, edit.clone()).unwrap();
    assert_eq!(entry(&s, id, "Mouse Up").1.as_deref(), Some("getField('log').value = 'walked';"));
    assert_eq!(s.get(id).unwrap().can_undo(), Some("Replace in JavaScripts"));
    // Nothing ran: the field is as it was and the console is empty.
    assert_eq!(s.get(id).unwrap().form.iter().find(|f| f.name == "log").unwrap().value.join(""), "");
    let out = s.take_js_output(id);
    assert!(out.errors.is_empty() && out.alerts.is_empty() && out.console.is_empty());
    s.undo(id).unwrap();
    assert_eq!(entry(&s, id, "Mouse Up").1.as_deref(), Some("getField('log').value = 'ran';"));
    assert_eq!(s.get(id).unwrap().bytes, original);

    // Refused: an empty find changes nothing and leaves no step.
    let depth = s.get(id).unwrap().undo_depth();
    let e = s.apply(id, Edit::ReplaceInJavaScripts { find: String::new(), replace: "x".into(), match_case: true }).unwrap_err();
    assert!(e.to_string().contains("the text to find is empty"), "{e}");
    assert_eq!(s.get(id).unwrap().undo_depth(), depth);

    let none = pdfcraft_cos::Permissions { bits: 0, owner: false, revision: 3 };
    assert_eq!(crate::edit::check_permission(&edit, &none), Err(EditError::NotPermitted("changes to the document")));
}

/// A replace that finds nothing is not an edit: no undo step and nothing unsaved.
#[test]
fn a_replace_that_finds_nothing_leaves_no_step_and_nothing_unsaved() {
    let (mut s, id) = session();
    let depth = s.get(id).unwrap().undo_depth();
    s.apply(id, Edit::ReplaceInJavaScripts { find: "no such text".into(), replace: "x".into(), match_case: true }).unwrap();
    let doc = s.get(id).unwrap();
    assert_eq!((doc.undo_depth(), doc.dirty), (depth, false));
    // Case ignored, a find equal to its replacement but for case changes no text either.
    s.apply(id, Edit::ReplaceInJavaScripts { find: "WILLSAVE".into(), replace: "willSave".into(), match_case: false }).unwrap();
    let doc = s.get(id).unwrap();
    assert_eq!((doc.undo_depth(), doc.dirty), (depth, false));
}
