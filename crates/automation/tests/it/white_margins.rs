//! `page_remove_white_margins` end to end, on synthetic pages whose painted rectangles sit at
//! known places: the saved and reopened crop boxes are the rectangles' bounds.

use std::path::Path;
use std::sync::Arc;

use pdfcraft_automation::{Automation, Content, ToolError};
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

/// Every page's crop box as the saved file at `path` has it.
fn crop_boxes(path: &Path) -> Vec<[f64; 4]> {
    let doc = pdfcraft_cos::Document::open(Arc::new(std::fs::read(path).unwrap())).unwrap();
    pdfcraft_organize::page_boxes(&doc).unwrap().into_iter().map(|b| b[1]).collect()
}

fn near(got: [f64; 4], want: [f64; 4]) {
    assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 1.0), "crop box {got:?} is not {want:?} within a point");
}

/// Four pages: a rectangle low on the left; blank; a rotated page; a page already cropped, whose
/// rectangle reaches past its crop box.
const PAGES: [(&str, &str); 4] = [
    ("", "0 0 0 rg 50 100 70 80 re f"),
    ("", "1 1 1 rg 10 10 100 100 re f"),
    ("/Rotate 90", "0 0 1 rg 20 40 40 50 re f"),
    ("/CropBox [10 10 190 290]", "1 0 0 rg 5 20 25 20 re f"),
];

