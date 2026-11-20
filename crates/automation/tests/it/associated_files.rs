//! `af_list`, `af_add` and `af_remove`: PDF 2.0 associated files (`/AF`) at document level,
//! read back from the saved file's objects.

use pdfcraft_automation::{Automation, Content, ToolError};
use pdfcraft_cos::{Document, Object, PdfString};
use pdfcraft_engine::Session;
use serde_json::{Value, json};
use std::sync::Arc;

fn tool(a: &mut Automation, name: &str, args: Value) -> Value {
    match a.call(name, &args).unwrap().remove(0) {
        Content::Json(value) => value,
        other => panic!("expected JSON: {other:?}"),
    }
}

fn setup() -> (tempfile::TempDir, Automation) {
    let dir = tempfile::tempdir().unwrap();
    let mut session = Session::new();
    let bytes = session.create_blank(300.0, 400.0, 2).unwrap();
    let id = session.open("a.pdf", None, bytes, None).unwrap();
    std::fs::write(dir.path().join("a.pdf"), session.save_bytes(id).unwrap().as_slice()).unwrap();
    std::fs::write(dir.path().join("table.csv"), b"a,b\n1,2\n").unwrap();
    std::fs::write(dir.path().join("src.xml"), b"<doc/>").unwrap();
    let auto = Automation::new().with_root(dir.path()).unwrap();
    (dir, auto)
}

fn open(a: &mut Automation, path: &str) -> u64 {
    tool(a, "doc_open", json!({ "path": path }))["doc"].as_u64().unwrap()
}

#[test]
fn an_associated_file_is_added_listed_saved_undone_and_removed() {
    let (dir, mut a) = setup();
    let doc = open(&mut a, "a.pdf");
    assert_eq!(tool(&mut a, "af_list", json!({ "doc": doc }))["count"], 0);

    let added = tool(&mut a, "af_add", json!({ "doc": doc, "path": "table.csv", "relationship": "Data", "description": "Figures" }));
    assert_eq!(
        (added["index"].as_u64(), added["name"].as_str(), added["relationship"].as_str()),
        (Some(1), Some("table.csv"), Some("Data")),
        "{added}"
    );
    tool(&mut a, "af_add", json!({ "doc": doc, "path": "src.xml", "relationship": "Source", "name": "origin.xml" }));
    let listed = tool(&mut a, "af_list", json!({ "doc": doc }));
    assert_eq!(listed["count"], 2, "{listed}");
    let first = &listed["associated_files"][0];
    assert_eq!(
        (first["level"].as_str(), first["relationship"].as_str(), first["name"].as_str(), first["size"].as_u64(), first["embedded"].as_bool()),
        (Some("document"), Some("Data"), Some("table.csv"), Some(8), Some(true)),
        "{listed}"
    );

    // One undo step per add.
    tool(&mut a, "edit_undo", json!({ "doc": doc }));
    assert_eq!(tool(&mut a, "af_list", json!({ "doc": doc }))["count"], 1);
    tool(&mut a, "edit_redo", json!({ "doc": doc }));
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "out.pdf" }));

    // The saved file's own objects, read without the tools.
    let bytes = std::fs::read(dir.path().join("out.pdf")).unwrap();
    let cos = Document::open(Arc::new(bytes)).unwrap();
    let catalog = cos.get(cos.root().unwrap()).as_dict().unwrap().clone();
    let list = cos.resolve(catalog.get(b"AF").unwrap()).as_array().cloned().unwrap();
    assert_eq!(list.len(), 2);
    let spec = cos.resolve(&list[0]);
    let spec = spec.as_dict().unwrap();
    assert_eq!(spec.get(b"Type"), Some(&Object::name("Filespec")));
    assert_eq!(spec.get(b"AFRelationship"), Some(&Object::name("Data")));
    assert_eq!(spec.get(b"UF").and_then(Object::as_string).map(PdfString::to_text).as_deref(), Some("table.csv"));
    assert_eq!(spec.get(b"Desc").and_then(Object::as_string).map(PdfString::to_text).as_deref(), Some("Figures"));
    let stream = spec.get(b"EF").and_then(Object::as_dict).unwrap().get(b"F").cloned().unwrap();
    let Object::Stream(stream) = cos.resolve(&stream).as_ref().clone() else { panic!("embedded file stream") };
    assert_eq!(stream.dict.get(b"Type"), Some(&Object::name("EmbeddedFile")));
    assert_eq!(stream.decoded().unwrap(), b"a,b\n1,2\n");
    let second = cos.resolve(&list[1]);
    assert_eq!(second.as_dict().unwrap().get(b"AFRelationship"), Some(&Object::name("Source")));

    // Reopened, the tool reads the same, then removes the first and its attachment.
    let re = open(&mut a, "out.pdf");
    let listed = tool(&mut a, "af_list", json!({ "doc": re }));
    assert_eq!(listed["associated_files"][1]["name"], "origin.xml", "{listed}");
    tool(&mut a, "af_remove", json!({ "doc": re, "index": 1 }));
    tool(&mut a, "doc_save", json!({ "doc": re, "path": "out2.pdf" }));
    let again = open(&mut a, "out2.pdf");
    let listed = tool(&mut a, "af_list", json!({ "doc": again }));
    assert_eq!((listed["count"].as_u64(), listed["associated_files"][0]["name"].as_str()), (Some(1), Some("origin.xml")), "{listed}");
    let info = tool(&mut a, "doc_info", json!({ "doc": again }));
    assert!(!info.to_string().contains("table.csv"), "its attachment went too: {info}");
    // Keeping the attachment removes only the association.
    tool(&mut a, "af_remove", json!({ "doc": again, "index": 1, "keep_attachment": true }));
    assert_eq!(tool(&mut a, "af_list", json!({ "doc": again }))["count"], 0);
    assert!(tool(&mut a, "doc_info", json!({ "doc": again })).to_string().contains("origin.xml"));
}

