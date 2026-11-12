//! Search the text of every PDF in a folder (View ▸ Search all PDFs in a folder).
//!
//! One walk, one matcher: each page's text is extracted by the renderer and searched with
//! [`PageText::find_opts_limited`], the same matcher the find bar, Advanced search and the
//! `text_find` tool use, so case-sensitive and whole-word searches agree with them. A file that
//! cannot be read is reported as skipped and never stops the search. Folders are never followed
//! through links.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pdfcraft_render::{OpenError, PageRenderer, PageText, RenderConfig, RenderRequest, RequestKind, inspect};

/// Most PDFs one search reads (the first ones, in path order); the rest are counted as `files_found`.
pub const MAX_FILES: usize = 2000;

/// Most matches one search returns.
pub const MAX_RESULTS: usize = 5000;

/// Largest PDF read into memory; a bigger one is skipped.
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// Most directory entries looked at while listing the folder, so a huge tree can't hold the search.
pub const MAX_ENTRIES: usize = 50_000;

/// Deepest folder level a recursive search enters.
pub const MAX_DEPTH: usize = 32;

/// Glyphs of context kept on each side of a match in its snippet.
pub const CONTEXT: usize = 36;

/// What to search for and where to stop.
#[derive(Clone, Debug)]
pub struct Options {
    pub query: String,
    pub case_sensitive: bool,
    pub whole_words: bool,
    /// Enter sub-folders too (links are never followed).
    pub recursive: bool,
    /// At most this many PDFs are read (clamped to [`MAX_FILES`]).
    pub max_files: usize,
    /// At most this many matches are returned (clamped to [`MAX_RESULTS`]).
    pub max_results: usize,
}

impl Options {
    /// Case-insensitive, any-position, non-recursive search with the default caps.
    pub fn new(query: impl Into<String>) -> Self {
        Self { query: query.into(), case_sensitive: false, whole_words: false, recursive: false, max_files: MAX_FILES, max_results: 500 }
    }
}

/// One match: a 0-based page, and the text around it with whitespace folded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub page: usize,
    pub before: String,
    pub text: String,
    pub after: String,
}

impl Match {
    /// `before`, the match and `after` as one line.
    pub fn snippet(&self) -> String {
        format!("{} {} {}", self.before, self.text, self.after).trim().to_owned()
    }
}

/// What happened to one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileOutcome {
    /// Searched; `matches` may be empty. `pages_failed` counts pages whose text could not be read.
    Searched { path: PathBuf, pages: usize, pages_failed: usize, matches: Vec<Match> },
    /// Not searched, and why (not a readable PDF, password protected, too big, a link, ...).
    Skipped { path: PathBuf, reason: String },
}

/// How a search ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// PDFs listed (before the file cap).
    pub files_found: usize,
    /// PDFs read and searched.
    pub files_searched: usize,
    pub files_skipped: usize,
    pub match_count: usize,
    /// More PDFs were listed than `max_files` allowed.
    pub files_capped: bool,
    /// More matches exist than `max_results` allowed.
    pub results_capped: bool,
    /// The listing stopped at [`MAX_ENTRIES`] or [`MAX_DEPTH`]; some PDFs may be missing.
    pub listing_capped: bool,
    pub cancelled: bool,
}

/// A whole search collected: files with matches (path order) and files not searched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub summary: Summary,
    /// Searched files that have at least one match.
    pub files: Vec<FileOutcome>,
    pub skipped: Vec<FileOutcome>,
}

/// Search `folder` and collect the outcome. See [`search_folder`].
pub fn search_folder_report(folder: &Path, options: &Options, cancel: &dyn Fn() -> bool) -> Result<Report, String> {
    let (mut files, mut skipped) = (Vec::new(), Vec::new());
    let summary = search_folder(folder, options, cancel, &mut |outcome| match &outcome {
        FileOutcome::Searched { matches, .. } if matches.is_empty() => {}
        FileOutcome::Searched { .. } => files.push(outcome),
        FileOutcome::Skipped { .. } => skipped.push(outcome),
    })?;
    Ok(Report { summary, files, skipped })
}

