//! Hand-off packages for the local signing plan: `signing_handoff_export` and
//! `signing_handoff_import`. The files on disk are compared before and after every refusal, and the
//! sign crate's validator (`sign_list`) is the oracle for a signed document surviving the trip.
//! All people are made up and at example.org; the digital IDs are self-signed test keys made here.

use std::path::{Path, PathBuf};

use craft_zip::{Archive, Compression, Limits, Writer};
use pdfcraft_automation::file_access::AutomationRoots;
use pdfcraft_automation::{Automation, Content, ToolError};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn tool(a: &mut Automation, name: &str, args: Value) -> Value {
    match a.call(name, &args).unwrap().remove(0) {
        Content::Json(value) => value,
        other => panic!("expected JSON: {other:?}"),
    }
}

fn refused(a: &mut Automation, name: &str, args: Value) -> String {
    match a.call(name, &args) {
        Err(ToolError::InvalidArgs(m) | ToolError::Failed(m)) => m,
        other => panic!("not refused: {other:?}"),
    }
}

/// Every file under `dir` with its bytes, for before and after comparisons.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((p.clone(), std::fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// A saved form `form.pdf` with signature fields labelled for Ana and Bo, a plan beside it, and ID
/// files for both, in a folder `dir`, opened as document `doc` by `a`.
struct Rig {
    dir: tempfile::TempDir,
    a: Automation,
    doc: u64,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let mut session = pdfcraft_engine::Session::new();
    let bytes = session.create_blank(300.0, 400.0, 1).unwrap();
    let id = session.open("a.pdf", None, bytes, None).unwrap();
    std::fs::write(dir.path().join("a.pdf"), session.save_bytes(id).unwrap().as_slice()).unwrap();
    let mut a = Automation::new().with_root(dir.path()).unwrap().with_clock(|| 1_700_000_000);
    let doc = tool(&mut a, "doc_open", json!({ "path": "a.pdf" }))["doc"].as_u64().unwrap();
    for (name, y) in [("SigAna", 60.0), ("SigBo", 120.0)] {
        tool(&mut a, "form_add_field", json!({ "doc": doc, "page": 1, "type": "signature", "name": name, "rect": [20.0, y, 180.0, y + 40.0] }));
    }
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "form.pdf" }));
    let recipients = json!([
        { "name": "Ana Example", "email": "ana@example.org" },
        { "name": "Bo Example", "email": "bo@example.org" },
        { "name": "Cy Example", "email": "cy@example.org", "role": "viewer" },
    ]);
    tool(&mut a, "signing_plan_set", json!({ "path": "form.pdf", "recipients": recipients }));
    let doc = tool(&mut a, "doc_open", json!({ "path": "form.pdf" }))["doc"].as_u64().unwrap();
    tool(&mut a, "form_field_signer", json!({ "doc": doc, "field": "SigAna", "signer": "ana@example.org" }));
    tool(&mut a, "form_field_signer", json!({ "doc": doc, "field": "SigBo", "signer": "bo@example.org" }));
    tool(&mut a, "doc_save", json!({ "doc": doc, "path": "form.pdf" }));
    for (name, file) in [("Ana Example", "ana.p12"), ("Bo Example", "bo.p12")] {
        tool(&mut a, "sign_id_create", json!({ "name": name, "key": "p256", "password": "secret12", "path": file }));
    }
    Rig { dir, a, doc }
}

impl Rig {
    fn file(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.dir.path().join(name)).unwrap()
    }

    fn export(&mut self, out: &str) -> Value {
        tool(&mut self.a, "signing_handoff_export", json!({ "doc": self.doc, "path": out }))
    }

    fn import(&mut self, zip: &str, folder: &str) -> Value {
        tool(&mut self.a, "signing_handoff_import", json!({ "path": zip, "folder": folder }))
    }

    fn import_refused(&mut self, zip: &str, folder: &str) -> String {
        refused(&mut self.a, "signing_handoff_import", json!({ "path": zip, "folder": folder }))
    }

    fn sign(&mut self, doc: u64, email: &str, id: &str) -> Value {
        tool(&mut self.a, "signing_plan_sign", json!({ "doc": doc, "email": email, "id": id, "password": "secret12" }))
    }

    fn statuses(&mut self, doc: u64) -> Vec<(String, String)> {
        tool(&mut self.a, "sign_list", json!({ "doc": doc }))["signatures"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| (s["field"].as_str().unwrap().to_string(), s["status"].as_str().unwrap().to_string()))
            .collect()
    }
}

