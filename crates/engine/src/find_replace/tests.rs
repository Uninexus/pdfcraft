use std::collections::BTreeMap;
use std::sync::Arc;

use super::*;
use crate::{DocId, Session};

/// A one-page document with `content` and two fonts: `F1` Helvetica, `F2` Helvetica-Bold, and
/// `F3` a subset of a face no computer has, with glyphs for `a` and `b` only.
fn pdf(content: &str) -> Vec<u8> {
    let objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R /Resources << /Font << /F1 5 0 R /F2 6 0 R /F3 7 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>".into(),
        "<< /Type /Font /Subtype /TrueType /BaseFont /ABCDEF+NoSuchFace /FirstChar 97 /LastChar 99 /Widths [500 520 0] /Encoding /WinAnsiEncoding >>"
            .into(),
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

fn open(bytes: Vec<u8>) -> (Session, DocId) {
    let mut s = Session::new().with_clock(|| 1_700_000_000);
    let id = s.open("replace.pdf", None, Arc::new(bytes), None).expect("opens");
    (s, id)
}

fn query(find: &str, with: &str) -> Replace {
    Replace { find: find.into(), with: with.into(), ..Default::default() }
}

/// Plan and apply; the plan's counts.
fn replace_all(s: &mut Session, id: DocId, q: &Replace) -> Replacement {
    let pages: Vec<usize> = (0..s.get(id).unwrap().info.pages.len()).collect();
    let plan = s.get(id).unwrap().plan_replace(&pages, q, Which::All, None).unwrap();
    if let Some(edit) = plan.edit.clone() {
        s.apply(id, edit).unwrap();
    }
    plan
}

fn texts(s: &Session, id: DocId, page: usize) -> Vec<String> {
    s.get(id).unwrap().text_blocks(page).into_iter().map(|b| b.text).collect()
}

/// The page text as the find bar reads it (the text layer's glyphs).
fn page_text(s: &Session, id: DocId, page: usize) -> pdfcraft_render::PageText {
    let doc = s.get(id).unwrap();
    let mut r = pdfcraft_render::PageRenderer::new(doc.bytes.clone(), pdfcraft_render::RenderConfig::default());
    let out = r.render(pdfcraft_render::RenderRequest { page, kind: pdfcraft_render::RequestKind::Text, scale: 1.0, ..Default::default() });
    out.text.map(|t| t.as_ref().clone()).unwrap_or_default()
}

#[test]
fn a_replaced_word_keeps_its_paragraphs_font_and_size() {
    let (mut s, id) = open(pdf("BT /F1 14 Tf 72 700 Td (The quick brown fox) Tj ET"));
    let plan = replace_all(&mut s, id, &query("quick", "slow"));
    assert_eq!((plan.replaced, plan.pages.clone(), plan.skipped_total()), (1, vec![0], 0));
    assert_eq!(texts(&s, id, 0), ["The slow brown fox"]);
    let lines = s.get(id).unwrap().text_lines(0);
    assert_eq!(lines.len(), 1);
    assert_eq!((lines[0].font.as_str(), lines[0].base_font.as_str(), lines[0].size), ("F1", "Helvetica", 14.0));
    assert_eq!(s.get(id).unwrap().can_undo(), Some("Replace all"));
}

/// A red word and a bold name inside a paragraph keep their own style when words around them
/// are replaced; a replacement inside the red word stays red.
#[test]
fn a_mixed_style_paragraph_keeps_the_other_runs_styles() {
    let content = "BT /F1 12 Tf 72 700 Td 0 g (Please pay the invoice ) Tj 1 0 0 rg (now) Tj 0 g (, before the end of the month,) Tj ET
BT /F1 12 Tf 72 686 Td 0 g (as agreed with ) Tj /F2 12 Tf (Ada Lovelace) Tj /F1 12 Tf (.) Tj ET";
    let (mut s, id) = open(pdf(content));
    assert_eq!(texts(&s, id, 0).len(), 1, "one paragraph of five runs");
    let q = Replace { whole_words: true, ..query("the", "THE") };
    let plan = replace_all(&mut s, id, &q);
    assert_eq!(plan.replaced, 3);
    let plan = replace_all(&mut s, id, &query("now", "today"));
    assert_eq!(plan.replaced, 1);
    assert_eq!(texts(&s, id, 0), ["Please pay THE invoice today, before THE end of THE month, as agreed with Ada Lovelace."]);
    let lines = s.get(id).unwrap().text_lines(0);
    let run = |word: &str| lines.iter().find(|l| l.text.contains(word)).unwrap_or_else(|| panic!("no run with {word:?}: {lines:?}"));
    assert_eq!(run("today").color, [1.0, 0.0, 0.0], "the replacement takes the red run it lands in");
    assert_eq!(run("today").text.trim(), "today", "and only it is red");
    assert_eq!(run("Ada").base_font, "Helvetica-Bold");
    assert_eq!(run("Ada").text.trim(), "Ada Lovelace");
    assert_eq!(run("invoice").color, [0.0, 0.0, 0.0]);
    assert_eq!(run("invoice").base_font, "Helvetica");
}

#[test]
fn replace_all_across_pages_is_one_undo_step() {
    let (mut s, id) = open(crate::tests::fixture(3));
    let before = s.get(id).unwrap().bytes.clone();
    let plan = replace_all(&mut s, id, &query("page", "Sheet"));
    assert_eq!((plan.replaced, plan.pages.clone()), (3, vec![0, 1, 2]));
    assert!(matches!(&plan.edit, Some(Edit::Batch { edits, .. }) if edits.len() == 3));
    for p in 0..3 {
        assert_eq!(texts(&s, id, p), [format!("Sheet {}", p + 1)]);
    }
    s.undo(id).unwrap();
    for p in 0..3 {
        assert_eq!(texts(&s, id, p), [format!("Page {}", p + 1)]);
    }
    assert_eq!(s.get(id).unwrap().can_undo(), None, "one step undid all three pages");
    assert_eq!(s.get(id).unwrap().bytes, before, "the original bytes are back");
}

#[test]
fn whole_words_and_case_follow_the_find_options() {
    let content = "BT /F1 12 Tf 72 700 Td (cat Cats concat CAT) Tj ET";
    let count = |q: Replace| {
        let (s, id) = open(pdf(content));
        s.get(id).unwrap().plan_replace(&[0], &q, Which::All, None).unwrap().replaced
    };
    assert_eq!(count(query("cat", "dog")), 4);
    assert_eq!(count(Replace { whole_words: true, ..query("cat", "dog") }), 2);
    assert_eq!(count(Replace { case_sensitive: true, ..query("cat", "dog") }), 2);
    assert_eq!(count(Replace { case_sensitive: true, whole_words: true, ..query("CAT", "dog") }), 1);
    let (mut s, id) = open(pdf(content));
    replace_all(&mut s, id, &Replace { whole_words: true, ..query("cat", "dog") });
    assert_eq!(texts(&s, id, 0), ["dog Cats concat dog"]);
    // An empty replacement deletes the match.
    replace_all(&mut s, id, &query(" concat", ""));
    assert_eq!(texts(&s, id, 0), ["dog Cats dog"]);
}

#[test]
fn a_match_spanning_two_paragraphs_is_skipped_and_counted() {
    let content = "BT /F1 12 Tf 72 700 Td (The first ends here) Tj ET BT /F1 12 Tf 72 500 Td (second starts there) Tj ET";
    let (mut s, id) = open(pdf(content));
    assert_eq!(texts(&s, id, 0).len(), 2);
    let plan = replace_all(&mut s, id, &query("here second", "x"));
    assert_eq!(plan.replaced, 0);
    assert_eq!(plan.skipped, BTreeMap::from([(Skip::SpansParagraphs, 1)]));
    assert!(plan.edit.is_none());
    assert_eq!(texts(&s, id, 0), ["The first ends here", "second starts there"]);
}

/// The subset in `F3` has glyphs for `a` and `b` only, from a face no computer has: a `c` would
/// be drawn in Helvetica instead. That paragraph is left alone and counted, the other replaced.
#[test]
fn a_character_the_font_cannot_draw_is_skipped_and_counted() {
    let content = "BT /F3 12 Tf 72 700 Td (ab ba) Tj ET BT /F1 12 Tf 72 500 Td (ab in Helvetica) Tj ET";
    let (mut s, id) = open(pdf(content));
    let before = texts(&s, id, 0);
    assert_eq!(before, ["ab ba", "ab in Helvetica"]);
    let plan = replace_all(&mut s, id, &query("ab", "cb"));
    assert_eq!(plan.replaced, 1);
    assert_eq!(plan.skipped, BTreeMap::from([(Skip::MissingGlyphs, 1)]));
    assert_eq!(texts(&s, id, 0), ["ab ba", "cb in Helvetica"]);
    let lines = s.get(id).unwrap().text_lines(0);
    assert!(lines.iter().filter(|l| l.text.contains("ab ba")).all(|l| l.font == "F3"), "the subset paragraph keeps its font: {lines:?}");
    // What the subset can draw still goes in.
    let plan = replace_all(&mut s, id, &query("ab ba", "ba ab"));
    assert_eq!((plan.replaced, plan.skipped_total()), (1, 0));
    assert_eq!(texts(&s, id, 0)[0], "ba ab");
}

/// The styled rewrite (a paragraph of several runs) reports the missing character as the
/// one-style rewrite above does.
#[test]
fn a_character_the_font_cannot_draw_is_skipped_in_a_paragraph_of_several_runs() {
    let (mut s, id) = open(pdf("BT /F3 12 Tf 72 700 Td (ab ) Tj /F1 12 Tf (and Helvetica) Tj ET"));
    let plan = replace_all(&mut s, id, &query("ab", "cb"));
    assert_eq!(plan.replaced, 0);
    assert_eq!(plan.skipped, BTreeMap::from([(Skip::MissingGlyphs, 1)]));
    assert!(plan.edit.is_none());
}

/// A character Helvetica has no code for is never written as a box or a `?`: whether another font
/// has it or none does, the match is left alone and reported.
#[test]
fn a_replacement_outside_the_fonts_encoding_is_reported_and_changes_nothing() {
    for with in ["\u{65E5}\u{672C}", "\u{1F600}"] {
        let (mut s, id) = open(pdf("BT /F1 12 Tf 72 700 Td (one cat) Tj ET"));
        let plan = replace_all(&mut s, id, &query("cat", with));
        assert_eq!((plan.replaced, plan.skipped_total()), (0, 1), "{with}: {plan:?}");
        assert!(plan.edit.is_none(), "{with}");
        assert_eq!(texts(&s, id, 0), ["one cat"]);
    }
}

/// The rects of the page's text lines holding `word`.
fn line_rects(s: &Session, id: DocId, word: &str) -> Vec<[f64; 4]> {
    s.get(id).unwrap().text_lines(0).into_iter().filter(|l| l.text.contains(word)).map(|l| l.rect).collect()
}

/// A paragraph of several rows keeps its width and gains rows below; what is under it stays where
/// it was, so the new rows run over it.
#[test]
fn a_longer_replacement_adds_rows_and_moves_nothing_else() {
    let content =
        "BT /F1 12 Tf 14 TL 72 700 Td (alpha beta gamma delta) Tj T* (epsilon zeta eta theta) Tj ET BT /F1 12 Tf 72 650 Td (next paragraph) Tj ET";
    let (mut s, id) = open(pdf(content));
    assert_eq!(texts(&s, id, 0).len(), 2);
    let right = s.get(id).unwrap().text_blocks(0)[0].rect[2];
    let next = line_rects(&s, id, "next paragraph");
    let with = "a much longer replacement that takes several more rows than the paragraph has";
    let plan = replace_all(&mut s, id, &query("beta", with));
    assert_eq!((plan.replaced, plan.skipped_total()), (1, 0));
    let lines = s.get(id).unwrap().text_lines(0);
    let rows: Vec<_> = lines.iter().filter(|l| !l.text.contains("next paragraph")).collect();
    assert!(rows.len() >= 5, "{rows:?}");
    assert!(rows.iter().all(|l| l.rect[2] <= right + 1.0), "the rows keep the width: {rows:?}");
    assert!(rows.iter().any(|l| l.rect[1] < next[0][1]), "the new rows go past what was below: {rows:?}");
    assert_eq!(line_rects(&s, id, "next paragraph"), next);
}

/// One row grows to the right up to the margin, then wraps; a word wider than that stays whole.
#[test]
fn a_longer_replacement_grows_one_row_to_the_margin() {
    let (mut s, id) = open(pdf("BT /F1 12 Tf 72 700 Td (short) Tj ET"));
    replace_all(&mut s, id, &query("short", "a line far wider than the word it took the place of"));
    assert_eq!(s.get(id).unwrap().text_lines(0).len(), 1);
    let (mut s, id) = open(pdf("BT /F1 12 Tf 72 700 Td (short) Tj ET"));
    replace_all(&mut s, id, &query("short", &"many words in a row ".repeat(8)));
    let lines = s.get(id).unwrap().text_lines(0);
    assert!(lines.len() > 1 && lines.iter().all(|l| l.rect[2] <= 595.0 - 36.0 + 1.0), "{lines:?}");
    let (mut s, id) = open(pdf("BT /F1 12 Tf 72 700 Td (short) Tj ET"));
    replace_all(&mut s, id, &query("short", &"w".repeat(80)));
    let lines = s.get(id).unwrap().text_lines(0);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].rect[2] > 595.0, "the word runs past the page: {lines:?}");
}

