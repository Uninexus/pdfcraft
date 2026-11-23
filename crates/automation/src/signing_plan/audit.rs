//! `signing_plan_audit`: one report that sets what the local signing plan says beside what the
//! saved file shows. The plan is a record somebody wrote down (`signing_plan_advance` writes what
//! the caller says happened); the file is the evidence, as the validator reads it
//! (`signature_audit_report`). For each recipient the two are compared through the signer label
//! (`/PdfCraftSigner`) on the signature fields, and every signature the plan does not account for
//! is listed.
//!
//! The report says what it could not determine, and never reads that as "none": a field whose
//! label could not be read, a name shared by two fields, a list cut at the audit's limit and
//! fields a too-large form kept unread all make an absence "not determined". It does not say
//! who signed (the label is text anyone could write, and a certificate is not compared with an
//! address) and it says nothing about legal effect.

use std::collections::BTreeSet;

use pdfcraft_engine::FormFieldKind;
use serde_json::{Value, json};

use super::{NOTE, Role, Status, bad, read_plan_sidecar, sidecar_of};
use crate::file_access::Access;
use crate::{Args, Automation, Result, failed, read_input};

/// What the report says about itself, in every result.
const AUDIT_NOTE: &str = "The plan is a local record that a person wrote down; the PDF is the evidence. This report compares the two and proves \
     nothing by itself: it does not verify who a signer is (a label on a field is text anyone can write, and a signature's certificate is not compared with \
     the recipient's address), it does not say that any signature or record has legal effect, nothing was sent and nobody was told. A field's signer label is read from the file as it is now, not as it was when the signature was applied. Where something could not \
     be determined the report says not_determined and does not read it as none.";

