//! `ai_edit_plan` and `ai_edit_apply`: the AI assistant changes the open document on request.
//!
//! Two steps, so the user is in control. `ai_edit_plan` sends the request and the page text to the
//! configured model, which answers with tool calls from a fixed allow-list; it may look at the
//! document first (the [`READ_TOOLS`], run for real) but every change it asks for is only
//! *proposed*: the document is untouched and the list comes back for review. `ai_edit_apply` runs
//! a confirmed list: every step is checked against the allow-list and its schema before the first
//! one runs, they run in order on the one document, and if one fails the earlier ones are undone.
//! Each applied step is an ordinary undo step. Nothing is saved; the user saves as usual.
//!
//! The model never gets file, network, signing or save tools, and never names the document: the
//! open one is always used.

use pdfcraft_ai::agent::{self, PlannedCall};
use pdfcraft_ai::{AiConfig, Message, document_context};
use serde_json::{Value, json};

use super::error::{Result, ToolError, failed};
use super::json::{Args, summary};
use super::tools::{self, ToolDef};
use super::{Automation, Content};

/// Tools the model may run while planning: they only look.
pub(crate) const READ_TOOLS: &[&str] = &["doc_info", "text_extract", "text_find", "form_fields", "comment_list", "content_list", "bookmark_list"];

/// Tools the model may propose and `ai_edit_apply` may run: they change the open document in memory,
/// as an undo step each. Nothing here writes a file, signs, prints or sends anything.
pub(crate) const EDIT_TOOLS: &[&str] = &[
    "page_rotate",
    "page_delete",
    "page_move",
    "page_reverse",
    "page_duplicate",
    "page_insert_blank",
    "page_insert_file",
    "page_add_text",
    "doc_set_info",
    "text_replace",
    "doc_watermark",
    "doc_background",
    "redact_mark",
    "redact_apply",
    "redact_clear",
    "form_fill",
    "form_reset",
    "comment_add",
    "bookmark_add",
    "bookmark_rename",
    "bookmark_delete",
    "doc_flatten",
    "content_update",
    "content_delete",
];

/// Model rounds in one plan (a round is one reply; reads add a round).
const MAX_ROUNDS: usize = 5;
/// Replies the model may get wrong (unknown tool, bad arguments) before the plan fails.
const MAX_RETRIES: usize = 2;
/// Read calls run in one round.
const MAX_READS_PER_ROUND: usize = 8;
/// The most characters of one read result sent back to the model.
const MAX_RESULT_CHARS: usize = 8_000;
/// The most pages whose text goes with the request.
const MAX_PAGES: usize = 200;
/// The most steps one `ai_edit_apply` runs.
const MAX_STEPS: usize = 100;

fn is_read(tool: &str) -> bool {
    READ_TOOLS.contains(&tool)
}

fn is_edit(tool: &str) -> bool {
    EDIT_TOOLS.contains(&tool)
}