/// The entries of a package, by name.
fn entries(zip: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = Archive::open(zip, Limits { max_entries: 16, max_total_bytes: 1 << 28 }).unwrap();
    let names: Vec<String> = archive.names().map(|n| n.to_string()).collect();
    names.into_iter().map(|n| (n.clone(), archive.read(&n, 1 << 28).unwrap().unwrap())).collect()
}

fn zip_of(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = Writer::new();
    for (name, data) in parts {
        w.add(name, data, Compression::Deflated).unwrap();
    }
    w.finish().unwrap()
}

/// A package like the one the tool wrote, with `change` applied to each entry (name, bytes).
fn repack(zip: &[u8], change: impl Fn(&str, Vec<u8>) -> Vec<u8>) -> Vec<u8> {
    let parts: Vec<(String, Vec<u8>)> = entries(zip).into_iter().map(|(n, d)| (n.clone(), change(&n, d))).collect();
    let parts: Vec<(&str, &[u8])> = parts.iter().map(|(n, d)| (n.as_str(), d.as_slice())).collect();
    zip_of(&parts)
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// A manifest as the tool writes it, for `pdf`, with `edit` applied to its JSON.
fn manifest_with(zip: &[u8], edit: impl Fn(&mut Value)) -> Vec<u8> {
    let (_, bytes) = entries(zip).into_iter().find(|(n, _)| n == "manifest.json").unwrap();
    let mut m: Value = serde_json::from_slice(&bytes).unwrap();
    edit(&mut m);
    serde_json::to_vec(&m).unwrap()
}

#[test]
fn a_package_round_trips_the_pdf_bytes_and_the_plan_and_holds_nothing_else() {
    let mut r = rig();
    let pdf = r.file("form.pdf");
    let plan = r.file("form.pdf.pdfcraft-sign.json");
    let exported = r.export("out.zip");
    assert_eq!(exported["pdf_sha256"], sha(&pdf));
    let zip = r.file("out.zip");
    let parts = entries(&zip);
    let mut names: Vec<&str> = parts.iter().map(|(n, _)| n.as_str()).collect();
    names.sort();
    assert_eq!(names, ["document.pdf", "manifest.json", "plan.json"], "a package holds only these");
    // No secret: the manifest has only the documented keys, and neither the password, the ID files
    // nor their paths are anywhere in the package.
    let manifest: Value = serde_json::from_slice(&parts.iter().find(|(n, _)| n == "manifest.json").unwrap().1).unwrap();
    let mut keys: Vec<&str> = manifest.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["file_name", "format", "pdf_bytes", "pdf_sha256", "plan", "plan_sha256", "version"]);
    for (name, data) in &parts {
        if name != "document.pdf" {
            let text = String::from_utf8_lossy(data);
            for secret in ["secret12", "p12", "password", "key"] {
                assert!(!text.contains(secret), "{name} mentions {secret}");
            }
        }
    }
    assert_eq!(manifest["plan"]["recipients"].as_array().unwrap().len(), 3);
    assert_eq!(manifest["plan"]["recipients"][1]["email"], "bo@example.org");
    assert_eq!(manifest["plan"]["recipients"][0]["status"], "pending");

    let imported = r.import("out.zip", "inbox");
    assert_eq!(r.file("inbox/form.pdf"), pdf, "the PDF keeps every byte");
    assert_eq!(r.file("inbox/form.pdf.pdfcraft-sign.json"), plan, "the plan is the one that was beside the PDF");
    assert_eq!(imported["next"]["email"], "ana@example.org");
    assert_eq!(imported["signatures_checked"], false);
    assert_eq!(imported["sent"], false);
    let there = tool(&mut r.a, "signing_plan_get", json!({ "path": "inbox/form.pdf" }));
    assert_eq!(there["plan"], tool(&mut r.a, "signing_plan_get", json!({ "path": "form.pdf" }))["plan"]);
}

