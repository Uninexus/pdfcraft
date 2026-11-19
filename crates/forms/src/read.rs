use super::tab_order::rank_tabs;
use super::{DecodeBudget, Dict, Document, Field, FieldKind, ObjRef, Object, PdfString, Widget, af, appearance, flags, name_text, text_of};

/// At most this many `/State` entries of a set-layer-visibility action are read.
const MAX_LAYER_STATE: usize = 1024;

/// A push button's mouse-up action (`/A`, or `/AA /U`, on the field or its widget).
fn button_action(doc: &Document, d: &Dict, widgets: &[ObjRef], scripts: &mut DecodeBudget) -> Option<af::ButtonAction> {
    // A button PdfCraft generated for an XFA image field (`/PCXfaImage true`, no PDF action):
    // clicking it picks a picture, as `buttonImportIcon()` does.
    if matches!(d.get(b"PCXfaImage").map(|o| doc.resolve(o)).as_deref(), Some(Object::Bool(true))) {
        return Some(af::ButtonAction::ImportIcon);
    }
    let pick = |dict: &Dict| -> Option<Object> {
        dict.get(b"A").cloned().or_else(|| dict.get(b"AA").map(|a| doc.resolve(a)).and_then(|a| a.as_dict().and_then(|aa| aa.get(b"U").cloned())))
    };
    let action = pick(d).or_else(|| widgets.iter().find_map(|w| doc.get(*w).as_dict().and_then(pick)))?;
    let a = doc.resolve(&action);
    let a = a.as_dict()?;
    let s = |k: &[u8]| a.get(k).and_then(|v| text_of(&doc.resolve(v)));
    Some(match a.name(b"S")? {
        b"ResetForm" => {
            let fields = a.get(b"Fields").map(|f| action_fields(doc, f)).unwrap_or_else(|| Some(Vec::new()))?;
            let exclude = a.get(b"Flags").and_then(|f| doc.resolve(f).as_int()).unwrap_or(0) & 1 != 0;
            af::ButtonAction::Reset { fields, exclude }
        }
        b"Named" => af::ButtonAction::Named(String::from_utf8_lossy(a.name(b"N")?).into_owned()),
        b"URI" => af::ButtonAction::Uri(s(b"URI")?),
        b"SubmitForm" => af::ButtonAction::Submit(
            a.get(b"F")
                .map(|f| doc.resolve(f))
                .and_then(|f| f.as_dict().and_then(|fd| fd.get(b"F").and_then(|x| text_of(&doc.resolve(x)))).or_else(|| text_of(&f)))
                .unwrap_or_default(),
        ),
        b"GoTo" => {
            let dest = doc.resolve(a.get(b"D")?);
            let target = dest.as_array()?.first()?.as_ref()?;
            af::ButtonAction::GoTo(page_refs(doc).iter().position(|p| *p == target)?)
        }
        b"Hide" => {
            let fields = a.get(b"T").map(|t| action_fields(doc, t)).unwrap_or_else(|| Some(Vec::new()))?;
            let hide = !a.get(b"H").is_some_and(|h| matches!(&*doc.resolve(h), Object::Bool(false)));
            af::ButtonAction::ShowHide { fields, hide }
        }
        b"SetOCGState" => {
            // /State: ON, OFF or Toggle, each followed by the groups it applies to.
            let mut changes = Vec::new();
            if let Some(state) = a.get(b"State") {
                let mut op = None;
                for item in doc.resolve(state).as_array().into_iter().flatten().take(MAX_LAYER_STATE) {
                    match item {
                        Object::Name(n) => {
                            op = match n.as_slice() {
                                b"ON" => Some(af::LayerOp::On),
                                b"OFF" => Some(af::LayerOp::Off),
                                b"Toggle" => Some(af::LayerOp::Toggle),
                                _ => None,
                            }
                        }
                        Object::Ref(r) => changes.extend(op.map(|op| (op, (r.num, r.generation)))),
                        _ => {}
                    }
                }
            }
            let preserve_rb = !a.get(b"PreserveRB").is_some_and(|p| matches!(&*doc.resolve(p), Object::Bool(false)));
            af::ButtonAction::SetLayers { changes, preserve_rb }
        }
        b"JavaScript" => af::button_script(&script(doc, &action, scripts)?),
        _ => return None,
    })
}

/// The option text (export values and display texts, in bytes) all the fields of one form may
/// hold when read: entries are cheap to repeat, and each can name one long string object.
pub const MAX_OPTION_BYTES: usize = 16 << 20;

/// A field's options (export value, display text) from its `/Opt` entries, spent from the form's
/// allowances; `None`, spending nothing, when they do not fit what is left.
/// A list refused after it was scanned: the byte allowance and the list's entries are spent, so a
/// field sharing the list is refused at once instead of scanning it again (entries that are no
/// text cost no bytes, and would otherwise be scanned in full every time).
fn refused(seen: &mut Seen, entries: usize) -> Option<Vec<(String, String)>> {
    seen.option_bytes_left = 0;
    seen.option_entries_left = seen.option_entries_left.saturating_sub(entries);
    None
}