/// One line for the model: `- name(arg: type, other?: type) [edit]: what it does`.
fn brief(def: &ToolDef, kind: &str) -> String {
    let props = def.input_schema["properties"].as_object();
    let required: Vec<&str> = def.input_schema["required"].as_array().map(|r| r.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let mut params = Vec::new();
    for (name, spec) in props.into_iter().flatten().filter(|(n, _)| n.as_str() != "doc") {
        let ty = match spec.get("enum").and_then(Value::as_array) {
            Some(values) if values.len() <= 12 => values.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("|"),
            _ => match (spec.get("type").and_then(Value::as_str), spec.pointer("/items/type").and_then(Value::as_str)) {
                (Some("array"), Some(item)) => format!("[{item}]"),
                (Some(t), _) => t.to_owned(),
                _ => "any".to_owned(),
            },
        };
        let mark = if required.contains(&name.as_str()) { "" } else { "?" };
        params.push(format!("{name}{mark}: {ty}"));
    }
    let mut what: String = def.description.chars().take(300).collect();
    if let Some(end) = what.find(". ").filter(|i| *i > 40) {
        what.truncate(end + 1);
    }
    format!("- {}({}) [{kind}]: {}", def.name, params.join(", "), what.replace('\n', " "))
}

/// The tool list the model is given: built from the real schemas, so it cannot drift from what runs.
pub(crate) fn catalog() -> String {
    let mut lines = Vec::new();
    for (names, kind) in [(READ_TOOLS, "read"), (EDIT_TOOLS, "edit")] {
        for name in names {
            if let Some(def) = tools::find(name) {
                lines.push(brief(def, kind));
            }
        }
    }
    lines.join("\n")
}

/// A call the model made, checked: the tool is allowed and the arguments fit its schema, with the
/// open document filled in. `Err` says what is wrong, for the model or the user.
fn check_call(call: &PlannedCall, doc: u64, allow_read: bool) -> std::result::Result<Value, String> {
    let allowed = is_edit(&call.tool) || (allow_read && is_read(&call.tool));
    let def = tools::find(&call.tool)
        .filter(|_| allowed)
        .ok_or_else(|| format!("{} is not a tool you may call here; use only the tools listed", call.tool))?;
    let mut args = call.args.clone();
    if def.input_schema["properties"].get("doc").is_some() {
        args["doc"] = json!(doc);
    }
    tools::check_args(def, &args).map_err(|e| format!("{}: {e}", call.tool))?;
    Ok(args)
}

fn json_of(contents: Vec<Content>) -> Value {
    contents
        .into_iter()
        .find_map(|c| match c {
            Content::Json(v) => Some(v),
            _ => None,
        })
        .unwrap_or(Value::Null)
}

fn clip(text: String) -> String {
    if text.chars().count() <= MAX_RESULT_CHARS {
        return text;
    }
    let mut cut: String = text.chars().take(MAX_RESULT_CHARS).collect();
    cut.push_str("\n[result cut]");
    cut
}

impl Automation {
    /// `ai_edit_plan`: asks the model what to do for `instruction` and returns the proposed edits.
    /// Changes nothing.
    pub(crate) fn ai_edit_plan(&mut self, a: &Args) -> Result<Value> {
        let instruction = a.str("instruction")?.trim().to_owned();
        if instruction.is_empty() {
            return Err(failed("instruction is empty"));
        }
        let config: AiConfig = self.ai_config(a)?;
        config.check().map_err(failed)?;
        let doc = self.doc_to_copy(a)?;
        let (id, doc_no, page_count) = (doc.id, a.int("doc")?, doc.info.pages.len());
        let doc_no = u64::try_from(doc_no).map_err(|_| ToolError::InvalidArgs("doc must be positive".into()))?;
        let description = format!("{page_count} page(s)");
        let pages: Vec<usize> = (0..page_count.min(MAX_PAGES)).collect();
        let texts = self.page_texts(id, &pages)?;
        let all: Vec<(usize, String)> = pages.iter().zip(&texts).map(|(p, t)| (p + 1, t.plain_text())).collect();
        let ctx = document_context(&all);
        let mut text_context = String::new();
        for (page, body) in &ctx.included {
            text_context.push_str(&format!("--- Page {page} ---\n{body}\n"));
        }
        if ctx.cut || page_count > MAX_PAGES {
            text_context.push_str("[The document text was cut here to fit the size limit; use the read tools to look further.]\n");
        }
        if text_context.trim().is_empty() {
            text_context.push_str("[No text could be read from the pages.]\n");
        }

        let mut messages: Vec<Message> =
            vec![Message::system(agent::system_prompt(&catalog())), agent::request_message(&instruction, &description, &text_context)];
        let (mut retries, mut reads_done) = (0usize, Vec::<Value>::new());
        for round in 1..=MAX_ROUNDS {
            let reply = pdfcraft_ai::chat(&config, &messages).map_err(failed)?;
            let turn = agent::parse_turn(&reply.content);
            messages.push(Message::assistant(reply.content.clone()));
            let model = reply.model.clone().unwrap_or_else(|| config.model.clone());

            // Everything the model asked for must be allowed and fit its schema, or it is told and asked again.
            let mut checked: Vec<(PlannedCall, Value)> = Vec::new();
            let mut problems = Vec::new();
            for call in &turn.calls {
                match check_call(call, doc_no, true) {
                    Ok(args) => checked.push((call.clone(), args)),
                    Err(p) => problems.push(p),
                }
            }
            if !problems.is_empty() {
                retries += 1;
                if retries > MAX_RETRIES {
                    return Err(failed(format!("the model kept asking for calls that cannot run: {}", problems.join("; "))));
                }
                messages.push(agent::retry_message(&problems.join("; ")));
                continue;
            }

            let edits: Vec<_> = checked.iter().filter(|(c, _)| is_edit(&c.tool)).collect();
            if !edits.is_empty() {
                let steps: Vec<Value> = edits
                    .iter()
                    .map(|(c, args)| {
                        let mut shown = args.clone();
                        if let Some(o) = shown.as_object_mut() {
                            o.remove("doc");
                        }
                        let destructive = tools::find(&c.tool).is_some_and(|d| d.destructive);
                        json!({ "tool": c.tool, "args": shown, "why": c.why, "destructive": destructive })
                    })
                    .collect();
                return Ok(json!({
                    "message": turn.message,
                    "steps": steps,
                    "looked_at": reads_done,
                    "model": model,
                    "rounds": round,
                    "pages_sent": ctx.included.iter().map(|(p, _)| *p).collect::<Vec<_>>(),
                    "truncated": ctx.cut || page_count > MAX_PAGES,
                    "note": "A proposal only: the document has not been changed. Review the steps, then pass them to ai_edit_apply.",
                }));
            }

            let reads: Vec<_> = checked.iter().filter(|(c, _)| is_read(&c.tool)).take(MAX_READS_PER_ROUND).collect();
            if reads.is_empty() {
                return Ok(json!({
                    "message": turn.message, "steps": [], "looked_at": reads_done, "model": model, "rounds": round,
                    "pages_sent": ctx.included.iter().map(|(p, _)| *p).collect::<Vec<_>>(),
                    "truncated": ctx.cut || page_count > MAX_PAGES,
                    "note": "The model proposed no change.",
                }));
            }
            let mut results = Vec::new();
            for (call, args) in reads {
                let out = match self.call(&call.tool, args) {
                    Ok(contents) => clip(json_of(contents).to_string()),
                    Err(e) => format!("error: {e}"),
                };
                reads_done.push(json!({ "tool": call.tool, "args": call.args }));
                results.push((call.tool.clone(), out));
            }
            messages.push(agent::results_message(&results));
        }
        Err(failed("the model did not settle on a plan; try a more specific request"))
    }

    /// `ai_edit_apply`: runs a confirmed list of steps in order, all or none.
    pub(crate) fn ai_edit_apply(&mut self, a: &Args) -> Result<Value> {
        let doc = self.doc(a)?;
        let (id, doc_no) = (doc.id, u64::try_from(a.int("doc")?).map_err(|_| ToolError::InvalidArgs("doc must be positive".into()))?);
        let list = a.get("steps").and_then(Value::as_array).ok_or_else(|| ToolError::InvalidArgs("steps must be a list".into()))?;
        if list.is_empty() || list.len() > MAX_STEPS {
            return Err(ToolError::InvalidArgs(format!("steps must hold 1 to {MAX_STEPS} steps")));
        }
        // Check everything first: nothing runs unless every step is allowed and fits its schema.
        let mut steps: Vec<(String, Value)> = Vec::new();
        for (i, step) in list.iter().enumerate() {
            let tool = step.get("tool").and_then(Value::as_str).ok_or_else(|| ToolError::InvalidArgs(format!("steps[{i}] has no tool")))?;
            let args = match step.get("args") {
                Some(v @ Value::Object(_)) => v.clone(),
                None => json!({}),
                _ => return Err(ToolError::InvalidArgs(format!("steps[{i}].args must be an object"))),
            };
            let call = PlannedCall { tool: tool.to_owned(), args, why: String::new() };
            let args = check_call(&call, doc_no, false).map_err(|p| ToolError::InvalidArgs(format!("steps[{i}]: {p}")))?;
            steps.push((tool.to_owned(), args));
        }

        let mut done: Vec<Value> = Vec::new();
        let mut undo_steps = 0usize;
        for (i, (tool, args)) in steps.iter().enumerate() {
            let before = self.session.get(id).map_or(0, |d| d.undo_depth());
            match self.call(tool, args) {
                Ok(contents) => {
                    let after = self.session.get(id).map_or(0, |d| d.undo_depth());
                    undo_steps += after.saturating_sub(before);
                    done.push(json!({ "tool": tool, "result": json_of(contents) }));
                }
                Err(e) => {
                    for _ in 0..undo_steps {
                        let _ = self.session.undo(id);
                    }
                    return Err(failed(format!(
                        "step {} ({tool}) failed: {e}. The {} step(s) before it were undone; the document is as it was.",
                        i + 1,
                        i
                    )));
                }
            }
        }
        let document = self.doc(a).map(summary)?;
        Ok(json!({
            "applied": done.len(),
            "results": done,
            "undo_steps": undo_steps,
            "document": document,
            "note": "Applied in memory, not saved. edit_undo undoes them one step at a time (undo_steps says how many).",
        }))
    }
}
