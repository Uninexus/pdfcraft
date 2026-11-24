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
    AiConfig { base_url: url.into(), api_key: Some("sk-secret".into()), model: "m1".into(), timeout_secs: 10, ..Default::default() }
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

#[test]
fn translate_question_names_the_language_and_refuses_junk() {
    assert!(translate_question(" Spanish ").expect("q").contains("into Spanish."));
    for bad in ["", "  ", "a\nb", &"x".repeat(61)] {
        assert_eq!(translate_question(bad), None, "{bad:?}");
    }
    assert!(SUMMARY_QUESTION.contains("Summarize"));
}

#[test]
fn the_chatgpt_provider_is_saved_and_needs_a_model_but_no_address_or_key() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("ai.json");
    let chatgpt = AiConfig { provider: Provider::ChatGpt, model: "m".into(), ..Default::default() };
    chatgpt.save(&path).expect("save");
    assert_eq!(AiConfig::load(&path).expect("load").provider, Provider::ChatGpt);
    assert!(chatgpt.check().is_ok());
    assert!(AiConfig { model: String::new(), ..chatgpt.clone() }.check().is_err());
    // The default stays the endpoint provider, which still needs an address.
    assert_eq!(AiConfig::default().provider, Provider::Endpoint);
    assert!(AiConfig { model: "m".into(), ..Default::default() }.check().is_err());
    assert_eq!(Provider::parse("ChatGPT"), Some(Provider::ChatGpt));
    assert_eq!(Provider::parse("bogus"), None);
}

mod page_references {
    use crate::refs::{Segment, split_page_references};

    fn t(s: &str) -> Segment {
        Segment::Text(s.into())
    }

    fn rejoin(segments: &[Segment]) -> String {
        segments
            .iter()
            .map(|s| match s {
                Segment::Text(t) => t.clone(),
                Segment::Page(p) => p.to_string(),
            })
            .collect()
    }

    #[test]
    fn the_citation_style_the_prompt_asks_for_is_found() {
        assert_eq!(split_page_references("The answer is on (p. 20).", 30), vec![t("The answer is on (p. "), Segment::Page(20), t(").")]);
        assert_eq!(split_page_references("see p.3", 5), vec![t("see p."), Segment::Page(3)]);
    }

    #[test]
    fn lists_and_ranges_link_each_number() {
        assert_eq!(
            split_page_references("pp. 3-5, 9 and 11", 20),
            vec![t("pp. "), Segment::Page(3), t("-"), Segment::Page(5), t(", "), Segment::Page(9), t(" and "), Segment::Page(11)]
        );
        assert_eq!(split_page_references("Pages 2 & 4", 9), vec![t("Pages "), Segment::Page(2), t(" & "), Segment::Page(4)]);
        assert_eq!(split_page_references("--- Page 3 ---", 9), vec![t("--- Page "), Segment::Page(3), t(" ---")]);
    }

    #[test]
    fn pages_outside_the_document_and_lookalikes_stay_text() {
        assert_eq!(split_page_references("on page 40", 30), vec![t("on page 40")]);
        assert_eq!(split_page_references("page 0", 30), vec![t("page 0")]);
        // Only the in-range number of a list becomes a link.
        assert_eq!(split_page_references("pp. 5, 99", 10), vec![t("pp. "), Segment::Page(5), t(", 99")]);
        for plain in ["a homepage 3 times", "top 3", "http://x/p.3", "page", "page x", "p.3rd", "page 1234567", "a sleep. 3 more"] {
            assert_eq!(split_page_references(plain, 2_000_000), vec![t(plain)], "{plain}");
        }
        assert!(split_page_references("", 5).is_empty());
    }

    #[test]
    fn words_are_never_lost_or_split_inside_a_character() {
        let text = "é page 2 – ünï p. 3, naïve pp. 1–2 \u{1F600} page 4";
        let segs = split_page_references(text, 10);
        assert_eq!(rejoin(&segs), text);
        assert_eq!(segs.iter().filter(|s| matches!(s, Segment::Page(_))).count(), 5);
    }
}

