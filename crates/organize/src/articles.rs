//! Article threads: the catalog's `/Threads` array, one thread dictionary for each article, whose
//! `/F` is the first bead and whose `/I` is an optional information dictionary with the title
//! (ISO 32000-2 §12.4.3). The beads of a thread form a ring through `/N`; a bead names its page
//! in `/P` and its rectangle in `/R`.
//!
//! This module lists the threads and takes one away by its position, and does nothing else: it
//! adds no thread or bead, changes no title, and reads only the first bead's page, no rectangle. A removal
//! takes the thread's entry out of `/Threads` only: the thread dictionary, its beads and the
//! `/B` arrays of the pages that list those beads are left as they are, so the beads stay in the
//! file unreferenced by `/Threads`.
//!
//! References are followed as `Document::resolve` follows them for the `/Threads` array and for
//! each entry (the first object and up to 32 more). A bead is reached by `/F` and by `/N`, which
//! are read only when they are references (a direct dictionary is not a bead), and a ring of
//! beads is walked for at most [`MAX_BEADS_WALKED`] steps.

use pdfcraft_cos::{Dict, Document, ObjRef, Object};

use crate::OrganizeError;
use crate::print_presets::held;

/// Most threads listed (all are counted).
pub const MAX_LISTED_THREADS: usize = 256;
/// Most characters of a title listed; a longer title is cut and not `title_exact`.
pub const MAX_TITLE_CHARS: usize = 127;
/// Most beads walked along one thread: this module's limit. A thread with more is counted as
/// this many and `beads_capped`.
pub const MAX_BEADS_WALKED: usize = 4096;
/// References followed after the first object, as `Document::resolve` follows them.
const MORE_HOPS: usize = 32;

/// One entry of `/Threads`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thread {
    /// Position in `/Threads`, counting from 0 (what [`remove_article_thread`] takes).
    pub index: usize,
    /// The `/Title` of the thread's `/I` dictionary, when it is a string, cut at
    /// [`MAX_TITLE_CHARS`] characters.
    pub title: Option<String>,
    /// Whether `title` is whole.
    pub title_exact: bool,
    /// Beads reached from `/F` along `/N`, counting `/F`; 0 when `/F` is not a reference to a
    /// dictionary or the entry is not a dictionary.
    pub beads: usize,
    /// The walk stopped at [`MAX_BEADS_WALKED`] beads before the ring closed or ended.
    pub beads_capped: bool,
    /// The last bead's `/N` leads back to `/F`, as a thread's ring does.
    pub closed: bool,
    /// The 1-based page of the first bead (`/F`), when its `/P` is a reference to a page of this document.
    pub first_page: Option<usize>,
}

/// What [`read_article_threads`] read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Threads {
    /// The first [`MAX_LISTED_THREADS`] entries, in order.
    pub threads: Vec<Thread>,
    /// How many entries `/Threads` has.
    pub count: usize,
    /// More entries exist than were listed.
    pub truncated: bool,
    /// `threads` when the catalog's `/Threads` is there but is neither an array nor null, or is
    /// behind more than 33 references; then nothing is listed and nothing is written.
    pub unreadable: Vec<&'static str>,
}

/// Where `/Threads` is.
#[derive(Clone, Copy)]
enum At {
    /// No entry, a null, or a reference that ends in nothing.
    Absent,
    /// An array written in the catalog.
    Direct,
    /// An array object, through one reference or a chain of them (the last one).
    Indirect(ObjRef),
    /// Anything else.
    Other,
}

fn locate(doc: &Document, entry: Option<&Object>) -> At {
    let mut at = match entry {
        None | Some(Object::Null) => return At::Absent,
        Some(Object::Array(_)) => return At::Direct,
        Some(Object::Ref(r)) => *r,
        Some(_) => return At::Other,
    };
    for _ in 0..=MORE_HOPS {
        match &*doc.get(at) {
            Object::Array(_) => return At::Indirect(at),
            Object::Null => return At::Absent,
            Object::Ref(next) => at = *next,
            _ => return At::Other,
        }
    }
    At::Other
}

/// The threads array and where it is, from one read of the catalog.
fn with_threads<R>(doc: &Document, f: impl FnOnce(&At, Option<&[Object]>) -> R) -> R {
    let Some(catalog) = doc.root().map(|r| doc.get(r)) else { return f(&At::Absent, None) };
    let Some(entry) = catalog.as_dict().and_then(|c| c.get(b"Threads")) else { return f(&At::Absent, None) };
    let at = locate(doc, Some(entry));
    let array = held(doc, entry);
    f(&at, array.as_array().map(Vec::as_slice))
}

