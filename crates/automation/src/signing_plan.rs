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

use std::fs::Metadata;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
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

/// The most recipients a plan holds.
pub(crate) const MAX_RECIPIENTS: usize = 50;
/// The longest name (characters), email address (bytes, the 254 octets of an SMTP path) and message
/// (characters); a local part is at most 64 bytes.
pub(crate) const MAX_NAME_CHARS: usize = 200;
pub(crate) const MAX_EMAIL_BYTES: usize = 254;
const MAX_LOCAL_BYTES: usize = 64;
pub(crate) const MAX_MESSAGE_CHARS: usize = 2000;
/// The most status events one recipient may have in a file read back.
const MAX_HISTORY: usize = 8;
/// The largest sidecar read (bytes); a plan written here is far smaller.
const MAX_SIDECAR_BYTES: u64 = 1 << 20;
/// The suffix of the sidecar's name, added to the PDF's whole file name after a dot.
const SIDECAR_EXTENSION: &str = "pdfcraft-sign.json";

/// What every result says, so no one reads a plan as a sent request.
pub(crate) const NOTE: &str = "This is a local record kept in a file beside the PDF. It does not send the document, notify anyone or collect \
     signatures, and marking a recipient signed, approved or declined only writes down what you say happened: it does not look for a signature in the PDF.";

/// What `signing_plan_sign` says besides `NOTE`: it signs for whoever holds the ID it is given.
pub(crate) const SIGN_NOTE: &str = "The plan does not check who holds the digital ID: the signature is made with the ID you gave, whoever it belongs to, \
     and its certificate is not compared with the recipient's address (identity_checked is false). Nothing was sent and nobody was told; the plan is a local record.";

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
enum Role {
    Signer,
    Approver,
    Viewer,
}

impl Role {
    /// A viewer has nothing to do; a signer and an approver each take a turn.
    fn acts(self) -> bool {
        self != Role::Viewer
    }
}

/// `Sent` is part of the file format for a later route that sends; these tools never set it.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
enum Status {
    Pending,
    Sent,
    Signed,
    /// An approver's counterpart of `Signed`.
    Approved,
    Declined,
}

