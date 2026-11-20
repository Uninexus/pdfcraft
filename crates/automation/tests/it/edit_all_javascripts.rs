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
    let bodies: Vec<Vec<u8>> = objs.iter().map(|o| o.clone().into_bytes()).collect();
    pdf_of_bytes(&bodies)
}

/// Like [`pdf_of`], for bodies that are not text (a stream of bytes that are not UTF-8).
fn pdf_of_bytes(objs: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
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

#[test]
fn names_past_the_reading_allowance_are_left_out_and_counted() {
    // 20 name-tree keys that are one indirect 1 MiB string: 16 MiB of names are read.
    let pairs: String = (0..20).map(|i| format!("4 0 R << /S /JavaScript /JS (s{i}\\(\\);) >> ")).collect();
    let (mut a, doc) = open(
        "labels",
        pdf_of(&[
            "<< /Type /Catalog /Pages 2 0 R /Names << /JavaScript 3 0 R >> >>".into(),
            "<< /Type /Pages /Kids [5 0 R] /Count 1 >>".into(),
            format!("<< /Names [{pairs}] >>"),
            format!("({})", "n".repeat(1 << 20)),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>".into(),
        ]),
    );
    let r = tool(&mut a, "doc_javascripts", json!({ "doc": doc }));
    let list = r["scripts"].as_array().unwrap();
    assert_eq!((r["count"].clone(), r["names_left_out"].clone(), r["max_label_bytes"].clone()), (json!(20), json!(4), json!(16 << 20)));
    assert_eq!(list.iter().filter(|s| s["name"].is_null()).count(), 4);
    assert!(list.iter().all(|s| s["code"].is_string()));
    // An ordinary document reports none.
    let (mut a, doc) = open("ordinary", form());
    assert_eq!(tool(&mut a, "doc_javascripts", json!({ "doc": doc }))["names_left_out"], 0);
}

/// The bytes of the document as saved now (the test folder is the one `open` made for `name`).
fn saved(a: &mut Automation, doc: u64, name: &str) -> Vec<u8> {
    tool(a, "doc_save", json!({ "doc": doc, "path": "saved.pdf" }));
    std::fs::read(std::env::temp_dir().join(format!("pdfcraft-edit-all-js-{name}-{}", std::process::id())).join("saved.pdf")).unwrap()
}

fn replace(a: &mut Automation, doc: u64, find: &str, put: &str, extra: Value) -> Result<Value, String> {
    let mut args = json!({ "doc": doc, "find": find, "replace": put });
    if let (Some(map), Some(more)) = (args.as_object_mut(), extra.as_object()) {
        map.extend(more.clone());
    }
    match a.call("doc_javascript_replace", &args) {
        Ok(mut out) => match out.remove(0) {
            Content::Json(v) => Ok(v),
            other => panic!("expected JSON: {other:?}"),
        },
        Err(e) => Err(e.to_string()),
    }
}

#[test]
fn replace_rewrites_every_kind_of_script_in_one_undoable_step() {
    let (mut a, doc) = open("replace", form());
    let before = scripts(&mut a, doc);
    // The preview says what would change and writes nothing.
    let preview = replace(&mut a, doc, "();", "(1);", json!({ "dry_run": true })).unwrap();
    assert_eq!((preview["dry_run"].clone(), preview["applied"].clone(), preview["occurrences"].clone()), (json!(true), json!(false), json!(7)));
    assert_eq!(scripts(&mut a, doc), before);
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "a preview leaves nothing to undo");

    let done = replace(&mut a, doc, "();", "(1);", json!({})).unwrap();
    assert_eq!(
        (done["applied"].clone(), done["searched"].clone(), done["occurrences"].clone(), done["skipped_unreadable"].clone()),
        (json!(true), json!(8), json!(7), json!(0))
    );
    let changed = done["changed"].as_array().unwrap();
    assert_eq!(changed.len(), 7, "{changed:?}");
    let kinds: Vec<&str> = changed.iter().map(|c| c["kind"].as_str().unwrap()).collect();
    for k in ["document_action", "page", "link", "field", "bookmark"] {
        assert!(kinds.contains(&k), "{k} missing from {kinds:?}");
    }
    let after = scripts(&mut a, doc);
    assert_eq!(after.len(), 8);
    // Same ids; only the code changed, and the script with no match is as it was. The field's
    // chained action is still there.
    for (b, n) in before.iter().zip(&after) {
        assert_eq!(b["id"], n["id"]);
        assert_eq!(n["code"], json!(b["code"].as_str().unwrap().replace("();", "(1);")));
    }
    assert_eq!(find(&after, "afterKeystroke(1);")["trigger"], "Keystroke");
    assert_eq!(find(&after, "var lib = 1;")["fingerprint"], find(&before, "var lib = 1;")["fingerprint"]);

    // One step undoes all of it.
    tool(&mut a, "edit_undo", json!({ "doc": doc }));
    assert_eq!(scripts(&mut a, doc), before);
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err());
}

