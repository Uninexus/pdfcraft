use super::error::{Result, ToolError, failed};
use super::{Automation, Path, PathBuf, StagingName, file_access, staging_suffixes};

impl Automation {
    // ---- paths -------------------------------------------------------------------------------

    /// Resolve a user-supplied path, enforcing the root (if any). `for_write` allows a file that
    /// does not exist yet (its nearest existing ancestor must be inside the root).
    ///
    /// Every path outside the root gets the same refusal, so a confined client can't learn what
    /// exists out there (#136): `..` is resolved by name first, another network share or device
    /// namespace is refused without touching it, and the deepest existing ancestor (links
    /// followed) must be inside the root before anything about the rest is reported.
    pub(crate) fn resolve(&self, path: &str, for_write: bool) -> Result<PathBuf> {
        let access = if for_write { file_access::Access::Write } else { file_access::Access::Read };
        self.resolve_for(path, access, for_write)
    }

    /// [`resolve`](Self::resolve) with the kind of access named, for the places where a file that
    /// is only read may still be missing (a path checked up front, whose absence is reported per
    /// file later). The automation roots, if any, get the last word.
    pub(crate) fn resolve_for(&self, path: &str, access: file_access::Access, may_be_missing: bool) -> Result<PathBuf> {
        let target = self.resolve_in_root(path, may_be_missing)?;
        self.check_roots(path, &target, access)?;
        Ok(target)
    }

    /// Hold `target`, which `path` named, to the automation read or write root. Always passes
    /// without roots.
    pub(crate) fn check_roots(&self, path: &str, target: &Path, access: file_access::Access) -> Result<()> {
        match &self.roots {
            Some(roots) => roots.check_path(path, target, access).map_err(failed),
            None => Ok(()),
        }
    }

    fn resolve_in_root(&self, path: &str, for_write: bool) -> Result<PathBuf> {
        let p = Path::new(path);
        let Some(root) = &self.root else { return Ok(p.to_path_buf()) };
        let outside = || failed(format!("{path} is outside the allowed directory {}", root.display()));
        let joined = lexical(&root.join(p)).ok_or_else(outside)?;
        // Followed one name at a time first: nothing outside the root or its own parent folders is
        // looked up, not even to see whether it exists. What can't be followed inside the root is
        // said below. Made absolute the way Windows reads it first: it drops a folder name's
        // trailing dot (`in.` is `in`), but `follow` looks names up below `\\?\` folders, where it
        // doesn't.
        let as_read = std::path::absolute(&joined).map_err(|_| outside())?;
        if matches!(file_access::follow(path, &as_read, root), file_access::Toward::Outside) {
            return Err(outside());
        }
        let mut existing = joined.as_path();
        let mut rest = Vec::new();
        while !existing.exists() {
            if existing.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(outside()); // a broken link: going on would tell whether its target exists
            }
            match (existing.file_name(), existing.parent()) {
                (Some(name), Some(parent)) => {
                    rest.push(name);
                    existing = parent;
                }
                _ => return Err(outside()), // not even a drive or share that exists
            }
        }
        let mut real = existing.canonicalize().map_err(|_| outside())?;
        if !real.starts_with(root) {
            return Err(outside());
        }
        if !rest.is_empty() && !for_write {
            // Missing, below a folder inside the root: say where it was looked for, so a path
            // written for another working directory can be corrected (#1890), and otherwise
            // why, as the system reports it. `joined` is under the root, so naming it reveals
            // nothing outside.
            real = joined.canonicalize().map_err(|e| {
                let (looked, root) = (joined.display(), root.display());
                match e.kind() {
                    std::io::ErrorKind::NotFound => failed(format!("{path} not found (looked for {looked}, --root {root})")),
                    _ => failed(format!("{path}: {e} (looked for {looked}, --root {root})")),
                }
            })?;
            if !real.starts_with(root) {
                return Err(outside()); // it appeared, as a link out, since the check above
            }
            return Ok(real);
        }
        real.extend(rest.iter().rev());
        Ok(real)
    }
}

