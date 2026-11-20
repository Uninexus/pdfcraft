//! `doc_guides`, `doc_guide_add`, `doc_guide_remove`, `doc_guide_snap` and `doc_grid_snap` end to end: guides are view state, bounded
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

#[test]
fn doc_guide_snap_picks_the_nearest_guide_within_reach_and_changes_nothing() {
    let dir = std::env::temp_dir().join(format!("pdfcraft-snap-{}-{:?}", std::process::id(), std::thread::current().id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("in.pdf"), pdf(&[("", "0 0 0 rg 50 100 70 80 re f"), ("", "")])).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap().with_clock(|| 1_700_000_000);
    let doc = call(&mut a, "doc_open", json!({ "path": "in.pdf" }))["doc"].as_u64().unwrap();
    call(&mut a, "doc_save", json!({ "doc": doc, "path": "before.pdf" }));
    for at in [50.0, 60.0] {
        call(&mut a, "doc_guide_add", json!({ "doc": doc, "page": 1, "axis": "vertical", "position": at }));
    }
    call(&mut a, "doc_guide_add", json!({ "doc": doc, "page": 1, "axis": "horizontal", "position": 120 }));
    call(&mut a, "doc_guide_add", json!({ "doc": doc, "page": 2, "axis": "vertical", "position": 10 }));

    // Nearest wins, and the horizontal guide does not catch an x.
    let v = call(&mut a, "doc_guide_snap", json!({ "doc": doc, "page": 1, "x": 57, "y": 118 }));
    assert_eq!(v["x"], json!({ "position": 60.0, "guide": 2, "snapped": true }), "{v}");
    assert_eq!(v["y"], json!({ "position": 120.0, "guide": 3, "snapped": true }), "{v}");
    assert_eq!(v["tolerance"], 4.0);
    // Out of reach (default 4, edge included), a bigger tolerance reaches, 0 needs an exact hit.
    let v = call(&mut a, "doc_guide_snap", json!({ "doc": doc, "page": 1, "x": 64.5 }));
    assert_eq!(v["x"], json!({ "position": 64.5, "guide": null, "snapped": false }));
    assert!(v.get("y").is_none());
    assert_eq!(call(&mut a, "doc_guide_snap", json!({ "doc": doc, "page": 1, "x": 64, "tolerance": 4 }))["x"]["guide"], 2);
    assert_eq!(call(&mut a, "doc_guide_snap", json!({ "doc": doc, "page": 1, "x": 64.5, "tolerance": 10 }))["x"]["guide"], 2);
    assert_eq!(call(&mut a, "doc_guide_snap", json!({ "doc": doc, "page": 1, "x": 50.5, "tolerance": 0 }))["x"]["snapped"], false);
    assert_eq!(call(&mut a, "doc_guide_snap", json!({ "doc": doc, "page": 1, "x": 50, "tolerance": 0 }))["x"]["guide"], 1);
    // A tie goes to the lower position; guides of another page and of a missing page do not count.
    assert_eq!(call(&mut a, "doc_guide_snap", json!({ "doc": doc, "page": 1, "x": 55, "tolerance": 5 }))["x"]["position"], 50.0);
    assert_eq!(call(&mut a, "doc_guide_snap", json!({ "doc": doc, "page": 2, "x": 55, "tolerance": 40 }))["x"]["snapped"], false);
    assert_eq!(call(&mut a, "doc_guide_snap", json!({ "doc": doc, "page": 9, "x": 10 }))["x"]["snapped"], false);

    // Hostile arguments are refused with something to act on.
    for (args, want) in [
        (json!({ "doc": doc, "page": 1 }), "x, y"),
        (json!({ "doc": doc, "page": 1, "x": 1, "tolerance": -1 }), "between 0 and 72"),
        (json!({ "doc": doc, "page": 1, "x": 1, "tolerance": 1e300 }), "between 0 and 72"),
        (json!({ "doc": doc, "page": 1, "x": "1" }), "x"),
        (json!({ "doc": doc, "page": 0, "x": 1 }), "page"),
        (json!({ "doc": doc, "x": 1 }), "page"),
    ] {
        let e = fails(&mut a, "doc_guide_snap", args.clone());
        assert!(e.contains(want), "{args}: {e}");
    }
    // Nothing changed: no guide added or removed, nothing unsaved.
    assert_eq!(call(&mut a, "doc_guides", json!({ "doc": doc }))["guides"].as_array().unwrap().len(), 4);
    assert_eq!(call(&mut a, "doc_info", json!({ "doc": doc }))["document"]["dirty"], false);
    call(&mut a, "doc_save", json!({ "doc": doc, "path": "after.pdf" }));
    assert_eq!(std::fs::read(dir.join("after.pdf")).unwrap(), std::fs::read(dir.join("before.pdf")).unwrap(), "snapping leaves the file as it was");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn doc_grid_snap_lands_on_the_nearest_grid_line_on_the_page_and_changes_nothing() {
    let dir = std::env::temp_dir().join(format!("pdfcraft-grid-{}-{:?}", std::process::id(), std::thread::current().id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("in.pdf"), pdf(&[("", "0 0 0 rg 50 100 70 80 re f"), ("/Rotate 90", "")])).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap().with_clock(|| 1_700_000_000);
    let doc = call(&mut a, "doc_open", json!({ "path": "in.pdf" }))["doc"].as_u64().unwrap();
    call(&mut a, "doc_save", json!({ "doc": doc, "path": "before.pdf" }));
    assert!(pdfcraft_automation::tools().iter().any(|t| t.name == "doc_grid_snap" && t.read_only));

    // The page is 200 by 300 points. A one-inch grid has lines at 0, 72 and 144 across.
    let v = call(&mut a, "doc_grid_snap", json!({ "doc": doc, "page": 1, "x": 100, "y": 250, "spacing": 1, "unit": "in" }));
    assert_eq!(v["x"], json!({ "position": 72.0, "snapped": true }), "{v}");
    assert_eq!(v["y"], json!({ "position": 216.0, "snapped": true }), "{v}");
    assert_eq!((v["spacing_pt"].clone(), v["pitch_pt"].clone(), v["tolerance"].clone()), (json!(72.0), json!(72.0), Value::Null));
    // Points are the default unit, and a lone x answers only x.
    let v = call(&mut a, "doc_grid_snap", json!({ "doc": doc, "page": 1, "x": 100, "spacing": 72 }));
    assert_eq!(v["x"]["position"], 72.0);
    assert!(v.get("y").is_none());
    // Subdivisions: lines every 18 points. An offset of half an inch moves the main lines to 36, 108.
    assert_eq!(call(&mut a, "doc_grid_snap", json!({ "doc": doc, "page": 1, "x": 100, "spacing": 72, "subdivisions": 4 }))["x"]["position"], 108.0);
    let v = call(&mut a, "doc_grid_snap", json!({ "doc": doc, "page": 1, "x": 95, "spacing": 1, "unit": "in", "offset_x": 0.5 }));
    assert_eq!(v["x"]["position"], 108.0, "{v}");
    // Millimetres and centimetres: 25.4 mm and 2.54 cm are one inch.
    assert_eq!(call(&mut a, "doc_grid_snap", json!({ "doc": doc, "page": 1, "x": 100, "spacing": 25.4, "unit": "mm" }))["x"]["position"], 72.0);
    assert_eq!(call(&mut a, "doc_grid_snap", json!({ "doc": doc, "page": 1, "x": 100, "spacing": 2.54, "unit": "cm" }))["pitch_pt"], 72.0);
    // A line past the page is not a place to land, though it is nearer: 240 is 50 from 190, 120 is 70.
    assert_eq!(
        call(&mut a, "doc_grid_snap", json!({ "doc": doc, "page": 1, "x": 190, "spacing": 120 }))["x"],
        json!({ "position": 120.0, "snapped": true })
    );
    // An unmoved position is returned as given, not rounded.
    assert_eq!(
        call(&mut a, "doc_grid_snap", json!({ "doc": doc, "page": 1, "x": 100.12345, "spacing": 72, "tolerance": 0 }))["x"],
        json!({ "position": 100.12345, "snapped": false })
    );
    // Page 2 is turned a quarter, so it is 300 wide and 200 tall: 288 is on it across, 288 is not down.
    let v = call(&mut a, "doc_grid_snap", json!({ "doc": doc, "page": 2, "x": 290, "y": 190, "spacing": 144 }));
    assert_eq!((v["x"]["position"].clone(), v["y"]["position"].clone()), (json!(288.0), json!(144.0)), "{v}");
    // Tolerance limits the reach (edge included); without it the nearest line always wins.
    assert_eq!(call(&mut a, "doc_grid_snap", json!({ "doc": doc, "page": 1, "x": 100, "spacing": 72, "tolerance": 28 }))["x"]["snapped"], true);
    let v = call(&mut a, "doc_grid_snap", json!({ "doc": doc, "page": 1, "x": 100, "spacing": 72, "tolerance": 27 }));
    assert_eq!(v["x"], json!({ "position": 100.0, "snapped": false }));
    assert_eq!(v["tolerance"], 27.0);

    // Hostile arguments are refused with something to act on.
    for (args, want) in [
        (json!({ "doc": doc, "page": 1, "spacing": 72 }), "x, y"),
        (json!({ "doc": doc, "page": 1, "x": 1 }), "spacing"),
        (json!({ "doc": doc, "page": 1, "x": 1, "spacing": 0 }), "between 0.01 and 14400"),
        (json!({ "doc": doc, "page": 1, "x": 1, "spacing": -5 }), "between 0.01 and 14400"),
        (json!({ "doc": doc, "page": 1, "x": 1, "spacing": 1e300, "unit": "in" }), "between 0.01 and 14400"),
        (json!({ "doc": doc, "page": 1, "x": 1, "spacing": 1, "unit": "furlong" }), "unit"),
        (json!({ "doc": doc, "page": 1, "x": 1, "spacing": 72, "subdivisions": 0 }), "subdivisions"),
        (json!({ "doc": doc, "page": 1, "x": 1, "spacing": 72, "subdivisions": -3 }), "subdivisions"),
        (json!({ "doc": doc, "page": 1, "x": 1, "spacing": 72, "subdivisions": 1e30 }), "subdivisions"),
        (json!({ "doc": doc, "page": 1, "x": 1, "spacing": 0.5, "subdivisions": 100 }), "too fine"),
        (json!({ "doc": doc, "page": 1, "x": 1, "spacing": 72, "offset_x": 1e300 }), "offset"),
        (json!({ "doc": doc, "page": 1, "x": 1, "spacing": 72, "tolerance": 73 }), "between 0 and 72"),
        (json!({ "doc": doc, "page": 1, "x": 201, "spacing": 72 }), "must lie on the page"),
        (json!({ "doc": doc, "page": 1, "y": -1, "spacing": 72 }), "must lie on the page"),
        (json!({ "doc": doc, "page": 3, "x": 1, "spacing": 72 }), "out of range"),
        (json!({ "doc": doc, "x": 1, "spacing": 72 }), "page"),
        (json!({ "doc": doc, "page": 1, "x": "1", "spacing": 72 }), "x"),
    ] {
        let e = fails(&mut a, "doc_grid_snap", args.clone());
        assert!(e.contains(want), "{args}: {e}");
    }
    // Nothing changed: no guide added, nothing unsaved, and the saved file is as it was.
    assert_eq!(call(&mut a, "doc_guides", json!({ "doc": doc })), json!({ "guides": [] }));
    assert_eq!(call(&mut a, "doc_info", json!({ "doc": doc }))["document"]["dirty"], false);
    call(&mut a, "doc_save", json!({ "doc": doc, "path": "after.pdf" }));
    assert_eq!(std::fs::read(dir.join("after.pdf")).unwrap(), std::fs::read(dir.join("before.pdf")).unwrap(), "snapping leaves the file as it was");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// What `grid_preferences` stores, as the tool answers it.
fn stored(a: &mut Automation) -> Value {
    call(a, "grid_preferences", json!({}))
}

#[test]
fn grid_preferences_are_set_read_cleared_and_fill_in_what_doc_grid_snap_is_not_given() {
    let dir = std::env::temp_dir().join(format!("pdfcraft-gridprefs-{}-{:?}", std::process::id(), std::thread::current().id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("in.pdf"), pdf(&[("", "")])).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap().with_clock(|| 1_700_000_000);
    let doc = call(&mut a, "doc_open", json!({ "path": "in.pdf" }))["doc"].as_u64().unwrap();
    // It changes session state, so it is not a read-only tool; doc_grid_snap still is.
    assert!(pdfcraft_automation::tools().iter().any(|t| t.name == "grid_preferences" && !t.read_only));
    assert!(pdfcraft_automation::tools().iter().any(|t| t.name == "doc_grid_snap" && t.read_only));

    // Nothing stored: as before, the spacing is required and the rest default.
    assert_eq!(stored(&mut a), json!({ "set": false }));
    assert!(fails(&mut a, "doc_grid_snap", json!({ "doc": doc, "page": 1, "x": 100 })).contains("store a grid with grid_preferences"));
    let v = call(&mut a, "doc_grid_snap", json!({ "doc": doc, "page": 1, "x": 100, "spacing": 72 }));
    assert_eq!(v["from_preferences"], json!([]));
    // doc_grid_snap converts units the same exact way: 5080 mm is a spacing of exactly 14400 points.
    let v = call(&mut a, "doc_grid_snap", json!({ "doc": doc, "page": 1, "x": 100, "spacing": 5080, "unit": "mm" }));
    assert_eq!(v["spacing_pt"], 14400.0);

    // A one-inch grid, given in inches; stored and answered in points.
    let v = call(&mut a, "grid_preferences", json!({ "spacing": 1, "unit": "in" }));
    assert_eq!(v, json!({ "set": true, "spacing_pt": 72.0, "subdivisions": 1, "pitch_pt": 72.0, "offset_x_pt": 0.0, "offset_y_pt": 0.0 }));
    assert_eq!(stored(&mut a), v);
    // Giving the same values again leaves the same grid (the tool is announced as idempotent).
    assert_eq!(call(&mut a, "grid_preferences", json!({ "spacing": 1, "unit": "in" })), v);
    let v = call(&mut a, "doc_grid_snap", json!({ "doc": doc, "page": 1, "x": 100, "y": 250 }));
    assert_eq!((v["x"]["position"].clone(), v["y"]["position"].clone(), v["spacing_pt"].clone()), (json!(72.0), json!(216.0), json!(72.0)));
    assert_eq!(v["from_preferences"], json!(["spacing", "offset_x", "offset_y", "subdivisions"]));

    // A later call changes only what it names; the unit applies to what that call gives.
    let v = call(&mut a, "grid_preferences", json!({ "subdivisions": 4, "offset_x": 0.5, "unit": "in" }));
    assert_eq!(
        (v["spacing_pt"].clone(), v["subdivisions"].clone(), v["offset_x_pt"].clone(), v["pitch_pt"].clone()),
        (json!(72.0), json!(4), json!(36.0), json!(18.0))
    );
    // A call's own value wins over the stored one, which stays stored in points whatever unit is named.
    let v = call(&mut a, "doc_grid_snap", json!({ "doc": doc, "page": 1, "x": 95, "spacing": 144, "offset_x": 0 }));
    assert_eq!(
        (v["x"]["position"].clone(), v["pitch_pt"].clone(), v["from_preferences"].clone()),
        (json!(108.0), json!(36.0), json!(["offset_y", "subdivisions"]))
    );
    let v = call(&mut a, "doc_grid_snap", json!({ "doc": doc, "page": 1, "x": 95, "unit": "mm" }));
    assert_eq!((v["x"]["position"].clone(), v["spacing_pt"].clone()), (json!(90.0), json!(72.0)));
    assert_eq!(stored(&mut a)["offset_x_pt"], 36.0, "naming a unit to snap does not convert the stored grid");

    // Cleared: back to requiring a spacing, and subdivisions default to 1 again.
    assert_eq!(call(&mut a, "grid_preferences", json!({ "clear": true })), json!({ "set": false }));
    assert_eq!(call(&mut a, "grid_preferences", json!({ "clear": false })), json!({ "set": false }));
    assert!(fails(&mut a, "doc_grid_snap", json!({ "doc": doc, "page": 1, "x": 100 })).contains("spacing"));
    let v = call(&mut a, "doc_grid_snap", json!({ "doc": doc, "page": 1, "x": 100, "spacing": 72 }));
    assert_eq!(v["pitch_pt"], 72.0);
    // From nothing stored, a lone value starts from one inch, no subdivisions and no offset.
    let v = call(&mut a, "grid_preferences", json!({ "subdivisions": 3 }));
    assert_eq!((v["spacing_pt"].clone(), v["subdivisions"].clone(), v["offset_y_pt"].clone()), (json!(72.0), json!(3), json!(0.0)));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn grid_preferences_accept_the_limits_and_refuse_beyond_them_storing_nothing() {
    let mut a = Automation::new();
    // Exactly at each limit.
    assert_eq!(call(&mut a, "grid_preferences", json!({ "spacing": 14400 }))["spacing_pt"], 14400.0);
    assert_eq!(call(&mut a, "grid_preferences", json!({ "spacing": 200, "unit": "in" }))["spacing_pt"], 14400.0);
    // Units divide last, so a value exactly at a limit stays exact (a pre-divided 72/25.4 gave 14400.000000000002).
    assert_eq!(call(&mut a, "grid_preferences", json!({ "spacing": 5080, "unit": "mm" }))["spacing_pt"], 14400.0);
    assert_eq!(call(&mut a, "grid_preferences", json!({ "spacing": 508, "unit": "cm" }))["spacing_pt"], 14400.0);
    assert_eq!(call(&mut a, "grid_preferences", json!({ "spacing": 25.4, "unit": "mm" }))["spacing_pt"], 72.0);
    assert_eq!(call(&mut a, "grid_preferences", json!({ "spacing": 2.54, "unit": "cm" }))["spacing_pt"], 72.0);
    // An offset in another unit, and a negative zero stored as 0.
    let v = call(&mut a, "grid_preferences", json!({ "offset_x": 13888, "offset_y": 0.25, "unit": "in" }));
    assert_eq!((v["offset_x_pt"].clone(), v["offset_y_pt"].clone()), (json!(999936.0), json!(18.0)));
    let v = call(&mut a, "grid_preferences", json!({ "offset_x": -0.0, "offset_y": -0.0, "unit": "mm" }));
    assert_eq!((v["offset_x_pt"].to_string(), v["offset_y_pt"].to_string()), ("0.0".to_string(), "0.0".to_string()));
    assert_eq!(call(&mut a, "grid_preferences", json!({ "spacing": 0.01 }))["pitch_pt"], 0.01);
    let v = call(&mut a, "grid_preferences", json!({ "spacing": 1, "subdivisions": 100 }));
    assert_eq!((v["subdivisions"].clone(), v["pitch_pt"].clone()), (json!(100), json!(0.01)));
    let v = call(&mut a, "grid_preferences", json!({ "spacing": 72, "subdivisions": 1, "offset_x": 1000000, "offset_y": -1000000 }));
    assert_eq!((v["offset_x_pt"].clone(), v["offset_y_pt"].clone()), (json!(1e6), json!(-1e6)));

    let before = call(&mut a, "grid_preferences", json!({ "spacing": 36, "subdivisions": 2, "offset_x": 3, "offset_y": 4 }));
    for (args, want) in [
        (json!({ "spacing": 14400.01 }), "between 0.01 and 14400"),
        (json!({ "spacing": 0.009 }), "between 0.01 and 14400"),
        (json!({ "spacing": 0 }), "between 0.01 and 14400"),
        (json!({ "spacing": -1 }), "between 0.01 and 14400"),
        (json!({ "spacing": 1e300, "unit": "in" }), "between 0.01 and 14400"),
        (json!({ "spacing": 201, "unit": "in" }), "between 0.01 and 14400"),
        (json!({ "spacing": 5080.01, "unit": "mm" }), "between 0.01 and 14400"),
        (json!({ "offset_x": 13889, "unit": "in" }), "offset"),
        (json!({ "subdivisions": 101 }), "between 1 and 100"),
        (json!({ "subdivisions": 0 }), "between 1 and 100"),
        (json!({ "subdivisions": -3 }), "subdivisions"),
        (json!({ "subdivisions": 1e30 }), "subdivisions"),
        (json!({ "spacing": 0.5, "subdivisions": 100 }), "too fine"),
        (json!({ "offset_x": 1000000.5 }), "offset"),
        (json!({ "offset_y": -1e300 }), "offset"),
        (json!({ "offset_x": 1e300, "unit": "cm" }), "offset"),
        (json!({ "unit": "furlong" }), "unit"),
        (json!({ "spacing": 72, "unit": "furlong" }), "unit"),
        (json!({ "spacing": "1" }), "spacing"),
        (json!({ "spacing": 72, "offset_x": "1" }), "offset_x"),
        (json!({ "spacing": 72, "clear": "yes" }), "clear"),
        (json!({ "spacing": 72, "clear": true }), "not both"),
        (json!({ "offset_y": 1, "clear": true }), "not both"),
    ] {
        let e = fails(&mut a, "grid_preferences", args.clone());
        assert!(e.contains(want), "{args}: {e}");
        assert_eq!(stored(&mut a), before, "{args} stored something");
    }
    // A refusal caused only by merging with what is stored: 0.5 points in 100 parts is too fine.
    let half = call(&mut a, "grid_preferences", json!({ "spacing": 0.5, "subdivisions": 1, "offset_x": 0, "offset_y": 0 }));
    assert!(fails(&mut a, "grid_preferences", json!({ "subdivisions": 100 })).contains("too fine"));
    assert_eq!(stored(&mut a), half, "the merged grid was refused, so the stored one stays");
    // The same checks stand when nothing is stored yet: a refused call leaves it unset.
    let mut b = Automation::new();
    assert!(fails(&mut b, "grid_preferences", json!({ "subdivisions": 101 })).contains("between 1 and 100"));
    assert!(fails(&mut b, "grid_preferences", json!({ "spacing": 14400.01 })).contains("between 0.01 and 14400"));
    assert_eq!(stored(&mut b), json!({ "set": false }));
    // The stored grid belongs to one tool session.
    let mut c = Automation::new();
    assert_eq!(stored(&mut c), json!({ "set": false }));
}