#[test]
fn replace_matches_case_only_when_asked_and_a_miss_adds_no_step() {
    let (mut a, doc) = open("replace-case", form());
    let miss = replace(&mut a, doc, "KEYSTROKE", "k", json!({})).unwrap();
    assert_eq!((miss["applied"].clone(), miss["occurrences"].clone()), (json!(false), json!(0)));
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "a replace that finds nothing leaves no step");
    let hit = replace(&mut a, doc, "KEYSTROKE", "k", json!({ "match_case": false })).unwrap();
    assert_eq!(hit["occurrences"], 2);
    let list = scripts(&mut a, doc);
    // `afterKeystroke` has a capital K, which the fold finds too.
    assert_eq!((find(&list, "k();")["trigger"].clone(), find(&list, "afterk();")["trigger"].clone()), (json!("Keystroke"), json!("Keystroke")));
    // Replacing with nothing removes the text.
    let gone = replace(&mut a, doc, "();", "", json!({})).unwrap();
    assert_eq!(gone["applied"], true);
    assert!(scripts(&mut a, doc).iter().all(|s| !s["code"].as_str().unwrap().contains("();")));
}

#[test]
fn a_refused_replace_writes_nothing() {
    let big = "a".repeat(600 * 1024);
    let (mut a, doc) = open(
        "replace-refused",
        pdf_of(&[
            "<< /Type /Catalog /Pages 2 0 R /Names << /JavaScript << /Names [(small) << /S /JavaScript /JS (var a = 1;) >> (big) 4 0 R] >> >> >>"
                .into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] >>".into(),
            "<< /Type /Page /Parent 2 0 R >>".into(),
            format!("<< /S /JavaScript /JS ({big}) >>"),
        ]),
    );
    let before = saved(&mut a, doc, "replace-refused");
    let scripts_before = scripts(&mut a, doc);
    // `a` doubles in the 600 KiB script: 1,228,800 bytes is past the 1 MiB a script may hold, so
    // the small script is not rewritten either.
    let e = replace(&mut a, doc, "a", "aa", json!({})).unwrap_err();
    assert!(e.contains("longer than 1048576 bytes"), "{e}");
    for (find, put) in [("", "x"), ("a", "a"), (&"y".repeat(1025), "x")] {
        assert!(replace(&mut a, doc, find, put, json!({})).is_err(), "{find:.5}");
    }
    let e = replace(&mut a, doc, "a", &"z".repeat((64 << 10) + 1), json!({})).unwrap_err();
    assert!(e.contains("65536"), "{e}");
    // The same preview refuses the same way.
    assert!(replace(&mut a, doc, "a", "aa", json!({ "dry_run": true })).is_err());
    assert_eq!(scripts(&mut a, doc), scripts_before);
    assert_eq!(saved(&mut a, doc, "replace-refused"), before);
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err());
}

