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

/// Width in points of `text` in 10 pt Helvetica.
fn width(text: &str) -> f64 {
    let rect = crate::text_lines(&page(&format!("BT /F1 10 Tf 72 700 Td ({text}) Tj ET")), 0).unwrap()[0].rect;
    rect[2] - rect[0]
}

/// `text` broken greedily into rows no wider than `measure` points, the way a word processor sets
/// ragged-right text: each row ends because the next word would not have fitted.
fn greedy_rows(text: &str, measure: f64) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match rows.last_mut() {
            Some(row) if width(&format!("{row} {word}")) <= measure => {
                row.push(' ');
                row.push_str(word);
            }
            _ => rows.push(word.to_owned()),
        }
    }
    rows
}

/// One unmarked 10 pt row per entry of `rows`, at 12 pt leading from x = 72, with no word spacing.
fn ragged_rows(rows: &[String]) -> Document {
    let mut content = String::new();
    for (i, row) in rows.iter().enumerate() {
        content.push_str(&format!("BT /F1 10 Tf 72 {} Td ({row}) Tj ET\n", 700.0 - 12.0 * i as f64));
    }
    page(&content)
}

const FLOWING: &str = "The committee reviewed the proposal and agreed that the schedule for the new building would depend on \
the weather during the autumn months so the contractor was asked to prepare a second plan that avoids the lowest field";

/// Ragged-right rows (none marked, none reaching a common edge but the longest) are wraps because
/// the next word would not have fitted: that is what a word processor's rows look like.
#[test]
fn ragged_right_rows_are_wraps_when_the_next_word_would_not_fit() {
    let rows = greedy_rows(FLOWING, 250.0);
    assert!(rows.len() >= 5, "{rows:?}");
    let doc = ragged_rows(&rows);
    let block = text_blocks(&doc, 0).unwrap().swap_remove(0);
    assert_eq!(block.rows.len(), rows.len());
    assert!(block.joins.iter().all(|j| *j == Some(1)), "{:?} in {rows:?}", block.joins);
    let at = block.editor_text.find("proposal").unwrap();
    let (text, _) = block.replaced_reflowing(std::slice::from_ref(&(at..at + 8)), "proposal and the long budget").unwrap();
    assert_eq!(text, FLOWING.replace("proposal", "proposal and the long budget"));
}

/// Rows that are lines of their own keep their breaks even when most of them are long: the next
/// word would have fitted on at least one of them.
#[test]
fn lines_the_next_word_would_have_fitted_on_stay_lines() {
    let poem: Vec<String> = [
        "Shall I compare thee to a summer's day thou art more",
        "Lovely and more temperate rough winds do shake the darling",
        "Buds of May and summer's lease hath all too short a date",
        "Sometime too hot the eye of heaven shines and often is his",
    ]
    .map(String::from)
    .to_vec();
    let block = text_blocks(&ragged_rows(&poem), 0).unwrap().swap_remove(0);
    assert_eq!(block.rows.len(), 4);
    assert!(block.joins.iter().all(Option::is_none), "{:?}", block.joins);
    // A list of short items with one long one.
    let items: Vec<String> =
        ["Apples", "Pears and plums", "Oranges, lemons and a great many other kinds of citrus fruit", "Figs"].map(String::from).to_vec();
    let block = text_blocks(&ragged_rows(&items), 0).unwrap().swap_remove(0);
    assert!(block.joins.iter().all(Option::is_none), "{:?}", block.joins);
    let at = block.editor_text.find("Pears").unwrap();
    let (text, _) = block.replaced_reflowing(std::slice::from_ref(&(at..at + 5)), "Green pears").unwrap();
    assert_eq!(text.matches('\n').count(), 3, "{text:?}");
}

