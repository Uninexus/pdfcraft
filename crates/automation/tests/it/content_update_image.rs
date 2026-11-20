//! `content_update` with an `image`: the new picture and the box and turns that come with it are
//! one edit, so a picture that cannot be used leaves the item as it was.

use pdfcraft_automation::{Automation, Content};
use serde_json::{Value, json};

fn pdf() -> Vec<u8> {
    let content = "0 g 10 10 20 20 re f";
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 300] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ];
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

fn png() -> Vec<u8> {
    let img = craft_image_codecs::Image::from_u8(40, 20, craft_image_codecs::ChannelLayout::Rgb, vec![200u8; 2400]).unwrap();
    craft_image_codecs::encode(&img, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default()).unwrap()
}

#[test]
fn an_image_that_cannot_be_read_leaves_the_item_unchanged() {
    let dir = std::env::temp_dir().join(format!("pdfcraft-content-update-image-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("in.pdf"), pdf()).unwrap();
    std::fs::write(dir.join("logo.png"), png()).unwrap();
    std::fs::write(dir.join("notes.txt"), b"this is not a picture").unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap().with_clock(|| 1_700_000_000);
    let doc = call(&mut a, "doc_open", json!({ "path": "in.pdf" }))["doc"].as_u64().unwrap();
    call(&mut a, "page_add_image", json!({ "doc": doc, "page": 1, "path": "logo.png", "rect": [100, 200, 180, 240] }));
    let before = call(&mut a, "content_list", json!({ "doc": doc }));

    let err = a
        .call(
            "content_update",
            &json!({ "doc": doc, "page": 1, "index": 1, "rect": [10, 10, 90, 50], "rotate": 90, "flip_h": true, "image": "notes.txt" }),
        )
        .expect_err("a text file is not a picture")
        .to_string();
    assert_eq!(err, "notes.txt: use a PNG, JPEG, JPEG 2000, TIFF, GIF or BMP image");
    assert_eq!(call(&mut a, "content_list", json!({ "doc": doc })), before, "the refused update moved nothing");

    //
    call(&mut a, "content_update", json!({ "doc": doc, "page": 1, "index": 1, "rect": [10, 10, 90, 50], "rotate": 90, "image": "logo.png" }));
    let after = call(&mut a, "content_list", json!({ "doc": doc }));
    assert_eq!(after["items"][0]["rect"], json!([10.0, 10.0, 90.0, 50.0]));
    // It is one undo step, labelled Replace image: one undo restores the item as it was.
    assert_eq!(call(&mut a, "edit_undo", json!({ "doc": doc }))["undone"], "Replace image");
    assert_eq!(call(&mut a, "content_list", json!({ "doc": doc })), before);
    let _ = std::fs::remove_dir_all(&dir);
}
