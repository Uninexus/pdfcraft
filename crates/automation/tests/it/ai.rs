//! `ai_status` and `ai_ask` end to end against a local stand-in for an OpenAI-compatible server.

use std::io::{Read, Write};
use std::net::TcpListener;

use pdfcraft_automation::{Automation, Content};
use serde_json::{Value, json};

fn pdf() -> Vec<u8> {
    pdf_with("BT /F1 24 Tf 20 40 Td (Invoice total 42) Tj ET")
}

fn pdf_with(c: &str) -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 600 4000] >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >> >>"
            .to_string(),
        format!("<< /Length {} >>\nstream\n{c}\nendstream", c.len()),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offs.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let x = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offs {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn server(body: &'static str) -> (String, std::thread::JoinHandle<String>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", l.local_addr().unwrap());
    let h = std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let (mut req, mut buf) = (Vec::new(), [0u8; 8192]);
        loop {
            let n = s.read(&mut buf).unwrap();
            req.extend_from_slice(&buf[..n]);
            let t = String::from_utf8_lossy(&req).into_owned();
            if let Some(i) = t.find("\r\n\r\n") {
                let len = t[..i]
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()))
                    .unwrap_or(0);
                if req.len() >= i + 4 + len {
                    break;
                }
            }
            if n == 0 {
                break;
            }
        }
        let r = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        s.write_all(r.as_bytes()).unwrap();
        String::from_utf8_lossy(&req).into_owned()
    });
    (url, h)
}

fn json_of(r: Vec<Content>) -> Value {
    match r.into_iter().next() {
        Some(Content::Json(v)) => v,
        other => panic!("expected JSON, got {other:?}"),
    }
}