#[test]
fn a_signed_document_round_trips_still_validates_and_the_next_recipient_can_sign_there() {
    let mut r = rig();
    let signed = r.sign(r.doc, "ana@example.org", "ana.p12");
    assert_eq!(signed["status"], "complete");
    let before = r.statuses(r.doc);
    r.export("out.zip");
    let imported = r.import("out.zip", "inbox");
    assert_eq!(imported["next"]["email"], "bo@example.org", "the plan says Ana signed");
    assert_eq!(r.file("inbox/form.pdf"), r.file("form.pdf"));
    let there = tool(&mut r.a, "doc_open", json!({ "path": "inbox/form.pdf" }))["doc"].as_u64().unwrap();
    let after = r.statuses(there);
    assert_eq!(after, before, "the validator reads the same signatures after the trip");
    assert!(after.iter().any(|(f, s)| f == "SigAna" && s != "invalid"));
    // The trip lost nothing the next turn needs: Bo signs the imported copy, Ana's signature stays.
    let bo = r.sign(there, "bo@example.org", "bo.p12");
    assert_eq!(bo["status"], "complete", "{bo}");
    let finished = r.statuses(there);
    assert_eq!(finished.iter().find(|(f, _)| f == "SigAna"), before.iter().find(|(f, _)| f == "SigAna"));
    assert_ne!(r.file("inbox/form.pdf"), r.file("form.pdf"), "only the imported copy took Bo's turn");
}

#[test]
fn a_pdf_that_does_not_match_the_manifest_is_refused_and_nothing_is_written() {
    let mut r = rig();
    r.export("out.zip");
    let zip = r.file("out.zip");
    // One byte of the PDF changed, its length kept: only the hash can tell.
    let tampered = repack(&zip, |n, mut d| {
        if n == "document.pdf" {
            let last = d.len() - 1;
            d[last] ^= 1;
        }
        d
    });
    std::fs::write(r.dir.path().join("tampered.zip"), tampered).unwrap();
    let before = snapshot(r.dir.path());
    let m = r.import_refused("tampered.zip", "inbox");
    assert!(m.contains("hash"), "{m}");
    assert!(snapshot(r.dir.path()) == before, "a refused import wrote something");
    assert!(!r.dir.path().join("inbox").exists());

    // A manifest whose hash is right for other bytes, and one for another size, are refused too.
    let other = zip_of(&[
        ("manifest.json", &manifest_with(&zip, |m| m["pdf_bytes"] = json!(3))),
        ("document.pdf", &r.file("form.pdf")),
        ("plan.json", &r.file("form.pdf.pdfcraft-sign.json")),
    ]);
    std::fs::write(r.dir.path().join("size.zip"), other).unwrap();
    assert!(r.import_refused("size.zip", "inbox").contains("hash"));
    assert!(!r.dir.path().join("inbox").exists());

    // Bytes that match their own manifest but are not a PDF are refused.
    let text = b"hello, this is not a PDF".to_vec();
    let m = manifest_with(&zip, |m| {
        m["pdf_bytes"] = json!(text.len());
        m["pdf_sha256"] = json!(sha(&text));
    });
    let not_pdf = zip_of(&[("manifest.json", &m), ("document.pdf", &text), ("plan.json", &r.file("form.pdf.pdfcraft-sign.json"))]);
    std::fs::write(r.dir.path().join("text.zip"), not_pdf).unwrap();
    assert!(r.import_refused("text.zip", "inbox").contains("not a PDF"));
    assert!(!r.dir.path().join("inbox").exists());
}

