//! Signing plan tools: a local record of who is asked to sign a document, in what order, with a
//! message and each person's status. The record is a JSON file beside the PDF
//! (`<file name>.pdfcraft-sign.json`, so `a.pdf` has `a.pdf.pdfcraft-sign.json`), so the PDF's bytes are never touched.
//!
//! It is a record only. Nothing is sent, nobody is notified and no signature is collected or
//! checked: `signing_plan_advance` writes down what the caller says happened. The order is a strict
//! list (1, 2, 3 ...), one recipient at a time; groups that may sign in parallel are not offered.
//! The data is plain serde so a later sending route can read the same file. The plan lives in the
//! sidecar; keeping it inside the PDF as an attachment is not built. A time (`at`, `created`) is 0
//! when the host has no clock.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::file_access::Access;
use crate::signing::Signing;
use crate::{Args, Automation, Result, ToolError, failed, write_atomic};
use pdfcraft_engine::FormFieldKind;

mod audit;
mod handoff;

fn bad(m: impl Into<String>) -> ToolError {
    ToolError::InvalidArgs(m.into())
}

pub(crate) use pdfcraft_organize::signing_plan::{MAX_RECIPIENTS, MAX_SIDECAR_BYTES};

/// What every result says, so no one reads a plan as a sent request.
pub(crate) const NOTE: &str = "This is a local record kept in a file beside the PDF. It does not send the document, notify anyone or collect \
     signatures, and marking a recipient signed, approved or declined only writes down what you say happened: it does not look for a signature in the PDF.";

/// What `signing_plan_sign` says besides `NOTE`: it signs for whoever holds the ID it is given.
pub(crate) const SIGN_NOTE: &str = "The plan does not check who holds the digital ID: the signature is made with the ID you gave, whoever it belongs to, \
     and its certificate is not compared with the recipient's address (identity_checked is false). Nothing was sent and nobody was told; the plan is a local record.";

pub(crate) use pdfcraft_organize::signing_plan::{Event, Plan, Recipient, Role, Status, plan_bytes, plan_from_bytes};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipientInput {
    name: String,
    email: String,
    role: Option<Role>,
    order: Option<u32>,
}

pub(crate) use pdfcraft_organize::field_signer::is_hidden;

fn check_name(name: &str) -> Result<String> {
    pdfcraft_organize::signing_plan::check_name(name).map_err(|e| bad(e.0))
}

/// A shallow check only (see `pdfcraft_organize::field_signer::check_address`, the one rule the
/// app's Signer entry uses too).
pub(crate) fn check_email(email: &str) -> Result<String> {
    pdfcraft_organize::field_signer::check_address(email).map_err(|e| bad(e.why()))
}

fn check_message(message: &str) -> Result<String> {
    pdfcraft_organize::signing_plan::check_message(message).map_err(|e| bad(e.0))
}

/// The JSON every plan tool answers with.
fn plan_json(plan: &Plan, sidecar: &Path) -> Value {
    let next = plan.next().filter(|_| plan.state() == "in_progress").map(|r| json!({ "name": r.name, "email": r.email, "order": r.order }));
    json!({
        "plan": plan, "state": plan.state(), "next": next, "sidecar": sidecar.to_string_lossy(),
        "local_record_only": true, "note": NOTE,
    })
}

