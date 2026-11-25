//! `ai_edit_plan` and `ai_edit_apply`: the AI assistant changes the open document on request.
//!
//! Two steps, so the user is in control. `ai_edit_plan` sends the request and the page text to the
//! configured model, which answers with calls from the fixed list in `pdfcraft_engine::ai_edit`;
//! the list is checked against the document and *proposed*: the document is untouched. `ai_edit_apply`
//! runs a confirmed list: every call is checked again before the first runs, they run in order on the
//! one document, and if one fails the earlier ones are undone. Each applied call is an ordinary undo
//! step. Nothing is saved. The desktop app uses the same engine code.

use pdfcraft_ai::agent::{self, PlanRequest, PlannedCall};
use pdfcraft_ai::document_context;
use pdfcraft_engine::ai_edit;
use serde_json::{Value, json};

use super::Automation;
use super::error::{Result, ToolError, failed};
use super::json::{Args, summary};

/// The most pages whose text goes with the request.
const MAX_PAGES: usize = 200;

fn pairs(calls: &[PlannedCall]) -> Vec<(String, Value)> {
    calls.iter().map(|c| (c.tool.clone(), c.args.clone())).collect()
}

impl Automation {
    /// `ai_edit_plan`: asks the model what to do for `instruction` and returns the proposed calls.
    /// Changes nothing.
    pub(crate) fn ai_edit_plan(&mut self, a: &Args) -> Result<Value> {
        let instruction = a.str("instruction")?.trim().to_owned();
        if instruction.is_empty() {
            return Err(failed("instruction is empty"));
        }
        let config = self.ai_config(a)?;
        config.check().map_err(failed)?;
        let doc = self.doc_to_copy(a)?;
        let (id, page_count) = (doc.id, doc.info.pages.len());
        let shape = ai_edit::Shape::of(&self.session, id).ok_or_else(|| failed("no such document"))?;
        let pages: Vec<usize> = (0..page_count.min(MAX_PAGES)).collect();
        let texts = self.page_texts(id, &pages)?;
        let all: Vec<(usize, String)> = pages.iter().zip(&texts).map(|(p, t)| (p + 1, t.plain_text())).collect();
        let ctx = document_context(&all);
        let mut text_context = String::new();
        for (page, body) in &ctx.included {
            text_context.push_str(&format!("--- Page {page} ---\n{body}\n"));
        }
        let cut = ctx.cut || page_count > MAX_PAGES;
        if cut {
            text_context.push_str("[The document text was cut here to fit the size limit.]\n");
        }
        if text_context.trim().is_empty() {
            text_context.push_str("[No text could be read from the pages.]\n");
        }
        let description = format!("{page_count} page(s)");
        let catalog = ai_edit::catalog();
        let request = PlanRequest { instruction: &instruction, document: &description, text_context: &text_context, catalog: &catalog };
        let plan = agent::plan(&config, &request, &|calls| ai_edit::check(&shape, &pairs(calls))).map_err(failed)?;
        let steps: Vec<Value> = plan
            .calls
            .iter()
            .zip(&plan.summaries)
            .map(|(c, line)| json!({ "tool": c.tool, "args": c.args, "why": c.why, "summary": line, "destructive": ai_edit::removes_content(&c.tool) }))
            .collect();
        Ok(json!({
            "message": plan.message,
            "steps": steps,
            "model": plan.model.unwrap_or(config.model),
            "pages_sent": ctx.included.iter().map(|(p, _)| *p).collect::<Vec<_>>(),
            "truncated": cut,
            "note": if steps.is_empty() { "The model proposed no change." } else { "A proposal only: the document has not been changed. Review the steps, then pass them to ai_edit_apply." },
        }))
    }

    /// `ai_edit_apply`: runs a confirmed list of steps in order, all or none.
    pub(crate) fn ai_edit_apply(&mut self, a: &Args) -> Result<Value> {
        let id = self.doc(a)?.id;
        let list = a.get("steps").and_then(Value::as_array).ok_or_else(|| ToolError::InvalidArgs("steps must be a list".into()))?;
        if list.is_empty() || list.len() > ai_edit::MAX_STEPS {
            return Err(ToolError::InvalidArgs(format!("steps must hold 1 to {} steps", ai_edit::MAX_STEPS)));
        }
        let mut calls = Vec::new();
        for (i, step) in list.iter().enumerate() {
            let tool = step.get("tool").and_then(Value::as_str).ok_or_else(|| ToolError::InvalidArgs(format!("steps[{i}] has no tool")))?;
            let args = match step.get("args") {
                Some(v @ Value::Object(_)) => v.clone(),
                None => json!({}),
                _ => return Err(ToolError::InvalidArgs(format!("steps[{i}].args must be an object"))),
            };
            calls.push((tool.to_owned(), args));
        }
        let done = ai_edit::apply(&mut self.session, id, &calls).map_err(failed)?;
        // Added text changes the page text; the cache sees the new document version on its next request.
        Ok(json!({
            "applied": done.steps,
            "undo_steps": done.undo_steps,
            "document": summary(self.doc(a)?),
            "note": "Applied in memory, not saved. edit_undo undoes them one step at a time (undo_steps says how many).",
        }))
    }
}
