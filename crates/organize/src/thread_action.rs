//! The article (thread) action of a link: an action dictionary `/S /Thread` in the link
//! annotation's `/A`, which tells a viewer to go to an article thread and, optionally, to one of
//! its beads (ISO 32000-2 §12.6.4.7). `/D` is the thread, as the thread dictionary
//! (an indirect reference), the position of the thread in the catalog's `/Threads`, or the title of
//! the thread as a text string; `/B` is the bead, as the bead dictionary (an indirect reference)
//! or its position along the thread; with an `/F` (a file specification) the thread is in another
//! file. A read reports a title-named `/D` as `title` and does not match it against the threads,
//! and does not resolve a thread or bead of a remote action (`remote`).
//!
//! This module reads the thread action of one link, sets one that names a thread of the catalog's
//! `/Threads` (and a bead of it), or takes one out. It does not run the action: nothing in the
//! viewer follows it, it adds no thread or bead, and it reads no other kind of action. A set
//! replaces the link's whole `/A` with a new direct action dictionary (`/Type /Action`, `/S
//! /Thread`, `/D` and, for a bead, `/B`, both as references) and takes `/Dest` out, which a link
//! with an action must not have; a `/Next` chain, or any other entry, of the old action is not
//! kept. An indirect old `/A` dictionary is left in the file unreferenced. A removal takes `/A`
//! out of the link only when it is a thread action; a link with no action, or another kind of
//! action, is left as it is.
//!
//! The link is given as an object, which the caller found as a link of a page; it must be a
//! dictionary whose `/Subtype` is the name `Link`, and must not be the catalog, the `/Pages` node,
//! a node under it or a page, compared by object number.
//!
//! Reads are charged against an allowance of [`MAX_GETS`] for each call of [`read_thread_action`]
//! and [`thread_action_change`] (one allowance for the read of the link and, for a set, the read of
//! the thread), and [`set_thread_action`] makes a change check and then reads the thread again
//! with an allowance of its own. Each read is paid for before `Document::get` is made, and the
//! call stops and says so when it cannot pay: the link, each hop of a reference chain (as
//! `Document::resolve` follows them: the first object and up to 32 more), the catalog, the
//! thread's dictionary (read when a bead of it is looked for, and by a set that names a thread
//! with no bead, to check that it is a dictionary) and each bead walked. At most [`MAX_THREADS`]
//! entries of `/Threads` are looked at (compared as references, with no read of their own) and at
//! most [`MAX_BEADS`] beads are walked along `/N`, each once. The page-tree walk that finds the structural objects is
//! [`structural`](crate::reference_xobjects) with its own allowance, whose first read, of the
//! catalog, is not charged; it is made once by a read of an object that is a link.

use std::collections::HashSet;

use pdfcraft_cos::{Dict, Document, ObjRef, Object};

use crate::OrganizeError;
use crate::reference_xobjects::{Gets, Spent, structural};
use crate::role_map::shown;

/// Most entries of `/Threads` looked at, and the most a set can name: positions 0 to this less one.
pub const MAX_THREADS: usize = 256;
/// Most beads walked along a thread, and the most a set can reach: positions 0 to this less one.
pub const MAX_BEADS: usize = 256;
/// The allowance of reads of each call that reads a link (the page-tree walk has its own).
/// Most characters of a title-named `/D` listed; a longer one is cut and not `exact`.
pub const MAX_TITLE_CHARS: usize = 127;
pub const MAX_GETS: usize = 10_000;

/// A thread and, optionally, a bead of it to write, as positions counting from 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadTarget {
    thread: usize,
    bead: Option<usize>,
}

impl ThreadTarget {
    /// A target, checked: `thread` is less than [`MAX_THREADS`] and `bead` less than
    /// [`MAX_BEADS`].
    pub fn new(thread: usize, bead: Option<usize>) -> Result<Self, OrganizeError> {
        if thread >= MAX_THREADS {
            return Err(OrganizeError::Invalid(format!("a thread is one of the first {MAX_THREADS}")));
        }
        if bead.is_some_and(|b| b >= MAX_BEADS) {
            return Err(OrganizeError::Invalid(format!("a bead is one of the first {MAX_BEADS} of its thread")));
        }
        Ok(ThreadTarget { thread, bead })
    }

    pub fn thread(&self) -> usize {
        self.thread
    }

    pub fn bead(&self) -> Option<usize> {
        self.bead
    }
}