/// `path` with `.` and `..` resolved by name, before the filesystem is consulted (Windows does
/// the same, and so does joining onto a canonical root there). `None` if a `..` would climb
/// above the start of the path.
pub(crate) fn lexical(path: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut parts: Vec<Component> = Vec::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => match parts.last() {
                Some(Component::Normal(_)) => {
                    parts.pop();
                }
                _ => return None,
            },
            other => parts.push(other),
        }
    }
    Some(parts.iter().collect())
}

/// The file `name` inside `dir`, where `name` comes from a document or an argument: separators,
/// colons (a Windows drive or stream) and control characters become `_`, and a name of only dots
/// gets a leading `_`, so it is one plain file name and can't lead out of `dir`.
pub(crate) fn child(dir: &Path, name: &str) -> PathBuf {
    let mut safe: String = name.chars().map(|c| if matches!(c, '/' | '\\' | ':') || c.is_control() { '_' } else { c }).collect();
    if safe.chars().all(|c| c == '.') {
        safe.insert(0, '_');
    }
    dir.join(safe)
}

/// `<stem>.pdf` in `dir`, or `<stem> (2).pdf` and so on when that name is taken.
pub(crate) fn unused(dir: &Path, stem: &str) -> PathBuf {
    let first = child(dir, &format!("{stem}.pdf"));
    if !first.exists() {
        return first;
    }
    (2..10_000u32).map(|n| child(dir, &format!("{stem} ({n}).pdf"))).find(|p| !p.exists()).unwrap_or(first)
}

/// What one call wrote into a folder, so that a call that fails part way can take it back.
pub(crate) struct Written {
    /// The folders this call made, deepest first.
    made_dirs: Vec<PathBuf>,
    created: Vec<PathBuf>,
}

impl Written {
    /// Run `body`, which writes files into `dir` through [`Written::write`] or [`Written::record`].
    /// `dir` is made first. If `body` fails, the files it wrote to a name that was free are removed
    /// again, and the folders this call made, `dir` and any of its parents that did not exist,
    /// deepest first, also when making them failed part way (best effort: a folder that something else put a file in stays, and is listed).
    /// The error returned is `body`'s, followed by
    /// `; could not remove: <paths>` for whatever could not be removed. A file that replaced one
    /// already there is not removed, and what it replaced is not restored.
    pub(crate) fn within<T>(dir: &Path, body: impl FnOnce(&mut Written) -> Result<T>) -> Result<T> {
        // `dir` and each parent that is not there yet, deepest first. The walk stops at an empty
        // path (the start of a relative `dir`) and at the first folder that is there; a path that
        // ends in `..` or `.` names no folder of its own, so it is not listed (its parent is).
        let mut made_dirs = Vec::new();
        for path in dir.ancestors() {
            if path.as_os_str().is_empty() || path.symlink_metadata().is_ok() {
                break;
            }
            if path.file_name().is_some() {
                made_dirs.push(path.to_path_buf());
            }
        }
        let mut written = Written { made_dirs, created: Vec::new() };
        // A `create_dir_all` that fails part way has made some of the folders already.
        if let Err(e) = std::fs::create_dir_all(dir) {
            return Err(written.take_back(failed(format!("{}: {e}", dir.display()))));
        }
        match body(&mut written) {
            Ok(value) => Ok(value),
            Err(error) => Err(written.take_back(error)),
        }
    }

    /// [`write_atomic`] to `path`; the file is taken back on failure unless the name was taken.
    pub(crate) fn write(&mut self, path: &Path, bytes: &[u8]) -> Result<()> {
        // A dangling link counts as existing: the write replaces it, and it is not ours to remove.
        let existed = path.symlink_metadata().is_ok();
        write_atomic(path, bytes)?;
        if !existed {
            self.created.push(path.to_path_buf());
        }
        Ok(())
    }

    /// A file the call made at a name that was free (a publish that never replaces).
    pub(crate) fn record(&mut self, path: &Path) {
        self.created.push(path.to_path_buf());
    }

