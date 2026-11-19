//! `doc_compare` with `ignore_punctuation`: words that differ only in sentence punctuation are not changes.

use pdfcraft_automation::{Automation, Content};
use serde_json::{Value, json};

fn call(a: &mut Automation, args: Value) -> Result<Value, String> {
    match a.call("doc_compare", &args) {
        Ok(mut c) => match c.remove(0) {
            Content::Json(v) => Ok(v),
            other => panic!("expected JSON, got {other:?}"),
        },
        Err(e) => Err(e.to_string()),
    }
}

fn make(a: &mut Automation, text: &str) -> u64 {
    match a.call("doc_create", &json!({ "from": "text", "text": text })).unwrap().remove(0) {
        Content::Json(v) => v["doc"].as_u64().unwrap(),
        other => panic!("expected JSON, got {other:?}"),
    }
}

#[test]
fn ignore_punctuation_hides_mark_only_changes_and_keeps_real_ones() {
    let mut a = Automation::new();
    let old = make(&mut a, "A, one B; two C. three $50 end");
    let new = make(&mut a, "A one B two C three $50 end");
    // Off (the default, or false): the marks are changes.
    let r = call(&mut a, json!({ "doc": new, "other": old })).unwrap();
    assert_eq!(r["replaced"], 3, "{r}");
    let r = call(&mut a, json!({ "doc": new, "other": old, "ignore_punctuation": false })).unwrap();
    assert_eq!(r["replaced"], 3, "{r}");
    // On: identical.
    let r = call(&mut a, json!({ "doc": new, "other": old, "ignore_punctuation": true })).unwrap();
    assert_eq!((r["identical"].clone(), r["changes"].clone()), (json!(true), json!([])), "{r}");
    // A symbol and a real word change still count, with each file's own text.
    let edited = make(&mut a, "A one B two C three $60 end");
    let r = call(&mut a, json!({ "doc": edited, "other": old, "ignore_punctuation": true })).unwrap();
    assert_eq!(
        (r["replaced"].clone(), r["changes"][0]["old"]["text"].clone(), r["changes"][0]["new"]["text"].clone()),
        (json!(1), json!("$50"), json!("$60")),
        "{r}"
    );
    // Combined with ignore_case.
    let loud = make(&mut a, "a one b two c three $50 end");
    let r = call(&mut a, json!({ "doc": loud, "other": old, "ignore_punctuation": true })).unwrap();
    assert_eq!(r["replaced"], 3, "{r}");
    let r = call(&mut a, json!({ "doc": loud, "other": old, "ignore_punctuation": true, "ignore_case": true })).unwrap();
    assert_eq!(r["identical"], true, "{r}");
    // A value that is not a boolean is refused.
    assert!(call(&mut a, json!({ "doc": new, "other": old, "ignore_punctuation": "yes" })).is_err());
    assert!(call(&mut a, json!({ "doc": new, "other": old, "ignore_punctuation": null })).is_ok());
    // Figures that change are not hidden: separators between digits, a sign, per mille, primes,
    // the fraction slash, the section sign and symbols are kept.
    for (x, y) in [
        ("10.5", "105"),
        ("$5.00", "$500"),
        ("1.000", "1,000"),
        ("-5", "5"),
        ("50\u{2030}", "50"),
        ("1\u{2044}2", "12"),
        ("5\u{2032}", "5\u{2033}"),
        ("\u{A7}5", "5"),
        ("50%", "50"),
        ("$5", "5"),
    ] {
        let (p, q) = (make(&mut a, &format!("A {x} B")), make(&mut a, &format!("A {y} B")));
        let r = call(&mut a, json!({ "doc": q, "other": p, "ignore_punctuation": true })).unwrap();
        assert_eq!(r["identical"], false, "{x} and {y}: {r}");
    }
    // A mark beside a figure or a word still goes, and lone marks equal each other.
    for (x, y) in [("5.", "5"), ("A-5", "A5"), ("-", "\u{2014}")] {
        let (p, q) = (make(&mut a, &format!("A {x} B")), make(&mut a, &format!("A {y} B")));
        let r = call(&mut a, json!({ "doc": q, "other": p, "ignore_punctuation": true })).unwrap();
        assert_eq!(r["identical"], true, "{x} and {y}: {r}");
    }
}