impl Status {
    /// The status that takes a turn for a role, or `None` for a viewer.
    fn done_for(role: Role) -> Option<Status> {
        match role {
            Role::Signer => Some(Status::Signed),
            Role::Approver => Some(Status::Approved),
            Role::Viewer => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
#[serde(deny_unknown_fields)]
struct Event {
    status: Status,
    /// Seconds since 1970, from the clock of the host.
    at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
struct Recipient {
    name: String,
    email: String,
    role: Role,
    order: u32,
    status: Status,
    history: Vec<Event>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
struct Plan {
    version: u32,
    message: String,
    created: i64,
    recipients: Vec<Recipient>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipientInput {
    name: String,
    email: String,
    role: Option<Role>,
    order: Option<u32>,
}

/// Characters that are controls or that hide, reorder or disguise text: the list of
/// `crates/engine/src/links.rs` (`hides_text`, not shared because it is private to that crate)
/// without its other whitespace, plus the line and paragraph separators.
pub(crate) fn is_hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{200B}'..='\u{200F}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{2069}'
                | '\u{FEFF}'
                | '\u{061C}'
                | '\u{00AD}'
                | '\u{034F}'
                | '\u{115F}'
                | '\u{1160}'
                | '\u{3164}'
                | '\u{FFA0}'
                | '\u{180E}'
                | '\u{E0000}'..='\u{E007F}'
        )
}

fn check_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(bad(format!("a recipient's name must have 1 to {MAX_NAME_CHARS} characters")));
    }
    if name.chars().any(is_hidden) {
        return Err(bad("a recipient's name must not hold control, hidden or direction-changing characters"));
    }
    Ok(name.to_string())
}

/// A shallow check only: one `@`, something either side, no spaces, a dot inside the domain. It
/// does not check that the domain exists, that the address can receive mail or that it is the
/// person's.
pub(crate) fn check_email(email: &str) -> Result<String> {
    let email = email.trim();
    let shape = || bad("an email address must look like name@example.org (one @, no spaces, a dot in the domain)");
    let too_long = || {
        bad(format!(
            "an email address may have at most {MAX_EMAIL_BYTES} bytes, and at most {MAX_LOCAL_BYTES} bytes before the @ (bytes, not characters)"
        ))
    };
    if email.len() > MAX_EMAIL_BYTES {
        return Err(too_long());
    }
    if email.chars().any(|c| is_hidden(c) || c.is_whitespace()) {
        return Err(shape());
    }
    let (local, domain) = email.split_once('@').ok_or_else(shape)?;
    let dotted = domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.') && !domain.contains("..");
    if local.len() > MAX_LOCAL_BYTES {
        return Err(too_long());
    }
    if local.is_empty() || domain.contains('@') || !dotted {
        return Err(shape());
    }
    Ok(email.to_string())
}

fn check_message(message: &str) -> Result<String> {
    if message.chars().count() > MAX_MESSAGE_CHARS {
        return Err(bad(format!("message is longer than {MAX_MESSAGE_CHARS} characters")));
    }
    if message.chars().any(|c| is_hidden(c) && !matches!(c, '\n' | '\t' | '\r')) {
        return Err(bad("message must not hold control, hidden or direction-changing characters"));
    }
    Ok(message.to_string())
}

impl Plan {
    /// The recipients who still have a turn and the one whose turn it is.
    fn next(&self) -> Option<&Recipient> {
        self.recipients.iter().find(|r| r.role.acts() && Some(r.status) != Status::done_for(r.role))
    }

    /// `declined`, `completed` (everyone who acts has signed or approved) or `in_progress`.
    fn state(&self) -> &'static str {
        if self.recipients.iter().any(|r| r.status == Status::Declined) {
            "declined"
        } else if self.next().is_none() {
            "completed"
        } else {
            "in_progress"
        }
    }

    /// The rules a plan must keep, for a file read back as well as one just made: the same
    /// limits as `signing_plan_set`, orders 1..n in list order, and turns taken in order.
    fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err(failed(format!("the plan file has version {}; this build reads version 1", self.version)));
        }
        let n = self.recipients.len();
        if n == 0 || n > MAX_RECIPIENTS || !self.recipients.iter().any(|r| r.role.acts()) {
            return Err(failed("the plan file does not hold a valid set of recipients"));
        }
        check_message(&self.message).map_err(|e| failed(format!("the plan file: {e}")))?;
        let mut seen = Vec::new();
        let mut stopped = false;
        for (i, r) in self.recipients.iter().enumerate() {
            let (name, email) = (check_name(&r.name), check_email(&r.email));
            if name.as_ref().ok() != Some(&r.name) || email.as_ref().ok() != Some(&r.email) {
                return Err(failed("the plan file has a name or address that is not valid or not trimmed"));
            }
            let key = r.email.to_lowercase();
            if seen.contains(&key) {
                return Err(failed("the plan file repeats an address"));
            }
            if r.order as usize != i + 1 {
                return Err(failed("the plan file's order numbers are not 1, 2, 3 ... in list order"));
            }
            seen.push(key);
            let last = r.history.last().map(|e| e.status);
            let starts_pending = r.history.first().is_some_and(|e| e.status == Status::Pending);
            if r.history.is_empty() || r.history.len() > MAX_HISTORY || last != Some(r.status) || !starts_pending {
                return Err(failed("the plan file's status history does not start pending and end at the recipient's status"));
            }
            if r.history.windows(2).any(|w| w[0].at > w[1].at) {
                return Err(failed("the plan file's status history has times that run backwards"));
            }
            let done = Status::done_for(r.role) == Some(r.status);
            let right_role = match r.status {
                Status::Signed | Status::Approved => done,
                Status::Declined => r.role.acts(),
                Status::Pending | Status::Sent => true,
            };
            if !right_role {
                return Err(failed("the plan file has a recipient who signed, approved or declined in the wrong role"));
            }
            if matches!(r.status, Status::Signed | Status::Approved | Status::Declined) && stopped {
                return Err(failed("the plan file has a recipient who signed, approved or declined out of turn"));
            }
            if r.role.acts() && !done {
                stopped = true;
            }
        }
        Ok(())
    }

    /// The index of the recipient whose address is `email` (lower case) when it is that
    /// recipient's turn, or why it is not: the plan is finished, no recipient has the address,
    /// the recipient is a viewer, or someone else is next. Shared by `signing_plan_advance` and
    /// `signing_plan_sign`.
    fn turn_of(&self, email: &str) -> Result<usize> {
        match self.state() {
            "completed" => return Err(failed("the plan is finished: everyone who signs or approves has done so")),
            "declined" => return Err(failed("the plan is finished: a recipient declined")),
            _ => {}
        }
        let index =
            self.recipients.iter().position(|r| r.email.to_lowercase() == email).ok_or_else(|| failed("no recipient has that email address"))?;
        if !self.recipients[index].role.acts() {
            return Err(failed("that recipient is a viewer, who has no turn to sign"));
        }
        if let Some(next) = self.next()
            && next.email.to_lowercase() != email
        {
            return Err(failed(format!("it is not that recipient's turn: the plan goes in order and {} ({}) is next", next.name, next.email)));
        }
        Ok(index)
    }

    /// Write down `status` for the recipient at `index` (whose turn it is) and check the plan
    /// again. Nothing is saved here.
    fn record(&mut self, index: usize, status: Status, now: i64) -> Result<()> {
        let r = &mut self.recipients[index];
        if status != Status::Declined && Some(status) != Status::done_for(r.role) {
            return Err(failed("a signer's step is signed and an approver's is approved; either may decline"));
        }
        r.status = status;
        // A clock that reads earlier than the last event (it was set back, or the file came from
        // another computer) does not make the history run backwards.
        let at = now.max(r.history.last().map_or(0, |e| e.at));
        r.history.push(Event { status, at });
        self.validate()
    }

    /// Whether anything beyond the first, pending state was recorded (`sent` counts).
    fn started(&self) -> bool {
        self.recipients.iter().any(|r| !matches!(r.status, Status::Pending))
    }

    fn json(&self, sidecar: &Path) -> Value {
        let next = self.next().filter(|_| self.state() == "in_progress").map(|r| json!({ "name": r.name, "email": r.email, "order": r.order }));
        json!({
            "plan": self, "state": self.state(), "next": next, "sidecar": sidecar.to_string_lossy(),
            "local_record_only": true, "note": NOTE,
        })
    }
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

