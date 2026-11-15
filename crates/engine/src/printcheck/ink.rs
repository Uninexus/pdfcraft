//! Ink checks for the print-readiness report: total ink, rich black text, overprint and spot
//! colours, read from the vector colours a page's content sets.
//!
//! Only colours set by operators are read: `k`/`K`, and `sc`/`scn` in a DeviceCMYK or four-component
//! ICCBased space. Colours inside images and shadings are not measured, and a colour a
//! form XObject sets is counted where the form is drawn. Verification only: nothing is changed.

use std::collections::{BTreeSet, HashSet};

use pdfcraft_cos::{DecodeBudget, Dict, Document, ObjRef, Object};
use pdfcraft_model::Page;

/// Form XObjects nested deeper than this are not read.
const MAX_FORM_DEPTH: usize = 12;
/// `q` nesting past this is ignored, so a hostile stream can't grow the stack.
const MAX_STACK: usize = 256;

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
    /// Names of the Separation and DeviceN colorants selected on the page.
    pub spot: BTreeSet<String>,
    /// Content past the page's decode budget, or a form nested too deeply, was not read.
    pub incomplete: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Space {
    Cmyk,
    Spot,
    Other,
}

#[derive(Clone, Debug)]
struct Gs {
    fill_space: Space,
    stroke_space: Space,
    fill: Option<[f64; 4]>,
    stroke: Option<[f64; 4]>,
}

impl Default for Gs {
    fn default() -> Self {
        Self { fill_space: Space::Other, stroke_space: Space::Other, fill: None, stroke: None }
    }
}

