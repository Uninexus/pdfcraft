//! Field signer tools: `form_field_signer` reads or sets which signing-plan recipient a form field
//! is meant for, and `form_signer_report` lists every field with its label.
//!
//! The label is a private key on the field's dictionary (`/PdfCraftSigner`, an email address as a
//! text string; see `pdfcraft_organize::field_signer`). It is a label stored in the file: it does
//! not restrict who can fill or sign the field, and other readers ignore it. The signing plan
//! (`signing_plan.rs`) is looked up beside the document's own file; its rules for an address
//! (`check_email`) and for who may be named (a signer or an approver of the plan) are its own.

use std::collections::BTreeMap;

use pdfcraft_engine::{Edit, FieldSignerChange, FormFieldKind};
use serde_json::{Value, json};

use crate::signing_plan::{PlanLookup, PlanPerson, check_email, is_hidden};
use crate::{Args, Automation, Result, failed};

/// What every result says, so no one reads a label as a restriction.
pub(crate) const NOTE: &str = "The signer is a label stored in this file. It does not restrict who can fill or sign the field, nothing is sent \
     and nobody is notified, and other PDF readers ignore it. The address is stored in the PDF and travels with every copy of the file.";

/// Most rows listed in each part of the report; the counts cover every field.
const MAX_ROWS: usize = 500;

fn is_signature(k: FormFieldKind) -> bool {
    k == FormFieldKind::Signature
}

fn kind_name(k: FormFieldKind) -> &'static str {
    match k {
        FormFieldKind::Text => "text",
        FormFieldKind::CheckBox => "checkbox",
        FormFieldKind::Radio => "radio",
        FormFieldKind::PushButton => "button",
        FormFieldKind::Combo => "combo",
        FormFieldKind::List => "list",
        FormFieldKind::Signature => "signature",
    }
}

/// A label as it is echoed: control, zero-width and direction-changing characters taken out, and
/// whether any were. A file can hold any text in the key; a result must not carry text that hides
/// or reorders what a reader sees.
fn shown(label: &str) -> (String, bool) {
    let clean: String = label.chars().filter(|c| !is_hidden(*c)).collect();
    let changed = clean != label;
    (clean, changed)
}

/// Whether the stored text is a well-formed address as it stands: `check_email` trims, but the
/// plan comparisons do not, so an address with spaces round it is not called well formed.
fn well_formed(text: &str) -> bool {
    text == text.trim() && check_email(text).is_ok()
}

fn person<'a>(people: &'a [PlanPerson], email: &str) -> Option<&'a PlanPerson> {
    let key = email.to_lowercase();
    people.iter().find(|p| p.email.to_lowercase() == key)
}

/// How a result says the plan was used. `error` is why a plan beside the document could not be
/// read (or the roots refused it), in which case no plan part of a result is filled.
fn plan_json(lookup: &PlanLookup, error: Option<&str>) -> Value {
    if let Some(why) = error {
        return json!({ "state": "error", "why": why });
    }
    match lookup {
        PlanLookup::Unchecked(why) => json!({ "state": "not_checked", "why": why }),
        PlanLookup::Absent => {
            json!({ "state": "none", "why": "there is no signing plan beside the document, so an address is not checked against one" })
        }
        PlanLookup::Found(p) => json!({ "state": "found", "recipients": p.len() }),
    }
}

enum Want {
    Read,
    Set(String),
    Clear,
}

