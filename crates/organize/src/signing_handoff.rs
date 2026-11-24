//! The pure part of a signing hand-off package: a zip of a saved PDF, the plan kept beside it and
//! a small manifest, and the checks that make a package safe to unpack. It touches no file; the
//! automation tools and the app read and write the files themselves, each under its own roots.
//!
//! Nothing here sends anything. The package holds no secret (a plan has none; a digital ID is
//! never part of it), and it is not signed or encrypted: the hashes in the manifest (of the PDF and
//! of the plan) catch a file damaged or changed on the way, not a person who changes a file and the
//! manifest together.

use craft_zip::{Archive, Compression, Limits, Writer};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::field_signer::is_hidden;
use crate::signing_plan::{MAX_SIDECAR_BYTES, Plan, PlanError, plan_bytes, plan_from_bytes};

type Result<T> = std::result::Result<T, PlanError>;

fn failed(m: impl Into<String>) -> PlanError {
    PlanError(m.into())
}

/// What the manifest's `format` says.
pub const FORMAT: &str = "pdfcraft-signing-handoff";
/// The one manifest version written and read.
pub const VERSION: u32 = 1;
/// The entries of a package, and the only names an archive may hold.
pub const MANIFEST_ENTRY: &str = "manifest.json";
pub const PDF_ENTRY: &str = "document.pdf";
pub const PLAN_ENTRY: &str = "plan.json";
/// What each entry may expand to (bytes). The plan's limit is the sidecar's own.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_PLAN_BYTES: u64 = MAX_SIDECAR_BYTES;
/// The longest file name taken from a manifest (bytes).
const MAX_NAME_BYTES: usize = 200;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    version: u32,
    /// The PDF's file name; the sidecar's name is derived from it.
    file_name: String,
    pdf_sha256: String,
    pdf_bytes: u64,
    /// The SHA-256 of `plan.json`, the plan file as written.
    plan_sha256: String,
    /// The plan's recipients, order and statuses as the plan file holds them.
    plan: Value,
}

/// The lower-case hex SHA-256 of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// What the manifest says about the plan: recipients in plan order with their order and status.
pub fn plan_summary(plan: &Plan) -> Value {
    json!({
        "state": plan.state(),
        "recipients": plan.recipients.iter().map(|r| json!({
            "name": r.name, "email": r.email, "role": r.role, "order": r.order, "status": r.status,
        })).collect::<Vec<_>>(),
    })
}

/// Whether a path component names a device on Windows (`CON`, `COM1` ...), with or without an
/// extension.
pub fn is_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component).trim_end_matches(' ').to_ascii_uppercase();
    let numbered = |prefix: &str| stem.strip_prefix(prefix).is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"));
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$") || numbered("COM") || numbered("LPT")
}

/// A file name that is one plain name: no folder part, no drive or stream marker, no control or
/// hidden character, ending in `.pdf`. It is the only thing from a package that names a path.
pub fn check_file_name(name: &str) -> Result<()> {
    let plain = !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name != "."
        && name != ".."
        && !name.starts_with('.')
        && !name.ends_with(['.', ' '])
        && !is_device_name(name)
        && !name.chars().any(|c| is_hidden(c) || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'));
    let is_pdf = name.len() > 4 && name.get(name.len() - 4..).is_some_and(|e| e.eq_ignore_ascii_case(".pdf"));
    if plain && is_pdf {
        Ok(())
    } else {
        Err(failed(format!("the package names its PDF {name:?}, which is not a plain file name ending in .pdf; nothing was written")))
    }
}

/// A package built by [`build`].
pub struct Built {
    /// The zip.
    pub bytes: Vec<u8>,
    pub pdf_sha256: String,
}

/// The zip of `pdf` (the saved file, named `file_name`) and `plan`.
pub fn build(file_name: &str, pdf: &[u8], plan: &Plan) -> Result<Built> {
    check_file_name(file_name)?;
    let digest = hex(pdf);
    let plan_file = plan_bytes(plan)?;
    let manifest = Manifest {
        format: FORMAT.into(),
        version: VERSION,
        file_name: file_name.to_string(),
        pdf_sha256: digest.clone(),
        pdf_bytes: pdf.len() as u64,
        plan_sha256: hex(&plan_file),
        plan: plan_summary(plan),
    };
    let mut zip = Writer::new();
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| failed(e.to_string()))?;
    for (name, data) in [(MANIFEST_ENTRY, manifest_bytes.as_slice()), (PDF_ENTRY, pdf), (PLAN_ENTRY, plan_file.as_slice())] {
        zip.add(name, data, Compression::Deflated).map_err(|e| failed(e.to_string()))?;
    }
    let bytes = zip.finish().map_err(|e| failed(e.to_string()))?;
    Ok(Built { bytes, pdf_sha256: digest })
}

/// A package that passed every check.
pub struct Package {
    /// The PDF's plain file name (checked).
    pub file_name: String,
    pub pdf: Vec<u8>,
    /// The plan file as written; its sidecar's bytes.
    pub plan_file: Vec<u8>,
    pub plan: Plan,
    pub pdf_sha256: String,
    pub version: u32,
}

