//! `redact_mark` with `keep_first` and `keep_last`: partial redaction of a pattern match, end to
//! end through mark, apply, save and reopen.

use pdfcraft_automation::{Automation, Content};
use serde_json::{Value, json};

fn call(a: &mut Automation, tool: &str, args: Value) -> Result<Value, String> {
    match a.call(tool, &args) {
        Ok(mut c) => match c.remove(0) {
            Content::Json(v) => Ok(v),
            other => panic!("{tool}: expected JSON, got {other:?}"),
        },
        Err(e) => Err(e.to_string()),
    }
}

fn ok(a: &mut Automation, tool: &str, args: Value) -> Value {
    call(a, tool, args.clone()).unwrap_or_else(|e| panic!("{tool} {args}: {e}"))
}

fn page_text(a: &mut Automation, doc: u64) -> String {
    ok(a, "text_extract", json!({ "doc": doc }))["pages"][0]["text"].as_str().unwrap().to_string()
}

fn start(name: &str, text: &str) -> (Automation, u64, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("pdfcraft-redact-keep-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("t.txt"), text).unwrap();
    let mut a = Automation::new().with_root(&dir).unwrap().with_clock(|| 1_700_000_000);
    let doc = ok(&mut a, "doc_create", json!({ "from": "text", "path": "t.txt" }))["doc"].as_u64().unwrap();
    (a, doc, dir)
}

#[test]
/// Keeping the last four digits of a card number removes the other twelve from the saved page and
/// leaves the four; a SIN keeps its first three; a match no longer than the kept ends is not marked.
fn keep_last_and_keep_first_leave_the_ends_of_a_match_readable() {
    let (mut a, doc, dir) = start("ends", "Card 4539 1488 0343 6467 on file\nSIN 130-692-544 here");
    let card = ok(&mut a, "redact_mark", json!({ "doc": doc, "pattern": "credit-card", "keep_last": 4 }));
    assert_eq!((card["marked"].clone(), card["left_readable"].clone()), (json!(1), json!(0)));
    let sin = ok(&mut a, "redact_mark", json!({ "doc": doc, "pattern": "sin", "keep_first": 3 }));
    assert_eq!(sin["marked"], 1);
    assert!(page_text(&mut a, doc).contains("4539 1488"), "marks alone remove nothing");
    ok(&mut a, "redact_apply", json!({ "doc": doc }));
    ok(&mut a, "doc_save", json!({ "doc": doc, "path": "out.pdf" }));
    let reopened = ok(&mut a, "doc_open", json!({ "path": "out.pdf" }))["doc"].as_u64().unwrap();
    let text = page_text(&mut a, reopened);
    for gone in ["4539", "1488", "0343", "692", "544"] {
        assert!(!text.contains(gone), "{gone} must be removed: {text}");
    }
    for kept in ["6467", "130", "Card", "on file", "SIN", "here"] {
        assert!(text.contains(kept), "{kept} must remain: {text}");
    }

    // Ends that cover the whole match leave nothing to mark: the call says so and changes nothing.
    let (mut b, doc, dir2) = start("short", "Ship to K1A 0B1 today");
    let err = call(&mut b, "redact_mark", json!({ "doc": doc, "pattern": "ca-postal-code", "keep_first": 3, "keep_last": 3 })).unwrap_err();
    assert!(err.contains("leave every one of them readable"), "{err}");
    let none = call(&mut b, "redact_clear", json!({ "doc": doc })).unwrap_err();
    assert!(none.contains("no redaction marks"), "no mark was made: {none}");
    // One of two matches is long enough: it is marked, the short one counted as left readable.
    let (mut c, doc, dir3) = start("mixed", "Codes 12-34 and 4539 1488 0343 6467 end");
    let r = ok(&mut c, "redact_mark", json!({ "doc": doc, "regex": "[0-9][0-9 -]*[0-9]", "keep_last": 4 }));
    assert_eq!((r["marked"].clone(), r["left_readable"].clone()), (json!(1), json!(1)), "{r}");
    assert!(r["warning"].as_str().unwrap().contains("stay readable"), "{r}");
    for d in [dir, dir2, dir3] {
        std::fs::remove_dir_all(d).unwrap();
    }
}

#[test]
/// Hostile or misplaced arguments are refused with a message and mark nothing.
fn keep_arguments_are_checked() {
    let (mut a, doc, dir) = start("args", "Card 4539 1488 0343 6467");
    for (args, needle) in [
        (json!({ "pattern": "credit-card", "keep_last": -1 }), "from 0 to 64"),
        (json!({ "pattern": "credit-card", "keep_first": 65 }), "from 0 to 64"),
        (json!({ "pattern": "credit-card", "keep_last": 9_007_199_254_740_993_i64 }), "from 0 to 64"),
        (json!({ "pattern": "credit-card", "keep_last": 2.5 }), "integer"),
        (json!({ "find": "Card", "keep_last": 1 }), "only to pattern or regex"),
        (json!({ "whole_pages": true, "keep_first": 1 }), "only to pattern or regex"),
    ] {
        let mut full = args.clone();
        full["doc"] = json!(doc);
        let err = call(&mut a, "redact_mark", full).unwrap_err();
        assert!(err.contains(needle), "{args}: {err}");
    }
    // A value of 0 is the old behaviour, even beside find.
    assert_eq!(ok(&mut a, "redact_mark", json!({ "doc": doc, "find": "Card", "keep_last": 0 }))["marked"], 1);
    std::fs::remove_dir_all(dir).unwrap();
}