/// What [`read_thread_action`] read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadAction {
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
    /// `action` is whole.
    pub exact: bool,
    /// `/S` is the name `Thread`.
    pub is_thread: bool,
    /// The thread action has a non-null `/F`: its thread is in another file, so `thread`, `title`
    /// and `bead` are not read.
    pub remote: bool,
    /// `/D` is a text string, which the standard allows as the title of the thread (its `/I`
    /// `/Title`): the string, cut at 127 characters. It is not matched against the titles of the
    /// threads, so `thread` stays `None`; `exact` is false when it was cut.
    pub title: Option<String>,
    /// The position in `/Threads` (among the first [`MAX_THREADS`] entries) that `/D` names, as a
    /// reference to one of them or as an integer position in range; `None` for a thread action
    /// whose `/D` is missing, is a title or a direct dictionary, names something else or is out
    /// of range, and for a remote one.
    pub thread: Option<usize>,
    /// The position along the thread's beads (the first [`MAX_BEADS`], from `/F` along `/N`) that
    /// `/B` names, as a reference to one of them or as an integer position in range; `None` when
    /// there is no `/B`, the thread was not found, or `/B` names something else or is out of
    /// range.
    pub bead: Option<usize>,
    /// [`set_thread_action`] and a removal can edit this link: it is a link, and not the catalog,
    /// the `/Pages` node, a node under it or a page (found by number), that page-tree walk was not
    /// cut short, and the read was not `capped`.
    pub settable: bool,
    /// The [`MAX_GETS`] allowance was spent, so what follows the stop was not read.
    pub capped: bool,
}

impl Default for ThreadAction {
    fn default() -> Self {
        ThreadAction {
            is_link: false,
            has_dest: false,
            has_entry: false,
            present: false,
            action: None,
            exact: true,
            is_thread: false,
            has_next: false,
            remote: false,
            title: None,
            thread: None,
            bead: None,
            settable: false,
            capped: false,
        }
    }
}

/// The first [`MAX_THREADS`] entries of the catalog's `/Threads` that are references, as
/// positions (`None` for an entry that is not one), after one read of the catalog and the array.
fn thread_entries(doc: &Document, gets: &mut Gets) -> Result<Vec<Option<ObjRef>>, Spent> {
    let Some(root) = doc.root() else { return Ok(Vec::new()) };
    let catalog = gets.get(doc, root)?;
    let Some(entry) = catalog.as_dict().and_then(|c| c.get(b"Threads")) else { return Ok(Vec::new()) };
    let held = gets.follow(doc, entry)?;
    let Object::Array(items) = &*held else { return Ok(Vec::new()) };
    Ok(items.iter().take(MAX_THREADS).map(|i| i.as_ref()).collect())
}

/// Whether the object `thread` is a dictionary (after one charged read), and if so the first bead
/// its `/F` names when that is a reference.
fn thread_first(doc: &Document, gets: &mut Gets, thread: ObjRef) -> Result<Option<Option<ObjRef>>, Spent> {
    let held = gets.get(doc, thread)?;
    Ok(held.as_dict().map(|d| d.get(b"F").and_then(|f| f.as_ref())))
}

/// The beads of a thread from its first one along `/N`, at most [`MAX_BEADS`], each read charged
/// and each bead once. A bead that is not a dictionary ends the walk.
fn bead_refs(doc: &Document, gets: &mut Gets, first: ObjRef) -> Result<Vec<ObjRef>, Spent> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut at = first;
    while out.len() < MAX_BEADS && seen.insert(at.num) {
        let bead = gets.get(doc, at)?;
        let Some(bead) = bead.as_dict() else { break };
        out.push(at);
        match bead.get(b"N").and_then(|n| n.as_ref()) {
            Some(next) => at = next,
            None => break,
        }
    }
    Ok(out)
}

/// The position `named` (a reference, or an integer position) has among `items`.
fn position(items: &[Option<ObjRef>], named: &Object) -> Option<usize> {
    match named {
        Object::Ref(r) => items.iter().position(|i| *i == Some(*r)),
        Object::Int(i) => usize::try_from(*i).ok().filter(|i| *i < items.len()),
        _ => None,
    }
}

fn cut(s: &pdfcraft_cos::PdfString) -> (String, bool) {
    let mut chars = s.text_chars();
    let shown: String = chars.by_ref().take(MAX_TITLE_CHARS).collect();
    (shown, chars.next().is_none())
}

fn read(doc: &Document, gets: &mut Gets, link: ObjRef, out: &mut ThreadAction) -> Result<(), Spent> {
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
    out.is_thread = s == b"Thread";
    if !out.is_thread {
        return Ok(());
    }
    if action.get(b"F").is_some_and(|v| !matches!(v, Object::Null)) {
        out.remote = true;
        return Ok(());
    }
    let Some(target) = action.get(b"D") else { return Ok(()) };
    if let Object::String(t) = target {
        let (text, whole) = cut(t);
        out.title = Some(text);
        out.exact &= whole;
        return Ok(());
    }
    let entries = thread_entries(doc, gets)?;
    out.thread = position(&entries, target);
    let (Some(at), Some(bead)) = (out.thread.and_then(|t| entries.get(t).copied().flatten()), action.get(b"B")) else { return Ok(()) };
    let Some(Some(first)) = thread_first(doc, gets, at)? else { return Ok(()) };
    let beads: Vec<Option<ObjRef>> = bead_refs(doc, gets, first)?.into_iter().map(Some).collect();
    out.bead = position(&beads, bead);
    Ok(())
}