/// The signature field `signing_plan_sign` is to sign: the one `named` when the caller names one
/// (it must be a signature field that is unsigned and not labelled for someone else), else the
/// only unsigned signature field labelled (`/PdfCraftSigner`, compared without regard to case)
/// for `email`. The error says what is wrong and what to do.
fn signature_fields(doc: &pdfcraft_engine::Document, email: &str, named: Option<&str>) -> Result<String> {
    let signed_names: Vec<&str> = doc.signatures.iter().filter(|s| s.signed).map(|s| s.field.as_str()).collect();
    let unsigned = |f: &pdfcraft_engine::FormField| {
        f.kind == FormFieldKind::Signature && !doc.field_signer(f.obj).signed && !signed_names.contains(&f.name.as_str())
    };
    let label = |f: &pdfcraft_engine::FormField| doc.field_signer(f.obj).email.map(|e| e.to_lowercase());
    let shared = |name: &str| doc.form.iter().filter(|f| f.name == name).count() > 1;
    if let Some(name) = named {
        let f = doc.form.iter().find(|f| f.name == name).ok_or_else(|| failed(format!("there is no field named {name:?} (see form_fields)")))?;
        if shared(name) {
            return Err(failed(format!("{name:?} names more than one field, so it cannot be signed by name")));
        }
        if f.kind != FormFieldKind::Signature {
            return Err(failed(format!("{name:?} is not a signature field")));
        }
        if !unsigned(f) {
            return Err(failed(format!(
                "{name:?} already holds a signature; if this recipient is next in the plan and an earlier signing_plan_sign was interrupted after it saved the file, record the step with signing_plan_advance (status signed)"
            )));
        }
        return match label(f) {
            Some(l) if l != email => Err(failed(format!("{name:?} is labelled for another recipient ({l}), not for this one"))),
            _ => Ok(name.to_string()),
        };
    }
    let mine: Vec<&pdfcraft_engine::FormField> =
        doc.form.iter().filter(|f| f.kind == FormFieldKind::Signature && label(f).as_deref() == Some(email)).collect();
    let open: Vec<&&pdfcraft_engine::FormField> = mine.iter().filter(|f| unsigned(f)).collect();
    match open.as_slice() {
        [one] if shared(&one.name) => Err(failed(format!("{:?} names more than one field, so it cannot be signed by name", one.name))),
        [one] => Ok(one.name.clone()),
        [] if mine.is_empty() => {
            Err(failed("no signature field is labelled for this recipient; label one with form_field_signer, or name the field to sign with `field`"))
        }
        [] => Err(failed(
            "every signature field labelled for this recipient already holds a signature; if this recipient is next in the plan and an earlier signing_plan_sign was interrupted after it saved the file, record the step with signing_plan_advance (status signed)",
        )),
        many => {
            let names: Vec<String> = many.iter().take(10).map(|f| format!("{:?}", f.name)).collect();
            Err(failed(format!(
                "{} unsigned signature fields are labelled for this recipient ({}); name the one to sign with `field`",
                many.len(),
                names.join(", ")
            )))
        }
    }
}

fn sidecar_of(pdf: &Path) -> Result<PathBuf> {
    pdfcraft_organize::signing_plan::sidecar_of(pdf).map_err(|e| bad(e.0))
}

/// The plan in `resolved_sidecar` (see `pdfcraft_organize::signing_plan::read_sidecar`, which the
/// app reads with too): `None` when there is no file, an error when it is not a valid plan.
fn read_plan_sidecar(resolved_sidecar: &Path) -> Result<Option<Plan>> {
    Ok(pdfcraft_organize::signing_plan::read_sidecar(resolved_sidecar)?)
}

/// Write the plan to its sidecar, atomically.
fn save(sidecar: &Path, plan: &Plan) -> Result<()> {
    write_atomic(sidecar, &plan_bytes(plan)?)
}

/// The recipients of `signing_plan_set`, checked whole, in plan order with orders 1..n.
fn recipients_arg(a: &Args, now: i64) -> Result<Vec<Recipient>> {
    let list = a.get("recipients").ok_or_else(|| Args::missing("recipients"))?;
    let items = list.as_array().ok_or_else(|| Args::wrong("recipients", "an array of {name, email, role, order}"))?;
    if items.is_empty() || items.len() > MAX_RECIPIENTS {
        return Err(bad(format!("recipients must hold 1 to {MAX_RECIPIENTS} entries")));
    }
    let inputs: Vec<RecipientInput> = serde_json::from_value(list.clone()).map_err(|e| bad(format!("recipients: {e}")))?;
    let given = inputs.iter().filter(|r| r.order.is_some()).count();
    if given != 0 && given != inputs.len() {
        return Err(bad("give an order number to every recipient or to none (none means the list order)"));
    }
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for (i, r) in inputs.iter().enumerate() {
        let (name, email) = (check_name(&r.name)?, check_email(&r.email)?);
        let key = email.to_lowercase();
        if seen.contains(&key) {
            return Err(bad(format!("recipients[{i}]: the address {email} is already in the list (compared without regard to case)")));
        }
        seen.push(key);
        let order = r.order.unwrap_or(i as u32 + 1);
        out.push(Recipient {
            name,
            email,
            role: r.role.unwrap_or(Role::Signer),
            order,
            status: Status::Pending,
            history: vec![Event { status: Status::Pending, at: now }],
        });
    }
    out.sort_by_key(|r| r.order);
    if out.iter().enumerate().any(|(i, r)| r.order as usize != i + 1) {
        return Err(bad(
            "order numbers must be 1, 2, 3 ... with none missing or repeated (one recipient at a time; parallel groups are not supported)",
        ));
    }
    if !out.iter().any(|r| r.role.acts()) {
        return Err(bad("at least one recipient must be a signer or an approver"));
    }
    Ok(out)
}

