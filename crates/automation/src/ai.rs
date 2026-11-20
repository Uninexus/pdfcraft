//! `ai_status`, `ai_ask`, `ai_summarize`, `ai_translate`, `ai_redaction_suggest` and `ai_alt_text_suggest`: the opt-in AI assistant (misc.ai-provider).
//!
//! The endpoint comes from the call's own arguments first, then from a settings file
//! (`config_file`, honouring `--root`), then from the `PDFCRAFT_AI_BASE_URL`,
//! `PDFCRAFT_AI_MODEL` and `PDFCRAFT_AI_API_KEY` environment variables. Nothing is sent anywhere
//! unless `ai_ask`, `ai_summarize`, `ai_translate`, `ai_redaction_suggest` or `ai_alt_text_suggest` is called with an
//! endpoint configured (`ai_status` with `test` only asks the server for its model list).

use pdfcraft_ai::{AiConfig, document_context, document_messages};
use serde_json::{Value, json};

use super::Automation;
use super::error::{Result, failed};
use super::json::Args;

/// What `ai_send` gives back.
struct Sent {
    reply: pdfcraft_ai::Reply,
    model: String,
    /// Every page that was selected (1-based), including any the size limit left out.
    selected: Vec<usize>,
    /// The page text that was really in the question.
    included: Vec<(usize, String)>,
    /// Whether the size limit left some text out.
    cut: bool,
}

/// The most figures `ai_alt_text_suggest` asks about in one call.
const MAX_FIGURES: usize = 100;

/// The most pages one question may send.
const MAX_PAGES: usize = 200;

