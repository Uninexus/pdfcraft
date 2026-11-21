//! The open-a-file action of a link: an action dictionary `/S /Launch` in the link annotation's
//! `/A`, whose `/F` is a file specification (ISO 32000-2 §12.6.4.6, Table 207) and whose optional
//! `/NewWindow` asks for a new window. A viewer may also read the deprecated platform entries
//! `/Win`, `/Mac` and `/Unix`.
//!
//! This module reads the launch action of one link, sets one that names a relative path to a PDF
//! file ([`LaunchTarget`]) or takes one out. It does not run the action, and it reads no other
//! kind of action. A set accepts only what PdfCraft's render crate reads from a launch (a `.pdf`
//! file, see `launch_pdf` in `crates/render/src/inspect/annotations.rs`) and no more: a document
//! that this tool writes must not name a program, an absolute path, a path that climbs out of the
//! folder (`..`) or a URL. A set replaces the link's whole `/A` with a new direct action
//! dictionary (`/Type /Action`, `/S /Launch`, `/F` a string, and `/NewWindow` only when given) and
//! takes `/Dest` out, which a link with an action must not have; a `/Next` chain, platform
//! entries, or any other entry of the old action is not kept. An indirect old `/A` dictionary is
//! left in the file unreferenced. A removal takes `/A` out of the link only when it is a launch
//! action (whatever it launches); a link with no action, or another kind of action, is left as it
//! is. A read reports what is stored, of any kind: `/F` as a string or as a file specification
//! dictionary (its `/UF`, else its `/F`, as text), and `/NewWindow` when it is a boolean.
//!
//! The link is given as an object, which the caller found as a link of a page; it must be a
//! dictionary whose `/Subtype` is the name `Link`, and must not be the catalog, the `/Pages` node,
//! a node under it or a page, compared by object number.
//!
//! Reads are charged against an allowance of [`MAX_GETS`] for each call of [`read_launch_action`]
//! and [`launch_action_change`]. Each read is paid for before `Document::get` is made, and the
//! call stops and says so when it cannot pay: the link (1), and each hop of the chain behind `/A`,
//! `/NewWindow`, `/F` and, when `/F` is a dictionary, behind its `/UF`, `/F` and `/FS` (as
//! `Document::resolve` follows them: the first object and up to 32 more, so at most 33 each), 1 +
//! 6 * 33 = 199 in all. That is far below the allowance, which is a second guard tested only with
//! small ones. The page-tree walk that finds the structural objects is
//! [`structural`](crate::reference_xobjects) with its own allowance, whose first read, of the
//! catalog, is not charged; it is made once by each read of an object that is a link.

use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString};

use crate::OrganizeError;
use crate::reference_xobjects::{Gets, Spent, structural};
use crate::role_map::shown;

/// The allowance of reads of each call that reads a link (the page-tree walk has its own).
pub const MAX_GETS: usize = 10_000;
/// The longest path a set accepts, in bytes, and the most characters of a stored one a read shows.
pub const MAX_FILE_BYTES: usize = 255;

/// A file to open, checked by [`LaunchTarget::new`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchTarget {
    file: String,
    new_window: Option<bool>,
}