#[test]
fn refusals_change_nothing_and_a_modify_restriction_holds() {
    let (dir, mut a) = setup();
    let doc = open(&mut a, "a.pdf");
    for relationship in ["EncryptedPayload", "Banana"] {
        let r = a.call("af_add", &json!({ "doc": doc, "path": "table.csv", "relationship": relationship }));
        assert!(matches!(r, Err(ToolError::Failed(_) | ToolError::InvalidArgs(_))), "{relationship}: {r:?}");
    }
    assert!(a.call("af_add", &json!({ "doc": doc, "path": "table.csv" })).is_err(), "relationship is required");
    assert!(a.call("af_remove", &json!({ "doc": doc, "index": 1 })).is_err());
    assert!(a.call("af_remove", &json!({ "doc": doc, "index": 0 })).is_err());
    assert_eq!(tool(&mut a, "af_list", json!({ "doc": doc }))["count"], 0);
    assert_eq!(tool(&mut a, "doc_info", json!({ "doc": doc }))["attachments"].as_array().map(Vec::len), Some(0));

    // A document that does not allow changes refuses.
    tool(&mut a, "doc_protect", json!({ "doc": doc, "open_password": "open", "permissions_password": "boss", "changes": "none" }));
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "locked.pdf" }));
    let locked = tool(&mut a, "doc_open", json!({ "path": "locked.pdf", "password": "open" }))["doc"].as_u64().unwrap();
    let r = a.call("af_add", &json!({ "doc": locked, "path": "table.csv", "relationship": "Data" }));
    match r {
        Err(ToolError::Failed(m)) => assert!(m.contains("allow") || m.contains("permission"), "the message names the restriction: {m}"),
        other => panic!("refused: {other:?}"),
    }
    assert_eq!(tool(&mut a, "af_list", json!({ "doc": locked }))["count"], 0);
    drop(dir);
}

/// A one-page PDF whose catalog carries an XMP packet declaring `pdfaid:part` 2.
fn pdfa2_pdf() -> Vec<u8> {
    let xmp = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:pdfaid="http://www.aiim.org/pdfa/ns/id/" pdfaid:part="2" pdfaid:conformance="B"/></rdf:RDF></x:xmpmeta><?xpacket end="w"?>"#;
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R /Metadata 4 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>".to_string(),
        format!("<< /Type /Metadata /Subtype /XML /Length {} >>\nstream\n{xmp}\nendstream", xmp.len()),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, b) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{b}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).as_bytes());
    out
}

