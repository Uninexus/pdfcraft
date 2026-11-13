//! `ai_status`, `ai_ask`, `ai_summarize` and `ai_translate`: the opt-in AI assistant (misc.ai-provider).
//!
//! The endpoint comes from the call's own arguments first, then from a settings file
//! (`config_file`, honouring `--root`), then from the `PDFCRAFT_AI_BASE_URL`,
//! `PDFCRAFT_AI_MODEL` and `PDFCRAFT_AI_API_KEY` environment variables. Nothing is sent anywhere
//! unless `ai_ask` is called with an endpoint configured.

use pdfcraft_ai::{AiConfig, document_messages};
use serde_json::{Value, json};

use super::Automation;
use super::error::{Result, failed};
use super::json::Args;

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

    /// Sends the pages named by `doc` and `pages` with `question` to the configured model.
    fn ai_run(&mut self, a: &Args, question: &str) -> Result<Value> {
        let config = self.ai_config(a)?;
        config.check().map_err(failed)?;
        let doc = self.doc_to_copy(a)?;
        let id = doc.id;
        let pages = match a.opt_ints("pages")? {
            Some(_) => self.pages(a, "pages")?,
            None => (0..doc.info.pages.len()).collect(),
        };
        if pages.len() > MAX_PAGES {
            return Err(failed(format!("at most {MAX_PAGES} pages can be sent in one question; name the pages that matter in pages")));
        }
        let texts = self.page_texts(id, &pages)?;
        let sent: Vec<(usize, String)> = pages.iter().zip(&texts).map(|(p, t)| (p + 1, t.plain_text())).collect();
        let reply = pdfcraft_ai::chat(&config, &document_messages(question, &sent)).map_err(failed)?;
        Ok(json!({
            "answer": reply.content,
            "model": reply.model.unwrap_or(config.model),
            "pages_sent": sent.iter().map(|(p, _)| *p).collect::<Vec<_>>(),
        }))
    }
}
