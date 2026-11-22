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
use crate::{Args, Automation, Result, ToolError, failed, write_atomic};

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
fn is_hidden(c: char) -> bool {
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
fn check_email(email: &str) -> Result<String> {
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
    let plan: Plan = serde_json::from_slice(&bytes).map_err(|e| failed(format!("{shown} is not a signing plan ({e}); it was left alone")))?;
    plan.validate().map_err(|e| failed(format!("{shown} is not a usable signing plan ({e}); it was left alone")))?;
    Ok(Some(plan))
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

fn save(sidecar: &Path, plan: &Plan) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(plan).map_err(failed)?;
    bytes.push(b'\n');
    write_atomic(sidecar, &bytes)
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

impl Automation {
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
        match plan.state() {
            "completed" => return Err(failed("the plan is finished: everyone who signs or approves has done so")),
            "declined" => return Err(failed("the plan is finished: a recipient declined")),
            _ => {}
        }
        let index =
            plan.recipients.iter().position(|r| r.email.to_lowercase() == email).ok_or_else(|| failed("no recipient has that email address"))?;
        if !plan.recipients[index].role.acts() {
            return Err(failed("that recipient is a viewer, who has no turn to sign"));
        }
        if let Some(next) = plan.next()
            && next.email.to_lowercase() != email
        {
            return Err(failed(format!("it is not that recipient's turn: the plan goes in order and {} ({}) is next", next.name, next.email)));
        }
        let r = &mut plan.recipients[index];
        if status != Status::Declined && Some(status) != Status::done_for(r.role) {
            return Err(failed("a signer's step is signed and an approver's is approved; either may decline"));
        }
        r.status = status;
        // A clock that reads earlier than the last event (it was set back, or the file came from
        // another computer) does not make the history run backwards.
        let at = now.max(r.history.last().map_or(0, |e| e.at));
        r.history.push(Event { status, at });
        plan.validate()?;
        save(&sidecar, &plan)?;
        Ok(plan.json(&sidecar))
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