fn read_options(doc: &Document, entries: &[Object], seen: &mut Seen) -> Option<Vec<(String, String)>> {
    if entries.len() > seen.option_entries_left {
        return None;
    }
    // The raw length of a text object, known before it is decoded.
    let raw = |o: &Object| match o {
        Object::String(s) => s.bytes.len(),
        Object::Name(n) => n.len(),
        _ => 0,
    };
    let left = seen.option_bytes_left;
    let mut bytes = 0usize;
    let mut out = Vec::with_capacity(entries.len().min(4096));
    for o in entries {
        let resolved = doc.resolve(o);
        let halves = match &*resolved {
            Object::Array(pair) if pair.len() >= 2 => Some((doc.resolve(&pair[0]), doc.resolve(&pair[1]))),
            _ => None,
        };
        let (first, second): (&Object, Option<&Object>) = match &halves {
            Some((a, b)) => (&**a, Some(&**b)),
            None => (&*resolved, None),
        };
        // A text object longer than what is left is refused before it is decoded, and a refusal
        // spends the rest of the allowance, so fields naming the same long list do not decode it
        // again.
        let raw_len = raw(first).saturating_add(second.map_or(0, raw));
        if raw_len > left.saturating_sub(bytes) {
            return refused(seen, entries.len());
        }
        let pair = match second {
            Some(second) => text_of(first).zip(text_of(second)),
            None => text_of(first).map(|t| (t.clone(), t)),
        };
        if let Some((export, shown)) = pair {
            bytes = bytes.saturating_add(export.len()).saturating_add(shown.len());
            if bytes > left {
                return refused(seen, entries.len());
            }
            out.push((export, shown));
        }
    }
    seen.option_entries_left -= entries.len();
    seen.option_bytes_left -= bytes;
    Some(out)
}

/// The `/Opt` entries all the fields of one form may hold when read. Fields can name one shared
/// array, so what a form may spend is not bounded by its size.
pub const MAX_OPTION_ENTRIES: usize = 1_000_000;

/// Most entries of a `ResetForm` `/Fields` or `Hide` `/T` array that [`action_fields`] reads.
const MAX_ACTION_FIELDS: usize = 65_536;
/// Most bytes of field names [`action_fields`] builds in all, over every entry. A name is the
/// partial names of up to 64 ancestors joined, and entries can share those ancestors, so the names
/// can be far larger than the array that lists them: each of a few thousand entries naming a
/// widget under the same chain of long partial names would otherwise build its own copy.
const MAX_ACTION_FIELD_BYTES: usize = 4 << 20;

/// The field names targeted by a `ResetForm` (`/Fields`) or `Hide` (`/T`) action entry: a field
/// name string, an indirect field or widget dictionary reference, or an array of them. `None`
/// when the array has more than [`MAX_ACTION_FIELDS`] entries or the names total more than
/// [`MAX_ACTION_FIELD_BYTES`]: a shortened list would reset or hide the wrong fields (a reset
/// that excludes the listed ones would reset more), so the action is not read at all.
pub(crate) fn action_fields(doc: &Document, o: &Object) -> Option<Vec<String>> {
    let target = |item: &Object| match item {
        Object::Ref(r) => qualified_name(doc, *r).or_else(|| text_of(&doc.resolve(item))),
        other => text_of(other),
    };
    let mut bytes = 0usize;
    let mut out = Vec::new();
    let mut take = |name: Option<String>| -> Option<()> {
        if let Some(name) = name {
            bytes = bytes.checked_add(name.len()).filter(|b| *b <= MAX_ACTION_FIELD_BYTES)?;
            out.push(name);
        }
        Some(())
    };
    match &*doc.resolve(o) {
        Object::Array(items) => {
            if items.len() > MAX_ACTION_FIELDS {
                return None;
            }
            for item in items {
                take(target(item))?;
            }
        }
        _ => take(target(o))?,
    }
    Some(out)
}

/// The fully qualified name of the field that `r` (a field or one of its widgets) belongs to,
/// from the partial names (`/T`) up its `/Parent` chain.
pub(crate) fn qualified_name(doc: &Document, r: ObjRef) -> Option<String> {
    let mut parts = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut cur = Some(r);
    while let Some(c) = cur {
        if seen.len() > 64 || !seen.insert(c) {
            break;
        }
        // An object that is only a reference to another is followed, as `Document::resolve` does.
        let obj = doc.resolve(&Object::Ref(c));
        let d = obj.as_dict()?;
        if let Some(t) = d.get(b"T").and_then(|o| text_of(&doc.resolve(o))) {
            parts.push(t);
        }
        cur = parent_of(doc, d);
    }
    parts.reverse();
    (!parts.is_empty()).then(|| parts.join("."))
}

/// The field `d` names as its `/Parent`, when that is a dictionary. A reference to an object the
/// file doesn't hold reads as `null`, and a `null` entry is the same as an absent one (ISO
/// 32000-2 §7.3.9): the field is then a root, as if it named no parent.
fn parent_of(doc: &Document, d: &Dict) -> Option<ObjRef> {
    d.reference(b"Parent").filter(|p| doc.get(*p).as_dict().is_some())
}

