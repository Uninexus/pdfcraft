//! Verify PDF/X (ISO 15930): identification and structure, report only.
//!
//! [`verify`] reads what a document declares (`GTS_PDFXVersion` and `GTS_PDFXConformance` in the
//! document information and `pdfxid:` in XMP) and checks the structural conditions of PDF/X-1a,
//! PDF/X-3 and PDF/X-4 that can be decided from the object graph:
//!
//! - the identification (where each family states it);
//! - a `GTS_PDFX` output intent naming its printing condition, with a profile that is a stream
//!   when one is given;
//! - `Trapped` True or False in the document information or XMP (every family);
//! - a `TrimBox` or an `ArtBox` on every page, and not both, each a rectangle;
//! - the trailer's file identifier; no encryption;
//! - every font embedded;
//! - no transparency (PDF/X-1a and PDF/X-3 only; PDF/X-4 allows it);
//! - PDF/X-1:2001 with GTS_PDFXConformance PDF/X-1a:2001.
//!
//! The result lists what is found. A document with no findings is not thereby PDF/X: colour
//! spaces, profile contents, image and font internals and the other conditions of the standard
//! are not read (see [`NOT_CHECKED`]). Nothing is changed or converted.

use pdfcraft_cos::{Dict, Document, Object};

use crate::{Level, catalog, xmp};

/// Findings of one page-by-page rule listed before the rest are counted.
const MAX_PAGE_FINDINGS: usize = 50;
/// Output intents read; a real file has one or two.
const MAX_INTENTS: usize = 64;

/// What the check does not read.
pub const NOT_CHECKED: &str = "Not checked: colour spaces, DestOutputProfile contents, image and font internals, \
registered printing conditions, page box geometry, and the other conditions of ISO 15930. \
No findings does not make a file PDF/X.";

/// The PDF/X families this check knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    X1a,
    X3,
    X4,
}

impl Family {
    pub fn label(self) -> &'static str {
        match self {
            Family::X1a => "PDF/X-1a",
            Family::X3 => "PDF/X-3",
            Family::X4 => "PDF/X-4",
        }
    }

    /// `x1a`, `x3`, `x4`, with or without `PDF/`, a dash or case.
    pub fn from_id(id: &str) -> Option<Family> {
        match id.to_ascii_lowercase().replace(['/', '-', ' '], "").as_str() {
            "pdfx1a" | "x1a" | "1a" => Some(Family::X1a),
            "pdfx3" | "x3" | "3" => Some(Family::X3),
            "pdfx4" | "x4" | "4" | "pdfx4p" | "x4p" => Some(Family::X4),
            _ => None,
        }
    }

    /// X-1a and X-3 (the 2001-2003 editions) state the version in the document information, and
    /// forbid transparency; X-4 states it in XMP.
    fn legacy(self) -> bool {
        self != Family::X4
    }
}

/// One thing that stands between the document and the PDF/X version it was checked against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// A stable id for the rule, such as `output-intent` or `trim-or-art-box`.
    pub rule: &'static str,
    pub message: String,
    /// 0-based page, when the finding is about one page.
    pub page: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// `GTS_PDFXVersion` as the document states it, from the document information or else XMP.
    pub version: Option<String>,
    /// `GTS_PDFXConformance`, which PDF/X-1a and PDF/X-3 add.
    pub conformance: Option<String>,
    /// The family the document declares, when it is one this check knows.
    pub declared: Option<Family>,
    /// The family the findings are measured against (the one asked for, else the declared one).
    pub checked: Option<Family>,
    pub findings: Vec<Finding>,
}

impl Report {
    /// Whether the check found nothing. It is not a claim of conformance.
    pub fn clean(&self) -> bool {
        self.checked.is_some() && self.findings.is_empty()
    }
}

/// The family one declared string names. Plain PDF/X-1 (1:1999) is not PDF/X-1a and is not mapped;
/// PDF/X-1:2001 is the version string of PDF/X-1a:2001, which also needs its conformance.
fn classify(text: Option<&str>) -> Option<Family> {
    let t = text?.to_ascii_lowercase().replace(' ', "");
    if t.contains("x-1a") || t.contains("x-1:2001") {
        Some(Family::X1a)
    } else if t.contains("x-3") {
        Some(Family::X3)
    } else if t.contains("x-4") {
        Some(Family::X4)
    } else {
        None
    }
}

/// The family a declared version and conformance name: the version's, else the conformance's.
fn family_of(version: Option<&str>, conformance: Option<&str>) -> Option<Family> {
    classify(version).or_else(|| classify(conformance))
}

