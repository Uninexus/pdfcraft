//! pdfcraft-ai: the opt-in AI assistant client (L4).
//!
//! Talks to any server that speaks the OpenAI chat-completions protocol: a local model server
//! (Ollama, LM Studio, llama.cpp, vLLM, ...) or a hosted API. Nothing is sent anywhere until the
//! user has set a base URL and a model and asks a question; there is no default endpoint and no
//! built-in key. The document text a question needs is sent to the configured endpoint and to
//! nowhere else.
//!
//! The crate is plain data in, plain data out: [`AiConfig`] says where to talk, [`chat`] makes one
//! request, [`document_messages`] builds the messages for a question about pages of a document.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::path::Path;

use serde_json::{Value, json};

/// The longest document excerpt sent with one question, in characters. Longer text is cut (at a
/// character boundary) and the model is told so.
pub const MAX_CONTEXT_CHARS: usize = 60_000;
/// The longest reply read from a server, in bytes.
const MAX_REPLY_BYTES: u64 = 8 * 1024 * 1024;
/// The longest settings file read, in bytes.
const MAX_CONFIG_BYTES: u64 = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    /// The settings are missing or unusable; the message says what to set.
    #[error("{0}")]
    Config(String),
    /// The server could not be reached, refused the request or sent something unreadable.
    #[error("{0}")]
    Request(String),
}

pub type Result<T> = std::result::Result<T, AiError>;

/// Where the assistant sends its questions.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct AiConfig {
    /// The API root, such as `http://localhost:11434/v1` or `https://api.openai.com/v1`. A pasted
    /// `.../chat/completions` address is accepted too.
    pub base_url: String,
    /// The bearer token. Local servers usually need none.
    pub api_key: Option<String>,
    /// The model name the server knows, such as `llama3.1` or `gpt-4.1-mini`.
    pub model: String,
    /// How long one request may take. Zero means [`AiConfig::DEFAULT_TIMEOUT_SECS`].
    pub timeout_secs: u64,
}

// The key never appears in logs or error messages.
impl std::fmt::Debug for AiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<set>"))
            .field("model", &self.model)
            .field("timeout_secs", &self.timeout_secs)
            .finish()
    }
}

impl AiConfig {
    pub const DEFAULT_TIMEOUT_SECS: u64 = 120;

    /// Settings from `PDFCRAFT_AI_BASE_URL`, `PDFCRAFT_AI_MODEL` and `PDFCRAFT_AI_API_KEY`
    /// (`OPENAI_API_KEY` when that one is not set). Unset variables leave the field empty.
    pub fn from_env() -> AiConfig {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    fn from_vars(var: impl Fn(&str) -> Option<String>) -> AiConfig {
        let get = |name: &str| var(name).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        AiConfig {
            base_url: get("PDFCRAFT_AI_BASE_URL").unwrap_or_default(),
            api_key: get("PDFCRAFT_AI_API_KEY").or_else(|| get("OPENAI_API_KEY")),
            model: get("PDFCRAFT_AI_MODEL").unwrap_or_default(),
            timeout_secs: get("PDFCRAFT_AI_TIMEOUT_SECS").and_then(|v| v.parse().ok()).unwrap_or(0),
        }
    }

    /// Fields set here win; empty ones fall back to `other`.
    pub fn or(self, other: AiConfig) -> AiConfig {
        AiConfig {
            base_url: if self.base_url.trim().is_empty() { other.base_url } else { self.base_url },
            api_key: self.api_key.filter(|k| !k.trim().is_empty()).or(other.api_key),
            model: if self.model.trim().is_empty() { other.model } else { self.model },
            timeout_secs: if self.timeout_secs == 0 { other.timeout_secs } else { self.timeout_secs },
        }
    }

    /// Reads a settings file written by [`AiConfig::save`]. A missing file is empty settings; an
    /// unreadable or malformed one is an error that names the file.
    pub fn load(path: &Path) -> Result<AiConfig> {
        use std::io::Read;
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(AiConfig::default()),
            Err(e) => return Err(AiError::Config(format!("cannot read AI settings {}: {e}", path.display()))),
        };
        let mut text = String::new();
        file.take(MAX_CONFIG_BYTES)
            .read_to_string(&mut text)
            .map_err(|e| AiError::Config(format!("cannot read AI settings {}: {e}", path.display())))?;
        let v: Value = serde_json::from_str(&text).map_err(|e| AiError::Config(format!("AI settings {} are not valid JSON: {e}", path.display())))?;
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        Ok(AiConfig {
            base_url: s("base_url").unwrap_or_default(),
            api_key: s("api_key"),
            model: s("model").unwrap_or_default(),
            timeout_secs: v.get("timeout_secs").and_then(Value::as_u64).unwrap_or(0),
        })
    }

