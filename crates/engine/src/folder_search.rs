//! Search the text of every PDF in a folder (Edit > Advanced search, scope Folder).
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

/// Most bytes one search reads in all; the search stops there (`bytes_capped`).
pub const MAX_TOTAL_BYTES: u64 = 4 * 1024 * 1024 * 1024;

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
    /// A PDF larger than this is skipped. Lowered only by tests.
    pub max_file_bytes: u64,
    /// The listing stops after this many directory entries. Lowered only by tests.
    pub max_entries: usize,
    /// The search stops once it has read this many bytes. Lowered only by tests.
    pub max_total_bytes: u64,
    /// Tests only: treat this 0-based page of every file as unreadable.
    #[cfg(test)]
    fail_page: Option<usize>,
}

impl Options {
    /// Case-insensitive, any-position, non-recursive search with the default caps.
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            case_sensitive: false,
            whole_words: false,
            recursive: false,
            max_files: MAX_FILES,
            max_results: 500,
            max_file_bytes: MAX_FILE_BYTES,
            max_entries: MAX_ENTRIES,
            max_total_bytes: MAX_TOTAL_BYTES,
            #[cfg(test)]
            fail_page: None,
        }
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

/// Why a file was not searched. [`Self::english`] is for tools; the window words each itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// A link (never followed).
    Link,
    /// Not a regular file (a device, a pipe, ...), or one that changed into something else.
    SpecialFile,
    /// Larger than the per-file cap, in MiB.
    TooLarge(u64),
    PasswordProtected,
    /// The file could not be read; the system's words.
    Unreadable(String),
    /// Not a PDF the reader can open; the reader's words.
    Invalid(String),
    /// A kind of encryption or a feature that is not supported; the reader's words.
    Unsupported(String),
    NoPages,
    /// Reading it failed unexpectedly (a caught panic); the detail.
    Failed(String),
}

impl SkipReason {
    /// The reason in English, for tools and logs.
    pub fn english(&self) -> String {
        match self {
            Self::Link => "a link is not followed".into(),
            Self::SpecialFile => "not a regular file".into(),
            Self::TooLarge(mib) => format!("larger than {mib} MiB"),
            Self::PasswordProtected => "password protected".into(),
            Self::Unreadable(e) | Self::Invalid(e) | Self::Unsupported(e) => e.clone(),
            Self::NoPages => "the document has no pages".into(),
            Self::Failed(e) => format!("reading it failed unexpectedly ({e})"),
        }
    }
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.english())
    }
}

/// What happened to one file or folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileOutcome {
    /// Searched; `matches` may be empty. `pages_failed` counts pages whose text could not be read
    /// and `pages_truncated` pages whose text was cut at the renderer's limits (so a match there
    /// can be missed).
    Searched { path: PathBuf, pages: usize, pages_failed: usize, pages_truncated: usize, matches: Vec<Match> },
    /// A PDF that was not searched, and why.
    Skipped { path: PathBuf, reason: SkipReason },
    /// A sub-folder that could not be listed (the system's words); its PDFs are not searched.
    FolderUnlistable { path: PathBuf, error: String },
}

/// How a search ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// PDFs listed (before the file cap).
    pub files_found: usize,
    /// PDFs read and searched.
    pub files_searched: usize,
    pub files_skipped: usize,
    pub folders_unlistable: usize,
    /// Pages, in searched files with or without matches, whose text could not be read.
    pub pages_failed: usize,
    /// Pages whose text was cut at the renderer's limits.
    pub pages_truncated: usize,
    pub match_count: usize,
    /// More PDFs were listed than `max_files` allowed.
    pub files_capped: bool,
    /// More matches exist than `max_results` allowed.
    pub results_capped: bool,
    /// The listing stopped at the entry or depth cap; some PDFs may be missing.
    pub listing_capped: bool,
    /// The search stopped at the cap on bytes read; later files were not searched.
    pub bytes_capped: bool,
    pub cancelled: bool,
}

impl Summary {
    /// Every listed PDF was searched in full: no cap or cancel cut the search, no folder was
    /// unreadable, and no page failed or was cut short. Skipped files are counted separately.
    pub fn complete(&self) -> bool {
        !(self.files_capped
            || self.results_capped
            || self.listing_capped
            || self.bytes_capped
            || self.cancelled
            || self.folders_unlistable > 0
            || self.pages_failed > 0
            || self.pages_truncated > 0)
    }
}

/// A whole search collected: files with matches (path order), files not searched and folders not listed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub summary: Summary,
    /// Searched files that have at least one match.
    pub files: Vec<FileOutcome>,
    pub skipped: Vec<FileOutcome>,
    pub unlistable: Vec<FileOutcome>,
}

