//! The action of a screen annotation: the `/A` of a dictionary whose `/Subtype` is `Screen`. A
//! screen annotation is the area of a page where a rendition (a media clip) plays, and the
//! rendition, sound, movie or rich-media action that sits in its `/A` starts it. A viewer that
//! follows the action plays a clip or, for a rendition action's `/JS`, runs a script.
//!
//! This module lists the screen annotations of a file with what their `/A` holds, and takes the
//! `/A` out of one when it is a sound, movie, rendition or rich-media action (compared byte for
//! byte), together with its `/Next` chain (the chain goes with the action). **It never writes an
//! action**: there is no set, because a document written by this tool must not start playback or
//! carry a script, and a rendition needs media clips and players that this crate does not build.
//! It does not play or run anything, does not read the rendition a rendition action names, and
//! does not touch the annotation's `/AA` (reported, not changed), its appearance, its `/MK`, the
//! page's `/Annots` array or the objects the action pointed at; an indirect `/A` dictionary is
//! left in the file unreferenced. Another kind of action in a screen annotation's `/A`, or an
//! `/S` that is an indirect reference (not followed), is listed and left as it is.
//!
//! The file is scanned by object number, not through the pages, so a screen annotation that no
//! page lists is listed as well, and one that is a direct dictionary inside another object is not
//! listed at all. An object counts when it is a dictionary (not a stream) whose `/Subtype` is the
//! name `Screen`, written in it and not behind a reference. The scan uses
//! `Document::scan_objects`, which keeps its own cache and leaves the editing cache alone.
//!
//! Reads are charged against an allowance of [`MAX_GETS`] for each scan: one before each object
//! is scanned, and one before each `Document::get` made for the object behind a reference in
//! `/A` (the first object and up to 32 more, as `Document::resolve` follows them, at most 33 for
//! each annotation). A rendition's `/JS` and a `/Next` are only looked at where they are written,
//! not followed. When the allowance is spent the scan stops and what it found stays. The
//! page-tree walk that finds the structural objects is [`structural`](crate::reference_xobjects)
//! with its own allowance, whose first read, of the catalog, is not charged. A read makes one scan and,
//! only when something is listed, one such walk. A call of [`screen_action_change`] or
//! [`remove_screen_action`] makes no scan for a position of [`MAX_LISTED`] or more, and otherwise
//! one scan, and one walk only when an annotation is listed at that position.

use pdfcraft_cos::{Dict, Document, ObjRef, Object};

use crate::OrganizeError;
use crate::reference_xobjects::{Gets, Spent, structural};
use crate::role_map::shown;

/// Most screen annotations listed (all that were found are counted).
pub const MAX_LISTED: usize = 256;
/// The allowance of reads of one scan.
pub const MAX_GETS: usize = 2_000_000;

/// One screen annotation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenAction {
    /// Position in the listing, counting from 0 (what [`remove_screen_action`] takes).
    pub index: usize,
    /// The annotation's object.
    pub object: ObjRef,
    /// The annotation has an `/A` that is not null.
    pub has_entry: bool,
    /// `/A` is a dictionary after references.
    pub present: bool,
    /// The action's `/S` when it is a direct name, cut at 127 bytes and shown with replacement
    /// characters when not valid UTF-8.
    pub action: Option<String>,
    /// `action` is whole.
    pub exact: bool,
    /// `"sound"`, `"movie"`, `"rendition"` or `"rich_media_execute"` when `/S` is one of `Sound`,
    /// `Movie`, `Rendition`, `RichMediaExecute` (compared byte for byte): the actions a removal
    /// takes out.
    pub kind: Option<&'static str>,
    /// The action has a `/Next` that is not null (a removal drops it).
    pub has_next: bool,
    /// A rendition action has a `/JS` whose value is not null (a script; it is not read).
    pub has_script: bool,
    /// The annotation has an `/AA` that is not null (additional actions; a removal keeps it).
    pub has_additional: bool,
    /// [`remove_screen_action`] can edit this object: it is not the catalog, the `/Pages` node or
    /// a node under it (found by number), and that page-tree walk was not cut short.
    pub removable: bool,
}

/// What a scan found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScreenActions {
    /// The first [`MAX_LISTED`] screen annotations, in object-number order.
    pub items: Vec<ScreenAction>,
    /// How many were found, whether listed or not.
    pub count: usize,
    /// More were found than listed.
    pub truncated: bool,
    /// The scan stopped because the allowance was spent, so `count` is a lower bound and objects
    /// after the stop were not looked at.
    pub capped: bool,
}

fn not_null(d: &Dict, key: &[u8]) -> bool {
    d.get(key).is_some_and(|v| !matches!(v, Object::Null))
}

fn kind_of(s: &[u8]) -> Option<&'static str> {
    match s {
        b"Sound" => Some("sound"),
        b"Movie" => Some("movie"),
        b"Rendition" => Some("rendition"),
        b"RichMediaExecute" => Some("rich_media_execute"),
        _ => None,
    }
}