    fn take_back(self, error: ToolError) -> ToolError {
        let mut stuck: Vec<String> = self.created.iter().filter(|p| std::fs::remove_file(p).is_err()).map(|p| p.display().to_string()).collect();
        for made in &self.made_dirs {
            // A folder that is not there (the call did not get to make it) is not stuck.
            if std::fs::remove_dir(made).is_err() && made.symlink_metadata().is_ok() {
                stuck.push(made.display().to_string());
            }
        }
        if stuck.is_empty() { error } else { failed(format!("{error}; could not remove: {}", stuck.join(", "))) }
    }
}

/// Largest input file a tool reads into memory (512 MiB, matching `doc_batch`).
pub(crate) const MAX_INPUT: u64 = 512 * 1024 * 1024;

/// Read a resolved input file of at most [`MAX_INPUT`] bytes without buffering past the cap.
pub(crate) fn read_input_bytes(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;
    let file = std::fs::File::open(path)?;
    if file.metadata().is_ok_and(|m| m.is_file() && m.len() > MAX_INPUT) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "input exceeds 512 MiB"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_INPUT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_INPUT {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "input exceeds 512 MiB"));
    }
    Ok(bytes)
}

pub(crate) fn read_input(path: &Path) -> Result<Vec<u8>> {
    read_input_bytes(path).map_err(|e| failed(format!("{}: {e}", path.display())))
}

/// Write and sync a temporary file in the same directory, then rename over the target.
/// This flushes staged contents; it does not promise parent-directory durability after a
/// power loss. Filesystems that refuse sync cause the save to fail before publication.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    write_atomic_with(path, bytes, staging_suffixes())
}

/// [`write_atomic`], trying the staging names that `suffixes` give.
pub(crate) fn write_atomic_with(path: &Path, bytes: &[u8], suffixes: impl IntoIterator<Item = u64>) -> Result<()> {
    write_atomic_synced(path, bytes, suffixes, std::fs::File::sync_all)
}

pub(crate) fn write_atomic_synced(
    path: &Path,
    bytes: &[u8],
    suffixes: impl IntoIterator<Item = u64>,
    sync: impl FnOnce(&std::fs::File) -> std::io::Result<()>,
) -> Result<()> {
    // A folder can't be replaced, and staging beside it could land outside the root: "." names
    // the root itself, whose parent isn't ours to write in.
    if path.is_dir() {
        return Err(failed(format!("{}: is a folder, not a file", path.display())));
    }
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    path.file_name().ok_or_else(|| failed(format!("{}: not a file path", path.display())))?;
    std::fs::create_dir_all(dir).map_err(|e| failed(format!("{}: {e}", dir.display())))?;
    // The writer itself is the platform crate's, shared with the desktop app and the CLI
    // (#2166): the staging file is written, flushed (delayed writeback failures surface before
    // the destination is replaced or a document marked clean), closed and renamed; a failure at
    // any step removes it and leaves the destination as it was.
    pdfcraft_platform::staging::write_atomically_with(path, bytes, StagingName::SuffixThenTag, suffixes, sync)
        .map_err(|e| failed(format!("{}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A relative folder of new levels, which has no root to stop at: nothing is left, and the
    /// error is the body's, with no `could not remove` after it. Made under the working folder
    /// with a name no other test uses.
    #[test]
    fn a_failed_call_in_a_new_relative_folder_leaves_nothing_and_adds_nothing_to_the_error() {
        let top = format!("pdfcraft-within-relative-{}", std::process::id());
        for dir in [format!("{top}/a/b"), format!("{top}/a/.."), format!("{top}/a/../b")] {
            let error = Written::within(Path::new(&dir), |_| Err::<(), _>(failed("boom"))).expect_err("the body fails");
            assert_eq!(error.to_string(), failed("boom").to_string(), "{dir}");
            assert!(!Path::new(&top).exists(), "{dir}: the folders it made are removed again");
        }
    }
}
