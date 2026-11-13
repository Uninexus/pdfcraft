//! Sign in with ChatGPT: lets the assistant use the user's ChatGPT plan instead of an API key.
//!
//! This follows OpenAI's published flow for open-source apps (developers.openai.com/siwc): an
//! OAuth authorization-code flow with PKCE through the system browser and a loopback listener on
//! `127.0.0.1`, then `POST https://api.openai.com/v1/responses` with the access token (streaming,
//! `store: false`). The app registers itself at the first sign-in (`dynamic_agent_client`); no
//! client secret and no API key are involved. Tokens are kept in `chatgpt.json` next to the AI
//! settings, owner-only, and never appear in logs or error messages. Nothing here reads another
//! program's tokens (the Codex CLI's, for example).
//!
//! Nothing is contacted until the user presses Sign in or asks a question with this provider.

use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{AiConfig, AiError, Message, Reply, Result, agent_with_timeout, write_private};

const AUTHORIZE_URL: &str = "https://auth.openai.com/api/accounts/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
const ISSUER: &str = "https://auth.openai.com";
const RESOURCE: &str = "https://api.openai.com/v1";
const REGISTRATION_CLIENT_ID: &str = "dynamic_agent_client";
const APP_NAME: &str = "PdfCraft";
const SCOPES: &str = "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";
/// OpenAI's example port; if it is taken, any free loopback port is tried (only the port may change).
const PREFERRED_PORT: u16 = 1455;
const CALLBACK_PATH: &str = "/auth/callback";
/// How long the browser has to finish the sign-in.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);
/// Refresh this long before the access token runs out.
const REFRESH_MARGIN_SECS: u64 = 120;
const MAX_BODY_BYTES: u64 = 8 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 64 * 1024;
/// Where a signed-in user manages plan usage.
pub const USAGE_URL: &str = "https://chatgpt.com/settings/usage";

/// What is kept on disk. Signed out, the tokens are empty but the host and client ids stay, so a
/// later sign-in reuses the registered app instead of making a new one.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Session {
    pub host_id: String,
    pub client_id: String,
    pub email: String,
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds at which the access token stops working.
    pub expires_at: u64,
    pub scopes: String,
}

// Tokens never appear in logs.
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").field("email", &self.email).field("signed_in", &self.signed_in()).finish()
    }
}

impl Session {
    pub fn signed_in(&self) -> bool {
        !self.refresh_token.is_empty() && !self.client_id.is_empty()
    }

    /// Whether the account granted the scope plan usage needs.
    pub fn plan_usage(&self) -> bool {
        self.scopes.split_whitespace().any(|s| s == PLAN_SCOPE)
    }

    /// Reads the session file; a missing file is a signed-out session.
    pub fn load(path: &Path) -> Result<Session> {
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Session::default()),
            Err(e) => return Err(AiError::Config(format!("cannot read the ChatGPT sign-in {}: {e}", path.display()))),
        };
        let mut text = String::new();
        file.take(MAX_FILE_BYTES)
            .read_to_string(&mut text)
            .map_err(|e| AiError::Config(format!("cannot read the ChatGPT sign-in {}: {e}", path.display())))?;
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| AiError::Config(format!("the ChatGPT sign-in file {} is not valid JSON: {e}", path.display())))?;
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
        Ok(Session {
            host_id: s("host_id"),
            client_id: s("client_id"),
            email: s("email"),
            access_token: s("access_token"),
            refresh_token: s("refresh_token"),
            expires_at: v.get("expires_at").and_then(Value::as_u64).unwrap_or(0),
            scopes: s("scopes"),
        })
    }

    /// Writes the session atomically, owner-only on Unix (the caller restricts it on Windows).
    pub fn save(&self, path: &Path) -> Result<()> {
        let fail = |e: std::io::Error| AiError::Config(format!("cannot save the ChatGPT sign-in {}: {e}", path.display()));
        let body = json!({
            "host_id": self.host_id, "client_id": self.client_id, "email": self.email, "access_token": self.access_token,
            "refresh_token": self.refresh_token, "expires_at": self.expires_at, "scopes": self.scopes,
        });
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
}