#[test]
fn replace_skips_scripts_it_cannot_read_whole_and_counts_them() {
    let huge = "x".repeat(2 << 20);
    let (mut a, doc) = open(
        "replace-skip",
        pdf_of(&[
            "<< /Type /Catalog /Pages 2 0 R /OpenAction 3 0 R /AA << /WS 4 0 R /DS 5 0 R /WC 7 0 R >> >>".into(),
            "<< /Type /Pages /Kids [8 0 R] /Count 1 >>".into(),
            "<< /S /JavaScript /JS (first\\(\\);) >>".into(),
            "<< /S /JavaScript >>".into(),
            "<< /S /JavaScript /JS 6 0 R >>".into(),
            "<< /Length 3 /Filter /NoSuchFilter >>\nstream\nabc\nendstream".into(),
            format!("<< /S /JavaScript /JS ({huge}) >>"),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>".into(),
        ]),
    );
    let r = replace(&mut a, doc, "x", "y", json!({})).unwrap();
    // The 2 MiB script is listed cut to 1 MiB: replacing in the cut copy would lose the rest.
    assert_eq!(
        (r["searched"].clone(), r["skipped_unreadable"].clone(), r["occurrences"].clone(), r["applied"].clone()),
        (json!(1), json!(3), json!(0), json!(false))
    );
    let r = replace(&mut a, doc, "first", "second", json!({})).unwrap();
    assert_eq!(r["occurrences"], 1);
    assert_eq!(r["skipped_unreadable"], 3);
    let list = scripts(&mut a, doc);
    assert_eq!(find(&list, "second();")["kind"], "open_action");
    assert_eq!(list.iter().find(|s| s["truncated"] == true).unwrap()["code"].as_str().unwrap().len(), 1 << 20);
}

#[test]
fn replace_refuses_a_document_with_more_scripts_than_a_listing_reads() {
    let pairs: String = (0..5010).map(|i| format!("(n{i:05}) << /S /JavaScript /JS (s\\(\\);) >> ")).collect();
    let (mut a, doc) = open(
        "replace-cap",
        pdf_of(&[
            "<< /Type /Catalog /Pages 2 0 R /Names << /JavaScript 3 0 R >> >>".into(),
            "<< /Type /Pages /Kids [4 0 R] /Count 1 >>".into(),
            format!("<< /Names [{pairs}] >>"),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>".into(),
        ]),
    );
    let before = saved(&mut a, doc, "replace-cap");
    let e = replace(&mut a, doc, "s", "t", json!({})).unwrap_err();
    assert!(e.contains("more scripts than a listing reads"), "{e}");
    assert_eq!(saved(&mut a, doc, "replace-cap"), before);
}

/// A stream object body holding `bytes`.
fn stream_of(bytes: &[u8]) -> Vec<u8> {
    let mut body = format!("<< /Length {} >>\nstream\n", bytes.len()).into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(b"\nendstream");
    body
}

