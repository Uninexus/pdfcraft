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

fn add_field(a: &mut Automation, doc: u64, name: &str, kind: &str) {
    let (kind, multiline) = if kind == "multiline" { ("text", true) } else { (kind, false) };
    tool(a, "form_add_field", json!({ "doc": doc, "page": 1, "type": kind, "name": name, "rect": [20, 20, 160, 40], "multiline": multiline }));
}

#[test]
fn auto_complete_is_off_until_asked_for_and_remembers_only_plain_text_fields() {
    let (mut a, doc) = setup("autocomplete");
    for (name, kind) in [("city", "text"), ("when", "date"), ("sig", "signature"), ("agree", "checkbox"), ("notes", "multiline")] {
        add_field(&mut a, doc, name, kind);
    }
    let off = tool(&mut a, "form_preferences", json!({}));
    assert_eq!((off["auto_complete"].clone(), off["entry_count"].clone()), (json!("off"), json!(0)));

    // Off: filling remembers nothing and nothing is offered.
    tool(&mut a, "form_fill", json!({ "doc": doc, "values": { "city": "Maplewood" } }));
    assert_eq!(tool(&mut a, "form_preferences", json!({ "suggest": "map" }))["suggestions"], json!([]));
    assert_eq!(tool(&mut a, "form_preferences", json!({}))["entry_count"], 0);

    tool(&mut a, "form_preferences", json!({ "auto_complete": "basic" }));
    assert!(a.session().form_autocomplete());
    tool(&mut a, "form_fill", json!({ "doc": doc, "values": { "city": "Maplewood", "when": "01/02/2003", "agree": true, "notes": "a long note" } }));
    tool(&mut a, "form_fill", json!({ "doc": doc, "values": { "city": "Oakdale" } }));
    let out = tool(&mut a, "form_preferences", json!({ "list_entries": true, "suggest": "MAP" }));
    assert_eq!(out["entries"], json!(["Oakdale", "Maplewood"]), "newest first; the date, the check box and the multi-line field are not kept");
    assert_eq!(out["suggestions"], json!(["Maplewood"]));
    assert_eq!(out["entry_count"], 2);
    assert!(tool(&mut a, "form_preferences", json!({}))["entries"].is_null(), "listed only on request");

    assert_eq!(tool(&mut a, "form_preferences", json!({ "forget_entry": " Oakdale " }))["entry_count"], 1);
    assert!(matches!(a.call("form_preferences", &json!({ "forget_entry": "Oakdale", "auto_complete": "off" })), Err(ToolError::InvalidArgs(_))));
    assert!(a.session().form_autocomplete(), "a refused call changes nothing");
    // Off again: what was remembered stays, but nothing is offered or added.
    let out = tool(&mut a, "form_preferences", json!({ "auto_complete": "off", "suggest": "map" }));
    assert_eq!((out["suggestions"].clone(), out["entry_count"].clone()), (json!([]), json!(1)));
    assert_eq!(tool(&mut a, "form_preferences", json!({ "clear_entries": true }))["entry_count"], 0);
    assert!(matches!(a.call("form_preferences", &json!({ "auto_complete": "advanced" })), Err(ToolError::InvalidArgs(_))));
    assert!(matches!(a.call("form_preferences", &json!({ "clear_entries": "yes" })), Err(ToolError::InvalidArgs(_))));
}

#[test]
fn a_form_preferences_call_with_a_bad_argument_changes_none_of_the_others() {
    let (mut a, doc) = setup("forms-atomic");
    add_field(&mut a, doc, "city", "text");
    tool(&mut a, "form_preferences", json!({ "auto_complete": "basic" }));
    tool(&mut a, "form_fill", json!({ "doc": doc, "values": { "city": "Maplewood" } }));
    // Each bad value sits beside arguments that would change something if they were applied first.
    let bad = [
        json!({ "auto_complete": "advanced" }),
        json!({ "clear_entries": "yes" }),
        json!({ "list_entries": 1 }),
        json!({ "suggest": 5 }),
        json!({ "forget_entry": "nobody" }),
        json!({ "forget_entry": 7 }),
    ];
    for extra in bad {
        let mut args = json!({ "highlight_fields": true, "clear_entries": true });
        if extra.get("clear_entries").is_none() {
            args["auto_complete"] = json!("off");
        }
        for (k, v) in extra.as_object().into_iter().flatten() {
            args[k] = v.clone();
        }
        assert!(matches!(a.call("form_preferences", &args), Err(ToolError::InvalidArgs(_))), "{args} is refused");
        assert!(!a.session().field_highlight(), "{args}: Highlight fields is still off");
        assert!(a.session().form_autocomplete(), "{args}: Auto-Complete is still on");
        assert_eq!(a.session().form_entries().len(), 1, "{args}: the remembered entry is still there");
    }
    let ok = tool(&mut a, "form_preferences", json!({ "highlight_fields": true, "auto_complete": "off", "clear_entries": true }));
    assert_eq!((ok["highlight_fields"].clone(), ok["auto_complete"].clone(), ok["entry_count"].clone()), (json!(true), json!("off"), json!(0)));
}

