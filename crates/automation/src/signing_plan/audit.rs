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

use pdfcraft_engine::plan_audit::{AUDIT_NOTE, SignatureRef, Unmatched, cross_check};
use serde_json::{Value, json};

use super::{NOTE, bad, read_plan_sidecar, sidecar_of};
use crate::file_access::Access;
use crate::{Args, Automation, Result, failed, read_input};

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
        let id = self.doc(a)?.id;
        let report = self.session.signature_audit(id).map_err(failed)?;
        let audit = cross_check(self.doc(a)?, &report, &plan);
        let sig = |s: &SignatureRef| json!({ "field": s.field, "status": s.status, "signer": s.signer, "reasons": s.reasons });
        let recipients: Vec<Value> = audit
            .recipients
            .iter()
            .map(|c| {
                json!({
                    "recipient": c.recipient, "plan_says": c.recipient.status, "cross_check": c.cross_check, "why": c.why,
                    "labelled_signature_count": c.signatures.len(), "signatures_labelled_for_recipient": c.signatures.iter().map(sig).collect::<Vec<_>>(),
                })
            })
            .collect();
        let unmatched = |list: &[Unmatched]| list.iter().map(|u| json!({ "signature": sig(&u.signature), "why": u.why })).collect::<Vec<_>>();
        let (unaccounted, not_determined) = (unmatched(&audit.unaccounted), unmatched(&audit.not_determined));
        let consistent = audit.consistent.map_or(Value::Null, |c| json!(c));
        let (problems, open_questions, complete) = (audit.problems, audit.open_questions, audit.complete);
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