#[test]
fn a_manifest_of_another_version_or_with_a_plan_that_disagrees_is_refused() {
    let mut r = rig();
    r.export("out.zip");
    let zip = r.file("out.zip");
    let with = |edit: &dyn Fn(&mut Value)| {
        let m = manifest_with(&zip, edit);
        repack(&zip, |n, d| if n == "manifest.json" { m.clone() } else { d })
    };
    for (name, bytes, why) in [
        ("v2.zip", with(&|m| m["version"] = json!(2)), "version"),
        ("v0.zip", with(&|m| m["version"] = json!(0)), "version"),
        ("fmt.zip", with(&|m| m["format"] = json!("something-else")), "hand-off manifest"),
        ("extra.zip", with(&|m| m["secret"] = json!("x")), "manifest is not readable"),
        ("plan.zip", with(&|m| m["plan"]["recipients"][0]["status"] = json!("signed")), "plan"),
    ] {
        std::fs::write(r.dir.path().join(name), bytes).unwrap();
        let m = r.import_refused(name, "inbox");
        assert!(m.contains(why), "{name}: {m}");
    }
    assert!(!r.dir.path().join("inbox").exists());
    // A plan in the package that is not a valid plan is refused, not extracted.
    let junk = b"{\"version\": 1}".to_vec();
    let m = manifest_with(&zip, |m| m["plan_sha256"] = json!(sha(&junk)));
    let broken = repack(&zip, |n, d| match n {
        "plan.json" => junk.clone(),
        "manifest.json" => m.clone(),
        _ => d,
    });
    std::fs::write(r.dir.path().join("broken.zip"), broken).unwrap();
    let m = r.import_refused("broken.zip", "inbox");
    assert!(m.contains("not a signing plan") && m.contains("nothing was written") && !m.contains("left alone"), "{m}");
    assert!(!r.dir.path().join("inbox").exists());
}

#[test]
fn paths_that_climb_out_of_the_folder_are_refused_in_the_archive_and_in_the_manifest() {
    let mut r = rig();
    r.export("out.zip");
    let zip = r.file("out.zip");
    let pdf = r.file("form.pdf");
    let plan = r.file("form.pdf.pdfcraft-sign.json");
    let manifest = entries(&zip).into_iter().find(|(n, _)| n == "manifest.json").unwrap().1;
    // An entry named like a zip-slip path, or an absolute one, in place of the PDF in an archive of
    // exactly three entries: the name list is what refuses it, not the entry count or a missing name.
    for evil in ["../evil.txt", "..\\evil.txt", "/tmp/evil.txt", "C:\\evil.txt", "sub/document.pdf", "inbox/", "document.pdf/../../x"] {
        let bad = zip_of(&[("manifest.json", &manifest), (evil, &pdf), ("plan.json", &plan)]);
        std::fs::write(r.dir.path().join("evil.zip"), bad).unwrap();
        let before = snapshot(r.dir.path());
        let m = r.import_refused("evil.zip", "inbox");
        assert!(m.contains("is not part of a hand-off package"), "{evil}: {m}");
        assert!(snapshot(r.dir.path()) == before && !r.dir.path().parent().unwrap().join("evil.txt").exists(), "{evil}: {m}");
    }
    // The same entry name twice (three entries: the same-length name is changed in the bytes).
    let twice = zip_of(&[("manifest.json", &manifest), ("manifest.jsoX", &manifest), ("document.pdf", &pdf)]);
    let twice: Vec<u8> = {
        let mut bytes = twice;
        let (from, to) = (b"manifest.jsoX", b"manifest.json");
        let mut i = 0;
        while i + from.len() <= bytes.len() {
            if &bytes[i..i + from.len()] == from {
                bytes[i..i + from.len()].copy_from_slice(to);
            }
            i += 1;
        }
        bytes
    };
    std::fs::write(r.dir.path().join("twice.zip"), twice).unwrap();
    let m = r.import_refused("twice.zip", "inbox");
    assert!(m.contains("twice"), "{m}");
    assert!(!r.dir.path().join("inbox").exists());
    // The file name in a manifest is what the files are written under: it must be one plain name.
    for name in [
        "../evil.pdf",
        "sub/evil.pdf",
        "/abs/evil.pdf",
        "..\\evil.pdf",
        "C:evil.pdf",
        "CON.pdf",
        "nul.PDF",
        "com1.pdf",
        "con .pdf",
        "evil.txt",
        ".pdf",
        "..pdf",
        "a\u{202e}fdp.pdf",
        "nul\0.pdf",
        "",
    ] {
        let m = manifest_with(&zip, |m| m["file_name"] = json!(name));
        let bad = repack(&zip, |n, d| if n == "manifest.json" { m.clone() } else { d });
        std::fs::write(r.dir.path().join("evil.zip"), bad).unwrap();
        let msg = r.import_refused("evil.zip", "inbox");
        assert!(msg.contains("plain file name"), "{name:?}: {msg}");
        assert!(!r.dir.path().join("inbox").exists());
    }
    assert!(!r.dir.path().join("evil.pdf").exists());
}

