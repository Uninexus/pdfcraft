//! Redaction tools: mark areas, text, patterns or whole pages for redaction, apply the marks
//! (removing what they cover for good) or clear them.

use pdfcraft_engine::{
    CustomRedactionCodeSet, Edit, HIDDEN, Hidden, NewAnnotation, REDACT_MAX_WORDS, REDACT_VALIDATORS, REDACTION_CODE_SETS, RedactPattern,
    RedactRegex, RedactValidator, RedactionCodeSet, Shape, Style, find_pattern, rect_quad, redact_word_list, redacted_file_name,
};
use pdfcraft_render::PageText;
use serde_json::{Value, json};
use std::ops::Range;

use crate::comments::parse_color;
use crate::{Args, Automation, Result, ToolError, failed};

/// The most letters or digits `keep_first` or `keep_last` may leave readable at one end of a match.
const KEEP_MAX: i64 = 64;

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
fn trim_kept_ends(text: &PageText, range: Range<usize>, first: usize, last: usize) -> Option<Range<usize>> {
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

/// User text echoed in an error: quoted, at most 64 characters.
fn shown(s: &str) -> String {
    let head: String = s.chars().take(64).collect();
    if head.len() < s.len() { format!("{head:?}…") } else { format!("{head:?}") }
}

/// The allowed codes for an error: the first 20, then how many more.
fn allowed_codes<S: AsRef<str>>(codes: &[S]) -> String {
    let head: Vec<&str> = codes.iter().take(20).map(AsRef::as_ref).collect();
    match codes.len().saturating_sub(20) {
        0 => head.join(", "),
        n => format!("{} and {n} more", head.join(", ")),
    }
}

impl Automation {
    /// The user-defined code set of `custom_code_set` ({id, name, codes}), validated; none when absent.
    fn custom_code_set(&self, a: &Args) -> Result<Option<CustomRedactionCodeSet>> {
        let Some(v) = a.get("custom_code_set") else { return Ok(None) };
        let wrong = || Args::wrong("custom_code_set", "an object with id, name and codes");
        let obj = v.as_object().ok_or_else(wrong)?;
        if let Some(extra) = obj.keys().find(|k| !matches!(k.as_str(), "id" | "name" | "codes")) {
            return Err(ToolError::InvalidArgs(format!("custom_code_set has an unknown key {}; allowed: id, name, codes", shown(extra))));
        }
        let field = |k: &str| match obj.get(k) {
            Some(Value::String(s)) => Ok(s.as_str()),
            Some(_) => Err(ToolError::InvalidArgs(format!("custom_code_set.{k} must be a string"))),
            None => Err(ToolError::InvalidArgs(format!("custom_code_set.{k} is missing"))),
        };
        let codes: Vec<&str> = match obj.get("codes") {
            Some(Value::Array(items)) => items
                .iter()
                .map(|x| x.as_str().ok_or_else(|| ToolError::InvalidArgs("custom_code_set.codes must be an array of strings".into())))
                .collect::<Result<_>>()?,
            Some(_) => return Err(ToolError::InvalidArgs("custom_code_set.codes must be an array of strings".into())),
            None => return Err(ToolError::InvalidArgs("custom_code_set.codes is missing".into())),
        };
        CustomRedactionCodeSet::new(field("id")?, field("name")?, &codes)
            .map(Some)
            .map_err(|e| ToolError::InvalidArgs(format!("custom_code_set.{e}")))
    }

    pub(crate) fn redact_mark(&mut self, a: &Args) -> Result<Value> {
        let doc = self.doc(a)?;
        let (id, n) = (doc.id, doc.info.pages.len());
        if a.get("page").is_some() && a.get("pages").is_some() {
            return Err(ToolError::InvalidArgs("pass page or pages, not both".into()));
        }
        if a.get("rect").is_some() && a.get("pages").is_some() {
            return Err(ToolError::InvalidArgs("rect requires page, not pages".into()));
        }
        let pages: Vec<usize> = if a.get("page").is_some() {
            vec![self.page(a)?]
        } else {
            match a.opt_ints("pages")? {
                Some(_) => self.pages(a, "pages")?,
                None => (0..n).collect(),
            }
        };
        let custom = self.custom_code_set(a)?;
        let overlay = match (a.opt_str("overlay")?, a.opt_str("code_set")?) {
            (Some(_), Some(_)) => return Err(ToolError::InvalidArgs("pass overlay or code_set with codes, not both".into())),
            (Some(_), None) if custom.is_some() => return Err(ToolError::InvalidArgs("pass overlay or custom_code_set with codes, not both".into())),
            // `codes` only means something inside a set; beside a literal overlay it used to be dropped without a word.
            (Some(_), None) if a.get("codes").is_some() => {
                return Err(ToolError::InvalidArgs(
                    "codes needs code_set or custom_code_set; overlay is literal text, so drop codes or use a code set".into(),
                ));
            }
            (Some(o), None) => o.to_string(),
            (None, id) if custom.is_some() || id.is_some() => {
                let picked = a.strs("codes")?;
                let (set_id, codes, result) = match (&custom, id) {
                    (Some(c), Some(id)) if id != c.id() => {
                        return Err(ToolError::InvalidArgs(format!(
                            "code_set {id:?} is not the custom_code_set id {:?}; omit code_set or pass that id",
                            c.id()
                        )));
                    }
                    (Some(c), _) => (c.id().to_owned(), allowed_codes(c.codes()), c.overlay(&picked)),
                    (None, id) => {
                        let id = id.unwrap_or_default();
                        let set = RedactionCodeSet::from_id(id).ok_or_else(|| {
                            let ids: Vec<_> = REDACTION_CODE_SETS.iter().map(|s| s.id).collect();
                            ToolError::InvalidArgs(format!(
                                "unknown code_set {id:?}; allowed: {} (or define one with custom_code_set)",
                                ids.join(", ")
                            ))
                        })?;
                        (set.id.to_owned(), allowed_codes(set.codes), set.overlay(&picked))
                    }
                };
                let allowed = || format!("allowed in {set_id}: {codes}");
                match result {
                    Ok(o) if !o.is_empty() => o,
                    Ok(_) => return Err(ToolError::InvalidArgs(format!("codes is empty; {}", allowed()))),
                    Err(bad) => return Err(ToolError::InvalidArgs(format!("unknown code {}; {}", shown(bad), allowed()))),
                }
            }
            (None, _) if a.get("codes").is_some() => return Err(ToolError::InvalidArgs("codes needs code_set or custom_code_set".into())),
            (None, _) => String::new(),
        };
        let author = self.author_arg(a)?;
        let mut style = Style::default_for(&Shape::Redact { quads: Vec::new(), overlay: String::new(), look: Default::default() });
        if let Some(c) = a.opt_str("fill")? {
            style.fill = Some(parse_color(c)?);
        }
        // (page, quads) per mark.
        let mut marks: Vec<(usize, Vec<[f64; 8]>)> = Vec::new();
        let chosen = [
            a.get("rect").is_some(),
            a.get("find").is_some(),
            a.get("words").is_some(),
            a.get("pattern").is_some(),
            a.get("regex").is_some(),
            a.opt_bool("whole_pages")?.unwrap_or(false),
        ];
        if chosen.iter().filter(|c| **c).count() != 1 {
            return Err(ToolError::InvalidArgs("pass exactly one of rect (with page), find, words, pattern, regex or whole_pages: true".into()));
        }
        let case_sensitive = a.opt_bool("case_sensitive")?.unwrap_or(false);
        let whole_words = a.opt_bool("whole_words")?.unwrap_or(false);
        if (case_sensitive || whole_words) && a.get("find").is_none() && a.get("words").is_none() {
            return Err(ToolError::InvalidArgs("case_sensitive and whole_words apply only to find or words".into()));
        }
        let keep = |key: &str| -> Result<usize> {
            match a.opt_int(key)? {
                None => Ok(0),
                Some(n) if (0..=KEEP_MAX).contains(&n) => Ok(usize::try_from(n).unwrap_or(0)),
                Some(n) => Err(ToolError::InvalidArgs(format!("{key} is {n}; it must be from 0 to {KEEP_MAX} (letters or digits left readable)"))),
            }
        };
        let (keep_first, keep_last) = (keep("keep_first")?, keep("keep_last")?);
        if (keep_first > 0 || keep_last > 0) && a.get("pattern").is_none() && a.get("regex").is_none() {
            return Err(ToolError::InvalidArgs("keep_first and keep_last apply only to pattern or regex".into()));
        }
        if a.get("validate").is_some() && a.get("regex").is_none() {
            return Err(ToolError::InvalidArgs("validate applies only to regex".into()));
        }
        // Per searched word or phrase: how many marks it made.
        let mut matched_words: Vec<(String, usize)> = Vec::new();
        // Matches left unmarked because keep_first and keep_last would leave all of them readable.
        let mut left_readable = 0usize;
        if a.get("rect").is_some() {
            let page = self.page(a)?;
            let info = &self.doc(a)?.info.pages[page];
            let r = a.nums::<4>("rect")?.ok_or_else(|| ToolError::InvalidArgs("rect must be 4 numbers".into()))?;
            let q = info.view_rect_to_quad([r[0] as f32, r[1] as f32, r[2] as f32, r[3] as f32]);
            marks.push((page, vec![q]));
        } else if a.opt_bool("whole_pages")?.unwrap_or(false) {
            let info = &self.doc(a)?.info;
            for &p in &pages {
                let c = info.pages[p].crop;
                marks.push((p, vec![rect_quad([c[0] as f64, c[1] as f64, c[2] as f64, c[3] as f64])]));
            }
        } else {
            let pattern = match a.opt_str("pattern")? {
                Some(p) => Some(RedactPattern::from_id(p).ok_or_else(|| ToolError::InvalidArgs(format!("unknown pattern {p:?}")))?),
                None => None,
            };
            let user_re = match a.opt_str("regex")? {
                Some(src) => {
                    let validator = match a.opt_str("validate")? {
                        Some(v) => Some(RedactValidator::from_id(v).ok_or_else(|| {
                            let ids: Vec<_> = REDACT_VALIDATORS.iter().map(|v| v.id()).collect();
                            ToolError::InvalidArgs(format!("unknown validate {v:?}; allowed: {}", ids.join(", ")))
                        })?),
                        None => None,
                    };
                    Some(RedactRegex::new(src, validator).map_err(|e| ToolError::InvalidArgs(e.to_string()))?)
                }
                None => None,
            };
            let needles: Vec<String> = match (a.opt_str("find")?, a.get("words")) {
                (Some(f), _) => vec![f.to_owned()],
                (None, Some(_)) => {
                    let raw = a.strs("words")?;
                    if raw.len() > REDACT_MAX_WORDS {
                        return Err(ToolError::InvalidArgs(format!("words has {} entries; at most {REDACT_MAX_WORDS} are allowed", raw.len())));
                    }
                    let list = redact_word_list(&raw.join("\n"));
                    if list.is_empty() {
                        return Err(ToolError::InvalidArgs("words must hold at least one non-empty word or phrase".into()));
                    }
                    list
                }
                (None, None) => Vec::new(),
            };
            matched_words = needles.iter().map(|w| (w.clone(), 0)).collect();
            let texts = self.page_texts(id, &pages)?;
            let info = self.doc(a)?.info.clone();
            for (&p, text) in pages.iter().zip(&texts) {
                let hits: Vec<(Option<usize>, _)> = match (&user_re, pattern) {
                    (Some(re), _) => {
                        // A page with too many matches fails the call: marking only the first ones
                        // would leave the rest readable after redact_apply.
                        let too_many = std::cell::Cell::new(false);
                        let found = text.find_with(|chars| {
                            re.find(chars).unwrap_or_else(|_| {
                                too_many.set(true);
                                Vec::new()
                            })
                        });
                        if too_many.get() {
                            return Err(ToolError::InvalidArgs(format!(
                                "regex has more than 10000 matches on page {}; nothing was marked. Narrow the expression so that every match can be marked",
                                p + 1
                            )));
                        }
                        found.into_iter().map(|h| (None, h)).collect()
                    }
                    (None, Some(pat)) => text.find_with(|chars| find_pattern(pat, chars)).into_iter().map(|h| (None, h)).collect(),
                    (None, None) => needles
                        .iter()
                        .enumerate()
                        .flat_map(|(i, w)| text.find_opts(w, case_sensitive, whole_words).into_iter().map(move |h| (Some(i), h)))
                        .collect(),
                };
                for (word, h) in hits {
                    let h = if keep_first > 0 || keep_last > 0 {
                        match trim_kept_ends(text, h, keep_first, keep_last) {
                            Some(h) => h,
                            None => {
                                left_readable += 1;
                                continue;
                            }
                        }
                    } else {
                        h
                    };
                    let quads: Vec<[f64; 8]> = text.line_rects(h).into_iter().map(|r| info.pages[p].view_rect_to_quad(r)).collect();
                    if !quads.is_empty() {
                        marks.push((p, quads));
                        if let Some(m) = word.and_then(|i| matched_words.get_mut(i)) {
                            m.1 += 1;
                        }
                    }
                }
            }
            if marks.is_empty() && left_readable > 0 {
                return Err(failed(format!(
                    "{left_readable} match(es) found, but keep_first and keep_last leave every one of them readable, so nothing was marked"
                )));
            }
            if marks.is_empty() {
                return Err(failed("nothing matched, so nothing was marked"));
            }
        }
        let count = marks.len();
        let edits: Vec<Edit> = marks
            .iter()
            .map(|(page, quads)| {
                let shape = Shape::Redact { quads: quads.clone(), overlay: overlay.clone(), look: Default::default() };
                Edit::AddAnnotation(NewAnnotation { page: *page, shape, style: style.clone(), contents: String::new(), author: author.clone() })
            })
            .collect();
        let edit = match <[Edit; 1]>::try_from(edits) {
            Ok([one]) => one,
            Err(edits) => Edit::Batch { label: "Mark for redaction".into(), edits },
        };
        let mut out = self.apply(a, edit)?;
        out["marked"] = json!(count);
        out["pages_marked"] = json!(marks.iter().map(|m| m.0 + 1).collect::<std::collections::BTreeSet<_>>());
        out["marks_pending"] = json!(self.doc(a)?.redaction_marks());
        if keep_first > 0 || keep_last > 0 {
            out["left_readable"] = json!(left_readable);
            if left_readable > 0 {
                out["warning"] = json!(format!(
                    "{left_readable} match(es) have no more letters or digits than keep_first and keep_last leave readable, so they were not marked and stay readable"
                ));
            }
        }
        if a.get("words").is_some() {
            out["matched_words"] = json!(matched_words.iter().map(|(w, n)| json!({ "word": w, "marked": n })).collect::<Vec<_>>());
        }
        Ok(out)
    }

    pub(crate) fn redact_apply(&mut self, a: &Args) -> Result<Value> {
        let pages = match a.opt_ints("pages")? {
            Some(_) => Some(self.pages(a, "pages")?),
            None => None,
        };
        let before = self.doc(a)?.redaction_marks();
        if before == 0 {
            return Err(failed("there are no redaction marks to apply (see redact_mark)"));
        }
        let mut out = self.apply(a, Edit::ApplyRedactions { pages })?;
        let after = self.doc(a)?.redaction_marks();
        out["applied"] = json!(before - after);
        out["marks_pending"] = json!(after);
        // Where the removed text is still held that applying leaves as it is.
        let left = self.doc(a)?.redaction_elsewhere().unwrap_or_default();
        out["still_holding"] = json!({
            "bookmarks": left.bookmarks,
            "attachments": left.attachments,
            "comments": left.comments,
            "xmp": left.xmp,
            "total": left.total(),
            "unsearched": left.unsearched,
        });
        // What became of a PDF/A or PDF/UA claim in the document XMP the scrub removed.
        let claim = |c: pdfcraft_engine::RedactionClaim| match c {
            pdfcraft_engine::RedactionClaim::None => "none",
            pdfcraft_engine::RedactionClaim::Kept => "kept",
            pdfcraft_engine::RedactionClaim::Lost => "lost",
        };
        let id = self.doc(a)?.redaction_identification().unwrap_or_default();
        out["identification"] = json!({ "pdfa": claim(id.pdfa), "pdfua": claim(id.pdfua) });
        // The name to save the redacted copy under, as Save does in the app.
        out["save_as"] = json!(redacted_file_name(&self.doc(a)?.name));
        Ok(out)
    }

    pub(crate) fn redact_clear(&mut self, a: &Args) -> Result<Value> {
        if self.doc(a)?.redaction_marks() == 0 {
            return Err(failed("there are no redaction marks"));
        }
        self.apply(a, Edit::ClearRedactions)
    }

    pub(crate) fn doc_hidden_info(&self, a: &Args) -> Result<Value> {
        let doc = self.doc(a)?;
        let items: Vec<Value> = doc.hidden_info().into_iter().map(|(h, n)| json!({ "category": h.id(), "label": h.label(), "count": n })).collect();
        let total: usize = doc.hidden_info().iter().map(|c| c.1).sum();
        Ok(json!({ "total": total, "categories": items }))
    }

    pub(crate) fn doc_audit_redactions(&self, a: &Args) -> Result<Value> {
        let d = self.doc(a)?;
        let findings = d.audit_redactions().map_err(ToolError::Failed)?;
        let items: Vec<Value> = findings
            .into_iter()
            .map(|f| {
                json!({
                    "page": f.page + 1,
                    "kind": f.kind.id(),
                    // Points from the top-left of the displayed page, like `comment_list` and `doc_info`.
                    "rect": crate::view_rect(&d.info, f.page, f.rect.map(|v| v as f32)),
                    "covered_text": f.covered_text,
                })
            })
            .collect();
        Ok(json!({ "findings": items, "count": items.len() }))
    }

    /// `doc_remove_hidden`: remove the listed categories of hidden information, or all of them
    /// (Sanitize Document) when none are listed, and report how many items the listed
    /// categories lost as `removed`.
    pub(crate) fn doc_remove_hidden(&mut self, a: &Args) -> Result<Value> {
        let which = match a.get("categories") {
            None => None,
            Some(_) => {
                let which = a
                    .strs("categories")?
                    .into_iter()
                    .map(|c| Hidden::from_id(c).ok_or_else(|| ToolError::InvalidArgs(format!("unknown category {c:?} (see doc_hidden_info)"))))
                    .collect::<Result<Vec<_>>>()?;
                if which.is_empty() {
                    return Err(ToolError::InvalidArgs("categories is empty".into()));
                }
                Some(which)
            }
        };
        // One scan, for this check and for what was removed. The removal refuses when it counts
        // every listed category as 0 (an AcroForm with no fields counts 1), with an error about
        // redaction marks; this reads the same scan, so it refuses exactly then. A document that
        // can't be edited has no scan: `apply` says why. Only the listed categories are counted,
        // here and after: the others would read every page's content and every object for
        // nothing (#5873).
        let listed = which.clone().unwrap_or_else(|| HIDDEN.to_vec());
        let info = self.doc(a)?.hidden_info_of(&listed);
        let edit = match which {
            None => Edit::Sanitize,
            Some(which) => {
                if !info.is_empty() && which.iter().all(|h| info.iter().find(|c| c.0 == *h).is_none_or(|c| c.1 == 0)) {
                    return Err(failed("nothing to remove: the listed categories hold nothing (see doc_hidden_info)"));
                }
                Edit::RemoveHidden { which }
            }
        };
        let before: usize = info.iter().map(|c| c.1).sum();
        let mut out = self.apply(a, edit)?;
        let after: usize = self.doc(a)?.hidden_info_of(&listed).iter().map(|c| c.1).sum();
        out["removed"] = json!(before.saturating_sub(after));
        Ok(out)
    }
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