fn state(doc: &Document, info_key: &str, xmp_text: Option<&str>) -> (Option<String>, Option<String>) {
    let in_info = crate::info(doc, info_key).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let in_xmp = xmp_text.and_then(|x| xmp::identifier(x, xmp::PDFX_ID, info_key)).filter(|v| !v.is_empty());
    (in_info, in_xmp)
}

/// Check `doc` against `requested`, or against the family it declares. With neither, the report
/// says so and carries no other findings.
pub fn verify(doc: &Document, requested: Option<Family>) -> Report {
    let xmp_text = crate::metadata(doc);
    let (info_version, xmp_version) = state(doc, "GTS_PDFXVersion", xmp_text.as_deref());
    let (info_conf, xmp_conf) = state(doc, "GTS_PDFXConformance", xmp_text.as_deref());
    let version = info_version.clone().or_else(|| xmp_version.clone());
    let conformance = info_conf.clone().or_else(|| xmp_conf.clone());
    let declared = family_of(version.as_deref(), conformance.as_deref());
    let checked = requested.or(declared);
    let mut report = Report { version, conformance, declared, checked, findings: Vec::new() };
    let mut add = |rule: &'static str, message: String, page: Option<usize>| report.findings.push(Finding { rule, message, page });

    let Some(family) = checked else {
        let message = match &report_version(&info_version, &xmp_version) {
            Some(v) => format!("GTS_PDFXVersion {v:?} is not a PDF/X version this check knows (PDF/X-1a, PDF/X-3 and PDF/X-4)."),
            None => "The document does not declare a PDF/X version (no GTS_PDFXVersion in the document information or XMP).".into(),
        };
        add("identification", message, None);
        return report;
    };

    // Identification.
    match (declared, requested) {
        (None, _) if info_version.is_some() || xmp_version.is_some() => add(
            "identification",
            format!("GTS_PDFXVersion {:?} is not a version of {}.", report_version(&info_version, &xmp_version).unwrap_or_default(), family.label()),
            None,
        ),
        (None, _) => add("identification", format!("The document does not declare {} (no GTS_PDFXVersion).", family.label()), None),
        (Some(d), Some(r)) if d != r => add("identification", format!("The document declares {}, not {}.", d.label(), r.label()), None),
        _ => {}
    }
    if family.legacy() && info_version.is_none() {
        add("identification", format!("{} states GTS_PDFXVersion in the document information, which has none.", family.label()), None);
    }
    if family == Family::X4 && xmp_version.is_none() {
        add("identification", "PDF/X-4 states pdfxid:GTS_PDFXVersion in XMP metadata, which has none.".into(), None);
    }
    if let (Some(a), Some(b)) = (&info_version, &xmp_version)
        && a != b
    {
        add("identification", format!("The document information says {a:?} and XMP says {b:?}."), None);
    }
    if let (Some(a), Some(b)) = (&info_conf, &xmp_conf)
        && a != b
    {
        add("identification", format!("GTS_PDFXConformance is {a:?} in the document information and {b:?} in XMP."), None);
    }
    if family == Family::X1a
        && info_version.as_ref().or(xmp_version.as_ref()).is_some_and(|v| v.to_ascii_lowercase().replace(' ', "") == "pdf/x-1:2001")
        && !info_conf.as_ref().or(xmp_conf.as_ref()).is_some_and(|c| c.to_ascii_lowercase().replace(' ', "") == "pdf/x-1a:2001")
    {
        add("identification", "PDF/X-1:2001 expects GTS_PDFXConformance to be PDF/X-1a:2001.".into(), None);
    }
    match trapped(doc, xmp_text.as_deref()) {
        Trapped::Valid => {}
        Trapped::Invalid => {
            add("trapped", "PDF/X expects Trapped to be the name True or False (Unknown and other values are not allowed).".into(), None)
        }
        Trapped::Absent => add("trapped", "PDF/X expects Trapped, True or False, in the document information or XMP (pdf:Trapped).".into(), None),
    }

    // File level.
    if doc.trailer().get(b"Encrypt").is_some() {
        add("encryption", "The document is encrypted.".into(), None);
    }
    if doc.trailer().get(b"ID").is_none() {
        add("file-id", "The trailer has no file identifier (ID).".into(), None);
    }
    output_intent(doc, &mut add);

    // Pages.
    let pages = pdfcraft_model::pages(doc);
    let mut boxes = Vec::new();
    for (i, page) in pages.iter().enumerate() {
        let trim = page.dict.get(b"TrimBox");
        let art = page.dict.get(b"ArtBox");
        match (trim, art) {
            (None, None) => boxes.push((i, "Page {} has neither a TrimBox nor an ArtBox.")),
            (Some(_), Some(_)) => boxes.push((i, "Page {} has both a TrimBox and an ArtBox; PDF/X expects only one.")),
            (Some(o), None) | (None, Some(o)) => {
                if !is_rect(doc, o) {
                    boxes.push((i, "Page {}'s TrimBox or ArtBox is not a rectangle of four numbers."));
                }
            }
        }
    }
    page_findings(&mut add, "trim-or-art-box", &boxes);
    if family.legacy() {
        let transparent: Vec<(usize, &str)> = pages
            .iter()
            .enumerate()
            .filter(|(_, p)| crate::page_uses_transparency(doc, &p.dict))
            .map(|(i, _)| (i, "Page {} uses transparency, which PDF/X-1a and PDF/X-3 don't allow."))
            .collect();
        page_findings(&mut add, "transparency", &transparent);
    }

    // Fonts: the rule PDF/A verification already applies to every font that is drawn.
    let mut seen = std::collections::BTreeSet::new();
    for issue in crate::verify(doc, Level::A2b).into_iter().filter(|i| i.clause == "6.2.11.4") {
        seen.insert(issue.value.unwrap_or_else(|| "unnamed font".into()));
    }
    for name in seen.iter().take(MAX_PAGE_FINDINGS) {
        add("fonts-embedded", format!("The font {name} isn't embedded."), None);
    }
    let more = seen.len().saturating_sub(MAX_PAGE_FINDINGS);
    if more > 0 {
        add("fonts-embedded", format!("{more} more fonts aren't embedded."), None);
    }
    report
}

