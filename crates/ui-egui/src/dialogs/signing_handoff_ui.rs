//! Sign > Save the hand-off package… (`sign.plan.handoff_export`) and Open a hand-off package… (`sign.plan.handoff_import`):
//! the saved PDF, the signing plan beside it and a manifest in one zip, and back out again somewhere else.
//!
//! The zip and its checks are `pdfcraft_engine::signing_handoff` (the same code the `signing_handoff_*` tools use). Nothing is
//! sent and nobody is told. The package is not signed or encrypted: its hashes catch a file damaged or changed in transit,
//! not a person who changes a file and the manifest together. Both commands only ever make new files: a file that is there
//! already is refused and left alone, whatever the file picker was told.

use std::path::{Path, PathBuf};

use craft::host::{PickRequest, WriteIntent};
use pdfcraft_engine::signing_handoff as handoff;
use pdfcraft_engine::signing_plan as plan_rules;

use super::signing_plan_ui::sidecar;
use crate::PdfCraftApp;
use crate::prepare::signer::clean;

/// The longest piece of a refusal read from a file that is shown (characters).
const SHOWN: usize = 300;
/// The most a package's PDF may expand to (bytes).
const MAX_PDF_BYTES: u64 = 512 * 1024 * 1024;

impl PdfCraftApp {
    /// The package of the active document: the name to offer for the zip and its bytes, or why there is none.
    fn build_handoff(&self) -> Result<(String, Vec<u8>), String> {
        let (_, id) = self.active_ids().ok_or_else(|| tl!("Open a document first").to_string())?;
        let doc = self.session.get(id).ok_or_else(|| tl!("Open a document first").to_string())?;
        let path = doc.path.clone();
        let (dirty, changed_on_open, saved) = (doc.dirty, doc.changed_on_open, doc.saved_bytes());
        let sidecar = sidecar(path.as_deref())?;
        let path = path.ok_or_else(|| tl!("The document has no saved file. Save it first: the package holds the file as saved.").to_string())?;
        if dirty {
            return Err(tl!("The document has changes that are not saved. Save it first: the package holds the file as saved.").to_string());
        }
        if changed_on_open {
            return Err(tl!(
                "Opening this file changed it (a repair or a form layout), so it cannot be compared with the file on disk. Save it, reopen it and try again."
            )
            .to_string());
        }
        let plan = match plan_rules::read_sidecar(&sidecar) {
            Ok(Some(plan)) => plan,
            Ok(None) => {
                return Err(tl!("There is no signing plan beside this document: add a recipient in Sign > Signing plan… first").to_string());
            }
            Err(e) => return Err(clean(&e.0, SHOWN)),
        };
        let pdf = Path::new(&path);
        let on_disk =
            self.host.files().read(pdf).map_err(|e| crate::i18n::fmt(tl!("Couldn't read the saved file: {e}"), &[("e", &e.to_string())]))?;
        if on_disk != *saved {
            return Err(tl!("The file changed on disk since it was opened. Reopen it; nothing was written.").to_string());
        }
        let name = pdf.file_name().and_then(|n| n.to_str()).ok_or_else(|| tl!("The document's file name is not text").to_string())?;
        let built = handoff::build(name, &on_disk, &plan).map_err(|e| clean(&e.0, SHOWN))?;
        let stem = pdf.file_stem().and_then(|n| n.to_str()).unwrap_or("document");
        Ok((format!("{stem} (hand-off).zip"), built.bytes))
    }

    /// `sign.plan.handoff_export`: asks where to save the package, then writes it as a new file.
    pub fn export_signing_handoff(&mut self) {
        if cfg!(target_arch = "wasm32") {
            return self.notify_tr("Hand-off packages are not available in the browser");
        }
        let (name, bytes) = match self.build_handoff() {
            Ok(built) => built,
            Err(why) => return self.say_plan(why, false),
        };
        let ask = PickRequest::save("handoff").title(tl!("Save the hand-off package").to_string()).file_name(name).filter("Zip", &["zip"]);
        self.pick_first(ask, None, move |app, _, path| app.write_handoff(path, &bytes));
    }