/// The fields of the form's tree whose `/Kids` list one of `kids` and whose own fully qualified
/// name is `name`: a field listing kids that don't name it as their `/Parent`, and so share its
/// name rather than extend it. Each node is visited once, at most 64 levels down.
pub(crate) fn listing_fields_named(doc: &Document, name: &str, kids: &[ObjRef]) -> Vec<ObjRef> {
    let mut out = Vec::new();
    let Some(af) = acroform(doc) else { return out };
    let top = af.get(b"Fields").map(|f| doc.resolve(f)).and_then(|f| f.as_array().cloned()).unwrap_or_default();
    let mut stack: Vec<(ObjRef, usize)> = top.iter().filter_map(Object::as_ref).map(|r| (r, 0)).collect();
    let mut seen = std::collections::HashSet::new();
    while let Some((r, depth)) = stack.pop() {
        if depth > 64 || !seen.insert(r) {
            continue;
        }
        let obj = doc.get(r);
        let Some(d) = obj.as_dict() else { continue };
        let list: Vec<ObjRef> = d
            .get(b"Kids")
            .map(|k| doc.resolve(k))
            .and_then(|k| k.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(Object::as_ref)
            .collect();
        if list.iter().any(|k| kids.contains(k)) && qualified_name(doc, r).as_deref() == Some(name) {
            out.push(r);
        }
        let next = depth.saturating_add(1);
        stack.extend(list.into_iter().filter(|k| doc.get(*k).as_dict().is_some_and(|kd| kd.contains(b"T"))).map(|k| (k, next)));
    }
    out
}

/// JavaScript streams one reading of the form (its fields, or its document scripts) may decode
/// in all, however many actions name the same stream.
pub(crate) const MAX_SCRIPT_BYTES: usize = 64 << 20;

/// A budget for the script streams of one reading of the form.
pub(crate) fn script_budget() -> DecodeBudget {
    DecodeBudget::new(MAX_SCRIPT_BYTES, 0)
}

/// The JavaScript of an action dictionary (`/JS` string or stream). A stream is decoded against
/// `budget`, and is `None` once it is spent.
pub(crate) fn script(doc: &Document, action: &Object, budget: &mut DecodeBudget) -> Option<String> {
    let a = doc.resolve(action);
    let js = a.as_dict()?.get(b"JS")?.clone();
    match &*doc.resolve(&js) {
        Object::String(s) => Some(s.to_text()),
        Object::Stream(s) => budget.decode(s).ok().map(|b| String::from_utf8_lossy(&b).into_owned()),
        _ => None,
    }
}

/// A field's format/keystroke, validate and calculate scripts (field dictionary, or its only
/// widget's when the field has none).
fn actions_of(doc: &Document, d: &Dict, widgets: &[ObjRef], scripts: &mut DecodeBudget) -> af::Actions {
    let aa = d.get(b"AA").map(|a| doc.resolve(a)).and_then(|a| a.as_dict().cloned()).or_else(|| {
        (widgets.len() == 1)
            .then(|| doc.get(widgets[0]))
            .and_then(|w| w.as_dict().and_then(|wd| wd.get(b"AA").map(|a| doc.resolve(a))))
            .and_then(|a| a.as_dict().cloned())
    });
    let mut out = af::Actions {
        // A date field the XFA layout generated: what its format doesn't read is kept as typed.
        xfa_date: matches!(d.get(b"PCXfaDate").map(|o| doc.resolve(o)).as_deref(), Some(Object::Bool(true))),
        ..af::Actions::default()
    };
    let Some(aa) = aa else { return out };
    let mut js = |k: &[u8]| aa.get(k).and_then(|a| script(doc, a, scripts));
    if let Some(f) = js(b"F") {
        match af::parse_format(&f) {
            Some(fm) => out.format = fm,
            None if !f.trim().is_empty() => {
                out.unsupported.push("format");
                out.scripts.format = Some(f);
            }
            None => {}
        }
    }
    if let Some(k) = js(b"K")
        && out.format == af::Format::None
    {
        // A mask has only a keystroke script.
        match af::parse_format(&k) {
            Some(fm @ af::Format::Mask(_)) => out.format = fm,
            Some(_) => {}
            None if !k.trim().is_empty() => {
                out.unsupported.push("keystroke");
                out.scripts.keystroke = Some(k);
            }
            None => {}
        }
    }
    if let Some(v) = js(b"V") {
        match af::parse_validate(&v) {
            Some(vv) => out.validate = vv,
            None if !v.trim().is_empty() => {
                out.unsupported.push("validate");
                out.scripts.validate = Some(v);
            }
            None => {}
        }
    }
    if let Some(c) = js(b"C") {
        match af::parse_calculate(&c) {
            Some(cc) => out.calculate = cc,
            None if !c.trim().is_empty() => {
                out.unsupported.push("calculate");
                out.scripts.calculate = Some(c);
            }
            None => {}
        }
    }
    out
}

/// Text streams of field values (`/V`, `/DV`) one reading of the form may decode in all, however
/// many fields name the same stream.
pub(crate) const MAX_VALUE_BYTES: usize = 64 << 20;

/// A field's value or default value (`/V`, `/DV`). A text stream is decoded against `budget`, and
/// reads as no value once it is spent, as one that won't decode does.
fn values_of(doc: &Document, o: Option<&Object>, budget: &mut DecodeBudget) -> Vec<String> {
    let Some(o) = o else { return Vec::new() };
    match &*doc.resolve(o) {
        Object::Array(a) => a.iter().filter_map(|x| text_of(&doc.resolve(x))).collect(),
        Object::Stream(s) => budget.decode(s).ok().map(|b| PdfString::literal(b).to_text()).into_iter().collect(),
        other => text_of(other).filter(|s| s != "Off" && !s.is_empty()).into_iter().collect(),
    }
}

/// A choice field's `/I` (ISO 32000-2 Table 234) as option positions, when it is usable: an array
/// of integers "sorted in ascending order", each naming one of `count` options. `None` when it is
/// absent or anything in it is not (a non-integer, a position out of range, out of order or
/// repeated), so the field falls back to `/V`.
pub(crate) fn listed_indices(doc: &Document, d: &Dict, count: usize) -> Option<Vec<usize>> {
    let listed = doc.resolve(d.get(b"I")?);
    let entries = listed.as_array()?;
    // Strictly ascending positions below `count` number at most `count`.
    if entries.len() > count {
        return None;
    }
    let mut out: Vec<usize> = Vec::with_capacity(entries.len());
    for e in entries {
        let i = usize::try_from(doc.resolve(e).as_int()?).ok().filter(|&i| i < count)?;
        if out.last().is_some_and(|&last| last >= i) {
            return None;
        }
        out.push(i);
    }
    Some(out)
}

/// The options a choice field shows selected, as ascending positions in `options`, given its
/// values (`/V`) and its usable `/I` ([`listed_indices`]).
///
/// `/I` decides when it names the same export values as `/V`: it "shall be used when two or more
/// elements in the Opt array have different names but the same export value", and "if the items
/// identified by this entry differ from those in the V entry …, the V entry shall be used" (ISO
/// 32000-2 Table 234). Otherwise each value selects the first option with that export value, as
/// the writer numbers `/I` when it is given only an export value, so a pick keeps showing the same
/// option when `/I` is lost. A value no option exports selects nothing.
pub fn selected_options(options: &[(String, String)], values: &[String], listed: Option<&[usize]>) -> Vec<usize> {
    if let Some(listed) = listed {
        // The same items, each as many times: `/V [(1) (1)]` names two items, which an `/I [1]`
        // naming one doesn't, so `/V` decides then.
        let mut named: Vec<&str> = values.iter().map(String::as_str).collect();
        named.sort_unstable();
        let exports: Option<Vec<&str>> = listed.iter().map(|&i| options.get(i).map(|(e, _)| e.as_str())).collect();
        if let Some(mut exports) = exports {
            exports.sort_unstable();
            if exports == named {
                return listed.to_vec();
            }
        }
    }
    let first = first_by_export(options);
    let mut out: Vec<usize> = values.iter().filter_map(|v| first.get(v.as_str()).copied()).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Each export value in `options` with the position of the first option exporting it, so
/// matching values against options takes one pass over each rather than one over the options per
/// value (a file can hold many of both).
pub(crate) fn first_by_export(options: &[(String, String)]) -> std::collections::HashMap<&str, usize> {
    let mut first: std::collections::HashMap<&str, usize> = std::collections::HashMap::with_capacity(options.len());
    for (i, (e, _)) in options.iter().enumerate() {
        first.entry(e.as_str()).or_insert(i);
    }
    first
}

fn nums(doc: &Document, o: Option<&Object>) -> Option<Vec<f64>> {
    let o = doc.resolve(o?);
    o.as_array()?.iter().map(|x| doc.resolve(x).as_f64()).collect()
}

/// Leaf page objects in order (a small walker; `organize` and `annot` have their own).
pub(crate) fn page_refs(doc: &Document) -> Vec<ObjRef> {
    let Some(root) = doc.root() else { return Vec::new() };
    let Some(pages) = doc.get(root).as_dict().and_then(|d| d.reference(b"Pages")) else { return Vec::new() };
    PageWalk::new(doc, pages).collect()
}

/// Depth-first walk over the leaf pages under `top`. It keeps one cursor per open `/Kids` array,
/// never one entry per child, so a child listed many times or shared between nodes is skipped by
/// the seen check without being queued first (`pdfcraft_model::pages` walks the same way).
/// Cycles end at the seen check and at most a million nodes are visited.
pub(crate) struct PageWalk<'a> {
    doc: &'a Document,
    seen: std::collections::HashSet<ObjRef>,
    pub(crate) pending: Vec<(std::sync::Arc<Object>, usize)>,
}

impl<'a> PageWalk<'a> {
    pub(crate) fn new(doc: &'a Document, top: ObjRef) -> Self {
        let top = std::sync::Arc::new(Object::Array(vec![Object::Ref(top)]));
        Self { doc, seen: Default::default(), pending: vec![(top, 0)] }
    }
}

impl Iterator for PageWalk<'_> {
    type Item = ObjRef;

    fn next(&mut self) -> Option<ObjRef> {
        while let Some((kids, next)) = self.pending.last_mut() {
            let Some(kid) = kids.as_array().and_then(|a| a.get(*next)) else {
                self.pending.pop();
                continue;
            };
            let node = kid.as_ref();
            *next += 1;
            let Some(node) = node else { continue };
            if self.seen.len() >= 1_000_000 {
                self.pending.clear();
                return None;
            }
            if !self.seen.insert(node) {
                continue;
            }
            let obj = self.doc.get(node);
            let Some(d) = obj.as_dict() else { continue };
            match d.get(b"Kids").map(|k| self.doc.resolve(k)) {
                Some(kids) if d.name(b"Type") != Some(b"Page") => self.pending.push((kids, 0)),
                _ => return Some(node),
            }
        }
        None
    }
}

