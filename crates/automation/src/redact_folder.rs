//! `redact_folder`: search and redact every PDF in a folder into copies, as Redact files in a
//! folder runs it (the engine's `redact_folder`).

use pdfcraft_engine::redact_folder::{self, Options, Outcome, Target};
use pdfcraft_engine::{REDACT_MAX_WORDS, RedactPattern, redact_word_list};

use super::error::{Result, ToolError, failed};
use super::json::Args;
use super::{Automation, Path, Value, json};

impl Automation {
    pub(crate) fn redact_folder(&self, a: &Args) -> Result<Value> {
        let case_sensitive = a.opt_bool("case_sensitive")?.unwrap_or(false);
        let whole_words = a.opt_bool("whole_words")?.unwrap_or(false);
        let target = match (a.get("words"), a.opt_str("pattern")?) {
            (Some(_), None) => {
                let raw = a.strs("words")?;
                if raw.len() > REDACT_MAX_WORDS {
                    return Err(ToolError::InvalidArgs(format!("words has {} entries; at most {REDACT_MAX_WORDS} are allowed", raw.len())));
                }
                let words = redact_word_list(&raw.join("\n"));
                if words.is_empty() {
                    return Err(Args::wrong("words", "at least one non-empty word or phrase"));
                }
                Target::Words { words, case_sensitive, whole_words }
            }
            (None, Some(id)) => {
                if case_sensitive || whole_words {
                    return Err(ToolError::InvalidArgs("case_sensitive and whole_words apply only to words".into()));
                }
                Target::Pattern(RedactPattern::from_id(id).ok_or_else(|| ToolError::InvalidArgs(format!("unknown pattern {id:?}")))?)
            }
            _ => return Err(ToolError::InvalidArgs("pass exactly one of words or pattern".into())),
        };
        let requested = a.str("folder")?;
        let out = a.str("out")?;
        if requested.is_empty() || out.is_empty() {
            return Err(ToolError::InvalidArgs("folder and out must be folder paths".into()));
        }
        let folder = self.resolve(requested, false)?;
        if !folder.is_dir() {
            return Err(failed(format!("{requested} is not a folder: pass the folder that holds the PDFs")));
        }
        let options = Options { target, recursive: a.opt_bool("recursive")?.unwrap_or(false), output: self.resolve(out, true)? };
        let name = |path: &Path| path.strip_prefix(&folder).unwrap_or(path).to_string_lossy().replace('\\', "/");
        let numbered = |pages: &[usize]| pages.iter().map(|p| p.saturating_add(1)).collect::<Vec<_>>();
        let mut files = Vec::new();
        let summary = redact_folder::redact_folder(&folder, &options, &|| false, &mut |_, file| {
            let mut entry = match &file.outcome {
                Outcome::Redacted { output, matches, still_holding, unsearched } => json!({ "outcome": "redacted",
                    "output": output.to_string_lossy(), "matches": matches, "still_holding": still_holding, "unsearched_pages": numbered(unsearched) }),
                Outcome::NoMatch { unsearched } => json!({ "outcome": "no_match", "unsearched_pages": numbered(unsearched) }),
                Outcome::Skipped(reason) => json!({ "outcome": "skipped", "reason": reason }),
            };
            entry["file"] = json!(name(&file.path));
            files.push(entry);
        })
        .map_err(failed)?;
        Ok(json!({
            "files_found": summary.files_found,
            "redacted": summary.redacted,
            "no_match": summary.no_match,
            "skipped": summary.skipped,
            "capped": summary.capped,
            "files": files,
        }))
    }
}
