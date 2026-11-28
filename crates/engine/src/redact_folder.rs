//! Search and redact every PDF in a folder: each file with a match is redacted into a new file in
//! an output folder; the originals are only read.
//!
//! A redaction that misses something is worse than none, so a file is written only when the text
//! of every page could be read in full. Pages with no text at all (scans) cannot be searched and
//! are named in the file's result. Signed files, protected files and files that already hold
//! redaction marks are left alone with the reason.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pdfcraft_render::{PageRenderer, PageText, RenderConfig, RenderRequest, RequestKind};

use crate::folder_search::{self, MAX_FILE_BYTES, MAX_FILES, SkipReason};
use crate::{Edit, NewAnnotation, RedactPattern, Session, Shape, Style, find_pattern, redacted_file_name};

/// What to redact.
#[derive(Clone, Debug)]
pub enum Target {
    /// Every occurrence of any of these words or phrases.
    Words { words: Vec<String>, case_sensitive: bool, whole_words: bool },
    /// Every match of a built-in pattern.
    Pattern(RedactPattern),
}

/// One run: what to redact, in which PDFs, and where the redacted copies go.
#[derive(Clone, Debug)]
pub struct Options {
    pub target: Target,
    /// Redact the PDFs of sub-folders too.
    pub recursive: bool,
    /// Where the redacted copies are written, in the same sub-folders as their originals.
    pub output: PathBuf,
}

/// What happened to one file.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// A redacted copy was written.
    Redacted {
        output: PathBuf,
        /// Matches redacted.
        matches: usize,
        /// Places (bookmarks, attachments, comments, metadata) that may still hold the text.
        still_holding: usize,
        /// Pages (0-based) with no text, which could not be searched.
        unsearched: Vec<usize>,
    },
    /// Nothing matched, so nothing was written. `unsearched` as above: a scan may still hold it.
    NoMatch { unsearched: Vec<usize> },
    /// The file was not redacted, and why.
    Skipped(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileResult {
    pub path: PathBuf,
    pub outcome: Outcome,
}

/// How a run went.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub files_found: usize,
    pub redacted: usize,
    pub no_match: usize,
    pub skipped: usize,
    /// The folder held more PDFs, or entries, than one run takes: some were never looked at.
    pub capped: bool,
    pub cancelled: bool,
}

/// Redact the PDFs of `folder` into `options.output`, calling `on_file` after each with the
/// number of PDFs found. `cancel` is asked between files and pages. Errors only when nothing can be run at all: nothing to look
/// for, a folder that cannot be listed or an output folder that cannot be made.
pub fn redact_folder(
    folder: &Path,
    options: &Options,
    cancel: &dyn Fn() -> bool,
    on_file: &mut dyn FnMut(usize, FileResult),
) -> Result<Summary, String> {
    if let Target::Words { words, .. } = &options.target
        && !words.iter().any(|w| !w.trim().is_empty())
    {
        return Err("there is nothing to look for: give at least one word or phrase".into());
    }
    let mut listing_options = folder_search::Options::new("-");
    listing_options.recursive = options.recursive;
    let listing = folder_search::list_pdfs(folder, &listing_options, cancel)?;
    std::fs::create_dir_all(&options.output).map_err(|e| format!("{}: {e}", options.output.display()))?;
    let root = folder.canonicalize().map_err(|e| format!("{}: {e}", folder.display()))?;
    let output = options.output.canonicalize().map_err(|e| format!("{}: {e}", options.output.display()))?;
    let mut summary = Summary {
        files_found: listing.files.len(),
        capped: listing.capped || listing.files.len() > MAX_FILES,
        cancelled: listing.cancelled,
        ..Summary::default()
    };
    let mut report = |summary: &mut Summary, path: PathBuf, outcome: Outcome| {
        match &outcome {
            Outcome::Redacted { .. } => summary.redacted += 1,
            Outcome::NoMatch { .. } => summary.no_match += 1,
            Outcome::Skipped(_) => summary.skipped += 1,
        }
        let found = summary.files_found;
        on_file(found, FileResult { path, outcome });
    };
    for (path, reason) in listing.skipped {
        report(&mut summary, path, Outcome::Skipped(reason.english()));
    }
    for (path, error) in listing.unlistable {
        report(&mut summary, path, Outcome::Skipped(error));
    }
    for path in listing.files.into_iter().take(MAX_FILES) {
        if summary.cancelled || cancel() {
            summary.cancelled = true;
            break;
        }
        // Copies from an earlier run into a folder inside this one are not redacted again.
        if output != root && path.canonicalize().is_ok_and(|p| p.starts_with(&output) && output.starts_with(&root)) {
            continue;
        }
        let relative = path.strip_prefix(folder).ok().and_then(Path::parent).map(Path::to_path_buf).unwrap_or_default();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        // The folder as the caller named it: the canonical form is only for comparing.
        let destination = options.output.join(relative).join(redacted_file_name(&name));
        let outcome = match crate::guard(|| redact_file(&root, &path, &name, (&output, &destination), &options.target, cancel)) {
            Ok(Some(outcome)) => outcome,
            Ok(None) => {
                summary.cancelled = true;
                break;
            }
            Err(panic) => Outcome::Skipped(SkipReason::Failed(panic).english()),
        };
        report(&mut summary, path, outcome);
    }
    Ok(summary)
}