/// Forgets the tokens but keeps the registered ids. Calling it when signed out is fine.
pub fn sign_out(path: &Path) -> Result<()> {
    let mut s = Session::load(path)?;
    s.email.clear();
    s.access_token.clear();
    s.refresh_token.clear();
    s.expires_at = 0;
    s.scopes.clear();
    s.save(path)
}

// ---------------------------------------------------------------------------------------------
// Small encodings: base64url, percent-encoding, random ids. Kept here so the flow has no extra
// dependencies.

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn b64url(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().saturating_mul(4) / 3 + 4);
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..=chunk.len() {
            let idx = ((n >> (18 - 6 * i)) & 63) as usize;
            out.push(B64.get(idx).copied().map(char::from).unwrap_or('A'));
        }
    }
    out
}

fn b64url_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.trim_end_matches('=').bytes() {
        let v = B64.iter().position(|&b| b == c)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 2;
                    }
                    None => out.push(b),
                }
            }
            _ => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| AiError::Request(format!("no randomness for the sign-in: {e}")))?;
    Ok(b)
}

fn random_token() -> Result<String> {
    Ok(b64url(&random_bytes::<32>()?))
}

/// A random UUID (version 4) as an opaque `urn:uuid:` host id.
fn new_host_id() -> Result<String> {
    let mut b = random_bytes::<16>()?;
    if let Some(v) = b.get_mut(6) {
        *v = (*v & 0x0f) | 0x40;
    }
    if let Some(v) = b.get_mut(8) {
        *v = (*v & 0x3f) | 0x80;
    }
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!("urn:uuid:{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32]))
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v))).collect::<Vec<_>>().join("&")
}

// ---------------------------------------------------------------------------------------------
// Sign-in

/// The authorize URL for one attempt.
struct Attempt {
    url: String,
    state: String,
    verifier: String,
    nonce: String,
    redirect: String,
}

fn build_attempt(session: &Session, port: u16) -> Result<Attempt> {
    let verifier = b64url(&random_bytes::<48>()?);
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    let state = random_token()?;
    let nonce = random_token()?;
    let redirect = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
    let registering = session.client_id.is_empty();
    let client = if registering { REGISTRATION_CLIENT_ID } else { session.client_id.as_str() };
    let mut pairs: Vec<(&str, &str)> = vec![("client_id", client)];
    if registering {
        pairs.push(("agent_name_hint", APP_NAME));
    }
    pairs.push(("ext_agent_host_id", &session.host_id));
    if !session.email.is_empty() {
        pairs.push(("login_hint", &session.email));
    }
    pairs.extend([
        ("response_type", "code"),
        ("redirect_uri", redirect.as_str()),
        ("scope", SCOPES),
        ("resource", RESOURCE),
        ("state", state.as_str()),
        ("nonce", nonce.as_str()),
        ("code_challenge_method", "S256"),
        ("code_challenge", challenge.as_str()),
    ]);
    Ok(Attempt { url: format!("{AUTHORIZE_URL}?{}", form(&pairs)), state, verifier, nonce, redirect })
}

/// What the browser sent back.
#[derive(Debug, PartialEq, Eq)]
struct Callback {
    code: String,
    client_id: String,
}

/// Reads the callback request line `GET /auth/callback?... HTTP/1.1` and checks it.
fn parse_callback(request_line: &str, state: &str) -> std::result::Result<Callback, String> {
    let target = request_line.split_whitespace().nth(1).ok_or("not an HTTP request")?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != CALLBACK_PATH {
        return Err("not the sign-in callback".into());
    }
    let get = |k: &str| query.split('&').filter_map(|p| p.split_once('=')).find(|(n, _)| *n == k).map(|(_, v)| percent_decode(v));
    if get("state").as_deref() != Some(state) {
        return Err("the sign-in answer did not match this attempt (state differs)".into());
    }
    if let Some(error) = get("error") {
        let why = get("error_description").unwrap_or_default();
        return Err(if error == "access_denied" {
            "sign-in was cancelled or declined in the browser".to_owned()
        } else {
            format!("OpenAI refused the sign-in: {error} {why}")
        });
    }
    let code = get("code").filter(|c| !c.is_empty()).ok_or("the sign-in answer has no code")?;
    Ok(Callback { code, client_id: get("client_id").unwrap_or_default() })
}