/// One recipient of the plan beside an open document, as the form tools see it.
pub(crate) struct PlanPerson {
    pub(crate) name: String,
    pub(crate) email: String,
    pub(crate) role: &'static str,
    /// A signer or an approver: someone a field can be meant for.
    pub(crate) acts: bool,
}

/// What the form tools found when they looked for the signing plan beside an open document.
pub(crate) enum PlanLookup {
    /// No look was possible, for this reason: no plan was checked.
    Unchecked(&'static str),
    /// There is no plan file beside the document.
    Absent,
    /// The plan's recipients, in plan order.
    Found(Vec<PlanPerson>),
}

impl Automation {
    /// The plan beside the document the form tools work on: its sidecar is derived from the
    /// document's own path, which goes through `resolve_for` like every path (the file itself may
    /// be gone) and is held to the read root. Nothing is written.
    pub(crate) fn plan_beside(&self, path: Option<&str>) -> Result<PlanLookup> {
        let Some(raw) = path else { return Ok(PlanLookup::Unchecked("the document has no file path, so no plan beside it could be looked for")) };
        let pdf = self.resolve_for(raw, Access::Read, true)?;
        let Ok(sidecar) = sidecar_of(&pdf) else {
            return Ok(PlanLookup::Unchecked("the document's file name does not end in .pdf, so no plan beside it could be looked for"));
        };
        let shown = sidecar.to_string_lossy();
        self.check_roots(&shown, &sidecar, Access::Read)?;
        Ok(match read_plan_sidecar(&sidecar)? {
            None => PlanLookup::Absent,
            Some(plan) => PlanLookup::Found(
                plan.recipients
                    .iter()
                    .map(|r| PlanPerson {
                        name: r.name.clone(),
                        email: r.email.clone(),
                        role: match r.role {
                            Role::Signer => "signer",
                            Role::Approver => "approver",
                            Role::Viewer => "viewer",
                        },
                        acts: r.role.acts(),
                    })
                    .collect(),
            ),
        })
    }

    /// The PDF the plan is about (it must exist, resolved through `resolve`) and its sidecar, which
    /// is derived from that resolved path and held to the read root and, for `write`, the write
    /// root as well (it does not itself pass through `resolve`).
    fn resolve_plan_paths(&self, a: &Args, write: bool) -> Result<(PathBuf, PathBuf)> {
        let raw = a.str("path")?;
        let pdf = self.resolve(raw, false)?;
        if !pdf.is_file() {
            return Err(bad("path must be an existing file"));
        }
        let sidecar = sidecar_of(&pdf)?;
        // The refusal names the sidecar, which is the file the roots are asked about.
        let shown = sidecar.to_string_lossy();
        self.check_roots(&shown, &sidecar, Access::Read)?;
        if write {
            self.check_roots(&shown, &sidecar, Access::Write)?;
        }
        Ok((pdf, sidecar))
    }

    /// `signing_plan_set`: make or replace the plan.
    pub(crate) fn signing_plan_set(&mut self, a: &Args) -> Result<Value> {
        let now = self.session.now_secs();
        let message = check_message(a.opt_str("message")?.unwrap_or(""))?;
        let replace = a.opt_bool("replace")?.unwrap_or(false);
        let recipients = recipients_arg(a, now)?;
        let (_, sidecar) = self.resolve_plan_paths(a, true)?;
        let old = read_plan_sidecar(&sidecar)?;
        let discards = old.as_ref().is_some_and(Plan::started);
        if discards && !replace {
            return Err(failed(
                "the plan already has a recipient who signed, approved, declined or was marked sent; set replace to true to discard that history",
            ));
        }
        let plan = Plan { version: 1, message, created: now, recipients };
        plan.validate()?;
        save(&sidecar, &plan)?;
        let mut result = plan_json(&plan, &sidecar);
        // Whether a plan was there and was replaced, and whether it had history that is now gone.
        result["replaced"] = json!(old.is_some());
        result["discarded_history"] = json!(discards);
        Ok(result)
    }