/// Search the PDFs of `folder` (its `.pdf` files, in path order; sub-folders only with
/// `recursive`), calling `on_file` for each file as it is done. `cancel` is asked between pages
/// and files; a cancelled search returns what it has with `cancelled` set. Only a folder that
/// cannot be listed or an empty query is an error; a bad file is a [`FileOutcome::Skipped`].
pub fn search_folder(folder: &Path, options: &Options, cancel: &dyn Fn() -> bool, on_file: &mut dyn FnMut(FileOutcome)) -> Result<Summary, String> {
    if options.query.trim().is_empty() {
        return Err("the search text must not be blank".into());
    }
    let (files, listing_capped, mut skipped) = list_pdfs(folder, options.recursive)?;
    let max_files = options.max_files.clamp(1, MAX_FILES);
    let limit = options.max_results.clamp(1, MAX_RESULTS);
    let mut summary = Summary { files_found: files.len(), files_capped: files.len() > max_files, listing_capped, ..Summary::default() };
    for (path, reason) in skipped.drain(..) {
        summary.files_skipped += 1;
        on_file(FileOutcome::Skipped { path, reason });
    }
    'files: for path in files.into_iter().take(max_files) {
        if cancel() {
            summary.cancelled = true;
            break;
        }
        let remaining = limit.saturating_sub(summary.match_count);
        let done = match crate::guard(|| search_file(&path, options, remaining, cancel)) {
            Ok(done) => done,
            Err(panic) => Err(format!("reading it failed unexpectedly ({panic})")),
        };
        match done {
            Ok(file) => {
                summary.files_searched += 1;
                summary.match_count += file.matches.len();
                summary.results_capped |= file.more;
                summary.cancelled |= file.cancelled;
                on_file(FileOutcome::Searched { path, pages: file.pages, pages_failed: file.pages_failed, matches: file.matches });
                if summary.results_capped || summary.cancelled {
                    break 'files;
                }
            }
            Err(reason) => {
                summary.files_skipped += 1;
                on_file(FileOutcome::Skipped { path, reason });
            }
        }
    }
    Ok(summary)
}

/// The `.pdf` files in `folder` by path, with whether the listing was cut short and the links
/// and special files named `.pdf` that were passed over.
#[allow(clippy::type_complexity)]
fn list_pdfs(folder: &Path, recursive: bool) -> Result<(Vec<PathBuf>, bool, Vec<(PathBuf, String)>), String> {
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    let mut seen = 0usize;
    let mut capped = false;
    let mut stack = vec![(folder.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            // The folder asked for must be listable; a sub-folder that is not is skipped.
            Err(e) if depth == 0 => return Err(format!("{}: {e}", folder.display())),
            Err(e) => {
                skipped.push((dir, format!("the folder could not be listed: {e}")));
                continue;
            }
        };
        for entry in entries {
            seen = seen.saturating_add(1);
            if seen > MAX_ENTRIES {
                capped = true;
                break;
            }
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            let Ok(meta) = path.symlink_metadata() else { continue };
            let is_pdf = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
            if meta.file_type().is_symlink() {
                if is_pdf {
                    skipped.push((path, "a link is not followed".into()));
                }
            } else if meta.is_dir() {
                if recursive {
                    if depth >= MAX_DEPTH {
                        capped = true;
                    } else {
                        stack.push((path, depth + 1));
                    }
                }
            } else if is_pdf {
                if meta.is_file() {
                    files.push(path);
                } else {
                    skipped.push((path, "not a regular file".into()));
                }
            }
        }
        if capped && seen > MAX_ENTRIES {
            break;
        }
    }
    files.sort();
    skipped.sort();
    Ok((files, capped, skipped))
}

struct Searched {
    pages: usize,
    pages_failed: usize,
    matches: Vec<Match>,
    /// A match past the allowance exists.
    more: bool,
    cancelled: bool,
}