/// The package in `bytes`, checked whole: the entries, the manifest, the PDF's and the plan's
/// hashes and the plan against its summary. `max_pdf_bytes` is what the PDF may expand to. Every
/// refusal says why and that nothing was written.
pub fn read(bytes: &[u8], max_pdf_bytes: u64) -> Result<Package> {
    let limits = Limits { max_entries: 3, max_total_bytes: MAX_MANIFEST_BYTES + MAX_PLAN_BYTES + max_pdf_bytes };
    let package = |e: craft_zip::Error| {
        if e.is_limit() {
            failed(format!("this is not a hand-off package: {e} (more than a package holds, or it expands far past its size); nothing was written"))
        } else {
            failed(format!("this is not a hand-off package: {e}; nothing was written"))
        }
    };
    let mut zip = Archive::open(bytes, limits).map_err(package)?;
    // Only the three names, once each: a path with a folder part, `..` or a drive in it, and
    // an absolute name, are none of them.
    if let Some(other) = zip.names().find(|n| ![MANIFEST_ENTRY, PDF_ENTRY, PLAN_ENTRY].contains(&n.as_ref())) {
        return Err(failed(format!("the archive holds {other:?}, which is not part of a hand-off package; nothing was written")));
    }
    if zip.has_duplicate_names() {
        return Err(failed("the archive names an entry twice; nothing was written"));
    }
    let mut read = |name: &str, max: u64| -> Result<Vec<u8>> {
        zip.read(name, max).map_err(package)?.ok_or_else(|| failed(format!("the package has no {name}; nothing was written")))
    };
    let manifest_bytes = read(MANIFEST_ENTRY, MAX_MANIFEST_BYTES)?;
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).map_err(|e| failed(format!("the manifest is not readable ({e}); nothing was written")))?;
    if manifest.format != FORMAT {
        return Err(failed("the manifest is not a PdfCraft signing hand-off manifest; nothing was written"));
    }
    if manifest.version != VERSION {
        return Err(failed(format!(
            "the package has format version {}, and this program reads version {VERSION}; nothing was written",
            manifest.version
        )));
    }
    check_file_name(&manifest.file_name)?;
    // Only now are the plan and the PDF inflated.
    let plan_file = read(PLAN_ENTRY, MAX_PLAN_BYTES)?;
    let pdf = read(PDF_ENTRY, max_pdf_bytes)?;
    if !pdf.starts_with(b"%PDF-") {
        return Err(failed("the package's document is not a PDF; nothing was written"));
    }
    if pdf.len() as u64 != manifest.pdf_bytes || hex(&pdf) != manifest.pdf_sha256 {
        return Err(failed("the PDF in the package does not match the hash in its manifest, so it was changed or damaged; nothing was written"));
    }
    if hex(&plan_file) != manifest.plan_sha256 {
        return Err(failed("the plan in the package does not match the hash in its manifest, so it was changed or damaged; nothing was written"));
    }
    let plan = plan_from_bytes(&plan_file, "the package's plan", "nothing was written")?;
    if plan_summary(&plan) != manifest.plan {
        return Err(failed("the plan in the package does not match the plan the manifest describes; nothing was written"));
    }
    Ok(Package { file_name: manifest.file_name, pdf, plan_file, plan, pdf_sha256: manifest.pdf_sha256, version: manifest.version })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signing_plan::Role;

    const PDF: &[u8] = b"%PDF-1.7\n% a stand-in\n";

    fn plan() -> Plan {
        let mut p = Plan::empty(0);
        p.add_recipient("Ana Example", "ana@example.org", Role::Signer, 0).unwrap();
        p
    }

    #[test]
    fn a_built_package_reads_back() {
        let built = build("a.pdf", PDF, &plan()).unwrap();
        let back = read(&built.bytes, 1 << 20).unwrap();
        assert_eq!(back.pdf, PDF);
        assert_eq!(back.file_name, "a.pdf");
        assert_eq!(back.pdf_sha256, built.pdf_sha256);
        assert_eq!(back.plan.recipients.len(), 1);
    }

    #[test]
    fn a_changed_document_is_refused_with_the_reason() {
        let built = build("a.pdf", PDF, &plan()).unwrap();
        // The same package with the PDF's entry swapped for other bytes.
        let mut zip = Archive::open(&built.bytes, Limits { max_entries: 3, max_total_bytes: 1 << 24 }).unwrap();
        let manifest = zip.read(MANIFEST_ENTRY, 1 << 16).unwrap().unwrap();
        let plan_file = zip.read(PLAN_ENTRY, 1 << 16).unwrap().unwrap();
        let mut w = Writer::new();
        w.add(MANIFEST_ENTRY, &manifest, Compression::Deflated).unwrap();
        w.add(PDF_ENTRY, b"%PDF-1.7\n% another\n", Compression::Deflated).unwrap();
        w.add(PLAN_ENTRY, &plan_file, Compression::Deflated).unwrap();
        let err = read(&w.finish().unwrap(), 1 << 20).err().unwrap();
        assert!(err.0.contains("hash"), "{err}");
    }

    #[test]
    fn a_file_name_with_a_folder_part_is_not_taken() {
        for bad in ["../a.pdf", "x/a.pdf", "a.txt", ".hidden.pdf", "NUL.pdf", "a:b.pdf"] {
            assert!(check_file_name(bad).is_err(), "{bad}");
        }
        assert!(check_file_name("Contract 2.PDF").is_ok());
    }
}