const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>PdfCraft</title><body style=\"font-family:sans-serif;margin:3em\"><h2>Signed in</h2><p>You can close this tab and go back to PdfCraft.</p></body>";
const FAIL_PAGE: &str = "<!doctype html><meta charset=utf-8><title>PdfCraft</title><body style=\"font-family:sans-serif;margin:3em\"><h2>Sign-in did not finish</h2><p>Go back to PdfCraft to see why.</p></body>";

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let msg =
        format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = stream.write_all(msg.as_bytes());
}

fn bind_listener() -> Result<(TcpListener, u16)> {
    let listener = TcpListener::bind(("127.0.0.1", PREFERRED_PORT))
        .or_else(|_| TcpListener::bind(("127.0.0.1", 0)))
        .map_err(|e| AiError::Request(format!("cannot listen on this computer for the sign-in answer: {e}")))?;
    listener.set_nonblocking(true).map_err(|e| AiError::Request(e.to_string()))?;
    let port = listener.local_addr().map_err(|e| AiError::Request(e.to_string()))?.port();
    Ok((listener, port))
}

/// Waits for the browser's answer. Other requests (a favicon, a probe) are answered and ignored.
fn wait_for_callback(listener: &TcpListener, state: &str, cancel: &AtomicBool) -> Result<Callback> {
    let deadline = Instant::now() + SIGN_IN_TIMEOUT;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(AiError::Request("sign-in was cancelled".into()));
        }
        if Instant::now() > deadline {
            return Err(AiError::Request("sign-in timed out; press Sign in to try again".into()));
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut line = String::new();
                if std::io::BufReader::new((&stream).take(8192)).read_line(&mut line).is_err() {
                    continue;
                }
                match parse_callback(&line, state) {
                    Ok(cb) => {
                        respond(&mut stream, "200 OK", DONE_PAGE);
                        return Ok(cb);
                    }
                    Err(why) if why == "not the sign-in callback" || why == "not an HTTP request" => {
                        respond(&mut stream, "404 Not Found", "not found")
                    }
                    Err(why) => {
                        respond(&mut stream, "200 OK", FAIL_PAGE);
                        return Err(AiError::Request(why));
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(AiError::Request(format!("the sign-in listener failed: {e}"))),
        }
    }
}

/// Runs the whole sign-in: `open` is given the address to show in the system browser, then this
/// waits for the answer (up to five minutes, or until `cancel` is set) and stores the tokens in
/// `path`. Blocks, so call it from a worker thread.
pub fn sign_in(path: &Path, open: &dyn Fn(&str), cancel: &AtomicBool) -> Result<Session> {
    let _guard = lock();
    let mut session = Session::load(path)?;
    if session.host_id.is_empty() {
        session.host_id = new_host_id()?;
        session.save(path)?;
    }
    let (listener, port) = bind_listener()?;
    let attempt = build_attempt(&session, port)?;
    open(&attempt.url);
    let cb = wait_for_callback(&listener, &attempt.state, cancel)?;
    drop(listener);
    // A returning sign-in must come back with the same client id; a first one must return its own.
    let client_id = if session.client_id.is_empty() {
        if cb.client_id.is_empty() {
            return Err(AiError::Request("OpenAI did not return an app registration; sign-in is incomplete, try again".into()));
        }
        cb.client_id.clone()
    } else if cb.client_id.is_empty() || cb.client_id == session.client_id {
        session.client_id.clone()
    } else {
        return Err(AiError::Request("OpenAI answered with a different app registration than the one saved; sign-in refused".into()));
    };
    // Saved before the exchange so a retry does not register a new app.
    session.client_id = client_id.clone();
    session.save(path)?;
    let tokens = token_request(&form(&[
        ("grant_type", "authorization_code"),
        ("client_id", &client_id),
        ("code", &cb.code),
        ("code_verifier", &attempt.verifier),
        ("redirect_uri", &attempt.redirect),
        ("resource", RESOURCE),
    ]))
    .map_err(|f| f.error)?;
    apply_tokens(&mut session, &tokens, Some(&attempt.nonce))?;
    if !session.plan_usage() {
        session.save(path)?;
        return Err(AiError::Request(
            "you signed in, but did not allow PdfCraft to use your ChatGPT plan, so it cannot answer questions this way; sign in again and allow it, or use an API key".into(),
        ));
    }
    session.save(path)?;
    Ok(session)
}

