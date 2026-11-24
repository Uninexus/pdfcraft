//! What the AI assistant may change in a document, and how a change it asks for is carried out.
//!
//! The assistant (`pdfcraft-ai`'s `agent` module) answers an editing request with a list of calls,
//! `{tool, args}`. This module is the one place that says which calls exist ([`catalog`]), checks a
//! list against the document before anything runs ([`check`]) and runs it on a session ([`apply`]).
//! The desktop app and the `ai_edit_*` tools both use it, so they can never disagree.
//!
//! Every call becomes ordinary [`Edit`]s, so each is an undo step, and nothing is saved. Pages are
//! numbered from 1 in the arguments. The model never names a document, a file or a path: it only
//! changes the document it was asked about, and nothing here reads or writes a file.

use serde_json::Value;

use crate::{DocId, Edit, FieldValue, Session};

/// The most calls one list may hold.
pub const MAX_STEPS: usize = 60;
/// The most characters in a document-info value, a field value or added text.
const MAX_TEXT_CHARS: usize = 4_000;
/// The most characters in a field name.
const MAX_NAME_CHARS: usize = 256;
/// The document-info keys the assistant may set.
const INFO_KEYS: [&str; 4] = ["Title", "Author", "Subject", "Keywords"];

/// One call the assistant may make: its name, the arguments the model is shown, and what it does.
pub struct Action {
    pub name: &'static str,
    pub arguments: &'static str,
    pub what: &'static str,
    /// Whether it can remove content (shown to the user as a warning before the list is applied).
    pub removes: bool,
}

/// The calls the assistant may make, in the order the model is shown them.
pub const ACTIONS: &[Action] = &[
    Action {
        name: "rotate_pages",
        arguments: "degrees: 90|180|270|-90 (clockwise, any multiple of 90), pages?: [int] (default all), subset?: all|even|odd, orientation?: all|landscape|portrait",
        what: "Rotate pages. subset and orientation narrow the pages (for example only landscape pages).",
        removes: false,
    },
    Action { name: "delete_pages", arguments: "pages: [int]", what: "Delete pages. Not all of them.", removes: true },
    Action {
        name: "move_pages",
        arguments: "pages: [int], to: int",
        what: "Move pages so they come before the page now at position `to` (the page count + 1 moves them to the end).",
        removes: false,
    },
    Action { name: "reverse_pages", arguments: "pages?: [int]", what: "Reverse the order of the pages (default all).", removes: false },
    Action {
        name: "insert_blank_page",
        arguments: "at: int, width?: number, height?: number",
        what: "Insert a blank page so it becomes page `at`; size in points (default: the neighbouring page).",
        removes: false,
    },
    Action {
        name: "duplicate_pages",
        arguments: "pages: [int], at?: int",
        what: "Insert copies of pages before position `at` (default: right after the last of them).",
        removes: false,
    },
    Action {
        name: "set_document_info",
        arguments: "key: Title|Author|Subject|Keywords, value: string",
        what: "Set a document property.",
        removes: false,
    },
    Action {
        name: "fill_form_field",
        arguments: "name: string, value: string|boolean",
        what: "Fill a form field: text for a text field, true/false for a check box. Use the exact field name.",
        removes: false,
    },
    Action { name: "reset_form", arguments: "names?: [string]", what: "Clear form fields (default all) back to empty.", removes: true },
    Action {
        name: "add_text",
        arguments: "page: int, text: string, x: number, y: number, width?: number, size?: number",
        what: "Add text to a page with its top-left at (x, y) points from the page's top-left; size in points (default 12).",
        removes: false,
    },
];

/// The list of calls for the model's prompt: one line each.
pub fn catalog() -> String {
    ACTIONS.iter().map(|a| format!("- {}({}): {}", a.name, a.arguments, a.what)).collect::<Vec<_>>().join("\n")
}

/// Whether `tool` may remove content (a warning is shown for it).
pub fn removes_content(tool: &str) -> bool {
    ACTIONS.iter().any(|a| a.name == tool && a.removes)
}