/// The AcroForm dictionary.
pub fn acroform(doc: &Document) -> Option<Dict> {
    let root = doc.root()?;
    let cat = doc.get(root);
    let af = cat.as_dict()?.get(b"AcroForm")?.clone();
    doc.resolve(&af).as_dict().cloned()
}

#[derive(Clone, Default)]
struct Inherited {
    name: String,
    ft: Option<Vec<u8>>,
    ff: Option<u32>,
    v: Option<Object>,
    dv: Option<Object>,
    da: Option<String>,
    q: Option<i64>,
    max_len: Option<usize>,
}

/// Every terminal field: the `/Fields` tree in order, then the fields reachable only through the
/// page annotations (see [`adopt_page_fields`]).
pub fn fields(doc: &Document) -> Vec<Field> {
    fields_with_adopted(doc).items
}

/// How many fields [`fields`] read with no options: see [`Fields::options_omitted`].
pub fn options_omitted(doc: &Document) -> usize {
    fields_with_adopted(doc).options_omitted
}

/// How many of [`fields`] were reachable only through the page annotations: the form's `/Fields`
/// list names none or only some of them, so the leniency that adopts them is worth recording.
pub fn adopted_page_fields(doc: &Document) -> usize {
    fields_with_adopted(doc).adopted_page_fields
}

