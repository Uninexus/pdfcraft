//! A replacement through the run sources must leave the rows it does not reach as they were.
use std::sync::Arc;

use pdfcraft_cos::{Document, SaveOptions, write_incremental};

use super::*;

fn page(content: &str) -> Document {
    let objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let x = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n", objs.len() + 1).as_bytes());
    Document::open(Arc::new(out)).unwrap()
}

fn content(doc: &Document) -> String {
    let bytes = write_incremental(doc, &SaveOptions::default()).unwrap();
    let doc = Document::open(Arc::new(bytes)).unwrap();
    let p = pdfcraft_model::pages(&doc).swap_remove(0);
    let list = match doc.resolve(p.dict.get(b"Contents").unwrap()).as_ref() {
        Object::Array(a) => a.clone(),
        _ => vec![p.dict.get(b"Contents").unwrap().clone()],
    };
    let mut out = Vec::new();
    for o in &list {
        if let Object::Stream(s) = &*doc.resolve(o) {
            out.extend(s.decoded().unwrap_or_default());
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[test]
fn replace_keeps_untouched_rows() {
    let mut doc = page("BT /F1 10 Tf 12 TL 100 700 Td [(Hello) -300 (World)] TJ T* (second row here) Tj ET");
    let block = text_blocks(&doc, 0).unwrap().swap_remove(0);
    let start = block.editor_text.find("here").expect("the second row's word is in the paragraph");
    let (text, sources) = block.replaced(std::slice::from_ref(&(start..start + 4)), "there").expect("the range is on boundaries");
    let style = BlockStyle { sources, ..Default::default() };
    rewrite_block(&mut doc, 0, 0, Some(&text), &style).unwrap();
    let after = content(&doc);
    assert!(after.contains("-300"), "the first row keeps its TJ kerning: {after}");
}

/// The rewrite keeps every space typed, so a deleted word must not leave its two neighbouring
/// spaces behind; a run of spaces the paragraph had is left as it was.
#[test]
fn a_deleted_word_leaves_one_space() {
    let doc = page("BT /F1 10 Tf 100 700 Td (one two  three) Tj ET");
    let block = text_blocks(&doc, 0).unwrap().swap_remove(0);
    assert_eq!(block.editor_text, "one two  three");
    let two = block.editor_text.find("two").unwrap();
    let (text, _) = block.replaced(std::slice::from_ref(&(two..two + 3)), "").unwrap();
    assert_eq!(text, "one  three");
    let one = block.editor_text.find("one").unwrap();
    let (text, _) = block.replaced(std::slice::from_ref(&(one..one + 3)), "").unwrap();
    assert_eq!(text, " two  three");
    let (text, _) = block.replaced(std::slice::from_ref(&(two..two + 3)), "2").unwrap();
    assert_eq!(text, "one 2  three");
}

/// Reflowing around a replacement rejoins the wrapped rows and keeps the lines that are lines of
/// their own: a poem's rows stay rows, a body paragraph's wraps become spaces.
#[test]
fn replacing_while_reflowing_rejoins_wraps_and_keeps_short_lines() {
    let doc = full_rows(&["This sentence is long enough to fill a measure", "and this one is long enough to fill it as well", "Tail"], 300.0);
    let block = text_blocks(&doc, 0).unwrap().swap_remove(0);
    assert_eq!(block.joins, [Some(1), Some(1)], "{:?}", block.reflow_text);
    let at = block.editor_text.find("long").unwrap();
    let (text, sources) = block.replaced_reflowing(std::slice::from_ref(&(at..at + 4)), "very long").unwrap();
    assert_eq!(text, "This sentence is very long enough to fill a measure and this one is long enough to fill it as well Tail");
    assert_eq!(sources.last().map(|(range, _)| range.end), Some(text.len()), "the spans cover the text: {sources:?}");
    // Not reflowing leaves the row break, which is what held the rows in place.
    let (kept, _) = block.replaced(std::slice::from_ref(&(at..at + 4)), "very long").unwrap();
    assert_eq!(kept, "This sentence is very long enough to fill a measure\nand this one is long enough to fill it as well\nTail");

    let poem = page("BT /F1 10 Tf 72 700 Td (One) Tj ET BT /F1 10 Tf 72 688 Td (Two) Tj ET BT /F1 10 Tf 72 676 Td (Three) Tj ET");
    let block = text_blocks(&poem, 0).unwrap().swap_remove(0);
    assert_eq!(block.joins, [None, None]);
    let at = block.editor_text.find("Two").unwrap();
    let (text, _) = block.replaced_reflowing(std::slice::from_ref(&(at..at + 3)), "Second").unwrap();
    assert_eq!(text, "One\nSecond\nThree");
}

/// `rows` set in 10 pt Helvetica, each but the last spread to `measure` points with word spacing,
/// so none of them is marked and every one but the last fills the measure.
fn full_rows(rows: &[&str], measure: f64) -> Document {
    let mut content = String::new();
    for (i, row) in rows.iter().enumerate() {
        let probe = page(&format!("BT /F1 10 Tf 72 700 Td ({row}) Tj ET"));
        let rect = crate::text_lines(&probe, 0).unwrap()[0].rect;
        let tw = if i + 1 < rows.len() { (measure - (rect[2] - rect[0])) / row.matches(' ').count().max(1) as f64 } else { 0.0 };
        content.push_str(&format!("BT /F1 10 Tf {tw:.4} Tw 72 {} Td ({row}) Tj ET\n", 700.0 - 12.0 * i as f64));
    }
    page(&content)
}

/// A hyphen that ends a full row may be a real one: reflowing around a replacement elsewhere
/// keeps the row break after it instead of deleting the hyphen ("well-" "known").
#[test]
fn reflowing_keeps_a_hyphen_that_ends_a_row() {
    let rows = [
        "The committee reviewed the proposal and agreed that the schedule",
        "for the new building would depend on the weather and on a well-",
        "known author who had promised to prepare a second plan that",
        "avoids the lowest field. Work starts in spring.",
    ];
    let doc = full_rows(&rows, 330.0);
    let block = text_blocks(&doc, 0).unwrap().swap_remove(0);
    assert_eq!(block.joins, [Some(1), None, Some(1)], "{:?}", block.reflow_text);
    let at = block.editor_text.find("committee").unwrap();
    let (text, _) = block.replaced_reflowing(std::slice::from_ref(&(at..at + 9)), "board").unwrap();
    assert!(text.contains("well-\nknown"), "{text}");
}

/// A hyphen the replacement itself ends in is never taken for a hyphenation to rejoin.
#[test]
fn reflowing_never_removes_a_hyphen_the_replacement_ends_in() {
    let mut block = text_blocks(&page("BT /F1 10 Tf 72 700 Td (one two) Tj ET"), 0).unwrap().swap_remove(0);
    block.editor_text = "one two\nthree".into();
    block.joins = vec![Some(1)];
    let (text, _) = block.replaced_reflowing(std::slice::from_ref(&(4..7)), "semi-").unwrap();
    assert_eq!(text, "one semi- three");
}

/// Rows of Han, kana or Thai rejoin without a space; a space is what the break stood for only
/// between words of a language that has them.
#[test]
fn rows_of_unspaced_scripts_rejoin_without_a_space() {
    let mut block = text_blocks(&page("BT /F1 10 Tf 72 700 Td (one) Tj ET"), 0).unwrap().swap_remove(0);
    block.editor_text = "日本語の\n文章です".into();
    block.joins = vec![Some(0)];
    let at = block.editor_text.find("文章").unwrap();
    let (text, _) = block.replaced_reflowing(std::slice::from_ref(&(at..at + 6)), "手紙").unwrap();
    assert_eq!(text, "日本語の手紙です");
}