/// Redact one file into `destination`; `None` when cancelled.
fn redact_file(root: &Path, path: &Path, name: &str, to: (&Path, &Path), target: &Target, cancel: &dyn Fn() -> bool) -> Option<Outcome> {
    let (output, destination) = to;
    let skip = |why: &str| Some(Outcome::Skipped(why.into()));
    let bytes = match folder_search::read_capped(root, path, MAX_FILE_BYTES, u64::MAX) {
        Ok(bytes) => Arc::new(bytes),
        Err(folder_search::Fail::Skip(reason)) => return Some(Outcome::Skipped(reason.english())),
        Err(folder_search::Fail::Budget) => return skip("it is too large"),
    };
    // A session of its own, with the file's scripts off: nothing in it runs or is remembered.
    let mut session = Session::new();
    session.set_javascript(false);
    let id = match session.open(name, None, bytes, None) {
        Ok(id) => id,
        Err(e) => return Some(Outcome::Skipped(e.to_string())),
    };
    let doc = session.get(id)?;
    if doc.is_signed() {
        return skip("it is signed, and redacting it would break the signature");
    }
    if let Some(why) = &doc.read_only_reason {
        return Some(Outcome::Skipped(why.clone()));
    }
    if doc.redaction_marks() > 0 {
        return skip("it already holds redaction marks; apply or clear them first");
    }
    let mut renderer = PageRenderer::new(doc.bytes.clone(), RenderConfig { password: doc.password.as_deref().map(Arc::from), ..Default::default() });
    let pages = doc.info.pages.len();
    if pages == 0 || renderer.page_count() != pages {
        return skip("its pages could not be read");
    }
    let mut marks = Vec::new();
    let mut matches = 0usize;
    let mut unsearched = Vec::new();
    for (page, info) in doc.info.pages.iter().enumerate() {
        if cancel() {
            return None;
        }
        let rendered = renderer.render(RenderRequest { page, kind: RequestKind::Text, tile: None, scale: 1.0, tag: 0, preview: false });
        let Some(text) = rendered.text.filter(|t| rendered.error.is_none() && !t.truncated) else {
            return Some(Outcome::Skipped(format!(
                "the text of page {} could not be read in full, so a match there could be missed",
                page.saturating_add(1)
            )));
        };
        if text.glyphs.is_empty() {
            unsearched.push(page);
            continue;
        }
        for range in found(&text, target) {
            let quads: Vec<[f64; 8]> = text.line_rects(range).into_iter().map(|r| info.view_rect_to_quad(r)).collect();
            if quads.is_empty() {
                continue;
            }
            matches += 1;
            let shape = Shape::Redact { quads, overlay: String::new(), look: Default::default() };
            let style = Style::default_for(&shape);
            marks.push(Edit::AddAnnotation(NewAnnotation { page, shape, style, contents: String::new(), author: String::new() }));
        }
    }
    if matches == 0 {
        return Some(Outcome::NoMatch { unsearched });
    }
    let applied = session
        .apply(id, Edit::Batch { label: "Mark for redaction".into(), edits: marks })
        .and_then(|_| session.apply(id, Edit::ApplyRedactions { pages: None }));
    if let Err(e) = applied {
        return Some(Outcome::Skipped(e.to_string()));
    }
    let still_holding = session.get(id)?.redaction_elsewhere().map_or(0, |left| left.total());
    let redacted = match session.save_bytes(id) {
        Ok(redacted) => redacted,
        Err(e) => return Some(Outcome::Skipped(e.to_string())),
    };
    match write_new(output, destination, &redacted) {
        Ok(()) => Some(Outcome::Redacted { output: destination.to_path_buf(), matches, still_holding, unsearched }),
        Err(e) => Some(Outcome::Skipped(format!("{}: {e}", destination.display()))),
    }
}