/// A sentence that ends a row with more than an em to spare ends a paragraph or an item, even
/// though the next word is wider than the room left; the rest of the paragraph still flows.
#[test]
fn a_sentence_end_with_room_to_spare_is_a_break() {
    let rows = [
        "The committee reviewed the proposal and agreed that the schedule for",
        "construction would depend on the weather during the autumn.",
        "Extraordinarily wide words open this row and the weather during the autumn",
        "the contractor was asked to prepare a second plan that avoids the lowest",
        "field and the river bank",
    ];
    let room = width(rows[2]) - width(rows[1]);
    assert!(room > 10.0 && room < width("Extraordinarily"), "{room}");
    let doc = ragged_rows(&rows.map(String::from));
    assert_eq!(joins_of(&doc), [Some(1), None, Some(1), Some(1)]);
}

/// One text object per row: its x position and the operators that show it.
fn placed_rows(rows: &[(f64, String)]) -> Document {
    let mut content = String::new();
    for (i, (x, ops)) in rows.iter().enumerate() {
        content.push_str(&format!("BT /F1 10 Tf {x} {} Td {ops} ET\n", 700.0 - 12.0 * i as f64));
    }
    page(&content)
}

fn shown(row: &str) -> String {
    format!("({row}) Tj")
}

/// `text` broken greedily, the first row no wider than `first` points and the others `rest`.
fn greedy_rows_in(text: &str, first: f64, rest: f64) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    for word in text.split(' ') {
        let cap = if rows.len() <= 1 { if rows.is_empty() { first } else { first.max(0.0) * 0.0 + rest } } else { rest };
        match rows.last_mut() {
            Some(row) if width(&format!("{row} {word}")) <= cap => {
                row.push(' ');
                row.push_str(word);
            }
            _ => rows.push(word.to_owned()),
        }
    }
    rows
}

fn joins_of(doc: &Document) -> Vec<Option<usize>> {
    let blocks = text_blocks(doc, 0).unwrap();
    assert_eq!(blocks.len(), 1, "one block: {:?}", blocks.iter().map(|b| b.editor_text.clone()).collect::<Vec<_>>());
    blocks.into_iter().next().map(|b| b.joins).unwrap_or_default()
}

fn at_margin(rows: &[&str]) -> Document {
    placed_rows(&rows.iter().map(|r| (72.0, shown(r))).collect::<Vec<_>>())
}

/// Rows that are lines of their own keep their breaks. The break after the widest row of a block
/// has no room left whatever follows, so it is no evidence of a wrap: two rows (or three, one of
/// them the widest) never are flow, and a few rows of similar width are not either.
#[test]
fn lines_of_similar_width_are_not_flow() {
    let cases: [(&str, Vec<&str>); 7] = [
        ("two-line address", vec!["1600 Pennsylvania Avenue NW", "Washington, DC 20500"]),
        ("three-line address", vec!["Department of Computer Science", "University of Somewhere Large", "Springfield, Illinois 62704"]),
        ("code listing", vec!["total = first + second;", "while (total < limit) { total = total * 2 + offset; }", "print(total);", "return total;"]),
        ("two-line code", vec!["let total = first + second + third;", "return total;"]),
        ("table column", vec!["Alice Johnson", "Bob Smith", "Carol Williams", "Dan Lee"]),
        ("couplet", vec!["Shall I compare thee to a summer's day", "Thou art more lovely and more temperate"]),
        ("signature block", vec!["Yours sincerely", "Jane Doe", "Chief Executive Officer, Acme Corporation"]),
    ];
    for (name, rows) in cases {
        let joins = joins_of(&at_margin(&rows));
        assert!(joins.iter().all(Option::is_none), "{name}: {joins:?}");
    }
}

/// Centred rows look ragged on the right by half of their slack: they are not ragged-right flow.
#[test]
fn centred_rows_are_not_ragged_right_flow() {
    let rows = greedy_rows(FLOWING, 130.0);
    let widest = rows.iter().map(|r| width(r)).fold(0.0, f64::max);
    let placed: Vec<(f64, String)> = rows.iter().map(|r| (72.0 + (widest - width(r)) / 2.0, shown(r))).collect();
    let doc = placed_rows(&placed);
    let blocks = text_blocks(&doc, 0).unwrap();
    assert_eq!(blocks.len(), 1);
    // The rows near the widest one end near the right edge too, which the full-measure rule has
    // always taken for justified text; the ragged rule adds no wrap to what it takes.
    let by_full_measure = blocks[0].editor_text.matches('\n').count() - blocks[0].reflow_text.matches('\n').count();
    let wraps = blocks[0].joins.iter().filter(|j| j.is_some()).count();
    assert!(wraps <= by_full_measure && wraps < blocks[0].joins.len(), "{:?}", blocks[0].joins);
}