fn report_version(info: &Option<String>, xmp: &Option<String>) -> Option<String> {
    info.clone().or_else(|| xmp.clone())
}

enum Trapped {
    Valid,
    Invalid,
    Absent,
}

/// `Trapped` from the document information (a name, True or False), else XMP `pdf:Trapped`. A
/// boolean, a string or any other name in the document information is invalid.
fn trapped(doc: &Document, xmp_text: Option<&str>) -> Trapped {
    let in_info = doc.trailer().get(b"Info").map(|i| doc.resolve(i)).and_then(|info| info.as_dict().and_then(|d| d.get(b"Trapped").cloned()));
    if let Some(v) = in_info {
        return match &*doc.resolve(&v) {
            Object::Name(n) if matches!(n.as_slice(), b"True" | b"False") => Trapped::Valid,
            _ => Trapped::Invalid,
        };
    }
    match xmp_text.and_then(|x| xmp::value(x, "pdf:Trapped")).map(|v| v.trim().to_string()) {
        Some(v) if v == "True" || v == "False" => Trapped::Valid,
        Some(_) => Trapped::Invalid,
        None => Trapped::Absent,
    }
}

fn is_rect(doc: &Document, o: &Object) -> bool {
    let o = doc.resolve(o);
    let Some(a) = o.as_array() else { return false };
    a.len() == 4 && a.iter().all(|v| doc.resolve(v).as_f64().is_some_and(f64::is_finite))
}

/// The page findings of one rule: the first [`MAX_PAGE_FINDINGS`], then one that counts the rest.
fn page_findings(add: &mut impl FnMut(&'static str, String, Option<usize>), rule: &'static str, found: &[(usize, &'static str)]) {
    for (page, pattern) in found.iter().take(MAX_PAGE_FINDINGS) {
        add(rule, pattern.replace("{}", &(page + 1).to_string()), Some(*page));
    }
    let more = found.len().saturating_sub(MAX_PAGE_FINDINGS);
    if more > 0 {
        add(rule, format!("{more} more pages have the same problem."), None);
    }
}

fn output_intent(doc: &Document, add: &mut impl FnMut(&'static str, String, Option<usize>)) {
    let cat = catalog(doc);
    let list = cat.get(b"OutputIntents").map(|o| doc.resolve(o));
    let intents: Vec<Dict> = list
        .as_deref()
        .and_then(Object::as_array)
        .map(|a| {
            a.iter().take(MAX_INTENTS).filter_map(|oi| doc.resolve(oi).as_dict().cloned()).filter(|d| d.name(b"S") == Some(b"GTS_PDFX")).collect()
        })
        .unwrap_or_default();
    if intents.is_empty() {
        add("output-intent", "The catalog has no output intent of type GTS_PDFX.".into(), None);
    }
    for d in &intents {
        let named = d.get(b"OutputConditionIdentifier").and_then(|v| doc.resolve(v).as_string().map(|s| !s.bytes.is_empty())).unwrap_or(false);
        if !named {
            add("output-intent", "A GTS_PDFX output intent has no OutputConditionIdentifier.".into(), None);
        }
        if let Some(p) = d.get(b"DestOutputProfile")
            && !matches!(&*doc.resolve(p), Object::Stream(_))
        {
            add("output-intent", "A GTS_PDFX output intent's DestOutputProfile is not a stream.".into(), None);
        }
    }
}
