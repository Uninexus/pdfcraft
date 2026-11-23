//! Hand-off packages for the local signing plan: `signing_handoff_export` puts a saved PDF, the plan
//! kept beside it and a small manifest into one zip, and `signing_handoff_import` puts them back
//! beside each other somewhere else, so the document can travel by mail or a shared folder.
//!
//! Nothing here sends anything. The package holds no secret (a plan has none; a digital ID is
//! never part of it), and it is not signed or encrypted: the hashes in the manifest (of the PDF and
//! of the plan) catch a file damaged or changed on the way, not a person who changes a file and the
//! manifest together.

use std::path::Path;

use craft_zip::{Archive, Compression, Limits, Writer};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{NOTE, Plan, bad, plan_bytes, plan_from_bytes, read_plan_sidecar, sidecar_of};
use crate::file_access::Access;
use crate::paths::MAX_INPUT;
use crate::{Args, Automation, Result, failed, read_input};

/// What the manifest's `format` says.
const FORMAT: &str = "pdfcraft-signing-handoff";
/// The one manifest version written and read.
const VERSION: u32 = 1;
/// The entries of a package, and the only names an archive may hold.
const MANIFEST_ENTRY: &str = "manifest.json";
const PDF_ENTRY: &str = "document.pdf";
const PLAN_ENTRY: &str = "plan.json";
/// What each entry may expand to (bytes). The plan's limit is the sidecar's own.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_PLAN_BYTES: u64 = super::MAX_SIDECAR_BYTES;
const MAX_PDF_BYTES: u64 = MAX_INPUT;
/// The longest file name taken from a manifest (bytes).
const MAX_NAME_BYTES: usize = 200;

/// What every hand-off result says besides `NOTE`.
const HANDOFF_NOTE: &str = "A hand-off package is a zip of the PDF, its plan and a manifest. Nothing was sent and nobody was told. The package is not signed \
     or encrypted: the hashes (of the PDF and of the plan) only catch a file damaged or changed in transit, not a person who changed a file and the manifest together, and it holds no digital ID or password.";

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

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// What the manifest says about the plan: recipients in plan order with their order and status.
fn plan_summary(plan: &Plan) -> Value {
    json!({
        "state": plan.state(),
        "recipients": plan.recipients.iter().map(|r| json!({
            "name": r.name, "email": r.email, "role": r.role, "order": r.order, "status": r.status,
        })).collect::<Vec<_>>(),
    })
}

