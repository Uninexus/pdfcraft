//! The go-to-an-embedded-file action of a link: an action dictionary `/S /GoToE` in the link
//! annotation's `/A` (ISO 32000-2, go-to-embedded actions). `/T` is a target dictionary that
//! says where the embedded file is (`/R` is `P` for the parent of this document or `C` for a child
//! of it, `/N` the name of the file in the document's `/EmbeddedFiles` name tree, `/P` and `/A` a
//! file attachment annotation, and a nested `/T` the target of the target); `/D` is the
//! destination in that file (a name, a string or an array whose first item is the page, a zero-based integer
//! page index, for a file other than this one); `/F` is an optional file specification that puts
//! the whole thing in another file; `/NewWindow` asks for a new window.
//!
//! This module reads the action of one link, sets one that goes to a page of a file attached to
//! this document ([`EmbeddedTarget`]) or takes one out. It does not run the action. A set is
//! deliberately narrower than a read: it writes `/T << /R /C /N (name) >>` and `/D [page-1 /Fit]` (a zero-based page index)
//! and nothing else (no `/F`, no `/P`, no `/A`, no nested `/T`, no named destination), for a name
//! that is made only of letters, digits, `.`, `_`, `-`, space and parentheses (the name `add_attachment` gives a second file of one name, `name (2)`) and is the key of an attachment of
//! this document at the time of the set (compared byte for byte with the raw keys of the
//! `/EmbeddedFiles` name tree, as a viewer looks a name up, so a key stored as UTF-16 is not found by an ASCII name); it does not check that the attachment is a PDF file or
//! that it has that many pages. A set replaces the link's whole `/A` with a new direct dictionary
//! (`/Type /Action`, `/S /GoToE`, `/D`, `/T`, and `/NewWindow` only when given) and takes `/Dest`
//! out, which a link with an action must not have; a `/Next` chain or any other entry of the old
//! action is not kept (a removal drops a `/Next` chain with the action), and an indirect old `/A` dictionary is left in the file unreferenced. A
//! removal takes `/A` out only when it is a go-to-embedded action; a link with no action, or
//! another kind of action, is left as it is.
//!
//! The link is given as an object, which the caller found as a link of a page; it must be a
//! dictionary whose `/Subtype` is the name `Link`, and must not be the catalog, the `/Pages` node,
//! a node under it or a page, compared by object number.
//!
//! Reads are charged against an allowance of [`MAX_GETS`] for each call of [`read_embedded_action`]
//! and [`embedded_action_change`]. Each read is paid for before `Document::get` is made, and the
//! call stops and says so when it cannot pay: the link (1), and each hop of the reference chain
//! behind `/A`, `/NewWindow`, `/F`, `/D`, the first item of a `/D` array, `/T`, and the `/R` and
//! `/N` of `/T` (as `Document::resolve` follows them: the first object and up to 32 more, so at
//! most 33 each), 1 + 8 * 33 = 265 in all. That is far below the allowance, which is a second
//! guard tested only with small ones. Not charged by this module: the page-tree walk that finds
//! the structural objects ([`structural`](crate::reference_xobjects), with its own allowance,
//! whose first read, of the catalog, is not charged; it is made once by each read of an object
//! that is a link, and 4 times by a set through the tool: its check, the extra read before the
//! edit, the applied edit and the answer) and, for a set, the listing of the attachment keys (its
//! own limits, in `crate::import::name_tree_entries`; a name tree past them lists only what was
//! read, so a set can refuse a name that is there).

use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString};

use crate::OrganizeError;
use crate::attachments::entries;
use crate::reference_xobjects::{Gets, Spent, structural};
use crate::role_map::shown;

/// The allowance of reads of each call that reads a link (the page-tree walk has its own).
pub const MAX_GETS: usize = 10_000;
/// The longest attachment name a set accepts, in bytes, and the most characters of a stored one a
/// read shows.
pub const MAX_NAME_BYTES: usize = 255;
/// The highest page a set accepts (1-based).
pub const MAX_PAGE: u32 = 1_000_000;

/// A page of an attached file to go to, checked by [`EmbeddedTarget::new`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedTarget {
    name: String,
    page: u32,
    new_window: Option<bool>,
}