/// The beads of a thread dictionary: how many are reached from `/F` along `/N`, whether the walk
/// was cut at [`MAX_BEADS_WALKED`], and whether `/N` led back to `/F`.
fn walk_beads(doc: &Document, thread: &Dict) -> (usize, bool, bool) {
    let Some(first) = thread.get(b"F").and_then(|o| o.as_ref()) else { return (0, false, false) };
    let mut at = first;
    for count in 1..=MAX_BEADS_WALKED {
        let bead = doc.get(at);
        let Some(bead) = bead.as_dict() else { return (count - 1, false, false) };
        match bead.get(b"N").and_then(|o| o.as_ref()) {
            Some(next) if next == first => return (count, false, true),
            Some(next) => at = next,
            None => return (count, false, false),
        }
    }
    (MAX_BEADS_WALKED, true, false)
}

/// The 1-based page the first bead of `thread` names in its `/P`, when that is a page of the document. `pages` is the
/// page list, read the first time a thread needs it.
fn first_page(doc: &Document, thread: &Dict, pages: &mut Option<Vec<ObjRef>>) -> Option<usize> {
    let first = thread.get(b"F").and_then(|o| o.as_ref())?;
    let page = doc.get(first).as_dict().and_then(|bead| bead.get(b"P").and_then(|o| o.as_ref()))?;
    let all = pages.get_or_insert_with(|| crate::pages(doc).map(|p| p.into_iter().map(|p| p.obj).collect()).unwrap_or_default());
    all.iter().position(|p| *p == page).map(|i| i + 1)
}

fn title(doc: &Document, thread: &Dict) -> (Option<String>, bool) {
    let info = thread.get(b"I").map(|o| held(doc, o));
    let text = info.as_deref().and_then(Object::as_dict).and_then(|d| d.get(b"Title")).map(|o| held(doc, o));
    let Some(Object::String(s)) = text.as_deref() else { return (None, true) };
    let mut chars = s.text_chars();
    let shown: String = chars.by_ref().take(MAX_TITLE_CHARS).collect();
    (Some(shown), chars.next().is_none())
}

/// The article threads as stored. One read of the catalog.
pub fn read_article_threads(doc: &Document) -> Threads {
    with_threads(doc, |at, array| {
        let mut out = Threads::default();
        if matches!(at, At::Other) {
            out.unreadable.push("threads");
            return out;
        }
        let items = array.unwrap_or_default();
        out.count = items.len();
        out.truncated = items.len() > MAX_LISTED_THREADS;
        let mut pages: Option<Vec<ObjRef>> = None;
        for (index, item) in items.iter().enumerate().take(MAX_LISTED_THREADS) {
            let value = held(doc, item);
            let (title, title_exact, beads, first_page) = match value.as_dict() {
                Some(d) => {
                    let (t, whole) = title(doc, d);
                    (t, whole, walk_beads(doc, d), first_page(doc, d, &mut pages))
                }
                None => (None, true, (0, false, false), None),
            };
            out.threads.push(Thread { index, title, title_exact, beads: beads.0, beads_capped: beads.1, closed: beads.2, first_page });
        }
        out
    })
}

/// Whether [`remove_article_thread`] with this position would write: `/Threads` is a readable
/// array with an entry there.
pub fn article_thread_removes(doc: &Document, index: usize) -> bool {
    with_threads(doc, |at, array| !matches!(at, At::Other) && array.is_some_and(|a| index < a.len()))
}

/// Take the entry at `index` out of `/Threads`, and `/Threads` with it when that was the last
/// entry (an indirect array is taken out of the catalog and its object is left unreferenced).
/// The other entries keep their order. A position with no entry, and a `/Threads` that is absent
/// or unreadable, are no change. The thread's dictionary, its beads and the pages' `/B` arrays
/// are not touched.
pub fn remove_article_thread(doc: &mut Document, index: usize) -> Result<(), OrganizeError> {
    let root = doc.root().ok_or(OrganizeError::NoPageTree)?;
    let found = with_threads(doc, |at, array| match (at, array) {
        (At::Other, _) => None,
        (_, Some(a)) if index < a.len() => Some((*at, a.to_vec())),
        _ => None,
    });
    let Some((at, mut rest)) = found else { return Ok(()) };
    rest.remove(index);
    match at {
        At::Indirect(r) if !rest.is_empty() => doc.set_existing(r, Object::Array(rest))?,
        At::Direct if !rest.is_empty() => doc.update_dict(root, |c| c.set(b"Threads".to_vec(), Object::Array(rest)))?,
        _ => doc.update_dict(root, |c| {
            c.remove(b"Threads");
        })?,
    }
    Ok(())
}