impl LaunchTarget {
    /// The target for `file`, which must be a relative path to a `.pdf` file written with `/`
    /// separators: only letters, digits, `.`, `_`, `-`, space and `/`, at most [`MAX_FILE_BYTES`]
    /// bytes (so no `\`, no `:`, no drive, no URL, and no character a shell treats as more than
    /// text), no leading `/` or space, no trailing space, and no part (between `/`) that is
    /// empty, made only of dots and spaces (`.`, `..`) or starts with `-`. `new_window` is written
    /// only when given.
    pub fn new(file: &str, new_window: Option<bool>) -> Result<Self, OrganizeError> {
        let bad = |why: &str| Err(OrganizeError::Invalid(format!("a file to open {why}")));
        if file.is_empty() || file.len() > MAX_FILE_BYTES {
            return bad(&format!("is 1 to {MAX_FILE_BYTES} bytes"));
        }
        if !file.bytes().all(|b| b.is_ascii_alphanumeric() || b" ._-/".contains(&b)) {
            return bad("is made only of letters, digits, . _ - space and /");
        }
        if file.starts_with('/') || file.starts_with(' ') || file.ends_with(' ') {
            return bad("is a relative path with no space at either end");
        }
        if file.split('/').any(|part| part.starts_with('-') || part.bytes().all(|b| b == b'.' || b == b' ')) {
            return bad("has no part that is empty, only dots or spaces, or starts with -");
        }
        let name = file.rsplit('/').next().unwrap_or(file);
        if name.len() <= 4 || !name.to_ascii_lowercase().ends_with(".pdf") {
            return bad("names a .pdf file");
        }
        Ok(LaunchTarget { file: file.to_owned(), new_window })
    }

    pub fn file(&self) -> &str {
        &self.file
    }

    pub fn new_window(&self) -> Option<bool> {
        self.new_window
    }
}

/// What [`read_launch_action`] read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchAction {
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
    /// The action has a `/Win`, `/Mac` or `/Unix` entry whose value is not null (a set or a
    /// removal drops it). A key whose value is null counts as none here, while the render crate
    /// (`has(b"Win")` in `launch_pdf`) counts a `/Win` key even when its value is null, and then
    /// does not read the launch as a link.
    pub has_platform: bool,
    /// The action's `/S` when it is a name, cut at 127 bytes and shown with replacement
    /// characters when not valid UTF-8.
    pub action: Option<String>,
    /// `/S` and `file` are whole.
    pub exact: bool,
    /// `/S` is the name `Launch`.
    pub is_launch: bool,
    /// `/F` is a string (`"string"`) or a dictionary (`"dictionary"`) after references; `None`
    /// when it is missing or anything else.
    pub file_kind: Option<&'static str>,
    /// The path `/F` holds (a string) or its `/UF`, else its `/F`, as text, cut at
    /// [`MAX_FILE_BYTES`] characters. `None` when there is none.
    pub file: Option<String>,
    /// A file specification dictionary whose `/FS` is the name `URL`: `file` is a URL, not a path.
    pub url: bool,
    /// `/NewWindow` when it is a boolean after references.
    pub new_window: Option<bool>,
    /// [`set_launch_action`] and a removal can edit this link: it is a link, and not the catalog,
    /// the `/Pages` node, a node under it or a page (found by number), that page-tree walk was not
    /// cut short, and the read was not `capped`.
    pub settable: bool,
    /// The [`MAX_GETS`] allowance was spent, so what follows the stop was not read.
    pub capped: bool,
}

impl Default for LaunchAction {
    fn default() -> Self {
        LaunchAction {
            is_link: false,
            has_dest: false,
            has_entry: false,
            present: false,
            has_next: false,
            has_platform: false,
            action: None,
            exact: true,
            is_launch: false,
            file_kind: None,
            file: None,
            url: false,
            new_window: None,
            settable: false,
            capped: false,
        }
    }
}

fn cut(s: &PdfString) -> (String, bool) {
    let mut chars = s.text_chars();
    let shown: String = chars.by_ref().take(MAX_FILE_BYTES).collect();
    (shown, chars.next().is_none())
}

fn text(doc: &Document, gets: &mut Gets, entry: Option<&Object>) -> Result<Option<(String, bool)>, Spent> {
    let Some(entry) = entry else { return Ok(None) };
    Ok(match &*gets.follow(doc, entry)? {
        Object::String(s) => Some(cut(s)),
        _ => None,
    })
}

fn not_null(d: &Dict, key: &[u8]) -> bool {
    d.get(key).is_some_and(|v| !matches!(v, Object::Null))
}