impl EmbeddedTarget {
    /// The target for page `page` (1-based, 1 to [`MAX_PAGE`]) of the attachment `name`, which is
    /// 1 to [`MAX_NAME_BYTES`] bytes of letters, digits, `.`, `_`, `-`, space, `(` and `)`, with no space at
    /// either end, no leading `-`, and at least one letter or digit. `new_window` is written only
    /// when given. That the attachment exists is checked when the change is made.
    pub fn new(name: &str, page: u32, new_window: Option<bool>) -> Result<Self, OrganizeError> {
        let bad = |why: &str| Err(OrganizeError::Invalid(format!("an embedded file {why}")));
        if name.is_empty() || name.len() > MAX_NAME_BYTES {
            return bad(&format!("is named with 1 to {MAX_NAME_BYTES} bytes"));
        }
        if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b" ._-()".contains(&b)) {
            return bad("is named only with letters, digits, . _ - ( ) and space");
        }
        if name.starts_with(' ') || name.ends_with(' ') || name.starts_with('-') || !name.bytes().any(|b| b.is_ascii_alphanumeric()) {
            return bad("is named with no space at either end, no leading - and a letter or digit");
        }
        if page == 0 || page > MAX_PAGE {
            return Err(OrganizeError::Invalid(format!("a page of an embedded file is 1 to {MAX_PAGE}")));
        }
        Ok(EmbeddedTarget { name: name.to_owned(), page, new_window })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The page, 1-based.
    pub fn page(&self) -> u32 {
        self.page
    }

    pub fn new_window(&self) -> Option<bool> {
        self.new_window
    }
}

/// What [`read_embedded_action`] read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedAction {
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
    /// `/S`, the name, the relationship and a string `/F` are whole (none was cut).
    pub exact: bool,
    /// `/S` is the name `GoToE`.
    pub is_goto_embedded: bool,
    /// `/F` is a string (`"string"`) or a dictionary (`"dictionary"`) after references; `None`
    /// when it is missing or anything else. A dictionary's file name is not read.
    pub file_kind: Option<&'static str>,
    /// `/F` when it is a string, as text, cut at [`MAX_NAME_BYTES`] characters.
    pub file: Option<String>,
    /// `/D` is a name (`"name"`), a string (`"string"`) or an array (`"array"`) after references;
    /// `None` when it is missing or anything else.
    pub destination_kind: Option<&'static str>,
    /// The first item of an array `/D` when it is an integer page index, plus one (1-based). A
    /// page index below zero reads as none.
    pub destination_page: Option<i64>,
    /// `/NewWindow` when it is a boolean after references.
    pub new_window: Option<bool>,
    /// `/T` is a dictionary after references.
    pub has_target: bool,
    /// The `/R` of the target when it is a name (`P`, `C` or anything else, shown as it is).
    pub relationship: Option<String>,
    /// The `/N` of the target when it is a string, as text, cut at [`MAX_NAME_BYTES`] characters.
    pub embedded_file: Option<String>,
    /// The target has a `/P` or an `/A` (a file attachment annotation) whose stored value is not
    /// null (a reference counts as present; it is not followed). A set never writes them.
    pub has_annotation_target: bool,
    /// The target has a nested `/T` whose stored value is not null (not followed).
    pub has_nested_target: bool,
    /// [`set_embedded_action`] and a removal can edit this link: it is a link, and not the
    /// catalog, the `/Pages` node, a node under it or a page (found by number), that page-tree
    /// walk was not cut short, and the read was not `capped`.
    pub settable: bool,
    /// The [`MAX_GETS`] allowance was spent, so what follows the stop was not read.
    pub capped: bool,
}

impl Default for EmbeddedAction {
    fn default() -> Self {
        EmbeddedAction {
            is_link: false,
            has_dest: false,
            has_entry: false,
            present: false,
            has_next: false,
            action: None,
            exact: true,
            is_goto_embedded: false,
            file_kind: None,
            file: None,
            destination_kind: None,
            destination_page: None,
            new_window: None,
            has_target: false,
            relationship: None,
            embedded_file: None,
            has_annotation_target: false,
            has_nested_target: false,
            settable: false,
            capped: false,
        }
    }
}

fn cut(s: &PdfString) -> (String, bool) {
    let mut chars = s.text_chars();
    let shown: String = chars.by_ref().take(MAX_NAME_BYTES).collect();
    (shown, chars.next().is_none())
}

fn not_null(d: &Dict, key: &[u8]) -> bool {
    d.get(key).is_some_and(|v| !matches!(v, Object::Null))
}

fn read_target(doc: &Document, gets: &mut Gets, t: &Dict, out: &mut EmbeddedAction) -> Result<(), Spent> {
    out.has_annotation_target = not_null(t, b"P") || not_null(t, b"A");
    out.has_nested_target = not_null(t, b"T");
    if let Some(r) = t.get(b"R")
        && let Object::Name(n) = &*gets.follow(doc, r)?
    {
        let (name, whole) = shown(n);
        out.relationship = Some(name);
        out.exact &= whole;
    }
    if let Some(n) = t.get(b"N")
        && let Object::String(s) = &*gets.follow(doc, n)?
    {
        let (name, whole) = cut(s);
        out.embedded_file = Some(name);
        out.exact &= whole;
    }
    Ok(())
}