/// Search `folder` and collect the outcome. See [`search_folder`].
pub fn search_folder_report(folder: &Path, options: &Options, cancel: &dyn Fn() -> bool) -> Result<Report, String> {
    let (mut files, mut skipped, mut unlistable) = (Vec::new(), Vec::new(), Vec::new());
    let summary = search_folder(folder, options, cancel, &mut |outcome| match &outcome {
        FileOutcome::Searched { matches, .. } if matches.is_empty() => {}
        FileOutcome::Searched { .. } => files.push(outcome),
        FileOutcome::Skipped { .. } => skipped.push(outcome),
        FileOutcome::FolderUnlistable { .. } => unlistable.push(outcome),
    })?;
    Ok(Report { summary, files, skipped, unlistable })
}

/// Search the PDFs of `folder` (its `.pdf` files, in path order; sub-folders only with
/// `recursive`), calling `on_file` for each file as it is done. `cancel` is asked while listing
/// and between pages and files; a cancelled search returns what it has with `cancelled` set. Only
/// a folder that cannot be listed or an empty query is an error; a bad file is a
/// [`FileOutcome::Skipped`].
pub fn search_folder(folder: &Path, options: &Options, cancel: &dyn Fn() -> bool, on_file: &mut dyn FnMut(FileOutcome)) -> Result<Summary, String> {
    if options.query.trim().is_empty() {
        return Err("the search text must not be blank".into());
    }
    let listing = list_pdfs(folder, options, cancel)?;
    let max_files = options.max_files.clamp(1, MAX_FILES);
    let limit = options.max_results.clamp(1, MAX_RESULTS);
    let mut summary = Summary {
        files_found: listing.files.len(),
        files_capped: listing.files.len() > max_files,
        listing_capped: listing.capped,
        cancelled: listing.cancelled,
        ..Summary::default()
    };
    for (path, reason) in listing.skipped {
        summary.files_skipped += 1;
        on_file(FileOutcome::Skipped { path, reason });
    }
    for (path, error) in listing.unlistable {
        summary.folders_unlistable += 1;
        on_file(FileOutcome::FolderUnlistable { path, error });
    }
    if summary.cancelled {
        return Ok(summary);
    }
    // The folder as the system names it, which every file must stay inside.
    let root = folder.canonicalize().map_err(|e| format!("{}: {e}", folder.display()))?;
    let mut bytes_read = 0u64;
    for path in listing.files.into_iter().take(max_files) {
        if cancel() {
            summary.cancelled = true;
            break;
        }
        let remaining = limit.saturating_sub(summary.match_count);
        let budget = options.max_total_bytes.saturating_sub(bytes_read);
        let done = match crate::guard(|| search_file(&root, &path, options, remaining, budget, cancel)) {
            Ok(done) => done,
            Err(panic) => Err(Fail::Skip(SkipReason::Failed(panic))),
        };
        match done {
            Ok(file) => {
                bytes_read = bytes_read.saturating_add(file.bytes);
                summary.files_searched += 1;
                summary.match_count += file.matches.len();
                summary.pages_failed += file.pages_failed;
                summary.pages_truncated += file.pages_truncated;
                summary.results_capped |= file.more;
                summary.cancelled |= file.cancelled;
                on_file(FileOutcome::Searched {
                    path,
                    pages: file.pages,
                    pages_failed: file.pages_failed,
                    pages_truncated: file.pages_truncated,
                    matches: file.matches,
                });
                if summary.results_capped || summary.cancelled {
                    break;
                }
            }
            Err(Fail::Budget) => {
                summary.bytes_capped = true;
                break;
            }
            Err(Fail::Skip(reason)) => {
                summary.files_skipped += 1;
                on_file(FileOutcome::Skipped { path, reason });
            }
        }
    }
    Ok(summary)
}

#[derive(Default)]
struct Listing {
    files: Vec<PathBuf>,
    /// The entry or depth cap cut the listing short.
    capped: bool,
    cancelled: bool,
    /// Links and special files named `.pdf`.
    skipped: Vec<(PathBuf, SkipReason)>,
    /// Sub-folders that could not be listed.
    unlistable: Vec<(PathBuf, String)>,
}