#[test]
fn removal_is_one_undo_step_unknown_types_get_a_media_type_and_pdfa_is_warned_about() {
    let (dir, mut a) = setup();
    std::fs::write(dir.path().join("pdfa.pdf"), pdfa2_pdf()).unwrap();
    std::fs::write(dir.path().join("blob.zzz"), b"\x01\x02").unwrap();
    let doc = open(&mut a, "a.pdf");
    let plain = tool(&mut a, "af_add", json!({ "doc": doc, "path": "blob.zzz", "relationship": "Unspecified" }));
    assert_eq!(plain["warnings"], json!([]), "{plain}");
    let listed = tool(&mut a, "af_list", json!({ "doc": doc }));
    assert_eq!(listed["associated_files"][0]["mime"], "application/octet-stream", "{listed}");
    assert_eq!(listed["truncated"], false);

    let removed = tool(&mut a, "af_remove", json!({ "doc": doc, "index": 1 }));
    assert_eq!((removed["attachment_kept"].as_bool(), removed["shared_with_other_af"].as_bool()), (Some(false), Some(false)), "{removed}");
    assert_eq!(tool(&mut a, "af_list", json!({ "doc": doc }))["count"], 0);
    tool(&mut a, "edit_undo", json!({ "doc": doc }));
    let back = tool(&mut a, "af_list", json!({ "doc": doc }));
    assert_eq!((back["count"].as_u64(), back["associated_files"][0]["name"].as_str()), (Some(1), Some("blob.zzz")), "{back}");
    assert!(tool(&mut a, "doc_info", json!({ "doc": doc })).to_string().contains("blob.zzz"), "its attachment came back too");

    let pdfa = open(&mut a, "pdfa.pdf");
    let added = tool(&mut a, "af_add", json!({ "doc": pdfa, "path": "table.csv", "relationship": "Data" }));
    let warnings = added["warnings"].as_array().unwrap();
    assert!(warnings.len() == 1 && warnings[0].as_str().unwrap().contains("PDF/A-2"), "{added}");
    assert_eq!(added["index"], 1, "the file was added anyway");
}

/// A one-page file whose catalog `/AF` lists, in order, an embedded file with a relationship, a
/// link to an outside file with a relationship, and a link with none.
fn pdf_with_three_entries() -> Vec<u8> {
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R /AF [4 0 R 6 0 R 7 0 R] >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Type /Filespec /F (a.csv) /AFRelationship /Data /EF << /F 5 0 R >> >>",
        "<< /Type /EmbeddedFile /Subtype /text#2Fcsv /Length 1 >>\nstream\nx\nendstream",
        "<< /Type /Filespec /F (b.xml) /AFRelationship /Source >>",
        "<< /Type /Filespec /F (c.xml) >>",
    ];
    let mut out = b"%PDF-2.0\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).as_bytes());
    out
}

#[test]
fn entries_without_a_relationship_or_an_embedded_file_are_counted_through_the_tool() {
    let (dir, mut a) = setup();
    let plain = open(&mut a, "a.pdf");
    let none = tool(&mut a, "af_list", json!({ "doc": plain }));
    assert_eq!((none["without_relationship"].as_u64(), none["not_embedded"].as_u64()), (Some(0), Some(0)), "{none}");

    std::fs::write(dir.path().join("three.pdf"), pdf_with_three_entries()).unwrap();
    let doc = open(&mut a, "three.pdf");
    let listed = tool(&mut a, "af_list", json!({ "doc": doc }));
    assert_eq!(listed["count"], 3, "{listed}");
    assert_eq!((listed["without_relationship"].as_u64(), listed["not_embedded"].as_u64()), (Some(1), Some(2)), "{listed}");
    let rows = listed["associated_files"].as_array().unwrap();
    assert_eq!(
        (rows[0]["embedded"].as_bool(), rows[1]["embedded"].as_bool(), rows[2]["relationship"].clone()),
        (Some(true), Some(false), Value::Null)
    );
}

