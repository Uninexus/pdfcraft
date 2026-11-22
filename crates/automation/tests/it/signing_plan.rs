//! The signing plan tools: a local record beside the PDF, with the order enforced and every
//! refused call leaving the folder as it was.

use std::path::Path;

use pdfcraft_automation::file_access::AutomationRoots;
use pdfcraft_automation::{Automation, Content, ToolError};
use serde_json::{Value, json};

fn setup() -> (tempfile::TempDir, Automation) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.pdf"), b"%PDF-1.7\n% placeholder, the plan never reads it\n").unwrap();
    let a = Automation::new().with_root(dir.path()).unwrap().with_clock(|| 1_700_000_000);
    (dir, a)
}

fn run(a: &mut Automation, tool: &str, args: Value) -> Result<Value, String> {
    match a.call(tool, &args) {
        Ok(mut c) => match c.remove(0) {
            Content::Json(v) => Ok(v),
            other => panic!("expected JSON: {other:?}"),
        },
        Err(ToolError::InvalidArgs(m) | ToolError::Failed(m)) => Err(m),
        Err(e) => panic!("{e:?}"),
    }
}

fn ok(a: &mut Automation, tool: &str, args: Value) -> Value {
    run(a, tool, args).unwrap_or_else(|e| panic!("{tool} refused: {e}"))
}

fn refused(a: &mut Automation, tool: &str, args: Value) -> String {
    run(a, tool, args).expect_err("the call should be refused")
}

fn three() -> Value {
    json!([
        { "name": "Ana Reyes", "email": "ana@example.org" },
        { "name": "Bo Chen", "email": "bo@example.org", "role": "approver" },
        { "name": "Cy Dunn", "email": "cy@example.org", "role": "viewer" },
        { "name": "Di Evans", "email": "di@example.org" }
    ])
}

/// Every file under `dir` with its bytes, so a test can say nothing changed.
fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap_or_default())
        })
        .collect();
    out.sort();
    out
}

fn sidecar(dir: &Path) -> std::path::PathBuf {
    dir.join("a.pdf.pdfcraft-sign.json")
}