#[test]
fn an_archive_that_is_too_big_or_has_too_many_entries_is_refused() {
    let mut r = rig();
    r.export("out.zip");
    let zip = r.file("out.zip");
    let pdf = r.file("form.pdf");
    let plan = r.file("form.pdf.pdfcraft-sign.json");
    let manifest = entries(&zip).into_iter().find(|(n, _)| n == "manifest.json").unwrap().1;
    // Zeros deflate to almost nothing: the archive is small, what it expands to is not.
    let bomb = vec![0u8; 4 << 20];
    let small = zip_of(&[("manifest.json", &bomb), ("document.pdf", &pdf), ("plan.json", &plan)]);
    let small_plan = zip_of(&[("manifest.json", &manifest), ("document.pdf", &pdf), ("plan.json", &bomb)]);
    assert!(small.len() < 20_000 && small_plan.len() < 40_000 + pdf.len(), "the test archives are small");
    let many = zip_of(&[("manifest.json", &manifest), ("document.pdf", &pdf), ("plan.json", &plan), ("a", b"1"), ("b", b"2")]);
    for (name, bytes) in [("bomb1.zip", small), ("bomb2.zip", small_plan), ("many.zip", many), ("junk.zip", b"not a zip at all".to_vec())] {
        std::fs::write(r.dir.path().join(name), bytes).unwrap();
        let before = snapshot(r.dir.path());
        let m = r.import_refused(name, "inbox");
        assert!(m.contains("hand-off package"), "{name}: {m}");
        let reason = match name {
            "many.zip" => "entries, more than 3",
            "junk.zip" => "not a ZIP archive",
            _ => "larger than",
        };
        assert!(m.contains(reason) && !m.contains("is not part of a hand-off package"), "{name}: {m}");
        assert!(snapshot(r.dir.path()) == before, "{name} wrote something");
    }
}

#[test]
fn an_existing_file_in_the_destination_is_never_overwritten_and_nothing_else_is_written() {
    let mut r = rig();
    r.export("out.zip");
    for existing in ["form.pdf", "form.pdf.pdfcraft-sign.json"] {
        let folder = r.dir.path().join(format!("inbox-{existing}"));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(existing), b"mine").unwrap();
        let before = snapshot(r.dir.path());
        let m = r.import_refused("out.zip", &format!("inbox-{existing}"));
        assert!(m.contains("already exists"), "{m}");
        assert!(snapshot(r.dir.path()) == before, "{existing}: the refusal changed or added a file");
    }
    // Importing twice into one folder: the second is refused, the first one's files stay.
    r.import("out.zip", "again");
    let before = snapshot(r.dir.path());
    assert!(r.import_refused("out.zip", "again").contains("already exists"));
    assert!(snapshot(r.dir.path()) == before);
    // A file where the folder should be.
    std::fs::write(r.dir.path().join("afile"), b"x").unwrap();
    assert!(r.import_refused("out.zip", "afile").contains("not a folder"));
}