fn read(doc: &Document, gets: &mut Gets, link: ObjRef, out: &mut LaunchAction) -> Result<(), Spent> {
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
    out.has_platform = [b"Win".as_slice(), b"Mac", b"Unix"].iter().any(|k| not_null(action, k));
    let Some(s) = action.name(b"S") else { return Ok(()) };
    let (name, whole) = shown(s);
    out.action = Some(name);
    out.exact = whole;
    out.is_launch = s == b"Launch";
    if !out.is_launch {
        return Ok(());
    }
    if let Some(n) = action.get(b"NewWindow")
        && let Object::Bool(b) = &*gets.follow(doc, n)?
    {
        out.new_window = Some(*b);
    }
    let Some(f) = action.get(b"F") else { return Ok(()) };
    let f = gets.follow(doc, f)?;
    match &*f {
        Object::String(s) => {
            let (file, whole) = cut(s);
            out.file_kind = Some("string");
            out.file = Some(file);
            out.exact &= whole;
        }
        Object::Dict(spec) => {
            out.file_kind = Some("dictionary");
            let found = match text(doc, gets, spec.get(b"UF"))? {
                Some(t) => Some(t),
                None => text(doc, gets, spec.get(b"F"))?,
            };
            if let Some((file, whole)) = found {
                out.file = Some(file);
                out.exact &= whole;
            }
            if let Some(fs) = spec.get(b"FS") {
                out.url = matches!(&*gets.follow(doc, fs)?, Object::Name(n) if n == b"URL");
            }
        }
        _ => {}
    }
    Ok(())
}

/// The launch action of `link` as stored, with whether it can be edited. At most [`MAX_GETS`]
/// reads of its own, plus one walk of the page tree when the object is a link. An object that is
/// not a dictionary, or is missing, reads as no action and is not settable.
pub fn read_launch_action(doc: &Document, link: ObjRef) -> LaunchAction {
    read_with(doc, link, MAX_GETS)
}

pub(crate) fn read_with(doc: &Document, link: ObjRef, gets: usize) -> LaunchAction {
    let mut out = LaunchAction::default();
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

/// What [`set_launch_action`] would do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// It would write the link's dictionary.
    Yes,
    /// It would write nothing: a removal from a link that has no launch action.
    NoChange,
    /// It is refused, for this reason.
    Refused(&'static str),
}

/// What setting `target` (a removal when `None`) on `link` would do.
pub fn launch_action_change(doc: &Document, link: ObjRef, target: Option<&LaunchTarget>) -> Change {
    change_with(doc, link, target, MAX_GETS)
}

pub(crate) fn change_with(doc: &Document, link: ObjRef, target: Option<&LaunchTarget>, gets: usize) -> Change {
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
    if target.is_none() && !read.is_launch {
        return Change::NoChange;
    }
    Change::Yes
}

/// Set the action of `link` to a launch action opening `target` (replacing its `/A` and taking
/// `/Dest` out), or, when `target` is `None`, take `/A` out if it is a launch action. A removal
/// from a link with no launch action is no change.
pub fn set_launch_action(doc: &mut Document, link: ObjRef, target: Option<&LaunchTarget>) -> Result<(), OrganizeError> {
    match launch_action_change(doc, link, target) {
        Change::NoChange => return Ok(()),
        Change::Refused(why) => return Err(OrganizeError::Invalid(why.into())),
        Change::Yes => {}
    }
    doc.update_dict(link, |d| match target {
        Some(t) => {
            let mut a = Dict::new();
            a.set(b"Type".to_vec(), Object::name("Action"));
            a.set(b"S".to_vec(), Object::name("Launch"));
            a.set(b"F".to_vec(), Object::String(PdfString::literal(t.file().as_bytes().to_vec())));
            if let Some(w) = t.new_window() {
                a.set(b"NewWindow".to_vec(), Object::Bool(w));
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
