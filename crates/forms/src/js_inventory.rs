//! Every JavaScript in a document, listed in one place and editable by id ("Edit all
//! JavaScripts"): the named document scripts, the catalog's open action and document actions,
//! field, link, annotation, page and bookmark actions. Scripts are untrusted document content:
//! this module only reads and rewrites them, it never runs one.
//!
//! An id is the route to the script's action dictionary: a root (`catalog`, or `obj:N` for an
//! indirect object) followed by dictionary keys (`/AA/K`) and array positions (`[2]`), with
//! indirect references followed transparently. Ids are stable while the structure around the
//! script is, and an id that no longer leads to a JavaScript action is refused.

use std::collections::HashSet;
use std::rc::Rc;

use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString};

use crate::{FormError, page_refs, script, script_budget};

/// Most scripts listed; the rest are left out and [`ScriptInventory::capped`] says so.
pub const MAX_SCRIPTS: usize = 5000;
/// Longest script listed or accepted, in bytes. A longer stored script is cut (and marked).
pub const MAX_SCRIPT_LEN: usize = 1 << 20;
/// Most actions read in one chain: along `/Next` links and through arrays of actions. The name
/// tree, the field tree, the outline and the pages have limits of their own.
const MAX_CHAIN: usize = 256;
/// Most code bytes held by one listing in all; scripts past it are listed with empty code,
/// marked truncated.
pub const MAX_TOTAL_CODE: usize = 16 << 20;
/// Most bytes of field names, document script names and bookmark titles one listing reads, and
/// (a second allowance of the same size) most bytes of them it copies into entries. The first
/// is spent by the stored length of each string read, before it is decoded (a field name by each
/// of its parts, every time the name is built); the second by the decoded length of each name
/// copied into an entry, before it is copied. A name that does not fit reads as absent and
/// [`ScriptInventory::names_left_out`] counts the entry.
pub const MAX_TOTAL_LABELS: usize = 16 << 20;
/// Most annotations (and pages) visited in all while listing page actions.
const MAX_VISITS: usize = 200_000;
/// Most name tree nodes, outline items and fields visited.
const MAX_NODES: usize = 100_000;
/// Deepest field or outline nesting followed.
const MAX_DEPTH: usize = 64;
/// Longest id accepted.
const MAX_ID_LEN: usize = 512;

/// What a script belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptKind {
    /// A named document-level script (`/Names /JavaScript`).
    Document,
    /// The catalog's `/OpenAction`.
    OpenAction,
    /// A document action (`/AA`: Will Close, Will Save, Did Save, Will Print, Did Print).
    DocumentAction,
    /// A form field's or widget's action (Mouse Up, Keystroke, Format, Validate, Calculate, ...).
    Field,
    /// A link annotation's action.
    Link,
    /// Another annotation's action.
    Annotation,
    /// A page's own open or close action.
    Page,
    /// A bookmark's action.
    Bookmark,
}

impl ScriptKind {
    /// Every kind, in listing order.
    pub const ALL: [ScriptKind; 8] = [
        ScriptKind::Document,
        ScriptKind::OpenAction,
        ScriptKind::DocumentAction,
        ScriptKind::Field,
        ScriptKind::Link,
        ScriptKind::Annotation,
        ScriptKind::Page,
        ScriptKind::Bookmark,
    ];

    /// The kind's name in tools.
    pub fn id(self) -> &'static str {
        match self {
            ScriptKind::Document => "document",
            ScriptKind::OpenAction => "open_action",
            ScriptKind::DocumentAction => "document_action",
            ScriptKind::Field => "field",
            ScriptKind::Link => "link",
            ScriptKind::Annotation => "annotation",
            ScriptKind::Page => "page",
            ScriptKind::Bookmark => "bookmark",
        }
    }

    /// The kind named `id`.
    pub fn from_id(id: &str) -> Option<ScriptKind> {
        Self::ALL.into_iter().find(|k| k.id() == id)
    }
}

/// One JavaScript of the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptEntry {
    /// Where to find it again; pass it to [`set_javascript`] or [`remove_javascript`].
    pub id: String,
    pub kind: ScriptKind,
    /// When it runs: "Will Save", "Calculate", "Mouse Up", "Page Open", "Activate", ...
    pub trigger: String,
    /// A field's full name. `None` for a script that is not a field's, or when the name did not
    /// fit in [`MAX_TOTAL_LABELS`] (see [`ScriptInventory::names_left_out`]).
    pub field: Option<String>,
    /// The 1-based page it is on or belongs to.
    pub page: Option<usize>,
    /// A document script's name, or a bookmark's title. `None` when the name did not fit in
    /// [`MAX_TOTAL_LABELS`] (see [`ScriptInventory::names_left_out`]).
    pub name: Option<String>,
    /// The source. `None` when the action has no readable text (no `/JS`, an undecodable
    /// stream, or the reading budget is spent).
    pub code: Option<String>,
    /// `code` is cut: the stored script was longer than [`MAX_SCRIPT_LEN`], or the listing's
    /// total of [`MAX_TOTAL_CODE`] bytes was spent (then `code` is empty).
    pub truncated: bool,
    /// A short hash of the stored code (64-bit FNV-1a, hex; not a security measure). Editing
    /// needs it as `expect`, so an id that came to lead to another script is refused.
    pub fingerprint: String,
}

