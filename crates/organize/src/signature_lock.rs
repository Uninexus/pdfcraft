//! The lock of a signature field: its `/Lock` dictionary (ISO 32000-2 §12.7.5.5, Table 236),
//! which names the form fields that become read-only once the field is signed. `/Action` is the
//! name `All` (every field of the form), `Include` (only the fields in `/Fields`) or `Exclude`
//! (every field but those in `/Fields`); `/Fields` is an array of the fully qualified names of
//! fields as text strings, and is required for `Include` and `Exclude`.
//!
//! This module reads the lock of one signature field, sets it (the field's dictionary gets a new
//! direct `/Lock`) or takes it out. It does not apply the lock: the viewer does not make the
//! fields read-only after a signature, and nothing here signs. The signature verifier already
//! reads these locks to judge later changes (`crates/sign`); this module is for editing them. A
//! `/Lock` that is written by a write here has no `/P` (the PDF 2.0 permission entry): a write
//! replaces the dictionary, and a read does not report `/P`.
//!
//! The field is given as an object, which the caller found by name among the form's signature
//! fields; the object must not be the catalog, the `/Pages` node, a node under it or a page, and
//! must not hold a signature (`/V` is not null): a lock is read from the signed revision, so a
//! lock edited afterwards would say something the signature does not.
//!
//! A read has one allowance of [`MAX_GETS`] reads, against which every read it makes is charged
//! before it is made: the field's own object, every hop of a reference chain (as
//! `Document::resolve` follows them, the first object and up to 32 more), and each entry of
//! `/Fields` looked at (one more, before the entry is looked at). When it is spent the read stops
//! and says so. Separate from that, [`structural`](crate::reference_xobjects) walks the page tree
//! with its own allowance of reads to find the nodes the field must not be, and its first read,
//! of the catalog, is not charged; a write walks it once and reads the field's dictionary again.

use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString};

use crate::OrganizeError;
use crate::reference_xobjects::{Gets, Spent, structural};
use crate::role_map::shown;

/// Most names of `/Fields` looked at and listed (all are counted); also the most a write accepts.
pub const MAX_LOCK_NAMES: usize = 256;
/// Most characters of a name listed or written; a longer one in a file is cut and not `exact`.
pub const MAX_NAME_CHARS: usize = 127;
/// Most reads one call of [`read_signature_lock`] makes: its allowance, shared by everything it
/// reads (the page-tree walk has its own).
pub const MAX_GETS: usize = 10_000;

/// What a lock covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockAction {
    /// Every field of the form.
    All,
    /// Only the fields listed.
    Include,
    /// Every field but the ones listed.
    Exclude,
}

impl LockAction {
    fn name(self) -> &'static str {
        match self {
            LockAction::All => "All",
            LockAction::Include => "Include",
            LockAction::Exclude => "Exclude",
        }
    }
}

/// A lock to write: the action and, for `Include` and `Exclude`, the fully qualified names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockSpec {
    action: LockAction,
    names: Vec<String>,
}

impl LockSpec {
    /// A lock, checked: `All` lists no name, `Include` and `Exclude` list one to
    /// [`MAX_LOCK_NAMES`], each a non-empty string of at most [`MAX_NAME_CHARS`] characters and
    /// none twice.
    pub fn new(action: LockAction, names: Vec<String>) -> Result<Self, OrganizeError> {
        if action == LockAction::All {
            if !names.is_empty() {
                return Err(OrganizeError::Invalid("a lock on all fields lists no field names".into()));
            }
        } else if names.is_empty() || names.len() > MAX_LOCK_NAMES {
            return Err(OrganizeError::Invalid(format!("a lock that includes or excludes fields lists 1 to {MAX_LOCK_NAMES} names")));
        }
        for (i, n) in names.iter().enumerate() {
            if n.is_empty() || n.chars().count() > MAX_NAME_CHARS {
                return Err(OrganizeError::Invalid(format!("a field name is 1 to {MAX_NAME_CHARS} characters")));
            }
            if names[..i].contains(n) {
                return Err(OrganizeError::Invalid(format!("the field {n:?} is listed twice")));
            }
        }
        Ok(LockSpec { action, names })
    }

    pub fn action(&self) -> LockAction {
        self.action
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    fn to_dict(&self) -> Dict {
        let mut d = Dict::new();
        d.set(b"Type".to_vec(), Object::name("SigFieldLock"));
        d.set(b"Action".to_vec(), Object::name(self.action.name()));
        if self.action != LockAction::All {
            d.set(b"Fields".to_vec(), Object::Array(self.names.iter().map(|n| Object::String(PdfString::text(n))).collect()));
        }
        d
    }
}

impl Default for SignatureLock {
    /// No lock, nothing cut (`exact` is true when nothing is listed), not settable.
    fn default() -> Self {
        SignatureLock {
            has_entry: false,
            present: false,
            action: None,
            names: Vec::new(),
            count: 0,
            truncated: false,
            not_names: 0,
            exact: true,
            signed: false,
            settable: false,
            capped: false,
        }
    }
}

/// What [`read_signature_lock`] read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureLock {
    /// The field has a `/Lock` entry that is not null: what a removal takes out.
    pub has_entry: bool,
    /// The entry is a dictionary after references.
    pub present: bool,
    /// `/Action` when it is a name written in the dictionary, cut at 127 bytes and shown with
    /// replacement characters when not valid UTF-8 (`All`, `Include` and `Exclude` are the
    /// specification's; a cut or lossy one makes `exact` false).
    pub action: Option<String>,
    /// The strings among the first [`MAX_LOCK_NAMES`] entries of `/Fields` (after references), as
    /// text cut at [`MAX_NAME_CHARS`] characters, in file order.
    pub names: Vec<String>,
    /// How many entries `/Fields` has when it is an array (after references), listed or not.
    pub count: usize,
    /// `/Fields` has more entries than [`MAX_LOCK_NAMES`], so the rest were not looked at.
    pub truncated: bool,
    /// Entries among the first [`MAX_LOCK_NAMES`] that are not strings, and so not listed.
    pub not_names: usize,
    /// `action` and every listed name are whole: false when one was cut or not valid UTF-8 (true
    /// when none is listed).
    pub exact: bool,
    /// The field holds a signature: its `/V` is there and not null (a reference counts, without
    /// being followed). A signed field's lock is not edited.
    pub signed: bool,
    /// [`set_signature_lock`] and a removal can edit this field: it is a dictionary, holds no
    /// signature and is not the catalog, the `/Pages` node, a node under it or a page (found by
    /// number), and that page-tree walk was not cut short.
    pub settable: bool,
    /// The [`MAX_GETS`] allowance was spent, so what follows the stop was not read.
    pub capped: bool,
}

