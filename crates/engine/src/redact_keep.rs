//! Redacting part of a match: the letters or digits at its ends that stay readable (all but the last four digits of
//! a number, say). Shared by `redact_mark` and Find text and redact.

use std::ops::Range;

use pdfcraft_render::PageText;

/// The most letters or digits that may be left readable at one end of a match.
pub const KEEP_MAX: usize = 64;

/// What one glyph's text counts for when the ends of a match are kept readable: `None` for a
/// separator (text that is not empty and holds only whitespace or punctuation: it is free in the
/// count and is never left at the edge of a box), otherwise the number of letters and digits it
/// holds, at least 1. Empty text, U+FFFD, symbols and anything else unknown are content, never
/// free and never stripped, so a glyph that cannot be read is covered rather than guessed to be a
/// hyphen.
fn glyph_weight(text: &str) -> Option<usize> {
    let separator = |c: char| c.is_whitespace() || c.is_ascii_punctuation() || matches!(c, '\u{2010}'..='\u{2015}' | '\u{2212}');
    if !text.is_empty() && text.chars().all(separator) {
        return None;
    }
    Some(text.chars().filter(|c| c.is_alphanumeric()).count().max(1))
}

/// The glyphs of `range` left to cover when the first `first` and last `last` letters or digits of a
/// match stay readable: those ends are cut off (a glyph that holds more than one character, such as a
/// ligature, is cut only when all of it fits, so the cover errs on the side of covering) along with
/// any separator ([`glyph_weight`]) beside them, which is free in the count, so none begins or ends the box. `None` when nothing is left
/// to cover. Cost is bounded by the length of `range`.
pub fn trim_kept_ends(text: &PageText, range: Range<usize>, first: usize, last: usize) -> Option<Range<usize>> {
    let weight = |i: usize| text.glyphs.get(i).map_or(Some(1), |g| glyph_weight(&g.text));
    let (mut start, mut end) = (range.start.min(text.glyphs.len()), range.end.min(text.glyphs.len()));
    if first > 0 {
        let mut kept = 0usize;
        while start < end && weight(start).is_none_or(|w| kept.saturating_add(w) <= first) {
            kept = kept.saturating_add(weight(start).unwrap_or(0));
            start += 1;
        }
    }
    if last > 0 {
        let mut kept = 0usize;
        while end > start && weight(end - 1).is_none_or(|w| kept.saturating_add(w) <= last) {
            kept = kept.saturating_add(weight(end - 1).unwrap_or(0));
            end -= 1;
        }
    }
    (start < end).then_some(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One line of glyphs with the given texts, one glyph each.
    fn line(texts: &[&str]) -> PageText {
        let glyph = |t: &str| pdfcraft_render::TextGlyph { text: t.into(), rect: [0.0; 4], quad: [[0.0; 2]; 4], direction: [1.0, 0.0] };
        PageText {
            glyphs: texts.iter().map(|t| glyph(t)).collect(),
            line_of: vec![0; texts.len()],
            space_before: vec![false; texts.len()],
            truncated: false,
        }
    }

    fn trim(texts: &[&str], first: usize, last: usize) -> Option<Range<usize>> {
        trim_kept_ends(&line(texts), 0..texts.len(), first, last)
    }

    #[test]
    fn a_ligature_is_kept_only_when_all_of_it_fits() {
        // "ffi" is three characters in one glyph: at the end with keep_last 2 it does not fit, so it
        // is covered; with 3 it is kept.
        assert_eq!(trim(&["1", "2", "ffi"], 0, 2), Some(0..3));
        assert_eq!(trim(&["1", "2", "ffi"], 0, 3), Some(0..2));
        assert_eq!(trim(&["ffi", "1", "2"], 2, 0), Some(0..3));
        assert_eq!(trim(&["ffi", "1", "2"], 3, 0), Some(1..3));
    }

    #[test]
    fn a_separator_does_not_begin_or_end_the_box_and_costs_nothing() {
        // 1 2 3 - 4 5 6 7: keeping the last four leaves "-" out of the box.
        let t = ["1", "2", "3", "-", "4", "5", "6", "7"];
        assert_eq!(trim(&t, 0, 4), Some(0..3));
        assert_eq!(trim(&t, 3, 0), Some(4..8));
        // A space or no-break space at the edge goes too.
        assert_eq!(trim(&["1", "2", " ", "3", "4"], 0, 2), Some(0..2));
        assert_eq!(trim(&["1", "2", "\u{a0}", "3", "4"], 2, 0), Some(3..5));
    }

    #[test]
    fn overlapping_ends_leave_nothing_and_never_cross() {
        assert_eq!(trim(&["1", "2", "3", "4"], 3, 3), None);
        assert_eq!(trim(&["1", "2", "3", "4"], 2, 2), None);
        assert_eq!(trim(&["1", "2", "3", "4", "5"], 2, 2), Some(2..3));
        assert_eq!(trim(&["1", "-", "2"], 1, 1), None);
        // A range reaching past the glyphs is cut to them, not indexed.
        assert_eq!(trim_kept_ends(&line(&["1", "2", "3"]), 1..99, 0, 1), Some(1..2));
    }

    #[test]
    fn a_glyph_that_cannot_be_read_is_covered_not_taken_for_a_separator() {
        // Empty text, U+FFFD and a symbol each count as one character kept and are never stripped
        // from the edge of the box.
        assert_eq!(trim(&["1", "2", "3", ""], 0, 1), Some(0..3));
        assert_eq!(trim(&["1", "2", "3", ""], 0, 0), Some(0..4));
        assert_eq!(trim(&["", "1", "2", "3"], 1, 0), Some(1..4));
        assert_eq!(trim(&["1", "2", "\u{fffd}", "3"], 0, 1), Some(0..3));
        assert_eq!(trim(&["1", "2", "3", "\u{fffd}"], 0, 1), Some(0..3));
        assert_eq!(trim(&["\u{fffd}", "1", "2"], 1, 0), Some(1..3));
        // An unreadable glyph beside a separator at the edge: the separator goes, the glyph stays covered.
        assert_eq!(trim(&["1", "2", "", "-", "3"], 0, 1), Some(0..3));
    }
}