/// How a page list reads to a person: `3-5, 8`.
pub fn page_list(pages: &[usize]) -> String {
    let mut sorted: Vec<usize> = pages.iter().map(|p| p + 1).collect();
    sorted.sort_unstable();
    sorted.dedup();
    let mut parts = Vec::new();
    let mut at = 0;
    while at < sorted.len() {
        let mut end = at;
        while end + 1 < sorted.len() && sorted[end + 1] == sorted[end] + 1 {
            end += 1;
        }
        parts.push(if end == at { sorted[at].to_string() } else { format!("{}-{}", sorted[at], sorted[end]) });
        at = end + 1;
    }
    parts.join(", ")
}

/// What a call needs to know about the document it will run on, so a list can be checked before
/// anything runs. `pages` is the count now; `sizes` is the `(width, height)` of each page.
#[derive(Clone, Debug)]
pub struct Shape {
    pub pages: usize,
    pub sizes: Vec<(f64, f64)>,
}

impl Shape {
    /// The shape of document `id`.
    pub fn of(session: &Session, id: DocId) -> Option<Shape> {
        let doc = session.get(id)?;
        let sizes = doc.info.pages.iter().map(|p| (f64::from(p.width), f64::from(p.height))).collect::<Vec<_>>();
        Some(Shape { pages: sizes.len(), sizes })
    }

    fn size_near(&self, at: usize) -> (f64, f64) {
        self.sizes.get(at.saturating_sub(1)).or(self.sizes.first()).copied().unwrap_or((612.0, 792.0))
    }
}

fn pages_arg(args: &Value, key: &str, shape: &Shape, default_all: bool) -> Result<Vec<usize>, String> {
    let Some(value) = args.get(key).filter(|v| !v.is_null()) else {
        return if default_all && shape.pages > 0 { Ok((0..shape.pages).collect()) } else { Err(format!("{key} is missing")) };
    };
    let list = value.as_array().ok_or_else(|| format!("{key} must be a list of page numbers"))?;
    if list.is_empty() {
        return Err(format!("{key} must name at least one page"));
    }
    let mut out = Vec::new();
    for v in list {
        let n = v.as_u64().ok_or_else(|| format!("{key} must be whole page numbers from 1"))?;
        if n < 1 || n > shape.pages as u64 {
            return Err(format!("page {n} does not exist: the document has {} page(s) at this point", shape.pages));
        }
        let page = n as usize - 1;
        if !out.contains(&page) {
            out.push(page);
        }
    }
    Ok(out)
}

fn position_arg(args: &Value, key: &str, shape: &Shape) -> Result<Option<usize>, String> {
    let Some(v) = args.get(key).filter(|v| !v.is_null()) else { return Ok(None) };
    let n = v.as_u64().ok_or_else(|| format!("{key} must be a whole number from 1"))?;
    if n < 1 || n > shape.pages as u64 + 1 {
        return Err(format!("{key} must be between 1 and {} (the document has {} page(s) at this point)", shape.pages + 1, shape.pages));
    }
    Ok(Some(n as usize - 1))
}

fn text_arg<'a>(args: &'a Value, key: &str, limit: usize) -> Result<&'a str, String> {
    let s = args.get(key).and_then(Value::as_str).ok_or_else(|| format!("{key} must be text"))?;
    if s.chars().count() > limit {
        return Err(format!("{key} is longer than {limit} characters"));
    }
    Ok(s)
}

fn number_arg(args: &Value, key: &str, range: std::ops::RangeInclusive<f64>) -> Result<Option<f64>, String> {
    let Some(v) = args.get(key).filter(|v| !v.is_null()) else { return Ok(None) };
    let n = v
        .as_f64()
        .filter(|n| n.is_finite() && range.contains(n))
        .ok_or_else(|| format!("{key} must be a number from {} to {}", range.start(), range.end()))?;
    Ok(Some(n))
}