/// A match can run over a row's end hyphen, typed as the find bar reads it (the break is a space);
/// Replace all then folds case, joins the rows and counts what the find bar counts. The word read
/// unhyphenated is also a find-bar match, but no paragraph's text holds it: left alone and counted.
#[test]
fn case_insensitive_replace_all_takes_a_match_across_a_hyphenated_row_end() {
    let content = "BT /F1 12 Tf 14 TL 72 700 Td (An EXAM-) Tj T* (PLE and an Exam-) Tj T* (ple to end) Tj ET";
    let (mut s, id) = open(pdf(content));
    assert_eq!(texts(&s, id, 0), ["An EXAMPLE and an Example to end"]);
    let text = page_text(&s, id, 0);
    let whole = query("example", "sample");
    let found = BTreeMap::from([(0, text.find_opts(&whole.find, false, false).len())]);
    assert_eq!(found[&0], 2);
    let none = s.get(id).unwrap().plan_replace(&[0], &whole, Which::All, Some(&found)).unwrap();
    assert_eq!((none.replaced, none.skipped_total(), none.edit.is_none()), (0, 2, true));
    let q = query("exam- ple", "sample");
    let found = BTreeMap::from([(0, text.find_opts(&q.find, false, false).len())]);
    assert_eq!(found[&0], 2);
    let plan = s.get(id).unwrap().plan_replace(&[0], &q, Which::All, Some(&found)).unwrap();
    assert_eq!((plan.replaced, plan.skipped_total()), (2, 0));
    s.apply(id, plan.edit.unwrap()).unwrap();
    assert_eq!(texts(&s, id, 0), ["An sample and an sample to end"]);
}

