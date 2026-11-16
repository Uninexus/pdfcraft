//! `doc_guides`, `doc_guide_add` and `doc_guide_remove` end to end: guides are view state, bounded
//! by the displayed page, and never reach the saved file.

use pdfcraft_automation::{Automation, Content};
use serde_json::{Value, json};

/// A PDF of `pages`, each (extra page-dictionary entries, content stream), all Letter-ish 200×300
/// unless an entry says otherwise.
fn pdf(pages: &[(&str, &str)]) -> Vec<u8> {
    let n = pages.len();
    let mut objs: Vec<String> = vec!["<< /Type /Catalog /Pages 2 0 R >>".into()];
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + 2 * i)).collect();
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {n} /MediaBox [0 0 200 300] >>", kids.join(" ")));
    for (i, (extra, content)) in pages.iter().enumerate() {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /Contents {} 0 R {extra} >>", 4 + 2 * i));
        objs.push(format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()));
    }
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

fn call(a: &mut Automation, tool: &str, args: Value) -> Value {
    match a.call(tool, &args).unwrap_or_else(|e| panic!("{tool}: {e}")).remove(0) {
        Content::Json(v) => v,
        _ => panic!("{tool}: expected JSON"),
    }
}

fn fails(a: &mut Automation, tool: &str, args: Value) -> String {
    a.call(tool, &args).expect_err(tool).to_string()
}

#[test]
fn guides_are_listed_added_and_removed_without_touching_the_file() {
    let dir = std::env::temp_dir().join(format!("pdfcraft-guides-{}-{:?}", std::process::id(), std::thread::current().id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let bytes = pdf(&[("", "0 0 0 rg 50 100 70 80 re f"), ("/Rotate 90", "0 0 1 rg 20 40 40 50 re f")]);
    std::fs::write(dir.join("in.pdf"), &bytes).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap().with_clock(|| 1_700_000_000);
    let doc = call(&mut a, "doc_open", json!({ "path": "in.pdf" }))["doc"].as_u64().unwrap();
    assert_eq!(call(&mut a, "doc_guides", json!({ "doc": doc })), json!({ "guides": [] }));
    call(&mut a, "doc_save", json!({ "doc": doc, "path": "before.pdf" }));

    let v = call(&mut a, "doc_guide_add", json!({ "doc": doc, "page": 1, "axis": "vertical", "position": 72.5 }));
    assert_eq!(v, json!({ "id": 1, "page": 1, "axis": "vertical", "position": 72.5, "created": true }));
    let again = call(&mut a, "doc_guide_add", json!({ "doc": doc, "page": 1, "axis": "vertical", "position": 72.5 }));
    assert_eq!(again["created"], false);
    assert_eq!(again["id"], 1);
    // Page 2 is turned a quarter, so it is displayed 300 wide and 200 tall.
    call(&mut a, "doc_guide_add", json!({ "doc": doc, "page": 2, "axis": "vertical", "position": 300 }));
    let h = call(&mut a, "doc_guide_add", json!({ "doc": doc, "page": 2, "axis": "horizontal", "position": 100 }));
    assert_eq!(h["id"], 3);
    assert!(fails(&mut a, "doc_guide_add", json!({ "doc": doc, "page": 2, "axis": "horizontal", "position": 250 })).contains("between 0 and 200"));
    assert!(fails(&mut a, "doc_guide_add", json!({ "doc": doc, "page": 1, "axis": "diagonal", "position": 1 })).contains("axis"));
    assert!(fails(&mut a, "doc_guide_add", json!({ "doc": doc, "page": 3, "axis": "vertical", "position": 1 })).contains("out of range"));
    assert!(fails(&mut a, "doc_guide_add", json!({ "doc": doc, "page": 1, "axis": "vertical", "position": -4 })).contains("must lie on the page"));

    assert_eq!(call(&mut a, "doc_guides", json!({ "doc": doc }))["guides"].as_array().unwrap().len(), 3);
    let page2 = call(&mut a, "doc_guides", json!({ "doc": doc, "page": 2 }));
    assert_eq!(page2["guides"].as_array().unwrap().len(), 2, "{page2}");

    // They are not an edit: nothing to undo, nothing unsaved, and the saved file is unchanged.
    let info = call(&mut a, "doc_info", json!({ "doc": doc }));
    assert_eq!(info["document"]["dirty"], false, "{info}");
    assert!(fails(&mut a, "edit_undo", json!({ "doc": doc })).contains("nothing to undo"));
    call(&mut a, "doc_save", json!({ "doc": doc, "path": "out.pdf" }));
    assert_eq!(std::fs::read(dir.join("out.pdf")).unwrap(), std::fs::read(dir.join("before.pdf")).unwrap(), "the saved file carries no guides");

    assert_eq!(call(&mut a, "doc_guide_remove", json!({ "doc": doc, "id": 1 })), json!({ "removed": 1 }));
    assert_eq!(call(&mut a, "doc_guide_remove", json!({ "doc": doc, "id": 1 })), json!({ "removed": 0 }));
    assert_eq!(call(&mut a, "doc_guide_remove", json!({ "doc": doc, "all": true, "page": 2 })), json!({ "removed": 2 }));
    assert!(fails(&mut a, "doc_guide_remove", json!({ "doc": doc })).contains("either id, or all"));
    assert!(fails(&mut a, "doc_guide_remove", json!({ "doc": doc, "id": 1, "all": true })).contains("either id, or all"));
    assert_eq!(call(&mut a, "doc_guides", json!({ "doc": doc })), json!({ "guides": [] }));
    assert!(pdfcraft_automation::tools().iter().any(|t| t.name == "doc_guides" && t.read_only));

    // Rounding, -0, bad ids, and a guide on a page that is gone can still be removed.
    let z = call(&mut a, "doc_guide_add", json!({ "doc": doc, "page": 1, "axis": "horizontal", "position": -0.0 }));
    assert_eq!(z["position"].to_string(), "0.0");
    let r = call(&mut a, "doc_guide_add", json!({ "doc": doc, "page": 1, "axis": "vertical", "position": 50.0004 }));
    let r2 = call(&mut a, "doc_guide_add", json!({ "doc": doc, "page": 1, "axis": "vertical", "position": 50.0 }));
    assert_eq!((r["id"].clone(), r2["created"].clone()), (r2["id"].clone(), json!(false)));
    assert!(fails(&mut a, "doc_guide_remove", json!({ "doc": doc, "id": 0 })).contains("id"));
    call(&mut a, "doc_guide_add", json!({ "doc": doc, "page": 2, "axis": "vertical", "position": 7 }));
    call(&mut a, "page_delete", json!({ "doc": doc, "pages": [2] }));
    let stale = call(&mut a, "doc_guides", json!({ "doc": doc, "page": 2 }));
    assert_eq!(stale["guides"].as_array().unwrap().len(), 1, "a guide on a deleted page is still listed: {stale}");
    assert_eq!(call(&mut a, "doc_guide_remove", json!({ "doc": doc, "all": true, "page": 2 })), json!({ "removed": 1 }));
    assert_eq!(call(&mut a, "doc_guides", json!({ "doc": doc, "page": 9 })), json!({ "guides": [] }));
    let _ = std::fs::remove_dir_all(&dir);
}
