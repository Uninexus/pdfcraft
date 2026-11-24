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

/// The system prompt, with `catalog` (one line per tool the model may call) inserted. Exactly one
/// system message is sent, because some local chat templates refuse several.
pub fn system_prompt(catalog: &str) -> String {
    format!(
        "You are the assistant inside a PDF editor and you can change the open document by calling the editor's tools.\n\
\n\
Reply with ONE JSON object and nothing else:\n\
{{\"message\": \"what you tell the user\", \"calls\": [{{\"tool\": \"page_rotate\", \"args\": {{\"pages\": [3, 4], \"degrees\": 90}}, \"why\": \"short reason\"}}]}}\n\
\n\
Rules:\n\
- Use only the tools listed below, with exactly the arguments shown. Never pass \"doc\": the open document is used.\n\
- Pages are numbered from 1. Rectangles and points are in PDF points from the top-left of the page.\n\
- Tools marked [read] only look at the document; their results come back to you in the next message so you can decide what to do. \
Tools marked [edit] change the document; the user sees the list of edit calls and confirms before any runs, and can undo them afterwards. \
Put every edit that is needed in one reply, in the order they must run, and remember that edits that remove or move pages change later page numbers.\n\
- Look before you change: when a request depends on what the document says (find an email address, a total, a name), call a [read] tool first, or use the document text you were given. Never invent text, names or numbers.\n\
- Do the smallest change that satisfies the request. Do not edit what was not asked for.\n\
- If the request is unclear, impossible with these tools, or only a question, answer in \"message\" and send an empty \"calls\" list.\n\
- When you only needed [read] tools, or when nothing more is needed, send an empty \"calls\" list and say what you did in \"message\".\n\
\n\
Tools:\n{catalog}"
    )
}

/// The first user message: the request, a description of the document and the text of its pages.
pub fn request_message(instruction: &str, document: &str, text_context: &str) -> Message {
    Message::user(format!("Document: {document}\n\nDocument text (may be incomplete or cut):\n{text_context}\nRequest: {instruction}"))
}

/// The message that returns the results of [read] calls to the model.
pub fn results_message(results: &[(String, String)]) -> Message {
    let mut text = String::from("Results of your [read] calls:\n");
    for (tool, result) in results {
        text.push_str(&format!("\n{tool} ->\n{result}\n"));
    }
    text.push_str("\nContinue: reply with one JSON object as before.");
    Message::user(text)
}

/// The message that tells the model its last reply could not be used.
pub fn retry_message(problem: &str) -> Message {
    Message::user(format!("That reply could not be used: {problem}\nReply again with one JSON object as described."))
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