/// The terminal fields and page-only fields adopted while reading a form.
#[derive(Clone, Debug, PartialEq)]
pub struct Fields {
    /// Terminal fields in tree order, followed by adopted page-only fields.
    pub items: Vec<Field>,
    /// How many fields were reachable only through page annotations.
    pub adopted_page_fields: usize,
    /// How many combo and list boxes were read with no options because the form's `/Opt` arrays
    /// together held more than [`MAX_OPTION_ENTRIES`] entries or [`MAX_OPTION_BYTES`] of text. Their `/Opt` is still in the file.
    pub options_omitted: usize,
}

/// Read all terminal fields and the page-only adoption count in one enumeration.
///
/// The last reading on this thread is kept with the state of the document it read
/// ([`pdfcraft_cos::ReadState`], which every change to the document replaces), and a document
/// that still reads as that state gets a copy of it instead of a new walk. One form edit reads
/// the fields in several steps (filling, recalculating, the XFA data, the view), most of them
/// with nothing changed since the step before: on a form with a thousand fields each walk takes
/// tens of milliseconds.
pub fn fields_with_adopted(doc: &Document) -> Fields {
    let kept = LAST_READING
        .try_with(|last| match last.try_borrow().as_deref() {
            Ok(Some((state, fields))) if state.matches(doc) => Some(fields.clone()),
            _ => None,
        })
        .ok()
        .flatten();
    if let Some(fields) = kept {
        return fields;
    }
    let (items, adopted_page_fields, options_omitted) = enumerate(doc);
    let fields = Fields { items, adopted_page_fields, options_omitted };
    // Not kept (the thread is ending, or the slot is in use) costs only a later walk.
    let _ = LAST_READING.try_with(|last| {
        if let Ok(mut last) = last.try_borrow_mut() {
            *last = Some((doc.read_state(), fields.clone()));
        }
    });
    fields
}