/// The thread action of `link` as stored, with whether it can be edited. At most [`MAX_GETS`]
/// reads of its own, plus one walk of the page tree when the object is a link. An object that is
/// not a dictionary, or is missing, reads as no action and is not settable.
pub fn read_thread_action(doc: &Document, link: ObjRef) -> ThreadAction {
    read_with(doc, link, MAX_GETS)
}

pub(crate) fn read_with(doc: &Document, link: ObjRef, gets: usize) -> ThreadAction {
    read_charging(doc, &mut Gets(gets), link)
}

fn read_charging(doc: &Document, gets: &mut Gets, link: ObjRef) -> ThreadAction {
    let mut out = ThreadAction::default();
    if read(doc, gets, link, &mut out).is_err() {
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

/// What [`set_thread_action`] would do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// It would write the link's dictionary.
    Yes,
    /// It would write nothing: a removal from a link that has no thread action.
    NoChange,
    /// It is refused, for this reason.
    Refused(&'static str),
}

/// The thread and bead references a target names, or why it names none.
fn target_refs(doc: &Document, gets: &mut Gets, target: &ThreadTarget) -> Result<Result<(ObjRef, Option<ObjRef>), &'static str>, Spent> {
    let entries = thread_entries(doc, gets)?;
    let Some(thread) = entries.get(target.thread).copied().flatten() else {
        return Ok(Err("the catalog has no article thread at that position, or it is not an indirect object"));
    };
    let Some(first) = thread_first(doc, gets, thread)? else { return Ok(Err("the entry of /Threads is not a thread dictionary")) };
    let Some(bead) = target.bead else { return Ok(Ok((thread, None))) };
    let Some(first) = first else { return Ok(Err("the thread has no bead at that position")) };
    Ok(match bead_refs(doc, gets, first)?.get(bead) {
        Some(b) => Ok((thread, Some(*b))),
        None => Err("the thread has no bead at that position"),
    })
}

/// What setting `target` (a removal when `None`) on `link` would do.
pub fn thread_action_change(doc: &Document, link: ObjRef, target: Option<&ThreadTarget>) -> Change {
    change_with(doc, link, target, MAX_GETS)
}

pub(crate) fn change_with(doc: &Document, link: ObjRef, target: Option<&ThreadTarget>, gets: usize) -> Change {
    let mut gets = Gets(gets);
    let read = read_charging(doc, &mut gets, link);
    if read.capped {
        return Change::Refused("the link could not be read within the allowance of reads");
    }
    if !read.is_link {
        return Change::Refused("the object is not a link annotation");
    }
    if !read.settable {
        return Change::Refused("the object is not a link this can edit (the catalog, the page tree and pages are not)");
    }
    let Some(target) = target else {
        return if read.is_thread { Change::Yes } else { Change::NoChange };
    };
    match target_refs(doc, &mut gets, target) {
        Err(Spent) => Change::Refused("the thread could not be read within the allowance of reads"),
        Ok(Err(why)) => Change::Refused(why),
        Ok(Ok(_)) => Change::Yes,
    }
}

/// Set the action of `link` to a thread action naming `target` (replacing its `/A` and taking
/// `/Dest` out), or, when `target` is `None`, take `/A` out if it is a thread action. A removal
/// from a link with no thread action is no change.
pub fn set_thread_action(doc: &mut Document, link: ObjRef, target: Option<&ThreadTarget>) -> Result<(), OrganizeError> {
    match thread_action_change(doc, link, target) {
        Change::NoChange => return Ok(()),
        Change::Refused(why) => return Err(OrganizeError::Invalid(why.into())),
        Change::Yes => {}
    }
    let refs = match target {
        Some(t) => match target_refs(doc, &mut Gets(MAX_GETS), t) {
            Ok(Ok(refs)) => Some(refs),
            Ok(Err(why)) => return Err(OrganizeError::Invalid(why.into())),
            Err(Spent) => return Err(OrganizeError::Invalid("the thread could not be read within the allowance of reads".into())),
        },
        None => None,
    };
    doc.update_dict(link, |d| match refs {
        Some((thread, bead)) => {
            let mut a = Dict::new();
            a.set(b"Type".to_vec(), Object::name("Action"));
            a.set(b"S".to_vec(), Object::name("Thread"));
            a.set(b"D".to_vec(), Object::Ref(thread));
            if let Some(bead) = bead {
                a.set(b"B".to_vec(), Object::Ref(bead));
            }
            d.set(b"A".to_vec(), Object::Dict(a));
            d.remove(b"Dest");
        }
        None => {
            d.remove(b"A");
        }
    })?;
    Ok(())
}