/// A failed token request; `dead` means the refresh token can never work again.
struct TokenFail {
    dead: bool,
    error: AiError,
}

impl From<AiError> for TokenFail {
    fn from(error: AiError) -> Self {
        TokenFail { dead: false, error }
    }
}

fn token_request(body: &str) -> std::result::Result<Value, TokenFail> {
    let agent = agent_with_timeout(Duration::from_secs(30));
    let mut resp = agent
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .send(body)
        .map_err(|e| AiError::Request(format!("cannot reach OpenAI to sign in: {e}")))?;
    let status = resp.status().as_u16();
    let text = resp
        .body_mut()
        .with_config()
        .limit(MAX_BODY_BYTES)
        .read_to_string()
        .map_err(|e| AiError::Request(format!("the sign-in answer could not be read: {e}")))?;
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        let code = v.get("error").and_then(|e| e.as_str().or_else(|| e.get("code").and_then(Value::as_str))).unwrap_or("").to_owned();
        let dead = is_dead_refresh(&code);
        let kind = if dead { "your ChatGPT sign-in has expired; sign in again" } else { "OpenAI refused the sign-in request" };
        let error = AiError::Request(format!("{kind} (HTTP {status}{}{code})", if code.is_empty() { "" } else { ": " }));
        return Err(TokenFail { dead, error });
    }
    Ok(v)
}

fn is_dead_refresh(code: &str) -> bool {
    matches!(
        code,
        "invalid_grant" | "invalid_refresh_token" | "token_expired" | "refresh_token_expired" | "refresh_token_invalidated" | "refresh_token_reused"
    )
}

/// Puts a token response into the session. `nonce` is checked against the ID token when given.
fn apply_tokens(session: &mut Session, v: &Value, nonce: Option<&str>) -> Result<()> {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    let access = s("access_token");
    if access.is_empty() {
        return Err(AiError::Request("OpenAI's sign-in answer has no access token".into()));
    }
    let id_token = s("id_token");
    if !id_token.is_empty() {
        // The token came straight from OpenAI over TLS, so its claims (not its signature) are checked.
        let claims = jwt_claims(&id_token).ok_or_else(|| AiError::Request("OpenAI's ID token is unreadable".into()))?;
        check_id_claims(&claims, &session.client_id, nonce, now())?;
        if let Some(email) = claims.get("email").and_then(Value::as_str) {
            session.email = email.to_owned();
        }
    }
    session.access_token = access;
    let refresh = s("refresh_token");
    if !refresh.is_empty() {
        session.refresh_token = refresh;
    }
    session.expires_at = now().saturating_add(v.get("expires_in").and_then(Value::as_u64).unwrap_or(3600));
    let scope = s("scope");
    if !scope.is_empty() {
        session.scopes = scope;
    }
    Ok(())
}

fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    serde_json::from_slice(&b64url_decode(payload)?).ok()
}