/// `<folder>/<file name>.pdfcraft-sign.json` for `<folder>/<file name>`: the whole name is kept, so
/// `a.pdf` and `a.PDF` never share a plan.
fn sidecar_of(pdf: &Path) -> Result<PathBuf> {
    let is_pdf = pdf.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
    if !is_pdf {
        return Err(bad("path must be a file whose name ends in .pdf"));
    }
    let mut name = pdf.file_name().ok_or_else(|| bad("path must name a file"))?.to_os_string();
    name.push(".");
    name.push(SIDECAR_EXTENSION);
    Ok(pdf.with_file_name(name))
}

/// The plan in `resolved_sidecar`: `None` when there is no file, an error when there is one that
/// is not a readable, valid plan. Nothing is written either way.
///
/// A link is refused (the name is looked at without following it), the file is opened once, and
/// what was opened must be a plain file, within the size limit and (on unix) the same file the
/// name showed, so swapping a link in after the look is refused too. The read itself stops one
/// byte past the limit. There is no `O_NOFOLLOW` here: it needs `libc` or `rustix`, and `rustix` is
/// only a macOS test dependency of this crate.
fn read_plan_sidecar(resolved_sidecar: &Path) -> Result<Option<Plan>> {
    use std::io::Read as _;
    let shown = resolved_sidecar.display();
    let named = match std::fs::symlink_metadata(resolved_sidecar) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(failed(format!("{shown}: {e}"))),
    };
    let not_plain = || failed(format!("{shown} is not a plain file (a link or folder is not followed); it was left alone"));
    let too_big = || failed(format!("{shown} is larger than {MAX_SIDECAR_BYTES} bytes, so it is not a plan this tool wrote; it was left alone"));
    if !named.file_type().is_file() {
        return Err(not_plain());
    }
    let file = std::fs::File::open(resolved_sidecar).map_err(|e| failed(format!("{shown}: {e}")))?;
    let opened = file.metadata().map_err(|e| failed(format!("{shown}: {e}")))?;
    if !opened.is_file() || !same_file(&named, &opened) {
        return Err(not_plain());
    }
    if opened.len() > MAX_SIDECAR_BYTES {
        return Err(too_big());
    }
    let mut bytes = Vec::new();
    file.take(MAX_SIDECAR_BYTES + 1).read_to_end(&mut bytes).map_err(|e| failed(format!("{shown}: {e}")))?;
    if bytes.len() as u64 > MAX_SIDECAR_BYTES {
        return Err(too_big());
    }
    plan_from_bytes(&bytes, &shown.to_string(), "it was left alone").map(Some)
}

/// A plan read from `bytes` (`shown` names where they came from) and checked whole; `after` ends
/// each refusal (what happened to the source).
fn plan_from_bytes(bytes: &[u8], shown: &str, after: &str) -> Result<Plan> {
    let plan: Plan = serde_json::from_slice(bytes).map_err(|e| failed(format!("{shown} is not a signing plan ({e}); {after}")))?;
    plan.validate().map_err(|e| failed(format!("{shown} is not a usable signing plan ({e}); {after}")))?;
    Ok(plan)
}

/// Whether two looks at a file are of the same file (always true where the system does not say).
fn same_file(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        (a.dev(), a.ino()) == (b.dev(), b.ino())
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        true
    }
}

/// Write the plan to its sidecar, atomically.
fn save(sidecar: &Path, plan: &Plan) -> Result<()> {
    write_atomic(sidecar, &plan_bytes(plan)?)
}

/// The bytes of a plan's sidecar file.
fn plan_bytes(plan: &Plan) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(plan).map_err(failed)?;
    bytes.push(b'\n');
    Ok(bytes)
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
        let mut result = plan.json(&sidecar);
        // Whether a plan was there and was replaced, and whether it had history that is now gone.
        result["replaced"] = json!(old.is_some());
        result["discarded_history"] = json!(discards);
        Ok(result)
    }

    /// `signing_plan_get`: read the plan; nothing is written.
    pub(crate) fn signing_plan_get(&self, a: &Args) -> Result<Value> {
        let (_, sidecar) = self.resolve_plan_paths(a, false)?;
        Ok(match read_plan_sidecar(&sidecar)? {
            Some(plan) => plan.json(&sidecar),
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
        Ok(plan.json(&sidecar))
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
            let index = plan.turn_of(&email).map_err(|e| (false, e))?;
            plan.record(index, Status::Signed, now).map_err(|e| (false, e))?;
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
        let mut result = plan.json(&sidecar);
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
