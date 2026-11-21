//! The play-a-sound, play-a-movie, play-a-rendition and rich-media actions of a link: an action
//! dictionary in the link annotation's `/A` whose `/S` is `Sound`, `Movie`, `Rendition` or
//! `RichMediaExecute`. The first two are deprecated in PDF 2.0; a rendition action can also carry a
//! `/JS` script. A viewer that follows one plays a clip, or runs a command in a media player.
//!
//! This module reads such an action of one link and takes it out. **It never writes one**: there
//! is no set, because a document written by this tool must not start playback or carry a script,
//! and a rendition needs media clips and players that this crate does not build. It does not run
//! or play anything, and it reads no other kind of action. A removal takes `/A` out of the link,
//! with any `/Next` chain it has (the chain goes with the action), only when `/S` is one of the
//! four names above; a link with no action, or another kind of action, is left as it is. The
//! annotation a rendition or a movie action points at (a screen or a movie annotation) is not
//! touched, and an indirect `/A` dictionary is left in the file unreferenced.
//!
//! A read reports what is stored: the kind, a movie's `/Operation` name, a rendition's `/OP`
//! integer, whether a `/JS` script is stored (not read), whether the action names a target
//! (`/Annotation` or `/T` of a movie, `/AN` of a rendition, `/TA` of a rich-media action; the
//! value is not followed) and whether there is a `/Next` chain. An `/S` that is an indirect
//! reference is not followed and reads as no action.
//!
//! The link is given as an object, which the caller found as a link of a page; it must be a
//! dictionary whose `/Subtype` is the name `Link`, and must not be the catalog, the `/Pages` node,
//! a node under it or a page, compared by object number.
//!
//! Reads are charged against an allowance of [`MAX_GETS`] for each call of [`read_media_action`]
//! and [`media_action_change`]. Each read is paid for before `Document::get` is made, and the
//! call stops and says so when it cannot pay: the link (1), and each hop of the chain behind `/A`
//! and behind the one entry read of the action (a movie's `/Operation` or a rendition's `/OP`), as
//! `Document::resolve` follows them (the first object and up to 32 more, so at most 33 each), 1 +
//! 2 * 33 = 67 in all. That is far below the allowance, which is a second guard tested only with
//! small ones. The page-tree walk that finds the structural objects is
//! [`structural`](crate::reference_xobjects) with its own allowance, whose first read, of the
//! catalog, is not charged; it is made once by each read of an object that is a link.

use pdfcraft_cos::{Dict, Document, ObjRef, Object};

use crate::OrganizeError;
use crate::reference_xobjects::{Gets, Spent, structural};
use crate::role_map::shown;

/// The allowance of reads of each call that reads a link (the page-tree walk has its own).
pub const MAX_GETS: usize = 10_000;

/// What [`read_media_action`] read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaAction {
    /// The object is a dictionary whose `/Subtype` is the name `Link`.
    pub is_link: bool,
    /// The link has a `/Dest` that is not null.
    pub has_dest: bool,
    /// The link has an `/A` that is not null.
    pub has_entry: bool,
    /// `/A` is a dictionary after references.
    pub present: bool,
    /// The action has a `/Next` that is not null (a removal drops it).
    pub has_next: bool,
    /// The action's `/S` when it is a direct name, cut at 127 bytes and shown with replacement
    /// characters when not valid UTF-8.
    pub action: Option<String>,
    /// `/S` and `operation` are whole.
    pub exact: bool,
    /// `/S` is one of `Sound`, `Movie`, `Rendition`, `RichMediaExecute` (compared byte for byte):
    /// the actions a removal takes out.
    pub is_media: bool,
    /// `"sound"`, `"movie"`, `"rendition"` or `"rich_media_execute"` when `is_media`.
    pub kind: Option<&'static str>,
    /// A movie action's `/Operation` when it is a name after references.
    pub operation: Option<String>,
    /// A rendition action's `/OP` when it is an integer after references.
    pub rendition_op: Option<i64>,
    /// A rendition action has a `/JS` whose value is not null (a script; it is not read).
    pub has_script: bool,
    /// The action has the entry that names what it acts on, and its value is not null: `/Annotation`
    /// or `/T` of a movie, `/AN` of a rendition, `/TA` of a rich-media action (a sound action has
    /// none, so this is false for it). The value is not followed.
    pub has_target: bool,
    /// A removal can edit this link: it is a link, and not the catalog, the `/Pages` node, a node
    /// under it or a page (found by number), that page-tree walk was not cut short, and the read
    /// was not `capped`.
    pub settable: bool,
    /// The [`MAX_GETS`] allowance was spent, so what follows the stop was not read.
    pub capped: bool,
}

