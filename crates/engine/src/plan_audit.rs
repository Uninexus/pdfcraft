//! The cross-check of a local signing plan against the signatures a saved file holds: what `signing_plan_audit` tells and
//! what the app's Check the plan button shows. The plan is a record somebody wrote down; the file is the evidence, as the
//! validator reads it ([`crate::signature_audit`]). For each recipient the two are compared through the signer label
//! (`/PdfCraftSigner`) on the signature fields, and every signature the plan does not account for is listed.
//!
//! The result says what could not be determined, and never reads that as "none": a field whose label could not be read, a
//! name shared by two fields, a list cut at the audit's limit and fields a too-large form kept unread all make an absence
//! "not determined". It does not say who signed (the label is text anyone could write, and a certificate is not compared
//! with an address) and it says nothing about legal effect. Reads only; nothing is written or sent.

use std::collections::BTreeSet;

use pdfcraft_organize::signing_plan::{Plan, Recipient, Role, Status};

use crate::signature_audit::{AuditEntry, AuditReport};
use crate::{Document, FormFieldKind};

/// What the result says about itself, in every result.
pub const AUDIT_NOTE: &str = "The plan is a local record that a person wrote down; the PDF is the evidence. This report compares the two and proves \
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

/// A signature as the cross-check names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureRef {
    pub field: String,
    pub status: &'static str,
    pub signer: Option<String>,
    pub reasons: Vec<String>,
}

impl SignatureRef {
    fn of(e: &AuditEntry) -> SignatureRef {
        SignatureRef { field: e.field.clone(), status: e.status, signer: e.signer.clone(), reasons: e.reasons.clone() }
    }
}

/// One recipient set against the file.
#[derive(Clone, Debug)]
pub struct RecipientCheck {
    pub recipient: Recipient,
    /// `agrees`, `not_determined`, `not_applicable` or a named disagreement.
    pub cross_check: &'static str,
    pub why: &'static str,
    /// The signatures labelled for this recipient.
    pub signatures: Vec<SignatureRef>,
}

/// A signature the plan does not account for, or could not be matched to a recipient, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unmatched {
    pub signature: SignatureRef,
    pub why: &'static str,
}

#[derive(Clone, Debug)]
pub struct PlanAudit {
    pub recipients: Vec<RecipientCheck>,
    pub unaccounted: Vec<Unmatched>,
    pub not_determined: Vec<Unmatched>,
    /// `Some(true)`: the plan and the file agree and nothing was left undetermined; `Some(false)`: see `problems`; `None`: no
    /// problem was found but something could not be determined (see `open_questions`).
    pub consistent: Option<bool>,
    pub problems: Vec<String>,
    pub open_questions: Vec<String>,
    /// Every signature field was listed and read.
    pub complete: bool,
}

