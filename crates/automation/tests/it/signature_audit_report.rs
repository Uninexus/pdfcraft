//! `signature_audit_report`: the signatures of an open document as a structured report, and the
//! same facts as a new PDF, made from files signed with the signing tools.

use pdfcraft_automation::{Automation, Content, ToolError};
use pdfcraft_engine::Session;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn tool(a: &mut Automation, name: &str, args: Value) -> Value {
    match a.call(name, &args).unwrap().remove(0) {
        Content::Json(value) => value,
        other => panic!("expected JSON: {other:?}"),
    }
}

fn refused(a: &mut Automation, args: Value) -> String {
    match a.call("signature_audit_report", &args) {
        Err(ToolError::InvalidArgs(m) | ToolError::Failed(m)) => m,
        other => panic!("not refused: {other:?}"),
    }
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// A folder with a two-page a.pdf and the tools rooted in it, the clock pinned.
fn setup() -> (tempfile::TempDir, Automation) {
    let dir = tempfile::tempdir().unwrap();
    let mut session = Session::new();
    let bytes = session.create_blank(300.0, 400.0, 2).unwrap();
    let id = session.open("a.pdf", None, bytes, None).unwrap();
    std::fs::write(dir.path().join("a.pdf"), session.save_bytes(id).unwrap().as_slice()).unwrap();
    let a = Automation::new().with_root(dir.path()).unwrap().with_clock(|| 1_700_000_000);
    (dir, a)
}

fn open(a: &mut Automation, path: &str) -> u64 {
    tool(a, "doc_open", json!({ "path": path }))["doc"].as_u64().unwrap()
}

/// Sign `from` as `who` into `out`, with `extra` arguments (a certification level, say), and open it.
fn sign(a: &mut Automation, from: &str, who: &str, out: &str, extra: Value) -> u64 {
    let id = format!("{who}.p12");
    tool(a, "sign_id_create", json!({ "name": who, "key": "p256", "password": "secret12", "path": id }));
    let doc = open(a, from);
    let mut args = json!({ "doc": doc, "id": id, "password": "secret12", "out": out, "reason": "Approved", "location": "London" });
    for (k, v) in extra.as_object().unwrap() {
        args[k] = v.clone();
    }
    tool(a, "sign_document", args);
    open(a, out)
}

fn text_of(a: &mut Automation, doc: u64) -> String {
    let pages = tool(a, "text_extract", json!({ "doc": doc }));
    pages["pages"].as_array().unwrap().iter().map(|p| p["text"].as_str().unwrap().to_string()).collect::<Vec<_>>().join("\n")
}

fn squeezed(s: &str) -> String {
    s.split_whitespace().collect()
}

#[test]
fn a_signed_file_is_reported_with_the_signer_hash_time_coverage_and_the_files_fingerprint() {
    let (dir, mut a) = setup();
    let doc = sign(&mut a, "a.pdf", "Ada Lovelace", "signed.pdf", json!({}));
    let r = tool(&mut a, "signature_audit_report", json!({ "doc": doc }));
    assert_eq!((r["pages"].as_u64(), r["found"].as_u64(), r["signed"].as_u64()), (Some(2), Some(1), Some(1)), "{r}");
    // The fingerprint is that of the file on disk, which is what was opened unchanged.
    assert_eq!(r["sha256"], hex(&std::fs::read(dir.path().join("signed.pdf")).unwrap()), "{r}");
    assert_eq!(r["bytes"].as_u64(), Some(std::fs::metadata(dir.path().join("signed.pdf")).unwrap().len()));
    assert_eq!((r["unsaved_changes"].as_bool(), r["changed_on_open"].as_bool()), (Some(false), Some(false)));
    let s = &r["signatures"][0];
    assert_eq!((s["kind"].as_str(), s["signer"].as_str()), (Some("signature"), Some("Ada Lovelace")), "{s}");
    assert!(s["certificate_subject"].as_str().unwrap().contains("Ada Lovelace"), "{s}");
    assert_eq!((s["self_signed"].as_bool(), s["hash_algorithm"].as_str()), (Some(true), Some("SHA-256")), "{s}");
    // Nobody trusts the new ID yet: intact, signer unknown. A timestamp is not claimed.
    assert_eq!((s["status"].as_str(), s["timestamp"].as_bool()), (Some("unknown"), Some(false)), "{s}");
    assert!(s["revision"].as_u64().is_some_and(|r| r >= 1), "{s}");
    assert!(s["claimed_time"].as_str().is_some_and(|t| t.starts_with("D:2023")), "{s}");
    assert!(s["signing_time"].as_str().is_some_and(|t| t.contains("2023")), "{s}");
    assert_eq!((s["covers"].as_str(), s["covers_whole_file"].as_bool()), (Some("whole_file"), Some(true)), "{s}");
    assert!(s["certification_level"].is_null(), "an approval signature has no level: {s}");
    assert_eq!((s["reason"].as_str(), s["location"].as_str(), s["page"].as_u64()), (Some("Approved"), Some("London"), Some(1)), "{s}");
    assert!(!s["reasons"].as_array().unwrap().is_empty(), "the validator's sentences are passed on: {s}");
    // It says what was and was not checked, and that it is not a signed or legal document.
    let not_checked = r["not_checked"].to_string();
    assert!(not_checked.contains("No online revocation check") && not_checked.contains("not itself signed"), "{not_checked}");
    assert!(r["generated"].as_str().unwrap().contains("not itself signed"), "{r}");
    assert_eq!(r["trusted_certificates"], 0);
    // The verdict follows the validator: trusting the ID makes it valid, as sign_list says.
    tool(&mut a, "sign_trust", json!({ "paths": ["Ada Lovelace.p12"], "password": "secret12" }));
    let r = tool(&mut a, "signature_audit_report", json!({ "doc": doc }));
    let listed = tool(&mut a, "sign_list", json!({ "doc": doc }));
    assert_eq!(
        (r["signatures"][0]["status"].as_str(), r["valid"].as_u64(), r["trusted_certificates"].as_u64()),
        (Some("valid"), Some(1), Some(1)),
        "{r}"
    );
    assert_eq!(listed["signatures"][0]["status"], "valid");
    // Reading changed nothing.
    assert_eq!(r["unsaved_changes"], false);
}

#[test]
fn the_first_of_two_signatures_permits_the_second_and_only_the_last_covers_the_whole_file() {
    let (_dir, mut a) = setup();
    sign(&mut a, "a.pdf", "Ada Lovelace", "one.pdf", json!({}));
    let doc = sign(&mut a, "one.pdf", "Grace Hopper", "two.pdf", json!({}));
    let r = tool(&mut a, "signature_audit_report", json!({ "doc": doc }));
    assert_eq!((r["found"].as_u64(), r["signed"].as_u64(), r["unknown"].as_u64(), r["listed"].as_u64()), (Some(2), Some(2), Some(2), Some(2)), "{r}");
    let signers: Vec<&str> = r["signatures"].as_array().unwrap().iter().map(|s| s["signer"].as_str().unwrap()).collect();
    assert!(signers.contains(&"Ada Lovelace") && signers.contains(&"Grace Hopper"), "{signers:?}");
    let by = |name: &str| r["signatures"].as_array().unwrap().iter().find(|s| s["signer"] == name).cloned().unwrap();
    let (first, second) = (by("Ada Lovelace"), by("Grace Hopper"));
    assert_eq!(second["covers_whole_file"], true, "{second}");
    // The second signature was added after the first: the file changed, and the first permits it.
    assert_eq!((first["covers_whole_file"].as_bool(), first["covers"].as_str()), (Some(false), Some("changes_allowed")), "{first}");
    assert!(!first["changes_after_signing"].as_array().unwrap().is_empty(), "{first}");
    assert!(first["revision"].as_u64() < second["revision"].as_u64(), "{r}");
}

#[test]
fn changes_a_certification_forbids_are_reported_as_changes_after_signing() {
    let (dir, mut a) = setup();
    let doc = sign(&mut a, "a.pdf", "Ada Lovelace", "cert.pdf", json!({ "certify": "no_changes" }));
    let before = tool(&mut a, "signature_audit_report", json!({ "doc": doc }));
    let s = &before["signatures"][0];
    assert_eq!((s["certification_level"].as_u64(), s["covers"].as_str()), (Some(1), Some("whole_file")), "{s}");
    let revision = s["revision"].as_u64();
    assert!(revision.is_some(), "{s}");
    tool(&mut a, "comment_add", json!({ "doc": doc, "page": 1, "type": "note", "at": [20, 20], "contents": "late" }));
    let r = tool(&mut a, "signature_audit_report", json!({ "doc": doc }));
    let s = &r["signatures"][0];
    assert_eq!(
        (s["covers"].as_str(), s["covers_whole_file"].as_bool(), s["status"].as_str()),
        (Some("changes_disallowed"), Some(false), Some("invalid")),
        "{s}"
    );
    assert_eq!((r["invalid"].as_u64(), r["unsaved_changes"].as_bool()), (Some(1), Some(true)), "{r}");
    assert!(!s["changes_after_signing"].as_array().unwrap().is_empty(), "{s}");
    // The signed revision is the same one; an invalid signature still says which it covers.
    assert_eq!(s["revision"].as_u64(), revision, "{s}");
    // The fingerprint is of the document as it is now, not of the file on disk, and says so.
    assert_ne!(r["sha256"], hex(&std::fs::read(dir.path().join("cert.pdf")).unwrap()), "{r}");
    let out = tool(&mut a, "signature_audit_report", json!({ "doc": doc, "output": "late.pdf" }));
    assert!(out["output"]["bytes"].as_u64().unwrap() > 0, "{out}");
    let report = open(&mut a, "late.pdf");
    let text = squeezed(&text_of(&mut a, report));
    assert!(text.contains(&squeezed("changes that are not saved")), "{text}");
    assert!(text.contains(&squeezed("it does not permit them")), "{text}");
}

#[test]
fn an_unsigned_document_gives_an_empty_report_and_no_report_file() {
    let (dir, mut a) = setup();
    let doc = open(&mut a, "a.pdf");
    let r = tool(&mut a, "signature_audit_report", json!({ "doc": doc }));
    assert_eq!((r["found"].as_u64(), r["signed"].as_u64(), r["listed"].as_u64(), r["pages"].as_u64()), (Some(0), Some(0), Some(0), Some(2)), "{r}");
    assert_eq!(r["sha256"], hex(&std::fs::read(dir.path().join("a.pdf")).unwrap()));
    let m = refused(&mut a, json!({ "doc": doc, "output": "none.pdf" }));
    assert!(m.contains("no signatures"), "{m}");
    assert!(!dir.path().join("none.pdf").exists(), "nothing written");
    // An empty signature field is listed as such and is not a signature.
    tool(&mut a, "form_add_field", json!({ "doc": doc, "page": 1, "type": "signature", "name": "Sig1", "rect": [20.0, 100.0, 180.0, 130.0] }));
    let r = tool(&mut a, "signature_audit_report", json!({ "doc": doc }));
    assert_eq!((r["found"].as_u64(), r["signed"].as_u64()), (Some(1), Some(0)), "{r}");
    assert_eq!((r["signatures"][0]["kind"].as_str(), r["signatures"][0]["status"].as_str()), (Some("unsigned_field"), Some("unsigned")), "{r}");
    // An empty field covers nothing: no revision, and no coverage to state.
    let e = &r["signatures"][0];
    assert!(e["revision"].is_null() && e["covers_whole_file"].is_null() && e["covers"] == "not_applicable", "{e}");
}

#[test]
fn the_report_file_is_a_pdf_that_says_what_it_is_and_what_it_did_not_check() {
    let (dir, mut a) = setup();
    let doc = sign(&mut a, "a.pdf", "Ada Lovelace", "signed.pdf", json!({}));
    let r = tool(&mut a, "signature_audit_report", json!({ "doc": doc, "output": "report.pdf" }));
    assert_eq!(r["output"]["bytes"].as_u64(), Some(std::fs::metadata(dir.path().join("report.pdf")).unwrap().len()), "{r}");
    let report = open(&mut a, "report.pdf");
    // A PDF made from the facts, with no signature of its own.
    assert_eq!(tool(&mut a, "sign_list", json!({ "doc": report }))["count"], 0);
    let plain = text_of(&mut a, report);
    let text = squeezed(&plain);
    for needle in [
        "Signature audit report",
        "summary generated locally from the signatures in the file",
        "It is not itself signed",
        r["sha256"].as_str().unwrap(),
        "Ada Lovelace",
        "SHA-256",
        "Pages: 2",
        "the signature covers the whole file",
        "No online revocation check was made",
        "not shown to be invalid but not shown to be valid either",
        "A signer counts as trusted if its certificate, or a certificate it chains to, is on that list",
        "Revocation of intermediate certificates was not checked",
    ] {
        assert!(text.contains(&squeezed(needle)), "missing {needle:?}: {text}");
    }
    assert!(text.contains(&squeezed("says nothing about whether a signature has legal effect")), "{text}");
    // The report is not signed and is not claimed to be anything but a summary.
    assert!(!plain.to_lowercase().contains("certificate of"), "{plain}");
    // Refused: outside the write root, or over the document's own file.
    assert!(a.call("signature_audit_report", &json!({ "doc": doc, "output": "../outside.pdf" })).is_err());
    let before = std::fs::read(dir.path().join("signed.pdf")).unwrap();
    let m = refused(&mut a, json!({ "doc": doc, "output": "signed.pdf" }));
    assert!(m.contains("own file"), "{m}");
    assert_eq!(std::fs::read(dir.path().join("signed.pdf")).unwrap(), before, "the signed file is untouched");
}

#[test]
fn hostile_signature_data_is_cut_cleaned_counted_and_never_a_panic() {
    let (dir, mut a) = setup();
    // A signer whose name is hundreds of characters with a line break in it: the report cuts it.
    let long = format!("Mallory\nEvil{}", "x".repeat(300));
    tool(&mut a, "sign_id_create", json!({ "name": long, "key": "p256", "password": "secret12", "path": "m.p12" }));
    let doc = open(&mut a, "a.pdf");
    tool(&mut a, "sign_document", json!({ "doc": doc, "id": "m.p12", "password": "secret12", "out": "m.pdf" }));
    let doc = open(&mut a, "m.pdf");
    let r = tool(&mut a, "signature_audit_report", json!({ "doc": doc, "output": "m-report.pdf" }));
    let s = &r["signatures"][0];
    let signer = s["signer"].as_str().unwrap();
    assert!(signer.chars().count() <= 240 && !signer.contains('\n') && signer.starts_with("Mallory Evil"), "{signer}");
    assert_eq!((s["text_cut"].as_bool(), s["reasons_cut"].as_bool()), (Some(true), Some(false)), "{s}");
    assert!(std::fs::metadata(dir.path().join("m-report.pdf")).unwrap().len() > 0);
    let report = open(&mut a, "m-report.pdf");
    assert!(squeezed(&text_of(&mut a, report)).contains("cutat240characters"));
    // Characters that reorder or hide text are dropped, in a signer's name and in the document's own.
    let sneaky = "Eve\u{202E}gnp\u{200B}\u{FEFF}.\u{2028}Smith";
    tool(&mut a, "sign_id_create", json!({ "name": sneaky, "key": "p256", "password": "secret12", "path": "e.p12" }));
    let doc = open(&mut a, "a.pdf");
    tool(&mut a, "sign_document", json!({ "doc": doc, "id": "e.p12", "password": "secret12", "out": "e\u{202E}\u{200B}.pdf" }));
    let doc = open(&mut a, "e\u{202E}\u{200B}.pdf");
    let r = tool(&mut a, "signature_audit_report", json!({ "doc": doc, "output": "e-report.pdf" }));
    let hidden = |t: &str| {
        t.chars().any(|c| matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}' | '\u{2028}' | '\u{2029}'))
    };
    let (signer, name) = (r["signatures"][0]["signer"].as_str().unwrap(), r["document"].as_str().unwrap());
    assert!(!hidden(signer) && !hidden(name), "{signer:?} {name:?}");
    assert!(signer.starts_with("Eve") && signer.contains("Smith") && name.starts_with('e'), "{signer:?} {name:?}");
    let rep = open(&mut a, "e-report.pdf");
    assert!(!hidden(&text_of(&mut a, rep)));
    // More signature fields than the report lists: the counts cover them all.
    let many = open(&mut a, "a.pdf");
    for i in 0..60 {
        tool(
            &mut a,
            "form_add_field",
            json!({ "doc": many, "page": 1, "type": "signature", "name": format!("S{i}"), "rect": [20.0, 20.0, 80.0, 40.0] }),
        );
    }
    let r = tool(&mut a, "signature_audit_report", json!({ "doc": many }));
    assert_eq!((r["found"].as_u64(), r["listed"].as_u64(), r["truncated"].as_bool()), (Some(60), Some(50), Some(true)), "{r}");
    assert_eq!(r["signatures"].as_array().unwrap().len(), 50);
}

#[test]
fn a_signature_with_no_usable_byte_range_is_invalid_with_reasons_and_no_coverage_claim() {
    use pdfcraft_cos::{Dict, Document, Lexer, Object, SaveOptions};
    let (dir, mut a) = setup();
    let mut cos = Document::open(std::sync::Arc::new(std::fs::read(dir.path().join("a.pdf")).unwrap())).unwrap();
    let dict = |text: &str| -> Dict { Lexer::new(text.as_bytes(), 0).object().unwrap().as_dict().unwrap().clone() };
    // A byte range that covers nothing (the signature is refused before its contents are read).
    let value = cos.add(dict("<< /Type /Sig /SubFilter /ETSI.CAdES.detached /ByteRange [0 0 0 0] /Contents <00> /Reason (x\\n\\r\\000y) >>"));
    let page = pdfcraft_organize::pages(&cos).unwrap()[0].obj;
    let widget = cos.add(dict(&format!("<< /FT /Sig /T (Bad) /Subtype /Widget /F 4 /Rect [50 50 150 150] /P {page} /V {value} >>")));
    cos.update_dict(page, |p| p.set(b"Annots".to_vec(), vec![Object::Ref(widget)])).unwrap();
    cos.update_dict(cos.root().unwrap(), |root| root.set(b"AcroForm".to_vec(), dict(&format!("<< /Fields [{widget}] >>")))).unwrap();
    std::fs::write(dir.path().join("bad.pdf"), pdfcraft_cos::write_full(&cos, &SaveOptions::default()).unwrap()).unwrap();
    let doc = open(&mut a, "bad.pdf");
    let r = tool(&mut a, "signature_audit_report", json!({ "doc": doc, "output": "bad-report.pdf" }));
    let s = &r["signatures"][0];
    assert_eq!((s["status"].as_str(), r["invalid"].as_u64(), s["signer"].is_null()), (Some("invalid"), Some(1), true), "{r}");
    assert!(!s["reasons"].as_array().unwrap().is_empty(), "{s}");
    assert!(!s["reason"].as_str().unwrap_or_default().contains(['\n', '\r', '\0']), "{s}");
    // Validation stopped before the byte range was read, so nothing is claimed about coverage.
    assert_eq!((s["covers"].as_str(), s["revision"].is_null(), s["covers_whole_file"].is_null()), (Some("not_determined"), true, true), "{s}");
    assert!(dir.path().join("bad-report.pdf").exists());
}

#[test]
fn a_document_timestamp_that_does_not_validate_is_not_called_validated() {
    use pdfcraft_cos::{Dict, Document, Lexer, SaveOptions};
    let (dir, mut a) = setup();
    let mut cos = Document::open(std::sync::Arc::new(std::fs::read(dir.path().join("a.pdf")).unwrap())).unwrap();
    let dict = |text: &str| -> Dict { Lexer::new(text.as_bytes(), 0).object().unwrap().as_dict().unwrap().clone() };
    // A document timestamp with an empty byte range and a token that is not one.
    let stamp = cos.add(dict("<< /Type /DocTimeStamp /SubFilter /ETSI.RFC3161 /ByteRange [0 0 0 0] /Contents <3080> >>"));
    cos.update_dict(cos.root().unwrap(), |root| root.set(b"Extras".to_vec(), pdfcraft_cos::Object::Ref(stamp))).unwrap();
    std::fs::write(dir.path().join("ts.pdf"), pdfcraft_cos::write_full(&cos, &SaveOptions::default()).unwrap()).unwrap();
    let doc = open(&mut a, "ts.pdf");
    let r = tool(&mut a, "signature_audit_report", json!({ "doc": doc, "output": "ts-report.pdf" }));
    let s = &r["signatures"][0];
    assert_eq!((s["kind"].as_str(), s["timestamp"].as_bool()), (Some("document_timestamp"), Some(true)), "{r}");
    assert_ne!(s["status"], "valid", "{s}");
    assert_eq!((s["covers"].as_str(), s["revision"].is_null()), (Some("not_determined"), true), "{s}");
    let rep = open(&mut a, "ts-report.pdf");
    let text = squeezed(&text_of(&mut a, rep));
    assert!(!text.contains(&squeezed("Timestamp: validated")), "{text}");
    // A document timestamp is not an approval signature and has no certification level.
    assert!(!text.contains("Certificationlevel") && !text.contains("approvalsignature"), "{text}");
}

#[test]
fn the_report_file_goes_to_the_write_root_and_never_to_the_read_root() {
    use pdfcraft_automation::file_access::AutomationRoots;
    let (dir, mut a) = setup();
    sign(&mut a, "a.pdf", "Ada Lovelace", "signed.pdf", json!({}));
    let write = tempfile::tempdir().unwrap();
    let roots = AutomationRoots::new(Some(dir.path()), Some(write.path())).unwrap();
    let mut b = Automation::new().with_automation_roots(roots);
    let doc = open(&mut b, dir.path().join("signed.pdf").to_str().unwrap());
    // Reading the report needs only the read root; the file goes to the write root.
    let ok = write.path().join("report.pdf");
    let r = tool(&mut b, "signature_audit_report", json!({ "doc": doc, "output": ok.to_str().unwrap() }));
    assert!(ok.exists() && r["output"]["bytes"].as_u64().unwrap() > 0, "{r}");
    // The read root is not writable, so the report cannot go there, and nothing is made.
    let inside = dir.path().join("report.pdf");
    assert!(b.call("signature_audit_report", &json!({ "doc": doc, "output": inside.to_str().unwrap() })).is_err());
    assert!(!inside.exists());
}