    /// Writes the package to `path` (a `.zip` is added when the name has none), unless something is there already.
    fn write_handoff(&mut self, path: PathBuf, bytes: &[u8]) {
        let is_zip = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("zip"));
        let path = if is_zip {
            path
        } else {
            let mut name = path.into_os_string();
            name.push(".zip");
            PathBuf::from(name)
        };
        let shown = clean(&path.display().to_string(), SHOWN);
        if self.host.files().exists(&path) {
            let why =
                crate::i18n::fmt(tl!("{path} already exists; it was left alone and nothing was written. Choose a new name."), &[("path", &shown)]);
            return self.say_plan(why, false);
        }
        match self.host.files().write(&path, bytes, WriteIntent::SaveAs) {
            Ok(_) => {
                let done = crate::i18n::fmt(
                    tl!(
                        "The hand-off package was saved to {path}. Nothing was sent and nobody was told. The package is not signed or encrypted: its hashes only catch a file damaged or changed on the way."
                    ),
                    &[("path", &shown)],
                );
                self.say_plan(done, true);
            }
            Err(e) => {
                let why = crate::i18n::fmt(tl!("Couldn't save the package: {e}"), &[("e", &clean(&e.to_string(), SHOWN))]);
                self.say_plan(why, false);
            }
        }
    }

    /// `sign.plan.handoff_import`: asks for a package and checks it whole, then asks for a folder.
    pub fn import_signing_handoff(&mut self) {
        if cfg!(target_arch = "wasm32") {
            return self.notify_tr("Hand-off packages are not available in the browser");
        }
        let ask = PickRequest::open("handoff").title(tl!("Open a hand-off package").to_string()).filter("Zip", &["zip"]);
        self.pick_first(ask, None, |app, _, zip| app.read_handoff(&zip));
    }

    /// Reads and checks the package at `zip`; a package that does not check out is refused with the reason and nothing is
    /// asked for or written.
    fn read_handoff(&mut self, zip: &Path) {
        let bytes = match self.host.files().read(zip) {
            Ok(bytes) => bytes,
            Err(e) => {
                let why = crate::i18n::fmt(tl!("Couldn't read the package: {e}"), &[("e", &clean(&e.to_string(), SHOWN))]);
                return self.say_plan(why, false);
            }
        };
        let package = match handoff::read(&bytes, MAX_PDF_BYTES) {
            Ok(package) => package,
            Err(e) => return self.say_plan(clean(&e.0, SHOWN), false),
        };
        let ask = PickRequest::folder("handoff.import").title(tl!("Choose a folder for the document and its plan").to_string());
        self.pick_first(ask, None, move |app, ctx, dir| app.place_handoff(ctx, package, &dir));
    }

    /// Writes the package's PDF and plan into `dir`, both as new files, and opens the PDF.
    fn place_handoff(&mut self, ctx: &egui::Context, package: handoff::Package, dir: &Path) {
        let pdf_target = dir.join(&package.file_name);
        let sidecar_target = match plan_rules::sidecar_of(&pdf_target) {
            Ok(path) => path,
            Err(e) => return self.say_plan(clean(&e.0, SHOWN), false),
        };
        let taken: Vec<String> = [&pdf_target, &sidecar_target]
            .into_iter()
            .filter(|t| self.host.files().exists(t))
            .map(|t| clean(&t.display().to_string(), 200))
            .collect();
        if !taken.is_empty() {
            let why = crate::i18n::fmt(
                tl!("{names} already exists; nothing was overwritten and nothing was written. Choose another folder."),
                &[("names", &taken.join(" and "))],
            );
            return self.say_plan(why, false);
        }
        let shown = clean(&pdf_target.display().to_string(), SHOWN);
        if let Err(e) = self.host.files().write(&pdf_target, &package.pdf, WriteIntent::SaveAs) {
            let why = crate::i18n::fmt(tl!("Couldn't write the document: {e}"), &[("e", &clean(&e.to_string(), SHOWN))]);
            return self.say_plan(why, false);
        }
        if let Err(e) = self.host.files().write(&sidecar_target, &package.plan_file, WriteIntent::SaveAs) {
            let why = crate::i18n::fmt(
                tl!("The document was saved to {path} but its signing plan could not be written: {e}. The plan was not imported."),
                &[("path", &shown), ("e", &clean(&e.to_string(), SHOWN))],
            );
            return self.say_plan(why, false);
        }
        let name = package.file_name.clone();
        let opened = self.open_bytes(ctx, &name, Some(pdf_target.to_string_lossy().into_owned()), package.pdf);
        let (said, ok) = match opened {
            Ok(()) => (
                crate::i18n::fmt(
                    tl!(
                        "The package was checked and saved to {path} with its signing plan. The signatures in the PDF were not checked: read them in the Signatures panel. Nothing was sent and nobody was told."
                    ),
                    &[("path", &shown)],
                ),
                true,
            ),
            Err(e) => (
                crate::i18n::fmt(
                    tl!("The package was saved to {path} but the document could not be opened: {e}"),
                    &[("path", &shown), ("e", &clean(&e, SHOWN))],
                ),
                false,
            ),
        };
        self.say_plan(said, ok);
    }
}
