//! The local signing plan: who is asked to sign a document, in what order, with a message and each
//! person's status, and the file it is kept in. The record is a JSON file beside the PDF
//! (`<file name>.pdfcraft-sign.json`, so `a.pdf` has `a.pdf.pdfcraft-sign.json`), so the PDF's bytes
//! are never touched.
//!
//! It is a record only. Nothing is sent, nobody is notified and no signature is collected or
//! checked: a status is what the caller says happened. The order is a strict list (1, 2, 3 ...),
//! one recipient at a time; groups that may sign in parallel are not offered. The data is plain
//! serde so a later sending route can read the same file. A time (`at`, `created`) is 0 when the
//! host has no clock.
//!
//! This is the plan's data, its rules and the reading of its file, shared by the `signing_plan_*`
//! tools (the automation crate, which adds the roots and the JSON each tool answers with) and the
//! app, which does not depend on the automation crate. Writing the file stays with the caller, which
//! knows its own atomic write.

use std::fmt;
use std::fs::Metadata;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::field_signer::is_hidden;

/// Why a plan or its file was refused, in English.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanError(pub String);

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PlanError {}

type Result<T> = std::result::Result<T, PlanError>;

fn err(m: impl Into<String>) -> PlanError {
    PlanError(m.into())
}

/// The most recipients a plan holds.
pub const MAX_RECIPIENTS: usize = 50;
/// The longest name (characters), email address (bytes, the 254 octets of an SMTP path) and message
/// (characters); a local part is at most 64 bytes.
pub const MAX_NAME_CHARS: usize = 200;
pub const MAX_MESSAGE_CHARS: usize = 2000;
/// The most status events one recipient may have in a file read back.
const MAX_HISTORY: usize = 8;
/// The largest sidecar read (bytes); a plan written here is far smaller.
pub const MAX_SIDECAR_BYTES: u64 = 1 << 20;
/// The suffix of the sidecar's name, added to the PDF's whole file name after a dot.
const SIDECAR_EXTENSION: &str = "pdfcraft-sign.json";

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Signer,
    Approver,
    Viewer,
}

impl Role {
    /// A viewer has nothing to do; a signer and an approver each take a turn.
    pub fn acts(self) -> bool {
        self != Role::Viewer
    }
}

/// `Sent` is part of the file format for a later route that sends; nothing here sets it.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pending,
    Sent,
    Signed,
    /// An approver's counterpart of `Signed`.
    Approved,
    Declined,
}

impl Status {
    /// The status that takes a turn for a role, or `None` for a viewer.
    pub fn done_for(role: Role) -> Option<Status> {
        match role {
            Role::Signer => Some(Status::Signed),
            Role::Approver => Some(Status::Approved),
            Role::Viewer => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub status: Status,
    /// Seconds since 1970, from the clock of the host.
    pub at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Recipient {
    pub name: String,
    pub email: String,
    pub role: Role,
    pub order: u32,
    pub status: Status,
    pub history: Vec<Event>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub version: u32,
    pub message: String,
    pub created: i64,
    pub recipients: Vec<Recipient>,
}

/// A recipient's name, trimmed, or why it is not allowed.
pub fn check_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(err(format!("a recipient's name must have 1 to {MAX_NAME_CHARS} characters")));
    }
    if name.chars().any(is_hidden) {
        return Err(err("a recipient's name must not hold control, hidden or direction-changing characters"));
    }
    Ok(name.to_string())
}

/// The message, or why it is not allowed.
pub fn check_message(message: &str) -> Result<String> {
    if message.chars().count() > MAX_MESSAGE_CHARS {
        return Err(err(format!("message is longer than {MAX_MESSAGE_CHARS} characters")));
    }
    if message.chars().any(|c| is_hidden(c) && !matches!(c, '\n' | '\t' | '\r')) {
        return Err(err("message must not hold control, hidden or direction-changing characters"));
    }
    Ok(message.to_string())
}

impl Plan {
    /// The recipients who still have a turn and the one whose turn it is.
    pub fn next(&self) -> Option<&Recipient> {
        self.recipients.iter().find(|r| r.role.acts() && Some(r.status) != Status::done_for(r.role))
    }