/// Only the arguments a call is documented to take: a typo fails loudly, as it does for every tool.
fn only_keys(args: &Value, keys: &[&str]) -> Result<(), String> {
    let obj = args.as_object().ok_or_else(|| "arguments must be an object".to_owned())?;
    match obj.keys().find(|k| !keys.contains(&k.as_str())) {
        Some(k) => Err(format!("unknown argument {k:?}")),
        None => Ok(()),
    }
}

/// The edits one call stands for, checked against `shape` (the document as it will be when this
/// call runs), and the shape after them. `info` is the document's page list, when known, for the
/// rotation filters.
pub fn plan_step(tool: &str, args: &Value, shape: &Shape, info: Option<&pdfcraft_render::DocInfo>) -> Result<(Vec<Edit>, Shape), String> {
    let mut next = shape.clone();
    let edit = match tool {
        "rotate_pages" => {
            only_keys(args, &["degrees", "pages", "subset", "orientation"])?;
            let degrees = args.get("degrees").and_then(Value::as_i64).ok_or("degrees must be a whole number")?;
            if degrees == 0 || degrees % 90 != 0 || degrees.abs() > 3600 {
                return Err("degrees must be a non-zero multiple of 90".into());
            }
            let base = pages_arg(args, "pages", shape, true)?;
            let parity = match args.get("subset").and_then(Value::as_str).unwrap_or("all") {
                "all" => crate::PageParity::Both,
                "even" => crate::PageParity::Even,
                "odd" => crate::PageParity::Odd,
                s => return Err(format!("unknown subset {s:?} (all, even, odd)")),
            };
            let orientation = match args.get("orientation").and_then(Value::as_str).unwrap_or("all") {
                "all" => crate::PageOrientation::Both,
                "landscape" => crate::PageOrientation::Landscape,
                "portrait" => crate::PageOrientation::Portrait,
                o => return Err(format!("unknown orientation {o:?} (all, landscape, portrait)")),
            };
            // Without the page list (a check before anything runs) only the page numbers are checked.
            let pages = match info {
                Some(info) => crate::filter_pages(info, &base, parity, orientation),
                None => base,
            };
            if pages.is_empty() {
                return Err("no pages match the subset and orientation".into());
            }
            Edit::RotatePages { pages, degrees }
        }
        "delete_pages" => {
            only_keys(args, &["pages"])?;
            let pages = pages_arg(args, "pages", shape, false)?;
            if pages.len() >= shape.pages {
                return Err("a document cannot lose all of its pages".into());
            }
            next.pages -= pages.len();
            let mut kept = Vec::new();
            for (i, s) in shape.sizes.iter().enumerate() {
                if !pages.contains(&i) {
                    kept.push(*s);
                }
            }
            next.sizes = kept;
            Edit::DeletePages { pages }
        }
        "move_pages" => {
            only_keys(args, &["pages", "to"])?;
            let pages = pages_arg(args, "pages", shape, false)?;
            let to = position_arg(args, "to", shape)?.ok_or("to is missing")?;
            Edit::MovePages { pages, to }
        }
        "reverse_pages" => {
            only_keys(args, &["pages"])?;
            Edit::ReversePages { pages: pages_arg(args, "pages", shape, true)? }
        }
        "insert_blank_page" => {
            only_keys(args, &["at", "width", "height"])?;
            let at = position_arg(args, "at", shape)?.ok_or("at is missing")?;
            let (w, h) = shape.size_near(at);
            let width = number_arg(args, "width", 1.0..=14_400.0)?.unwrap_or(w);
            let height = number_arg(args, "height", 1.0..=14_400.0)?.unwrap_or(h);
            next.pages += 1;
            next.sizes.insert(at.min(next.sizes.len()), (width, height));
            Edit::InsertBlankPage { at, width, height }
        }
        "duplicate_pages" => {
            only_keys(args, &["pages", "at"])?;
            let pages = pages_arg(args, "pages", shape, false)?;
            let at = position_arg(args, "at", shape)?;
            next.pages += pages.len();
            let copies: Vec<(f64, f64)> = pages.iter().map(|p| shape.sizes.get(*p).copied().unwrap_or((612.0, 792.0))).collect();
            let place = at.unwrap_or_else(|| pages.iter().max().map_or(shape.pages, |m| m + 1)).min(next.sizes.len());
            for (i, s) in copies.into_iter().enumerate() {
                next.sizes.insert(place + i, s);
            }
            Edit::DuplicatePages { pages, at }
        }
        "set_document_info" => {
            only_keys(args, &["key", "value"])?;
            let key = text_arg(args, "key", 32)?;
            if !INFO_KEYS.contains(&key) {
                return Err(format!("key must be one of {}", INFO_KEYS.join(", ")));
            }
            Edit::SetInfo { key: key.to_owned(), value: text_arg(args, "value", MAX_TEXT_CHARS)?.to_owned() }
        }
        "fill_form_field" => {
            only_keys(args, &["name", "value"])?;
            let name = text_arg(args, "name", MAX_NAME_CHARS)?;
            if name.is_empty() {
                return Err("name is empty".into());
            }
            let value = match args.get("value") {
                Some(Value::Bool(b)) => FieldValue::Check(*b),
                Some(Value::String(s)) if s.chars().count() <= MAX_TEXT_CHARS => FieldValue::Text(s.clone()),
                Some(Value::String(_)) => return Err(format!("value is longer than {MAX_TEXT_CHARS} characters")),
                _ => return Err("value must be text or true/false".into()),
            };
            Edit::SetFieldValue { name: name.to_owned(), value }
        }
        "reset_form" => {
            only_keys(args, &["names"])?;
            let names = match args.get("names").filter(|v| !v.is_null()) {
                None => None,
                Some(v) => {
                    let list = v.as_array().ok_or("names must be a list of field names")?;
                    let mut out = Vec::new();
                    for n in list {
                        let n = n
                            .as_str()
                            .filter(|s| !s.is_empty() && s.chars().count() <= MAX_NAME_CHARS)
                            .ok_or("names must be non-empty field names")?;
                        out.push(n.to_owned());
                    }
                    if out.is_empty() {
                        return Err("names must name at least one field (leave it out for all)".into());
                    }
                    Some(out)
                }
            };
            Edit::ResetForm { names }
        }
        "add_text" => {
            only_keys(args, &["page", "text", "x", "y", "width", "size"])?;
            let page = pages_arg(&serde_json::json!({ "pages": [args.get("page").cloned().unwrap_or(Value::Null)] }), "pages", shape, false)
                .map_err(|e| e.replace("pages", "page"))?[0];
            let text = text_arg(args, "text", MAX_TEXT_CHARS)?;
            if text.trim().is_empty() {
                return Err("text is empty".into());
            }
            let (pw, ph) = shape.sizes.get(page).copied().unwrap_or((612.0, 792.0));
            let x = number_arg(args, "x", 0.0..=14_400.0)?.ok_or("x is missing")?;
            let y = number_arg(args, "y", 0.0..=14_400.0)?.ok_or("y is missing")?;
            let size = number_arg(args, "size", 1.0..=500.0)?.unwrap_or(12.0);
            let width = number_arg(args, "width", 1.0..=14_400.0)?.unwrap_or((pw - x).clamp(20.0, 200.0));
            // Display space has its origin at the top-left, like the arguments; the box is one line
            // tall to start with and grows with the text.
            let rect = [x, y, x + width, (y + size * 1.4).min(ph.max(y + size))];
            let mut added = crate::AddedText { rect, text: text.to_owned(), ..crate::AddedText::default() };
            added.size = size;
            Edit::AddText { page, text: added }
        }
        other => return Err(format!("{other} is not a tool you may call here; use only the tools listed")),
    };
    Ok((vec![edit], next))
}

