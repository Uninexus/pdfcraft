//! Ink checks for the print-readiness report: total ink, rich black text, overprint and spot
//! colours, read from the vector colours a page's content sets.
//!
//! Only colours set by operators are read: `k`/`K`, and `sc`/`scn` in a DeviceCMYK or four-component
//! ICCBased space. Colours inside images, shadings, patterns, annotation appearance streams and
//! Indexed bases are not measured, and a colour a form XObject sets is counted where the form is
//! drawn. Text drawn invisibly (text rendering mode 3, as in the text layer of a scan) is not
//! counted. Verification only: nothing is changed.
//!
//! The work is bounded: content and forms are decoded against the page's [`DecodeBudget`] (which
//! also limits how many form entries one page may draw), forms nest at most [`MAX_FORM_DEPTH`]
//! deep, a form's resources are resolved once however often it is drawn, and the colorant names a
//! page can report are capped.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

use pdfcraft_cos::{DecodeBudget, Dict, Document, ObjRef, Object};
use pdfcraft_model::Page;

/// Form XObjects nested deeper than this are not read.
const MAX_FORM_DEPTH: usize = 12;
/// `q` nesting past this is ignored, so a hostile stream can't grow the stack.
const MAX_STACK: usize = 256;
/// Spot colorant names kept for one page, and the bytes kept of each.
const MAX_SPOT_NAMES: usize = 64;
const MAX_SPOT_NAME_BYTES: usize = 64;
/// Entries of a DeviceN colorant list read.
const MAX_COLORANT_ENTRIES: usize = 256;

/// What a page's content sets, measured in the vector colours it paints with.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct InkPage {
    /// 1-based page number.
    pub page: u32,
    /// The most ink any painted CMYK colour uses, in percent (C + M + Y + K).
    pub max_total_percent: f64,
    /// Painted objects whose CMYK colour uses more than the limit given to [`ink_pages`].
    pub over_limit: usize,
    /// Text showings in rich black: K at 90% or more with some other ink.
    pub rich_black_text: usize,
    /// A graphics state (`gs`) turned overprint on.
    pub overprint: bool,
    /// Names of the Separation and DeviceN colorants selected with `cs` (process colorants, All
    /// and None are not spot colours), at most [`MAX_SPOT_NAMES`].
    pub spot: BTreeSet<String>,
    /// More spot colorants were selected than `spot` holds.
    pub spot_capped: bool,
    /// Content was missing, damaged, past the page's decode budget, or in a form nested too deeply
    /// or drawing itself, so some colours may be missing.
    pub incomplete: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Space {
    Cmyk,
    Spot,
    Other,
}

/// What `cs` selects: the space and its initial colour (DeviceCMYK starts at 100% K; any other
/// four-component space starts at zero).
#[derive(Clone, Copy, Debug)]
struct Selected {
    space: Space,
    initial: Option<[f64; 4]>,
}

const OTHER: Selected = Selected { space: Space::Other, initial: None };

#[derive(Clone, Debug)]
struct Gs {
    fill_space: Space,
    stroke_space: Space,
    fill: Option<[f64; 4]>,
    stroke: Option<[f64; 4]>,
    /// Text rendering mode (`Tr`).
    render: i64,
}

impl Default for Gs {
    fn default() -> Self {
        Self { fill_space: Space::Other, stroke_space: Space::Other, fill: None, stroke: None, render: 0 }
    }
}

/// The resources a content stream reads, taken from its dictionary once.
struct Res {
    spaces: Dict,
    xobjects: Dict,
    states: Dict,
    /// What `cs` selected for each name already looked up.
    selected: RefCell<HashMap<Vec<u8>, Selected>>,
}

