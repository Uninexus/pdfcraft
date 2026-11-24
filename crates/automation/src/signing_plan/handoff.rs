//! Hand-off packages for the local signing plan: `signing_handoff_export` puts a saved PDF, the plan
//! kept beside it and a small manifest into one zip, and `signing_handoff_import` puts them back
//! beside each other somewhere else, so the document can travel by mail or a shared folder.
//!
//! Nothing here sends anything. The package holds no secret (a plan has none; a digital ID is
//! never part of it), and it is not signed or encrypted: the hashes in the manifest (of the PDF and
//! of the plan) catch a file damaged or changed on the way, not a person who changes a file and the
//! manifest together.

use std::path::Path;

use pdfcraft_organize::signing_handoff as package;
use pdfcraft_organize::signing_handoff::{MANIFEST_ENTRY, PDF_ENTRY, PLAN_ENTRY, Package, VERSION, plan_summary};
use serde_json::{Value, json};

use super::{NOTE, bad, read_plan_sidecar, sidecar_of};
use crate::file_access::Access;
use crate::paths::MAX_INPUT;
use crate::{Args, Automation, Result, failed, read_input};

/// What every hand-off result says besides `NOTE`.
const HANDOFF_NOTE: &str = "A hand-off package is a zip of the PDF, its plan and a manifest. Nothing was sent and nobody was told. The package is not signed \
     or encrypted: the hashes (of the PDF and of the plan) only catch a file damaged or changed in transit, not a person who changed a file and the manifest together, and it holds no digital ID or password.";

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
        let built = package::build(&file_name, &on_disk, &plan)?;
        let digest = built.pdf_sha256;
        let bytes = built.bytes;
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
        let Package { file_name, pdf: pdf_bytes, plan_file, plan, pdf_sha256, version } = package::read(&bytes, MAX_INPUT)?;

        // Where it goes: names the plan expects (the sidecar is the PDF's whole name plus a suffix),
        // both new. Nothing is touched until both are known to be free.
        let pdf_target = folder.join(&file_name);
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
        let mut result = super::plan_json(&plan, &sidecar_target);
        result["ok"] = json!(true);
        result["path"] = json!(pdf_target.to_string_lossy());
        result["pdf_sha256"] = json!(pdf_sha256);
        result["pdf_bytes"] = json!(pdf_bytes.len());
        result["format_version"] = json!(version);
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