/// Every script found, with whether the walk stopped at a limit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptInventory {
    pub entries: Vec<ScriptEntry>,
    /// A limit stopped the walk (more than [`MAX_SCRIPTS`] scripts, or the page annotation
    /// visit limit): some scripts may not be listed.
    pub capped: bool,
    /// How many listed scripts have `truncated` set.
    pub truncated: usize,
    /// How many listed scripts lack the field name, script name or bookmark title they have,
    /// because it was past the [`MAX_TOTAL_LABELS`] allowances. Their code and ids are listed.
    pub names_left_out: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Seg {
    Key(String),
    Idx(usize),
}

#[derive(Clone, Debug)]
struct Route {
    root: Root,
    segs: Vec<Seg>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Root {
    Catalog,
    Obj(u32),
}

impl Route {
    fn catalog() -> Self {
        Route { root: Root::Catalog, segs: Vec::new() }
    }
    fn obj(num: u32) -> Self {
        Route { root: Root::Obj(num), segs: Vec::new() }
    }
    fn key(&self, k: &str) -> Self {
        let mut r = self.clone();
        r.segs.push(Seg::Key(k.to_string()));
        r
    }
    fn idx(&self, i: usize) -> Self {
        let mut r = self.clone();
        r.segs.push(Seg::Idx(i));
        r
    }
    fn id(&self) -> String {
        let mut s = match self.root {
            Root::Catalog => "catalog".to_string(),
            Root::Obj(n) => format!("obj:{n}"),
        };
        for seg in &self.segs {
            match seg {
                Seg::Key(k) => {
                    s.push('/');
                    s.push_str(k);
                }
                Seg::Idx(i) => s.push_str(&format!("[{i}]")),
            }
        }
        s
    }
    fn parse(id: &str) -> Option<Route> {
        if id.len() > MAX_ID_LEN {
            return None;
        }
        let end = id.find(['/', '[']).unwrap_or(id.len());
        let root = match id.get(..end)? {
            "catalog" => Root::Catalog,
            r => Root::Obj(r.strip_prefix("obj:")?.parse().ok()?),
        };
        let mut rest = id.get(end..)?;
        let mut segs = Vec::new();
        while !rest.is_empty() {
            if let Some(r) = rest.strip_prefix('/') {
                let e = r.find(['/', '[']).unwrap_or(r.len());
                let key = r.get(..e)?;
                if key.is_empty() || !key.bytes().all(|b| b.is_ascii_alphanumeric()) {
                    return None;
                }
                segs.push(Seg::Key(key.to_string()));
                rest = r.get(e..)?;
            } else {
                let r = rest.strip_prefix('[')?;
                let e = r.find(']')?;
                segs.push(Seg::Idx(r.get(..e)?.parse().ok()?));
                rest = r.get(e + 1..)?;
            }
        }
        Some(Route { root, segs })
    }
    fn start(&self, doc: &Document) -> Option<Object> {
        match self.root {
            Root::Catalog => doc.root().map(Object::Ref),
            Root::Obj(n) => Some(Object::Ref(ObjRef::new(n, doc.generation(n)))),
        }
    }
}

fn is_javascript(d: &Dict) -> bool {
    d.name(b"S") == Some(b"JavaScript".as_slice())
}

/// The fingerprint of script text (`None`: no readable text): 64-bit FNV-1a over its bytes.
/// It only tells one script from another; it is not a security measure.
pub(crate) fn fingerprint(code: Option<&str>) -> String {
    let Some(code) = code else { return "unreadable".into() };
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in code.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// The object at `segs` below `cur`, references followed. `None` when the route is broken or
/// passes through the same object twice.
fn follow(doc: &Document, cur: &Object, segs: &[Seg]) -> Option<Object> {
    let mut cur = cur.clone();
    let mut passed: Vec<u32> = Vec::new();
    for seg in segs {
        if let Object::Ref(r) = &cur {
            if passed.contains(&r.num) {
                return None;
            }
            passed.push(r.num);
        }
        let r = doc.resolve(&cur);
        cur = match (seg, &*r) {
            (Seg::Key(k), Object::Dict(d)) => d.get(k.as_bytes())?.clone(),
            (Seg::Idx(i), Object::Array(a)) => a.get(*i)?.clone(),
            _ => return None,
        };
    }
    if let Object::Ref(r) = &cur
        && passed.contains(&r.num)
    {
        return None;
    }
    Some(cur)
}

/// What the listing has read of one script object, kept so actions sharing it read it once.
#[derive(Clone)]
struct Read {
    text: std::rc::Rc<String>,
    fingerprint: std::rc::Rc<str>,
}

struct Collector<'a> {
    doc: &'a Document,
    out: ScriptInventory,
    budget: pdfcraft_cos::DecodeBudget,
    seen_ids: HashSet<String>,
    seen_actions: HashSet<u32>,
    /// Code bytes handed out so far, against [`MAX_TOTAL_CODE`].
    used: usize,
    /// Scripts read, by the object number of an indirect `/JS`; bounded like the decode budget.
    read: std::collections::HashMap<u32, Option<Read>>,
    read_bytes: usize,
    /// Annotations and pages visited, against [`MAX_VISITS`].
    visits: usize,
    seen_annots: HashSet<u32>,
    /// Name bytes still to be read and still to be copied ([`MAX_TOTAL_LABELS`]).
    read_left: usize,
    copy_left: usize,
}

#[derive(Clone, Default)]
struct Ctx {
    field: Option<Rc<str>>,
    page: Option<usize>,
    name: Option<Rc<str>>,
    /// A name of this script was past the reading allowance, so it is absent.
    left_out: bool,
}

impl Collector<'_> {
    fn full(&self) -> bool {
        self.out.capped
    }

