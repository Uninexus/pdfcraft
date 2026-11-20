//! Find and replace across every JavaScript of a document (Edit all JavaScripts). The scripts are
//! the ones [`javascripts`] lists; each one that holds the text gets its `/JS` rewritten as a
//! string (a stream that held it is left unreferenced from that action), so every other key of
//! its action and everything around it stays. Scripts are written where they physically are:
//! an action several places refer to changes for all of them, and each object that holds actions
//! is copied and rewritten once however many it holds. Nothing is run.
//!
//! Everything a replace costs is bounded before it is spent: the text to find and the text to put
//! in are capped, a script is read from the listing (at most [`MAX_SCRIPTS`] scripts and
//! [`MAX_TOTAL_CODE`] bytes of code), the length each rewritten script would have is worked out
//! with checked arithmetic before it is built, and the rewritten scripts together may hold at
//! most [`MAX_TOTAL_CODE`] bytes. Matching is plain text, left to right without overlaps; with
//! case ignored only the ASCII letters fold.

use std::collections::HashSet;

use pdfcraft_cos::Document;

use crate::FormError;
use crate::js_inventory::{MAX_SCRIPT_LEN, MAX_TOTAL_CODE, Place, javascripts, place_of, rewrite_places};

/// Longest text to find, in bytes.
pub const MAX_FIND_LEN: usize = 1024;
/// Longest text to put in, in bytes.
pub const MAX_REPLACEMENT_LEN: usize = 64 << 10;

/// One script a replace changes (or, from a preview, would change).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplacedScript {
    /// The script's id in [`javascripts`] (a route into the document; list again after an edit).
    pub id: String,
    pub kind: crate::ScriptKind,
    pub trigger: String,
    /// How many times the text was found in it.
    pub occurrences: usize,
}

/// What a replace did, or a preview says it would do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReplaceReport {
    /// Listed scripts whose code was searched.
    pub searched: usize,
    /// Scripts left alone because their code could not be read whole and exactly: it has no
    /// readable text, is longer than [`MAX_SCRIPT_LEN`] or past the listing's allowance, or is a
    /// stream that is not plain UTF-8 (PDFDocEncoding, UTF-16, a byte-order mark) or a string that
    /// does not decode cleanly, so writing it back would damage it.
    pub skipped_unreadable: usize,
    /// The scripts that hold the text and would read differently, in listing order, each place
    /// once: two ids that lead to one shared dictionary or action are listed under the first.
    pub changed: Vec<ReplacedScript>,
    /// Total finds over all of `changed`.
    pub occurrences: usize,
}

/// A script's matches: where the text starts in `code`, left to right without overlaps.
fn match_starts(code: &str, find: &str, match_case: bool) -> Vec<usize> {
    if match_case {
        code.match_indices(find).map(|(i, _)| i).collect()
    } else {
        // ASCII folding keeps every byte offset and character boundary, so offsets found in the
        // folded copy are valid in `code`.
        code.to_ascii_lowercase().match_indices(&find.to_ascii_lowercase()).map(|(i, _)| i).collect()
    }
}

/// The length `code` would have after `occurrences` finds of `find_len` bytes become `put_len`
/// bytes, or `None` when that overflows.
fn replaced_len(code_len: usize, occurrences: usize, find_len: usize, put_len: usize) -> Option<usize> {
    code_len.checked_sub(occurrences.checked_mul(find_len)?)?.checked_add(occurrences.checked_mul(put_len)?)
}

struct Planned {
    place: Place,
    new: String,
}

struct Plan {
    /// For each entry of `report.changed`, what to write.
    planned: Vec<Planned>,
    report: ReplaceReport,
}