fn open(test: &str, pages: &[(&str, &str)]) -> (std::path::PathBuf, Automation, u64) {
    let dir = std::env::temp_dir().join(format!("pdfcraft-margins-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("in.pdf"), pdf(pages)).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap().with_clock(|| 1_700_000_000);
    let doc = call(&mut a, "doc_open", json!({ "path": "in.pdf" }))["doc"].as_u64().unwrap();
    (dir, a, doc)
}

#[test]
fn every_page_is_cropped_to_its_content_and_a_blank_page_is_left_alone() {
    let (dir, mut a, doc) = open("all", &PAGES);
    let r = call(&mut a, "page_remove_white_margins", json!({ "doc": doc }));
    assert_eq!(r["blank"], json!([2]), "the white page is reported, not cropped");
    assert_eq!(r["unchanged"], json!([]));
    assert_eq!(r["failed"], json!([]));
    let cropped: Vec<u64> = r["cropped"].as_array().unwrap().iter().map(|c| c["page"].as_u64().unwrap()).collect();
    assert_eq!(cropped, [1, 3, 4]);
    call(&mut a, "doc_save", json!({ "doc": doc, "path": "out.pdf" }));
    let boxes = crop_boxes(&dir.join("out.pdf"));
    near(boxes[0], [50.0, 100.0, 120.0, 180.0]);
    near(boxes[1], [0.0, 0.0, 200.0, 300.0]);
    // /Rotate 90 doesn't change which user-space rectangle the paint is in.
    near(boxes[2], [20.0, 40.0, 60.0, 90.0]);
    // The rectangle starts left of and below the old crop box: the new box stays inside it.
    near(boxes[3], [10.0, 20.0, 30.0, 40.0]);
    // The view shows the new sizes: page 3 is rotated, so its width is the box's height.
    assert_eq!(r["page_sizes"][0], json!([70.0, 80.0]));
    assert_eq!(r["page_sizes"][2], json!([50.0, 40.0]));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn padding_selection_and_undo() {
    let (dir, mut a, doc) = open("pad", &PAGES);
    // Only page 1, with 5 points of padding; the other pages are not touched.
    let r = call(&mut a, "page_remove_white_margins", json!({ "doc": doc, "pages": [1], "padding": 5 }));
    assert_eq!(r["cropped"].as_array().unwrap().len(), 1);
    assert_eq!(r["blank"], json!([]), "pages outside the selection aren't looked at");
    call(&mut a, "doc_save", json!({ "doc": doc, "path": "out.pdf" }));
    let boxes = crop_boxes(&dir.join("out.pdf"));
    near(boxes[0], [45.0, 95.0, 125.0, 185.0]);
    near(boxes[2], [0.0, 0.0, 200.0, 300.0]);
    // Padding never grows the crop box: 500 points of it is the box the page already has.
    let r = call(&mut a, "page_remove_white_margins", json!({ "doc": doc, "pages": [1], "padding": 500 }));
    assert_eq!(r["cropped"], json!([]));
    assert_eq!(r["unchanged"], json!([1]));
    // Undo is one step for the whole call.
    call(&mut a, "page_remove_white_margins", json!({ "doc": doc }));
    call(&mut a, "edit_undo", json!({ "doc": doc }));
    call(&mut a, "doc_save", json!({ "doc": doc, "path": "undone.pdf" }));
    let boxes = crop_boxes(&dir.join("undone.pdf"));
    near(boxes[0], [45.0, 95.0, 125.0, 185.0]);
    near(boxes[2], [0.0, 0.0, 200.0, 300.0]);
    near(boxes[3], [10.0, 10.0, 190.0, 290.0]);
    call(&mut a, "edit_undo", json!({ "doc": doc }));
    call(&mut a, "doc_save", json!({ "doc": doc, "path": "undone2.pdf" }));
    near(crop_boxes(&dir.join("undone2.pdf"))[0], [0.0, 0.0, 200.0, 300.0]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn user_unit_and_bad_arguments() {
    // UserUnit 2: a 100×150 box of user units is 200×300 points, the rectangle 10..40 × 20..60.
    let (dir, mut a, doc) = open("unit", &[("/MediaBox [0 0 100 150] /UserUnit 2", "0 g 10 20 30 40 re f")]);
    // 10 points of padding is 5 user units.
    call(&mut a, "page_remove_white_margins", json!({ "doc": doc, "padding": 10 }));
    call(&mut a, "doc_save", json!({ "doc": doc, "path": "out.pdf" }));
    near(crop_boxes(&dir.join("out.pdf"))[0], [5.0, 15.0, 45.0, 65.0]);
    for bad in [json!({ "doc": doc, "padding": -1 }), json!({ "doc": doc, "padding": 1e9 }), json!({ "doc": doc, "pages": [9] })] {
        assert!(matches!(a.call("page_remove_white_margins", &bad), Err(ToolError::InvalidArgs(_))), "{bad}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// The saved crop box of each page of `pages` after removing white margins: it holds `want`
/// (never cutting the content) and is within `tol` of it. Edges of `want` sit on the raster's
/// half-point grid, so the bounds are exact.
fn assert_tight(test: &str, pages: &[(&str, &str)], want: &[[f64; 4]], tol: f64) {
    let (dir, mut a, doc) = open(test, pages);
    call(&mut a, "page_remove_white_margins", json!({ "doc": doc }));
    call(&mut a, "doc_save", json!({ "doc": doc, "path": "out.pdf" }));
    for (i, (got, want)) in crop_boxes(&dir.join("out.pdf")).into_iter().zip(want).enumerate() {
        let holds = got[0] <= want[0] + 1e-6 && got[1] <= want[1] + 1e-6 && got[2] >= want[2] - 1e-6 && got[3] >= want[3] - 1e-6;
        assert!(holds, "page {}: crop box {got:?} cuts into the content {want:?}", i + 1);
        assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= tol), "page {}: crop box {got:?} is not within {tol} of {want:?}", i + 1);
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_page_that_is_not_a_whole_number_of_points_is_cropped_without_cutting_content() {
    // 200.25 pt is 400.5 pixels at 2 pixels a point, drawn 401 wide: a pixel is half a point, not 200.25 / 401.
    assert_tight("fraction", &[("/MediaBox [0 0 200.25 300.5]", "0 g 100 100.5 100 150 re f")], &[[100.0, 100.5, 200.0, 250.5]], 0.05);
}

#[test]
fn rotated_pages_with_a_crop_box_origin_and_tiny_content() {
    let rect = "0 g 20 40 40 50 re f";
    assert_tight("rot180", &[("/Rotate 180", rect)], &[[20.0, 40.0, 60.0, 90.0]], 0.05);
    assert_tight("rot270", &[("/Rotate 270", rect)], &[[20.0, 40.0, 60.0, 90.0]], 0.05);
    // The crop box doesn't start at the origin, and the page is turned.
    assert_tight("origin", &[("/Rotate 90 /CropBox [10 20 190 280]", "0 g 30 60 50 40 re f")], &[[30.0, 60.0, 80.0, 100.0]], 0.05);
    assert_tight("origin270", &[("/Rotate 270 /CropBox [10 20 190 280]", "0 g 30 60 50 40 re f")], &[[30.0, 60.0, 80.0, 100.0]], 0.05);
    // Content under a point wide gets the one-point minimum, around it.
    assert_tight("tiny", &[("", "0 g 100 100 0.5 0.5 re f")], &[[100.0, 100.0, 100.5, 100.5]], 0.51);
}