    /// The JavaScript actions at `value` (an action, or an array of them) and along their
    /// `/Next` chains, each listed once.
    fn chain(&mut self, route: Route, value: &Object, kind: ScriptKind, trigger: &str, ctx: &Ctx) {
        let mut pending = vec![(route, value.clone())];
        let mut visited = 0usize;
        while let Some((route, raw)) = pending.pop() {
            visited += 1;
            if visited > MAX_CHAIN || self.full() {
                return;
            }
            if let Some(r) = raw.as_ref()
                && !self.seen_actions.insert(r.num)
            {
                continue;
            }
            let action = self.doc.resolve(&raw);
            if let Some(list) = action.as_array() {
                for (i, item) in list.iter().enumerate().take(MAX_CHAIN).rev() {
                    pending.push((route.idx(i), item.clone()));
                }
                continue;
            }
            let Some(d) = action.as_dict() else { continue };
            if is_javascript(d) {
                self.emit(&route, &raw, kind, trigger, ctx);
            }
            if let Some(next) = d.get(b"Next") {
                pending.push((route.key("Next"), next.clone()));
            }
        }
    }

    fn emit(&mut self, route: &Route, action: &Object, kind: ScriptKind, trigger: &str, ctx: &Ctx) {
        let id = route.id();
        if !self.seen_ids.insert(id.clone()) {
            return;
        }
        if self.out.entries.len() >= MAX_SCRIPTS {
            self.out.capped = true;
            return;
        }
        let read = self.read_code(action);
        let (mut code, mut truncated) = (None, false);
        if let Some(r) = &read {
            // Cut before copying: only what fits is ever duplicated.
            let room = MAX_SCRIPT_LEN.min(MAX_TOTAL_CODE.saturating_sub(self.used));
            let mut cut = r.text.len().min(room);
            while !r.text.is_char_boundary(cut) {
                cut -= 1;
            }
            truncated = cut < r.text.len();
            self.used += cut;
            code = r.text.get(..cut).map(str::to_string);
        }
        if truncated {
            self.out.truncated += 1;
        }
        let fingerprint = read.as_ref().map_or_else(|| fingerprint(None), |r| r.fingerprint.to_string());
        let mut left_out = ctx.left_out;
        let field = self.copied(ctx.field.as_deref(), &mut left_out);
        let name = self.copied(ctx.name.as_deref(), &mut left_out);
        if left_out {
            self.out.names_left_out += 1;
        }
        self.out.entries.push(ScriptEntry { id, kind, trigger: trigger.to_string(), field, page: ctx.page, name, code, truncated, fingerprint });
    }

    /// `text` as an owned string when it fits in what is left to copy, charged before it is
    /// copied; else `None`, and `left_out` is set.
    fn copied(&mut self, text: Option<&str>, left_out: &mut bool) -> Option<String> {
        let text = text?;
        if text.len() > self.copy_left {
            *left_out = true;
            return None;
        }
        self.copy_left -= text.len();
        Some(text.to_string())
    }

    /// A name decoded from `s`, when its stored length fits in what is left to read (charged
    /// before it is decoded); else `None`, and `left_out` is set.
    fn read_string(&mut self, s: &PdfString, left_out: &mut bool) -> Option<Rc<str>> {
        if s.bytes.len() > self.read_left {
            *left_out = true;
            return None;
        }
        self.read_left -= s.bytes.len();
        Some(Rc::from(s.to_text()))
    }

    /// The fully qualified name of the field `r`, as `qualified_name` builds it but with each
    /// part charged by its stored length before it is decoded. `None` when it has no name or
    /// reading it is not allowed; `left_out` is set in the second case.
    fn field_name(&mut self, r: ObjRef, left_out: &mut bool) -> Option<Rc<str>> {
        let doc = self.doc;
        let mut parts: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        let mut cur = Some(r);
        while let Some(c) = cur {
            if seen.len() > 64 || !seen.insert(c) {
                break;
            }
            let obj = doc.resolve(&Object::Ref(c));
            let d = obj.as_dict()?;
            if let Some(t) = d.get(b"T") {
                match &*held(doc, t) {
                    Object::String(s) => {
                        if s.bytes.len() > self.read_left {
                            *left_out = true;
                            return None;
                        }
                        self.read_left -= s.bytes.len();
                        parts.push(s.to_text());
                    }
                    Object::Name(n) => {
                        if n.len() > self.read_left {
                            *left_out = true;
                            return None;
                        }
                        self.read_left -= n.len();
                        parts.push(crate::name_text(n));
                    }
                    _ => {}
                }
            }
            cur = d.reference(b"Parent").filter(|p| doc.get(*p).as_dict().is_some());
        }
        parts.reverse();
        (!parts.is_empty()).then(|| Rc::from(parts.join(".")))
    }