thread_local! {
    /// The last [`fields_with_adopted`] on this thread, with the state of the document it read.
    /// One entry: one form's fields, and the objects that document had changed when it was read.
    static LAST_READING: std::cell::RefCell<Option<(pdfcraft_cos::ReadState, Fields)>> = const { std::cell::RefCell::new(None) };
    /// Whole-form walks ([`enumerate`]) on this thread.
    static FORM_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many times this thread has walked a whole form: [`fields`] and the readings like it, but
/// not one answered from the last reading. Per thread, so tests running in parallel don't count
/// each other's; for tests that check how often an edit reads the form.
#[doc(hidden)]
pub fn form_reads_on_this_thread() -> usize {
    FORM_READS.try_with(std::cell::Cell::get).unwrap_or(0)
}

pub(crate) fn enumerate(doc: &Document) -> (Vec<Field>, usize, usize) {
    let _ = FORM_READS.try_with(|n| n.set(n.get().saturating_add(1)));
    let af = acroform(doc);
    let mut page_of = std::collections::HashMap::new();
    let mut annot_index = std::collections::HashMap::new();
    let mut by_parent: std::collections::HashMap<ObjRef, Vec<ObjRef>> = std::collections::HashMap::new();
    let pages = page_refs(doc);
    for (i, p) in pages.iter().enumerate() {
        if let Some(a) = doc.get(*p).as_dict().and_then(|d| d.get(b"Annots").cloned()) {
            for (k, e) in doc.resolve(&a).as_array().into_iter().flatten().enumerate() {
                if let Some(r) = e.as_ref()
                    && let std::collections::hash_map::Entry::Vacant(slot) = page_of.entry(r)
                {
                    slot.insert(i);
                    annot_index.insert(r, k);
                    if let Some(parent) = widget_parent(doc, r) {
                        by_parent.entry(parent).or_default().push(r);
                    }
                }
            }
        }
    }
    let base = match &af {
        Some(af) => Inherited {
            da: af.get(b"DA").and_then(|o| text_of(&doc.resolve(o))),
            q: af.get(b"Q").and_then(|o| doc.resolve(o).as_int()),
            ..Default::default()
        },
        None => Inherited::default(),
    };
    let mut out = Vec::new();
    let listed_roots: Vec<ObjRef> = af
        .as_ref()
        .and_then(|af| af.get(b"Fields").map(|o| doc.resolve(o)).and_then(|o| o.as_array().cloned()))
        .unwrap_or_default()
        .iter()
        .filter_map(Object::as_ref)
        .collect();
    let mut seen = Seen {
        fields: std::collections::HashSet::new(),
        widgets: std::collections::HashSet::new(),
        roots: listed_roots.iter().copied().collect(),
        path: Vec::new(),
        by_parent,
        scripts: script_budget(),
        values: DecodeBudget::new(MAX_VALUE_BYTES, 0),
        option_entries_left: MAX_OPTION_ENTRIES,
        option_bytes_left: MAX_OPTION_BYTES,
        options_omitted: 0,
    };
    for r in listed_roots {
        // A `/Fields` entry that is a nameless kid of a field `/Fields` doesn't list is read
        // through that field, which names it (ISO 32000-2 §12.7.4.2). A kid a field already read
        // has claimed as its widget is not read again as a nameless field; one its parent's
        // `/Kids` omits is still read on its own, as before.
        let nameless_kid = doc.get(r).as_dict().is_some_and(|d| !d.contains(b"T") && d.contains(b"Parent"));
        if nameless_kid {
            if let Some(root) = unlisted_root(doc, r, &seen.fields) {
                walk(doc, root, &base, &page_of, &mut seen, &mut out, 0);
            }
            if seen.widgets.contains(&r) {
                continue;
            }
        }
        walk(doc, r, &base, &page_of, &mut seen, &mut out, 0);
    }
    let listed = out.len();
    adopt_page_fields(doc, &pages, &base, &page_of, &mut seen, &mut out);
    let adopted = out.len() - listed;
    rank_tabs(doc, &pages, &annot_index, &mut out);
    (out, adopted, seen.options_omitted)
}

/// Widget annotations no `/Fields` entry reaches are adopted as fields, as Acrobat and the
/// browsers do: some writers list only some of the fields, or none at all. Each widget is adopted
/// through its topmost unlisted `/Parent`, so a field split across several widgets stays one
/// field. Tree-listed fields keep their order first; the rest follow in page-annotation order.
fn adopt_page_fields(
    doc: &Document,
    pages: &[ObjRef],
    base: &Inherited,
    page_of: &std::collections::HashMap<ObjRef, usize>,
    seen: &mut Seen,
    out: &mut Vec<Field>,
) {
    for p in pages {
        let Some(a) = doc.get(*p).as_dict().and_then(|d| d.get(b"Annots").cloned()) else { continue };
        let annots = doc.resolve(&a);
        let Some(annots) = annots.as_array() else { continue };
        for e in annots {
            let Some(r) = e.as_ref() else { continue };
            if seen.fields.contains(&r) || seen.widgets.contains(&r) {
                continue;
            }
            let obj = doc.get(r);
            let Some(d) = obj.as_dict() else { continue };
            if d.name(b"Subtype") != Some(b"Widget") {
                continue;
            }
            if !d.contains(b"FT") && !d.contains(b"T") && !d.contains(b"Parent") {
                continue;
            }
            if let Some(root) = unlisted_root(doc, r, &seen.fields) {
                walk(doc, root, base, page_of, seen, out, 0);
            }
        }
    }
}

/// The topmost field above `r` (itself without a `/Parent`), or `None` when `r` or an ancestor is
/// already reachable from the `/Fields` tree, or a `/Parent` loop hides the top.
fn unlisted_root(doc: &Document, r: ObjRef, seen: &std::collections::HashSet<ObjRef>) -> Option<ObjRef> {
    let mut top = r;
    let mut visited = std::collections::HashSet::from([r]);
    loop {
        let parent = doc.get(top).as_dict().and_then(|d| parent_of(doc, d));
        let Some(p) = parent else {
            return if seen.contains(&top) { None } else { Some(top) };
        };
        if !visited.insert(p) || visited.len() > 64 {
            return None;
        }
        if seen.contains(&p) {
            return None;
        }
        top = p;
    }
}

/// What one reading of the form's fields has used: the field and widget dictionaries read, the
/// script budget their actions share, and the budget their value streams share.
struct Seen {
    fields: std::collections::HashSet<ObjRef>,
    /// A widget belongs to the first field that lists it, however often it is listed.
    widgets: std::collections::HashSet<ObjRef>,
    /// The fields `/Fields` lists: root fields, with no ancestors (ISO 32000-2 §12.7.3, Table 224).
    roots: std::collections::HashSet<ObjRef>,
    /// The fields being walked, outermost first, whose `/Kids` are being read.
    path: Vec<ObjRef>,
    /// The page widgets (no `/T`) that name each field as their `/Parent`, in page-annotation order.
    by_parent: std::collections::HashMap<ObjRef, Vec<ObjRef>>,
    scripts: DecodeBudget,
    values: DecodeBudget,
    /// Choice options the fields read so far may still take, from [`MAX_OPTION_ENTRIES`].
    option_entries_left: usize,
    /// Bytes of option text the fields read so far may still take, from [`MAX_OPTION_BYTES`].
    option_bytes_left: usize,
    /// Fields read with no options because their `/Opt` array did not fit what was left.
    options_omitted: usize,
}

/// The field a page widget `r` names as its `/Parent`, when `r` is a widget and not a field of its
/// own (no `/T`).
fn widget_parent(doc: &Document, r: ObjRef) -> Option<ObjRef> {
    let obj = doc.get(r);
    let d = obj.as_dict()?;
    if d.name(b"Subtype") != Some(b"Widget") || d.contains(b"T") {
        return None;
    }
    d.reference(b"Parent")
}

/// Whether `k`, listed in the `/Kids` of field `r`, is not a child of `r` after all: `r` itself or
/// a field `r` descends from (a loop), or a root field (one `/Fields` lists) that doesn't name `r`
/// as its `/Parent`. Root fields have no ancestors (ISO 32000-2 §12.7.3, Table 224), and a
/// child names its parent (§12.7.4.1, Table 226): only the `/Kids` entry says otherwise, so the
/// field is read as the root it is, and `r` keeps its other kids.
fn not_a_child(doc: &Document, seen: &Seen, r: ObjRef, k: ObjRef) -> bool {
    k == r || seen.path.contains(&k) || (seen.roots.contains(&k) && doc.get(k).as_dict().and_then(|d| d.reference(b"Parent")) != Some(r))
}

/// Read the field at `r` and the fields under it into `out`, with what they inherit from
/// `parent`: kids with a name are fields of their own, kids without are this field's widgets.
fn walk(
    doc: &Document,
    r: ObjRef,
    parent: &Inherited,
    page_of: &std::collections::HashMap<ObjRef, usize>,
    seen: &mut Seen,
    out: &mut Vec<Field>,
    depth: usize,
) {
    if depth > 64 || !seen.fields.insert(r) {
        return;
    }
    let obj = doc.get(r);
    let Some(d) = obj.as_dict() else { return };
    let mut inh = parent.clone();
    if let Some(t) = d.get(b"T").and_then(|o| text_of(&doc.resolve(o))) {
        inh.name = if inh.name.is_empty() { t } else { format!("{}.{t}", inh.name) };
    }
    if let Some(ft) = d.name(b"FT") {
        inh.ft = Some(ft.to_vec());
    }
    if let Some(ff) = field_flags(doc, d) {
        inh.ff = Some(ff);
    }
    for (k, slot) in [(&b"V"[..], &mut inh.v), (b"DV", &mut inh.dv)] {
        if let Some(v) = d.get(k) {
            *slot = Some(v.clone());
        }
    }
    if let Some(da) = d.get(b"DA").and_then(|o| text_of(&doc.resolve(o))) {
        inh.da = Some(da);
    }
    if let Some(q) = d.get(b"Q").and_then(|o| doc.resolve(o).as_int()) {
        inh.q = Some(q);
    }
    if let Some(m) = d.get(b"MaxLen").and_then(|o| doc.resolve(o).as_int()) {
        // Compatibility: some writers emit zero for an unlimited text field. Keep /MaxLen
        // untouched in the object graph, but expose only positive limits to both UI and API.
        // An explicit zero also clears an inherited limit rather than inheriting it again.
        inh.max_len = usize::try_from(m).ok().filter(|limit| *limit > 0);
    }
    let kids: Vec<ObjRef> =
        d.get(b"Kids").map(|k| doc.resolve(k)).and_then(|k| k.as_array().cloned()).unwrap_or_default().iter().filter_map(|k| k.as_ref()).collect();
    // Kids with /T are fields; kids without are this field's widgets.
    let (field_kids, mut kids): (Vec<ObjRef>, Vec<ObjRef>) =
        kids.into_iter().partition(|k| doc.get(*k).as_dict().is_some_and(|kd| kd.contains(b"T")));
    let field_kids: Vec<ObjRef> = field_kids.into_iter().filter(|k| !not_a_child(doc, seen, r, *k)).collect();
    if !field_kids.is_empty() {
        seen.path.push(r);
        for k in field_kids {
            // A fully qualified name is built up the field's own `/Parent` chain (ISO 32000-2
            // §12.7.4.2). A kid that doesn't name this field as its `/Parent` is named from its
            // own chain, as other readers name it; it still inherits this field's attributes.
            // The `/Parent` as written: `qualified_name` follows an object that only refers on,
            // and names a missing one as no parent.
            let parent = doc.get(k).as_dict().and_then(|kd| kd.reference(b"Parent"));
            if parent == Some(r) {
                walk(doc, k, &inh, page_of, seen, out, depth + 1);
            } else {
                let mut own = inh.clone();
                own.name = parent.and_then(|p| qualified_name(doc, p)).unwrap_or_default();
                walk(doc, k, &own, page_of, seen, out, depth + 1);
            }
        }
        seen.path.pop();
        // "A field's children in the hierarchy may also include widget annotations" (ISO 32000-2
        // §12.7.4): widget annotations listed beside named kids are still this field's widgets,
        // read as a field of this name next to the named kids.
        kids.retain(|k| doc.get(*k).as_dict().is_some_and(|kd| kd.name(b"Subtype") == Some(b"Widget")));
        if kids.is_empty() {
            return;
        }
    }
    // A field of no kind this reads is skipped before it claims widgets or scripts, which a later
    // field listing them then reads.
    let ff = inh.ff.unwrap_or(0);
    let kind = match inh.ft.as_deref() {
        Some(b"Tx") => FieldKind::Text,
        Some(b"Btn") if ff & flags::PUSH_BUTTON != 0 => FieldKind::PushButton,
        Some(b"Btn") if ff & flags::RADIO != 0 => FieldKind::Radio,
        Some(b"Btn") => FieldKind::CheckBox,
        Some(b"Ch") if ff & flags::COMBO != 0 => FieldKind::Combo,
        Some(b"Ch") => FieldKind::List,
        Some(b"Sig") => FieldKind::Signature,
        _ => return,
    };
    // A child names its parent (ISO 32000-2 §12.7.4.1, Table 226): a page widget naming this field
    // is one of its widgets even when its `/Kids` leave it out, as MuPDF and PDFium read it.
    // A field's widgets are its widget kids. The field dictionary is its own widget only when it
    // has none (no kid whose `/Subtype` is `/Widget`): field and widget are merged "when a field has only a single associated widget
    // annotation" (ISO 32000-2 §12.7.4), so a merged dictionary that also lists widget kids is a
    // field whose buttons are those kids, not one more (often zero-area) button.
    let has_widget_kids = kids.iter().any(|k| doc.get(*k).as_dict().is_some_and(|kd| kd.name(b"Subtype") == Some(b"Widget")));
    let widget_refs: Vec<ObjRef> = if d.contains(b"Rect") && !has_widget_kids {
        vec![r]
    } else {
        kids.into_iter().chain(seen.by_parent.get(&r).into_iter().flatten().copied()).collect()
    };
    let widget_refs: Vec<ObjRef> = widget_refs.into_iter().filter(|w| seen.widgets.insert(*w)).collect();
    let actions = actions_of(doc, d, &widget_refs, &mut seen.scripts);
    let button = (inh.ft.as_deref() == Some(b"Btn") && inh.ff.unwrap_or(0) & flags::PUSH_BUTTON != 0)
        .then(|| button_action(doc, d, &widget_refs, &mut seen.scripts))
        .flatten();
    let widgets: Vec<Widget> = widget_refs
        .into_iter()
        .filter_map(|w| {
            let wo = doc.get(w);
            let wd = wo.as_dict()?;
            let rect = nums(doc, wd.get(b"Rect")).filter(|r| r.len() == 4).unwrap_or_else(|| vec![0.0; 4]);
            let annot_flags = wd.get(b"F").and_then(|f| doc.resolve(f).as_int()).unwrap_or(0);
            let own_ff = if w == r { 0 } else { field_flags(doc, wd).unwrap_or(0) };
            let on_state = wd
                .get(b"AP")
                .map(|ap| doc.resolve(ap))
                .and_then(|ap| ap.as_dict().and_then(|a| a.get(b"N").cloned()))
                .and_then(|n| doc.resolve(&n).as_dict().cloned())
                .and_then(|n| n.iter().map(|(k, _)| name_text(k)).find(|k| k != "Off"));
            Some(Widget {
                obj: w,
                page: page_of.get(&w).copied(),
                rect: [rect[0].min(rect[2]), rect[1].min(rect[3]), rect[0].max(rect[2]), rect[1].max(rect[3])],
                on_state: on_state.filter(|_| matches!(kind, FieldKind::CheckBox | FieldKind::Radio)),
                state: wd.name(b"AS").map(name_text),
                tab: usize::MAX,
                locked: annot_flags & 128 != 0,
                hidden: annot_flags & (2 | 32) != 0,
                read_only: (ff | own_ff) & flags::READ_ONLY != 0,
                rotation: appearance::mk_rotation(doc, wd),
                caption: (kind == FieldKind::PushButton)
                    .then(|| {
                        let mk = doc.resolve(wd.get(b"MK")?);
                        let value = doc.resolve(mk.as_dict()?.get(b"CA")?);
                        value.as_string().map(|s| s.to_text())
                    })
                    .flatten(),
                mouse_up_script: if kind == FieldKind::Text {
                    let aa = wd.get(b"AA").map(|a| doc.resolve(a));
                    let action = aa.as_deref().and_then(Object::as_dict).and_then(|aa| aa.get(b"U")).or_else(|| wd.get(b"A"));
                    action
                        .filter(|a| doc.resolve(a).as_dict().and_then(|a| a.name(b"S")) == Some(b"JavaScript"))
                        .and_then(|a| script(doc, a, &mut seen.scripts))
                } else {
                    None
                },
                mouse_down: wd.get(b"AA").map(|a| doc.resolve(a)).as_deref().and_then(Object::as_dict).is_some_and(|aa| aa.contains(b"D")),
                list: (kind == FieldKind::List).then(|| appearance::list_rows_of(doc, inh.da.as_deref().unwrap_or("/Helv 0 Tf 0 g"), Some(d), wd)),
                round: kind == FieldKind::Radio || (kind == FieldKind::CheckBox && appearance::is_round(doc, wd)),
            })
        })
        .collect();
    // `/Opt` entries and their text are spent from allowances for the whole form: fields can share
    // one array, and each field's own copy of its strings made fields x options of them (entries
    // can name one long string object too). A field whose list no longer fits reads with no
    // options and says so ([`Field::options_omitted`]).
    let opt = d.get(b"Opt").map(|o| doc.resolve(o));
    let choice = matches!(kind, FieldKind::Combo | FieldKind::List);
    // Only a choice field's list, and a check box's or radio group's list of one entry per widget
    // (`Field::export_of`), is used; other `/Opt` entries are not read. A button group whose list
    // is refused falls back to its on-state names, as with no `/Opt`; only a choice field is
    // flagged, as its options are what it shows and matches values against.
    let entries = opt.as_deref().and_then(Object::as_array).filter(|e| match kind {
        FieldKind::Combo | FieldKind::List => true,
        FieldKind::CheckBox | FieldKind::Radio => e.len() == widgets.len(),
        _ => false,
    });
    let (options, options_omitted) = match entries {
        None => (Vec::new(), false),
        Some(entries) => match read_options(doc, entries, seen) {
            Some(options) => (options, false),
            None => {
                seen.options_omitted += usize::from(choice);
                (Vec::new(), choice)
            }
        },
    };
    let mut value = values_of(doc, inh.v.as_ref(), &mut seen.values);
    // A list box without MultiSelect selects one option: "if clear, at most one item shall be
    // selected" (ISO 32000-2 Table 233). Given an array /V or /DV anyway, it holds the first
    // value only, and shows and resets to it; the file keeps its entries until the field is filled.
    let mut default = values_of(doc, inh.dv.as_ref(), &mut seen.values);
    let single = kind == FieldKind::List && ff & flags::MULTI_SELECT == 0;
    let single_value_is_array = single && inh.v.as_ref().is_some_and(|v| doc.resolve(v).as_array().is_some());
    if single {
        value.truncate(1);
        default.truncate(1);
    }
    let mut selected = match kind {
        FieldKind::Combo | FieldKind::List => selected_options(&options, &value, listed_indices(doc, d, options.len()).as_deref()),
        _ => Vec::new(),
    };
    if single {
        selected.truncate(1);
    }
    out.push(Field {
        name: inh.name.clone(),
        obj: r,
        kind,
        value,
        default,
        flags: ff,
        max_len: inh.max_len,
        options,
        options_omitted,
        selected,
        single_value_is_array,
        da: inh.da.clone().unwrap_or_else(|| "/Helv 0 Tf 0 g".into()),
        quadding: inh.q.unwrap_or(0),
        tooltip: d.get(b"TU").and_then(|o| text_of(&doc.resolve(o))),
        widgets,
        actions,
        button,
    });
}

pub(crate) fn field_flags(doc: &Document, dict: &Dict) -> Option<u32> {
    dict.get(b"Ff").and_then(|value| doc.resolve(value).as_int()).and_then(|value| u32::try_from(value).ok())
}
