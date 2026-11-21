//! The menu-item (named) action of a link: an action dictionary `/S /Named` in the link
//! annotation's `/A`, whose `/N` names a menu item of the viewer (ISO 32000-2 §12.6.4.12). The
//! standard defines four: `NextPage`, `PrevPage`, `FirstPage` and `LastPage`; a viewer may accept
//! others. PdfCraft's form buttons run those four and `Print` and `SaveAs`.
//!
//! This module reads the named action of one link, sets one from a short safe list
//! ([`SAFE_NAMES`]) or takes one out. It does not run the action, and it reads no other kind of
//! action. A set replaces the link's whole `/A` with a new direct action dictionary (`/Type
//! /Action`, `/S /Named`, `/N`) and takes `/Dest` out, which a link with an action must not have;
//! a `/Next` chain, or any other entry, of the old action is not kept. An indirect old `/A`
//! dictionary is left in the file unreferenced. A removal takes `/A` out of the link only when it
//! is a named action (whatever its `/N`); a link with no action, or another kind of action, is
//! left as it is. A read reports `/N` only when it is a name (after references), as the standard
//! gives it; any name is reported, and `safe` says whether it is on the list a set accepts.
//!
//! The link is given as an object, which the caller found as a link of a page; it must be a
//! dictionary whose `/Subtype` is the name `Link`, and must not be the catalog, the `/Pages` node,
//! a node under it or a page, compared by object number.
//!
//! Reads are charged against an allowance of [`MAX_GETS`] for each call of [`read_named_action`]
//! and [`named_action_change`]. Each read is paid for before `Document::get` is made, and the call
//! stops and says so when it cannot pay: the link (1), each hop of the chain behind `/A` and of the
//! chain behind `/N` (as `Document::resolve` follows them: the first object and up to 32 more, so
//! at most 33 each), 67 in all. That is far below the allowance, which is a second guard tested
//! only with small ones. The page-tree walk that finds the structural objects is
//! [`structural`](crate::reference_xobjects) with its own allowance, whose first read, of the
//! catalog, is not charged; it is made once by each read of an object that is a link.

use pdfcraft_cos::{Dict, Document, ObjRef, Object};

use crate::OrganizeError;
use crate::reference_xobjects::{Gets, Spent, structural};
use crate::role_map::shown;

/// The names a set accepts: the four the standard defines, and `Print`, which PdfCraft's form
/// buttons run. That is five of the six names the buttons run (`crates/ui-egui/src/canvas/area.rs`);
/// `SaveAs` is left out because it opens a dialog and is not one of the standard names. `Print` is
/// not allowed by PDF/A (only the four page names are), and a set does not check for that.
pub const SAFE_NAMES: [&str; 5] = ["NextPage", "PrevPage", "FirstPage", "LastPage", "Print"];
/// The allowance of reads of each call that reads a link (the page-tree walk has its own).
pub const MAX_GETS: usize = 10_000;

/// A menu item to write, from [`SAFE_NAMES`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedTarget(&'static str);

impl NamedTarget {
    /// The target for `name`, which must be exactly one of [`SAFE_NAMES`].
    pub fn new(name: &str) -> Result<Self, OrganizeError> {
        SAFE_NAMES
            .iter()
            .find(|n| **n == name)
            .map(|n| NamedTarget(n))
            .ok_or_else(|| OrganizeError::Invalid(format!("a menu item is one of {}", SAFE_NAMES.join(", "))))
    }

    pub fn name(&self) -> &'static str {
        self.0
    }
}

/// What [`read_named_action`] read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedAction {
    /// The object is a dictionary whose `/Subtype` is the name `Link`.
    pub is_link: bool,
    /// The link has a `/Dest` that is not null (which a link with an action must not have).
    pub has_dest: bool,
    /// The link has an `/A` that is not null.
    pub has_entry: bool,
    /// `/A` is a dictionary after references.
    pub present: bool,
    /// The action has a `/Next` that is not null (a set or a removal drops it).
    pub has_next: bool,
    /// The action's `/S` when it is a name, cut at 127 bytes and shown with replacement
    /// characters when not valid UTF-8.
    pub action: Option<String>,
    /// `/S` and `name` are whole.
    pub exact: bool,
    /// `/S` is the name `Named`.
    pub is_named: bool,
    /// The menu item `/N` names, when it is a name (after references), cut like `action`.
    pub name: Option<String>,
    /// `name` is one of [`SAFE_NAMES`], whole.
    pub safe: bool,
    /// [`set_named_action`] and a removal can edit this link: it is a link, and not the catalog,
    /// the `/Pages` node, a node under it or a page (found by number), that page-tree walk was not
    /// cut short, and the read was not `capped`.
    pub settable: bool,
    /// The [`MAX_GETS`] allowance was spent, so what follows the stop was not read.
    pub capped: bool,
}

