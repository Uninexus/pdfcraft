//! Organize ▸ Insert ▸ From Clipboard: the pictures and files on the system clipboard become
//! pages after the selection, all as one undo step.

use std::path::{Path, PathBuf};
use std::sync::Arc;

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

/// A file's name and size, or why it can't be used. Nothing is opened: a pipe or a device
/// copied by mistake would wait for ever.
fn file_size(path: &Path) -> Result<(String, u64), Skipped> {
    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    match std::fs::metadata(path) {
        Ok(meta) if !meta.is_file() => Err((name, "it is not a file".into())),
        Ok(meta) if meta.len() > MAX_FILE_BYTES => Err((name, "it is larger than 256 MB".into())),
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
            return Err(std::io::Error::other("it is larger than 256 MB"));
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
    /// What the clipboard (or the test stand-in) holds: copied files first, else a picture.
    fn clipboard_pages(&self) -> Option<Found> {
        let picture = |clip: Option<crate::create_ui::Clip>| match clip {
            Some(crate::create_ui::Clip::Image { width, height, rgba }) => Some(Found::Picture { width, height, rgba }),
            _ => None,
        };
        if let Some(files) = self.clipboard_files_override.clone().filter(|f| !f.is_empty()) {
            return Some(Found::Files(files));
        }
        if self.clipboard_override.is_some() {
            return picture(self.clipboard_override.clone());
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.system_clipboard {
            let files = crate::os_clipboard::files();
            if !files.is_empty() {
                return Some(Found::Files(files));
            }
            return picture(crate::create_ui::read_clipboard());
        }
        None
    }

    /// Insert pages from the clipboard: the picture or the copied PDFs, pictures and text files
    /// it holds go after the selection (or the current page), in order, as one undo step, and are
    /// selected. Nothing it can't use stops the rest; it says what it left out.
    pub(crate) fn insert_from_clipboard(&mut self) {
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
        match self.clipboard_pages() {
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
                Err(e) => {
                    self.notify_error(e);
                    return;
                }
            },
        }
        self.insert_sources_as_one_step(sources, left_out);
    }

    /// Insert `sources` (name, bytes) after the selection as one edit and select them; `left_out`
    /// is what could not be read already. Says what was left out.
    fn insert_sources_as_one_step(&mut self, sources: Vec<Source>, mut left_out: Vec<Skipped>) {
        let Some(view) = self.active.and_then(|i| self.views.get_mut(i)) else { return };
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
        if self.apply_edit(edit) {
            let pages: Vec<usize> = (plan.start..plan.start.saturating_add(plan.pages)).collect();
            if let Some(view) = self.active.and_then(|i| self.views.get_mut(i)) {
                view.select_pages(&pages);
            }
            if !left_out.is_empty() {
                let (list, more) = left_out_list(&left_out);
                if more > 0 {
                    self.notify_fmt(
                        "Inserted what could be used. Left out: {list}, and {more} more.",
                        &[("list", &list), ("more", &more.to_string())],
                    );
                } else {
                    self.notify_fmt("Inserted what could be used. Left out: {list}.", &[("list", &list)]);
                }
            }
        }
    }
}
