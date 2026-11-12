use std::io::{Read, Write};
use std::net::TcpListener;

use super::*;

/// Serves one HTTP request with `status` and `body`; returns the base URL and the raw request.
fn serve_once(status: &'static str, body: &'static str) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://{}/v1", listener.local_addr().expect("addr"));
    let handle = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().expect("accept");
        let mut req = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = s.read(&mut buf).expect("read");
            req.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&req).into_owned();
            if let Some(split) = text.find("\r\n\r\n") {
                let len = text[..split]
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()))
                    .unwrap_or(0);
                if req.len() >= split + 4 + len {
                    break;
                }
            }
            if n == 0 {
                break;
            }
        }
        let reply =
            format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        s.write_all(reply.as_bytes()).expect("write");
        String::from_utf8_lossy(&req).into_owned()
    });
    (url, handle)
}

fn cfg(url: &str) -> AiConfig {
    AiConfig { base_url: url.into(), api_key: Some("sk-secret".into()), model: "m1".into(), timeout_secs: 10 }
}

#[test]
fn chat_posts_to_chat_completions_with_key_and_reads_the_reply() {
    let (url, req) = serve_once("200 OK", r#"{"model":"m1-2026","choices":[{"message":{"role":"assistant","content":"Hello"}}]}"#);
    let reply = chat(&cfg(&url), &[Message::user("hi")]).expect("reply");
    assert_eq!(reply, Reply { content: "Hello".into(), model: Some("m1-2026".into()) });
    let req = req.join().expect("server");
    assert!(req.starts_with("POST /v1/chat/completions "), "{req}");
    assert!(req.to_ascii_lowercase().contains("authorization: bearer sk-secret"), "{req}");
    assert!(req.contains(r#""model":"m1""#) && req.contains(r#""content":"hi""#), "{req}");
}

#[test]
fn no_key_sends_no_authorization_header() {
    let (url, req) = serve_once("200 OK", r#"{"choices":[{"message":{"content":"ok"}}]}"#);
    let mut c = cfg(&url);
    c.api_key = None;
    chat(&c, &[Message::user("hi")]).expect("reply");
    assert!(!req.join().expect("server").to_ascii_lowercase().contains("authorization"));
}

#[test]
fn http_errors_say_what_to_check() {
    let (url, _h) = serve_once("401 Unauthorized", r#"{"error":{"message":"Incorrect API key provided"}}"#);
    let e = chat(&cfg(&url), &[Message::user("hi")]).expect_err("401").to_string();
    assert!(e.contains("401") && e.contains("API key") && e.contains("Incorrect API key"), "{e}");
    assert!(!e.contains("sk-secret"));
    let (url, _h) = serve_once("404 Not Found", "{}");
    let e = chat(&cfg(&url), &[Message::user("hi")]).expect_err("404").to_string();
    assert!(e.contains("404") && e.contains("model"), "{e}");
}

#[test]
fn unreachable_server_is_an_error_not_a_panic() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://{}/v1", listener.local_addr().expect("addr"));
    drop(listener);
    let e = chat(&cfg(&url), &[Message::user("hi")]).expect_err("refused").to_string();
    assert!(e.contains("cannot reach"), "{e}");
}

#[test]
fn list_models_reads_openai_and_ollama_shapes() {
    let (url, _h) = serve_once("200 OK", r#"{"data":[{"id":"b"},{"id":"a"}]}"#);
    assert_eq!(list_models(&cfg(&url)).expect("models"), vec!["a", "b"]);
    assert_eq!(parse_models(r#"{"models":[{"name":"llama3.1:8b"}]}"#).expect("ollama"), vec!["llama3.1:8b"]);
}

#[test]
fn replies_in_parts_or_errors_in_a_200_are_handled() {
    let r = parse_reply(r#"{"choices":[{"message":{"content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]}}]}"#).expect("parts");
    assert_eq!(r.content, "ab");
    assert!(parse_reply(r#"{"error":{"message":"overloaded"}}"#).expect_err("err").to_string().contains("overloaded"));
    assert!(parse_reply("<html>").expect_err("html").to_string().contains("OpenAI-compatible"));
    assert!(parse_reply(r#"{"choices":[]}"#).is_err());
}

#[test]
fn endpoint_accepts_pasted_full_addresses_and_rejects_bad_ones() {
    let mut c = cfg("http://localhost:11434/v1/");
    assert_eq!(c.endpoint("chat/completions").expect("url"), "http://localhost:11434/v1/chat/completions");
    c.base_url = "https://api.openai.com/v1/chat/completions".into();
    assert_eq!(c.endpoint("models").expect("url"), "https://api.openai.com/v1/models");
    for bad in ["", "   ", "localhost:11434", "ftp://x/v1", "http://a b/v1"] {
        c.base_url = bad.into();
        assert!(matches!(c.check(), Err(AiError::Config(_))), "{bad:?}");
    }
    c.base_url = "http://localhost:1234/v1".into();
    c.model = " ".into();
    assert!(c.check().expect_err("model").to_string().contains("model"));
}

#[test]
fn debug_hides_the_key() {
    let shown = format!("{:?}", cfg("http://x/v1"));
    assert!(!shown.contains("sk-secret") && shown.contains("<set>"));
}

#[test]
fn settings_round_trip_and_merge() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("sub").join("ai.json");
    assert_eq!(AiConfig::load(&path).expect("missing"), AiConfig::default());
    cfg("http://localhost:11434/v1").save(&path).expect("save");
    assert_eq!(AiConfig::load(&path).expect("load"), cfg("http://localhost:11434/v1"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).expect("meta").permissions().mode() & 0o077, 0);
    }
    std::fs::write(&path, "not json").expect("write");
    assert!(AiConfig::load(&path).expect_err("bad").to_string().contains("not valid JSON"));

    let env = AiConfig::from_vars(|n| match n {
        "PDFCRAFT_AI_MODEL" => Some("env-model".into()),
        "PDFCRAFT_AI_API_KEY" => Some("env-key".into()),
        "OPENAI_API_KEY" => Some("must-not-be-read".into()),
        _ => None,
    });
    let merged = AiConfig { base_url: "http://h/v1".into(), ..Default::default() }.or(env);
    assert_eq!((merged.model.as_str(), merged.api_key.as_deref()), ("env-model", Some("env-key")));
}

#[test]
fn document_messages_cut_long_text_on_a_char_boundary() {
    let long = "é".repeat(MAX_CONTEXT_CHARS + 10);
    let m = document_messages("what?", &[(1, "short page".into()), (2, long)]);
    assert_eq!(m[0].role, "system");
    assert!(m[1].content.contains("--- Page 1 ---\nshort page") && m[1].content.contains("cut here") && m[1].content.ends_with("Question: what?"));
    let empty = document_messages("q", &[(1, String::new())]);
    assert!(empty[1].content.contains("--- Page 1 ---"));
    assert!(document_messages("q", &[])[1].content.contains("No text could be read"));
}
