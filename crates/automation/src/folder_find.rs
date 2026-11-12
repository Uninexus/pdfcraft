//! `text_find_folder`: the text search of every PDF in a folder, as Edit > Advanced search with the Folder
//! scope runs it (the engine's `folder_search`).

use pdfcraft_engine::folder_search::{self, FileOutcome, MAX_FILES, MAX_RESULTS, Options};

use super::error::{Result, failed};
use super::json::Args;
use super::{Automation, Path, Value, json};

/// Skipped files listed in one answer; `skipped_truncated` says when more were left out.
const MAX_SKIPPED_LISTED: usize = 1000;

/// A bounded integer argument: absent is `default`, out of `1..=max` is refused, not clamped.
fn bounded(a: &Args, key: &str, default: usize, max: usize) -> Result<usize> {
    match a.opt_int(key)? {
        None => Ok(default),
        Some(n) => usize::try_from(n).ok().filter(|n| (1..=max).contains(n)).ok_or_else(|| Args::wrong(key, &format!("between 1 and {max}"))),
    }
}

impl Automation {
    pub(crate) fn text_find_folder(&self, a: &Args) -> Result<Value> {
        let query = a.str("query")?;
        if query.trim().is_empty() {
            return Err(Args::wrong("query", "text to search for, not blank"));
        }
        let requested = a.str("folder")?;
        if requested.is_empty() {
            return Err(Args::wrong("folder", "a folder path"));
        }
        let options = Options {
            case_sensitive: a.opt_bool("case_sensitive")?.unwrap_or(false),
            whole_words: a.opt_bool("whole_words")?.unwrap_or(false),
            recursive: a.opt_bool("recursive")?.unwrap_or(false),
            max_files: bounded(a, "max_files", MAX_FILES, MAX_FILES)?,
            max_results: bounded(a, "limit", 500, MAX_RESULTS)?,
            ..Options::new(query)
        };
        let folder = self.resolve(requested, false)?;
        if !folder.is_dir() {
            return Err(failed(format!("{requested} is not a folder: pass the folder that holds the PDFs")));
        }
        let report = folder_search::search_folder_report(&folder, &options, &|| false).map_err(failed)?;
        let name = |path: &Path| path.strip_prefix(&folder).unwrap_or(path).to_string_lossy().replace('\\', "/");
        // A name that is not UTF-8 is shown with replacement characters: `path_lossy` says so, and
        // such a path can't be passed back as given.
        let lossy = |path: &Path| path.to_str().is_none();
        let files: Vec<Value> = report
            .files
            .iter()
            .filter_map(|f| match f {
                FileOutcome::Searched { path, pages, pages_failed, pages_truncated, matches } => {
                    let matches: Vec<Value> = matches
                        .iter()
                        .map(|m| {
                            json!({ "page": m.page.saturating_add(1), "text": m.text, "before": m.before, "after": m.after, "snippet": m.snippet() })
                        })
                        .collect();
                    Some(json!({ "path": path.to_string_lossy(), "relative": name(path), "path_lossy": lossy(path), "pages": pages,
                        "pages_failed": pages_failed, "pages_truncated": pages_truncated, "count": matches.len(), "matches": matches }))
                }
                _ => None,
            })
            .collect();
        let skipped: Vec<Value> = report
            .skipped
            .iter()
            .filter_map(|f| match f {
                FileOutcome::Skipped { path, reason } => {
                    Some(json!({ "path": path.to_string_lossy(), "relative": name(path), "path_lossy": lossy(path), "reason": reason.english() }))
                }
                _ => None,
            })
            .collect();
        let unlistable: Vec<Value> = report
            .unlistable
            .iter()
            .filter_map(|f| match f {
                FileOutcome::FolderUnlistable { path, error } => {
                    Some(json!({ "path": path.to_string_lossy(), "relative": name(path), "path_lossy": lossy(path), "error": error }))
                }
                _ => None,
            })
            .collect();
        let s = &report.summary;
        Ok(json!({
            "folder": folder.to_string_lossy(), "query": query, "count": s.match_count,
            "files_found": s.files_found, "files_searched": s.files_searched, "files_with_matches": files.len(), "files_skipped": s.files_skipped,
            // Files without a match are not listed, but their failed or cut pages count here.
            "complete": s.complete(), "files_capped": s.files_capped, "results_capped": s.results_capped, "listing_capped": s.listing_capped,
            "bytes_capped": s.bytes_capped, "pages_failed": s.pages_failed, "pages_truncated": s.pages_truncated,
            "files": files, "skipped_truncated": skipped.len() > MAX_SKIPPED_LISTED,
            "skipped": skipped.into_iter().take(MAX_SKIPPED_LISTED).collect::<Vec<_>>(),
            "folders_unlistable": s.folders_unlistable, "unlistable_folders": unlistable.into_iter().take(MAX_SKIPPED_LISTED).collect::<Vec<_>>(),
        }))
    }
}