fn plan(doc: &Document, find: &str, replacement: &str, match_case: bool) -> Result<Plan, FormError> {
    if find.is_empty() {
        return Err(FormError::Invalid("the text to find is empty".into()));
    }
    if find.len() > MAX_FIND_LEN {
        return Err(FormError::Invalid(format!("the text to find may be at most {MAX_FIND_LEN} bytes; this is {} bytes", find.len())));
    }
    if replacement.len() > MAX_REPLACEMENT_LEN {
        return Err(FormError::Invalid(format!("the replacement may be at most {MAX_REPLACEMENT_LEN} bytes; this is {} bytes", replacement.len())));
    }
    if match_case && find == replacement {
        return Err(FormError::Invalid("the replacement is the text to find; nothing would change".into()));
    }
    let inventory = javascripts(doc);
    if inventory.capped {
        return Err(FormError::Invalid(
            "the document has more scripts than a listing reads, or a chain, tree or array past a limit, so a replace could not reach all of them; nothing was changed".into(),
        ));
    }
    let mut report = ReplaceReport::default();
    let mut planned: Vec<Planned> = Vec::new();
    let mut places: HashSet<String> = HashSet::new();
    let mut written = 0usize;
    for e in &inventory.entries {
        // Only text that was read whole and exactly is searched: a cut script, one with no text
        // and one decoded with replacement characters would be written back damaged or short.
        let Some(code) = e.code.as_deref().filter(|_| !e.truncated && e.exact) else {
            report.skipped_unreadable += 1;
            continue;
        };
        report.searched += 1;
        let starts = match_starts(code, find, match_case);
        if starts.is_empty() {
            continue;
        }
        // Two ids can lead to one place (a dictionary several objects share): it is one script.
        let place =
            place_of(doc, &e.id).ok_or_else(|| FormError::Invalid(format!("the script {} cannot be edited safely; nothing was changed", e.id)))?;
        if !places.insert(place.key()) {
            continue;
        }
        let new_len = replaced_len(code.len(), starts.len(), find.len(), replacement.len()).filter(|n| *n <= MAX_SCRIPT_LEN).ok_or_else(|| {
            FormError::Invalid(format!("the script {} would be longer than {MAX_SCRIPT_LEN} bytes after the replace; nothing was changed", e.id))
        })?;
        // The rewritten scripts' total is checked, with checked arithmetic, before this one is
        // built; a script that turns out to read as before gives its share back below.
        written = written.checked_add(new_len).filter(|w| *w <= MAX_TOTAL_CODE).ok_or_else(|| {
            FormError::Invalid(format!("the rewritten scripts would hold more than {MAX_TOTAL_CODE} bytes in all; nothing was changed"))
        })?;
        let mut new = String::with_capacity(new_len);
        let mut at = 0usize;
        for s in &starts {
            new.push_str(code.get(at..*s).unwrap_or_default());
            new.push_str(replacement);
            at = s.saturating_add(find.len());
        }
        new.push_str(code.get(at..).unwrap_or_default());
        // Case ignored, a find can equal its replacement but for case: a script that already
        // reads so is not changed.
        if new == code {
            written = written.saturating_sub(new_len);
            continue;
        }
        report.occurrences = report.occurrences.saturating_add(starts.len());
        report.changed.push(ReplacedScript { id: e.id.clone(), kind: e.kind, trigger: e.trigger.clone(), occurrences: starts.len() });
        planned.push(Planned { place, new });
    }
    Ok(Plan { planned, report })
}

/// What [`replace_in_javascripts`] would do, without changing anything. Refuses what it refuses.
pub fn preview_javascript_replace(doc: &Document, find: &str, replacement: &str, match_case: bool) -> Result<ReplaceReport, FormError> {
    plan(doc, find, replacement, match_case).map(|p| p.report)
}

/// Replace every find of `find` with `replacement` in every JavaScript the document lists. A
/// script whose code cannot be read whole and exactly is left alone and counted. Everything is
/// worked out, and every object that holds a changed action is built, before the document is
/// written, so a refusal leaves it as it was.
pub fn replace_in_javascripts(doc: &mut Document, find: &str, replacement: &str, match_case: bool) -> Result<ReplaceReport, FormError> {
    let Plan { planned, report } = plan(doc, find, replacement, match_case)?;
    let edits: Vec<(&Place, &str)> = planned.iter().map(|p| (&p.place, p.new.as_str())).collect();
    rewrite_places(doc, &edits)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_do_not_overlap_and_fold_only_ascii() {
        assert_eq!(match_starts("aaaa", "aa", true), vec![0, 2]);
        assert_eq!(match_starts("Var VAR var", "var", false), vec![0, 4, 8]);
        // Not folded: a non-ASCII capital is a different character.
        assert_eq!(match_starts("É é", "é", false), vec![3]);
    }

    #[test]
    fn a_length_that_overflows_is_none() {
        assert_eq!(replaced_len(10, 2, 3, 5), Some(14));
        assert_eq!(replaced_len(10, usize::MAX, 2, 5), None);
        assert_eq!(replaced_len(usize::MAX - 1, 2, 0, 1), None);
    }
}