    /// The text of the JavaScript action `action`, read once per script object.
    fn read_code(&mut self, action: &Object) -> Option<Read> {
        let doc = self.doc;
        let a = doc.resolve(action);
        let js = a.as_dict()?.get(b"JS")?.clone();
        let num = js.as_ref().map(|r| r.num);
        if let Some(hit) = num.and_then(|n| self.read.get(&n)) {
            return hit.clone();
        }
        let text = match &*doc.resolve(&js) {
            Object::String(s) => Some(s.to_text()),
            Object::Stream(s) => self.budget.decode(s).ok().map(|b| String::from_utf8_lossy(&b).into_owned()),
            _ => None,
        };
        let read = text.map(|t| Read { fingerprint: fingerprint(Some(&t)).into(), text: std::rc::Rc::new(t) });
        if let (Some(n), Some(r)) = (num, &read) {
            // Kept for the actions sharing it, within the same bound as stream decoding.
            if self.read_bytes.saturating_add(r.text.len()) <= crate::read::MAX_SCRIPT_BYTES {
                self.read_bytes += r.text.len();
                self.read.insert(n, read.clone());
            }
        }
        read
    }

    /// `/A` and the `/AA` entries of the dictionary at `route`.
    fn actions_of(&mut self, route: &Route, dict: &Dict, kind: ScriptKind, ctx: &Ctx, with_a: Option<&str>, aa_keys: &[(&str, &str)]) {
        if let (Some(trigger), Some(a)) = (with_a, dict.get(b"A")) {
            self.chain(route.key("A"), a, kind, trigger, ctx);
        }
        let Some(aa_raw) = dict.get(b"AA") else { return };
        let aa = self.doc.resolve(aa_raw);
        let Some(aa) = aa.as_dict() else { return };
        let aa_route = route.key("AA");
        for (key, trigger) in aa_keys {
            if let Some(a) = aa.get(key.as_bytes()) {
                self.chain(aa_route.key(key), a, kind, trigger, ctx);
            }
        }
    }

    fn named(&mut self) {
        let doc = self.doc;
        let Some(root) = doc.root() else { return };
        let route = Route::catalog().key("Names").key("JavaScript");
        let Some(tree) = follow(doc, &Object::Ref(root), &route.segs) else { return };
        let mut seen: HashSet<u32> = tree.as_ref().map(|r| r.num).into_iter().collect();
        let mut names_read: HashSet<u32> = HashSet::new();
        // An indirect /Kids array shared by several nodes is read once.
        let mut kids_read: HashSet<u32> = HashSet::new();
        let mut stack = vec![(route, tree)];
        let mut visited = 0usize;
        while let Some((route, raw)) = stack.pop() {
            visited += 1;
            if visited > MAX_NODES || self.full() {
                return;
            }
            let node = doc.resolve(&raw);
            let Some(d) = node.as_dict() else { continue };
            if let Some(kids) = d.get(b"Kids")
                && kids.as_ref().is_none_or(|r| kids_read.insert(r.num))
            {
                let kids_route = route.key("Kids");
                if let Some(list) = doc.resolve(kids).as_array().cloned() {
                    #[cfg(test)]
                    KIDS_READ.with(|n| n.set(n.get().saturating_add(list.len())));
                    let new: Vec<(Route, Object)> = list
                        .iter()
                        .enumerate()
                        .take(MAX_NODES)
                        .filter(|(_, k)| k.as_ref().is_some_and(|r| seen.insert(r.num)))
                        .map(|(i, k)| (kids_route.idx(i), k.clone()))
                        .collect();
                    stack.extend(new.into_iter().rev());
                }
            }
            if let Some(pairs) = d.get(b"Names")
                && pairs.as_ref().is_none_or(|r| names_read.insert(r.num))
                && let Some(list) = doc.resolve(pairs).as_array().cloned()
            {
                let names_route = route.key("Names");
                for (i, pair) in list.chunks(2).enumerate() {
                    let [k, v] = pair else { continue };
                    let Object::String(name) = &*held(doc, k) else { continue };
                    let mut left_out = false;
                    let name = self.read_string(name, &mut left_out);
                    let ctx = Ctx { name, left_out, ..Ctx::default() };
                    self.chain(names_route.idx(i * 2 + 1), v, ScriptKind::Document, "Document script", &ctx);
                    if self.full() {
                        return;
                    }
                }
            }
        }
    }

    fn catalog(&mut self) {
        let doc = self.doc;
        let Some(root) = doc.root() else { return };
        self.named();
        let cat = doc.get(root);
        let Some(cat) = cat.as_dict() else { return };
        if let Some(open) = cat.get(b"OpenAction") {
            self.chain(Route::catalog().key("OpenAction"), open, ScriptKind::OpenAction, "Document open", &Ctx::default());
        }
        let keys = [("WC", "Will Close"), ("WS", "Will Save"), ("DS", "Did Save"), ("WP", "Will Print"), ("DP", "Did Print")];
        self.actions_of(&Route::catalog(), cat, ScriptKind::DocumentAction, &Ctx::default(), None, &keys);
    }