/// `plan` set against the signatures of `doc` (`report` is [`crate::signature_audit::audit`] of the same document).
pub fn cross_check(doc: &Document, report: &AuditReport, plan: &Plan) -> PlanAudit {
    let entries = &report.entries;
    // Whether every signature field was listed and read: when not, an absence is not shown.
    let complete = !report.truncated && !report.stand_in_counted && !entries.iter().any(|e| e.kind == "unread_fields");
    let labels: Vec<(&AuditEntry, Label)> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.kind == "signature")
        .map(|(i, e)| {
            // The listed name is cleaned (hidden characters dropped, spaces trimmed, cut), so the field is looked up by the
            // raw name of the same signature (the audit lists `doc.signatures` in order), and a raw name that differs from
            // the listed one is not matched to any field.
            let raw = doc.signatures.get(i).map(|s| s.field.as_str());
            let listed = e.field.as_str();
            // The validator reads a field's name with its ends trimmed, so every signature field whose trimmed name is the
            // signature's is a candidate, and only an exact one-to-one match is used.
            let trimmed = |n: &str| n.trim_matches('\0').trim().to_string();
            let mut named = doc.form.iter().filter(|f| f.kind == FormFieldKind::Signature && Some(trimmed(&f.name).as_str()) == raw);
            let label = match (named.next(), named.next()) {
                _ if raw != Some(listed) => Label::Unknown(
                    "the field's name has hidden characters, spaces at its ends or is cut, so it cannot be matched to a field by the name listed",
                ),
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
            (e, label)
        })
        .collect();
    let undetermined = labels.iter().filter(|(_, l)| matches!(l, Label::Unknown(_))).count();

    let mut problems: Vec<String> = Vec::new();
    let mut open_questions: Vec<String> = Vec::new();
    let mut recipients = Vec::new();
    let known: BTreeSet<String> = plan.recipients.iter().map(|r| r.email.to_lowercase()).collect();
    for r in &plan.recipients {
        let key = r.email.to_lowercase();
        let mine: Vec<&(&AuditEntry, Label)> = labels.iter().filter(|(_, l)| matches!(l, Label::For(e) if *e == key)).collect();
        let statuses: Vec<&str> = mine.iter().map(|(e, _)| e.status).collect();
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
                if statuses.contains(&"invalid") {
                    (
                        "signature_invalid",
                        "the plan says signed and a signature labelled for this recipient is in the file, but the validator reads it as invalid",
                    )
                } else if statuses.contains(&"valid") {
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
            (Role::Signer, _) if !mine.is_empty() => {
                ("signature_present_but_plan_not_signed", "a signature labelled for this recipient is in the file, but the plan does not say signed")
            }
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
        recipients.push(RecipientCheck {
            recipient: r.clone(),
            cross_check: check,
            why,
            signatures: mine.iter().map(|(e, _)| SignatureRef::of(e)).collect(),
        });
    }

    let mut unaccounted = Vec::new();
    let mut not_determined = Vec::new();
    for (e, l) in &labels {
        match l {
            Label::None => unaccounted.push(Unmatched {
                signature: SignatureRef::of(e),
                why: "the signature field has no signer label, so the plan does not account for it",
            }),
            Label::For(k) if !known.contains(k) => unaccounted.push(Unmatched {
                signature: SignatureRef::of(e),
                why: "the signature field is labelled for an address that is not a recipient of the plan",
            }),
            Label::For(_) => {}
            Label::Unknown(w) => not_determined.push(Unmatched { signature: SignatureRef::of(e), why: w }),
        }
    }
    // The field name as JSON writes it, so the sentence reads the same wherever it is shown.
    let quoted = |s: &str| serde_json::Value::String(s.to_string()).to_string();
    for u in &unaccounted {
        problems.push(format!("signature {} is not accounted for by the plan: {}", quoted(&u.signature.field), u.why));
    }
    for n in &not_determined {
        open_questions.push(format!("signature {} could not be matched to a recipient: {}", quoted(&n.signature.field), n.why));
    }
    // Any invalid entry, a document timestamp included.
    for e in entries.iter().filter(|e| e.status == "invalid") {
        problems.push(format!(
            "signature {} is invalid as the validator reads it (altered, not verifying, or followed by changes it does not permit)",
            quoted(&e.field)
        ));
    }
    let mut seen = BTreeSet::new();
    problems.retain(|p| seen.insert(p.clone()));
    if !complete {
        open_questions.push("the signature list is cut, or fields were kept unread, so not every signature was looked at".to_string());
    }
    let consistent = if !problems.is_empty() {
        Some(false)
    } else if open_questions.is_empty() {
        Some(true)
    } else {
        None
    };
    PlanAudit { recipients, unaccounted, not_determined, consistent, problems, open_questions, complete }
}

impl PlanAudit {
    /// The result in plain English, for a screen or a copy. `plan` is the plan it was made from.
    pub fn text(&self, plan: &Plan) -> String {
        let mut t = String::new();
        let mut line = |s: String| {
            t.push_str(&s);
            t.push('\n');
        };
        line("Signing plan check".into());
        line(String::new());
        line(match self.consistent {
            Some(true) => "The plan and the file agree, and nothing was left undetermined.".into(),
            Some(false) => "The plan and the file do not agree. See the problems below.".into(),
            None => "No problem was found, but something could not be determined. See the open questions below.".into(),
        });
        line(format!("Plan state: {}", plan.state()));
        line(String::new());
        line("Recipients".into());
        for r in &self.recipients {
            let mark = &r.recipient;
            line(format!(
                "{}. {} ({}), {:?}, plan says {:?}: {} ({} labelled signature{})",
                mark.order,
                mark.name,
                mark.email,
                mark.role,
                mark.status,
                r.cross_check,
                r.signatures.len(),
                if r.signatures.len() == 1 { "" } else { "s" }
            ));
            line(format!("    {}", r.why));
        }
        let mut list = |title: &str, rows: &[String]| {
            if !rows.is_empty() {
                line(String::new());
                line(title.to_string());
                for row in rows {
                    line(format!("- {row}"));
                }
            }
        };
        list("Problems", &self.problems);
        list("Open questions", &self.open_questions);
        let named = |u: &Unmatched| format!("{}: {}", u.signature.field, u.why);
        list("Signatures the plan does not account for", &self.unaccounted.iter().map(named).collect::<Vec<_>>());
        line(String::new());
        line(AUDIT_NOTE.into());
        t
    }
}
