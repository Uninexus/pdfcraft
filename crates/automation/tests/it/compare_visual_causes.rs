//! `doc_compare` with `causes`: visual regions are returned by what explains them.

use pdfcraft_automation::{Automation, Content};
use serde_json::{Value, json};

fn call(a: &mut Automation, tool: &str, args: Value) -> Result<Value, String> {
    match a.call(tool, &args) {
        Ok(mut c) => match c.remove(0) {
            Content::Json(v) => Ok(v),
            other => panic!("expected JSON, got {other:?}"),
        },
        Err(e) => Err(e.to_string()),
    }
}

fn make(a: &mut Automation, text: &str) -> u64 {
    call(a, "doc_create", json!({ "from": "text", "text": text })).unwrap()["doc"].as_u64().unwrap()
}

fn causes(r: &Value) -> Vec<String> {
    r["visual"].as_array().unwrap().iter().map(|v| v["cause"].as_str().unwrap().to_string()).collect()
}

#[test]
fn causes_return_only_the_regions_asked_for_and_count_the_rest() {
    let mut a = Automation::new();
    let old = make(&mut a, "Rent is 900 per month.");
    let new = make(&mut a, "Rent is 950 per month.");
    let rect = json!({ "doc": new, "page": 1, "type": "rectangle", "rect": [300, 100, 400, 160] });
    call(&mut a, "comment_add", rect).unwrap();
    let base = json!({ "doc": new, "other": old, "visual": true });
    let all = call(&mut a, "doc_compare", base.clone()).unwrap();
    let (n_ann, n_other) = (causes(&all).iter().filter(|c| *c == "annotation").count(), causes(&all).iter().filter(|c| *c == "other").count());
    assert!(n_ann > 0 && n_other > 0, "{all}");
    assert_eq!(all["hidden_visual"], 0, "{all}");
    // An explicit list of both is the default.
    let both = call(&mut a, "doc_compare", json!({ "doc": new, "other": old, "visual": true, "causes": ["Other", "annotation"] })).unwrap();
    assert_eq!(both["visual"], all["visual"], "{both}");
    // Only annotations: the text-change regions are left out and counted.
    let r = call(&mut a, "doc_compare", json!({ "doc": new, "other": old, "visual": true, "causes": ["annotation"] })).unwrap();
    assert_eq!(causes(&r).len(), n_ann, "{r}");
    assert!(causes(&r).iter().all(|c| c == "annotation"), "{r}");
    assert_eq!(r["hidden_visual"], n_other, "{r}");
    // Only the others: the annotation regions are left out and counted.
    let r = call(&mut a, "doc_compare", json!({ "doc": new, "other": old, "visual": true, "causes": ["other"] })).unwrap();
    assert_eq!(causes(&r).len(), n_other, "{r}");
    assert!(causes(&r).iter().all(|c| c == "other"), "{r}");
    assert_eq!(r["hidden_visual"], n_ann, "{r}");
    // The text changes are unaffected by the filter.
    assert_eq!(r["replaced"], all["replaced"], "{r}");
    // Without visual nothing is read, so nothing is counted as hidden.
    let r = call(&mut a, "doc_compare", json!({ "doc": new, "other": old, "causes": ["other"] })).unwrap();
    assert_eq!((r["visual"].clone(), r["hidden_visual"].clone()), (json!([]), json!(0)), "{r}");
}

#[test]
fn bad_causes_are_refused_with_the_allowed_names() {
    let mut a = Automation::new();
    let (old, new) = (make(&mut a, "A"), make(&mut a, "B"));
    for bad in [json!([]), json!(["images"]), json!(["other", 3]), json!("other")] {
        let e = call(&mut a, "doc_compare", json!({ "doc": new, "other": old, "visual": true, "causes": bad })).unwrap_err();
        assert!(e.contains("causes") || e.contains("array"), "{e}");
    }
    // null is the default, as for the other options.
    assert!(call(&mut a, "doc_compare", json!({ "doc": new, "other": old, "visual": true, "causes": null })).is_ok());
    let e = call(&mut a, "doc_compare", json!({ "doc": new, "other": old, "visual": true, "causes": ["images"] })).unwrap_err();
    assert!(e.contains("annotation, other"), "{e}");
}