/// Reads each page in `pages` and reports the ones that set any of what is looked for.
pub(crate) fn ink_pages(doc: &Document, pages: &[Page], limit_percent: f64) -> Vec<InkPage> {
    let mut out = Vec::new();
    for (i, page) in pages.iter().enumerate() {
        let mut walk = Walk {
            doc,
            limit: limit_percent,
            forms: HashSet::new(),
            resources: HashMap::new(),
            budget: DecodeBudget::page(),
            report: InkPage { page: (i + 1) as u32, ..InkPage::default() },
        };
        let res = Rc::new(walk.res_of(page.dict.get(b"Resources").map(|r| doc.resolve(r)).as_deref()));
        let mut data = Vec::new();
        if let Some(contents) = page.dict.get(b"Contents") {
            for read in walk.budget.decode_contents_with_damage(doc, contents) {
                match read {
                    Ok((chunk, damaged)) => {
                        walk.report.incomplete |= damaged;
                        data.extend_from_slice(&chunk);
                        data.push(b' ');
                    }
                    Err(_) => walk.report.incomplete = true,
                }
            }
        }
        walk.content(&data, &res, Gs::default(), 0);
        let mut r = std::mem::take(&mut walk.report);
        r.incomplete |= walk.budget.refused() > 0;
        if r.over_limit > 0 || r.rich_black_text > 0 || r.overprint || !r.spot.is_empty() || r.incomplete {
            out.push(r);
        }
    }
    out
}

struct Walk<'a> {
    doc: &'a Document,
    limit: f64,
    forms: HashSet<ObjRef>,
    /// Resources already taken from their dictionaries, by the object they came from: a form's
    /// resources are read once, however many times it is drawn.
    resources: HashMap<ObjRef, Rc<Res>>,
    budget: DecodeBudget,
    report: InkPage,
}