    /// Writes the settings atomically. On Unix the file is readable by its owner only, because it
    /// can hold an API key.
    pub fn save(&self, path: &Path) -> Result<()> {
        let fail = |e: std::io::Error| AiError::Config(format!("cannot save AI settings {}: {e}", path.display()));
        let body = json!({ "base_url": self.base_url, "api_key": self.api_key, "model": self.model, "timeout_secs": self.timeout_secs });
        let text = serde_json::to_string_pretty(&body).map_err(|e| AiError::Config(e.to_string()))?;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(fail)?;
        }
        let tmp = path.with_extension("json.tmp");
        write_private(&tmp, text.as_bytes()).map_err(fail)?;
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            fail(e)
        })
    }

    /// Checks that the settings can make a request, with a message that says what to fix.
    pub fn check(&self) -> Result<()> {
        self.endpoint("chat/completions").map(|_| ())?;
        if self.model.trim().is_empty() {
            return Err(AiError::Config("no AI model is set; give the model name your server uses (for example llama3.1 or gpt-4.1-mini)".into()));
        }
        Ok(())
    }

    /// The URL of `leaf` (`chat/completions` or `models`) under the base URL.
    fn endpoint(&self, leaf: &str) -> Result<String> {
        let base = self.base_url.trim();
        if base.is_empty() {
            return Err(AiError::Config(
                "no AI endpoint is set; give the base URL of an OpenAI-compatible server (for example http://localhost:11434/v1 for Ollama)".into(),
            ));
        }
        if !(base.starts_with("http://") || base.starts_with("https://")) || base.contains(char::is_whitespace) {
            return Err(AiError::Config(format!("the AI endpoint {base:?} must be an http:// or https:// address")));
        }
        let mut root = base.trim_end_matches('/');
        for tail in ["/chat/completions", "/models"] {
            if let Some(r) = root.strip_suffix(tail) {
                root = r.trim_end_matches('/');
            }
        }
        Ok(format!("{root}/{leaf}"))
    }

    fn timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(if self.timeout_secs == 0 { Self::DEFAULT_TIMEOUT_SECS } else { self.timeout_secs })
    }
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// `system`, `user` or `assistant`.
    pub role: &'static str,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Message {
        Message { role: "system", content: content.into() }
    }
    pub fn user(content: impl Into<String>) -> Message {
        Message { role: "user", content: content.into() }
    }
    pub fn assistant(content: impl Into<String>) -> Message {
        Message { role: "assistant", content: content.into() }
    }
}

/// What the server answered.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub content: String,
    /// The model the server says it used, when it says.
    pub model: Option<String>,
}

/// The request body for `chat/completions`.
pub fn request_body(model: &str, messages: &[Message]) -> Value {
    json!({
        "model": model,
        "messages": messages.iter().map(|m| json!({ "role": m.role, "content": m.content })).collect::<Vec<_>>(),
        "stream": false,
    })
}

/// Reads the reply out of a `chat/completions` response body.
pub fn parse_reply(body: &str) -> Result<Reply> {
    let v: Value = serde_json::from_str(body)
        .map_err(|_| AiError::Request("the server's answer is not JSON; is the base URL an OpenAI-compatible /v1 address?".into()))?;
    if let Some(msg) = error_message(&v) {
        return Err(AiError::Request(format!("the server reported an error: {msg}")));
    }
    let content = v.pointer("/choices/0/message/content").and_then(|c| match c {
        Value::String(s) => Some(s.clone()),
        // Some servers return a list of typed parts.
        Value::Array(parts) => Some(parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("")),
        _ => None,
    });
    match content {
        Some(content) => Ok(Reply { content, model: v.get("model").and_then(Value::as_str).map(str::to_owned) }),
        None => Err(AiError::Request("the server's answer has no reply text (no choices[0].message.content)".into())),
    }
}

fn error_message(v: &Value) -> Option<String> {
    let e = v.get("error")?;
    Some(e.get("message").and_then(Value::as_str).or_else(|| e.as_str()).unwrap_or("unknown error").to_owned())
}

/// What to tell the user for an HTTP failure.
fn status_message(status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<Value>(body).ok().and_then(|v| error_message(&v)).unwrap_or_default();
    let hint = match status {
        401 | 403 => " (check the API key)",
        404 => " (check the base URL and the model name)",
        429 => " (rate limit or quota reached)",
        _ => "",
    };
    if detail.is_empty() { format!("the server answered HTTP {status}{hint}") } else { format!("the server answered HTTP {status}{hint}: {detail}") }
}