    /// `signing_plan_get`: read the plan; nothing is written.
    pub(crate) fn signing_plan_get(&self, a: &Args) -> Result<Value> {
        let (_, sidecar) = self.resolve_plan_paths(a, false)?;
        Ok(match read_plan_sidecar(&sidecar)? {
            Some(plan) => plan_json(&plan, &sidecar),
            None => json!({ "plan": null, "state": "none", "sidecar": sidecar.to_string_lossy(), "local_record_only": true, "note": NOTE }),
        })
    }

    /// `signing_plan_advance`: record that the next recipient in order signed (a signer), approved (an approver) or declined.
    pub(crate) fn signing_plan_advance(&mut self, a: &Args) -> Result<Value> {
        let now = self.session.now_secs();
        let email = a.str("email")?.trim().to_lowercase();
        // `signed` is a signer's step and `approved` an approver's: the role decides which is allowed.
        let status = match a.str("status")? {
            "signed" => Status::Signed,
            "approved" => Status::Approved,
            "declined" => Status::Declined,
            _ => return Err(bad("status must be signed, approved or declined")),
        };
        let (_, sidecar) = self.resolve_plan_paths(a, true)?;
        let mut plan =
            read_plan_sidecar(&sidecar)?.ok_or_else(|| failed("there is no signing plan for this file; make one with signing_plan_set"))?;
        let index = plan.turn_of(&email)?;
        plan.record(index, status, now)?;
        save(&sidecar, &plan)?;
        Ok(plan_json(&plan, &sidecar))
    }

    /// `signing_plan_sign`: the next signer's turn in one step. Checks the plan and the field,
    /// signs (the machinery of `sign_document`), has the validator read the signed file before it
    /// is saved, saves it over the document's file and only then records the step in the plan.
    /// `save_plan` writes the plan (the real one is [`save`]); it is a parameter so a test can
    /// make the write fail after the document is signed.
    pub(crate) fn signing_plan_sign(&mut self, a: &Args) -> Result<Value> {
        self.sign_as_recipient(a, &save)
    }

