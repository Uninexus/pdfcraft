use std::sync::Arc;

use pdfcraft_cos::{Document, Object, SaveOptions, write_full, write_incremental};

use super::*;

#[test]
fn page_rotation_resolves_inheritance_overrides_and_missing_pages() {
    let mut doc = Document::open(Arc::new(fixture())).unwrap();
    for page in 0..3 {
        assert_eq!(page_rotation(&doc, page).unwrap(), 90);
    }
    rotate_pages(&mut doc, &[1], -180).unwrap();
    assert_eq!(page_rotation(&doc, 1).unwrap(), 270);
    assert_eq!(page_rotation(&doc, 0).unwrap(), 90);
    assert_eq!(page_rotation(&doc, 3), Err(OrganizeError::NoSuchPage(3)));
    // One walk gives each page's rotation, as the per-page call does.
    assert_eq!(crate::page_rotations(&doc).unwrap(), [90, 270, 90]);
}

#[test]
fn indirect_page_rotation_is_read_and_updated_from_its_effective_value() {
    let large = i64::MAX - i64::MAX % 90;
    for (current, degrees, expected) in [(270, 90, 0), (-90, 90, 0), (large, 90, 90), (90, -180, 270)] {
        for level in 0..3 {
            let mut doc = open(fixture());
            let rotation = doc.add(Object::Int(current));
            let page = pages(&doc).unwrap()[0].obj;
            let target = match level {
                0 => page,
                1 => doc.get(page).as_dict().unwrap().reference(b"Parent").unwrap(),
                _ => pages_root(&doc).unwrap(),
            };
            doc.update_dict(target, |d| d.set(b"Rotate".to_vec(), Object::Ref(rotation))).unwrap();
            let bytes = write_full(&doc, &SaveOptions::default()).unwrap();
            let mut doc = open(bytes);
            let rotation = walk(&doc).unwrap()[0].1.reference(b"Rotate").unwrap();
            let initial = page_rotation(&doc, 0).unwrap();
            rotate_pages(&mut doc, &[0], degrees).unwrap();
            assert_eq!(*doc.get(rotation), Object::Int(current), "shared rotation object unchanged");
            for saved in [write_incremental(&doc, &SaveOptions::default()).unwrap(), write_full(&doc, &SaveOptions::default()).unwrap()] {
                let reopened = open(saved.clone());
                let pdf = hayro_syntax::Pdf::new(saved).unwrap();
                let dimensions = if expected % 180 == 0 { (300.0, 400.0) } else { (400.0, 300.0) };
                assert_eq!(pdf.pages()[0].render_dimensions(), dimensions, "written PDF has the correct orientation");
                assert_eq!(page_rotation(&reopened, 0).unwrap(), expected);
                assert_eq!(page_rotation(&reopened, 1).unwrap(), if level == 0 { 90 } else { current.rem_euclid(360) }, "sibling unchanged");
                assert_eq!(page_rotation(&reopened, 2).unwrap(), if level == 2 { current.rem_euclid(360) } else { 90 }, "other subtree unchanged");
            }
            assert_eq!(initial, current.rem_euclid(360), "effective indirect rotation at level {level}");
        }
    }
}

#[test]
fn malformed_indirect_rotations_keep_the_zero_fallback() {
    for value in [Object::Null, Object::name("invalid"), Object::Ref(ObjRef::new(99_999, 0))] {
        let mut doc = open(fixture());
        let rotation = doc.add(value);
        let page = pages(&doc).unwrap()[0].obj;
        doc.update_dict(page, |d| d.set(b"Rotate".to_vec(), Object::Ref(rotation))).unwrap();
        assert_eq!(page_rotation(&doc, 0).unwrap(), 0);
        rotate_pages(&mut doc, &[0], 90).unwrap();
        assert_eq!(page_rotation(&doc, 0).unwrap(), 90);
    }
}

#[test]
fn rotate_rejects_angles_that_are_not_multiples_of_90() {
    // The page tree only turns in quarter steps, so anything else used to be silently truncated
    // by the integer division. It has to be an error, and the document must stay as it was.
    let mut doc = Document::open(Arc::new(fixture())).unwrap();
    assert_eq!(rotate_pages(&mut doc, &[0], 45), Err(OrganizeError::Invalid("rotation must be a multiple of 90 degrees".into())));
    assert_eq!(rotate_pages(&mut doc, &[0], 91), Err(OrganizeError::Invalid("rotation must be a multiple of 90 degrees".into())));
    assert_eq!(page_rotation(&doc, 0).unwrap(), 90);
    rotate_pages(&mut doc, &[0], -90).unwrap();
    assert_eq!(page_rotation(&doc, 0).unwrap(), 0);
}

#[test]
fn rotate_handles_large_quarter_turns() {
    let large = i64::MAX - i64::MAX % 90;
    for (current, degrees, expected) in [(90, large, 90), (-90, -large, 270), (large, 90, 90), (-large, -90, 270), (180, large - 90, 90)] {
        for inherited in [false, true] {
            let mut doc = open(fixture());
            let target = if inherited { pages_root(&doc).unwrap() } else { pages(&doc).unwrap()[0].obj };
            doc.update_dict(target, |d| d.set(b"Rotate".to_vec(), Object::Int(current))).unwrap();
            rotate_pages(&mut doc, &[0], degrees).unwrap();
            assert_eq!(page_rotation(&doc, 0).unwrap(), expected);
        }
    }
}

/// A 3-page document with a nested page tree. MediaBox and Rotate are inherited from the root,
/// Resources from an intermediate node; each page's content says which page it is.
fn fixture() -> Vec<u8> {
    let objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),                                                  // 1
        b"<< /Type /Pages /Kids [3 0 R 6 0 R] /Count 3 /MediaBox [0 0 300 400] /Rotate 90 >>".to_vec(), // 2
        b"<< /Type /Pages /Parent 2 0 R /Kids [4 0 R 5 0 R] /Count 2 /Resources << /Font << /F1 10 0 R >> >> >>".to_vec(), // 3
        b"<< /Type /Page /Parent 3 0 R /Contents 7 0 R >>".to_vec(),                                    // 4
        b"<< /Type /Page /Parent 3 0 R /Contents 8 0 R >>".to_vec(),                                    // 5
        b"<< /Type /Page /Parent 2 0 R /Contents 9 0 R /Resources << /Font << /F1 10 0 R >> >> /MediaBox [0 0 500 500] >>".to_vec(), // 6
        stream(b"BT /F1 12 Tf 20 20 Td (Page 1) Tj ET"),                                                // 7
        stream(b"BT /F1 12 Tf 20 20 Td (Page 2) Tj ET"),                                                // 8
        stream(b"BT /F1 12 Tf 20 20 Td (Page 3) Tj ET"),                                                // 9
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),                             // 10
        b"<< /Title (Original) /Producer (fixture) >>".to_vec(),                                        // 11
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R /Info 11 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn stream(body: &[u8]) -> Vec<u8> {
    let mut v = format!("<< /Length {} >>\nstream\n", body.len()).into_bytes();
    v.extend_from_slice(body);
    v.extend_from_slice(b"\nendstream");
    v
}

fn open(bytes: Vec<u8>) -> Document {
    Document::open(Arc::new(bytes)).expect("opens")
}

/// The text label of each page, via its content stream, in order.
fn labels(doc: &Document) -> Vec<String> {
    pages(doc)
        .unwrap()
        .iter()
        .map(|p| {
            let page = doc.get(p.obj);
            let c = page.as_dict().and_then(|d| d.get(b"Contents").cloned());
            match c.map(|c| doc.resolve(&c)).as_deref() {
                Some(Object::Stream(s)) => {
                    let data = s.decoded().unwrap();
                    let t = String::from_utf8_lossy(&data);
                    t.split('(').nth(1).and_then(|x| x.split(')').next()).unwrap_or("").to_string()
                }
                _ => "blank".into(),
            }
        })
        .collect()
}

/// Save incrementally, verify the prefix is untouched, and reopen — both with our reader and with
/// an independent parser (hayro-syntax) that must agree on the page count.
fn save_and_reopen(doc: &Document) -> Document {
    let original = doc.bytes().clone();
    let bytes = write_incremental(doc, &SaveOptions::default()).expect("saves");
    assert_eq!(&bytes[..original.len()], &original[..], "incremental save must not modify existing bytes");
    let reopened = open(bytes.clone());
    let independent = hayro_syntax::Pdf::new(bytes).expect("hayro opens our output");
    assert_eq!(independent.pages().len(), page_count(&reopened).unwrap(), "readers agree on page count");
    reopened
}

#[test]
fn walks_nested_tree_in_order() {
    let doc = open(fixture());
    assert_eq!(labels(&doc), ["Page 1", "Page 2", "Page 3"]);
}

#[test]
fn rotate_uses_inherited_rotation_and_round_trips() {
    let mut doc = open(fixture());
    rotate_pages(&mut doc, &[0, 2], 90).unwrap();
    rotate_pages(&mut doc, &[2], -270).unwrap(); // page 3: inherited 90 + 90 - 270 = -90 ≡ 270
    let doc = save_and_reopen(&doc);
    let rot = |i: usize| {
        let p = pages(&doc).unwrap()[i].obj;
        doc.get(p).as_dict().and_then(|d| d.int(b"Rotate"))
    };
    assert_eq!(rot(0), Some(180), "inherited 90 + 90");
    assert_eq!(rot(1), None, "untouched page keeps inheriting");
    assert_eq!(rot(2), Some(270), "normalised into 0..360");
}

#[test]
fn delete_keeps_inherited_attributes_on_survivors() {
    let mut doc = open(fixture());
    delete_pages(&mut doc, &[0]).unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(labels(&doc), ["Page 2", "Page 3"]);
    // Page 2 inherited Resources (from the intermediate node) and MediaBox/Rotate (from the root);
    // the tree was flattened, so they must now be on the page itself.
    let p = doc.get(pages(&doc).unwrap()[0].obj);
    let d = p.as_dict().unwrap();
    assert!(d.contains(b"Resources") && d.contains(b"MediaBox"));
    assert_eq!(d.int(b"Rotate"), Some(90));
    let root = doc.get(doc.get(doc.root().unwrap()).as_dict().unwrap().reference(b"Pages").unwrap());
    assert_eq!(root.as_dict().unwrap().int(b"Count"), Some(2));
}

#[test]
fn deleting_a_page_prunes_deep_named_destination_trees() {
    let mut doc = doc_a();
    let original_pages = pages(&doc).unwrap();
    let mut leaf = Dict::new();
    leaf.set(
        b"Names".to_vec(),
        Object::Array(vec![
            Object::String(PdfString::text("gone")),
            Object::Array(vec![Object::Ref(original_pages[0].obj), Object::name("Fit")]),
            Object::String(PdfString::text("kept")),
            Object::Array(vec![Object::Ref(original_pages[2].obj), Object::name("Fit")]),
        ]),
    );
    let leaf = doc.add(leaf);
    let mut tree = leaf;
    for _ in 0..16_384 {
        let mut node = Dict::new();
        node.set(b"Kids".to_vec(), Object::Array(vec![Object::Ref(tree)]));
        tree = doc.add(node);
    }
    let mut names = Dict::new();
    names.set(b"Dests".to_vec(), Object::Ref(tree));
    doc.update_dict(doc.root().unwrap(), |d| d.set(b"Names".to_vec(), Object::Dict(names))).unwrap();

    // Every node is a shallow indirect object: the page operation must not turn the
    // document's graph depth into call-stack depth.
    delete_pages(&mut doc, &[0]).unwrap();
    for bytes in [write_incremental(&doc, &SaveOptions::default()).unwrap(), write_full(&doc, &SaveOptions::default()).unwrap()] {
        let reopened = open(bytes);
        assert_eq!(labels(&reopened), ["A2", "A3"]);
        let catalog = reopened.get(reopened.root().unwrap());
        let names = reopened.resolve(catalog.as_dict().unwrap().get(b"Names").unwrap());
        let mut node = reopened.resolve(names.as_dict().unwrap().get(b"Dests").unwrap());
        for _ in 0..16_384 {
            let child = node.as_dict().unwrap().get(b"Kids").unwrap().as_array().unwrap()[0].clone();
            node = reopened.resolve(&child);
        }
        let pairs = node.as_dict().unwrap().get(b"Names").and_then(Object::as_array).cloned().unwrap();
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].as_string().unwrap().to_text(), "kept");
        assert_eq!(pairs[1].as_array().unwrap()[0].as_ref(), Some(pages(&reopened).unwrap()[1].obj));
    }
}

#[test]
fn destination_pruning_visits_shared_and_cyclic_nodes_once() {
    for inline_names in [false, true] {
        for inline_tree in [false, true] {
            let mut doc = doc_a();
            let gone = pages(&doc).unwrap()[0].obj;
            let mut leaf = Dict::new();
            leaf.set(
                b"Names".to_vec(),
                Object::Array(vec![Object::String(PdfString::text("gone")), Object::Array(vec![Object::Ref(gone), Object::name("Fit")])]),
            );
            leaf.set(b"Unknown".to_vec(), Object::Int(42));
            let leaf = doc.add(leaf);
            let cycle = doc.add(Object::Null);
            let mut node = Dict::new();
            node.set(b"Kids".to_vec(), Object::Array(vec![Object::Ref(cycle), Object::Ref(leaf), Object::Ref(leaf)]));
            doc.set(cycle, Object::Dict(node.clone()));
            let mut names = Dict::new();
            names.set(b"Dests".to_vec(), if inline_tree { Object::Dict(node) } else { Object::Ref(cycle) });
            let names = if inline_names { Object::Dict(names) } else { Object::Ref(doc.add(names)) };
            doc.update_dict(doc.root().unwrap(), |d| d.set(b"Names".to_vec(), names)).unwrap();

            assert_eq!(drop_named_destinations(&mut doc, &[gone]).unwrap(), [b"gone".to_vec()]);
            let leaf = doc.get(leaf);
            assert_eq!(leaf.as_dict().unwrap().get(b"Names").and_then(Object::as_array).unwrap().len(), 0);
            assert_eq!(leaf.as_dict().unwrap().int(b"Unknown"), Some(42));
        }
    }
}

#[test]
fn deleting_a_page_keeps_sixty_thousand_named_destinations_usable() {
    let mut doc = doc_a();
    let original_pages = pages(&doc).unwrap();
    let pairs: Vec<Object> = (0..60_000)
        .flat_map(|i| {
            let page = if i % 2 == 0 { original_pages[0].obj } else { original_pages[2].obj };
            [Object::String(PdfString::text(&format!("dest-{i:05}"))), Object::Array(vec![Object::Ref(page), Object::name("Fit")])]
        })
        .collect();
    let mut leaf = Dict::new();
    leaf.set(b"Names".to_vec(), Object::Array(pairs));
    leaf.set(b"Unknown".to_vec(), Object::Int(42));
    let leaf = doc.add(leaf);
    let mut names = Dict::new();
    names.set(b"Dests".to_vec(), Object::Ref(leaf));
    doc.update_dict(doc.root().unwrap(), |d| d.set(b"Names".to_vec(), Object::Dict(names))).unwrap();

    // A valid large book exceeds the former 100,000-entry budget (two entries per name).
    delete_pages(&mut doc, &[0]).unwrap();
    for bytes in [write_incremental(&doc, &SaveOptions::default()).unwrap(), write_full(&doc, &SaveOptions::default()).unwrap()] {
        let reopened = open(bytes);
        assert_eq!(labels(&reopened), ["A2", "A3"]);
        let catalog = reopened.get(reopened.root().unwrap());
        let names = reopened.resolve(catalog.as_dict().unwrap().get(b"Names").unwrap());
        let leaf = reopened.resolve(names.as_dict().unwrap().get(b"Dests").unwrap());
        let leaf = leaf.as_dict().unwrap();
        assert_eq!(leaf.int(b"Unknown"), Some(42));
        let pairs = leaf.get(b"Names").unwrap().as_array().unwrap();
        assert_eq!(pairs.len(), 60_000, "30,000 surviving name/value pairs");
        let page = pages(&reopened).unwrap()[1].obj;
        for (pair, i) in pairs.as_chunks::<2>().0.iter().zip((1..60_000).step_by(2)) {
            assert_eq!(pair[0].as_string().unwrap().to_text(), format!("dest-{i:05}"));
            assert_eq!(pair[1].as_array().unwrap()[0].as_ref(), Some(page));
        }
    }
}

#[test]
fn deleted_pages_are_not_kept_by_what_points_at_them() {
    // A1 links to A3, a bookmark goes to A3, and A2 holds the form's only field. After deleting
    // A2 and A3 neither page, its content nor the field may stay in the saved file.
    let mut doc = doc_a();
    add_bookmark(&mut doc, &[], 0, "To A3", 2).unwrap();
    delete_pages(&mut doc, &[1, 2]).unwrap();
    let out = full_roundtrip(&doc);
    assert_eq!(labels(&out), ["A1"]);
    // The bookmark and A1's link now go nowhere instead of to a missing page.
    let mark = out.get(bookmarks(&out)[0].obj).as_dict().cloned().unwrap();
    assert!(!mark.contains(b"Dest") && !mark.contains(b"A"), "{mark:?}");
    let link = &annots(&out, 0)[0];
    assert!(!link.contains(b"A") && !link.contains(b"Dest"), "{link:?}");
    let mut page_objects = 0;
    for n in out.object_numbers() {
        let o = out.get(ObjRef::new(n, out.generation(n)));
        if let Object::Stream(s) = &*o {
            let data = String::from_utf8_lossy(&s.decoded().unwrap()).into_owned();
            assert!(!data.contains("(A2)") && !data.contains("(A3)"), "object {n} still holds a deleted page's content");
        }
        page_objects += usize::from(o.as_dict().is_some_and(|d| d.name(b"Type") == Some(b"Page")));
    }
    assert_eq!(page_objects, 1);
    let cat = out.get(out.root().unwrap()).as_dict().cloned().unwrap();
    let form = out.resolve(cat.get(b"AcroForm").unwrap());
    assert_eq!(form.as_dict().and_then(|f| f.get(b"Fields")).and_then(Object::as_array).map(Vec::len), Some(0), "the field went with its page");
}

#[test]
fn deleting_the_opening_page_drops_the_open_action() {
    let open_action = |doc: &Document| doc.get(doc.root().unwrap()).as_dict().unwrap().get(b"OpenAction").cloned();
    let set_open = |doc: &mut Document, value: Object| {
        let root = doc.root().unwrap();
        doc.update_dict(root, |c| c.set(b"OpenAction".to_vec(), value)).unwrap();
    };
    let page = |doc: &Document, i: usize| pages(doc).unwrap()[i].obj;
    for as_action in [false, true] {
        let target = |doc: &Document, i: usize| {
            let dest = Object::Array(vec![Object::Ref(page(doc, i)), Object::Name(b"Fit".to_vec())]);
            if as_action {
                let mut a = Dict::new();
                a.set(b"S".to_vec(), Object::Name(b"GoTo".to_vec()));
                a.set(b"D".to_vec(), dest);
                Object::Dict(a)
            } else {
                dest
            }
        };
        // The opening page goes: no OpenAction is left to point at nothing.
        let mut doc = doc_a();
        let open = target(&doc, 0);
        set_open(&mut doc, open);
        delete_pages(&mut doc, &[0]).unwrap();
        let out = full_roundtrip(&doc);
        assert_eq!(labels(&out), ["A2", "A3"]);
        assert_eq!(open_action(&out), None);
        // Another page goes: the opening page keeps its OpenAction.
        let mut doc = doc_a();
        let open = target(&doc, 0);
        set_open(&mut doc, open);
        delete_pages(&mut doc, &[2]).unwrap();
        let out = full_roundtrip(&doc);
        assert_eq!(labels(&out), ["A1", "A2"]);
        assert!(open_action(&out).is_some());
    }
}

#[test]
fn cannot_delete_every_page_or_missing_pages() {
    let mut doc = open(fixture());
    assert_eq!(delete_pages(&mut doc, &[0, 1, 2]), Err(OrganizeError::WouldRemoveAllPages));
    assert_eq!(delete_pages(&mut doc, &[7]), Err(OrganizeError::NoSuchPage(7)));
    assert_eq!(rotate_pages(&mut doc, &[3], 90), Err(OrganizeError::NoSuchPage(3)));
    assert_eq!(OrganizeError::NoSuchPage(3).to_string(), "page 4 does not exist", "the message counts pages from 1 (#1550)");
    assert!(!doc.is_modified(), "failed operations leave the document untouched");
}

#[test]
fn move_pages_reorders() {
    let mut doc = open(fixture());
    move_pages(&mut doc, &[2], 0).unwrap();
    assert_eq!(labels(&doc), ["Page 3", "Page 1", "Page 2"]);
    move_pages(&mut doc, &[0, 1], 3).unwrap();
    assert_eq!(labels(&doc), ["Page 2", "Page 3", "Page 1"]);
    let doc = save_and_reopen(&doc);
    assert_eq!(labels(&doc), ["Page 2", "Page 3", "Page 1"]);
}

#[test]
fn reverse_pages_reorders() {
    let mut doc = open(fixture());
    reverse_pages(&mut doc, &[0, 1, 2]).unwrap();
    assert_eq!(labels(&doc), ["Page 3", "Page 2", "Page 1"]);
    // Unsorted, repeated indices name the same slots; page 1's slot stays put.
    reverse_pages(&mut doc, &[2, 1, 2]).unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(labels(&doc), ["Page 3", "Page 1", "Page 2"]);
}

#[test]
fn cannot_reverse_missing_or_fewer_than_two_pages() {
    let mut doc = open(fixture());
    assert_eq!(reverse_pages(&mut doc, &[0, 3]), Err(OrganizeError::NoSuchPage(3)));
    assert!(matches!(reverse_pages(&mut doc, &[1, 1]), Err(OrganizeError::Invalid(_))));
    assert!(!doc.is_modified(), "failed operations leave the document untouched");
}

#[test]
fn inserts_blank_page() {
    let mut doc = open(fixture());
    insert_blank_page(&mut doc, 1, 612.0, 792.0).unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(labels(&doc), ["Page 1", "blank", "Page 2", "Page 3"]);
    let hay = hayro_syntax::Pdf::new(doc.bytes().clone()).unwrap();
    let (w, h) = hay.pages().get(1).unwrap().render_dimensions();
    assert_eq!((w, h), (612.0, 792.0));
}

#[test]
fn document_info_edits_round_trip_with_unicode() {
    let mut doc = open(fixture());
    set_info(&mut doc, "Title", "Résumé — 履歴書").unwrap();
    set_info(&mut doc, "Author", "PdfCraft").unwrap();
    set_info(&mut doc, "Producer", "").unwrap(); // clearing removes the key
    let doc = save_and_reopen(&doc);
    assert_eq!(info(&doc, "Title").as_deref(), Some("Résumé — 履歴書"));
    assert_eq!(info(&doc, "Author").as_deref(), Some("PdfCraft"));
    assert_eq!(info(&doc, "Producer"), None);
}

/// Page transitions (§12.4.4): written as `/Trans` and `/Dur` with only the entries the style
/// reads and that differ from the defaults, removed again, checked before any page changes, and
/// kept through a save.
#[test]
fn page_transitions_are_set_removed_and_checked() {
    let mut doc = open(fixture());
    let wipe = Transition { duration: 2.5, direction: Some(270), ..Transition::new("Wipe") };
    set_transition(&mut doc, &[0, 2], &TransitionSpec { transition: Some(wipe), display: Some(5.0) }).unwrap();
    let d = page_dict(&doc, 0);
    let t = d.get(b"Trans").and_then(|o| o.as_dict()).expect("/Trans").clone();
    assert_eq!(t.name(b"Type"), Some(&b"Trans"[..]));
    assert_eq!(t.name(b"S"), Some(&b"Wipe"[..]));
    assert_eq!(t.get(b"D"), Some(&Object::Real(2.5)));
    assert_eq!(t.get(b"Di"), Some(&Object::Int(270)));
    assert!(
        t.get(b"Dm").is_none() && t.get(b"M").is_none() && t.get(b"SS").is_none() && t.get(b"B").is_none(),
        "entries Wipe does not read stay out: {t:?}"
    );
    assert_eq!(d.get(b"Dur"), Some(&Object::Real(5.0)));
    assert!(page_dict(&doc, 1).get(b"Trans").is_none(), "page 2 was not asked for");
    // Fly with everything non-default; the defaults themselves are not written.
    let fly = Transition { style: "Fly".into(), duration: 1.0, horizontal: false, inward: false, direction: None, scale: 0.5, opaque: true };
    set_transition(&mut doc, &[1], &TransitionSpec { transition: Some(fly), display: None }).unwrap();
    let t = page_dict(&doc, 1).get(b"Trans").and_then(|o| o.as_dict()).expect("/Trans").clone();
    assert_eq!((t.name(b"S"), t.name(b"M"), t.name(b"Di")), (Some(&b"Fly"[..]), Some(&b"O"[..]), Some(&b"None"[..])));
    assert_eq!((t.get(b"SS"), t.get(b"B")), (Some(&Object::Real(0.5)), Some(&Object::Bool(true))));
    assert!(t.get(b"D").is_none() && t.get(b"Dm").is_none(), "{t:?}");
    // Removed again.
    set_transition(&mut doc, &[0], &TransitionSpec { transition: None, display: None }).unwrap();
    let d = page_dict(&doc, 0);
    assert!(d.get(b"Trans").is_none() && d.get(b"Dur").is_none());
    // Checked before anything changes.
    for (what, spec) in [
        ("style", TransitionSpec { transition: Some(Transition::new("Swirl")), display: None }),
        ("direction", TransitionSpec { transition: Some(Transition { direction: Some(45), ..Transition::new("Wipe") }), display: None }),
        ("duration", TransitionSpec { transition: Some(Transition { duration: -1.0, ..Transition::new("Fade") }), display: None }),
        ("scale", TransitionSpec { transition: Some(Transition { scale: 0.0, ..Transition::new("Fly") }), display: None }),
        ("display", TransitionSpec { transition: None, display: Some(f32::NAN) }),
    ] {
        assert!(set_transition(&mut doc, &[2], &spec).is_err(), "{what} must be refused");
    }
    assert!(set_transition(&mut doc, &[7], &TransitionSpec { transition: None, display: None }).is_err(), "no such page");
    let t = page_dict(&doc, 2).get(b"Trans").and_then(|o| o.as_dict()).expect("page 3 unchanged").clone();
    assert_eq!(t.name(b"S"), Some(&b"Wipe"[..]));
    // Kept through a save.
    let doc = save_and_reopen(&doc);
    // (A whole number of seconds may come back as an integer: the writer's choice.)
    assert_eq!(page_dict(&doc, 2).get(b"Dur").and_then(Object::as_f64), Some(5.0));
}

/// Custom document properties (Document Properties ▸ Custom): any `/Info` key the spec does not
/// name, set and cleared like the standard ones, listed apart from them, and with the names
/// Acrobat refuses refused here too.
#[test]
fn custom_document_properties_are_listed_set_removed_and_their_names_checked() {
    let mut doc = open(fixture());
    assert!(custom_info(&doc).is_empty());
    set_info(&mut doc, "Title", "Report").unwrap();
    set_info(&mut doc, "Project", "Apollo").unwrap();
    set_info(&mut doc, "Revision", "7").unwrap();
    let mut listed = custom_info(&doc);
    listed.sort();
    assert_eq!(listed, [("Project".to_string(), "Apollo".to_string()), ("Revision".to_string(), "7".to_string())], "standard keys are not custom");
    assert_eq!(info(&doc, "Project").as_deref(), Some("Apollo"));
    set_info(&mut doc, "Project", "").unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(custom_info(&doc), [("Revision".to_string(), "7".to_string())], "cleared, and the rest survives a save");
    assert_eq!(info(&doc, "Title").as_deref(), Some("Report"));

    for bad in ["", "Title", "ModDate", "Bad Name", "a/b", "tab\tbed", "hash#tag", "(x)"] {
        assert!(check_custom_key(bad).is_err(), "{bad:?} must be refused");
    }
    let long = "x".repeat(MAX_CUSTOM_KEY_LEN + 1);
    assert!(check_custom_key(&long).is_err());
    for good in ["Project", "Révision", "部門", "key-2.0", &"y".repeat(MAX_CUSTOM_KEY_LEN)] {
        assert!(check_custom_key(good).is_ok(), "{good:?} is a usable name");
    }
    let mut doc = open(fixture());
    assert!(set_info(&mut doc, "Bad Name", "x").is_err(), "set_info checks custom names");
    assert!(custom_info(&doc).is_empty());
}

/// An `/Info` dictionary written directly in the trailer: a cleared entry is removed, and the
/// other entries are kept.
#[test]
fn document_info_entries_are_cleared_from_an_info_dictionary_in_the_trailer() {
    let b = build(
        &[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R >>".into(),
            body("X"),
        ],
        "/Root 1 0 R /Info << /Title (Report) /Project (Apollo) /Revision (7) >>",
    );
    let mut doc = open(b);
    set_info(&mut doc, "Missing", "").unwrap();
    assert!(matches!(doc.trailer().get(b"Info"), Some(Object::Dict(_))), "clearing an absent entry changes nothing");
    set_info(&mut doc, "Project", "").unwrap();
    set_info(&mut doc, "Title", "").unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(custom_info(&doc), [("Revision".to_string(), "7".to_string())]);
    assert_eq!(info(&doc, "Title"), None);
}

/// Padding is not part of a custom value, in any mix of NULs and whitespace, a value that is only
/// padding is not listed, and such entries don't count towards the 256 listed: a real property
/// after 300 blank ones is still there.
#[test]
fn custom_document_properties_drop_padding_before_the_cap() {
    let mut doc = open(fixture());
    set_info(&mut doc, "Title", "Report").unwrap();
    let Some(Object::Ref(info_ref)) = doc.trailer().get(b"Info").cloned() else { panic!("an /Info reference") };
    let text = |s: &str| Object::String(pdfcraft_cos::PdfString { bytes: s.as_bytes().to_vec(), hex: false });
    doc.update_dict(info_ref, |d| {
        d.set(b"Mixed".to_vec(), text(" \0Lab\0 "));
        d.set(b"Pad".to_vec(), text(" \0 \0"));
        for i in 0..300 {
            d.set(format!("Blank{i}").into_bytes(), text("  "));
        }
        d.set(b"Keep".to_vec(), text("yes"));
    })
    .unwrap();
    assert_eq!(custom_info(&doc), [("Mixed".to_string(), "Lab".to_string()), ("Keep".to_string(), "yes".to_string())]);
}

/// A one-page document whose catalog has `xmp` as its (Flate-compressed) XMP packet, and the
/// same title and author in `/Info`, as LibreOffice writes them.
fn doc_with_xmp(xmp: &[u8]) -> Document {
    let b = build(
        &[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R >>".into(),
            body("X"),
            "<< /Title (Untitled 1) /Author (Old Author) >>".into(),
        ],
        "/Root 1 0 R /Info 5 0 R",
    );
    let mut doc = open(b);
    let mut dict = Dict::new();
    dict.set(b"Type".to_vec(), Object::name("Metadata"));
    dict.set(b"Subtype".to_vec(), Object::name("XML"));
    let m = doc.add(Object::Stream(pdfcraft_cos::Stream::flate(dict, xmp)));
    let root = doc.root().unwrap();
    doc.update_dict(root, |c| c.set(b"Metadata".to_vec(), Object::Ref(m))).unwrap();
    full_roundtrip(&doc)
}

/// The catalog's metadata stream.
fn xmp_stream(doc: &Document) -> Option<pdfcraft_cos::Stream> {
    match &*doc.get(catalog(doc).reference(b"Metadata")?) {
        Object::Stream(s) => Some(s.clone()),
        _ => None,
    }
}

fn xmp_text(doc: &Document) -> Option<String> {
    xmp_stream(doc).map(|s| String::from_utf8(s.decoded().unwrap()).unwrap())
}

/// The values the XMP packet gives a property: the items of its rdf:Alt or rdf:Seq, else its text.
fn xmp_values(xmp: &str, name: &str) -> Vec<String> {
    let Some(start) = xmp.find(&format!("<{name}>")) else { return Vec::new() };
    let end = xmp[start..].find(&format!("</{name}>")).unwrap() + start;
    let inner = &xmp[start + name.len() + 2..end];
    if !inner.contains("<rdf:li") {
        return vec![inner.to_string()];
    }
    inner.split("<rdf:li").skip(1).map(|li| li[li.find('>').unwrap() + 1..li.find("</rdf:li>").unwrap()].to_string()).collect()
}

const LIBREOFFICE_XMP: &str = "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
  <rdf:Description rdf:about=\"\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\"><pdf:Producer>Writer 1.0</pdf:Producer></rdf:Description>\n\
  <rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n\
   <dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">Untitled 1</rdf:li></rdf:Alt></dc:title>\n\
   <dc:creator><rdf:Seq><rdf:li>Old Author</rdf:li></rdf:Seq></dc:creator>\n\
   <dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">Old subject</rdf:li></rdf:Alt></dc:description>\n\
  </rdf:Description>\n </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>";

#[test]
fn document_info_edits_update_the_xmp_packet() {
    // Viewers that read XMP first (pdf.js takes dc:title) showed the old title after a change.
    let mut doc = doc_with_xmp(LIBREOFFICE_XMP.as_bytes());
    assert_eq!(xmp_values(&xmp_text(&doc).unwrap(), "dc:title"), ["Untitled 1"]);
    set_info(&mut doc, "Title", "Sales Guide v1.0 & <draft>").unwrap();
    set_info(&mut doc, "Author", "Demo Consulting").unwrap();
    set_info(&mut doc, "Subject", "").unwrap();
    set_info(&mut doc, "Keywords", "alpha, beta").unwrap();
    set_info(&mut doc, "Rating", "5").unwrap(); // a custom property: pdfx:Rating
    let doc = save_and_reopen(&doc);
    assert_eq!(info(&doc, "Title").as_deref(), Some("Sales Guide v1.0 & <draft>"));
    let xmp = xmp_text(&doc).unwrap();
    assert_eq!(xmp_values(&xmp, "dc:title"), ["Sales Guide v1.0 &amp; &lt;draft&gt;"]);
    assert_eq!(xmp_values(&xmp, "dc:creator"), ["Demo Consulting"]);
    assert!(xmp_values(&xmp, "dc:description").is_empty(), "{xmp}");
    assert_eq!(xmp_values(&xmp, "pdf:Keywords"), ["alpha, beta"]);
    assert_eq!(xmp_values(&xmp, "pdfx:Rating"), ["5"]);
    assert_eq!(xmp_values(&xmp, "pdf:Producer"), ["Writer 1.0"]);
    assert!(xmp.starts_with("<?xpacket begin=") && xmp.ends_with("<?xpacket end=\"w\"?>"), "{xmp}");
    // Stored unfiltered, still typed as metadata.
    let dict = xmp_stream(&doc).unwrap().dict;
    assert_eq!((dict.name(b"Type"), dict.name(b"Subtype"), dict.get(b"Filter")), (Some(&b"Metadata"[..]), Some(&b"XML"[..]), None));
}

/// A file that states its title, author and so on only in XMP (PDF 2.0 deprecates `/Info`) shows
/// them in Document Properties (#2373); `/Info` wins where it has a value.
#[test]
fn document_info_falls_back_to_the_xmp_packet() {
    let xmp = LIBREOFFICE_XMP
        .replace(
            "<rdf:li xml:lang=\"x-default\">Untitled 1</rdf:li>",
            "<rdf:li xml:lang=\"de\">Bericht</rdf:li><rdf:li xml:lang=\"x-default\">Report &amp; &#x201C;notes&#8221;</rdf:li>",
        )
        .replace("<rdf:li>Old Author</rdf:li>", "<rdf:li>Ada</rdf:li>\n<rdf:li>Grace</rdf:li>")
        .replace("<pdf:Producer>Writer 1.0</pdf:Producer>", "<pdf:Producer>Writer 1.0</pdf:Producer><pdf:Keywords/>");
    let mut doc = doc_with_xmp(xmp.as_bytes());
    // Both: /Info's own values, XMP for what /Info lacks.
    assert_eq!(info(&doc, "Title").as_deref(), Some("Untitled 1"));
    assert_eq!(info(&doc, "Subject").as_deref(), Some("Old subject"));
    // XMP only.
    doc.trailer_mut().remove(b"Info");
    let doc = full_roundtrip(&doc);
    assert_eq!(info(&doc, "Title").as_deref(), Some("Report & \u{201C}notes\u{201D}"));
    assert_eq!(info(&doc, "Author").as_deref(), Some("Ada; Grace"));
    assert_eq!(info(&doc, "Subject").as_deref(), Some("Old subject"));
    assert_eq!(info(&doc, "Producer").as_deref(), Some("Writer 1.0"));
    assert_eq!(info(&doc, "Keywords"), None, "an empty property is no value");
    assert_eq!(info(&doc, "Rating"), None, "no XMP counterpart");
    // The attribute form, and a packet that isn't UTF-8 is not read.
    let attrs = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
                 <rdf:Description rdf:about=\"\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\" pdf:Keywords='a, b &amp; c'/></rdf:RDF></x:xmpmeta>";
    let mut doc = doc_with_xmp(attrs.as_bytes());
    doc.trailer_mut().remove(b"Info");
    assert_eq!(info(&doc, "Keywords").as_deref(), Some("a, b & c"));
    let mut latin1 = LIBREOFFICE_XMP.as_bytes().to_vec();
    latin1.extend_from_slice(b"<!-- \xE9 -->");
    let mut doc = doc_with_xmp(&latin1);
    doc.trailer_mut().remove(b"Info");
    assert_eq!(info(&doc, "Subject"), None);
}

/// The title read from XMP each frame (Initial View ▸ Show: Document Title) decodes the packet
/// once, not on every read; an edit of the packet, or another document, reads afresh.
#[test]
fn the_xmp_packet_is_decoded_once_for_repeated_reads() {
    let decodes = || crate::xmp::XMP_DECODES.with(std::cell::Cell::get);
    let mut doc = doc_with_xmp(LIBREOFFICE_XMP.as_bytes());
    doc.trailer_mut().remove(b"Info");
    let before = decodes();
    for _ in 0..10 {
        assert_eq!(info(&doc, "Title").as_deref(), Some("Untitled 1"));
        assert_eq!(info(&doc, "Author").as_deref(), Some("Old Author"));
    }
    assert_eq!(decodes() - before, 1);
    set_info(&mut doc, "Title", "Report").unwrap();
    doc.trailer_mut().remove(b"Info");
    assert_eq!(info(&doc, "Title").as_deref(), Some("Report"), "the edited packet, not the one read before");
    assert_eq!(decodes() - before, 2);
    let mut other = doc_with_xmp(LIBREOFFICE_XMP.replace("Untitled 1", "Other").as_bytes());
    other.trailer_mut().remove(b"Info");
    assert_eq!(info(&other, "Title").as_deref(), Some("Other"), "another document's packet");
    assert_eq!(info(&doc, "Title").as_deref(), Some("Report"));
    let mut latin1 = LIBREOFFICE_XMP.as_bytes().to_vec();
    latin1.extend_from_slice(&[b'<', 0xE9, b'>']);
    let mut bad = doc_with_xmp(&latin1);
    bad.trailer_mut().remove(b"Info");
    let before = decodes();
    for _ in 0..10 {
        assert_eq!(info(&bad, "Title"), None);
    }
    assert_eq!(decodes() - before, 1, "an unreadable packet is not decoded again either");
}

#[test]
fn document_info_edits_leave_files_without_usable_xmp_alone() {
    // No packet: none is added, for a custom property either.
    let mut doc = open(fixture());
    set_info(&mut doc, "Title", "New").unwrap();
    set_info(&mut doc, "Project", "Apollo").unwrap();
    assert_eq!(xmp_text(&save_and_reopen(&doc)), None);
    // A packet that isn't UTF-8 is not rewritten.
    let mut latin1 = LIBREOFFICE_XMP.as_bytes().to_vec();
    latin1.extend_from_slice(b"<!-- \xE9 -->");
    let mut doc = doc_with_xmp(&latin1);
    set_info(&mut doc, "Title", "New").unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(info(&doc, "Title").as_deref(), Some("New"));
    assert_eq!(xmp_stream(&doc).unwrap().decoded().unwrap(), latin1);
}

/// Acrobat keeps each custom property in the XMP packet too, as `pdfx:<name>` (the
/// http://ns.adobe.com/pdfx/1.3/ schema), so tools that read XMP see the same properties as
/// Document Properties ▸ Custom.
#[test]
fn custom_document_properties_are_kept_in_the_xmp_packet_as_pdfx() {
    // An Acrobat-style packet already holding one, in a description of its own.
    let acrobat = LIBREOFFICE_XMP.replace(
        " </rdf:RDF>",
        "  <rdf:Description rdf:about=\"\" xmlns:pdfx=\"http://ns.adobe.com/pdfx/1.3/\"><pdfx:Dept>Sales</pdfx:Dept></rdf:Description>\n </rdf:RDF>",
    );
    let mut doc = doc_with_xmp(acrobat.as_bytes());
    set_info(&mut doc, "Project", "Apollo & <X>").unwrap();
    set_info(&mut doc, "Größe", "A4").unwrap();
    set_info(&mut doc, "Dept", "R&D").unwrap();
    let mut doc = save_and_reopen(&doc);
    assert_eq!((info(&doc, "Project").as_deref(), info(&doc, "Dept").as_deref()), (Some("Apollo & <X>"), Some("R&D")));
    let xmp = xmp_text(&doc).unwrap();
    assert_eq!(xmp_values(&xmp, "pdfx:Project"), ["Apollo &amp; &lt;X&gt;"], "{xmp}");
    assert_eq!(xmp_values(&xmp, "pdfx:Größe"), ["A4"], "{xmp}");
    assert!(xmp.contains("<rdf:Description rdf:about=\"\" xmlns:pdfx=\"http://ns.adobe.com/pdfx/1.3/\"><pdfx:Project>"), "{xmp}");
    // One already there changes where it stands; the rest of the packet is as it was.
    assert!(xmp.contains("xmlns:pdfx=\"http://ns.adobe.com/pdfx/1.3/\"><pdfx:Dept>R&amp;D</pdfx:Dept></rdf:Description>"), "{xmp}");
    assert_eq!(xmp_values(&xmp, "dc:title"), ["Untitled 1"]);
    assert_eq!(xmp_values(&xmp, "pdf:Producer"), ["Writer 1.0"]);
    // Changed again, then removed with the property.
    set_info(&mut doc, "Project", "Gemini").unwrap();
    let xmp = xmp_text(&doc).unwrap();
    assert_eq!((xmp.matches("<pdfx:Project>").count(), xmp_values(&xmp, "pdfx:Project")), (1, vec!["Gemini".to_string()]), "{xmp}");
    for key in ["Project", "Größe", "Dept"] {
        set_info(&mut doc, key, "").unwrap();
    }
    let xmp = xmp_text(&save_and_reopen(&doc)).unwrap();
    assert!(!xmp.contains("<pdfx:"), "{xmp}");

    // A name that can't be an XML element name stays in /Info only, and the packet is untouched.
    let mut doc = doc_with_xmp(LIBREOFFICE_XMP.as_bytes());
    for key in ["1st", "Q&A", "a:b"] {
        set_info(&mut doc, key, "v").unwrap();
        assert_eq!(info(&doc, key).as_deref(), Some("v"));
    }
    // So do the standard entries XMP doesn't mirror here.
    set_info(&mut doc, "Trapped", "False").unwrap();
    assert_eq!(xmp_text(&save_and_reopen(&doc)).unwrap(), LIBREOFFICE_XMP);
}

/// PDF/A-1 to -3 allow an XMP schema only when the packet describes it (ISO 19005-2 §6.6.2.3), so
/// a PDF/A file gets no new `pdfx:` property, while one it already has still follows `/Info`. A
/// packet that binds `pdfx` to another namespace keeps that namespace's properties.
#[test]
fn custom_properties_add_nothing_to_pdfa_packets_or_another_schemas_pdfx() {
    let pdfa = LIBREOFFICE_XMP.replace(
        " </rdf:RDF>",
        "  <rdf:Description rdf:about=\"\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" pdfaid:part=\"2\" pdfaid:conformance=\"B\"/>\n </rdf:RDF>",
    );
    let mut doc = doc_with_xmp(pdfa.as_bytes());
    set_info(&mut doc, "Project", "Apollo").unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(info(&doc, "Project").as_deref(), Some("Apollo"));
    assert_eq!(xmp_text(&doc).unwrap(), pdfa, "no undescribed schema in a PDF/A packet");
    // The declaration counts however its value is written (here CDATA, which isn't read as text).
    let cdata = LIBREOFFICE_XMP.replace(
        " </rdf:RDF>",
        "  <rdf:Description rdf:about=\"\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\"><pdfaid:part><![CDATA[2]]></pdfaid:part></rdf:Description>\n </rdf:RDF>",
    );
    let mut doc = doc_with_xmp(cdata.as_bytes());
    set_info(&mut doc, "Project", "Apollo").unwrap();
    assert_eq!(xmp_text(&doc).unwrap(), cdata);
    // One the packet already has (described, as an Acrobat PDF/A file would) is changed and removed.
    let described = pdfa.replace(
        " </rdf:RDF>",
        "  <rdf:Description rdf:about=\"\" xmlns:pdfx=\"http://ns.adobe.com/pdfx/1.3/\"><pdfx:Dept>Sales</pdfx:Dept></rdf:Description>\n </rdf:RDF>",
    );
    let mut doc = doc_with_xmp(described.as_bytes());
    set_info(&mut doc, "Dept", "R&D").unwrap();
    assert_eq!(xmp_text(&doc).unwrap(), described.replace(">Sales<", ">R&amp;D<"));
    set_info(&mut doc, "Dept", "").unwrap();
    assert_eq!(xmp_text(&doc).unwrap(), described.replace("<pdfx:Dept>Sales</pdfx:Dept>", ""));

    // `pdfx` bound to the PDF/X identification namespace: neither changed nor added to.
    let other = LIBREOFFICE_XMP.replace(
        " </rdf:RDF>",
        "  <rdf:Description rdf:about=\"\" xmlns:pdfx = 'http://www.npes.org/pdfx/ns/id/'><pdfx:Dept>keep</pdfx:Dept></rdf:Description>\n </rdf:RDF>",
    );
    let mut doc = doc_with_xmp(other.as_bytes());
    set_info(&mut doc, "Dept", "R&D").unwrap();
    set_info(&mut doc, "Project", "Apollo").unwrap();
    assert_eq!((info(&doc, "Dept").as_deref(), info(&doc, "Project").as_deref()), (Some("R&D"), Some("Apollo")));
    assert_eq!(xmp_text(&save_and_reopen(&doc)).unwrap(), other);
}

#[test]
fn successive_incremental_saves_stack_revisions() {
    let mut doc = open(fixture());
    rotate_pages(&mut doc, &[0], 90).unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(doc.revisions().len(), 2);
    let mut doc = doc;
    delete_pages(&mut doc, &[1]).unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(doc.revisions().len(), 3);
    assert_eq!(labels(&doc), ["Page 1", "Page 3"]);
}

#[test]
fn full_save_drops_unreachable_objects() {
    let mut doc = open(fixture());
    delete_pages(&mut doc, &[0, 1]).unwrap();
    let bytes = write_full(&doc, &SaveOptions::default()).unwrap();
    let full = open(bytes.clone());
    assert_eq!(labels(&full), ["Page 3"]);
    // Deleted pages, their content streams and the orphaned intermediate node are gone.
    assert!(full.object_numbers().len() < doc.object_numbers().len());
    assert_eq!(info(&full, "Title").as_deref(), Some("Original"));
    assert_eq!(hayro_syntax::Pdf::new(bytes).unwrap().pages().len(), 1);
}

#[test]
fn deleting_a_page_drops_the_named_destinations_to_it() {
    // Four names (two in the /Names tree, two in the legacy /Dests dictionary) go to P1 or P3;
    // P2 has a link for each. Deleting P1 must drop its names and their links, keep P3's.
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Names << /Dests 6 0 R >> /Dests 7 0 R >>".into(), // 1
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 300 400] >>".into(), // 2
        "<< /Type /Page /Parent 2 0 R >>".into(),                                          // 3
        "<< /Type /Page /Parent 2 0 R /Annots [8 0 R 9 0 R 10 0 R 11 0 R] >>".into(),      // 4
        "<< /Type /Page /Parent 2 0 R >>".into(),                                          // 5
        "<< /Names [(treeFirst) [3 0 R /Fit] (treeLast) [5 0 R /Fit]] >>".into(),          // 6
        "<< /legacyFirst [3 0 R /Fit] /legacyLast << /D [5 0 R /Fit] >> >>".into(),        // 7
        "<< /Type /Annot /Subtype /Link /Rect [0 0 9 9] /Dest (treeFirst) >>".into(),      // 8
        "<< /Type /Annot /Subtype /Link /Rect [0 0 9 9] /Dest (treeLast) >>".into(),       // 9
        "<< /Type /Annot /Subtype /Link /Rect [0 0 9 9] /Dest /legacyFirst >>".into(),     // 10
        "<< /Type /Annot /Subtype /Link /Rect [0 0 9 9] /A << /S /GoTo /D /legacyLast >> >>".into(), // 11
    ];
    let mut doc = open(build(&b, "/Root 1 0 R"));
    delete_pages(&mut doc, &[0]).unwrap();
    let out = full_roundtrip(&doc);
    let cat = out.get(out.root().unwrap()).as_dict().cloned().unwrap();
    let tree = out.resolve(out.resolve(cat.get(b"Names").unwrap()).as_dict().unwrap().get(b"Dests").unwrap());
    let pairs = tree.as_dict().unwrap().get(b"Names").and_then(Object::as_array).cloned().unwrap();
    assert_eq!(pairs.len(), 2, "{pairs:?}");
    assert_eq!(pairs[0].as_string().map(|s| s.bytes.clone()), Some(b"treeLast".to_vec()));
    let page3 = pages(&out).unwrap()[1].obj;
    assert_eq!(out.resolve(&pairs[1]).as_array().and_then(|a| a.first()).and_then(Object::as_ref), Some(page3));
    let legacy = out.resolve(cat.get(b"Dests").unwrap()).as_dict().cloned().unwrap();
    assert!(!legacy.contains(b"legacyFirst") && legacy.contains(b"legacyLast"), "{legacy:?}");
    let links = annots(&out, 0);
    assert!(!links[0].contains(b"Dest"), "{:?}", links[0]);
    assert!(links[1].contains(b"Dest"), "{:?}", links[1]);
    assert!(!links[2].contains(b"Dest"), "{:?}", links[2]);
    assert!(links[3].contains(b"A"), "{:?}", links[3]);
}

// ── Combine / extract / split ──────────────────────────────────────────────────────────────────

/// Build a classic-xref PDF from object bodies (object i+1 = bodies[i]).
fn build(bodies: &[String], trailer: &str) -> Vec<u8> {
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
    out.extend_from_slice(format!("trailer\n<< /Size {} {trailer} >>\nstartxref\n{xref}\n%%EOF\n", bodies.len() + 1).as_bytes());
    out
}

fn body(text: &str) -> String {
    let s = format!("BT /F1 12 Tf 20 20 Td ({text}) Tj ET");
    format!("<< /Length {} >>\nstream\n{s}\nendstream", s.len())
}

/// Document A: pages A1–A3 sharing one font. A1 links to A3 (GoTo action), A2 links to A1
/// (/Dest) and carries a text field widget; A3 has a note with a popup (a reference cycle).
fn doc_a() -> Document {
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [12 0 R] >> >>".into(), // 1
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 300 400] >>".into(), // 2
        "<< /Type /Page /Parent 2 0 R /Contents 6 0 R /Resources << /Font << /F1 9 0 R >> >> /Annots [10 0 R] >>".into(), // 3
        "<< /Type /Page /Parent 2 0 R /Contents 7 0 R /Resources << /Font << /F1 9 0 R >> >> /Annots [11 0 R 12 0 R] >>".into(), // 4
        "<< /Type /Page /Parent 2 0 R /Contents 8 0 R /Resources << /Font << /F1 9 0 R >> >> /Annots [13 0 R] /StructParents 4 >>".into(), // 5
        body("A1"),                                                                  // 6
        body("A2"),                                                                  // 7
        body("A3"),                                                                  // 8
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),             // 9
        "<< /Type /Annot /Subtype /Link /Rect [0 0 50 50] /P 3 0 R /A << /S /GoTo /D [5 0 R /Fit] >> >>".into(), // 10
        "<< /Type /Annot /Subtype /Link /Rect [0 0 50 50] /P 4 0 R /Dest [3 0 R /XYZ 0 400 0] >>".into(), // 11
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /V (Ada) /Rect [60 60 200 80] /P 4 0 R >>".into(), // 12
        "<< /Type /Annot /Subtype /Text /Rect [10 10 30 30] /Contents (note) /Popup 14 0 R /P 5 0 R /StructParent 7 >>".into(), // 13
        "<< /Type /Annot /Subtype /Popup /Rect [40 40 200 120] /Parent 13 0 R >>".into(), // 14
        "<< /Title (Doc A) /Author (Alice) >>".into(),                               // 15
    ];
    open(build(&b, "/Root 1 0 R /Info 15 0 R"))
}

/// Document B: pages B1, B2 in a nested tree with MediaBox and Rotate inherited from the root.
fn doc_b() -> Document {
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 2 /MediaBox [0 0 500 250] /Rotate 90 >>".into(),
        "<< /Type /Pages /Parent 2 0 R /Kids [4 0 R 5 0 R] /Count 2 >>".into(),
        "<< /Type /Page /Parent 3 0 R /Contents 6 0 R /Resources << /Font << /F1 8 0 R >> >> >>".into(),
        "<< /Type /Page /Parent 3 0 R /Contents 7 0 R /Resources << /Font << /F1 8 0 R >> >> >>".into(),
        body("B1"),
        body("B2"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman >>".into(),
    ];
    open(build(&b, "/Root 1 0 R"))
}

/// Write in full, reopen with our reader, and require hayro to agree on the page count.
fn full_roundtrip(doc: &Document) -> Document {
    let bytes = write_full(doc, &SaveOptions::default()).expect("writes");
    let reopened = open(bytes.clone());
    let hay = hayro_syntax::Pdf::new(bytes).expect("hayro opens");
    assert_eq!(hay.pages().len(), page_count(&reopened).unwrap());
    reopened
}

fn page_dict(doc: &Document, i: usize) -> Dict {
    doc.get(pages(doc).unwrap()[i].obj).as_dict().cloned().unwrap()
}

fn annots(doc: &Document, i: usize) -> Vec<Dict> {
    let p = page_dict(doc, i);
    let list = p.get(b"Annots").map(|a| doc.resolve(a).as_array().cloned().unwrap_or_default()).unwrap_or_default();
    list.iter().map(|a| doc.resolve(a).as_dict().cloned().unwrap()).collect()
}

fn count_fonts(doc: &Document) -> usize {
    doc.object_numbers().iter().filter(|n| doc.get(ObjRef::new(**n, 0)).as_dict().is_some_and(|d| d.name(b"Type") == Some(b"Font"))).count()
}

#[test]
fn combine_concatenates_with_a_bookmark_per_file() {
    let out = full_roundtrip(&combine(&[("Report A", &doc_a()), ("Report B", &doc_b())]).unwrap());
    assert_eq!(labels(&out), ["A1", "A2", "A3", "B1", "B2"]);
    // Shared resources are copied once per source: one Helvetica, one Times.
    assert_eq!(count_fonts(&out), 2);
    // Bookmarks: one per source file, pointing at its first page.
    let cat = out.get(out.root().unwrap()).as_dict().cloned().unwrap();
    let outlines = out.get(cat.reference(b"Outlines").unwrap()).as_dict().cloned().unwrap();
    let first = out.get(outlines.reference(b"First").unwrap()).as_dict().cloned().unwrap();
    assert_eq!(first.get(b"Title").and_then(|t| t.as_string()).map(|s| s.to_text()).as_deref(), Some("Report A"));
    let second = out.get(first.reference(b"Next").unwrap()).as_dict().cloned().unwrap();
    assert_eq!(second.get(b"Title").and_then(|t| t.as_string()).map(|s| s.to_text()).as_deref(), Some("Report B"));
    let dest_page = second.get(b"Dest").and_then(|d| d.as_array()).and_then(|a| a[0].as_ref()).unwrap();
    assert_eq!(dest_page, pages(&out).unwrap()[3].obj);
}

#[test]
fn combiner_adds_each_source_then_releases_it() {
    let bytes = Arc::new(fixture());
    let src = Document::open(bytes.clone()).unwrap();
    let mut combiner = crate::Combiner::new();
    combiner.add("A", &src, None).unwrap();
    drop(src);
    // No output intents to copy, so the combine keeps nothing of the source's file.
    assert_eq!(Arc::strong_count(&bytes), 1);
    let picked: &[usize] = &[1];
    combiner.add("B", &open(fixture()), Some(picked)).unwrap();
    let streamed = combiner.finish().unwrap();
    let whole = combine_selected(&[("A", &open(fixture()), None), ("B", &open(fixture()), Some(picked))]).unwrap();
    assert_eq!(write_full(&streamed, &SaveOptions::default()).unwrap(), write_full(&whole, &SaveOptions::default()).unwrap());
}

#[test]
fn combining_hostile_outlines_ends_their_runs_too() {
    // The bookmark cases of `hostile_outlines_end_their_runs_and_never_loop`, combined: each run ends
    // where it would repeat, and the source's bookmarks sit under its own entry.
    let cases: [(&[&str], &str); 3] = [
        (&["<< /Title (Self) /Parent 3 0 R /Next 5 0 R >>"], "H[Self]"),
        (&["<< /Title (A) /Parent 3 0 R /Next 6 0 R >>", "<< /Title (B) /Parent 3 0 R /Next 5 0 R >>"], "H[A,B]"),
        (&["<< /Title (A) /Parent 3 0 R /Next 99 0 R >>"], "H[A]"),
    ];
    for (items, listed) in cases {
        let out = full_roundtrip(&combine(&[("H", &outline_with(items))]).unwrap());
        assert_eq!(titles(&crate::bookmarks(&out)), [listed]);
    }
    // A bookmark whose first child is itself: nesting is cut off, so the combine still ends.
    let looped = outline_with(&["<< /Title (Self) /Parent 3 0 R /First 5 0 R >>"]);
    let out = full_roundtrip(&combine(&[("H", &looped)]).unwrap());
    assert_eq!(crate::bookmarks(&out).first().map(|b| b.title.as_str()), Some("H"));
}

#[test]
fn inherited_attributes_travel_with_copied_pages() {
    let out = full_roundtrip(&extract_pages(&doc_b(), &[1]).unwrap());
    let p = page_dict(&out, 0);
    assert_eq!(p.int(b"Rotate"), Some(90));
    assert!(p.contains(b"MediaBox") && p.contains(b"Resources"));
    let hay = hayro_syntax::Pdf::new(write_full(&out, &SaveOptions::default()).unwrap()).unwrap();
    assert_eq!(hay.pages().first().unwrap().render_dimensions(), (250.0, 500.0), "rotated 90°");
}

#[test]
fn resources_split_between_page_and_tree_travel_together() {
    // The page names its own font but draws a form that only the page-tree node lists. Readers
    // (ours included) look a name up the whole chain, so the page shows; a copy that kept only
    // the page's own dictionary drew nothing (#1628).
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] /Resources << /XObject << /X0 5 0 R >> /Font << /F1 6 0 R /F2 6 0 R >> >> >>"
            .into(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F2 7 0 R >> >> >>".into(),
        {
            let s = "q 100 0 0 100 50 50 cm /X0 Do Q BT /F2 12 Tf (x) Tj ET";
            format!("<< /Length {} >>\nstream\n{s}\nendstream", s.len())
        },
        {
            let s = "0 0 1 rg 0 0 1 1 re f";
            format!("<< /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Length {} >>\nstream\n{s}\nendstream", s.len())
        },
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".into(),
    ];
    let src = open(build(&b, "/Root 1 0 R"));
    let out = full_roundtrip(&combine(&[("S", &src), ("S again", &src)]).unwrap());
    for i in 0..2 {
        let res = out.dict(page_dict(&out, i).get(b"Resources").unwrap()).unwrap();
        let entry = |cat: &[u8], name: &[u8]| res.get(cat).and_then(|c| out.dict(c)).and_then(|d| d.get(name).map(|v| out.resolve(v)));
        let form = entry(b"XObject", b"X0").expect("the tree's form comes along");
        assert_eq!(form.as_dict().and_then(|d| d.name(b"Subtype")), Some(&b"Form"[..]));
        let font = |name: &[u8]| entry(b"Font", name).and_then(|f| f.as_dict().and_then(|d| d.name(b"BaseFont").map(<[u8]>::to_vec)));
        assert_eq!(font(b"F1").as_deref(), Some(&b"Helvetica"[..]), "a name only the tree has is added");
        assert_eq!(font(b"F2").as_deref(), Some(&b"Courier"[..]), "the page's own entry wins over the tree's");
    }
}

#[test]
fn a_scan_whose_xref_misses_the_image_combines_with_the_image() {
    // A cheap image-to-PDF converter's table: "0 5" followed by the entries of objects 1 to 5,
    // so the image (object 5) has none. Viewers rebuild the table and show the scan; the
    // combined copy got a null image and came out blank (#1628).
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>".into(),
        "<< /Length 26 >>\nstream\n100 0 0 100 0 0 cm /Im0 Do\nendstream".into(),
        "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 1 >>\nstream\nA\nendstream".into(),
    ];
    let good = build(&b, "/Root 1 0 R");
    let text = String::from_utf8_lossy(&good).into_owned();
    let at = text.rfind("\nxref\n").unwrap() + 1;
    let entries: Vec<&str> = text[at..].lines().skip(3).take(5).collect();
    let mut bytes = good[..at].to_vec();
    bytes.extend_from_slice(format!("xref\n0 5\n{}\ntrailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{at}\n%%EOF\n", entries.join("\n")).as_bytes());
    let src = open(bytes);
    let out = full_roundtrip(&combine(&[("scan", &src)]).unwrap());
    let res = out.dict(page_dict(&out, 0).get(b"Resources").unwrap()).unwrap();
    let image = res.get(b"XObject").and_then(|x| out.dict(x)).and_then(|d| d.get(b"Im0").map(|v| out.resolve(v))).expect("the page lists its image");
    let Object::Stream(image) = &*image else { panic!("the image is a stream, not {image:?}") };
    assert_eq!(image.dict.name(b"Subtype"), Some(&b"Image"[..]));
    assert_eq!(image.decoded().unwrap(), b"A");
}

#[test]
fn links_are_rewired_to_copied_pages_or_dropped() {
    let a = doc_a();
    // Extract A3, A1 (reordered): A1's GoTo A3 must now point at the copy of A3 (index 0).
    let out = full_roundtrip(&extract_pages(&a, &[2, 0]).unwrap());
    assert_eq!(labels(&out), ["A3", "A1"]);
    let link = &annots(&out, 1)[0];
    let action = out.resolve(link.get(b"A").unwrap());
    let target = action.as_dict().unwrap().get(b"D").and_then(|d| d.as_array()).and_then(|a| a[0].as_ref()).unwrap();
    assert_eq!(target, pages(&out).unwrap()[0].obj);
    assert_eq!(link.reference(b"P"), Some(pages(&out).unwrap()[1].obj), "/P points at the new page");
    // Extract only A2: its link to A1 would dangle, so the destination is dropped.
    let out = full_roundtrip(&extract_pages(&a, &[1]).unwrap());
    let link = annots(&out, 0).into_iter().find(|d| d.name(b"Subtype") == Some(b"Link")).unwrap();
    assert!(!link.contains(b"Dest") && !link.contains(b"A"));
}

/// Issue #1810: a widget that is its own field was shared between both copies of a repeated
/// page, so only the last page could identify it (its `/P`), and per-page annotation edits were
/// impossible. Each copy must get its own widget sharing one parent field (name and value).
#[test]
fn repeated_page_splits_a_combined_field_widget() {
    let out = full_roundtrip(&extract_pages(&doc_a(), &[1, 1]).unwrap());
    let ps = pages(&out).unwrap();
    let widget = |i: usize| -> ObjRef {
        let list = page_dict(&out, i).get(b"Annots").map(|a| out.resolve(a).as_array().cloned().unwrap_or_default()).unwrap_or_default();
        list.iter()
            .filter_map(|a| a.as_ref())
            .find(|r| out.get(*r).as_dict().is_some_and(|d| d.name(b"Subtype") == Some(b"Widget")))
            .expect("a widget annotation")
    };
    let (w1, w2) = (widget(0), widget(1));
    assert_ne!(w1, w2, "each copy of the page has its own widget");
    for (i, w) in [(0, w1), (1, w2)] {
        let d = out.get(w).as_dict().unwrap().clone();
        for key in [b"T".as_slice(), b"V", b"FT"] {
            assert!(!d.contains(key), "a widget inherits field entries: {key:?}");
        }
        assert_eq!(d.reference(b"P"), Some(ps[i].obj), "/P names the page holding the widget");
        let parent = d.reference(b"Parent").expect("the widget has a parent field");
        let f = out.get(parent).as_dict().unwrap().clone();
        assert_eq!(f.get(b"T").and_then(|v| v.as_string()).map(|s| s.to_text()).as_deref(), Some("name"));
        assert_eq!(f.get(b"V").and_then(|v| v.as_string()).map(|s| s.to_text()).as_deref(), Some("Ada"));
        let kids: Vec<ObjRef> =
            f.get(b"Kids").and_then(|k| out.resolve(k).as_array().cloned()).unwrap_or_default().iter().filter_map(|k| k.as_ref()).collect();
        assert!(kids.contains(&w1) && kids.contains(&w2), "one field holds both widgets: {kids:?}");
        let fields: Vec<ObjRef> = catalog(&out)
            .get(b"AcroForm")
            .map(|f| out.resolve(f))
            .and_then(|f| f.as_dict().cloned())
            .and_then(|f| f.get(b"Fields").map(|x| out.resolve(x)))
            .and_then(|f| f.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|f| f.as_ref())
            .collect();
        assert_eq!(fields, [parent], "the parent is the one top-level field");
    }
}

#[test]
fn splitting_a_combined_widget_keeps_annotation_actions_and_field_triggers() {
    let mut src = doc_a();
    let page = pages(&src).unwrap()[1].obj;
    let list = page_dict(&src, 1).get(b"Annots").and_then(|a| a.as_array()).cloned().unwrap();
    let widget =
        list.iter().filter_map(Object::as_ref).find(|r| src.get(*r).as_dict().is_some_and(|d| d.name(b"Subtype") == Some(b"Widget"))).unwrap();
    let mut action = Dict::new();
    action.set(b"S".to_vec(), Object::name("GoTo"));
    action.set(b"D".to_vec(), Object::Array(vec![Object::Ref(page), Object::name("Fit")]));
    let mut named = Dict::new();
    named.set(b"S".to_vec(), Object::name("Named"));
    named.set(b"N".to_vec(), Object::name("NextPage"));
    let mut aa = Dict::new();
    aa.set(b"E".to_vec(), Object::Dict(named.clone()));
    aa.set(b"K".to_vec(), Object::Dict(named));
    let aa = src.add(Object::Dict(aa));
    src.update_dict(widget, |d| {
        d.set(b"A".to_vec(), Object::Dict(action));
        d.set(b"AA".to_vec(), Object::Ref(aa));
    })
    .unwrap();
    let out = full_roundtrip(&extract_pages(&src, &[1, 1]).unwrap());
    let ps = pages(&out).unwrap();
    for i in 0..2 {
        let w = annots(&out, i).into_iter().find(|d| d.name(b"Subtype") == Some(b"Widget")).unwrap();
        let action = out.resolve(w.get(b"A").expect("the widget keeps its action"));
        let dest = action.as_dict().unwrap().get(b"D").and_then(|a| a.as_array()).unwrap()[0].as_ref().unwrap();
        assert!(ps.iter().any(|p| p.obj == dest), "the action targets a copied page");
        let waa = out.resolve(w.get(b"AA").expect("widget triggers"));
        assert!(waa.as_dict().unwrap().contains(b"E"));
        assert!(!waa.as_dict().unwrap().contains(b"K"));
        let parent = out.get(w.reference(b"Parent").unwrap()).as_dict().unwrap().clone();
        assert!(!parent.contains(b"A"));
        let faa = out.resolve(parent.get(b"AA").expect("field triggers"));
        assert!(faa.as_dict().unwrap().contains(b"K"));
        assert!(!faa.as_dict().unwrap().contains(b"E"));
    }
}

#[test]
fn splitting_a_combined_widget_keeps_its_annotation_only_keys() {
    let mut src = doc_a();
    let list = page_dict(&src, 1).get(b"Annots").and_then(|a| a.as_array()).cloned().unwrap();
    let widget =
        list.iter().filter_map(Object::as_ref).find(|r| src.get(*r).as_dict().is_some_and(|d| d.name(b"Subtype") == Some(b"Widget"))).unwrap();
    src.update_dict(widget, |d| {
        d.set(b"Contents".to_vec(), Object::String(PdfString::text("alt text")));
        d.set(b"BM".to_vec(), Object::name("Multiply"));
        d.set(b"Lang".to_vec(), Object::String(PdfString::text("en-US")));
        d.set(b"AF".to_vec(), Object::Array(Vec::new()));
    })
    .unwrap();
    let out = full_roundtrip(&extract_pages(&src, &[1, 1]).unwrap());
    for i in 0..2 {
        let w = annots(&out, i).into_iter().find(|d| d.name(b"Subtype") == Some(b"Widget")).unwrap();
        let parent = out.get(w.reference(b"Parent").unwrap()).as_dict().unwrap().clone();
        for key in [b"Contents".as_slice(), b"BM", b"Lang", b"AF"] {
            assert!(w.contains(key), "the widget keeps {key:?}");
            assert!(!parent.contains(key), "the parent field does not take {key:?}");
        }
    }
}

#[test]
fn repeated_page_gets_independent_annotations() {
    // A1 has a link, A2 a link and a form field, A3 a note with a popup; each is listed twice.
    let out = full_roundtrip(&extract_pages(&doc_a(), &[0, 0, 1, 1, 2, 2]).unwrap());
    let ps = pages(&out).unwrap();
    let refs: Vec<Vec<ObjRef>> = (0..6)
        .map(|i| page_dict(&out, i).get(b"Annots").map(|a| out.resolve(a).as_array().cloned().unwrap_or_default()).unwrap_or_default())
        .map(|list| list.iter().filter_map(|a| a.as_ref()).collect())
        .collect();
    for i in (0..6).step_by(2) {
        assert!(!refs[i].is_empty());
        assert_eq!(refs[i].len(), refs[i + 1].len());
        for (a, b) in refs[i].iter().zip(&refs[i + 1]) {
            assert_ne!(a, b, "each copy of the page has its own annotation objects");
        }
    }
    for (i, list) in refs.iter().enumerate() {
        for a in list.iter() {
            assert_eq!(out.get(*a).as_dict().unwrap().reference(b"P"), Some(ps[i].obj), "/P names the page holding the copy");
        }
    }
    // Each note keeps its own popup, pointing back at that note.
    let popups: Vec<ObjRef> = [4, 5].iter().map(|&i| out.get(refs[i][0]).as_dict().unwrap().reference(b"Popup").unwrap()).collect();
    assert_ne!(popups[0], popups[1]);
    for (k, p) in popups.iter().enumerate() {
        assert_eq!(out.get(*p).as_dict().unwrap().reference(b"Parent"), Some(refs[4 + k][0]));
    }
}

#[test]
fn form_fields_come_along_and_are_registered() {
    let out = full_roundtrip(&extract_pages(&doc_a(), &[1]).unwrap());
    let cat = out.get(out.root().unwrap()).as_dict().cloned().unwrap();
    let form = out.resolve(cat.get(b"AcroForm").unwrap());
    let fields = out.resolve(form.as_dict().unwrap().get(b"Fields").unwrap()).as_array().cloned().unwrap();
    assert_eq!(fields.len(), 1);
    let field = out.resolve(&fields[0]).as_dict().cloned().unwrap();
    assert_eq!(field.get(b"V").and_then(|v| v.as_string()).map(|s| s.to_text()).as_deref(), Some("Ada"));
    // The widget child, not the field, names the page (#1810).
    let kid =
        field.get(b"Kids").map(|k| out.resolve(k)).and_then(|k| k.as_array().cloned()).and_then(|k| k.first().and_then(|k| k.as_ref())).unwrap();
    assert_eq!(out.get(kid).as_dict().unwrap().reference(b"P"), Some(pages(&out).unwrap()[0].obj));
}

/// #4020: Extract keeps the bookmarks that go to the pages it copies, mapped to the copies. One whose
/// page was left out goes, unless a bookmark under it stays.
#[test]
fn extract_keeps_the_bookmarks_of_the_pages_it_copies() {
    let mut src = doc_a();
    crate::add_bookmark(&mut src, &[], 0, "One", 0).unwrap();
    crate::add_bookmark(&mut src, &[], 1, "Two", 1).unwrap();
    crate::add_bookmark(&mut src, &[1], 0, "Two.a", 2).unwrap();
    crate::add_bookmark(&mut src, &[], 2, "Three", 2).unwrap();
    let marks = |d: &Document| -> Vec<(String, usize, Option<usize>)> { bookmarks(d).into_iter().map(|b| (b.title, b.path.len(), b.page)).collect() };
    let s = |t: &str| t.to_string();

    let all = full_roundtrip(&extract_pages(&src, &[0, 1, 2]).unwrap());
    assert_eq!(marks(&all), [(s("One"), 1, Some(0)), (s("Two"), 1, Some(1)), (s("Two.a"), 2, Some(2)), (s("Three"), 1, Some(2))]);

    let last = full_roundtrip(&extract_pages(&src, &[2]).unwrap());
    assert_eq!(marks(&last), [(s("Two"), 1, None), (s("Two.a"), 2, Some(0)), (s("Three"), 1, Some(0))], "One's page was left out");

    let none = full_roundtrip(&extract_pages(&doc_a(), &[0]).unwrap());
    assert!(!none.get(none.root().unwrap()).as_dict().unwrap().contains(b"Outlines"), "no bookmarks, no outline");
}

#[test]
fn popup_cycles_copy_once_and_structure_links_are_removed() {
    let out = full_roundtrip(&extract_pages(&doc_a(), &[2]).unwrap());
    let p = page_dict(&out, 0);
    assert!(!p.contains(b"StructParents"));
    let note = &annots(&out, 0)[0];
    assert!(!note.contains(b"StructParent"));
    let popup = out.get(note.reference(b"Popup").unwrap()).as_dict().cloned().unwrap();
    assert_eq!(
        out.get(popup.reference(b"Parent").unwrap()).as_dict().unwrap().get(b"Contents").and_then(|c| c.as_string()).map(|s| s.to_text()).as_deref(),
        Some("note")
    );
    // Nothing unrelated was dragged in: no pages A1/A2, no other content streams.
    assert_eq!(labels(&out), ["A3"]);
}

#[test]
fn inserting_pages_from_another_file_is_an_incremental_edit() {
    let mut a = doc_a();
    import_pages(&mut a, &doc_b(), &[1, 0], 1).unwrap();
    let a = save_and_reopen(&a); // checks the prefix invariant and hayro agreement
    assert_eq!(labels(&a), ["A1", "B2", "B1", "A2", "A3"]);
    assert_eq!(info(&a, "Title").as_deref(), Some("Doc A"));
    assert_eq!(import_pages(&mut a.clone(), &doc_b(), &[5], 0), Err(OrganizeError::NoSuchPage(5)));
}

#[test]
fn split_ranges_cover_every_page_once() {
    assert_eq!(split_ranges(5, &SplitBy::PageCount(2)), [0..2, 2..4, 4..5]);
    assert_eq!(split_ranges(5, &SplitBy::PageCount(0)), [0..1, 1..2, 2..3, 3..4, 4..5]);
    assert_eq!(split_ranges(5, &SplitBy::Before(vec![3, 1, 1, 9, 0])), [0..1, 1..3, 3..5]);
    assert_eq!(split_ranges(3, &SplitBy::PageCount(10)), vec![0..3]);
}

#[test]
fn split_produces_standalone_documents_with_metadata() {
    let parts = split(&doc_a(), &SplitBy::PageCount(1)).unwrap();
    assert_eq!(parts.len(), 3);
    for (i, part) in parts.iter().enumerate() {
        let part = full_roundtrip(part);
        assert_eq!(labels(&part), [format!("A{}", i + 1)]);
        assert_eq!(info(&part, "Author").as_deref(), Some("Alice"));
    }
}

// ── PDF/X (#263) ───────────────────────────────────────────────────────────────────────────────

/// PDF/X-4 identification in XMP, as an attribute, next to a PDF/UA claim.
const X4_XMP: &str = "<?xpacket begin=\"\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?><x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF \
                      xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" \
                      xmlns:pdfxid=\"http://www.npes.org/pdfx/ns/id/\" xmlns:pdfuaid=\"http://www.aiim.org/pdfua/ns/id/\" \
                      pdfxid:GTS_PDFXVersion=\"PDF/X-4\" pdfuaid:part=\"1\"/></rdf:RDF></x:xmpmeta><?xpacket end=\"r\"?>";

/// A two-page print file whose output intent prints to `condition` with `profile` as its ICC
/// profile, with `info` as its document information and `xmp` as its XMP metadata.
fn doc_print(condition: &str, profile: &str, info: &str, xmp: Option<&str>) -> Document {
    let metadata = if xmp.is_some() { " /Metadata 10 0 R" } else { "" };
    let mut b: Vec<String> = vec![
        format!("<< /Type /Catalog /Pages 2 0 R /OutputIntents [7 0 R]{metadata} >>"), // 1
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 252 144] >>".into(), // 2
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /TrimBox [9 9 243 135] >>".into(), // 3
        "<< /Type /Page /Parent 2 0 R /Contents 6 0 R /TrimBox [9 9 243 135] >>".into(), // 4
        body("X1"),                                                                    // 5
        body("X2"),                                                                    // 6
        format!("<< /Type /OutputIntent /S /GTS_PDFX /OutputConditionIdentifier ({condition}) /DestOutputProfile 8 0 R >>"), // 7
        format!("<< /N 4 /Length {} >>\nstream\n{profile}\nendstream", profile.len()), // 8
        format!("<< {info} >>"),                                                       // 9
    ];
    if let Some(x) = xmp {
        b.push(format!("<< /Type /Metadata /Subtype /XML /Length {} >>\nstream\n{x}\nendstream", x.len())); // 10
    }
    open(build(&b, "/Root 1 0 R /Info 9 0 R"))
}

/// A PDF/X-4 business card printing to `condition` with `profile`.
fn doc_x4(condition: &str, profile: &str) -> Document {
    doc_print(condition, profile, "/Title (Card) /GTS_PDFXVersion (PDF/X-4) /Trapped /False", Some(X4_XMP))
}

/// What a document says about PDF/X.
#[derive(Debug, Default, PartialEq)]
struct Pdfx {
    /// Each output intent's printing condition and decoded profile.
    intents: Vec<(String, Vec<u8>)>,
    version: Option<String>,
    trapped: Option<Vec<u8>>,
    xmp: Option<String>,
}

fn pdfx_of(doc: &Document) -> Pdfx {
    let cat = catalog(doc);
    let list = cat.get(b"OutputIntents").map(|o| doc.resolve(o).as_array().cloned().unwrap_or_default()).unwrap_or_default();
    let intents = list
        .iter()
        .filter_map(|oi| {
            let d = doc.resolve(oi).as_dict().cloned()?;
            let id = d.get(b"OutputConditionIdentifier").and_then(|v| doc.resolve(v).as_string().map(|s| s.to_text())).unwrap_or_default();
            let profile = match d.get(b"DestOutputProfile").map(|p| doc.resolve(p)).as_deref() {
                Some(Object::Stream(s)) => s.decoded().unwrap(),
                _ => Vec::new(),
            };
            Some((id, profile))
        })
        .collect();
    let trapped = doc.trailer().get(b"Info").map(|i| doc.resolve(i)).and_then(|i| i.as_dict().and_then(|d| d.name(b"Trapped")).map(<[u8]>::to_vec));
    let xmp = match cat.get(b"Metadata").map(|m| doc.resolve(m)).as_deref() {
        Some(Object::Stream(s)) => Some(String::from_utf8(s.decoded().unwrap()).unwrap()),
        _ => None,
    };
    Pdfx { intents, version: info(doc, "GTS_PDFXVersion"), trapped, xmp }
}

#[test]
fn extract_and_split_keep_the_pdfx_output_intent_and_identification() {
    // #263: the parts had no /OutputIntents, no GTS_PDFXVersion and no XMP pdfxid, so a PDF/X
    // file stopped being one.
    let src = doc_x4("FOGRA39", "icc-fogra39");
    let mut outs = vec![extract_pages(&src, &[1]).unwrap()];
    outs.extend(split(&src, &SplitBy::PageCount(1)).unwrap());
    for out in &outs {
        let x = pdfx_of(&full_roundtrip(out));
        assert_eq!(x.intents, [("FOGRA39".to_string(), b"icc-fogra39".to_vec())]);
        assert_eq!(x.version.as_deref(), Some("PDF/X-4"));
        assert_eq!(x.trapped.as_deref(), Some(&b"False"[..]));
        let xmp = x.xmp.expect("XMP metadata");
        assert!(xmp.contains("<pdfxid:GTS_PDFXVersion>PDF/X-4</pdfxid:GTS_PDFXVersion>"), "{xmp}");
        assert!(xmp.contains(">Card</rdf:li>") && xmp.contains("<pdf:Trapped>False</pdf:Trapped>"), "{xmp}");
        // The structure tree is not copied, so the source's PDF/UA claim must not be either.
        assert!(!xmp.contains("pdfuaid"), "{xmp}");
    }
}

#[test]
fn combine_keeps_pdfx_when_every_source_agrees() {
    // #263: a PDF/X-4 card combined with itself.
    let (a, b) = (doc_x4("FOGRA39", "icc-fogra39"), doc_x4("FOGRA39", "icc-fogra39"));
    let out = full_roundtrip(&combine(&[("a", &a), ("b", &b)]).unwrap());
    assert_eq!(labels(&out), ["X1", "X2", "X1", "X2"]);
    let x = pdfx_of(&out);
    assert_eq!(x.intents, [("FOGRA39".to_string(), b"icc-fogra39".to_vec())], "one output intent, not one per source");
    assert_eq!(x.version.as_deref(), Some("PDF/X-4"));
    assert!(x.xmp.is_some_and(|x| x.contains("<pdfxid:GTS_PDFXVersion>PDF/X-4</pdfxid:GTS_PDFXVersion>")));
}

#[test]
fn combine_claims_no_pdfx_when_the_sources_disagree() {
    // Another printing condition, the same name with another profile, or a source that is not
    // PDF/X: no one standard describes the result, so it claims none.
    let x4 = doc_x4("FOGRA39", "icc-fogra39");
    for other in [doc_x4("GRACoL2013", "icc-gracol"), doc_x4("FOGRA39", "icc-other"), doc_b()] {
        let out = full_roundtrip(&combine(&[("a", &x4), ("b", &other)]).unwrap());
        assert_eq!(pdfx_of(&out), Pdfx::default());
    }
}

#[test]
fn documents_without_a_print_standard_gain_none() {
    assert_eq!(pdfx_of(&full_roundtrip(&extract_pages(&doc_a(), &[0]).unwrap())), Pdfx::default());
    assert_eq!(pdfx_of(&full_roundtrip(&combine(&[("a", &doc_a()), ("b", &doc_b())]).unwrap())), Pdfx::default());
}

#[test]
fn the_pdfx_version_comes_from_xmp_or_the_document_information() {
    // A PDF/X-4 file may name its version only in XMP (here as an element); a PDF/X-1a:2001 file
    // (PDF 1.3) only in its document information, with no XMP, and gets no XMP added.
    let xmp = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description \
               rdf:about=\"\" xmlns:pdfxid=\"http://www.npes.org/pdfx/ns/id/\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\">\
               <pdfxid:GTS_PDFXVersion> PDF/X-4 </pdfxid:GTS_PDFXVersion><pdf:Trapped>True</pdf:Trapped></rdf:Description></rdf:RDF></x:xmpmeta>";
    let x = pdfx_of(&full_roundtrip(&extract_pages(&doc_print("FOGRA51", "icc", "/Title (Flyer)", Some(xmp)), &[0]).unwrap()));
    assert_eq!((x.version.as_deref(), x.trapped.as_deref()), (Some("PDF/X-4"), Some(&b"True"[..])));
    assert!(x.xmp.is_some_and(|x| x.contains("<pdfxid:GTS_PDFXVersion>PDF/X-4</pdfxid:GTS_PDFXVersion>")));

    let x1a = doc_print("CGATS TR 001", "icc", "/GTS_PDFXVersion (PDF/X-1:2001) /GTS_PDFXConformance (PDF/X-1a:2001) /Trapped (False)", None);
    let out = full_roundtrip(&extract_pages(&x1a, &[0]).unwrap());
    let x = pdfx_of(&out);
    assert_eq!((x.version.as_deref(), x.trapped.as_deref()), (Some("PDF/X-1:2001"), Some(&b"False"[..])));
    assert_eq!(info(&out, "GTS_PDFXConformance").as_deref(), Some("PDF/X-1a:2001"));
    assert_eq!(x.xmp, None);
}

#[test]
fn malformed_output_intents_and_identification_do_not_fail() {
    // Untrusted input: intents that are not dictionaries, a missing profile, an intent that is
    // its own profile, an XMP packet that never closes, and a version full of markup and NULs.
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /OutputIntents [5 0 R 42 (junk) 6 0 R] /Metadata 7 0 R >>".into(), // 1
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] >>".into(),                        // 2
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".into(),                                          // 3
        body("M1"),                                                                                        // 4
        "<< /Type /OutputIntent /S /GTS_PDFX /DestOutputProfile 99 0 R >>".into(),                         // 5
        "<< /Type /OutputIntent /S /GTS_PDFX /OutputConditionIdentifier (x) /DestOutputProfile 6 0 R >>".into(), // 6
        "<< /Length 30 >>\nstream\n<pdfxid:GTS_PDFXConformance>X\nendstream".into(),                       // 7
        "<< /GTS_PDFXVersion (<PDF/X-4 & \"more\">\\000) /Trapped /Maybe >>".into(),                       // 8
    ];
    let src = open(build(&b, "/Root 1 0 R /Info 8 0 R"));
    let out = full_roundtrip(&extract_pages(&src, &[0]).unwrap());
    assert_eq!(labels(&out), ["M1"]);
    let list = catalog(&out).get(b"OutputIntents").map(|o| out.resolve(o).as_array().cloned().unwrap_or_default()).unwrap_or_default();
    assert_eq!(list.len(), 4, "every entry is kept as written");
    let x = pdfx_of(&out);
    assert_eq!(x.trapped, None, "/Maybe is not a trapping state");
    let xmp = x.xmp.expect("XMP metadata");
    assert!(xmp.contains("<pdfxid:GTS_PDFXVersion>&lt;PDF/X-4 &amp; &quot;more&quot;&gt;</pdfxid:GTS_PDFXVersion>"), "{xmp}");
    assert!(!xmp.contains('\0') && !xmp.contains("GTS_PDFXConformance"), "{xmp}");
}

#[test]
fn removing_the_pdfx_identification_keeps_the_output_intent_and_other_claims() {
    let mut x4 = doc_x4("FOGRA51", "icc");
    assert!(matches!(remove_standard_id(&mut x4, Standard::PdfA), Err(OrganizeError::Invalid(_))));
    assert!(!x4.is_modified(), "a refused removal changes nothing");
    remove_standard_id(&mut x4, Standard::PdfX).unwrap();
    let x = pdfx_of(&full_roundtrip(&x4));
    assert_eq!((x.version, x.intents), (None, vec![("FOGRA51".to_string(), b"icc".to_vec())]));
    assert!(x.xmp.is_some_and(|x| !x.contains("pdfxid:GTS") && x.contains("pdfuaid:part=\"1\"")), "PDF/UA's claim stays");
    assert!(matches!(remove_standard_id(&mut x4, Standard::PdfX), Err(OrganizeError::Invalid(_))), "nothing left to remove");

    let mut x1a = doc_print("CGATS TR 001", "icc", "/GTS_PDFXVersion (PDF/X-1:2001) /GTS_PDFXConformance (PDF/X-1a:2001)", None);
    remove_standard_id(&mut x1a, Standard::PdfX).unwrap();
    assert_eq!((info(&x1a, "GTS_PDFXVersion"), info(&x1a, "GTS_PDFXConformance")), (None, None));

    // Those information entries are custom properties, so XMP may also hold them as pdfx: (an
    // element or an attribute); the copies go with them, and other custom properties stay.
    let xmp = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description \
               rdf:about=\"\" xmlns:pdfx=\"http://ns.adobe.com/pdfx/1.3/\" pdfx:GTS_PDFXConformance=\"PDF/X-1a:2001\">\
               <pdfx:GTS_PDFXVersion>PDF/X-1:2001</pdfx:GTS_PDFXVersion><pdfx:Dept>Print</pdfx:Dept></rdf:Description></rdf:RDF></x:xmpmeta>";
    let mut x1a = doc_print("CGATS TR 001", "icc", "/GTS_PDFXVersion (PDF/X-1:2001) /GTS_PDFXConformance (PDF/X-1a:2001)", Some(xmp));
    remove_standard_id(&mut x1a, Standard::PdfX).unwrap();
    let x = pdfx_of(&full_roundtrip(&x1a));
    assert!(x.xmp.as_deref().is_some_and(|x| !x.contains("GTS_") && x.contains("<pdfx:Dept>Print</pdfx:Dept>")), "{:?}", x.xmp);

    // A copy alone declares nothing: refused, and the copy stays.
    let mut copy_only = doc_print("CGATS TR 001", "icc", "/Title (Flyer)", Some(xmp));
    assert!(matches!(remove_standard_id(&mut copy_only, Standard::PdfX), Err(OrganizeError::Invalid(_))));
    assert!(!copy_only.is_modified(), "a refused removal changes nothing");

    // `pdfx` bound to another namespace: the declaration in /Info goes, that namespace's property stays.
    let foreign = xmp.replace("http://ns.adobe.com/pdfx/1.3/", "http://example.com/other/");
    let mut x1a = doc_print("CGATS TR 001", "icc", "/GTS_PDFXVersion (PDF/X-1:2001)", Some(foreign.as_str()));
    remove_standard_id(&mut x1a, Standard::PdfX).unwrap();
    assert_eq!(info(&x1a, "GTS_PDFXVersion"), None);
    assert_eq!(pdfx_of(&full_roundtrip(&x1a)).xmp.as_deref(), Some(foreign.as_str()));
}

#[test]
fn removing_the_pdfa_identification_removes_its_xmp_properties_in_either_form() {
    let xmp = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description \
               rdf:about=\"\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" pdfaid:conformance='B'>\
               <pdfaid:part>2</pdfaid:part><dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">Report</rdf:li></rdf:Alt></dc:title>\
               </rdf:Description></rdf:RDF></x:xmpmeta>";
    let mut doc = doc_print("sRGB IEC61966-2.1", "icc", "/Title (Report)", Some(xmp));
    remove_standard_id(&mut doc, Standard::PdfA).unwrap();
    let x = pdfx_of(&full_roundtrip(&doc));
    let packet = x.xmp.unwrap();
    assert!(!packet.contains("pdfaid:part") && !packet.contains("pdfaid:conformance"), "{packet}");
    assert!(packet.contains(">Report</rdf:li>"), "the rest of the packet stays: {packet}");
    assert_eq!(x.intents.len(), 1, "the output intent stays");
}

#[test]
fn duplicate_pages_become_independent_copies() {
    let out = full_roundtrip(&extract_pages(&doc_b(), &[0, 0]).unwrap());
    assert_eq!(labels(&out), ["B1", "B1"]);
    let ps = pages(&out).unwrap();
    assert_ne!(ps[0].obj, ps[1].obj, "a page object may appear only once in the tree");
}

/// Document C: two pages. Page 1 has a link to the named destination `chap2` (page 2); page 2's
/// content uses a layer that is off by default. It has a bookmark tree and one attachment.
fn doc_c() -> Document {
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Names << /Dests 9 0 R /EmbeddedFiles 13 0 R >> /OCProperties << /OCGs [8 0 R] /D << /OFF [8 0 R] >> >> /Outlines 10 0 R >>".into(), // 1
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] >>".into(), // 2
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /Font << /F1 7 0 R >> >> /Annots [<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /Dest (chap2) >>] >>".into(), // 3
        "<< /Type /Page /Parent 2 0 R /Contents 6 0 R /Resources << /Font << /F1 7 0 R >> /Properties << /L1 8 0 R >> >> >>".into(), // 4
        body("C1"),                                                                   // 5
        body("C2"),                                                                   // 6
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),             // 7
        "<< /Type /OCG /Name (Draft marks) >>".into(),                                // 8
        "<< /Names [(chap2) [4 0 R /Fit]] >>".into(),                                 // 9
        "<< /Type /Outlines /First 11 0 R /Last 11 0 R /Count 1 >>".into(),          // 10
        "<< /Title (Chapter one) /Parent 10 0 R /Dest [3 0 R /Fit] /First 12 0 R /Last 12 0 R /Count 1 >>".into(), // 11
        "<< /Title (Section) /Parent 11 0 R /A << /S /GoTo /D (chap2) >> >>".into(),  // 12
        "<< /Names [(notes.txt) 14 0 R] >>".into(),                                   // 13
        "<< /Type /Filespec /F (notes.txt) /EF << /F 15 0 R >> >>".into(),            // 14
        "<< /Type /EmbeddedFile /Length 5 >>\nstream\nhello\nendstream".into(),       // 15
    ];
    open(build(&b, "/Root 1 0 R"))
}

fn catalog(doc: &Document) -> Dict {
    doc.get(doc.root().unwrap()).as_dict().cloned().unwrap()
}

#[test]
fn named_destinations_are_resolved_to_the_copies() {
    let out = full_roundtrip(&extract_pages(&doc_c(), &[0, 1]).unwrap());
    let link = &annots(&out, 0)[0];
    let dest = link.get(b"Dest").and_then(|d| d.as_array()).expect("explicit destination");
    assert_eq!(dest[0].as_ref(), Some(pages(&out).unwrap()[1].obj));
    // Without the target page, the link loses its destination instead of dangling.
    let alone = full_roundtrip(&extract_pages(&doc_c(), &[0]).unwrap());
    assert!(!annots(&alone, 0)[0].contains(b"Dest"));
}

#[test]
fn layers_are_registered_with_their_default_state() {
    let out = full_roundtrip(&extract_pages(&doc_c(), &[1]).unwrap());
    let props = out.resolve(catalog(&out).get(b"OCProperties").unwrap()).as_dict().cloned().unwrap();
    let ocgs = props.get(b"OCGs").and_then(|o| o.as_array()).unwrap().clone();
    assert_eq!(ocgs.len(), 1);
    let name = out.resolve(&ocgs[0]).as_dict().unwrap().get(b"Name").and_then(|n| n.as_string()).map(|s| s.to_text());
    assert_eq!(name.as_deref(), Some("Draft marks"));
    let d = props.get(b"D").and_then(|d| d.as_dict()).unwrap();
    assert_eq!(d.get(b"OFF").and_then(|o| o.as_array()).unwrap(), &ocgs, "stays off by default");
    // Page 1 uses no layer, so extracting it alone adds no OCProperties.
    let out = extract_pages(&doc_c(), &[0]).unwrap();
    assert!(!catalog(&out).contains(b"OCProperties"));
}

/// The top-level bookmarks' titles, in order.
fn top_titles(doc: &Document) -> Vec<String> {
    let Some(outlines) = catalog(doc).reference(b"Outlines") else { return Vec::new() };
    let mut item = doc.get(outlines).as_dict().and_then(|o| o.reference(b"First"));
    let mut titles = Vec::new();
    while let Some(r) = item {
        let d = doc.get(r).as_dict().cloned().unwrap();
        titles.push(d.get(b"Title").and_then(|t| t.as_string()).map(|s| s.to_text()).unwrap());
        item = d.reference(b"Next");
    }
    titles
}

#[test]
fn a_file_split_around_another_is_copied_once_with_one_bookmark() {
    // C's first page, then all of A, then C's second page.
    let (a, c) = (doc_a(), doc_c());
    let out = full_roundtrip(&crate::combine_grouped(&[(0, "C", &c, Some(&[0])), (1, "A", &a, None), (0, "C", &c, Some(&[1]))]).unwrap());
    assert_eq!(labels(&out), ["C1", "A1", "A2", "A3", "C2"]);
    assert_eq!(top_titles(&out), ["C", "A"], "one bookmark per file, not per run");
    let ps = pages(&out).unwrap();
    let outlines = out.get(catalog(&out).reference(b"Outlines").unwrap()).as_dict().cloned().unwrap();
    let file = out.get(outlines.reference(b"First").unwrap()).as_dict().cloned().unwrap();
    assert_eq!(file.get(b"Dest").and_then(|d| d.as_array()).unwrap()[0].as_ref(), Some(ps[0].obj), "C's bookmark: its first page shown");
    // Its own bookmarks follow its pages wherever they went: the section is on C2, now last.
    let chapter = out.get(file.reference(b"First").unwrap()).as_dict().cloned().unwrap();
    let section = out.get(chapter.reference(b"First").unwrap()).as_dict().cloned().unwrap();
    assert_eq!(section.get(b"Dest").and_then(|d| d.as_array()).unwrap()[0].as_ref(), Some(ps[4].obj));
    // The link from C1 to C2 still works across A's pages between them.
    let link = &annots(&out, 0)[0];
    assert_eq!(link.get(b"Dest").and_then(|d| d.as_array()).expect("kept").first().and_then(|o| o.as_ref()), Some(ps[4].obj));
    // Its attachment once, not once per run.
    let names = out.resolve(catalog(&out).get(b"Names").unwrap()).as_dict().cloned().unwrap();
    let tree = out.resolve(names.get(b"EmbeddedFiles").unwrap()).as_dict().cloned().unwrap();
    let keys: Vec<String> = tree.get(b"Names").and_then(|n| n.as_array()).unwrap().chunks(2).map(|p| p[0].as_string().unwrap().to_text()).collect();
    assert_eq!(keys, ["notes.txt"]);
}

#[test]
fn a_page_shown_twice_gets_its_own_annotations() {
    // One page with an indirect comment (and its popup), and a second page.
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),                                                          // 1
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] >>".into(),                    // 2
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Annots [6 0 R] >>".into(),                            // 3
        "<< /Type /Page /Parent 2 0 R /Contents 7 0 R >>".into(),                                            // 4
        body("D1"),                                                                                          // 5
        "<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (note) /P 3 0 R /Popup 8 0 R >>".into(), // 6
        body("D2"),                                                                                          // 7
        "<< /Type /Annot /Subtype /Popup /Rect [10 10 50 50] /Parent 6 0 R >>".into(),                       // 8
    ];
    let d = open(build(&b, "/Root 1 0 R"));
    // Page 1 twice, from one file split around its page 2 (and the same through combine_selected).
    for out in [
        crate::combine_grouped(&[(0, "D", &d, Some(&[0, 1])), (0, "D", &d, Some(&[0]))]).unwrap(),
        combine_selected(&[("D", &d, Some(&[0, 1, 0]))]).unwrap(),
    ] {
        let out = full_roundtrip(&out);
        assert_eq!(labels(&out), ["D1", "D2", "D1"]);
        let ps = pages(&out).unwrap();
        let annot_of = |i: usize| page_dict(&out, i).get(b"Annots").and_then(|a| a.as_array()).and_then(|a| a[0].as_ref()).unwrap();
        let (first, again) = (annot_of(0), annot_of(2));
        assert_ne!(first, again, "two pages, two comments");
        for (annot, page) in [(first, ps[0].obj), (again, ps[2].obj)] {
            let a = out.get(annot).as_dict().cloned().unwrap();
            assert_eq!(a.reference(b"P"), Some(page), "each comment belongs to its own page");
            let popup = out.get(a.reference(b"Popup").unwrap()).as_dict().cloned().unwrap();
            assert_eq!(popup.reference(b"Parent"), Some(annot), "and its popup to it");
        }
    }
}

#[test]
fn grouped_runs_keep_their_order_repeats_and_separate_files() {
    let (a, b) = (doc_a(), doc_b());
    // B's page 2, A's page 1, B's page 1, A's page 1 again (a repeat is a page of its own).
    let out = full_roundtrip(
        &crate::combine_grouped(&[(0, "B", &b, Some(&[1])), (1, "A", &a, Some(&[0])), (0, "B", &b, Some(&[0])), (1, "A", &a, Some(&[0]))]).unwrap(),
    );
    assert_eq!(labels(&out), ["B2", "A1", "B1", "A1"]);
    assert_eq!(top_titles(&out), ["B", "A"]);
    let ps = pages(&out).unwrap();
    assert_ne!(ps[1].obj, ps[3].obj, "a page shown twice is two pages");
    // The same file added twice on purpose (two groups) keeps two bookmarks, as combine does.
    let twice = full_roundtrip(&crate::combine_grouped(&[(0, "A", &a, None), (1, "A again", &a, None)]).unwrap());
    assert_eq!(top_titles(&twice), ["A", "A again"]);
    // A page that isn't there is refused, as combine_selected does.
    assert!(matches!(crate::combine_grouped(&[(0, "A", &a, Some(&[9]))]), Err(crate::OrganizeError::NoSuchPage(9))));
}

#[test]
fn combine_nests_source_bookmarks_and_keeps_attachments() {
    let out = full_roundtrip(&combine(&[("C", &doc_c()), ("C again", &doc_c())]).unwrap());
    assert_eq!(labels(&out), ["C1", "C2", "C1", "C2"]);
    let outlines = out.get(catalog(&out).reference(b"Outlines").unwrap()).as_dict().cloned().unwrap();
    let file = out.get(outlines.reference(b"First").unwrap()).as_dict().cloned().unwrap();
    assert!(file.int(b"Count").unwrap() < 0, "source bookmarks start collapsed");
    let chapter = out.get(file.reference(b"First").unwrap()).as_dict().cloned().unwrap();
    assert_eq!(chapter.get(b"Title").and_then(|t| t.as_string()).map(|s| s.to_text()).as_deref(), Some("Chapter one"));
    let section = out.get(chapter.reference(b"First").unwrap()).as_dict().cloned().unwrap();
    let dest = section.get(b"Dest").and_then(|d| d.as_array()).expect("GoTo (named) became an explicit destination");
    assert_eq!(dest[0].as_ref(), Some(pages(&out).unwrap()[1].obj));
    // The second copy's bookmarks point into the second copy.
    let file2 = out.get(file.reference(b"Next").unwrap()).as_dict().cloned().unwrap();
    let chapter2 = out.get(file2.reference(b"First").unwrap()).as_dict().cloned().unwrap();
    assert_eq!(chapter2.get(b"Dest").and_then(|d| d.as_array()).unwrap()[0].as_ref(), Some(pages(&out).unwrap()[2].obj));
    // Attachments from both sources, the duplicate name disambiguated.
    let names = out.resolve(catalog(&out).get(b"Names").unwrap()).as_dict().cloned().unwrap();
    let tree = out.resolve(names.get(b"EmbeddedFiles").unwrap()).as_dict().cloned().unwrap();
    let keys: Vec<String> = tree.get(b"Names").and_then(|n| n.as_array()).unwrap().chunks(2).map(|p| p[0].as_string().unwrap().to_text()).collect();
    assert_eq!(keys, ["notes.txt", "notes.txt (2)"]);
}

/// Insert Pages from File brings the inserted file's bookmarks along (they used to be dropped),
/// collapsed under one bookmark named after the file at its first inserted page, placed among the
/// host's bookmarks where the pages went. Destinations follow the copies; named ones are resolved.
#[test]
fn inserting_a_file_brings_its_bookmarks_under_one_named_after_it() {
    let mut host = open(fixture());
    add_bookmark(&mut host, &[], 0, "Start", 0).unwrap();
    add_bookmark(&mut host, &[], 1, "End", 2).unwrap();
    insert_file_pages(&mut host, &doc_c(), "annex", &[0, 1], 1, FieldConflicts::Rename).unwrap();
    let host = save_and_reopen(&host);
    assert_eq!(labels(&host), ["Page 1", "C1", "C2", "Page 2", "Page 3"]);
    let marks = bookmarks(&host);
    assert_eq!(titles(&marks), ["Start", "annex[Chapter one[Section]]", "End"]);
    let page_of = |title: &str| marks.iter().find(|b| b.title == title).and_then(|b| b.page);
    assert_eq!((page_of("annex"), page_of("Chapter one"), page_of("Section"), page_of("End")), (Some(1), Some(1), Some(2), Some(4)));
    assert!(!marks.iter().find(|b| b.title == "annex").unwrap().open, "the file's bookmarks start collapsed");
    // Inserted last, after every bookmark; a bookmark to a page left out stays behind with it.
    let mut host = open(fixture());
    add_bookmark(&mut host, &[], 0, "Start", 0).unwrap();
    insert_file_pages(&mut host, &doc_c(), "annex", &[0], 9, FieldConflicts::Rename).unwrap();
    let marks = bookmarks(&host);
    assert_eq!(titles(&marks), ["Start", "annex[Chapter one]"], "Section's page was left out");
    // A file without bookmarks adds none, and a host without an outline gets one only when needed.
    let mut host = open(fixture());
    insert_file_pages(&mut host, &doc_b(), "plain", &[0], 0, FieldConflicts::Rename).unwrap();
    assert!(bookmarks(&host).is_empty());
    assert!(!catalog(&host).contains(b"Outlines"));
}

/// Bookmarks say where on their page they go, through every kind of destination: explicit,
/// named (resolved), and a GoTo action's.
#[test]
fn bookmarks_report_the_view_their_destination_asks_for() {
    let mut objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 5 0 R /Names << /Dests << /Names [(sec) [4 0 R /FitH 300]] >> >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 400] >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Type /Outlines /First 6 0 R /Last 9 0 R /Count 4 >>".into(),
    ];
    objs.push("<< /Title (Explicit) /Parent 5 0 R /Next 7 0 R /Dest [4 0 R /XYZ 20 250 null] >>".into()); // 6
    objs.push("<< /Title (Named) /Parent 5 0 R /Prev 6 0 R /Next 8 0 R /Dest (sec) >>".into()); // 7
    objs.push("<< /Title (Action) /Parent 5 0 R /Prev 7 0 R /Next 9 0 R /A << /S /GoTo /D [3 0 R /FitR 10 20 110 (x) 999] >> >>".into()); // 8
    objs.push("<< /Title (Nowhere) /Parent 5 0 R /Prev 8 0 R /Dest (missing) >>".into()); // 9
    let doc = open(build(&objs, "/Root 1 0 R"));
    type View = Option<(Vec<u8>, Vec<Option<f64>>)>;
    let views: Vec<(String, Option<usize>, View)> = bookmarks(&doc).into_iter().map(|b| (b.title, b.page, b.view)).collect();
    assert_eq!(
        views,
        [
            ("Explicit".into(), Some(1), Some((b"XYZ".to_vec(), vec![Some(20.0), Some(250.0), None]))),
            ("Named".into(), Some(1), Some((b"FitH".to_vec(), vec![Some(300.0)]))),
            ("Action".into(), Some(0), Some((b"FitR".to_vec(), vec![Some(10.0), Some(20.0), Some(110.0), None]))),
            ("Nowhere".into(), None, None),
        ]
    );
}

#[test]
fn malformed_resource_categories_preserve_annotation_identity_when_combining_and_inserting() {
    for typed in [true, false] {
        for with_popups in [false, true] {
            let mut source = doc_a();
            let page = pages(&source).unwrap()[0].obj;
            let mut annot = Dict::new();
            if typed {
                annot.set(b"Type".to_vec(), Object::name("Annot"));
            }
            annot.set(b"Subtype".to_vec(), Object::name("Text"));
            annot.set(b"Rect".to_vec(), Object::Array([0, 0, 10, 10].map(Object::Int).to_vec()));
            let first = source.add(Object::Dict(annot.clone()));
            let second = source.add(Object::Dict(annot.clone()));
            if with_popups {
                annot.set(b"Subtype".to_vec(), Object::name("Popup"));
                let popup1 = source.add(Object::Dict(annot.clone()));
                let popup2 = source.add(Object::Dict(annot));
                source.update_dict(first, |d| d.set(b"Popup".to_vec(), Object::Ref(popup1))).unwrap();
                source.update_dict(second, |d| d.set(b"Popup".to_vec(), Object::Ref(popup2))).unwrap();
            }
            source
                .update_dict(page, |d| {
                    let mut font_map = Dict::new();
                    font_map.set(b"F1".to_vec(), Object::Ref(first));
                    font_map.set(b"F2".to_vec(), Object::Ref(second));
                    let mut resources = Dict::new();
                    resources.set(b"Font".to_vec(), Object::Dict(font_map));
                    resources.set(b"ExtGState".to_vec(), Object::Ref(second));
                    d.set(b"Resources".to_vec(), Object::Dict(resources));
                    d.set(b"Annots".to_vec(), Object::Array(vec![Object::Ref(first), Object::Ref(second)]));
                })
                .unwrap();
            let combined = crate::combine(&[("source", &source)]).unwrap();
            let mut inserted = doc_a();
            crate::import_pages(&mut inserted, &source, &[0], 0).unwrap();
            for doc in [combined, inserted] {
                let doc = full_roundtrip(&doc);
                let page = pages(&doc).unwrap()[0].obj;
                let refs = doc.get(page).as_dict().unwrap().get(b"Annots").unwrap().as_array().unwrap().clone();
                assert_eq!(refs.len(), 2);
                assert_ne!(refs[0], refs[1], "equal annotations retain their separate identities");
                if with_popups {
                    let popups: Vec<_> = refs.iter().map(|r| doc.resolve(r).as_dict().unwrap().reference(b"Popup").unwrap()).collect();
                    assert_ne!(popups[0], popups[1], "malformed category entries must not merge distinct popups");
                    for popup in popups {
                        assert_eq!(doc.get(popup).as_dict().unwrap().name(b"Subtype"), Some(b"Popup".as_slice()));
                    }
                }
                for r in refs {
                    assert_eq!(doc.resolve(&r).as_dict().unwrap().name(b"Subtype"), Some(b"Text".as_slice()));
                }
            }
        }
    }
}

#[test]
fn combine_and_insert_share_identical_resources() {
    let a = doc_a();
    let fonts = |d: &Document| {
        d.object_numbers()
            .into_iter()
            .filter(|n| d.get(ObjRef::new(*n, d.generation(*n))).as_dict().is_some_and(|x| x.name(b"Type") == Some(b"Font")))
            .count()
    };
    assert_eq!(fonts(&a), 1);
    // The same document three times: one font, and every page still renders its own text.
    let out = full_roundtrip(&crate::combine(&[("one", &a), ("two", &a), ("three", &a)]).unwrap());
    assert_eq!(fonts(&out), 1);
    assert_eq!(labels(&out), ["A1", "A2", "A3", "A1", "A2", "A3", "A1", "A2", "A3"]);
    // Inserting pages that use the same font reuses the font already there, without touching it.
    let mut d = doc_a();
    let font_before =
        d.object_numbers().into_iter().find(|n| d.get(ObjRef::new(*n, 0)).as_dict().is_some_and(|x| x.name(b"Type") == Some(b"Font"))).unwrap();
    crate::import_pages(&mut d, &doc_a(), &[0, 1], 3).unwrap();
    assert_eq!(fonts(&d), 1);
    assert!(!d.modified_objects().contains(&font_before));
    let d = full_roundtrip(&d);
    assert_eq!(labels(&d), ["A1", "A2", "A3", "A1", "A2"]);
}

/// A one-page document drawing one image, written the way ImageMagick writes it: the colour
/// space is an indirect `/DeviceRGB` object of its own (#878).
fn indirect_colour_space_doc() -> Document {
    let objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        stream(b"q 100 0 0 100 0 0 cm /Im0 Do Q"),
        [
            b"<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace 6 0 R /BitsPerComponent 8 /Length 6 >>\nstream\n".as_slice(),
            &[255, 0, 0, 0, 0, 255],
            b"\nendstream",
        ]
        .concat(),
        b"/DeviceRGB".to_vec(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    open(out)
}

#[test]
fn combine_stores_an_image_once_when_its_colour_space_is_an_indirect_name() {
    let images = |d: &Document| {
        d.object_numbers()
            .into_iter()
            .filter(|n| matches!(&*d.get(ObjRef::new(*n, d.generation(*n))), Object::Stream(s) if s.dict.name(b"Subtype") == Some(b"Image")))
            .count()
    };
    let src = indirect_colour_space_doc();
    let out = full_roundtrip(&crate::combine(&[("one", &src), ("two", &src), ("three", &src)]).unwrap());
    assert_eq!(images(&out), 1);
    assert_eq!(pages(&out).unwrap().len(), 3);
    // Inserting the same pages again reuses the image already there.
    let mut d = indirect_colour_space_doc();
    crate::import_pages(&mut d, &indirect_colour_space_doc(), &[0], 1).unwrap();
    assert_eq!(images(&full_roundtrip(&d)), 1);
}

// ---- bookmarks (M4.6) --------------------------------------------------------------------------

/// The titles with children in brackets (`Parent[Child,Child]`), one per top-level bookmark.
fn titles(b: &[crate::Bookmark]) -> Vec<String> {
    fn level(b: &[crate::Bookmark], at: &mut usize, depth: usize) -> Vec<String> {
        let mut out = Vec::new();
        while let Some(x) = b.get(*at).filter(|x| x.path.len() == depth + 1) {
            *at += 1;
            let kids = level(b, at, depth + 1);
            out.push(if kids.is_empty() { x.title.clone() } else { format!("{}[{}]", x.title, kids.join(",")) });
        }
        out
    }
    let mut at = 0;
    level(b, &mut at, 0)
}

fn count_of(doc: &Document, r: ObjRef) -> Option<i64> {
    doc.get(r).as_dict().and_then(|d| d.int(b"Count"))
}

#[test]
fn move_bookmark_to_missing_parent_keeps_outline() {
    let mut d = doc_a();
    crate::add_bookmark(&mut d, &[], 0, "Intro", 0).unwrap();
    crate::add_bookmark(&mut d, &[], 9, "Results", 2).unwrap();
    assert!(crate::move_bookmark(&mut d, &[0], &[7], 0).is_err());
    assert_eq!(titles(&crate::bookmarks(&d)), ["Intro", "Results"]);
}

#[test]
fn bookmarks_add_rename_move_delete() {
    let mut d = doc_a();
    assert!(crate::bookmarks(&d).is_empty());
    assert_eq!(crate::add_bookmark(&mut d, &[], 0, "Intro", 0).unwrap(), vec![0]);
    assert_eq!(crate::add_bookmark(&mut d, &[], 9, "Results", 2).unwrap(), vec![1]);
    assert_eq!(crate::add_bookmark(&mut d, &[1], 0, "Table", 1).unwrap(), vec![1, 0]);
    crate::add_bookmark(&mut d, &[1], 1, "Chart", 2).unwrap();
    assert_eq!(titles(&crate::bookmarks(&d)), ["Intro", "Results[Table,Chart]"]);

    crate::rename_bookmark(&mut d, &[1, 1], "Figure 1").unwrap();
    assert_eq!(crate::move_bookmark(&mut d, &[1, 1], &[], 0).unwrap(), vec![0]);
    assert_eq!(titles(&crate::bookmarks(&d)), ["Figure 1", "Intro", "Results[Table]"]);
    // Into a later sibling: the target path is adjusted for the removal.
    assert_eq!(crate::move_bookmark(&mut d, &[0], &[2], 9).unwrap(), vec![1, 1]);
    assert_eq!(titles(&crate::bookmarks(&d)), ["Intro", "Results[Table,Figure 1]"]);
    assert_eq!(crate::move_bookmark(&mut d, &[1], &[1, 0], 0), Err(crate::OutlineError::IntoItself));
    // Into a bookmark with no children yet (Indent): it opens, so the moved one shows.
    assert_eq!(crate::move_bookmark(&mut d, &[1, 1], &[0], 0).unwrap(), vec![0, 0]);
    let intro = crate::bookmarks(&d)[0].obj;
    assert_eq!(count_of(&d, intro), Some(1));
    // Reordering inside a closed parent leaves it closed.
    crate::add_bookmark(&mut d, &[0], 1, "Second", 0).unwrap();
    crate::set_bookmark_open(&mut d, &[0], false).unwrap();
    crate::move_bookmark(&mut d, &[0, 1], &[0], 0).unwrap();
    assert_eq!(count_of(&d, intro), Some(-2));
    crate::delete_bookmark(&mut d, &[0, 0]).unwrap();
    crate::move_bookmark(&mut d, &[0, 0], &[1], 9).unwrap();
    assert_eq!(titles(&crate::bookmarks(&d)), ["Intro", "Results[Table,Figure 1]"]);
    crate::delete_bookmark(&mut d, &[1, 0]).unwrap();
    assert_eq!(titles(&crate::bookmarks(&d)), ["Intro", "Results[Figure 1]"]);

    // Counts: the root counts visible items; open items count their visible descendants.
    let b = crate::bookmarks(&d);
    let root = d.get(d.root().unwrap()).as_dict().unwrap().reference(b"Outlines").unwrap();
    assert_eq!(count_of(&d, root), Some(3));
    assert_eq!(count_of(&d, b[1].obj), Some(1));
    crate::set_bookmark_open(&mut d, &[1], false).unwrap();
    assert_eq!((count_of(&d, b[1].obj), count_of(&d, root)), (Some(-1), Some(2)));

    // Destinations point at the chosen page; errors are specific.
    crate::set_bookmark_page(&mut d, &[0], 2).unwrap();
    let dest = d.get(b[0].obj).as_dict().unwrap().get(b"Dest").cloned().unwrap();
    let pages = crate::walk(&d).unwrap();
    assert_eq!(dest.as_array().unwrap()[0], Object::Ref(pages[2].0));
    assert_eq!(crate::rename_bookmark(&mut d, &[5], "x"), Err(crate::OutlineError::NoSuchBookmark(vec![5])));
    assert_eq!(crate::rename_bookmark(&mut d, &[0], "  "), Err(crate::OutlineError::EmptyTitle));
    assert!(crate::add_bookmark(&mut d, &[], 0, "Nowhere", 7).is_err());

    // Everything survives a save and reopen.
    let back = full_roundtrip(&d);
    assert_eq!(titles(&crate::bookmarks(&back)), ["Intro", "Results[Figure 1]"]);
}

#[test]
fn a_bookmark_tree_nests_entries_by_level_under_a_new_first_bookmark() {
    let mut d = doc_a();
    crate::add_bookmark(&mut d, &[], 0, "Existing", 0).unwrap();
    let e = |level, title: &str, page, element| crate::OutlineEntry { level, title: title.into(), page, element, left: None, top: None };
    let se = ObjRef::new(4242, 0);
    let entries = [e(1, "One", 0, Some(se)), e(3, "Deep", 1, None), e(2, "Two", 1, None), e(1, "Next", 2, None)];
    assert_eq!(crate::add_bookmark_tree(&mut d, "Untitled", &entries).unwrap(), vec![0]);
    assert_eq!(titles(&crate::bookmarks(&d)), ["Untitled[One[Deep,Two],Next]", "Existing"]);

    let b = crate::bookmarks(&d);
    let root = d.get(d.root().unwrap()).as_dict().unwrap().reference(b"Outlines").unwrap();
    // Listing order: Untitled, One, Deep, Two, Next, Existing.
    assert_eq!((count_of(&d, root), count_of(&d, b[0].obj), count_of(&d, b[1].obj)), (Some(6), Some(4), Some(2)));
    let one = d.get(b[1].obj).as_dict().cloned().unwrap();
    assert_eq!(one.get(b"SE"), Some(&Object::Ref(se)));
    let pages = crate::walk(&d).unwrap();
    let next = d.get(b[4].obj).as_dict().cloned().unwrap();
    assert_eq!(next.get(b"Dest").and_then(|x| x.as_array()).map(|a| a[0].clone()), Some(Object::Ref(pages[2].0)));

    assert_eq!(crate::add_bookmark_tree(&mut d, "Untitled", &[]), Err(crate::OutlineError::NoEntries));
    assert!(crate::add_bookmark_tree(&mut d, "Untitled", &[e(1, "Nowhere", 9, None)]).is_err());
    assert_eq!(titles(&crate::bookmarks(&full_roundtrip(&d))), ["Untitled[One[Deep,Two],Next]", "Existing"]);
}

#[test]
fn bookmark_edits_keep_unknown_keys_and_touch_few_objects() {
    let mut d = doc_a();
    crate::add_bookmark(&mut d, &[], 0, "One", 0).unwrap();
    crate::add_bookmark(&mut d, &[], 1, "Two", 1).unwrap();
    let first = crate::bookmarks(&d)[0].obj;
    d.update_dict(first, |x| x.set(b"C".to_vec(), Object::Array(vec![Object::Real(1.0), Object::Int(0), Object::Int(0)]))).unwrap();
    let bytes = std::sync::Arc::new(write_full(&d, &SaveOptions::default()).unwrap());
    let mut d = Document::open(bytes).unwrap();
    crate::rename_bookmark(&mut d, &[0], "Uno").unwrap();
    let b = crate::bookmarks(&d);
    assert_eq!(b[0].title, "Uno");
    assert!(d.get(b[0].obj).as_dict().unwrap().get(b"C").is_some(), "colour kept");
    assert_eq!(d.modified_objects(), vec![b[0].obj.num], "a rename rewrites only that item");
}

/// A flat outline of `n` top-level bookmarks, `Item 0` to `Item n-1`, all going to one page.
fn flat_outline(n: usize) -> Vec<u8> {
    let mut b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 3 0 R >>".into(),
        "<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),
        format!("<< /Type /Outlines /First 5 0 R /Last {} 0 R /Count {n} >>", 4 + n),
        "<< /Type /Page /Parent 2 0 R >>".into(),
    ];
    for k in 0..n {
        let prev = if k == 0 { String::new() } else { format!(" /Prev {} 0 R", 4 + k) };
        let next = if k + 1 == n { String::new() } else { format!(" /Next {} 0 R", 6 + k) };
        b.push(format!("<< /Title (Item {k}) /Parent 3 0 R{prev}{next} /Dest [4 0 R /Fit] >>"));
    }
    build(&b, "/Root 1 0 R")
}

#[test]
fn a_bookmark_past_the_hundred_thousandth_is_listed() {
    let d = open(flat_outline(100_005));
    let all = crate::bookmarks(&d);
    assert_eq!(all.len(), 100_005);
    assert_eq!(all.last().map(|b| b.title.as_str()), Some("Item 100004"));
}

/// A chain of `n` nested bookmarks: each is the first child of the one before it.
fn deep_outline(n: usize) -> Vec<u8> {
    let mut b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 3 0 R >>".into(),
        "<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),
        "<< /Type /Outlines /First 5 0 R /Last 5 0 R /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
    ];
    for k in 0..n {
        let parent = if k == 0 { 3 } else { 4 + k };
        let first = if k + 1 == n { String::new() } else { format!(" /First {} 0 R /Count -1", 6 + k) };
        b.push(format!("<< /Title (Deep {k}) /Parent {parent} 0 R{first} /Dest [4 0 R /Fit] >>"));
    }
    build(&b, "/Root 1 0 R")
}

#[test]
fn deep_bookmark_chains_edit_without_recursion() {
    let mut d = open(deep_outline(100_000));
    // Adding at the top level recounts the whole outline, which walks the entire chain.
    crate::add_bookmark(&mut d, &[], 1, "Extra", 0).unwrap();
    // Listing enters levels 0 to 32 of the chain; the rest is reached by path only.
    let listed = crate::bookmarks(&d);
    assert_eq!(listed.len(), 34);
    assert_eq!(listed.last().map(|b| (b.title.as_str(), b.path.clone())), Some(("Extra", vec![1])));
    assert_eq!(listed[32].path.len(), 33);
    assert_eq!(crate::bookmark_page(&d, 33, 10).bookmarks.len(), 1);
}

/// A one-page document whose outline is the given bookmark bodies, as objects 5, 6, … in order.
fn outline_with(items: &[&str]) -> Document {
    let mut b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 3 0 R >>".into(),
        "<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),
        format!("<< /Type /Outlines /First 5 0 R /Count {} >>", items.len()),
        "<< /Type /Page /Parent 2 0 R >>".into(),
    ];
    b.extend(items.iter().map(ToString::to_string));
    open(build(&b, "/Root 1 0 R"))
}

#[test]
fn hostile_outlines_end_their_runs_and_never_loop() {
    let listed = |items: &[&str]| titles(&crate::bookmarks(&outline_with(items)));
    // A bookmark that is its own next sibling, or its own first child.
    assert_eq!(listed(&["<< /Title (Self) /Parent 3 0 R /Next 5 0 R >>"]), ["Self"]);
    assert_eq!(listed(&["<< /Title (Self) /Parent 3 0 R /First 5 0 R >>"]), ["Self"]);
    // Two siblings that point at each other: the run ends where it would repeat.
    assert_eq!(listed(&["<< /Title (A) /Parent 3 0 R /Next 6 0 R >>", "<< /Title (B) /Parent 3 0 R /Next 5 0 R >>"]), ["A", "B"]);
    // A child that leads back to its ancestor.
    assert_eq!(listed(&["<< /Title (A) /Parent 3 0 R /First 6 0 R >>", "<< /Title (B) /Parent 5 0 R /First 5 0 R >>"]), ["A[B]"]);
    // A broken `/Next` (a missing object, then a number) and a missing first bookmark.
    assert_eq!(listed(&["<< /Title (A) /Parent 3 0 R /Next 99 0 R >>"]), ["A"]);
    assert_eq!(listed(&["<< /Title (A) /Parent 3 0 R /Next 6 0 R >>", "42"]), ["A"]);
    assert!(listed(&[]).is_empty());
}

/// `n` top-level bookmarks that all share one indirect `/Title` string of `title_len` bytes (object 5).
fn shared_title_outline(n: usize, title_len: usize) -> Document {
    let title = "x".repeat(title_len);
    let mut items = vec![format!("({title})")];
    for k in 0..n {
        let prev = if k == 0 { String::new() } else { format!(" /Prev {} 0 R", 5 + k) };
        let next = if k + 1 == n { String::new() } else { format!(" /Next {} 0 R", 7 + k) };
        items.push(format!("<< /Title 5 0 R /Parent 3 0 R{prev}{next} /Dest [4 0 R /Fit] >>"));
    }
    let mut b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 3 0 R >>".into(),
        "<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),
        format!("<< /Type /Outlines /First 6 0 R /Last {} 0 R /Count {n} >>", 5 + n),
        "<< /Type /Page /Parent 2 0 R >>".into(),
    ];
    b.extend(items);
    open(build(&b, "/Root 1 0 R"))
}

#[test]
fn bookmarks_sharing_one_huge_title_are_listed_within_the_title_allowance() {
    // 40 bookmarks share a 1 MiB title: copied into each, a listing would hold 40 MiB of text.
    let d = shared_title_outline(40, 1 << 20);
    let page = crate::bookmark_page(&d, 0, 40);
    assert_eq!(page.bookmarks.len(), 40, "every bookmark is still listed");
    let total: usize = page.bookmarks.iter().map(|b| b.title.len()).sum();
    assert!(total <= 16 << 20, "titles listed {total} bytes");
    assert_eq!(page.bookmarks.iter().filter(|b| !b.title.is_empty()).count(), 16);
    assert!(page.bookmarks.iter().filter(|b| b.title.is_empty()).all(|b| b.title_dropped));
    // Each call has its own allowance: a second page of 20 lists 16 titles again.
    let second = crate::bookmark_page(&d, 20, 20);
    assert_eq!(second.bookmarks.iter().filter(|b| !b.title.is_empty()).count(), 16);
    // A title that fits is untouched.
    let small = shared_title_outline(40, 100);
    assert!(crate::bookmarks(&small).iter().all(|b| b.title.len() == 100 && !b.title_dropped));
}

#[test]
fn a_long_destination_array_is_copied_only_as_far_as_it_is_read() {
    // Object 5 is the array, which a bookmark (object 6) names as its /Dest.
    let array = format!("[4 0 R /XYZ 1 2 3 {}]", "0 ".repeat(100_000));
    let d = outline_with(&[&array, "<< /Title (A) /Parent 3 0 R /Dest 5 0 R >>"]);
    let dest = crate::outline::resolve_dest(&d, &Object::Ref(pdfcraft_cos::ObjRef::new(5, 0))).unwrap();
    assert!(dest.len() <= 6, "a destination array of 100,000 items was copied whole: {}", dest.len());
    assert_eq!(dest.get(1), Some(&Object::name("XYZ")));
}

#[test]
fn an_outline_with_a_title_past_the_allowance_is_exported_as_truncated() {
    let d = shared_title_outline(40, 1 << 20);
    let tree = crate::outline_tree(&d, &[]).unwrap();
    assert!(tree.truncated, "empty titles must not pass as a faithful copy");
    assert_eq!(tree.nodes.len(), 40);
    assert_eq!(tree.nodes.iter().filter(|n| n.title.is_empty()).count(), 24);
    // A subtree whose titles all fit is complete.
    let tree = crate::outline_tree(&d, &[0]).unwrap();
    assert!(!tree.truncated);
    assert_eq!(tree.nodes.len(), 1);
    let small = crate::outline_tree(&shared_title_outline(40, 100), &[]).unwrap();
    assert!(!small.truncated);
}

#[test]
fn bookmark_pages_list_every_bookmark_once() {
    let d = open(flat_outline(250));
    let mut seen = Vec::new();
    let mut offset = 0;
    while let Some(next) = {
        let page = crate::bookmark_page(&d, offset, 100);
        assert!(page.bookmarks.len() <= 100);
        seen.extend(page.bookmarks.iter().map(|b| b.title.clone()));
        page.next
    } {
        assert_eq!(next, seen.len(), "a page ends where the next one starts");
        offset = next;
    }
    assert_eq!(seen.len(), 250);
    assert_eq!(seen.last().map(String::as_str), Some("Item 249"));

    let big = open(flat_outline(100_005));
    let first = crate::bookmark_page(&big, 0, 100);
    assert_eq!((first.bookmarks.len(), first.next), (100, Some(100)));
    let late = crate::bookmark_page(&big, 100_000, 100);
    assert_eq!(
        late.bookmarks.iter().map(|b| b.title.as_str()).collect::<Vec<_>>(),
        ["Item 100000", "Item 100001", "Item 100002", "Item 100003", "Item 100004"]
    );
    assert_eq!((late.bookmarks[4].path.clone(), late.next), (vec![100_004], None));
}

#[test]
fn bookmarks_name_their_pages_through_every_kind_of_destination() {
    let objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 3 0 R /Names << /Dests 9 0 R >> >>".into(),
        "<< /Type /Pages /Kids [4 0 R 5 0 R] /Count 2 /MediaBox [0 0 300 400] >>".into(),
        "<< /Type /Outlines /First 6 0 R /Last 11 0 R /Count 6 >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Title (Named) /Parent 3 0 R /Next 7 0 R /Dest (intro) >>".into(),
        "<< /Title (GoTo) /Parent 3 0 R /Prev 6 0 R /Next 8 0 R /A << /S /GoTo /D [5 0 R /Fit] >> >>".into(),
        "<< /Title (Appendix) /Parent 3 0 R /Prev 7 0 R /Next 10 0 R /Dest /appendix >>".into(),
        "<< /Names [(appendix) 12 0 R (intro) [4 0 R /Fit]] >>".into(),
        "<< /Title <FEFF00480069> /Parent 3 0 R /Prev 8 0 R /Next 11 0 R /Dest [1 /Fit] >>".into(),
        "<< /Title (  Padded  ) /Parent 3 0 R /Prev 10 0 R /Dest [2 0 R /Fit] >>".into(),
        "<< /D [5 0 R /Fit] >>".into(),
    ];
    let d = open(build(&objs, "/Root 1 0 R"));
    let listed: Vec<String> = crate::bookmarks(&d).iter().map(|b| format!("{}@{:?}", b.title, b.page)).collect();
    assert_eq!(listed, ["Named@Some(0)", "GoTo@Some(1)", "Appendix@Some(1)", "Hi@Some(1)", "Padded@None"]);
}

/// Only a go-to's `/D` is a page of this document. A remote go-to's names a page or destination of
/// the other file (ISO 32000-2 §12.6.4.3), so `[1 /Fit]` there is not this file's page 2, and a
/// destination name is not looked up here; neither is any other action's `/D`. An action without
/// a type is still read as the go-to its `/D` suggests.
#[test]
fn only_a_go_to_action_names_a_page_here() {
    let objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 3 0 R /Names << /Dests 12 0 R >> >>".into(),
        "<< /Type /Pages /Kids [4 0 R 5 0 R] /Count 2 /MediaBox [0 0 300 400] >>".into(),
        "<< /Type /Outlines /First 6 0 R /Last 11 0 R /Count 6 >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Title (GoTo) /Parent 3 0 R /Next 7 0 R /A << /S /GoTo /D [5 0 R /Fit] >> >>".into(),
        "<< /Title (Other file) /Parent 3 0 R /Prev 6 0 R /Next 8 0 R /A << /S /GoToR /F (other.pdf) /D [1 /Fit] >> >>".into(),
        "<< /Title (Other file, named) /Parent 3 0 R /Prev 7 0 R /Next 9 0 R /A << /S /GoToR /F (other.pdf) /D (intro) >> >>".into(),
        "<< /Title (Web) /Parent 3 0 R /Prev 8 0 R /Next 10 0 R /A << /S /URI /URI (https://example.org/) /D [5 0 R /Fit] >> >>".into(),
        "<< /Title (Indirect type) /Parent 3 0 R /Prev 9 0 R /Next 11 0 R /A << /S 13 0 R /D [5 0 R /Fit] >> >>".into(),
        "<< /Title (No type) /Parent 3 0 R /Prev 10 0 R /A << /D [5 0 R /Fit] >> >>".into(),
        "<< /Names [(intro) [4 0 R /Fit]] >>".into(),
        "/GoTo".into(),
    ];
    let d = open(build(&objs, "/Root 1 0 R"));
    let listed: Vec<String> = crate::bookmarks(&d).iter().map(|b| format!("{}@{:?}", b.title, b.page)).collect();
    assert_eq!(listed, ["GoTo@Some(1)", "Other file@None", "Other file, named@None", "Web@None", "Indirect type@Some(1)", "No type@Some(1)"]);
}

// ---- page labels (M4.4) ------------------------------------------------------------------------

#[test]
fn number_pages_like_acrobat() {
    use crate::LabelStyle::*;
    let mut d = doc_a();
    crate::insert_blank_page(&mut d, 3, 300.0, 400.0).unwrap();
    crate::insert_blank_page(&mut d, 4, 300.0, 400.0).unwrap();
    crate::insert_blank_page(&mut d, 5, 300.0, 400.0).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap(), ["1", "2", "3", "4", "5", "6"]);
    // Front matter in roman numerals; later pages keep their labels.
    crate::number_pages(&mut d, 0, 1, LowerRoman, "", 1).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap(), ["i", "ii", "3", "4", "5", "6"]);
    // The body restarts at 1, an appendix gets a prefix.
    crate::number_pages(&mut d, 2, 5, Decimal, "", 1).unwrap();
    crate::number_pages(&mut d, 4, 5, Decimal, "A-", 1).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap(), ["i", "ii", "1", "2", "A-1", "A-2"]);
    assert_eq!(crate::page_label_ranges(&d).len(), 3, "continuations are merged");
    // Relabelling the middle keeps the pages after it.
    crate::number_pages(&mut d, 3, 3, UpperAlpha, "Fig ", 3).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap(), ["i", "ii", "1", "Fig C", "A-1", "A-2"]);
    assert!(crate::number_pages(&mut d, 4, 9, Decimal, "", 1).is_err());
    // Survives a save and reopen; back to plain numbers removes /PageLabels.
    let back = full_roundtrip(&d);
    assert_eq!(crate::page_labels(&back).unwrap(), ["i", "ii", "1", "Fig C", "A-1", "A-2"]);
    crate::number_pages(&mut d, 0, 5, Decimal, "", 1).unwrap();
    assert!(crate::page_label_ranges(&d).is_empty());
    assert!(d.get(d.root().unwrap()).as_dict().unwrap().get(b"PageLabels").is_none());
}

/// Deleting pages moves the label ranges with the pages they label (#2091): later ranges start that
/// many pages earlier and keep numbering on, a range whose first page went starts on the next page
/// left, and a range left with no pages gives way to the one after it.
#[test]
fn deleting_pages_keeps_the_label_ranges_on_their_pages() {
    use crate::{LabelRange, LabelStyle::*};
    let ten = || {
        let mut d = doc_a();
        for at in 3..10 {
            crate::insert_blank_page(&mut d, at, 300.0, 400.0).unwrap();
        }
        let ranges = [
            LabelRange { start: 0, style: LowerRoman, prefix: String::new(), first: 1 },
            LabelRange { start: 3, style: Decimal, prefix: String::new(), first: 1 },
            LabelRange { start: 7, style: Decimal, prefix: "A-".into(), first: 1 },
        ];
        crate::set_page_label_ranges(&mut d, &ranges).unwrap();
        assert_eq!(crate::page_labels(&d).unwrap(), ["i", "ii", "iii", "1", "2", "3", "4", "A-1", "A-2", "A-3"]);
        d
    };
    for (deleted, expected) in [
        (&[4, 5][..], &["i", "ii", "iii", "1", "2", "A-1", "A-2", "A-3"][..]),
        (&[3], &["i", "ii", "iii", "1", "2", "3", "A-1", "A-2", "A-3"]),
        (&[3, 4, 5, 6], &["i", "ii", "iii", "A-1", "A-2", "A-3"]),
    ] {
        let mut d = ten();
        crate::delete_pages(&mut d, deleted).unwrap();
        assert_eq!(crate::page_labels(&d).unwrap(), expected, "after deleting {deleted:?}");
        assert_eq!(crate::page_labels(&full_roundtrip(&d)).unwrap(), expected, "reopened after deleting {deleted:?}");
    }
}

#[test]
fn extracting_splitting_and_combining_preserve_page_labels() {
    use crate::{
        LabelRange,
        LabelStyle::{Decimal, LowerRoman, UpperAlpha},
        SplitBy,
    };
    let ten = || {
        let mut d = doc_a();
        for at in 3..10 {
            crate::insert_blank_page(&mut d, at, 300.0, 400.0).unwrap();
        }
        let ranges = [
            LabelRange { start: 0, style: LowerRoman, prefix: String::new(), first: 1 },
            LabelRange { start: 3, style: Decimal, prefix: String::new(), first: 1 },
            LabelRange { start: 7, style: Decimal, prefix: "A-".into(), first: 1 },
        ];
        crate::set_page_label_ranges(&mut d, &ranges).unwrap();
        d
    };
    let d = ten();

    // Extracting non-contiguous pages (e.g. first front-matter page and first appendix page)
    // keeps their labels and writes /PageLabels (#2670).
    let pair = full_roundtrip(&extract_pages(&d, &[0, 7]).unwrap());
    assert_eq!(crate::page_labels(&pair).unwrap(), ["i", "A-1"]);
    assert_eq!(
        crate::page_label_ranges(&pair),
        [
            LabelRange { start: 0, style: LowerRoman, prefix: String::new(), first: 1 },
            LabelRange { start: 1, style: Decimal, prefix: "A-".into(), first: 1 },
        ]
    );

    // Contiguous runs and out-of-order picks keep each page's label and merge continuations.
    let picked = full_roundtrip(&extract_pages(&d, &[1, 2, 8, 9, 0]).unwrap());
    assert_eq!(crate::page_labels(&picked).unwrap(), ["ii", "iii", "A-2", "A-3", "i"]);
    assert_eq!(
        crate::page_label_ranges(&picked),
        [
            LabelRange { start: 0, style: LowerRoman, prefix: String::new(), first: 2 },
            LabelRange { start: 2, style: Decimal, prefix: "A-".into(), first: 2 },
            LabelRange { start: 4, style: LowerRoman, prefix: String::new(), first: 1 },
        ]
    );

    // An unlabelled source stays unlabelled after extraction.
    let unlabelled = full_roundtrip(&extract_pages(&doc_a(), &[2, 0]).unwrap());
    assert!(crate::page_label_ranges(&unlabelled).is_empty());

    // Splitting preserves each part's labels, including mid-range starting numbers.
    let parts = crate::split(&d, &SplitBy::PageCount(4)).unwrap();
    let part_labels: Vec<Vec<String>> = parts.iter().map(|p| crate::page_labels(&full_roundtrip(p)).unwrap()).collect();
    assert_eq!(part_labels, vec![vec!["i", "ii", "iii", "1"], vec!["2", "3", "4", "A-1"], vec!["A-2", "A-3"],]);

    // Combining whole files, selected pages, and grouped runs preserves labels from each source.
    let mut b = doc_b();
    crate::set_page_label_ranges(&mut b, &[LabelRange { start: 0, style: UpperAlpha, prefix: "B-".into(), first: 1 }]).unwrap();
    let combined = full_roundtrip(&combine_selected(&[("D", &d, Some(&[0, 1, 7])), ("Plain", &doc_a(), Some(&[0, 1])), ("B", &b, None)]).unwrap());
    assert_eq!(crate::page_labels(&combined).unwrap(), ["i", "ii", "A-1", "1", "2", "B-A", "B-B"]);

    let grouped =
        full_roundtrip(&crate::combine_grouped(&[(0, "D", &d, Some(&[0, 1])), (1, "B", &b, Some(&[1])), (0, "D", &d, Some(&[2, 7]))]).unwrap());
    assert_eq!(crate::page_labels(&grouped).unwrap(), ["i", "ii", "B-B", "iii", "A-1"]);

    // Ranges starting after page 0 and prefix-only ranges (LabelStyle::None) merge and carry over.
    let mut late = doc_a();
    crate::set_page_label_ranges(
        &mut late,
        &[
            LabelRange { start: 1, style: crate::LabelStyle::None, prefix: "Cover".into(), first: 1 },
            LabelRange { start: 2, style: LowerRoman, prefix: String::new(), first: 4 },
        ],
    )
    .unwrap();
    let from_late = full_roundtrip(&extract_pages(&late, &[0, 1, 1, 2]).unwrap());
    assert_eq!(crate::page_labels(&from_late).unwrap(), ["1", "Cover", "Cover", "iv"]);
    assert_eq!(
        crate::page_label_ranges(&from_late),
        [
            LabelRange { start: 0, style: Decimal, prefix: String::new(), first: 1 },
            LabelRange { start: 1, style: crate::LabelStyle::None, prefix: "Cover".into(), first: 1 },
            LabelRange { start: 3, style: LowerRoman, prefix: String::new(), first: 4 },
        ]
    );

    // An empty selection from a labelled source in combine_selected does not add /PageLabels.
    let empty_pick = full_roundtrip(&combine_selected(&[("D", &d, Some(&[])), ("Plain", &doc_a(), Some(&[0, 1]))]).unwrap());
    assert!(crate::page_label_ranges(&empty_pick).is_empty());

    // A damaged (cyclic) /PageLabels tree does not prevent extracting pages.
    let mut cyclic = doc_a();
    let tree = cyclic.add(Object::Null);
    let mut dict = Dict::new();
    dict.set(b"Kids".to_vec(), Object::Array(vec![Object::Ref(tree)]));
    cyclic.set(tree, Object::Dict(dict));
    cyclic.update_dict(cyclic.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree))).unwrap();
    let extracted_cyclic = full_roundtrip(&extract_pages(&cyclic, &[0, 2]).unwrap());
    assert_eq!(crate::page_labels(&extracted_cyclic).unwrap(), ["1", "2"]);

    // An empty prefix-only range falls back to physical page numbers in the source; extracting a
    // later page covered by it preserves that physical page number across save/reopen.
    let mut empty_prefix = doc_a();
    crate::set_page_label_ranges(
        &mut empty_prefix,
        &[
            LabelRange { start: 0, style: crate::LabelStyle::None, prefix: String::new(), first: 1 },
            LabelRange { start: 2, style: LowerRoman, prefix: String::new(), first: 1 },
        ],
    )
    .unwrap();
    assert_eq!(crate::PageLabels::new(&empty_prefix).unwrap().label(1).unwrap(), "2");
    let extracted_empty_prefix = full_roundtrip(&extract_pages(&empty_prefix, &[1]).unwrap());
    assert_eq!(crate::PageLabels::new(&extracted_empty_prefix).unwrap().label(0).unwrap(), "2");

    // Repeated non-continuing ranges that exceed the 4 MiB total-prefix limit or the range-count
    // limit are refused rather than writing an unreadable tree or silently dropping labels.
    let mut one = flat_doc(1);
    crate::number_pages(&mut one, 0, 0, Decimal, &"x".repeat(1_000), 1).unwrap();
    let err = extract_pages(&one, &vec![0; 4_195]).unwrap_err().to_string();
    assert!(err.contains("decoded prefixes exceed 4 MiB in total"), "{err}");

    crate::number_pages(&mut one, 0, 0, LowerRoman, "", 2).unwrap();
    let err = extract_pages(&one, &vec![0; pdfcraft_cos::page_labels::MAX_LABEL_TREE_WORK]).unwrap_err().to_string();
    assert!(err.contains("the number tree has too many entries"), "{err}");
}

/// A range the reader accepts but the writer cannot store (a Roman numeral past 1,024 bytes, or a
/// decimal span whose labels grow past that) degrades to the source physical numbers. Extract
/// still succeeds, and a neighbouring range that does fit is kept.
#[test]
fn extracting_unrepresentable_labels_uses_physical_numbers() {
    let mut roman = doc_a();
    let mut spec = Dict::new();
    spec.set(b"S".to_vec(), Object::name("R"));
    spec.set(b"St".to_vec(), Object::Int(4_000_000));
    let mut tree = Dict::new();
    tree.set(b"Nums".to_vec(), Object::Array(vec![Object::Int(0), Object::Dict(spec)]));
    let tree = roman.add(Object::Dict(tree));
    roman.update_dict(roman.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree))).unwrap();
    let extracted = full_roundtrip(&extract_pages(&roman, &[0, 2]).unwrap());
    assert_eq!(crate::page_labels(&extracted).unwrap(), ["1", "3"]);

    let mut wide = flat_doc(5);
    let mut spec = Dict::new();
    spec.set(b"P".to_vec(), Object::String(PdfString::text(&"p".repeat(1_020))));
    spec.set(b"S".to_vec(), Object::name("D"));
    spec.set(b"St".to_vec(), Object::Int(9_996));
    let mut tree = Dict::new();
    tree.set(b"Nums".to_vec(), Object::Array(vec![Object::Int(0), Object::Dict(spec)]));
    let tree = wide.add(Object::Dict(tree));
    wide.update_dict(wide.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree))).unwrap();
    let extracted = full_roundtrip(&extract_pages(&wide, &[0, 1, 2, 3, 4]).unwrap());
    assert_eq!(crate::page_labels(&extracted).unwrap(), ["1", "2", "3", "4", "5"]);

    let mut mixed = doc_a();
    let mut ok = Dict::new();
    ok.set(b"P".to_vec(), Object::String(PdfString::text("ok-")));
    ok.set(b"S".to_vec(), Object::name("D"));
    let mut huge = Dict::new();
    huge.set(b"S".to_vec(), Object::name("R"));
    huge.set(b"St".to_vec(), Object::Int(4_000_000));
    let mut tree = Dict::new();
    tree.set(b"Nums".to_vec(), Object::Array(vec![Object::Int(0), Object::Dict(ok), Object::Int(1), Object::Dict(huge)]));
    let tree = mixed.add(Object::Dict(tree));
    mixed.update_dict(mixed.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree))).unwrap();
    let extracted = full_roundtrip(&extract_pages(&mixed, &[0, 1]).unwrap());
    assert_eq!(crate::page_labels(&extracted).unwrap(), ["ok-1", "2"]);
}

#[test]
fn page_label_limits_reject_unsupported_edits_before_mutating() {
    use crate::LabelStyle::*;
    for (style, prefix, first) in [
        (LowerRoman, String::new(), 1_025_000),
        (LowerAlpha, String::new(), 26 * 1024 + 1),
        (Decimal, "x".repeat(1025), 1),
        (Decimal, String::new(), u32::MAX),
    ] {
        let mut d = doc_a();
        let before = d.modified_objects();
        assert!(crate::number_pages(&mut d, 0, 1, style, &prefix, first).is_err());
        assert_eq!(d.modified_objects(), before, "a refused label edit must not change the tree");
        assert!(crate::page_label_ranges(&d).is_empty());
    }
}

#[test]
fn page_label_limits_reject_unsupported_ranges_before_mutating() {
    let mut d = doc_a();
    let before = d.modified_objects();
    let ranges = [crate::LabelRange { start: 0, style: crate::LabelStyle::LowerRoman, prefix: String::new(), first: 1_025_000 }];
    assert!(crate::set_page_label_ranges(&mut d, &ranges).is_err());
    assert_eq!(d.modified_objects(), before);
}

#[test]
fn page_label_limits_reject_cyclic_old_trees_without_rewriting() {
    let mut d = doc_a();
    let tree = d.add(Object::Null);
    let mut dict = Dict::new();
    dict.set(b"Kids".to_vec(), Object::Array(vec![Object::Ref(tree)]));
    d.set(tree, Object::Dict(dict));
    d.update_dict(d.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree))).unwrap();
    let before = d.modified_objects();
    assert!(crate::page_labels(&d).is_err());
    assert!(crate::page_label_ranges(&d).is_empty(), "compatibility query has documented empty fallback");
    assert!(crate::number_pages(&mut d, 0, 1, crate::LabelStyle::Decimal, "", 1).is_err());
    assert_eq!(d.modified_objects(), before);
    assert_eq!(d.get(d.root().unwrap()).as_dict().unwrap().reference(b"PageLabels"), Some(tree));
}

#[test]
fn page_label_limits_reject_oversized_old_arrays_without_rewriting() {
    let mut d = doc_a();
    let mut dict = Dict::new();
    dict.set(b"Kids".to_vec(), Object::Array(vec![Object::Null; pdfcraft_cos::page_labels::MAX_LABEL_TREE_WORK + 1]));
    let tree = d.add(Object::Dict(dict));
    d.update_dict(d.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree))).unwrap();
    let before = d.modified_objects();
    assert!(crate::page_labels(&d).is_err());
    assert!(crate::number_pages(&mut d, 0, 1, crate::LabelStyle::Decimal, "", 1).is_err());
    assert_eq!(d.modified_objects(), before);
}

#[test]
fn page_label_limits_keep_preview_fallback_and_valid_boundary_edits() {
    use crate::LabelStyle::*;
    assert_eq!(LowerRoman.format(u32::MAX), u32::MAX.to_string());
    assert_eq!(LowerAlpha.format(u32::MAX), u32::MAX.to_string());
    let mut d = doc_a();
    crate::number_pages(&mut d, 0, 2, None, &"x".repeat(1024), 1).unwrap();
    assert!(crate::page_labels(&d).unwrap().iter().all(|s| s.len() == 1024));
}

#[test]
fn page_label_limits_preserve_prefix_only_max_first_and_aliases() {
    let mut d = doc_a();
    crate::number_pages(&mut d, 0, 2, crate::LabelStyle::None, "prefix", u32::MAX).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap(), ["prefix", "prefix", "prefix"]);
    crate::number_pages(&mut d, 0, 0, crate::LabelStyle::Decimal, "", 1).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap(), ["1", "prefix", "prefix"]);

    let prefix = d.add(Object::String(PdfString::text("alias")));
    let prefix_alias = d.add(Object::Ref(prefix));
    let mut spec = Dict::new();
    spec.set(b"P".to_vec(), Object::Ref(prefix_alias));
    spec.set(b"St".to_vec(), Object::Int(i64::MAX)); // unused without /S
    let spec = d.add(Object::Dict(spec));
    let spec_alias = d.add(Object::Ref(spec));
    let nums = d.add(Object::Array(vec![Object::Int(0), Object::Ref(spec_alias)]));
    let nums_alias = d.add(Object::Ref(nums));
    let mut tree = Dict::new();
    tree.set(b"Nums".to_vec(), Object::Ref(nums_alias));
    let tree = d.add(Object::Dict(tree));
    let tree_alias = d.add(Object::Ref(tree));
    d.update_dict(d.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree_alias))).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap(), ["alias", "alias", "alias"]);
    crate::number_pages(&mut d, 0, 0, crate::LabelStyle::Decimal, "", 1).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap(), ["1", "alias", "alias"]);
}

#[test]
fn page_label_limits_refuse_reference_only_cycles_without_rewriting() {
    let mut d = doc_a();
    let tree = d.add(Object::Null);
    d.set(tree, Object::Ref(tree));
    d.update_dict(d.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree))).unwrap();
    let before = d.modified_objects();
    assert!(crate::page_labels(&d).is_err());
    assert!(crate::number_pages(&mut d, 0, 1, crate::LabelStyle::Decimal, "", 1).is_err());
    assert_eq!(d.modified_objects(), before);
}

#[test]
fn page_label_limits_validate_the_same_ranges_that_are_written() {
    use crate::{LabelRange, LabelStyle};
    let mut d = doc_a();
    let duplicates = [
        LabelRange { start: 0, style: LabelStyle::None, prefix: "ok".into(), first: 1 },
        LabelRange { start: 0, style: LabelStyle::None, prefix: "x".repeat(1024), first: 1 },
    ];
    let before = d.modified_objects();
    assert!(crate::set_page_label_ranges(&mut d, &duplicates).unwrap_err().to_string().contains("unique"));
    assert_eq!(d.modified_objects(), before);
    let unsorted = [
        LabelRange { start: 1, style: LabelStyle::UpperAlpha, prefix: "".into(), first: 1 },
        LabelRange { start: 0, style: LabelStyle::LowerRoman, prefix: "".into(), first: 4 },
    ];
    crate::set_page_label_ranges(&mut d, &unsorted).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap(), ["iv", "A", "B"]);
    let catalog = d.get(d.root().unwrap());
    let tree = d.resolve(catalog.as_dict().unwrap().get(b"PageLabels").unwrap());
    let nums = tree.as_dict().unwrap().get(b"Nums").unwrap().as_array().unwrap();
    assert_eq!(nums[0].as_int(), Some(0));
    assert_eq!(nums[2].as_int(), Some(1));
}

/// Each label dictionary is its own object, referenced from `/Nums`, as Acrobat writes them. A
/// direct dictionary is just as valid, but some readers (pdfannots2json, through unipdf) skip it
/// and fall back to physical page numbers (#1763).
#[test]
fn page_labels_are_written_as_indirect_dictionaries() {
    use crate::{LabelRange, LabelStyle};
    let mut d = doc_a();
    let ranges = [
        LabelRange { start: 0, style: LabelStyle::LowerRoman, prefix: String::new(), first: 1 },
        LabelRange { start: 1, style: LabelStyle::Decimal, prefix: "A-".into(), first: 324 },
        LabelRange { start: 2, style: LabelStyle::None, prefix: "Cover".into(), first: 1 },
    ];
    crate::set_page_label_ranges(&mut d, &ranges).unwrap();
    let back = full_roundtrip(&d);
    let catalog = back.get(back.root().unwrap());
    let tree = back.resolve(catalog.as_dict().unwrap().get(b"PageLabels").unwrap());
    let nums = tree.as_dict().unwrap().get(b"Nums").unwrap().as_array().unwrap();
    assert_eq!(nums.len(), 6);
    let expected = [(0, Some(b"r".as_slice()), None, None), (1, Some(b"D".as_slice()), Some("A-"), Some(324)), (2, None, Some("Cover"), None)];
    for (pair, (start, style, prefix, first)) in nums.chunks(2).zip(expected) {
        assert_eq!(pair[0].as_int(), Some(start));
        let Object::Ref(r) = &pair[1] else { panic!("the label of page {start} is not an indirect object: {:?}", pair[1]) };
        let spec = back.get(*r);
        let spec = spec.as_dict().unwrap();
        assert_eq!(spec.name(b"S"), style);
        assert_eq!(spec.get(b"P").and_then(|p| p.as_string()).map(|p| p.to_text()).as_deref(), prefix);
        assert_eq!(spec.int(b"St"), first);
    }
    assert_eq!(crate::page_label_ranges(&back), ranges);
    assert_eq!(crate::page_labels(&back).unwrap(), ["i", "A-324", "Cover"]);
}

#[test]
fn page_label_duplicate_starts_keep_the_later_range() {
    let mut d = doc_a();
    let mut a = Dict::new();
    a.set(b"P".to_vec(), Object::String(PdfString::text("first")));
    let mut b = Dict::new();
    b.set(b"P".to_vec(), Object::String(PdfString::text("last")));
    let mut tree = Dict::new();
    tree.set(b"Nums".to_vec(), Object::Array(vec![Object::Int(0), Object::Dict(a), Object::Int(0), Object::Dict(b)]));
    let tree = d.add(Object::Dict(tree));
    d.update_dict(d.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree))).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap()[0], "last");
    crate::number_pages(&mut d, 0, 0, crate::LabelStyle::Decimal, "", 1).unwrap();
}

/// Two kids naming objects the file doesn't have both resolve to a fresh `Null`; they are not
/// the same node, so the tree isn't refused as cyclic.
#[test]
fn page_label_missing_kids_are_not_a_cycle() {
    let mut d = doc_a();
    let mut leaf = Dict::new();
    let mut spec = Dict::new();
    spec.set(b"P".to_vec(), Object::String(PdfString::text("p-")));
    spec.set(b"S".to_vec(), Object::name("D"));
    leaf.set(b"Nums".to_vec(), Object::Array(vec![Object::Int(0), Object::Dict(spec)]));
    let leaf = d.add(Object::Dict(leaf));
    let mut tree = Dict::new();
    let missing = |n| Object::Ref(pdfcraft_cos::ObjRef { num: n, generation: 0 });
    tree.set(b"Kids".to_vec(), Object::Array(vec![missing(9_000), missing(9_001), Object::Ref(leaf)]));
    let tree = d.add(Object::Dict(tree));
    d.update_dict(d.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree))).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap()[0], "p-1");
}

/// A range whose labels would be too long shows physical page numbers; other ranges keep
/// their labels.
#[test]
fn page_label_too_long_range_falls_back_alone() {
    let mut d = doc_a();
    let mut ok = Dict::new();
    ok.set(b"P".to_vec(), Object::String(PdfString::text("ok-")));
    ok.set(b"S".to_vec(), Object::name("D"));
    let mut huge = Dict::new();
    huge.set(b"S".to_vec(), Object::name("r"));
    huge.set(b"St".to_vec(), Object::Int(1_025_000));
    let mut tree = Dict::new();
    tree.set(b"Nums".to_vec(), Object::Array(vec![Object::Int(0), Object::Dict(ok), Object::Int(1), Object::Dict(huge)]));
    let tree = d.add(Object::Dict(tree));
    d.update_dict(d.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree))).unwrap();
    let labels = crate::page_labels(&d).unwrap();
    assert_eq!(labels[0], "ok-1");
    assert_eq!(labels[1], "2");
}

/// A document whose page tree is one flat node over `pages` leaves. The label tests below need
/// more pages than `doc_a` has.
fn flat_doc(pages: usize) -> Document {
    let kids = (0..pages).map(|i| format!("{} 0 R", i + 3)).collect::<Vec<_>>().join(" ");
    let mut objs = vec!["<< /Type /Catalog /Pages 2 0 R >>".to_string(), format!("<< /Type /Pages /Kids [{kids}] /Count {pages} >>")];
    objs.extend((0..pages).map(|_| "<< /Type /Page /Parent 2 0 R >>".to_string()));
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    open(out)
}

/// Each label is valid (1,004 bytes), but the 5,000 of them total about 5 MB, past the old 4 MiB
/// limit on the whole sequence. Every label is readable.
#[test]
fn page_labels_past_four_mib_in_total_stay_readable() {
    let mut d = flat_doc(5_000);
    let mut spec = Dict::new();
    spec.set(b"P".to_vec(), Object::String(PdfString::text(&"x".repeat(1000))));
    spec.set(b"S".to_vec(), Object::name("D"));
    let mut tree = Dict::new();
    tree.set(b"Nums".to_vec(), Object::Array(vec![Object::Int(0), Object::Dict(spec)]));
    let tree = d.add(Object::Dict(tree));
    d.update_dict(d.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree))).unwrap();
    let labels = crate::page_labels(&d).unwrap();
    assert_eq!(labels.len(), 5_000);
    assert_eq!(labels[0], format!("{}1", "x".repeat(1000)));
    assert_eq!(labels[4_999], format!("{}5000", "x".repeat(1000)));
}

/// The same 5,000 labels written by an edit: the edit is accepted and reads back.
#[test]
fn number_pages_past_four_mib_in_total_is_accepted() {
    let mut d = flat_doc(5_000);
    crate::number_pages(&mut d, 0, 4_999, crate::LabelStyle::Decimal, &"x".repeat(1000), 1).unwrap();
    assert_eq!(crate::page_labels(&d).unwrap()[4_999], format!("{}5000", "x".repeat(1000)));
}

/// Each range keeps its own decoded prefix, so a small file sharing one prefix object asks for a
/// copy per range (about 200 MiB at the tree-work limit). Past 4 MiB of decoded prefixes in all
/// the tree is refused, and so is relabelling it, which changes nothing.
#[test]
fn page_label_prefixes_are_bounded_in_total() {
    // `ranges` ranges naming one shared prefix object of `prefix` bytes, without a style.
    let shared = |ranges: i64, prefix: usize| {
        let mut d = doc_a();
        let prefix = d.add(Object::String(PdfString::text(&"x".repeat(prefix))));
        let mut spec = Dict::new();
        spec.set(b"P".to_vec(), Object::Ref(prefix));
        let spec = d.add(Object::Dict(spec));
        let mut tree = Dict::new();
        tree.set(b"Nums".to_vec(), Object::Array((0..ranges).flat_map(|i| [Object::Int(i), Object::Ref(spec)]).collect()));
        let tree = d.add(Object::Dict(tree));
        d.update_dict(d.root().unwrap(), |c| c.set(b"PageLabels".to_vec(), Object::Ref(tree))).unwrap();
        d
    };
    let mut d = shared(5_000, 1_000);
    let e = crate::page_labels(&d).unwrap_err().to_string();
    assert!(e.contains("decoded prefixes exceed 4 MiB in total"), "{e}");
    let before = d.modified_objects();
    assert!(crate::number_pages(&mut d, 0, 1, crate::LabelStyle::Decimal, "", 1).is_err());
    assert_eq!(d.modified_objects(), before, "a refused relabelling changes nothing");
    // Exactly 4 MiB of prefixes is still read.
    let max = pdfcraft_cos::page_labels::MAX_PREFIX_TOTAL_BYTES / 1024;
    assert!(crate::page_labels(&shared(max as i64, 1024)).unwrap().iter().all(|l| l.len() == 1024));
}

#[test]
fn page_boxes_default_inherit_and_set() {
    let mut doc = open(fixture());
    let b = page_boxes(&doc).unwrap();
    // Pages 1–2 inherit a 300×400 media box; page 3 has its own 500×500. Crop defaults to media.
    assert_eq!(b[0][0], [0.0, 0.0, 300.0, 400.0]);
    assert_eq!(b[2][1], [0.0, 0.0, 500.0, 500.0]);
    set_page_box(&mut doc, &[0, 2], PageBox::Crop, BoxSpec::Margins([10.0, 20.0, 30.0, 40.0])).unwrap();
    set_page_box(&mut doc, &[1], PageBox::Trim, BoxSpec::Rect([-50.0, 50.0, 100.0, 1000.0])).unwrap();
    let doc = open(write_incremental(&doc, &SaveOptions::default()).unwrap());
    let b = page_boxes(&doc).unwrap();
    assert_eq!(b[0][1], [10.0, 20.0, 270.0, 360.0]);
    assert_eq!(b[2][1], [10.0, 20.0, 470.0, 460.0], "margins apply to each page's own media box");
    assert_eq!(b[1][3], [0.0, 50.0, 100.0, 400.0], "clipped to the media box");
    assert_eq!(b[1][4], b[1][1], "art box defaults to the crop box");
    // Removing goes back to the default; impossible requests change nothing.
    let mut doc = doc;
    set_page_box(&mut doc, &[0], PageBox::Crop, BoxSpec::Remove).unwrap();
    assert_eq!(page_boxes(&doc).unwrap()[0][1], [0.0, 0.0, 300.0, 400.0]);
    let before = page_boxes(&doc).unwrap();
    assert!(matches!(set_page_box(&mut doc, &[0, 1], PageBox::Crop, BoxSpec::Margins([200.0, 0.0, 200.0, 0.0])), Err(OrganizeError::InvalidBox(_))));
    assert!(matches!(set_page_box(&mut doc, &[0], PageBox::Media, BoxSpec::Remove), Err(OrganizeError::InvalidBox(_))));
    assert_eq!(page_boxes(&doc).unwrap(), before);
    assert_eq!(set_page_box(&mut doc, &[7], PageBox::Crop, BoxSpec::Remove), Err(OrganizeError::NoSuchPage(7)));
}

#[test]
fn page_box_report_says_which_boxes_the_file_sets() {
    let mut doc = open(fixture());
    let r = page_box_report(&doc).unwrap();
    assert_eq!(r.len(), 3);
    assert_eq!(r[0].boxes, page_boxes(&doc).unwrap()[0], "the effective rectangles are page_boxes'");
    assert_eq!(r[0].explicit, [true, false, false, false, false], "pages 1-2 inherit the media box and set nothing else");
    assert_eq!(r[0].ignored, [false; 5]);
    assert_eq!(r[0].rotate, 90, "the fixture inherits /Rotate 90 from the page-tree root");
    assert_eq!(r[0].rotate, page_rotation(&doc, 0).unwrap());
    set_page_box(&mut doc, &[0], PageBox::Trim, BoxSpec::Margins([10.0, 10.0, 10.0, 10.0])).unwrap();
    set_page_box(&mut doc, &[1], PageBox::Bleed, BoxSpec::Rect([0.0, 0.0, 50.0, 50.0])).unwrap();
    let doc = open(write_incremental(&doc, &SaveOptions::default()).unwrap());
    let r = page_box_report(&doc).unwrap();
    assert_eq!(r[0].explicit, [true, false, false, true, false]);
    assert_eq!(r[0].boxes[3], [10.0, 10.0, 290.0, 390.0]);
    assert_eq!(r[0].boxes[2], r[0].boxes[1], "an unset bleed box is the crop box");
    assert_eq!(r[1].explicit, [true, false, true, false, false]);
}

#[test]
fn page_box_report_covers_inheritance_reversed_corners_bad_entries_and_rotation() {
    let d = open(build(
        &[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 300 400] /CropBox [20 20 280 380] >>".into(),
            // Reversed corners, an indirect rectangle, a negative rotation.
            "<< /Type /Page /Parent 2 0 R /TrimBox [280 380 40 60] /BleedBox 6 0 R /Rotate -90 >>".into(),
            // A rectangle clipped away entirely, one that is not numbers, one with no area, a quarter turn plus 360 (450).
            "<< /Type /Page /Parent 2 0 R /TrimBox [400 400 500 500] /ArtBox [(a) 0 10 10] /BleedBox [5 5 5 50] /Rotate 450 >>".into(),
            // Its own media box makes the inherited crop box clip to nothing.
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Rotate 45 >>".into(),
            "[10 10 290 390]".into(),
        ],
        "/Root 1 0 R",
    ));
    let r = page_box_report(&d).unwrap();
    assert_eq!(r.len(), 3);
    // Page 1.
    assert_eq!(r[0].explicit, [true, true, true, true, false], "crop is set when inherited; an indirect rectangle counts");
    assert_eq!(r[0].boxes[1], [20.0, 20.0, 280.0, 380.0]);
    assert_eq!(r[0].boxes[3], [40.0, 60.0, 280.0, 380.0], "corners are put in order");
    assert_eq!(r[0].boxes[2], [10.0, 10.0, 290.0, 390.0]);
    assert_eq!(r[0].ignored, [false; 5]);
    assert_eq!(r[0].rotate, 270, "a negative rotation is turned into 0..360");
    // Page 2.
    assert_eq!(r[1].explicit, [true, true, false, false, false]);
    assert_eq!(r[1].ignored, [false, false, true, true, true], "a box clipped away, not numbers and no area are ignored");
    assert_eq!(r[1].boxes[3], r[1].boxes[1], "an ignored box defaults to the crop box");
    assert_eq!(r[1].rotate, 90);
    // Page 3: the inherited crop box lies outside its media box.
    assert_eq!(r[2].explicit, [true, false, false, false, false]);
    assert_eq!(r[2].ignored, [false, true, false, false, false]);
    assert_eq!(r[2].boxes[1], [0.0, 0.0, 10.0, 10.0]);
    assert_eq!(r[2].rotate, 45, "a rotation that is not a right angle is passed on as page_rotation gives it");
    for (i, page) in r.iter().enumerate() {
        assert_eq!(page.rotate, page_rotation(&d, i).unwrap());
    }
}

#[test]
fn a_box_is_its_first_four_entries_all_numbers_as_the_viewer_reads_it() {
    // A hostile file shares one huge array between every page's boxes. The viewer reads the first
    // four entries and stops; a non-number among them, or fewer than four entries, is no box.
    let long = format!("[0 0 300 400 {}]", "7 ".repeat(100_000));
    let d = open(build(
        &[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox 5 0 R >>".into(),
            // A fifth number is not read; a non-number first; a non-number third.
            "<< /Type /Page /Parent 2 0 R /CropBox [10 20 290 380 1] /TrimBox [(a) 0 30 40] /BleedBox [0 0 /X 300 400] >>".into(),
            // Fewer than four entries; a number after a non-number past position four.
            "<< /Type /Page /Parent 2 0 R /ArtBox [1 2 3] /TrimBox [0 0 30 40 (a)] >>".into(),
            long,
        ],
        "/Root 1 0 R",
    ));
    let r = page_box_report(&d).unwrap();
    assert_eq!(r[0].boxes[0], [0.0, 0.0, 300.0, 400.0], "the shared media box is its first four numbers");
    assert_eq!(r[0].explicit, [true, true, false, false, false]);
    assert_eq!(r[0].boxes[1], [10.0, 20.0, 290.0, 380.0], "a fifth number is not read");
    assert_eq!(r[0].ignored, [false, false, true, true, false], "a non-number among the first four is no box");
    assert_eq!((r[1].explicit[4], r[1].ignored[4]), (false, true), "fewer than four entries is no box");
    assert_eq!((r[1].explicit[3], r[1].ignored[3]), (true, false), "entries after the fourth are irrelevant");
    assert_eq!(page_boxes(&d).unwrap()[0][3], [10.0, 20.0, 290.0, 380.0], "an ignored trim box defaults to the crop box");
}

#[test]
fn page_boxes_remove_uninherited_crop_clip_disjoint_boxes_and_reject_overflow() {
    let mut doc = open(fixture());
    set_page_box(&mut doc, &[0], PageBox::Crop, BoxSpec::Margins([10.0, 20.0, 30.0, 40.0])).unwrap();
    set_page_box(&mut doc, &[0], PageBox::Crop, BoxSpec::Remove).unwrap();
    assert!(!page_dict(&doc, 0).contains(b"CropBox"), "removing an uninherited CropBox deletes the key instead of pinning the old MediaBox");
    set_page_box(&mut doc, &[0], PageBox::Media, BoxSpec::Rect([0.0, 0.0, 400.0, 500.0])).unwrap();
    assert_eq!(page_boxes(&doc).unwrap()[0][1], [0.0, 0.0, 400.0, 500.0], "CropBox tracks the resized MediaBox after removal");

    // When /Pages itself carries an inherited /CropBox, removing a page's CropBox pins the page's MediaBox
    // even when the page also had its own /CropBox and omitted /Parent.
    let mut d = open(build(
        &[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] /CropBox [20 20 280 380] >>".into(),
            "<< /Type /Page /CropBox [40 40 260 360] >>".into(),
        ],
        "/Root 1 0 R",
    ));
    assert_eq!(page_boxes(&d).unwrap()[0][1], [40.0, 40.0, 260.0, 360.0]);
    set_page_box(&mut d, &[0], PageBox::Crop, BoxSpec::Remove).unwrap();
    assert_eq!(page_boxes(&d).unwrap()[0][1], [0.0, 0.0, 300.0, 400.0]);

    // An inherited /CropBox that is currently disjoint from the page's /MediaBox still causes removal to pin the MediaBox.
    let mut d_disjoint_inh = open(build(
        &[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] /CropBox [350 350 450 450] >>".into(),
            "<< /Type /Page /CropBox [40 40 260 360] >>".into(),
        ],
        "/Root 1 0 R",
    ));
    set_page_box(&mut d_disjoint_inh, &[0], PageBox::Crop, BoxSpec::Remove).unwrap();
    set_page_box(&mut d_disjoint_inh, &[0], PageBox::Media, BoxSpec::Rect([0.0, 0.0, 500.0, 500.0])).unwrap();
    assert_eq!(page_boxes(&d_disjoint_inh).unwrap()[0][1], [0.0, 0.0, 300.0, 400.0]);

    // Disjoint or zero-area boxes in a file fall back to MediaBox/CropBox instead of producing inverted rectangles.
    let disjoint = open(build(
        &[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 300 400] >>".into(),
            "<< /Type /Page /Parent 2 0 R /CropBox [500 500 600 600] /TrimBox [700 700 800 800] >>".into(),
            "<< /Type /Page /Parent 2 0 R /CropBox [10 20 290 380] /TrimBox [500 500 600 600] /ArtBox [50 50 50 100] >>".into(),
        ],
        "/Root 1 0 R",
    ));
    let boxes = page_boxes(&disjoint).unwrap();
    assert_eq!(boxes[0][1], [0.0, 0.0, 300.0, 400.0], "disjoint CropBox falls back to MediaBox");
    assert_eq!(boxes[0][3], [0.0, 0.0, 300.0, 400.0], "disjoint TrimBox falls back to CropBox");
    assert_eq!(boxes[1][3], [10.0, 20.0, 290.0, 380.0], "disjoint TrimBox falls back to page's CropBox");
    assert_eq!(boxes[1][4], [10.0, 20.0, 290.0, 380.0], "zero-width ArtBox falls back to page's CropBox");

    // Finite coordinates whose span overflows f64 are rejected.
    assert!(matches!(set_page_box(&mut doc, &[0], PageBox::Media, BoxSpec::Rect([-1e308, 0.0, 1e308, 100.0])), Err(OrganizeError::InvalidBox(_))));
}

#[test]
fn duplicate_pages_inserts_copies_after_the_last() {
    let mut doc = open(fixture());
    duplicate_pages(&mut doc, &[0, 1], None).unwrap();
    let doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
    assert_eq!(labels(&doc), ["Page 1", "Page 2", "Page 1", "Page 2", "Page 3"]);
}

#[test]
fn duplicate_pages_at_inserts_the_copies_there() {
    let mut doc = open(fixture());
    duplicate_pages(&mut doc, &[2], Some(0)).unwrap();
    let doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
    assert_eq!(labels(&doc), ["Page 3", "Page 1", "Page 2", "Page 3"]);
    // An index past the end lands at the end.
    let mut doc = open(fixture());
    duplicate_pages(&mut doc, &[0], Some(99)).unwrap();
    let doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
    assert_eq!(labels(&doc), ["Page 1", "Page 2", "Page 3", "Page 1"]);
}

#[test]
fn replace_pages_swaps_content_but_keeps_annotations() {
    // Page 2 of the target gets an annotation; replacing its content keeps it.
    let mut doc = open(fixture());
    let p2 = pages(&doc).unwrap()[1].obj;
    let annot = doc.add(Object::Dict({
        let mut d = Dict::new();
        d.set(b"Type".to_vec(), Object::name("Annot"));
        d.set(b"Subtype".to_vec(), Object::name("Square"));
        d.set(b"Rect".to_vec(), Object::Array(vec![0.into(), 0.into(), 10.into(), 10.into()]));
        d
    }));
    doc.update_dict(p2, |d| d.set(b"Annots".to_vec(), Object::Array(vec![Object::Ref(annot)]))).unwrap();
    let src = open(fixture());
    replace_pages(&mut doc, &[1], &src, &[2]).unwrap();
    let doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
    assert_eq!(labels(&doc), ["Page 1", "Page 3", "Page 3"]);
    let p2 = doc.get(pages(&doc).unwrap()[1].obj);
    assert!(p2.as_dict().unwrap().contains(b"Annots"), "the original page's annotations stay");
    let mut doc = doc;
    assert!(replace_pages(&mut doc, &[0, 1], &src, &[0]).is_err(), "counts must match");
}

#[test]
fn a_page_becomes_a_form_xobject_upright() {
    let src = open(fixture());
    let mut dst = Document::new_empty();
    // Page 1 inherits a 300 × 400 media box, rotation 90 and its font resources.
    let (r, size) = crate::page_as_form(&mut dst, &src, 0).unwrap();
    assert_eq!(size, (400.0, 300.0), "displayed size of a quarter-turned page");
    let obj = dst.get(r);
    let Object::Stream(s) = &*obj else { panic!("a stream") };
    let m: Vec<f64> = s.dict.get(b"Matrix").unwrap().as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    assert_eq!(m, vec![0.0, -1.0, 1.0, 0.0, 0.0, 300.0]);
    assert!(String::from_utf8_lossy(&s.decoded().unwrap()).contains("(Page 1) Tj"));
    let res = s.dict.get(b"Resources").and_then(|r| dst.resolve(r).as_dict().cloned()).unwrap();
    assert!(res.get(b"Font").is_some(), "inherited resources come along");
    assert!(crate::page_as_form(&mut dst, &src, 7).is_err());
}

#[test]
fn initial_view_round_trips_and_keeps_scripts() {
    use crate::view::{Layout, Magnification, Navigation};
    let mut doc = open(fixture());
    assert_eq!(crate::initial_view(&doc), crate::InitialView::default());
    let v = crate::InitialView {
        navigation: Navigation::Bookmarks,
        layout: Layout::TwoUpContinuousCoverPage,
        magnification: Magnification::Percent(150.0),
        page: 2,
        fit_window: true,
        display_title: true,
        hide_toolbar: true,
        language: Some("fr-FR".into()),
        right_to_left: true,
        ..Default::default()
    };
    crate::set_initial_view(&mut doc, &v).unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(crate::initial_view(&doc), v);
    // A script opening action survives a change that needs no destination.
    let mut doc = doc;
    let root = doc.root().unwrap();
    let mut js = pdfcraft_cos::Dict::new();
    js.set(b"S".to_vec(), Object::name("JavaScript"));
    js.set(b"JS".to_vec(), pdfcraft_cos::PdfString::text("app.alert('hi')"));
    doc.update_dict(root, |c| c.set(b"OpenAction".to_vec(), Object::Dict(js))).unwrap();
    crate::set_initial_view(&mut doc, &crate::InitialView { layout: Layout::SinglePage, ..Default::default() }).unwrap();
    assert!(doc.get(root).as_dict().unwrap().get(b"OpenAction").and_then(|o| o.as_dict()).is_some_and(|d| d.name(b"S") == Some(b"JavaScript")));
    assert!(crate::set_initial_view(&mut doc, &crate::InitialView { page: 9, ..Default::default() }).is_err());
}

#[test]
fn an_unchanged_opening_destination_keeps_its_position_and_name() {
    use crate::view::{Magnification, Navigation};
    let page2 = Object::Ref(pdfcraft_cos::ObjRef::new(5, 0));
    let xyz = Object::Array(vec![page2.clone(), Object::name("XYZ"), Object::Int(100), Object::Int(500), Object::Int(2)]);
    let mut named = pdfcraft_cos::Dict::new();
    named.set(b"S".to_vec(), Object::name("GoTo"));
    named.set(b"D".to_vec(), Object::name("there"));
    let mut dests = pdfcraft_cos::Dict::new();
    dests.set(b"there".to_vec(), Object::Array(vec![page2, Object::name("Fit")]));
    let mut prefs = pdfcraft_cos::Dict::new();
    prefs.set(b"NonFullScreenPageMode".to_vec(), Object::name("UseOutlines"));
    for action in [xyz, Object::Dict(named)] {
        let mut doc = open(fixture());
        let root = doc.root().unwrap();
        doc.update_dict(root, |c| {
            c.set(b"OpenAction".to_vec(), action.clone());
            c.set(b"Dests".to_vec(), Object::Dict(dests.clone()));
            c.set(b"PageMode".to_vec(), Object::name("FullScreen"));
            c.set(b"ViewerPreferences".to_vec(), Object::Dict(prefs.clone()));
        })
        .unwrap();
        // Another option changes: the opening action is written back as it was, and so is the
        // full-screen panel, which the dialog doesn't edit.
        let v = crate::initial_view(&doc);
        assert_eq!(v.non_full_screen, Navigation::Bookmarks);
        crate::set_initial_view(&mut doc, &crate::InitialView { hide_toolbar: true, ..v.clone() }).unwrap();
        let catalog = doc.get(root).as_dict().cloned().unwrap();
        assert_eq!(catalog.get(b"OpenAction"), Some(&action));
        let doc = save_and_reopen(&doc);
        assert_eq!(crate::initial_view(&doc), crate::InitialView { hide_toolbar: true, ..v.clone() });
        // A changed magnification writes a destination of its own.
        let mut doc = doc;
        crate::set_initial_view(&mut doc, &crate::InitialView { magnification: Magnification::FitWidth, ..v }).unwrap();
        let catalog = doc.get(root).as_dict().cloned().unwrap();
        assert_ne!(catalog.get(b"OpenAction"), Some(&action));
        assert_eq!(crate::initial_view(&doc).magnification, Magnification::FitWidth);
    }
}

/// A percentage is written as the `/XYZ` zoom factor (ISO 32000-2 Table 149), not the percentage:
/// 25% as 0.25 and 100% as 1, and each survives a save.
#[test]
fn initial_view_percentages_are_written_as_zoom_factors() {
    use crate::view::Magnification;
    for (percent, factor) in [(25.0, 0.25), (100.0, 1.0), (150.0, 1.5)] {
        let mut doc = open(fixture());
        let v = crate::initial_view(&doc);
        crate::set_initial_view(&mut doc, &crate::InitialView { magnification: Magnification::Percent(percent), ..v }).unwrap();
        let doc = save_and_reopen(&doc);
        let root = doc.root().unwrap();
        let open_action = doc.get(root).as_dict().and_then(|c| c.get(b"OpenAction").cloned()).map(|o| doc.resolve(&o).as_ref().clone());
        let zoom = open_action.as_ref().and_then(Object::as_array).and_then(|a| a.get(4)).and_then(Object::as_f64);
        assert_eq!(zoom, Some(factor), "{percent}%: {open_action:?}");
    }
}

#[test]
fn choosing_100_percent_keeps_an_actual_size_opening_destination() {
    use crate::view::Magnification;
    let page2 = Object::Ref(pdfcraft_cos::ObjRef::new(5, 0));
    let xyz = Object::Array(vec![page2.clone(), Object::name("XYZ"), Object::Int(100), Object::Int(500), Object::Int(1)]);
    let mut named = pdfcraft_cos::Dict::new();
    named.set(b"S".to_vec(), Object::name("GoTo"));
    named.set(b"D".to_vec(), Object::name("there"));
    let mut dests = pdfcraft_cos::Dict::new();
    dests.set(b"there".to_vec(), Object::Array(vec![page2, Object::name("XYZ"), Object::Null, Object::Null, Object::Int(1)]));
    for action in [xyz, Object::Dict(named)] {
        let mut doc = open(fixture());
        let root = doc.root().unwrap();
        doc.update_dict(root, |c| {
            c.set(b"OpenAction".to_vec(), action.clone());
            c.set(b"Dests".to_vec(), Object::Dict(dests.clone()));
        })
        .unwrap();
        let v = crate::initial_view(&doc);
        assert_eq!(v.magnification, Magnification::ActualSize);
        // 100% is the same zoom: the action keeps its position or name.
        crate::set_initial_view(&mut doc, &crate::InitialView { magnification: Magnification::Percent(100.0), ..v }).unwrap();
        let catalog = doc.get(root).as_dict().cloned().unwrap();
        assert_eq!(catalog.get(b"OpenAction"), Some(&action));
    }
    // Without one, 100% writes zoom 1, which reads back as Actual Size.
    let mut doc = open(fixture());
    let v = crate::initial_view(&doc);
    crate::set_initial_view(&mut doc, &crate::InitialView { magnification: Magnification::Percent(100.0), ..v }).unwrap();
    let doc = save_and_reopen(&doc);
    assert_eq!(crate::initial_view(&doc).magnification, Magnification::ActualSize);
}

/// A document whose `/OpenAction` is `action`, with the named destination `there` (page 3 at
/// 200%) in the catalog `/Dests` and `here` (page 1, fit) in a `/Names /Dests` tree.
fn doc_opening_at(action: Object) -> Document {
    let page1 = Object::Ref(pdfcraft_cos::ObjRef::new(4, 0));
    let page3 = Object::Ref(pdfcraft_cos::ObjRef::new(6, 0));
    let mut dests = pdfcraft_cos::Dict::new();
    dests.set(b"there".to_vec(), Object::Array(vec![page3, Object::name("XYZ"), Object::Null, Object::Null, Object::Int(2)]));
    let mut tree = pdfcraft_cos::Dict::new();
    tree.set(b"Names".to_vec(), Object::Array(vec![Object::String(PdfString::text("here")), Object::Array(vec![page1, Object::name("Fit")])]));
    let mut names = pdfcraft_cos::Dict::new();
    names.set(b"Dests".to_vec(), Object::Dict(tree));
    let mut doc = open(fixture());
    let root = doc.root().unwrap();
    doc.update_dict(root, |c| {
        c.set(b"OpenAction".to_vec(), action);
        c.set(b"Dests".to_vec(), Object::Dict(dests));
        c.set(b"Names".to_vec(), Object::Dict(names));
    })
    .unwrap();
    doc
}

fn open_action(doc: &Document) -> Option<Object> {
    doc.get(doc.root().unwrap()).as_dict().and_then(|c| c.get(b"OpenAction").cloned())
}

/// ISO 32000-2 §12.3.2.4: a named destination stands for the explicit one it names. Choosing
/// page 1 in Document Properties on a document that opens at a named destination elsewhere
/// writes a destination to page 1; it used to read the name as page 1 and change nothing.
#[test]
fn choosing_page_one_replaces_a_named_opening_destination() {
    use crate::view::Magnification;
    let page1 = Object::Ref(pdfcraft_cos::ObjRef::new(4, 0));
    let mut goto = pdfcraft_cos::Dict::new();
    goto.set(b"S".to_vec(), Object::name("GoTo"));
    goto.set(b"D".to_vec(), Object::name("there"));
    for action in [Object::Dict(goto), Object::name("there"), Object::String(PdfString::text("there"))] {
        let mut doc = doc_opening_at(action.clone());
        // The dialog shows where the document really opens.
        let v = crate::initial_view(&doc);
        assert_eq!((v.page, v.magnification), (2, Magnification::Percent(200.0)), "{action:?}");
        // Page 1 at the same magnification: an explicit destination to page 1.
        crate::set_initial_view(&mut doc, &crate::InitialView { page: 0, ..v.clone() }).unwrap();
        let written = Object::Array(vec![page1.clone(), Object::name("XYZ"), Object::Null, Object::Null, Object::Real(2.0)]);
        assert_eq!(open_action(&doc), Some(written), "{action:?}");
        let reopened = save_and_reopen(&doc);
        assert_eq!(crate::initial_view(&reopened), crate::InitialView { page: 0, ..v.clone() }, "{action:?}");
        // Page 1 at the default magnification: no opening action, so the first page opens.
        let mut doc = doc_opening_at(action.clone());
        crate::set_initial_view(&mut doc, &crate::InitialView { page: 0, magnification: Magnification::Default, ..v }).unwrap();
        assert_eq!(open_action(&doc), None, "{action:?}");
        assert_eq!(crate::initial_view(&save_and_reopen(&doc)).page, 0, "{action:?}");
    }
}

/// A named opening destination that already goes to the chosen page and magnification is kept,
/// and so is one that names nothing (it reads as page 1) until another page is chosen.
#[test]
fn a_named_opening_destination_is_kept_while_it_still_holds() {
    use crate::view::Magnification;
    let page2 = Object::Ref(pdfcraft_cos::ObjRef::new(5, 0));
    for (action, magnification) in
        [(Object::String(PdfString::text("here")), Magnification::FitPage), (Object::name("nowhere"), Magnification::Default)]
    {
        let mut doc = doc_opening_at(action.clone());
        let v = crate::initial_view(&doc);
        assert_eq!((v.page, v.magnification), (0, magnification), "{action:?}");
        crate::set_initial_view(&mut doc, &crate::InitialView { hide_toolbar: true, ..v.clone() }).unwrap();
        assert_eq!(open_action(&doc), Some(action.clone()), "{action:?}");
        crate::set_initial_view(&mut doc, &crate::InitialView { page: 1, ..v.clone() }).unwrap();
        let kind = if magnification == Magnification::FitPage { "Fit" } else { "XYZ" };
        assert_eq!(
            open_action(&doc).as_ref().and_then(Object::as_array).map(|a| (a.first().cloned(), a.get(1).cloned())),
            Some((Some(page2.clone()), Some(Object::name(kind)))),
            "{action:?}"
        );
    }
}

// ── Split / Extract keep only the resources the part draws (#204) ──────────────────────────────

/// A content stream with no text operators (an image draw).
fn raw(content: &str) -> String {
    format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len())
}

fn image(width: u32, height: u32, payload: usize) -> String {
    let data = "x".repeat(payload);
    format!(
        "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceGray /BitsPerComponent 8 /Length {payload} >>\nstream\n{data}\nendstream"
    )
}

/// Document D: two pages sharing one resources dictionary with two image XObjects; each page
/// draws a different one — the shared-dictionary shape of #204.
fn doc_images() -> Document {
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),                                                        // 1
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] /Resources 7 0 R >>".into(), // 2
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>".into(),                                          // 3
        "<< /Type /Page /Parent 2 0 R /Contents 6 0 R >>".into(),                                          // 4
        raw("/ImA Do"),                                                                                    // 5
        raw("/ImB Do"),                                                                                    // 6
        "<< /XObject << /ImA 8 0 R /ImB 9 0 R >> >>".into(),                                               // 7
        image(1, 1, 1),                                                                                    // 8
        image(1, 1, 2000),                                                                                 // 9
    ];
    open(build(&b, "/Root 1 0 R"))
}

fn xobject_names(doc: &Document, page: usize) -> Vec<Vec<u8>> {
    let resources = page_dict(doc, page).get(b"Resources").map(|r| doc.resolve(r)).and_then(|r| r.as_dict().cloned()).expect("page resources");
    let xobjects = resources.get(b"XObject").map(|x| doc.resolve(x)).and_then(|x| x.as_dict().cloned()).expect("xobject dict");
    xobjects.iter().map(|(k, _)| k.clone()).collect()
}

#[test]
fn extract_keeps_only_the_images_the_part_draws() {
    let src = doc_images();
    let whole = write_full(&src, &SaveOptions::default()).expect("writes").len();
    let part = full_roundtrip(&extract_pages(&src, &[0]).unwrap());
    let names = xobject_names(&part, 0);
    assert!(names.iter().any(|n| n.as_slice() == b"ImA"), "the drawn image stays");
    assert!(!names.iter().any(|n| n.as_slice() == b"ImB"), "an image no page of the part draws must go");
    let part_bytes = write_full(&part, &SaveOptions::default()).expect("writes").len();
    assert!(part_bytes + 1000 < whole, "the part must not carry the other page's image: {part_bytes} vs {whole}");
}

#[test]
fn extract_keeps_the_union_when_the_part_has_both_pages() {
    let part = full_roundtrip(&extract_pages(&doc_images(), &[0, 1]).unwrap());
    let names = xobject_names(&part, 0);
    assert!(names.iter().any(|n| n.as_slice() == b"ImA"));
    assert!(names.iter().any(|n| n.as_slice() == b"ImB"));
}

/// Like `doc_images`, but the second page's content can't be decoded and the first page also
/// lists a font: pruning must not guess what the unreadable page draws.
fn doc_images_with(page2_content: &str, font: &str) -> Document {
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] /Resources 7 0 R >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 6 0 R >>".into(),
        raw("/ImA Do"),
        page2_content.into(),
        format!("<< /XObject << /ImA 8 0 R /ImB 9 0 R >> {font} >>"),
        image(1, 1, 1),
        image(1, 1, 2000),
    ];
    open(build(&b, "/Root 1 0 R"))
}

#[test]
fn extract_keeps_images_an_undecodable_page_might_draw() {
    let bogus = "<< /Length 7 /Filter /NoSuchFilter >>\nstream\n/ImB Do\nendstream";
    let part = full_roundtrip(&extract_pages(&doc_images_with(bogus, ""), &[0, 1]).unwrap());
    let names = xobject_names(&part, 0);
    assert!(names.iter().any(|n| n.as_slice() == b"ImB"), "an image the unreadable page may draw must stay");
}

/// zlib data that reads as `readable` and is then damaged, with more of the stream after the
/// damage: a stored block, then a block of the reserved type (RFC 1951 §3.2.3), then `rest`.
fn damaged_flate(readable: &[u8], rest: &[u8]) -> pdfcraft_cos::Stream {
    let len = u16::try_from(readable.len()).unwrap();
    let mut raw = vec![0x78, 0x01, 0x00];
    raw.extend_from_slice(&len.to_le_bytes());
    raw.extend_from_slice(&(!len).to_le_bytes());
    raw.extend_from_slice(readable);
    raw.push(0x07);
    raw.extend_from_slice(rest);
    let mut dict = Dict::new();
    dict.set(b"Filter".to_vec(), Object::name("FlateDecode"));
    pdfcraft_cos::Stream::from_raw(dict, raw)
}

/// A page or form damaged partway may draw images past the damage, which other readers may still
/// read: pruning keeps them, as it does for content that doesn't decode.
#[test]
fn extract_keeps_images_content_damaged_partway_might_draw() {
    use pdfcraft_cos::ObjRef;
    let mut page = doc_images_with(&raw("/ImA Do"), "");
    page.set(ObjRef::new(6, 0), damaged_flate(b"/ImA Do\n", b"/ImB Do"));

    let mut form = doc_images_with(&raw("/Fm Do"), "");
    let mut fm = damaged_flate(b"/ImA Do\n", b"/ImB Do");
    fm.dict.set(b"Subtype".to_vec(), Object::name("Form"));
    fm.dict.set(b"BBox".to_vec(), Object::Array(vec![0.into(), 0.into(), 1.into(), 1.into()]));
    let fm = form.add(fm);
    let xobjects: Dict =
        [(b"ImA".to_vec(), Object::Ref(ObjRef::new(8, 0))), (b"ImB".to_vec(), Object::Ref(ObjRef::new(9, 0))), (b"Fm".to_vec(), Object::Ref(fm))]
            .into_iter()
            .collect();
    form.update_dict(ObjRef::new(7, 0), |d| d.set(b"XObject".to_vec(), Object::Dict(xobjects))).unwrap();

    for (src, what) in [(page, "page"), (form, "form")] {
        let part = full_roundtrip(&extract_pages(&src, &[0, 1]).unwrap());
        let names = xobject_names(&part, 0);
        assert!(names.iter().any(|n| n.as_slice() == b"ImB"), "{what}: an image drawn past the damage must stay: {names:?}");
    }
}

/// Two pages: the first draws `/Fm` through resources of its own, `/Fm` draws `/Nested` through
/// resources of its own too, and `/Nested` draws `/ImA` through resources it shares with the second
/// page, which draws `/ImB` through them. The three streams are the first page's content, `/Fm`
/// and `/Nested`.
fn doc_shared_form_resources(page: pdfcraft_cos::Stream, fm: pdfcraft_cos::Stream, nested: pdfcraft_cos::Stream) -> Document {
    use pdfcraft_cos::ObjRef;
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Resources << /XObject << /Fm 7 0 R >> >> >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 6 0 R /Resources 8 0 R >>".into(),
        raw(""),
        raw("/ImB Do"),
        raw(""),
        "<< /XObject << /ImA 9 0 R /ImB 10 0 R >> >>".into(),
        image(1, 1, 1),
        image(1, 1, 2000),
        raw(""),
    ];
    let mut doc = open(build(&b, "/Root 1 0 R"));
    doc.set(ObjRef::new(5, 0), page);
    let nested_res: Dict =
        [(b"XObject".to_vec(), Object::Dict([(b"Nested".to_vec(), Object::Ref(ObjRef::new(11, 0)))].into_iter().collect()))].into_iter().collect();
    for (num, mut form, resources) in [(7, fm, Object::Dict(nested_res)), (11, nested, Object::Ref(ObjRef::new(8, 0)))] {
        form.dict.set(b"Subtype".to_vec(), Object::name("Form"));
        form.dict.set(b"BBox".to_vec(), Object::Array(vec![0.into(), 0.into(), 1.into(), 1.into()]));
        form.dict.set(b"Resources".to_vec(), resources);
        doc.set(ObjRef::new(num, 0), form);
    }
    doc
}

/// Every form the resources of a page or form damaged partway name may be drawn, before the
/// damage or past it, so each is read: one whose resources another page shares keeps what it draws
/// from them. So is each one content that doesn't decode may draw.
#[test]
fn extract_keeps_images_forms_drawn_around_damage_draw_through_shared_resources() {
    let whole = |content: &[u8]| pdfcraft_cos::Stream::from_raw(Dict::new(), content.to_vec());
    let undecodable = |content: &[u8]| {
        let mut s = whole(content);
        s.dict.set(b"Filter".to_vec(), Object::name("NoSuchFilter"));
        s
    };
    let cases = [
        ("page, before the damage", damaged_flate(b"/Fm Do\n", b"0 g"), whole(b"/Nested Do")),
        ("page, past the damage", damaged_flate(b"0 g\n", b"/Fm Do"), whole(b"/Nested Do")),
        ("page that doesn't decode", undecodable(b"/Fm Do"), whole(b"/Nested Do")),
        ("form, before the damage", whole(b"/Fm Do"), damaged_flate(b"/Nested Do\n", b"0 g")),
        ("form, past the damage", whole(b"/Fm Do"), damaged_flate(b"0 g\n", b"/Nested Do")),
        ("form that doesn't decode", whole(b"/Fm Do"), undecodable(b"/Nested Do")),
    ];
    for (what, page, fm) in cases {
        let part = full_roundtrip(&extract_pages(&doc_shared_form_resources(page, fm, whole(b"/ImA Do")), &[0, 1]).unwrap());
        let names = xobject_names(&part, 1);
        assert!(names.iter().any(|n| n.as_slice() == b"ImA"), "{what}: {names:?}");
        assert!(names.iter().any(|n| n.as_slice() == b"ImB"), "{what}: {names:?}");
    }
    // What nothing draws still goes.
    let src = doc_shared_form_resources(whole(b"0 g"), whole(b"/Nested Do"), whole(b"/ImA Do"));
    let part = full_roundtrip(&extract_pages(&src, &[0, 1]).unwrap());
    assert_eq!(xobject_names(&part, 1), [b"ImB".to_vec()]);
}

/// Forms are read 8 deep (`prune.rs`'s `MAX_FORM_DEPTH`). A damaged form that deep still has
/// every form it names read as drawn: here `/Nested`, which draws through resources another page
/// shares. Read 9 deep, it leaves the file unpruned, so what it draws from them stays.
#[test]
fn a_damaged_form_at_the_depth_limit_has_its_forms_read() {
    use pdfcraft_cos::ObjRef;
    let whole = |content: &[u8]| pdfcraft_cos::Stream::from_raw(Dict::new(), content.to_vec());
    let mut src = doc_shared_form_resources(whole(b"/C Do"), damaged_flate(b"0 g\n", b"/Nested Do"), whole(b"/ImA Do"));
    // Seven forms between the page and `/Fm`, each drawing the next as `/C`: `/Fm` is 8 deep.
    let mut next = ObjRef::new(7, 0);
    for _ in 1..8 {
        let mut form = whole(b"/C Do");
        form.dict.set(b"Subtype".to_vec(), Object::name("Form"));
        form.dict.set(b"BBox".to_vec(), Object::Array(vec![0.into(), 0.into(), 1.into(), 1.into()]));
        let names: Dict = [(b"C".to_vec(), Object::Ref(next))].into_iter().collect();
        form.dict.set(b"Resources".to_vec(), Object::Dict([(b"XObject".to_vec(), Object::Dict(names))].into_iter().collect()));
        next = src.add(Object::Stream(form));
    }
    let names: Dict = [(b"C".to_vec(), Object::Ref(next))].into_iter().collect();
    src.update_dict(ObjRef::new(3, 0), |p| {
        p.set(b"Resources".to_vec(), Object::Dict([(b"XObject".to_vec(), Object::Dict(names))].into_iter().collect()))
    })
    .unwrap();
    let part = full_roundtrip(&extract_pages(&src, &[0, 1]).unwrap());
    let names = xobject_names(&part, 1);
    assert!(names.iter().any(|n| n.as_slice() == b"ImA"), "{names:?}");
}

/// Past the depth limit a form isn't read, nor what the forms it draws draw: one of those may draw
/// through resources another page shares, and keep images there that page doesn't draw, so nothing
/// is pruned. Here `/Fm` is 9 deep, whole or damaged partway, and `/Nested`, which it draws, draws
/// `/ImA` through the resources the second page draws `/ImB` through.
#[test]
fn nothing_is_pruned_past_the_depth_limit() {
    use pdfcraft_cos::ObjRef;
    let whole = |content: &[u8]| pdfcraft_cos::Stream::from_raw(Dict::new(), content.to_vec());
    for (what, fm) in [("whole", whole(b"/Nested Do")), ("damaged", damaged_flate(b"0 g\n", b"/Nested Do"))] {
        let mut src = doc_shared_form_resources(whole(b"/C Do"), fm, whole(b"/ImA Do"));
        // Eight forms between the page and `/Fm`, each drawing the next as `/C`: `/Fm` is 9 deep.
        let mut next = ObjRef::new(7, 0);
        for _ in 0..8 {
            let mut form = whole(b"/C Do");
            form.dict.set(b"Subtype".to_vec(), Object::name("Form"));
            form.dict.set(b"BBox".to_vec(), Object::Array(vec![0.into(), 0.into(), 1.into(), 1.into()]));
            let names: Dict = [(b"C".to_vec(), Object::Ref(next))].into_iter().collect();
            form.dict.set(b"Resources".to_vec(), Object::Dict([(b"XObject".to_vec(), Object::Dict(names))].into_iter().collect()));
            next = src.add(Object::Stream(form));
        }
        let names: Dict = [(b"C".to_vec(), Object::Ref(next))].into_iter().collect();
        src.update_dict(ObjRef::new(3, 0), |p| {
            p.set(b"Resources".to_vec(), Object::Dict([(b"XObject".to_vec(), Object::Dict(names))].into_iter().collect()))
        })
        .unwrap();
        let part = full_roundtrip(&extract_pages(&src, &[0, 1]).unwrap());
        let names = xobject_names(&part, 1);
        assert!(names.iter().any(|n| n.as_slice() == b"ImA"), "{what}: {names:?}");
    }
}

/// Down to the depth limit pruning goes on: under a page whole or damaged partway, a form 8 deep,
/// whole or damaged, is read, and the image nothing draws, `/ImA`, still goes from the resources
/// the second page draws `/ImB` through.
#[test]
fn pruning_goes_on_down_to_the_depth_limit() {
    use pdfcraft_cos::ObjRef;
    let whole = |content: &[u8]| pdfcraft_cos::Stream::from_raw(Dict::new(), content.to_vec());
    let cases = [
        ("whole", whole(b"/C Do"), whole(b"0 g")),
        ("damaged form", whole(b"/C Do"), damaged_flate(b"0 g\n", b"0 g")),
        ("damaged page", damaged_flate(b"/C Do\n", b"0 g"), whole(b"0 g")),
    ];
    for (what, page, fm) in cases {
        let mut src = doc_shared_form_resources(page, fm, whole(b""));
        // `/Fm` names an image, not `/Nested`: nothing is read 9 deep.
        let image: Dict = [(b"Img".to_vec(), Object::Ref(ObjRef::new(9, 0)))].into_iter().collect();
        let resources = Object::Dict([(b"XObject".to_vec(), Object::Dict(image))].into_iter().collect());
        src.update_dict(ObjRef::new(7, 0), |f| f.set(b"Resources".to_vec(), resources)).unwrap();
        // Seven forms between the page and `/Fm`, each drawing the next as `/C`: `/Fm` is 8 deep.
        let mut next = ObjRef::new(7, 0);
        for _ in 1..8 {
            let mut form = whole(b"/C Do");
            form.dict.set(b"Subtype".to_vec(), Object::name("Form"));
            form.dict.set(b"BBox".to_vec(), Object::Array(vec![0.into(), 0.into(), 1.into(), 1.into()]));
            let names: Dict = [(b"C".to_vec(), Object::Ref(next))].into_iter().collect();
            form.dict.set(b"Resources".to_vec(), Object::Dict([(b"XObject".to_vec(), Object::Dict(names))].into_iter().collect()));
            next = src.add(Object::Stream(form));
        }
        let names: Dict = [(b"C".to_vec(), Object::Ref(next))].into_iter().collect();
        src.update_dict(ObjRef::new(3, 0), |p| {
            p.set(b"Resources".to_vec(), Object::Dict([(b"XObject".to_vec(), Object::Dict(names))].into_iter().collect()))
        })
        .unwrap();
        let part = full_roundtrip(&extract_pages(&src, &[0, 1]).unwrap());
        assert_eq!(xobject_names(&part, 1), [b"ImB".to_vec()], "{what}");
    }
}

/// A chain of forms nested far past the depth limit, whole or each damaged partway, is read only
/// down to it: the walk's depth stays bounded however long the chain is, and the run ends without
/// an error, having pruned nothing.
#[test]
fn a_long_chain_of_forms_is_read_only_to_the_depth_limit() {
    use pdfcraft_cos::ObjRef;
    let whole = |content: &[u8]| pdfcraft_cos::Stream::from_raw(Dict::new(), content.to_vec());
    let xobjects = |names: Dict| Object::Dict([(b"XObject".to_vec(), Object::Dict(names))].into_iter().collect());
    for damaged in [false, true] {
        let mut doc = doc_shared_form_resources(whole(b"/C Do"), whole(b""), whole(b""));
        let mut names = Dict::new();
        for _ in 0..100_000 {
            let mut form = if damaged { damaged_flate(b"", b"/C Do") } else { whole(b"/C Do") };
            form.dict.set(b"Subtype".to_vec(), Object::name("Form"));
            form.dict.set(b"BBox".to_vec(), Object::Array(vec![0.into(), 0.into(), 1.into(), 1.into()]));
            form.dict.set(b"Resources".to_vec(), xobjects(names));
            let next = doc.add(Object::Stream(form));
            names = [(b"C".to_vec(), Object::Ref(next))].into_iter().collect();
        }
        doc.update_dict(ObjRef::new(3, 0), |p| p.set(b"Resources".to_vec(), xobjects(names))).unwrap();
        assert!(matches!(crate::prune::prune_unused_xobjects(&mut doc), Ok(0)), "damaged: {damaged}");
    }
}

/// A form whose `/DecodeParms` is given by reference is read through it: the image it draws is
/// kept, not lost to content read with the predictor's tag bytes left in it.
#[test]
fn extract_keeps_images_a_form_with_parameters_by_reference_draws() {
    use pdfcraft_cos::ObjRef;
    let content = b"q 10 0 0 10 0 0 cm /ImB Do Q";
    // PNG Up, one column: each byte a row of its own, tagged 2.
    let mut rows = Vec::new();
    let mut above = 0u8;
    for &b in content {
        rows.extend([2, b.wrapping_sub(above)]);
        above = b;
    }
    let mut src = doc_images_with(&raw("/Fm Do"), "");
    let parms = src.add(Object::Dict([(b"Predictor".to_vec(), Object::Int(12))].into_iter().collect::<Dict>()));
    let mut fm = pdfcraft_cos::Stream::flate(Dict::new(), &rows);
    fm.dict.set(b"DecodeParms".to_vec(), Object::Ref(parms));
    fm.dict.set(b"Subtype".to_vec(), Object::name("Form"));
    fm.dict.set(b"BBox".to_vec(), Object::Array(vec![0.into(), 0.into(), 1.into(), 1.into()]));
    let fm = src.add(fm);
    let xobjects: Dict =
        [(b"ImA".to_vec(), Object::Ref(ObjRef::new(8, 0))), (b"ImB".to_vec(), Object::Ref(ObjRef::new(9, 0))), (b"Fm".to_vec(), Object::Ref(fm))]
            .into_iter()
            .collect();
    src.update_dict(ObjRef::new(7, 0), |d| d.set(b"XObject".to_vec(), Object::Dict(xobjects))).unwrap();
    let part = full_roundtrip(&extract_pages(&src, &[1]).unwrap());
    let names = xobject_names(&part, 0);
    assert!(names.iter().any(|n| n.as_slice() == b"ImB"), "{names:?}");
    assert!(!names.iter().any(|n| n.as_slice() == b"ImA"), "what the part doesn't draw still goes: {names:?}");
}

#[test]
fn extract_keeps_images_type3_glyphs_may_draw() {
    // A Type 3 font without its own /Resources draws through the page's: its glyphs could `Do`.
    let font = "/Font << /T3 << /Type /Font /Subtype /Type3 /FontBBox [0 0 1 1] /FontMatrix [1 0 0 1 0 0] /CharProcs << >> /Encoding << /Differences [] >> /FirstChar 0 /LastChar 0 /Widths [0] >> >>";
    let part = full_roundtrip(&extract_pages(&doc_images_with(&raw("/ImB Do"), font), &[0]).unwrap());
    let names = xobject_names(&part, 0);
    assert!(names.iter().any(|n| n.as_slice() == b"ImB"), "pruning must not run through Type 3 resources");
}

/// The font has its own resources, but its XObject dictionary is also used by the page.
/// Looking only at page `Do` operators must not discard the image painted by a glyph.
fn doc_type3_shared_xobjects(font_resources: &str) -> Document {
    let objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources 5 0 R >>".into(),
        raw("q 30 0 0 30 10 10 cm /ImA Do Q BT /T3 50 Tf 50 80 Td (A) Tj ET"),
        "<< /XObject 6 0 R /Font << /T3 7 0 R >> >>".into(),
        "<< /ImA 9 0 R /ImB 10 0 R /Unused 11 0 R >>".into(),
        format!(
            "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1 1] /FontMatrix [1 0 0 1 0 0] /CharProcs << /A 8 0 R >> /Encoding << /Type /Encoding /Differences [65 /A] >> /FirstChar 65 /LastChar 65 /Widths [1] /Resources {font_resources} >>"
        ),
        raw("1 0 0 0 1 1 d1 /ImB Do"),
        image(1, 1, 1),
        image(1, 1, 1),
        image(1, 1, 2000),
    ];
    open(build(&objects, "/Root 1 0 R"))
}

#[test]
fn extract_keeps_shared_xobjects_used_by_type3_own_resources() {
    for resources in ["<< /XObject 6 0 R >>", "5 0 R"] {
        let src = doc_type3_shared_xobjects(resources);
        let part = full_roundtrip(&extract_pages(&src, &[0]).unwrap());
        let names = xobject_names(&part, 0);
        assert!(names.iter().any(|n| n.as_slice() == b"ImB"), "the Type 3 glyph's image must survive extraction: {resources}");
    }
}

#[test]
fn extract_still_prunes_page_xobjects_when_type3_resources_are_separate() {
    let src = doc_type3_shared_xobjects("<< /XObject << /ImB 10 0 R >> >>");
    let part = full_roundtrip(&extract_pages(&src, &[0]).unwrap());
    assert_eq!(xobject_names(&part, 0), [b"ImA".to_vec()], "only the page's unused entries are removed");
    let resources = part.resolve(page_dict(&part, 0).get(b"Resources").unwrap());
    let fonts = part.resolve(resources.as_dict().unwrap().get(b"Font").unwrap());
    let font = part.resolve(fonts.as_dict().unwrap().get(b"T3").unwrap());
    let resources = part.resolve(font.as_dict().unwrap().get(b"Resources").unwrap());
    let xobjects = part.resolve(resources.as_dict().unwrap().get(b"XObject").unwrap());
    assert!(xobjects.as_dict().unwrap().contains(b"ImB"), "the separate glyph resource survives the full rewrite");
}

fn doc_type3_glyph_form_with_shared_resources(cycle: bool) -> Document {
    let mut doc = doc_type3_shared_xobjects("<< >>");
    let mut resources = pdfcraft_cos::Dict::new();
    resources.set(b"XObject".to_vec(), Object::Ref(ObjRef::new(6, 0)));
    let mut form = pdfcraft_cos::Dict::new();
    form.set(b"Type".to_vec(), Object::name("XObject"));
    form.set(b"Subtype".to_vec(), Object::name("Form"));
    form.set(b"BBox".to_vec(), Object::Array(vec![Object::Int(0), Object::Int(0), Object::Int(1), Object::Int(1)]));
    form.set(b"Resources".to_vec(), Object::Dict(resources));
    let form = doc.add(Object::Stream(pdfcraft_cos::Stream::from_raw(form, b"/ImB Do".to_vec())));
    let mut xobjects = pdfcraft_cos::Dict::new();
    xobjects.set(b"Fm".to_vec(), Object::Ref(form));
    let mut resources = pdfcraft_cos::Dict::new();
    resources.set(b"XObject".to_vec(), Object::Dict(xobjects));
    doc.update_dict(ObjRef::new(7, 0), |font| font.set(b"Resources".to_vec(), Object::Dict(resources))).unwrap();
    doc.set(ObjRef::new(8, 0), Object::Stream(pdfcraft_cos::Stream::from_raw(pdfcraft_cos::Dict::new(), b"1 0 0 0 1 1 d1 /Fm Do".to_vec())));
    if cycle {
        doc.update_dict(ObjRef::new(6, 0), |xobjects| xobjects.set(b"Again".to_vec(), Object::Ref(form))).unwrap();
    }
    full_roundtrip(&doc)
}

#[test]
fn extract_keeps_shared_resources_reached_through_a_type3_glyph_form() {
    for cycle in [false, true] {
        let src = doc_type3_glyph_form_with_shared_resources(cycle);
        let part = full_roundtrip(&extract_pages(&src, &[0]).unwrap());
        assert!(xobject_names(&part, 0).iter().any(|n| n.as_slice() == b"ImB"), "a glyph's nested form draws ImB; resource cycle: {cycle}");
    }
}

#[test]
fn extract_keeps_an_indirect_resource_owner_shared_with_a_type3_font() {
    let mut src = doc_type3_shared_xobjects("5 0 R");
    let xobjects = src.get(ObjRef::new(6, 0)).as_dict().unwrap().clone();
    src.update_dict(ObjRef::new(5, 0), |resources| resources.set(b"XObject".to_vec(), Object::Dict(xobjects))).unwrap();
    let src = full_roundtrip(&src);
    let part = full_roundtrip(&extract_pages(&src, &[0]).unwrap());
    assert!(xobject_names(&part, 0).iter().any(|n| n.as_slice() == b"ImB"), "the resource owner itself is shared, with inline XObjects");
}

#[test]
fn extract_skips_pruning_when_direct_glyph_resource_children_exceed_the_budget() {
    let mut src = doc_type3_shared_xobjects("<< /XObject << /ImB 10 0 R >> >>");
    let font = src.get(ObjRef::new(7, 0));
    let mut resources = font.as_dict().unwrap().get(b"Resources").unwrap().as_dict().unwrap().clone();
    let mut unknown = vec![Object::Null; 20_000];
    unknown.push(Object::Ref(ObjRef::new(6, 0)));
    resources.set(b"Unknown".to_vec(), Object::Array(unknown));
    src.update_dict(ObjRef::new(7, 0), |font| font.set(b"Resources".to_vec(), Object::Dict(resources))).unwrap();
    let src = full_roundtrip(&src);
    let part = full_roundtrip(&extract_pages(&src, &[0]).unwrap());
    assert_eq!(xobject_names(&part, 0), xobject_names(&src, 0), "an incomplete glyph resource walk cannot authorize pruning any owners");
}

fn doc_type3_resources_of_undecodable_content(form: bool) -> Document {
    let mut doc = doc_type3_shared_xobjects("<< /XObject 6 0 R >>");
    doc.update_dict(ObjRef::new(5, 0), |resources| {
        resources.remove(b"Font");
    })
    .unwrap();
    doc.set(ObjRef::new(4, 0), Object::Stream(pdfcraft_cos::Stream::from_raw(pdfcraft_cos::Dict::new(), b"/ImA Do".to_vec())));
    let mut fonts = pdfcraft_cos::Dict::new();
    fonts.set(b"T3".to_vec(), Object::Ref(ObjRef::new(7, 0)));
    let mut glyph_resources = pdfcraft_cos::Dict::new();
    glyph_resources.set(b"Font".to_vec(), Object::Dict(fonts));
    let mut unreadable = pdfcraft_cos::Dict::new();
    unreadable.set(b"Filter".to_vec(), Object::name("NoSuchFilter"));
    if form {
        unreadable.set(b"Type".to_vec(), Object::name("XObject"));
        unreadable.set(b"Subtype".to_vec(), Object::name("Form"));
        unreadable.set(b"BBox".to_vec(), Object::Array(vec![Object::Int(0), Object::Int(0), Object::Int(200), Object::Int(200)]));
        unreadable.set(b"Resources".to_vec(), Object::Dict(glyph_resources.clone()));
    }
    let unreadable = doc.add(Object::Stream(pdfcraft_cos::Stream::from_raw(unreadable, b"BT /T3 50 Tf 50 80 Td (A) Tj ET".to_vec())));
    let (contents, resources) = if form {
        let contents = doc.add(Object::Stream(pdfcraft_cos::Stream::from_raw(pdfcraft_cos::Dict::new(), b"/Fm Do".to_vec())));
        let mut xobjects = pdfcraft_cos::Dict::new();
        xobjects.set(b"Fm".to_vec(), Object::Ref(unreadable));
        let mut resources = pdfcraft_cos::Dict::new();
        resources.set(b"XObject".to_vec(), Object::Dict(xobjects));
        (contents, resources)
    } else {
        (unreadable, glyph_resources)
    };
    let mut page = pdfcraft_cos::Dict::new();
    page.set(b"Type".to_vec(), Object::name("Page"));
    page.set(b"Parent".to_vec(), Object::Ref(ObjRef::new(2, 0)));
    page.set(b"Contents".to_vec(), Object::Ref(contents));
    page.set(b"Resources".to_vec(), Object::Dict(resources));
    let page = doc.add(page);
    doc.update_dict(ObjRef::new(2, 0), |pages| {
        pages.set(b"Kids".to_vec(), Object::Array(vec![Object::Ref(ObjRef::new(3, 0)), Object::Ref(page)]));
        pages.set(b"Count".to_vec(), Object::Int(2));
    })
    .unwrap();
    full_roundtrip(&doc)
}

#[test]
fn extract_keeps_type3_glyph_resources_of_an_undecodable_page() {
    let part = full_roundtrip(&extract_pages(&doc_type3_resources_of_undecodable_content(false), &[0, 1]).unwrap());
    assert!(xobject_names(&part, 0).iter().any(|n| n.as_slice() == b"ImB"), "an unreadable page's font shares a different page's XObject dictionary");
    let contents = part.resolve(page_dict(&part, 1).get(b"Contents").unwrap());
    let Object::Stream(contents) = &*contents else { panic!("content stream") };
    assert_eq!(contents.dict.name(b"Filter"), Some(b"NoSuchFilter".as_slice()));
    assert_eq!(contents.raw.as_ref(), b"BT /T3 50 Tf 50 80 Td (A) Tj ET");
}

#[test]
fn extract_keeps_type3_glyph_resources_of_an_undecodable_form() {
    let part = full_roundtrip(&extract_pages(&doc_type3_resources_of_undecodable_content(true), &[0, 1]).unwrap());
    assert!(xobject_names(&part, 0).iter().any(|n| n.as_slice() == b"ImB"), "an unreadable form's font shares a different page's XObject dictionary");
}

#[test]
fn extract_still_prunes_separate_resources_when_many_pages_share_a_type3_font() {
    let mut src = doc_type3_shared_xobjects("<< /XObject << /ImB 10 0 R >> >>");
    src.set(ObjRef::new(4, 0), Object::Stream(pdfcraft_cos::Stream::from_raw(pdfcraft_cos::Dict::new(), b"/ImA Do".to_vec())));
    let page = src.get(ObjRef::new(3, 0)).as_dict().unwrap().clone();
    let mut kids = vec![Object::Ref(ObjRef::new(3, 0))];
    for _ in 1..4_000 {
        kids.push(Object::Ref(src.add(page.clone())));
    }
    src.update_dict(ObjRef::new(2, 0), |pages| {
        pages.set(b"Kids".to_vec(), Object::Array(kids));
        pages.set(b"Count".to_vec(), Object::Int(4_000));
    })
    .unwrap();
    let src = full_roundtrip(&src);
    let part = full_roundtrip(&extract_pages(&src, &(0..4_000).collect::<Vec<_>>()).unwrap());
    assert_eq!(xobject_names(&part, 0), [b"ImA".to_vec()], "one shared font's direct resource graph is protected once");
}

// Both sides of the comparison go through the same pdftoppm, so the assertion is about our two
// documents rather than about Poppler's output, and no Poppler version can change the verdict.
// It skips where pdftoppm is absent and fails where PDFCRAFT_ORACLES_REQUIRED is set.
#[test]
fn extract_type3_shared_xobjects_matches_poppler_pixels() {
    let pdftoppm = pdfcraft_oracle::or_skip!(pdfcraft_oracle::pdftoppm());
    let dir = std::env::temp_dir().join(format!("pdfcraft-type3-extract-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fixtures = [
        doc_type3_shared_xobjects("<< /XObject 6 0 R >>"),
        doc_type3_shared_xobjects("5 0 R"),
        doc_type3_shared_xobjects("<< /XObject << /ImB 10 0 R >> >>"),
        doc_type3_glyph_form_with_shared_resources(false),
        doc_type3_glyph_form_with_shared_resources(true),
    ];
    for (i, src) in fixtures.into_iter().enumerate() {
        let part = full_roundtrip(&extract_pages(&src, &[0]).unwrap());
        let render = |doc: &Document, name: &str| {
            let path = dir.join(format!("{i}-{name}.pdf"));
            std::fs::write(&path, write_full(doc, &SaveOptions::default()).unwrap()).unwrap();
            let output = pdftoppm.command().args(["-singlefile", "-r", "72", "-gray"]).arg(path).output().unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            assert!(output.stderr.is_empty(), "{}", String::from_utf8_lossy(&output.stderr));
            output.stdout
        };
        let before = render(&src, "source");
        let after = render(&part, "extracted");
        assert!(before.starts_with(b"P5\n200 200\n255\n"));
        assert!(before.iter().filter(|&&v| v < 200).count() > 2500, "the source contains a drawn glyph");
        assert_eq!(after, before, "extraction must keep every rendered pixel in fixture {i}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// A page listed twice whose widget's field keeps `/Kids` as an array object of its own: the
/// second widget joins that array, and the field keeps every widget it had.
#[test]
fn a_repeated_widget_joins_an_indirect_kids_array() {
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R] >> >>".into(), // 1
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 300 400] >>".into(), // 2
        "<< /Type /Page /Parent 2 0 R /Annots [5 0 R] >>".into(),                   // 3
        "<< /Type /Page /Parent 2 0 R /Annots [8 0 R] >>".into(),                   // 4
        "<< /Type /Annot /Subtype /Widget /Parent 6 0 R /Rect [10 10 90 30] /P 3 0 R >>".into(), // 5
        "<< /FT /Tx /T (name) /V (Ada) /Kids 7 0 R >>".into(),                      // 6
        "[5 0 R 8 0 R]".into(),                                                     // 7
        "<< /Type /Annot /Subtype /Widget /Parent 6 0 R /Rect [10 40 90 60] /P 4 0 R >>".into(), // 8
    ];
    let src = open(build(&b, "/Root 1 0 R"));
    let out = full_roundtrip(&extract_pages(&src, &[0, 0, 1]).unwrap());
    let form = out.resolve(catalog(&out).get(b"AcroForm").unwrap());
    let fields = form.as_dict().unwrap().get(b"Fields").and_then(Object::as_array).cloned().unwrap();
    assert_eq!(fields.len(), 1, "one field");
    let field = out.resolve(&fields[0]).as_dict().cloned().unwrap();
    let kids = out.resolve(field.get(b"Kids").unwrap()).as_array().cloned().unwrap();
    assert_eq!(kids.len(), 3, "both original widgets and the repeat's own: {kids:?}");
}

/// A one-page document whose form holds one text field named `name` (a widget that is its own
/// field, the smallest form a page can carry).
fn doc_with_field(name: &str) -> Document {
    doc_with_fields(&[name])
}

/// A one-page document whose form holds one widget field per name, in order.
fn doc_with_fields(names: &[&str]) -> Document {
    let first = 5; // object number of the first field
    let refs: Vec<String> = names.iter().enumerate().map(|(i, _)| format!("{} 0 R", first + i)).collect();
    let mut b: Vec<String> = vec![
        format!("<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [{}] >> >>", refs.join(" ")), // 1
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),                // 2
        format!("<< /Type /Page /Parent 2 0 R /Annots [{}] >>", refs.join(" ")),                   // 3
        "<< /Title (form) >>".into(),                                                              // 4
    ];
    for name in names.iter() {
        b.push(format!("<< /Type /Annot /Subtype /Widget /FT /Tx /T ({name}) /V (x) /Rect [60 60 200 80] /P 3 0 R >>")); // 5 + i
    }
    open(build(&b, "/Root 1 0 R /Info 4 0 R"))
}

/// The /T names of a document's /AcroForm /Fields, in order.
fn form_field_names(doc: &Document) -> Vec<String> {
    let Some(form) = catalog(doc).get(b"AcroForm").map(|f| doc.resolve(f)).and_then(|f| f.as_dict().cloned()) else {
        return Vec::new();
    };
    form.get(b"Fields")
        .map(|f| doc.resolve(f))
        .and_then(|f| f.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|f| {
            doc.resolve(f)
                .as_dict()
                .and_then(|d| d.get(b"T").cloned())
                .map(|t| doc.resolve(&t))
                .and_then(|t| t.as_string().map(|s| String::from_utf8_lossy(&s.bytes).into_owned()))
                .unwrap_or_default()
        })
        .collect()
}

/// Two documents whose forms both name a field Text1 cannot just be put together: fields of
/// one fully-qualified name are one field, so the values would blur into it. The incoming
/// field is renamed to the first free variant, Text1-1.
#[test]
fn import_renames_a_colliding_field_name() {
    let mut dst = doc_with_field("Text1");
    let src = doc_with_field("Text1");
    import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Rename).unwrap();
    assert_eq!(form_field_names(&dst), ["Text1", "Text1-1"]);
}

/// The rename takes the first free variant: with Text1 and Text1-1 already in the form, a
/// colliding Text1 becomes Text1-2 (not Text1-1, which would collide again).
#[test]
fn import_renames_to_the_first_free_variant() {
    let mut dst = doc_with_fields(&["Text1", "Text1-1"]);
    let src = doc_with_field("Text1");
    import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Rename).unwrap();
    assert_eq!(form_field_names(&dst), ["Text1", "Text1-1", "Text1-2"]);
}

/// A one-page document whose form holds one text field whose `/T` is a UTF-16BE hex string —
/// the standard encoding for non-ASCII names — decoding to `name`.
fn doc_with_field_utf16(name: &str) -> Document {
    let t: String = std::iter::once(0xFEFFu16).chain(name.encode_utf16()).map(|u| format!("{u:04X}")).collect();
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R] >> >>".into(), // 1
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(), // 2
        "<< /Type /Page /Parent 2 0 R /Annots [5 0 R] >>".into(),                   // 3
        "<< /Title (form) >>".into(),                                               // 4
        format!("<< /Type /Annot /Subtype /Widget /FT /Tx /T <{t}> /V (x) /Rect [60 60 200 80] /P 3 0 R >>"), // 5
    ];
    open(build(&b, "/Root 1 0 R /Info 4 0 R"))
}

/// The decoded `/T` names of a document's /AcroForm /Fields, in order (whatever encoding each
/// was written in).
fn decoded_form_field_names(doc: &Document) -> Vec<String> {
    let Some(form) = catalog(doc).get(b"AcroForm").map(|f| doc.resolve(f)).and_then(|f| f.as_dict().cloned()) else {
        return Vec::new();
    };
    form.get(b"Fields")
        .map(|f| doc.resolve(f))
        .and_then(|f| f.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|f| {
            doc.resolve(f)
                .as_dict()
                .and_then(|d| d.get(b"T").cloned())
                .map(|t| doc.resolve(&t))
                .and_then(|t| t.as_string().map(|s| s.to_text()))
                .unwrap_or_default()
        })
        .collect()
}

/// A field's name is its text, not the bytes it was written in: a destination field named in
/// PDFDocEncoding and an incoming UTF-16BE field that decode to the same name are one name,
/// so the incoming field is renamed out of the way (not silently blurred into the other).
#[test]
fn import_renames_by_decoded_name_whatever_its_encoding() {
    let mut dst = doc_with_field("Text1");
    let src = doc_with_field_utf16("Text1");
    let out = import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Rename).unwrap();
    assert_eq!(out.renamed_fields, [("Text1".to_string(), "Text1-1".to_string())]);
    assert_eq!(decoded_form_field_names(&dst), ["Text1", "Text1-1"]);
}

/// Renaming a non-ASCII name keeps it readable: the new `/T` stores the new name as text
/// (UTF-16BE again), never as mojibake of the old bytes.
#[test]
fn a_renamed_non_ascii_name_stays_readable() {
    let mut dst = doc_with_field_utf16("Förm");
    let src = doc_with_field_utf16("Förm");
    let out = import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Rename).unwrap();
    assert_eq!(out.renamed_fields, [("Förm".to_string(), "Förm-1".to_string())]);
    assert_eq!(decoded_form_field_names(&dst), ["Förm", "Förm-1"]);
}

/// A one-page document whose form holds one field tree: a root field `root` with one child
/// field `kid` (a merged field-and-widget, as page tools make them).
fn doc_with_tree(root: &str, kid: &str) -> Document {
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R] >> >>".into(), // 1
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(), // 2
        "<< /Type /Page /Parent 2 0 R /Annots [6 0 R] >>".into(),                   // 3
        "<< /Title (form) >>".into(),                                               // 4
        format!("<< /FT /Tx /T ({root}) /Kids [6 0 R] >>"),                         // 5
        format!("<< /Type /Annot /Subtype /Widget /FT /Tx /T ({kid}) /Parent 5 0 R /Rect [60 60 200 80] /P 3 0 R >>"), // 6
    ];
    open(build(&b, "/Root 1 0 R /Info 4 0 R"))
}

/// A field's name is its place in the tree, not just its own `/T`: an incoming field called
/// x.b collides with the child b of the destination's field x, and is renamed out of the way.
#[test]
fn import_renames_a_nested_field_whose_full_name_collides() {
    let mut dst = doc_with_tree("x", "b");
    let src = doc_with_field("x.b");
    import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Rename).unwrap();
    assert_eq!(form_field_names(&dst), ["x", "x.b-1"]);
}

/// The collision may sit below the top level: the destination holds a field literally named
/// a.b, and the incoming tree a ▸ b walks into it. Only the colliding child is renamed — the
/// incoming root a never collided, so it keeps its name.
#[test]
fn import_renames_the_colliding_child_not_the_whole_tree() {
    let mut dst = doc_with_field("a.b");
    let src = doc_with_tree("a", "b");
    import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Rename).unwrap();
    assert_eq!(form_field_names(&dst), ["a.b", "a"], "the incoming root keeps its name");
    // The incoming tree is the second /Fields entry; its child was renamed.
    let fields = catalog(&dst)
        .get(b"AcroForm")
        .map(|f| dst.resolve(f))
        .and_then(|f| f.as_dict().cloned())
        .unwrap()
        .get(b"Fields")
        .map(|l| dst.resolve(l))
        .and_then(|l| l.as_array().cloned())
        .unwrap();
    let kid = dst.resolve(&fields[1]).as_dict().unwrap().get(b"Kids").map(|k| dst.resolve(k)).and_then(|k| k.as_array().cloned()).unwrap();
    let kid_t = dst.resolve(&kid[0]).as_dict().unwrap().get(b"T").cloned().unwrap();
    let resolved = dst.resolve(&kid_t);
    let name = resolved.as_string().unwrap();
    assert_eq!(String::from_utf8_lossy(&name.bytes), "b-1", "the child's /T is renamed, the FQN follows the root");
}

/// A field name is taken even when it hangs only off a page's annotations: a widget that is
/// its own field, never registered in `/AcroForm /Fields`, still names a field, and an import
/// must not silently blur into it.
#[test]
fn import_renames_away_from_an_orphaned_widgets_name() {
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),                                 // 1 (no /AcroForm)
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(), // 2
        "<< /Type /Page /Parent 2 0 R /Annots [5 0 R] >>".into(),                   // 3
        "<< /Title (orphan) >>".into(),                                             // 4
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (Text1) /V (x) /Rect [60 60 200 80] /P 3 0 R >>".into(), // 5
    ];
    let mut dst = open(build(&b, "/Root 1 0 R /Info 4 0 R"));
    let src = doc_with_field("Text1");
    import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Rename).unwrap();
    assert_eq!(form_field_names(&dst), ["Text1-1"], "the orphaned Text1 must count as taken");
}

/// Callers can keep the older behaviour — an incoming field of a taken name becomes one field
/// with the one already there — by asking to merge. Nothing is renamed.
#[test]
fn import_can_merge_into_same_named_fields_instead() {
    let mut dst = doc_with_field("Text1");
    let src = doc_with_field("Text1");
    let outcome = import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Merge).unwrap();
    assert_eq!(form_field_names(&dst), ["Text1", "Text1"]);
    assert!(outcome.renamed_fields.is_empty(), "merge renames nothing");
}

/// Duplicating a page copies the document's own fields: the copies keep their names (one field
/// shown twice, as before field-conflict renaming existed) instead of silently becoming Text1-1.
#[test]
fn duplicating_a_page_with_a_field_keeps_the_field_name() {
    let mut doc = doc_with_field("Text1");
    crate::duplicate_pages(&mut doc, &[0], None).unwrap();
    assert_eq!(crate::page_count(&doc).unwrap(), 2);
    assert_eq!(form_field_names(&doc), ["Text1", "Text1"], "nothing renamed");
}

/// Replace Pages keeps the field names too.
#[test]
fn replacing_a_page_renames_no_fields() {
    let mut doc = doc_with_field("Text1");
    crate::replace_pages(&mut doc, &[0], &doc_with_field("Text1"), &[0]).unwrap();
    assert!(form_field_names(&doc).iter().all(|n| n == "Text1"), "{:?}", form_field_names(&doc));
}

/// Extract and Split copy one document: two same-named field roots (malformed but common) stay
/// one field by name rather than being split apart as Name and Name-1.
#[test]
fn extract_and_split_keep_same_named_fields_together() {
    let src = doc_with_fields(&["Text1", "Text1"]);
    let out = extract_pages(&src, &[0]).unwrap();
    assert_eq!(form_field_names(&out), ["Text1", "Text1"]);
    for part in split(&src, &SplitBy::PageCount(1)).unwrap() {
        assert_eq!(form_field_names(&part), ["Text1", "Text1"]);
    }
}

/// An import says what it renamed, so callers can tell the user — and their scripts — that a
/// field they knew by one name now goes by another.
#[test]
fn import_reports_the_fields_it_renamed() {
    let mut dst = doc_with_fields(&["Text1", "Text1-1"]);
    let src = doc_with_field("Text1");
    let outcome = import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Rename).unwrap();
    assert_eq!(outcome.renamed_fields, [("Text1".to_string(), "Text1-2".to_string())]);
    assert_eq!(outcome.pages.len(), 1);
}

/// A renamed tree survives a save and reopen: the parent keeps its new name, the child still
/// hangs under it with its own name unchanged, and an independent parser reads the same form.
#[test]
fn a_renamed_field_tree_survives_a_save_and_reopen() {
    let mut dst = doc_with_tree("a", "b");
    let src = doc_with_tree("a", "b");
    import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Rename).unwrap();
    let out = full_roundtrip(&dst);
    assert_eq!(form_field_names(&out), ["a", "a-1"], "the incoming root was renamed");
    let fields = catalog(&out)
        .get(b"AcroForm")
        .map(|f| out.resolve(f))
        .and_then(|f| f.as_dict().cloned())
        .unwrap()
        .get(b"Fields")
        .map(|l| out.resolve(l))
        .and_then(|l| l.as_array().cloned())
        .unwrap();
    let root = out.resolve(&fields[1]).as_dict().cloned().unwrap();
    let kids = root.get(b"Kids").map(|k| out.resolve(k)).and_then(|k| k.as_array().cloned()).unwrap();
    let kid = out.resolve(&kids[0]).as_dict().cloned().unwrap();
    assert_eq!(kid.reference(b"Parent"), fields[1].as_ref(), "the child still hangs under the renamed root");
    let kid_t = kid.get(b"T").cloned().unwrap();
    let resolved = out.resolve(&kid_t);
    assert_eq!(String::from_utf8_lossy(&resolved.as_string().unwrap().bytes), "b", "the child keeps its own name");
}

/// The pre-scan says what an import of `src` into `dst` would rename, without changing
/// anything — the Combine tab shows it before anything is combined.
#[test]
fn field_name_conflicts_previews_what_an_import_renames() {
    let dst = doc_with_fields(&["Text1", "Text1-1"]);
    let src = doc_with_field("Text1");
    assert_eq!(field_name_conflicts(&dst, &src), [("Text1".to_string(), "Text1-2".to_string())]);
    let apart = doc_with_field("Signature1");
    assert!(field_name_conflicts(&dst, &apart).is_empty(), "nothing collides, nothing to preview");
}

/// The collision is not only with the destination: two incoming fields of one name would blur
/// into each other just the same, so the second is renamed too.
#[test]
fn import_renames_a_field_colliding_with_another_incoming_field() {
    let mut dst = doc_with_field("Other");
    let src = doc_with_fields(&["Text1", "Text1"]);
    import_pages_with(&mut dst, &src, &[0], 1, FieldConflicts::Rename).unwrap();
    assert_eq!(form_field_names(&dst), ["Other", "Text1", "Text1-1"]);
}

/// A page listed twice shows its widget on both copies — also when the field was renamed: each
/// copy has its own widget, both widgets belong to the one renamed field (#1810), and no second
/// Text1 appears.
#[test]
fn a_repeated_page_joins_the_renamed_field() {
    let mut dst = doc_with_field("Text1");
    let src = doc_with_field("Text1");
    import_pages_with(&mut dst, &src, &[0, 0], 2, FieldConflicts::Rename).unwrap();
    assert_eq!(form_field_names(&dst), ["Text1", "Text1-1"], "one field, renamed once");
    let fields = catalog(&dst)
        .get(b"AcroForm")
        .map(|f| dst.resolve(f))
        .and_then(|f| f.as_dict().cloned())
        .unwrap()
        .get(b"Fields")
        .map(|l| dst.resolve(l))
        .and_then(|l| l.as_array().cloned())
        .unwrap();
    let renamed = fields[1].clone();
    let mut widgets = Vec::new();
    for page in [1, 2] {
        let list = page_dict(&dst, page).get(b"Annots").map(|a| dst.resolve(a)).and_then(|a| a.as_array().cloned()).unwrap();
        let widget = list.iter().find_map(|a| a.as_ref()).expect("a widget on the page");
        widgets.push(widget);
        assert_eq!(dst.get(widget).as_dict().unwrap().reference(b"Parent"), renamed.as_ref(), "page {page}'s widget belongs to the renamed field");
    }
    assert_ne!(widgets[0], widgets[1], "each copy has its own widget");
}

/// Combining forms keeps same-named fields separate — the second file's colliding field is
/// renamed — and the combine says what it renamed.
#[test]
fn combine_renames_colliding_field_names_and_reports_them() {
    let a = doc_with_field("Text1");
    let b = doc_with_field("Text1");
    let out = combine_selected_with(&[("a", &a as &Document, None), ("b", &b, None)], FieldConflicts::Rename).unwrap();
    assert_eq!(out.renamed_fields, [("Text1".to_string(), "Text1-1".to_string())]);
    assert_eq!(form_field_names(&out.doc), ["Text1", "Text1-1"]);
    let merged = combine_selected_with(&[("a", &a, None), ("b", &b, None)], FieldConflicts::Merge).unwrap();
    assert!(merged.renamed_fields.is_empty());
    assert_eq!(form_field_names(&merged.doc), ["Text1", "Text1"], "merge keeps the older behaviour");
}

/// The grouped combine (a file's pages split around others') renames colliding names between
/// files just the same, while a file combined with itself once keeps its fields whole.
#[test]
fn combine_grouped_renames_colliding_field_names() {
    let a = doc_with_field("Text1");
    let b = doc_with_field("Text1");
    let runs: Vec<Run<'_>> = vec![(0, "a", &a, None), (1, "b", &b, None)];
    let out = combine_grouped_with(&runs, FieldConflicts::Rename).unwrap();
    assert_eq!(out.renamed_fields, [("Text1".to_string(), "Text1-1".to_string())]);
    assert_eq!(form_field_names(&out.doc), ["Text1", "Text1-1"]);
    let split: Vec<Run<'_>> = vec![(0, "a", &a, Some(&[0])), (0, "a", &a, None), (1, "b", &b, None)];
    let once = combine_grouped_with(&split, FieldConflicts::Rename).unwrap();
    assert_eq!(
        once.renamed_fields,
        [("Text1".to_string(), "Text1-1".to_string())],
        "the grouped file is copied once; only b's colliding field renames"
    );
}

/// The pre-flight scan over a whole combine list: what each file's fields would be renamed,
/// in order, exactly as the combine would rename them — the Combine tab shows this before
/// anything is combined.
#[test]
fn combine_field_conflicts_previews_each_file_in_order() {
    let a = doc_with_field("Text1");
    let b = doc_with_field("Text1");
    let c = doc_with_fields(&["Text1", "Other"]);
    let preview = combine_field_conflicts(&[&a, &b, &c]);
    assert_eq!(preview[0], [], "the first file collides with nothing");
    assert_eq!(preview[1], [("Text1".to_string(), "Text1-1".to_string())]);
    assert_eq!(preview[2], [("Text1".to_string(), "Text1-2".to_string())], "the cascade continues past the earlier rename");
}

/// A one-page source whose page refers (`/Foo`) to object 6, followed by `tail` as objects 6, 7, …
fn chained(tail: &[String]) -> Document {
    let mut b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> /Foo 6 0 R >>".into(),
        body("C1"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
    ];
    b.extend_from_slice(tail);
    open(build(&b, "/Root 1 0 R"))
}

/// [`chained`] with `n` objects, each referring to the next.
fn chain(n: usize) -> Document {
    let mut tail: Vec<String> = (0..n).map(|i| format!("<< /Next {} 0 R >>", 7 + i)).collect();
    tail.push("<< /End true >>".into());
    chained(&tail)
}

/// #1800: thousands of shallow objects, each referring to the next, used to recurse once per
/// object and exhaust the stack. The copy is refused instead, and the destination is as it was;
/// a chain within the budget and a short cycle copy as before.
#[test]
fn a_chain_of_objects_too_deep_to_copy_is_refused_and_leaves_the_destination_alone() {
    // On a thread with less stack than any platform's main thread, so the budget is proven.
    let run = std::thread::Builder::new()
        .stack_size(512 << 10)
        .spawn(|| {
            let mut dst = open(fixture());
            let before = (labels(&dst), dst.object_numbers());
            let err = import_pages(&mut dst, &chain(20_000), &[0], 0).unwrap_err();
            assert!(matches!(&err, OrganizeError::Invalid(m) if m.contains("levels deep")), "{err:?}");
            assert_eq!((labels(&dst), dst.object_numbers()), before, "the destination is as it was");
            import_pages(&mut dst, &chain(100), &[0], 0).unwrap();
            let cycle = chained(&["<< /Next 7 0 R >>".into(), "<< /Next 6 0 R >>".into()]);
            import_pages(&mut dst, &cycle, &[0], 0).unwrap();
            assert_eq!(labels(&dst)[..3], ["C1", "C1", "Page 1"]);
        })
        .unwrap()
        .join();
    assert!(run.is_ok(), "the copy must not overflow the stack");
}

/// The fixture after an update that adds object 20 and declares `/Size 4294967295`. No number is
/// left above that, so [`Document::add`] fills free numbers from 12 on, the gap below object 20 first.
fn fixture_with_an_exhausted_size() -> Document {
    let mut bytes = fixture();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let prev: usize = text.rsplit("startxref").next().and_then(|t| t.split_whitespace().next()).and_then(|n| n.parse().ok()).expect("startxref");
    let added = bytes.len();
    bytes.extend_from_slice(b"20 0 obj\n<< /Unused true >>\nendobj\n");
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!(
            "xref\n20 1\n{added:010} 00000 n \ntrailer\n<< /Size 4294967295 /Root 1 0 R /Info 11 0 R /Prev {prev} >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .as_bytes(),
    );
    let doc = open(bytes);
    assert!(doc.repair_log().iter().any(|l| l.contains("exhausted trailer")), "{:?}", doc.repair_log());
    doc
}

/// The fixture holding object 4294967295, as a document does once an edit writes to an object of
/// that number (a damaged file's catalog, say). [`Document::add`] then fills free numbers below it.
fn fixture_holding_object_4294967295() -> Document {
    let mut doc = open(fixture());
    doc.set(ObjRef::new(u32::MAX, 0), Object::String(PdfString::text("top")));
    doc
}

/// Documents whose new objects are numbered in the gaps below their highest number.
fn documents_numbering_new_objects_below_their_highest() -> [(&'static str, Document); 2] {
    [("an exhausted /Size", fixture_with_an_exhausted_size()), ("object 4294967295", fixture_holding_object_4294967295())]
}

/// Duplicate, Replace and Insert Pages took the objects numbered above the destination's highest
/// number for the ones the import added. When new objects fill gaps below it, the copies escaped
/// resource sharing (each import stored the fonts again), and with object 4294967295 the number
/// after the highest overflowed: a panic in debug builds, and in release a wrap to 0 that counted
/// every object as added. The import now compares the object numbers before and after.
#[test]
fn an_import_shares_the_resources_it_added_wherever_they_are_numbered() {
    let mut plain = open(fixture());
    duplicate_pages(&mut plain, &[0], None).unwrap();
    for (what, mut doc) in documents_numbering_new_objects_below_their_highest() {
        let before = doc.object_numbers();
        duplicate_pages(&mut doc, &[0], None).unwrap();
        assert_eq!(labels(&doc), ["Page 1", "Page 1", "Page 2", "Page 3"], "{what}");
        assert_eq!(count_fonts(&doc), count_fonts(&plain), "{what}: the copy shares the font, as it does in the plain fixture");
        insert_file_pages(&mut doc, &open(fixture()), "Inserted", &[2], 0, FieldConflicts::Rename).unwrap();
        assert_eq!(labels(&doc), ["Page 3", "Page 1", "Page 1", "Page 2", "Page 3"], "{what}");
        assert_eq!(count_fonts(&doc), count_fonts(&plain), "{what}: the inserted page shares the font too");
        let after = doc.object_numbers();
        assert!(before.iter().all(|n| after.contains(n)), "{what}: every object the document had is still there");
        assert_eq!(labels(&full_roundtrip(&doc)), ["Page 3", "Page 1", "Page 1", "Page 2", "Page 3"], "{what}");
    }
}

/// A refused import takes back what it added, which in these documents is numbered below their
/// highest object, and leaves the objects they had.
#[test]
fn a_refused_import_takes_back_what_it_added_wherever_it_is_numbered() {
    let run = std::thread::Builder::new()
        .stack_size(512 << 10)
        .spawn(|| {
            for (what, mut dst) in documents_numbering_new_objects_below_their_highest() {
                let before = (labels(&dst), dst.object_numbers());
                let err = import_pages(&mut dst, &chain(20_000), &[0], 0).unwrap_err();
                assert!(matches!(&err, OrganizeError::Invalid(m) if m.contains("levels deep")), "{what}: {err:?}");
                assert_eq!((labels(&dst), dst.object_numbers()), before, "{what}: the destination is as it was");
            }
        })
        .unwrap()
        .join();
    assert!(run.is_ok(), "a refused import must leave the document as it was, without panicking");
}

/// What Edit ▸ Attach file added survives Combine files (upstream's writer), names
/// with accents included, and the combined document takes more attachments and deletions.
#[test]
fn attachments_added_in_the_app_survive_combine_and_more_can_be_attached() {
    let embedded = |doc: &Document, key: &str| -> (Vec<u8>, Option<String>) {
        let names = doc.resolve(catalog(doc).get(b"Names").unwrap()).as_dict().cloned().unwrap();
        let tree = doc.resolve(names.get(b"EmbeddedFiles").unwrap()).as_dict().cloned().unwrap();
        let pairs = tree.get(b"Names").and_then(|n| n.as_array()).cloned().unwrap();
        let at = pairs.chunks(2).position(|p| p[0].as_string().unwrap().to_text() == key).unwrap_or_else(|| panic!("no attachment {key}"));
        let spec = doc.resolve(&pairs[at * 2 + 1]).as_dict().cloned().unwrap();
        let desc = spec.get(b"Desc").and_then(Object::as_string).map(|s| s.to_text());
        let ef = spec.get(b"EF").and_then(Object::as_dict).unwrap().get(b"F").cloned().unwrap();
        let Object::Stream(stream) = doc.resolve(&ef).as_ref().clone() else { panic!("the embedded file is a stream") };
        (stream.decoded().unwrap(), desc)
    };
    let mut a = Document::open(Arc::new(fixture())).unwrap();
    add_attachment(&mut a, "invoice.csv", b"a,b", Some("March"), Some("text/csv"), None).unwrap();
    add_attachment(&mut a, "año.txt", b"uno", None, None, None).unwrap();
    let mut b = Document::open(Arc::new(fixture())).unwrap();
    add_attachment(&mut b, "invoice.csv", b"x,y", None, None, None).unwrap();
    add_attachment(&mut b, "año.txt", b"dos", None, None, None).unwrap();
    let a = save_and_reopen(&a);
    let b = save_and_reopen(&b);
    let mut out = full_roundtrip(&combine(&[("A", &a), ("B", &b)]).unwrap());
    // Both files' attachments, the second copies numbered as text (not bytes glued onto UTF-16).
    assert_eq!(attachment_keys(&out), ["invoice.csv", "invoice.csv (2)", "año.txt", "año.txt (2)"]);
    assert_eq!(embedded(&out, "invoice.csv"), (b"a,b".to_vec(), Some("March".into())));
    assert_eq!(embedded(&out, "invoice.csv (2)"), (b"x,y".to_vec(), None));
    assert_eq!(embedded(&out, "año.txt (2)").0, b"dos");
    // The combined document goes on taking attachments, and losing them, like any other.
    assert_eq!(add_attachment(&mut out, "invoice.csv", b"z", None, None, None).unwrap(), "invoice.csv (3)");
    delete_attachment(&mut out, "invoice.csv (2)").unwrap();
    let out = full_roundtrip(&out);
    assert_eq!(attachment_keys(&out), ["invoice.csv", "invoice.csv (3)", "año.txt", "año.txt (2)"]);
    assert_eq!(embedded(&out, "invoice.csv (3)").0, b"z");
}

#[test]
fn attachments_are_added_listed_and_deleted() {
    let mut doc = Document::open(Arc::new(fixture())).unwrap();
    assert!(attachment_keys(&doc).is_empty());
    // A path loses its folder; the key is the file name.
    let key = add_attachment(&mut doc, "/Users/eva/Desktop/invoice.csv", b"a,b\n1,2\n", Some("March"), Some("text/csv"), Some("D:20261012120000Z"))
        .unwrap();
    assert_eq!(key, "invoice.csv");
    // The same name again is kept apart.
    assert_eq!(add_attachment(&mut doc, "invoice.csv", b"x", None, None, None).unwrap(), "invoice.csv (2)");
    assert_eq!(add_attachment(&mut doc, "  ", b"x", None, None, None), Err(OrganizeError::Invalid("the attachment needs a file name".into())));
    let doc = save_and_reopen(&doc);
    assert_eq!(attachment_keys(&doc), ["invoice.csv", "invoice.csv (2)"]);
    // What was written: a Filespec with the embedded stream, its size and type, and the description.
    let root = doc.root().unwrap();
    let names = doc.get(root).as_dict().unwrap().get(b"Names").cloned().unwrap();
    let tree = doc.resolve(&names).as_dict().unwrap().get(b"EmbeddedFiles").cloned().unwrap();
    let tree = doc.resolve(&tree);
    let pairs = tree.as_dict().unwrap().get(b"Names").cloned().unwrap();
    let pairs = doc.resolve(&pairs).as_array().cloned().unwrap();
    assert_eq!(pairs.len(), 4);
    let spec = doc.resolve(&pairs[1]);
    let spec = spec.as_dict().unwrap();
    assert_eq!(spec.get(b"Type"), Some(&Object::name("Filespec")));
    assert_eq!(spec.get(b"UF").and_then(Object::as_string).map(PdfString::to_text).as_deref(), Some("invoice.csv"));
    assert_eq!(spec.get(b"Desc").and_then(Object::as_string).map(PdfString::to_text).as_deref(), Some("March"));
    let ef = spec.get(b"EF").and_then(Object::as_dict).unwrap().get(b"F").cloned().unwrap();
    let ef = doc.resolve(&ef);
    let Object::Stream(stream) = ef.as_ref() else { panic!("the embedded file is a stream") };
    assert_eq!(stream.dict.get(b"Subtype"), Some(&Object::Name(b"text/csv".to_vec())));
    let params = stream.dict.get(b"Params").and_then(Object::as_dict).unwrap();
    assert_eq!(params.get(b"Size"), Some(&Object::Int(8)));
    assert_eq!(stream.decoded().unwrap(), b"a,b\n1,2\n");
    // Deleting: the one named goes, the other stays; the last one takes the tree with it.
    let mut doc = doc;
    delete_attachment(&mut doc, "invoice.csv").unwrap();
    assert_eq!(attachment_keys(&doc), ["invoice.csv (2)"]);
    assert_eq!(delete_attachment(&mut doc, "invoice.csv"), Err(OrganizeError::Invalid("there is no attachment named invoice.csv".into())));
    delete_attachment(&mut doc, "invoice.csv (2)").unwrap();
    let doc = save_and_reopen(&doc);
    assert!(attachment_keys(&doc).is_empty());
    assert!(doc.get(doc.root().unwrap()).as_dict().unwrap().get(b"Names").is_none());
}

#[test]
fn attachment_description_changes_only_the_file_spec_and_can_be_cleared() {
    let mut doc = Document::open(Arc::new(fixture())).unwrap();
    let key = add_attachment(&mut doc, "notes.txt", b"keep these bytes", Some("Original"), Some("text/plain"), None).unwrap();
    let root = doc.root().unwrap();
    let names = doc.get(root).as_dict().unwrap().get(b"Names").cloned().unwrap();
    let tree = doc.resolve(&names).as_dict().unwrap().get(b"EmbeddedFiles").cloned().unwrap();
    let tree = doc.resolve(&tree);
    let pairs = tree.as_dict().unwrap().get(b"Names").cloned().unwrap();
    let resolved_pairs = doc.resolve(&pairs);
    let pairs = resolved_pairs.as_array().unwrap();
    let spec_ref = pairs.get(1).and_then(Object::as_ref).unwrap();
    let stream_ref = doc.get(spec_ref).as_dict().unwrap().get(b"EF").and_then(Object::as_dict).unwrap().reference(b"F").unwrap();
    doc.update_dict(spec_ref, |spec| spec.set(b"CustomKey".to_vec(), Object::name("preserved"))).unwrap();

    set_attachment_description(&mut doc, &key, Some("Revised description")).unwrap();
    let spec_object = doc.get(spec_ref);
    let spec = spec_object.as_dict().unwrap();
    assert_eq!(spec.get(b"Desc").and_then(Object::as_string).map(pdfcraft_cos::PdfString::to_text).as_deref(), Some("Revised description"));
    assert_eq!(spec.get(b"UF").and_then(Object::as_string).map(pdfcraft_cos::PdfString::to_text).as_deref(), Some("notes.txt"));
    assert_eq!(spec.get(b"CustomKey"), Some(&Object::name("preserved")));
    assert_eq!(spec.get(b"EF").and_then(Object::as_dict).and_then(|ef| ef.reference(b"F")), Some(stream_ref));
    let stream_object = doc.get(stream_ref);
    let pdfcraft_cos::Object::Stream(stream) = stream_object.as_ref() else { panic!("the attachment file remains a stream") };
    assert_eq!(stream.decoded().unwrap(), b"keep these bytes");
    assert_eq!(attachment_keys(&doc), ["notes.txt"]);

    set_attachment_description(&mut doc, &key, Some("  ")).unwrap();
    assert!(!doc.get(spec_ref).as_dict().unwrap().contains(b"Desc"));
    let reopened = save_and_reopen(&doc);
    assert_eq!(attachment_keys(&reopened), ["notes.txt"]);
    assert_eq!(
        set_attachment_description(&mut doc, "missing.txt", Some("x")),
        Err(OrganizeError::Invalid("there is no attachment named missing.txt".into()))
    );
}

#[test]
fn rotate_repeated_indices_accumulate_from_the_current_page_value() {
    for level in 0..3 {
        for indices in [vec![0, 0], vec![0, 1, 0]] {
            let mut doc = open(fixture());
            let rotation = doc.add(Object::Int(90));
            let page = pages(&doc).unwrap()[0].obj;
            let target = match level {
                0 => page,
                1 => doc.get(page).as_dict().unwrap().reference(b"Parent").unwrap(),
                _ => pages_root(&doc).unwrap(),
            };
            doc.update_dict(target, |d| d.set(b"Rotate".to_vec(), Object::Ref(rotation))).unwrap();
            let mut doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
            let rotation = walk(&doc).unwrap()[0].1.reference(b"Rotate").unwrap();
            let original_labels = labels(&doc);
            rotate_pages(&mut doc, &indices, 90).unwrap();
            assert_eq!(page_rotation(&doc, 0).unwrap(), 270, "level {level}, indices {indices:?}");
            assert_eq!(*doc.get(rotation), Object::Int(90), "shared source angle is unchanged");
            for saved in [write_incremental(&doc, &SaveOptions::default()).unwrap(), write_full(&doc, &SaveOptions::default()).unwrap()] {
                let reopened = open(saved.clone());
                assert_eq!(page_rotation(&reopened, 0).unwrap(), 270);
                assert_eq!(page_rotation(&reopened, 1).unwrap(), if indices.len() == 3 { 180 } else { 90 });
                assert_eq!(page_rotation(&reopened, 2).unwrap(), 90);
                assert_eq!(labels(&reopened), original_labels);
                let independent = hayro_syntax::Pdf::new(saved).unwrap();
                assert_eq!(independent.pages()[0].render_dimensions(), (400.0, 300.0));
            }
        }
    }
}

#[test]
fn a_bookmark_can_go_to_a_point_on_its_page_and_survives_save() {
    let mut d = doc_a();
    // No position: as before, `/XYZ null null null`, the page top at the reader's zoom.
    crate::add_bookmark(&mut d, &[], 9, "Top", 0).unwrap();
    // Only a height (a bookmark made at the top of the window), a full point, and only a left.
    assert_eq!(crate::add_bookmark_at(&mut d, &[], 9, "Section", 1, None, Some(612.25)).unwrap(), vec![1]);
    crate::add_bookmark_at(&mut d, &[], 9, "Table", 1, Some(72.5), Some(300.0)).unwrap();
    crate::add_bookmark_at(&mut d, &[1], 9, "Inside", 2, Some(10.0), None).unwrap();
    let view = |d: &Document, title: &str| crate::bookmarks(d).into_iter().find(|b| b.title == title).unwrap().view;
    let xyz = |left, top| Some((b"XYZ".to_vec(), vec![left, top, None]));
    let check = |d: &Document| {
        assert_eq!(view(d, "Top"), xyz(None, None));
        assert_eq!(view(d, "Section"), xyz(None, Some(612.25)));
        assert_eq!(view(d, "Table"), xyz(Some(72.5), Some(300.0)));
        assert_eq!(view(d, "Inside"), xyz(Some(10.0), None));
        let page_of = |title: &str| crate::bookmarks(d).into_iter().find(|b| b.title == title).unwrap().page;
        assert_eq!((page_of("Top"), page_of("Section"), page_of("Table"), page_of("Inside")), (Some(0), Some(1), Some(1), Some(2)));
    };
    check(&d);
    check(&full_roundtrip(&d));
}

#[test]
fn a_bookmark_position_that_is_not_a_number_is_refused_and_changes_nothing() {
    let mut d = doc_a();
    crate::add_bookmark(&mut d, &[], 9, "Keep", 0).unwrap();
    for (left, top) in [(Some(f64::NAN), None), (None, Some(f64::INFINITY)), (Some(1.0), Some(f64::NEG_INFINITY))] {
        assert_eq!(crate::add_bookmark_at(&mut d, &[], 9, "Bad", 0, left, top), Err(crate::OutlineError::BadPosition), "{left:?} {top:?}");
    }
    assert_eq!(titles(&crate::bookmarks(&d)), ["Keep"]);
    // The page is still checked, and so is the title.
    assert!(crate::add_bookmark_at(&mut d, &[], 9, "Nowhere", 7, Some(1.0), Some(1.0)).is_err());
    assert_eq!(crate::add_bookmark_at(&mut d, &[], 9, " ", 0, Some(1.0), Some(1.0)), Err(crate::OutlineError::EmptyTitle));
    assert_eq!(titles(&crate::bookmarks(&d)), ["Keep"]);
}

#[test]
fn page_walk_skips_repeated_and_cyclic_kids_without_copying_attributes() {
    let mut doc = open(fixture());
    let (root, branch) = (ObjRef { num: 2, generation: 0 }, ObjRef { num: 3, generation: 0 });
    let (first, second) = (ObjRef { num: 4, generation: 0 }, ObjRef { num: 5, generation: 0 });
    // The branch node carries Resources for its pages: a cycle back to it, then both pages
    // 128 times over.
    let mut kids = vec![Object::Ref(branch)];
    kids.extend([Object::Ref(first), Object::Ref(second)].into_iter().cycle().take(256));
    doc.update_dict(branch, |d| d.set(b"Kids".to_vec(), Object::Array(kids))).unwrap();
    let mut walk = PageWalk::new(&doc, root);
    let (page, attrs) = walk.next().unwrap();
    assert_eq!(page, first);
    assert!(walk.pending.len() <= 3, "repeated kids retained {} pending entries", walk.pending.len());
    assert!(attrs.get(b"Resources").and_then(|r| r.as_dict()).is_some_and(|r| r.contains(b"Font")), "Resources from the branch");
    assert_eq!(attrs.get(b"Rotate").and_then(Object::as_int), Some(90), "Rotate from the root");
    assert_eq!(walk.next().map(|(p, _)| p), Some(second));
    let (_, last) = walk.next().unwrap();
    let media_box = last.get(b"MediaBox").and_then(Object::as_array).map(|m| m.iter().filter_map(Object::as_int).collect::<Vec<_>>());
    assert_eq!(media_box, Some(vec![0, 0, 500, 500]), "own MediaBox kept over the root's");
    assert!(walk.next().is_none());
    assert_eq!(labels(&doc), ["Page 1", "Page 2", "Page 3"]);
}

/// A one-page document whose page carries `annots` (object bodies 5…), with `extra` bodies after them.
fn page_with_annots(annots: &[String], extra: &[String], catalog_extra: &str) -> Document {
    let n = annots.len();
    let refs: String = (0..n).map(|i| format!("{} 0 R ", i + 5)).collect();
    let mut b: Vec<String> = vec![
        format!("<< /Type /Catalog /Pages 2 0 R {catalog_extra} >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),
        format!("<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Annots [{refs}] >>"),
        body("P1"),
    ];
    b.extend(annots.iter().cloned());
    b.extend(extra.iter().cloned());
    open(build(&b, "/Root 1 0 R"))
}

#[test]
fn registering_many_imported_fields_is_linear() {
    let n = 2_000;
    let widgets: Vec<String> = (0..n).map(|i| format!("<< /Type /Annot /Subtype /Widget /FT /Tx /T (f{i}) /Rect [0 0 10 10] >>")).collect();
    let src = page_with_annots(&widgets, &[], "");
    take_work();
    let out = extract_pages(&src, &[0]).unwrap();
    let work = take_work();
    assert!(work <= 4 * n, "{work} steps registering {n} fields");
    let form = out.resolve(catalog(&out).get(b"AcroForm").unwrap()).as_dict().cloned().unwrap();
    assert_eq!(form.get(b"Fields").and_then(|f| f.as_array()).map(Vec::len), Some(n));
}

#[test]
fn renaming_many_same_named_fields_resumes_the_suffix_search() {
    let n = 2_000;
    let widgets: Vec<String> = (0..n).map(|_| "<< /Type /Annot /Subtype /Widget /FT /Tx /T (x) /Rect [0 0 10 10] >>".to_string()).collect();
    let src = page_with_annots(&widgets, &[], "");
    take_work();
    let renames = combine_field_conflicts(&[&src]).remove(0);
    let work = take_work();
    assert!(work <= 4 * n, "{work} suffix attempts for {n} fields");
    // Still the first free variant, in order.
    assert_eq!(renames.len(), n - 1);
    assert_eq!(renames.first(), Some(&("x".to_string(), "x-1".to_string())));
    assert_eq!(renames.last(), Some(&("x".to_string(), format!("x-{}", n - 1))));
}

#[test]
fn annotation_actions_share_the_objects_they_copy() {
    // Fifty links whose URI actions all point at one string object.
    let n = 50;
    let links: Vec<String> =
        (0..n).map(|_| format!("<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A << /S /URI /URI {} 0 R >> >>", n + 5)).collect();
    let src = page_with_annots(&links, &["(https://example.com/shared)".into()], "");
    let out = extract_pages(&src, &[0]).unwrap();
    let copies = out
        .object_numbers()
        .into_iter()
        .filter(|&o| out.get(ObjRef::new(o, out.generation(o))).as_string().is_some_and(|s| s.bytes == b"https://example.com/shared"))
        .count();
    assert_eq!(copies, 1, "the shared URI is copied once, not once per action");
}

#[test]
fn name_trees_with_shared_kids_are_walked_once_per_node() {
    // Each level lists the next one twice: 2^depth paths through `depth` nodes.
    let depth = 10;
    let first = 6; // after the catalog, pages, page, content and the link
    let mut extra = Vec::new();
    for tree in 0..2 {
        let base = first + tree * (depth + 1);
        for level in 0..depth {
            let next = base + level + 1;
            extra.push(format!("<< /Kids [{next} 0 R {next} 0 R] >>"));
        }
        extra.push(if tree == 0 {
            "<< /Names [(a.txt) << /Type /Filespec /F (a.txt) >>] >>".into()
        } else {
            "<< /Names [(other) [3 0 R /Fit]] >>".into()
        });
    }
    let link = "<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /Dest (missing) >>".to_string();
    let catalog_extra = format!("/Names << /EmbeddedFiles {first} 0 R /Dests {} 0 R >>", first + depth + 1);
    let src = page_with_annots(&[link], &extra, &catalog_extra);
    take_work();
    let out = combine(&[("a", &src)]).unwrap();
    let work = take_work();
    assert!(work <= 8 * (depth + 1), "{work} name-tree nodes visited");
    let names = out.resolve(catalog(&out).get(b"Names").unwrap()).as_dict().cloned().unwrap();
    let tree = out.resolve(names.get(b"EmbeddedFiles").unwrap()).as_dict().cloned().unwrap();
    assert_eq!(tree.get(b"Names").and_then(|n| n.as_array()).map(Vec::len), Some(2), "one attachment, listed once");
}

#[test]
fn a_form_drawing_itself_repeatedly_is_analysed_once() {
    let form = "/F Do /F Do /F Do /F Do";
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /XObject << /F 5 0 R /Unused 6 0 R >> >> >>".into(),
        "<< /Length 5 >>\nstream\n/F Do\nendstream".into(),
        format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /XObject << /F 5 0 R >> >> /Length {} >>\nstream\n{form}\nendstream",
            form.len()
        ),
        "<< /Type /XObject /Subtype /Form /BBox [0 0 10 10] /Length 0 >>\nstream\n\nendstream".into(),
    ];
    let src = open(build(&b, "/Root 1 0 R"));
    take_work();
    let out = extract_pages(&src, &[0]).unwrap();
    let work = take_work();
    assert!(work <= 4, "{work} form analyses");
    let page = out.get(pages(&out).unwrap()[0].obj).as_dict().cloned().unwrap();
    let res = out.resolve(page.get(b"Resources").unwrap()).as_dict().cloned().unwrap();
    let x = out.resolve(res.get(b"XObject").unwrap()).as_dict().cloned().unwrap();
    assert!(x.contains(b"F") && !x.contains(b"Unused"), "pruning still keeps what is drawn and drops the rest");
}

/// One page whose /Contents names the same 1 MiB stream 300 times (a few kilobytes on disk).
fn repeated_contents() -> Document {
    use pdfcraft_cos::{Dict, Stream};
    let mut doc = Document::new_empty();
    let stream = doc.add(Stream::flate(Dict::new(), &vec![b' '; 1 << 20]));
    let pages = doc.add(Object::Null);
    let page: Dict = [
        (b"Type".to_vec(), Object::Name(b"Page".to_vec())),
        (b"Parent".to_vec(), Object::Ref(pages)),
        (b"MediaBox".to_vec(), Object::Array([0, 0, 200, 300].map(Object::from).to_vec())),
        (b"Contents".to_vec(), Object::Array(vec![Object::Ref(stream); 300])),
    ]
    .into_iter()
    .collect();
    let page = doc.add(page);
    let tree: Dict = [
        (b"Type".to_vec(), Object::Name(b"Pages".to_vec())),
        (b"Kids".to_vec(), Object::Array(vec![Object::Ref(page)])),
        (b"Count".to_vec(), Object::Int(1)),
    ]
    .into_iter()
    .collect();
    doc.set(pages, Object::Dict(tree));
    let catalog: Dict = [(b"Type".to_vec(), Object::Name(b"Catalog".to_vec())), (b"Pages".to_vec(), Object::Ref(pages))].into_iter().collect();
    let catalog = doc.add(catalog);
    doc.trailer_mut().set(b"Root".to_vec(), Object::Ref(catalog));
    doc
}

#[test]
fn repeated_content_references_share_one_decode_budget() {
    let src = repeated_contents();
    let mut dst = Document::new_empty();
    if let Ok((form, _)) = page_as_form(&mut dst, &src, 0) {
        let Object::Stream(s) = &*dst.get(form) else { panic!("not a stream") };
        let len = s.decoded().unwrap().len();
        assert!(len <= pdfcraft_cos::MAX_PAGE_DECODED + 300, "{len} bytes");
    }
}

/// #3193: the bookmark list reads a bookmark's colour and style as the inspector does (ISO
/// 32000-2 §12.3.3, Tables 151 and 152): `/C` within 0–1, black and malformed values as none,
/// `/F` bit 1 italic and bit 2 bold, through indirect values too.
#[test]
fn bookmarks_report_their_colour_and_style() {
    let mut objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 400] >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Type /Outlines /First 5 0 R /Last 10 0 R /Count 6 >>".into(),
    ];
    objs.push("<< /Title (Plain) /Parent 4 0 R /Next 6 0 R /Dest [3 0 R /Fit] >>".into()); // 5
    objs.push("<< /Title (Red bold) /Parent 4 0 R /Prev 5 0 R /Next 7 0 R /Dest [3 0 R /Fit] /C [1 0 0] /F 2 >>".into()); // 6
    objs.push("<< /Title (Indirect) /Parent 4 0 R /Prev 6 0 R /Next 8 0 R /Dest [3 0 R /Fit] /C 11 0 R /F 12 0 R >>".into()); // 7
    objs.push("<< /Title (Black) /Parent 4 0 R /Prev 7 0 R /Next 9 0 R /Dest [3 0 R /Fit] /C [0 0 0] /F 1 >>".into()); // 8
    objs.push("<< /Title (Bright) /Parent 4 0 R /Prev 8 0 R /Next 10 0 R /Dest [3 0 R /Fit] /C [2 -1 0.5] >>".into()); // 9
    objs.push("<< /Title (Short) /Parent 4 0 R /Prev 9 0 R /Dest [3 0 R /Fit] /C [1 0] /F 4 >>".into()); // 10
    objs.push("[0 0.5 0]".into()); // 11
    objs.push("3".into()); // 12
    let doc = open(build(&objs, "/Root 1 0 R"));
    let got: Vec<(String, Option<[f32; 3]>, bool, bool)> = bookmarks(&doc).into_iter().map(|b| (b.title, b.color, b.italic, b.bold)).collect();
    assert_eq!(
        got,
        [
            ("Plain".into(), None, false, false),
            ("Red bold".into(), Some([1.0, 0.0, 0.0]), false, true),
            ("Indirect".into(), Some([0.0, 0.5, 0.0]), true, true),
            ("Black".into(), None, true, false),
            ("Bright".into(), Some([1.0, 0.0, 0.5]), false, false),
            ("Short".into(), None, false, false),
        ]
    );
}

/// #3193: setting a bookmark's style and colour writes what ISO 32000-2 §12.3.3 asks for: bold is
/// `/F 2` and bold italic `/F 3` (Table 152), and neither removes `/F`, whose default is 0; a
/// style change leaves `/C` alone and a colour change leaves `/F`; black, the default (Table 151),
/// removes `/C`. Bits the table doesn't define are kept, and a colour outside 0–1 is refused.
#[test]
fn bookmark_style_and_colour_are_set_as_the_table_says() {
    let mut objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 400] >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Type /Outlines /First 5 0 R /Last 6 0 R /Count 2 >>".into(),
    ];
    objs.push("<< /Title (Plain) /Parent 4 0 R /Next 6 0 R /Dest [3 0 R /Fit] >>".into()); // 5
    objs.push("<< /Title (Odd bits) /Parent 4 0 R /Prev 5 0 R /Dest [3 0 R /Fit] /F 4 /C [1 0 0] >>".into()); // 6
    let mut doc = open(build(&objs, "/Root 1 0 R"));
    let raw = |doc: &Document, n: u32, key: &[u8]| doc.get(ObjRef { num: n, generation: 0 }).as_dict().and_then(|d| d.get(key).cloned());
    crate::set_bookmark_style(&mut doc, &[0], Some(true), None).unwrap();
    assert_eq!(raw(&doc, 5, b"F"), Some(Object::Int(2)), "bold");
    crate::set_bookmark_style(&mut doc, &[0], None, Some(true)).unwrap();
    assert_eq!(raw(&doc, 5, b"F"), Some(Object::Int(3)), "bold italic");
    crate::set_bookmark_color(&mut doc, &[0], Some([0.0, 0.5, 0.0])).unwrap();
    assert_eq!(raw(&doc, 5, b"C"), Some(Object::Array(vec![Object::Real(0.0), Object::Real(0.5), Object::Real(0.0)])));
    assert_eq!(raw(&doc, 5, b"F"), Some(Object::Int(3)), "a colour change keeps the style");
    crate::set_bookmark_style(&mut doc, &[0], Some(false), Some(false)).unwrap();
    assert_eq!(raw(&doc, 5, b"F"), None, "regular removes /F");
    assert!(raw(&doc, 5, b"C").is_some(), "a style change keeps the colour");
    crate::set_bookmark_color(&mut doc, &[0], Some([0.0; 3])).unwrap();
    assert_eq!(raw(&doc, 5, b"C"), None, "black removes /C");

    crate::set_bookmark_style(&mut doc, &[1], Some(true), None).unwrap();
    assert_eq!(raw(&doc, 6, b"F"), Some(Object::Int(6)), "bold beside a bit the table doesn't define");
    crate::set_bookmark_style(&mut doc, &[1], Some(false), Some(false)).unwrap();
    assert_eq!(raw(&doc, 6, b"F"), Some(Object::Int(4)), "that bit stays");
    crate::set_bookmark_color(&mut doc, &[1], None).unwrap();
    assert_eq!(raw(&doc, 6, b"C"), None, "no colour is black");
    for bad in [[1.5, 0.0, 0.0], [0.0, -0.1, 0.0], [f64::NAN, 0.0, 0.0]] {
        assert_eq!(crate::set_bookmark_color(&mut doc, &[1], Some(bad)), Err(crate::OutlineError::BadColor), "{bad:?}");
    }
    assert_eq!(crate::set_bookmark_style(&mut doc, &[7], Some(true), None), Err(crate::OutlineError::NoSuchBookmark(vec![7])));

    crate::set_bookmark_style(&mut doc, &[0], Some(true), Some(true)).unwrap();
    crate::set_bookmark_color(&mut doc, &[0], Some([0.0, 0.0, 1.0])).unwrap();
    let saved = open(write_full(&doc, &SaveOptions::default()).unwrap());
    let got: Vec<_> = bookmarks(&saved).into_iter().map(|b| (b.color, b.bold, b.italic)).collect();
    assert_eq!(got, [(Some([0.0, 0.0, 1.0]), true, true), (None, false, false)]);
}

/// Gives the fixture a PDF 2.0 document part tree (`/DPartRoot`, ISO 32000-2 14.12) that starts
/// at its first page, and a structure tree root listing a structure namespace (14.7.4).
fn with_document_parts_and_namespaces() -> Document {
    // PDF 2.0, the version these structures belong to.
    let mut bytes = fixture();
    let header = bytes.get_mut(..8).expect("the fixture starts with its header");
    assert!(header.starts_with(b"%PDF-1."));
    header.copy_from_slice(b"%PDF-2.0");
    let mut doc = open(bytes);
    let first = pages(&doc).unwrap()[0].obj;
    let mut namespace = pdfcraft_cos::Dict::new();
    namespace.set(b"Type".to_vec(), Object::name("Namespace"));
    namespace.set(b"NS".to_vec(), Object::String(pdfcraft_cos::PdfString { bytes: b"http://iso.org/pdf2/ssn".to_vec(), hex: false }));
    let namespace = doc.add(Object::Dict(namespace));
    let mut tree = pdfcraft_cos::Dict::new();
    tree.set(b"Type".to_vec(), Object::name("StructTreeRoot"));
    tree.set(b"Namespaces".to_vec(), Object::Array(vec![Object::Ref(namespace)]));
    let tree = doc.add(Object::Dict(tree));
    let mut part = pdfcraft_cos::Dict::new();
    part.set(b"Type".to_vec(), Object::name("DPart"));
    part.set(b"Start".to_vec(), Object::Ref(first));
    part.set(b"End".to_vec(), Object::Ref(first));
    let part = doc.add(Object::Dict(part));
    let mut root = pdfcraft_cos::Dict::new();
    root.set(b"Type".to_vec(), Object::name("DPartRoot"));
    root.set(b"DPartRootNode".to_vec(), Object::Ref(part));
    let root = doc.add(Object::Dict(root));
    doc.update_dict(doc.root().unwrap(), |c| {
        c.set(b"DPartRoot".to_vec(), Object::Ref(root));
        c.set(b"StructTreeRoot".to_vec(), Object::Ref(tree));
    })
    .unwrap();
    doc
}

fn assert_document_parts_and_namespaces_kept(doc: &Document, what: &str) {
    let catalog = doc.get(doc.root().unwrap());
    let catalog = catalog.as_dict().unwrap();
    let root = doc.resolve(catalog.get(b"DPartRoot").unwrap_or_else(|| panic!("{what}: /DPartRoot dropped")));
    let node = doc.resolve(root.as_dict().unwrap().get(b"DPartRootNode").unwrap_or_else(|| panic!("{what}: /DPartRootNode dropped")));
    let part = node.as_dict().unwrap();
    assert_eq!(part.name(b"Type"), Some(&b"DPart"[..]), "{what}");
    assert!(part.get(b"Start").is_some(), "{what}: /Start dropped");
    assert!(part.get(b"End").is_some(), "{what}: /End dropped");
    let tree = doc.resolve(catalog.get(b"StructTreeRoot").unwrap_or_else(|| panic!("{what}: /StructTreeRoot dropped")));
    let namespaces = doc.resolve(tree.as_dict().unwrap().get(b"Namespaces").unwrap_or_else(|| panic!("{what}: /Namespaces dropped")));
    let namespace = doc.resolve(&namespaces.as_array().unwrap()[0]);
    assert_eq!(namespace.as_dict().unwrap().name(b"Type"), Some(&b"Namespace"[..]), "{what}");
    assert_eq!(namespace.as_dict().unwrap().get(b"NS").unwrap().as_string().unwrap().bytes, b"http://iso.org/pdf2/ssn", "{what}: /NS changed");
    assert_eq!(doc.version(), "2.0", "{what}: the PDF 2.0 version is kept");
}

#[test]
fn page_edits_keep_document_parts_and_structure_namespaces() {
    type Edit = fn(&mut Document);
    let edits: [(&str, Edit); 6] = [
        ("rotate", |d| rotate_pages(d, &[0], 90).unwrap()),
        ("move", |d| move_pages(d, &[0], 2).unwrap()),
        ("reverse", |d| reverse_pages(d, &[0, 1, 2]).unwrap()),
        ("duplicate", |d| duplicate_pages(d, &[0], None).unwrap()),
        ("insert blank", |d| {
            let _ = insert_blank_page(d, 0, 100.0, 100.0).unwrap();
        }),
        // The part's start page goes: the keys stay, the page reference is the part's to repair.
        ("delete", |d| delete_pages(d, &[0]).unwrap()),
    ];
    for (what, edit) in edits {
        let mut doc = with_document_parts_and_namespaces();
        let base = write_full(&doc, &SaveOptions::default()).unwrap();
        assert_document_parts_and_namespaces_kept(&open(base.clone()), "setup");
        doc = open(base);
        edit(&mut doc);
        assert_document_parts_and_namespaces_kept(&doc, what);
        assert_document_parts_and_namespaces_kept(
            &open(write_incremental(&doc, &SaveOptions::default()).unwrap()),
            &format!("{what}, incremental save"),
        );
        assert_document_parts_and_namespaces_kept(&open(write_full(&doc, &SaveOptions::default()).unwrap()), &format!("{what}, full save"));
    }
}

/// #3502: a `/Contents` array's pieces are joined with a space, as the renderer joins them, so a
/// `%` comment at the end of a piece without a line break continues through the first line of the
/// next piece in both resource pruning (`extract_pages`) and `page_as_form`.
#[test]
fn contents_pieces_join_with_a_space_across_a_trailing_comment() {
    let build_doc = |first_piece: &str| {
        let b: Vec<String> = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".into(),                                        // 1
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >>".into(),        // 2
            "<< /Type /Page /Parent 2 0 R /Contents [4 0 R 5 0 R] /Resources 6 0 R >>".into(), // 3
            raw(first_piece),                                                                  // 4
            raw("/ImHidden Do\n/ImShown Do"),                                                  // 5
            "<< /XObject << /ImHidden 7 0 R /ImShown 8 0 R >> >>".into(),                      // 6
            image(1, 1, 1),                                                                    // 7
            image(1, 1, 1),                                                                    // 8
        ];
        open(build(&b, "/Root 1 0 R"))
    };

    // Without a trailing newline in piece 1, `% comment` swallows `/ImHidden Do` on the first
    // line of piece 2; only `/ImShown` is drawn.
    let commented = build_doc("q Q % comment without newline");
    let extracted = full_roundtrip(&extract_pages(&commented, &[0]).unwrap());
    assert_eq!(xobject_names(&extracted, 0), [b"ImShown".to_vec()], "commented-out Do across piece boundary is pruned");

    let mut form_dst = Document::new_empty();
    let (form_ref, _) = crate::page_as_form(&mut form_dst, &commented, 0).unwrap();
    let Object::Stream(form_stream) = &*form_dst.get(form_ref) else { panic!("form stream") };
    let form_bytes = form_stream.decoded().unwrap();
    let drawn: Vec<Vec<u8>> = pdfcraft_content::parse(&form_bytes)
        .ops
        .into_iter()
        .filter(|op| op.op == b"Do")
        .filter_map(|op| op.operands.first().and_then(Object::as_name).map(|n| n.to_vec()))
        .collect();
    assert_eq!(drawn, [b"ImShown".to_vec()], "page_as_form keeps the first line of piece 2 inside the comment");

    // Control: when piece 1 ends the comment with a newline, `/ImHidden Do` on piece 2's first
    // line is a real operator and stays in both the extracted page and the form XObject.
    let newline_ended = build_doc("q Q % comment with newline\n");
    let extracted_ctrl = full_roundtrip(&extract_pages(&newline_ended, &[0]).unwrap());
    let mut ctrl_names = xobject_names(&extracted_ctrl, 0);
    ctrl_names.sort();
    assert_eq!(ctrl_names, [b"ImHidden".to_vec(), b"ImShown".to_vec()]);
}

/// Intro (closed, red bold, at a point) > [Part (italic, FitH), Figure (FitR)], then Results.
fn outline_source() -> Document {
    let mut d = doc_a();
    crate::add_bookmark_at(&mut d, &[], 0, "Intro", 0, Some(20.0), Some(380.0)).unwrap();
    crate::add_bookmark(&mut d, &[0], 0, "Part", 1).unwrap();
    crate::add_bookmark(&mut d, &[0], 1, "Figure", 2).unwrap();
    crate::add_bookmark(&mut d, &[], 1, "Results", 2).unwrap();
    crate::set_bookmark_color(&mut d, &[0], Some([1.0, 0.0, 0.0])).unwrap();
    crate::set_bookmark_style(&mut d, &[0], Some(true), None).unwrap();
    crate::set_bookmark_style(&mut d, &[0, 0], None, Some(true)).unwrap();
    let pages = crate::walk(&d).unwrap();
    let (part, figure) = (crate::bookmarks(&d)[1].obj, crate::bookmarks(&d)[2].obj);
    let dest = |p: usize, rest: Vec<Object>| Object::Array([vec![Object::Ref(pages[p].0)], rest].concat());
    d.update_dict(part, |x| x.set(b"Dest".to_vec(), dest(1, vec![Object::name("FitH"), Object::Real(300.0)]))).unwrap();
    let fit_r = vec![Object::name("FitR"), Object::Int(10), Object::Int(20), Object::Int(110), Object::Int(220)];
    d.update_dict(figure, |x| x.set(b"Dest".to_vec(), dest(2, fit_r))).unwrap();
    crate::set_bookmark_open(&mut d, &[0], false).unwrap();
    d
}

fn node(title: &str, page: Option<usize>, children: Vec<crate::OutlineNode>) -> crate::OutlineNode {
    crate::OutlineNode { title: title.into(), page, view: None, color: None, italic: false, bold: false, open: true, children }
}

/// The `/Prev`, `/Last` and `/Parent` links agree with `/First`–`/Next` at every level.
fn assert_back_links(doc: &Document) {
    let root = doc.get(doc.root().unwrap()).as_dict().unwrap().reference(b"Outlines").unwrap();
    let mut stack = vec![root];
    while let Some(parent) = stack.pop() {
        let kids = crate::outline::children(doc, parent);
        let d = doc.get(parent).as_dict().cloned().unwrap();
        assert_eq!(d.reference(b"Last"), kids.last().copied());
        for (i, k) in kids.iter().enumerate() {
            let kd = doc.get(*k).as_dict().cloned().unwrap();
            assert_eq!(kd.reference(b"Prev"), i.checked_sub(1).map(|p| kids[p]));
            assert_eq!(kd.reference(b"Parent"), Some(parent));
        }
        stack.extend(kids);
    }
}

#[test]
fn an_exported_outline_imports_into_another_document_unchanged() {
    let source = outline_source();
    let tree = crate::outline_tree(&source, &[]).unwrap();
    assert!(!tree.truncated);
    let intro = &tree.nodes[0];
    assert_eq!((intro.title.as_str(), intro.page, intro.open, intro.bold, intro.color), ("Intro", Some(0), false, true, Some([1.0, 0.0, 0.0])));
    assert_eq!(intro.view, Some((b"XYZ".to_vec(), vec![Some(20.0), Some(380.0), None])));
    assert_eq!(intro.children[0].view, Some((b"FitH".to_vec(), vec![Some(300.0)])));
    assert!(intro.children[0].italic);
    assert_eq!(intro.children[1].view, Some((b"FitR".to_vec(), vec![Some(10.0), Some(20.0), Some(110.0), Some(220.0)])));

    let mut target = doc_a();
    assert_eq!(crate::import_outline(&mut target, &[], 0, &tree.nodes, false).unwrap(), vec![0]);
    assert_back_links(&target);
    let back = full_roundtrip(&target);
    assert_eq!(crate::outline_tree(&back, &[]).unwrap(), tree);
    // The closed parent stays closed: its count is negative and the root counts only what shows.
    let first = crate::bookmarks(&back)[0].obj;
    let root = back.get(back.root().unwrap()).as_dict().unwrap().reference(b"Outlines").unwrap();
    assert_eq!((count_of(&back, first), count_of(&back, root)), (Some(-2), Some(2)));
}

#[test]
fn a_subtree_exports_alone_and_a_missing_path_is_refused() {
    let source = outline_source();
    let tree = crate::outline_tree(&source, &[0]).unwrap();
    assert_eq!(tree.nodes.len(), 1);
    assert_eq!(tree.nodes[0].title, "Intro");
    assert_eq!(tree.nodes[0].children.iter().map(|n| n.title.as_str()).collect::<Vec<_>>(), ["Part", "Figure"]);
    let leaf = crate::outline_tree(&source, &[0, 1]).unwrap();
    assert_eq!((leaf.nodes.len(), leaf.nodes[0].title.as_str()), (1, "Figure"));
    assert_eq!(crate::outline_tree(&source, &[5]), Err(crate::OutlineError::NoSuchBookmark(vec![5])));
    assert_eq!(crate::outline_tree(&doc_a(), &[]).unwrap().nodes, []);
}

#[test]
fn a_page_the_target_lacks_keeps_its_bookmark_without_a_destination() {
    let tree = crate::outline_tree(&outline_source(), &[]).unwrap();
    // doc_b has two pages: Figure and Results go to the third.
    assert_eq!(crate::unmapped(&tree.nodes, 2), [vec![0, 1], vec![1]]);
    let mut target = doc_b();
    crate::import_outline(&mut target, &[], 0, &tree.nodes, false).unwrap();
    let listed = crate::bookmarks(&target);
    assert_eq!(titles(&listed), ["Intro[Part,Figure]", "Results"]);
    assert_eq!(listed.iter().map(|b| b.page).collect::<Vec<_>>(), [Some(0), Some(1), None, None]);
    assert!(target.get(listed[2].obj).as_dict().unwrap().get(b"Dest").is_none());
}

#[test]
fn a_fit_r_missing_a_value_copies_with_its_page_and_no_view() {
    let mut src = outline_source();
    let pages = crate::walk(&src).unwrap();
    let figure = crate::bookmarks(&src)[2].obj;
    let short = vec![Object::Ref(pages[2].0), Object::name("FitR"), Object::Int(10), Object::Int(20), Object::Int(110)];
    src.update_dict(figure, |x| x.set(b"Dest".to_vec(), Object::Array(short))).unwrap();
    let tree = crate::outline_tree(&src, &[]).unwrap();
    let copied = &tree.nodes[0].children[1];
    assert_eq!((copied.page, copied.view.clone()), (Some(2), None));
    let mut target = doc_a();
    crate::import_outline(&mut target, &[], 0, &tree.nodes, false).unwrap();
    assert_eq!(titles(&crate::bookmarks(&target)), ["Intro[Part,Figure]", "Results"]);
}

#[test]
fn an_import_appends_inserts_or_replaces() {
    let mut d = doc_a();
    crate::add_bookmark(&mut d, &[], 0, "Old A", 0).unwrap();
    crate::add_bookmark(&mut d, &[], 1, "Old B", 1).unwrap();
    let new = [node("New", Some(2), vec![node("Child", Some(1), vec![])])];
    assert_eq!(crate::import_outline(&mut d, &[], 1, &new, false).unwrap(), vec![1]);
    assert_eq!(titles(&crate::bookmarks(&d)), ["Old A", "New[Child]", "Old B"]);
    assert_eq!(crate::import_outline(&mut d, &[2], 9, &new, false).unwrap(), vec![2, 0]);
    assert_eq!(titles(&crate::bookmarks(&d)), ["Old A", "New[Child]", "Old B[New[Child]]"]);
    assert_back_links(&d);
    assert_eq!(crate::import_outline(&mut d, &[], 5, &new, true).unwrap(), vec![0]);
    assert_eq!(titles(&crate::bookmarks(&d)), ["New[Child]"]);
    assert_back_links(&d);
    let refused = crate::import_outline(&mut d, &[0], 0, &new, true);
    assert!(matches!(refused, Err(crate::OutlineError::Organize(crate::OrganizeError::Invalid(_)))));
    assert_eq!(crate::import_outline(&mut d, &[], 0, &[], false), Err(crate::OutlineError::NothingToImport));
    assert_eq!(crate::import_outline(&mut d, &[7], 0, &new, false), Err(crate::OutlineError::NoSuchBookmark(vec![7])));
}

#[test]
fn a_bad_node_anywhere_refuses_the_whole_import() {
    let mut d = doc_a();
    crate::add_bookmark(&mut d, &[], 0, "Kept", 0).unwrap();
    let before = crate::bookmarks(&d);
    let with = |f: &dyn Fn(&mut crate::OutlineNode)| {
        let mut child = node("Child", Some(0), vec![]);
        f(&mut child);
        vec![node("Fine", Some(0), vec![]), node("Parent", Some(0), vec![child])]
    };
    let bad_views: [(&[u8], Vec<Option<f64>>); 4] =
        [(b"XYZ", vec![Some(1.0)]), (b"Zoom", vec![]), (b"FitH", vec![Some(f64::NAN)]), (b"FitR", vec![Some(0.0), None, Some(1.0), Some(1.0)])];
    for (kind, values) in bad_views {
        let nodes = with(&|n| n.view = Some((kind.to_vec(), values.clone())));
        assert_eq!(crate::import_outline(&mut d, &[], 0, &nodes, false), Err(crate::OutlineError::BadView(vec![1, 0])), "{kind:?}");
    }
    let nodes = with(&|n| n.color = Some([0.5, 1.5, 0.0]));
    assert_eq!(crate::import_outline(&mut d, &[], 0, &nodes, false), Err(crate::OutlineError::BadColor));
    let nodes = with(&|n| n.color = Some([f32::NAN, 0.0, 0.0]));
    assert_eq!(crate::import_outline(&mut d, &[], 0, &nodes, false), Err(crate::OutlineError::BadColor));
    let mut deep = node("Leaf", None, vec![]);
    for _ in 0..33 {
        deep = node("Level", None, vec![deep]);
    }
    assert_eq!(crate::import_outline(&mut d, &[], 0, &[deep], false), Err(crate::OutlineError::TooDeep(33)));
    // 33 levels fit at the top, but not under a bookmark: the deepest would be past the listing.
    let mut chain = node("Leaf", None, vec![]);
    for _ in 0..32 {
        chain = node("Level", None, vec![chain]);
    }
    assert_eq!(crate::import_outline(&mut d, &[0], 0, std::slice::from_ref(&chain), false), Err(crate::OutlineError::TooDeep(33)));
    let many = vec![node("Item", None, vec![]); crate::MAX_IMPORT_BOOKMARKS + 1];
    assert_eq!(crate::import_outline(&mut d, &[], 0, &many, false), Err(crate::OutlineError::TooManyBookmarks(crate::MAX_IMPORT_BOOKMARKS)));
    assert_eq!(crate::bookmarks(&d), before);
}

#[test]
fn an_outline_deeper_than_the_listing_is_exported_as_truncated() {
    let tree = crate::outline_tree(&open(deep_outline(40)), &[]).unwrap();
    assert!(tree.truncated);
    assert!(!crate::outline_tree(&open(deep_outline(33)), &[]).unwrap().truncated);
    // What was exported imports back: 33 levels is the deepest an import takes.
    let mut target = doc_a();
    crate::import_outline(&mut target, &[], 0, &tree.nodes, false).unwrap();
    assert_eq!(crate::bookmarks(&target).len(), 33);
}

/// Does any object in `doc` hold the string value `/V (value)`?
fn holds_value(doc: &Document, value: &str) -> bool {
    doc.scan_objects()
        .into_iter()
        .any(|(_, o)| o.as_dict().and_then(|d| d.get(b"V")).and_then(|v| v.as_string()).is_some_and(|v| v.to_text() == value))
}

/// A field with a widget on each of two pages: extracting one page brings that widget and the field
/// above it, not the other kid with its value.
#[test]
fn extracting_a_page_leaves_the_rest_of_its_form_behind() {
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [7 0 R] >> >>".into(), // 1
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] >>".into(), // 2
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Annots [8 0 R] >>".into(),   // 3
        "<< /Type /Page /Parent 2 0 R /Contents 6 0 R /Annots [9 0 R] >>".into(),   // 4
        raw("0 0 m"),                                                               // 5
        raw("0 0 m"),                                                               // 6
        "<< /T (applicant) /Kids [8 0 R 9 0 R] >>".into(),                          // 7
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /V (Ada) /Parent 7 0 R /P 3 0 R /Rect [0 0 50 20] >>".into(), // 8
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (salary) /V (90000) /Parent 7 0 R /P 4 0 R /Rect [0 0 50 20] >>".into(), // 9
    ];
    let src = open(build(&b, "/Root 1 0 R"));
    let part = full_roundtrip(&extract_pages(&src, &[0]).unwrap());
    assert!(holds_value(&part, "Ada"));
    assert!(!holds_value(&part, "90000"), "the widget on the page left behind came along with its value");
    let form = part.resolve(catalog(&part).get(b"AcroForm").unwrap()).as_dict().cloned().unwrap();
    let fields = part.resolve(form.get(b"Fields").unwrap()).as_array().cloned().unwrap();
    let applicant = part.resolve(&fields[0]).as_dict().cloned().unwrap();
    assert_eq!(part.resolve(applicant.get(b"Kids").unwrap()).as_array().map(Vec::len), Some(1));
    // With every page, the whole form comes along, as before.
    let both = full_roundtrip(&extract_pages(&src, &[0, 1]).unwrap());
    assert!(holds_value(&both, "Ada") && holds_value(&both, "90000"));
}

/// A form action lists fields by reference: extracting the page with the button doesn't bring
/// the fields it submits, resets or hides from a page left behind, with their values. A reset of
/// only those would list none, which reads as every field: it goes.
#[test]
fn a_form_action_brings_no_field_from_a_page_left_behind() {
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [7 0 R 8 0 R] >> >>".into(), // 1
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] >>".into(), // 2
        "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Annots [7 0 R 9 0 R] >>".into(),   // 3
        "<< /Type /Page /Parent 2 0 R /Contents 6 0 R /Annots [8 0 R] >>".into(),         // 4
        raw("0 0 m"),                                                                     // 5
        raw("0 0 m"),                                                                     // 6
        "<< /Type /Annot /Subtype /Widget /FT /Btn /Ff 65536 /T (send) /P 3 0 R /Rect [0 0 50 20] \
         /A << /S /SubmitForm /F (https://example.invalid/) /Fields [8 0 R (other)] \
         /Next << /S /ResetForm /Fields [8 0 R] /Next << /S /Hide /T 8 0 R >> >> >> >>"
            .into(), // 7
        "<< /Type /Annot /Subtype /Widget /FT /Tx /T (salary) /V (90000) /P 4 0 R /Rect [0 0 50 20] >>".into(), // 8
        "<< /Type /Annot /Subtype /Widget /FT /Btn /Ff 65536 /T (clear) /P 3 0 R /Rect [0 30 50 50] \
         /A << /S /ResetForm /Fields [8 0 R] >> /AA << /U << /S /ResetForm /Flags 1 /Fields [8 0 R] >> >> >>"
            .into(), // 9
    ];
    let src = open(build(&b, "/Root 1 0 R"));
    let part = full_roundtrip(&extract_pages(&src, &[0]).unwrap());
    assert!(!holds_value(&part, "90000"), "a field the action lists came along from the page left behind");
    let submit = part.resolve(annots(&part, 0)[0].get(b"A").unwrap()).as_dict().cloned().unwrap();
    assert_eq!(part.resolve(submit.get(b"Fields").unwrap()).as_array().map(Vec::len), Some(1), "the field named by text stays");
    assert!(submit.get(b"Next").is_none_or(|n| part.resolve(n).as_dict().is_none()), "the reset after the submit would clear every field");
    let clear = annots(&part, 0).remove(1);
    assert!(clear.get(b"A").is_none(), "the clear button would clear every field");
    // A reset of every field but the one left behind still resets them all.
    let aa = part.resolve(clear.get(b"AA").unwrap()).as_dict().cloned().unwrap();
    assert!(aa.get(b"U").is_some_and(|u| part.resolve(u).as_dict().is_some()));
    // With every page, the actions still list it.
    assert!(holds_value(&full_roundtrip(&extract_pages(&src, &[0, 1]).unwrap()), "90000"));
}

/// The titles of a bookmark's children, each with its own children's titles.
fn bookmark_titles(doc: &Document, first: Option<pdfcraft_cos::ObjRef>) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let mut at = first;
    while let Some(r) = at {
        let item = doc.get(r).as_dict().cloned().unwrap();
        let title = item.get(b"Title").and_then(|t| t.as_string()).map(|t| t.to_text()).unwrap_or_default();
        let children = bookmark_titles(doc, item.reference(b"First")).into_iter().map(|(t, _)| t).collect();
        out.push((title, children));
        at = item.reference(b"Next");
    }
    out
}

/// Combining some of a file's pages brings only the bookmarks that lead to them (and those above
/// them), and none of the file's attachments.
#[test]
fn combining_some_pages_brings_only_their_bookmarks_and_no_attachments() {
    let source_bookmarks = |out: &Document| {
        let outlines = out.get(catalog(out).reference(b"Outlines").unwrap()).as_dict().cloned().unwrap();
        let file = out.get(outlines.reference(b"First").unwrap()).as_dict().cloned().unwrap();
        bookmark_titles(out, file.reference(b"First"))
    };
    let attachments = |out: &Document| {
        catalog(out).get(b"Names").map(|n| out.resolve(n)).is_some_and(|n| n.as_dict().is_some_and(|d| d.contains(b"EmbeddedFiles")))
    };
    // Page 1 only: "Section" leads to page 2.
    let first = full_roundtrip(&combine_selected(&[("C", &doc_c(), Some(&[0]))]).unwrap());
    assert_eq!(source_bookmarks(&first), [("Chapter one".to_string(), vec![])]);
    assert!(!attachments(&first), "an attachment came along with a selection of pages");
    // Page 2 only: "Chapter one" stays, as the way to "Section".
    let second = full_roundtrip(&combine_selected(&[("C", &doc_c(), Some(&[1]))]).unwrap());
    assert_eq!(source_bookmarks(&second), [("Chapter one".to_string(), vec!["Section".to_string()])]);
    // Every page: all of it, attachments too.
    let all = full_roundtrip(&combine_selected(&[("C", &doc_c(), Some(&[0, 1]))]).unwrap());
    assert_eq!(source_bookmarks(&all), [("Chapter one".to_string(), vec!["Section".to_string()])]);
    assert!(attachments(&all));
}

fn af_catalog(doc: &Document) -> pdfcraft_cos::Dict {
    doc.get(doc.root().unwrap()).as_dict().unwrap().clone()
}

#[test]
fn associated_files_are_written_as_filespecs_in_the_catalog_af_and_removed_again() {
    let mut doc = Document::open(Arc::new(fixture())).unwrap();
    assert!(associated_files(&doc).is_empty());
    let (key, index) = add_associated_file(&mut doc, "/tmp/data.csv", b"a,b\n1,2\n", "Data", Some("Table"), Some("text/csv"), None).unwrap();
    assert_eq!((key.as_str(), index), ("data.csv", 1));
    let (_, index) = add_associated_file(&mut doc, "source.xml", b"<a/>", "Source", None, None, None).unwrap();
    assert_eq!(index, 2);
    let mut doc = save_and_reopen(&doc);

    // The object contents: /AF is an array of Filespecs with the embedded stream and the relationship.
    let catalog = af_catalog(&doc);
    let list = doc.resolve(catalog.get(b"AF").unwrap()).as_array().cloned().unwrap();
    assert_eq!(list.len(), 2);
    let spec = doc.resolve(&list[0]);
    let spec = spec.as_dict().unwrap();
    assert_eq!(spec.get(b"Type"), Some(&Object::name("Filespec")));
    assert_eq!(spec.get(b"AFRelationship"), Some(&Object::name("Data")));
    assert_eq!(spec.get(b"F").and_then(Object::as_string).map(PdfString::to_text).as_deref(), Some("data.csv"));
    assert_eq!(spec.get(b"UF").and_then(Object::as_string).map(PdfString::to_text).as_deref(), Some("data.csv"));
    let ef = spec.get(b"EF").and_then(Object::as_dict).unwrap().get(b"F").cloned().unwrap();
    let ef = doc.resolve(&ef);
    let Object::Stream(stream) = ef.as_ref() else { panic!("the embedded file is a stream") };
    assert_eq!(stream.dict.get(b"Type"), Some(&Object::name("EmbeddedFile")));
    assert_eq!(stream.decoded().unwrap(), b"a,b\n1,2\n");
    let second = doc.resolve(&list[1]);
    assert_eq!(second.as_dict().unwrap().get(b"AFRelationship"), Some(&Object::name("Source")));

    let found = associated_files(&doc);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].relationship.as_deref(), Some("Data"));
    assert_eq!(found[0].name.as_deref(), Some("data.csv"));
    assert_eq!(found[0].description.as_deref(), Some("Table"));
    assert_eq!(found[0].mime.as_deref(), Some("text/csv"));
    assert_eq!(found[0].size, Some(8));
    assert!(found[0].embedded && found[0].page.is_none());
    // The files are attachments too.
    assert_eq!(attachment_keys(&doc), ["data.csv", "source.xml"]);

    // Removing the first drops its /AF entry and its attachment; the other stays.
    remove_associated_file(&mut doc, 1, false).unwrap();
    let doc2 = save_and_reopen(&doc);
    let left = associated_files(&doc2);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].name.as_deref(), Some("source.xml"));
    assert_eq!(attachment_keys(&doc2), ["source.xml"]);
    // Keeping the attachment removes only the association, and the last entry takes /AF with it.
    let mut doc2 = doc2;
    assert!(remove_associated_file(&mut doc2, 2, false).is_err());
    assert!(remove_associated_file(&mut doc2, 0, false).is_err());
    remove_associated_file(&mut doc2, 1, true).unwrap();
    let doc2 = save_and_reopen(&doc2);
    assert!(af_catalog(&doc2).get(b"AF").is_none());
    assert_eq!(attachment_keys(&doc2), ["source.xml"]);
}

#[test]
fn associated_files_refuse_before_changing_anything() {
    let mut doc = Document::open(Arc::new(fixture())).unwrap();
    for bad in ["Banana", "", "data", "EncryptedPayload"] {
        assert!(matches!(add_associated_file(&mut doc, "a.txt", b"x", bad, None, None, None), Err(OrganizeError::Invalid(_))), "{bad}");
    }
    assert!(add_associated_file(&mut doc, " / ", b"x", "Data", None, None, None).is_err());
    let big = vec![0u8; MAX_AF_BYTES + 1];
    assert!(add_associated_file(&mut doc, "big.bin", &big, "Data", None, None, None).is_err());
    assert!(attachment_keys(&doc).is_empty());
    assert!(af_catalog(&doc).get(b"AF").is_none());

    // A catalog /AF that is not an array is left alone.
    let root = doc.root().unwrap();
    doc.update_dict(root, |c| c.set(b"AF".to_vec(), Object::Int(3))).unwrap();
    assert!(add_associated_file(&mut doc, "a.txt", b"x", "Data", None, None, None).is_err());
    assert!(remove_associated_file(&mut doc, 1, false).is_err());
    assert_eq!(af_catalog(&doc).get(b"AF"), Some(&Object::Int(3)));
    assert!(attachment_keys(&doc).is_empty());
    assert!(associated_files(&doc).is_empty());

    // The array may not grow past the cap.
    let mut doc = Document::open(Arc::new(fixture())).unwrap();
    let filler: Vec<Object> = (0..MAX_AF_ENTRIES).map(|_| Object::Null).collect();
    doc.update_dict(root, |c| c.set(b"AF".to_vec(), Object::Array(filler))).unwrap();
    assert!(add_associated_file(&mut doc, "a.txt", b"x", "Data", None, None, None).is_err());
    assert!(attachment_keys(&doc).is_empty());
}

#[test]
fn page_level_associated_files_are_listed_with_their_page() {
    let mut doc = Document::open(Arc::new(fixture())).unwrap();
    let page = pages(&doc).unwrap()[1].obj;
    let mut spec = pdfcraft_cos::Dict::new();
    spec.set(b"Type".to_vec(), Object::name("Filespec"));
    spec.set(b"F".to_vec(), Object::String(PdfString::text("ext.xml")));
    spec.set(b"AFRelationship".to_vec(), Object::name("Schema"));
    doc.update_dict(page, |p| p.set(b"AF".to_vec(), Object::Array(vec![Object::Dict(spec), Object::Int(7)]))).unwrap();
    let found = associated_files(&doc);
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].page, found[0].index), (Some(1), 1));
    assert_eq!(found[0].relationship.as_deref(), Some("Schema"));
    assert_eq!(found[0].name.as_deref(), Some("ext.xml"));
    assert!(!found[0].embedded);
}

#[test]
fn entries_without_a_relationship_name_or_an_embedded_file_are_counted() {
    let mut doc = Document::open(Arc::new(fixture())).unwrap();
    // One embedded file with a relationship, then three links to outside files added by hand: one
    // with a relationship name, one whose relationship is a string, one with none; and a number.
    add_associated_file(&mut doc, "in.csv", b"a", "Data", None, None, None).unwrap();
    let link = |relationship: Option<Object>| {
        let mut spec = pdfcraft_cos::Dict::new();
        spec.set(b"Type".to_vec(), Object::name("Filespec"));
        spec.set(b"F".to_vec(), Object::String(PdfString::text("out.xml")));
        if let Some(r) = relationship {
            spec.set(b"AFRelationship".to_vec(), r);
        }
        Object::Dict(spec)
    };
    let extra = [link(Some(Object::name("Source"))), link(Some(Object::String(PdfString::text("Data")))), link(None), Object::Int(7)];
    let root = doc.root().unwrap();
    doc.update_dict(root, |c| {
        let mut list = c.get(b"AF").and_then(Object::as_array).cloned().unwrap_or_default();
        list.extend(extra);
        c.set(b"AF".to_vec(), Object::Array(list));
    })
    .unwrap();
    let listed = list_associated_files(&doc);
    assert_eq!(listed.files.len(), 4, "the number is skipped");
    assert_eq!((listed.without_relationship, listed.not_embedded), (2, 3));
    // Nothing listed: nothing counted.
    let empty = list_associated_files(&Document::open(Arc::new(fixture())).unwrap());
    assert_eq!((empty.files.len(), empty.without_relationship, empty.not_embedded), (0, 0, 0));
}

fn spec_body(name: &str) -> String {
    format!("<< /Type /Filespec /F ({name}) /UF ({name}) >>")
}

#[test]
fn listing_associated_files_is_bounded_and_reads_a_shared_array_once() {
    // One indirect /AF array of 256 files shared by 50 pages and the catalog: read once.
    let files = (0..MAX_AF_ENTRIES).map(|i| spec_body(&format!("f{i}"))).collect::<Vec<_>>().join(" ");
    let mut bodies = vec!["<< /Type /Catalog /Pages 2 0 R /AF 3 0 R >>".to_string()];
    let kids: String = (0..50).map(|i| format!("{} 0 R ", 4 + i)).collect();
    bodies.push(format!("<< /Type /Pages /Kids [{kids}] /Count 50 /MediaBox [0 0 10 10] >>"));
    bodies.push(format!("[{files}]"));
    for _ in 0..50 {
        bodies.push("<< /Type /Page /Parent 2 0 R /AF 3 0 R >>".to_string());
    }
    let doc = open(build(&bodies, "/Root 1 0 R"));
    let listed = list_associated_files(&doc);
    assert_eq!((listed.files.len(), listed.truncated), (MAX_AF_ENTRIES, false));
    assert!(listed.files.iter().all(|f| f.page.is_none()));

    // Ten pages with an array of their own each: 2,560 entries, cut at the row cap.
    let mut bodies = vec!["<< /Type /Catalog /Pages 2 0 R >>".to_string()];
    let kids: String = (0..10).map(|i| format!("{} 0 R ", 3 + i)).collect();
    bodies.push(format!("<< /Type /Pages /Kids [{kids}] /Count 10 /MediaBox [0 0 10 10] >>"));
    for _ in 0..10 {
        bodies.push(format!("<< /Type /Page /Parent 2 0 R /AF [{files}] >>"));
    }
    let doc = open(build(&bodies, "/Root 1 0 R"));
    let listed = list_associated_files(&doc);
    assert_eq!((listed.files.len(), listed.truncated), (MAX_AF_ROWS, true));
}

#[test]
fn a_shared_af_array_is_copied_not_changed_and_shared_files_keep_their_attachment() {
    let bodies = vec![
        "<< /Type /Catalog /Pages 2 0 R /AF 4 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 10 10] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /AF 4 0 R >>".to_string(),
        "[5 0 R]".to_string(),
        spec_body("old.txt"),
    ];
    let mut doc = open(build(&bodies, "/Root 1 0 R"));
    let page = pages(&doc).unwrap()[0].obj;
    add_associated_file(&mut doc, "new.txt", b"n", "Data", None, None, None).unwrap();
    let doc = save_and_reopen(&doc);
    // The catalog has both; the page still has the one array it had, untouched.
    let found = associated_files(&doc);
    assert_eq!(found.iter().filter(|f| f.page.is_none()).count(), 2);
    let on_page: Vec<_> = found.iter().filter(|f| f.page == Some(0)).collect();
    assert_eq!((on_page.len(), on_page[0].name.as_deref()), (1, Some("old.txt")));
    let page_af = doc.get(page).as_dict().unwrap().get(b"AF").cloned();
    assert_eq!(page_af.and_then(|o| o.as_ref()).map(|r| r.num), Some(4));
    let array = doc.get(pdfcraft_cos::ObjRef { num: 4, generation: 0 });
    assert_eq!(array.as_array().map(Vec::len), Some(1));

    // Removing the catalog's first entry (old.txt, which the page lists too) and the second.
    let mut doc = doc;
    assert!(associated_file_is_shared(&doc, 1).unwrap());
    assert!(!associated_file_is_shared(&doc, 2).unwrap());
    remove_associated_file(&mut doc, 1, false).unwrap();
    // new.txt is listed by nothing else: it goes with its attachment.
    assert!(remove_associated_file(&mut doc, 1, false).unwrap());
    assert!(attachment_keys(&doc).is_empty());
    let doc = save_and_reopen(&doc);
    let found = associated_files(&doc);
    assert_eq!((found.len(), found[0].page, found[0].name.as_deref()), (1, Some(0), Some("old.txt")));
    assert!(af_catalog(&doc).get(b"AF").is_none());
}

#[test]
fn a_file_listed_on_a_page_or_twice_keeps_its_attachment_and_deletion_goes_by_reference() {
    let mut doc = open(fixture());
    add_associated_file(&mut doc, "a.txt", b"a", "Data", None, None, None).unwrap();
    let spec = {
        let (_, list) = catalog_af_for_test(&doc);
        list[0].as_ref().unwrap()
    };
    // The same file listed on a page: removing it from the catalog keeps the attachment.
    let page = pages(&doc).unwrap()[0].obj;
    doc.update_dict(page, |p| p.set(b"AF".to_vec(), Object::Array(vec![Object::Ref(spec)]))).unwrap();
    assert!(!remove_associated_file(&mut doc, 1, false).unwrap());
    assert_eq!(attachment_keys(&doc), ["a.txt"]);

    // Listed twice in the catalog: removing one entry keeps the attachment, the last deletes it.
    let mut doc = open(fixture());
    add_associated_file(&mut doc, "a.txt", b"a", "Data", None, None, None).unwrap();
    let root = doc.root().unwrap();
    doc.update_dict(root, |c| c.set(b"AF".to_vec(), Object::Array(vec![Object::Ref(spec), Object::Ref(spec)]))).unwrap();
    assert!(!remove_associated_file(&mut doc, 1, false).unwrap());
    assert_eq!(attachment_keys(&doc), ["a.txt"]);
    assert!(remove_associated_file(&mut doc, 1, false).unwrap());
    assert!(attachment_keys(&doc).is_empty());

    // Two attachments whose keys read the same: only the one holding this specification goes.
    let mut doc = open(fixture());
    add_associated_file(&mut doc, "a.txt", b"first", "Data", None, None, None).unwrap();
    add_associated_file(&mut doc, "b.txt", b"second", "Data", None, None, None).unwrap();
    let mut all = crate::attachments::entries(&doc).0;
    let key = all[0].0.clone();
    all[1].0 = key;
    crate::import::set_attachments_exact(&mut doc, all.clone()).unwrap();
    assert!(remove_associated_file(&mut doc, 2, false).unwrap());
    let left = crate::attachments::entries(&doc).0;
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].1, all[0].1);
    assert_eq!(associated_files(&doc).len(), 1);
}

fn catalog_af_for_test(doc: &Document) -> ((), Vec<Object>) {
    let c = af_catalog(doc);
    ((), doc.resolve(c.get(b"AF").unwrap()).as_array().cloned().unwrap())
}

/// Adds a `/DPart` node (not yet linked to its children) and returns its reference.
fn add_part(doc: &mut Document, start_end: Option<(ObjRef, ObjRef)>) -> ObjRef {
    let mut part = pdfcraft_cos::Dict::new();
    part.set(b"Type".to_vec(), Object::name("DPart"));
    if let Some((start, end)) = start_end {
        part.set(b"Start".to_vec(), Object::Ref(start));
        part.set(b"End".to_vec(), Object::Ref(end));
    }
    doc.add(Object::Dict(part))
}

fn set_children(doc: &mut Document, node: ObjRef, children: &[ObjRef]) {
    let row = Object::Array(children.iter().map(|c| Object::Ref(*c)).collect());
    doc.update_dict(node, |d| d.set(b"DParts".to_vec(), Object::Array(vec![row]))).unwrap();
}

fn set_document_part_root(doc: &mut Document, node: ObjRef) {
    let mut root = pdfcraft_cos::Dict::new();
    root.set(b"Type".to_vec(), Object::name("DPartRoot"));
    root.set(b"DPartRootNode".to_vec(), Object::Ref(node));
    root.set(b"RecordLevel".to_vec(), Object::Int(1));
    let root = doc.add(Object::Dict(root));
    doc.update_dict(doc.root().unwrap(), |c| c.set(b"DPartRoot".to_vec(), Object::Ref(root))).unwrap();
}

#[test]
fn document_parts_are_listed_in_order_with_pages_and_metadata() {
    let mut doc = open(fixture());
    assert_eq!(list_document_parts(&doc), DocumentPartList::default(), "no /DPartRoot");
    let pages = pages(&doc).unwrap();
    let (p1, p2, p3) = (pages[0].obj, pages[1].obj, pages[2].obj);
    let top = add_part(&mut doc, None);
    let a = add_part(&mut doc, Some((p1, p1)));
    let b = add_part(&mut doc, Some((p2, p3)));
    let c = add_part(&mut doc, Some((p3, p3)));
    // The root lists a and b; a lists c; b lists a again (listed twice) and the root (a cycle).
    set_children(&mut doc, top, &[a, b]);
    set_children(&mut doc, a, &[c]);
    set_children(&mut doc, b, &[a, top]);
    let mut dpm = pdfcraft_cos::Dict::new();
    dpm.set(b"Kind".to_vec(), Object::String(pdfcraft_cos::PdfString::text("cover")));
    dpm.set(b"Copies".to_vec(), Object::Int(3));
    dpm.set(b"Nested".to_vec(), Object::Array(vec![]));
    doc.update_dict(a, |d| d.set(b"DPM".to_vec(), Object::Dict(dpm))).unwrap();
    set_document_part_root(&mut doc, top);

    let list = list_document_parts(&doc);
    assert!(list.has_root && !list.truncated);
    assert_eq!(list.record_level, Some(1));
    let rows: Vec<_> = list.parts.iter().map(|p| (p.object, p.parent, p.depth, p.start_page, p.end_page, p.children)).collect();
    assert_eq!(
        rows,
        [
            (top.num, None, 0, None, None, 2),
            (a.num, Some(top.num), 1, Some(1), Some(1), 1),
            (c.num, Some(a.num), 2, Some(3), Some(3), 0),
            (b.num, Some(top.num), 1, Some(2), Some(3), 2),
        ]
    );
    assert_eq!(list.skipped, 2, "a listed twice, the root reached again");
    let kind = list.parts[1].metadata.iter().find(|(k, _)| k == "Kind").map(|(_, v)| v.as_str());
    assert_eq!(kind, Some("cover"));
    let nested = list.parts[1].metadata.iter().find(|(k, _)| k == "Nested").map(|(_, v)| v.as_str());
    assert_eq!((nested, list.parts[1].metadata.len()), (Some("(array)"), 3));

    // It reads back from a saved file too.
    let saved = open(write_full(&doc, &SaveOptions::default()).unwrap());
    assert_eq!(list_document_parts(&saved).parts.len(), 4);
}

#[test]
fn a_part_whose_end_page_comes_before_its_start_page_is_flagged() {
    let mut doc = open(fixture());
    let pages = pages(&doc).unwrap();
    let (p1, p2, p3) = (pages[0].obj, pages[1].obj, pages[2].obj);
    let top = add_part(&mut doc, None);
    let backwards = add_part(&mut doc, Some((p3, p1)));
    let single = add_part(&mut doc, Some((p2, p2)));
    let forwards = add_part(&mut doc, Some((p1, p3)));
    // An /End that names no page of the document (the catalog) is not inverted.
    let doc_root = doc.root().unwrap();
    let unknown_end = add_part(&mut doc, Some((p3, doc_root)));
    set_children(&mut doc, top, &[backwards, single, forwards, unknown_end]);
    set_document_part_root(&mut doc, top);

    let list = list_document_parts(&doc);
    let flags: Vec<_> = list.parts.iter().map(|p| (p.object, p.range_inverted)).collect();
    assert_eq!(flags, [(top.num, false), (backwards.num, true), (single.num, false), (forwards.num, false), (unknown_end.num, false)]);
    assert_eq!(list.inverted_ranges, 1);
}

#[test]
fn a_part_whose_start_or_end_names_no_page_is_flagged_unresolved() {
    let mut doc = open(fixture());
    let pages = pages(&doc).unwrap();
    let (p1, p2) = (pages[0].obj, pages[1].obj);
    let top = add_part(&mut doc, None);
    let good = add_part(&mut doc, Some((p1, p2)));
    let none = add_part(&mut doc, None);
    let catalog = doc.root().unwrap();
    let end_is_catalog = add_part(&mut doc, Some((p1, catalog)));
    let start_is_direct = add_part(&mut doc, Some((p1, p2)));
    doc.update_dict(start_is_direct, |d| d.set(b"Start".to_vec(), Object::Int(1))).unwrap();
    // A null is the same as absent.
    let nulls = add_part(&mut doc, Some((p1, p2)));
    doc.update_dict(nulls, |d| {
        d.set(b"Start".to_vec(), Object::Null);
        d.set(b"End".to_vec(), Object::Null);
    })
    .unwrap();
    // A reference to an object that was never added is flagged, unlike a direct null.
    let dangling = add_part(&mut doc, Some((p1, p2)));
    doc.update_dict(dangling, |d| d.set(b"Start".to_vec(), Object::Ref(ObjRef { num: 99_999, generation: 0 }))).unwrap();
    set_children(&mut doc, top, &[good, none, end_is_catalog, start_is_direct, nulls, dangling]);
    set_document_part_root(&mut doc, top);

    let list = list_document_parts(&doc);
    let flags: Vec<_> = list.parts.iter().map(|p| (p.object, p.range_unresolved)).collect();
    assert_eq!(
        flags,
        [
            (top.num, false),
            (good.num, false),
            (none.num, false),
            (end_is_catalog.num, true),
            (start_is_direct.num, true),
            (nulls.num, false),
            (dangling.num, true)
        ]
    );
    assert_eq!(list.unresolved_ranges, 3);
}

#[test]
fn a_part_whose_parent_is_not_the_node_listing_it_is_flagged() {
    let mut doc = open(fixture());
    let top = add_part(&mut doc, None);
    let mid = add_part(&mut doc, None);
    let set_parent = |doc: &mut Document, node: ObjRef, value: Object| {
        doc.update_dict(node, |d| d.set(b"Parent".to_vec(), value)).unwrap();
    };
    // The root node's /Parent names the /DPartRoot (here, the catalog will do); it is not checked.
    let catalog = doc.root().unwrap();
    set_parent(&mut doc, top, Object::Ref(catalog));
    set_parent(&mut doc, mid, Object::Ref(top));
    let good = add_part(&mut doc, None);
    set_parent(&mut doc, good, Object::Ref(mid));
    let missing = add_part(&mut doc, None);
    let sibling = add_part(&mut doc, None);
    set_parent(&mut doc, sibling, Object::Ref(good));
    let direct = add_part(&mut doc, None);
    set_parent(&mut doc, direct, Object::Int(i64::from(mid.num)));
    let null = add_part(&mut doc, None);
    set_parent(&mut doc, null, Object::Null);
    let other_generation = add_part(&mut doc, None);
    set_parent(&mut doc, other_generation, Object::Ref(ObjRef { num: mid.num, generation: 1 }));
    // Listed by two parents, it is judged against the node given as its `parent` (`good`, which
    // lists it before `top`'s second entry does), and it names `mid`.
    let twice = add_part(&mut doc, None);
    set_parent(&mut doc, twice, Object::Ref(mid));
    set_children(&mut doc, mid, &[good, missing, sibling, direct, null, other_generation]);
    set_children(&mut doc, top, &[mid, twice]);
    doc.update_dict(good, |d| d.set(b"DParts".to_vec(), Object::Array(vec![Object::Array(vec![Object::Ref(twice)])]))).unwrap();
    set_document_part_root(&mut doc, top);

    let list = list_document_parts(&doc);
    let flags: Vec<_> = list.parts.iter().map(|p| (p.object, p.parent, p.parent_mismatch)).collect();
    // The root's /Parent is not checked; it is never flagged.
    assert_eq!(
        flags,
        [
            (top.num, None, false),
            (mid.num, Some(top.num), false),
            (good.num, Some(mid.num), false),
            (twice.num, Some(good.num), true),
            (missing.num, Some(mid.num), true),
            (sibling.num, Some(mid.num), true),
            (direct.num, Some(mid.num), true),
            (null.num, Some(mid.num), true),
            (other_generation.num, Some(mid.num), true),
        ]
    );
    assert_eq!(list.parent_mismatches, 6);
    assert_eq!(list.skipped, 1, "twice, listed again by top");
}

#[test]
fn document_part_listing_is_bounded_by_depth_and_node_count() {
    // A chain deeper than the limit.
    let mut doc = open(fixture());
    let top = add_part(&mut doc, None);
    let mut last = top;
    for _ in 0..(MAX_DPART_DEPTH + 10) {
        let next = add_part(&mut doc, None);
        set_children(&mut doc, last, &[next]);
        last = next;
    }
    set_document_part_root(&mut doc, top);
    let list = list_document_parts(&doc);
    assert!(list.truncated);
    assert_eq!(list.parts.len(), MAX_DPART_DEPTH + 1);
    assert_eq!(list.parts.last().map(|p| p.depth), Some(MAX_DPART_DEPTH));

    // More siblings than the node limit.
    let mut doc = open(fixture());
    let top = add_part(&mut doc, None);
    let kids: Vec<ObjRef> = (0..(MAX_DPART_NODES + 20)).map(|_| add_part(&mut doc, None)).collect();
    set_children(&mut doc, top, &kids);
    set_document_part_root(&mut doc, top);
    let list = list_document_parts(&doc);
    assert!(list.truncated);
    assert_eq!(list.parts.len(), MAX_DPART_NODES);

    // Hostile shapes: a root node that is not a dictionary, /DParts holding numbers.
    let mut doc = open(fixture());
    let number = doc.add(Object::Int(7));
    set_document_part_root(&mut doc, number);
    let list = list_document_parts(&doc);
    assert!((list.has_root, list.parts.len(), list.skipped) == (true, 0, 1), "{list:?}");
    let node = add_part(&mut doc, None);
    doc.update_dict(node, |d| d.set(b"DParts".to_vec(), Object::Array(vec![Object::Int(1), Object::Array(vec![Object::Int(2), Object::Null])])))
        .unwrap();
    set_document_part_root(&mut doc, node);
    let list = list_document_parts(&doc);
    assert_eq!((list.parts.len(), list.skipped), (1, 2), "{list:?}");
}

/// A document part tree of `nodes` nodes under one root, every node's `/DPM` the one indirect
/// dictionary `dpm`; the listing of each node's metadata.
fn shared_dpm_listing(dpm: pdfcraft_cos::Dict, nodes: usize) -> DocumentPartList {
    let mut doc = open(fixture());
    let shared = doc.add(Object::Dict(dpm));
    let top = add_part(&mut doc, None);
    let kids: Vec<ObjRef> = (0..nodes).map(|_| add_part(&mut doc, None)).collect();
    for kid in &kids {
        doc.update_dict(*kid, |d| d.set(b"DPM".to_vec(), Object::Ref(shared))).unwrap();
    }
    set_children(&mut doc, top, &kids);
    set_document_part_root(&mut doc, top);
    list_document_parts(&doc)
}

/// How many nodes list their whole `/DPM` (`entries` entries, none flagged) and how many are cut.
fn listed_and_cut(list: &DocumentPartList, entries: usize) -> (usize, usize) {
    let listed = list.parts.iter().filter(|p| p.metadata.len() == entries && !p.metadata_truncated).count();
    let cut = list.parts.iter().filter(|p| p.metadata_truncated).count();
    (listed, cut)
}

#[test]
fn document_part_metadata_is_read_within_one_allowance_whoever_shares_it() {
    // The whole listing may read MAX_DPM_BYTES of /DPM keys and string and name values. Each of
    // the three charges below has its own test: a dictionary one node lists is listed by every
    // node that names it, so what it holds is paid again for each.
    let per_node = |entries: usize, key: usize, value: usize| entries * (key + value + DPM_ENTRY_COST);

    // Keys: 8 entries with 100 KB names, an integer value each.
    let mut dpm = pdfcraft_cos::Dict::new();
    for i in 0..8 {
        dpm.set(format!("{i}{}", "k".repeat(100_000)).into_bytes(), Object::Int(i));
    }
    let cost = per_node(8, 100_001, 0);
    let fit = MAX_DPM_BYTES / cost;
    let list = shared_dpm_listing(dpm, fit + 3);
    assert_eq!(list.parts.len(), fit + 4, "the root and every kid is a node");
    assert_eq!(listed_and_cut(&list, 8), (fit, 3), "keys are charged");
    // The listed key is cut to MAX_DPM_CHARS characters.
    assert!(list.parts.iter().filter(|p| !p.metadata.is_empty()).all(|p| p.metadata.iter().all(|(k, _)| k.chars().count() == MAX_DPM_CHARS)));

    // String values: one entry holding a 1 MiB string.
    let mut dpm = pdfcraft_cos::Dict::new();
    dpm.set(b"K".to_vec(), Object::String(pdfcraft_cos::PdfString::text(&"s".repeat(1 << 20))));
    let fit = MAX_DPM_BYTES / per_node(1, 1, 1 << 20);
    let list = shared_dpm_listing(dpm, fit + 3);
    assert_eq!(listed_and_cut(&list, 1), (fit, 3), "string values are charged");
    assert!(list.parts.iter().filter(|p| !p.metadata.is_empty()).all(|p| p.metadata[0].1.len() == MAX_DPM_CHARS));

    // Name values: one entry whose value is a 1 MiB name.
    let mut dpm = pdfcraft_cos::Dict::new();
    dpm.set(b"K".to_vec(), Object::Name("n".repeat(1 << 20).into_bytes()));
    let fit = MAX_DPM_BYTES / per_node(1, 1, 1 << 20);
    let list = shared_dpm_listing(dpm, fit + 3);
    assert_eq!(listed_and_cut(&list, 1), (fit, 3), "name values are charged");
    assert!(list.parts.iter().filter(|p| !p.metadata.is_empty()).all(|p| p.metadata[0].1.len() == MAX_DPM_CHARS));

    // Entries: 32 one-byte keys with integer values cost DPM_ENTRY_COST each, so a tree of the
    // most nodes cannot list them all.
    let mut dpm = pdfcraft_cos::Dict::new();
    for i in 0..MAX_DPM_ENTRIES {
        dpm.set(vec![b'a' + i as u8], Object::Int(1));
    }
    let fit = MAX_DPM_BYTES / per_node(MAX_DPM_ENTRIES, 1, 0);
    assert!(fit + 1 < MAX_DPART_NODES, "the test needs more nodes than fit");
    let list = shared_dpm_listing(dpm, MAX_DPART_NODES - 1);
    assert_eq!(list.parts.len(), MAX_DPART_NODES);
    assert_eq!(listed_and_cut(&list, MAX_DPM_ENTRIES), (fit, MAX_DPART_NODES - 1 - fit), "each entry has a cost");
    // Once one does not fit, none after it is read: the first node cut lists whole entries only,
    // and every later node lists none.
    let first_cut = list.parts.iter().position(|p| p.metadata_truncated).unwrap();
    assert!(list.parts[first_cut + 1..].iter().all(|p| p.metadata.is_empty() && p.metadata_truncated));
}

#[test]
fn once_the_metadata_allowance_is_spent_later_dpms_are_flagged_and_not_read() {
    let mut doc = open(fixture());
    let top = add_part(&mut doc, None);
    let text = |len: usize| Object::String(pdfcraft_cos::PdfString::text(&"t".repeat(len)));
    let dict_of = |entries: Vec<(&str, Object)>| {
        let mut dpm = pdfcraft_cos::Dict::new();
        for (key, value) in entries {
            dpm.set(key.as_bytes().to_vec(), value);
        }
        dpm
    };
    let part_with = |doc: &mut Document, dpm: Object| {
        let part = add_part(doc, None);
        doc.update_dict(part, |d| d.set(b"DPM".to_vec(), dpm)).unwrap();
        part
    };
    let first = part_with(&mut doc, Object::Dict(dict_of(vec![("Big", text(MAX_DPM_BYTES - 1000))])));
    // Cannot fit what is left; the small entry after it would.
    let second = part_with(&mut doc, Object::Dict(dict_of(vec![("Big", text(5000)), ("Small", Object::Int(1))])));
    // Later nodes whose whole /DPM would fit: direct, indirect (never read), and an empty one.
    let third = part_with(&mut doc, Object::Dict(dict_of(vec![("Small", Object::Int(1))])));
    let shared = doc.add(Object::Dict(dict_of(vec![("Small", Object::Int(1))])));
    let fourth = part_with(&mut doc, Object::Ref(shared));
    let fifth = part_with(&mut doc, Object::Dict(dict_of(vec![])));
    set_children(&mut doc, top, &[first, second, third, fourth, fifth]);
    set_document_part_root(&mut doc, top);
    crate::dparts::PREPARES.with(|n| n.set(0));
    let list = list_document_parts(&doc);
    let rows: Vec<_> = list.parts.iter().map(|p| (p.metadata.len(), p.metadata_truncated)).collect();
    assert_eq!(rows, [(0, false), (1, false), (0, true), (0, true), (0, true), (0, false)], "{list:?}");
    // Only the first two were read: the allowance was spent by the second.
    assert_eq!(crate::dparts::PREPARES.with(|n| n.get()), 2);
}

/// A PDF whose object 3 is one string of 33 MiB (more than the document keeps cached once read),
/// and `nodes` document parts. In `shared_dictionary` every part's `/DPM` is object 3, a
/// dictionary holding the string directly; otherwise object 3 is the string and every part's
/// `/DPM` is a direct dictionary that names it.
fn pdf_with_a_big_shared_metadata_value(nodes: usize, shared_dictionary: bool) -> Vec<u8> {
    let big = "s".repeat(33 << 20);
    let kids: String = (0..nodes).map(|i| format!("{} 0 R ", 7 + i)).collect();
    let mut objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R /DPartRoot 5 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [4 0 R] /Count 1 /MediaBox [0 0 200 200] >>".to_vec(),
        if shared_dictionary { format!("<< /K ({big}) >>").into_bytes() } else { format!("({big})").into_bytes() },
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /Type /DPartRoot /DPartRootNode 6 0 R >>".to_vec(),
        format!("<< /Type /DPart /DParts [[{kids}]] >>").into_bytes(),
    ];
    let dpm = if shared_dictionary { "3 0 R" } else { "<< /K 3 0 R >>" };
    objs.extend((0..nodes).map(|_| format!("<< /Type /DPart /DPM {dpm} >>").into_bytes()));
    let mut out = b"%PDF-2.0\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

#[test]
fn a_metadata_object_too_big_to_list_is_not_parsed_again_by_every_node_that_shares_it() {
    // The document does not keep an object this large once read, so reading it again per node
    // costs about 70 ms each: 300 nodes would take half a minute; it is read once, by the node that finds it does not fit, and the nodes after it read no /DPM.
    for shared_dictionary in [true, false] {
        let doc = open(pdf_with_a_big_shared_metadata_value(300, shared_dictionary));
        let started = std::time::Instant::now();
        let list = list_document_parts(&doc);
        let took = started.elapsed();
        assert_eq!(list.parts.len(), 301);
        // Too big to list: every node is flagged, none lists it.
        assert!(list.parts[1..].iter().all(|p| p.metadata.is_empty() && p.metadata_truncated));
        assert!(took < std::time::Duration::from_secs(8), "shared dictionary {shared_dictionary}: {took:?}");
    }
}

#[test]
fn document_part_metadata_keys_and_values_are_cut_by_characters_not_bytes() {
    // 300 four-byte characters: the cut is at 200 characters, not at a byte count.
    let wide = "\u{1F600}".repeat(300);
    let mut dpm = pdfcraft_cos::Dict::new();
    dpm.set(wide.clone().into_bytes(), Object::Name(wide.clone().into_bytes()));
    dpm.set(b"Text".to_vec(), Object::String(pdfcraft_cos::PdfString::text(&wide)));
    let list = shared_dpm_listing(dpm, 1);
    let part = &list.parts[1];
    assert!(!part.metadata_truncated);
    let entry = |key: &str| part.metadata.iter().find(|(k, _)| k == key).unwrap().1.clone();
    assert_eq!(part.metadata.iter().find(|(k, _)| k.starts_with('\u{1F600}')).unwrap().0, "\u{1F600}".repeat(MAX_DPM_CHARS));
    assert_eq!(part.metadata.iter().find(|(k, _)| k.starts_with('\u{1F600}')).unwrap().1, format!("/{}", "\u{1F600}".repeat(MAX_DPM_CHARS - 1)));
    assert_eq!(entry("Text"), "\u{1F600}".repeat(MAX_DPM_CHARS));
}

fn ns_dict(doc: &mut Document, ns: Option<&str>, roles: &[(&str, Object)]) -> ObjRef {
    let mut d = pdfcraft_cos::Dict::new();
    d.set(b"Type".to_vec(), Object::name("Namespace"));
    if let Some(ns) = ns {
        d.set(b"NS".to_vec(), Object::String(pdfcraft_cos::PdfString::text(ns)));
    }
    if !roles.is_empty() {
        let mut map = pdfcraft_cos::Dict::new();
        for (k, v) in roles {
            map.set(k.as_bytes().to_vec(), v.clone());
        }
        d.set(b"RoleMapNS".to_vec(), Object::Dict(map));
    }
    doc.add(Object::Dict(d))
}

fn set_namespaces(doc: &mut Document, items: Vec<Object>) {
    let mut root = pdfcraft_cos::Dict::new();
    root.set(b"Type".to_vec(), Object::name("StructTreeRoot"));
    root.set(b"Namespaces".to_vec(), Object::Array(items));
    let root = doc.add(Object::Dict(root));
    doc.update_dict(doc.root().unwrap(), |c| c.set(b"StructTreeRoot".to_vec(), Object::Ref(root))).unwrap();
}

#[test]
fn structure_namespaces_are_listed_with_their_role_maps() {
    let mut doc = open(fixture());
    assert_eq!(list_structure_namespaces(&doc), NamespaceList::default(), "no /StructTreeRoot");
    let pdf2 = ns_dict(&mut doc, Some("http://iso.org/pdf2/ssn"), &[]);
    let custom = ns_dict(
        &mut doc,
        Some("urn:example:tags"),
        &[("Note", Object::Array(vec![Object::name("P"), Object::Ref(pdf2)])), ("Plain", Object::name("Span")), ("Odd", Object::Int(3))],
    );
    let nameless = ns_dict(&mut doc, None, &[]);
    // custom is listed twice, a number and a null are not namespaces.
    set_namespaces(&mut doc, vec![Object::Ref(pdf2), Object::Ref(custom), Object::Ref(custom), Object::Int(4), Object::Ref(nameless), Object::Null]);

    let list = list_structure_namespaces(&doc);
    assert!(list.has_structure_tree && !list.truncated);
    assert_eq!(list.skipped, 2, "{list:?}");
    // Only the dictionary with no /NS lacks an identifier.
    assert_eq!(list.without_ns, 1, "{list:?}");
    let rows: Vec<_> = list.namespaces.iter().map(|n| (n.object, n.ns.as_deref(), n.role_count)).collect();
    assert_eq!(
        rows,
        [(Some(pdf2.num), Some("http://iso.org/pdf2/ssn"), 0), (Some(custom.num), Some("urn:example:tags"), 3), (Some(nameless.num), None, 0)]
    );
    let roles: Vec<_> =
        list.namespaces[1].roles.iter().map(|r| (r.structure_type.as_str(), r.role.as_deref(), r.role_namespace.as_deref())).collect();
    assert_eq!(roles, [("Note", Some("P"), Some("http://iso.org/pdf2/ssn")), ("Plain", Some("Span"), None), ("Odd", None, None)]);
}

#[test]
fn namespaces_without_a_string_identifier_are_counted() {
    let mut doc = open(fixture());
    let named = ns_dict(&mut doc, Some("urn:named"), &[]);
    let empty = ns_dict(&mut doc, Some(""), &[]);
    let none = ns_dict(&mut doc, None, &[]);
    // A /NS that is a number names nothing either.
    let number = {
        let mut d = pdfcraft_cos::Dict::new();
        d.set(b"NS".to_vec(), Object::Int(5));
        doc.add(Object::Dict(d))
    };
    set_namespaces(&mut doc, vec![Object::Ref(named), Object::Ref(empty), Object::Ref(none), Object::Ref(none), Object::Ref(number), Object::Int(1)]);
    let list = list_structure_namespaces(&doc);
    assert_eq!((list.namespaces.len(), list.without_ns, list.skipped), (4, 2, 1), "{list:?}");
    assert!(list.namespaces.iter().filter(|n| n.ns.is_none()).map(|n| n.object).eq([Some(none.num), Some(number.num)]));

    let mut doc = open(fixture());
    let x = ns_dict(&mut doc, Some("urn:x"), &[]);
    set_namespaces(&mut doc, vec![Object::Ref(x)]);
    assert_eq!(list_structure_namespaces(&doc).without_ns, 0);
}

#[test]
fn repeated_namespace_identifiers_are_reported() {
    let mut doc = open(fixture());
    let a = ns_dict(&mut doc, Some("urn:a"), &[]);
    let b = ns_dict(&mut doc, Some("urn:b"), &[]);
    let a2 = ns_dict(&mut doc, Some("urn:a"), &[]);
    let a3 = ns_dict(&mut doc, Some("urn:a"), &[]);
    let none1 = ns_dict(&mut doc, None, &[]);
    let none2 = ns_dict(&mut doc, None, &[]);
    // `a` listed twice by reference is one dictionary, not a repeat.
    set_namespaces(
        &mut doc,
        vec![Object::Ref(a), Object::Ref(a), Object::Ref(b), Object::Ref(a2), Object::Ref(none1), Object::Ref(none2), Object::Ref(a3)],
    );
    let list = list_structure_namespaces(&doc);
    assert_eq!(
        list.duplicates,
        [DuplicateNamespace { ns: "urn:a".into(), count: 3, objects: vec![Some(a.num), Some(a2.num), Some(a3.num)] }],
        "{list:?}"
    );

    // Many repeats name a bounded number of objects but are all counted.
    let mut doc = open(fixture());
    let many: Vec<Object> = (0..(MAX_DUPLICATE_OBJECTS + 9)).map(|_| Object::Ref(ns_dict(&mut doc, Some("urn:same"), &[]))).collect();
    set_namespaces(&mut doc, many);
    let list = list_structure_namespaces(&doc);
    assert_eq!(
        (list.duplicates.len(), list.duplicates[0].count, list.duplicates[0].objects.len()),
        (1, MAX_DUPLICATE_OBJECTS + 9, MAX_DUPLICATE_OBJECTS)
    );

    // Distinct identifiers report nothing.
    let mut doc = open(fixture());
    let x = ns_dict(&mut doc, Some("urn:x"), &[]);
    let y = ns_dict(&mut doc, Some("urn:y"), &[]);
    set_namespaces(&mut doc, vec![Object::Ref(x), Object::Ref(y)]);
    assert!(list_structure_namespaces(&doc).duplicates.is_empty());
}

#[test]
fn role_pairs_say_whether_their_namespace_is_listed() {
    let mut doc = open(fixture());
    let listed = ns_dict(&mut doc, Some("urn:listed"), &[]);
    let stray = ns_dict(&mut doc, Some("urn:stray"), &[]);
    let number = doc.add(Object::Int(7));
    let mut direct = pdfcraft_cos::Dict::new();
    direct.set(b"NS".to_vec(), Object::String(pdfcraft_cos::PdfString::text("urn:direct")));
    let owner = ns_dict(
        &mut doc,
        Some("urn:owner"),
        &[
            ("A", Object::Array(vec![Object::name("P"), Object::Ref(listed)])),
            ("B", Object::Array(vec![Object::name("P"), Object::Ref(stray)])),
            ("C", Object::Array(vec![Object::name("P"), Object::Dict(direct)])),
            ("D", Object::Array(vec![Object::name("P"), Object::Int(3)])),
            ("E", Object::name("Span")),
            ("F", Object::Array(vec![Object::name("P")])),
            ("G", Object::Array(vec![Object::name("P"), Object::Ref(number)])),
        ],
    );
    set_namespaces(&mut doc, vec![Object::Ref(listed), Object::Ref(owner)]);
    let list = list_structure_namespaces(&doc);
    let flags: Vec<_> = list.namespaces[1].roles.iter().map(|r| (r.structure_type.as_str(), r.role_namespace_listed)).collect();
    assert_eq!(flags, [("A", Some(true)), ("B", Some(false)), ("C", Some(false)), ("D", None), ("E", None), ("F", None), ("G", None)], "{list:?}");
    assert_eq!(list.namespaces[1].roles[6].role_namespace_object, Some(number.num));
    assert_eq!(list.namespaces[1].roles[1].role_namespace_object, Some(stray.num));
    assert_eq!(list.unlisted_role_namespaces, 2, "{list:?}");

    // An entry with the wrong generation resolves to nothing, so it lists nothing: a role pointing
    // at the object by its real reference is not listed.
    let mut doc = open(fixture());
    let target = ns_dict(&mut doc, Some("urn:target"), &[]);
    let owner = ns_dict(&mut doc, Some("urn:owner"), &[("A", Object::Array(vec![Object::name("P"), Object::Ref(target)]))]);
    set_namespaces(&mut doc, vec![Object::Ref(owner), Object::Ref(ObjRef::new(target.num, 3))]);
    let list = list_structure_namespaces(&doc);
    assert_eq!(
        (list.namespaces.len(), list.namespaces[0].roles[0].role_namespace_listed, list.unlisted_role_namespaces),
        (1, Some(false), 1),
        "{list:?}"
    );

    // Past the limit, a target that was not read may be further on: unknown, not unlisted.
    let mut doc = open(fixture());
    let late = ns_dict(&mut doc, Some("urn:late"), &[]);
    let owner = ns_dict(&mut doc, Some("urn:owner"), &[("A", Object::Array(vec![Object::name("P"), Object::Ref(late)]))]);
    let mut items = vec![Object::Ref(owner)];
    items.extend((0..MAX_NAMESPACES).map(|i| Object::Ref(ns_dict(&mut doc, Some(&format!("urn:n{i}")), &[]))));
    items.push(Object::Ref(late));
    set_namespaces(&mut doc, items);
    let list = list_structure_namespaces(&doc);
    assert!(list.truncated);
    assert_eq!((list.namespaces[0].roles[0].role_namespace_listed, list.unlisted_role_namespaces), (None, 0), "{list:?}");
}

#[test]
fn structure_namespace_listing_is_bounded() {
    let mut doc = open(fixture());
    let many: Vec<Object> = (0..(MAX_NAMESPACES + 5)).map(|i| Object::Ref(ns_dict(&mut doc, Some(&format!("urn:n{i}")), &[]))).collect();
    set_namespaces(&mut doc, many);
    let list = list_structure_namespaces(&doc);
    assert!(list.truncated);
    assert_eq!(list.namespaces.len(), MAX_NAMESPACES);

    let mut doc = open(fixture());
    let roles: Vec<(String, Object)> = (0..(MAX_ROLE_ENTRIES + 10)).map(|i| (format!("T{i}"), Object::name("P"))).collect();
    let roles: Vec<(&str, Object)> = roles.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    let big = ns_dict(&mut doc, Some(&"x".repeat(MAX_NS_CHARS + 50)), &roles);
    set_namespaces(&mut doc, vec![Object::Ref(big)]);
    let list = list_structure_namespaces(&doc);
    let n = &list.namespaces[0];
    assert_eq!(
        (n.role_count, n.roles.len(), n.ns.as_ref().map(|s| s.chars().count())),
        (MAX_ROLE_ENTRIES + 10, MAX_ROLE_ENTRIES, Some(MAX_NS_CHARS))
    );

    // /Namespaces that is not an array, and a /StructTreeRoot that is not a dictionary.
    let mut doc = open(fixture());
    let root = pdfcraft_cos::Dict::from_iter([(b"Namespaces".to_vec(), Object::Int(1))]);
    let root = doc.add(Object::Dict(root));
    doc.update_dict(doc.root().unwrap(), |c| c.set(b"StructTreeRoot".to_vec(), Object::Ref(root))).unwrap();
    let list = list_structure_namespaces(&doc);
    assert_eq!((list.has_structure_tree, list.namespaces.len()), (true, 0), "{list:?}");

    // A /StructTreeRoot that is not a dictionary is no structure tree.
    let mut doc = open(fixture());
    let number = doc.add(Object::Int(7));
    doc.update_dict(doc.root().unwrap(), |c| c.set(b"StructTreeRoot".to_vec(), Object::Ref(number))).unwrap();
    assert_eq!(list_structure_namespaces(&doc), NamespaceList::default());

    // Exactly the limit is not truncated.
    let mut doc = open(fixture());
    let exact: Vec<Object> = (0..MAX_NAMESPACES).map(|i| Object::Ref(ns_dict(&mut doc, Some(&format!("urn:n{i}")), &[]))).collect();
    set_namespaces(&mut doc, exact);
    let list = list_structure_namespaces(&doc);
    assert_eq!((list.namespaces.len(), list.truncated), (MAX_NAMESPACES, false));
}

/// A name tree whose `kids` nodes all name one `/Names` array of `pairs` entries: the file holds
/// `kids + pairs` objects, and a reader that copies the array for every node lists `kids x pairs`.
fn doc_with_shared_names_array(kids: usize, pairs: usize) -> Document {
    // Objects: 1 catalog, 2 pages, 3 page, 4 root node, 5 the shared array, 6.. the kid nodes.
    let entries: String = (0..pairs).map(|i| format!("(f{i:05}.txt) << /Type /Filespec /F (f{i:05}.txt) >> ")).collect();
    let refs: String = (0..kids).map(|i| format!("{} 0 R ", i + 6)).collect();
    let mut b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles 4 0 R >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        format!("<< /Kids [{refs}] >>"),
        format!("[{entries}]"),
    ];
    b.extend((0..kids).map(|_| "<< /Names 5 0 R >>".to_string()));
    open(build(&b, "/Root 1 0 R"))
}

#[test]
fn a_names_array_shared_by_many_nodes_is_listed_once() {
    let (kids, pairs) = (300, 300);
    let doc = doc_with_shared_names_array(kids, pairs);
    let keys = attachment_keys(&doc);
    assert_eq!(keys.len(), pairs, "each attachment is listed once, not once per node that names the array");
    assert_eq!((keys.first().map(String::as_str), keys.last().map(String::as_str)), (Some("f00000.txt"), Some("f00299.txt")));
}

/// Objects: 1 catalog, 2 pages, 3 page, 4 root node, 5 an array of two entries, 6 a node dictionary
/// as given by `node6`. `root_kids` is the root's `/Kids`, `root_names` its `/Names`.
fn doc_with_name_tree(root_names: &str, root_kids: &str, node6: &str) -> Document {
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles 4 0 R >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        format!("<< /Names {root_names} /Kids {root_kids} >>"),
        "[(a.txt) << /Type /Filespec /F (a.txt) >> (b.txt) << /Type /Filespec /F (b.txt) >>]".into(),
        node6.into(),
    ];
    open(build(&b, "/Root 1 0 R"))
}

#[test]
fn a_kids_entry_naming_the_names_array_does_not_hide_its_entries() {
    // /Kids lists object 5, the /Names array itself, then node 6, which names that array.
    let doc = doc_with_name_tree("[]", "[5 0 R 6 0 R]", "<< /Names 5 0 R >>");
    assert_eq!(attachment_keys(&doc), ["a.txt", "b.txt"]);
}

#[test]
fn a_kids_entry_naming_a_node_dictionary_does_not_hide_its_entries() {
    // /Names names node 6, a dictionary, where an array belongs; node 6 is still read as a kid.
    let doc = doc_with_name_tree("6 0 R", "[6 0 R]", "<< /Names [(c.txt) << /Type /Filespec /F (c.txt) >>] >>");
    assert_eq!(attachment_keys(&doc), ["c.txt"]);
}

#[test]
fn a_named_destination_lookup_reads_a_shared_names_array_once() {
    // Link annotation 5 goes to a name no tree entry has. Objects 6..: the root node, one /Names
    // array of `pairs` entries, and `kids` leaf nodes that all name that array.
    let (kids, pairs) = (400, 400);
    let entries: String = (0..pairs).map(|i| format!("(d{i:04}) [3 0 R /Fit] ")).collect();
    let refs: String = (0..kids).map(|i| format!("{} 0 R ", i + 8)).collect();
    let mut extra = vec![format!("<< /Kids [{refs}] >>"), format!("[{entries}]")];
    extra.extend((0..kids).map(|_| "<< /Names 7 0 R >>".to_string()));
    let link = "<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A << /S /GoTo /D (missing) >> >>".to_string();
    let src = page_with_annots(&[link], &extra, "/Names << /Dests 6 0 R >>");
    take_work();
    let _ = extract_pages(&src, &[0]).unwrap();
    let work = take_work();
    assert!(work <= 8 * (kids + pairs), "{work} steps looking a name up in {kids} nodes sharing {pairs} pairs");
}

#[test]
fn many_links_to_named_destinations_read_the_name_tree_once() {
    // `links` link annotations each name a different destination in one tree of `pairs` names.
    // Looking each up on its own read the tree per link: links x pairs steps.
    let (links, pairs) = (400, 400);
    let entries: String = (0..pairs).map(|i| format!("(d{i:04}) [3 0 R /Fit] ")).collect();
    let links_src: Vec<String> =
        (0..links).map(|i| format!("<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A << /S /GoTo /D (d{:04}) >> >>", i % pairs)).collect();
    let extra = vec![format!("<< /Names [{entries}] >>")];
    let src = page_with_annots(&links_src, &extra, &format!("/Names << /Dests {} 0 R >>", links + 5));
    take_work();
    let out = extract_pages(&src, &[0]).unwrap();
    let work = take_work();
    assert!(work <= 8 * (links + pairs), "{work} steps resolving {links} links into {pairs} names");
    let resolved = annots(&out, 0).iter().filter(|a| a.get(b"A").is_some()).count();
    assert!(resolved == links, "only {resolved} of {links} links kept their destination");
}

#[test]
fn pairs_sharing_one_large_key_do_not_copy_it_per_pair() {
    // Object 7 is one 128 KiB string used as the key of every pair of the tree. Collecting all
    // pairs before dropping duplicates copied it once per pair (pairs x 128 KiB).
    let (pairs, key_len) = (1_000, 128 * 1024);
    let entries: String = (0..pairs).map(|_| "7 0 R [3 0 R /Fit] ").collect();
    let extra = vec![format!("<< /Names [{entries}] >>"), format!("({})", "k".repeat(key_len))];
    let link = "<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A << /S /GoTo /D (missing) >> >>".to_string();
    let src = page_with_annots(&[link], &extra, "/Names << /Dests 6 0 R >>");
    take_work();
    let _ = extract_pages(&src, &[0]).unwrap();
    let work = take_work();
    assert!(work <= 4 * (pairs + key_len), "{work} steps and key bytes copied for {pairs} pairs sharing one key");
}

#[test]
fn a_named_destination_dictionary_entry_beats_a_tree_entry_and_the_first_tree_entry_wins() {
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Dests << /a [3 0 R /Fit] >> /Names << /Dests 4 0 R >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Names [(a) [3 0 R /XYZ 0 0 0] (b) [3 0 R /Fit] (b) [3 0 R /FitH 0]] >>".into(),
    ];
    let doc = open(build(&b, "/Root 1 0 R"));
    let index = crate::import::DestIndex::default();
    let kind = |name: &[u8]| index.get(&doc, name).and_then(|o| o.as_array().and_then(|a| a.get(1).and_then(|n| n.as_name().map(<[u8]>::to_vec))));
    assert!(kind(b"a").as_deref() == Some(b"Fit".as_slice()), "dictionary entry should win");
    assert!(kind(b"b").as_deref() == Some(b"Fit".as_slice()), "first tree entry should win");
    assert!(kind(b"c").is_none(), "unknown name");
}

#[test]
fn a_kids_array_that_another_node_uses_as_its_names_array_hides_nothing() {
    // Node 6 names array 5 as its /Kids and node 7 as its /Names. The first must not hide the
    // entries the second holds, in the attachment listing or in a named-destination lookup.
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles 4 0 R >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Kids [6 0 R 7 0 R] >>".into(),
        "[(a.txt) << /Type /Filespec /F (a.txt) >>]".into(),
        "<< /Kids 5 0 R >>".into(),
        "<< /Names 5 0 R >>".into(),
    ];
    assert_eq!(attachment_keys(&open(build(&b, "/Root 1 0 R"))), ["a.txt"]);

    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Names << /Dests 4 0 R >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Kids [6 0 R 7 0 R] >>".into(),
        "[(sec) [3 0 R /Fit]]".into(),
        "<< /Kids 5 0 R >>".into(),
        "<< /Names 5 0 R >>".into(),
    ];
    assert!(crate::import::DestIndex::default().get(&open(build(&b, "/Root 1 0 R")), b"sec").is_some());
}

/// A document whose `/EmbeddedFiles` tree has `pairs` pairs that all use object 6, a string of
/// `key_len` bytes, as their key.
fn shared_key_attachments(pairs: usize, key_len: usize) -> Document {
    let entries: String = (0..pairs).map(|_| "6 0 R 3 0 R ".to_string()).collect();
    let extra = vec![format!("<< /Names [{entries}] >>"), format!("({})", "k".repeat(key_len))];
    page_with_annots(&[], &extra, "/Names << /EmbeddedFiles 5 0 R >>")
}

#[test]
fn attachment_names_sharing_one_large_key_are_refused_not_copied_per_pair() {
    // 100 pairs on one 1 MiB key would copy 100 MiB (2,000 pairs: 2 GiB) and write it back.
    let (pairs, key_len) = (100, 1024 * 1024);
    let cap = crate::import::MAX_NAME_TREE_KEY_BYTES;
    let mut doc = shared_key_attachments(pairs, key_len);
    take_work();
    let listed = attachment_keys(&doc).len();
    let work = take_work();
    assert!(work <= cap + 4 * pairs + 1024, "{work} bytes copied listing {pairs} pairs on a {key_len}-byte key");
    assert!(listed < pairs, "{listed} names listed past the allowance");
    let before = attachment_keys(&doc);
    let err = add_attachment(&mut doc, "a.txt", b"x", None, None, None);
    assert!(err.is_err(), "add rewrote a tree past the allowance");
    assert!(delete_attachment(&mut doc, "nope").is_err(), "delete rewrote a tree past the allowance");
    assert!(attachment_keys(&doc) == before, "a refused change altered the tree");
    take_work();
    let combined = combine(&[("big.pdf", &doc)]).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(take_work() <= cap + 4 * pairs + 1024, "combine copied the key per pair");
    assert!(
        combined.contains("big.pdf") && combined.contains("attachments (EmbeddedFiles)"),
        "combine error does not name the file and tree: {combined}"
    );
    assert!(combined.contains("page selection") && !combined.contains("not read"), "combine error: {combined}");
    let grouped = crate::combine_grouped(&[(0, "grouped.pdf", &doc, None)]).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(grouped.contains("grouped.pdf") && grouped.contains("attachments (EmbeddedFiles)"), "grouped combine error: {grouped}");
}

#[test]
fn attachment_names_that_repeat_a_small_shared_key_are_all_kept() {
    let mut doc = shared_key_attachments(3, 5);
    assert!(attachment_keys(&doc).len() == 3, "not every pair listed");
    let key = add_attachment(&mut doc, "kkkkk", b"x", None, None, None).unwrap();
    assert!(key != "kkkkk", "the new name was not made unique");
    assert!(attachment_keys(&doc).len() == 4, "an entry was lost by the rewrite");
}

#[test]
fn template_names_sharing_one_large_key_refuse_a_rewrite() {
    let (pairs, key_len) = (100, 1024 * 1024);
    let entries: String = (0..pairs).map(|_| "6 0 R 3 0 R ".to_string()).collect();
    let extra = vec![format!("<< /Names [{entries}] >>"), format!("({})", "k".repeat(key_len))];
    let mut doc = page_with_annots(&[], &extra, "/Names << /Templates 5 0 R >>");
    let err = set_template_hidden(&mut doc, "kkk", false).unwrap_err().to_string();
    assert!(err.contains("name tree's keys"), "refused for another reason: {err}");
}

#[test]
fn attachment_edits_that_rewrite_refuse_on_a_tree_past_the_allowance() {
    let mut doc = shared_key_attachments(100, 1024 * 1024);
    let err = set_attachment_description(&mut doc, "kkk", Some("d")).unwrap_err().to_string();
    assert!(err.contains("attachments (EmbeddedFiles)") && err.contains("32 MiB"), "{err}");
    let err = crate::attachments::delete_attachment_by_ref(&mut doc, ObjRef::new(3, 0)).unwrap_err().to_string();
    assert!(err.contains("nothing was changed"), "{err}");
}

#[test]
fn hiding_a_template_leaves_the_document_alone_when_the_target_tree_is_refused() {
    // Object 7 lists template "t" (page 8) under /Pages; /Templates, where it would go, is
    // past the key allowance. The refusal must come before the page leaves the page tree.
    let entries: String = (0..100).map(|_| "6 0 R 3 0 R ".to_string()).collect();
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /Names << /Pages 7 0 R /Templates 5 0 R >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R 8 0 R] /Count 2 /MediaBox [0 0 300 400] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".into(),
        body("P1"),
        format!("<< /Names [{entries}] >>"),
        format!("({})", "k".repeat(1024 * 1024)),
        "<< /Names [(t) 8 0 R] >>".into(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".into(),
    ];
    let mut doc = open(build(&b, "/Root 1 0 R"));
    assert!(set_template_hidden(&mut doc, "t", true).is_err());
    assert_eq!(crate::page_count(&doc).unwrap(), 2, "the page left the tree before the refusal");
    assert_eq!(doc.get(ObjRef::new(8, 0)).as_dict().and_then(|d| d.name(b"Type")), Some(&b"Page"[..]));
}

#[test]
fn removing_an_associated_file_leaves_af_alone_when_the_attachment_tree_is_refused() {
    let entries: String = (0..100).map(|_| "6 0 R 4 0 R ".to_string()).collect();
    let b: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R /AF [4 0 R] /Names << /EmbeddedFiles 5 0 R >> >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 300 400] >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        "<< /Type /Filespec /F (a.txt) >>".into(),
        format!("<< /Names [{entries}] >>"),
        format!("({})", "k".repeat(1024 * 1024)),
    ];
    let mut doc = open(build(&b, "/Root 1 0 R"));
    assert!(remove_associated_file(&mut doc, 1, false).is_err());
    let af = doc.root().and_then(|r| doc.get(r).as_dict().and_then(|c| c.get(b"AF").cloned()));
    assert!(af.is_some_and(|a| doc.resolve(&a).as_array().is_some_and(|l| l.len() == 1)), "/AF changed before the refusal");
}

#[test]
fn a_relationship_is_set_on_an_entry_with_none_and_every_refusal_changes_nothing() {
    // The catalog and the one page share the indirect array 4: a file specification with no
    // relationship (object 5, by reference), one written in the array itself with none, a number.
    let bodies = vec![
        "<< /Type /Catalog /Pages 2 0 R /AF 4 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 10 10] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /AF 4 0 R >>".to_string(),
        "[5 0 R << /Type /Filespec /F (direct.xml) >> 7]".to_string(),
        spec_body("one.txt"),
    ];
    let mut doc = open(build(&bodies, "/Root 1 0 R"));
    let rows = |doc: &Document| associated_files(doc).into_iter().map(|f| (f.page, f.index, f.relationship)).collect::<Vec<_>>();
    assert_eq!(rows(&doc), vec![(None, 1, None), (None, 2, None)], "array 4 is read once, for the catalog");

    // Everything refused leaves the document exactly as it was.
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    for (index, relationship) in [(1, "Banana"), (1, ""), (1, "data"), (1, "EncryptedPayload"), (0, "Data"), (4, "Data"), (3, "Data"), (99, "Data")] {
        assert!(matches!(set_associated_relationship(&mut doc, index, relationship), Err(OrganizeError::Invalid(_))), "{index} {relationship:?}");
    }
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);

    // The direct specification: the catalog gets a new direct array, the page keeps array 4 as it was.
    assert!(catalog_af_is_indirect(&doc));
    assert_eq!(set_associated_relationship(&mut doc, 2, "Schema").unwrap(), None);
    assert_eq!(rows(&doc), vec![(None, 1, None), (None, 2, Some("Schema".into())), (Some(0), 1, None), (Some(0), 2, None)]);
    let array = doc.get(pdfcraft_cos::ObjRef { num: 4, generation: 0 });
    assert_eq!(array.as_array().map(Vec::len), Some(3));
    assert!(!catalog_af_is_indirect(&doc), "the catalog now has its own direct array");
    // The same name again would change nothing, so it is refused.
    assert!(set_associated_relationship(&mut doc, 2, "Schema").is_err());

    // The one by reference changes where it is written, so the page's array, which lists it too, shows it.
    assert!(associated_file_is_shared(&doc, 1).unwrap());
    assert_eq!(set_associated_relationship(&mut doc, 1, "Data").unwrap(), None);
    assert_eq!(set_associated_relationship(&mut doc, 1, "Source").unwrap().as_deref(), Some("Data"));
    assert_eq!(rows(&doc)[0], (None, 1, Some("Source".into())));
    assert_eq!(rows(&doc)[2], (Some(0), 1, Some("Source".into())), "the page's array lists the same object");
    assert_eq!(rows(&doc)[3], (Some(0), 2, None), "and its direct specification is the one that was not changed");
    assert!(set_associated_relationship(&mut doc, 1, "Source").is_err());
    // It survives a save.
    let doc = save_and_reopen(&doc);
    assert_eq!(rows(&doc)[0], (None, 1, Some("Source".into())));
}

#[test]
fn setting_a_relationship_refuses_an_af_that_is_not_a_usable_array_or_a_specification_that_is_not_a_dictionary() {
    let mut doc = Document::open(Arc::new(fixture())).unwrap();
    // No /AF at all.
    assert!(set_associated_relationship(&mut doc, 1, "Data").is_err());
    let root = doc.root().unwrap();
    // Not an array.
    doc.update_dict(root, |c| c.set(b"AF".to_vec(), Object::Int(3))).unwrap();
    assert!(set_associated_relationship(&mut doc, 1, "Data").is_err());
    assert_eq!(af_catalog(&doc).get(b"AF"), Some(&Object::Int(3)));
    // Longer than the listing reads: refused even for an entry it could reach (the first is a specification).
    let mut spec = pdfcraft_cos::Dict::new();
    spec.set(b"Type".to_vec(), Object::name("Filespec"));
    spec.set(b"F".to_vec(), Object::String(PdfString::text("first.xml")));
    let mut long: Vec<Object> = (0..MAX_AF_ENTRIES).map(|_| Object::Null).collect();
    long.insert(0, Object::Dict(spec));
    assert_eq!(long.len(), MAX_AF_ENTRIES + 1);
    doc.update_dict(root, |c| c.set(b"AF".to_vec(), Object::Array(long.clone()))).unwrap();
    assert!(set_associated_relationship(&mut doc, 1, "Data").is_err());
    assert_eq!(af_catalog(&doc).get(b"AF"), Some(&Object::Array(long)));
    // A reference to something that is not a dictionary.
    let mut doc = open(build(
        &["<< /Type /Catalog /Pages 2 0 R /AF [3 0 R] >>".to_string(), "<< /Type /Pages /Kids [] /Count 0 >>".to_string(), "(text)".to_string()],
        "/Root 1 0 R",
    ));
    assert!(set_associated_relationship(&mut doc, 1, "Data").is_err());
}

#[test]
fn an_entry_that_points_at_a_stream_the_catalog_or_a_non_specification_is_refused_and_nothing_is_written() {
    let bodies = vec![
        // Entries, in order: 1 a stream, 2 the catalog, 3 a dictionary typed as an annotation, 4 a
        // dictionary with none of /F /UF /EF /FS, 5 a good specification, 6 a direct dictionary
        // typed as a page, 7 an untyped annotation whose /F is a number, 8 an untyped file
        // attachment annotation whose /FS is a dictionary, 9 a specification with only /UF,
        // 10 one with /FS /URL and a string /F, 11 a typed one with only an /EF dictionary.
        // 1 to 4 and 6 to 8 are refused; 5 and 9 to 11 are accepted.
        "<< /Type /Catalog /Pages 2 0 R /AF [3 0 R 1 0 R 4 0 R 5 0 R 6 0 R << /Type /Page /F 1 >> << /Subtype /Text /F 4 >> << /Subtype /FileAttachment /F 4 /FS << /F (x) >> >> << /UF (u.xml) >> << /FS /URL /F (http://example.com/a) >> << /Type /Filespec /EF << /F 3 0 R >> >>] >>".to_string(),
        "<< /Type /Pages /Kids [] /Count 0 >>".to_string(),
        "<< /Length 1 /F (looks-like-a-spec) >>\nstream\nx\nendstream".to_string(),
        "<< /Type /Annot /F 4 >>".to_string(),
        "<< /Foo 1 >>".to_string(),
        spec_body("good.xml"),
    ];
    let mut doc = open(build(&bodies, "/Root 1 0 R"));
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    for index in [1, 2, 3, 4, 6, 7, 8] {
        assert!(matches!(set_associated_relationship(&mut doc, index, "Data"), Err(OrganizeError::Invalid(_))), "entry {index}");
    }
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before, "no object was written to");
    for index in [5, 9, 10, 11] {
        assert_eq!(set_associated_relationship(&mut doc, index, "Data").unwrap(), None, "entry {index}");
    }
}

/// The catalog's `/ViewerPreferences` as a dictionary, resolved.
fn viewer_preferences_of(doc: &Document) -> Option<pdfcraft_cos::Dict> {
    let c = doc.get(doc.root().unwrap()).as_dict().cloned().unwrap();
    c.get(b"ViewerPreferences").map(|o| doc.resolve(o)).and_then(|o| o.as_dict().cloned())
}

fn set_preferences(doc: &mut Document, entries: Vec<(&str, Object)>) {
    let mut vp = pdfcraft_cos::Dict::new();
    for (k, v) in entries {
        vp.set(k.as_bytes().to_vec(), v);
    }
    let root = doc.root().unwrap();
    doc.update_dict(root, |c| c.set(b"ViewerPreferences".to_vec(), Object::Dict(vp))).unwrap();
}

#[test]
fn print_presets_round_trip_through_a_save_and_a_default_removes_the_entry() {
    use crate::print_presets::{Duplex, PrintPresets, PrintScaling, print_presets, set_print_presets, unreadable_print_presets};
    let mut doc = open(fixture());
    assert_eq!(print_presets(&doc), PrintPresets::default());
    assert!(viewer_preferences_of(&doc).is_none());
    let want = PrintPresets {
        scaling: PrintScaling::None,
        duplex: Some(Duplex::FlipShortEdge),
        pick_tray_by_size: Some(true),
        page_ranges: vec![(1, 2), (3, 3)],
        copies: Some(4),
    };
    set_print_presets(&mut doc, &want).unwrap();
    let doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
    assert_eq!(print_presets(&doc), want);
    assert!(unreadable_print_presets(&doc).is_empty());
    let vp = viewer_preferences_of(&doc).unwrap();
    assert_eq!(vp.name(b"PrintScaling"), Some(&b"None"[..]));
    assert_eq!(vp.name(b"Duplex"), Some(&b"DuplexFlipShortEdge"[..]));
    assert_eq!(vp.get(b"PickTrayByPDFSize"), Some(&Object::Bool(true)));
    assert_eq!(vp.get(b"NumCopies"), Some(&Object::Int(4)));
    assert_eq!(vp.get(b"PrintPageRange"), Some(&Object::Array(vec![Object::Int(1), Object::Int(2), Object::Int(3), Object::Int(3)])));

    // Each setting back to its default removes its entry, and an empty dictionary goes with them.
    let mut doc = doc;
    set_print_presets(&mut doc, &PrintPresets { copies: Some(4), ..PrintPresets::default() }).unwrap();
    let keys: Vec<Vec<u8>> = viewer_preferences_of(&doc).unwrap().iter().map(|(k, _)| k.clone()).collect();
    assert_eq!(keys, [b"NumCopies".to_vec()]);
    set_print_presets(&mut doc, &PrintPresets::default()).unwrap();
    assert!(viewer_preferences_of(&doc).is_none());
}

#[test]
fn print_presets_beside_other_preferences_and_through_indirect_values() {
    use crate::print_presets::{Duplex, PrintScaling, print_presets, set_print_presets};
    let mut doc = open(fixture());
    let copies = doc.add(Object::Int(3));
    let range = doc.add(Object::Int(2));
    let mut vp = pdfcraft_cos::Dict::new();
    vp.set(b"NumCopies".to_vec(), Object::Ref(copies));
    vp.set(b"PrintPageRange".to_vec(), Object::Array(vec![Object::Int(1), Object::Ref(range)]));
    vp.set(b"Duplex".to_vec(), Object::name("Simplex"));
    vp.set(b"FitWindow".to_vec(), Object::Bool(true));
    let vp_ref = doc.add(Object::Dict(vp));
    let root = doc.root().unwrap();
    doc.update_dict(root, |c| c.set(b"ViewerPreferences".to_vec(), Object::Ref(vp_ref))).unwrap();
    let mut p = print_presets(&doc);
    assert_eq!((p.copies, p.page_ranges.clone(), p.duplex), (Some(3), vec![(1, 2)], Some(Duplex::Simplex)));
    p.scaling = PrintScaling::None;
    set_print_presets(&mut doc, &p).unwrap();
    let vp = viewer_preferences_of(&doc).unwrap();
    assert_eq!(vp.get(b"FitWindow"), Some(&Object::Bool(true)), "a preference that is not a print one stays");
    assert_eq!(vp.name(b"PrintScaling"), Some(&b"None"[..]));
    assert_eq!(print_presets(&doc), p);
    // The dictionary behind the reference was changed where it is, not copied into the catalog.
    let c = doc.get(root).as_dict().cloned().unwrap();
    assert_eq!(c.get(b"ViewerPreferences"), Some(&Object::Ref(vp_ref)));
}

#[test]
fn print_presets_that_cannot_be_read_are_named_kept_and_replaced_only_by_a_real_value() {
    use crate::print_presets::{Duplex, PrintPresets, print_presets, set_print_presets, unreadable_print_presets};
    let mut doc = open(fixture());
    set_preferences(
        &mut doc,
        vec![
            ("PrintScaling", Object::name("Shrink")),
            ("Duplex", Object::Int(3)),
            ("PickTrayByPDFSize", Object::name("Yes")),
            ("NumCopies", Object::Int(9)),
            ("PrintPageRange", Object::Array(vec![Object::Int(3), Object::Int(1)])),
        ],
    );
    assert_eq!(print_presets(&doc), PrintPresets::default());
    assert_eq!(unreadable_print_presets(&doc), ["scaling", "duplex", "pick_tray_by_size", "copies", "page_ranges"]);
    // Asking for what is read, the default, changes nothing: the entries are not removed.
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    set_print_presets(&mut doc, &PrintPresets::default()).unwrap();
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
    // Setting one replaces that one only.
    set_print_presets(&mut doc, &PrintPresets { duplex: Some(Duplex::FlipLongEdge), ..PrintPresets::default() }).unwrap();
    assert_eq!(unreadable_print_presets(&doc), ["scaling", "pick_tray_by_size", "copies", "page_ranges"]);
    let vp = viewer_preferences_of(&doc).unwrap();
    assert_eq!(vp.get(b"NumCopies"), Some(&Object::Int(9)));
    assert_eq!(vp.name(b"Duplex"), Some(&b"DuplexFlipLongEdge"[..]));
    // Other malformed range arrays: odd length, a real number, a zero page, too many pairs.
    for items in [
        vec![Object::Int(1)],
        vec![Object::Real(1.0), Object::Int(2)],
        vec![Object::Int(0), Object::Int(2)],
        (0..514).map(|_| Object::Int(1)).collect(),
    ] {
        set_preferences(&mut doc, vec![("PrintPageRange", Object::Array(items))]);
        assert_eq!(unreadable_print_presets(&doc), ["page_ranges"]);
    }
    // Table 147 lists 2 through 5, so a stored 1 is not read as a number of copies and stays.
    set_preferences(&mut doc, vec![("NumCopies", Object::Int(1))]);
    assert_eq!(unreadable_print_presets(&doc), ["copies"]);
    assert_eq!(print_presets(&doc).copies, None);
    let read = print_presets(&doc);
    set_print_presets(&mut doc, &PrintPresets { copies: None, ..read }).unwrap();
    assert_eq!(viewer_preferences_of(&doc).unwrap().get(b"NumCopies"), Some(&Object::Int(1)));
    set_preferences(&mut doc, vec![("PrintPageRange", Object::Array((0..512).map(|_| Object::Int(1)).collect()))]);
    assert!(unreadable_print_presets(&doc).is_empty());
    assert_eq!(print_presets(&doc).page_ranges.len(), 256);
}

#[test]
fn a_print_preset_that_is_refused_writes_nothing() {
    use crate::print_presets::{Duplex, PrintPresets, set_print_presets};
    let mut doc = open(fixture());
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    let base = PrintPresets { duplex: Some(Duplex::Simplex), ..PrintPresets::default() };
    for (p, message) in [
        (PrintPresets { copies: Some(0), ..base.clone() }, "copies must be between 2 and 5, not 0"),
        (PrintPresets { copies: Some(1), ..base.clone() }, "copies must be between 2 and 5, not 1"),
        (PrintPresets { copies: Some(6), ..base.clone() }, "copies must be between 2 and 5, not 6"),
        (PrintPresets { page_ranges: vec![(1, 1); 257], ..base.clone() }, "a print page range list holds at most 256 ranges, not 257"),
        (
            PrintPresets { page_ranges: vec![(0, 2)], ..base.clone() },
            "print page range 0-2 must start at page 1 or later and not end before it starts",
        ),
        (
            PrintPresets { page_ranges: vec![(3, 2)], ..base.clone() },
            "print page range 3-2 must start at page 1 or later and not end before it starts",
        ),
        (PrintPresets { page_ranges: vec![(2, 4)], ..base.clone() }, "print page range 2-4 goes past the last page (3)"),
    ] {
        assert_eq!(set_print_presets(&mut doc, &p).unwrap_err().to_string(), message);
        assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before, "{message}");
    }
    // A /ViewerPreferences that is not a dictionary is not replaced.
    let root = doc.root().unwrap();
    doc.update_dict(root, |c| c.set(b"ViewerPreferences".to_vec(), Object::Int(7))).unwrap();
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(
        set_print_presets(&mut doc, &base).unwrap_err().to_string(),
        "the document's /ViewerPreferences is not a dictionary, so it was left as it is"
    );
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
}

#[test]
fn a_stored_page_range_past_the_last_page_does_not_block_another_change() {
    use crate::print_presets::{Duplex, print_presets, set_print_presets};
    let mut doc = open(fixture());
    set_preferences(&mut doc, vec![("PrintPageRange", Object::Array(vec![Object::Int(2), Object::Int(40)]))]);
    let mut p = print_presets(&doc);
    assert_eq!(p.page_ranges, [(2, 40)]);
    p.duplex = Some(Duplex::Simplex);
    set_print_presets(&mut doc, &p).unwrap();
    assert_eq!(print_presets(&doc).page_ranges, [(2, 40)]);
    // Writing a different range is checked.
    p.page_ranges = vec![(2, 41)];
    assert_eq!(set_print_presets(&mut doc, &p).unwrap_err().to_string(), "print page range 2-41 goes past the last page (3)");
}

#[test]
fn an_explicit_false_and_no_entry_are_different_and_each_is_kept() {
    use crate::print_presets::{PrintPresets, print_presets, set_print_presets};
    let mut doc = open(fixture());
    set_preferences(&mut doc, vec![("PickTrayByPDFSize", Object::Bool(false))]);
    assert_eq!(print_presets(&doc).pick_tray_by_size, Some(false));
    // Asking for what is read changes nothing, so the explicit false stays.
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    let read = print_presets(&doc);
    set_print_presets(&mut doc, &read).unwrap();
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
    // No entry removes it; an explicit false is written as false.
    set_print_presets(&mut doc, &PrintPresets::default()).unwrap();
    assert!(viewer_preferences_of(&doc).is_none());
    set_print_presets(&mut doc, &PrintPresets { pick_tray_by_size: Some(false), ..PrintPresets::default() }).unwrap();
    assert_eq!(viewer_preferences_of(&doc).unwrap().get(b"PickTrayByPDFSize"), Some(&Object::Bool(false)));
    assert_eq!(print_presets(&doc).pick_tray_by_size, Some(false));
}

#[test]
fn a_null_or_dangling_viewer_preferences_is_absent_and_another_object_is_named() {
    use crate::print_presets::{Duplex, PrintPresets, print_presets, read_print_presets, set_print_presets};
    let want = PrintPresets { duplex: Some(Duplex::Simplex), ..PrintPresets::default() };
    for value in [Object::Null, Object::Ref(pdfcraft_cos::ObjRef::new(999_999, 0))] {
        let mut doc = open(fixture());
        let root = doc.root().unwrap();
        doc.update_dict(root, |c| c.set(b"ViewerPreferences".to_vec(), value.clone())).unwrap();
        assert_eq!(read_print_presets(&doc), (PrintPresets::default(), vec![]), "{value:?}");
        set_print_presets(&mut doc, &want).unwrap();
        assert_eq!(print_presets(&doc), want, "{value:?}");
        assert!(matches!(doc.get(root).as_dict().unwrap().get(b"ViewerPreferences"), Some(Object::Dict(_))), "{value:?}");
    }
    // Anything else is named, read as no entry, and never replaced.
    for value in [Object::Int(7), Object::name("Yes"), Object::Array(vec![])] {
        let mut doc = open(fixture());
        let root = doc.root().unwrap();
        doc.update_dict(root, |c| c.set(b"ViewerPreferences".to_vec(), value.clone())).unwrap();
        assert_eq!(read_print_presets(&doc), (PrintPresets::default(), vec!["viewer_preferences"]), "{value:?}");
        assert!(set_print_presets(&mut doc, &want).is_err(), "{value:?}");
        assert_eq!(doc.get(root).as_dict().unwrap().get(b"ViewerPreferences"), Some(&value));
    }
}

/// A direct `/PrintPageRange` of a million page numbers is read as unreadable from its length alone and
/// kept through a change to another setting, whose other entries are written beside it.
#[test]
fn a_huge_direct_range_array_is_unreadable_from_its_length_and_kept() {
    use crate::print_presets::{Duplex, PrintPresets, print_presets, read_print_presets, set_print_presets};
    let mut doc = open(fixture());
    // Every entry is a usable page number, so only the length makes the array unreadable.
    let huge: Vec<Object> = (0..1_000_000).map(|_| Object::Int(1)).collect();
    set_preferences(&mut doc, vec![("PrintPageRange", Object::Array(huge))]);
    let (p, bad) = read_print_presets(&doc);
    assert_eq!((p, bad), (PrintPresets::default(), vec!["page_ranges"]));
    let read = print_presets(&doc);
    set_print_presets(&mut doc, &PrintPresets { duplex: Some(Duplex::Simplex), ..read }).unwrap();
    let vp = viewer_preferences_of(&doc).unwrap();
    assert!(matches!(vp.get(b"PrintPageRange"), Some(Object::Array(a)) if a.len() == 1_000_000));
    assert_eq!(vp.name(b"Duplex"), Some(&b"Simplex"[..]));
}

/// The catalog's `/URI`, resolved.
fn uri_dict_of(doc: &Document) -> Option<pdfcraft_cos::Dict> {
    let c = doc.get(doc.root().unwrap()).as_dict().cloned().unwrap();
    c.get(b"URI").map(|o| doc.resolve(o)).and_then(|o| o.as_dict().cloned())
}

fn set_catalog_entry(doc: &mut Document, key: &str, value: Object) {
    let root = doc.root().unwrap();
    doc.update_dict(root, |c| c.set(key.as_bytes().to_vec(), value)).unwrap();
}

fn uri_with(entries: Vec<(&str, Object)>) -> Object {
    let mut d = pdfcraft_cos::Dict::new();
    for (k, v) in entries {
        d.set(k.as_bytes().to_vec(), v);
    }
    Object::Dict(d)
}

fn string(bytes: &[u8]) -> Object {
    Object::String(pdfcraft_cos::PdfString::literal(bytes.to_vec()))
}

#[test]
fn base_url_round_trips_through_a_save_and_none_removes_the_entry() {
    use crate::base_url::{read_base_url, set_base_url};
    let mut doc = open(fixture());
    assert_eq!(read_base_url(&doc), (None, vec![]));
    set_base_url(&mut doc, Some("https://example.com/docs/")).unwrap();
    let mut doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
    assert_eq!(read_base_url(&doc), (Some("https://example.com/docs/".to_owned()), vec![]));
    assert_eq!(uri_dict_of(&doc).unwrap().get(b"Base"), Some(&string(b"https://example.com/docs/")));
    // A new value replaces it, and none takes the entry away with the empty dictionary.
    set_base_url(&mut doc, Some("HTTP://Example.org/")).unwrap();
    assert_eq!(read_base_url(&doc).0.as_deref(), Some("HTTP://Example.org/"));
    set_base_url(&mut doc, None).unwrap();
    assert!(uri_dict_of(&doc).is_none());
    assert_eq!(read_base_url(&doc), (None, vec![]));
}

#[test]
fn a_base_url_beside_other_entries_and_through_references() {
    use crate::base_url::{read_base_url, set_base_url};
    // /URI behind a chain of two references, /Base behind another.
    let mut doc = open(fixture());
    let base = doc.add(string(b"https://a.example/x/"));
    let dict = doc.add(uri_with(vec![("IsMap", Object::Bool(true)), ("Base", Object::Ref(base))]));
    let hop = doc.add(Object::Ref(dict));
    set_catalog_entry(&mut doc, "URI", Object::Ref(hop));
    assert_eq!(read_base_url(&doc), (Some("https://a.example/x/".to_owned()), vec![]));
    set_base_url(&mut doc, Some("https://b.example/")).unwrap();
    let c = doc.get(doc.root().unwrap()).as_dict().cloned().unwrap();
    assert_eq!(c.get(b"URI"), Some(&Object::Ref(hop)), "the catalog still holds the reference");
    let d = doc.get(dict).as_dict().cloned().unwrap();
    assert_eq!(d.get(b"Base"), Some(&string(b"https://b.example/")), "changed where it is");
    assert_eq!(d.get(b"IsMap"), Some(&Object::Bool(true)), "the other entry stays");
    // None removes /Base only; the other entry keeps the dictionary.
    set_base_url(&mut doc, None).unwrap();
    let d = doc.get(dict).as_dict().cloned().unwrap();
    assert!(d.get(b"Base").is_none());
    assert_eq!(d.get(b"IsMap"), Some(&Object::Bool(true)));
    assert!(uri_dict_of(&doc).is_some());
    // A direct dictionary with only /Base goes whole, and so does an indirect one.
    let mut doc = open(fixture());
    set_catalog_entry(&mut doc, "URI", uri_with(vec![("Base", string(b"https://a.example/"))]));
    set_base_url(&mut doc, None).unwrap();
    assert!(uri_dict_of(&doc).is_none());
    let only = doc.add(uri_with(vec![("Base", string(b"https://a.example/"))]));
    set_catalog_entry(&mut doc, "URI", Object::Ref(only));
    set_base_url(&mut doc, None).unwrap();
    assert!(uri_dict_of(&doc).is_none(), "the emptied indirect dictionary is taken out of the catalog");
}

#[test]
fn a_base_url_that_cannot_be_read_is_named_kept_and_replaced_only_by_a_real_value() {
    use crate::base_url::{MAX_BASE_URL_BYTES, read_base_url, set_base_url};
    let long = vec![b'a'; MAX_BASE_URL_BYTES + 1];
    for bad in [Object::Int(3), Object::name("https"), string(&long), string(&[0xFF, 0xFE, b'a']), Object::Array(vec![])] {
        let mut doc = open(fixture());
        set_catalog_entry(&mut doc, "URI", uri_with(vec![("Base", bad.clone())]));
        assert_eq!(read_base_url(&doc), (None, vec!["base"]), "{bad:?}");
        // Asking for none is asking for what is read: nothing changes, the entry stays.
        let before = write_full(&doc, &SaveOptions::default()).unwrap();
        set_base_url(&mut doc, None).unwrap();
        assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before, "{bad:?}");
        set_base_url(&mut doc, Some("https://example.com/")).unwrap();
        assert_eq!(read_base_url(&doc), (Some("https://example.com/".to_owned()), vec![]), "{bad:?}");
    }
    // The longest base that is read: exactly the limit.
    let mut doc = open(fixture());
    let edge = format!("https://e.example/{}", "a".repeat(MAX_BASE_URL_BYTES - 18));
    assert_eq!(edge.len(), MAX_BASE_URL_BYTES);
    set_base_url(&mut doc, Some(&edge)).unwrap();
    assert_eq!(read_base_url(&doc).0.as_deref(), Some(edge.as_str()));
}

#[test]
fn a_base_url_that_is_refused_writes_nothing() {
    use crate::base_url::{MAX_BASE_URL_BYTES, set_base_url};
    let mut doc = open(fixture());
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    let too_long = format!("https://e.example/{}", "a".repeat(MAX_BASE_URL_BYTES - 17));
    let web = "a base URL must be a web address that names its host, such as https://example.com/docs/";
    let ascii = "a base URL must be printable ASCII with no space";
    for (base, message) in [
        (too_long.as_str(), "a base URL holds at most 8192 bytes, not 8193"),
        ("https://example.com/a b", ascii),
        ("https://example.com/\u{e9}", ascii),
        ("https://example.com/\n", ascii),
        ("ftp://example.com/", web),
        ("example.com/docs/", web),
        ("https://", web),
        ("https:///example.com/", web),
        ("http://?x", web),
        ("javascript:alert(1)", web),
        ("", web),
    ] {
        assert_eq!(set_base_url(&mut doc, Some(base)).unwrap_err().to_string(), message, "{base:?}");
        assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before, "{base:?}");
    }
    // A stored base that would be refused as a new value is accepted when it is sent back as it
    // is, and changes no byte; a different one is checked.
    let mut stored = open(fixture());
    set_catalog_entry(&mut stored, "URI", uri_with(vec![("Base", string(b"ftp://example.com/x/"))]));
    let before = write_full(&stored, &SaveOptions::default()).unwrap();
    set_base_url(&mut stored, Some("ftp://example.com/x/")).unwrap();
    assert_eq!(write_full(&stored, &SaveOptions::default()).unwrap(), before);
    assert_eq!(set_base_url(&mut stored, Some("ftp://example.com/y/")).unwrap_err().to_string(), web);
    assert_eq!(write_full(&stored, &SaveOptions::default()).unwrap(), before);
    // A /URI that is not a dictionary is not replaced, and none is not a change to it.
    for other in [Object::Int(7), Object::name("x"), Object::Array(vec![]), string(b"https://a.example/")] {
        let mut doc = open(fixture());
        set_catalog_entry(&mut doc, "URI", other);
        let before = write_full(&doc, &SaveOptions::default()).unwrap();
        assert_eq!(
            set_base_url(&mut doc, Some("https://example.com/")).unwrap_err().to_string(),
            "the document's /URI is not a dictionary, or is behind more than 128 references, so it was left as it is"
        );
        set_base_url(&mut doc, None).unwrap();
        assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
    }
}

#[test]
fn a_null_dangling_or_endless_uri_is_read_as_the_tool_describes() {
    use crate::base_url::{read_base_url, set_base_url};
    // Null and a reference to nothing are absent: nothing unreadable, and a base starts a dictionary.
    for absent in [Object::Null, Object::Ref(pdfcraft_cos::ObjRef::new(900, 0))] {
        let mut doc = open(fixture());
        set_catalog_entry(&mut doc, "URI", absent);
        assert_eq!(read_base_url(&doc), (None, vec![]));
        set_base_url(&mut doc, Some("https://example.com/")).unwrap();
        assert_eq!(read_base_url(&doc).0.as_deref(), Some("https://example.com/"));
        assert!(uri_dict_of(&doc).unwrap().get(b"Base").is_some());
    }
    // A chain of 100 references to the dictionary is followed, for /URI and for /Base, and the
    // dictionary at its end is the one changed.
    let chain = |doc: &mut Document, end: Object, hops: usize| {
        let mut at = doc.add(end);
        for _ in 0..hops {
            at = doc.add(Object::Ref(at));
        }
        at
    };
    let mut doc = open(fixture());
    let base = chain(&mut doc, string(b"https://deep.example/"), 100);
    let end = chain(&mut doc, uri_with(vec![("Base", Object::Ref(base))]), 100);
    set_catalog_entry(&mut doc, "URI", Object::Ref(end));
    assert_eq!(read_base_url(&doc), (Some("https://deep.example/".to_owned()), vec![]));
    set_base_url(&mut doc, Some("https://example.com/")).unwrap();
    assert_eq!(read_base_url(&doc).0.as_deref(), Some("https://example.com/"));
    // A chain of 200 is not reached: /URI is `uri`, refused and left, and /Base is `base`.
    let mut doc = open(fixture());
    let far = chain(&mut doc, uri_with(vec![("Base", string(b"https://a.example/"))]), 200);
    set_catalog_entry(&mut doc, "URI", Object::Ref(far));
    assert_eq!(read_base_url(&doc), (None, vec!["uri"]));
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(
        set_base_url(&mut doc, Some("https://example.com/")).unwrap_err().to_string(),
        "the document's /URI is not a dictionary, or is behind more than 128 references, so it was left as it is"
    );
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
    let mut doc = open(fixture());
    let far_base = chain(&mut doc, string(b"https://a.example/"), 200);
    set_catalog_entry(&mut doc, "URI", uri_with(vec![("Base", Object::Ref(far_base))]));
    assert_eq!(read_base_url(&doc), (None, vec!["base"]));
}

#[test]
fn a_uri_that_is_a_stream_counts_by_its_dictionary_like_the_viewers_reader() {
    use crate::base_url::{read_base_url, set_base_url};
    let mut doc = open(fixture());
    let mut d = pdfcraft_cos::Dict::new();
    d.set(b"Base".to_vec(), string(b"https://stream.example/"));
    d.set(b"IsMap".to_vec(), Object::Bool(true));
    let at = doc.add(Object::Stream(pdfcraft_cos::Stream::from_raw(d, b"x".to_vec())));
    set_catalog_entry(&mut doc, "URI", Object::Ref(at));
    assert_eq!(read_base_url(&doc), (Some("https://stream.example/".to_owned()), vec![]));
    set_base_url(&mut doc, Some("https://example.com/")).unwrap();
    let doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
    assert_eq!(read_base_url(&doc).0.as_deref(), Some("https://example.com/"));
    let held = uri_dict_of(&doc).unwrap();
    assert_eq!(held.get(b"IsMap"), Some(&Object::Bool(true)), "the stream's other entry stays");
}

#[test]
fn asking_for_the_stored_base_url_changes_no_byte() {
    use crate::base_url::set_base_url;
    let mut doc = open(fixture());
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    set_base_url(&mut doc, None).unwrap();
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before, "no base, none asked");
    set_base_url(&mut doc, Some("https://example.com/")).unwrap();
    let with = write_full(&doc, &SaveOptions::default()).unwrap();
    assert_ne!(with, before);
    set_base_url(&mut doc, Some("https://example.com/")).unwrap();
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), with);
}

/// A dictionary of names, `(key, value)` each.
fn names_dict(pairs: &[(&str, &str)]) -> Object {
    uri_with(pairs.iter().map(|(k, v)| (*k, Object::name(v))).collect())
}

/// The fixture with a `/StructTreeRoot` (one indirect dictionary) that has `role_map` as its
/// `/RoleMap` when given; returns the document and the tree root's reference.
fn tree_doc(role_map: Option<Object>) -> (Document, pdfcraft_cos::ObjRef) {
    let mut doc = open(fixture());
    let mut entries = vec![("Type", Object::name("StructTreeRoot"))];
    if let Some(m) = role_map {
        entries.push(("RoleMap", m));
    }
    let root = doc.add(uri_with(entries));
    set_catalog_entry(&mut doc, "StructTreeRoot", Object::Ref(root));
    (doc, root)
}

/// The role map of `doc` as `(role, maps_to)` rows.
fn roles(doc: &Document) -> Vec<(String, Option<String>)> {
    crate::role_map::read_role_map(doc).entries.into_iter().map(|e| (e.role, e.maps_to)).collect()
}

fn pair(a: &str, b: &str) -> (String, Option<String>) {
    (a.to_owned(), Some(b.to_owned()))
}

/// The stored `/RoleMap` dictionary, resolved.
fn stored_role_map(doc: &Document) -> Option<pdfcraft_cos::Dict> {
    let c = doc.get(doc.root().unwrap()).as_dict().cloned().unwrap();
    let root = c.get(b"StructTreeRoot").map(|o| doc.resolve(o))?;
    root.as_dict()?.get(b"RoleMap").map(|o| doc.resolve(o)).and_then(|o| o.as_dict().cloned())
}

#[test]
fn a_role_map_entry_round_trips_through_a_save_and_removing_the_last_takes_the_dictionary() {
    use crate::role_map::{read_role_map, set_role_map};
    let (mut doc, _) = tree_doc(None);
    let none = read_role_map(&doc);
    assert!(none.has_structure_tree && none.entries.is_empty() && none.count == 0 && !none.truncated && none.unreadable.is_empty());
    set_role_map(&mut doc, "Chapter", Some("H1")).unwrap();
    set_role_map(&mut doc, "Photo", Some("Figure")).unwrap();
    let mut doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
    assert_eq!(roles(&doc), [pair("Chapter", "H1"), pair("Photo", "Figure")]);
    assert_eq!(stored_role_map(&doc).unwrap().get(b"Chapter"), Some(&Object::name("H1")), "the file holds a name");
    // A new target replaces the old one in place; removing one leaves the other.
    set_role_map(&mut doc, "Chapter", Some("H2")).unwrap();
    assert_eq!(roles(&doc), [pair("Chapter", "H2"), pair("Photo", "Figure")]);
    set_role_map(&mut doc, "Chapter", None).unwrap();
    assert_eq!(roles(&doc), [pair("Photo", "Figure")]);
    // The last one takes the dictionary with it.
    set_role_map(&mut doc, "Photo", None).unwrap();
    assert!(stored_role_map(&doc).is_none());
    let tree = doc.resolve(doc.get(doc.root().unwrap()).as_dict().unwrap().get(b"StructTreeRoot").unwrap());
    assert!(tree.as_dict().unwrap().get(b"RoleMap").is_none());
    assert_eq!(tree.as_dict().unwrap().name(b"Type"), Some(&b"StructTreeRoot"[..]), "the rest of the tree root stays");
}

#[test]
fn a_role_map_is_changed_where_it_is_through_references_and_in_a_direct_tree_root() {
    use crate::role_map::{read_role_map, set_role_map};
    // /StructTreeRoot behind a chain of two references, /RoleMap behind another, the value of an
    // entry behind one more.
    let mut doc = open(fixture());
    let target = doc.add(Object::name("H1"));
    let map = doc.add(uri_with(vec![("Chapter", Object::Ref(target)), ("Keep", Object::name("P"))]));
    let map_hop = doc.add(Object::Ref(map));
    let root = doc.add(uri_with(vec![("Type", Object::name("StructTreeRoot")), ("RoleMap", Object::Ref(map_hop))]));
    let root_hop = doc.add(Object::Ref(root));
    set_catalog_entry(&mut doc, "StructTreeRoot", Object::Ref(root_hop));
    assert_eq!(roles(&doc), [pair("Chapter", "H1"), pair("Keep", "P")], "a value behind a reference is read");
    set_role_map(&mut doc, "Chapter", Some("H2")).unwrap();
    let c = doc.get(doc.root().unwrap()).as_dict().cloned().unwrap();
    assert_eq!(c.get(b"StructTreeRoot"), Some(&Object::Ref(root_hop)), "the catalog still holds the reference");
    assert_eq!(doc.get(root).as_dict().unwrap().get(b"RoleMap"), Some(&Object::Ref(map_hop)), "so does the tree root");
    assert_eq!(doc.get(map).as_dict().unwrap().get(b"Chapter"), Some(&Object::name("H2")), "changed where it is");
    assert_eq!(roles(&doc), [pair("Chapter", "H2"), pair("Keep", "P")]);
    // Emptying the indirect dictionary takes it out of the tree root, which stays.
    set_role_map(&mut doc, "Chapter", None).unwrap();
    set_role_map(&mut doc, "Keep", None).unwrap();
    assert!(doc.get(root).as_dict().unwrap().get(b"RoleMap").is_none());
    assert_eq!(doc.get(root).as_dict().unwrap().name(b"Type"), Some(&b"StructTreeRoot"[..]));
    // A tree root written in the catalog itself, with a direct and then an indirect map.
    let mut doc = open(fixture());
    set_catalog_entry(&mut doc, "StructTreeRoot", uri_with(vec![("Type", Object::name("StructTreeRoot")), ("RoleMap", names_dict(&[("A", "P")]))]));
    assert_eq!(roles(&doc), [pair("A", "P")]);
    set_role_map(&mut doc, "B", Some("H1")).unwrap();
    assert_eq!(roles(&doc), [pair("A", "P"), pair("B", "H1")]);
    set_role_map(&mut doc, "A", None).unwrap();
    set_role_map(&mut doc, "B", None).unwrap();
    assert!(!read_role_map(&doc).entries.iter().any(|_| true) && read_role_map(&doc).has_structure_tree);
    let map = doc.add(names_dict(&[("A", "P")]));
    set_catalog_entry(&mut doc, "StructTreeRoot", uri_with(vec![("RoleMap", Object::Ref(map))]));
    set_role_map(&mut doc, "A", None).unwrap();
    let c = doc.get(doc.root().unwrap()).as_dict().cloned().unwrap();
    assert!(matches!(c.get(b"StructTreeRoot"), Some(Object::Dict(d)) if d.is_empty()), "the direct tree root lost its emptied /RoleMap");
    // A tree root that is a stream counts by its dictionary, as Document::resolve and as_dict do.
    let mut doc = open(fixture());
    let mut s = pdfcraft_cos::Stream::from_raw(pdfcraft_cos::Dict::new(), b"x".to_vec());
    s.dict.set(b"RoleMap".to_vec(), names_dict(&[("A", "P")]));
    let stream = doc.add(Object::Stream(s));
    set_catalog_entry(&mut doc, "StructTreeRoot", Object::Ref(stream));
    assert_eq!(roles(&doc), [pair("A", "P")]);
    set_role_map(&mut doc, "B", Some("H1")).unwrap();
    assert_eq!(roles(&doc), [pair("A", "P"), pair("B", "H1")]);
    assert!(
        matches!(&*doc.get(stream), Object::Stream(s) if matches!(s.dict.get(b"RoleMap"), Some(Object::Dict(d)) if d.name(b"B") == Some(&b"H1"[..])))
    );
}

#[test]
fn a_role_map_that_is_a_stream_the_catalog_or_the_tree_root_is_unreadable_and_never_written() {
    use crate::role_map::{read_role_map, set_role_map};
    let refused = "the structure tree's /StructTreeRoot or /RoleMap cannot be changed (it is not a dictionary, is behind more than 33 references, is a stream role map, or is the same object as the catalog, the /Pages node, a page or inner /Pages node, or (for a map) the tree root or its other map, or the page tree has over 1000000 nodes or 2000000 /Kids entries and could not be checked), so it was left as it is";
    // /RoleMap that is a stream: its keys (/Length, /Filter) are not roles, so none is listed.
    let mut doc = open(fixture());
    let mut s = pdfcraft_cos::Stream::from_raw(pdfcraft_cos::Dict::new(), b"x".to_vec());
    s.dict.set(b"Length".to_vec(), Object::Int(1));
    s.dict.set(b"A".to_vec(), Object::name("P"));
    let stream = doc.add(Object::Stream(s));
    let root = doc.add(uri_with(vec![("RoleMap", Object::Ref(stream))]));
    set_catalog_entry(&mut doc, "StructTreeRoot", Object::Ref(root));
    let r = read_role_map(&doc);
    assert_eq!((r.has_structure_tree, r.entries.len(), r.count, r.unreadable.clone()), (true, 0, 0, vec!["role_map"]));
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(set_role_map(&mut doc, "B", Some("H1")).unwrap_err().to_string(), refused);
    set_role_map(&mut doc, "A", None).unwrap();
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
    // /RoleMap that is the tree root, or the catalog: editing it would change /K or /Pages.
    for target in ["root", "catalog"] {
        let mut doc = open(fixture());
        let catalog = doc.root().unwrap();
        let root = doc.add(uri_with(vec![("Type", Object::name("StructTreeRoot"))]));
        set_catalog_entry(&mut doc, "StructTreeRoot", Object::Ref(root));
        let at = if target == "root" { root } else { catalog };
        doc.update_dict(root, |d| d.set(b"RoleMap".to_vec(), Object::Ref(at))).unwrap();
        let r = read_role_map(&doc);
        assert_eq!((r.entries.len(), r.unreadable.clone()), (0, vec!["role_map"]), "{target}");
        let before = write_full(&doc, &SaveOptions::default()).unwrap();
        assert_eq!(set_role_map(&mut doc, "B", Some("H1")).unwrap_err().to_string(), refused, "{target}");
        assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before, "{target}");
        assert!(doc.get(catalog).as_dict().unwrap().get(b"Pages").is_some());
    }
    // A tree root that is the catalog.
    let mut doc = open(fixture());
    let catalog = doc.root().unwrap();
    set_catalog_entry(&mut doc, "StructTreeRoot", Object::Ref(catalog));
    let r = read_role_map(&doc);
    assert_eq!((r.has_structure_tree, r.unreadable.clone()), (false, vec!["struct_tree_root"]));
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(set_role_map(&mut doc, "B", Some("H1")).unwrap_err().to_string(), refused);
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
}

#[test]
fn taking_away_a_role_that_is_not_stored_needs_a_name_the_tool_writes_but_a_stored_key_goes_whatever_it_is() {
    use crate::role_map::{read_role_map, role_map_changes, set_role_map};
    let long = "k".repeat(200);
    let mut bad = pdfcraft_cos::Dict::new();
    bad.set(long.as_bytes().to_vec(), Object::name("P"));
    bad.set(b"Fine".to_vec(), Object::name("P"));
    let (mut doc, _) = tree_doc(Some(Object::Dict(bad)));
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    for (role, message) in [
        ("", "a role must be 1 to 127 bytes, not 0 bytes"),
        ("N\0", "a role must not contain a NUL byte"),
        (&*"j".repeat(128), "a role must be 1 to 127 bytes, not 128 bytes"),
    ] {
        assert_eq!(set_role_map(&mut doc, role, None).unwrap_err().to_string(), message);
        assert_eq!(role_map_changes(&doc, role, None).unwrap_err().to_string(), message);
        assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
    }
    // A good name that is not stored is no change.
    assert!(!role_map_changes(&doc, "Nothing", None).unwrap());
    set_role_map(&mut doc, "Nothing", None).unwrap();
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
    // A stored key over 127 bytes (which no tool call can write) is taken away by its exact name.
    assert!(role_map_changes(&doc, &long, None).unwrap());
    set_role_map(&mut doc, &long, None).unwrap();
    assert_eq!(roles(&doc), [pair("Fine", "P")]);
    // A stored empty key and a stored key with a NUL byte likewise.
    let mut odd = pdfcraft_cos::Dict::new();
    odd.set(Vec::<u8>::new(), Object::name("P"));
    odd.set(vec![b'N', 0], Object::name("P"));
    let (mut doc, _) = tree_doc(Some(Object::Dict(odd)));
    assert_eq!(read_role_map(&doc).count, 2);
    set_role_map(&mut doc, "", None).unwrap();
    set_role_map(&mut doc, "N\0", None).unwrap();
    assert_eq!(read_role_map(&doc).count, 0);
}

#[test]
fn a_tree_root_or_role_map_that_is_not_a_dictionary_is_named_and_never_replaced() {
    use crate::role_map::{read_role_map, set_role_map};
    let refused = "the structure tree's /StructTreeRoot or /RoleMap cannot be changed (it is not a dictionary, is behind more than 33 references, is a stream role map, or is the same object as the catalog, the /Pages node, a page or inner /Pages node, or (for a map) the tree root or its other map, or the page tree has over 1000000 nodes or 2000000 /Kids entries and could not be checked), so it was left as it is";
    for other in [Object::Int(7), Object::name("x"), Object::Array(vec![]), string(b"s")] {
        let mut doc = open(fixture());
        set_catalog_entry(&mut doc, "StructTreeRoot", other.clone());
        let r = read_role_map(&doc);
        assert_eq!((r.has_structure_tree, r.unreadable.clone()), (false, vec!["struct_tree_root"]), "{other:?}");
        let before = write_full(&doc, &SaveOptions::default()).unwrap();
        assert_eq!(set_role_map(&mut doc, "A", Some("P")).unwrap_err().to_string(), refused);
        // Taking away an entry that is not there is not a change.
        set_role_map(&mut doc, "A", None).unwrap();
        assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before, "{other:?}");

        let (mut doc, _) = tree_doc(Some(other.clone()));
        let r = read_role_map(&doc);
        assert_eq!((r.has_structure_tree, r.unreadable.clone(), r.entries.len()), (true, vec!["role_map"], 0), "{other:?}");
        let before = write_full(&doc, &SaveOptions::default()).unwrap();
        assert_eq!(set_role_map(&mut doc, "A", Some("P")).unwrap_err().to_string(), refused);
        set_role_map(&mut doc, "A", None).unwrap();
        assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before, "{other:?}");
    }
}

#[test]
fn a_role_map_change_that_is_refused_writes_nothing() {
    use crate::role_map::{MAX_ROLE_BYTES, set_role_map};
    let (mut doc, _) = tree_doc(Some(names_dict(&[("B", "C"), ("C", "D"), ("X", "Y")])));
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    let long = "a".repeat(MAX_ROLE_BYTES + 1);
    let role_bad = "a role must be 1 to 127 bytes, not ";
    let to_bad = "what a role maps to must be 1 to 127 bytes, not ";
    for (role, to, message) in [
        ("", "P", format!("{role_bad}0 bytes")),
        (long.as_str(), "P", format!("{role_bad}128 bytes")),
        ("A\0", "P", "a role must not contain a NUL byte".to_owned()),
        ("A", long.as_str(), format!("{to_bad}128 bytes")),
        ("A", "P\0Q", "what a role maps to must not contain a NUL byte".to_owned()),
        ("A", "A", "the role A cannot map to itself".to_owned()),
        // B maps to C and C to D, so D to B would be a loop of three, and D to C one of two.
        ("D", "B", "B already leads back to D, so mapping D to it would make a loop".to_owned()),
        ("D", "C", "C already leads back to D, so mapping D to it would make a loop".to_owned()),
        ("Y", "X", "X already leads back to Y, so mapping Y to it would make a loop".to_owned()),
    ] {
        assert_eq!(set_role_map(&mut doc, role, Some(to)).unwrap_err().to_string(), message, "{role:?} {to:?}");
        assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before, "{role:?} {to:?}");
    }
    // An empty target is no mapping here: the tool turns it into a removal before it gets here,
    // and an empty name given as a target is refused.
    assert_eq!(set_role_map(&mut doc, "A", Some("")).unwrap_err().to_string(), format!("{to_bad}0 bytes"));
    // A document with no structure tree.
    let mut plain = open(fixture());
    let before = write_full(&plain, &SaveOptions::default()).unwrap();
    assert_eq!(
        set_role_map(&mut plain, "A", Some("P")).unwrap_err().to_string(),
        "the document has no structure tree, so there is no role map to change"
    );
    set_role_map(&mut plain, "A", None).unwrap();
    assert_eq!(write_full(&plain, &SaveOptions::default()).unwrap(), before);
    // A chain N0 -> N1 -> ... -> N65. Mapping N64 to N0 closes a loop that is seen after
    // MAX_LOOP_STEPS (64) names (N1 to N64) are followed from N0; mapping N65 to N0 needs 65 and
    // is not seen.
    let names: Vec<String> = (0..=65).map(|i| format!("N{i}")).collect();
    let pairs: Vec<(&str, &str)> = names.windows(2).map(|w| (w[0].as_str(), w[1].as_str())).collect();
    let (mut doc, _) = tree_doc(Some(names_dict(&pairs)));
    let last = names[64].as_str();
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    set_role_map(&mut doc, &names[65], Some(&names[0])).expect("a loop longer than the steps followed is not seen");
    assert_ne!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
    let (mut doc, _) = tree_doc(Some(names_dict(&pairs)));
    let err = set_role_map(&mut doc, last, Some(&names[0])).unwrap_err().to_string();
    assert_eq!(err, format!("N0 already leads back to {last}, so mapping {last} to it would make a loop"));
}

#[test]
fn a_stored_entry_sent_back_is_no_change_and_a_value_that_is_not_a_name_is_listed_and_replaced() {
    use crate::role_map::{read_role_map, set_role_map};
    // A stored loop, and a stored name over 127 bytes, are sent back as they are: no byte changes.
    let long = "q".repeat(130);
    let (mut doc, _) = tree_doc(Some(names_dict(&[("A", "B"), ("B", "A"), ("L", long.as_str())])));
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    let r = read_role_map(&doc);
    assert_eq!(
        r.entries.iter().map(|e| (e.role.as_str(), e.loops, e.exact)).collect::<Vec<_>>(),
        [("A", true, true), ("B", true, true), ("L", false, false)]
    );
    assert_eq!(r.entries[2].maps_to.as_deref().map(str::len), Some(127), "a long name is listed cut");
    set_role_map(&mut doc, "A", Some("B")).unwrap();
    set_role_map(&mut doc, "L", Some(&long)).unwrap();
    set_role_map(&mut doc, "Nothing", None).unwrap();
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
    // A different target for a stored loop is checked as a new one.
    assert!(set_role_map(&mut doc, "A", Some("A")).is_err());
    set_role_map(&mut doc, "A", Some("P")).unwrap();
    assert_eq!(roles(&doc)[0], pair("A", "P"));
    assert!(!read_role_map(&doc).entries[0].loops);
    // A value that is not a name is listed with no target, is not a change to nothing, and is
    // replaced by a name or removed.
    let (mut doc, _) = tree_doc(Some(uri_with(vec![("N", Object::Int(5)), ("S", string(b"P")), ("Z", Object::Null)])));
    assert_eq!(roles(&doc), [("N".to_owned(), None), ("S".to_owned(), None), ("Z".to_owned(), None)]);
    let r = read_role_map(&doc);
    assert!(r.entries.iter().all(|e| !e.loops && e.exact));
    assert!(r.unreadable.is_empty());
    set_role_map(&mut doc, "N", Some("P")).unwrap();
    set_role_map(&mut doc, "S", None).unwrap();
    assert_eq!(roles(&doc), [pair("N", "P"), ("Z".to_owned(), None)]);
}

#[test]
fn the_references_followed_are_those_of_document_resolve() {
    use crate::role_map::{read_role_map, set_role_map};
    // A chain of n references to a dictionary: n = 33 resolves, n = 34 does not.
    let chain = |doc: &mut Document, end: Object, hops: usize| {
        let mut at = doc.add(end);
        for _ in 1..hops {
            at = doc.add(Object::Ref(at));
        }
        Object::Ref(at)
    };
    for (hops, readable) in [(33usize, true), (34, false)] {
        // /RoleMap at the end of the chain.
        let (mut doc, root) = tree_doc(None);
        let start = chain(&mut doc, names_dict(&[("A", "P")]), hops);
        doc.update_dict(root, |d| d.set(b"RoleMap".to_vec(), start.clone())).unwrap();
        assert_eq!(doc.resolve(&start).as_dict().is_some(), readable, "Document::resolve at {hops} references");
        let r = read_role_map(&doc);
        assert_eq!((r.entries.len(), r.unreadable.clone()), if readable { (1, vec![]) } else { (0, vec!["role_map"]) }, "{hops} references");
        let before = write_full(&doc, &SaveOptions::default()).unwrap();
        let result = set_role_map(&mut doc, "B", Some("H1"));
        if readable {
            result.unwrap();
            assert_eq!(roles(&doc), [pair("A", "P"), pair("B", "H1")]);
        } else {
            assert!(result.is_err());
            assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
        }
        // /StructTreeRoot at the end of one.
        let mut doc = open(fixture());
        let start = chain(&mut doc, uri_with(vec![("RoleMap", names_dict(&[("A", "P")]))]), hops);
        set_catalog_entry(&mut doc, "StructTreeRoot", start.clone());
        assert_eq!(doc.resolve(&start).as_dict().is_some(), readable);
        let r = read_role_map(&doc);
        assert_eq!(
            (r.has_structure_tree, r.unreadable.clone()),
            if readable { (true, vec![]) } else { (false, vec!["struct_tree_root"]) },
            "{hops} references"
        );
        // The value of an entry at the end of one: a name at 33, nothing at 34.
        let (mut doc, _) = tree_doc(None);
        let value = chain(&mut doc, Object::name("H1"), hops);
        let map = doc.add(uri_with(vec![("A", value)]));
        set_catalog_entry(&mut doc, "StructTreeRoot", uri_with(vec![("RoleMap", Object::Ref(map))]));
        assert_eq!(roles(&doc), [("A".to_owned(), readable.then(|| "H1".to_owned()))], "a value behind {hops} references");
    }
}

#[test]
fn a_null_or_dangling_role_map_is_absent_and_an_entry_starts_a_dictionary() {
    use crate::role_map::{read_role_map, set_role_map};
    for absent in [Object::Null, Object::Ref(pdfcraft_cos::ObjRef::new(900, 0))] {
        let (mut doc, root) = tree_doc(Some(absent));
        let r = read_role_map(&doc);
        assert!(r.has_structure_tree && r.entries.is_empty() && r.unreadable.is_empty());
        set_role_map(&mut doc, "A", Some("P")).unwrap();
        assert_eq!(roles(&doc), [pair("A", "P")]);
        assert!(matches!(doc.get(root).as_dict().unwrap().get(b"RoleMap"), Some(Object::Dict(_))));
    }
    // A tree root that is null or dangling is no tree.
    for absent in [Object::Null, Object::Ref(pdfcraft_cos::ObjRef::new(900, 0))] {
        let mut doc = open(fixture());
        set_catalog_entry(&mut doc, "StructTreeRoot", absent);
        let r = read_role_map(&doc);
        assert!(!r.has_structure_tree && r.unreadable.is_empty());
        assert!(set_role_map(&mut doc, "A", Some("P")).is_err());
    }
}

#[test]
fn the_listing_is_bounded_names_are_cut_and_not_exact_and_names_that_need_escaping_survive_a_save() {
    use crate::role_map::{MAX_LISTED_ROLES, read_role_map, set_role_map};
    let many: Vec<(String, String)> = (0..MAX_LISTED_ROLES + 44).map(|i| (format!("R{i}"), "P".to_owned())).collect();
    let pairs: Vec<(&str, &str)> = many.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let (doc, _) = tree_doc(Some(names_dict(&pairs)));
    let r = read_role_map(&doc);
    assert_eq!((r.count, r.entries.len(), r.truncated), (300, MAX_LISTED_ROLES, true));
    assert_eq!(r.entries.last().map(|e| e.role.as_str()), Some("R255"));
    // A key that is not UTF-8 is listed lossily and is not exact.
    let mut bad = pdfcraft_cos::Dict::new();
    bad.set(vec![b'A', 0xFF], Object::name("P"));
    bad.set(b"Fine".to_vec(), Object::name("P"));
    let (doc, _) = tree_doc(Some(Object::Dict(bad)));
    let r = read_role_map(&doc);
    assert_eq!(r.entries.iter().map(|e| e.exact).collect::<Vec<_>>(), [false, true]);
    assert_eq!(r.entries[0].role, "A\u{FFFD}");
    // A name with a space, a slash, brackets and a number sign is written as a name and read back.
    let (mut doc, _) = tree_doc(None);
    set_role_map(&mut doc, "My Role/(x)#[y]", Some("Sp ace")).unwrap();
    let doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
    assert_eq!(roles(&doc), [pair("My Role/(x)#[y]", "Sp ace")]);
}

/// The fixture with a `/StructTreeRoot` (one indirect dictionary) that has `class_map` as its
/// `/ClassMap` when given; returns the document and the tree root's reference.
fn class_tree(class_map: Option<Object>) -> (Document, pdfcraft_cos::ObjRef) {
    let mut doc = open(fixture());
    let mut entries = vec![("Type", Object::name("StructTreeRoot"))];
    if let Some(m) = class_map {
        entries.push(("ClassMap", m));
    }
    let root = doc.add(uri_with(entries));
    set_catalog_entry(&mut doc, "StructTreeRoot", Object::Ref(root));
    (doc, root)
}

/// An attribute dictionary with `/O` set to the name `owner`.
fn attrs(owner: &str) -> Object {
    uri_with(vec![("O", Object::name(owner)), ("Placement", Object::name("Block"))])
}

/// The class map of `doc` as `(class, kind, items, owners)` rows.
fn classes(doc: &Document) -> Vec<(String, &'static str, usize, Vec<Option<String>>)> {
    crate::class_map::read_class_map(doc).entries.into_iter().map(|e| (e.class, e.kind, e.items, e.owners)).collect()
}

fn row(class: &str, kind: &'static str, items: usize, owners: &[Option<&str>]) -> (String, &'static str, usize, Vec<Option<String>>) {
    (class.to_owned(), kind, items, owners.iter().map(|o| o.map(str::to_owned)).collect())
}

/// The stored `/ClassMap` dictionary, resolved.
fn stored_class_map(doc: &Document) -> Option<pdfcraft_cos::Dict> {
    let c = doc.get(doc.root().unwrap()).as_dict().cloned().unwrap();
    let root = c.get(b"StructTreeRoot").map(|o| doc.resolve(o))?;
    root.as_dict()?.get(b"ClassMap").map(|o| doc.resolve(o)).and_then(|o| o.as_dict().cloned())
}

#[test]
fn a_class_map_is_listed_with_what_each_entry_maps_to() {
    use crate::class_map::read_class_map;
    let none = read_class_map(&open(fixture()));
    assert!(!none.has_structure_tree && none.entries.is_empty() && none.count == 0 && !none.truncated && none.unreadable.is_empty());
    let (doc, _) = class_tree(None);
    let none = read_class_map(&doc);
    assert!(none.has_structure_tree && none.entries.is_empty() && none.count == 0 && none.unreadable.is_empty());

    let mut doc = open(fixture());
    let layout = doc.add(attrs("Layout"));
    let via = doc.add(Object::Ref(layout));
    let mut s = pdfcraft_cos::Stream::from_raw(pdfcraft_cos::Dict::new(), b"x".to_vec());
    s.dict.set(b"O".to_vec(), Object::name("Table"));
    let stream = doc.add(Object::Stream(s));
    let owner_name = doc.add(Object::name("List"));
    let list = doc.add(uri_with(vec![("O", Object::Ref(owner_name))]));
    let map = uri_with(vec![
        ("Direct", attrs("Layout")),
        ("Indirect", Object::Ref(via)),
        ("Stream", Object::Ref(stream)),
        (
            "Several",
            Object::Array(vec![
                attrs("Layout"),
                Object::Ref(list),
                uri_with(vec![("Placement", Object::name("Inline"))]),
                uri_with(vec![("O", Object::Int(5))]),
                Object::Int(3),
                Object::Ref(pdfcraft_cos::ObjRef::new(900, 0)),
            ]),
        ),
        ("Empty", Object::Array(vec![])),
        ("Number", Object::Int(5)),
        ("Nothing", Object::Null),
        ("Dangling", Object::Ref(pdfcraft_cos::ObjRef::new(901, 0))),
    ]);
    set_catalog_entry(&mut doc, "StructTreeRoot", uri_with(vec![("ClassMap", map)]));
    let doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
    let r = read_class_map(&doc);
    assert!(r.has_structure_tree && r.unreadable.is_empty() && !r.truncated && r.count == 8);
    assert!(r.entries.iter().all(|e| e.exact));
    assert_eq!(
        classes(&doc),
        [
            row("Direct", "dictionary", 1, &[Some("Layout")]),
            row("Indirect", "dictionary", 1, &[Some("Layout")]),
            row("Stream", "stream", 1, &[Some("Table")]),
            row("Several", "array", 6, &[Some("Layout"), Some("List"), None, None, None, None]),
            row("Empty", "array", 0, &[]),
            row("Number", "other", 0, &[]),
            row("Nothing", "other", 0, &[]),
            row("Dangling", "other", 0, &[]),
        ]
    );
    // Reading changes nothing.
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    let _ = read_class_map(&doc);
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
}

#[test]
fn removing_a_class_map_entry_keeps_the_rest_and_the_last_takes_the_dictionary() {
    use crate::class_map::{class_map_removes, read_class_map, remove_class_map_entry};
    let mut doc = open(fixture());
    let layout = doc.add(attrs("Layout"));
    let map = uri_with(vec![("A", Object::Ref(layout)), ("B", attrs("Table")), ("C", Object::Array(vec![attrs("List")]))]);
    set_catalog_entry(&mut doc, "StructTreeRoot", uri_with(vec![("Type", Object::name("StructTreeRoot")), ("ClassMap", map)]));
    let mut doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
    assert_eq!(read_class_map(&doc).count, 3);
    assert!(class_map_removes(&doc, "B") && !class_map_removes(&doc, "b") && !class_map_removes(&doc, "Z"));
    remove_class_map_entry(&mut doc, "B").unwrap();
    assert_eq!(classes(&doc), [row("A", "dictionary", 1, &[Some("Layout")]), row("C", "array", 1, &[Some("List")])]);
    assert!(!class_map_removes(&doc, "B"), "gone");
    // Saved and reopened it is still gone, and an entry that is not there is no change.
    let mut doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
    assert_eq!(read_class_map(&doc).count, 2);
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    remove_class_map_entry(&mut doc, "B").unwrap();
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
    remove_class_map_entry(&mut doc, "A").unwrap();
    remove_class_map_entry(&mut doc, "C").unwrap();
    // The last takes the dictionary with it and keeps the rest of the tree root.
    assert!(stored_class_map(&doc).is_none());
    let tree = doc.resolve(doc.get(doc.root().unwrap()).as_dict().unwrap().get(b"StructTreeRoot").unwrap());
    assert!(tree.as_dict().unwrap().get(b"ClassMap").is_none());
    assert_eq!(tree.as_dict().unwrap().name(b"Type"), Some(&b"StructTreeRoot"[..]), "the rest of the tree root stays");
    // The role map is not the class map: removing a class leaves a role of the same name alone.
    let mut doc = open(fixture());
    set_catalog_entry(
        &mut doc,
        "StructTreeRoot",
        uri_with(vec![("RoleMap", names_dict(&[("A", "P")])), ("ClassMap", uri_with(vec![("A", attrs("Layout"))]))]),
    );
    remove_class_map_entry(&mut doc, "A").unwrap();
    assert!(stored_class_map(&doc).is_none());
    assert_eq!(roles(&doc), [pair("A", "P")], "the role map keeps its entry of the same name");
}

#[test]
fn a_class_map_is_changed_where_it_is_through_references_and_in_a_direct_tree_root() {
    use crate::class_map::{read_class_map, remove_class_map_entry};
    // /StructTreeRoot behind a chain of two references, /ClassMap behind another, an entry's value
    // behind one more.
    let mut doc = open(fixture());
    let attr = doc.add(attrs("Layout"));
    let map = doc.add(uri_with(vec![("Gone", Object::Ref(attr)), ("Keep", attrs("Table"))]));
    let map_hop = doc.add(Object::Ref(map));
    let root = doc.add(uri_with(vec![("Type", Object::name("StructTreeRoot")), ("ClassMap", Object::Ref(map_hop))]));
    let root_hop = doc.add(Object::Ref(root));
    set_catalog_entry(&mut doc, "StructTreeRoot", Object::Ref(root_hop));
    assert_eq!(classes(&doc), [row("Gone", "dictionary", 1, &[Some("Layout")]), row("Keep", "dictionary", 1, &[Some("Table")])]);
    remove_class_map_entry(&mut doc, "Gone").unwrap();
    let c = doc.get(doc.root().unwrap()).as_dict().cloned().unwrap();
    assert_eq!(c.get(b"StructTreeRoot"), Some(&Object::Ref(root_hop)), "the catalog still holds the reference");
    assert_eq!(doc.get(root).as_dict().unwrap().get(b"ClassMap"), Some(&Object::Ref(map_hop)), "so does the tree root");
    assert!(doc.get(map).as_dict().unwrap().get(b"Gone").is_none() && doc.get(attr).as_dict().is_some(), "the attribute object stays");
    assert_eq!(classes(&doc), [row("Keep", "dictionary", 1, &[Some("Table")])]);
    // Emptying the indirect dictionary takes it out of the tree root, which stays.
    remove_class_map_entry(&mut doc, "Keep").unwrap();
    assert!(doc.get(root).as_dict().unwrap().get(b"ClassMap").is_none());
    assert_eq!(doc.get(root).as_dict().unwrap().name(b"Type"), Some(&b"StructTreeRoot"[..]));
    // A tree root written in the catalog itself, with a direct and then an indirect map.
    let mut doc = open(fixture());
    set_catalog_entry(&mut doc, "StructTreeRoot", uri_with(vec![("ClassMap", uri_with(vec![("A", attrs("Layout")), ("B", attrs("List"))]))]));
    remove_class_map_entry(&mut doc, "A").unwrap();
    assert_eq!(classes(&doc), [row("B", "dictionary", 1, &[Some("List")])]);
    remove_class_map_entry(&mut doc, "B").unwrap();
    assert!(read_class_map(&doc).has_structure_tree && read_class_map(&doc).count == 0);
    let map = doc.add(uri_with(vec![("A", attrs("Layout"))]));
    set_catalog_entry(&mut doc, "StructTreeRoot", uri_with(vec![("ClassMap", Object::Ref(map))]));
    remove_class_map_entry(&mut doc, "A").unwrap();
    let c = doc.get(doc.root().unwrap()).as_dict().cloned().unwrap();
    assert!(matches!(c.get(b"StructTreeRoot"), Some(Object::Dict(d)) if d.is_empty()), "the direct tree root lost its emptied /ClassMap");
    // A tree root that is a stream counts by its dictionary.
    let mut doc = open(fixture());
    let mut s = pdfcraft_cos::Stream::from_raw(pdfcraft_cos::Dict::new(), b"x".to_vec());
    s.dict.set(b"ClassMap".to_vec(), uri_with(vec![("A", attrs("Layout")), ("B", attrs("List"))]));
    let stream = doc.add(Object::Stream(s));
    set_catalog_entry(&mut doc, "StructTreeRoot", Object::Ref(stream));
    assert_eq!(read_class_map(&doc).count, 2);
    remove_class_map_entry(&mut doc, "A").unwrap();
    assert!(matches!(&*doc.get(stream), Object::Stream(s) if matches!(s.dict.get(b"ClassMap"), Some(Object::Dict(d)) if d.len() == 1)));
}

#[test]
fn a_class_map_that_is_a_stream_the_catalog_or_the_tree_root_is_unreadable_and_never_written() {
    use crate::class_map::{class_map_removes, read_class_map, remove_class_map_entry};
    // /ClassMap that is a stream: its keys (/Length, /A) are not classes, so none is listed.
    let mut doc = open(fixture());
    let mut s = pdfcraft_cos::Stream::from_raw(pdfcraft_cos::Dict::new(), b"x".to_vec());
    s.dict.set(b"Length".to_vec(), Object::Int(1));
    s.dict.set(b"A".to_vec(), attrs("Layout"));
    let stream = doc.add(Object::Stream(s));
    let root = doc.add(uri_with(vec![("ClassMap", Object::Ref(stream))]));
    set_catalog_entry(&mut doc, "StructTreeRoot", Object::Ref(root));
    let r = read_class_map(&doc);
    assert_eq!((r.has_structure_tree, r.entries.len(), r.count, r.unreadable.clone()), (true, 0, 0, vec!["class_map"]));
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    assert!(!class_map_removes(&doc, "A") && !class_map_removes(&doc, "Length"));
    remove_class_map_entry(&mut doc, "A").unwrap();
    remove_class_map_entry(&mut doc, "Length").unwrap();
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
    // /ClassMap that is the tree root, or the catalog: removing from it would change /K or /Pages.
    for target in ["root", "catalog"] {
        let mut doc = open(fixture());
        let catalog = doc.root().unwrap();
        let root = doc.add(uri_with(vec![("Type", Object::name("StructTreeRoot")), ("K", attrs("Layout"))]));
        set_catalog_entry(&mut doc, "StructTreeRoot", Object::Ref(root));
        let at = if target == "root" { root } else { catalog };
        doc.update_dict(root, |d| d.set(b"ClassMap".to_vec(), Object::Ref(at))).unwrap();
        let r = read_class_map(&doc);
        assert_eq!((r.entries.len(), r.unreadable.clone()), (0, vec!["class_map"]), "{target}");
        let before = write_full(&doc, &SaveOptions::default()).unwrap();
        for key in ["K", "Pages", "Type", "ClassMap"] {
            assert!(!class_map_removes(&doc, key), "{target} {key}");
            remove_class_map_entry(&mut doc, key).unwrap();
        }
        assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before, "{target}");
        assert!(doc.get(catalog).as_dict().unwrap().get(b"Pages").is_some());
    }
    // A tree root that is the catalog.
    let mut doc = open(fixture());
    let catalog = doc.root().unwrap();
    set_catalog_entry(&mut doc, "StructTreeRoot", Object::Ref(catalog));
    let r = read_class_map(&doc);
    assert_eq!((r.has_structure_tree, r.unreadable.clone()), (false, vec!["struct_tree_root"]));
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    assert!(!class_map_removes(&doc, "Pages"));
    remove_class_map_entry(&mut doc, "Pages").unwrap();
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
}

#[test]
fn a_tree_root_or_class_map_that_is_not_a_dictionary_is_named_and_never_replaced() {
    use crate::class_map::{class_map_removes, read_class_map, remove_class_map_entry};
    for other in [Object::Int(7), Object::name("x"), Object::Array(vec![attrs("Layout")]), string(b"s")] {
        let mut doc = open(fixture());
        set_catalog_entry(&mut doc, "StructTreeRoot", other.clone());
        let r = read_class_map(&doc);
        assert_eq!((r.has_structure_tree, r.unreadable.clone()), (false, vec!["struct_tree_root"]), "{other:?}");
        let before = write_full(&doc, &SaveOptions::default()).unwrap();
        assert!(!class_map_removes(&doc, "A"));
        remove_class_map_entry(&mut doc, "A").unwrap();
        assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before, "{other:?}");

        let (mut doc, _) = class_tree(Some(other.clone()));
        let r = read_class_map(&doc);
        assert_eq!((r.has_structure_tree, r.unreadable.clone(), r.entries.len()), (true, vec!["class_map"], 0), "{other:?}");
        let before = write_full(&doc, &SaveOptions::default()).unwrap();
        remove_class_map_entry(&mut doc, "A").unwrap();
        assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before, "{other:?}");
    }
    // A null or a dangling /ClassMap, and a null or a dangling tree root, are absent: nothing to
    // remove, nothing unreadable, nothing written.
    for absent in [Object::Null, Object::Ref(pdfcraft_cos::ObjRef::new(900, 0))] {
        let (mut doc, _) = class_tree(Some(absent.clone()));
        let r = read_class_map(&doc);
        assert_eq!((r.has_structure_tree, r.count, r.unreadable.clone()), (true, 0, vec![]), "{absent:?}");
        let before = write_full(&doc, &SaveOptions::default()).unwrap();
        remove_class_map_entry(&mut doc, "A").unwrap();
        assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before, "{absent:?}");
        let mut doc = open(fixture());
        set_catalog_entry(&mut doc, "StructTreeRoot", absent.clone());
        let r = read_class_map(&doc);
        assert_eq!((r.has_structure_tree, r.unreadable.clone()), (false, vec![]), "{absent:?}");
        let before = write_full(&doc, &SaveOptions::default()).unwrap();
        remove_class_map_entry(&mut doc, "A").unwrap();
        assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before, "{absent:?}");
    }
}

#[test]
fn a_stored_class_goes_by_its_exact_name_whatever_it_is_and_one_that_is_not_stored_is_no_change() {
    use crate::class_map::{class_map_removes, read_class_map, remove_class_map_entry};
    let long = "k".repeat(200);
    let mut map = pdfcraft_cos::Dict::new();
    map.set(long.as_bytes().to_vec(), attrs("Layout"));
    map.set(Vec::<u8>::new(), attrs("Layout"));
    map.set(vec![b'N', 0], attrs("Layout"));
    map.set(vec![b'U', 0xff], attrs("Layout"));
    map.set(b"Fine".to_vec(), attrs("Layout"));
    let (mut doc, _) = class_tree(Some(Object::Dict(map)));
    let r = read_class_map(&doc);
    assert_eq!(r.count, 5);
    let listed: Vec<(String, bool)> = r.entries.iter().map(|e| (e.class.chars().take(3).collect(), e.exact)).collect();
    assert_eq!(listed, [("kkk".into(), false), ("".into(), true), ("N\0".into(), true), ("U\u{fffd}".into(), false), ("Fin".into(), true)]);
    assert_eq!(r.entries[0].class.len(), 127, "a long class is cut before it is copied");
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    let before_incremental = write_incremental(&doc, &SaveOptions::default()).unwrap();
    // The cut name and the lossy name are not stored names.
    for unstored in [&long[..127], "U\u{fffd}", "fine", "Fine ", "Nothing"] {
        assert!(!class_map_removes(&doc, unstored), "{unstored}");
        remove_class_map_entry(&mut doc, unstored).unwrap();
        assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before, "{unstored}");
        assert_eq!(write_incremental(&doc, &SaveOptions::default()).unwrap(), before_incremental, "{unstored}: no object was touched");
    }
    for stored in [long.as_str(), "", "N\0", "Fine"] {
        assert!(class_map_removes(&doc, stored));
        remove_class_map_entry(&mut doc, stored).unwrap();
        assert!(!class_map_removes(&doc, stored));
    }
    let r = read_class_map(&doc);
    assert_eq!(r.count, 1, "only the key that is not UTF-8 is left, and it cannot be asked for");
    assert!(!r.entries[0].exact);
}

#[test]
fn the_references_followed_are_those_of_document_resolve_for_the_class_map() {
    use crate::class_map::{read_class_map, remove_class_map_entry};
    // A chain of n references to an object: n = 33 resolves, n = 34 does not.
    let chain = |doc: &mut Document, end: Object, hops: usize| {
        let mut at = doc.add(end);
        for _ in 1..hops {
            at = doc.add(Object::Ref(at));
        }
        Object::Ref(at)
    };
    for (hops, readable) in [(33usize, true), (34, false)] {
        // /ClassMap at the end of the chain.
        let (mut doc, root) = class_tree(None);
        let start = chain(&mut doc, uri_with(vec![("A", attrs("Layout")), ("B", attrs("List"))]), hops);
        doc.update_dict(root, |d| d.set(b"ClassMap".to_vec(), start.clone())).unwrap();
        assert_eq!(doc.resolve(&start).as_dict().is_some(), readable, "Document::resolve at {hops} references");
        let r = read_class_map(&doc);
        assert_eq!((r.entries.len(), r.unreadable.clone()), if readable { (2, vec![]) } else { (0, vec!["class_map"]) }, "{hops} references");
        let before = write_full(&doc, &SaveOptions::default()).unwrap();
        remove_class_map_entry(&mut doc, "A").unwrap();
        if readable {
            assert_eq!(classes(&doc), [row("B", "dictionary", 1, &[Some("List")])]);
        } else {
            assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
        }
        // /StructTreeRoot at the end of one.
        let mut doc = open(fixture());
        let start = chain(&mut doc, uri_with(vec![("ClassMap", uri_with(vec![("A", attrs("Layout"))]))]), hops);
        set_catalog_entry(&mut doc, "StructTreeRoot", start.clone());
        assert_eq!(doc.resolve(&start).as_dict().is_some(), readable);
        let r = read_class_map(&doc);
        assert_eq!(
            (r.has_structure_tree, r.unreadable.clone()),
            if readable { (true, vec![]) } else { (false, vec!["struct_tree_root"]) },
            "{hops} references"
        );
        // An entry's value, an array item and an owner at the end of one.
        let (mut doc, _) = class_tree(None);
        let value = chain(&mut doc, attrs("Layout"), hops);
        let item = chain(&mut doc, attrs("Table"), hops);
        let name = chain(&mut doc, Object::name("List"), hops);
        let named = doc.add(uri_with(vec![("O", name)]));
        let map = doc.add(uri_with(vec![("V", value), ("I", Object::Array(vec![item])), ("O", Object::Ref(named))]));
        set_catalog_entry(&mut doc, "StructTreeRoot", uri_with(vec![("ClassMap", Object::Ref(map))]));
        let some = |n: &'static str| readable.then_some(n);
        assert_eq!(
            classes(&doc),
            [
                row("V", if readable { "dictionary" } else { "other" }, usize::from(readable), if readable { &[Some("Layout")] } else { &[] }),
                row("I", "array", 1, &[some("Table")]),
                row("O", "dictionary", 1, &[some("List")]),
            ],
            "{hops} references"
        );
    }
}

#[test]
fn the_class_listing_is_bounded_names_are_cut_and_an_array_lists_only_its_first_owners() {
    use crate::class_map::{MAX_LISTED_CLASSES, MAX_LISTED_OWNERS, read_class_map, remove_class_map_entry};
    let mut map = pdfcraft_cos::Dict::new();
    for i in 0..300 {
        map.set(format!("C{i:03}").into_bytes(), attrs("Layout"));
    }
    let (mut doc, _) = class_tree(Some(Object::Dict(map)));
    let r = read_class_map(&doc);
    assert_eq!((r.count, r.entries.len(), r.truncated), (300, MAX_LISTED_CLASSES, true));
    // An entry past the listed ones is still counted, and can be removed by its name.
    remove_class_map_entry(&mut doc, "C299").unwrap();
    let r = read_class_map(&doc);
    assert_eq!((r.count, r.entries.len(), r.truncated), (299, MAX_LISTED_CLASSES, true));
    // Exactly 256 entries are not truncated.
    let mut map = pdfcraft_cos::Dict::new();
    for i in 0..MAX_LISTED_CLASSES {
        map.set(format!("C{i:03}").into_bytes(), Object::Null);
    }
    let (doc, _) = class_tree(Some(Object::Dict(map)));
    let r = read_class_map(&doc);
    assert_eq!((r.count, r.entries.len(), r.truncated), (256, 256, false));
    // An array of 1000 items lists 16 owners and counts all of them; the owners are cut.
    let items: Vec<Object> = (0..1000).map(|i| attrs(&format!("O{i}"))).collect();
    let long_owner = "w".repeat(200);
    let (doc, _) = class_tree(Some(uri_with(vec![
        ("Many", Object::Array(items)),
        ("Long", attrs(&long_owner)),
        ("Exactly", Object::Array((0..MAX_LISTED_OWNERS).map(|i| attrs(&format!("O{i}"))).collect())),
    ])));
    let r = read_class_map(&doc);
    assert_eq!((r.entries[0].items, r.entries[0].owners.len(), r.entries[0].exact), (1000, MAX_LISTED_OWNERS, true));
    assert_eq!(r.entries[0].owners[15].as_deref(), Some("O15"));
    assert_eq!(r.entries[1].owners[0].as_deref().map(str::len), Some(127));
    assert!(!r.entries[1].exact, "a cut owner is not exact");
    assert_eq!((r.entries[2].items, r.entries[2].owners.len()), (MAX_LISTED_OWNERS, MAX_LISTED_OWNERS));
    // A name that needs escaping in a file survives a save.
    let (doc, _) = class_tree(Some(uri_with(vec![("A b/(c)#", attrs("Layout"))])));
    let doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
    assert_eq!(classes(&doc), [row("A b/(c)#", "dictionary", 1, &[Some("Layout")])]);
}

/// The fixture with a tree root holding the given `(key, object)` entries; returns the document.
fn tree_with(entries: &[(&str, pdfcraft_cos::ObjRef)]) -> Document {
    let mut doc = open(fixture());
    let mut all = vec![("Type", Object::name("StructTreeRoot"))];
    all.extend(entries.iter().map(|(k, r)| (*k, Object::Ref(*r))));
    let root = doc.add(uri_with(all));
    set_catalog_entry(&mut doc, "StructTreeRoot", Object::Ref(root));
    doc
}

const MAP_REFUSED: &str = "the structure tree's /StructTreeRoot or /RoleMap cannot be changed (it is not a dictionary, is behind more than 33 references, is a stream role map, or is the same object as the catalog, the /Pages node, a page or inner /Pages node, or (for a map) the tree root or its other map, or the page tree has over 1000000 nodes or 2000000 /Kids entries and could not be checked), so it was left as it is";

/// A role map or class map that is one object with the `/Pages` node, an inner `/Pages` node or a
/// page would be changed as a map, and `Kids` or `Parent` in it would break the page tree: it is
/// unreadable and nothing is written. The fixture's objects: 2 the `/Pages` node, 3 an inner
/// `/Pages` node, 4 and 6 pages.
#[test]
fn a_map_that_is_the_pages_node_an_inner_node_or_a_page_is_unreadable_and_never_written() {
    use crate::class_map::{class_map_removes, read_class_map, remove_class_map_entry};
    use crate::role_map::{read_role_map, role_map_changes, set_role_map};
    for (what, at) in [("the /Pages node", 2), ("an inner /Pages node", 3), ("a page", 4), ("a page under the root", 6)] {
        let at = pdfcraft_cos::ObjRef::new(at, 0);
        // The object is the tree root's /RoleMap alone, and then its /ClassMap alone, so that the
        // other map is not what makes it unreadable.
        for map_key in ["RoleMap", "ClassMap"] {
            let mut doc = tree_with(&[(map_key, at)]);
            let before = write_full(&doc, &SaveOptions::default()).unwrap();
            let held = doc.get(at).clone();
            let (r, c) = (read_role_map(&doc), read_class_map(&doc));
            let (own, other) =
                if map_key == "RoleMap" { (r.unreadable.clone(), c.unreadable.clone()) } else { (c.unreadable.clone(), r.unreadable.clone()) };
            assert_eq!((own, other), (vec![if map_key == "RoleMap" { "role_map" } else { "class_map" }], vec![]), "{what} {map_key}");
            assert_eq!((r.has_structure_tree, r.count, r.entries.len(), c.count, c.entries.len()), (true, 0, 0, 0, 0), "{what} {map_key}");
            for key in ["Kids", "Parent", "Type"] {
                if map_key == "RoleMap" {
                    assert_eq!(set_role_map(&mut doc, key, Some("H1")).unwrap_err().to_string(), MAP_REFUSED, "{what} {key}");
                    assert!(!role_map_changes(&doc, key, None).unwrap(), "{what} {key}");
                    set_role_map(&mut doc, key, None).unwrap();
                } else {
                    assert!(!class_map_removes(&doc, key), "{what} {key}");
                    remove_class_map_entry(&mut doc, key).unwrap();
                }
            }
            assert_eq!(*doc.get(at), *held, "{what} {map_key}: the object is as it was");
            assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before, "{what} {map_key}");
        }
    }
    // The tree root that is one of them: unreadable as the tree root, and the node is not given a /RoleMap.
    for at in [2, 3, 4] {
        let at = pdfcraft_cos::ObjRef::new(at, 0);
        let mut doc = open(fixture());
        set_catalog_entry(&mut doc, "StructTreeRoot", Object::Ref(at));
        let before = write_full(&doc, &SaveOptions::default()).unwrap();
        let r = read_role_map(&doc);
        assert_eq!((r.has_structure_tree, r.unreadable.clone()), (false, vec!["struct_tree_root"]), "tree root {at:?}");
        let c = read_class_map(&doc);
        assert_eq!((c.has_structure_tree, c.unreadable.clone()), (false, vec!["struct_tree_root"]), "tree root {at:?}");
        assert_eq!(set_role_map(&mut doc, "A", Some("H1")).unwrap_err().to_string(), MAP_REFUSED, "tree root {at:?}");
        remove_class_map_entry(&mut doc, "A").unwrap();
        assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before, "tree root {at:?}");
        assert!(!doc.get(at).as_dict().unwrap().contains(b"RoleMap"));
    }
}

/// The tree root's `/RoleMap` and `/ClassMap` that are one object are each unreadable and
/// unchanged; kept apart they are read and changed as before.
#[test]
fn a_role_map_and_a_class_map_that_are_one_object_are_unreadable_and_apart_they_are_not() {
    use crate::class_map::{read_class_map, remove_class_map_entry};
    use crate::role_map::{read_role_map, set_role_map};
    let mut shared = pdfcraft_cos::Dict::new();
    shared.set(b"Chapter".to_vec(), Object::name("H1"));
    let mut doc = open(fixture());
    let both = doc.add(Object::Dict(shared.clone()));
    let root = doc.add(uri_with(vec![("RoleMap", Object::Ref(both)), ("ClassMap", Object::Ref(both))]));
    set_catalog_entry(&mut doc, "StructTreeRoot", Object::Ref(root));
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    let (r, c) = (read_role_map(&doc), read_class_map(&doc));
    assert_eq!((r.entries.len(), r.unreadable.clone()), (0, vec!["role_map"]));
    assert_eq!((c.entries.len(), c.unreadable.clone()), (0, vec!["class_map"]));
    set_role_map(&mut doc, "Chapter", None).unwrap();
    assert_eq!(set_role_map(&mut doc, "Other", Some("P")).unwrap_err().to_string(), MAP_REFUSED);
    remove_class_map_entry(&mut doc, "Chapter").unwrap();
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
    assert_eq!(*doc.get(both), Object::Dict(shared.clone()));
    // Two objects: each reads and changes as a map.
    let mut doc = open(fixture());
    let (a, b) = (doc.add(Object::Dict(shared.clone())), doc.add(Object::Dict(shared)));
    let root = doc.add(uri_with(vec![("RoleMap", Object::Ref(a)), ("ClassMap", Object::Ref(b))]));
    set_catalog_entry(&mut doc, "StructTreeRoot", Object::Ref(root));
    assert_eq!((read_role_map(&doc).entries.len(), read_class_map(&doc).entries.len()), (1, 1));
    set_role_map(&mut doc, "Chapter", Some("H2")).unwrap();
    remove_class_map_entry(&mut doc, "Chapter").unwrap();
    assert_eq!(roles(&doc), [pair("Chapter", "H2")]);
    assert_eq!(read_class_map(&doc).entries.len(), 0);
}

/// The walk behind the guard visits each node once, ends on a cycle, and gives up (answering
/// "could not show it is not one") at its cap.
#[test]
fn the_page_tree_walk_ends_on_a_cycle_and_at_its_cap() {
    use crate::role_map::reaches_page_tree_node;
    let r = |n| pdfcraft_cos::ObjRef::new(n, 0);
    let mut doc = open(fixture());
    // The fixture: 2 -> 3 -> (4, 5), 2 -> 6. Not in the tree: 7 (a stream), 10, 11.
    for n in [2, 3, 4, 5, 6] {
        assert!(reaches_page_tree_node(&doc, r(n), 1000, 1000), "{n}");
    }
    for n in [1, 7, 10, 11, 999] {
        assert!(!reaches_page_tree_node(&doc, r(n), 1000, 1000), "{n}");
    }
    // A cycle (the inner node lists the root) and a list that repeats a kid end.
    doc.update_dict(r(3), |d| {
        d.set(b"Kids".to_vec(), Object::Array(vec![Object::Ref(r(2)), Object::Ref(r(4)), Object::Ref(r(4)), Object::Ref(r(3))]))
    })
    .unwrap();
    assert!(reaches_page_tree_node(&doc, r(4), 1000, 1000));
    assert!(!reaches_page_tree_node(&doc, r(11), 1000, 1000));
    // /Kids behind a reference is followed.
    let kids = doc.add(Object::Array(vec![Object::Ref(r(5))]));
    doc.update_dict(r(3), |d| d.set(b"Kids".to_vec(), Object::Ref(kids))).unwrap();
    assert!(reaches_page_tree_node(&doc, r(5), 1000, 1000));
    assert!(!reaches_page_tree_node(&doc, r(4), 1000, 1000), "no longer a kid of the inner node");
    // The cap: with room for two nodes, the walk visits 2 and 6 or 3 and gives up before the rest.
    let doc = open(fixture());
    assert!(reaches_page_tree_node(&doc, r(11), 2, 1000), "gave up, so it could not show 11 is not a node");
    assert!(!reaches_page_tree_node(&doc, r(11), 5, 1000), "all five nodes visited");
    assert!(reaches_page_tree_node(&doc, r(11), 4, 1000));
    // No /Pages node.
    let mut doc = open(fixture());
    doc.update_dict(doc.root().unwrap(), |c| {
        c.remove(b"Pages");
    })
    .unwrap();
    assert!(!reaches_page_tree_node(&doc, r(4), 1000, 1000));
}

/// A hostile page tree: one `/Kids` array shared by the `/Pages` node and by each of its `k`
/// kids. A walk that pushes every entry it scans would hold about k * k / 2 entries on its stack
/// and scan k * k; this one puts each node on the stack once and counts every entry it scans.
#[test]
fn a_shared_kids_array_cannot_make_the_page_tree_walk_hold_or_scan_a_square() {
    use crate::role_map::{reaches_page_tree_node, walk_page_tree};
    let k: u32 = 1000;
    let mut doc = open(fixture());
    let nodes: Vec<_> = (0..k).map(|_| doc.add(uri_with(vec![]))).collect();
    let top = doc.add(uri_with(vec![]));
    let shared = doc.add(Object::Array(nodes.iter().map(|n| Object::Ref(*n)).collect()));
    for n in nodes.iter().chain([&top]) {
        doc.update_dict(*n, |d| d.set(b"Kids".to_vec(), Object::Ref(shared))).unwrap();
    }
    set_catalog_entry(&mut doc, "Pages", Object::Ref(top));
    let outside = pdfcraft_cos::ObjRef::new(11, 0);
    // With room for everything (k * (k + 1) entries are scanned), the answer is by what was found,
    // and the stack never held more than the k kids.
    let (answer, peak) = walk_page_tree(&doc, outside, 10_000, 10_000_000);
    assert!(!answer);
    assert!(peak <= k as usize, "the stack held {peak} entries");
    assert!(walk_page_tree(&doc, nodes[999], 10_000, 10_000_000).0);
    // A scan cap below k * (k + 1) gives up, which refuses.
    assert!(reaches_page_tree_node(&doc, outside, 10_000, 500_000));
    assert!(reaches_page_tree_node(&doc, outside, 10_000, k as usize * (k as usize + 1) - 1));
    assert!(!reaches_page_tree_node(&doc, outside, 10_000, k as usize * (k as usize + 1)));
    // A node cap below the k + 1 nodes gives up too.
    assert!(reaches_page_tree_node(&doc, outside, k as usize, 10_000_000));
    assert!(!reaches_page_tree_node(&doc, outside, k as usize + 1, 10_000_000));
    // Entries that are not references are scanned and counted too.
    let mut doc = open(fixture());
    doc.update_dict(pdfcraft_cos::ObjRef::new(2, 0), |d| d.set(b"Kids".to_vec(), Object::Array(vec![Object::Int(1); 10]))).unwrap();
    assert!(reaches_page_tree_node(&doc, outside, 1000, 9));
    assert!(!reaches_page_tree_node(&doc, outside, 1000, 10));
}

/// A thread of `beads` beads linked through `/N`: the last one leads back to the first when
/// `closed`, and has no `/N` otherwise. Returns the thread object and the beads.
fn article_thread(doc: &mut Document, title: Option<Object>, beads: usize, closed: bool) -> (pdfcraft_cos::ObjRef, Vec<pdfcraft_cos::ObjRef>) {
    let refs: Vec<_> = (0..beads).map(|_| doc.add(uri_with(vec![]))).collect();
    for (i, r) in refs.iter().enumerate() {
        let next = refs.get(i + 1).or(if closed { refs.first() } else { None }).copied();
        doc.update_dict(*r, |d| {
            if let Some(n) = next {
                d.set(b"N".to_vec(), Object::Ref(n));
            }
        })
        .unwrap();
    }
    let mut entries = vec![];
    if let Some(first) = refs.first() {
        entries.push(("F", Object::Ref(*first)));
    }
    if let Some(t) = title {
        entries.push(("I", uri_with(vec![("Title", t)])));
    }
    (doc.add(uri_with(entries)), refs)
}

fn threads_of(doc: &Document) -> Vec<(usize, Option<String>, usize, bool)> {
    crate::articles::read_article_threads(doc).threads.iter().map(|t| (t.index, t.title.clone(), t.beads, t.closed)).collect()
}

fn thread_entry(doc: &Document) -> Option<Object> {
    doc.get(doc.root().unwrap()).as_dict().and_then(|c| c.get(b"Threads").cloned())
}

#[test]
fn article_threads_are_listed_with_their_title_and_how_many_beads_they_reach() {
    use crate::articles::read_article_threads;
    let none = read_article_threads(&open(fixture()));
    assert!(none.threads.is_empty() && none.count == 0 && !none.truncated && none.unreadable.is_empty());

    let mut doc = open(fixture());
    let (three, _) = article_thread(&mut doc, Some(string(b"Front page")), 3, true);
    let (open_ring, _) = article_thread(&mut doc, None, 2, false);
    let (utf16, _) = article_thread(&mut doc, Some(string(&[0xFE, 0xFF, 0x00, 0xE9, 0x00, 0x41])), 1, true);
    let (no_beads, _) = article_thread(&mut doc, Some(Object::Int(7)), 0, false);
    let number = doc.add(Object::Int(5));
    let loose = doc.add(uri_with(vec![("F", Object::Ref(number))]));
    // A bead given as a direct dictionary is not a bead.
    let direct = doc.add(uri_with(vec![("F", uri_with(vec![]))]));
    set_catalog_entry(
        &mut doc,
        "Threads",
        Object::Array(vec![
            Object::Ref(three),
            Object::Ref(open_ring),
            Object::Ref(utf16),
            Object::Ref(no_beads),
            Object::Int(3),
            Object::Ref(loose),
            Object::Ref(direct),
            Object::Ref(pdfcraft_cos::ObjRef::new(900, 0)),
        ]),
    );
    let doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
    let r = read_article_threads(&doc);
    assert!(r.count == 8 && !r.truncated && r.unreadable.is_empty());
    assert!(r.threads.iter().all(|t| t.title_exact && !t.beads_capped));
    assert_eq!(
        threads_of(&doc),
        [
            (0, Some("Front page".to_string()), 3, true),
            (1, None, 2, false),
            (2, Some("\u{e9}A".to_string()), 1, true),
            (3, None, 0, false),
            (4, None, 0, false),
            (5, None, 0, false),
            (6, None, 0, false),
            (7, None, 0, false),
        ]
    );
    // Reading changes nothing.
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    let _ = read_article_threads(&doc);
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
}

#[test]
fn removing_an_article_thread_keeps_the_others_in_order_and_leaves_the_beads() {
    use crate::articles::{article_thread_removes, remove_article_thread};
    let mut doc = open(fixture());
    let (a, a_beads) = article_thread(&mut doc, Some(string(b"A")), 2, true);
    let (b, _) = article_thread(&mut doc, Some(string(b"B")), 1, true);
    let (c, _) = article_thread(&mut doc, Some(string(b"C")), 1, true);
    set_catalog_entry(&mut doc, "Threads", Object::Array(vec![Object::Ref(a), Object::Ref(b), Object::Ref(c)]));
    let titles = |d: &Document| threads_of(d).into_iter().map(|t| t.1.unwrap()).collect::<Vec<_>>();

    // A position with no entry is not offered and changes no byte.
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    assert!(!article_thread_removes(&doc, 3) && !article_thread_removes(&doc, usize::MAX));
    remove_article_thread(&mut doc, 3).unwrap();
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);

    assert!(article_thread_removes(&doc, 1));
    remove_article_thread(&mut doc, 1).unwrap();
    assert_eq!(titles(&doc), ["A", "C"]);
    for saved in [write_incremental(&doc, &SaveOptions::default()).unwrap(), write_full(&doc, &SaveOptions::default()).unwrap()] {
        assert_eq!(titles(&open(saved)), ["A", "C"]);
    }
    // The thread dictionary and its beads stay where they were.
    assert!(doc.get(a).as_dict().is_some() && doc.get(b).as_dict().is_some());
    assert!(a_beads.iter().all(|r| doc.get(*r).as_dict().is_some_and(|d| d.get(b"N").is_some())));
    remove_article_thread(&mut doc, 0).unwrap();
    assert_eq!(titles(&doc), ["C"]);
    // The last one takes `/Threads` with it.
    remove_article_thread(&mut doc, 0).unwrap();
    assert!(thread_entry(&doc).is_none() && read_article_threads_count(&doc) == 0);
    assert!(thread_entry(&open(write_full(&doc, &SaveOptions::default()).unwrap())).is_none());
}

fn read_article_threads_count(doc: &Document) -> usize {
    crate::articles::read_article_threads(doc).count
}

#[test]
fn an_article_thread_array_behind_references_is_changed_where_it_is() {
    use crate::articles::remove_article_thread;
    let mut doc = open(fixture());
    let (a, _) = article_thread(&mut doc, Some(string(b"A")), 1, true);
    let (b, _) = article_thread(&mut doc, Some(string(b"B")), 1, true);
    let array = doc.add(Object::Array(vec![Object::Ref(a), Object::Ref(b)]));
    let via = doc.add(Object::Ref(array));
    set_catalog_entry(&mut doc, "Threads", Object::Ref(via));
    assert_eq!(threads_of(&doc).len(), 2);
    remove_article_thread(&mut doc, 0).unwrap();
    // The catalog and the reference between keep what they were; the array object changed.
    assert!(matches!(thread_entry(&doc), Some(Object::Ref(r)) if r == via));
    assert!(matches!(&*doc.get(via), Object::Ref(r) if *r == array));
    assert_eq!(doc.get(array).as_array().map(Vec::len), Some(1));
    assert_eq!(threads_of(&doc)[0].1.as_deref(), Some("B"));
    // The last entry takes the catalog's entry; the array object is left in the file.
    remove_article_thread(&mut doc, 0).unwrap();
    assert!(thread_entry(&doc).is_none());
    assert_eq!(doc.get(array).as_array().map(Vec::len), Some(1));
}

#[test]
fn an_article_threads_entry_that_is_not_an_array_is_named_and_never_replaced() {
    use crate::articles::{article_thread_removes, read_article_threads, remove_article_thread};
    let mut chain33 = open(fixture());
    let array = chain33.add(Object::Array(vec![Object::Int(1)]));
    let mut at = array;
    let mut chain34 = chain33.clone();
    for _ in 0..32 {
        at = chain33.add(Object::Ref(at));
    }
    set_catalog_entry(&mut chain33, "Threads", Object::Ref(at));
    // A chain of 32 reference objects is followed to the array (33 gets) and one of 33 is not, as with
    // `Document::resolve`.
    let resolves = |d: &Document| d.resolve(&thread_entry(d).unwrap()).as_array().is_some();
    assert!(resolves(&chain33));
    assert_eq!(read_article_threads(&chain33).count, 1);
    assert!(read_article_threads(&chain33).unreadable.is_empty());
    let array = chain34.add(Object::Array(vec![Object::Int(1)]));
    let mut at = array;
    for _ in 0..33 {
        at = chain34.add(Object::Ref(at));
    }
    set_catalog_entry(&mut chain34, "Threads", Object::Ref(at));
    assert_eq!(read_article_threads(&chain34).unreadable, ["threads"]);
    assert!(!resolves(&chain34));

    for (name, value) in [("number", Object::Int(5)), ("string", string(b"x")), ("dict", uri_with(vec![])), ("name", Object::name("T"))] {
        let mut doc = open(fixture());
        set_catalog_entry(&mut doc, "Threads", value);
        let doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
        let r = read_article_threads(&doc);
        assert!(r.unreadable == ["threads"] && r.threads.is_empty() && r.count == 0, "{name}");
        assert!(!article_thread_removes(&doc, 0), "{name}");
        let mut doc = doc;
        let before = write_full(&doc, &SaveOptions::default()).unwrap();
        remove_article_thread(&mut doc, 0).unwrap();
        assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before, "{name}");
    }
    for value in [Object::Null, Object::Ref(pdfcraft_cos::ObjRef::new(900, 0)), Object::Array(vec![])] {
        let mut doc = open(fixture());
        set_catalog_entry(&mut doc, "Threads", value);
        let r = read_article_threads(&doc);
        assert!(r.unreadable.is_empty() && r.count == 0);
        assert!(!article_thread_removes(&doc, 0));
    }
}

#[test]
fn the_thread_listing_and_the_bead_walk_are_bounded() {
    use crate::articles::{MAX_BEADS_WALKED, MAX_LISTED_THREADS, MAX_TITLE_CHARS, read_article_threads, remove_article_thread};
    let mut doc = open(fixture());
    let entries: Vec<Object> =
        (0..MAX_LISTED_THREADS + 44).map(|i| Object::Ref(article_thread(&mut doc, Some(string(format!("t{i}").as_bytes())), 1, true).0)).collect();
    set_catalog_entry(&mut doc, "Threads", Object::Array(entries));
    let r = read_article_threads(&doc);
    assert!(r.count == MAX_LISTED_THREADS + 44 && r.threads.len() == MAX_LISTED_THREADS && r.truncated);
    // An entry past the listed ones is still removed by its position.
    remove_article_thread(&mut doc, MAX_LISTED_THREADS + 43).unwrap();
    assert_eq!(read_article_threads(&doc).count, MAX_LISTED_THREADS + 43);
    // Exactly the listed number is not truncated.
    for _ in 0..43 {
        remove_article_thread(&mut doc, MAX_LISTED_THREADS).unwrap();
    }
    let r = read_article_threads(&doc);
    assert!(r.count == MAX_LISTED_THREADS && !r.truncated);

    // Titles are cut by characters, not bytes.
    let mut doc = open(fixture());
    let long = "\u{e9}".repeat(MAX_TITLE_CHARS + 1);
    let whole = "\u{e9}".repeat(MAX_TITLE_CHARS);
    let (cut, _) = article_thread(&mut doc, Some(string(long.as_bytes())), 1, true);
    let (fits, _) = article_thread(&mut doc, Some(string(whole.as_bytes())), 1, true);
    set_catalog_entry(&mut doc, "Threads", Object::Array(vec![Object::Ref(cut), Object::Ref(fits)]));
    let r = read_article_threads(&doc);
    assert!(!r.threads[0].title_exact && r.threads[0].title.as_deref() == Some(whole.as_str()));
    assert!(r.threads[1].title_exact && r.threads[1].title.as_deref() == Some(whole.as_str()));

    // Beads: a ring of exactly the cap is counted whole and closed, one more is cut.
    let mut doc = open(fixture());
    let (ring, _) = article_thread(&mut doc, None, MAX_BEADS_WALKED, true);
    let (chain, _) = article_thread(&mut doc, None, MAX_BEADS_WALKED, false);
    let (over, _) = article_thread(&mut doc, None, MAX_BEADS_WALKED + 1, true);
    // A loop that does not pass through the first bead ends at the cap too.
    let (spun, beads) = article_thread(&mut doc, None, 3, false);
    doc.update_dict(beads[2], |d| d.set(b"N".to_vec(), Object::Ref(beads[1]))).unwrap();
    set_catalog_entry(&mut doc, "Threads", Object::Array([ring, chain, over, spun].map(Object::Ref).to_vec()));
    let r = read_article_threads(&doc);
    let shape: Vec<_> = r.threads.iter().map(|t| (t.beads, t.beads_capped, t.closed)).collect();
    assert_eq!(
        shape,
        [(MAX_BEADS_WALKED, false, true), (MAX_BEADS_WALKED, false, false), (MAX_BEADS_WALKED, true, false), (MAX_BEADS_WALKED, true, false)]
    );
}

fn dest_array(page: Object, view: &str) -> Object {
    Object::Array(vec![page, Object::name(view)])
}

fn page_ref(doc: &Document, i: usize) -> Object {
    Object::Ref(crate::pages(doc).unwrap()[i].obj)
}

/// A listed name as (index, source, name, page, view, removable).
type DestRow = (usize, crate::destinations::Source, String, Option<usize>, Option<String>, bool);

fn dests_of(doc: &Document) -> Vec<DestRow> {
    let r = crate::destinations::read_destinations(doc);
    r.destinations.into_iter().map(|d| (d.index, d.source, d.name, d.page, d.view, d.removable)).collect()
}

fn names_of(doc: &Document) -> Vec<String> {
    dests_of(doc).into_iter().map(|d| d.2).collect()
}

/// Names in a leaf: `(name, destination)` pairs as a `/Names` array.
fn name_pairs(pairs: Vec<(&[u8], Object)>) -> Object {
    Object::Array(pairs.into_iter().flat_map(|(k, v)| [string(k), v]).collect())
}

fn set_names_root(doc: &mut Document, root: Object) {
    set_catalog_entry(doc, "Names", uri_with(vec![("Dests", root)]));
}

#[test]
fn named_destinations_are_listed_from_the_tree_and_the_dictionary_with_their_page_and_view() {
    use crate::destinations::{Source, read_destinations};
    let none = read_destinations(&open(fixture()));
    assert!(none.destinations.is_empty() && !none.truncated && !none.tree_capped && none.unreadable.is_empty());

    let mut doc = open(fixture());
    let (p0, p1, p2) = (page_ref(&doc, 0), page_ref(&doc, 1), page_ref(&doc, 2));
    let leaf = doc.add(uri_with(vec![
        (
            "Names",
            name_pairs(vec![
                (b"alpha", Object::Array(vec![p0.clone(), Object::name("XYZ"), Object::Int(0), Object::Int(100), Object::Null])),
                (b"beta", uri_with(vec![("D", dest_array(p1.clone(), "Fit"))])),
                (&[0xFE, 0xFF, 0x00, 0xE9], dest_array(p0.clone(), "Fit")),
                (b"remote", dest_array(Object::Int(3), "Fit")),
                (b"number", Object::Int(5)),
                (b"long view", Object::Array(vec![p2.clone(), Object::name("AVeryLongViewNameIndeed")])),
            ]),
        ),
        ("Limits", Object::Array(vec![string(b"alpha"), string(b"remote")])),
    ]));
    // A key that is not a string is skipped, and a leaf written directly is read.
    let direct = uri_with(vec![(
        "Names",
        Object::Array(vec![Object::Int(7), dest_array(p0.clone(), "Fit"), string(b"direct"), dest_array(p1.clone(), "Fit")]),
    )]);
    let root = doc.add(uri_with(vec![(
        "Kids",
        Object::Array(vec![Object::Ref(leaf), direct, Object::Int(4), Object::Ref(pdfcraft_cos::ObjRef::new(900, 0))]),
    )]));
    set_names_root(&mut doc, Object::Ref(root));
    let mut old = pdfcraft_cos::Dict::new();
    old.set(b"old".to_vec(), dest_array(p1.clone(), "FitH"));
    old.set(b"caf\xE9".to_vec(), dest_array(p2.clone(), "Fit"));
    set_catalog_entry(&mut doc, "Dests", Object::Dict(old));
    let doc = open(write_full(&doc, &SaveOptions::default()).unwrap());

    let r = read_destinations(&doc);
    assert!(!r.truncated && !r.tree_capped && r.unreadable.is_empty());
    assert!(r.destinations.iter().all(|d| d.name_exact));
    let t = Source::Names;
    let d = Source::Dests;
    let some = |s: &str| Some(s.to_string());
    let mut listed = dests_of(&doc);
    // The dictionary's order is the file's, so compare the two dictionary names in sorted order.
    let dict_part = listed.split_off(7);
    assert_eq!(
        listed,
        [
            (0, t, "alpha".to_string(), Some(1), some("XYZ"), true),
            (1, t, "beta".to_string(), Some(2), some("Fit"), true),
            (2, t, "\u{e9}".to_string(), Some(1), some("Fit"), true),
            (3, t, "remote".to_string(), None, some("Fit"), true),
            (4, t, "number".to_string(), None, None, true),
            (5, t, "long view".to_string(), Some(3), None, true),
            (6, t, "direct".to_string(), Some(2), some("Fit"), false),
        ]
    );
    let mut dict_part = dict_part;
    assert_eq!(dict_part.len(), 2);
    dict_part.sort_by(|a, b| a.2.cmp(&b.2));
    assert_eq!(
        dict_part.iter().map(|x| (x.1, x.2.clone(), x.3, x.4.clone(), x.5)).collect::<Vec<_>>(),
        [(d, "caf\u{e9}".to_string(), Some(3), some("Fit"), true), (d, "old".to_string(), Some(2), some("FitH"), true),]
    );
    assert!(dict_part.iter().all(|x| x.0 >= 7));
    // Reading changes nothing.
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    let _ = read_destinations(&doc);
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
}

#[test]
fn a_destinations_entry_that_is_not_a_dictionary_is_named_and_not_listed() {
    use crate::destinations::{Removal, destination_removal, read_destinations};
    let cases: [(&str, Object, &[&str]); 5] = [
        ("Names", Object::Int(3), &["names"]),
        ("Names", uri_with(vec![("Dests", Object::Name(b"x".to_vec()))]), &["names"]),
        ("Dests", Object::Array(vec![]), &["dests"]),
        ("Dests", Object::Null, &[]),
        ("Names", uri_with(vec![("Dests", Object::Null)]), &[]),
    ];
    for (key, value, expect) in cases {
        let mut doc = open(fixture());
        set_catalog_entry(&mut doc, key, value);
        let doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
        let r = read_destinations(&doc);
        assert!(r.destinations.is_empty(), "{key}");
        assert_eq!(r.unreadable, expect, "{key}");
        assert_eq!(destination_removal(&doc, 0), Removal::NoEntry);
    }
}

#[test]
fn removing_a_named_destination_keeps_the_others_in_order_and_everything_else() {
    use crate::destinations::{Removal, destination_removal, remove_destination};
    let mut doc = open(fixture());
    let p0 = page_ref(&doc, 0);
    let limits = Object::Array(vec![string(b"a"), string(b"c")]);
    let leaf = doc.add(uri_with(vec![
        (
            "Names",
            name_pairs(vec![(b"a", dest_array(p0.clone(), "Fit")), (b"b", dest_array(p0.clone(), "Fit")), (b"c", dest_array(p0.clone(), "Fit"))]),
        ),
        ("Limits", limits.clone()),
    ]));
    let direct = uri_with(vec![("Names", name_pairs(vec![(b"d", dest_array(p0.clone(), "Fit"))]))]);
    let held_names = doc.add(name_pairs(vec![(b"e", dest_array(p0.clone(), "Fit"))]));
    let indirect_names = Object::Ref(doc.add(uri_with(vec![("Names", Object::Ref(held_names))])));
    let root = doc.add(uri_with(vec![("Kids", Object::Array(vec![Object::Ref(leaf), direct, indirect_names]))]));
    set_names_root(&mut doc, Object::Ref(root));
    let mut dict = pdfcraft_cos::Dict::new();
    dict.set(b"x".to_vec(), dest_array(p0.clone(), "Fit"));
    dict.set(b"y".to_vec(), dest_array(p0.clone(), "Fit"));
    set_catalog_entry(&mut doc, "Dests", Object::Dict(dict));
    assert_eq!(names_of(&doc), ["a", "b", "c", "d", "e", "x", "y"]);

    // A position with no entry is not offered and changes no byte.
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    assert_eq!((destination_removal(&doc, 7), destination_removal(&doc, usize::MAX)), (Removal::NoEntry, Removal::NoEntry));
    remove_destination(&mut doc, 7).unwrap();
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
    // A name in a direct node or in a /Names array of its own is named and not changed.
    for i in [3, 4] {
        assert_eq!(destination_removal(&doc, i), Removal::Unsupported);
        assert!(remove_destination(&mut doc, i).is_err());
        assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
    }
    assert_eq!(dests_of(&doc).iter().map(|d| d.5).collect::<Vec<_>>(), [true, true, true, false, false, true, true]);

    // The middle name of a leaf: the others stay in order, /Limits stays as written, and the
    // page is untouched, in both saves.
    assert_eq!(destination_removal(&doc, 1), Removal::Yes);
    remove_destination(&mut doc, 1).unwrap();
    for saved in [write_incremental(&doc, &SaveOptions::default()).unwrap(), write_full(&doc, &SaveOptions::default()).unwrap()] {
        let d = open(saved);
        assert_eq!(names_of(&d), ["a", "c", "d", "e", "x", "y"]);
        assert_eq!(crate::pages(&d).unwrap().len(), 3);
    }
    assert_eq!(doc.get(leaf).as_dict().and_then(|l| l.get(b"Limits")), Some(&limits));
    // A name of the catalog's dictionary written in the catalog.
    remove_destination(&mut doc, 4).unwrap();
    assert_eq!(names_of(&doc), ["a", "c", "d", "e", "y"]);
    // Taking the last name of a leaf leaves the leaf, and the one of the dictionary the
    // dictionary.
    remove_destination(&mut doc, 0).unwrap();
    remove_destination(&mut doc, 0).unwrap();
    assert_eq!(names_of(&doc), ["d", "e", "y"]);
    assert_eq!(doc.get(leaf).as_dict().and_then(|l| l.get(b"Names")), Some(&Object::Array(vec![])));
    remove_destination(&mut doc, 2).unwrap();
    let catalog = doc.get(doc.root().unwrap());
    assert_eq!(catalog.as_dict().and_then(|c| c.get(b"Dests")), Some(&uri_with(vec![])));
}

#[test]
fn a_destination_dictionary_held_as_an_object_has_its_name_removed_where_it_is() {
    use crate::destinations::remove_destination;
    let mut doc = open(fixture());
    let p0 = page_ref(&doc, 0);
    let mut dict = pdfcraft_cos::Dict::new();
    dict.set(b"x".to_vec(), dest_array(p0.clone(), "Fit"));
    dict.set(b"y".to_vec(), dest_array(p0, "Fit"));
    let held = doc.add(Object::Dict(dict));
    set_catalog_entry(&mut doc, "Dests", Object::Ref(held));
    let mut names = names_of(&doc);
    names.sort();
    assert_eq!(names, ["x", "y"]);
    let first = names_of(&doc)[0].clone();
    remove_destination(&mut doc, 0).unwrap();
    let left = names_of(&doc);
    assert!(left.len() == 1 && left[0] != first);
    // The catalog still holds the reference.
    let catalog = doc.get(doc.root().unwrap());
    assert_eq!(catalog.as_dict().and_then(|c| c.get(b"Dests")), Some(&Object::Ref(held)));
}

#[test]
fn the_destination_listing_and_the_name_tree_walk_are_bounded() {
    use crate::destinations::{MAX_LISTED_DESTINATIONS, MAX_NAME_CHARS, MAX_TREE_DEPTH, MAX_TREE_NODES, read_destinations, remove_destination};
    let mut doc = open(fixture());
    let p0 = page_ref(&doc, 0);
    let dest = |p: &Object| dest_array(p.clone(), "Fit");
    let named = |n: usize| -> Vec<(Vec<u8>, Object)> { (0..n).map(|i| (format!("n{i:04}").into_bytes(), dest(&p0))).collect() };
    let leaf_of = |doc: &mut Document, n: usize| {
        let pairs = named(n);
        doc.add(uri_with(vec![("Names", name_pairs(pairs.iter().map(|(k, v)| (k.as_slice(), v.clone())).collect()))]))
    };

    // 300 names: 256 listed and truncated; a name past the 256th is still removed by position.
    let big = leaf_of(&mut doc, 300);
    set_names_root(&mut doc, Object::Ref(big));
    let r = read_destinations(&doc);
    assert!(r.destinations.len() == MAX_LISTED_DESTINATIONS && r.truncated);
    remove_destination(&mut doc, 280).unwrap();
    let after = doc.get(big);
    let Some(Object::Array(items)) = after.as_dict().and_then(|d| d.get(b"Names")) else { panic!("names") };
    assert_eq!(items.len(), 598);
    assert!(!items.iter().any(|o| matches!(o, Object::String(s) if s.bytes == b"n0280")));
    // Exactly 256 are not truncated.
    let exact = leaf_of(&mut doc, MAX_LISTED_DESTINATIONS);
    set_names_root(&mut doc, Object::Ref(exact));
    let r = read_destinations(&doc);
    assert!(r.destinations.len() == MAX_LISTED_DESTINATIONS && !r.truncated);

    // A name of 128 characters is cut at 127 and not exact; one of 127 is whole.
    let long = "\u{e9}".repeat(MAX_NAME_CHARS + 1);
    let mut utf16 = vec![0xFE, 0xFF];
    utf16.extend(long.encode_utf16().flat_map(u16::to_be_bytes));
    let whole = &utf16[..utf16.len() - 2];
    let leaf = doc.add(uri_with(vec![("Names", name_pairs(vec![(&utf16, dest(&p0)), (whole, dest(&p0))]))]));
    set_names_root(&mut doc, Object::Ref(leaf));
    let r = read_destinations(&doc);
    assert_eq!(r.destinations[0].name.chars().count(), MAX_NAME_CHARS);
    assert!(!r.destinations[0].name_exact && r.destinations[1].name_exact);
    assert_eq!(r.destinations[1].name.chars().count(), MAX_NAME_CHARS);

    // The tree: the root and up to 1023 kids are entered, so a name in the 1023rd kid is listed
    // and one in the 1024th is not, with the cut reported.
    let tree_of = |doc: &mut Document, kids: usize| {
        let named_leaf = leaf_of(doc, 1);
        let mut list: Vec<Object> = (1..kids).map(|_| Object::Ref(doc.add(uri_with(vec![("Names", Object::Array(vec![]))])))).collect();
        list.push(Object::Ref(named_leaf));
        doc.add(uri_with(vec![("Kids", Object::Array(list))]))
    };
    let t = tree_of(&mut doc, MAX_TREE_NODES - 1);
    set_names_root(&mut doc, Object::Ref(t));
    let r = read_destinations(&doc);
    assert!(r.destinations.len() == 1 && !r.tree_capped);
    let t = tree_of(&mut doc, MAX_TREE_NODES);
    set_names_root(&mut doc, Object::Ref(t));
    let r = read_destinations(&doc);
    assert!(r.destinations.is_empty() && r.tree_capped);

    // Levels: a name at level 16 is listed, one at level 17 is not.
    let chain = |doc: &mut Document, levels: usize| {
        let mut at = leaf_of(doc, 1);
        for _ in 1..levels {
            at = doc.add(uri_with(vec![("Kids", Object::Array(vec![Object::Ref(at)]))]));
        }
        at
    };
    let c = chain(&mut doc, MAX_TREE_DEPTH + 1);
    set_names_root(&mut doc, Object::Ref(c));
    let r = read_destinations(&doc);
    assert!(r.destinations.len() == 1 && !r.tree_capped);
    // An empty /Kids at level 16 has nothing to cut.
    let c = chain(&mut doc, MAX_TREE_DEPTH + 1);
    let mut deepest = c;
    for _ in 0..MAX_TREE_DEPTH {
        deepest = doc
            .get(deepest)
            .as_dict()
            .and_then(|d| d.reference(b"Kids").or_else(|| d.get(b"Kids").and_then(|k| k.as_array()?.first()?.as_ref())))
            .unwrap_or(deepest);
    }
    doc.update_dict(deepest, |d| d.set(b"Kids".to_vec(), Object::Array(vec![]))).unwrap();
    set_names_root(&mut doc, Object::Ref(c));
    let r = read_destinations(&doc);
    assert!(r.destinations.len() == 1 && !r.tree_capped);
    let c = chain(&mut doc, MAX_TREE_DEPTH + 2);
    set_names_root(&mut doc, Object::Ref(c));
    let r = read_destinations(&doc);
    assert!(r.destinations.is_empty() && r.tree_capped);

    // A node that is its own kid, or named by two parents, is read once.
    let looped = leaf_of(&mut doc, 1);
    doc.update_dict(looped, |d| d.set(b"Kids".to_vec(), Object::Array(vec![Object::Ref(looped)]))).unwrap();
    set_names_root(&mut doc, Object::Ref(looped));
    let r = read_destinations(&doc);
    assert!(r.destinations.len() == 1 && !r.tree_capped);
    let small = |doc: &mut Document, n: &[u8]| doc.add(uri_with(vec![("Names", name_pairs(vec![(n, dest(&p0))]))]));
    let (s, t) = (small(&mut doc, b"s"), small(&mut doc, b"t"));
    let twice = doc.add(uri_with(vec![("Kids", Object::Array(vec![Object::Ref(s), Object::Ref(s), Object::Ref(t)]))]));
    set_names_root(&mut doc, Object::Ref(twice));
    assert_eq!(names_of(&doc), ["s", "t"]);
    let (pa, pb) = (
        doc.add(uri_with(vec![("Kids", Object::Array(vec![Object::Ref(s)]))])),
        doc.add(uri_with(vec![("Kids", Object::Array(vec![Object::Ref(s), Object::Ref(t)]))])),
    );
    let two_parents = doc.add(uri_with(vec![("Kids", Object::Array(vec![Object::Ref(pa), Object::Ref(pb)]))]));
    set_names_root(&mut doc, Object::Ref(two_parents));
    assert_eq!(names_of(&doc), ["s", "t"]);
    // One `/Names` array shared by two leaves is read once.
    let shared = doc.add(name_pairs(vec![(b"s", dest(&p0))]));
    let two = |doc: &mut Document| doc.add(uri_with(vec![("Names", Object::Ref(shared))]));
    let (l1, l2) = (two(&mut doc), two(&mut doc));
    let root = doc.add(uri_with(vec![("Kids", Object::Array(vec![Object::Ref(l1), Object::Ref(l2)]))]));
    set_names_root(&mut doc, Object::Ref(root));
    assert_eq!(names_of(&doc), ["s"]);
}

#[test]
fn the_destination_walk_scans_a_huge_array_only_as_far_as_the_allowance() {
    use crate::destinations::{MAX_ITEMS_SCANNED, MAX_TREE_NODES, destination_removal, read_destinations};
    let mut doc = open(fixture());
    // Kids that are not nodes cost a scan each and are never listed.
    let kids = doc.add(uri_with(vec![("Kids", Object::Array(vec![Object::Int(1); MAX_ITEMS_SCANNED * 3]))]));
    set_names_root(&mut doc, Object::Ref(kids));
    take_work();
    let r = read_destinations(&doc);
    assert!(r.tree_capped && r.destinations.is_empty());
    assert_eq!(take_work(), MAX_ITEMS_SCANNED);
    // So do pairs whose key is not a string, however far a removal asks to look.
    let pairs = doc.add(uri_with(vec![("Names", Object::Array(vec![Object::Int(1); MAX_ITEMS_SCANNED * 6]))]));
    set_names_root(&mut doc, Object::Ref(pairs));
    take_work();
    assert_eq!(destination_removal(&doc, usize::MAX), crate::destinations::Removal::NoEntry);
    assert_eq!(take_work(), MAX_ITEMS_SCANNED);
    // Once the allowance is spent no later kid is looked at: a name behind the cut is not found.
    let named = doc.add(uri_with(vec![("Names", name_pairs(vec![(b"late", dest_array(Object::Int(1), "Fit"))]))]));
    let mut list = vec![Object::Int(1); MAX_ITEMS_SCANNED];
    list.push(Object::Ref(named));
    let root = doc.add(uri_with(vec![("Kids", Object::Array(list))]));
    set_names_root(&mut doc, Object::Ref(root));
    let r = read_destinations(&doc);
    assert!(r.tree_capped && r.destinations.is_empty());
    // Entries of the catalog's dictionary are scanned against an allowance of their own.
    let mut big = pdfcraft_cos::Dict::new();
    for i in 0..=MAX_ITEMS_SCANNED {
        big.set(format!("k{i}").into_bytes(), Object::Int(1));
    }
    set_catalog_entry(&mut doc, "Names", Object::Null);
    set_catalog_entry(&mut doc, "Dests", Object::Dict(big));
    take_work();
    assert_eq!(destination_removal(&doc, usize::MAX), crate::destinations::Removal::NoEntry);
    assert_eq!(take_work(), MAX_ITEMS_SCANNED);
    assert!(read_destinations(&doc).truncated);
    // After a tree cut by the allowance the dictionary still has its own whole allowance.
    set_catalog_entry(&mut doc, "Names", uri_with(vec![("Dests", Object::Ref(kids))]));
    take_work();
    assert_eq!(destination_removal(&doc, usize::MAX), crate::destinations::Removal::NoEntry);
    assert_eq!(take_work(), MAX_ITEMS_SCANNED * 2);
    // The dictionary of a file written for PDF 1.1 is read after a tree cut by the allowance
    // and after one cut by the node limit.
    set_catalog_entry(&mut doc, "Dests", uri_with(vec![("old", dest_array(Object::Int(1), "Fit"))]));
    let r = read_destinations(&doc);
    assert!(r.tree_capped && names_of(&doc) == ["old"]);
    let empties: Vec<Object> = (0..MAX_TREE_NODES + 500).map(|_| Object::Ref(doc.add(uri_with(vec![("Names", Object::Array(vec![]))])))).collect();
    let wide = doc.add(uri_with(vec![("Kids", Object::Array(empties))]));
    set_names_root(&mut doc, Object::Ref(wide));
    take_work();
    let r = read_destinations(&doc);
    // Once the node limit is reached no later kid is scanned.
    assert_eq!(take_work(), MAX_TREE_NODES + 1);
    assert!(r.tree_capped && names_of(&doc) == ["old"]);
}

#[test]
fn a_structural_dictionary_is_never_read_as_the_destinations_or_written() {
    use crate::destinations::{Removal, destination_removal, read_destinations, remove_destination};
    let doc = open(fixture());
    let root = doc.root().unwrap();
    let pages_node = doc.get(root).as_dict().and_then(|c| c.reference(b"Pages")).unwrap();
    let page = crate::pages(&doc).unwrap()[0].obj;
    let inner = pdfcraft_cos::ObjRef::new(3, 0);
    for target in [root, pages_node, inner, page] {
        for (key, expect) in [("Dests", "dests"), ("Names", "names")] {
            let mut doc = open(fixture());
            if key == "Dests" {
                set_catalog_entry(&mut doc, "Dests", Object::Ref(target));
            } else {
                set_names_root(&mut doc, Object::Ref(target));
            }
            let before = write_full(&doc, &SaveOptions::default()).unwrap();
            let r = read_destinations(&doc);
            assert!(r.destinations.is_empty() && r.unreadable == [expect], "{key} {target:?} {:?}", r.unreadable);
            assert_eq!(destination_removal(&doc, 0), Removal::NoEntry);
            remove_destination(&mut doc, 0).unwrap();
            assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
        }
    }
    // The catalog's /Names dictionary, as an object of its own, is not a /Dests dictionary.
    let mut doc = open(fixture());
    let dict = doc.add(uri_with(vec![("Dests", Object::Null)]));
    set_catalog_entry(&mut doc, "Names", Object::Ref(dict));
    set_catalog_entry(&mut doc, "Dests", Object::Ref(dict));
    let r = read_destinations(&doc);
    assert!(r.destinations.is_empty() && r.unreadable == ["dests"]);
    // An ordinary dictionary object is still read.
    let mut doc = open(fixture());
    let ordinary = doc.add(uri_with(vec![("a", dest_array(Object::Int(1), "Fit"))]));
    set_catalog_entry(&mut doc, "Dests", Object::Ref(ordinary));
    assert_eq!(names_of(&doc), ["a"]);
}

/// A form XObject stream with `/Subtype /Form`, the extra entries given and, when there is one,
/// `/Ref`.
fn ref_form(doc: &mut Document, entries: Vec<(&str, Object)>, reference: Option<Object>) -> pdfcraft_cos::ObjRef {
    let mut d = pdfcraft_cos::Dict::new();
    d.set(b"Subtype".to_vec(), Object::name("Form"));
    d.set(b"BBox".to_vec(), Object::Array(vec![Object::Int(0), Object::Int(0), Object::Int(10), Object::Int(10)]));
    for (k, v) in entries {
        d.set(k.as_bytes().to_vec(), v);
    }
    if let Some(r) = reference {
        d.set(b"Ref".to_vec(), r);
    }
    doc.add(Object::Stream(pdfcraft_cos::Stream::flate(d, b"0 0 5 5 re f")))
}

fn ref_to(file: Object, page: Option<Object>, id: Option<Object>) -> Object {
    let mut e = vec![("F", file)];
    if let Some(p) = page {
        e.push(("Page", p));
    }
    if let Some(i) = id {
        e.push(("ID", i));
    }
    uri_with(e)
}

/// (index, file, page index, page label, has /ID, exact) of each reference XObject listed.
type Listed = (usize, Option<String>, Option<i64>, Option<String>, bool, bool);

fn listed(doc: &Document) -> Vec<Listed> {
    crate::reference_xobjects::read_reference_xobjects(doc)
        .items
        .iter()
        .map(|r| (r.index, r.file.clone(), r.page_index, r.page_label.clone(), r.has_id, r.exact))
        .collect()
}

/// Hang every object added after the fixture's eleven from the catalog, so that a full save keeps
/// it (the writer keeps what the trailer reaches: `/Root`, `/Info` and the other trailer entries).
fn keep_added(doc: &mut Document) {
    let last = doc.object_numbers().last().copied().unwrap_or(11);
    let all = (12..=last).map(|n| Object::Ref(pdfcraft_cos::ObjRef::new(n, 0))).collect();
    set_catalog_entry(doc, "Kept", Object::Array(all));
}

fn has_ref(doc: &Document, r: pdfcraft_cos::ObjRef) -> bool {
    doc.get(r).as_dict().is_some_and(|d| d.contains(b"Ref"))
}

#[test]
fn reference_xobjects_are_listed_with_their_file_page_and_id() {
    use crate::reference_xobjects::read_reference_xobjects;
    let none = read_reference_xobjects(&open(fixture()));
    assert!(none.items.is_empty() && none.count == 0 && !none.truncated && !none.capped);

    let mut doc = open(fixture());
    let id = Object::Array(vec![string(b"a"), string(b"b")]);
    ref_form(&mut doc, vec![], Some(ref_to(string(b"a.pdf"), Some(Object::Int(2)), Some(id))));
    // A file specification dictionary: /UF wins over /F, and is a UTF-16 string here; the
    // reference dictionary, the specification and the page are each behind a reference.
    let spec = doc.add(uri_with(vec![("UF", string(&[0xFE, 0xFF, 0x00, 0xE9, 0x00, 0x41])), ("F", string(b"plain.pdf"))]));
    let page = doc.add(string(b"iv"));
    let b_ref = doc.add(ref_to(Object::Ref(spec), Some(Object::Ref(page)), Some(Object::Array(vec![string(b"only")]))));
    ref_form(&mut doc, vec![], Some(Object::Ref(b_ref)));
    // /UF that is not a string leaves /F.
    let spec = doc.add(uri_with(vec![("UF", Object::Int(1)), ("F", string(b"fallback.pdf"))]));
    ref_form(&mut doc, vec![], Some(ref_to(Object::Ref(spec), None, Some(Object::Array(vec![string(b"x"), Object::Int(1)])))));
    // /ID behind a reference counts when it holds two direct strings, and not when its two items
    // are references to strings.
    let id_ok = doc.add(Object::Array(vec![string(b"a"), string(b"b")]));
    ref_form(&mut doc, vec![], Some(ref_to(string(b"id1.pdf"), None, Some(Object::Ref(id_ok)))));
    let s1 = doc.add(string(b"a"));
    ref_form(&mut doc, vec![], Some(ref_to(string(b"id2.pdf"), None, Some(Object::Array(vec![Object::Ref(s1), Object::Ref(s1)])))));
    // Not reference XObjects: no /Ref, a /Ref that is not a dictionary, a stream that is not a
    // form, a /Subtype that is behind a reference, and a dictionary (not a stream).
    ref_form(&mut doc, vec![], None);
    ref_form(&mut doc, vec![], Some(Object::Int(3)));
    ref_form(&mut doc, vec![], Some(Object::Ref(pdfcraft_cos::ObjRef::new(900, 0))));
    let mut image = pdfcraft_cos::Dict::new();
    image.set(b"Subtype".to_vec(), Object::name("Image"));
    image.set(b"Ref".to_vec(), ref_to(string(b"i.pdf"), None, None));
    doc.add(Object::Stream(pdfcraft_cos::Stream::flate(image, b"x")));
    let form_name = doc.add(Object::name("Form"));
    let mut behind = pdfcraft_cos::Dict::new();
    behind.set(b"Subtype".to_vec(), Object::Ref(form_name));
    behind.set(b"Ref".to_vec(), ref_to(string(b"b.pdf"), None, None));
    doc.add(Object::Stream(pdfcraft_cos::Stream::flate(behind, b"x")));
    doc.add(uri_with(vec![("Subtype", Object::name("Form")), ("Ref", ref_to(string(b"d.pdf"), None, None))]));
    // A /Ref with nothing in it, and one whose /F and /Page are neither a string nor an integer.
    ref_form(&mut doc, vec![], Some(uri_with(vec![])));
    ref_form(&mut doc, vec![], Some(uri_with(vec![("F", Object::Int(5)), ("Page", Object::Real(1.5))])));

    keep_added(&mut doc);
    let doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
    let r = read_reference_xobjects(&doc);
    assert!(r.count == 7 && !r.truncated && !r.capped, "{r:?}");
    // The save numbers the objects again, but in the order they were added.
    assert!(r.items.windows(2).all(|w| w[0].object.num < w[1].object.num));
    assert_eq!(
        listed(&doc),
        [
            (0, Some("a.pdf".to_string()), Some(2), None, true, true),
            (1, Some("\u{e9}A".to_string()), None, Some("iv".to_string()), false, true),
            (2, Some("fallback.pdf".to_string()), None, None, false, true),
            (3, Some("id1.pdf".to_string()), None, None, true, true),
            (4, Some("id2.pdf".to_string()), None, None, false, true),
            (5, None, None, None, false, true),
            (6, None, None, None, false, true),
        ]
    );
    // Reading changes nothing.
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    let _ = read_reference_xobjects(&doc);
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);
}

/// The form the fixture-made test marked with `/Marker n`.
fn marked(doc: &Document, n: i64) -> pdfcraft_cos::ObjRef {
    doc.object_numbers()
        .into_iter()
        .map(|num| pdfcraft_cos::ObjRef::new(num, doc.generation(num)))
        .find(|r| doc.get(*r).as_dict().and_then(|d| d.get(b"Marker")) == Some(&Object::Int(n)))
        .expect("marked form")
}

fn removes(doc: &Document, index: usize) -> bool {
    crate::reference_xobjects::reference_xobject_removal(doc, index) == crate::reference_xobjects::Removal::Yes
}

fn files(doc: &Document) -> Vec<String> {
    crate::reference_xobjects::read_reference_xobjects(doc).items.into_iter().filter_map(|r| r.file).collect()
}

#[test]
fn removing_a_reference_xobject_takes_only_its_ref_and_keeps_the_others_in_order() {
    use crate::reference_xobjects::remove_reference_xobject;
    let mut doc = open(fixture());
    let target = doc.add(ref_to(string(b"B.pdf"), Some(Object::Int(1)), None));
    ref_form(&mut doc, vec![("Marker", Object::Int(1))], Some(ref_to(string(b"A.pdf"), None, None)));
    ref_form(&mut doc, vec![("Marker", Object::Int(2))], Some(Object::Ref(target)));
    ref_form(&mut doc, vec![("Marker", Object::Int(3))], Some(ref_to(string(b"C.pdf"), None, None)));
    keep_added(&mut doc);
    let mut doc = open(write_full(&doc, &SaveOptions::default()).unwrap());
    assert_eq!(files(&doc), ["A.pdf", "B.pdf", "C.pdf"]);

    // A position with nothing listed is not offered and changes no byte.
    let before = write_full(&doc, &SaveOptions::default()).unwrap();
    for index in [3, 256, usize::MAX] {
        assert!(!removes(&doc, index));
        remove_reference_xobject(&mut doc, index).unwrap();
    }
    assert_eq!(write_full(&doc, &SaveOptions::default()).unwrap(), before);

    // The middle one: only its /Ref goes; its content, its other entries and the dictionary the
    // /Ref pointed at stay, and the others keep their order and their positions.
    let b = marked(&doc, 2);
    let (content, bbox, old_ref) = {
        let o = doc.get(b);
        let Object::Stream(s) = &*o else { panic!("a stream") };
        (s.decoded().unwrap(), s.dict.get(b"BBox").cloned(), s.dict.get(b"Ref").and_then(Object::as_ref).expect("indirect /Ref"))
    };
    assert!(removes(&doc, 1));
    remove_reference_xobject(&mut doc, 1).unwrap();
    assert_eq!(files(&doc), ["A.pdf", "C.pdf"]);
    assert!(!has_ref(&doc, b) && doc.get(old_ref).as_dict().is_some());
    {
        let o = doc.get(b);
        let Object::Stream(s) = &*o else { panic!("a stream") };
        assert_eq!(s.decoded().unwrap(), content);
        assert_eq!(s.dict.get(b"BBox").cloned(), bbox);
        assert_eq!(s.dict.name(b"Subtype"), Some(&b"Form"[..]));
    }
    // It is gone in the incremental and in the full save too.
    for bytes in [write_incremental(&doc, &SaveOptions::default()).unwrap(), write_full(&doc, &SaveOptions::default()).unwrap()] {
        let saved = open(bytes);
        assert_eq!(files(&saved), ["A.pdf", "C.pdf"]);
        assert!(!has_ref(&saved, marked(&saved, 2)));
    }
    // The position now holds the next one; the first goes by position 0, and the last by 0 again.
    assert!(!removes(&doc, 2));
    remove_reference_xobject(&mut doc, 0).unwrap();
    assert!(!has_ref(&doc, marked(&doc, 1)) && has_ref(&doc, marked(&doc, 3)));
    remove_reference_xobject(&mut doc, 0).unwrap();
    assert!(crate::reference_xobjects::read_reference_xobjects(&doc).items.is_empty());
}

#[test]
fn a_reference_xobject_that_is_the_catalog_or_a_page_tree_node_keeps_its_ref() {
    use crate::reference_xobjects::{Removal, read_reference_xobjects, reference_xobject_removal, remove_reference_xobject};
    // The catalog (1), the /Pages node (2), an inner /Pages node (3) and a page (4) of the
    // fixture, each made a form XObject with a /Ref in turn.
    for num in 1..=4u32 {
        let mut doc = open(fixture());
        let at = pdfcraft_cos::ObjRef::new(num, 0);
        let mut d = doc.get(at).as_dict().cloned().unwrap();
        d.set(b"Subtype".to_vec(), Object::name("Form"));
        d.set(b"Ref".to_vec(), ref_to(string(b"x.pdf"), None, None));
        doc.set_existing(at, Object::Stream(pdfcraft_cos::Stream::flate(d, b"q Q"))).unwrap();
        // An ordinary reference XObject after it.
        ref_form(&mut doc, vec![], Some(ref_to(string(b"y.pdf"), None, None)));
        let found = read_reference_xobjects(&doc);
        assert!(found.count == 2 && found.items[0].object == at, "object {num}: {found:?}");
        let before = format!("{:?}", doc.get(at));
        assert_eq!(reference_xobject_removal(&doc, 0), Removal::Refused, "object {num}");
        assert!(!found.items[0].removable && found.items[1].removable, "object {num}");
        remove_reference_xobject(&mut doc, 0).unwrap();
        assert_eq!(format!("{:?}", doc.get(at)), before, "object {num}");
        // The other one is still removed, so it is the guard that refused.
        assert_eq!(reference_xobject_removal(&doc, 1), Removal::Yes, "object {num}");
        remove_reference_xobject(&mut doc, 1).unwrap();
        assert_eq!(files(&doc), ["x.pdf"], "object {num}");
    }
}

#[test]
fn a_reference_xobject_that_is_a_page_tree_node_is_found_by_its_number_whatever_generation_the_kids_entry_names() {
    use crate::reference_xobjects::{Removal, read_reference_xobjects, reference_xobject_removal};
    let mut doc = open(fixture());
    let page = pdfcraft_cos::ObjRef::new(4, 0);
    let mut d = doc.get(page).as_dict().cloned().unwrap();
    d.set(b"Subtype".to_vec(), Object::name("Form"));
    d.set(b"Ref".to_vec(), ref_to(string(b"x.pdf"), None, None));
    doc.set_existing(page, Object::Stream(pdfcraft_cos::Stream::flate(d, b"q Q"))).unwrap();
    // The inner /Pages node names the page with another generation, in an array behind a
    // reference.
    let kids = doc.add(Object::Array(vec![Object::Ref(pdfcraft_cos::ObjRef::new(4, 7)), Object::Ref(pdfcraft_cos::ObjRef::new(5, 0))]));
    doc.update_dict(pdfcraft_cos::ObjRef::new(3, 0), |n| n.set(b"Kids".to_vec(), Object::Ref(kids))).unwrap();
    assert_eq!(reference_xobject_removal(&doc, 0), Removal::Refused);
    assert!(!read_reference_xobjects(&doc).items[0].removable);

    // The walk is cut short, and says so, when an allowance is spent.
    let whole = crate::reference_xobjects::structural_with(&doc, 1000, 1000, 1000);
    assert!(!whole.1 && [1, 2, 3, 4, 5, 6].iter().all(|n| whole.0.contains(n)));
    // Reads: the /Pages node, the page 6, node 3, its /Kids (behind a reference) and the pages 5
    // and 4: six in all.
    assert!(!crate::reference_xobjects::structural_with(&doc, 6, 1000, 1000).1);
    assert!(crate::reference_xobjects::structural_with(&doc, 5, 1000, 1000).1);
    // ... and with three, the /Kids behind the reference (the fourth read) cannot be read.
    assert!(crate::reference_xobjects::structural_with(&doc, 3, 1000, 1000).1);
    // Nodes counted: the /Pages node and four under it.
    assert!(!crate::reference_xobjects::structural_with(&doc, 1000, 5, 1000).1);
    assert!(crate::reference_xobjects::structural_with(&doc, 1000, 4, 1000).1);
    // Entries: two in the /Pages node's /Kids and two in node 3's.
    assert!(!crate::reference_xobjects::structural_with(&doc, 1000, 1000, 4).1);
    assert!(crate::reference_xobjects::structural_with(&doc, 1000, 1000, 3).1);
}

/// `count` forms, each marked `/Marker` from 0, with a direct `/Ref`.
fn many_forms(count: usize) -> Document {
    let mut doc = open(fixture());
    for i in 0..count {
        ref_form(&mut doc, vec![("Marker", Object::Int(i as i64))], Some(ref_to(string(format!("{i}.pdf").as_bytes()), None, None)));
    }
    doc
}

#[test]
fn the_reference_xobject_listing_is_bounded_and_counts_what_it_does_not_list() {
    use crate::reference_xobjects::{MAX_LISTED, read_reference_xobjects, remove_reference_xobject};
    let exact = read_reference_xobjects(&many_forms(MAX_LISTED));
    assert!(exact.items.len() == MAX_LISTED && exact.count == MAX_LISTED && !exact.truncated);
    let mut doc = many_forms(MAX_LISTED + 44);
    let r = read_reference_xobjects(&doc);
    assert!(r.items.len() == MAX_LISTED && r.count == MAX_LISTED + 44 && r.truncated && !r.capped);
    assert!(r.items.iter().enumerate().all(|(i, x)| x.index == i));
    // The last position listed goes, the first one past the listing is not offered, though the
    // form is there.
    assert!(removes(&doc, MAX_LISTED - 1) && !removes(&doc, MAX_LISTED));
    remove_reference_xobject(&mut doc, MAX_LISTED).unwrap();
    assert!(has_ref(&doc, marked(&doc, MAX_LISTED as i64)));
    remove_reference_xobject(&mut doc, MAX_LISTED - 1).unwrap();
    assert!(!has_ref(&doc, marked(&doc, MAX_LISTED as i64 - 1)) && has_ref(&doc, marked(&doc, MAX_LISTED as i64 - 2)));
    assert_eq!(read_reference_xobjects(&doc).count, MAX_LISTED + 43);
}

#[test]
fn the_reference_xobject_scan_stops_when_its_allowance_is_spent_and_when_it_has_what_it_wants() {
    use crate::reference_xobjects::scan;
    let doc = many_forms(300);
    let objects = doc.object_numbers().len();
    // One read for each object scanned, none more for a /Ref written in the form.
    let whole = scan(&doc, None, objects);
    assert!(whole.count == 300 && !whole.capped);
    let short = scan(&doc, None, objects - 1);
    assert!(short.count == 299 && short.capped);
    assert!(scan(&doc, None, 0).capped);
    // Looking for the item at a position ends the walk there: the first form is the 12th object,
    // and the scan for position 0 with 12 reads is not capped while the full scan is.
    let first = 12;
    let one = scan(&doc, Some(0), first);
    assert!(one.items.len() == 1 && !one.capped);
    assert!(scan(&doc, None, first).capped);
    let four = scan(&doc, Some(3), first + 3);
    assert!(four.items.len() == 4 && !four.capped);
    assert!(scan(&doc, Some(3), first + 2).capped);
}

/// A document of the fixture's objects and one form whose `/Ref` is `reference`, written
/// directly.
fn one_form_with(reference: impl FnOnce(&mut Document) -> Object) -> Document {
    let mut doc = open(fixture());
    let r = reference(&mut doc);
    ref_form(&mut doc, vec![], Some(r));
    doc
}

#[test]
fn every_object_a_reference_xobject_leads_through_is_paid_for_before_it_is_read() {
    use crate::reference_xobjects::scan;
    // (what the reads are for, the document, how many reads beyond one for each object it takes)
    let cases: Vec<(&str, Document, usize)> = vec![
        (
            "/F behind a reference",
            one_form_with(|d| {
                let f = d.add(string(b"f.pdf"));
                ref_to_obj("F", Object::Ref(f))
            }),
            1,
        ),
        (
            "/Page behind a reference",
            one_form_with(|d| {
                let p = d.add(Object::Int(3));
                ref_to_obj("Page", Object::Ref(p))
            }),
            1,
        ),
        (
            "/ID behind a reference",
            one_form_with(|d| {
                let id = d.add(Object::Array(vec![string(b"a"), string(b"b")]));
                ref_to_obj("ID", Object::Ref(id))
            }),
            1,
        ),
        (
            "/F a file specification behind a reference with /UF behind a reference",
            one_form_with(|d| {
                let uf = d.add(string(b"u.pdf"));
                let spec = d.add(uri_with(vec![("UF", Object::Ref(uf))]));
                ref_to_obj("F", Object::Ref(spec))
            }),
            2,
        ),
        (
            "/F a file specification with /UF not a string and /F behind a reference",
            one_form_with(|d| {
                let f = d.add(string(b"f.pdf"));
                let spec = d.add(uri_with(vec![("UF", Object::Int(1)), ("F", Object::Ref(f))]));
                ref_to_obj("F", Object::Ref(spec))
            }),
            2,
        ),
        ("/Ref behind a reference", one_form_with(|d| Object::Ref(d.add(ref_to(string(b"r.pdf"), None, None)))), 1),
    ];
    for (what, doc, extra) in cases {
        let objects = doc.object_numbers().len();
        let ok = scan(&doc, None, objects + extra);
        assert!(ok.count == 1 && !ok.capped, "{what}: {ok:?}");
        let short = scan(&doc, None, objects + extra - 1);
        assert!(short.count == 0 && short.capped, "{what}: {short:?}");
    }
}

fn ref_to_obj(key: &str, value: Object) -> Object {
    uri_with(vec![(key, value)])
}

#[test]
fn a_reference_xobjects_ref_behind_references_is_followed_as_document_resolve_follows_it() {
    use crate::reference_xobjects::{read_reference_xobjects, scan};
    // A chain of `n` objects that are each a reference to the next, ending in the dictionary.
    let chain = |n: usize| {
        let mut doc = open(fixture());
        let mut next = Object::Ref(doc.add(ref_to(string(b"end.pdf"), None, None)));
        for _ in 0..n {
            next = Object::Ref(doc.add(next));
        }
        ref_form(&mut doc, vec![], Some(next.clone()));
        (doc, next)
    };
    let (doc, entry) = chain(32);
    assert!(doc.resolve(&entry).as_dict().is_some());
    assert_eq!(files(&doc), ["end.pdf"]);
    // 33 reads for the chain, one for each object scanned.
    let objects = doc.object_numbers().len();
    assert!(scan(&doc, None, objects + 33).count == 1 && scan(&doc, None, objects + 32).capped);
    let (doc, entry) = chain(33);
    assert!(doc.resolve(&entry).as_dict().is_none());
    assert!(read_reference_xobjects(&doc).items.is_empty());
    // A reference that leads back to itself ends in nothing and is not a scan that never ends.
    let mut doc = open(fixture());
    let looped = doc.add(Object::Null);
    doc.set_existing(looped, Object::Ref(looped)).unwrap();
    ref_form(&mut doc, vec![], Some(Object::Ref(looped)));
    let r = read_reference_xobjects(&doc);
    assert!(r.items.is_empty() && r.count == 0 && !r.capped);
}

#[test]
fn a_reference_xobjects_file_name_and_page_label_are_cut_at_127_characters() {
    use crate::reference_xobjects::{MAX_TEXT_CHARS, read_reference_xobjects};
    let long = |n: usize| "n".repeat(n);
    let mut doc = open(fixture());
    let whole = long(MAX_TEXT_CHARS);
    let over = long(MAX_TEXT_CHARS + 1);
    ref_form(&mut doc, vec![], Some(ref_to(string(whole.as_bytes()), Some(string(whole.as_bytes())), None)));
    ref_form(&mut doc, vec![], Some(ref_to(string(over.as_bytes()), None, None)));
    ref_form(&mut doc, vec![], Some(ref_to(string(b"ok.pdf"), Some(string(over.as_bytes())), None)));
    // Two forms may share one /Ref dictionary: each is listed.
    let shared = doc.add(ref_to(string(b"shared.pdf"), None, None));
    ref_form(&mut doc, vec![], Some(Object::Ref(shared)));
    ref_form(&mut doc, vec![], Some(Object::Ref(shared)));
    let r = read_reference_xobjects(&doc);
    assert_eq!(r.count, 5);
    let items = &r.items;
    assert!(items[0].exact && items[0].file.as_deref() == Some(whole.as_str()) && items[0].page_label.as_deref() == Some(whole.as_str()));
    assert!(!items[1].exact && items[1].file.as_deref() == Some(whole.as_str()));
    assert!(!items[2].exact && items[2].file.as_deref() == Some("ok.pdf") && items[2].page_label.as_deref() == Some(whole.as_str()));
    assert!(items[3].file == items[4].file && items[3].file.as_deref() == Some("shared.pdf"));
}