/// The `.pdf` files in `folder`, by path.
fn list_pdfs(folder: &Path, options: &Options, cancel: &dyn Fn() -> bool) -> Result<Listing, String> {
    let mut out = Listing::default();
    let mut seen = 0usize;
    let mut stack = vec![(folder.to_path_buf(), 0usize)];
    'walk: while let Some((dir, depth)) = stack.pop() {
        if cancel() {
            out.cancelled = true;
            break;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            // The folder asked for must be listable; a sub-folder that is not is reported.
            Err(e) if depth == 0 => return Err(format!("{}: {e}", folder.display())),
            Err(e) => {
                out.unlistable.push((dir, e.to_string()));
                continue;
            }
        };
        for entry in entries {
            seen = seen.saturating_add(1);
            if seen > options.max_entries {
                out.capped = true;
                break 'walk;
            }
            if seen.is_multiple_of(256) && cancel() {
                out.cancelled = true;
                break 'walk;
            }
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            let Ok(meta) = path.symlink_metadata() else { continue };
            let is_pdf = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
            if meta.file_type().is_symlink() {
                if is_pdf {
                    out.skipped.push((path, SkipReason::Link));
                }
            } else if meta.is_dir() {
                if recursive_ok(options, depth) {
                    stack.push((path, depth + 1));
                } else if options.recursive {
                    out.capped = true;
                }
            } else if is_pdf {
                if meta.is_file() {
                    out.files.push(path);
                } else {
                    out.skipped.push((path, SkipReason::SpecialFile));
                }
            }
        }
    }
    out.files.sort();
    out.skipped.sort_by(|a, b| a.0.cmp(&b.0));
    out.unlistable.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn recursive_ok(options: &Options, depth: usize) -> bool {
    options.recursive && depth < MAX_DEPTH
}

enum Fail {
    Skip(SkipReason),
    /// Reading this file would pass the cap on bytes read.
    Budget,
}

struct Searched {
    pages: usize,
    pages_failed: usize,
    pages_truncated: usize,
    matches: Vec<Match>,
    /// A match past the allowance exists.
    more: bool,
    cancelled: bool,
    bytes: u64,
}