    fn sign_as_recipient(&mut self, a: &Args, save_plan: &dyn Fn(&Path, &Plan) -> Result<()>) -> Result<Value> {
        let email = a.str("email")?.trim().to_lowercase();
        let named = a.opt_str("field")?.map(str::to_string);
        let (path, field_choice) = {
            let doc = self.doc(a)?;
            let path =
                doc.path.clone().ok_or_else(|| bad("the document has no file; the plan is kept beside the document's file, so save it first"))?;
            if doc.dirty {
                return Err(failed(
                    "the document has changes that are not saved; save it first (doc_save), because the signed file replaces the saved one",
                ));
            }
            (path, signature_fields(doc, &email, named.as_deref()))
        };
        let pdf = self.resolve_for(&path, Access::Write, false)?;
        let sidecar = sidecar_of(&pdf)?;
        let shown = sidecar.to_string_lossy();
        self.check_roots(&shown, &sidecar, Access::Read)?;
        self.check_roots(&shown, &sidecar, Access::Write)?;
        let plan =
            read_plan_sidecar(&sidecar)?.ok_or_else(|| failed("there is no signing plan beside this document; make one with signing_plan_set"))?;
        let index = plan.turn_of(&email)?;
        let recipient = &plan.recipients[index];
        if recipient.role != Role::Signer {
            return Err(failed(format!(
                "{} ({}) is an approver, not a signer: this tool signs; record an approval with signing_plan_advance (status approved)",
                recipient.name, recipient.email
            )));
        }
        let (name, order, plan_email) = (recipient.name.clone(), recipient.order, recipient.email.clone());
        let field = field_choice?;

        // A field with signing scripts is refused first: this tool does not run them.
        let doc_id = self.doc(a)?.id;
        let scripted = [pdfcraft_engine::js::SignEvent::Before, pdfcraft_engine::js::SignEvent::After]
            .into_iter()
            .any(|e| self.session.sign_event_scripted(doc_id, &field, e));
        if scripted {
            return Err(failed(format!(
                "the field {field:?} has signing scripts (preSign or postSign), which this tool does not run; sign it with sign_document and run_form_scripts, then record the step with signing_plan_advance (status signed)"
            )));
        }
        // The file this tool replaces must be the file the open document was read from: if it
        // changed on disk (another copy of the document signed it, say) the open copy is stale and
        // writing it would delete what was added.
        let on_disk_is_open_copy = |me: &Self| -> Result<()> {
            let doc = me.doc(a)?;
            if doc.changed_on_open {
                return Err(failed(
                    "opening this file changed it (a repair or a form layout), so it cannot be compared with the file on disk; save it with doc_save, reopen it and try again (nothing was signed and the plan was not changed)",
                ));
            }
            let saved = doc.saved_bytes();
            let now = crate::read_input(&pdf)?;
            if *now == **saved {
                Ok(())
            } else {
                Err(failed("the file changed on disk since it was opened; reopen it (nothing was signed and the plan was not changed)"))
            }
        };
        on_disk_is_open_copy(self)?;

        // Sign, as an incremental update on the document as it is (the earlier signatures' bytes
        // stay in front), but write nothing yet: the validator reads the result first.
        let mut args = serde_json::Map::new();
        for key in ["doc", "id", "password", "reason", "location", "contact", "appearance", "labels", "graphic"] {
            if let Some(v) = a.get(key) {
                args.insert(key.to_string(), v.clone());
            }
        }
        args.insert("field".into(), json!(field));
        args.insert("out".into(), json!(path));
        let sign_args = Value::Object(args);
        let Signing { signed, doc_id, out, .. } = self.sign_bytes(&Args(&sign_args))?;
        let earlier = self.doc(a)?.signatures.iter().filter(|s| s.signed).count();
        self.session.check_signing(doc_id, &signed, &field).map_err(failed)?;

        // Just before the write: the file is still the one that was read, and the plan still says
        // it is this recipient's turn.
        let again = read_plan_sidecar(&sidecar)?.ok_or_else(|| failed("the signing plan is gone; nothing was signed"))?;
        again.turn_of(&email).map_err(|e| failed(format!("the plan changed while this call ran, so nothing was signed ({e})")))?;

        on_disk_is_open_copy(self)?;
        write_atomic(&out, &signed)?;
        let out_path = out.to_string_lossy().into_owned();
        let partial = |advanced: bool, why: String, what: String| -> Result<Value> {
            Ok(json!({
                "ok": false, "needs_attention": true,
                "status": "partial", "signed": true, "plan_advanced": advanced, "field": field,
                "recipient": { "name": name, "email": plan_email, "order": order }, "path": out_path, "bytes": signed.len(),
                "problem": why,
                "warning": format!("The document WAS signed and its file saved. {what}"),
                "identity_checked": false, "sent": false, "notified": false, "local_record_only": true, "note": format!("{SIGN_NOTE} {NOTE}"),
            }))
        };
        // The file is signed on disk whatever happens to the open document, so the plan step is
        // still recorded if the open document cannot adopt it.
        let adopt_error = self.session.mark_signed(doc_id, signed.clone(), Some(out_path.clone())).err();
        // The plan again, as it is now: the step is written only if it is still this recipient's turn.
        let now = self.session.now_secs();
        // `Err((true, _))` is a write that failed, `Err((false, _))` a plan that could not be used.
        let advanced = (|| -> std::result::Result<Plan, (bool, ToolError)> {
            let mut plan = read_plan_sidecar(&sidecar).map_err(|e| (false, e))?.ok_or_else(|| (false, failed("the plan file is gone")))?;
            let index = plan.turn_of(&email).map_err(|e| (false, e.into()))?;
            plan.record(index, Status::Signed, now).map_err(|e| (false, e.into()))?;
            save_plan(&sidecar, &plan).map_err(|e| (true, e))?;
            Ok(plan)
        })();
        let plan = match advanced {
            Ok(p) => p,
            Err((write_failed, e)) => {
                let why = e.to_string();
                let what = if write_failed {
                    format!(
                        "The signing plan could not be written: {why}. The plan still shows {name} ({plan_email}) as next; record the step with signing_plan_advance (status signed) once the plan can be written, and do not sign again."
                    )
                } else {
                    format!(
                        "The signing plan was not changed by this call, because it could not be used after the signing: {why}. Look at it with signing_plan_get and, if it is still {name}'s ({plan_email}) turn, record the step with signing_plan_advance (status signed); do not sign again."
                    )
                };
                let what = match &adopt_error {
                    Some(m) => format!("{what} The open document could not adopt the signed file either ({m}); reopen the file."),
                    None => what,
                };
                return partial(false, why, what);
            }
        };
        if let Some(m) = adopt_error {
            let what = format!("The signing plan WAS updated, but the open document could not adopt the signed file ({m}); reopen the file.");
            return partial(true, m.to_string(), what);
        }
        let doc = self.doc(a)?;
        let new = doc.signatures.iter().find(|s| s.signed && s.field == field).cloned();
        let all = doc.signatures.iter().filter(|s| s.signed).count();
        let new = new.map(|s| self.sig_json(a, &s)).transpose()?;
        let mut result = plan_json(&plan, &sidecar);
        result["ok"] = json!(true);
        result["needs_attention"] = json!(false);
        result["status"] = json!("complete");
        result["signed"] = json!(true);
        result["plan_advanced"] = json!(true);
        result["recipient"] = json!({ "name": name, "email": plan_email, "order": order });
        result["field"] = json!(field);
        result["signature"] = json!(new);
        result["path"] = json!(out_path);
        result["bytes"] = json!(signed.len());
        result["signatures_before"] = json!(earlier);
        result["signatures_now"] = json!(all);
        result["identity_checked"] = json!(false);
        result["sent"] = json!(false);
        result["notified"] = json!(false);
        result["note"] = json!(format!("{SIGN_NOTE} {NOTE}"));
        Ok(result)
    }