#[test]
fn replace_leaves_scripts_in_other_encodings_alone_and_rewrites_a_utf8_stream_as_a_string() {
    // Object 4: PDFDocEncoding bytes (0xE9 is e-acute, and not UTF-8); 5: UTF-16BE with a
    // byte-order mark, `x();`; 6: plain UTF-8 `y();`.
    let latin = stream_of(b"caf\xe9x();");
    let utf16 = stream_of(&[0xFE, 0xFF, 0, b'x', 0, b'(', 0, b')', 0, b';']);
    let plain = stream_of(b"y(); // x");
    let bodies: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R /AA << /WC 7 0 R /WS 8 0 R /DS 9 0 R >> >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        latin,
        utf16,
        plain,
        b"<< /S /JavaScript /JS 4 0 R >>".to_vec(),
        b"<< /S /JavaScript /JS 5 0 R >>".to_vec(),
        b"<< /S /JavaScript /JS 6 0 R >>".to_vec(),
    ];
    let (mut a, doc) = open("replace-encodings", pdf_of_bytes(&bodies));
    let list = scripts(&mut a, doc);
    assert_eq!(list.len(), 3, "{list:?}");
    // The listing says which scripts are read exactly.
    let exact: Vec<(String, bool)> = list.iter().map(|s| (s["trigger"].as_str().unwrap().to_string(), s["exact"].as_bool().unwrap())).collect();
    assert_eq!(exact, [("Will Close".to_string(), false), ("Will Save".to_string(), false), ("Did Save".to_string(), true)]);
    let r = replace(&mut a, doc, "x", "z", json!({})).unwrap();
    // The two scripts that are not plain UTF-8 are not searched; the plain one is.
    assert_eq!((r["searched"].clone(), r["skipped_unreadable"].clone(), r["occurrences"].clone()), (json!(1), json!(2), json!(1)));
    let after = scripts(&mut a, doc);
    assert_eq!(find(&after, "y(); // z")["trigger"], "Did Save");
    let before_text: Vec<_> =
        list.iter().filter(|s| s["trigger"] != "Did Save").map(|s| (s["id"].clone(), s["code"].clone(), s["fingerprint"].clone())).collect();
    let after_text: Vec<_> =
        after.iter().filter(|s| s["trigger"] != "Did Save").map(|s| (s["id"].clone(), s["code"].clone(), s["fingerprint"].clone())).collect();
    assert_eq!(before_text, after_text, "the scripts in other encodings are as they were");

    // With only those two, nothing is written at all.
    let bodies: Vec<Vec<u8>> = bodies.iter().take(8).cloned().collect();
    let mut only = bodies;
    if let Some(cat) = only.first_mut() {
        *cat = b"<< /Type /Catalog /Pages 2 0 R /AA << /WC 7 0 R /WS 8 0 R >> >>".to_vec();
    }
    let (mut a, doc) = open("replace-encodings-only", pdf_of_bytes(&only));
    let before = saved(&mut a, doc, "replace-encodings-only");
    let r = replace(&mut a, doc, "x", "z", json!({})).unwrap();
    assert_eq!((r["applied"].clone(), r["searched"].clone(), r["skipped_unreadable"].clone()), (json!(false), json!(0), json!(2)));
    assert_eq!(saved(&mut a, doc, "replace-encodings-only"), before);
}

#[test]
fn replace_changes_a_shared_action_for_every_place_that_holds_it() {
    // Object 6 is one action: two links hold it by /A, and object 8 chains it with /Next.
    let (mut a, doc) = open(
        "replace-shared",
        pdf_of(&[
            "<< /Type /Catalog /Pages 2 0 R /OpenAction 8 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] >>".into(),
            "<< /Type /Page /Parent 2 0 R /Annots [4 0 R 5 0 R] >>".into(),
            "<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /Border [0 0 0] /A 6 0 R >>".into(),
            "<< /Type /Annot /Subtype /Link /Rect [20 0 30 10] /Border [0 0 0] /A 6 0 R >>".into(),
            "<< /S /JavaScript /JS (shared\\(\\);) >>".into(),
            "<< /S /JavaScript /JS (other\\(\\);) >>".into(),
            "<< /S /JavaScript /JS (opening\\(\\);) /Next 6 0 R >>".into(),
        ]),
    );
    let before = scripts(&mut a, doc);
    assert!(before.iter().any(|s| s["code"] == "shared();"), "{before:?}");
    let preview = replace(&mut a, doc, "shared", "common", json!({ "dry_run": true })).unwrap();
    let r = replace(&mut a, doc, "shared", "common", json!({})).unwrap();
    assert_eq!((r["occurrences"].clone(), r["changed"].as_array().map(Vec::len)), (json!(1), Some(1)));
    // The tool reports the plan it applied, so what proves the apply is the fresh listing and
    // the saved file below, not a comparison with the dry run.
    assert_eq!(preview["occurrences"], 1);
    // A fresh listing finds the text nowhere: every place that held the action sees the change.
    let after = scripts(&mut a, doc);
    assert!(after.iter().all(|s| !s["code"].as_str().unwrap().contains("shared")), "{after:?}");
    assert!(after.iter().any(|s| s["code"] == "common();"));
    assert_eq!(replace(&mut a, doc, "shared", "x", json!({})).unwrap()["occurrences"], 0);
    // The saved file says it too.
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "out.pdf" }));
    let reopened = tool(&mut a, "doc_open", json!({ "path": "out.pdf" }))["doc"].as_u64().unwrap();
    assert!(scripts(&mut a, reopened).iter().all(|s| !s["code"].as_str().unwrap().contains("shared")));
}