fn check_id_claims(claims: &Value, client_id: &str, nonce: Option<&str>, now: u64) -> Result<()> {
    let bad = |what: &str| AiError::Request(format!("OpenAI's ID token was rejected: {what}"));
    if claims.get("iss").and_then(Value::as_str) != Some(ISSUER) {
        return Err(bad("wrong issuer"));
    }
    let aud_ok = match claims.get("aud") {
        Some(Value::String(a)) => a == client_id,
        Some(Value::Array(a)) => a.iter().any(|x| x.as_str() == Some(client_id)),
        _ => false,
    };
    if !aud_ok {
        return Err(bad("wrong audience"));
    }
    if claims.get("exp").and_then(Value::as_u64).is_none_or(|exp| exp < now) {
        return Err(bad("expired"));
    }
    if let Some(n) = nonce
        && claims.get("nonce").and_then(Value::as_str) != Some(n)
    {
        return Err(bad("nonce differs"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Using the plan

static REFRESH_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    REFRESH_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// A usable access token, refreshed (and the rotated refresh token saved) when it is near its end.
fn access_token(path: &Path) -> Result<String> {
    let _guard = lock();
    let mut session = Session::load(path)?;
    if !session.signed_in() {
        return Err(AiError::Config("not signed in to ChatGPT; open Preferences ▸ AI Assistant and press Sign in with ChatGPT".into()));
    }
    if !session.plan_usage() {
        return Err(AiError::Config("this ChatGPT sign-in does not allow plan usage; sign in again and allow it".into()));
    }
    if session.access_token.is_empty() || session.expires_at <= now().saturating_add(REFRESH_MARGIN_SECS) {
        let v = token_request(&form(&[
            ("grant_type", "refresh_token"),
            ("client_id", &session.client_id),
            ("refresh_token", &session.refresh_token),
            ("resource", RESOURCE),
        ]))
        .map_err(|f| {
            // A dead refresh token cannot recover: drop it so the settings say "signed out".
            if f.dead {
                let _ = sign_out_locked(path, &mut session);
            }
            f.error
        })?;
        apply_tokens(&mut session, &v, None)?;
        session.save(path)?;
    }
    Ok(session.access_token)
}

fn sign_out_locked(path: &Path, s: &mut Session) -> Result<()> {
    s.email.clear();
    s.access_token.clear();
    s.refresh_token.clear();
    s.expires_at = 0;
    s.scopes.clear();
    s.save(path)
}

fn session_path(config: &AiConfig) -> Result<&PathBuf> {
    config.chatgpt_session.as_ref().ok_or_else(|| AiError::Config("there is nowhere to keep a ChatGPT sign-in here".into()))
}

/// The models the signed-in account may use (`GET /v1/models`).
pub fn list_models(config: &AiConfig) -> Result<Vec<String>> {
    let token = access_token(session_path(config)?)?;
    let agent = agent_with_timeout(config.timeout());
    let mut resp = agent
        .get(&format!("{RESOURCE}/models"))
        .header("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| AiError::Request(format!("cannot reach OpenAI: {e}")))?;
    let status = resp.status().as_u16();
    let text = resp
        .body_mut()
        .with_config()
        .limit(MAX_BODY_BYTES)
        .read_to_string()
        .map_err(|e| AiError::Request(format!("the model list could not be read: {e}")))?;
    if !(200..300).contains(&status) {
        return Err(AiError::Request(plan_error(status, &text)));
    }
    parse_plan_models(&text)
}

/// Model slugs in a plan `models` answer (entries with `visibility: "list"`), or OpenAI's `data[].id`.
pub fn parse_plan_models(body: &str) -> Result<Vec<String>> {
    let v: Value = serde_json::from_str(body).map_err(|_| AiError::Request("the model list is not JSON".into()))?;
    let mut names: Vec<String> = match v.get("models").and_then(Value::as_array) {
        Some(list) => list
            .iter()
            .filter(|m| m.get("visibility").and_then(Value::as_str).is_none_or(|x| x == "list"))
            .filter_map(|m| m.get("slug").and_then(Value::as_str).map(str::to_owned))
            .collect(),
        None => v
            .get("data")
            .and_then(Value::as_array)
            .map(|l| l.iter().filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_owned)).collect())
            .unwrap_or_default(),
    };
    names.sort();
    names.dedup();
    Ok(names)
}

/// The Responses request body: system messages become `instructions` (system-role items are not
/// accepted), the rest become `input`, and the answer is streamed without being stored.
pub fn request_body(model: &str, messages: &[Message]) -> Value {
    let instructions: Vec<&str> = messages.iter().filter(|m| m.role == "system").map(|m| m.content.as_str()).collect();
    let input: Vec<Value> = messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            let kind = if m.role == "assistant" { "output_text" } else { "input_text" };
            json!({ "role": m.role, "content": [{ "type": kind, "text": m.content }] })
        })
        .collect();
    json!({ "model": model, "instructions": instructions.join("\n\n"), "input": input, "store": false, "stream": true })
}

/// What a streamed answer produced.
#[derive(Debug, PartialEq, Eq)]
pub struct Streamed {
    pub text: String,
}

/// Reads server-sent events until `response.completed`.
pub fn read_stream(reader: impl BufRead) -> Result<Streamed> {
    let mut text = String::new();
    let mut completed = false;
    for line in reader.lines() {
        let line = line.map_err(|e| AiError::Request(format!("the answer stream broke off: {e}")))?;
        let Some(data) = line.strip_prefix("data:") else { continue };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else { continue };
        match v.get("type").and_then(Value::as_str).unwrap_or_default() {
            "response.output_text.delta" => {
                if let Some(d) = v.get("delta").and_then(Value::as_str) {
                    text.push_str(d);
                }
            }
            "response.completed" => {
                if text.is_empty() {
                    text = final_text(&v);
                }
                completed = true;
                break;
            }
            "response.failed" | "error" => {
                let err = v.pointer("/response/error").or_else(|| v.get("error")).unwrap_or(&v);
                let code = err.get("code").and_then(Value::as_str).unwrap_or_default();
                let msg = err.get("message").and_then(Value::as_str).unwrap_or("the request failed");
                return Err(AiError::Request(code_message(code, msg)));
            }
            "response.incomplete" => {
                let why = v.pointer("/response/incomplete_details/reason").and_then(Value::as_str).unwrap_or("the answer was cut short");
                if text.is_empty() {
                    return Err(AiError::Request(format!("the answer was incomplete: {why}")));
                }
                text.push_str(&format!("\n\n[The answer was cut short: {why}]"));
                completed = true;
                break;
            }
            _ => {}
        }
    }
    if !completed {
        return Err(AiError::Request("the answer stream ended before the reply was complete".into()));
    }
    Ok(Streamed { text })
}

fn final_text(v: &Value) -> String {
    v.pointer("/response/output")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|i| i.get("content").and_then(Value::as_array))
                .flatten()
                .filter_map(|c| c.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

/// What to tell the user for OpenAI's plan error codes.
fn code_message(code: &str, message: &str) -> String {
    let hint = match code {
        "subscription_sharing_usage_limit_exceeded" => {
            format!("your ChatGPT plan's usage limit for this app is used up; see {USAGE_URL} (no reset time is known)")
        }
        "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable" => {
            "ChatGPT plan usage could not be checked right now; try again in a minute".to_owned()
        }
        "subscription_sharing_user_not_eligible" => {
            "your ChatGPT plan, workspace or admin policy does not allow plan usage for apps; use an API key instead".to_owned()
        }
        "subscription_sharing_unsupported_capability" => format!("ChatGPT plan usage does not support something in this request: {message}"),
        _ => String::new(),
    };
    if !hint.is_empty() {
        hint
    } else if code.is_empty() {
        message.to_owned()
    } else {
        format!("{message} ({code})")
    }
}

fn plan_error(status: u16, body: &str) -> String {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let code = v.pointer("/error/code").and_then(Value::as_str).unwrap_or_default();
    let message = v.pointer("/error/message").or_else(|| v.get("detail")).and_then(Value::as_str).unwrap_or_default();
    let base = code_message(code, message);
    let hint = match status {
        401 => " (sign in to ChatGPT again)",
        403 if code.is_empty() => " (not allowed; plan usage may be unavailable for this account or region)",
        429 if code.is_empty() => " (rate limit reached)",
        503 if code.is_empty() => " (temporarily unavailable)",
        _ => "",
    };
    if base.is_empty() { format!("OpenAI answered HTTP {status}{hint}") } else { format!("OpenAI answered HTTP {status}{hint}: {base}") }
}

/// Sends `messages` to the signed-in account's model and returns the streamed reply.
pub fn chat(config: &AiConfig, messages: &[Message]) -> Result<Reply> {
    config.check()?;
    let token = access_token(session_path(config)?)?;
    let body = request_body(config.model.trim(), messages).to_string();
    let agent = agent_with_timeout(config.timeout());
    let mut resp = agent
        .post(&format!("{RESOURCE}/responses"))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("Authorization", &format!("Bearer {token}"))
        .send(body.as_str())
        .map_err(|e| AiError::Request(format!("cannot reach OpenAI: {e}")))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        let text = resp.body_mut().with_config().limit(MAX_BODY_BYTES).read_to_string().unwrap_or_default();
        return Err(AiError::Request(plan_error(status, &text)));
    }
    let reader = std::io::BufReader::new(resp.body_mut().with_config().limit(MAX_BODY_BYTES).reader());
    let streamed = read_stream(reader)?;
    Ok(Reply { content: streamed.text, model: Some(config.model.trim().to_owned()) })
}

#[cfg(test)]
mod tests;