#[test]
fn ask_sends_the_page_text_and_returns_the_answer() {
    let dir = std::env::temp_dir().join(format!("pdfcraft-automation-ai-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.pdf"), pdf()).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap();
    let doc = json_of(a.call("doc_open", &json!({ "path": "a.pdf" })).unwrap())["doc"].as_u64().unwrap();

    let (url, req) = server(r#"{"model":"local","choices":[{"message":{"content":"The total is 42."}}]}"#);
    let out = json_of(a.call("ai_ask", &json!({ "doc": doc, "question": "What is the total?", "base_url": url, "model": "local" })).unwrap());
    assert_eq!(out["answer"], "The total is 42.");
    assert_eq!(out["pages_sent"], json!([1]));
    let req = req.join().unwrap();
    assert!(req.contains("Invoice total 42") && req.contains("What is the total?"), "{req}");
}

#[test]
fn unconfigured_ask_fails_clearly_and_status_hides_the_key() {
    let mut a = Automation::new();
    let s = json_of(a.call("ai_status", &json!({ "base_url": "http://localhost:1/v1", "api_key": "sk-secret" })).unwrap());
    assert_eq!(s["api_key_set"], true);
    assert_eq!(s["configured"], false);
    assert!(!s.to_string().contains("sk-secret"));
    let e = a.call("ai_ask", &json!({ "doc": 1, "question": "hi", "model": "m" })).unwrap_err().to_string();
    assert!(e.contains("endpoint") || e.contains("base URL"), "{e}");
}

#[test]
fn summarize_and_translate_send_their_instruction_with_the_page_text() {
    let dir = std::env::temp_dir().join(format!("pdfcraft-automation-ai-st-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.pdf"), pdf()).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap();
    let doc = json_of(a.call("doc_open", &json!({ "path": "a.pdf" })).unwrap())["doc"].as_u64().unwrap();

    let (url, req) = server(r#"{"choices":[{"message":{"content":"Short."}}]}"#);
    let out = json_of(a.call("ai_summarize", &json!({ "doc": doc, "base_url": url, "model": "m" })).unwrap());
    assert_eq!(out["answer"], "Short.");
    let req = req.join().unwrap();
    assert!(req.contains("Summarize this document") && req.contains("Invoice total 42"), "{req}");

    let (url, req) = server(r#"{"choices":[{"message":{"content":"Total de la factura 42"}}]}"#);
    let out = json_of(a.call("ai_translate", &json!({ "doc": doc, "language": "Spanish", "base_url": url, "model": "m" })).unwrap());
    assert_eq!(out["answer"], "Total de la factura 42");
    assert!(req.join().unwrap().contains("into Spanish"));

    let e =
        a.call("ai_translate", &json!({ "doc": doc, "language": "  ", "base_url": "http://127.0.0.1:1/v1", "model": "m" })).unwrap_err().to_string();
    assert!(e.contains("language"), "{e}");
}

#[test]
fn redaction_suggestions_keep_only_passages_that_are_in_the_text_and_mark_nothing() {
    let dir = std::env::temp_dir().join(format!("pdfcraft-automation-ai-rs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.pdf"), pdf()).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap();
    let doc = json_of(a.call("doc_open", &json!({ "path": "a.pdf" })).unwrap())["doc"].as_u64().unwrap();
    let before = json_of(a.call("doc_info", &json!({ "doc": doc })).unwrap());

    // The model names one passage the page holds, one it invented, one twice and a non-string.
    let (url, req) = server(r#"{"choices":[{"message":{"content":"```json\n[\"Invoice total 42\", \"Nobody Here\", \"42\", \"42\", 7]\n```"}}]}"#);
    let out = json_of(a.call("ai_redaction_suggest", &json!({ "doc": doc, "base_url": url, "model": "m" })).unwrap());
    assert_eq!(
        out["suggestions"],
        json!([{ "text": "Invoice total 42", "pages": [1], "occurrences": 1 }, { "text": "42", "pages": [1], "occurrences": 1 }])
    );
    assert_eq!(out["not_in_text"], 1);
    assert_eq!(out["pages_sent"], json!([1]));
    let req = req.join().unwrap();
    assert!(req.contains("redact") && req.contains("Invoice total 42"), "{req}");
    assert_eq!(json_of(a.call("doc_info", &json!({ "doc": doc })).unwrap()), before, "suggesting changes nothing");

    // A reply that is not a list is an error that says so, not an empty result.
    let (url, req) = server(r#"{"choices":[{"message":{"content":"I cannot help with that."}}]}"#);
    let e = a.call("ai_redaction_suggest", &json!({ "doc": doc, "base_url": url, "model": "m" })).unwrap_err().to_string();
    assert!(e.contains("JSON list"), "{e}");
    req.join().unwrap();

    let e = a
        .call("ai_redaction_suggest", &json!({ "doc": doc, "model": "m", "base_url": "http://127.0.0.1:1/v1", "pages": [9] }))
        .unwrap_err()
        .to_string();
    assert!(e.contains("page 9 is out of range"), "{e}");
}

#[test]
fn redaction_suggestions_only_count_text_the_model_was_given() {
    // One page of about 70,000 characters: the limit is 60,000, so the last line is never sent.
    let mut c = String::from("BT /F1 4 Tf 5 TL 5 3990 Td (Jane Doe) '\n");
    for _ in 0..700 {
        c.push_str(&format!("({}) '\n", "lorem ipsum ".repeat(8)));
    }
    c.push_str("(Late Person) ' ET");
    let bytes = pdf_with(&c);
    let dir = std::env::temp_dir().join(format!("pdfcraft-automation-ai-rt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("big.pdf"), bytes).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap();
    let doc = json_of(a.call("doc_open", &json!({ "path": "big.pdf" })).unwrap())["doc"].as_u64().unwrap();

    let (url, req) = server(r#"{"choices":[{"message":{"content":"[\"Jane Doe\", \"Late Person\"]"}}]}"#);
    let out = json_of(a.call("ai_redaction_suggest", &json!({ "doc": doc, "base_url": url, "model": "m" })).unwrap());
    let req = req.join().unwrap();
    assert!(req.contains("Jane Doe") && !req.contains("Late Person"), "the last line must not be sent");
    assert_eq!(out["truncated"], true);
    assert_eq!(out["pages_sent"], json!([1]));
    assert_eq!(out["suggestions"], json!([{ "text": "Jane Doe", "pages": [1], "occurrences": 1 }]));
    assert_eq!(out["not_in_text"], 1, "a passage the model was not given is not a finding");
}