impl Automation {
    /// `form_field_signer`: read, set or clear the signer label of one field.
    pub(crate) fn form_field_signer(&mut self, a: &Args) -> Result<Value> {
        let name = a.str("field")?.to_owned();
        // An explicit null or an empty string clears; an absent argument only reads.
        let want = match a.0.get("signer") {
            None => Want::Read,
            Some(Value::Null) => Want::Clear,
            Some(Value::String(s)) if s.trim().is_empty() => Want::Clear,
            Some(Value::String(s)) => Want::Set(check_email(s)?),
            Some(_) => return Err(Args::wrong("signer", "an email address, an empty string or null")),
        };
        let (obj, kind, path) = {
            let doc = self.doc(a)?;
            let mut named = doc.form.iter().filter(|f| f.name == name);
            let field = named.next().ok_or_else(|| failed(format!("there is no field named {name:?} (see form_fields)")))?;
            if named.next().is_some() {
                return Err(failed(format!("{name:?} names more than one field")));
            }
            (field.obj, field.kind, doc.path.clone())
        };
        let writes = !matches!(want, Want::Read);
        let lookup = match self.plan_beside(path.as_deref()) {
            Ok(l) => l,
            // A read still answers; a write that cannot check the plan is refused.
            Err(e) if !writes => {
                return self.field_signer_json(a, &name, obj, kind, &PlanLookup::Unchecked("the plan could not be read"), Some(&e.to_string()));
            }
            Err(e) => return Err(failed(format!("the signing plan beside the document could not be read, so the signer was not set ({e})"))),
        };
        let mut changed = false;
        if writes {
            let email = match &want {
                Want::Set(e) => match &lookup {
                    PlanLookup::Found(people) => {
                        let p = person(people, e)
                            .ok_or_else(|| failed(format!("{e} is not a recipient of the signing plan beside the document, so it was not set")))?;
                        if !p.acts {
                            return Err(failed(format!(
                                "{} ({e}) is a viewer in the signing plan, who has no turn to sign, so it was not set",
                                p.name
                            )));
                        }
                        // The plan's own spelling, so the label matches the plan exactly.
                        Some(p.email.clone())
                    }
                    _ => Some(e.clone()),
                },
                _ => None,
            };
            match self.doc(a)?.field_signer_change(obj, email.as_deref()) {
                FieldSignerChange::Refused(why) => return Err(failed(why)),
                FieldSignerChange::NoChange => {}
                FieldSignerChange::Yes => {
                    self.apply(a, Edit::SetFieldSigner { field: obj, email })?;
                    changed = true;
                }
            }
        }
        let mut out = self.field_signer_json(a, &name, obj, kind, &lookup, None)?;
        if writes {
            out["changed"] = json!(changed);
        }
        Ok(out)
    }

    fn field_signer_json(
        &self,
        a: &Args,
        name: &str,
        obj: pdfcraft_cos::ObjRef,
        kind: FormFieldKind,
        lookup: &PlanLookup,
        error: Option<&str>,
    ) -> Result<Value> {
        let read = self.doc(a)?.field_signer(obj);
        let in_plan = match (lookup, &read.email) {
            (PlanLookup::Found(p), Some(e)) => json!(person(p, e).is_some()),
            _ => Value::Null,
        };
        let (signer, cleaned) = match &read.email {
            Some(e) => {
                let (c, changed) = shown(e);
                (json!(c), changed)
            }
            None => (Value::Null, false),
        };
        let well_formed = read.email.as_deref().map_or(Value::Null, |e| json!(well_formed(e)));
        Ok(json!({
            "field": name,
            "kind": kind_name(kind),
            "signature_field": is_signature(kind),
            "signer": signer,
            "signer_cleaned": cleaned,
            "well_formed": well_formed,
            "has_entry": read.has_entry,
            "not_text": read.not_text,
            "empty_label": read.empty,
            "exact": read.exact,
            "signed": read.signed,
            "settable": read.settable && !read.signed && !read.capped,
            "in_plan": in_plan,
            "plan": plan_json(lookup, error),
            "label_only": true,
            "note": NOTE,
        }))
    }