#[test]
fn an_export_is_refused_without_a_saved_unchanged_document_a_plan_or_a_new_zip_name() {
    let mut r = rig();
    let doc = r.doc;
    let before = snapshot(r.dir.path());
    // Not a .zip, and a name that is already taken.
    assert!(refused(&mut r.a, "signing_handoff_export", json!({ "doc": doc, "path": "out.txt" })).contains(".zip"));
    assert!(refused(&mut r.a, "signing_handoff_export", json!({ "doc": doc, "path": ".zip" })).contains(".zip"));
    assert!(refused(&mut r.a, "signing_handoff_export", json!({ "doc": doc, "path": "sub/.zip" })).contains(".zip"));
    refused(&mut r.a, "signing_handoff_export", json!({ "doc": doc, "path": "ana.p12.zip/x.zip" }));
    std::fs::write(r.dir.path().join("taken.zip"), b"mine").unwrap();
    let with_taken = snapshot(r.dir.path());
    assert!(refused(&mut r.a, "signing_handoff_export", json!({ "doc": doc, "path": "taken.zip" })).contains("already exists"));
    assert!(snapshot(r.dir.path()) == with_taken);
    // Unsaved changes.
    tool(&mut r.a, "form_add_field", json!({ "doc": doc, "page": 1, "type": "text", "name": "extra", "rect": [20.0, 300.0, 100.0, 330.0] }));
    assert!(refused(&mut r.a, "signing_handoff_export", json!({ "doc": doc, "path": "out.zip" })).contains("not saved"));
    // A document with no file.
    let blank = tool(&mut r.a, "doc_create", json!({ "from": "blank", "pages": 1 }))["doc"].as_u64().unwrap();
    assert!(refused(&mut r.a, "signing_handoff_export", json!({ "doc": blank, "path": "out.zip" })).contains("no file"));
    // No plan beside the document.
    tool(&mut r.a, "doc_save", json!({ "doc": doc, "path": "form.pdf" }));
    tool(&mut r.a, "signing_plan_clear", json!({ "path": "form.pdf" }));
    assert!(refused(&mut r.a, "signing_handoff_export", json!({ "doc": doc, "path": "out.zip" })).contains("no signing plan"));
    assert!(!r.dir.path().join("out.zip").exists());
    let names_before: Vec<_> = before.iter().map(|(p, _)| p.clone()).collect();
    assert!(snapshot(r.dir.path()).iter().all(|(p, _)| names_before.contains(p) || p.ends_with("taken.zip") || p.ends_with("form.pdf")));
}

#[test]
fn a_stale_copy_of_the_document_is_not_exported() {
    let mut r = rig();
    // A second copy signs through the first id; the first id's open document is now stale.
    let other = tool(&mut r.a, "doc_open", json!({ "path": "form.pdf" }))["doc"].as_u64().unwrap();
    r.sign(other, "ana@example.org", "ana.p12");
    let before = snapshot(r.dir.path());
    let m = refused(&mut r.a, "signing_handoff_export", json!({ "doc": r.doc, "path": "out.zip" }));
    assert!(m.contains("changed on disk"), "{m}");
    assert!(snapshot(r.dir.path()) == before);
    // The copy that signed holds the file as it is, so it exports.
    tool(&mut r.a, "signing_handoff_export", json!({ "doc": other, "path": "out.zip" }));
    assert!(r.dir.path().join("out.zip").is_file());
}