/// The ranges of `text` that `target` names.
fn found(text: &PageText, target: &Target) -> Vec<std::ops::Range<usize>> {
    match target {
        Target::Pattern(pattern) => text.find_with(|chars| find_pattern(*pattern, chars)),
        Target::Words { words, case_sensitive, whole_words } => {
            words.iter().filter(|w| !w.trim().is_empty()).flat_map(|w| text.find_opts(w, *case_sensitive, *whole_words)).collect()
        }
    }
}

/// Write `bytes` to a file that must not exist yet, inside `output` (canonical): a link among
/// its sub-folders can't take the copy elsewhere. A partly written file is removed.
fn write_new(output: &Path, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        if !parent.canonicalize()?.starts_with(output) {
            return Err(std::io::Error::other("the folder leads outside the output folder"));
        }
    }
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    if written.is_err() {
        drop(file);
        let _ = std::fs::remove_file(path);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One page drawing `content`.
    fn pdf(content: &str) -> Vec<u8> {
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
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

    fn says(text: &str) -> Vec<u8> {
        pdf(&format!("BT /F1 14 Tf 72 700 Td ({text}) Tj ET"))
    }

    #[test]
    fn only_matching_files_get_a_redacted_copy_and_originals_are_never_touched() {
        let dir = std::env::temp_dir().join(format!("pdfcraft-redact-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let secret = says("call Zebedee today");
        std::fs::write(dir.join("a.pdf"), &secret).unwrap();
        std::fs::write(dir.join("b.pdf"), says("nothing here")).unwrap();
        std::fs::write(dir.join("c.pdf"), pdf("0 0 100 100 re f")).unwrap();
        std::fs::write(dir.join("d.pdf"), b"not a PDF").unwrap();
        std::fs::write(dir.join("sub").join("e.pdf"), says("ZEBEDEE again")).unwrap();
        // Inside the folder being redacted: a second run must not redact the copies.
        let out = dir.join("out");
        let options = Options {
            target: Target::Words { words: vec!["zebedee".into()], case_sensitive: false, whole_words: false },
            recursive: true,
            output: out.clone(),
        };
        let run = || {
            let mut results = Vec::new();
            let summary = redact_folder(&dir, &options, &|| false, &mut |_, r| results.push(r)).unwrap();
            (summary, results)
        };
        let (summary, results) = run();
        assert_eq!((summary.redacted, summary.no_match, summary.skipped), (2, 2, 1), "{results:?}");
        let of = |name: &str| &results.iter().find(|r| r.path.ends_with(name)).unwrap().outcome;
        assert!(matches!(of("c.pdf"), Outcome::NoMatch { unsearched } if unsearched == &[0]), "a page without text is named");
        assert!(matches!(of("d.pdf"), Outcome::Skipped(_)));
        let copy = std::fs::read(out.join("a_Redacted.pdf")).unwrap();
        assert!(!copy.windows(8).any(|w| w.eq_ignore_ascii_case(b"Zebedee ")), "no earlier revision keeps the words");
        let mut session = Session::new();
        let id = session.open("x.pdf", None, Arc::new(copy.clone()), None).unwrap();
        let mut renderer = PageRenderer::new(session.get(id).unwrap().bytes.clone(), RenderConfig::default());
        let text = renderer.render(RenderRequest { page: 0, kind: RequestKind::Text, tile: None, scale: 1.0, tag: 0, preview: false }).text.unwrap();
        assert!(text.find_opts("zebedee", false, false).is_empty());
        assert!(!text.find_opts("today", false, false).is_empty(), "the rest of the line stays");
        assert!(out.join("sub").join("e_Redacted.pdf").exists());
        assert_eq!(std::fs::read(dir.join("a.pdf")).unwrap(), secret, "the original is untouched");

        // Again: the copies are not read as input, and none is overwritten.
        let (again, results) = run();
        assert_eq!((again.redacted, again.skipped), (0, 3), "{results:?}");
        assert_eq!(std::fs::read(out.join("a_Redacted.pdf")).unwrap(), copy);

        assert!(
            redact_folder(
                &dir,
                &Options { target: Target::Words { words: vec![" ".into()], case_sensitive: false, whole_words: false }, ..options.clone() },
                &|| false,
                &mut |_, _| {}
            )
            .is_err()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