fn cut(s: &PdfString) -> (String, bool) {
    let mut chars = s.text_chars();
    let shown: String = chars.by_ref().take(MAX_NAME_CHARS).collect();
    (shown, chars.next().is_none())
}

fn read(doc: &Document, gets: &mut Gets, field: ObjRef, out: &mut SignatureLock) -> Result<(), Spent> {
    let held = gets.get(doc, field)?;
    let Some(d) = held.as_dict() else { return Ok(()) };
    out.settable = true;
    out.signed = d.get(b"V").is_some_and(|v| !matches!(v, Object::Null));
    let Some(entry) = d.get(b"Lock") else { return Ok(()) };
    out.has_entry = !matches!(entry, Object::Null);
    let lock = gets.follow(doc, entry)?;
    let Object::Dict(lock) = &*lock else { return Ok(()) };
    out.present = true;
    if let Some(n) = lock.name(b"Action") {
        let (text, whole) = shown(n);
        out.action = Some(text);
        out.exact &= whole;
    }
    let Some(fields) = lock.get(b"Fields") else { return Ok(()) };
    let fields = gets.follow(doc, fields)?;
    let Object::Array(items) = &*fields else { return Ok(()) };
    out.count = items.len();
    out.truncated = items.len() > MAX_LOCK_NAMES;
    for item in items.iter().take(MAX_LOCK_NAMES) {
        gets.charge()?;
        match &*gets.follow(doc, item)? {
            Object::String(s) => {
                let (text, whole) = cut(s);
                out.names.push(text);
                out.exact &= whole;
            }
            _ => out.not_names += 1,
        }
    }
    Ok(())
}

/// The lock of `field` as stored, with whether it can be edited. At most [`MAX_GETS`] reads of
/// its own, plus one walk of the page tree when the object is a dictionary that holds no
/// signature (see the module notes). An object that is not a dictionary, or is missing, reads as
/// no lock and is not settable.
pub fn read_signature_lock(doc: &Document, field: ObjRef) -> SignatureLock {
    read_with(doc, field, MAX_GETS)
}

pub(crate) fn read_with(doc: &Document, field: ObjRef, gets: usize) -> SignatureLock {
    let mut out = SignatureLock::default();
    let mut gets = Gets(gets);
    if read(doc, &mut gets, field, &mut out).is_err() {
        out.capped = true;
    }
    // `settable` was set by the read when the object is a dictionary; the walk narrows it.
    if out.settable && !out.signed && !out.capped {
        let (nodes, cut_short) = structural(doc);
        out.settable = !cut_short && !nodes.contains(&field.num);
    } else {
        out.settable = false;
    }
    out
}

/// What [`set_signature_lock`] would do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// It would write the field's dictionary.
    Yes,
    /// It would write nothing: a removal of a lock the field does not have.
    NoChange,
    /// It is refused, for this reason.
    Refused(&'static str),
}

/// What setting `lock` (a removal when `None`) on `field` would do.
pub fn signature_lock_change(doc: &Document, field: ObjRef, lock: Option<&LockSpec>) -> Change {
    change_with(doc, field, lock, MAX_GETS)
}

pub(crate) fn change_with(doc: &Document, field: ObjRef, lock: Option<&LockSpec>, gets: usize) -> Change {
    let read = read_with(doc, field, gets);
    if read.capped {
        return Change::Refused("the field could not be read within the allowance of reads");
    }
    if read.signed {
        return Change::Refused("the field holds a signature, and a signed field's lock is not edited");
    }
    if !read.settable {
        return Change::Refused("the object is not a field dictionary this can edit (the catalog, the page tree and pages are not)");
    }
    if lock.is_none() && !read.has_entry {
        return Change::NoChange;
    }
    Change::Yes
}

/// Set the `/Lock` of `field` to a new direct dictionary, or take `/Lock` out when `lock` is
/// `None`. A removal of a lock the field does not have is no change. An indirect `/Lock`
/// dictionary that is replaced or removed is left in the file unreferenced.
pub fn set_signature_lock(doc: &mut Document, field: ObjRef, lock: Option<&LockSpec>) -> Result<(), OrganizeError> {
    match signature_lock_change(doc, field, lock) {
        Change::NoChange => return Ok(()),
        Change::Refused(why) => return Err(OrganizeError::Invalid(why.into())),
        Change::Yes => {}
    }
    doc.update_dict(field, |d| match lock {
        Some(spec) => d.set(b"Lock".to_vec(), Object::Dict(spec.to_dict())),
        None => {
            d.remove(b"Lock");
        }
    })?;
    Ok(())
}