    fn fields(&mut self) {
        let doc = self.doc;
        let Some(root) = doc.root() else { return };
        let cat = doc.get(root);
        let Some(form) = cat.as_dict().and_then(|c| c.get(b"AcroForm")).map(|f| doc.resolve(f)) else { return };
        let Some(top) = form.as_dict().and_then(|f| f.get(b"Fields")).map(|f| doc.resolve(f)) else { return };
        let Some(top) = top.as_array() else { return };
        let mut seen: HashSet<u32> = HashSet::new();
        // An indirect /Kids array shared by several fields is read once.
        let mut kids_read: HashSet<u32> = HashSet::new();
        let mut stack: Vec<(ObjRef, usize)> = top.iter().filter_map(Object::as_ref).map(|r| (r, 0)).collect();
        stack.reverse();
        let mut visited = 0usize;
        while let Some((r, depth)) = stack.pop() {
            visited += 1;
            if visited > MAX_NODES || self.full() {
                return;
            }
            if !seen.insert(r.num) {
                continue;
            }
            let obj = doc.get(r);
            let Some(d) = obj.as_dict() else { continue };
            // The name matters only to a field that has actions.
            if d.get(b"A").is_some() || d.get(b"AA").is_some() {
                let mut left_out = false;
                let field = self.field_name(r, &mut left_out);
                let ctx = Ctx { field, left_out, ..Ctx::default() };
                self.actions_of(&Route::obj(r.num), d, ScriptKind::Field, &ctx, Some("Mouse Up"), FIELD_AA);
            }
            if depth < MAX_DEPTH
                && d.get(b"Kids").is_none_or(|k| k.as_ref().is_none_or(|r| kids_read.insert(r.num)))
                && let Some(kids) = d.get(b"Kids").map(|k| doc.resolve(k))
                && let Some(kids) = kids.as_array()
            {
                #[cfg(test)]
                KIDS_READ.with(|n| n.set(n.get().saturating_add(kids.len())));
                stack.extend(kids.iter().take(MAX_NODES).filter_map(Object::as_ref).map(|k| (k, depth + 1)).rev().collect::<Vec<_>>());
            }
        }
    }

    fn visit(&mut self) -> bool {
        self.visits += 1;
        if self.visits > MAX_VISITS {
            self.out.capped = true;
            return false;
        }
        true
    }

    fn pages(&mut self) {
        let doc = self.doc;
        for (n, page) in page_refs(doc).into_iter().enumerate() {
            if self.full() || !self.visit() {
                return;
            }
            let obj = doc.get(page);
            let Some(pd) = obj.as_dict() else { continue };
            let page_route = Route::obj(page.num);
            let ctx = Ctx { page: Some(n + 1), ..Ctx::default() };
            self.actions_of(&page_route, pd, ScriptKind::Page, &ctx, None, &[("O", "Page Open"), ("C", "Page Close")]);
            let raw = pd.get(b"Annots");
            // An /Annots array shared by many pages is read once.
            if let Some(r) = raw.and_then(Object::as_ref)
                && !self.seen_annots.insert(r.num | SHARED_ARRAY)
            {
                continue;
            }
            let annots = raw.map(|a| doc.resolve(a));
            let Some(annots) = annots.as_deref().and_then(Object::as_array) else { continue };
            for (i, item) in annots.iter().enumerate() {
                if self.full() || !self.visit() {
                    return;
                }
                if let Some(r) = item.as_ref()
                    && !self.seen_annots.insert(r.num)
                {
                    continue;
                }
                let route = match item {
                    Object::Ref(r) => Route::obj(r.num),
                    _ => page_route.key("Annots").idx(i),
                };
                let annot = doc.resolve(item);
                let Some(ad) = annot.as_dict() else { continue };
                match ad.name(b"Subtype") {
                    Some(b"Widget") => {
                        if ad.get(b"A").is_some() || ad.get(b"AA").is_some() {
                            let mut left_out = false;
                            let field = item.as_ref().and_then(|r| self.field_name(r, &mut left_out));
                            let ctx = Ctx { field, page: Some(n + 1), name: None, left_out };
                            self.actions_of(&route, ad, ScriptKind::Field, &ctx, Some("Mouse Up"), FIELD_AA);
                        }
                    }
                    Some(b"Link") => self.actions_of(&route, ad, ScriptKind::Link, &ctx, Some("Activate"), FIELD_AA),
                    _ => self.actions_of(&route, ad, ScriptKind::Annotation, &ctx, Some("Activate"), FIELD_AA),
                }
            }
        }
    }

    fn bookmarks(&mut self) {
        let doc = self.doc;
        let Some(root) = doc.root() else { return };
        let cat = doc.get(root);
        let Some(outlines) = cat.as_dict().and_then(|c| c.get(b"Outlines")).and_then(Object::as_ref) else { return };
        let Some(first) = doc.get(outlines).as_dict().and_then(|o| o.get(b"First")).and_then(Object::as_ref) else { return };
        let mut seen: HashSet<u32> = HashSet::new();
        // (item, depth); siblings follow /Next, children /First.
        let mut stack = vec![(first, 0usize)];
        let mut visited = 0usize;
        while let Some((r, depth)) = stack.pop() {
            visited += 1;
            if visited > MAX_NODES || self.full() {
                return;
            }
            if !seen.insert(r.num) {
                continue;
            }
            let obj = doc.get(r);
            let Some(d) = obj.as_dict() else { continue };
            if let Some(next) = d.get(b"Next").and_then(Object::as_ref) {
                stack.push((next, depth));
            }
            if depth < MAX_DEPTH
                && let Some(child) = d.get(b"First").and_then(Object::as_ref)
            {
                stack.push((child, depth + 1));
            }
            if let Some(a) = d.get(b"A") {
                let mut left_out = false;
                let title = match d.get(b"Title").map(|t| held(doc, t)).as_deref() {
                    Some(Object::String(s)) => self.read_string(s, &mut left_out),
                    _ => None,
                };
                let ctx = Ctx { name: title, left_out, ..Ctx::default() };
                self.chain(Route::obj(r.num).key("A"), a, ScriptKind::Bookmark, "Activate", &ctx);
            }
        }
    }
}