fn read(doc: &Document, gets: &mut Gets, link: ObjRef, out: &mut EmbeddedAction) -> Result<(), Spent> {
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
    out.is_goto_embedded = s == b"GoToE";
    if !out.is_goto_embedded {
        return Ok(());
    }
    if let Some(n) = action.get(b"NewWindow")
        && let Object::Bool(b) = &*gets.follow(doc, n)?
    {
        out.new_window = Some(*b);
    }
    if let Some(f) = action.get(b"F") {
        match &*gets.follow(doc, f)? {
            Object::String(s) => {
                let (file, whole) = cut(s);
                out.file_kind = Some("string");
                out.file = Some(file);
                out.exact &= whole;
            }
            Object::Dict(_) => out.file_kind = Some("dictionary"),
            _ => {}
        }
    }
    if let Some(dest) = action.get(b"D") {
        match &*gets.follow(doc, dest)? {
            Object::Name(_) => out.destination_kind = Some("name"),
            Object::String(_) => out.destination_kind = Some("string"),
            Object::Array(items) => {
                out.destination_kind = Some("array");
                if let Some(first) = items.first()
                    && let Object::Int(i) = &*gets.follow(doc, first)?
                {
                    out.destination_page = i.checked_add(1).filter(|p| *p >= 1);
                }
            }
            _ => {}
        }
    }
    if let Some(t) = action.get(b"T")
        && let Object::Dict(t) = &*gets.follow(doc, t)?
    {
        out.has_target = true;
        read_target(doc, gets, t, out)?;
    }
    Ok(())
}

/// The go-to-embedded action of `link` as stored, with whether it can be edited. At most
/// [`MAX_GETS`] reads of its own, plus one walk of the page tree when the object is a link. An
/// object that is not a dictionary, or is missing, reads as no action and is not settable.
pub fn read_embedded_action(doc: &Document, link: ObjRef) -> EmbeddedAction {
    read_with(doc, link, MAX_GETS)
}

pub(crate) fn read_with(doc: &Document, link: ObjRef, gets: usize) -> EmbeddedAction {
    let mut out = EmbeddedAction::default();
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

/// What [`set_embedded_action`] would do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// It would write the link's dictionary.
    Yes,
    /// It would write nothing: a removal from a link that has no go-to-embedded action.
    NoChange,
    /// It is refused, for this reason.
    Refused(&'static str),
}

/// What setting `target` (a removal when `None`) on `link` would do.
pub fn embedded_action_change(doc: &Document, link: ObjRef, target: Option<&EmbeddedTarget>) -> Change {
    change_with(doc, link, target, MAX_GETS)
}

pub(crate) fn change_with(doc: &Document, link: ObjRef, target: Option<&EmbeddedTarget>, gets: usize) -> Change {
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
    match target {
        None if !read.is_goto_embedded => Change::NoChange,
        Some(t) if !entries(doc).0.iter().any(|(k, _)| k.as_slice() == t.name().as_bytes()) => {
            Change::Refused("the document has no attachment with that name")
        }
        _ => Change::Yes,
    }
}

/// Set the action of `link` to a go-to-embedded action for `target` (replacing its `/A` and
/// taking `/Dest` out), or, when `target` is `None`, take `/A` out if it is one. A removal from a
/// link with no go-to-embedded action is no change.
pub fn set_embedded_action(doc: &mut Document, link: ObjRef, target: Option<&EmbeddedTarget>) -> Result<(), OrganizeError> {
    match embedded_action_change(doc, link, target) {
        Change::NoChange => return Ok(()),
        Change::Refused(why) => return Err(OrganizeError::Invalid(why.into())),
        Change::Yes => {}
    }
    doc.update_dict(link, |d| match target {
        Some(t) => {
            let mut to = Dict::new();
            to.set(b"R".to_vec(), Object::name("C"));
            to.set(b"N".to_vec(), Object::String(PdfString::literal(t.name().as_bytes().to_vec())));
            let page = Object::Int(i64::from(t.page()) - 1);
            let mut a = Dict::new();
            a.set(b"Type".to_vec(), Object::name("Action"));
            a.set(b"S".to_vec(), Object::name("GoToE"));
            a.set(b"D".to_vec(), Object::Array(vec![page, Object::name("Fit")]));
            a.set(b"T".to_vec(), Object::Dict(to));
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
