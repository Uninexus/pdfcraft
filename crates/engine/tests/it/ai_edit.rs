//! What the AI assistant may change: lists are checked before anything runs, run as undo steps,
//! and a list that fails half way leaves the document as it was.

use pdfcraft_engine::ai_edit::{self, Shape};
use pdfcraft_engine::{DocId, Session};
use serde_json::{Value, json};

fn doc(pages: usize) -> (Session, DocId) {
    let text = (1..=pages).map(|n| format!("Page number {n}")).collect::<Vec<_>>().join("\u{c}");
    let mut s = Session::new();
    let bytes = s.create_from_text("Doc", &text).unwrap();
    let id = s.open_new("doc.pdf", bytes).unwrap();
    (s, id)
}

fn call(tool: &str, args: Value) -> (String, Value) {
    (tool.to_owned(), args)
}

fn pages(s: &Session, id: DocId) -> usize {
    s.get(id).unwrap().info.pages.len()
}

#[test]
fn a_list_changes_the_document_in_order_and_undo_takes_it_all_back() {
    let (mut s, id) = doc(4);
    // After deleting page 2 the document has three pages, so page 3 is the last one.
    let list = [
        call("delete_pages", json!({ "pages": [2] })),
        call("rotate_pages", json!({ "pages": [3], "degrees": 90 })),
        call("set_document_info", json!({ "key": "Title", "value": "Hello" })),
    ];
    let done = ai_edit::apply(&mut s, id, &list).unwrap();
    assert_eq!(done.steps, 3);
    assert!(done.undo_steps >= 3, "{done:?}");
    assert_eq!(pages(&s, id), 3);
    for _ in 0..done.undo_steps {
        s.undo(id).unwrap();
    }
    assert_eq!(pages(&s, id), 4);
    assert!(s.undo(id).is_err(), "undo_steps counts every step");
}

#[test]
fn a_list_is_checked_against_the_document_as_earlier_calls_leave_it() {
    let (s, id) = doc(3);
    let shape = Shape::of(&s, id).unwrap();
    // Page 3 exists now but not after page 3 is deleted.
    let e = ai_edit::check(&shape, &[call("delete_pages", json!({ "pages": [3] })), call("rotate_pages", json!({ "pages": [3], "degrees": 90 }))])
        .unwrap_err();
    assert!(e.contains("step 2") && e.contains("page 3 does not exist"), "{e}");
    // A blank page added first makes page 4 real.
    ai_edit::check(&shape, &[call("insert_blank_page", json!({ "at": 4 })), call("rotate_pages", json!({ "pages": [4], "degrees": 90 }))]).unwrap();
}

#[test]
fn calls_outside_the_vocabulary_or_with_odd_arguments_are_refused_before_anything_runs() {
    let (mut s, id) = doc(2);
    for bad in [
        call("doc_save", json!({})),
        call("page_rotate", json!({ "pages": [1], "degrees": 90 })),
        call("rotate_pages", json!({ "pages": [1], "degrees": 45 })),
        call("rotate_pages", json!({ "pages": [1], "degrees": 90, "typo": 1 })),
        call("delete_pages", json!({ "pages": [1, 2] })),
        call("set_document_info", json!({ "key": "Producer", "value": "x" })),
        call("fill_form_field", json!({ "name": "A", "value": 7 })),
        call("add_text", json!({ "page": 9, "text": "x", "x": 1, "y": 1 })),
    ] {
        let list = [call("rotate_pages", json!({ "pages": [1], "degrees": 90 })), bad.clone()];
        assert!(ai_edit::apply(&mut s, id, &list).is_err(), "{bad:?}");
        assert!(s.undo(id).is_err(), "{bad:?}: nothing may run when a later call is refused");
    }
}

#[test]
fn a_call_that_fails_when_it_runs_undoes_the_ones_before_it() {
    let (mut s, id) = doc(2);
    // The check passes (a field name is any text); the engine refuses a field the form does not have.
    let list =
        [call("rotate_pages", json!({ "pages": [1], "degrees": 90 })), call("fill_form_field", json!({ "name": "NoSuchField", "value": "x" }))];
    let e = ai_edit::apply(&mut s, id, &list).unwrap_err();
    assert!(e.contains("step 2") && e.contains("undone"), "{e}");
    assert!(s.undo(id).is_err(), "the first call should have been undone");
}

#[test]
fn rotation_can_be_limited_to_landscape_pages_and_add_text_lands_on_the_page() {
    let (mut s, id) = doc(2);
    let done = ai_edit::apply(&mut s, id, &[call("add_text", json!({ "page": 2, "text": "Reviewed", "x": 50, "y": 50, "size": 14 }))]).unwrap();
    assert_eq!(done.steps, 1);
    assert_eq!(s.get(id).unwrap().added().iter().filter(|a| a.page == 1).count(), 1);
    // Both pages are portrait, so a landscape-only rotation matches nothing and is refused.
    let e = ai_edit::apply(&mut s, id, &[call("rotate_pages", json!({ "degrees": 90, "orientation": "landscape" }))]).unwrap_err();
    assert!(e.contains("no pages match"), "{e}");
}

#[test]
fn page_lists_read_as_ranges() {
    assert_eq!(ai_edit::page_list(&[2, 3, 4, 7, 0]), "1, 3-5, 8");
}
