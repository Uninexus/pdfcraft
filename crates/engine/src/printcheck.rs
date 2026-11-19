//! Print readiness, verification only.
//!
//! Measures what a press looks at and changes nothing: effective image resolution where each
//! image is drawn, the page boxes (media, crop, trim, bleed), the output intent, fonts that
//! aren't embedded, and the existing PDF/A-2b result. There is no fixup and no generated colour
//! profile. A screen profile such as sRGB is reported as a screen profile, not accepted as a
//! press condition. Two thresholds can be set by a caller: the total-ink limit and the least
//! bleed (`print_readiness_with_options`); everything else is fixed.

use pdfcraft_cos::Document;
use pdfcraft_edit::PageImage;
use pdfcraft_preflight::{Issue, Level};

use crate::Document as OpenDocument;

mod ink;

/// Below this, ordinary sheet printing cannot hold the picture. Large-format work sometimes can.
const FAIL_PPI: f64 = 150.0;
/// Ordinary close-up print. Between this and [`FAIL_PPI`] the picture is usable but soft.
const WARN_PPI: f64 = 300.0;
/// Less than this past the trim, on the tightest side, is not a normal 3 mm bleed.
const MIN_BLEED_MM: f64 = 2.0;
/// Total ink (C + M + Y + K, in percent) above which a painted CMYK colour is reported. Press
/// conditions run from about 240% (uncoated) to 340% (coated); 300% is the usual SWOP limit.
const TAC_LIMIT_PERCENT: f64 = 300.0;
/// The limit a caller may set is a percentage of total ink. Below 100 every coloured object is
/// flagged and above 400 (four full inks) nothing can be, so those are mistakes, not settings.
pub const TAC_LIMIT_RANGE: std::ops::RangeInclusive<f64> = 100.0..=400.0;
/// The bleed a caller may ask for, in millimetres past the trim on the tightest side. Zero turns the
/// `page-bleed` finding off for every trimmed page and more than 50 mm is a mistake, not a setting.
pub const MIN_BLEED_RANGE: std::ops::RangeInclusive<f64> = 0.0..=50.0;
/// What the report says it does not check (the text report and the dialog both show it).
pub const NOT_CHECKED: &str =
    "Not checked: PDF/X, ink inside images, shadings, patterns and annotation appearances, overprint effects, transparency flattening.";
/// Spot colour names a finding lists before saying how many more there are.
const MAX_SPOT_LISTED: usize = 8;