#[test]
fn a_missing_relationship_is_set_through_the_tool_in_one_undo_step() {
    let (dir, mut a) = setup();
    std::fs::write(dir.path().join("three.pdf"), pdf_with_three_entries()).unwrap();
    let doc = open(&mut a, "three.pdf");
    assert_eq!(tool(&mut a, "af_list", json!({ "doc": doc }))["without_relationship"], 1);

    // Entry 3 has none; the tool says so, sets one, and the count drops to 0.
    let done = tool(&mut a, "af_set_relationship", json!({ "doc": doc, "index": 3, "relationship": "Supplement" }));
    assert_eq!(
        (done["previous"].clone(), done["relationship"].as_str(), done["shared_with_other_af"].as_bool()),
        (Value::Null, Some("Supplement"), Some(false)),
        "{done}"
    );
    let listed = tool(&mut a, "af_list", json!({ "doc": doc }));
    assert_eq!(
        (listed["without_relationship"].as_u64(), listed["associated_files"][2]["relationship"].as_str()),
        (Some(0), Some("Supplement")),
        "{listed}"
    );
    // The other two are as they were.
    assert_eq!(listed["associated_files"][0]["relationship"], "Data");
    assert_eq!(listed["associated_files"][1]["relationship"], "Source");

    // A name that is there is replaced, and says what it was.
    let again = tool(&mut a, "af_set_relationship", json!({ "doc": doc, "index": 1, "relationship": "Alternative" }));
    assert_eq!(again["previous"], "Data", "{again}");

    // One undo step each.
    tool(&mut a, "edit_undo", json!({ "doc": doc }));
    assert_eq!(tool(&mut a, "af_list", json!({ "doc": doc }))["associated_files"][0]["relationship"], "Data");
    tool(&mut a, "edit_undo", json!({ "doc": doc }));
    assert_eq!(tool(&mut a, "af_list", json!({ "doc": doc }))["without_relationship"], 1);
    tool(&mut a, "edit_redo", json!({ "doc": doc }));
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "out.pdf" }));

    // The saved file's own object.
    let cos = Document::open(Arc::new(std::fs::read(dir.path().join("out.pdf")).unwrap())).unwrap();
    let catalog = cos.get(cos.root().unwrap()).as_dict().unwrap().clone();
    let list = cos.resolve(catalog.get(b"AF").unwrap()).as_array().cloned().unwrap();
    assert_eq!(cos.resolve(&list[2]).as_dict().unwrap().get(b"AFRelationship"), Some(&Object::name("Supplement")));
    assert_eq!(cos.resolve(&list[0]).as_dict().unwrap().get(b"AFRelationship"), Some(&Object::name("Data")));
}

#[test]
fn setting_a_relationship_refuses_with_nothing_written_and_a_modify_restriction_holds() {
    let (dir, mut a) = setup();
    std::fs::write(dir.path().join("three.pdf"), pdf_with_three_entries()).unwrap();
    let doc = open(&mut a, "three.pdf");
    let before = tool(&mut a, "af_list", json!({ "doc": doc }));
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "before.pdf" }));
    for (index, relationship) in [(3, "EncryptedPayload"), (3, "Banana"), (0, "Data"), (4, "Data"), (1, "Data")] {
        let r = a.call("af_set_relationship", &json!({ "doc": doc, "index": index, "relationship": relationship }));
        assert!(matches!(r, Err(ToolError::Failed(_) | ToolError::InvalidArgs(_))), "{index} {relationship}: {r:?}");
    }
    assert!(a.call("af_set_relationship", &json!({ "doc": doc, "index": 3 })).is_err(), "relationship is required");
    assert!(a.call("af_set_relationship", &json!({ "doc": doc, "relationship": "Data" })).is_err(), "index is required");
    assert_eq!(tool(&mut a, "af_list", json!({ "doc": doc })), before);
    assert_eq!(before["unsaved_changes"], false);
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "after.pdf" }));
    assert_eq!(std::fs::read(dir.path().join("before.pdf")).unwrap(), std::fs::read(dir.path().join("after.pdf")).unwrap());

    // A document that does not allow changes refuses.
    let plain = open(&mut a, "three.pdf");
    tool(&mut a, "doc_protect", json!({ "doc": plain, "open_password": "open", "permissions_password": "boss", "changes": "none" }));
    tool(&mut a, "doc_save", json!({ "doc": plain, "path": "locked.pdf" }));
    let locked = tool(&mut a, "doc_open", json!({ "path": "locked.pdf", "password": "open" }))["doc"].as_u64().unwrap();
    match a.call("af_set_relationship", &json!({ "doc": locked, "index": 1, "relationship": "Data" })) {
        Err(ToolError::Failed(m)) => assert!(m.contains("allow") || m.contains("permission"), "{m}"),
        other => panic!("refused: {other:?}"),
    }
    assert_eq!(tool(&mut a, "af_list", json!({ "doc": locked }))["without_relationship"], 1);
}