#[test]
fn invisible_text_over_a_scan_is_skipped() {
    let (mut s, id) = open(pdf("BT 3 Tr /F1 12 Tf 72 700 Td (recognized words) Tj ET"));
    let plan = replace_all(&mut s, id, &query("words", "text"));
    assert_eq!(plan.skipped, BTreeMap::from([(Skip::ScannedText, 1)]));
    assert!(plan.edit.is_none());
}

/// What the find bar counts on a page beyond what the paragraphs hold is reported as not
/// editable text (a form field, a comment), so the counts add up to what was highlighted.
#[test]
fn matches_the_paragraphs_dont_hold_are_counted_as_not_editable() {
    let (s, id) = open(pdf("BT /F1 12 Tf 72 700 Td (one cat) Tj ET"));
    let found = BTreeMap::from([(0, 3)]);
    let plan = s.get(id).unwrap().plan_replace(&[0], &query("cat", "dog"), Which::All, Some(&found)).unwrap();
    assert_eq!(plan.replaced, 1);
    assert_eq!(plan.skipped, BTreeMap::from([(Skip::NotEditable, 2)]));
}

/// Replace (one match): the highlighted one, found by where the find bar's glyphs are, even when
/// the paragraph holds the word twice.
#[test]
fn the_highlighted_match_is_the_one_replaced() {
    let (mut s, id) = open(pdf("BT /F1 12 Tf 72 700 Td (cat and cat) Tj ET BT /F1 12 Tf 72 500 Td (no match) Tj ET"));
    let text = page_text(&s, id, 0);
    let found = text.find("cat");
    assert_eq!(found.len(), 2);
    let centre = |g: usize| {
        let r = text.glyphs[g].rect;
        [(r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0]
    };
    let second = &found[1];
    let which = Which::At { page: 0, start: centre(second.start), end: centre(second.end - 1) };
    let plan = s.get(id).unwrap().plan_replace(&[0], &query("cat", "dog"), which, None).unwrap();
    assert_eq!(plan.replaced, 1);
    assert!(matches!(&plan.edit, Some(Edit::EditTextBlock { block: 0, .. })), "one paragraph edit, not a batch: {:?}", plan.edit);
    s.apply(id, plan.edit.unwrap()).unwrap();
    assert_eq!(texts(&s, id, 0), ["cat and dog", "no match"]);
    // A point in no paragraph is not editable text.
    let away = Which::At { page: 0, start: [500.0, 20.0], end: [510.0, 20.0] };
    let plan = s.get(id).unwrap().plan_replace(&[0], &query("cat", "dog"), away, None).unwrap();
    assert_eq!(plan.skipped, BTreeMap::from([(Skip::NotEditable, 1)]));
}

#[test]
fn first_replaces_only_the_first_match_in_document_order() {
    let (mut s, id) = open(crate::tests::fixture(2));
    let q = query("page", "Sheet");
    let texts_of: Vec<_> = (0..2).map(|p| page_text(&s, id, p)).collect();
    let which = Which::first_in(texts_of.iter().enumerate(), &q).expect("the find bar has a match");
    assert!(matches!(which, Which::At { page: 0, .. }));
    let plan = s.get(id).unwrap().plan_replace(&[0, 1], &q, which, None).unwrap();
    assert_eq!((plan.replaced, plan.pages.clone()), (1, vec![0]));
    s.apply(id, plan.edit.unwrap()).unwrap();
    assert_eq!((texts(&s, id, 0), texts(&s, id, 1)), (vec!["Sheet 1".to_string()], vec!["Page 2".to_string()]));
    assert!(Which::first_in(texts_of.iter().enumerate(), &query("absent", "x")).is_none());
}

/// The first match is the find bar's, not the first one a paragraph holds: when it is a match no
/// paragraph holds (a word split by a line-end hyphen), it is reported and nothing is replaced,
/// not the later paragraph that has the word whole.
#[test]
fn first_does_not_skip_a_match_no_paragraph_holds_for_a_later_one() {
    let content = "BT /F1 12 Tf 14 TL 72 700 Td (An EXAM-) Tj T* (PLE to end) Tj ET BT /F1 12 Tf 72 400 Td (a second example) Tj ET";
    let (s, id) = open(pdf(content));
    let q = query("example", "sample");
    let text = page_text(&s, id, 0);
    assert_eq!(text.find_opts("example", false, false).len(), 2, "the find bar has both");
    let which = Which::first_in([(0, &text)], &q).unwrap();
    let plan = s.get(id).unwrap().plan_replace(&[0], &q, which, None).unwrap();
    assert_eq!((plan.replaced, plan.edit.is_none()), (0, true), "{plan:?}");
    assert_eq!(plan.skipped, BTreeMap::from([(Skip::NotEditable, 1)]));
    assert_eq!(texts(&s, id, 0).last().unwrap(), "a second example");
}

#[test]
fn nothing_to_find_is_refused() {
    let (s, id) = open(crate::tests::fixture(1));
    assert!(matches!(s.get(id).unwrap().plan_replace(&[0], &query("  ", "x"), Which::All, None), Err(EditError::Invalid(_))));
}

/// A Chrome-style subset (Type0 / Identity-H over a subset-free copy of Inter, CID = glyph id)
/// drawing "Poland" and "Germany and more". Its ToUnicode has an entry for `z` with no glyph, so
/// "Graz" needs a letter the font lacks while "Germany" is drawn from letters it has.
///
/// The glyph ids and widths below are those of `assets/fonts/Inter-Regular.ttf` with SHA-256
/// 40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82 (the hash `ATTRIBUTION.toml`
/// records); a different Inter build needs them looked up again.
fn skia_subset_pdf() -> Vec<u8> {
    let program = include_bytes!("../../../../assets/fonts/Inter-Regular.ttf");
    let content = "BT /F3 12 Tf 72 700 Td <0177031602E001FB03050255> Tj ET \
         BT /F3 12 Tf 72 650 Td <008D0266035402FE01FB030503E606F101FB0305025506F102FE031603540266> Tj ET";
    let unicode = "1 begincodespacerange <0000> <FFFF> endcodespacerange 13 beginbfchar <0177> <0050> <0316> <006F> <02E0> <006C> \
          <01FB> <0061> <0305> <006E> <0255> <0064> <008D> <0047> <0266> <0065> <0354> <0072> <02FE> <006D> <03E6> <0079> \
          <06F1> <0020> <1000> <007A> endbfchar";
    let widths = "[141 [746] 375 [639] 597 [612] 614 [583] 736 [242] 766 [876] 773 [591] 790 [600] 852 [376] 1777 [281]]";
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    let mut put = |out: &mut Vec<u8>, n: usize, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    let stream = |dict: &str, data: &[u8]| {
        let mut v = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
        v.extend_from_slice(data);
        v.extend_from_slice(b"\nendstream");
        v
    };
    put(&mut out, 1, b"<< /Type /Catalog /Pages 2 0 R >>");
    put(&mut out, 2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    put(&mut out, 3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Contents 4 0 R /Resources << /Font << /F3 5 0 R >> >> >>");
    put(&mut out, 4, &stream("", content.as_bytes()));
    put(
        &mut out,
        5,
        b"<< /Type /Font /Subtype /Type0 /BaseFont /BAAAAA+SampleSans /Encoding /Identity-H /DescendantFonts [6 0 R] /ToUnicode 7 0 R >>",
    );
    put(
        &mut out,
        6,
        format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /BAAAAA+SampleSans /CIDToGIDMap /Identity /DW 562 /W {} /FontDescriptor 8 0 R >>",
            widths
        )
        .as_bytes(),
    );
    put(&mut out, 7, &stream("", unicode.as_bytes()));
    put(&mut out, 8, b"<< /Type /FontDescriptor /FontName /BAAAAA+SampleSans /Flags 32 /FontFile2 9 0 R >>");
    put(&mut out, 9, &stream(&format!("/Length1 {}", program.len()), program));
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    out
}

/// Off by default: a name the subset can't spell is left alone and counted. With
/// `substitute_font` the match is replaced in the substitute font, counted, named, one undo step.
#[test]
fn a_substitute_font_replaces_where_the_subset_lacks_a_letter_when_asked() {
    let original = skia_subset_pdf();
    let (mut s, id) = open(original.clone());
    assert_eq!(texts(&s, id, 0), ["Poland", "Germany and more"]);
    let before = s.get(id).unwrap().bytes.clone();

    // Default: skipped, nothing changes.
    let plan = replace_all(&mut s, id, &query("Poland", "Graz"));
    assert_eq!((plan.replaced, plan.substituted), (0, 0));
    assert_eq!(plan.skipped, BTreeMap::from([(Skip::MissingGlyphs, 1)]));
    assert!(plan.edit.is_none() && plan.substitute_fonts.is_empty());
    assert_eq!(texts(&s, id, 0), ["Poland", "Germany and more"]);

    // Asked for: replaced in another font, and said so.
    let q = Replace { substitute_font: true, ..query("Poland", "Graz") };
    let plan = replace_all(&mut s, id, &q);
    assert_eq!((plan.replaced, plan.substituted, plan.skipped_total()), (1, 1, 0), "{plan:?}");
    assert_eq!(plan.substitute_fonts.len(), 1, "{plan:?}");
    assert!(!plan.substitute_fonts[0].is_empty());
    assert_eq!(texts(&s, id, 0), ["Graz", "Germany and more"]);
    assert!(!plan.substitute_fonts[0].ends_with("Type3"), "{plan:?}");
    assert_eq!(plan.substituted_paragraphs, 1);
    assert_eq!(page_text(&s, id, 0).plain_text().split_whitespace().next(), Some("Graz"), "the page text reads Graz");
    assert_eq!(s.get(id).unwrap().can_undo(), Some("Replace all"));
    s.undo(id).unwrap();
    assert_eq!(texts(&s, id, 0), ["Poland", "Germany and more"], "one undo restores the paragraph");
    assert_eq!(s.get(id).unwrap().can_undo(), None);
    assert_eq!(s.get(id).unwrap().bytes, before);
}

/// Edit text on a paragraph whose subset font lacks a letter redraws it in a stand-in font and the
/// session says so (`EditNotice::SubstituteFont`, named letters and font), never silently; text
/// the subset can draw says nothing; one undo clears the notice's cause.
#[test]
fn editing_a_paragraph_in_a_subset_says_which_letters_forced_a_substitute() {
    let (mut s, id) = open(skia_subset_pdf());
    let edit = |text: &str| crate::Edit::EditTextBlock { page: 0, block: 0, text: text.into(), style: Default::default() };
    s.apply(id, edit("Germany")).unwrap();
    assert!(s.get(id).unwrap().edit_notices.is_empty(), "{:?}", s.get(id).unwrap().edit_notices);
    s.undo(id).unwrap();
    s.apply(id, edit("Graz")).unwrap();
    let notices = s.get(id).unwrap().edit_notices.clone();
    let [crate::EditNotice::SubstituteFont { fonts, missing }] = notices.as_slice() else { panic!("{notices:?}") };
    assert_eq!(fonts.len(), 1, "{notices:?}");
    assert!(!fonts[0].is_empty() && !fonts[0].ends_with("Type3"), "{notices:?}");
    assert!(!missing.is_empty() && missing.iter().all(|c| "Graz".contains(*c)), "{notices:?}");
    let text = notices[0].to_string();
    assert!(text.contains("U+") && text.contains(&fonts[0]) && text.contains("Undo"), "{text}");
    // A weight change alone names no lost letter: no notice, and not refused with substitution off.
    s.undo(id).unwrap();
    for refuse_substitute in [false, true] {
        let style = pdfcraft_edit::BlockStyle { bold: Some(true), refuse_substitute, ..Default::default() };
        s.apply(id, crate::Edit::EditTextBlock { page: 0, block: 0, text: "Poland".into(), style }).unwrap();
        assert!(s.get(id).unwrap().edit_notices.is_empty(), "{:?}", s.get(id).unwrap().edit_notices);
        s.undo(id).unwrap();
    }
    // Refused with substitution off: the paragraph is as it was.
    let strict = pdfcraft_edit::BlockStyle { refuse_substitute: true, ..Default::default() };
    let refused = s.apply(id, crate::Edit::EditTextBlock { page: 0, block: 0, text: "Graz".into(), style: strict });
    assert!(refused.is_err_and(|e| e.to_string().contains("substitute fonts are off")));
    assert_eq!(texts(&s, id, 0), ["Poland", "Germany and more"]);
}

/// Letters the subset has are replaced in its own font in both modes, and nothing is substituted.
#[test]
fn a_replacement_the_subset_can_draw_never_uses_a_substitute_font() {
    for substitute_font in [false, true] {
        let (mut s, id) = open(skia_subset_pdf());
        let q = Replace { substitute_font, ..query("Poland", "Germany") };
        let plan = replace_all(&mut s, id, &q);
        assert_eq!((plan.replaced, plan.substituted, plan.skipped_total()), (1, 0, 0), "substitute_font {substitute_font}");
        assert!(plan.substitute_fonts.is_empty());
        assert_eq!(texts(&s, id, 0), ["Germany", "Germany and more"]);
        let lines = s.get(id).unwrap().text_lines(0);
        assert!(lines.iter().filter(|l| l.text == "Germany").all(|l| l.font == "F3"), "the subset keeps its font: {lines:?}");
    }
}

/// The paragraph of several runs (styled rewrite) takes the option too.
#[test]
fn a_substitute_font_is_used_in_a_paragraph_of_several_runs_when_asked() {
    let (mut s, id) = open(pdf("BT /F3 12 Tf 72 700 Td (ab ) Tj /F1 12 Tf (and Helvetica) Tj ET"));
    let q = Replace { substitute_font: true, ..query("ab", "cb") };
    let plan = replace_all(&mut s, id, &q);
    assert_eq!((plan.replaced, plan.substituted, plan.skipped_total()), (1, 1, 0), "{plan:?}");
    assert_eq!(texts(&s, id, 0), ["cb and Helvetica"]);
}

/// The substitute redraws the whole paragraph, not only the match: a word that was not replaced
/// leaves the subset's font too.
#[test]
fn the_whole_paragraph_is_redrawn_in_the_substitute_font() {
    let (mut s, id) = open(skia_subset_pdf());
    let q = Replace { substitute_font: true, ..query("Germany", "Graz") };
    let plan = replace_all(&mut s, id, &q);
    assert_eq!((plan.replaced, plan.substituted, plan.substituted_paragraphs), (1, 1, 1), "{plan:?}");
    assert_eq!(texts(&s, id, 0), ["Poland", "Graz and more"]);
    let lines = s.get(id).unwrap().text_lines(0);
    let more = lines.iter().find(|l| l.text.contains("more")).expect("the untouched words");
    assert_ne!(more.font, "F3", "the untouched words are drawn in the substitute too: {lines:?}");
    let poland = lines.iter().find(|l| l.text.contains("Poland")).expect("the other paragraph");
    assert_eq!(poland.font, "F3", "another paragraph keeps its font");
}

/// Two pages, each with a paragraph the subset can't spell: Replace all counts both, names the
/// font once and is one undo step; Which::At takes one only.
#[test]
fn substituted_paragraphs_on_two_pages_are_counted_and_undone_together() {
    let content = ["BT /F3 12 Tf 72 700 Td (ab ba) Tj ET", "BT /F3 12 Tf 72 700 Td (ba ab) Tj ET"];
    let objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R /Resources << /Font << /F3 7 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{}\nendstream", content[0].len(), content[0]),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 6 0 R /Resources << /Font << /F3 7 0 R >> >> >>".into(),
        format!("<< /Length {} >>\nstream\n{}\nendstream", content[1].len(), content[1]),
        "<< /Type /Font /Subtype /TrueType /BaseFont /ABCDEF+NoSuchFace /FirstChar 97 /LastChar 99 /Widths [500 520 0] /Encoding /WinAnsiEncoding >>"
            .into(),
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
    let q = Replace { substitute_font: true, ..query("ab", "cb") };

    let (mut s, id) = open(out.clone());
    let before = s.get(id).unwrap().bytes.clone();
    let plan = replace_all(&mut s, id, &q);
    assert_eq!((plan.replaced, plan.substituted, plan.substituted_paragraphs, plan.skipped_total()), (2, 2, 2, 0), "{plan:?}");
    assert_eq!(plan.substitute_fonts.len(), 1, "one font, named once: {plan:?}");
    assert_eq!(plan.pages, vec![0, 1]);
    assert_eq!((texts(&s, id, 0), texts(&s, id, 1)), (vec!["cb ba".to_string()], vec!["ba cb".to_string()]));
    assert_eq!(s.get(id).unwrap().can_undo(), Some("Replace all"));
    s.undo(id).unwrap();
    assert_eq!(s.get(id).unwrap().can_undo(), None, "one undo step");
    assert_eq!(s.get(id).unwrap().bytes, before);

    // The match on page 2 only.
    let text = page_text(&s, id, 1);
    let which = Which::first_in([(1, &text)], &q).expect("a match on page 2");
    let plan = s.get(id).unwrap().plan_replace(&[0, 1], &q, which, None).unwrap();
    assert_eq!((plan.replaced, plan.substituted, plan.substituted_paragraphs, plan.pages.clone()), (1, 1, 1, vec![1]), "{plan:?}");
    s.apply(id, plan.edit.clone().expect("an edit")).unwrap();
    assert_eq!((texts(&s, id, 0), texts(&s, id, 1)), (vec!["ab ba".to_string()], vec!["ba cb".to_string()]));
    assert_eq!(s.get(id).unwrap().can_undo(), Some("Edit text"));
}

const BODY: [&str; 4] = [
    "The committee reviewed the proposal and agreed that the schedule",
    "for the new building would depend on the weather during the autumn",
    "months, so the contractor was asked to prepare a second plan that",
    "avoids the lowest field. Work starts in spring.",
];

/// A paragraph of `rows` set in 10 pt Helvetica at 12 pt leading from the left margin 72: every row
/// but the last is spread to `measure` points with word spacing, as a justified paragraph from
/// another producer is, so none of its rows is marked as a wrap or a break.
fn body_paragraph(rows: &[&str], measure: f64) -> String {
    let mut out = String::new();
    for (i, row) in rows.iter().enumerate() {
        let (s, id) = open(pdf(&format!("BT /F1 10 Tf 72 700 Td ({row}) Tj ET")));
        let rect = s.get(id).unwrap().text_lines(0)[0].rect;
        let spaces = row.matches(' ').count().max(1) as f64;
        let tw = if i + 1 < rows.len() { (measure - (rect[2] - rect[0])) / spaces } else { 0.0 };
        let y = 700.0 - 12.0 * i as f64;
        out.push_str(&format!("BT /F1 10 Tf {tw:.4} Tw 72 {y} Td ({row}) Tj ET\n"));
    }
    out
}

/// Each row of the page: its text and width.
fn rows_of(s: &Session, id: DocId) -> Vec<(String, f64)> {
    let mut lines = s.get(id).unwrap().text_lines(0);
    lines.sort_by(|a, b| b.rect[1].total_cmp(&a.rect[1]));
    lines.into_iter().map(|l| (l.text.trim().to_owned(), l.rect[2] - l.rect[0])).collect()
}

/// A longer replacement in a wrapped paragraph reflows it as one text: no row but the last is left
/// short, and the words keep their order (the rows used to keep their starts, so the word that
/// overflowed row 2 stood alone on a row of its own above the unchanged row 3).
#[test]
fn a_longer_replacement_reflows_a_wrapped_paragraph_without_an_orphan_row() {
    let (s0, id0) = open(pdf(&body_paragraph(&BODY, 330.0)));
    let before = s0.get(id0).unwrap().text_blocks(0);
    assert_eq!(before.len(), 1, "one paragraph");
    assert!(before[0].joins.iter().all(Option::is_some), "the rows are wraps: {:?}", before[0].joins);
    let (mut s, id) = open(pdf(&body_paragraph(&BODY, 330.0)));
    let plan = replace_all(&mut s, id, &query("weather", "unpredictable and often very wet weather conditions"));
    assert_eq!((plan.replaced, plan.skipped_total()), (1, 0));
    let rows = rows_of(&s, id);
    let widest = rows.iter().map(|r| r.1).fold(0.0, f64::max);
    assert!(widest <= 330.0 + 6.0, "the rows keep the paragraph's width: {rows:?}");
    for (text, width) in &rows[..rows.len() - 1] {
        assert!(*width >= 330.0 * 0.8, "orphan row {text:?} ({width} pt) in {rows:?}");
    }
    let text = texts(&s, id, 0).concat();
    let words: Vec<&str> = text.split_whitespace().collect();
    let expected = BODY.join(" ").replace("weather", "unpredictable and often very wet weather conditions");
    assert_eq!(words, expected.split_whitespace().collect::<Vec<_>>());
}

/// Rows that are lines of their own (an address, a poem) stay lines of their own, even when the
/// widest of them reaches the paragraph's right edge: "London" must not be pulled up onto the
/// street's row, which a row that "reaches the measure" would otherwise suggest.
#[test]
fn a_longer_replacement_keeps_the_rows_of_short_lines() {
    let content = "BT /F1 10 Tf 12 TL 72 700 Td (12 Analytical Street) Tj T* (London) Tj T* (United Kingdom) Tj ET";
    let (mut s, id) = open(pdf(content));
    let plan = replace_all(&mut s, id, &query("Analytical", "Analytical Engine Works"));
    assert_eq!(plan.replaced, 1);
    let rows: Vec<String> = rows_of(&s, id).into_iter().map(|r| r.0).collect();
    assert!(rows.iter().any(|r| r == "London"), "London lost its row: {rows:?}");
    assert_eq!(rows.last().map(String::as_str), Some("United Kingdom"), "{rows:?}");
    assert_eq!(plan.grown, 1, "the added rows are still reported");
}

/// Hyphens the paragraph really has, at the end of a full row, are not outside text to delete: a
/// reflow would join "well-" and "known" as "wellknown".
#[test]
fn a_hyphen_at_a_row_end_survives_a_replacement_elsewhere() {
    let rows = [
        "The committee reviewed the proposal and agreed that the schedule",
        "for the new building would depend on the weather and on a well-",
        "known author who had promised to prepare a second plan that",
        "avoids the lowest field. Work starts in spring.",
    ];
    let (mut s, id) = open(pdf(&body_paragraph(&rows, 330.0)));
    let plan = replace_all(&mut s, id, &query("committee", "committee of the whole board and all of its many members"));
    assert_eq!((plan.replaced, plan.skipped_total()), (1, 0));
    // (The paragraph's `text` joins a hyphenated row end without it; the rows show what is drawn.)
    let drawn: Vec<String> = rows_of(&s, id).into_iter().map(|r| r.0).collect();
    assert!(drawn.concat().contains("well-") && !drawn.join(" ").contains("wellknown"), "{drawn:?}");
    assert!(drawn.iter().any(|r| r.ends_with("well-")), "the hyphen still ends its row: {drawn:?}");
}

/// A reflow must not bring in a substitute font that the row-by-row rewrite avoided. (Today the
/// spaces a reflow puts back are never what a subset font lacks, since `source_encodes` splits
/// them out, so this pins the invariant rather than a reachable flip; the guard in
/// `plan_replace` is what keeps it if that changes.)
#[test]
fn a_reflow_that_would_need_a_substitute_font_is_not_taken() {
    let row = |s: &str| format!("({s}) Tj");
    let content = format!(
        "BT /F3 12 Tf 14 TL 72 700 Td {} T* {} T* {} T* {} ET",
        row("ababababababababababab"),
        row("ababababbababababababa"),
        row("ababababababababababab"),
        row("abab")
    );
    let (mut s, id) = open(pdf(&content));
    assert_eq!(s.get(id).unwrap().text_blocks(0)[0].joins, [Some(1), Some(1), Some(1)], "the rows are wraps");
    let with = "ab".repeat(24);
    let plan = replace_all(&mut s, id, &query("bb", &with));
    assert_eq!((plan.replaced, plan.substituted, plan.skipped_total()), (1, 0, 0), "{plan:?}");
}

/// A paragraph that gained rows says so, and by how much, since what is below it did not move.
#[test]
fn a_paragraph_that_gains_rows_is_reported_as_grown() {
    let (mut s, id) = open(pdf(&body_paragraph(&BODY, 330.0)));
    let plan = replace_all(&mut s, id, &query("weather", &"very wet and windy ".repeat(12)));
    assert_eq!(plan.replaced, 1);
    assert_eq!(plan.grown, 1);
    assert!(plan.grown_rows >= 2, "{plan:?}");
    assert!(plan.grown_by >= 20.0 && plan.grown_by.is_finite(), "{plan:?}");
    // A same-size replacement does not grow it.
    let (mut s, id) = open(pdf(&body_paragraph(&BODY, 330.0)));
    let plan = replace_all(&mut s, id, &query("weather", "climate"));
    assert_eq!((plan.replaced, plan.grown, plan.grown_rows), (1, 0, 0));
    assert_eq!(plan.grown_by, 0.0);
}

/// `text` broken greedily into rows no wider than `measure` points in 10 pt Helvetica, the way a
/// word processor sets ragged-right text, and drawn with no word spacing and no marks: each row
/// ends because the next word would not have fitted, and none reaches a common edge but the longest.
fn ragged_paragraph(text: &str, measure: f64) -> String {
    let width = |t: &str| {
        let (s, id) = open(pdf(&format!("BT /F1 10 Tf 72 700 Td ({t}) Tj ET")));
        let rect = s.get(id).unwrap().text_lines(0)[0].rect;
        rect[2] - rect[0]
    };
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
    lines_page(&rows.iter().map(String::as_str).collect::<Vec<_>>())
}

/// One unmarked 10 pt row per entry, at 12 pt leading from the left margin 72.
fn lines_page(rows: &[&str]) -> String {
    rows.iter().enumerate().map(|(i, row)| format!("BT /F1 10 Tf 72 {} Td ({row}) Tj ET\n", 700.0 - 12.0 * i as f64)).collect()
}

const RAGGED: &str = "The committee reviewed the proposal and agreed that the schedule for the new building would depend on \
the weather during the autumn months so the contractor was asked to prepare a second plan that avoids the lowest field";

/// Ragged-right text from a word processor (rows of different lengths, each ended because the next
/// word would not have fitted, none marked) reflows around a longer replacement like any other
/// paragraph: no short row is left in the middle of it.
#[test]
fn a_longer_replacement_reflows_ragged_right_text_without_an_orphan_row() {
    let content = ragged_paragraph(RAGGED, 250.0);
    let (s0, id0) = open(pdf(&content));
    let before = s0.get(id0).unwrap().text_blocks(0);
    assert_eq!(before.len(), 1, "one paragraph");
    assert!(before[0].rows.len() >= 5, "{:?}", before[0].rows);
    assert!(before[0].joins.iter().all(Option::is_some), "the rows are wraps: {:?}", before[0].joins);
    let (mut s, id) = open(pdf(&content));
    let plan = replace_all(&mut s, id, &query("proposal", "proposal and its long, detailed budget"));
    assert_eq!((plan.replaced, plan.skipped_total()), (1, 0));
    let rows = rows_of(&s, id);
    let widest = rows.iter().map(|r| r.1).fold(0.0, f64::max);
    assert!(widest <= 250.0 + 6.0, "the rows keep the paragraph's width: {rows:?}");
    for (text, width) in &rows[..rows.len() - 1] {
        assert!(*width >= 250.0 * 0.8, "orphan row {text:?} ({width} pt) in {rows:?}");
    }
    let words = texts(&s, id, 0).concat();
    let expected = RAGGED.replace("proposal", "proposal and its long, detailed budget");
    assert_eq!(words.split_whitespace().collect::<Vec<_>>(), expected.split_whitespace().collect::<Vec<_>>());
}

/// Items and lines of verse that are shorter than the measure keep their rows, even though one of
/// them is long: the next word would have fitted on the others.
#[test]
fn a_longer_replacement_keeps_the_rows_of_a_list_and_of_a_poem() {
    let list = ["Apples", "Pears and plums", "Oranges, lemons and a great many other kinds of citrus fruit", "Figs"];
    let (mut s, id) = open(pdf(&lines_page(&list)));
    let plan = replace_all(&mut s, id, &query("Pears", "Green pears and ripe pears"));
    assert_eq!(plan.replaced, 1);
    let rows: Vec<String> = rows_of(&s, id).into_iter().map(|r| r.0).collect();
    assert_eq!(rows.first().map(String::as_str), Some("Apples"), "{rows:?}");
    assert_eq!(rows.last().map(String::as_str), Some("Figs"), "{rows:?}");
    assert!(rows.iter().any(|r| r.starts_with("Oranges, lemons and a great many")), "the long item keeps its row: {rows:?}");

    let poem = [
        "Shall I compare thee to a summer's day thou art more",
        "Lovely and more temperate rough winds do shake the darling",
        "Buds of May and summer's lease hath all too short a date",
        "Sometime too hot the eye of heaven shines and often is his",
    ];
    let (mut s, id) = open(pdf(&lines_page(&poem)));
    let plan = replace_all(&mut s, id, &query("Lovely", "Lovely and ever so gentle"));
    assert_eq!(plan.replaced, 1);
    let rows: Vec<String> = rows_of(&s, id).into_iter().map(|r| r.0).collect();
    for line in [poem[0], poem[2], poem[3]] {
        assert!(rows.iter().any(|r| r == line), "{line:?} lost its row: {rows:?}");
    }
}

/// A two-line address is two lines: the break after its widest row has no room left whatever
/// follows, which is no sign of a wrap, so a longer street name must not pull the city up.
#[test]
fn a_longer_replacement_keeps_the_rows_of_a_two_line_address() {
    let (s0, id0) = open(pdf(&lines_page(&["1600 Pennsylvania Avenue NW", "Washington, DC 20500"])));
    assert!(s0.get(id0).unwrap().text_blocks(0)[0].joins.iter().all(Option::is_none));
    let (mut s, id) = open(pdf(&lines_page(&["1600 Pennsylvania Avenue NW", "Washington, DC 20500"])));
    let plan = replace_all(&mut s, id, &query("Avenue", "Boulevard of the Americas"));
    assert_eq!(plan.replaced, 1);
    let rows: Vec<String> = rows_of(&s, id).into_iter().map(|r| r.0).collect();
    assert_eq!(rows.last().map(String::as_str), Some("Washington, DC 20500"), "{rows:?}");
}

/// The entries of a table of contents (dot leaders and page numbers) keep their own rows when a
/// longer replacement lands in one of them: the next entry's first word would not have fitted on
/// the row above, which is no sign of a wrap.
#[test]
fn a_longer_replacement_keeps_the_entries_of_a_table_of_contents() {
    let entries = [
        "Introduction and Overview of the Platform ............... 4",
        "Architecture ........................................... 7",
        "Deployment Considerations for Large Clusters ........ 9",
        "Networking ............................................. 12",
        "Self-Service File Restore and Recovery Options ..... 22",
        "Replication ......................................... 25",
    ];
    let (mut s, id) = open(pdf(&lines_page(&entries)));
    assert!(s.get(id).unwrap().text_blocks(0)[0].joins.iter().all(Option::is_none));
    let plan = replace_all(&mut s, id, &query("Networking", "Networking and the Cabling of Every Rack"));
    assert_eq!(plan.replaced, 1);
    let rows: Vec<String> = rows_of(&s, id).into_iter().map(|r| r.0).collect();
    for entry in [entries[0], entries[1], entries[2], entries[4], entries[5]] {
        assert!(rows.iter().any(|r| r == entry), "{entry:?} lost its row: {rows:?}");
    }
}
