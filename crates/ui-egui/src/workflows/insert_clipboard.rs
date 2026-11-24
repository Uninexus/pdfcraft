//! Pages ▸ Insert pages from clipboard (`page.insert_clipboard`): the picture or the copied files on the system
//! clipboard become pages after the selection, all as one undo step, with the new pages selected.
//!
//! The clipboard is read through the app's clipboard service (`PdfCraftApp::clipboard`), never the operating system
//! directly, so the tests give it a stand-in. A file that can't be used is named and left out; nothing usable says what
//! may be copied. Office files are not converted here: they are named and left out (the `page_insert_image` tool
//! converts them through LibreOffice).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use craft::platform::clipboard::ClipboardError;

use crate::PdfCraftApp;

/// Most files one insert takes from the clipboard.
const MAX_FILES: usize = 100;
/// Largest file one insert reads.
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
/// Most bytes of files one insert holds at once.
const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;

/// What the clipboard holds that can become pages.
enum Found {
    /// Copied files, in order.
    Files(Vec<PathBuf>),
    /// A picture: RGBA pixels, row by row.
    Picture { width: usize, height: usize, rgba: Vec<u8> },
}

/// A file's name and bytes.
type Source = (String, Arc<Vec<u8>>);
/// A file's name and why it can't be used.
type Skipped = (String, String);

/// A file's name and size, or why it can't be used. Nothing is opened: a pipe or a device copied by mistake would wait
/// for ever.
fn file_size(path: &Path) -> Result<(String, u64), Skipped> {
    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    match std::fs::metadata(path) {
        Ok(meta) if !meta.is_file() => Err((name, tl!("it is not a file").to_string())),
        Ok(meta) if meta.len() > MAX_FILE_BYTES => Err((name, tl!("it is larger than 256 MB").to_string())),
        Ok(meta) => Ok((name, meta.len())),
        Err(e) => Err((name, e.to_string())),
    }
}

/// A file's name and bytes, or why it can't be used.
fn read_file(path: &Path, name: String) -> Result<Source, Skipped> {
    use std::io::Read as _;
    let read = || -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)?.take(MAX_FILE_BYTES.saturating_add(1)).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(std::io::Error::other(tl!("it is larger than 256 MB")));
        }
        Ok(bytes)
    };
    match read() {
        Ok(bytes) => Ok((name, Arc::new(bytes))),
        Err(e) => Err((name, e.to_string())),
    }
}

/// The files left out as "name (why)", the first three, and how many more there are.
fn left_out_list(left_out: &[Skipped]) -> (String, usize) {
    let list = left_out.iter().take(3).map(|(name, why)| format!("{name} ({why})")).collect::<Vec<_>>().join(", ");
    (list, left_out.len().saturating_sub(3))
}

impl PdfCraftApp {
    /// What the clipboard holds: copied files first, else a picture; `None` when it holds neither.
    fn clipboard_pages(&mut self) -> Result<Option<Found>, ClipboardError> {
        if let Some(files) = self.clipboard.read_files()? {
            return Ok(Some(Found::Files(files)));
        }
        Ok(self.clipboard.read_image()?.map(|image| Found::Picture { width: image.width as usize, height: image.height as usize, rgba: image.rgba }))
    }

    /// Says why the clipboard couldn't be read.
    fn say_clipboard_unreadable(&mut self, error: &ClipboardError) {
        if matches!(error, ClipboardError::TooLarge { .. }) {
            self.notify_tr("The clipboard holds too much to insert from it");
            return;
        }
        let why = error.text().map_or_else(|| error.to_string(), crate::i18n::platform_text);
        self.notify_fmt("Couldn't read the clipboard: {e}", &[("e", &why)]);
    }