/// An item with a hanging indent (its wrapped rows start further right than its marker) flows.
#[test]
fn a_hanging_indent_item_is_ragged_right_flow() {
    let rows = greedy_rows_in(&format!("1. {FLOWING}"), 250.0, 238.0);
    let placed: Vec<(f64, String)> = rows.iter().enumerate().map(|(i, r)| (if i == 0 { 72.0 } else { 84.0 }, shown(r))).collect();
    let joins = joins_of(&placed_rows(&placed));
    assert!(rows.len() >= 4 && joins.iter().all(|j| *j == Some(1)), "{joins:?} in {rows:?}");
}

/// A first line's indent is no reason to take a paragraph for lines; a later row indented that
/// way opens the next paragraph, whose break stays.
#[test]
fn a_first_line_indent_flows_and_an_indented_row_is_a_paragraph_boundary() {
    let rows = greedy_rows_in(FLOWING, 238.0, 250.0);
    let placed: Vec<(f64, String)> = rows.iter().enumerate().map(|(i, r)| (if i == 0 { 84.0 } else { 72.0 }, shown(r))).collect();
    let joins = joins_of(&placed_rows(&placed));
    assert!(rows.len() >= 4 && joins.iter().all(|j| *j == Some(1)), "{joins:?} in {rows:?}");

    let second = greedy_rows_in("Meanwhile the contractor ordered the steel and the glass for the new building from two suppliers", 238.0, 250.0);
    let mut placed: Vec<(f64, String)> = rows.iter().map(|r| (72.0, shown(r))).collect();
    let boundary = placed.len() - 1;
    placed.extend(second.iter().enumerate().map(|(i, r)| (if i == 0 { 81.0 } else { 72.0 }, shown(r))));
    let doc = placed_rows(&placed);
    let blocks = text_blocks(&doc, 0).unwrap();
    assert_eq!(blocks.len(), 1, "one block for the test to mean anything");
    let joins = &blocks[0].joins;
    assert_eq!(joins.get(boundary).copied().flatten(), None, "{joins:?}");
    assert!(joins.iter().enumerate().all(|(i, j)| i == boundary || *j == Some(1)), "{joins:?}");
}

/// One row much longer than the rest makes the others short of it by far more than a word.
#[test]
fn an_outlier_long_row_keeps_the_rows() {
    let mut rows = greedy_rows(FLOWING, 250.0);
    rows[2] = format!("{} and a great deal more text that makes this row far wider than the rest", rows[2]);
    let joins = joins_of(&ragged_rows(&rows));
    assert!(joins.iter().all(Option::is_none), "{joins:?}");
}

/// The first word of a row may be drawn in several runs (here two colours): it is measured whole.
#[test]
fn a_first_word_drawn_in_two_runs_is_measured_whole() {
    // The room left on the second row (46 pt) holds "Extra" but not the whole word.
    let rows = [
        "The committee reviewed the proposal and agreed that the schedule for",
        "construction would depend on the weather during the autumn rain",
        "Extraordinarily wide words open this row and the weather during the autumn",
        "the contractor was asked to prepare a second plan that avoids the lowest",
        "field and the river bank",
    ];
    let mut placed: Vec<(f64, String)> = rows.iter().map(|r| (72.0, shown(r))).collect();
    let whole = joins_of(&placed_rows(&placed));
    assert!(whole.iter().all(|j| *j == Some(1)), "{whole:?}");
    let rest = rows[2].strip_prefix("Extraordinarily").unwrap();
    placed[2].1 = format!("(Extra) Tj 1 0 0 rg (ordinarily) Tj 0 g ({rest}) Tj");
    let split = joins_of(&placed_rows(&placed));
    assert_eq!(split, whole, "the runs of the word are one word");
}
