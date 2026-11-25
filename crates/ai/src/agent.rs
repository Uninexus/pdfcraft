//! The assistant's edit protocol: how a model is asked to change a document, and how its answer is read.
//!
//! Servers differ in whether and how they do function calling, and the ChatGPT-plan route does not
//! offer it in the form chat-completions servers do. So the protocol is plain text every model can
//! follow: the model answers with one JSON object, `{"message": "...", "calls": [{"tool": "...",
//! "args": {...}, "why": "..."}]}`. This module only builds the prompt and reads that object; the
//! caller (the automation crate) owns the tool table, checks every call against it and runs it.

use serde_json::Value;

use crate::Message;

/// The most calls read from one reply; the rest are ignored.
pub const MAX_CALLS: usize = 60;
/// The most characters kept of a call's reason.
pub const MAX_WHY_CHARS: usize = 240;
/// The most characters kept of the model's message to the user.
pub const MAX_MESSAGE_CHARS: usize = 4_000;

/// One tool call a model asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedCall {
    pub tool: String,
    /// The arguments object (empty when the model gave none or something that is not an object).
    pub args: Value,
    /// Why the model wants it, in a sentence, for the confirmation list.
    pub why: String,
}

/// What one model reply says: words for the user and calls to make.
#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    pub message: String,
    pub calls: Vec<PlannedCall>,
}

/// The system prompt, with `catalog` (one line per call the model may make) inserted. Exactly one
/// system message is sent, because some local chat templates refuse several.
pub fn system_prompt(catalog: &str) -> String {
    format!(
        "You are the assistant inside a PDF editor and you can change the open document by making the calls listed below.\n\
\n\
Reply with ONE JSON object and nothing else:\n\
{{\"message\": \"what you tell the user\", \"calls\": [{{\"tool\": \"rotate_pages\", \"args\": {{\"pages\": [3, 4], \"degrees\": 90}}, \"why\": \"short reason\"}}]}}\n\
\n\
Rules:\n\
- Use only the calls listed, with exactly the arguments shown. Never name a document or a file: the open document is used.\n\
- Pages are numbered from 1. Points are measured from the top-left of the page.\n\
- The user sees your list of calls and confirms before any runs, and can undo them afterwards. Put every change that is needed in one reply, in the order they must run, and remember that calls that delete, move or add pages change the page numbers of the calls after them.\n\
- Decide from the document text you are given. Never invent text, names, numbers or field names.\n\
- Do the smallest change that satisfies the request. Do not change what was not asked for.\n\
- If the request is unclear, cannot be done with these calls, or is only a question, answer in \"message\" and send an empty \"calls\" list.\n\
\n\
Calls:\n{catalog}"
    )
}

/// The first user message: the request, a description of the document and the text of its pages.
pub fn request_message(instruction: &str, document: &str, text_context: &str) -> Message {
    Message::user(format!("Document: {document}\n\nDocument text (may be incomplete or cut):\n{text_context}\nRequest: {instruction}"))
}

/// The message that tells the model its last reply could not be used.
pub fn retry_message(problem: &str) -> Message {
    Message::user(format!("That reply could not be used: {problem}\nReply again with one JSON object as described."))
}

/// What a request for edits needs.
pub struct PlanRequest<'a> {
    pub instruction: &'a str,
    /// A short description of the document (its page count).
    pub document: &'a str,
    /// The page text sent with the request.
    pub text_context: &'a str,
    /// The calls the model may make ([`system_prompt`]'s `catalog`).
    pub catalog: &'a str,
}

/// What the model proposed, checked.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// What the model tells the user.
    pub message: String,
    /// The calls, in order, each already accepted by the check.
    pub calls: Vec<PlannedCall>,
    /// One line for each call, from the check.
    pub summaries: Vec<String>,
    /// The model the server says it used.
    pub model: Option<String>,
}

/// Decides whether calls can run: one line for each when they can, the reason when they cannot.
pub type CheckFn<'a> = dyn Fn(&[PlannedCall]) -> std::result::Result<Vec<String>, String> + 'a;

/// How many times the model is asked: the first reply and two more when its calls cannot run.
pub const MAX_ATTEMPTS: usize = 3;

/// Asks the configured model for the edits that carry out `request`. `check` decides whether the
/// calls can run on the document (and says in one line what each does); when it refuses, the model
/// is told why and asked again, up to [`MAX_ATTEMPTS`] times. Nothing is changed here.
pub fn plan(config: &crate::AiConfig, request: &PlanRequest<'_>, check: &CheckFn) -> crate::Result<Plan> {
    let mut messages =
        vec![Message::system(system_prompt(request.catalog)), request_message(request.instruction, request.document, request.text_context)];
    let mut last = String::new();
    for _ in 0..MAX_ATTEMPTS {
        let reply = crate::chat(config, &messages)?;
        let turn = parse_turn(&reply.content);
        messages.push(Message::assistant(reply.content));
        match check(&turn.calls) {
            Ok(summaries) => return Ok(Plan { message: turn.message, calls: turn.calls, summaries, model: reply.model }),
            Err(problem) => {
                messages.push(retry_message(&problem));
                last = problem;
            }
        }
    }
    Err(crate::AiError::Request(format!("the model kept asking for changes that cannot be made: {last}")))
}

/// Reads a model's reply: the first JSON object in it that has `calls` or `message` (models often
/// wrap it in a code fence or a sentence). A reply with no such object is taken as the model talking
/// to the user, with no calls. Entries of `calls` that are not objects with a `tool` string are
/// dropped; whitespace in `why` is folded, and at most [`MAX_CALLS`] calls are kept.
pub fn parse_turn(reply: &str) -> Turn {
    for (i, _) in reply.match_indices('{') {
        let Some(rest) = reply.get(i..) else { continue };
        let Some(Ok(Value::Object(obj))) = serde_json::Deserializer::from_str(rest).into_iter::<Value>().next() else { continue };
        if !obj.contains_key("calls") && !obj.contains_key("message") {
            continue;
        }
        let message = obj.get("message").and_then(Value::as_str).unwrap_or("").trim();
        let mut calls = Vec::new();
        if let Some(list) = obj.get("calls").and_then(Value::as_array) {
            for item in list.iter().take(MAX_CALLS) {
                let Some(tool) = item.get("tool").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()) else { continue };
                let args = match item.get("args") {
                    Some(v @ Value::Object(_)) => v.clone(),
                    _ => Value::Object(Default::default()),
                };
                let why: String = item.get("why").and_then(Value::as_str).unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ");
                calls.push(PlannedCall { tool: tool.to_owned(), args, why: why.chars().take(MAX_WHY_CHARS).collect() });
            }
        }
        return Turn { message: message.chars().take(MAX_MESSAGE_CHARS).collect(), calls };
    }
    Turn { message: reply.trim().chars().take(MAX_MESSAGE_CHARS).collect(), calls: Vec::new() }
}