/// Sends `messages` to the configured model and returns its reply.
#[cfg(not(target_arch = "wasm32"))]
pub fn chat(config: &AiConfig, messages: &[Message]) -> Result<Reply> {
    config.check()?;
    let url = config.endpoint("chat/completions")?;
    let body = request_body(config.model.trim(), messages).to_string();
    let agent = agent(config);
    let mut req = agent.post(&url).header("Content-Type", "application/json");
    if let Some(key) = config.api_key.as_deref().filter(|k| !k.is_empty()) {
        req = req.header("Authorization", &format!("Bearer {key}"));
    }
    let mut resp = req.send(body.as_str()).map_err(|e| transport_error(&url, &e))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().with_config().limit(MAX_REPLY_BYTES).read_to_string().map_err(|e| transport_error(&url, &e))?;
    if !(200..300).contains(&status) {
        return Err(AiError::Request(status_message(status, &text)));
    }
    parse_reply(&text)
}

/// The model names the server lists (`GET /models`), for a settings screen's drop-down or a
/// connection test. Servers that do not list models give an error the caller may ignore.
#[cfg(not(target_arch = "wasm32"))]
pub fn list_models(config: &AiConfig) -> Result<Vec<String>> {
    let url = config.endpoint("models")?;
    let agent = agent(config);
    let mut req = agent.get(&url);
    if let Some(key) = config.api_key.as_deref().filter(|k| !k.is_empty()) {
        req = req.header("Authorization", &format!("Bearer {key}"));
    }
    let mut resp = req.call().map_err(|e| transport_error(&url, &e))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().with_config().limit(MAX_REPLY_BYTES).read_to_string().map_err(|e| transport_error(&url, &e))?;
    if !(200..300).contains(&status) {
        return Err(AiError::Request(status_message(status, &text)));
    }
    parse_models(&text)
}

/// The model ids in a `GET /models` body (OpenAI's `data[].id`, or Ollama's `models[].name`).
pub fn parse_models(body: &str) -> Result<Vec<String>> {
    let v: Value = serde_json::from_str(body).map_err(|_| AiError::Request("the server's model list is not JSON".into()))?;
    let list = |key: &str, field: &str| -> Option<Vec<String>> {
        Some(v.get(key)?.as_array()?.iter().filter_map(|m| m.get(field).and_then(Value::as_str).map(str::to_owned)).collect())
    };
    let mut names = list("data", "id").or_else(|| list("models", "name")).unwrap_or_default();
    names.sort();
    names.dedup();
    Ok(names)
}

#[cfg(not(target_arch = "wasm32"))]
fn agent(config: &AiConfig) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(config.timeout())).http_status_as_error(false).build().into()
}

#[cfg(not(target_arch = "wasm32"))]
fn transport_error(url: &str, e: &ureq::Error) -> AiError {
    use ureq::Error;
    match e {
        Error::Timeout(_) => {
            AiError::Request(format!("the AI server at {url} did not answer in time; a local model may still be loading, or raise the timeout"))
        }
        Error::ConnectionFailed | Error::HostNotFound | Error::Io(_) => {
            AiError::Request(format!("cannot reach the AI server at {url}; check that it is running and the address is right ({e})"))
        }
        other => AiError::Request(format!("the AI request to {url} failed: {other}")),
    }
}

/// The web build cannot reach arbitrary servers from here.
#[cfg(target_arch = "wasm32")]
pub fn chat(_config: &AiConfig, _messages: &[Message]) -> Result<Reply> {
    Err(AiError::Request("the AI assistant is not available in the web build".into()))
}

#[cfg(target_arch = "wasm32")]
pub fn list_models(_config: &AiConfig) -> Result<Vec<String>> {
    Err(AiError::Request("the AI assistant is not available in the web build".into()))
}

const SYSTEM_PROMPT: &str = "You are the assistant inside a PDF editor. Answer the user's question using the document text provided. \
Cite page numbers like (p. 3) when you rely on a page. If the text does not contain the answer, say so instead of guessing. \
The text may be incomplete: scanned pages without OCR have none.";

/// Messages for a question about pages of a document: a system prompt, the text of each page
/// (`(1-based page, text)`), cut to [`MAX_CONTEXT_CHARS`], and the question.
pub fn document_messages(question: &str, pages: &[(usize, String)]) -> Vec<Message> {
    let mut context = String::new();
    let mut used = 0usize;
    let mut cut = false;
    for (page, text) in pages {
        let head = format!("--- Page {page} ---\n");
        let room = MAX_CONTEXT_CHARS.saturating_sub(used);
        if room <= head.chars().count() {
            cut = true;
            break;
        }
        let body_room = room - head.chars().count();
        let body: String = text.chars().take(body_room).collect();
        if body.chars().count() < text.chars().count() {
            cut = true;
        }
        used += head.chars().count() + body.chars().count() + 1;
        context.push_str(&head);
        context.push_str(&body);
        context.push('\n');
        if cut {
            break;
        }
    }
    if cut {
        context.push_str("[The document text was cut here to fit the size limit.]\n");
    }
    if context.trim().is_empty() {
        context.push_str("[No text could be read from the selected pages.]\n");
    }
    vec![Message::system(SYSTEM_PROMPT), Message::user(format!("Document text:\n{context}\nQuestion: {question}"))]
}

#[cfg(test)]
mod tests;