    /// `signing_plan_clear`: delete the sidecar, but only a file that reads as a plan.
    pub(crate) fn signing_plan_clear(&mut self, a: &Args) -> Result<Value> {
        let (_, sidecar) = self.resolve_plan_paths(a, true)?;
        let cleared = if read_plan_sidecar(&sidecar)?.is_some() {
            std::fs::remove_file(&sidecar).map_err(|e| failed(format!("{}: {e}", sidecar.display())))?;
            true
        } else {
            false
        };
        Ok(json!({ "cleared": cleared, "sidecar": sidecar.to_string_lossy(), "local_record_only": true, "note": NOTE }))
    }
}

#[cfg(test)]
mod sign_tests {
    use std::cell::Cell;

    use super::*;
    use crate::Content;

    fn call(a: &mut Automation, name: &str, args: Value) -> Value {
        match a.call(name, &args).expect("the call works").remove(0) {
            Content::Json(v) => v,
            Content::Png { .. } => panic!("expected JSON"),
        }
    }

    /// A saved form with a signature field labelled for Ana, a plan of Ana then Bo, and Ana's ID.
    fn rig() -> (tempfile::TempDir, Automation, u64) {
        let dir = tempfile::tempdir().unwrap();
        let mut a = Automation::new().with_root(dir.path()).unwrap().with_clock(|| 1_700_000_000);
        let doc = call(&mut a, "doc_create", json!({ "from": "blank", "pages": 1 }))["doc"].as_u64().unwrap();
        call(&mut a, "form_add_field", json!({ "doc": doc, "page": 1, "type": "signature", "name": "SigAna", "rect": [10, 10, 100, 60] }));
        call(&mut a, "doc_save", json!({ "doc": doc, "path": "form.pdf" }));
        let people = json!([{ "name": "Ana Example", "email": "ana@example.org" }, { "name": "Bo Example", "email": "bo@example.org" }]);
        call(&mut a, "signing_plan_set", json!({ "path": "form.pdf", "recipients": people }));
        call(&mut a, "form_field_signer", json!({ "doc": doc, "field": "SigAna", "signer": "ana@example.org" }));
        call(&mut a, "doc_save", json!({ "doc": doc, "path": "form.pdf" }));
        call(&mut a, "sign_id_create", json!({ "name": "Ana Example", "key": "p256", "password": "secret12", "path": "ana.p12" }));
        (dir, a, doc)
    }

    fn has_signature(dir: &tempfile::TempDir) -> bool {
        std::fs::read(dir.path().join("form.pdf")).unwrap().windows(9).any(|w| w == b"ByteRange")
    }

