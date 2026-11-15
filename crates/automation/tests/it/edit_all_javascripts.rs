//! Edit all JavaScripts through the tool table: `doc_javascripts` lists every script of a
//! document and `doc_javascript_edit` changes or removes one by id, undoably and without
//! running anything, whatever hostile structure the document holds.

use pdfcraft_automation::{Automation, Content};
use serde_json::{Value, json};

fn tool(a: &mut Automation, name: &str, args: Value) -> Value {
    match a.call(name, &args).unwrap().remove(0) {
        Content::Json(value) => value,
        other => panic!("expected JSON: {other:?}"),
    }
}

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

fn open(name: &str, bytes: Vec<u8>) -> (Automation, u64) {
    let dir = std::env::temp_dir().join(format!("pdfcraft-edit-all-js-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("in.pdf"), bytes).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap().with_clock(|| 1_700_000_000);
    let doc = tool(&mut a, "doc_open", json!({ "path": "in.pdf" }))["doc"].as_u64().unwrap();
    (a, doc)
}

fn scripts(a: &mut Automation, doc: u64) -> Vec<Value> {
    tool(a, "doc_javascripts", json!({ "doc": doc }))["scripts"].as_array().unwrap().clone()
}

/// Edit the script listed as `entry`, with the fingerprint the list gave.
fn edit(a: &mut Automation, doc: u64, entry: &Value, action: &str, code: Option<&str>) -> Value {
    tool(a, "doc_javascript_edit", json!({ "doc": doc, "id": entry["id"], "expect": entry["fingerprint"], "action": action, "code": code }))
}

fn find(list: &[Value], code: &str) -> Value {
    list.iter().find(|s| s["code"] == code).cloned().unwrap_or_else(|| panic!("{code} is not listed in {list:?}"))
}

/// One script of every kind; none of them runs on opening (no open action, and JavaScript
/// edits must not run any either).
fn form() -> Vec<u8> {
    pdf_of(&[
        "<< /Type /Catalog /Pages 2 0 R /Names << /JavaScript << /Names [(lib) << /S /JavaScript /JS (var lib = 1;) >>] >> >> /AA << /WC << /S /JavaScript /JS (closing\\(\\);) >> /WP << /S /JavaScript /JS (printing\\(\\);) >> >> /AcroForm << /Fields [4 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 5 0 R >> >> >> /Outlines 7 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Annots [4 0 R 6 0 R] /AA << /C << /S /JavaScript /JS (pageClose\\(\\);) >> >> >>".into(),
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (log) /V () /Rect [20 250 180 270] /P 3 0 R /F 4 /Q 0 /AA << /K << /S /JavaScript /JS (keystroke\\(\\);) /Next << /S /JavaScript /JS (afterKeystroke\\(\\);) >> >> >> >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into(),
        "<< /Type /Annot /Subtype /Link /Rect [0 0 20 20] /Border [0 0 0] /A << /S /JavaScript /JS (link\\(\\);) >> >>".into(),
        "<< /Type /Outlines /First 8 0 R /Last 8 0 R /Count 1 >>".into(),
        "<< /Title (Chapter) /Parent 7 0 R /A << /S /JavaScript /JS (chapter\\(\\);) >> >>".into(),
    ])
}

#[test]
fn every_script_is_listed_edited_and_removed_by_id() {
    let (mut a, doc) = open("all", form());
    let list = scripts(&mut a, doc);
    let kinds: Vec<(String, String)> = list.iter().map(|s| (s["kind"].as_str().unwrap().into(), s["trigger"].as_str().unwrap().into())).collect();
    for want in [
        ("document", "Document script"),
        ("document_action", "Will Close"),
        ("document_action", "Will Print"),
        ("page", "Page Close"),
        ("link", "Activate"),
        ("field", "Keystroke"),
        ("bookmark", "Activate"),
    ] {
        assert!(kinds.contains(&(want.0.into(), want.1.into())), "{want:?} missing from {kinds:?}");
    }
    let key = find(&list, "keystroke();");
    assert_eq!((key["field"].as_str(), key["page"].as_u64()), (Some("log"), Some(1)));
    assert_eq!(find(&list, "var lib = 1;")["name"], "lib");
    assert_eq!(find(&list, "chapter();")["name"], "Chapter");
    assert_eq!(find(&list, "link();")["page"], 1);
    assert_eq!(list.len(), 8, "{list:?}");

    // The filter.
    let fields = tool(&mut a, "doc_javascripts", json!({ "doc": doc, "kind": "field" }));
    assert_eq!(fields["count"], 2);
    assert!(a.call("doc_javascripts", &json!({ "doc": doc, "kind": "nonsense" })).unwrap_err().to_string().contains("kind must be one of"));

    // Set one: only its code changes, the chained action stays, and nothing ran.
    let id = key["id"].as_str().unwrap();
    let after = edit(&mut a, doc, &key, "set", Some("event.rc = true;"));
    let list = after["scripts"].as_array().unwrap();
    assert_eq!(find(list, "event.rc = true;")["id"], id);
    assert_eq!(find(list, "afterKeystroke();")["trigger"], "Keystroke");
    assert_eq!(list.len(), 8);
    // Editing ran nothing: the field is as it was.
    let fields = tool(&mut a, "form_fields", json!({ "doc": doc }));
    assert!(fields["fields"].as_array().unwrap().iter().all(|f| f["value"].as_str().is_none_or(str::is_empty)));
    assert!(after.get("console").is_none() && after.get("alerts").is_none());

    // Undo restores the script; the edit is one step.
    tool(&mut a, "edit_undo", json!({ "doc": doc }));
    assert_eq!(find(&scripts(&mut a, doc), "keystroke();")["id"], id);

    // Remove: the chained action takes its place.
    let current = find(&scripts(&mut a, doc), "keystroke();");
    let removed = edit(&mut a, doc, &current, "remove", None);
    let list = removed["scripts"].as_array().unwrap();
    assert!(list.iter().all(|s| s["code"] != "keystroke();"));
    assert_eq!(find(list, "afterKeystroke();")["id"], id);
    // A named script goes with its name.
    let lib = find(list, "var lib = 1;");
    let removed = edit(&mut a, doc, &lib, "remove", None);
    assert_eq!(removed["scripts"].as_array().unwrap().len(), 6);

    // Saved and reopened, the edits are there.
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "out.pdf" }));
    let reopened = tool(&mut a, "doc_open", json!({ "path": "out.pdf" }))["doc"].as_u64().unwrap();
    let list = scripts(&mut a, reopened);
    assert_eq!(list.len(), 6);
    assert!(list.iter().all(|s| s["code"] != "var lib = 1;"));
}

