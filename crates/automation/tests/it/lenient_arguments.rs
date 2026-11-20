//! An argument that is present but of the wrong type is refused, not read as if it were absent.
//!
//! `form_set_actions` read a `reset`, `show` or `hide` list by keeping its strings and treating a
//! non-array as empty, and an empty `reset` list means every field; `doc_optimize` ignored a
//! wrongly typed `color` or `gray` setting; `form_set_props` ignored a wrongly typed `format`
//! setting and `action_run` read a non-string step `arg` as an empty one.

use pdfcraft_automation::{Automation, Content, ToolError};
use serde_json::{Value, json};

/// A one-page 200 x 300 PDF with an empty content stream.
fn pdf() -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_string(),
        "<< /Length 0 >>\nstream\n\nendstream".to_string(),
    ];
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

fn json_of(content: Vec<Content>) -> Value {
    match content.into_iter().next() {
        Some(Content::Json(v)) => v,
        _ => panic!("expected JSON"),
    }
}

fn open(test: &str) -> (Automation, u64, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("pdfcraft-lenient-args-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("in.pdf"), pdf()).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap();
    let doc = json_of(a.call("doc_open", &json!({ "path": "in.pdf" })).unwrap())["doc"].as_u64().unwrap();
    (a, doc, dir)
}

/// Every case must be refused with exactly its message; returns the cases that were not.
fn not_refused(a: &mut Automation, cases: Vec<(&str, Value, String)>) -> Vec<String> {
    let mut accepted = Vec::new();
    for (tool, args, want) in cases {
        match a.call(tool, &args) {
            Err(ToolError::InvalidArgs(m)) if m == want => {}
            other => accepted.push(format!("{tool} {args}: {other:?}")),
        }
    }
    accepted
}

fn document(a: &mut Automation, doc: u64) -> Value {
    json_of(a.call("doc_info", &json!({ "doc": doc })).unwrap())["document"].clone()
}

#[test]
fn a_field_list_that_is_not_an_array_of_strings_is_refused_and_the_actions_stay() {
    let (mut a, doc, _dir) = open("actions");
    a.call("form_add_field", &json!({ "doc": doc, "page": 1, "type": "text", "name": "f", "rect": [10, 10, 60, 30] })).unwrap();
    a.call("form_set_actions", &json!({ "doc": doc, "field": "f", "actions": [{ "trigger": "mouse_up", "menu": "Print" }] })).unwrap();
    let before = (document(&mut a, doc), json_of(a.call("form_actions", &json!({ "doc": doc, "field": "f" })).unwrap()));
    let want = |k: &str| format!("{k} must be an array of strings");
    let one = |item: Value| json!({ "doc": doc, "field": "f", "actions": [item] });
    let cases = vec![
        ("form_set_actions", one(json!({ "trigger": "mouse_up", "reset": ["a", 5] })), want("reset")),
        ("form_set_actions", one(json!({ "trigger": "mouse_up", "reset": "a" })), want("reset")),
        ("form_set_actions", one(json!({ "trigger": "mouse_up", "reset": {} })), want("reset")),
        ("form_set_actions", one(json!({ "trigger": "mouse_up", "show": ["a", null] })), want("show")),
        ("form_set_actions", one(json!({ "trigger": "mouse_up", "hide": "b" })), want("hide")),
        // A bad item after a good one changes nothing either.
        (
            "form_set_actions",
            json!({ "doc": doc, "field": "f", "actions": [{ "trigger": "mouse_down", "menu": "Print" }, { "trigger": "mouse_up", "hide": [1] }] }),
            want("hide"),
        ),
    ];
    let accepted = not_refused(&mut a, cases);
    assert!(accepted.is_empty(), "these calls were not refused with the expected message: {accepted:#?}");
    let after = (document(&mut a, doc), json_of(a.call("form_actions", &json!({ "doc": doc, "field": "f" })).unwrap()));
    assert_eq!(before, after, "a refused call leaves the document and the field's actions as they were");
}

#[test]
fn field_lists_of_strings_still_work() {
    let (mut a, doc, _dir) = open("actions-ok");
    a.call("form_add_field", &json!({ "doc": doc, "page": 1, "type": "text", "name": "f", "rect": [10, 10, 60, 30] })).unwrap();
    let items = json!([
        { "trigger": "mouse_up", "reset": ["a", "b"] },
        { "trigger": "mouse_down", "reset": [] },
        { "trigger": "mouse_enter", "show": ["a"] },
        { "trigger": "mouse_exit", "hide": ["b"] },
    ]);
    let r = json_of(a.call("form_set_actions", &json!({ "doc": doc, "field": "f", "actions": items })).unwrap());
    let found = r["actions"].to_string();
    for want in [r#""reset":["a","b"]"#, r#""reset":[]"#, r#""show":["a"]"#, r#""hide":["b"]"#] {
        assert!(found.contains(want), "{want} missing from {found}");
    }
}

#[test]
fn a_wrongly_typed_image_setting_is_refused_and_nothing_is_written() {
    let (mut a, doc, dir) = open("optimize");
    let case = |args: Value, want: &str| {
        let mut o = json!({ "doc": doc, "path": "out.pdf" });
        o.as_object_mut().unwrap().extend(args.as_object().unwrap().clone());
        ("doc_optimize", o, want.to_string())
    };
    let shape = "must be an object: { downsample, ppi, above_ppi, compression: jpeg|zip|retain, quality }";
    let keys = "(downsample, ppi, above_ppi, compression, quality)";
    let cases = vec![
        case(json!({ "color": { "ppi": "x" } }), r#"color ppi must be a number, not "x""#),
        case(json!({ "color": { "above_ppi": true } }), "color above_ppi must be a number, not true"),
        case(json!({ "gray": { "downsample": "yes" } }), r#"gray downsample must be true or false, not "yes""#),
        case(json!({ "color": { "compression": 5 } }), "unknown color compression 5 (jpeg, zip, retain)"),
        case(json!({ "gray": { "compression": "lzw" } }), r#"unknown gray compression "lzw" (jpeg, zip, retain)"#),
        case(
            json!({ "color": { "compression": "jpeg", "quality": "high" } }),
            r#"color quality must be a whole number (clamped to 1 to 100), not "high""#,
        ),
        case(json!({ "gray": "fast" }), &format!("gray {shape}")),
        case(json!({ "color": [1] }), &format!("color {shape}")),
        case(json!({ "color": { "ppis": 72 } }), &format!(r#"unknown color key "ppis" {keys}"#)),
    ];
    let accepted = not_refused(&mut a, cases);
    assert!(accepted.is_empty(), "these calls were not refused with the expected message: {accepted:#?}");
    assert!(!dir.join("out.pdf").exists(), "a refused call writes no file");
}

#[test]
fn image_settings_of_the_right_types_still_work() {
    let (mut a, doc, dir) = open("optimize-ok");
    let r = a.call(
        "doc_optimize",
        &json!({
            "doc": doc, "path": "out.pdf",
            "color": { "downsample": true, "ppi": 72, "above_ppi": 100, "compression": "jpeg", "quality": 50 },
            "gray": { "ppi": null, "compression": "zip" },
        }),
    );
    assert!(r.is_ok(), "{r:?}");
    assert!(dir.join("out.pdf").exists());
}

#[test]
fn a_wrongly_typed_format_setting_is_refused_and_the_field_stays() {
    let (mut a, doc, _dir) = open("format");
    a.call("form_add_field", &json!({ "doc": doc, "page": 1, "type": "text", "name": "f", "rect": [10, 10, 60, 30] })).unwrap();
    let before = document(&mut a, doc);
    let set = |format: Value| json!({ "doc": doc, "field": "f", "format": format });
    let cases = vec![
        ("form_set_props", set(json!({ "type": "number", "decimals": "two" })), r#"format decimals must be a whole number, not "two""#.to_string()),
        ("form_set_props", set(json!({ "type": "number", "separator": -1 })), "format separator must be a whole number, not -1".to_string()),
        ("form_set_props", set(json!({ "type": "number", "currency": 5 })), "format currency must be a string, not 5".to_string()),
        (
            "form_set_props",
            set(json!({ "type": "number", "currency_after": "yes" })),
            r#"format currency_after must be true or false, not "yes""#.to_string(),
        ),
        ("form_set_props", set(json!({ "type": "percent", "decimals": 1.5 })), "format decimals must be a whole number, not 1.5".to_string()),
        ("form_set_props", set(json!({ "type": "date", "pattern": 5 })), "format pattern must be a string, not 5".to_string()),
    ];
    let accepted = not_refused(&mut a, cases);
    assert!(accepted.is_empty(), "these calls were not refused with the expected message: {accepted:#?}");
    assert_eq!(before, document(&mut a, doc), "a refused call leaves the document as it was");
    let ok = a.call("form_set_props", &set(json!({ "type": "number", "decimals": 3, "currency": "$", "currency_after": null })));
    assert!(ok.is_ok(), "{ok:?}");
}

#[test]
fn a_step_arg_that_is_not_a_string_is_refused_before_any_folder_is_made() {
    let (mut a, _doc, dir) = open("steps");
    let cases = vec![(
        "action_run",
        json!({ "paths": ["in.pdf"], "folder": "out", "steps": [{ "step": "add_watermark", "arg": 5 }] }),
        "arg must be a string".to_string(),
    )];
    let accepted = not_refused(&mut a, cases);
    assert!(accepted.is_empty(), "these calls were not refused with the expected message: {accepted:#?}");
    assert!(!dir.join("out").exists(), "a refused call makes no output folder");
}

/// A JSON null is the same as a left-out argument in the Args helpers, and `js_run` read its
/// `answers` and `field_id` around them: a null was refused as the wrong type.
#[test]
fn a_null_answers_or_field_id_is_left_out_and_a_wrong_one_is_refused_by_name() {
    let (mut a, doc, dir) = open("null-js-run");
    let ran = json_of(a.call("js_run", &json!({ "doc": doc, "script": "1 + 1", "answers": null, "field_id": null })).unwrap());
    assert_eq!(ran["error"], Value::Null, "{ran}");
    assert_eq!(ran["result"], json!("2"), "{ran}");
    let cases = vec![
        ("js_run", json!({ "doc": doc, "script": "1", "answers": "yes" }), "answers must be an array of at most 32 strings or null".to_string()),
        ("js_run", json!({ "doc": doc, "script": "1", "answers": [1] }), "answers must contain strings or null".to_string()),
        ("js_run", json!({ "doc": doc, "script": "1", "field_id": "x" }), "field_id must be an integer between 0 and 281474976710655".to_string()),
        ("js_run", json!({ "doc": doc, "script": "1", "field_id": -1 }), "field_id must be an integer between 0 and 281474976710655".to_string()),
    ];
    assert_eq!(not_refused(&mut a, cases), Vec::<String>::new());
    let _ = std::fs::remove_dir_all(&dir);
}

/// `form_set_image` read `field_id` with the raw `get`, so a null was refused as the wrong type.
#[test]
fn a_null_field_id_of_form_set_image_is_left_out_and_a_wrong_one_is_refused_by_name() {
    let (mut a, doc, dir) = open("null-set-image");
    std::fs::write(dir.join("picture.gif"), pdfcraft_xfa::fixtures::TINY_GIF).unwrap();
    a.call("form_add_field", &json!({ "doc": doc, "page": 1, "type": "image", "name": "pic", "rect": [10, 10, 60, 30] })).unwrap();
    a.call("form_set_image", &json!({ "doc": doc, "field": "pic", "field_id": null, "path": "picture.gif" })).unwrap();
    let wrong = "field_id must be an integer between 0 and 281474976710655".to_string();
    let cases = vec![
        ("form_set_image", json!({ "doc": doc, "field": "pic", "field_id": "x", "path": "picture.gif" }), wrong.clone()),
        ("form_set_image", json!({ "doc": doc, "field": "pic", "field_id": -1, "path": "picture.gif" }), wrong),
    ];
    assert_eq!(not_refused(&mut a, cases), Vec::<String>::new());
    let _ = std::fs::remove_dir_all(&dir);
}