    #[test]
    fn a_plan_that_cannot_be_written_after_signing_says_the_document_was_signed() {
        let (dir, mut a, doc) = rig();
        let plan_before = std::fs::read(dir.path().join("form.pdf.pdfcraft-sign.json")).unwrap();
        let args = json!({ "doc": doc, "email": "ana@example.org", "id": "ana.p12", "password": "secret12" });
        let calls = Cell::new(0);
        let signed_when_called = Cell::new(false);
        let out = a
            .sign_as_recipient(&Args(&args), &|_, _| {
                calls.set(calls.get() + 1);
                signed_when_called.set(has_signature(&dir));
                Err(failed("disk full"))
            })
            .expect("a partial success is an answer, not an error");
        // The plan is written only once the signature is in the saved file.
        assert_eq!(calls.get(), 1);
        assert!(signed_when_called.get(), "the plan was written before the signature was in the document");
        assert_eq!(
            (out["status"].as_str(), out["signed"].as_bool(), out["plan_advanced"].as_bool()),
            (Some("partial"), Some(true), Some(false)),
            "{out}"
        );
        let warning = out["warning"].as_str().unwrap();
        assert!(warning.contains("WAS signed") && warning.contains("disk full") && warning.contains("signing_plan_advance"), "{warning}");
        // The document is signed (open and on disk), the plan is as it was and still has Ana next.
        assert!(has_signature(&dir));
        assert_eq!(call(&mut a, "sign_list", json!({ "doc": doc }))["signed"], 1);
        assert_eq!(std::fs::read(dir.path().join("form.pdf.pdfcraft-sign.json")).unwrap(), plan_before);
        assert_eq!(call(&mut a, "signing_plan_get", json!({ "path": "form.pdf" }))["next"]["email"], "ana@example.org");
    }

    #[test]
    fn the_plan_is_not_written_when_the_signature_is_refused() {
        let (dir, mut a, doc) = rig();
        let calls = Cell::new(0);
        // Bo's turn is not yet; Ana's own field is then named wrongly.
        for args in [
            json!({ "doc": doc, "email": "bo@example.org", "id": "ana.p12", "password": "secret12" }),
            json!({ "doc": doc, "email": "ana@example.org", "id": "ana.p12", "password": "secret12", "field": "Nope" }),
        ] {
            let result = a.sign_as_recipient(&Args(&args), &|_, _| {
                calls.set(calls.get() + 1);
                Ok(())
            });
            assert!(result.is_err());
        }
        assert_eq!(calls.get(), 0);
        assert!(!has_signature(&dir));
    }

    #[test]
    fn the_plan_is_not_written_for_a_signature_the_validator_refuses() {
        let (dir, mut a, doc) = rig();
        // Ana's signature locks Bo's field; Bo's turn is then refused by the validator.
        call(&mut a, "form_add_field", json!({ "doc": doc, "page": 1, "type": "signature", "name": "SigBo", "rect": [10, 100, 100, 150] }));
        call(&mut a, "form_field_signer", json!({ "doc": doc, "field": "SigBo", "signer": "bo@example.org" }));
        call(&mut a, "signature_field_lock", json!({ "doc": doc, "field": "SigAna", "action": "include", "fields": ["SigBo"] }));
        call(&mut a, "doc_save", json!({ "doc": doc, "path": "form.pdf" }));
        call(&mut a, "sign_id_create", json!({ "name": "Bo Example", "key": "p256", "password": "secret12", "path": "bo.p12" }));
        let calls = Cell::new(0);
        let count = |_: &Path, _: &Plan| {
            calls.set(calls.get() + 1);
            Ok(())
        };
        let first = json!({ "doc": doc, "email": "ana@example.org", "id": "ana.p12", "password": "secret12" });
        a.sign_as_recipient(&Args(&first), &count).expect("Ana signs");
        assert_eq!(calls.get(), 1);
        let second = json!({ "doc": doc, "email": "bo@example.org", "id": "bo.p12", "password": "secret12" });
        assert!(a.sign_as_recipient(&Args(&second), &count).is_err());
        assert_eq!(calls.get(), 1, "the plan was written for a refused signature");
        assert!(has_signature(&dir));
    }
}
