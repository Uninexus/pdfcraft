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

/// One page with a caption and three tagged figures: 7 and 9 have no alternate text (9 an empty one), 8 has some.
fn figures_pdf() -> Vec<u8> {
    let c = "BT /F1 12 Tf 20 150 Td (Figure 1: Quarterly revenue) Tj ET /Figure << /MCID 0 >> BDC 10 100 40 20 re f EMC \
/Figure << /MCID 1 >> BDC 100 100 50 30 re f EMC /Figure << /MCID 2 >> BDC 100 20 50 30 re f EMC";
    format!(
        "%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R /MarkInfo << /Marked true >> /StructTreeRoot 5 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /StructParents 0 /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >> >> endobj
4 0 obj << /Length {} >> stream
{c}
endstream endobj
5 0 obj << /Type /StructTreeRoot /K 6 0 R /ParentTree 10 0 R >> endobj
6 0 obj << /S /Document /P 5 0 R /K [7 0 R 8 0 R 9 0 R] >> endobj
7 0 obj << /S /Figure /P 6 0 R /Pg 3 0 R /K 0 >> endobj
8 0 obj << /S /Figure /P 6 0 R /Pg 3 0 R /K 1 /Alt (Existing text) >> endobj
9 0 obj << /S /Figure /P 6 0 R /Pg 3 0 R /K 2 /Alt () >> endobj
10 0 obj << /Nums [0 [7 0 R 8 0 R 9 0 R]] >> endobj
trailer << /Root 1 0 R >>
%%EOF",
        c.len()
    )
    .into_bytes()
}