#[test]
fn a_dictionary_shared_by_two_fields_is_one_script_and_the_replace_succeeds() {
    let (mut a, doc) = open(
        "replace-shared-aa",
        pdf_of(&[
            "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R 5 0 R] >> >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] >>".into(),
            "<< /Type /Page /Parent 2 0 R /Annots [4 0 R 5 0 R] >>".into(),
            "<< /Type /Annot /Subtype /Widget /FT /Tx /T (a) /Rect [0 0 10 10] /P 3 0 R /AA 6 0 R >>".into(),
            "<< /Type /Annot /Subtype /Widget /FT /Tx /T (b) /Rect [20 0 30 10] /P 3 0 R /AA 6 0 R >>".into(),
            "<< /K << /S /JavaScript /JS (key\\(\\);) >> >>".into(),
        ]),
    );
    let listed = scripts(&mut a, doc);
    assert_eq!(listed.len(), 2, "two ids lead to the one dictionary: {listed:?}");
    let preview = replace(&mut a, doc, "key", "press", json!({ "dry_run": true })).unwrap();
    let r = replace(&mut a, doc, "key", "press", json!({})).unwrap();
    // The tool reports the plan it applied, so what proves the apply is the fresh listing and
    // the saved file below, not a comparison with the dry run.
    assert_eq!(preview["occurrences"], 1);
    assert_eq!((r["applied"].clone(), r["occurrences"].clone(), r["changed"].as_array().map(Vec::len)), (json!(true), json!(1), Some(1)));
    let after = scripts(&mut a, doc);
    assert_eq!(after.len(), 2);
    assert!(after.iter().all(|s| s["code"] == "press();"), "{after:?}");
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "out.pdf" }));
    let reopened = tool(&mut a, "doc_open", json!({ "path": "out.pdf" }))["doc"].as_u64().unwrap();
    assert!(scripts(&mut a, reopened).iter().all(|s| s["code"] == "press();"));
}

#[test]
fn thousands_of_direct_scripts_are_rewritten_with_one_copy_of_their_container() {
    // 4,990 named scripts written directly in the catalog, 2 KiB each: rewriting each by
    // copying the catalog would copy about 10 MiB 4,990 times.
    let pad = "p".repeat(2000);
    let pairs: String = (0..4990).map(|i| format!("(n{i:05}) << /S /JavaScript /JS (needle{i} {pad}) >> ")).collect();
    let (mut a, doc) = open(
        "replace-direct",
        pdf_of(&[
            format!("<< /Type /Catalog /Pages 2 0 R /Names << /JavaScript << /Names [{pairs}] >> >> >>"),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] >>".into(),
            "<< /Type /Page /Parent 2 0 R >>".into(),
        ]),
    );
    let started = std::time::Instant::now();
    let r = replace(&mut a, doc, "needle", "pin", json!({})).unwrap();
    assert_eq!((r["occurrences"].clone(), r["changed"].as_array().map(Vec::len)), (json!(4990), Some(4990)));
    let list = scripts(&mut a, doc);
    assert_eq!(list.len(), 4990);
    assert!(list.iter().all(|s| s["code"].as_str().unwrap().starts_with("pin")));
    assert!(started.elapsed() < std::time::Duration::from_secs(60), "took {:?}", started.elapsed());
}

