//! Preferences ▸ Identity and ▸ Forms through the tool table: the name is the default author and
//! signature text, the email is the default signature contact, and Highlight fields is shared
//! with what form scripts read.

use pdfcraft_automation::{Automation, Content, ToolError};
use pdfcraft_engine::Session;
use serde_json::{Value, json};

fn tool(a: &mut Automation, name: &str, args: Value) -> Value {
    match a.call(name, &args) {
        Ok(mut c) => match c.remove(0) {
            Content::Json(v) => v,
            other => panic!("{name}: expected JSON, got {other:?}"),
        },
        Err(e) => panic!("{name} {args}: {e}"),
    }
}

fn setup(tag: &str) -> (Automation, u64) {
    let dir = std::env::temp_dir().join(format!("pdfcraft-preferences-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let s = Session::new();
    let bytes = s.create_blank(300.0, 400.0, 1).unwrap();
    std::fs::write(dir.join("a.pdf"), bytes.as_slice()).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap().with_clock(|| 1_700_000_000);
    let doc = tool(&mut a, "doc_open", json!({ "path": "a.pdf" }))["doc"].as_u64().unwrap();
    (a, doc)
}

fn authors(a: &mut Automation, doc: u64) -> Vec<String> {
    let list = tool(a, "comment_list", json!({ "doc": doc }));
    list["comments"].as_array().unwrap().iter().map(|c| c["author"].as_str().unwrap_or_default().to_string()).collect()
}

#[test]
fn the_identity_name_is_the_default_comment_author() {
    let (mut a, doc) = setup("author");
    let note = |a: &mut Automation, extra: Value| {
        let mut args = json!({ "doc": doc, "page": 1, "type": "note", "at": [20, 20], "contents": "Check" });
        args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        tool(a, "comment_add", args);
    };
    // Unset: the tools' own default, as before.
    assert_eq!(tool(&mut a, "identity", json!({}))["author"], "PdfCraft");
    note(&mut a, json!({}));
    let set = tool(&mut a, "identity", json!({ "name": "  Grace Hopper ", "email": "grace@example.com" }));
    assert_eq!(
        (set["name"].clone(), set["email"].clone(), set["author"].clone()),
        (json!("Grace Hopper"), json!("grace@example.com"), json!("Grace Hopper"))
    );
    note(&mut a, json!({}));
    // An explicit author still wins.
    note(&mut a, json!({ "author": "Ada" }));
    assert_eq!(authors(&mut a, doc), ["PdfCraft", "Grace Hopper", "Ada"]);
    // Setting one leaves the other; an empty string clears.
    let cleared = tool(&mut a, "identity", json!({ "name": "" }));
    assert_eq!((cleared["name"].clone(), cleared["email"].clone()), (json!(""), json!("grace@example.com")));
    note(&mut a, json!({}));
    assert_eq!(authors(&mut a, doc).last().map(String::as_str), Some("PdfCraft"));
    // Too long a name is refused.
    let long = "x".repeat(201);
    assert!(matches!(a.call("identity", &json!({ "name": long })), Err(ToolError::InvalidArgs(_))));
}

#[test]
fn a_typed_signature_starts_from_the_identity_name() {
    let (mut a, doc) = setup("signature");
    let add = |a: &mut Automation, kind: &str| a.call("fill_sign_add", &json!({ "doc": doc, "page": 1, "type": kind, "at": [20, 60] }));
    // Without a name or text there is nothing to draw.
    assert!(matches!(add(&mut a, "signature"), Err(ToolError::InvalidArgs(_))));
    tool(&mut a, "identity", json!({ "name": "Grace Hopper" }));
    tool(&mut a, "fill_sign_add", json!({ "doc": doc, "page": 1, "type": "signature", "at": [20, 60] }));
    tool(&mut a, "fill_sign_add", json!({ "doc": doc, "page": 1, "type": "initials", "at": [20, 100] }));
    let list = tool(&mut a, "comment_list", json!({ "doc": doc }));
    assert_eq!(list["count"], 2, "{list}");
    // The author of both is the identity name too.
    assert_eq!(authors(&mut a, doc), ["Grace Hopper", "Grace Hopper"]);
}

#[test]
fn the_identity_email_is_the_signatures_contact_unless_one_is_given() {
    let (mut a, doc) = setup("contact");
    tool(&mut a, "sign_id_create", json!({ "name": "Ada Lovelace", "key": "p256", "password": "secret12", "path": "ada.p12" }));
    let sign = |a: &mut Automation, out: &str, extra: Value| {
        let mut args = json!({ "doc": doc, "id": "ada.p12", "password": "secret12", "out": out });
        args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let signed = tool(a, "sign_document", args);
        assert_eq!(signed["signature"]["status"], "unknown", "{signed}");
        signed["signature"]["contact"].clone()
    };
    assert_eq!(sign(&mut a, "none.pdf", json!({})), Value::Null);
    tool(&mut a, "identity", json!({ "email": "ada@example.com" }));
    assert_eq!(sign(&mut a, "id.pdf", json!({})), "ada@example.com");
    assert_eq!(sign(&mut a, "given.pdf", json!({ "contact": "555-0100" })), "555-0100");
}

#[test]
fn form_preferences_share_highlight_fields_with_the_session() {
    let (mut a, _) = setup("forms");
    assert_eq!(tool(&mut a, "form_preferences", json!({}))["highlight_fields"], false);
    assert_eq!(tool(&mut a, "form_preferences", json!({ "highlight_fields": true }))["highlight_fields"], true);
    assert!(a.session().field_highlight());
    assert!(matches!(a.call("form_preferences", &json!({ "highlight_fields": "yes" })), Err(ToolError::InvalidArgs(_))));
}