    /// `declined`, `completed` (everyone who acts has signed or approved) or `in_progress`.
    pub fn state(&self) -> &'static str {
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
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err(err(format!("the plan file has version {}; this build reads version 1", self.version)));
        }
        let n = self.recipients.len();
        if n == 0 || n > MAX_RECIPIENTS || !self.recipients.iter().any(|r| r.role.acts()) {
            return Err(err("the plan file does not hold a valid set of recipients"));
        }
        check_message(&self.message).map_err(|e| err(format!("the plan file: {e}")))?;
        let mut seen = Vec::new();
        let mut stopped = false;
        for (i, r) in self.recipients.iter().enumerate() {
            let (name, email) = (check_name(&r.name), crate::field_signer::check_address(&r.email));
            if name.as_ref().ok() != Some(&r.name) || email.as_ref().ok() != Some(&r.email) {
                return Err(err("the plan file has a name or address that is not valid or not trimmed"));
            }
            let key = r.email.to_lowercase();
            if seen.contains(&key) {
                return Err(err("the plan file repeats an address"));
            }
            if r.order as usize != i + 1 {
                return Err(err("the plan file's order numbers are not 1, 2, 3 ... in list order"));
            }
            seen.push(key);
            let last = r.history.last().map(|e| e.status);
            let starts_pending = r.history.first().is_some_and(|e| e.status == Status::Pending);
            if r.history.is_empty() || r.history.len() > MAX_HISTORY || last != Some(r.status) || !starts_pending {
                return Err(err("the plan file's status history does not start pending and end at the recipient's status"));
            }
            if r.history.windows(2).any(|w| w[0].at > w[1].at) {
                return Err(err("the plan file's status history has times that run backwards"));
            }
            let done = Status::done_for(r.role) == Some(r.status);
            let right_role = match r.status {
                Status::Signed | Status::Approved => done,
                Status::Declined => r.role.acts(),
                Status::Pending | Status::Sent => true,
            };
            if !right_role {
                return Err(err("the plan file has a recipient who signed, approved or declined in the wrong role"));
            }
            if matches!(r.status, Status::Signed | Status::Approved | Status::Declined) && stopped {
                return Err(err("the plan file has a recipient who signed, approved or declined out of turn"));
            }
            if r.role.acts() && !done {
                stopped = true;
            }
        }
        Ok(())
    }

    /// The index of the recipient whose address is `email` (lower case) when it is that
    /// recipient's turn, or why it is not: the plan is finished, no recipient has the address,
    /// the recipient is a viewer, or someone else is next.
    pub fn turn_of(&self, email: &str) -> Result<usize> {
        match self.state() {
            "completed" => return Err(err("the plan is finished: everyone who signs or approves has done so")),
            "declined" => return Err(err("the plan is finished: a recipient declined")),
            _ => {}
        }
        let index = self.recipients.iter().position(|r| r.email.to_lowercase() == email).ok_or_else(|| err("no recipient has that email address"))?;
        if !self.recipients[index].role.acts() {
            return Err(err("that recipient is a viewer, who has no turn to sign"));
        }
        if let Some(next) = self.next()
            && next.email.to_lowercase() != email
        {
            return Err(err(format!("it is not that recipient's turn: the plan goes in order and {} ({}) is next", next.name, next.email)));
        }
        Ok(index)
    }

    /// Write down `status` for the recipient at `index` (whose turn it is) and check the plan
    /// again. Nothing is saved here.
    pub fn record(&mut self, index: usize, status: Status, now: i64) -> Result<()> {
        let r = &mut self.recipients[index];
        if status != Status::Declined && Some(status) != Status::done_for(r.role) {
            return Err(err("a signer's step is signed and an approver's is approved; either may decline"));
        }
        r.status = status;
        // A clock that reads earlier than the last event (it was set back, or the file came from
        // another computer) does not make the history run backwards.
        let at = now.max(r.history.last().map_or(0, |e| e.at));
        r.history.push(Event { status, at });
        self.validate()
    }

    /// Whether anything beyond the first, pending state was recorded (`sent` counts).
    pub fn started(&self) -> bool {
        self.recipients.iter().any(|r| !matches!(r.status, Status::Pending))
    }
}

