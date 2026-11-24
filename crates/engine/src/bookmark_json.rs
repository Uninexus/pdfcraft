//! The portable JSON form of a bookmark tree, shared by the `bookmark_export`, `bookmark_import` and
//! `bookmark_copy` tools and the Bookmarks panel's Export and Import.
//!
//! A file is `{"bookmarks": [node, …]}`, where a node is `title`, `page` (1-based, or `null` for one
//! that goes nowhere), `view` (`{"type", "values"}`: a destination type and its values in PDF user
//! space, as the file stores them), `color`, `bold`, `italic`, `open` and `children`. A file also
//! carries `"truncated": true` when the export left out bookmarks nested past the listing limit or a
//! title that did not fit the listing's title allowance (that node has an empty title). What export
//! writes, import takes back unchanged.
use crate::OutlineNode;
use serde_json::{Value, json};

/// Deepest nesting a bookmark tree may have (the listing reaches 33 levels).
const MAX_DEPTH: usize = 33;

/// The keys a node may have: the ones [`node_json`] writes.
const NODE_KEYS: [&str; 8] = ["title", "page", "view", "color", "bold", "italic", "open", "children"];

/// A node as JSON, leaving out what is at its default (no view, black, regular, no children).
pub fn node_json(n: &OutlineNode) -> Value {
    let mut v = json!({ "title": n.title, "page": n.page.map(|p| p.saturating_add(1)), "open": n.open });
    if let Some((kind, values)) = &n.view {
        v["view"] = json!({ "type": String::from_utf8_lossy(kind), "values": values });
    }
    if let Some(c) = n.color {
        v["color"] = json!(c);
    }
    if n.bold {
        v["bold"] = json!(true);
    }
    if n.italic {
        v["italic"] = json!(true);
    }
    if !n.children.is_empty() {
        v["children"] = json!(n.children.iter().map(node_json).collect::<Vec<_>>());
    }
    v
}

/// The bytes of a bookmark file: pretty JSON, with `"truncated": true` only when something was left out.
pub fn bookmark_file_bytes(nodes: &[OutlineNode], truncated: bool) -> Result<Vec<u8>, String> {
    let mut saved = json!({ "bookmarks": nodes.iter().map(node_json).collect::<Vec<_>>() });
    if truncated {
        saved["truncated"] = json!(true);
    }
    serde_json::to_vec_pretty(&saved).map_err(|e| e.to_string())
}

/// The bookmark list of a file's bytes, and whether the export that wrote it left bookmarks out. The
/// error says why the bytes are not a bookmark file.
pub fn parse_bookmark_file(bytes: &[u8]) -> Result<(Value, bool), String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|e| format!("not a bookmark file: {e}"))?;
    let list = value.get("bookmarks").ok_or_else(|| "not a bookmark file: it has no \"bookmarks\" list".to_string())?;
    Ok((list.clone(), value.get("truncated") == Some(&Value::Bool(true))))
}

/// The bookmarks of a file's bytes ([`parse_bookmark_file`], then [`nodes_of`]) and its truncated flag.
pub fn read_bookmark_file(bytes: &[u8]) -> Result<(Vec<OutlineNode>, bool), String> {
    let (list, truncated) = parse_bookmark_file(bytes)?;
    Ok((nodes_of(&list)?, truncated))
}

/// The nodes of a JSON list, checked whole: the first bad node refuses the lot and the message names it.
pub fn nodes_of(list: &Value) -> Result<Vec<OutlineNode>, String> {
    let mut count = 0;
    list_of(list, &[], &mut count)
}

/// The nodes of one level; `at` is the 1-based path of their parent, for messages.
fn list_of(list: &Value, at: &[usize], count: &mut usize) -> Result<Vec<OutlineNode>, String> {
    let items = list.as_array().ok_or_else(|| bad(at, "children must be a list of bookmarks"))?;
    if at.len() >= MAX_DEPTH && !items.is_empty() {
        return Err(bad(at, &format!("bookmarks may nest at most {MAX_DEPTH} levels deep")));
    }
    *count = count.saturating_add(items.len());
    if *count > crate::MAX_IMPORT_BOOKMARKS {
        return Err(format!("at most {} bookmarks can be imported at once", crate::MAX_IMPORT_BOOKMARKS));
    }
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let mut path = at.to_vec();
        path.push(i.saturating_add(1));
        out.push(node_of(item, &path, count)?);
    }
    Ok(out)
}