/// An object as it is read: borrowed when it is direct, so a long string written in place is not
/// copied by being looked at, and held through its reference when it is indirect.
enum Held<'a> {
    Direct(&'a Object),
    Indirect(std::sync::Arc<Object>),
}

impl std::ops::Deref for Held<'_> {
    type Target = Object;
    fn deref(&self) -> &Object {
        match self {
            Held::Direct(o) => o,
            Held::Indirect(o) => o,
        }
    }
}

/// `o` itself when it is direct, else what its reference leads to (`Document::resolve` copies a
/// direct object, string bytes and all).
fn held<'a>(doc: &Document, o: &'a Object) -> Held<'a> {
    if matches!(o, Object::Ref(_)) {
        Held::Indirect(doc.resolve(o))
    } else {
        #[cfg(test)]
        BORROWED.with(|n| n.set(n.get().saturating_add(1)));
        Held::Direct(o)
    }
}

#[cfg(test)]
thread_local! {
    /// Direct objects looked at without a copy by listings on this thread.
    pub(crate) static BORROWED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Entries of /Kids arrays read by listings on this thread, for tests of the sharing rule.
    pub(crate) static KIDS_READ: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Marks an /Annots array's object number in the seen set, apart from annotations'.
const SHARED_ARRAY: u32 = 1 << 31;

/// Annotation and field additional actions, in the order they are listed.
const FIELD_AA: &[(&str, &str)] = &[
    ("E", "Mouse Enter"),
    ("X", "Mouse Exit"),
    ("D", "Mouse Down"),
    ("U", "Mouse Up"),
    ("Fo", "Receive Focus"),
    ("Bl", "Lose Focus"),
    ("PO", "Page Open"),
    ("PC", "Page Close"),
    ("PV", "Page Visible"),
    ("PI", "Page Invisible"),
    ("K", "Keystroke"),
    ("F", "Format"),
    ("V", "Validate"),
    ("C", "Calculate"),
];

/// Every JavaScript of the document: document scripts, the open action and document actions,
/// then field, page, annotation and bookmark actions. Bounded: at most [`MAX_SCRIPTS`] scripts,
/// every walk keeps a seen set and a depth or length limit, and an action reached twice (cyclic
/// or shared) is listed once.
pub fn javascripts(doc: &Document) -> ScriptInventory {
    let mut c = Collector {
        doc,
        out: ScriptInventory::default(),
        budget: script_budget(),
        seen_ids: HashSet::new(),
        seen_actions: HashSet::new(),
        used: 0,
        read: Default::default(),
        read_bytes: 0,
        visits: 0,
        seen_annots: HashSet::new(),
        read_left: MAX_TOTAL_LABELS,
        copy_left: MAX_TOTAL_LABELS,
    };
    c.catalog();
    // Pages first, so a widget's script carries its page; the field tree then adds the rest.
    c.pages();
    c.fields();
    c.bookmarks();
    c.out
}

fn stale(id: &str) -> FormError {
    FormError::Invalid(format!(
        "no JavaScript is stored at {id:?}: it was moved or removed, or the id is not one from the document's JavaScript list; list the JavaScripts again and use an id from that list"
    ))
}

fn changed() -> FormError {
    FormError::Invalid("the script at this id changed; list the JavaScripts again".into())
}

/// What to do at the end of a route.
#[derive(Clone, Copy)]
enum Op<'a> {
    Set(&'a str),
    Remove,
    /// Put this object in the slot (used to write a repaired name tree back).
    Replace(&'a Object),
}

/// The action `action` with `/JS` set to `code`.
fn with_code(action: &Object, code: &str) -> Result<Object, FormError> {
    let Object::Dict(d) = action else { return Err(FormError::Invalid("the script's action is not a dictionary".into())) };
    let mut d = d.clone();
    d.set(b"JS".to_vec(), PdfString::text(code));
    Ok(Object::Dict(d))
}

/// Whether `new` is `old` with its last entry taken out and a direct array left empty (a key
/// holding `[]` goes). The name tree's own arrays keep their keys.
fn emptied(old: &Object, new: &Object, key: Option<&str>) -> bool {
    matches!((old, new), (Object::Array(o), Object::Array(n)) if !o.is_empty() && n.is_empty()) && !matches!(key, Some("Names" | "Kids"))
}

/// Apply `op` below `cur`, following `segs`. Returns what replaces `cur` in its slot (`None`: the
/// slot goes; the same object: nothing changed). Only objects that change are written. An
/// indirect action that is set is copied, so another place sharing it keeps its script.
/// `parent_key` is the dictionary key `cur` was reached by, with indirect references between
/// not counting; `trail` holds the objects passed, so a route through one twice is refused.
fn apply(
    doc: &mut Document,
    cur: &Object,
    segs: &[Seg],
    op: Op<'_>,
    parent_key: Option<&str>,
    trail: &mut Vec<u32>,
) -> Result<Option<Object>, FormError> {
    if let Object::Ref(r) = cur {
        if trail.contains(&r.num) {
            return Err(FormError::Invalid("this id passes through the same object twice, so it cannot be edited safely".into()));
        }
        trail.push(r.num);
        let inner = (*doc.get(*r)).clone();
        if segs.is_empty() {
            return match op {
                Op::Remove => Ok(splice(&inner)),
                Op::Set(code) => {
                    let new = with_code(&inner, code)?;
                    Ok(Some(if new == inner { cur.clone() } else { Object::Ref(doc.add(new)) }))
                }
                Op::Replace(obj) => {
                    if &inner != obj {
                        doc.set(*r, obj.clone());
                    }
                    Ok(Some(cur.clone()))
                }
            };
        }
        return match apply(doc, &inner, segs, op, parent_key, trail)? {
            Some(new) if emptied(&inner, &new, parent_key) => Ok(None),
            Some(new) => {
                if new != inner {
                    doc.set(*r, new);
                }
                Ok(Some(cur.clone()))
            }
            None => Ok(None),
        };
    }
    let Some((seg, rest)) = segs.split_first() else {
        return Ok(match op {
            Op::Set(code) => Some(with_code(cur, code)?),
            Op::Remove => splice(cur),
            Op::Replace(obj) => Some(obj.clone()),
        });
    };
    match (seg, cur) {
        (Seg::Key(k), Object::Dict(d)) => {
            let child = d.get(k.as_bytes()).ok_or_else(|| stale("that location"))?.clone();
            let mut nd = d.clone();
            match apply(doc, &child, rest, op, Some(k), trail)? {
                Some(v) if v == child => return Ok(Some(cur.clone())),
                Some(v) if emptied(&child, &v, Some(k)) => {
                    nd.remove(k.as_bytes());
                }
                Some(v) => nd.set(k.as_bytes().to_vec(), v),
                None => {
                    nd.remove(k.as_bytes());
                }
            }
            Ok(Some(Object::Dict(nd)))
        }
        (Seg::Idx(i), Object::Array(a)) => {
            let child = a.get(*i).ok_or_else(|| stale("array position"))?.clone();
            let mut na = a.clone();
            // Directly under `Names`, an odd position is the action of a name.
            let named = rest.is_empty() && parent_key == Some("Names") && i % 2 == 1;
            match apply(doc, &child, rest, op, None, trail)? {
                Some(v) if v == child => return Ok(Some(cur.clone())),
                Some(v) => {
                    if let Some(slot) = na.get_mut(*i) {
                        *slot = v;
                    }
                }
                // A name with nothing chained after its script goes with it.
                None if named => {
                    na.drain(i - 1..=*i);
                }
                None => {
                    na.remove(*i);
                }
            }
            Ok(Some(Object::Array(na)))
        }
        _ => Err(stale("path")),
    }
}

/// What takes the place of a removed action: the one chained after it, if any.
fn splice(action: &Object) -> Option<Object> {
    action.as_dict().and_then(|d| d.get(b"Next")).cloned()
}

fn locate(doc: &Document, id: &str, expect: &str) -> Result<(Route, Object), FormError> {
    let route = Route::parse(id).ok_or_else(|| stale(id))?;
    let start = route.start(doc).ok_or_else(|| stale(id))?;
    let target = follow(doc, &start, &route.segs).ok_or_else(|| stale(id))?;
    if !doc.resolve(&target).as_dict().is_some_and(is_javascript) {
        return Err(stale(id));
    }
    let code = script(doc, &target, &mut script_budget());
    if fingerprint(code.as_deref()) != expect {
        return Err(changed());
    }
    Ok((route, start))
}

/// Replace the source of the JavaScript at `id` (from [`javascripts`]) with `code`; every other
/// key of the action, and everything around it, is left as it is. `expect` is the script's
/// `fingerprint` from that list: if the script at `id` is not that one any more (the list is
/// stale), nothing is changed. An action shared with other places is copied, so only this one
/// changes.
pub fn set_javascript(doc: &mut Document, id: &str, expect: &str, code: &str) -> Result<(), FormError> {
    if code.len() > MAX_SCRIPT_LEN {
        return Err(FormError::Invalid(format!("a script may be at most {MAX_SCRIPT_LEN} bytes; this one is {} bytes", code.len())));
    }
    let (route, start) = locate(doc, id, expect)?;
    if route.segs.is_empty() {
        // The route names the action object itself: there is no slot to point elsewhere.
        let Object::Ref(r) = start else { return Err(stale(id)) };
        let new = with_code(&doc.get(r), code)?;
        doc.set(r, new);
        return Ok(());
    }
    apply(doc, &start, &route.segs, Op::Set(code), None, &mut Vec::new())?;
    Ok(())
}

/// Remove the JavaScript at `id`: its slot goes (an action key, or a name's entry), and an
/// action chained after it with `/Next` takes its place (a name then points at that action).
/// `expect` is as for [`set_javascript`]. After a name is removed the name tree's `/Limits` are
/// made right and a leaf left empty is dropped.
pub fn remove_javascript(doc: &mut Document, id: &str, expect: &str) -> Result<(), FormError> {
    let (route, start) = locate(doc, id, expect)?;
    if route.segs.is_empty() {
        return Err(FormError::Invalid("this JavaScript is not held in an action slot that can be removed".into()));
    }
    apply(doc, &start, &route.segs, Op::Remove, None, &mut Vec::new())?;
    if id.starts_with("catalog/Names/JavaScript") {
        repair_name_tree(doc)?;
    }
    Ok(())
}

/// The name tree after names were removed: each node's `/Limits` match its names, emptied
/// leaves leave their parent's `/Kids`, and a root left with nothing keeps an empty `/Names`.
fn repair_name_tree(doc: &mut Document) -> Result<(), FormError> {
    let Some(root) = doc.root() else { return Ok(()) };
    let start = Object::Ref(root);
    let segs = [Seg::Key("Names".into()), Seg::Key("JavaScript".into())];
    let Some(tree) = follow(doc, &start, &segs) else { return Ok(()) };
    let mut seen = HashSet::new();
    let (new, _, _) = repair_node(doc, &tree, true, 0, &mut seen)?;
    if new != tree {
        apply(doc, &start, &segs, Op::Replace(&new), None, &mut Vec::new())?;
    }
    Ok(())
}

type Bounds = (Vec<u8>, Vec<u8>);

fn key_bytes(doc: &Document, k: &Object) -> Option<Vec<u8>> {
    match &*doc.resolve(k) {
        Object::String(s) => Some(s.bytes.clone()),
        _ => None,
    }
}

fn widen(into: &mut Option<Bounds>, low: Vec<u8>, high: Vec<u8>) {
    match into {
        Some((l, h)) => {
            if low < *l {
                *l = low;
            }
            if high > *h {
                *h = high;
            }
        }
        None => *into = Some((low, high)),
    }
}

/// Repair the node `node`: its new value, the least and greatest name below it, and whether
/// nothing is left in it. An indirect node is written here; a direct one is returned.
fn repair_node(
    doc: &mut Document,
    node: &Object,
    root: bool,
    depth: usize,
    seen: &mut HashSet<u32>,
) -> Result<(Object, Option<Bounds>, bool), FormError> {
    if let Object::Ref(r) = node {
        if !seen.insert(r.num) || depth > MAX_DEPTH {
            return Ok((node.clone(), None, false));
        }
        let inner = (*doc.get(*r)).clone();
        let (new, bounds, empty) = repair_dict(doc, &inner, root, depth, seen)?;
        if new != inner {
            doc.set(*r, new);
        }
        return Ok((node.clone(), bounds, empty));
    }
    repair_dict(doc, node, root, depth, seen)
}

fn repair_dict(
    doc: &mut Document,
    node: &Object,
    root: bool,
    depth: usize,
    seen: &mut HashSet<u32>,
) -> Result<(Object, Option<Bounds>, bool), FormError> {
    let Object::Dict(d) = node else { return Ok((node.clone(), None, false)) };
    let mut nd = d.clone();
    let mut bounds: Option<Bounds> = None;
    let mut kids_left = false;
    if let Some(kids) = d.get(b"Kids") {
        let list = doc.resolve(kids).as_array().cloned().unwrap_or_default();
        let mut kept = Vec::with_capacity(list.len());
        for kid in &list {
            let (new, b, empty) = repair_node(doc, kid, false, depth + 1, seen)?;
            if empty {
                continue;
            }
            if let Some((l, h)) = b {
                widen(&mut bounds, l, h);
            }
            kept.push(new);
        }
        kids_left = !kept.is_empty();
        if kept != list {
            match kids {
                Object::Ref(r) => doc.set(*r, Object::Array(kept)),
                _ => nd.set(b"Kids".to_vec(), Object::Array(kept)),
            }
        }
    }
    let pairs = d.get(b"Names").map(|n| doc.resolve(n)).and_then(|n| n.as_array().cloned()).unwrap_or_default();
    if let (Some(first), Some(last)) = (pairs.first(), pairs.chunks(2).last().and_then(<[Object]>::first))
        && let (Some(l), Some(h)) = (key_bytes(doc, first), key_bytes(doc, last))
    {
        widen(&mut bounds, l, h);
    }
    let empty = !kids_left && pairs.is_empty();
    if root {
        if d.contains(b"Kids") && !kids_left {
            nd.remove(b"Kids");
            if !d.contains(b"Names") {
                nd.set(b"Names".to_vec(), Object::Array(Vec::new()));
            }
        }
    } else if let Some((l, h)) = &bounds {
        let limits = Object::Array(vec![
            Object::String(PdfString { bytes: l.clone(), hex: false }),
            Object::String(PdfString { bytes: h.clone(), hex: false }),
        ]);
        if d.get(b"Limits") != Some(&limits) {
            nd.set(b"Limits".to_vec(), limits);
        }
    }
    let new = Object::Dict(nd);
    Ok((if &new == node { node.clone() } else { new }, bounds, empty))
}