    /// `page.insert_clipboard`: the picture or the copied PDFs, pictures and text files go after the selection (or the
    /// current page), in order, as one undo step, and are selected. What can't be used doesn't stop the rest, and is
    /// named.
    pub(crate) fn insert_from_clipboard(&mut self, ctx: &egui::Context) {
        if cfg!(target_arch = "wasm32") {
            self.notify_tr("Inserting from the clipboard is not available in the browser");
            return;
        }
        if self.active.is_none() {
            self.notify_tr("Open a document first");
            return;
        }
        let mut left_out: Vec<Skipped> = Vec::new();
        let mut sources = Vec::new();
        let found = match self.clipboard_pages() {
            Ok(found) => found,
            Err(e) => return self.say_clipboard_unreadable(&e),
        };
        match found {
            None => {
                self.notify_tr("The clipboard has no picture or PDF, image or text file to insert");
                return;
            }
            Some(Found::Files(files)) => {
                if files.len() > MAX_FILES {
                    self.notify_fmt("Too many files are copied: insert at most {max} at a time.", &[("max", &MAX_FILES.to_string())]);
                    return;
                }
                // Sizes first, so too much is refused before any of it is read.
                let mut sized = Vec::new();
                let mut total = 0u64;
                for file in &files {
                    match file_size(file) {
                        Ok((name, len)) => {
                            total = total.saturating_add(len);
                            sized.push((file, name));
                        }
                        Err(skipped) => left_out.push(skipped),
                    }
                }
                if total > MAX_TOTAL_BYTES {
                    self.notify_tr("The copied files total more than 512 MB; insert fewer at a time.");
                    return;
                }
                for (file, name) in sized {
                    match read_file(file, name) {
                        Ok(source) => sources.push(source),
                        Err(skipped) => left_out.push(skipped),
                    }
                }
            }
            Some(Found::Picture { width, height, rgba }) => match crate::create_ui::png_from_rgba(width, height, &rgba) {
                Ok(png) => sources.push(("Clipboard.png".to_string(), Arc::new(png))),
                Err(e) => return self.notify_error(e),
            },
        }
        self.insert_sources_as_one_step(ctx, sources, left_out);
    }

    /// Insert `sources` (name, bytes) after the selection as one edit and select them; `left_out` is what could not be
    /// read already. Says what was left out.
    fn insert_sources_as_one_step(&mut self, ctx: &egui::Context, sources: Vec<Source>, mut left_out: Vec<Skipped>) {
        let Some(view) = self.active.and_then(|i| self.views.get(i)) else { return };
        // The selection decides where: a gap a "+" chose earlier is not this gesture's.
        let id = view.id;
        let after = view.target_pages().last().map_or(0, |p| p.saturating_add(1));
        let plan = self.session.plan_insert(id, after, sources);
        left_out.extend(plan.skipped);
        let Some(edit) = plan.edit else {
            let (list, more) = left_out_list(&left_out);
            if more > 0 {
                self.notify_fmt(
                    "Nothing on the clipboard could be inserted ({list}, and {more} more). Copy a picture, or PDF, image (PNG, JPEG, TIFF, GIF, BMP) or text files.",
                    &[("list", &list), ("more", &more.to_string())],
                );
            } else {
                self.notify_fmt(
                    "Nothing on the clipboard could be inserted ({list}). Copy a picture, or PDF, image (PNG, JPEG, TIFF, GIF, BMP) or text files.",
                    &[("list", &list)],
                );
            }
            return;
        };
        if !self.apply_edit(ctx, edit) {
            // The edit's own refusal is showing.
            return;
        }
        let pages: Vec<usize> = (plan.start..plan.start.saturating_add(plan.pages)).collect();
        if let Some(view) = self.active.and_then(|i| self.views.get_mut(i)) {
            view.select_pages(&pages);
        }
        if !left_out.is_empty() {
            let (list, more) = left_out_list(&left_out);
            if more > 0 {
                self.notify_fmt("Inserted what could be used. Left out: {list}, and {more} more.", &[("list", &list), ("more", &more.to_string())]);
            } else {
                self.notify_fmt("Inserted what could be used. Left out: {list}.", &[("list", &list)]);
            }
        }
    }
}