/// `<folder>/<file name>.pdfcraft-sign.json` for `<folder>/<file name>`: the whole name is kept, so
/// `a.pdf` and `a.PDF` never share a plan.
pub fn sidecar_of(pdf: &Path) -> Result<PathBuf> {
    let is_pdf = pdf.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
    if !is_pdf {
        return Err(err("path must be a file whose name ends in .pdf"));
    }
    let mut name = pdf.file_name().ok_or_else(|| err("path must name a file"))?.to_os_string();
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
/// byte past the limit. There is no `O_NOFOLLOW` here: it needs `libc` or `rustix`.
pub fn read_sidecar(resolved_sidecar: &Path) -> Result<Option<Plan>> {
    use std::io::Read as _;
    let shown = resolved_sidecar.display();
    let named = match std::fs::symlink_metadata(resolved_sidecar) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(err(format!("{shown}: {e}"))),
    };
    let not_plain = || err(format!("{shown} is not a plain file (a link or folder is not followed); it was left alone"));
    let too_big = || err(format!("{shown} is larger than {MAX_SIDECAR_BYTES} bytes, so it is not a plan this tool wrote; it was left alone"));
    if !named.file_type().is_file() {
        return Err(not_plain());
    }
    let file = std::fs::File::open(resolved_sidecar).map_err(|e| err(format!("{shown}: {e}")))?;
    let opened = file.metadata().map_err(|e| err(format!("{shown}: {e}")))?;
    if !opened.is_file() || !same_file(&named, &opened) {
        return Err(not_plain());
    }
    if opened.len() > MAX_SIDECAR_BYTES {
        return Err(too_big());
    }
    let mut bytes = Vec::new();
    file.take(MAX_SIDECAR_BYTES + 1).read_to_end(&mut bytes).map_err(|e| err(format!("{shown}: {e}")))?;
    if bytes.len() as u64 > MAX_SIDECAR_BYTES {
        return Err(too_big());
    }
    plan_from_bytes(&bytes, &shown.to_string(), "it was left alone").map(Some)
}