/// What the file says about who a signature field was meant for.
enum Label {
    /// The field has a label (lower case).
    For(String),
    /// The field was read and has no label.
    None,
    /// The field or its label could not be read with certainty.
    Unknown(&'static str),
}

impl Automation {
    /// `signing_plan_audit`: the plan and the file's signatures in one report; nothing is written.
    pub(crate) fn signing_plan_audit(&mut self, a: &Args) -> Result<Value> {
        let (path, dirty, changed_on_open) = {
            let doc = self.doc(a)?;
            (doc.path.clone(), doc.dirty, doc.changed_on_open)
        };
        let path = path.ok_or_else(|| bad("the document has no file; the plan is kept beside the document's file, so save it first"))?;
        if dirty {
            return Err(failed("the document has changes that are not saved; save it first (doc_save), because the report is about the file"));
        }
        if changed_on_open {
            return Err(failed(
                "opening this file changed it (a repair or a form layout), so it cannot be compared with the file on disk; save it with doc_save, reopen it and try again",
            ));
        }
        let pdf = self.resolve_for(&path, Access::Read, false)?;
        let sidecar = sidecar_of(&pdf)?;
        let shown = sidecar.to_string_lossy();
        self.check_roots(&shown, &sidecar, Access::Read)?;
        let plan =
            read_plan_sidecar(&sidecar)?.ok_or_else(|| failed("there is no signing plan beside this document; make one with signing_plan_set"))?;
        if *read_input(&pdf)? != **self.doc(a)?.saved_bytes() {
            return Err(failed("the file changed on disk since it was opened; reopen it"));
        }

        // The audit is asked for with the document only, so an `output` can never write a file here.
        let only_doc = json!({ "doc": a.0.get("doc").cloned().unwrap_or(Value::Null) });
        let file = self.signature_audit_report(&Args(&only_doc))?;
        let entries: Vec<Value> = file["signatures"].as_array().cloned().unwrap_or_default();
        let kind_of = |e: &Value| e["kind"].as_str().unwrap_or("").to_string();
        // Whether every signature field was listed and read: when not, an absence is not shown.
        let complete =
            file["truncated"] != json!(true) && file["stand_in_counted"] != json!(true) && !entries.iter().any(|e| kind_of(e) == "unread_fields");
        let labels: Vec<(Value, Label)> = {
            let doc = self.doc(a)?;
            entries
                .iter()
                .enumerate()
                .filter(|(_, e)| kind_of(e) == "signature")
                .map(|(i, e)| {
                    // The listed name is cleaned (hidden characters dropped, spaces trimmed, cut), so the field is
                    // looked up by the raw name of the same signature (the audit lists `doc.signatures` in order),
                    // and a raw name that differs from the listed one is not matched to any field.
                    let raw = doc.signatures.get(i).map(|s| s.field.as_str());
                    let listed = e["field"].as_str().unwrap_or("");
                    // The validator reads a field's name with its ends trimmed, so every signature field whose
                    // trimmed name is the signature's is a candidate, and only an exact one-to-one match is used.
                    let trimmed = |n: &str| n.trim_matches('\0').trim().to_string();
                    let mut named = doc.form.iter().filter(|f| f.kind == FormFieldKind::Signature && Some(trimmed(&f.name).as_str()) == raw);
                    let label = match (named.next(), named.next()) {
                        _ if raw != Some(listed) => Label::Unknown("the field's name has hidden characters, spaces at its ends or is cut, so it cannot be matched to a field by the name listed"),
                        (None, _) => Label::Unknown("no signature field of that name was found in the form"),
                        (Some(_), Some(_)) => Label::Unknown("two fields have that name"),
                        (Some(f), None) => {
                            let s = doc.field_signer(f.obj);
                            match s {
                                _ if s.capped => Label::Unknown("the label could not be read within the read allowance"),
                                _ if s.not_text => Label::Unknown("the field's signer key is not text"),
                                _ if !s.exact => Label::Unknown("the label is longer than is read"),
                                _ => s.email.map_or(Label::None, |e| Label::For(e.to_lowercase())),
                            }
                        }
                    };
                    (e.clone(), label)
                })
                .collect()
        };
        let undetermined = labels.iter().filter(|(_, l)| matches!(l, Label::Unknown(_))).count();
        let status_of = |e: &Value| e["status"].as_str().unwrap_or("").to_string();
        let brief = |e: &Value| json!({ "field": e["field"], "status": e["status"], "signer": e["signer"], "reasons": e["reasons"] });

        let mut problems: Vec<String> = Vec::new();
        let mut open_questions: Vec<String> = Vec::new();
        let mut recipients = Vec::new();
        let known: BTreeSet<String> = plan.recipients.iter().map(|r| r.email.to_lowercase()).collect();
        for r in &plan.recipients {
            let key = r.email.to_lowercase();
            let mine: Vec<&(Value, Label)> = labels.iter().filter(|(_, l)| matches!(l, Label::For(e) if *e == key)).collect();
            let statuses: Vec<String> = mine.iter().map(|(e, _)| status_of(e)).collect();
            let absent_known = complete && undetermined == 0;
            if mine.len() > 1 {
                open_questions.push(format!(
                    "{} ({}): {} signatures are labelled for this recipient; the cross-check says what the best of them shows, so look at each",
                    r.name,
                    r.email,
                    mine.len()
                ));
            }
            let (check, why) = match (r.role, r.status) {
                (Role::Signer, Status::Signed) if !mine.is_empty() => {
                    if statuses.iter().any(|s| s == "invalid") {
                        (
                            "signature_invalid",
                            "the plan says signed and a signature labelled for this recipient is in the file, but the validator reads it as invalid",
                        )
                    } else if statuses.iter().any(|s| s == "valid") {
                        ("agrees", "the plan says signed and a signature labelled for this recipient is in the file and valid")
                    } else {
                        (
                            "signature_present_not_shown_valid",
                            "the plan says signed and a signature labelled for this recipient is in the file, but the validator did not show it valid (for example its signer is not trusted, or something could not be checked)",
                        )
                    }
                }
                (Role::Signer, Status::Signed) if absent_known => {
                    ("plan_says_signed_but_no_signature", "the plan says signed but no signature labelled for this recipient is in the file")
                }
                (Role::Signer, Status::Signed) => (
                    "not_determined",
                    "the plan says signed; no signature labelled for this recipient was found, but not every signature field could be read, so that is not shown",
                ),
                (Role::Signer, _) if !mine.is_empty() => (
                    "signature_present_but_plan_not_signed",
                    "a signature labelled for this recipient is in the file, but the plan does not say signed",
                ),
                (Role::Signer, _) if absent_known => {
                    ("agrees", "the plan does not say signed and no signature labelled for this recipient is in the file")
                }
                (Role::Signer, _) => (
                    "not_determined",
                    "the plan does not say signed; no signature labelled for this recipient was found, but not every signature field could be read, so that is not shown",
                ),
                (_, _) if !mine.is_empty() => (
                    "signature_labelled_for_a_recipient_who_does_not_sign",
                    "a signature is labelled for this approver or viewer, who has no signing turn in the plan",
                ),
                _ => ("not_applicable", "this recipient is an approver or a viewer, and no signature is expected from them"),
            };
            match check {
                "agrees" | "not_applicable" => {}
                "not_determined" => open_questions.push(format!("{} ({}): {why}", r.name, r.email)),
                "signature_present_not_shown_valid" => open_questions.push(format!("{} ({}): {why}", r.name, r.email)),
                _ => problems.push(format!("{} ({}): {why}", r.name, r.email)),
            }
            recipients.push(json!({
                "recipient": r, "plan_says": r.status, "cross_check": check, "why": why,
                "labelled_signature_count": mine.len(), "signatures_labelled_for_recipient": mine.iter().map(|(e, _)| brief(e)).collect::<Vec<_>>(),
            }));
        }

        let mut unaccounted = Vec::new();
        let mut not_determined = Vec::new();
        for (e, l) in &labels {
            match l {
                Label::None => unaccounted
                    .push(json!({ "signature": brief(e), "why": "the signature field has no signer label, so the plan does not account for it" })),
                Label::For(k) if !known.contains(k) => {
                    unaccounted.push(
                        json!({ "signature": brief(e), "why": "the signature field is labelled for an address that is not a recipient of the plan" }),
                    );
                }
                Label::For(_) => {}
                Label::Unknown(w) => not_determined.push(json!({ "signature": brief(e), "why": w })),
            }
        }
        for u in &unaccounted {
            problems.push(format!("signature {} is not accounted for by the plan: {}", u["signature"]["field"], u["why"].as_str().unwrap_or("")));
        }
        for n in &not_determined {
            open_questions.push(format!(
                "signature {} could not be matched to a recipient: {}",
                n["signature"]["field"],
                n["why"].as_str().unwrap_or("")
            ));
        }
        // Any invalid entry, a document timestamp included.
        for e in entries.iter().filter(|e| status_of(e) == "invalid") {
            {
                problems.push(format!(
                    "signature {} is invalid as the validator reads it (altered, not verifying, or followed by changes it does not permit)",
                    e["field"]
                ));
            }
        }
        let mut seen = BTreeSet::new();
        problems.retain(|p| seen.insert(p.clone()));
        if !complete {
            open_questions.push("the signature list is cut, or fields were kept unread, so not every signature was looked at".to_string());
        }
        let consistent = if !problems.is_empty() {
            json!(false)
        } else if open_questions.is_empty() {
            json!(true)
        } else {
            Value::Null
        };
        Ok(json!({
            "ok": true,
            "plan": { "state": plan.state(), "message": plan.message, "created": plan.created, "recipients": plan.recipients, "sidecar": sidecar.to_string_lossy() },
            "file": file,
            "recipients": recipients,
            "unaccounted_signatures": unaccounted,
            "not_determined_signatures": not_determined,
            "consistent": consistent,
            "consistent_meaning": "true: the plan and the file agree and nothing was left undetermined; false: see problems; null: no problem was found but something could not be determined (see open_questions)",
            "problems": problems, "open_questions": open_questions,
            "all_signature_fields_read": complete,
            "identity_checked": false, "legal_effect_assessed": false, "local_record_only": true, "sent": false, "notified": false,
            "note": format!("{AUDIT_NOTE} {NOTE}"),
        }))
    }
}