/// One thing a printer should see, without the file having been rewritten.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub id: &'static str,
    pub severity: &'static str,
    pub message: String,
    /// 1-based page, when the finding is about one page.
    pub page: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PageBoxes {
    pub page: u32,
    pub media_mm: [f64; 2],
    pub crop_mm: [f64; 2],
    pub trim_mm: [f64; 2],
    pub bleed_mm: [f64; 2],
    pub trim_set: bool,
    pub bleed_set: bool,
    /// Left, bottom, right, top, in millimetres, from the trim out to the bleed.
    pub bleed_margin_mm: [f64; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct DrawnImage {
    pub page: u32,
    pub name: String,
    pub ppi: f64,
    pub pixels: [u32; 2],
    pub size_mm: [f64; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrintReadiness {
    pub pages: Vec<PageBoxes>,
    pub images: Vec<DrawnImage>,
    pub output_intents: Vec<String>,
    pub fonts_not_embedded: Vec<String>,
    pub pdfa_issues: Vec<Issue>,
    pub findings: Vec<Finding>,
    /// Image placements whose resolution couldn't be measured (a degenerate or non-finite
    /// placement matrix). They are neither passed nor failed.
    pub resolution_unknown: usize,
}

impl PrintReadiness {
    /// What was measured, one line each: the output intent, page boxes and bleed, each image's
    /// resolution, fonts that aren't embedded and the PDF/A-2b result. The findings that follow
    /// from it are in [`PrintReadiness::findings`].
    pub fn details(&self) -> String {
        let mut out = String::new();
        if self.output_intents.is_empty() {
            out.push_str("Output intent: none\n");
        } else {
            out.push_str(&format!("Output intent: {}\n", self.output_intents.join(", ")));
        }
        for page in &self.pages {
            if !page.trim_set {
                out.push_str(&format!("Page {}: media {} mm, no TrimBox\n", page.page, size_mm(page.media_mm)));
                continue;
            }
            out.push_str(&format!(
                "Page {}: media {} mm, trim {} mm, bleed margin {} mm{}\n",
                page.page,
                size_mm(page.media_mm),
                size_mm(page.trim_mm),
                min_mm(page.bleed_margin_mm),
                if page.bleed_set { "" } else { " (no BleedBox)" },
            ));
        }
        for image in &self.images {
            out.push_str(&format!(
                "Image {} on page {}: {:.0} ppi, {} × {} px, {} mm\n",
                image.name,
                image.page,
                image.ppi,
                image.pixels[0],
                image.pixels[1],
                size_mm(image.size_mm),
            ));
        }
        if self.resolution_unknown > 0 {
            out.push_str(&format!("Images with unknown resolution: {}\n", self.resolution_unknown));
        }
        if self.fonts_not_embedded.is_empty() {
            out.push_str("Fonts: every referenced font is embedded, or the file has none\n");
        } else {
            out.push_str(&format!("Fonts not embedded: {}\n", self.fonts_not_embedded.join(", ")));
        }
        let level = Level::A2b.label();
        if self.pdfa_issues.is_empty() {
            out.push_str(&format!("{level}: no problems found by the structural check\n"));
        } else {
            out.push_str(&format!("{level}: {} problem{}\n", self.pdfa_issues.len(), if self.pdfa_issues.len() == 1 { "" } else { "s" }));
            for issue in &self.pdfa_issues {
                match issue.page {
                    Some(p) => out.push_str(&format!("- page {}: {}\n", p + 1, issue.message)),
                    None => out.push_str(&format!("- {}\n", issue.message)),
                }
            }
        }
        out
    }

    /// A plain-text report. The document is unchanged.
    pub fn text(&self) -> String {
        let mut out = String::from("Print readiness (verification only; the file was not changed)\n");
        out.push_str(&self.details());
        if self.findings.is_empty() {
            out.push_str("Print checks: no stop for image resolution or bleed\n");
        } else {
            out.push_str("Print checks:\n");
            for finding in &self.findings {
                out.push_str(&format!("- [{}] {}\n", finding.severity, finding.message));
            }
        }
        out.push_str(NOT_CHECKED);
        out.push('\n');
        out
    }
}

/// Refuses a total-ink limit that is not a number within [`TAC_LIMIT_RANGE`] (NaN and infinities included).
fn check_limit(limit_percent: f64) -> Result<(), String> {
    if TAC_LIMIT_RANGE.contains(&limit_percent) {
        return Ok(());
    }
    Err(format!("ink_limit must be a number from {} to {} percent (got {limit_percent}).", TAC_LIMIT_RANGE.start(), TAC_LIMIT_RANGE.end()))
}

/// Refuses a minimum bleed that is not a number within [`MIN_BLEED_RANGE`] (NaN and infinities included).
fn check_min_bleed(min_bleed_mm: f64) -> Result<(), String> {
    if MIN_BLEED_RANGE.contains(&min_bleed_mm) {
        return Ok(());
    }
    Err(format!("min_bleed_mm must be a number from {} to {} millimetres (got {min_bleed_mm}).", MIN_BLEED_RANGE.start(), MIN_BLEED_RANGE.end()))
}

impl OpenDocument {
    /// A print-readiness report. Reads the file and does not edit it.
    pub fn print_readiness(&self) -> PrintReadiness {
        self.print_readiness_with_ink_limit(TAC_LIMIT_PERCENT)
    }

    /// Like [`Self::print_readiness`], with the total-ink limit (percent) the `ink-total` finding
    /// uses instead of 300. Refuses a limit that is not a number within [`TAC_LIMIT_RANGE`].
    /// Retained public API (the tool now calls [`Self::print_readiness_with_options`]).
    pub fn print_readiness_with_limit(&self, limit_percent: f64) -> Result<PrintReadiness, String> {
        check_limit(limit_percent)?;
        Ok(self.print_readiness_with_ink_limit(limit_percent))
    }

    /// Like [`Self::print_readiness`], with either threshold replaced: the total-ink limit
    /// (percent, [`TAC_LIMIT_RANGE`]) and the least bleed (millimetres, [`MIN_BLEED_RANGE`]) the
    /// `page-bleed` finding accepts instead of 2 mm. `None` keeps the default. Refuses a value
    /// outside its range before measuring anything.
    pub fn print_readiness_with_options(&self, ink_limit: Option<f64>, min_bleed_mm: Option<f64>) -> Result<PrintReadiness, String> {
        if let Some(limit) = ink_limit {
            check_limit(limit)?;
        }
        if let Some(bleed) = min_bleed_mm {
            check_min_bleed(bleed)?;
        }
        Ok(self.measure(ink_limit.unwrap_or(TAC_LIMIT_PERCENT), min_bleed_mm))
    }

    fn print_readiness_with_ink_limit(&self, limit_percent: f64) -> PrintReadiness {
        self.measure(limit_percent, None)
    }

    fn measure(&self, limit_percent: f64, min_bleed_mm: Option<f64>) -> PrintReadiness {
        match &self.editor {
            Some(editor) => inspect_with(&editor.cos, limit_percent, min_bleed_mm),
            None => PrintReadiness {
                pages: Vec::new(),
                images: Vec::new(),
                output_intents: Vec::new(),
                fonts_not_embedded: Vec::new(),
                pdfa_issues: Vec::new(),
                resolution_unknown: 0,
                findings: vec![Finding {
                    id: "document",
                    severity: "info",
                    message: "The document has no object graph to measure.".into(),
                    page: None,
                }],
            },
        }
    }
}

#[cfg(test)]
fn inspect(doc: &Document, limit_percent: f64) -> PrintReadiness {
    inspect_with(doc, limit_percent, None)
}

fn inspect_with(doc: &Document, limit_percent: f64, min_bleed_mm: Option<f64>) -> PrintReadiness {
    let min_bleed = min_bleed_mm.unwrap_or(MIN_BLEED_MM);
    let declared = pdfcraft_preflight::declared(doc);
    let pdfa_issues = pdfcraft_preflight::verify(doc, Level::A2b);
    let fonts_not_embedded = pdfa_issues
        .iter()
        .filter(|issue| issue.pattern.contains("isn't embedded") || issue.pattern == "A font isn't embedded")
        .map(|issue| issue.value.clone().unwrap_or_else(|| "unnamed font".into()))
        .collect();
    let model_pages = pdfcraft_model::pages(doc);
    let pages = page_reports(doc, &model_pages);
    let (images, resolution_unknown) = image_reports(doc, &model_pages);
    let mut findings = Vec::new();
    // Without a TrimBox the trim, the bleed and the crop are the same box, so there is no bleed to
    // measure. That is a property of the document, not a fault on every page: report it once.
    let untrimmed = pages.iter().filter(|page| !page.trim_set).count();
    if untrimmed > 0 {
        findings.push(Finding {
            id: "page-trim",
            severity: "info",
            message: if untrimmed == pages.len() {
                "No page has a TrimBox, so bleed can't be measured.".to_string()
            } else {
                format!("{untrimmed} of {} pages have no TrimBox, so their bleed can't be measured.", pages.len())
            },
            page: None,
        });
    }
    for page in pages.iter().filter(|page| page.trim_set) {
        let tight = page.bleed_margin_mm.into_iter().fold(f64::INFINITY, f64::min);
        // Zero means "don't warn": a BleedBox smaller than the TrimBox gives a negative margin, which
        // `tight < 0` would still flag.
        if min_bleed > 0.0 && tight < min_bleed - 1e-6 {
            let want = match min_bleed_mm {
                None => "a press usually wants about 3 mm".to_string(),
                Some(m) => format!("{m} mm was asked for"),
            };
            findings.push(Finding {
                id: "page-bleed",
                severity: "warn",
                message: format!(
                    "Page {} has {:.1} mm of bleed past the trim on its tightest side ({want}). Nothing was added.",
                    page.page,
                    tight.max(0.0)
                ),
                page: Some(page.page),
            });
        }
    }
    for image in &images {
        if image.ppi < FAIL_PPI {
            findings.push(Finding {
                id: "image-ppi",
                severity: "fail",
                message: format!(
                    "Image {} on page {} is drawn at {:.0} ppi, below {:.0}. Enlarging it would not add detail.",
                    image.name, image.page, image.ppi, FAIL_PPI
                ),
                page: Some(image.page),
            });
        } else if image.ppi < WARN_PPI {
            findings.push(Finding {
                id: "image-ppi",
                severity: "warn",
                message: format!(
                    "Image {} on page {} is drawn at {:.0} ppi, below {:.0} for close-up print.",
                    image.name, image.page, image.ppi, WARN_PPI
                ),
                page: Some(image.page),
            });
        }
    }
    if declared.output_intents.iter().any(|intent| is_screen_profile(intent))
        && !declared.output_intents.iter().any(|intent| is_press_profile(intent))
    {
        findings.push(Finding {
            id: "output-intent",
            severity: "warn",
            message: format!(
                "The output intent ({}) is a screen profile. A press condition was not added, because inventing one would retint the file.",
                declared.output_intents.join(", ")
            ),
            page: None,
        });
    }
    if resolution_unknown > 0 {
        findings.push(Finding {
            id: "image-ppi-unknown",
            severity: "info",
            message: format!(
                "The resolution of {resolution_unknown} image placement{} couldn't be measured (its placement matrix is degenerate or not finite).",
                if resolution_unknown == 1 { "" } else { "s" }
            ),
            page: None,
        });
    }
    ink_findings(&ink::ink_pages(doc, &model_pages, limit_percent), limit_percent, &mut findings);
    findings.sort_by(|a, b| a.page.cmp(&b.page).then(a.id.cmp(b.id)).then(a.message.cmp(&b.message)));
    PrintReadiness { pages, images, output_intents: declared.output_intents, fonts_not_embedded, pdfa_issues, findings, resolution_unknown }
}

/// What the ink checks found, as findings. Colours in images and shadings are not measured.
fn ink_findings(pages: &[ink::InkPage], limit_percent: f64, findings: &mut Vec<Finding>) {
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    let mut spot = std::collections::BTreeSet::new();
    let mut capped = false;
    for page in pages {
        if page.over_limit > 0 {
            findings.push(Finding {
                id: "ink-total",
                severity: "warn",
                message: format!(
                    "Page {} paints {} object{} with more than {:.0}% total ink (up to {:.0}%). Press conditions usually allow 240% to 340%. Colours inside images, shadings and patterns were not measured.",
                    page.page,
                    page.over_limit,
                    plural(page.over_limit),
                    limit_percent,
                    page.max_total_percent
                ),
                page: Some(page.page),
            });
        }
        if page.rich_black_text > 0 {
            findings.push(Finding {
                id: "ink-rich-black-text",
                severity: "info",
                message: format!(
                    "Page {} shows text {} time{} in rich black (K 90% or more with other inks). Small rich-black text can misregister on press.",
                    page.page,
                    page.rich_black_text,
                    plural(page.rich_black_text)
                ),
                page: Some(page.page),
            });
        }
        if page.overprint {
            findings.push(Finding {
                id: "ink-overprint",
                severity: "info",
                message: format!("Page {} turns overprint on. Overprint is not previewed here, so check it on a proof.", page.page),
                page: Some(page.page),
            });
        }
        if page.incomplete {
            findings.push(Finding {
                id: "ink-incomplete",
                severity: "info",
                message: format!("Page {}: some content could not be read in full (damaged, over the read limit, or nested too deeply), so its colours may be missing.", page.page),
                page: Some(page.page),
            });
        }
        spot.extend(page.spot.iter().cloned());
        capped |= page.spot_capped;
    }
    if !spot.is_empty() {
        let names: Vec<String> = spot.into_iter().collect();
        let shown = names.iter().take(MAX_SPOT_LISTED).cloned().collect::<Vec<_>>().join(", ");
        let more = if capped {
            ", and more".to_string()
        } else if names.len() > MAX_SPOT_LISTED {
            format!(", and {} more", names.len() - MAX_SPOT_LISTED)
        } else {
            String::new()
        };
        findings.push(Finding {
            id: "ink-spot",
            severity: "info",
            message: format!("Spot colour{} selected: {shown}{more}. They print on their own plates unless converted.", plural(names.len())),
            page: None,
        });
    }
}

fn page_reports(doc: &Document, pages: &[pdfcraft_model::Page]) -> Vec<PageBoxes> {
    let boxes = pdfcraft_organize::page_boxes(doc).unwrap_or_default();
    pages
        .iter()
        .enumerate()
        .map(|(i, page)| {
            let row = boxes.get(i).copied().unwrap_or([[0.0; 4]; 5]);
            let trim = row[3];
            let bleed = row[2];
            // The boxes are in user space; a `/UserUnit` page counts each unit as that many points.
            let u = page.user_unit(doc);
            let (pt_mm, rect_mm) = (|points: f64| mm(points * u), |rect: [f64; 4]| wh_mm(rect).map(|v| v * u));
            PageBoxes {
                page: (i + 1) as u32,
                media_mm: rect_mm(row[0]),
                crop_mm: rect_mm(row[1]),
                trim_mm: rect_mm(trim),
                bleed_mm: rect_mm(bleed),
                trim_set: page.dict.contains(b"TrimBox"),
                bleed_set: page.dict.contains(b"BleedBox"),
                bleed_margin_mm: [pt_mm(trim[0] - bleed[0]), pt_mm(trim[1] - bleed[1]), pt_mm(bleed[2] - trim[2]), pt_mm(bleed[3] - trim[3])],
            }
        })
        .collect()
}

/// The images drawn on each page, and how many placements had no measurable resolution.
fn image_reports(doc: &Document, pages: &[pdfcraft_model::Page]) -> (Vec<DrawnImage>, usize) {
    let mut out = Vec::new();
    let mut unknown = 0usize;
    let mut seen = std::collections::HashSet::new();
    for (i, page) in pages.iter().enumerate() {
        let page_no = (i + 1) as u32;
        if let Ok(placed) = pdfcraft_edit::page_images(doc, i) {
            for image in placed {
                if let Some(ppi) = placed_ppi(&image) {
                    if let Some(object) = image.object {
                        seen.insert((page_no, object));
                    }
                    if !ppi.is_finite() {
                        unknown = unknown.saturating_add(1);
                        continue;
                    }
                    let w = (image.rect[2] - image.rect[0]).abs();
                    let h = (image.rect[3] - image.rect[1]).abs();
                    out.push(DrawnImage { page: page_no, name: image.name, ppi, pixels: [image.width, image.height], size_mm: [mm(w), mm(h)] });
                }
            }
        }
        // Images drawn inside a form XObject are not listed above. Their resolution is the
        // smallest use on this page, which is what the optimizer already measures.
        for (object, ppi) in pdfcraft_optimize::effective_resolutions(doc, &[page.obj]) {
            if seen.contains(&(page_no, object)) {
                continue;
            }
            if !ppi.is_finite() {
                unknown = unknown.saturating_add(1);
                continue;
            }
            out.push(DrawnImage { page: page_no, name: format!("object {object}"), ppi, pixels: [0, 0], size_mm: [0.0, 0.0] });
        }
    }
    out.sort_by(|a, b| a.page.cmp(&b.page).then(a.name.cmp(&b.name)).then(a.ppi.total_cmp(&b.ppi)));
    (out, unknown)
}

/// The effective resolution of a raster placement, `None` for Form artwork. Not finite (NaN)
/// when the placement can't be measured: callers report that rather than pass it.
fn placed_ppi(image: &PageImage) -> Option<f64> {
    if image.is_form {
        return None;
    }
    Some(matrix_ppi(image.matrix, image.width, image.height))
}

/// Pixels per inch of a `width` × `height` image drawn through `matrix`, or NaN when the matrix
/// is degenerate or not finite or the image has no pixels.
fn matrix_ppi(matrix: [f64; 6], width: u32, height: u32) -> f64 {
    let [a, b, c, d, ..] = matrix;
    let (sx, sy) = (a.hypot(b), c.hypot(d));
    if width == 0 || height == 0 || !sx.is_finite() || !sy.is_finite() || sx <= 1e-6 || sy <= 1e-6 {
        return f64::NAN;
    }
    let ppi = (f64::from(width) / (sx / 72.0)).min(f64::from(height) / (sy / 72.0));
    if ppi.is_finite() { ppi } else { f64::NAN }
}

fn is_screen_profile(intent: &str) -> bool {
    let id = intent.to_ascii_lowercase();
    id.contains("srgb") || id.contains("adobe rgb") || id.contains("display")
}

fn is_press_profile(intent: &str) -> bool {
    let id = intent.to_ascii_lowercase();
    id.contains("cmyk") || id.contains("fogra") || id.contains("coated") || id.contains("swop") || id.contains("gracol")
}

fn mm(points: f64) -> f64 {
    points * 25.4 / 72.0
}

fn wh_mm(rect: [f64; 4]) -> [f64; 2] {
    [mm((rect[2] - rect[0]).abs()), mm((rect[3] - rect[1]).abs())]
}

fn size_mm(wh: [f64; 2]) -> String {
    format!("{:.1} × {:.1}", wh[0], wh[1])
}

fn min_mm(sides: [f64; 4]) -> String {
    let n = sides.into_iter().fold(f64::INFINITY, f64::min);
    if n.is_finite() { format!("{n:.1}") } else { "0.0".into() }
}

#[cfg(test)]
mod tests {
    use pdfcraft_cos::{Dict, Document, Object, Stream};

    use super::*;

    fn image(doc: &mut Document, w: u32, h: u32) -> pdfcraft_cos::ObjRef {
        let mut d = Dict::new();
        d.set(b"Type".to_vec(), Object::name("XObject"));
        d.set(b"Subtype".to_vec(), Object::name("Image"));
        d.set(b"Width".to_vec(), Object::Int(w as i64));
        d.set(b"Height".to_vec(), Object::Int(h as i64));
        d.set(b"BitsPerComponent".to_vec(), Object::Int(8));
        d.set(b"ColorSpace".to_vec(), Object::name("DeviceGray"));
        doc.add(Object::Stream(Stream::flate(d, &vec![0; (w as usize) * (h as usize)])))
    }

    fn page(doc: &mut Document, name: &str, object: pdfcraft_cos::ObjRef, content: &str) -> pdfcraft_cos::ObjRef {
        let pages = doc.root().and_then(|r| doc.get(r).as_dict().and_then(|d| d.reference(b"Pages"))).unwrap();
        let mut x = Dict::new();
        x.set(name.as_bytes().to_vec(), Object::Ref(object));
        let mut res = Dict::new();
        res.set(b"XObject".to_vec(), Object::Dict(x));
        let contents = doc.add(Object::Stream(Stream::from_raw(Dict::new(), content.as_bytes().to_vec())));
        let mut p = Dict::new();
        p.set(b"Type".to_vec(), Object::name("Page"));
        p.set(b"Parent".to_vec(), Object::Ref(pages));
        p.set(b"MediaBox".to_vec(), Object::Array(vec![0.into(), 0.into(), 612.into(), 792.into()]));
        p.set(b"Resources".to_vec(), Object::Dict(res));
        p.set(b"Contents".to_vec(), Object::Ref(contents));
        let r = doc.add(Object::Dict(p));
        doc.update_dict(pages, |d| {
            let mut kids = d.get(b"Kids").and_then(|k| k.as_array().cloned()).unwrap_or_default();
            kids.push(Object::Ref(r));
            d.set(b"Count".to_vec(), Object::Int(kids.len() as i64));
            d.set(b"Kids".to_vec(), Object::Array(kids));
        })
        .unwrap();
        r
    }

    #[test]
    fn a_drawn_image_is_measured_where_it_is_placed() {
        let mut doc = Document::new_empty();
        // 100 px across 144 pt (2 in) is 50 ppi, not "100 px on a letter page".
        let soft = image(&mut doc, 100, 100);
        // 600 px across 72 pt (1 in) is 600 ppi.
        let sharp = image(&mut doc, 600, 600);
        page(&mut doc, "Soft", soft, "q 144 0 0 144 36 600 cm /Soft Do Q");
        let sharp_page = page(&mut doc, "Sharp", sharp, "q 72 0 0 72 36 36 cm /Sharp Do Q");
        doc.update_dict(sharp_page, |d| {
            let inset = |n: i64| Object::Array(vec![n.into(), n.into(), (612 - n).into(), (792 - n).into()]);
            d.set(b"TrimBox".to_vec(), inset(18));
            d.set(b"BleedBox".to_vec(), inset(0));
        })
        .unwrap();

        let report = inspect(&doc, TAC_LIMIT_PERCENT);
        let soft = report.images.iter().find(|image| image.name == "Soft").unwrap();
        let sharp = report.images.iter().find(|image| image.name == "Sharp").unwrap();
        assert!((soft.ppi - 50.0).abs() < 0.5, "{}", soft.ppi);
        assert!((sharp.ppi - 600.0).abs() < 0.5, "{}", sharp.ppi);
        assert!(report.findings.iter().any(|f| f.id == "image-ppi" && f.severity == "fail" && f.page == Some(1)));
        assert!(!report.findings.iter().any(|f| f.id == "image-ppi" && f.page == Some(2)));
        // Page 1 has no TrimBox: one document-level note, not a bleed warning per page.
        assert!(!report.findings.iter().any(|f| f.id == "page-bleed"), "{:?}", report.findings);
        let trim: Vec<_> = report.findings.iter().filter(|f| f.id == "page-trim").collect();
        assert_eq!(trim.len(), 1, "{:?}", report.findings);
        assert_eq!((trim[0].severity, trim[0].page), ("info", None));
        assert!(trim[0].message.starts_with("1 of 2 pages"), "{}", trim[0].message);
        assert_eq!(report.resolution_unknown, 0);
        assert!(report.text().contains("50 ppi"));
        assert!(report.text().contains("verification only"));
    }

    #[test]
    fn box_sizes_and_bleed_follow_the_pages_user_unit() {
        let mut doc = Document::new_empty();
        let img = image(&mut doc, 600, 600);
        let p = page(&mut doc, "Im", img, "q 72 0 0 72 36 36 cm /Im Do Q");
        doc.update_dict(p, |d| {
            d.set(b"UserUnit".to_vec(), Object::Int(2));
            d.set(b"TrimBox".to_vec(), Object::Array(vec![1.into(), 1.into(), 611.into(), 791.into()]));
        })
        .unwrap();
        let page = &inspect(&doc, TAC_LIMIT_PERCENT).pages[0];
        // 612 x 792 user units of 2 points each.
        assert!((page.media_mm[0] - 612.0 * 2.0 * 25.4 / 72.0).abs() < 1e-9, "{:?}", page.media_mm);
        assert!((page.bleed_margin_mm[0] - 2.0 * 25.4 / 72.0).abs() < 1e-9, "{:?}", page.bleed_margin_mm);
    }

    #[test]
    fn untrimmed_pages_get_one_note_and_a_tight_trim_gets_a_warning() {
        let mut doc = Document::new_empty();
        let img = image(&mut doc, 600, 600);
        for _ in 0..3 {
            page(&mut doc, "Im", img, "q 72 0 0 72 36 36 cm /Im Do Q");
        }
        let report = inspect(&doc, TAC_LIMIT_PERCENT);
        let notes: Vec<_> = report.findings.iter().filter(|f| f.id.starts_with("page-")).collect();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert_eq!(notes[0].message, "No page has a TrimBox, so bleed can't be measured.");

        // A TrimBox 1 pt inside the crop, with no BleedBox: 0.35 mm of bleed is a real warning.
        let tight = page(&mut doc, "Im", img, "q 72 0 0 72 36 36 cm /Im Do Q");
        doc.update_dict(tight, |d| d.set(b"TrimBox".to_vec(), Object::Array(vec![1.into(), 1.into(), 611.into(), 791.into()]))).unwrap();
        let report = inspect(&doc, TAC_LIMIT_PERCENT);
        assert!(report.findings.iter().any(|f| f.id == "page-bleed" && f.severity == "warn" && f.page == Some(4)));
        assert_eq!(report.findings.iter().filter(|f| f.id == "page-bleed").count(), 1);
    }

    #[test]
    fn the_least_bleed_is_settable_and_bad_values_are_refused() {
        // A 1 pt margin is 0.35 mm: flagged by default, silent when 0.3 is enough, and flagged
        // when 5 mm is asked for. Zero turns the finding off, even for a BleedBox inside the TrimBox.
        let mut doc = Document::new_empty();
        let img = image(&mut doc, 600, 600);
        let p = page(&mut doc, "Im", img, "q 72 0 0 72 36 36 cm /Im Do Q");
        doc.update_dict(p, |d| d.set(b"TrimBox".to_vec(), Object::Array(vec![1.into(), 1.into(), 611.into(), 791.into()]))).unwrap();
        let bleed = |r: &PrintReadiness| r.findings.iter().filter(|f| f.id == "page-bleed").map(|f| f.message.clone()).collect::<Vec<_>>();
        assert_eq!(bleed(&inspect(&doc, TAC_LIMIT_PERCENT)).len(), 1);
        assert!(bleed(&inspect_with(&doc, TAC_LIMIT_PERCENT, Some(0.3))).is_empty());
        assert!(bleed(&inspect_with(&doc, TAC_LIMIT_PERCENT, Some(0.0))).is_empty());
        let strict = bleed(&inspect_with(&doc, TAC_LIMIT_PERCENT, Some(5.0)));
        assert!(strict.len() == 1 && strict[0].contains("5 mm was asked for") && !strict[0].contains("about 3 mm"), "{strict:?}");
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.1, 50.1] {
            let e = check_min_bleed(bad).expect_err("refused");
            assert!(e.contains("min_bleed_mm") && e.contains("50"), "{e}");
        }
        // A BleedBox smaller than the TrimBox is a negative margin: still no warning at 0.
        let mut odd = Document::new_empty();
        let img2 = image(&mut odd, 600, 600);
        let q = page(&mut odd, "Im", img2, "q 72 0 0 72 36 36 cm /Im Do Q");
        odd.update_dict(q, |d| {
            d.set(b"MediaBox".to_vec(), Object::Array(vec![0.into(), 0.into(), 200.into(), 200.into()]));
            d.set(b"TrimBox".to_vec(), Object::Array(vec![0.into(), 0.into(), 200.into(), 200.into()]));
            d.set(b"BleedBox".to_vec(), Object::Array(vec![10.into(), 10.into(), 190.into(), 190.into()]));
        })
        .unwrap();
        assert!(bleed(&inspect_with(&odd, TAC_LIMIT_PERCENT, Some(0.0))).is_empty());
        assert_eq!(bleed(&inspect_with(&odd, TAC_LIMIT_PERCENT, Some(1.0))).len(), 1);
        assert!(check_min_bleed(0.0).is_ok() && check_min_bleed(50.0).is_ok());
    }

    #[test]
    fn an_unmeasurable_placement_is_never_passed() {
        assert!(matrix_ppi([f64::NAN, 0.0, 0.0, 72.0, 0.0, 0.0], 600, 600).is_nan());
        assert!(matrix_ppi([f64::INFINITY, 0.0, 0.0, 72.0, 0.0, 0.0], 600, 600).is_nan());
        assert!(matrix_ppi([0.0, 0.0, 0.0, 72.0, 0.0, 0.0], 600, 600).is_nan());
        assert!(matrix_ppi([72.0, 0.0, 0.0, 72.0, 0.0, 0.0], 0, 600).is_nan());
        assert!((matrix_ppi([72.0, 0.0, 0.0, 72.0, 0.0, 0.0], 600, 600) - 600.0).abs() < 1e-9);

        // A placement scaled past f64's range and then multiplied by zero has a NaN matrix. The
        // content readers drop it before it is measured; it must never be reported as passing.
        let mut doc = Document::new_empty();
        let img = image(&mut doc, 100, 100);
        let huge = format!("1{}", "0".repeat(400));
        page(&mut doc, "Im", img, &format!("q {huge} 0 0 {huge} 0 0 cm 0 1 1 0 0 0 cm /Im Do Q"));
        let report = inspect(&doc, TAC_LIMIT_PERCENT);
        assert!(report.images.iter().all(|image| image.ppi.is_finite()), "{:?}", report.images);
        assert!(!report.findings.iter().any(|f| f.id == "image-ppi"), "{:?}", report.findings);
    }

    /// A page with the given resources and content, for the ink checks.
    fn ink_page(doc: &mut Document, resources: Dict, content: &str) {
        let pages = doc.root().and_then(|r| doc.get(r).as_dict().and_then(|d| d.reference(b"Pages"))).unwrap();
        let contents = doc.add(Object::Stream(Stream::from_raw(Dict::new(), content.as_bytes().to_vec())));
        let mut p = Dict::new();
        p.set(b"Type".to_vec(), Object::name("Page"));
        p.set(b"Parent".to_vec(), Object::Ref(pages));
        p.set(b"MediaBox".to_vec(), Object::Array(vec![0.into(), 0.into(), 612.into(), 792.into()]));
        p.set(b"Resources".to_vec(), Object::Dict(resources));
        p.set(b"Contents".to_vec(), Object::Ref(contents));
        let r = doc.add(Object::Dict(p));
        doc.update_dict(pages, |d| {
            let mut kids = d.get(b"Kids").and_then(|k| k.as_array().cloned()).unwrap_or_default();
            kids.push(Object::Ref(r));
            d.set(b"Count".to_vec(), Object::Int(kids.len() as i64));
            d.set(b"Kids".to_vec(), Object::Array(kids));
        })
        .unwrap();
    }

    fn ink_ids(report: &PrintReadiness, page: Option<u32>) -> Vec<&'static str> {
        report.findings.iter().filter(|f| f.id.starts_with("ink-") && f.page == page).map(|f| f.id).collect()
    }

    #[test]
    fn total_ink_over_the_limit_is_a_warning_and_under_it_is_silent() {
        let mut doc = Document::new_empty();
        // 90 + 80 + 70 + 90 = 330%.
        ink_page(&mut doc, Dict::new(), "0.9 0.8 0.7 0.9 k 10 10 100 100 re f 0.9 0.8 0.7 0.9 K 10 10 50 50 re S");
        // 60%: fine.
        ink_page(&mut doc, Dict::new(), "0.2 0.2 0.1 0.1 k 10 10 100 100 re f");
        // A colour set inside q ... Q is gone after Q: nothing is painted in CMYK afterwards.
        ink_page(&mut doc, Dict::new(), "q 1 1 1 1 k Q 10 10 100 100 re f");
        let report = inspect(&doc, TAC_LIMIT_PERCENT);
        let first: Vec<_> = report.findings.iter().filter(|f| f.id == "ink-total" && f.page == Some(1)).collect();
        assert_eq!(first.len(), 1, "{:?}", report.findings);
        assert_eq!(first[0].severity, "warn");
        assert!(first[0].message.contains("2 objects") && first[0].message.contains("up to 330%"), "{}", first[0].message);
        assert!(ink_ids(&report, Some(2)).is_empty() && ink_ids(&report, Some(3)).is_empty(), "{:?}", report.findings);
        assert!(report.text().contains("ink inside images, shadings, patterns and annotation appearances"));
    }

    #[test]
    fn a_lower_limit_flags_what_the_default_lets_through() {
        let mut doc = Document::new_empty();
        // 60 + 60 + 60 + 60 = 240%: silent at 300, flagged at 200.
        ink_page(&mut doc, Dict::new(), "0.6 0.6 0.6 0.6 k 10 10 100 100 re f");
        assert!(!inspect(&doc, TAC_LIMIT_PERCENT).findings.iter().any(|f| f.id == "ink-total"));
        let low = inspect(&doc, 200.0);
        let f = low.findings.iter().find(|f| f.id == "ink-total").expect("flagged at 200");
        assert!(f.message.contains("more than 200% total ink") && f.message.contains("up to 240%"), "{}", f.message);
        assert!(!inspect(&doc, 240.0).findings.iter().any(|f| f.id == "ink-total"), "equal is not over");
    }

    #[test]
    fn a_limit_outside_100_to_400_or_not_a_number_is_refused() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 99.9, 400.1, -1.0, 0.0] {
            let e = check_limit(bad).expect_err("refused");
            assert!(e.contains("ink_limit") && e.contains("100") && e.contains("400"), "{e}");
        }
        assert!(check_limit(100.0).is_ok() && check_limit(400.0).is_ok() && check_limit(300.0).is_ok());
    }

    #[test]
    fn rich_black_text_is_reported_and_pure_black_text_is_not() {
        let mut doc = Document::new_empty();
        ink_page(&mut doc, Dict::new(), "BT 0.6 0.4 0.4 1 k (Rich) Tj ET BT 0 0 0 1 k (Plain) Tj ET");
        let report = inspect(&doc, TAC_LIMIT_PERCENT);
        let rich: Vec<_> = report.findings.iter().filter(|f| f.id == "ink-rich-black-text").collect();
        assert_eq!(rich.len(), 1, "{:?}", report.findings);
        assert!(rich[0].message.contains("1 time in rich black"), "{}", rich[0].message);
        // 140% total: no total-ink warning for it.
        assert!(!report.findings.iter().any(|f| f.id == "ink-total"), "{:?}", report.findings);
    }

    #[test]
    fn overprint_and_spot_colours_are_noted() {
        let mut doc = Document::new_empty();
        let mut state = Dict::new();
        state.set(b"OP".to_vec(), Object::Bool(true));
        let mut states = Dict::new();
        states.set(b"On".to_vec(), Object::Dict(state));
        let mut spaces = Dict::new();
        spaces.set(
            b"Gold".to_vec(),
            Object::Array(vec![Object::name("Separation"), Object::name("Pantone Gold"), Object::name("DeviceCMYK"), Object::Dict(Dict::new())]),
        );
        let mut res = Dict::new();
        res.set(b"ExtGState".to_vec(), Object::Dict(states));
        res.set(b"ColorSpace".to_vec(), Object::Dict(spaces));
        ink_page(&mut doc, res, "/On gs /Gold cs 1 scn 10 10 100 100 re f");
        let report = inspect(&doc, TAC_LIMIT_PERCENT);
        assert_eq!(ink_ids(&report, Some(1)), vec!["ink-overprint"], "{:?}", report.findings);
        let spot: Vec<_> = report.findings.iter().filter(|f| f.id == "ink-spot").collect();
        assert_eq!(spot.len(), 1, "{:?}", report.findings);
        assert_eq!(spot[0].page, None);
        assert!(spot[0].message.contains("Pantone Gold"), "{}", spot[0].message);
        // A spot tint is not CMYK ink: no total-ink finding.
        assert!(!report.findings.iter().any(|f| f.id == "ink-total"));
    }

    #[test]
    fn a_colour_a_form_sets_is_counted_where_it_is_drawn_and_a_self_drawing_form_is_flagged() {
        let mut doc = Document::new_empty();
        let mut fd = Dict::new();
        fd.set(b"Subtype".to_vec(), Object::name("Form"));
        fd.set(b"BBox".to_vec(), Object::Array(vec![0.into(), 0.into(), 100.into(), 100.into()]));
        let inner = doc.add(Object::Stream(Stream::from_raw(fd.clone(), b"1 1 1 1 k 0 0 50 50 re f".to_vec())));
        // A form that draws itself: bounded, and reported as not fully read.
        let looping = doc.add(Object::Stream(Stream::from_raw(fd, b"/Self Do".to_vec())));
        let mut self_res = Dict::new();
        let mut xs = Dict::new();
        xs.set(b"Self".to_vec(), Object::Ref(looping));
        self_res.set(b"XObject".to_vec(), Object::Dict(xs));
        doc.update_dict(looping, |d| d.set(b"Resources".to_vec(), Object::Dict(self_res))).unwrap();
        let mut xs = Dict::new();
        xs.set(b"Art".to_vec(), Object::Ref(inner));
        xs.set(b"Self".to_vec(), Object::Ref(looping));
        let mut res = Dict::new();
        res.set(b"XObject".to_vec(), Object::Dict(xs));
        ink_page(&mut doc, res, "/Art Do /Self Do");
        let report = inspect(&doc, TAC_LIMIT_PERCENT);
        assert!(report.findings.iter().any(|f| f.id == "ink-total" && f.page == Some(1)), "{:?}", report.findings);
        assert!(report.findings.iter().any(|f| f.id == "ink-incomplete" && f.page == Some(1)), "{:?}", report.findings);
    }

    #[test]
    fn invisible_text_is_not_counted_and_the_stroking_modes_use_the_stroke_colour() {
        let mut doc = Document::new_empty();
        // Tr 3 is the OCR text layer of a scan: nothing is painted. Tr is saved by q and Q.
        ink_page(&mut doc, Dict::new(), "BT 3 Tr 0.9 0.8 0.7 0.9 k (hidden) Tj ET");
        ink_page(&mut doc, Dict::new(), "q 3 Tr Q BT 0.6 0.4 0.4 1 k (shown) Tj ET");
        // Tr 1 strokes: the fill colour is not what is painted.
        ink_page(&mut doc, Dict::new(), "BT 1 Tr 0 0 0 0 k 0.9 0.8 0.7 0.9 K (outlined) Tj ET");
        ink_page(&mut doc, Dict::new(), "BT 7 Tr 0.9 0.8 0.7 0.9 k (clip only) Tj ET");
        let report = inspect(&doc, TAC_LIMIT_PERCENT);
        assert!(ink_ids(&report, Some(1)).is_empty(), "{:?}", report.findings);
        assert_eq!(ink_ids(&report, Some(2)), vec!["ink-rich-black-text"], "{:?}", report.findings);
        // The stroke colour is what is painted: 330% total, and rich black (K 90% with other inks).
        assert_eq!(ink_ids(&report, Some(3)), vec!["ink-rich-black-text", "ink-total"], "{:?}", report.findings);
        assert!(ink_ids(&report, Some(4)).is_empty(), "{:?}", report.findings);
    }

    fn device_n(names: Vec<Object>) -> Object {
        Object::Array(vec![Object::name("DeviceN"), Object::Array(names), Object::name("DeviceCMYK"), Object::Dict(Dict::new())])
    }

    fn spaces_page(doc: &mut Document, spaces: Vec<(&str, Object)>, content: &str) {
        let mut dict = Dict::new();
        for (name, space) in spaces {
            dict.set(name.as_bytes().to_vec(), space);
        }
        let mut res = Dict::new();
        res.set(b"ColorSpace".to_vec(), Object::Dict(dict));
        ink_page(doc, res, content);
    }

    #[test]
    fn process_and_special_colorants_are_not_spot_colours() {
        let mut doc = Document::new_empty();
        let sep = |n: &str| Object::Array(vec![Object::name("Separation"), Object::name(n), Object::name("DeviceCMYK"), Object::Dict(Dict::new())]);
        spaces_page(
            &mut doc,
            vec![
                ("All", sep("All")),
                ("None", sep("None")),
                ("Process", device_n(["Cyan", "Magenta", "Yellow", "Black"].map(Object::name).to_vec())),
                ("Mixed", device_n(["Cyan", "PANTONE 185 C", "None"].map(Object::name).to_vec())),
            ],
            "/All cs /None cs /Process cs /Mixed cs",
        );
        let report = inspect(&doc, TAC_LIMIT_PERCENT);
        let spot: Vec<_> = report.findings.iter().filter(|f| f.id == "ink-spot").collect();
        assert_eq!(spot.len(), 1, "{:?}", report.findings);
        assert!(spot[0].message.contains("PANTONE 185 C") && !spot[0].message.contains("Cyan"), "{}", spot[0].message);
        assert!(spot[0].message.starts_with("Spot colour selected:"), "{}", spot[0].message);
    }

    #[test]
    fn a_huge_devicen_list_is_capped_in_the_report() {
        let mut doc = Document::new_empty();
        let names: Vec<Object> = (0..5_000).map(|i| Object::name(&format!("Spot{i:05}{}", "x".repeat(300)))).collect();
        spaces_page(&mut doc, vec![("Many", device_n(names))], "/Many cs");
        let report = inspect(&doc, TAC_LIMIT_PERCENT);
        let spot = report.findings.iter().find(|f| f.id == "ink-spot").expect("a spot finding");
        assert!(spot.message.len() < 1_500, "{} bytes", spot.message.len());
        assert!(spot.message.contains(", and "), "{}", spot.message);
    }

    #[test]
    fn an_icc_cmyk_space_starts_at_no_ink_and_reads_n_given_by_reference() {
        let mut doc = Document::new_empty();
        let n = doc.add(Object::Int(4));
        let mut profile = Dict::new();
        profile.set(b"N".to_vec(), Object::Ref(n));
        let icc = doc.add(Object::Stream(Stream::from_raw(profile, vec![0; 8])));
        let space = Object::Array(vec![Object::name("ICCBased"), Object::Ref(icc)]);
        // No colour set yet: ICC CMYK starts at 0 0 0 0, so text in it is not rich black.
        spaces_page(&mut doc, vec![("Icc", space.clone())], "/Icc cs BT (plain) Tj ET");
        // A colour set with scn in the space is read.
        spaces_page(&mut doc, vec![("Icc", space)], "/Icc cs 0.9 0.8 0.7 0.9 scn 0 0 10 10 re f");
        let report = inspect(&doc, TAC_LIMIT_PERCENT);
        assert!(ink_ids(&report, Some(1)).is_empty(), "{:?}", report.findings);
        assert_eq!(ink_ids(&report, Some(2)), vec!["ink-total"], "{:?}", report.findings);
    }

    #[test]
    fn forms_nested_too_deeply_are_flagged_and_do_not_overflow() {
        let mut doc = Document::new_empty();
        let mut form = Dict::new();
        form.set(b"Subtype".to_vec(), Object::name("Form"));
        form.set(b"BBox".to_vec(), Object::Array(vec![0.into(), 0.into(), 10.into(), 10.into()]));
        let mut inner = doc.add(Object::Stream(Stream::from_raw(form.clone(), b"0.9 0.8 0.7 0.9 k 0 0 5 5 re f".to_vec())));
        for _ in 0..20 {
            let mut xs = Dict::new();
            xs.set(b"X".to_vec(), Object::Ref(inner));
            let mut res = Dict::new();
            res.set(b"XObject".to_vec(), Object::Dict(xs));
            let mut d = form.clone();
            d.set(b"Resources".to_vec(), Object::Dict(res));
            inner = doc.add(Object::Stream(Stream::from_raw(d, b"/X Do".to_vec())));
        }
        let mut xs = Dict::new();
        xs.set(b"X".to_vec(), Object::Ref(inner));
        let mut res = Dict::new();
        res.set(b"XObject".to_vec(), Object::Dict(xs));
        ink_page(&mut doc, res, "/X Do");
        let report = inspect(&doc, TAC_LIMIT_PERCENT);
        assert!(report.findings.iter().any(|f| f.id == "ink-incomplete" && f.page == Some(1)), "{:?}", report.findings);
        assert!(!report.findings.iter().any(|f| f.id == "ink-total"), "the colour is past the depth limit: {:?}", report.findings);
    }

    /// A page drawing one form thousands of times, with a large XObject dictionary shared by form
    /// and page, reads that dictionary once rather than once per drawing.
    #[test]
    fn many_form_drawings_with_a_big_resource_dictionary_stay_fast() {
        let mut doc = Document::new_empty();
        let mut xs = Dict::new();
        for i in 0..3_000 {
            xs.set(format!("Filler{i}").into_bytes(), Object::Int(i));
        }
        let mut form = Dict::new();
        form.set(b"Subtype".to_vec(), Object::name("Form"));
        form.set(b"BBox".to_vec(), Object::Array(vec![0.into(), 0.into(), 10.into(), 10.into()]));
        let shared = doc.add(Object::Dict({
            let mut r = Dict::new();
            r.set(b"XObject".to_vec(), Object::Dict(xs.clone()));
            r
        }));
        form.set(b"Resources".to_vec(), Object::Ref(shared));
        let f = doc.add(Object::Stream(Stream::from_raw(form, b"0.9 0.8 0.7 0.9 k 0 0 5 5 re f".to_vec())));
        xs.set(b"F".to_vec(), Object::Ref(f));
        let mut res = Dict::new();
        res.set(b"XObject".to_vec(), Object::Dict(xs));
        let content = "/F Do ".repeat(20_000);
        ink_page(&mut doc, res, &content);
        let started = std::time::Instant::now();
        let report = inspect(&doc, TAC_LIMIT_PERCENT);
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
        assert!(report.findings.iter().any(|f| f.id == "ink-total"), "{:?}", report.findings);
    }

    #[test]
    fn q_nesting_past_the_cap_does_not_unbalance_the_pops() {
        let mut doc = Document::new_empty();
        // 300 q (44 past the cap), a colour, 300 Q: the colour is set inside the stack, so after
        // the last Q nothing paints with it.
        let content = format!("{} 0.9 0.8 0.7 0.9 k {} 10 10 5 5 re f", "q ".repeat(300), "Q ".repeat(300));
        ink_page(&mut doc, Dict::new(), &content);
        let report = inspect(&doc, TAC_LIMIT_PERCENT);
        assert!(ink_ids(&report, Some(1)).is_empty(), "the colour was restored away: {:?}", report.findings);
        let inside = format!("{} 0.9 0.8 0.7 0.9 k 10 10 5 5 re f {}", "q ".repeat(300), "Q ".repeat(300));
        ink_page(&mut doc, Dict::new(), &inside);
        let report = inspect(&doc, TAC_LIMIT_PERCENT);
        assert_eq!(ink_ids(&report, Some(2)), vec!["ink-total"], "{:?}", report.findings);
    }
}
