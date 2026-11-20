//! `accessibility_set_alts`: several figures' alternate text as one undo step, checked whole
//! before anything is written.

use pdfcraft_automation::{Automation, Content, ToolError};
use serde_json::{Value, json};

/// Three figures (objects 7, 8, 9; 9 already says "Old") and a non-figure (6) on one page.
fn pdf() -> Vec<u8> {
    let content = "/Figure << /MCID 0 >> BDC 10 150 40 20 re f EMC /Figure << /MCID 1 >> BDC 100 100 50 30 re f EMC /Figure << /MCID 2 >> BDC 100 20 50 30 re f EMC";
    format!(
        "%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R /MarkInfo << /Marked true >> /StructTreeRoot 5 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /StructParents 0 >> endobj
4 0 obj << /Length {} >> stream
{content}
endstream endobj
5 0 obj << /Type /StructTreeRoot /K 6 0 R /ParentTree 10 0 R >> endobj
6 0 obj << /S /Document /P 5 0 R /K [7 0 R 8 0 R 9 0 R] >> endobj
7 0 obj << /S /Figure /P 6 0 R /Pg 3 0 R /K 0 >> endobj
8 0 obj << /S /Figure /P 6 0 R /Pg 3 0 R /K 1 >> endobj
9 0 obj << /S /Figure /P 6 0 R /Pg 3 0 R /K 2 /Alt (Old) >> endobj
10 0 obj << /Nums [0 [7 0 R 8 0 R 9 0 R]] >> endobj
trailer << /Root 1 0 R >>
%%EOF",
        content.len()
    )
    .into_bytes()
}

fn run(a: &mut Automation, tool: &str, args: Value) -> Result<Value, ToolError> {
    a.call(tool, &args).map(|mut c| match c.remove(0) {
        Content::Json(v) => v,
        other => panic!("{tool}: expected JSON, got {other:?}"),
    })
}

fn ok(a: &mut Automation, tool: &str, args: Value) -> Value {
    run(a, tool, args).unwrap_or_else(|e| panic!("{tool}: {e}"))
}

fn open() -> (tempfile::TempDir, Automation, u64) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("f.pdf"), pdf()).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap();
    let doc = ok(&mut a, "doc_open", json!({ "path": "f.pdf" }))["doc"].as_u64().unwrap();
    (dir, a, doc)
}

fn alts(a: &mut Automation, doc: u64) -> Vec<(u64, Value)> {
    let f = ok(a, "accessibility_figures", json!({ "doc": doc }));
    f["figures"].as_array().unwrap().iter().map(|x| (x["figure"].as_u64().unwrap(), x["alt"].clone())).collect()
}

/// Whether the document has unsaved changes, and whether an undo step exists (taking it if so).
fn state(a: &mut Automation, doc: u64) -> (Value, bool) {
    let l = ok(a, "doc_list", json!({}));
    let dirty = l["documents"].as_array().unwrap().iter().find(|d| d["doc"] == doc).unwrap()["dirty"].clone();
    (dirty, run(a, "edit_undo", json!({ "doc": doc })).is_ok())
}

#[test]
fn several_figures_change_as_one_undo_step_with_exact_text() {
    let (dir, mut a, doc) = open();
    assert_eq!(alts(&mut a, doc), [(7, Value::Null), (8, Value::Null), (9, json!("Old"))]);
    let text = "  Ünï – 図 \"quoted\" & <b>  ";
    let r = ok(
        &mut a,
        "accessibility_set_alts",
        json!({ "doc": doc, "figures": [{ "figure": 7, "alt": text }, { "figure": 9, "alt": "" }, { "figure": 8, "decorative": true }] }),
    );
    assert_eq!(
        (r["changed"].clone(), r["set"].clone(), r["cleared"].clone(), r["decorative"].clone()),
        (json!(3), json!([7]), json!([9]), json!([8])),
        "{r}"
    );
    // Decorative figure 8 left the tags; 9 was cleared; 7 holds the text, trimmed and exact.
    assert_eq!(alts(&mut a, doc), [(7, json!(text.trim())), (9, Value::Null)]);
    ok(&mut a, "doc_save", json!({ "doc": doc, "path": "out.pdf" }));
    let d2 = ok(&mut a, "doc_open", json!({ "path": "out.pdf" }))["doc"].as_u64().unwrap();
    // The saved file numbers its objects afresh: compare the texts.
    let saved: Vec<Value> = alts(&mut a, d2).into_iter().map(|(_, t)| t).collect();
    assert_eq!(saved, [json!(text.trim()), Value::Null], "survives a save");
    // One undo restores all three.
    assert_eq!(ok(&mut a, "edit_undo", json!({ "doc": doc }))["undone"], "Set alternate text");
    assert_eq!(alts(&mut a, doc), [(7, Value::Null), (8, Value::Null), (9, json!("Old"))]);
    drop(dir);
}