/// A file name that is one plain name: no folder part, no drive or stream marker, no control or
/// hidden character, ending in `.pdf`. It is the only thing from a package that names a path.
fn check_file_name(name: &str) -> Result<()> {
    let plain = !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name != "."
        && name != ".."
        && !name.starts_with('.')
        && !name.ends_with(['.', ' '])
        && !crate::file_access::is_device_name(name)
        && !name.chars().any(|c| super::is_hidden(c) || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'));
    let is_pdf = name.len() > 4 && name.get(name.len() - 4..).is_some_and(|e| e.eq_ignore_ascii_case(".pdf"));
    if plain && is_pdf {
        Ok(())
    } else {
        Err(failed(format!("the package names its PDF {name:?}, which is not a plain file name ending in .pdf; nothing was written")))
    }
}

impl Automation {
    /// `signing_handoff_export`: one zip holding the saved PDF as it is on disk, the plan beside it
    /// and a manifest. The output must be a new `.zip` inside the write root.
    pub(crate) fn signing_handoff_export(&mut self, a: &Args) -> Result<Value> {
        // The output first, so a path outside the roots is refused before the document is looked at.
        let raw_out = a.str("path")?;
        let zip_name = Path::new(raw_out).file_name().and_then(|n| n.to_str()).unwrap_or("");
        if zip_name.len() <= 4 || !zip_name.get(zip_name.len() - 4..).is_some_and(|e| e.eq_ignore_ascii_case(".zip")) {
            return Err(bad("path must name the zip to write, ending in .zip"));
        }
        let out = self.resolve_for(raw_out, Access::Write, true)?;
        if out.symlink_metadata().is_ok() {
            return Err(failed(format!("{} already exists; it was left alone and nothing was written (choose a new name)", out.display())));
        }
        let (path, dirty, changed_on_open) = {
            let doc = self.doc(a)?;
            (doc.path.clone(), doc.dirty, doc.changed_on_open)
        };
        let path = path.ok_or_else(|| bad("the document has no file; the plan is kept beside the document's file, so save it first"))?;
        if dirty {
            return Err(failed("the document has changes that are not saved; save it first (doc_save), because the package holds the file as saved"));
        }
        if changed_on_open {
            return Err(failed(
                "opening this file changed it (a repair or a form layout), so it cannot be compared with the file on disk; save it with doc_save, reopen it and try again",
            ));
        }
        let pdf = self.resolve_for(&path, Access::Read, false)?;
        let sidecar = sidecar_of(&pdf)?;
        let shown = sidecar.to_string_lossy();
        self.check_roots(&shown, &sidecar, Access::Read)?;
        let plan =
            read_plan_sidecar(&sidecar)?.ok_or_else(|| failed("there is no signing plan beside this document; make one with signing_plan_set"))?;
        let on_disk = read_input(&pdf)?;
        if *on_disk != **self.doc(a)?.saved_bytes() {
            return Err(failed("the file changed on disk since it was opened; reopen it (nothing was written)"));
        }
        let file_name = pdf.file_name().and_then(|n| n.to_str()).ok_or_else(|| failed("the document's file name is not text"))?.to_string();
        check_file_name(&file_name)?;
        let digest = hex(&on_disk);
        let plan_file = plan_bytes(&plan)?;
        let manifest = Manifest {
            format: FORMAT.into(),
            version: VERSION,
            file_name,
            pdf_sha256: digest.clone(),
            pdf_bytes: on_disk.len() as u64,
            plan_sha256: hex(&plan_file),
            plan: plan_summary(&plan),
        };

        let mut zip = Writer::new();
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(failed)?;
        for (name, data) in [(MANIFEST_ENTRY, manifest_bytes.as_slice()), (PDF_ENTRY, on_disk.as_slice()), (PLAN_ENTRY, plan_file.as_slice())] {
            zip.add(name, data, Compression::Deflated).map_err(|e| failed(e.to_string()))?;
        }
        let bytes = zip.finish().map_err(|e| failed(e.to_string()))?;
        crate::bates::publish_new(&out, &bytes)?;
        Ok(json!({
            "ok": true, "path": out.to_string_lossy(), "bytes": bytes.len(), "entries": [MANIFEST_ENTRY, PDF_ENTRY, PLAN_ENTRY],
            "pdf_sha256": digest, "pdf_bytes": on_disk.len(), "format_version": VERSION,
            "plan": plan_summary(&plan), "sent": false, "notified": false, "local_record_only": true, "note": format!("{HANDOFF_NOTE} {NOTE}"),
        }))
    }

    /// `signing_handoff_import`: the package's PDF and plan, checked, written beside each other in
    /// a folder inside the write root, under names that do not exist yet.
    pub(crate) fn signing_handoff_import(&mut self, a: &Args) -> Result<Value> {
        let zip_path = self.resolve(a.str("path")?, false)?;
        let folder = self.resolve(a.str("folder")?, true)?;
        let bytes = read_input(&zip_path)?;
        let limits = Limits { max_entries: 3, max_total_bytes: MAX_MANIFEST_BYTES + MAX_PLAN_BYTES + MAX_PDF_BYTES };
        let package = |e: craft_zip::Error| {
            if e.is_limit() {
                failed(format!(
                    "this is not a hand-off package: {e} (more than a package holds, or it expands far past its size); nothing was written"
                ))
            } else {
                failed(format!("this is not a hand-off package: {e}; nothing was written"))
            }
        };
        let mut zip = Archive::open(&bytes, limits).map_err(package)?;
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
        let pdf_bytes = read(PDF_ENTRY, MAX_PDF_BYTES)?;
        if !pdf_bytes.starts_with(b"%PDF-") {
            return Err(failed("the package's document is not a PDF; nothing was written"));
        }
        if pdf_bytes.len() as u64 != manifest.pdf_bytes || hex(&pdf_bytes) != manifest.pdf_sha256 {
            return Err(failed("the PDF in the package does not match the hash in its manifest, so it was changed or damaged; nothing was written"));
        }
        if hex(&plan_file) != manifest.plan_sha256 {
            return Err(failed("the plan in the package does not match the hash in its manifest, so it was changed or damaged; nothing was written"));
        }
        let plan = plan_from_bytes(&plan_file, "the package's plan", "nothing was written")?;
        if plan_summary(&plan) != manifest.plan {
            return Err(failed("the plan in the package does not match the plan the manifest describes; nothing was written"));
        }

        // Where it goes: names the plan expects (the sidecar is the PDF's whole name plus a suffix),
        // both new. Nothing is touched until both are known to be free.
        let pdf_target = folder.join(&manifest.file_name);
        let sidecar_target = sidecar_of(&pdf_target)?;
        for target in [&pdf_target, &sidecar_target] {
            self.check_roots(&target.to_string_lossy(), target, Access::Write)?;
        }
        if folder.symlink_metadata().is_ok() && !folder.is_dir() {
            return Err(failed(format!("{} is not a folder; nothing was written", folder.display())));
        }
        let taken: Vec<String> =
            [&pdf_target, &sidecar_target].into_iter().filter(|t| t.symlink_metadata().is_ok()).map(|t| t.display().to_string()).collect();
        if !taken.is_empty() {
            return Err(failed(format!(
                "{} already exists; nothing was overwritten and nothing was written (choose another folder)",
                taken.join(" and ")
            )));
        }
        std::fs::create_dir_all(&folder).map_err(|e| failed(format!("{}: {e}", folder.display())))?;
        crate::bates::publish_new(&pdf_target, &pdf_bytes)?;
        if let Err(e) = crate::bates::publish_new(&sidecar_target, &plan_file) {
            // Only the file this call made.
            let _ = std::fs::remove_file(&pdf_target);
            return Err(e);
        }
        let mut result = plan.json(&sidecar_target);
        result["ok"] = json!(true);
        result["path"] = json!(pdf_target.to_string_lossy());
        result["pdf_sha256"] = json!(manifest.pdf_sha256);
        result["pdf_bytes"] = json!(pdf_bytes.len());
        result["format_version"] = json!(manifest.version);
        result["hash_checked"] = json!(true);
        result["signatures_checked"] = json!(false);
        result["sent"] = json!(false);
        result["notified"] = json!(false);
        result["note"] = json!(format!(
            "{HANDOFF_NOTE} The signatures in the PDF were not validated by this call: open the file and read them with sign_list. {NOTE}"
        ));
        Ok(result)
    }
}