    /// `form_signer_report`: every field with its signer, and how that fits the plan.
    pub(crate) fn form_signer_report(&self, a: &Args) -> Result<Value> {
        let doc = self.doc(a)?;
        // A plan that cannot be read, or that the roots refuse, does not fail a report: the plan
        // parts are left empty and plan.state says error.
        let (lookup, plan_error) = match self.plan_beside(doc.path.as_deref()) {
            Ok(l) => (l, None),
            Err(e) => (PlanLookup::Unchecked("the plan could not be read"), Some(e.to_string())),
        };
        let mut shared: BTreeMap<&str, usize> = BTreeMap::new();
        for f in doc.form.iter() {
            *shared.entry(f.name.as_str()).or_default() += 1;
        }
        let (mut assigned, mut unassigned, mut unreadable) = (Vec::new(), Vec::new(), Vec::new());
        let (mut assigned_n, mut unassigned_n, mut unreadable_n, mut unassigned_signatures) = (0usize, 0usize, 0usize, 0usize);
        // Distinct labels (compared without regard to case) and how many fields hold each.
        let mut labels: BTreeMap<String, (String, usize)> = BTreeMap::new();
        for f in doc.form.iter() {
            let read = doc.field_signer(f.obj);
            let sig = is_signature(f.kind);
            let ambiguous = shared.get(f.name.as_str()).is_some_and(|n| *n > 1);
            match read.email {
                Some(email) => {
                    assigned_n += 1;
                    labels.entry(email.to_lowercase()).or_insert_with(|| (email.clone(), 0)).1 += 1;
                    let known = match &lookup {
                        PlanLookup::Found(p) => json!(person(p, &email).map(|x| json!({ "name": x.name, "role": x.role }))),
                        _ => Value::Null,
                    };
                    if assigned.len() < MAX_ROWS {
                        let (signer, cleaned) = shown(&email);
                        assigned.push(json!({
                            "field": f.name, "kind": kind_name(f.kind), "signature_field": sig, "signer": signer, "signer_cleaned": cleaned,
                            "well_formed": well_formed(&email), "plan_recipient": known, "name_shared_by_fields": ambiguous,
                        }));
                    }
                }
                // An empty string is no label: the field counts as unassigned (a clear takes the key out).
                None if read.has_entry && !read.empty => {
                    unreadable_n += 1;
                    if unreadable.len() < MAX_ROWS {
                        unreadable.push(json!({ "field": f.name, "kind": kind_name(f.kind), "signature_field": sig }));
                    }
                }
                _ => {
                    unassigned_n += 1;
                    unassigned_signatures += usize::from(sig);
                    if unassigned.len() < MAX_ROWS {
                        unassigned
                            .push(json!({ "field": f.name, "kind": kind_name(f.kind), "signature_field": sig, "name_shared_by_fields": ambiguous }));
                    }
                }
            }
        }
        let (mut not_in_plan, mut without_fields) = (Vec::new(), Vec::new());
        let (mut not_in_plan_n, mut without_fields_n) = (0usize, 0usize);
        if let PlanLookup::Found(people) = &lookup {
            for (key, (email, n)) in &labels {
                if person(people, key).is_none() {
                    not_in_plan_n += 1;
                    if not_in_plan.len() < MAX_ROWS {
                        let (signer, cleaned) = shown(email);
                        not_in_plan.push(json!({ "signer": signer, "signer_cleaned": cleaned, "fields": n }));
                    }
                }
            }
            for p in people {
                if !labels.contains_key(&p.email.to_lowercase()) {
                    without_fields_n += 1;
                    if without_fields.len() < MAX_ROWS {
                        without_fields.push(json!({ "name": p.name, "email": p.email, "role": p.role, "has_a_turn": p.acts }));
                    }
                }
            }
        }
        Ok(json!({
            "fields": doc.form.len(),
            "plan": plan_json(&lookup, plan_error.as_deref()),
            "assigned_count": assigned_n,
            "assigned": assigned,
            "unassigned_count": unassigned_n,
            "unassigned": unassigned,
            "unassigned_signature_fields": unassigned_signatures,
            "unreadable_count": unreadable_n,
            "unreadable": unreadable,
            "truncated": assigned.len() < assigned_n
                || unassigned.len() < unassigned_n
                || unreadable.len() < unreadable_n
                || not_in_plan.len() < not_in_plan_n
                || without_fields.len() < without_fields_n,
            "signers_not_in_plan_count": not_in_plan_n,
            "signers_not_in_plan": not_in_plan,
            "plan_recipients_without_fields_count": without_fields_n,
            "plan_recipients_without_fields": without_fields,
            "label_only": true,
            "note": NOTE,
        }))
    }
}