impl Default for MediaAction {
    fn default() -> Self {
        MediaAction {
            is_link: false,
            has_dest: false,
            has_entry: false,
            present: false,
            has_next: false,
            action: None,
            exact: true,
            is_media: false,
            kind: None,
            operation: None,
            rendition_op: None,
            has_script: false,
            has_target: false,
            settable: false,
            capped: false,
        }
    }
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

fn read(doc: &Document, gets: &mut Gets, link: ObjRef, out: &mut MediaAction) -> Result<(), Spent> {
    let held = gets.get(doc, link)?;
    let Some(d) = held.as_dict() else { return Ok(()) };
    out.is_link = d.name(b"Subtype") == Some(b"Link");
    out.settable = out.is_link;
    if !out.is_link {
        return Ok(());
    }
    out.has_dest = not_null(d, b"Dest");
    let Some(entry) = d.get(b"A") else { return Ok(()) };
    out.has_entry = !matches!(entry, Object::Null);
    let action = gets.follow(doc, entry)?;
    let Object::Dict(action) = &*action else { return Ok(()) };
    out.present = true;
    out.has_next = not_null(action, b"Next");
    let Some(s) = action.name(b"S") else { return Ok(()) };
    let (name, whole) = shown(s);
    out.action = Some(name);
    out.exact = whole;
    out.kind = kind_of(s);
    out.is_media = out.kind.is_some();
    match out.kind {
        Some("movie") => {
            out.has_target = not_null(action, b"Annotation") || not_null(action, b"T");
            if let Some(op) = action.get(b"Operation")
                && let Object::Name(n) = &*gets.follow(doc, op)?
            {
                let (name, whole) = shown(n);
                out.operation = Some(name);
                out.exact &= whole;
            }
        }
        Some("rendition") => {
            out.has_target = not_null(action, b"AN");
            out.has_script = not_null(action, b"JS");
            if let Some(op) = action.get(b"OP")
                && let Object::Int(n) = &*gets.follow(doc, op)?
            {
                out.rendition_op = Some(*n);
            }
        }
        Some("rich_media_execute") => out.has_target = not_null(action, b"TA"),
        _ => {}
    }
    Ok(())
}

/// The media action of `link` as stored, with whether a removal can edit it. At most
/// [`MAX_GETS`] reads of its own, plus one walk of the page tree when the object is a link. An
/// object that is not a dictionary, or is missing, reads as no action and is not settable.
pub fn read_media_action(doc: &Document, link: ObjRef) -> MediaAction {
    read_with(doc, link, MAX_GETS)
}

pub(crate) fn read_with(doc: &Document, link: ObjRef, gets: usize) -> MediaAction {
    let mut out = MediaAction::default();
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

/// What [`remove_media_action`] would do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// It would write the link's dictionary.
    Yes,
    /// It would write nothing: the link has no media action.
    NoChange,
    /// It is refused, for this reason.
    Refused(&'static str),
}

/// What removing the media action of `link` would do.
pub fn media_action_change(doc: &Document, link: ObjRef) -> Change {
    change_with(doc, link, MAX_GETS)
}

pub(crate) fn change_with(doc: &Document, link: ObjRef, gets: usize) -> Change {
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
    if !read.is_media {
        return Change::NoChange;
    }
    Change::Yes
}

/// Take `/A` out of `link` if it is a sound, movie, rendition or rich-media action, with its
/// `/Next` chain. A link with no such action is no change.
pub fn remove_media_action(doc: &mut Document, link: ObjRef) -> Result<(), OrganizeError> {
    match media_action_change(doc, link) {
        Change::NoChange => return Ok(()),
        Change::Refused(why) => return Err(OrganizeError::Invalid(why.into())),
        Change::Yes => {}
    }
    doc.update_dict(link, |d| {
        d.remove(b"A");
    })?;
    Ok(())
}