#[test]
fn a_figure_whose_text_would_not_change_makes_no_undo_step() {
    let (_dir, mut a, doc) = open();
    let r = ok(&mut a, "accessibility_set_alts", json!({ "doc": doc, "figures": [{ "figure": 9, "alt": " Old " }, { "figure": 7, "alt": "  " }] }));
    assert_eq!((r["changed"].clone(), r["unchanged"].clone()), (json!(0), json!([9, 7])), "{r}");
    assert_eq!(state(&mut a, doc), (json!(false), false), "no undo step");
}

#[test]
fn a_refused_call_writes_nothing() {
    let (_dir, mut a, doc) = open();
    let before = alts(&mut a, doc);
    let good = json!({ "figure": 7, "alt": "fine" });
    let long = "x".repeat(4001);
    let many: Vec<Value> = (1..=101).map(|n| json!({ "figure": n, "alt": "a" })).collect();
    let bad: Vec<(&str, Value, bool)> = vec![
        ("not a figure", json!([good.clone(), { "figure": 6, "alt": "x" }]), false),
        ("absent figure", json!([good.clone(), { "figure": 99, "alt": "x" }]), false),
        ("repeated", json!([good.clone(), { "figure": 7, "alt": "y" }]), true),
        ("alt and decorative", json!([good.clone(), { "figure": 8, "alt": "x", "decorative": true }]), true),
        ("neither", json!([good.clone(), { "figure": 8 }]), true),
        ("decorative false and no alt", json!([good.clone(), { "figure": 8, "decorative": false }]), true),
        ("alt not a string", json!([good.clone(), { "figure": 8, "alt": 5 }]), true),
        ("decorative not a bool", json!([good.clone(), { "figure": 8, "decorative": "yes" }]), true),
        ("figure not a number", json!([good.clone(), { "figure": "8", "alt": "x" }]), true),
        ("figure zero", json!([good.clone(), { "figure": 0, "alt": "x" }]), true),
        ("unknown key", json!([good.clone(), { "figure": 8, "alt": "x", "title": "t" }]), true),
        ("item not an object", json!([good.clone(), 8]), true),
        ("alt over 4000 bytes", json!([good.clone(), { "figure": 8, "alt": long }]), true),
        ("empty list", json!([]), true),
        ("101 items", Value::Array(many), true),
        ("100 items pass the limit, then none is a figure", Value::Array((1..=100).map(|n| json!({ "figure": n, "alt": "a" })).collect()), false),
        ("not a list", json!({ "figure": 7, "alt": "x" }), true),
    ];
    for (what, figures, invalid) in bad {
        let r = run(&mut a, "accessibility_set_alts", json!({ "doc": doc, "figures": figures }));
        match (r, invalid) {
            (Err(ToolError::InvalidArgs(_)), true) | (Err(ToolError::Failed(_)), false) => {}
            (other, _) => panic!("{what}: unexpected {other:?}"),
        }
        assert_eq!(alts(&mut a, doc), before, "{what}: the figures changed");
        assert_eq!(state(&mut a, doc), (json!(false), false), "{what}: the document is marked changed or has an undo step");
    }
    // The byte cap's edge: 4000 bytes go through.
    let edge = "x".repeat(4000);
    let r = ok(&mut a, "accessibility_set_alts", json!({ "doc": doc, "figures": [{ "figure": 7, "alt": edge }] }));
    assert_eq!(r["changed"], 1);
}