fn read(doc: &Document, gets: &mut Gets, d: &Dict, object: ObjRef) -> Result<ScreenAction, Spent> {
    let mut out = ScreenAction {
        index: 0,
        object,
        has_entry: false,
        present: false,
        action: None,
        exact: true,
        kind: None,
        has_next: false,
        has_script: false,
        has_additional: not_null(d, b"AA"),
        removable: true,
    };
    let Some(entry) = d.get(b"A") else { return Ok(out) };
    out.has_entry = !matches!(entry, Object::Null);
    let action = gets.follow(doc, entry)?;
    let Object::Dict(action) = &*action else { return Ok(out) };
    out.present = true;
    out.has_next = not_null(action, b"Next");
    if let Some(s) = action.name(b"S") {
        let (name, whole) = shown(s);
        out.action = Some(name);
        out.exact = whole;
        out.kind = kind_of(s);
        out.has_script = out.kind == Some("rendition") && not_null(action, b"JS");
    }
    Ok(out)
}

/// The scan, with `allowance` reads. With `stop_after` it ends as soon as the item at that
/// position is listed, which is enough to find it.
pub(crate) fn scan(doc: &Document, stop_after: Option<usize>, allowance: usize) -> ScreenActions {
    let mut out = ScreenActions::default();
    let mut gets = Gets(allowance);
    let mut objects = doc.scan_objects();
    // `scan_objects` knows how many objects are left, so the last one is paid for only when it
    // is there to read.
    while objects.size_hint().1 != Some(0) {
        if gets.charge().is_err() {
            out.capped = true;
            break;
        }
        let Some((at, object)) = objects.next() else { break };
        let Object::Dict(d) = &*object else { continue };
        if d.name(b"Subtype") != Some(b"Screen") {
            continue;
        }
        match read(doc, &mut gets, d, at) {
            Err(Spent) => {
                out.capped = true;
                break;
            }
            Ok(mut item) => {
                out.count += 1;
                if out.items.len() < MAX_LISTED {
                    item.index = out.items.len();
                    out.items.push(item);
                    if stop_after.is_some_and(|i| out.items.len() > i) {
                        break;
                    }
                }
            }
        }
    }
    out.truncated = out.count > out.items.len();
    out
}

/// The screen annotations of the file with what their `/A` holds, from one scan (at most
/// [`MAX_GETS`] reads) and, when something is listed, one walk of the page tree.
pub fn read_screen_actions(doc: &Document) -> ScreenActions {
    read_with(doc, MAX_GETS)
}

pub(crate) fn read_with(doc: &Document, allowance: usize) -> ScreenActions {
    let mut found = scan(doc, None, allowance);
    if !found.items.is_empty() {
        let (nodes, cut) = structural(doc);
        for item in &mut found.items {
            item.removable = !cut && !nodes.contains(&item.object.num);
        }
    }
    found
}

/// What [`remove_screen_action`] with a position would do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// It would write the annotation's dictionary.
    Yes,
    /// It would write nothing: the annotation has no media action.
    NoChange,
    /// Nothing is listed at that position (a position of [`MAX_LISTED`] or more never is).
    NoEntry,
    /// It is refused, for this reason.
    Refused(&'static str),
}

/// The object to take `/A` from and whether its action is one a removal takes out, or why not.
fn target(doc: &Document, index: usize, allowance: usize) -> Result<(ObjRef, bool), Change> {
    if index >= MAX_LISTED {
        return Err(Change::NoEntry);
    }
    let found = scan(doc, Some(index), allowance);
    let Some(item) = found.items.get(index) else {
        return Err(if found.capped { Change::Refused("the file could not be scanned within the allowance of reads") } else { Change::NoEntry });
    };
    let (nodes, cut) = structural(doc);
    if cut || nodes.contains(&item.object.num) {
        return Err(Change::Refused("the object is the catalog or a node of the page tree, which this does not change"));
    }
    Ok((item.object, item.kind.is_some()))
}

/// What removing the media action of the screen annotation listed at `index` would do.
pub fn screen_action_change(doc: &Document, index: usize) -> Change {
    change_with(doc, index, MAX_GETS)
}

pub(crate) fn change_with(doc: &Document, index: usize, allowance: usize) -> Change {
    match target(doc, index, allowance) {
        Err(change) => change,
        Ok((_, true)) => Change::Yes,
        Ok((_, false)) => Change::NoChange,
    }
}

/// Take `/A` out of the screen annotation listed at `index` if it is a sound, movie, rendition or
/// rich-media action, with its `/Next` chain. A position with nothing listed, and an annotation
/// with no such action, are no change; an object that is not removable is refused.
pub fn remove_screen_action(doc: &mut Document, index: usize) -> Result<(), OrganizeError> {
    match target(doc, index, MAX_GETS) {
        Err(Change::Refused(why)) => Err(OrganizeError::Invalid(why.into())),
        Err(_) | Ok((_, false)) => Ok(()),
        Ok((at, true)) => {
            doc.update_dict(at, |d| {
                d.remove(b"A");
            })?;
            Ok(())
        }
    }
}