/// Checks a whole list against `shape` without changing anything: every call must be known, fit its
/// arguments and the document as the calls before it leave it. Returns each call's one-line
/// description in English, in order.
pub fn check(shape: &Shape, calls: &[(String, Value)]) -> Result<Vec<String>, String> {
    if calls.len() > MAX_STEPS {
        return Err(format!("at most {MAX_STEPS} calls are allowed"));
    }
    let mut now = shape.clone();
    let mut out = Vec::new();
    for (i, (tool, args)) in calls.iter().enumerate() {
        let (_, next) = plan_step(tool, args, &now, None).map_err(|e| format!("step {} ({tool}): {e}", i + 1))?;
        out.push(describe(tool, args));
        now = next;
    }
    Ok(out)
}

/// One line saying what a call does, in English.
pub fn describe(tool: &str, args: &Value) -> String {
    let pages = |key: &str| -> String {
        let list: Vec<usize> =
            args.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).map(|n| n as usize - 1).collect()).unwrap_or_default();
        if list.is_empty() { "all pages".to_owned() } else { format!("page(s) {}", page_list(&list)) }
    };
    match tool {
        "rotate_pages" => format!("Rotate {} by {}°", pages("pages"), args.get("degrees").and_then(Value::as_i64).unwrap_or(0)),
        "delete_pages" => format!("Delete {}", pages("pages")),
        "move_pages" => format!("Move {} to position {}", pages("pages"), args.get("to").and_then(Value::as_u64).unwrap_or(0)),
        "reverse_pages" => format!("Reverse the order of {}", pages("pages")),
        "insert_blank_page" => format!("Insert a blank page as page {}", args.get("at").and_then(Value::as_u64).unwrap_or(0)),
        "duplicate_pages" => format!("Duplicate {}", pages("pages")),
        "set_document_info" => format!(
            "Set {} to \"{}\"",
            args.get("key").and_then(Value::as_str).unwrap_or(""),
            args.get("value").and_then(Value::as_str).unwrap_or("")
        ),
        "fill_form_field" => format!("Fill the field \"{}\"", args.get("name").and_then(Value::as_str).unwrap_or("")),
        "reset_form" => "Clear the form".to_owned(),
        "add_text" => format!("Add text to page {}", args.get("page").and_then(Value::as_u64).unwrap_or(0)),
        other => other.to_owned(),
    }
}