#[test]
fn the_login_name_authors_comments_only_when_asked_and_a_refused_call_changes_nothing() {
    let (mut a, doc) = setup("login");
    let note = |a: &mut Automation| {
        tool(a, "comment_add", json!({ "doc": doc, "page": 1, "type": "note", "at": [20, 20], "contents": "Check" }));
    };
    let first = tool(&mut a, "identity", json!({}));
    assert_eq!((first["login_name"].clone(), first["author_from_login_name"].clone()), (json!(""), json!(false)));
    let set = tool(&mut a, "identity", json!({ "name": "Grace Hopper", "login_name": "  ghopper " }));
    // Set but not asked for: the name is still the author.
    assert_eq!((set["login_name"].clone(), set["author"].clone()), (json!("ghopper"), json!("Grace Hopper")));
    note(&mut a);
    let on = tool(&mut a, "identity", json!({ "author_from_login_name": true }));
    assert_eq!((on["author"].clone(), on["name"].clone(), on["login_name"].clone()), (json!("ghopper"), json!("Grace Hopper"), json!("ghopper")));
    note(&mut a);
    // Changing the name leaves the login name and the switch alone; the typed signature still uses the name.
    let renamed = tool(&mut a, "identity", json!({ "name": "G. Hopper" }));
    assert_eq!((renamed["author"].clone(), renamed["author_from_login_name"].clone()), (json!("ghopper"), json!(true)));
    // A typed signature with no text and no author: its text is the name, its author the login name.
    tool(&mut a, "fill_sign_add", json!({ "doc": doc, "page": 1, "type": "signature", "at": [20, 150] }));
    let width = |a: &mut Automation, doc: u64, idx: usize| {
        let list = tool(a, "comment_list", json!({ "doc": doc }));
        let stamps: Vec<&Value> = list["comments"].as_array().map(|c| c.iter().filter(|c| c["type"] == "Stamp").collect()).unwrap_or_default();
        let r = stamps[idx]["rect"].clone();
        (r[2].as_f64().unwrap_or(0.0) - r[0].as_f64().unwrap_or(0.0), r[3].as_f64().unwrap_or(0.0) - r[1].as_f64().unwrap_or(0.0))
    };
    let list = tool(&mut a, "comment_list", json!({ "doc": doc }));
    let sig_authors: Vec<&Value> =
        list["comments"].as_array().map(|c| c.iter().filter(|c| c["type"] == "Stamp").map(|c| &c["author"]).collect()).unwrap_or_default();
    assert_eq!(sig_authors, [&json!("ghopper")]);
    let from_name = width(&mut a, doc, 0);
    // The same signature drawn with the text given: the name's text is the same size, the login name's is not.
    let (mut b, doc_b) = setup("login-text");
    tool(&mut b, "identity", json!({ "name": "G. Hopper", "login_name": "ghopper", "author_from_login_name": true }));
    tool(&mut b, "fill_sign_add", json!({ "doc": doc_b, "page": 1, "type": "signature", "at": [20, 150], "text": "G. Hopper" }));
    tool(&mut b, "fill_sign_add", json!({ "doc": doc_b, "page": 1, "type": "signature", "at": [20, 150], "text": "ghopper" }));
    let (named, logged) = (width(&mut b, doc_b, 0), width(&mut b, doc_b, 1));
    assert_ne!(named, logged, "the two texts must draw differently for this check to mean anything");
    assert_eq!(from_name, named, "a signature with no text is drawn from the name, not the login name");
    // With no login name the name is used, and the switch stays on.
    let cleared = tool(&mut a, "identity", json!({ "login_name": "" }));
    assert_eq!((cleared["author"].clone(), cleared["author_from_login_name"].clone()), (json!("G. Hopper"), json!(true)));
    note(&mut a);
    assert_eq!(authors(&mut a, doc), ["Grace Hopper", "ghopper", "G. Hopper", "ghopper"]);
    // A refused call changes nothing, whichever argument is bad: a long login name beside a valid name,
    // and a switch that is not a boolean beside a valid login name.
    // The cap counts characters, not bytes, and is applied after trimming.
    let set = tool(&mut a, "identity", json!({ "login_name": "é".repeat(200) }));
    assert_eq!(set["login_name"].as_str().map(|s| s.chars().count()), Some(200));
    let set = tool(&mut a, "identity", json!({ "login_name": format!("   {}  ", "y".repeat(200)) }));
    assert_eq!(set["login_name"], json!("y".repeat(200)));
    assert!(matches!(a.call("identity", &json!({ "login_name": "é".repeat(201) })), Err(ToolError::InvalidArgs(_))));
    tool(&mut a, "identity", json!({ "login_name": "ghopper" }));
    let before = tool(&mut a, "identity", json!({}));
    let long = "x".repeat(201);
    for bad in [
        json!({ "name": "Other", "login_name": long }),
        json!({ "login_name": "other", "author_from_login_name": "yes" }),
        json!({ "login_name": 7 }),
    ] {
        assert!(matches!(a.call("identity", &bad), Err(ToolError::InvalidArgs(_))), "{bad}");
        assert_eq!(tool(&mut a, "identity", json!({})), before, "{bad}");
    }
}