#[test]
fn a_plan_round_trips_through_the_sidecar_and_leaves_the_pdf_alone() {
    let (dir, mut a) = setup();
    let pdf_before = std::fs::read(dir.path().join("a.pdf")).unwrap();
    let set = ok(&mut a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": three(), "message": "Please sign by Friday." }));
    assert!(sidecar(dir.path()).is_file());
    assert_eq!(std::fs::read(dir.path().join("a.pdf")).unwrap(), pdf_before);
    let got = ok(&mut a, "signing_plan_get", json!({ "path": "a.pdf" }));
    assert_eq!(got["plan"], set["plan"]);
    assert_eq!((&set["replaced"], &set["discarded_history"]), (&json!(false), &json!(false)));
    let names: Vec<_> = got["plan"]["recipients"].as_array().unwrap().iter().map(|r| r["email"].as_str().unwrap()).collect();
    assert_eq!(names, ["ana@example.org", "bo@example.org", "cy@example.org", "di@example.org"]);
    assert_eq!(got["plan"]["message"], "Please sign by Friday.");
    assert_eq!(got["state"], "in_progress");
    assert_eq!(got["next"]["email"], "ana@example.org");
    // Every recipient starts pending with one history entry, and the result says it is only a record.
    for r in got["plan"]["recipients"].as_array().unwrap() {
        assert_eq!(r["status"], "pending");
        assert_eq!(r["history"].as_array().unwrap().len(), 1);
    }
    assert_eq!(got["local_record_only"], true);
    let note = got["note"].as_str().unwrap();
    assert!(note.contains("does not send") && note.contains("collect signatures"), "{note}");
    // Reading and clearing: a plan is gone after clear, and the PDF is still there.
    assert_eq!(ok(&mut a, "signing_plan_clear", json!({ "path": "a.pdf" }))["cleared"], true);
    assert!(!sidecar(dir.path()).exists() && dir.path().join("a.pdf").is_file());
    assert_eq!(ok(&mut a, "signing_plan_get", json!({ "path": "a.pdf" }))["state"], "none");
    assert_eq!(ok(&mut a, "signing_plan_clear", json!({ "path": "a.pdf" }))["cleared"], false);
}

#[test]
fn listed_order_numbers_decide_the_turns() {
    let (_dir, mut a) = setup();
    let plan = json!([
        { "name": "Second", "email": "second@example.org", "order": 2 },
        { "name": "First", "email": "first@example.org", "order": 1 }
    ]);
    let set = ok(&mut a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": plan }));
    assert_eq!(set["next"]["email"], "first@example.org");
    assert_eq!(set["plan"]["recipients"][1]["email"], "second@example.org");
}

#[test]
fn turns_are_taken_in_order_and_the_plan_finishes() {
    let (dir, mut a) = setup();
    ok(&mut a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": three() }));
    let step = |email: &str, status: &str| json!({ "path": "a.pdf", "email": email, "status": status });

    // Out of turn, a viewer, an unknown address: each refused, the file not touched.
    let before = snapshot(dir.path());
    assert!(refused(&mut a, "signing_plan_advance", step("bo@example.org", "approved")).contains("not that recipient's turn"));
    assert!(refused(&mut a, "signing_plan_advance", step("cy@example.org", "signed")).contains("viewer"));
    assert!(refused(&mut a, "signing_plan_advance", step("nobody@example.org", "signed")).contains("no recipient"));
    assert!(
        refused(&mut a, "signing_plan_advance", json!({ "path": "a.pdf", "email": "ana@example.org", "status": "pending" }))
            .contains("signed, approved or declined")
    );
    assert_eq!(snapshot(dir.path()), before);

    // The address is compared without regard to case; the turn then passes to the next, skipping the viewer.
    let one = ok(&mut a, "signing_plan_advance", step("ANA@Example.org", "signed"));
    assert_eq!(one["next"]["email"], "bo@example.org");
    // An approver approves rather than signs, and a signer signs rather than approves.
    assert!(refused(&mut a, "signing_plan_advance", step("bo@example.org", "signed")).contains("approver"));
    let two = ok(&mut a, "signing_plan_advance", step("bo@example.org", "approved"));
    assert_eq!(two["plan"]["recipients"][1]["status"], "approved");
    assert_eq!(two["next"]["email"], "di@example.org");
    assert!(refused(&mut a, "signing_plan_advance", step("di@example.org", "approved")).contains("signer"));
    // Earlier signers cannot sign again.
    assert!(refused(&mut a, "signing_plan_advance", step("ana@example.org", "signed")).contains("not that recipient's turn"));
    let done = ok(&mut a, "signing_plan_advance", step("di@example.org", "signed"));
    assert_eq!(done["state"], "completed");
    assert!(done["next"].is_null());
    // The history of a signer grew by one event, later than its first.
    let ana = &done["plan"]["recipients"][0]["history"];
    assert_eq!(ana.as_array().unwrap().len(), 2);
    assert_eq!(ana[1]["status"], "signed");
    assert!(ana[1]["at"].as_i64().unwrap() >= ana[0]["at"].as_i64().unwrap());
    // A finished plan takes no more steps.
    let before = snapshot(dir.path());
    assert!(refused(&mut a, "signing_plan_advance", step("di@example.org", "declined")).contains("finished"));
    assert_eq!(snapshot(dir.path()), before);
}

#[test]
fn a_declined_plan_is_finished_and_needs_replace_to_be_remade() {
    let (dir, mut a) = setup();
    ok(&mut a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": three() }));
    let declined = ok(&mut a, "signing_plan_advance", json!({ "path": "a.pdf", "email": "ana@example.org", "status": "declined" }));
    assert_eq!(declined["state"], "declined");
    assert!(declined["next"].is_null());
    let before = snapshot(dir.path());
    assert!(refused(&mut a, "signing_plan_advance", json!({ "path": "a.pdf", "email": "bo@example.org", "status": "signed" })).contains("declined"));
    // Remaking a plan with history is refused unless replace is given; then it starts over.
    assert!(refused(&mut a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": three() })).contains("replace"));
    assert_eq!(snapshot(dir.path()), before);
    let again = ok(&mut a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": three(), "replace": true }));
    assert_eq!(again["state"], "in_progress");
    assert_eq!((&again["replaced"], &again["discarded_history"]), (&json!(true), &json!(true)));
}

#[test]
fn an_unprogressed_plan_is_replaced_without_replace() {
    let (_dir, mut a) = setup();
    ok(&mut a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": three() }));
    let one = json!([{ "name": "Only", "email": "only@example.org" }]);
    let set = ok(&mut a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": one }));
    assert_eq!(set["plan"]["recipients"].as_array().unwrap().len(), 1);
    // A plan was replaced, but it had no history to lose.
    assert_eq!((&set["replaced"], &set["discarded_history"]), (&json!(true), &json!(false)));
}

#[test]
fn a_first_plan_replaces_nothing_and_discards_nothing() {
    let (_dir, mut a) = setup();
    let set = ok(&mut a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": three() }));
    assert_eq!((&set["replaced"], &set["discarded_history"]), (&json!(false), &json!(false)));
}

#[test]
fn a_clock_set_back_does_not_make_the_history_run_backwards() {
    let (dir, mut a) = setup();
    ok(&mut a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": three() }));
    // The same folder, read by a host whose clock reads long before the plan was made.
    let mut early = Automation::new().with_root(dir.path()).unwrap().with_clock(|| 0);
    let step = ok(&mut early, "signing_plan_advance", json!({ "path": "a.pdf", "email": "ana@example.org", "status": "signed" }));
    let history = step["plan"]["recipients"][0]["history"].as_array().unwrap();
    assert!(history[1]["at"].as_i64().unwrap() >= history[0]["at"].as_i64().unwrap());
    // The file is still one the tools read back.
    assert_eq!(ok(&mut a, "signing_plan_get", json!({ "path": "a.pdf" }))["plan"], step["plan"]);
}

#[test]
fn the_limits_count_characters_for_names_and_messages_and_bytes_for_addresses() {
    let (dir, mut a) = setup();
    let set = |a: &mut Automation, name: &str, email: &str, message: &str| {
        run(a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": [{ "name": name, "email": email }], "message": message }))
    };
    let before = snapshot(dir.path());
    // 200 characters of two bytes each and 2000 characters of three bytes each are accepted; one more is not.
    set(&mut a, &"é".repeat(200), "a@example.org", &"日".repeat(2000)).expect("the limits in characters");
    clear(&mut a);
    assert_eq!(snapshot(dir.path()), before);
    assert!(set(&mut a, &"é".repeat(201), "a@example.org", "").unwrap_err().contains("200 characters"));
    assert!(set(&mut a, "A", "a@example.org", &"日".repeat(2001)).unwrap_err().contains("2000 characters"));
    // An address counts bytes: 254 bytes (124 two-byte characters in 134 characters) and a local part of 64 are accepted.
    let long = format!("a@{}.org", "é".repeat(124));
    assert_eq!(long.len(), 254);
    set(&mut a, "A", &long, "").expect("254 bytes");
    clear(&mut a);
    set(&mut a, "A", &format!("{}@example.org", "é".repeat(32)), "").expect("a local part of 64 bytes");
    clear(&mut a);
    let over = set(&mut a, "A", &format!("a@{}.org", "é".repeat(125)), "").unwrap_err();
    assert!(over.contains("254 bytes") && over.contains("64 bytes"), "{over}");
    let local = set(&mut a, "A", &format!("{}@example.org", "é".repeat(33)), "").unwrap_err();
    assert!(local.contains("64 bytes"), "{local}");
    assert_eq!(snapshot(dir.path()), before);
}

#[test]
fn a_refused_set_writes_nothing_new_and_keeps_the_old_plan() {
    let (dir, mut a) = setup();
    // No plan yet: nothing appears.
    let before = snapshot(dir.path());
    let many: Vec<Value> = (0..51).map(|i| json!({ "name": format!("N{i}"), "email": format!("n{i}@example.org") })).collect();
    let bad: Vec<(&str, Value)> = vec![
        ("empty list", json!([])),
        ("51 recipients", Value::Array(many)),
        ("duplicate address, other case", json!([{ "name": "A", "email": "a@example.org" }, { "name": "B", "email": "A@EXAMPLE.org" }])),
        ("no at sign", json!([{ "name": "A", "email": "example.org" }])),
        ("two at signs", json!([{ "name": "A", "email": "a@b@example.org" }])),
        ("no dot in the domain", json!([{ "name": "A", "email": "a@localhost" }])),
        ("a space in the address", json!([{ "name": "A", "email": "a b@example.org" }])),
        ("empty name", json!([{ "name": "  ", "email": "a@example.org" }])),
        ("name too long", json!([{ "name": "x".repeat(201), "email": "a@example.org" }])),
        ("paragraph separator in a name", json!([{ "name": "A\u{2029}B", "email": "a@example.org" }])),
        ("word joiner in a name", json!([{ "name": "A\u{2060}B", "email": "a@example.org" }])),
        ("isolate in a name", json!([{ "name": "A\u{2066}B", "email": "a@example.org" }])),
        ("byte order mark in a name", json!([{ "name": "A\u{FEFF}B", "email": "a@example.org" }])),
        ("combining grapheme joiner in a name", json!([{ "name": "A\u{034F}B", "email": "a@example.org" }])),
        ("Hangul filler in a name", json!([{ "name": "A\u{115F}B", "email": "a@example.org" }])),
        ("Mongolian vowel separator in a name", json!([{ "name": "A\u{180E}B", "email": "a@example.org" }])),
        ("hidden characters in a name", json!([{ "name": "A\u{061C}B", "email": "a@example.org" }])),
        ("soft hyphen in a name", json!([{ "name": "A\u{00AD}B", "email": "a@example.org" }])),
        ("line separator in a name", json!([{ "name": "A\u{2028}B", "email": "a@example.org" }])),
        ("tag character in a name", json!([{ "name": "A\u{E0041}B", "email": "a@example.org" }])),
        ("filler in a name", json!([{ "name": "A\u{3164}B", "email": "a@example.org" }])),
        ("zero width in an address", json!([{ "name": "A", "email": "a\u{200B}@example.org" }])),
        ("a line break in a name", json!([{ "name": "A\nB", "email": "a@example.org" }])),
        ("direction control in a name", json!([{ "name": "A\u{202E}B", "email": "a@example.org" }])),
        ("email too long", json!([{ "name": "A", "email": format!("{}@example.org", "x".repeat(250)) }])),
        ("email over 254 bytes but under 254 characters", json!([{ "name": "A", "email": format!("a@{}.org", "é".repeat(127)) }])),
        ("local part over 64 bytes", json!([{ "name": "A", "email": format!("{}@example.org", "é".repeat(33)) }])),
        ("unknown role", json!([{ "name": "A", "email": "a@example.org", "role": "boss" }])),
        ("unknown key", json!([{ "name": "A", "email": "a@example.org", "phone": "1" }])),
        ("viewers only", json!([{ "name": "A", "email": "a@example.org", "role": "viewer" }])),
        ("order with a gap", json!([{ "name": "A", "email": "a@example.org", "order": 1 }, { "name": "B", "email": "b@example.org", "order": 3 }])),
        ("order repeated", json!([{ "name": "A", "email": "a@example.org", "order": 1 }, { "name": "B", "email": "b@example.org", "order": 1 }])),
        ("order from zero", json!([{ "name": "A", "email": "a@example.org", "order": 0 }])),
        ("order on some only", json!([{ "name": "A", "email": "a@example.org", "order": 1 }, { "name": "B", "email": "b@example.org" }])),
        ("not an array", json!("a@example.org")),
    ];
    for (what, recipients) in &bad {
        run(&mut a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": recipients })).expect_err(what);
        assert_eq!(snapshot(dir.path()), before, "{what} wrote something");
    }
    // The duplicate is named as such in the refusal, not only caught by the file's own rules.
    let dup = json!([{ "name": "A", "email": "a@example.org" }, { "name": "B", "email": "A@EXAMPLE.org" }]);
    assert!(refused(&mut a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": dup })).contains("already in the list"));
    let message_bad = [
        json!("m".repeat(2001)),
        json!("a\u{0}b"),
        json!(7),
        json!("a\u{2028}b"),
        json!("a\u{7f}b"),
        json!("a\u{85}b"),
        json!("a\u{202E}b"),
        json!("a\u{E0041}b"),
    ];
    for message in message_bad {
        run(&mut a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": three(), "message": message })).expect_err("bad message");
        assert_eq!(snapshot(dir.path()), before);
    }
    for path in ["missing.pdf", "a.txt", "../a.pdf", "."] {
        run(&mut a, "signing_plan_set", json!({ "path": path, "recipients": three() })).expect_err(path);
        assert_eq!(snapshot(dir.path()), before, "{path} wrote something");
    }
    // A message may hold line breaks and tabs.
    ok(&mut a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": three(), "message": "a\nb\tc\r\nd" }));
    clear(&mut a);
    // The limits themselves are accepted.
    let fifty: Vec<Value> = (0..50).map(|i| json!({ "name": "x".repeat(200), "email": format!("n{i}@example.org") })).collect();
    ok(&mut a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": fifty, "message": "m".repeat(2000) }));
    // With a plan in place, a refused set leaves it byte for byte.
    let with_plan = snapshot(dir.path());
    for (what, recipients) in &bad {
        run(&mut a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": recipients })).expect_err(what);
        assert_eq!(snapshot(dir.path()), with_plan, "{what} changed the plan");
    }
}

fn clear(a: &mut Automation) {
    ok(a, "signing_plan_clear", json!({ "path": "a.pdf" }));
}

/// A valid plan of two signers (Ana then Bo), as the tools write it.
fn valid_plan(a: &mut Automation, dir: &Path) -> Value {
    let recipients = json!([{ "name": "Ana Reyes", "email": "ana@example.org" }, { "name": "Bo Chen", "email": "bo@example.org" }]);
    ok(a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": recipients }));
    let plan = serde_json::from_slice(&std::fs::read(sidecar(dir)).unwrap()).unwrap();
    clear(a);
    plan
}

fn edited(base: &Value, change: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut plan = base.clone();
    change(&mut plan);
    serde_json::to_vec(&plan).unwrap()
}

#[test]
fn a_sidecar_that_is_not_a_plan_is_refused_and_never_overwritten_or_deleted() {
    let (dir, mut a) = setup();
    let base = valid_plan(&mut a, dir.path());
    let mut padded = serde_json::to_vec(&base).unwrap();
    padded.resize((1 << 20) + 1, b' ');
    // Each file is valid but for one thing, so only that rule can refuse it.
    let hand_made: Vec<(&str, Vec<u8>, &str)> = vec![
        ("garbage", b"{ this is not json".to_vec(), "not a signing plan"),
        ("empty", b"".to_vec(), "not a signing plan"),
        ("not UTF-8", vec![0xff, 0xfe, 0x00, 0x01], "not a signing plan"),
        ("missing fields", b"{\"version\":1}".to_vec(), "not a signing plan"),
        ("another version", edited(&base, |p| p["version"] = json!(2)), "version 2"),
        ("unknown field", edited(&base, |p| p["extra"] = json!(1)), "not a signing plan"),
        ("an order number off", edited(&base, |p| p["recipients"][1]["order"] = json!(5)), "order numbers are not"),
        ("an address twice", edited(&base, |p| p["recipients"][1]["email"] = json!("ANA@example.org")), "repeats an address"),
        (
            "signed out of turn",
            edited(&base, |p| {
                p["recipients"][1]["status"] = json!("signed");
                p["recipients"][1]["history"] = json!([{ "status": "pending", "at": 1 }, { "status": "signed", "at": 2 }]);
            }),
            "signed, approved or declined out of turn",
        ),
        (
            "approved in the signer's role",
            edited(&base, |p| {
                p["recipients"][0]["status"] = json!("approved");
                p["recipients"][0]["history"] = json!([{ "status": "pending", "at": 1 }, { "status": "approved", "at": 2 }]);
            }),
            "in the wrong role",
        ),
        ("a name not trimmed", edited(&base, |p| p["recipients"][0]["name"] = json!(" Ana Reyes")), "not trimmed"),
        ("an address not trimmed", edited(&base, |p| p["recipients"][0]["email"] = json!("ana@example.org ")), "not trimmed"),
        ("a hidden character in a name", edited(&base, |p| p["recipients"][0]["name"] = json!("Ana\u{202E}Reyes")), "not trimmed"),
        ("a message with a control character", edited(&base, |p| p["message"] = json!("a\u{7}b")), "message"),
        (
            "a history that does not start pending",
            edited(&base, |p| {
                p["recipients"][0]["status"] = json!("sent");
                p["recipients"][0]["history"] = json!([{ "status": "sent", "at": 1 }]);
            }),
            "history",
        ),
        (
            "a history that runs backwards in time",
            edited(&base, |p| {
                p["recipients"][0]["status"] = json!("signed");
                p["recipients"][0]["history"] = json!([{ "status": "pending", "at": 9 }, { "status": "signed", "at": 3 }]);
            }),
            "run backwards",
        ),
        (
            "a history that ends at another status",
            edited(&base, |p| p["recipients"][0]["history"] = json!([{ "status": "pending", "at": 1 }, { "status": "signed", "at": 2 }])),
            "history",
        ),
        ("a valid plan padded past the size limit", padded, "larger than"),
    ];
    for (what, bytes, expected) in hand_made {
        std::fs::write(sidecar(dir.path()), &bytes).unwrap();
        let before = snapshot(dir.path());
        for (tool, args) in [
            ("signing_plan_get", json!({ "path": "a.pdf" })),
            ("signing_plan_set", json!({ "path": "a.pdf", "recipients": three(), "replace": true })),
            ("signing_plan_advance", json!({ "path": "a.pdf", "email": "ana@example.org", "status": "signed" })),
            ("signing_plan_clear", json!({ "path": "a.pdf" })),
        ] {
            let m = refused(&mut a, tool, args);
            assert!(m.contains("left alone") && m.contains(expected), "{what}: {tool}: {m}");
            assert_eq!(snapshot(dir.path()), before, "{what}: {tool} changed the sidecar");
        }
    }
    // The unaltered plan is read, so each refusal above is down to its one change.
    std::fs::write(sidecar(dir.path()), serde_json::to_vec(&base).unwrap()).unwrap();
    assert_eq!(ok(&mut a, "signing_plan_get", json!({ "path": "a.pdf" }))["state"], "in_progress");
}

#[cfg(unix)]
#[test]
fn a_sidecar_that_is_a_link_to_a_valid_plan_is_not_followed() {
    let (dir, mut a) = setup();
    let base = valid_plan(&mut a, dir.path());
    let target = dir.path().join("elsewhere.json");
    let bytes = serde_json::to_vec(&base).unwrap();
    std::fs::write(&target, &bytes).unwrap();
    std::os::unix::fs::symlink(&target, sidecar(dir.path())).unwrap();
    for (tool, args) in [
        ("signing_plan_get", json!({ "path": "a.pdf" })),
        ("signing_plan_clear", json!({ "path": "a.pdf" })),
        ("signing_plan_set", json!({ "path": "a.pdf", "recipients": three(), "replace": true })),
        ("signing_plan_advance", json!({ "path": "a.pdf", "email": "ana@example.org", "status": "signed" })),
    ] {
        let m = refused(&mut a, tool, args);
        assert!(m.contains("not a plain file"), "{tool}: {m}");
    }
    assert_eq!(std::fs::read(&target).unwrap(), bytes);
    assert!(std::fs::symlink_metadata(sidecar(dir.path())).unwrap().file_type().is_symlink());
}

#[test]
fn a_plan_with_a_recipient_marked_sent_is_not_overwritten_without_replace() {
    let (dir, mut a) = setup();
    let base = valid_plan(&mut a, dir.path());
    let sent = edited(&base, |p| {
        p["recipients"][0]["status"] = json!("sent");
        p["recipients"][0]["history"] = json!([{ "status": "pending", "at": 1 }, { "status": "sent", "at": 2 }]);
    });
    std::fs::write(sidecar(dir.path()), &sent).unwrap();
    let before = snapshot(dir.path());
    assert!(refused(&mut a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": three() })).contains("replace"));
    assert_eq!(snapshot(dir.path()), before);
    ok(&mut a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": three(), "replace": true }));
    assert_ne!(snapshot(dir.path()), before);
}

#[test]
fn files_that_differ_only_in_the_case_of_the_extension_keep_separate_plans() {
    let (dir, mut a) = setup();
    std::fs::write(dir.path().join("a.PDF"), b"%PDF-1.7\n").unwrap();
    if std::fs::read_dir(dir.path()).unwrap().count() != 2 {
        eprintln!("a.pdf and a.PDF are one file here (a case-insensitive folder); nothing to compare");
        return;
    }
    let one = json!([{ "name": "One", "email": "one@example.org" }]);
    let two = json!([{ "name": "Two", "email": "two@example.org" }]);
    ok(&mut a, "signing_plan_set", json!({ "path": "a.pdf", "recipients": one }));
    ok(&mut a, "signing_plan_set", json!({ "path": "a.PDF", "recipients": two }));
    assert!(dir.path().join("a.pdf.pdfcraft-sign.json").is_file() && dir.path().join("a.PDF.pdfcraft-sign.json").is_file());
    let first = ok(&mut a, "signing_plan_get", json!({ "path": "a.pdf" }));
    let second = ok(&mut a, "signing_plan_get", json!({ "path": "a.PDF" }));
    assert_ne!(first["plan"]["recipients"][0]["email"], second["plan"]["recipients"][0]["email"]);
}

#[test]
fn the_plan_stays_inside_the_roots() {
    let base = tempfile::tempdir().unwrap();
    let (read, write) = (base.path().join("read"), base.path().join("write"));
    std::fs::create_dir_all(&read).unwrap();
    std::fs::create_dir_all(&write).unwrap();
    std::fs::write(read.join("a.pdf"), b"%PDF-1.7\n").unwrap();
    let roots = AutomationRoots::new(Some(&read), Some(&write)).unwrap();
    let mut a = Automation::new().with_automation_roots(roots);
    // The sidecar would be beside the PDF, in the read root: writing there is refused.
    let args = json!({ "path": read.join("a.pdf").to_string_lossy(), "recipients": three() });
    let m = run(&mut a, "signing_plan_set", args).expect_err("the write root does not hold the sidecar");
    assert!(m.contains("a.pdf.pdfcraft-sign.json"), "the refusal names the sidecar: {m}");
    assert!(!read.join("a.pdf.pdfcraft-sign.json").exists());
    // A PDF outside both roots is refused.
    std::fs::write(base.path().join("out.pdf"), b"%PDF-1.7\n").unwrap();
    run(&mut a, "signing_plan_get", json!({ "path": base.path().join("out.pdf").to_string_lossy() })).expect_err("outside the read root");
    // With one folder for both, it works.
    let roots = AutomationRoots::new(Some(&read), Some(&read)).unwrap();
    let mut a = Automation::new().with_automation_roots(roots);
    ok(&mut a, "signing_plan_set", json!({ "path": read.join("a.pdf").to_string_lossy(), "recipients": three() }));
    assert!(read.join("a.pdf.pdfcraft-sign.json").is_file());
}