/// What [`apply`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    /// Calls that ran.
    pub steps: usize,
    /// How many times Undo takes the whole list back.
    pub undo_steps: usize,
}

fn undo_depth(session: &Session, id: DocId) -> usize {
    session.get(id).map_or(0, |d| d.undo_depth())
}

/// Runs `calls` on document `id`, in order, as ordinary undo steps; nothing is saved. The whole list
/// is checked first ([`check`]); if a call still fails when it runs, the ones before it are undone and
/// the document is as it was.
pub fn apply(session: &mut Session, id: DocId, calls: &[(String, Value)]) -> Result<Applied, String> {
    let shape = Shape::of(session, id).ok_or("no such document")?;
    check(&shape, calls)?;
    let mut undo_steps = 0usize;
    for (i, (tool, args)) in calls.iter().enumerate() {
        let before = undo_depth(session, id);
        let result = (|| {
            let now = Shape::of(session, id).ok_or_else(|| "the document is gone".to_owned())?;
            let edits = {
                let doc = session.get(id).ok_or_else(|| "the document is gone".to_owned())?;
                plan_step(tool, args, &now, Some(&doc.info))?.0
            };
            for edit in edits {
                session.apply(id, edit).map_err(|e| e.to_string())?;
            }
            Ok::<(), String>(())
        })();
        let added = undo_depth(session, id).saturating_sub(before);
        undo_steps += added;
        if let Err(e) = result {
            for _ in 0..undo_steps {
                let _ = session.undo(id);
            }
            return Err(format!("step {} ({tool}) failed: {e}. The {i} step(s) before it were undone; the document is as it was.", i + 1));
        }
    }
    Ok(Applied { steps: calls.len(), undo_steps })
}