#[test]
fn bad_edits_are_refused_with_something_to_act_on() {
    let (mut a, doc) = open("bad", form());
    let before = scripts(&mut a, doc);
    let fp = before[0]["fingerprint"].clone();
    for args in [
        json!({ "doc": doc, "id": "obj:99/A", "expect": "x", "action": "set", "code": "x" }),
        json!({ "doc": doc, "id": "catalog/Names", "expect": "x", "action": "remove" }),
        json!({ "doc": doc, "id": "not an id", "expect": "x", "action": "set", "code": "x" }),
        json!({ "doc": doc, "id": before[0]["id"], "expect": fp, "action": "set" }),
        json!({ "doc": doc, "id": before[0]["id"], "expect": fp, "action": "rename", "code": "x" }),
        json!({ "doc": doc, "id": before[0]["id"], "expect": fp, "action": "remove", "code": "x" }),
        json!({ "doc": doc, "id": before[0]["id"], "expect": fp, "action": "set", "code": "x".repeat((1 << 20) + 1) }),
        // Without the fingerprint, or with another script's.
        json!({ "doc": doc, "id": before[0]["id"], "action": "remove" }),
        json!({ "doc": doc, "id": before[0]["id"], "expect": before[1]["fingerprint"], "action": "remove" }),
    ] {
        let e = a.call("doc_javascript_edit", &args).unwrap_err().to_string();
        assert!(!e.is_empty());
        assert_eq!(scripts(&mut a, doc), before, "{e}");
    }
    let e =
        a.call("doc_javascript_edit", &json!({ "doc": doc, "id": "obj:99/A", "expect": "x", "action": "set", "code": "x" })).unwrap_err().to_string();
    assert!(e.contains("list the JavaScripts again"), "{e}");
    let e = a
        .call("doc_javascript_edit", &json!({ "doc": doc, "id": before[0]["id"], "expect": before[1]["fingerprint"], "action": "remove" }))
        .unwrap_err()
        .to_string();
    assert!(e.contains("changed; list the JavaScripts again"), "{e}");
    // `code` may be null for a removal.
    let removed = tool(&mut a, "doc_javascript_edit", json!({ "doc": doc, "id": before[0]["id"], "expect": fp, "action": "remove", "code": null }));
    assert_eq!(removed["count"].as_u64().unwrap() + 1, before.len() as u64);
    tool(&mut a, "edit_undo", json!({ "doc": doc }));
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "a refused edit leaves nothing to undo");
}