fn search_file(root: &Path, path: &Path, options: &Options, allowance: usize, budget: u64, cancel: &dyn Fn() -> bool) -> Result<Searched, Fail> {
    let bytes = read_capped(root, path, options.max_file_bytes, budget)?;
    let len = bytes.len() as u64;
    let bytes = Arc::new(bytes);
    let mut renderer = PageRenderer::new(bytes.clone(), RenderConfig::default());
    let pages = renderer.page_count();
    if pages == 0 {
        return Err(Fail::Skip(match inspect(bytes, None) {
            Err(OpenError::NeedsPassword | OpenError::WrongPassword) => SkipReason::PasswordProtected,
            Err(e @ OpenError::Unsupported(_)) => SkipReason::Unsupported(e.to_string()),
            Err(e) => SkipReason::Invalid(e.to_string()),
            Ok(_) => SkipReason::NoPages,
        }));
    }
    let mut out = Searched { pages, pages_failed: 0, pages_truncated: 0, matches: Vec::new(), more: false, cancelled: false, bytes: len };
    for page in 0..pages {
        if cancel() {
            out.cancelled = true;
            break;
        }
        let rendered = renderer.render(RenderRequest { page, kind: RequestKind::Text, tile: None, scale: 1.0, tag: 0, preview: false });
        #[cfg(test)]
        let forced = options.fail_page == Some(page);
        #[cfg(not(test))]
        let forced = false;
        let Some(text) = rendered.text.filter(|_| rendered.error.is_none() && !forced) else {
            out.pages_failed += 1;
            continue;
        };
        out.pages_truncated += usize::from(text.truncated);
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

/// The file's bytes, refusing a link or special file swapped in since the listing, a path that
/// leaves `root`, a file over `max` and one that would pass the remaining `budget`. The file is
/// checked again after it is open: it must be the same regular file the listing saw.
fn read_capped(root: &Path, path: &Path, max: u64, budget: u64) -> Result<Vec<u8>, Fail> {
    let skip = |r: SkipReason| Fail::Skip(r);
    let before = path.symlink_metadata().map_err(|e| skip(SkipReason::Unreadable(e.to_string())))?;
    if !before.is_file() {
        return Err(skip(if before.file_type().is_symlink() { SkipReason::Link } else { SkipReason::SpecialFile }));
    }
    if !path.canonicalize().is_ok_and(|real| real.starts_with(root)) {
        return Err(skip(SkipReason::Link));
    }
    if before.len() > max {
        return Err(skip(SkipReason::TooLarge(max >> 20)));
    }
    if before.len() > budget {
        return Err(Fail::Budget);
    }
    let file = std::fs::File::open(path).map_err(|e| skip(SkipReason::Unreadable(e.to_string())))?;
    let after = file.metadata().map_err(|e| skip(SkipReason::Unreadable(e.to_string())))?;
    if !after.is_file() || !same_file(&before, &after) {
        return Err(skip(SkipReason::SpecialFile));
    }
    let mut bytes = Vec::new();
    file.take(max.min(budget).saturating_add(1)).read_to_end(&mut bytes).map_err(|e| skip(SkipReason::Unreadable(e.to_string())))?;
    if bytes.len() as u64 > max {
        return Err(skip(SkipReason::TooLarge(max >> 20)));
    }
    if bytes.len() as u64 > budget {
        return Err(Fail::Budget);
    }
    Ok(bytes)
}

/// Whether two looks at a file are of the same file (device and inode where the system has them).
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        a.len() == b.len()
    }
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
        // Asked once for the listing, then before each file and each page: yes before the second file.
        let report = search_folder_report(&d, &Options::new("needle"), &|| asked.fetch_add(1, Ordering::Relaxed) >= 3).unwrap();
        assert!(report.summary.cancelled);
        assert_eq!(report.summary.files_searched, 1, "{report:?}");
        assert_eq!(report.files.len(), 1);
        let all = search_folder_report(&d, &Options::new("needle"), &|| false).unwrap();
        assert_eq!((all.summary.files_searched, all.summary.match_count, all.summary.cancelled), (3, 3, false));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A page that can't be read in a file with no match still makes the search incomplete.
    #[test]
    fn a_failed_page_in_a_file_without_a_match_is_counted() {
        let d = folder("failed-page");
        let o = Options { fail_page: Some(0), ..Options::new("absent words") };
        let report = search_folder_report(&d, &o, &|| false).unwrap();
        assert_eq!((report.summary.files_searched, report.summary.match_count), (3, 0), "{report:?}");
        assert!(report.files.is_empty(), "no file has a match");
        assert_eq!(report.summary.pages_failed, 3, "{report:?}");
        assert!(!report.summary.complete());
        let clean = folder("clean");
        assert!(search_folder_report(&clean, &Options::new("absent words"), &|| false).unwrap().summary.complete());
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&clean);
    }

    #[test]
    fn complete_is_false_after_any_cut() {
        let cuts: [fn(&mut Summary); 8] = [
            |s| s.files_capped = true,
            |s| s.results_capped = true,
            |s| s.listing_capped = true,
            |s| s.bytes_capped = true,
            |s| s.cancelled = true,
            |s| s.folders_unlistable = 1,
            |s| s.pages_failed = 1,
            |s| s.pages_truncated = 1,
        ];
        assert!(Summary::default().complete());
        for cut in cuts {
            let mut s = Summary::default();
            cut(&mut s);
            assert!(!s.complete(), "{s:?}");
        }
    }

    #[test]
    fn caps_on_file_size_entries_and_bytes_are_reported() {
        let d = folder("caps");
        let small = Options { max_file_bytes: 100, ..Options::new("needle") };
        let r = search_folder_report(&d, &small, &|| false).unwrap();
        assert_eq!(r.summary.files_skipped, 3, "{r:?}");
        assert!(r.skipped.iter().all(|f| matches!(f, FileOutcome::Skipped { reason: SkipReason::TooLarge(_), .. })));
        let few = Options { max_entries: 2, ..Options::new("needle") };
        let r = search_folder_report(&d, &few, &|| false).unwrap();
        assert!(r.summary.listing_capped && !r.summary.complete(), "{r:?}");
        let size = std::fs::metadata(d.join("a.pdf")).unwrap().len();
        let budget = Options { max_total_bytes: size + size / 2, ..Options::new("needle") };
        let r = search_folder_report(&d, &budget, &|| false).unwrap();
        assert_eq!((r.summary.files_searched, r.summary.bytes_capped), (1, true), "{r:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A sub-folder that can't be listed is its own outcome, not a skipped file.
    #[cfg(unix)]
    #[test]
    fn an_unlistable_subfolder_is_reported_apart_from_skipped_files() {
        use std::os::unix::fs::PermissionsExt;
        let d = folder("unlistable");
        let locked = d.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Root can list anything: there is nothing to see then.
        if std::fs::read_dir(&locked).is_err() {
            let o = Options { recursive: true, ..Options::new("needle") };
            let r = search_folder_report(&d, &o, &|| false).unwrap();
            assert_eq!((r.summary.folders_unlistable, r.summary.files_skipped), (1, 0), "{r:?}");
            assert!(!r.summary.complete());
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_cancel_during_the_listing_returns_at_once() {
        let d = folder("cancel-listing");
        let r = search_folder_report(&d, &Options::new("needle"), &|| true).unwrap();
        assert!(r.summary.cancelled);
        assert_eq!(r.summary.files_searched, 0);
        let _ = std::fs::remove_dir_all(&d);
    }
}