#[test]
fn passages_are_read_from_a_wrapped_reply_and_junk_is_dropped() {
    let reply = "Sure:\n```json\n[\"Jane Doe\", 7, \"  \", \"Jane Doe\", \"a\\u0007b\", null, \"+1 555 0100\"]\n```";
    assert_eq!(parse_passages(reply), Some(vec!["Jane Doe".to_owned(), "+1 555 0100".to_owned()]));
    assert_eq!(parse_passages("[]"), Some(vec![]));
    assert_eq!(parse_passages("nothing to redact"), None);
    assert_eq!(parse_passages("] [ not json ["), None);
    let long = format!("[\"{}\"]", "x".repeat(MAX_PASSAGE_CHARS + 1));
    assert_eq!(parse_passages(&long), Some(vec![]));
    let many = format!("[{}]", (0..MAX_PASSAGES + 50).map(|i| format!("\"w{i}\"")).collect::<Vec<_>>().join(","));
    assert_eq!(parse_passages(&many).map(|v| v.len()), Some(MAX_PASSAGES));
}

#[test]
fn passages_ignore_brackets_around_the_list_and_fold_newlines_and_tabs() {
    assert_eq!(parse_passages("[\"Jane\"] see [p. 2]"), Some(vec!["Jane".to_owned()]));
    assert_eq!(parse_passages("As in [1]: [\"Jane\"]"), Some(vec!["Jane".to_owned()]));
    assert_eq!(
        parse_passages("[\"12 Elm\\nStreet\", \"Jane\\tDoe\", \"12 Elm Street\"]"),
        Some(vec!["12 Elm Street".to_owned(), "Jane Doe".to_owned()])
    );
}

#[test]
fn alt_texts_are_read_from_a_wrapped_reply_and_junk_is_dropped() {
    let reply = "As in [1]: ```json\n[{\"figure\": 7, \"alt\": \"Bar\\n chart\"}, {\"figure\": \"8\", \"alt\": \"x\"}, {\"figure\": 9}, \
{\"figure\": 9, \"alt\": \"  \"}, {\"figure\": -1, \"alt\": \"x\"}, {\"figure\": 4294967296, \"alt\": \"x\"}, {\"figure\": 5, \"alt\": \"a\\u0007b\"}, \
7, {\"figure\": 7, \"alt\": \"Again\"}]\n```";
    assert_eq!(parse_alt_texts(reply), Some(vec![(7, "Bar chart".to_owned()), (7, "Again".to_owned())]));
    assert_eq!(parse_alt_texts("[]"), Some(vec![]));
    assert_eq!(parse_alt_texts("no figures"), None);
    let long = format!("[{{\"figure\": 1, \"alt\": \"{}\"}}]", "x".repeat(MAX_ALT_CHARS + 1));
    assert_eq!(parse_alt_texts(&long), Some(vec![]));
    let many = format!("[{}]", (0..MAX_ALT_ENTRIES + 50).map(|i| format!("{{\"figure\": {i}, \"alt\": \"a\"}}")).collect::<Vec<_>>().join(","));
    assert_eq!(parse_alt_texts(&many).map(|v| v.len()), Some(MAX_ALT_ENTRIES));
}

#[test]
fn the_alt_text_question_names_each_figure_and_its_page() {
    let q = alt_text_question(&[(7, 1), (12, 3)]);
    assert!(q.contains("figure 7 (page 1), figure 12 (page 3)") && q.contains("cannot see the pictures"), "{q}");
}

#[test]
fn document_context_reports_what_was_really_included() {
    let ctx = document_context(&[(1, "short".into()), (2, "é".repeat(MAX_CONTEXT_CHARS + 10)), (3, "never seen".into())]);
    assert!(ctx.cut);
    assert_eq!(ctx.included.iter().map(|(p, _)| *p).collect::<Vec<_>>(), vec![1, 2]);
    assert!(ctx.included[1].1.chars().count() < MAX_CONTEXT_CHARS);
    let fits = document_context(&[(1, "a".into()), (2, "b".into())]);
    assert!(!fits.cut && fits.included.len() == 2);
}

mod agent_turns {
    use crate::agent::{MAX_CALLS, parse_turn};
    use serde_json::json;