#[test]
fn hostile_structures_are_listed_safely() {
    let huge = "x".repeat(2 << 20);
    let (mut a, doc) = open(
        "hostile",
        pdf_of(&[
            // Open action chain that loops back; a Will Save with no /JS; a stream script in a
            // filter nobody can undo; a name tree that lists itself; an outline that loops.
            "<< /Type /Catalog /Pages 2 0 R /OpenAction 3 0 R /AA << /WS 5 0 R /DS 6 0 R /WC 8 0 R >> /Names << /JavaScript 9 0 R >> /Outlines 10 0 R >>".into(),
            "<< /Type /Pages /Kids [12 0 R] /Count 1 >>".into(),
            "<< /S /JavaScript /JS (first\\(\\);) /Next 4 0 R >>".into(),
            "<< /S /JavaScript /JS (second\\(\\);) /Next 3 0 R >>".into(),
            "<< /S /JavaScript >>".into(),
            "<< /S /JavaScript /JS 7 0 R >>".into(),
            "<< /Length 3 /Filter /NoSuchFilter >>\nstream\nabc\nendstream".into(),
            format!("<< /S /JavaScript /JS ({huge}) >>"),
            "<< /Kids [9 0 R] /Names [(self) << /S /JavaScript /JS (named\\(\\);) >>] >>".into(),
            "<< /First 11 0 R >>".into(),
            "<< /Title (loop) /First 11 0 R /Next 11 0 R /A << /S /JavaScript /JS (bookmark\\(\\);) >> >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>".into(),
        ]),
    );
    let list = scripts(&mut a, doc);
    assert_eq!(list.len(), 7, "{:?}", list.iter().map(|s| (&s["trigger"], s["code"].as_str().map(str::len))).collect::<Vec<_>>());
    let codes: Vec<_> = list.iter().filter(|s| s["kind"] == "open_action").map(|s| s["code"].as_str().unwrap()).collect();
    assert_eq!(codes, ["first();", "second();"]);
    let by_trigger = |t: &str| list.iter().find(|s| s["trigger"] == t).unwrap().clone();
    assert_eq!(by_trigger("Will Save")["code"], Value::Null);
    assert_eq!(by_trigger("Did Save")["code"], Value::Null);
    let big = by_trigger("Will Close");
    assert_eq!((big["truncated"].clone(), big["code"].as_str().map(str::len)), (json!(true), Some(1 << 20)));
    // A script with no text can be given some; the huge one is replaced whole.
    edit(&mut a, doc, &by_trigger("Will Save"), "set", Some("ok();"));
    edit(&mut a, doc, &big, "set", Some("small();"));
    let list = scripts(&mut a, doc);
    assert_eq!(find(&list, "small();")["truncated"], false);
    assert_eq!(find(&list, "ok();")["trigger"], "Will Save");
}

#[test]
fn the_list_is_capped() {
    let pairs: String = (0..5010).map(|i| format!("(n{i:05}) << /S /JavaScript /JS (s\\(\\);) >> ")).collect();
    let (mut a, doc) = open(
        "cap",
        pdf_of(&[
            "<< /Type /Catalog /Pages 2 0 R /Names << /JavaScript 3 0 R >> >>".into(),
            "<< /Type /Pages /Kids [4 0 R] /Count 1 >>".into(),
            format!("<< /Names [{pairs}] >>"),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>".into(),
        ]),
    );
    let r = tool(&mut a, "doc_javascripts", json!({ "doc": doc }));
    assert_eq!((r["count"].clone(), r["capped"].clone()), (json!(5000), json!(true)));
}