fn search_file(path: &Path, options: &Options, allowance: usize, cancel: &dyn Fn() -> bool) -> Result<Searched, String> {
    let bytes = read_capped(path)?;
    let bytes = Arc::new(bytes);
    let mut renderer = PageRenderer::new(bytes.clone(), RenderConfig::default());
    let pages = renderer.page_count();
    if pages == 0 {
        return Err(match inspect(bytes, None) {
            Err(OpenError::NeedsPassword | OpenError::WrongPassword) => "password protected".to_owned(),
            Err(e) => e.to_string(),
            Ok(_) => "the document has no pages".to_owned(),
        });
    }
    let mut out = Searched { pages, pages_failed: 0, matches: Vec::new(), more: false, cancelled: false };
    for page in 0..pages {
        if cancel() {
            out.cancelled = true;
            break;
        }
        let rendered = renderer.render(RenderRequest { page, kind: RequestKind::Text, tile: None, scale: 1.0, tag: 0, preview: false });
        let Some(text) = rendered.text.filter(|_| rendered.error.is_none()) else {
            out.pages_failed += 1;
            continue;
        };
        // One past the allowance proves that more exist.
        let room = allowance.saturating_sub(out.matches.len());
        let found = text.find_opts_limited(&options.query, options.case_sensitive, options.whole_words, room.saturating_add(1));
        for (k, range) in found.into_iter().enumerate() {
            if k >= room {
                out.more = true;
                return Ok(out);
            }
            out.matches.push(snippet(&text, page, range));
        }
    }
    Ok(out)
}

fn snippet(text: &PageText, page: usize, range: std::ops::Range<usize>) -> Match {
    let n = text.glyphs.len();
    let (start, end) = (range.start.min(n), range.end.min(n));
    let flat = |s: String| s.split_whitespace().collect::<Vec<_>>().join(" ");
    Match {
        page,
        before: flat(text.text_of(start.saturating_sub(CONTEXT)..start)),
        text: flat(text.text_of(start..end)),
        after: flat(text.text_of(end..end.saturating_add(CONTEXT).min(n))),
    }
}

/// The file's bytes, refusing a link swapped in since the listing and a file over the cap.
fn read_capped(path: &Path) -> Result<Vec<u8>, String> {
    let meta = path.symlink_metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("a link or special file is not read".into());
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("larger than {} MiB", MAX_FILE_BYTES >> 20));
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES.saturating_add(1)).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(format!("larger than {} MiB", MAX_FILE_BYTES >> 20));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// One page saying `text`.
    fn pdf(text: &str) -> Vec<u8> {
        let body = format!("BT /F1 14 Tf 72 700 Td ({text}) Tj ET");
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()),
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

    fn folder(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pdfcraft-folder-search-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for n in ["a", "b", "c"] {
            std::fs::write(d.join(format!("{n}.pdf")), pdf("needle in a haystack")).unwrap();
        }
        d
    }

    #[test]
    fn a_blank_query_is_refused_and_a_missing_folder_is_an_error() {
        let d = folder("blank");
        assert!(search_folder_report(&d, &Options::new("  "), &|| false).is_err());
        assert!(search_folder_report(&d.join("missing"), &Options::new("x"), &|| false).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A search stopped between files keeps what it found and says it was cancelled.
    #[test]
    fn cancelling_keeps_the_files_done_and_stops_the_rest() {
        let d = folder("cancel");
        let asked = AtomicUsize::new(0);
        // Asked once before each file and once before each page: yes on the third question.
        let report = search_folder_report(&d, &Options::new("needle"), &|| asked.fetch_add(1, Ordering::Relaxed) >= 2).unwrap();
        assert!(report.summary.cancelled);
        assert_eq!(report.summary.files_searched, 1, "{report:?}");
        assert_eq!(report.files.len(), 1);
        let all = search_folder_report(&d, &Options::new("needle"), &|| false).unwrap();
        assert_eq!((all.summary.files_searched, all.summary.match_count, all.summary.cancelled), (3, 3, false));
        let _ = std::fs::remove_dir_all(&d);
    }
}