impl Automation {
    fn ai_config(&self, a: &Args) -> Result<AiConfig> {
        let given = |key: &str| -> Result<Option<String>> { Ok(a.opt_str(key)?.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())) };
        let provider = match given("provider")? {
            Some(p) => pdfcraft_ai::Provider::parse(&p).ok_or_else(|| failed("provider must be \"endpoint\" or \"chatgpt\""))?,
            None => pdfcraft_ai::Provider::default(),
        };
        let chatgpt_session = given("chatgpt_session")?.map(|p| self.resolve(&p, false)).transpose()?;
        let direct = AiConfig {
            provider,
            chatgpt_session,
            base_url: given("base_url")?.unwrap_or_default(),
            api_key: given("api_key")?,
            model: given("model")?.unwrap_or_default(),
            timeout_secs: a.opt_int("timeout_secs")?.and_then(|v| u64::try_from(v).ok()).unwrap_or(0),
        };
        let from_file = match given("config_file")? {
            Some(path) => AiConfig::load(&self.resolve(&path, false)?).map_err(failed)?,
            None => AiConfig::default(),
        };
        Ok(direct.or(from_file).or(AiConfig::from_env()))
    }

    pub(crate) fn ai_status(&self, a: &Args) -> Result<Value> {
        let config = self.ai_config(a)?;
        let check = config.check();
        let mut out = json!({
            "configured": check.is_ok(),
            "provider": config.provider.as_str(),
            "base_url": config.base_url,
            "model": config.model,
            "api_key_set": config.api_key.is_some(),
        });
        match &check {
            Err(e) => out["problem"] = json!(e.to_string()),
            Ok(()) if a.opt_bool("test")?.unwrap_or(false) => match pdfcraft_ai::list_models(&config) {
                Ok(models) => {
                    out["reachable"] = json!(true);
                    out["models"] = json!(models);
                    if !models.is_empty() && !models.iter().any(|m| m == config.model.trim()) {
                        out["note"] = json!("the server does not list the configured model; check the name");
                    }
                }
                Err(e) => {
                    out["reachable"] = json!(false);
                    out["problem"] = json!(e.to_string());
                }
            },
            Ok(()) => {}
        }
        Ok(out)
    }

    pub(crate) fn ai_ask(&mut self, a: &Args) -> Result<Value> {
        let question = a.str("question")?.trim().to_owned();
        if question.is_empty() {
            return Err(failed("question is empty"));
        }
        self.ai_run(a, &question)
    }

    pub(crate) fn ai_summarize(&mut self, a: &Args) -> Result<Value> {
        self.ai_run(a, pdfcraft_ai::SUMMARY_QUESTION)
    }

    pub(crate) fn ai_translate(&mut self, a: &Args) -> Result<Value> {
        let language = a.str("language")?;
        let question = pdfcraft_ai::translate_question(language).ok_or_else(|| failed("language must be a short language name such as Spanish"))?;
        self.ai_run(a, &question)
    }

    /// `ai_redaction_suggest`: asks the model for passages worth redacting and keeps only those
    /// that really occur in the text the model was given. Marks nothing.
    pub(crate) fn ai_redaction_suggest(&mut self, a: &Args) -> Result<Value> {
        let sent = self.ai_send(a, pdfcraft_ai::REDACTION_QUESTION, None)?;
        let passages = pdfcraft_ai::parse_passages(&sent.reply.content)
            .ok_or_else(|| failed("the model did not answer with a JSON list of passages; try again or use a different model"))?;
        let squash = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ");
        // Only the text the model saw: a passage from a page it never received is not a finding.
        let texts: Vec<(usize, String)> = sent.included.iter().map(|(p, t)| (*p, squash(t))).collect();
        let (mut suggestions, mut not_in_text) = (Vec::new(), 0usize);
        for passage in &passages {
            let needle = squash(passage);
            // At most 200 passages are read and the text is at most the size limit, so this is bounded.
            let hits: Vec<(usize, usize)> = texts.iter().map(|(p, t)| (*p, t.matches(needle.as_str()).count())).filter(|(_, n)| *n > 0).collect();
            if needle.is_empty() || hits.is_empty() {
                // A model can invent or rewrite text; only what the pages hold can be marked.
                not_in_text += 1;
                continue;
            }
            suggestions.push(json!({
                "text": needle,
                "pages": hits.iter().map(|(p, _)| *p).collect::<Vec<_>>(),
                "occurrences": hits.iter().map(|(_, n)| *n).sum::<usize>(),
            }));
        }
        Ok(json!({
            "suggestions": suggestions,
            "not_in_text": not_in_text,
            "model": sent.reply.model.unwrap_or(sent.model),
            "pages_sent": sent.included.iter().map(|(p, _)| *p).collect::<Vec<_>>(),
            "truncated": sent.cut,
            "note": "Suggestions only: nothing is marked or changed. Review them, then pass the ones you accept as words to redact_mark (which matches ignoring case, so it can mark more than listed). With truncated true the model saw only part of the selected pages.",
        }))
    }

    /// `ai_alt_text_suggest`: asks the model for alternate text for the tagged figures that have none,
    /// from the text of their pages (it never sees the pictures), and keeps only what answers a figure
    /// that was asked about and whose page it was sent. Sets nothing.
    pub(crate) fn ai_alt_text_suggest(&mut self, a: &Args) -> Result<Value> {
        let wanted: Option<Vec<u32>> = a
            .opt_ints("figures")?
            .map(|v| {
                v.into_iter().map(|n| u32::try_from(n).map_err(|_| failed("figures must be figure numbers from accessibility_figures"))).collect()
            })
            .transpose()?;
        // Before the document is scanned: the work is the list's length times the figures' count.
        if wanted.as_ref().is_some_and(|w| w.len() > MAX_FIGURES) {
            return Err(failed(format!("figures names more than {MAX_FIGURES} figures; ask about at most {MAX_FIGURES} per call")));
        }
        // Resolved first, so a path that leaves the root is refused even when there is nothing to send.
        self.ai_config(a)?;
        let doc = self.doc(a)?;
        let figures = doc.figures();
        // Checked before anything is sent: a name that is not a figure, or has no page, is an error, not a silent skip.
        for n in wanted.iter().flatten() {
            match figures.iter().find(|f| f.obj.num == *n) {
                None => return Err(failed(format!("{n} is not a figure of this document (see accessibility_figures)"))),
                Some(f) if f.page.is_none() => {
                    return Err(failed(format!("figure {n} is not drawn on any page, so there is no page text to describe it from")));
                }
                Some(_) => {}
            }
        }
        // Without `figures`, those with no text; with it, those named, whatever they have now.
        let asked: Vec<(u32, usize)> = figures
            .iter()
            .filter(|f| match &wanted {
                Some(list) => list.contains(&f.obj.num),
                None => f.alt.as_deref().is_none_or(|t| t.trim().is_empty()),
            })
            .filter_map(|f| Some((f.obj.num, f.page? + 1)))
            .take(MAX_FIGURES)
            .collect();
        if asked.is_empty() {
            return Ok(json!({
                "suggestions": [], "asked": [], "unanswered": [], "ignored": 0, "not_sent": [], "pages_sent": [], "truncated": false,
                "note": "No figure to describe, so nothing was sent: the tagged figures that sit on a page all have alternate text, or the document has none.",
            }));
        }
        let mut pages: Vec<usize> = asked.iter().map(|(_, p)| p - 1).collect();
        pages.sort_unstable();
        pages.dedup();
        let sent = self.ai_send(a, &pdfcraft_ai::alt_text_question(&asked), Some(pages))?;
        let entries = pdfcraft_ai::parse_alt_texts(&sent.reply.content)
            .ok_or_else(|| failed("the model did not answer with a JSON list of figures; try again or use a different model"))?;
        let seen: Vec<usize> = sent.included.iter().map(|(p, _)| *p).collect();
        // A figure whose page the size limit left out was never described to the model.
        let (heard, not_sent): (Vec<_>, Vec<_>) = asked.iter().copied().partition(|(_, p)| seen.contains(p));
        let (mut suggestions, mut ignored) = (Vec::<Value>::new(), 0usize);
        for (figure, alt) in entries {
            let page = heard.iter().find(|(n, _)| *n == figure).map(|(_, p)| *p);
            match page {
                Some(page) if !suggestions.iter().any(|s| s["figure"] == figure) => {
                    suggestions.push(json!({ "figure": figure, "page": page, "alt": alt }))
                }
                _ => ignored += 1,
            }
        }
        let unanswered: Vec<u32> = heard.iter().map(|(n, _)| *n).filter(|n| !suggestions.iter().any(|s| s["figure"] == *n)).collect();
        Ok(json!({
            "suggestions": suggestions,
            "asked": asked.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            "unanswered": unanswered,
            "ignored": ignored,
            "not_sent": not_sent.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            "model": sent.reply.model.unwrap_or(sent.model),
            "pages_sent": seen,
            "truncated": sent.cut,
            "note": "Suggestions only: nothing is changed. The model saw the page text, not the pictures, so check each against the figure, then set the ones you accept with accessibility_set_alts (one undo step) or accessibility_set_alt.",
        }))
    }

    /// Sends the pages named by `doc` and `pages` with `question` to the configured model.
    fn ai_run(&mut self, a: &Args, question: &str) -> Result<Value> {
        let sent = self.ai_send(a, question, None)?;
        Ok(json!({
            "answer": sent.reply.content,
            "model": sent.reply.model.unwrap_or(sent.model),
            "pages_sent": sent.selected,
        }))
    }

    /// `only_pages` (0-based, already checked against the document) replaces the call's `pages`.
    fn ai_send(&mut self, a: &Args, question: &str, only_pages: Option<Vec<usize>>) -> Result<Sent> {
        let config = self.ai_config(a)?;
        config.check().map_err(failed)?;
        let doc = self.doc_to_copy(a)?;
        let id = doc.id;
        let pages = match (only_pages, a.opt_ints("pages")?) {
            (Some(pages), _) => pages,
            (None, Some(_)) => self.pages(a, "pages")?,
            (None, None) => (0..doc.info.pages.len()).collect(),
        };
        if pages.len() > MAX_PAGES {
            return Err(failed(format!("at most {MAX_PAGES} pages can be sent in one question; name the pages that matter in pages")));
        }
        let texts = self.page_texts(id, &pages)?;
        let all: Vec<(usize, String)> = pages.iter().zip(&texts).map(|(p, t)| (p + 1, t.plain_text())).collect();
        let ctx = document_context(&all);
        let reply = pdfcraft_ai::chat(&config, &document_messages(question, &all)).map_err(failed)?;
        Ok(Sent { reply, model: config.model, selected: all.iter().map(|(p, _)| *p).collect(), included: ctx.included, cut: ctx.cut })
    }
}