impl Walk<'_> {
    fn sub(&self, res: &Dict, key: &[u8]) -> Dict {
        res.get(key).map(|x| self.doc.resolve(x)).and_then(|x| x.as_dict().cloned()).unwrap_or_default()
    }

    /// The resources in `object` (a dictionary, or nothing).
    fn res_of(&self, object: Option<&Object>) -> Res {
        let dict = object.and_then(Object::as_dict).cloned().unwrap_or_default();
        Res {
            spaces: self.sub(&dict, b"ColorSpace"),
            xobjects: self.sub(&dict, b"XObject"),
            states: self.sub(&dict, b"ExtGState"),
            selected: RefCell::default(),
        }
    }

    /// What `cs` with `name` selects. The colorants of a Separation or DeviceN space are
    /// recorded the first time it is looked up.
    fn select(&mut self, name: &[u8], res: &Res) -> Selected {
        match name {
            b"DeviceCMYK" | b"CMYK" => return Selected { space: Space::Cmyk, initial: Some([0.0, 0.0, 0.0, 1.0]) },
            b"DeviceGray" | b"G" | b"DeviceRGB" | b"RGB" | b"Pattern" => return OTHER,
            _ => {}
        }
        if let Some(found) = res.selected.borrow().get(name) {
            return *found;
        }
        let (selected, colorants) = self.resolve_space(name, &res.spaces);
        for colorant in colorants {
            self.note_spot(colorant);
        }
        if res.selected.borrow().len() < 4096 {
            res.selected.borrow_mut().insert(name.to_vec(), selected);
        }
        selected
    }

    fn resolve_space(&self, name: &[u8], spaces: &Dict) -> (Selected, Vec<Vec<u8>>) {
        let Some(value) = spaces.get(name).map(|x| self.doc.resolve(x)) else { return (OTHER, Vec::new()) };
        if let Some(n) = value.as_name() {
            return if n == b"DeviceCMYK" {
                (Selected { space: Space::Cmyk, initial: Some([0.0, 0.0, 0.0, 1.0]) }, Vec::new())
            } else {
                (OTHER, Vec::new())
            };
        }
        let Some(array) = value.as_array() else { return (OTHER, Vec::new()) };
        let family = array.first().map(|x| self.doc.resolve(x));
        match family.as_ref().and_then(|x| x.as_name()) {
            Some(b"ICCBased") => {
                let components = array
                    .get(1)
                    .map(|x| self.doc.resolve(x))
                    .and_then(|x| match &*x {
                        Object::Stream(s) => s.dict.get(b"N").map(|n| self.doc.resolve(n)).and_then(|n| n.as_int()),
                        _ => None,
                    })
                    .unwrap_or(0);
                // A four-component ICC space starts at zero in every component.
                (if components == 4 { Selected { space: Space::Cmyk, initial: Some([0.0; 4]) } } else { OTHER }, Vec::new())
            }
            Some(b"Separation") => {
                let name = array.get(1).map(|x| self.doc.resolve(x)).and_then(|x| x.as_name().map(<[u8]>::to_vec));
                (Selected { space: Space::Spot, initial: None }, name.into_iter().filter(|n| is_spot(n)).collect())
            }
            Some(b"DeviceN") => {
                let list = array.get(1).map(|x| self.doc.resolve(x));
                let names: Vec<Vec<u8>> = list
                    .as_deref()
                    .and_then(Object::as_array)
                    .map(|list| {
                        list.iter()
                            .take(MAX_COLORANT_ENTRIES)
                            .filter_map(|n| self.doc.resolve(n).as_name().map(<[u8]>::to_vec))
                            .filter(|n| is_spot(n))
                            .collect()
                    })
                    .unwrap_or_default();
                (Selected { space: Space::Spot, initial: None }, names)
            }
            _ => (OTHER, Vec::new()),
        }
    }

    /// Records a spot colorant, within the caps.
    fn note_spot(&mut self, colorant: Vec<u8>) {
        let mut name = String::from_utf8_lossy(&colorant).into_owned();
        if name.len() > MAX_SPOT_NAME_BYTES {
            let mut end = MAX_SPOT_NAME_BYTES;
            while end > 0 && !name.is_char_boundary(end) {
                end -= 1;
            }
            name.truncate(end);
            name.push('…');
        }
        if self.report.spot.len() >= MAX_SPOT_NAMES && !self.report.spot.contains(&name) {
            self.report.spot_capped = true;
            return;
        }
        self.report.spot.insert(name);
    }

    /// Counts the use of colour `c` for painting. Returns whether it is rich black.
    fn paint(&mut self, c: Option<[f64; 4]>) -> bool {
        let Some([cy, m, y, k]) = c else { return false };
        let total = (cy + m + y + k) * 100.0;
        if total.is_finite() && total > self.report.max_total_percent {
            self.report.max_total_percent = total;
        }
        if total.is_finite() && total > self.limit {
            self.report.over_limit = self.report.over_limit.saturating_add(1);
        }
        k >= 0.9 && cy + m + y >= 0.1
    }

    /// Text shown in the graphics state `gs`: invisible text (mode 3 and 7) paints nothing, and
    /// the stroke colour is used by the stroking modes.
    fn show_text(&mut self, gs: &Gs) {
        let (fill, stroke) = match gs.render {
            0 | 4 => (true, false),
            1 | 5 => (false, true),
            2 | 6 => (true, true),
            _ => (false, false),
        };
        let mut rich = false;
        if fill {
            rich |= self.paint(gs.fill);
        }
        if stroke {
            rich |= self.paint(gs.stroke);
        }
        if rich {
            self.report.rich_black_text = self.report.rich_black_text.saturating_add(1);
        }
    }

    fn content(&mut self, data: &[u8], res: &Rc<Res>, start: Gs, depth: usize) {
        let mut gs = start;
        let mut stack: Vec<Gs> = Vec::new();
        // `q` operators past MAX_STACK are not stacked; their `Q`s must not pop the ones that were.
        let mut overflow = 0usize;
        for op in pdfcraft_content::ops(data) {
            match op.op.as_slice() {
                b"q" => {
                    if stack.len() < MAX_STACK {
                        stack.push(gs.clone());
                    } else {
                        overflow = overflow.saturating_add(1);
                    }
                }
                b"Q" => {
                    if overflow > 0 {
                        overflow -= 1;
                    } else if let Some(prev) = stack.pop() {
                        gs = prev;
                    }
                }
                b"k" => {
                    gs.fill_space = Space::Cmyk;
                    gs.fill = op.nums::<4>().map(|v| v.map(|x| x.clamp(0.0, 1.0)));
                }
                b"K" => {
                    gs.stroke_space = Space::Cmyk;
                    gs.stroke = op.nums::<4>().map(|v| v.map(|x| x.clamp(0.0, 1.0)));
                }
                b"g" | b"rg" => {
                    gs.fill_space = Space::Other;
                    gs.fill = None;
                }
                b"G" | b"RG" => {
                    gs.stroke_space = Space::Other;
                    gs.stroke = None;
                }
                b"cs" | b"CS" => {
                    let selected = op.last_name().map(|n| self.select(n, res)).unwrap_or(OTHER);
                    if op.op.as_slice() == b"cs" {
                        gs.fill_space = selected.space;
                        gs.fill = selected.initial;
                    } else {
                        gs.stroke_space = selected.space;
                        gs.stroke = selected.initial;
                    }
                }
                b"sc" | b"scn" | b"SC" | b"SCN" => {
                    let fill = matches!(op.op.as_slice(), b"sc" | b"scn");
                    let space = if fill { gs.fill_space } else { gs.stroke_space };
                    if space == Space::Cmyk {
                        // The last four operands, as `k` reads them; a pattern name ends the list.
                        if let Some(values) = op.nums::<4>() {
                            let colour = Some(values.map(|x| x.clamp(0.0, 1.0)));
                            if fill {
                                gs.fill = colour;
                            } else {
                                gs.stroke = colour;
                            }
                        }
                    }
                }
                b"Tr" => {
                    if let Some([mode]) = op.nums::<1>() {
                        gs.render = mode as i64;
                    }
                }
                b"gs" => {
                    let Some(state) = op.last_name().and_then(|n| res.states.get(n)).map(|x| self.doc.resolve(x)) else { continue };
                    if let Some(d) = state.as_dict() {
                        let on = |key: &[u8]| matches!(d.get(key).map(|x| self.doc.resolve(x)).as_deref(), Some(Object::Bool(true)));
                        if on(b"OP") || on(b"op") {
                            self.report.overprint = true;
                        }
                    }
                }
                b"f" | b"F" | b"f*" => {
                    self.paint(gs.fill);
                }
                b"S" | b"s" => {
                    self.paint(gs.stroke);
                }
                b"B" | b"B*" | b"b" | b"b*" => {
                    self.paint(gs.fill);
                    self.paint(gs.stroke);
                }
                b"Tj" | b"TJ" | b"'" | b"\"" => self.show_text(&gs),
                b"Do" => {
                    let Some(r) = op.last_name().and_then(|n| res.xobjects.get(n)).and_then(Object::as_ref) else { continue };
                    let obj = self.doc.get(r);
                    let Object::Stream(x) = &*obj else { continue };
                    if x.dict.name(b"Subtype") != Some(b"Form") {
                        continue;
                    }
                    if depth >= MAX_FORM_DEPTH || self.forms.contains(&r) {
                        self.report.incomplete = true;
                        continue;
                    }
                    if !self.budget.enter_form() {
                        self.report.incomplete = true;
                        continue;
                    }
                    let inner_data = match self.budget.decode_with_damage(self.doc, x, usize::MAX) {
                        Ok((d, damaged)) => {
                            self.report.incomplete |= damaged;
                            d
                        }
                        Err(_) => {
                            self.report.incomplete = true;
                            continue;
                        }
                    };
                    // A form without resources of its own reads the caller's. Its own are taken
                    // once, keyed by the shared object when the entry is a reference.
                    let inner = match x.dict.get(b"Resources") {
                        None => Rc::clone(res),
                        Some(entry) => {
                            let key = entry.as_ref().unwrap_or(r);
                            match self.resources.get(&key) {
                                Some(found) => Rc::clone(found),
                                None => {
                                    let made = Rc::new(self.res_of(Some(&*self.doc.resolve(entry))));
                                    self.resources.insert(key, Rc::clone(&made));
                                    made
                                }
                            }
                        }
                    };
                    self.forms.insert(r);
                    self.content(&inner_data, &inner, gs.clone(), depth + 1);
                    self.forms.remove(&r);
                }
                _ => {}
            }
        }
    }
}

/// Whether a colorant name is a spot colour: not None, All or one of DeviceCMYK's.
fn is_spot(name: &[u8]) -> bool {
    name != b"None" && name != b"All" && !pdfcraft_preflight::PROCESS_COLORANTS.contains(&name)
}