/// A plan read from `bytes` (`shown` names where they came from) and checked whole; `after` ends
/// each refusal (what happened to the source).
pub fn plan_from_bytes(bytes: &[u8], shown: &str, after: &str) -> Result<Plan> {
    let plan: Plan = serde_json::from_slice(bytes).map_err(|e| err(format!("{shown} is not a signing plan ({e}); {after}")))?;
    plan.validate().map_err(|e| err(format!("{shown} is not a usable signing plan ({e}); {after}")))?;
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

/// The bytes of a plan's sidecar file.
pub fn plan_bytes(plan: &Plan) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(plan).map_err(|e| err(e.to_string()))?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(name: &str, email: &str, role: Role) -> Recipient {
        Recipient {
            name: name.into(),
            email: email.into(),
            role,
            order: 0,
            status: Status::Pending,
            history: vec![Event { status: Status::Pending, at: 5 }],
        }
    }

    fn plan(people: Vec<Recipient>) -> Plan {
        let recipients = people.into_iter().enumerate().map(|(i, mut r)| {
            r.order = i as u32 + 1;
            r
        });
        Plan { version: 1, message: String::new(), created: 5, recipients: recipients.collect() }
    }

    fn two() -> Plan {
        plan(vec![person("Ana", "ana@example.org", Role::Signer), person("Bo", "bo@example.org", Role::Approver)])
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pdfcraft-organize-plan-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_turn_is_taken_in_order_and_only_by_the_next_recipient() {
        let mut p = two();
        assert_eq!(p.state(), "in_progress");
        assert!(p.turn_of("bo@example.org").unwrap_err().0.contains("not that recipient's turn"));
        assert!(p.turn_of("nobody@example.org").is_err());
        let i = p.turn_of("ana@example.org").unwrap();
        p.record(i, Status::Signed, 9).unwrap();
        assert_eq!(p.next().map(|r| r.name.as_str()), Some("Bo"));
        // An approver approves; "signed" is a signer's word.
        let j = p.turn_of("bo@example.org").unwrap();
        assert!(p.clone().record(j, Status::Signed, 9).is_err());
        p.record(j, Status::Approved, 10).unwrap();
        assert_eq!(p.state(), "completed");
        assert!(p.turn_of("ana@example.org").unwrap_err().0.contains("finished"));
    }

    #[test]
    fn a_decline_ends_the_plan_and_a_clock_set_back_does_not_reverse_history() {
        let mut p = two();
        p.record(0, Status::Declined, 1).unwrap();
        assert_eq!(p.state(), "declined");
        assert!(p.started());
        // The first event was at 5; a clock reading 1 does not put the new event before it.
        let h = &p.recipients[0].history;
        assert!(h.last().map(|e| e.at) >= h.first().map(|e| e.at));
    }

    #[test]
    fn a_plan_that_breaks_a_rule_is_refused_when_read_back() {
        assert!(two().validate().is_ok());
        let mut out_of_turn = two();
        out_of_turn.recipients[1].status = Status::Approved;
        out_of_turn.recipients[1].history.push(Event { status: Status::Approved, at: 6 });
        assert!(out_of_turn.validate().unwrap_err().0.contains("out of turn"));
        let mut repeated = two();
        repeated.recipients[1].email = "ANA@example.org".into();
        assert!(repeated.validate().is_err());
        let mut viewers = two();
        viewers.recipients.iter_mut().for_each(|r| r.role = Role::Viewer);
        assert!(viewers.validate().is_err());
        let mut hidden = two();
        hidden.recipients[0].name = "A\u{202E}na".into();
        assert!(hidden.validate().is_err());
    }

    #[test]
    fn the_file_name_decides_the_sidecar_and_a_plain_file_round_trips() {
        let dir = scratch("roundtrip");
        let pdf = dir.join("a.pdf");
        let sidecar = sidecar_of(&pdf).unwrap();
        assert_eq!(sidecar.file_name().and_then(|n| n.to_str()), Some("a.pdf.pdfcraft-sign.json"));
        assert_ne!(sidecar, sidecar_of(&dir.join("A.PDF")).unwrap());
        assert!(sidecar_of(&dir.join("a.txt")).is_err());
        assert_eq!(read_sidecar(&sidecar).unwrap().map(|p| p.recipients.len()), None);
        std::fs::write(&sidecar, plan_bytes(&two()).unwrap()).unwrap();
        let back = read_sidecar(&sidecar).unwrap().expect("a plan");
        assert_eq!(back.recipients.len(), 2);
        assert_eq!(back.recipients[1].role, Role::Approver);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_is_not_a_plan_or_is_too_big_is_refused_and_left_alone() {
        let dir = scratch("refused");
        let sidecar = dir.join("a.pdf.pdfcraft-sign.json");
        std::fs::write(&sidecar, b"{ not json").unwrap();
        assert!(read_sidecar(&sidecar).unwrap_err().0.contains("not a signing plan"));
        // Well-formed JSON that breaks a rule (a recipient who signed out of turn) is refused too.
        let mut out_of_turn = two();
        out_of_turn.recipients[1].status = Status::Approved;
        out_of_turn.recipients[1].history.push(Event { status: Status::Approved, at: 6 });
        std::fs::write(&sidecar, plan_bytes(&out_of_turn).unwrap()).unwrap();
        assert!(read_sidecar(&sidecar).unwrap_err().0.contains("not a usable signing plan"));
        std::fs::write(&sidecar, vec![b' '; MAX_SIDECAR_BYTES as usize + 1]).unwrap();
        assert!(read_sidecar(&sidecar).unwrap_err().0.contains("larger than"));
        // A folder where the file should be is not read as a plan.
        let folder = dir.join("b.pdf.pdfcraft-sign.json");
        std::fs::create_dir(&folder).unwrap();
        assert!(read_sidecar(&folder).unwrap_err().0.contains("not a plain file"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_not_followed() {
        let dir = scratch("link");
        let real = dir.join("real.json");
        std::fs::write(&real, plan_bytes(&two()).unwrap()).unwrap();
        let link = dir.join("a.pdf.pdfcraft-sign.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(read_sidecar(&link).unwrap_err().0.contains("not a plain file"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