/// A PDF from object bodies (object numbers start at 1; the catalog is object 1).
fn raw_pdf(bodies: &[&str]) -> Vec<u8> {
    let mut out = b"%PDF-2.0\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).as_bytes());
    out
}

#[test]
fn a_shared_array_is_reported_and_a_direct_specification_moves_the_catalog_to_its_own_array() {
    let (dir, mut a) = setup();
    // The catalog and one page list array 4: a specification by reference (5), and one written in it.
    let bytes = raw_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /AF 4 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 10 10] >>",
        "<< /Type /Page /Parent 2 0 R /AF 4 0 R >>",
        "[5 0 R << /Type /Filespec /F (direct.xml) >>]",
        "<< /Type /Filespec /F (ref.xml) >>",
    ]);
    std::fs::write(dir.path().join("shared.pdf"), bytes).unwrap();
    let doc = open(&mut a, "shared.pdf");
    // Array 4 is read once, for the catalog: two entries without a relationship.
    let before = tool(&mut a, "af_list", json!({ "doc": doc }));
    assert_eq!((before["count"].as_u64(), before["without_relationship"].as_u64()), (Some(2), Some(2)), "{before}");

    // The direct one: the catalog's array is copied away from the page's, which keeps its entry
    // with none, so the page now lists it and the count goes up.
    let direct = tool(&mut a, "af_set_relationship", json!({ "doc": doc, "index": 2, "relationship": "Schema" }));
    assert_eq!((direct["array_copied"].as_bool(), direct["shared_with_other_af"].as_bool()), (Some(true), Some(false)), "{direct}");
    let after = tool(&mut a, "af_list", json!({ "doc": doc }));
    assert_eq!((after["count"].as_u64(), after["without_relationship"].as_u64()), (Some(4), Some(3)), "{after}");

    // The one by reference is shared with the page's array, and the catalog's array is already its own.
    let by_ref = tool(&mut a, "af_set_relationship", json!({ "doc": doc, "index": 1, "relationship": "Data" }));
    assert_eq!((by_ref["array_copied"].as_bool(), by_ref["shared_with_other_af"].as_bool()), (Some(false), Some(true)), "{by_ref}");
    let last = tool(&mut a, "af_list", json!({ "doc": doc }));
    assert_eq!(last["without_relationship"], 1, "{last}");
}

#[test]
fn an_entry_that_is_not_a_file_specification_is_refused_through_the_tool_with_nothing_written() {
    let (dir, mut a) = setup();
    let bytes = raw_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /AF [1 0 R 3 0 R] >>",
        "<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 10 10] >>",
        "<< /Length 1 /F (x) >>\nstream\nx\nendstream",
        "<< /Type /Page /Parent 2 0 R >>",
    ]);
    std::fs::write(dir.path().join("odd.pdf"), bytes).unwrap();
    let doc = open(&mut a, "odd.pdf");
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "before.pdf" }));
    for index in [1, 2] {
        let r = a.call("af_set_relationship", &json!({ "doc": doc, "index": index, "relationship": "Data" }));
        assert!(matches!(r, Err(ToolError::Failed(_) | ToolError::InvalidArgs(_))), "{index}: {r:?}");
    }
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "after.pdf" }));
    assert_eq!(std::fs::read(dir.path().join("before.pdf")).unwrap(), std::fs::read(dir.path().join("after.pdf")).unwrap());
}