/// One node and, through [`list_of`], its children (recursion bounded by `MAX_DEPTH`).
fn node_of(v: &Value, at: &[usize], count: &mut usize) -> Result<OutlineNode, String> {
    let o = v.as_object().ok_or_else(|| bad(at, "must be an object"))?;
    if let Some(k) = o.keys().find(|k| !NODE_KEYS.contains(&k.as_str())) {
        return Err(bad(at, &format!("unknown key {k:?} (a bookmark has {})", NODE_KEYS.join(", "))));
    }
    let field = |k: &str| o.get(k).filter(|x| !x.is_null());
    // An empty title is kept: files have them, and export writes them as "".
    let title = field("title").and_then(Value::as_str).ok_or_else(|| bad(at, "needs a title"))?.to_string();
    let page = match field("page") {
        None => None,
        Some(p) => Some(p.as_u64().filter(|p| *p >= 1).and_then(|p| usize::try_from(p - 1).ok()).ok_or_else(|| bad(at, "page must be 1 or more"))?),
    };
    let flag = |k: &str| field(k).map_or(Ok(None), |x| x.as_bool().map(Some).ok_or_else(|| bad(at, &format!("{k} must be true or false"))));
    let color = match field("color") {
        None => None,
        Some(c) => {
            let rgb: Option<Vec<f32>> = c.as_array().and_then(|c| c.iter().map(|x| x.as_f64().map(|x| x as f32)).collect());
            match rgb.as_deref() {
                Some(&[r, g, b]) if [r, g, b].iter().all(|c| (0.0..=1.0).contains(c)) => Some([r, g, b]),
                _ => return Err(bad(at, "color must be [r, g, b], each from 0 to 1")),
            }
        }
    };
    let children = match field("children") {
        None => Vec::new(),
        Some(list) => list_of(list, at, count)?,
    };
    Ok(OutlineNode {
        title,
        page,
        view: field("view").map(|x| view_of(x, at)).transpose()?,
        color,
        italic: flag("italic")?.unwrap_or(false),
        bold: flag("bold")?.unwrap_or(false),
        open: flag("open")?.unwrap_or(true),
        children,
    })
}

/// `{"type": "XYZ", "values": [left, top, zoom]}`: a destination type with exactly its values.
fn view_of(v: &Value, at: &[usize]) -> Result<(Vec<u8>, Vec<Option<f64>>), String> {
    let kind = v.get("type").and_then(Value::as_str).ok_or_else(|| bad(at, "view needs a type such as XYZ or Fit"))?;
    let values = match v.get("values").filter(|x| !x.is_null()) {
        None => Vec::new(),
        Some(list) => {
            let list = list.as_array().filter(|l| l.len() <= 4).ok_or_else(|| bad(at, "view values must be a list of at most 4 numbers or nulls"))?;
            list.iter()
                .map(|x| match x {
                    Value::Null => Ok(None),
                    x => x.as_f64().map(Some).ok_or_else(|| bad(at, "view values must be numbers or null")),
                })
                .collect::<Result<_, String>>()?
        }
    };
    let arity = crate::outline_view_arity(kind.as_bytes()).ok_or_else(|| bad(at, VIEW_TYPES))?;
    if values.len() != arity {
        return Err(bad(at, &format!("a {kind} view takes {arity} values (numbers, or null to keep the reader's)")));
    }
    if kind == "FitR" && values.iter().any(Option::is_none) {
        return Err(bad(at, "a FitR view needs all four values: left, bottom, right, top"));
    }
    Ok((kind.as_bytes().to_vec(), values))
}

const VIEW_TYPES: &str = "view type must be XYZ (3 values: left, top, zoom), Fit, FitB (none), FitH, FitV, FitBH, FitBV (1) or FitR (4)";

fn bad(at: &[usize], what: &str) -> String {
    if at.is_empty() {
        return format!("bookmarks: {what}");
    }
    format!("bookmark {at:?}: {what}")
}