#[test]
fn alt_text_suggestions_answer_only_the_figures_asked_about_and_change_nothing() {
    let dir = std::env::temp_dir().join(format!("pdfcraft-automation-ai-alt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("f.pdf"), figures_pdf()).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap();
    let doc = json_of(a.call("doc_open", &json!({ "path": "f.pdf" })).unwrap())["doc"].as_u64().unwrap();
    let before = json_of(a.call("accessibility_figures", &json!({ "doc": doc })).unwrap());
    assert_eq!(before["count"], 3, "{before}");

    // Figures 7 and 9 are asked about; the model answers 7, a figure that has text (8), one that does not exist (99), 7 again and a
    // blank text for 9.
    let (url, req) = server(
        r#"{"choices":[{"message":{"content":"[{\"figure\":7,\"alt\":\"Bar chart of\\nquarterly revenue\"},{\"figure\":8,\"alt\":\"x\"},{\"figure\":99,\"alt\":\"y\"},{\"figure\":7,\"alt\":\"again\"},{\"figure\":9,\"alt\":\" \"}]"}}]}"#,
    );
    let out = json_of(a.call("ai_alt_text_suggest", &json!({ "doc": doc, "base_url": url, "model": "m" })).unwrap());
    assert_eq!(out["suggestions"], json!([{ "figure": 7, "page": 1, "alt": "Bar chart of quarterly revenue" }]), "{out}");
    assert_eq!((out["asked"].clone(), out["unanswered"].clone(), out["ignored"].clone()), (json!([7, 9]), json!([9]), json!(3)));
    assert_eq!((out["not_sent"].clone(), out["pages_sent"].clone(), out["truncated"].clone()), (json!([]), json!([1]), json!(false)));
    let req = req.join().unwrap();
    assert!(req.contains("figure 7 (page 1), figure 9 (page 1)") && req.contains("Quarterly revenue") && !req.contains("figure 8 (page"), "{req}");
    assert_eq!(json_of(a.call("accessibility_figures", &json!({ "doc": doc })).unwrap()), before, "suggesting changes nothing");

    // Naming a figure asks about it even with text; one that is not a figure is refused before the endpoint is contacted.
    let (url, req) = server(r#"{"choices":[{"message":{"content":"[{\"figure\":8,\"alt\":\"A revenue bar\"}]"}}]}"#);
    let out = json_of(a.call("ai_alt_text_suggest", &json!({ "doc": doc, "figures": [8], "base_url": url, "model": "m" })).unwrap());
    assert_eq!(out["suggestions"], json!([{ "figure": 8, "page": 1, "alt": "A revenue bar" }]), "{out}");
    assert!(req.join().unwrap().contains("figure 8 (page 1)"));
    let e = a
        .call("ai_alt_text_suggest", &json!({ "doc": doc, "figures": [6], "base_url": "http://127.0.0.1:1/v1", "model": "m" }))
        .unwrap_err()
        .to_string();
    assert!(e.contains("6 is not a figure"), "{e}");
    // More than 100 names are refused before the document is scanned or anything is sent.
    let many: Vec<u32> = (1..=101).collect();
    let e = a
        .call("ai_alt_text_suggest", &json!({ "doc": doc, "figures": many, "base_url": "http://127.0.0.1:1/v1", "model": "m" }))
        .unwrap_err()
        .to_string();
    assert!(e.contains("more than 100 figures"), "{e}");

    // A reply that is not a list is an error that says so.
    let (url, req) = server(r#"{"choices":[{"message":{"content":"I cannot see them."}}]}"#);
    let e = a.call("ai_alt_text_suggest", &json!({ "doc": doc, "base_url": url, "model": "m" })).unwrap_err().to_string();
    assert!(e.contains("JSON list"), "{e}");
    req.join().unwrap();
}

#[test]
fn alt_text_suggestions_send_nothing_when_no_figure_needs_text() {
    let dir = std::env::temp_dir().join(format!("pdfcraft-automation-ai-alt0-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.pdf"), pdf()).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap();
    let doc = json_of(a.call("doc_open", &json!({ "path": "a.pdf" })).unwrap())["doc"].as_u64().unwrap();
    // An untagged page has no figure; the endpoint does not exist, and nothing tries it.
    let out = json_of(a.call("ai_alt_text_suggest", &json!({ "doc": doc, "base_url": "http://127.0.0.1:1/v1", "model": "m" })).unwrap());
    assert_eq!((out["suggestions"].clone(), out["asked"].clone(), out["pages_sent"].clone()), (json!([]), json!([]), json!([])));
}

#[test]
fn a_figure_on_a_page_the_size_limit_left_out_is_not_described() {
    // Page 1 holds about 70,000 characters of text and figure 10; page 2 holds figure 12 and so is never sent.
    let mut big = String::from("BT /F1 4 Tf 5 TL 5 3990 Td\n");
    for _ in 0..700 {
        big.push_str(&format!("({}) '\n", "lorem ipsum ".repeat(8)));
    }
    big.push_str("ET /Figure << /MCID 0 >> BDC 10 10 40 20 re f EMC");
    let small = "/Figure << /MCID 0 >> BDC 10 10 40 20 re f EMC";
    let pdf = format!(
        "%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R /MarkInfo << /Marked true >> /StructTreeRoot 5 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 600 4000] /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >> >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /Contents 6 0 R /StructParents 0 >> endobj
4 0 obj << /Type /Page /Parent 2 0 R /Contents 7 0 R /StructParents 1 >> endobj
5 0 obj << /Type /StructTreeRoot /K 8 0 R /ParentTree 11 0 R >> endobj
6 0 obj << /Length {} >> stream
{big}
endstream endobj
7 0 obj << /Length {} >> stream
{small}
endstream endobj
8 0 obj << /S /Document /P 5 0 R /K [10 0 R 12 0 R] >> endobj
10 0 obj << /S /Figure /P 8 0 R /Pg 3 0 R /K 0 >> endobj
12 0 obj << /S /Figure /P 8 0 R /Pg 4 0 R /K 0 >> endobj
11 0 obj << /Nums [0 [10 0 R] 1 [12 0 R]] >> endobj
trailer << /Root 1 0 R >>
%%EOF",
        big.len(),
        small.len()
    );
    let dir = std::env::temp_dir().join(format!("pdfcraft-automation-ai-alt2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("two.pdf"), pdf).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap();
    let doc = json_of(a.call("doc_open", &json!({ "path": "two.pdf" })).unwrap())["doc"].as_u64().unwrap();
    let (url, req) = server(r#"{"choices":[{"message":{"content":"[{\"figure\":10,\"alt\":\"First\"},{\"figure\":12,\"alt\":\"Second\"}]"}}]}"#);
    let out = json_of(a.call("ai_alt_text_suggest", &json!({ "doc": doc, "base_url": url, "model": "m" })).unwrap());
    req.join().unwrap();
    assert_eq!(out["asked"], json!([10, 12]), "{out}");
    assert_eq!(out["suggestions"], json!([{ "figure": 10, "page": 1, "alt": "First" }]), "{out}");
    assert_eq!(
        (out["not_sent"].clone(), out["ignored"].clone(), out["pages_sent"].clone(), out["truncated"].clone()),
        (json!([12]), json!(1), json!([1]), json!(true))
    );
}

#[test]
fn a_named_figure_drawn_on_no_page_is_refused_before_anything_is_sent() {
    let pdf = "%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R /MarkInfo << /Marked true >> /StructTreeRoot 5 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 200 200] >> endobj
3 0 obj << /Type /Page /Parent 2 0 R >> endobj
5 0 obj << /Type /StructTreeRoot /K 6 0 R >> endobj
6 0 obj << /S /Document /P 5 0 R /K [7 0 R] >> endobj
7 0 obj << /S /Figure /P 6 0 R >> endobj
trailer << /Root 1 0 R >>
%%EOF";
    let dir = std::env::temp_dir().join(format!("pdfcraft-automation-ai-alt3-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("n.pdf"), pdf).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap();
    let doc = json_of(a.call("doc_open", &json!({ "path": "n.pdf" })).unwrap())["doc"].as_u64().unwrap();
    let listed = json_of(a.call("accessibility_figures", &json!({ "doc": doc })).unwrap());
    assert_eq!(listed["figures"], json!([{ "figure": 7, "page": null, "alt": null, "rect": null }]), "{listed}");
    let e = a
        .call("ai_alt_text_suggest", &json!({ "doc": doc, "figures": [7], "base_url": "http://127.0.0.1:1/v1", "model": "m" }))
        .unwrap_err()
        .to_string();
    assert!(e.contains("figure 7 is not drawn on any page"), "{e}");
}

/// Serves `replies` (model message contents) one per connection, in order; returns the base URL and
/// the raw requests seen.
fn sequence(replies: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", l.local_addr().unwrap());
    let h = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for content in replies {
            let (mut s, _) = l.accept().unwrap();
            let (mut req, mut buf) = (Vec::new(), [0u8; 16384]);
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
            let body = json!({ "choices": [{ "message": { "content": content } }] }).to_string();
            let r = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            s.write_all(r.as_bytes()).unwrap();
            seen.push(String::from_utf8_lossy(&req).into_owned());
        }
        seen
    });
    (url, h)
}

fn open(tag: &str) -> (Automation, u64) {
    let dir = std::env::temp_dir().join(format!("pdfcraft-automation-ai-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.pdf"), pdf()).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap();
    let doc = json_of(a.call("doc_open", &json!({ "path": "a.pdf" })).unwrap())["doc"].as_u64().unwrap();
    (a, doc)
}

#[test]
fn a_plan_proposes_edits_and_changes_nothing() {
    let (mut a, doc) = open("plan");
    let reply = r#"Sure: {"message":"Rotating page 1.","calls":[{"tool":"page_rotate","args":{"pages":[1],"degrees":90},"why":"as asked"}]}"#;
    let (url, reqs) = sequence(vec![reply.into()]);
    let out = json_of(a.call("ai_edit_plan", &json!({ "doc": doc, "instruction": "rotate page 1", "base_url": url, "model": "m" })).unwrap());
    assert_eq!(out["steps"][0]["tool"], "page_rotate");
    assert_eq!(out["steps"][0]["args"], json!({ "pages": [1], "degrees": 90 }));
    assert_eq!(out["steps"][0]["why"], "as asked");
    let req = reqs.join().unwrap().remove(0);
    assert!(req.contains("page_rotate") && req.contains("Invoice total 42") && req.contains("rotate page 1"), "{req}");
    // The model's proposal never touched the document.
    let s = json_of(a.call("doc_list", &json!({})).unwrap());
    assert_eq!(s["documents"][0]["dirty"], false);
}

#[test]
fn a_plan_lets_the_model_look_first_and_returns_what_it_saw() {
    let (mut a, doc) = open("look");
    let look = r#"{"message":"","calls":[{"tool":"text_find","args":{"query":"Invoice"}}]}"#;
    let act = r#"{"message":"done","calls":[{"tool":"page_rotate","args":{"pages":[1],"degrees":180}}]}"#;
    let (url, reqs) = sequence(vec![look.into(), act.into()]);
    let out = json_of(
        a.call("ai_edit_plan", &json!({ "doc": doc, "instruction": "turn the invoice page upside down", "base_url": url, "model": "m" })).unwrap(),
    );
    assert_eq!(out["steps"].as_array().unwrap().len(), 1);
    assert_eq!(out["looked_at"][0]["tool"], "text_find");
    let second = &reqs.join().unwrap()[1];
    assert!(second.contains("text_find ->"), "the model should get the find result back: {second}");
}

#[test]
fn a_plan_refuses_tools_outside_the_allow_list_after_asking_the_model_to_retry() {
    let (mut a, doc) = open("deny");
    let bad = r#"{"message":"","calls":[{"tool":"doc_save","args":{"path":"x.pdf"}}]}"#;
    let (url, reqs) = sequence(vec![bad.into(), bad.into(), bad.into()]);
    let e = a.call("ai_edit_plan", &json!({ "doc": doc, "instruction": "save it", "base_url": url, "model": "m" })).unwrap_err().to_string();
    assert!(e.contains("doc_save"), "{e}");
    let seen = reqs.join().unwrap();
    assert!(seen[1].contains("could not be used") && seen[1].contains("doc_save"));
}

#[test]
fn a_reply_that_is_only_talk_proposes_no_change() {
    let (mut a, doc) = open("talk");
    let (url, reqs) = sequence(vec!["The total is 42.".into()]);
    let out = json_of(a.call("ai_edit_plan", &json!({ "doc": doc, "instruction": "what is the total?", "base_url": url, "model": "m" })).unwrap());
    assert_eq!(out["steps"], json!([]));
    assert_eq!(out["message"], "The total is 42.");
    reqs.join().unwrap();
}

#[test]
fn apply_runs_the_steps_as_undo_steps_and_undo_takes_them_back() {
    let (mut a, doc) = open("apply");
    let steps = json!([{ "tool": "page_rotate", "args": { "pages": [1], "degrees": 90 } }, { "tool": "doc_set_info", "args": { "key": "Title", "value": "Hi" } }]);
    let out = json_of(a.call("ai_edit_apply", &json!({ "doc": doc, "steps": steps })).unwrap());
    assert_eq!(out["applied"], 2);
    let n = out["undo_steps"].as_u64().unwrap();
    assert!(n >= 2, "{out}");
    assert_eq!(out["document"]["dirty"], true);
    for _ in 0..n {
        a.call("edit_undo", &json!({ "doc": doc })).unwrap();
    }
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "undo_steps should account for every step");
}

#[test]
fn apply_checks_every_step_first_and_undoes_earlier_steps_when_one_fails() {
    let (mut a, doc) = open("fail");
    // A tool outside the allow-list: nothing runs, even the valid first step.
    let e = a
        .call(
            "ai_edit_apply",
            &json!({ "doc": doc, "steps": [{ "tool": "page_rotate", "args": { "pages": [1], "degrees": 90 } }, { "tool": "doc_save", "args": {} }] }),
        )
        .unwrap_err()
        .to_string();
    assert!(e.contains("steps[1]") && e.contains("doc_save"), "{e}");
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "nothing should have run");

    // A step that fails when it runs: the earlier one is rolled back.
    let e = a
        .call("ai_edit_apply", &json!({ "doc": doc, "steps": [{ "tool": "page_rotate", "args": { "pages": [1], "degrees": 90 } }, { "tool": "page_rotate", "args": { "pages": [99], "degrees": 90 } }] }))
        .unwrap_err()
        .to_string();
    assert!(e.contains("step 2") && e.contains("undone"), "{e}");
    assert!(a.call("edit_undo", &json!({ "doc": doc })).is_err(), "the first step should have been undone");
    let list = json_of(a.call("doc_list", &json!({})).unwrap());
    assert_eq!(list["documents"][0]["dirty"], false);
}
