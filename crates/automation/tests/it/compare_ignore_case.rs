//! `doc_compare` with `ignore_case`: words that differ only in letter case are not changes.

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
fn ignore_case_hides_case_only_changes_and_keeps_real_ones() {
    let mut a = Automation::new();
    let old = make(&mut a, "Invoice TOTAL is due on Friday");
    let new = make(&mut a, "invoice total is DUE on Monday");
    // Off (the default): the three case changes and the real one are all reported.
    let r = call(&mut a, json!({ "doc": new, "other": old })).unwrap();
    assert_eq!(r["replaced"], 3, "{r}");
    let r = call(&mut a, json!({ "doc": new, "other": old, "ignore_case": false })).unwrap();
    assert_eq!(r["replaced"], 3, "{r}");
    // On: only Friday to Monday remains, with each file's own text.
    let r = call(&mut a, json!({ "doc": new, "other": old, "ignore_case": true })).unwrap();
    assert_eq!(
        (r["replaced"].clone(), r["inserted"].clone(), r["deleted"].clone(), r["identical"].clone()),
        (json!(1), json!(0), json!(0), json!(false)),
        "{r}"
    );
    assert_eq!((r["changes"][0]["old"]["text"].clone(), r["changes"][0]["new"]["text"].clone()), (json!("Friday"), json!("Monday")));
    // Case-only differences alone make the text identical.
    let same = make(&mut a, "INVOICE total IS due ON friday");
    let r = call(&mut a, json!({ "doc": same, "other": old, "ignore_case": true })).unwrap();
    assert_eq!((r["identical"].clone(), r["changes"].clone()), (json!(true), json!([])), "{r}");
    // A value that is not a boolean is refused.
    assert!(call(&mut a, json!({ "doc": new, "other": old, "ignore_case": "yes" })).is_err());
}