impl Default for NamedAction {
    fn default() -> Self {
        NamedAction {
            is_link: false,
            has_dest: false,
            has_entry: false,
            present: false,
            has_next: false,
            action: None,
            exact: true,
            is_named: false,
            name: None,
            safe: false,
            settable: false,
            capped: false,
        }
    }
}

fn read(doc: &Document, gets: &mut Gets, link: ObjRef, out: &mut NamedAction) -> Result<(), Spent> {
    let held = gets.get(doc, link)?;
    let Some(d) = held.as_dict() else { return Ok(()) };
    out.is_link = d.name(b"Subtype") == Some(b"Link");
    out.settable = out.is_link;
    if !out.is_link {
        return Ok(());
    }
    out.has_dest = d.get(b"Dest").is_some_and(|v| !matches!(v, Object::Null));
    let Some(entry) = d.get(b"A") else { return Ok(()) };
    out.has_entry = !matches!(entry, Object::Null);
    let action = gets.follow(doc, entry)?;
    let Object::Dict(action) = &*action else { return Ok(()) };
    out.present = true;
    out.has_next = action.get(b"Next").is_some_and(|v| !matches!(v, Object::Null));
    let Some(s) = action.name(b"S") else { return Ok(()) };
    let (text, whole) = shown(s);
    out.action = Some(text);
    out.exact = whole;
    out.is_named = s == b"Named";
    if !out.is_named {
        return Ok(());
    }
    let Some(n) = action.get(b"N") else { return Ok(()) };
    let n = gets.follow(doc, n)?;
    if let Object::Name(n) = &*n {
        let (text, whole) = shown(n);
        out.exact &= whole;
        out.safe = whole && SAFE_NAMES.contains(&text.as_str());
        out.name = Some(text);
    }
    Ok(())
}

/// The named action of `link` as stored, with whether it can be edited. At most [`MAX_GETS`]
/// reads of its own, plus one walk of the page tree when the object is a link. An object that is
/// not a dictionary, or is missing, reads as no action and is not settable.
pub fn read_named_action(doc: &Document, link: ObjRef) -> NamedAction {
    read_with(doc, link, MAX_GETS)
}

pub(crate) fn read_with(doc: &Document, link: ObjRef, gets: usize) -> NamedAction {
    let mut out = NamedAction::default();
    if read(doc, &mut Gets(gets), link, &mut out).is_err() {
        out.capped = true;
    }
    if out.settable && !out.capped {
        let (nodes, cut_short) = structural(doc);
        out.settable = !cut_short && !nodes.contains(&link.num);
    } else {
        out.settable = false;
    }
    out
}

/// What [`set_named_action`] would do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// It would write the link's dictionary.
    Yes,
    /// It would write nothing: a removal from a link that has no named action.
    NoChange,
    /// It is refused, for this reason.
    Refused(&'static str),
}

/// What setting `target` (a removal when `None`) on `link` would do.
pub fn named_action_change(doc: &Document, link: ObjRef, target: Option<&NamedTarget>) -> Change {
    change_with(doc, link, target, MAX_GETS)
}

pub(crate) fn change_with(doc: &Document, link: ObjRef, target: Option<&NamedTarget>, gets: usize) -> Change {
    let read = read_with(doc, link, gets);
    if read.capped {
        return Change::Refused("the link could not be read within the allowance of reads");
    }
    if !read.is_link {
        return Change::Refused("the object is not a link annotation");
    }
    if !read.settable {
        return Change::Refused("the object is not a link this can edit (the catalog, the page tree and pages are not)");
    }
    if target.is_none() && !read.is_named {
        return Change::NoChange;
    }
    Change::Yes
}

/// Set the action of `link` to a named action running `target` (replacing its `/A` and taking
/// `/Dest` out), or, when `target` is `None`, take `/A` out if it is a named action. A removal
/// from a link with no named action is no change.
pub fn set_named_action(doc: &mut Document, link: ObjRef, target: Option<&NamedTarget>) -> Result<(), OrganizeError> {
    match named_action_change(doc, link, target) {
        Change::NoChange => return Ok(()),
        Change::Refused(why) => return Err(OrganizeError::Invalid(why.into())),
        Change::Yes => {}
    }
    doc.update_dict(link, |d| match target {
        Some(t) => {
            let mut a = Dict::new();
            a.set(b"Type".to_vec(), Object::name("Action"));
            a.set(b"S".to_vec(), Object::name("Named"));
            a.set(b"N".to_vec(), Object::name(t.name()));
            d.set(b"A".to_vec(), Object::Dict(a));
            d.remove(b"Dest");
        }
        None => {
            d.remove(b"A");
        }
    })?;
    Ok(())
}