#[test]
fn rewritten_scripts_together_past_16_mib_are_refused() {
    // Each script is 960,000 bytes (950,000 `b` and 10,000 `a`): 17 of them are 16,320,000 bytes
    // and are all listed whole. Each `a` becoming six makes a script 1,010,000 bytes, under the
    // 1 MiB a script may hold, but 17 of them are 17,170,000 bytes, over the 16 MiB (16,777,216)
    // the rewritten scripts may hold together. 16 of them are 16,160,000 bytes and pass.
    let code = format!("{}{}", "b".repeat(950_000), "a".repeat(10_000));
    let build = |n: usize| {
        let pairs: String = (0..n).map(|i| format!("(n{i:02}) << /S /JavaScript /JS ({code}) >> ")).collect();
        pdf_of(&[
            "<< /Type /Catalog /Pages 2 0 R /Names << /JavaScript 4 0 R >> >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] >>".into(),
            "<< /Type /Page /Parent 2 0 R >>".into(),
            format!("<< /Names [{pairs}] >>"),
        ])
    };
    let (mut a, doc) = open("replace-total", build(17));
    assert_eq!(scripts(&mut a, doc).iter().filter(|s| s["truncated"] == false).count(), 17);
    let before = saved(&mut a, doc, "replace-total");
    let e = replace(&mut a, doc, "a", "aaaaaa", json!({})).unwrap_err();
    assert!(e.contains("more than 16777216 bytes in all"), "{e}");
    assert!(replace(&mut a, doc, "a", "aaaaaa", json!({ "dry_run": true })).is_err());
    assert_eq!(saved(&mut a, doc, "replace-total"), before);
    let (mut a, doc) = open("replace-total-16", build(16));
    let r = replace(&mut a, doc, "a", "aaaaaa", json!({})).unwrap();
    assert_eq!(r["applied"], true);
}

#[test]
fn a_replace_that_changes_no_text_is_not_a_change() {
    // Case ignored, `AFTERKEYSTROKE` to `afterKeystroke` rewrites nothing: the script already reads so.
    let (mut a, doc) = open("replace-noop", form());
    let r = replace(&mut a, doc, "AFTERKEYSTROKE", "afterKeystroke", json!({ "match_case": false })).unwrap();
    assert_eq!((r["applied"].clone(), r["occurrences"].clone(), r["changed"].as_array().map(Vec::len)), (json!(false), json!(0), Some(0)));
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err());
}

#[test]
fn a_document_that_does_not_allow_changes_refuses_a_replace() {
    let (mut a, doc) = open("replace-locked", form());
    tool(&mut a, "doc_protect", json!({ "doc": doc, "open_password": "open", "permissions_password": "boss", "changes": "none" }));
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "locked.pdf" }));
    let locked = tool(&mut a, "doc_open", json!({ "path": "locked.pdf", "password": "open" }))["doc"].as_u64().unwrap();
    let before = saved(&mut a, locked, "replace-locked");
    let listed = scripts(&mut a, locked);
    let e = replace(&mut a, locked, "();", "(1);", json!({})).unwrap_err();
    assert!(e.contains("allow") || e.contains("permission"), "{e}");
    assert_eq!(scripts(&mut a, locked), listed);
    assert_eq!(saved(&mut a, locked, "replace-locked"), before);
}

#[test]
fn a_chain_past_its_limit_is_reported_capped_and_a_replace_refuses() {
    // An open action that is an array of 300 actions: a chain visits at most 256 things, the
    // array itself and 255 of its actions.
    let items: String = (0..300).map(|i| format!("<< /S /JavaScript /JS (s{i}\\\\(\\\\);) >> ")).collect();
    let (mut a, doc) = open(
        "replace-chain",
        pdf_of(&[
            format!("<< /Type /Catalog /Pages 2 0 R /OpenAction [{items}] >>"),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] >>".into(),
            "<< /Type /Page /Parent 2 0 R >>".into(),
        ]),
    );
    let r = tool(&mut a, "doc_javascripts", json!({ "doc": doc }));
    assert_eq!((r["count"].clone(), r["capped"].clone()), (json!(255), json!(true)));
    let before = saved(&mut a, doc, "replace-chain");
    let e = replace(&mut a, doc, "s", "t", json!({})).unwrap_err();
    assert!(e.contains("past a limit"), "{e}");
    assert_eq!(saved(&mut a, doc, "replace-chain"), before);
}