#[test]
fn paths_outside_the_roots_are_refused_and_nothing_is_created() {
    let base = tempfile::tempdir().unwrap();
    let (read, write, outside) = (base.path().join("read"), base.path().join("write"), base.path().join("outside"));
    for d in [&read, &write, &outside] {
        std::fs::create_dir_all(d).unwrap();
    }
    // The document and its plan are made in the read root with an unrestricted session.
    let mut setup = Automation::new().with_root(&read).unwrap();
    let mut session = pdfcraft_engine::Session::new();
    let bytes = session.create_blank(300.0, 400.0, 1).unwrap();
    let id = session.open("a.pdf", None, bytes, None).unwrap();
    std::fs::write(read.join("form.pdf"), session.save_bytes(id).unwrap().as_slice()).unwrap();
    tool(&mut setup, "signing_plan_set", json!({ "path": "form.pdf", "recipients": [{ "name": "Ana", "email": "ana@example.org" }] }));

    let roots = AutomationRoots::new(Some(&read), Some(&write)).unwrap();
    let mut a = Automation::new().with_automation_roots(roots);
    let p = |d: &Path, n: &str| d.join(n).to_string_lossy().into_owned();
    let doc = tool(&mut a, "doc_open", json!({ "path": p(&read, "form.pdf") }))["doc"].as_u64().unwrap();
    let all_before = (snapshot(&read), snapshot(&write), snapshot(&outside));
    for out in [p(&outside, "x.zip"), p(&read, "x.zip"), p(&write, "../outside/x.zip"), p(&write, "../read/x.zip")] {
        refused(&mut a, "signing_handoff_export", json!({ "doc": doc, "path": out }));
    }
    assert!((snapshot(&read), snapshot(&write), snapshot(&outside)) == all_before, "a refused export wrote a file");

    // A real package, then moved around: readable only from the read root, writable only to the write root.
    tool(&mut a, "signing_handoff_export", json!({ "doc": doc, "path": p(&write, "ok.zip") }));
    let zip = std::fs::read(write.join("ok.zip")).unwrap();
    std::fs::write(outside.join("p.zip"), &zip).unwrap();
    std::fs::write(read.join("p.zip"), &zip).unwrap();
    let all_before = (snapshot(&read), snapshot(&write), snapshot(&outside));
    for (zip, folder) in [
        (p(&write, "ok.zip"), p(&write, "in")),
        (p(&outside, "p.zip"), p(&write, "in")),
        (p(&read, "p.zip"), p(&read, "in")),
        (p(&read, "p.zip"), p(&outside, "in")),
        (p(&read, "p.zip"), p(&write, "../outside/in")),
    ] {
        refused(&mut a, "signing_handoff_import", json!({ "path": zip, "folder": folder }));
    }
    assert!((snapshot(&read), snapshot(&write), snapshot(&outside)) == all_before, "a refused import wrote a file");
    let done = tool(&mut a, "signing_handoff_import", json!({ "path": p(&read, "p.zip"), "folder": p(&write, "in") }));
    assert_eq!(done["next"]["email"], "ana@example.org");
    assert!(write.join("in/form.pdf").is_file() && write.join("in/form.pdf.pdfcraft-sign.json").is_file());
}

#[test]
fn a_plan_changed_inside_the_package_is_refused_even_when_it_is_still_a_valid_plan() {
    let mut r = rig();
    r.export("out.zip");
    let zip = r.file("out.zip");
    type Edit = fn(&mut Value);
    let edits: [(&str, Edit); 3] = [
        ("message", |p| p["message"] = json!("Please sign somewhere else")),
        ("history", |p| p["recipients"][0]["history"] = json!([{ "status": "pending", "at": 5 }])),
        ("name", |p| p["recipients"][1]["name"] = json!("Someone Else")),
    ];
    for (what, edit) in edits {
        let tampered = repack(&zip, |n, d| {
            if n != "plan.json" {
                return d;
            }
            let mut plan: Value = serde_json::from_slice(&d).unwrap();
            edit(&mut plan);
            serde_json::to_vec_pretty(&plan).unwrap()
        });
        std::fs::write(r.dir.path().join("plan-edit.zip"), tampered).unwrap();
        let before = snapshot(r.dir.path());
        let m = r.import_refused("plan-edit.zip", "inbox");
        assert!(m.contains("plan in the package does not match the hash"), "{what}: {m}");
        assert!(snapshot(r.dir.path()) == before, "{what}: a refused import wrote something");
    }
}