/// Reads each page in `pages` and reports the ones that set any of what is looked for.
pub(crate) fn ink_pages(doc: &Document, pages: &[Page], limit_percent: f64) -> Vec<InkPage> {
    let mut out = Vec::new();
    for (i, page) in pages.iter().enumerate() {
        let res = page.dict.get(b"Resources").map(|r| doc.resolve(r)).and_then(|r| r.as_dict().cloned()).unwrap_or_default();
        let mut budget = DecodeBudget::page();
        let mut data = Vec::new();
        if let Some(contents) = page.dict.get(b"Contents") {
            for chunk in budget.decode_contents(doc, contents).into_iter().flatten() {
                data.extend_from_slice(&chunk);
                data.push(b' ');
            }
        }
        let mut walk =
            Walk { doc, limit: limit_percent, forms: HashSet::new(), budget, report: InkPage { page: (i + 1) as u32, ..InkPage::default() } };
        if walk.budget.remaining() == 0 {
            walk.report.incomplete = true;
        }
        walk.content(&data, &res, Gs::default(), 0);
        let r = walk.report;
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
    budget: DecodeBudget,
    report: InkPage,
}

impl Walk<'_> {
    fn sub(&self, res: &Dict, key: &[u8]) -> Dict {
        res.get(key).map(|x| self.doc.resolve(x)).and_then(|x| x.as_dict().cloned()).unwrap_or_default()
    }

    /// The space a `cs`/`CS` operand names, and the colorants if it is a spot space.
    fn space(&self, name: &[u8], spaces: &Dict) -> (Space, Vec<String>) {
        match name {
            b"DeviceCMYK" | b"CMYK" => return (Space::Cmyk, Vec::new()),
            b"DeviceGray" | b"G" | b"DeviceRGB" | b"RGB" | b"Pattern" => return (Space::Other, Vec::new()),
            _ => {}
        }
        let Some(value) = spaces.get(name).map(|x| self.doc.resolve(x)) else { return (Space::Other, Vec::new()) };
        if let Some(n) = value.as_name() {
            return if n == b"DeviceCMYK" { (Space::Cmyk, Vec::new()) } else { (Space::Other, Vec::new()) };
        }
        let Some(array) = value.as_array() else { return (Space::Other, Vec::new()) };
        let family = array.first().map(|x| self.doc.resolve(x));
        match family.as_ref().and_then(|x| x.as_name()) {
            Some(b"ICCBased") => {
                let components = array
                    .get(1)
                    .map(|x| self.doc.resolve(x))
                    .and_then(|x| match &*x {
                        Object::Stream(s) => s.dict.int(b"N"),
                        _ => None,
                    })
                    .unwrap_or(0);
                (if components == 4 { Space::Cmyk } else { Space::Other }, Vec::new())
            }
            Some(b"Separation") => {
                let names =
                    array.get(1).map(|x| self.doc.resolve(x)).and_then(|x| x.as_name().map(|n| vec![String::from_utf8_lossy(n).into_owned()]));
                (Space::Spot, names.unwrap_or_default())
            }
            Some(b"DeviceN") => {
                let names = array
                    .get(1)
                    .map(|x| self.doc.resolve(x))
                    .and_then(|x| x.as_array().cloned())
                    .map(|list| {
                        list.iter()
                            .filter_map(|n| self.doc.resolve(n).as_name().map(|n| String::from_utf8_lossy(n).into_owned()))
                            .filter(|n| n != "None")
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                (Space::Spot, names)
            }
            _ => (Space::Other, Vec::new()),
        }
    }

    /// Counts the use of colour `c` for painting; `text` when it is text being shown.
    fn paint(&mut self, c: Option<[f64; 4]>, text: bool) {
        let Some([cy, m, y, k]) = c else { return };
        let total = (cy + m + y + k) * 100.0;
        if total.is_finite() && total > self.report.max_total_percent {
            self.report.max_total_percent = total;
        }
        if total.is_finite() && total > self.limit {
            self.report.over_limit = self.report.over_limit.saturating_add(1);
        }
        if text && k >= 0.9 && cy + m + y >= 0.1 {
            self.report.rich_black_text = self.report.rich_black_text.saturating_add(1);
        }
    }

    fn content(&mut self, data: &[u8], res: &Dict, start: Gs, depth: usize) {
        let (spaces, xobjects, states) = (self.sub(res, b"ColorSpace"), self.sub(res, b"XObject"), self.sub(res, b"ExtGState"));
        let mut gs = start;
        let mut stack: Vec<Gs> = Vec::new();
        for op in pdfcraft_content::ops(data) {
            match op.op.as_slice() {
                b"q" => {
                    if stack.len() < MAX_STACK {
                        stack.push(gs.clone());
                    }
                }
                b"Q" => {
                    if let Some(prev) = stack.pop() {
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
                    let (space, names) = op.name(0).map(|n| self.space(n, &spaces)).unwrap_or((Space::Other, Vec::new()));
                    self.report.spot.extend(names);
                    // The initial colour of DeviceCMYK is black (0 0 0 1).
                    let initial = (space == Space::Cmyk).then_some([0.0, 0.0, 0.0, 1.0]);
                    if op.op.as_slice() == b"cs" {
                        gs.fill_space = space;
                        gs.fill = initial;
                    } else {
                        gs.stroke_space = space;
                        gs.stroke = initial;
                    }
                }
                b"sc" | b"scn" | b"SC" | b"SCN" => {
                    let fill = matches!(op.op.as_slice(), b"sc" | b"scn");
                    let space = if fill { &gs.fill_space } else { &gs.stroke_space };
                    if *space == Space::Cmyk {
                        let nums: Vec<f64> = op.operands.iter().filter_map(Object::as_f64).collect();
                        if let [c, m, y, k] = nums.as_slice() {
                            let colour = Some([*c, *m, *y, *k].map(|x| x.clamp(0.0, 1.0)));
                            if fill {
                                gs.fill = colour;
                            } else {
                                gs.stroke = colour;
                            }
                        }
                    }
                }
                b"gs" => {
                    let Some(state) = op.name(0).and_then(|n| states.get(n)).map(|x| self.doc.resolve(x)) else { continue };
                    if let Some(d) = state.as_dict() {
                        let on = |key: &[u8]| matches!(d.get(key).map(|x| self.doc.resolve(x)).as_deref(), Some(Object::Bool(true)));
                        if on(b"OP") || on(b"op") {
                            self.report.overprint = true;
                        }
                    }
                }
                b"f" | b"F" | b"f*" => self.paint(gs.fill, false),
                b"S" | b"s" => self.paint(gs.stroke, false),
                b"B" | b"B*" | b"b" | b"b*" => {
                    self.paint(gs.fill, false);
                    self.paint(gs.stroke, false);
                }
                b"Tj" | b"TJ" | b"'" | b"\"" => self.paint(gs.fill, true),
                b"Do" => {
                    let Some(r) = op.name(0).and_then(|n| xobjects.get(n)).and_then(Object::as_ref) else { continue };
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
                    let inner_data = match self.budget.decode(x) {
                        Ok(d) => d,
                        Err(_) => {
                            if self.budget.exhausted() {
                                self.report.incomplete = true;
                            }
                            continue;
                        }
                    };
                    self.forms.insert(r);
                    let inner =
                        x.dict.get(b"Resources").map(|o| self.doc.resolve(o)).and_then(|o| o.as_dict().cloned()).unwrap_or_else(|| res.clone());
                    self.content(&inner_data, &inner, gs.clone(), depth + 1);
                    self.forms.remove(&r);
                }
                _ => {}
            }
        }
    }
}