    #[test]
    fn a_fenced_reply_with_prose_around_it_is_read() {
        let t = parse_turn(
            "Here you go:\n```json\n{\"message\": \" Done \", \"calls\": [{\"tool\": \"page_rotate\", \"args\": {\"pages\": [2]}, \"why\": \"a\\n  b\"}]}\n```",
        );
        assert_eq!(t.message, "Done");
        assert_eq!(t.calls.len(), 1);
        assert_eq!(t.calls[0].tool, "page_rotate");
        assert_eq!(t.calls[0].args, json!({ "pages": [2] }));
        assert_eq!(t.calls[0].why, "a b");
    }

    #[test]
    fn an_object_without_calls_or_message_is_skipped_and_plain_talk_is_a_message() {
        let t = parse_turn("see {\"x\": 1} and {\"calls\": []}");
        assert!(t.calls.is_empty() && t.message.is_empty());
        let t = parse_turn("The total is 42.");
        assert_eq!((t.message.as_str(), t.calls.len()), ("The total is 42.", 0));
    }

    #[test]
    fn junk_calls_are_dropped_and_the_count_is_capped() {
        let t = parse_turn(r#"{"calls": [1, {"tool": ""}, {"tool": "a", "args": [1]}, {"nothing": true}]}"#);
        assert_eq!(t.calls.len(), 1);
        assert_eq!(t.calls[0].args, json!({}));
        let many = format!("{{\"calls\": [{}]}}", vec![r#"{"tool":"a"}"#; MAX_CALLS + 20].join(","));
        assert_eq!(parse_turn(&many).calls.len(), MAX_CALLS);
    }
}

mod agent_plan {
    use super::*;
    use crate::agent::{PlanRequest, plan};

    fn request() -> PlanRequest<'static> {
        PlanRequest { instruction: "rotate page 1", document: "2 page(s)", text_context: "--- Page 1 ---\nHello\n", catalog: "- rotate_pages(): x" }
    }

    #[test]
    fn a_plan_is_checked_and_the_model_is_told_why_when_it_is_refused() {
        let first = r#"{"message":"","calls":[{"tool":"nope","args":{}}]}"#;
        let second = r#"{"message":"ok","calls":[{"tool":"rotate_pages","args":{"degrees":90}}]}"#;
        let (url, seen) = serve_sequence(vec![first.into(), second.into()]);
        let check = |calls: &[crate::agent::PlannedCall]| {
            if calls.iter().any(|c| c.tool == "nope") {
                Err("nope is not a tool".to_owned())
            } else {
                Ok(calls.iter().map(|c| c.tool.clone()).collect())
            }
        };
        let got = plan(&cfg(&url), &request(), &check).expect("plan");
        assert_eq!(got.calls.len(), 1);
        assert_eq!(got.summaries, vec!["rotate_pages".to_owned()]);
        let seen = seen.join().expect("server");
        assert!(seen[0].contains("Hello") && seen[0].contains("rotate page 1") && seen[0].contains("- rotate_pages(): x"));
        assert!(seen[1].contains("nope is not a tool"), "the second request carries the reason");
    }

    #[test]
    fn a_model_that_keeps_getting_it_wrong_ends_in_an_error_after_a_few_tries() {
        let bad = r#"{"calls":[{"tool":"nope"}]}"#;
        let (url, seen) = serve_sequence(vec![bad.into(); crate::agent::MAX_ATTEMPTS]);
        let e = plan(&cfg(&url), &request(), &|_| Err("no".to_owned())).unwrap_err().to_string();
        assert!(e.contains("cannot be made"), "{e}");
        assert_eq!(seen.join().expect("server").len(), crate::agent::MAX_ATTEMPTS);
    }

    /// Serves one chat reply per connection, in order, and returns the raw requests.
    fn serve_sequence(replies: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://{}/v1", listener.local_addr().expect("addr"));
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for content in replies {
                let (mut s, _) = listener.accept().expect("accept");
                let (mut req, mut buf) = (Vec::new(), [0u8; 16384]);
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
                let body = serde_json::json!({ "choices": [{ "message": { "content": content } }] }).to_string();
                let r =
                    format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                s.write_all(r.as_bytes()).expect("write");
                seen.push(String::from_utf8_lossy(&req).into_owned());
            }
            seen
        });
        (url, handle)
    }
}
