use super::error::{Result, ToolError, failed};
use super::json::{Args, nest_rows, security, summary};
use super::paths::{Written, child, read_input, write_atomic};
use super::{Arc, Automation, BlockKey, DocId, Document, Edit, Path, PathBuf, TextBlock, Value, comments, file_access, json};

/// How far a watermark may be moved from the page centre, in points (a page is at most 14,400).
const WATERMARK_MAX_OFFSET: f64 = 20_000.0;

/// `doc_optimize` with `target_bytes`: (max target ppi, max JPEG quality) per extra attempt, mildest first.
/// The quality applies only to images the caller left on JPEG; zip and retain keep their encoding.
const TARGET_LADDER: [(f64, u8); 5] = [(110.0, 45), (96.0, 35), (72.0, 28), (54.0, 20), (36.0, 12)];

/// Bookmarks in one page of the bookmark list, by default and at most.
const BOOKMARK_PAGE: usize = 100;

const MAX_BOOKMARK_PAGE: usize = 1000;

impl Automation {
    // ---- documents ---------------------------------------------------------------------------

    /// One page of a document's bookmarks: `offset` (from 0) and `limit` (1 to `MAX_BOOKMARK_PAGE`).
    pub(crate) fn bookmark_listing(&self, a: &Args) -> Result<Value> {
        let id = a.int("doc")?;
        let id = DocId(u64::try_from(id).map_err(|_| ToolError::InvalidArgs("doc must be positive".into()))?);
        let offset = a.opt_int("offset")?.unwrap_or(0);
        let offset = usize::try_from(offset).map_err(|_| ToolError::InvalidArgs("offset must be 0 or more".into()))?;
        let limit = a.opt_int("limit")?.unwrap_or(BOOKMARK_PAGE as i64);
        let limit = usize::try_from(limit)
            .ok()
            .filter(|l| (1..=MAX_BOOKMARK_PAGE).contains(l))
            .ok_or_else(|| ToolError::InvalidArgs(format!("limit must be from 1 to {MAX_BOOKMARK_PAGE}")))?;
        let page = self
            .session
            .bookmark_page(id, offset, limit)
            .ok_or_else(|| ToolError::Failed(format!("no open document with id {} (see doc_list)", id.0)))?;
        let pages = &self.doc(a)?.info.pages;
        Ok(json!({ "bookmarks": nest_rows(&page.rows, pages), "next": page.next, "truncated": page.truncated }))
    }

    pub(crate) fn doc(&self, a: &Args) -> Result<&Document> {
        let id = a.int("doc")?;
        let id = u64::try_from(id).map_err(|_| ToolError::InvalidArgs("doc must be positive".into()))?;
        self.session.get(DocId(id)).ok_or_else(|| ToolError::Failed(format!("no open document with id {id} (see doc_list)")))
    }

    /// Most pages whose paragraph listing is remembered.
    const MAX_LISTINGS: usize = 256;

    /// Remember the paragraphs `text_paragraphs` just listed for `page`, so their numbers can be
    /// resolved against that listing.
    pub(crate) fn record_listing(&mut self, doc: DocId, page: usize, blocks: &[TextBlock]) {
        if self.listed.len() >= Self::MAX_LISTINGS && !self.listed.contains_key(&(doc, page)) {
            self.evicted.extend(self.listed.keys().map(|(d, _)| *d));
            self.listed.clear();
        }
        self.listed.insert((doc, page), blocks.iter().map(BlockKey::of).collect());
    }

    /// The index in `blocks` (the page's paragraphs now) of paragraph number `k` (1-based).
    ///
    /// After `text_paragraphs` a number names the paragraph that listing showed, wherever it is
    /// now. Without a listing it is a position in `blocks`. A paragraph an edit merged into a
    /// neighbour (or changed) is refused: the number would otherwise rewrite another paragraph
    /// and lose its text (#2516). So is any number on a page of a document whose listings were
    /// dropped at the cap, until the page is listed again.
    pub(crate) fn paragraph_index(&self, doc: DocId, page: usize, k: i64, blocks: &[TextBlock]) -> Result<usize> {
        let keys = self.listed.get(&(doc, page));
        if keys.is_none() && self.evicted.contains(&doc) {
            return Err(ToolError::InvalidArgs(format!(
                "page {} has no paragraph listing any more: too many pages were listed and the oldest listings were dropped. List the page again with text_paragraphs",
                page + 1
            )));
        }
        let n = keys.map_or(blocks.len(), Vec::len);
        let i = usize::try_from(k)
            .ok()
            .and_then(|k| k.checked_sub(1))
            .filter(|i| *i < n)
            .ok_or_else(|| ToolError::InvalidArgs(format!("paragraph {k} is out of range: page {} has {n} paragraphs", page + 1)))?;
        let Some(keys) = keys else { return Ok(i) };
        keys.get(i).and_then(|key| key.find(blocks, i)).ok_or_else(|| {
            ToolError::InvalidArgs(format!(
                "paragraph {k} of page {} is no longer a paragraph of its own: an edit changed it or how the page is grouped. List the page again with text_paragraphs",
                page + 1
            ))
        })
    }

    /// Update the listing after paragraph `k` was edited into `edited`, one of the page's
    /// paragraphs `after`. Returns the numbers of the listed paragraphs that are gone.
    pub(crate) fn follow_edit(&mut self, doc: DocId, page: usize, k: i64, edited: Option<&TextBlock>, after: &[TextBlock]) -> Vec<usize> {
        let Some(keys) = self.listed.get_mut(&(doc, page)) else { return Vec::new() };
        if let (Some(edited), Some(key)) = (edited, usize::try_from(k).ok().and_then(|k| k.checked_sub(1)).and_then(|i| keys.get_mut(i))) {
            *key = BlockKey::of(edited);
        }
        keys.iter().enumerate().filter(|(i, key)| key.find(after, *i).is_none()).map(|(i, _)| i + 1).collect()
    }

    /// The document, if its security allows copying text and images out of it (Table 22, bit 5).
    pub(crate) fn doc_to_copy(&self, a: &Args) -> Result<&Document> {
        let doc = self.doc(a)?;
        doc.check_copying().map_err(failed)?;
        Ok(doc)
    }

    pub(crate) fn doc_open(&mut self, a: &Args) -> Result<Value> {
        let requested = a.str("path")?;
        // `manual.pdf#nameddest=Intro`, as a help system starts a PDF reader (#3425): the file under
        // --root, and where it opens. A path that names a file is never split.
        let parameters = pdfcraft_engine::open_parameters::split(requested, |p| self.resolve(p, false).is_ok_and(|p| p.is_file()));
        let (requested, destination) = match &parameters {
            Some((file, destination)) => (file.as_str(), Some(destination.clone())),
            None => (requested, None),
        };
        let path = self.resolve(requested, false)?;
        let bytes = read_input(&path)?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let id = self.session.open(name, Some(path.to_string_lossy().into_owned()), Arc::new(bytes), a.opt_str("password")?).map_err(failed)?;
        let doc = self.session.get(id).ok_or_else(|| failed("the document vanished"))?;
        let mut value = summary(doc);
        if let Some(destination) = destination {
            let (applied, destination, warning) = crate::links::destination_report(doc, destination);
            value["destination_applied"] = json!(applied);
            value["destination"] = destination;
            value["warning"] = json!(warning);
        }
        Ok(value)
    }

    pub(crate) fn doc_close(&mut self, a: &Args) -> Result<Value> {
        let doc = self.doc(a)?;
        let id = doc.id;
        let discard_changes = a.opt_bool("discard_changes")?.unwrap_or(false);
        if doc.dirty && !discard_changes {
            return Err(failed("the document has unsaved changes: save it with doc_save, or pass discard_changes: true"));
        }
        // The document's Will Close actions run first; what they change goes with the document.
        let will_close = self.session.run_document_actions(id, pdfcraft_engine::js::DocumentScriptEvent::WillClose);
        self.session.close(id);
        self.renderers.remove(&id);
        self.texts.remove(&id);
        self.listed.retain(|(d, _), _| *d != id);
        self.evicted.remove(&id);
        self.ocr_reviews.remove(&id);
        let mut out = json!({ "closed": id.0 });
        if !will_close.is_empty() {
            out["will_close"] = crate::a11y::form_tools::js_output_json(&will_close);
        }
        Ok(out)
    }

    pub(crate) fn doc_save(&mut self, a: &Args) -> Result<Value> {
        self.doc_save_with(a, write_atomic)
    }

    pub(crate) fn doc_save_with(&mut self, a: &Args, write: impl FnOnce(&Path, &[u8]) -> Result<()>) -> Result<Value> {
        let doc = self.doc(a)?;
        let id = doc.id;
        let explicit = a.opt_str("path")?;
        let target = match explicit {
            Some(p) => self.resolve(p, true)?,
            None => {
                // Saving in place writes where the file was opened from, which the read root
                // allows and the write root may not. It also goes through --root again: a
                // folder on the remembered path may have become a link out since (#1791).
                let own = doc.path.clone().ok_or_else(|| failed("the document has never been saved: pass a path"))?;
                self.resolve(&own, true)?
            }
        };
        let same_file = explicit.is_none() || doc.path.as_deref().is_some_and(|p| Path::new(p) == target);
        // Saving to a new file is a full rewrite unless asked otherwise, like Save As.
        let full = a.opt_bool("full")?.unwrap_or(!same_file);
        let flatten_fill_sign = a.opt_bool("flatten_fill_sign")?.unwrap_or(false);
        // The flatten is an undoable edit made only to be saved. When it added a step and the save
        // then fails, that step is taken back, so a refused save leaves the document as it was.
        // Not when something else changed the document after it (a Will Save script): the last
        // step is then not the flatten, so the document is left alone and the error says so.
        let before = self.doc(a)?.edit_generation();
        if flatten_fill_sign {
            self.apply(a, Edit::FlattenFillSign)?;
        }
        let doc = self.doc(a)?;
        let flattened = (doc.edit_generation() != before).then(|| (doc.edit_generation(), doc.can_undo().map(str::to_owned)));
        let saved = self.save_prepared(a, id, &target, full, write);
        if let (Err(error), Some((generation, label))) = (&saved, flattened) {
            let doc = self.doc(a)?;
            if doc.edit_generation() == generation && doc.can_undo() == label.as_deref() {
                if let Err(undo) = self.session.discard_last_edit(id) {
                    return Err(failed(format!("{error}; the Fill & Sign flatten could not be taken back: {undo}")));
                }
            } else {
                return Err(failed(format!(
                    "{error}; the Fill & Sign flatten was kept, because the document changed after it (a Will Save script): undo it to take it back"
                )));
            }
        }
        let (mut out, scripts) = saved?;
        // The document's Did Save actions run once the file is written (a save that failed ran none).
        // What they change is not in the file: the document is left with unsaved changes.
        let did_save = self.session.run_document_actions(id, pdfcraft_engine::js::DocumentScriptEvent::DidSave);
        if let Some(o) = out.as_object_mut() {
            if !scripts.is_empty() {
                o.insert("will_save".into(), crate::a11y::form_tools::js_output_json(&scripts));
            }
            if !did_save.is_empty() {
                o.insert("did_save".into(), crate::a11y::form_tools::js_output_json(&did_save));
            }
            if let Some(doc) = self.session.get(id) {
                o.insert("document".into(), summary(doc));
            }
        }
        Ok(out)
    }

    /// Run the Will Save actions, make the bytes and publish them: the part of a save that can fail.
    fn save_prepared(
        &mut self,
        a: &Args,
        id: DocId,
        target: &Path,
        full: bool,
        write: impl FnOnce(&Path, &[u8]) -> Result<()>,
    ) -> Result<(Value, pdfcraft_engine::js::JsOutput)> {
        // The document's Will Save actions run first: what they change is saved (#269).
        let scripts = self.session.run_document_actions(id, pdfcraft_engine::js::DocumentScriptEvent::WillSave);
        let bytes = if full { self.session.save_full_bytes(id) } else { self.session.save_bytes(id) }.map_err(failed)?;
        // A save that adds or removes encryption, or must not keep the old revision (Sanitize,
        // redaction), is rewritten in full even when an incremental save was asked for.
        let incremental = !full && bytes.starts_with(&self.doc(a)?.saved_bytes());
        let out = self.publish_save(id, target, bytes, incremental, write)?;
        Ok((out, scripts))
    }

    /// Rename the document's file in its own folder (#2102). Both the old and the new name are
    /// writes there, so the automation write root must allow the folder; unsaved changes are refused
    /// rather than left behind in a file that no longer matches the document.
    pub(crate) fn doc_rename(&mut self, a: &Args) -> Result<Value> {
        let doc = self.doc(a)?;
        let id = doc.id;
        let own = doc.path.clone().ok_or_else(|| failed("the document has no file yet: save it with doc_save and a path first"))?;
        let name = a.str("name")?;
        let own_path = PathBuf::from(&own);
        self.resolve_for(&own, file_access::Access::Write, true)?;
        if let Some(target) = pdfcraft_engine::rename_target(&own_path, name).map_err(failed)? {
            let shown = target.to_string_lossy().into_owned();
            self.resolve_for(&shown, file_access::Access::Write, true)?;
        }
        // The move is made below the write folder where the system allows it (Linux: no link on the
        // way is followed, so a folder swapped for a link after the checks above can't carry the
        // file out). macOS and Windows keep the check-then-rename model.
        let root = match (self.roots.as_ref().and_then(|r| r.write_root()), self.root.as_deref()) {
            (Some(w), Some(r)) => Some(if r.starts_with(w) { r } else { w }),
            (w, r) => w.or(r),
        }
        .map(Path::to_path_buf);
        let path = self.session.rename_file_in(id, name, root.as_deref()).map_err(failed)?;
        let doc = self.session.get(id).ok_or_else(|| failed("the renamed document is no longer open"))?;
        Ok(json!({ "path": path, "name": doc.name.clone(), "document": summary(doc) }))
    }

    /// Validate the prepared bytes before publishing them or marking the document saved.
    pub(crate) fn publish_save(
        &mut self,
        id: DocId,
        target: &Path,
        bytes: Arc<Vec<u8>>,
        incremental: bool,
        write: impl FnOnce(&Path, &[u8]) -> Result<()>,
    ) -> Result<Value> {
        self.session.check_save(id, &bytes).map_err(failed)?;
        write(target, &bytes)?;
        let path = target.to_string_lossy().into_owned();
        self.session.mark_saved(id, bytes.clone(), Some(path.clone())).map_err(failed)?;
        let doc = self.session.get(id).ok_or_else(|| failed("the saved document is no longer open"))?;
        Ok(json!({ "path": path, "bytes": bytes.len(), "incremental": incremental, "document": summary(doc) }))
    }

    pub(crate) fn doc_revisions(&mut self, a: &Args) -> Result<Value> {
        let doc = self.doc(a)?;
        let ends = doc.revision_ends();
        let sigs = doc.signatures.clone();
        let signed_by = signers_by_revision(&ends, sigs.iter().enumerate().filter(|(_, s)| s.signed).map(|(i, s)| (i, s.signed_len)));
        let list: Vec<Value> = ends
            .iter()
            .zip(&signed_by)
            .enumerate()
            .map(|(i, (end, signers))| {
                let start = if i == 0 { 0 } else { ends[i - 1] };
                let signed: Vec<&str> = signers.iter().filter_map(|&k| sigs.get(k)).map(|s| s.field.as_str()).collect();
                json!({ "revision": i + 1, "end": end, "bytes": end - start, "signed_by": signed })
            })
            .collect();
        Ok(json!({ "revisions": list, "unsaved_changes": doc.dirty }))
    }

    pub(crate) fn af_list(&mut self, a: &Args) -> Result<Value> {
        let doc = self.doc(a)?;
        let list = doc.associated_list();
        let (without_relationship, not_embedded) = (list.without_relationship, list.not_embedded);
        let rows: Vec<Value> = list
            .files
            .into_iter()
            .map(|f| {
                json!({
                    "level": if f.page.is_some() { "page" } else { "document" },
                    "page": f.page.map(|p| p + 1),
                    "index": f.index,
                    "relationship": f.relationship,
                    "name": f.name,
                    "description": f.description,
                    "mime": f.mime,
                    "size": f.size,
                    "embedded": f.embedded,
                })
            })
            .collect();
        Ok(json!({
            "associated_files": rows,
            "count": rows.len(),
            "without_relationship": without_relationship,
            "not_embedded": not_embedded,
            "truncated": list.truncated,
            "unsaved_changes": doc.dirty,
        }))
    }

    pub(crate) fn dpart_list(&mut self, a: &Args) -> Result<Value> {
        let doc = self.doc(a)?;
        let list = doc.document_parts();
        let rows: Vec<Value> = list
            .parts
            .into_iter()
            .map(|p| {
                let metadata: Vec<Value> = p.metadata.into_iter().map(|(key, value)| json!({ "key": key, "value": value })).collect();
                json!({
                    "object": p.object,
                    "parent": p.parent,
                    "depth": p.depth,
                    "start_page": p.start_page,
                    "end_page": p.end_page,
                    "range_inverted": p.range_inverted,
                    "range_unresolved": p.range_unresolved,
                    "parent_mismatch": p.parent_mismatch,
                    "children": p.children,
                    "metadata": metadata,
                    "metadata_truncated": p.metadata_truncated,
                })
            })
            .collect();
        Ok(json!({
            "has_root": list.has_root,
            "record_level": list.record_level,
            "parts": rows,
            "count": rows.len(),
            "skipped": list.skipped,
            "inverted_ranges": list.inverted_ranges,
            "unresolved_ranges": list.unresolved_ranges,
            "parent_mismatches": list.parent_mismatches,
            "truncated": list.truncated,
            "unsaved_changes": doc.dirty,
        }))
    }

    pub(crate) fn ns_list(&mut self, a: &Args) -> Result<Value> {
        let doc = self.doc(a)?;
        let list = doc.structure_namespaces();
        let rows: Vec<Value> = list
            .namespaces
            .into_iter()
            .map(|n| {
                let roles: Vec<Value> =
                    n.roles.into_iter().map(|r| json!({ "type": r.structure_type, "role": r.role, "role_namespace": r.role_namespace, "role_namespace_object": r.role_namespace_object, "role_namespace_listed": r.role_namespace_listed })).collect();
                json!({
                    "object": n.object,
                    "ns": n.ns,
                    "has_schema": n.has_schema,
                    "role_count": n.role_count,
                    "roles": roles,
                })
            })
            .collect();
        let duplicates: Vec<Value> = list.duplicates.into_iter().map(|d| json!({ "ns": d.ns, "count": d.count, "objects": d.objects })).collect();
        Ok(json!({
            "has_structure_tree": list.has_structure_tree,
            "namespaces": rows,
            "duplicate_ns": duplicates,
            "unlisted_role_namespaces": list.unlisted_role_namespaces,
            "without_ns": list.without_ns,
            "count": rows.len(),
            "skipped": list.skipped,
            "truncated": list.truncated,
            "unsaved_changes": doc.dirty,
        }))
    }

    pub(crate) fn af_add(&mut self, a: &Args) -> Result<Value> {
        let path = self.resolve(a.str("path")?, false)?;
        let relationship = a.str("relationship")?.to_string();
        // Refuse a file past the cap before reading it, as the engine would after.
        let limit = pdfcraft_engine::MAX_AF_BYTES as u64;
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > limit) {
            return Err(ToolError::InvalidArgs(format!("the file is larger than the {limit}-byte limit for an associated file")));
        }
        let bytes = read_input(&path)?;
        let name = match a.opt_str("name")? {
            Some(n) => n.to_string(),
            None => path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        };
        // Without a known type the file is stored as application/octet-stream (PDF/A-3 wants a media type).
        let mime = Some(pdfcraft_engine::mime_for_name(&name).unwrap_or("application/octet-stream").to_string());
        let description = a.opt_str("description")?.map(str::to_string);
        // PDF/A-1 forbids embedded files and PDF/A-2 allows only PDF/A ones: say so, and add anyway.
        let mut warnings = Vec::new();
        if let Some((part @ (1 | 2), _)) = self.doc(a)?.standards().pdfa {
            warnings.push(format!(
                "the document declares PDF/A-{part}, which {}; the file was added and the document may no longer conform (standards_remove_id takes the claim off)",
                if part == 1 { "forbids embedded files" } else { "allows only embedded PDF/A files" }
            ));
        }
        let before = self.doc(a)?.associated_files().into_iter().filter(|f| f.page.is_none()).count();
        let out = self.apply(a, pdfcraft_engine::Edit::AddAssociatedFile { name, bytes, relationship, description, mime })?;
        let added = self.doc(a)?.associated_files().into_iter().filter(|f| f.page.is_none()).nth(before);
        Ok(json!({
            "index": added.as_ref().map(|f| f.index),
            "name": added.as_ref().and_then(|f| f.name.clone()),
            "relationship": added.and_then(|f| f.relationship),
            "warnings": warnings,
            "document": out,
        }))
    }

    pub(crate) fn af_remove(&mut self, a: &Args) -> Result<Value> {
        let index = usize::try_from(a.int("index")?).map_err(|_| ToolError::InvalidArgs("index counts from 1".into()))?;
        let keep_attachment = a.opt_bool("keep_attachment")?.unwrap_or(false);
        let shared = self.doc(a)?.associated_file_is_shared(index);
        let out = self.apply(a, pdfcraft_engine::Edit::RemoveAssociatedFile { index, keep_attachment })?;
        let note = if keep_attachment {
            "the attachment was kept as asked"
        } else if shared {
            "the attachment was kept because another /AF entry (the catalog's or a page's) still lists the same file"
        } else {
            "the attachment of this file, if it had one, was removed"
        };
        Ok(json!({ "attachment_kept": keep_attachment || shared, "shared_with_other_af": shared, "note": note, "document": out }))
    }

    pub(crate) fn af_set_relationship(&mut self, a: &Args) -> Result<Value> {
        let index = usize::try_from(a.int("index")?).map_err(|_| ToolError::InvalidArgs("index counts from 1".into()))?;
        let relationship = a.str("relationship")?.to_string();
        let previous = self.doc(a)?.associated_files().into_iter().find(|f| f.page.is_none() && f.index == index).and_then(|f| f.relationship);
        let was_indirect = self.doc(a)?.associated_array_is_indirect();
        let out = self.apply(a, pdfcraft_engine::Edit::SetAssociatedRelationship { index, relationship: relationship.clone() })?;
        // A specification given by reference changes where it is written, so who lists it is the
        // same after as before. One written in the array itself moves the catalog to a new direct
        // array, which a page that listed the old indirect array no longer shares.
        let shared = self.doc(a)?.associated_file_is_shared(index);
        let array_copied = was_indirect && !self.doc(a)?.associated_array_is_indirect();
        Ok(json!({
            "index": index,
            "relationship": relationship,
            "previous": previous,
            "shared_with_other_af": shared,
            "array_copied": array_copied,
            "document": out,
        }))
    }

    pub(crate) fn doc_linearization(&mut self, a: &Args) -> Result<Value> {
        let doc = self.doc(a)?;
        let r = doc.linearization();
        let hints: Vec<Value> = r.hint_streams.iter().map(|h| json!({ "offset": h.offset, "length": h.length, "object": h.object })).collect();
        Ok(json!({
            "linearized": r.linearized,
            "valid": r.valid,
            "version": r.version,
            "file_length": r.file_length,
            "declared_length": r.declared_length,
            "appended_bytes": r.appended_bytes,
            "first_page_object": r.first_page_object,
            "first_page_end": r.first_page_end,
            "page_count": r.page_count,
            "main_xref_offset": r.main_xref_offset,
            "first_page_number": r.first_page_number,
            "hint_streams": hints,
            "problems": r.problems,
            "unsaved_changes": doc.dirty,
        }))
    }

    pub(crate) fn doc_initial_view(&mut self, a: &Args) -> Result<Value> {
        use pdfcraft_engine::{InitialLayout as L, Magnification as M, Navigation as N};
        let bad = |m: String| ToolError::InvalidArgs(m);
        let mut v = self.doc(a)?.initial_view();
        let before = v.clone();
        if let Some(n) = a.opt_str("navigation")? {
            v.navigation = match n {
                "page" => N::PageOnly,
                "bookmarks" => N::Bookmarks,
                "pages" => N::Pages,
                "attachments" => N::Attachments,
                "layers" => N::Layers,
                o => return Err(bad(format!("unknown navigation {o:?}"))),
            };
        }
        if let Some(l) = a.opt_str("layout")? {
            v.layout = match l {
                "default" => L::Default,
                "single" => L::SinglePage,
                "continuous" => L::SinglePageContinuous,
                "two_up" => L::TwoUp,
                "two_up_continuous" => L::TwoUpContinuous,
                "two_up_cover" => L::TwoUpCoverPage,
                "two_up_continuous_cover" => L::TwoUpContinuousCoverPage,
                o => return Err(bad(format!("unknown layout {o:?}"))),
            };
        }
        match a.get("magnification") {
            None => {}
            Some(Value::Number(p)) => v.magnification = M::Percent(p.as_f64().unwrap_or(100.0).clamp(1.0, 6400.0)),
            Some(Value::String(m)) => {
                v.magnification = match m.as_str() {
                    "default" => M::Default,
                    "actual" => M::ActualSize,
                    "fit_page" => M::FitPage,
                    "fit_width" => M::FitWidth,
                    "fit_height" => M::FitHeight,
                    "fit_visible" => M::FitVisible,
                    o => return Err(bad(format!("unknown magnification {o:?}"))),
                }
            }
            Some(_) => return Err(bad("magnification is a name or a percentage".into())),
        }
        if let Some(p) = a.opt_int("page")? {
            v.page = (p.max(1) - 1) as usize;
        }
        for (k, f) in [
            ("fit_window", &mut v.fit_window),
            ("center_window", &mut v.center_window),
            ("full_screen", &mut v.full_screen),
            ("display_title", &mut v.display_title),
            ("hide_menubar", &mut v.hide_menubar),
            ("hide_toolbar", &mut v.hide_toolbar),
            ("hide_window_ui", &mut v.hide_window_ui),
        ] {
            if let Some(b) = a.opt_bool(k)? {
                *f = b;
            }
        }
        if let Some(l) = a.opt_str("language")? {
            v.language = (!l.trim().is_empty()).then(|| l.trim().to_string());
        }
        if let Some(b) = a.opt_str("binding")? {
            v.right_to_left = match b {
                "left" => false,
                "right" => true,
                o => return Err(bad(format!("binding is left or right, not {o:?}"))),
            };
        }
        let mut out = if v != before { self.apply(a, Edit::SetInitialView(Box::new(v.clone())))? } else { json!({}) };
        out["initial_view"] = json!({
            "navigation": format!("{:?}", v.navigation),
            "layout": format!("{:?}", v.layout),
            "magnification": format!("{:?}", v.magnification),
            "page": v.page + 1,
            "fit_window": v.fit_window,
            "center_window": v.center_window,
            "full_screen": v.full_screen,
            "display_title": v.display_title,
            "hide_menubar": v.hide_menubar,
            "hide_toolbar": v.hide_toolbar,
            "hide_window_ui": v.hide_window_ui,
            "language": v.language,
            "binding": if v.right_to_left { "right" } else { "left" },
        });
        Ok(out)
    }

    /// Document Properties ▸ Advanced ▸ Print Dialog Presets: read, and change what is given.
    pub(crate) fn doc_print_presets(&mut self, a: &Args) -> Result<Value> {
        use pdfcraft_engine::{Duplex, MAX_PRINT_COPIES, MAX_PRINT_RANGES, MIN_PRINT_COPIES, PrintScaling};
        let bad = |m: String| ToolError::InvalidArgs(m);
        let (mut p, unreadable_before) = self.doc(a)?.print_presets();
        let before = p.clone();
        if let Some(s) = a.opt_str("scaling")? {
            p.scaling = match s {
                "default" => PrintScaling::AppDefault,
                "none" => PrintScaling::None,
                o => return Err(bad(format!("unknown scaling {o:?} (default, none)"))),
            };
        }
        if let Some(d) = a.opt_str("duplex")? {
            p.duplex = match d {
                "default" => None,
                "simplex" => Some(Duplex::Simplex),
                "flip_short_edge" => Some(Duplex::FlipShortEdge),
                "flip_long_edge" => Some(Duplex::FlipLongEdge),
                o => return Err(bad(format!("unknown duplex {o:?} (default, simplex, flip_short_edge, flip_long_edge)"))),
            };
        }
        match a.get("pick_tray_by_size") {
            None => {}
            Some(Value::Bool(b)) => p.pick_tray_by_size = Some(*b),
            Some(Value::String(s)) if s == "default" => p.pick_tray_by_size = None,
            Some(_) => return Err(Args::wrong("pick_tray_by_size", "true, false or \"default\" (no entry)")),
        }
        match a.get("copies") {
            None => {}
            Some(Value::String(s)) if s == "default" => p.copies = None,
            Some(v) => {
                let c = v.as_i64().ok_or_else(|| Args::wrong("copies", "a whole number or \"default\" (no entry)"))?;
                p.copies = Some(u8::try_from(c).ok().filter(|c| (MIN_PRINT_COPIES..=MAX_PRINT_COPIES).contains(c)).ok_or_else(|| {
                    bad(format!("copies must be a whole number from {MIN_PRINT_COPIES} to {MAX_PRINT_COPIES} or \"default\", not {c}"))
                })?);
            }
        }
        if let Some(v) = a.get("page_ranges") {
            let wrong =
                || Args::wrong("page_ranges", "an array of [first, last] page pairs, 1-based, such as [[1, 3], [7, 7]], or [] for every page");
            let list = v.as_array().ok_or_else(wrong)?;
            if list.len() > MAX_PRINT_RANGES {
                return Err(bad(format!("page_ranges holds at most {MAX_PRINT_RANGES} ranges, not {}", list.len())));
            }
            let mut ranges = Vec::with_capacity(list.len());
            let whole = |x: &Value| x.as_u64().and_then(|n| usize::try_from(n).ok());
            for item in list {
                // A pair is exactly two whole numbers, so both reads below are of a checked length.
                let (first, last) = match item.as_array().map(Vec::as_slice) {
                    Some([first, last]) => (whole(first), whole(last)),
                    _ => return Err(wrong()),
                };
                ranges.push((first.ok_or_else(wrong)?, last.ok_or_else(wrong)?));
            }
            p.page_ranges = ranges;
        }
        let (mut out, now, unreadable) = if p != before {
            let out = self.apply(a, Edit::SetPrintPresets(Box::new(p.clone())))?;
            let (now, unreadable) = self.doc(a)?.print_presets();
            (out, now, unreadable)
        } else {
            (json!({}), before, unreadable_before)
        };
        out["print_presets"] = json!({
            "scaling": match now.scaling { PrintScaling::AppDefault => "default", PrintScaling::None => "none" },
            "duplex": match now.duplex {
                None => "default",
                Some(Duplex::Simplex) => "simplex",
                Some(Duplex::FlipShortEdge) => "flip_short_edge",
                Some(Duplex::FlipLongEdge) => "flip_long_edge",
            },
            "pick_tray_by_size": now.pick_tray_by_size.map_or(json!("default"), Value::Bool),
            "copies": now.copies.map_or(json!("default"), |c| json!(c)),
            "page_ranges": now.page_ranges.iter().map(|(f, l)| json!([f, l])).collect::<Vec<_>>(),
            "unreadable": unreadable,
        });
        Ok(out)
    }

    /// Document Properties ▸ Advanced ▸ base URL for relative links (`/URI` `/Base`): read, and
    /// change when `base` is given (an empty string takes it away).
    pub(crate) fn doc_base_url(&mut self, a: &Args) -> Result<Value> {
        let wanted = a.opt_str("base")?.map(|b| (!b.is_empty()).then(|| b.to_owned()));
        let (stored, unreadable_before) = self.doc(a)?.base_url();
        // A stored base sent back as it is is no change, so it is not checked again.
        if let Some(Some(b)) = wanted.as_ref().filter(|w| **w != stored) {
            pdfcraft_engine::check_base_url(b).map_err(ToolError::InvalidArgs)?;
        }
        let (mut out, now, unreadable) = match wanted {
            Some(base) if base != stored => {
                let out = self.apply(a, Edit::SetBaseUrl(base))?;
                let (now, unreadable) = self.doc(a)?.base_url();
                (out, now, unreadable)
            }
            _ => (json!({}), stored, unreadable_before),
        };
        out["base_url"] = json!({
            "base": now,
            "links_use_it": now.as_deref().is_some_and(|b| pdfcraft_render::resolve_uri(Some(b), "x").is_some()),
            "unreadable": unreadable,
        });
        Ok(out)
    }

    /// The structure tree's role map (`/StructTreeRoot` `/RoleMap`): read, and change one entry
    /// when `role` and `maps_to` are given (an empty `maps_to` takes the entry away).
    pub(crate) fn role_map(&mut self, a: &Args) -> Result<Value> {
        let role = a.opt_str("role")?;
        let maps_to = a.opt_str("maps_to")?;
        let mut out = match (role, maps_to) {
            (None, None) => json!({}),
            (Some(role), Some(maps_to)) => {
                let maps_to = (!maps_to.is_empty()).then_some(maps_to);
                if self.doc(a)?.role_map_changes(role, maps_to).map_err(ToolError::Failed)? {
                    self.apply(a, Edit::SetRoleMap { role: role.to_owned(), maps_to: maps_to.map(str::to_owned) })?
                } else {
                    json!({})
                }
            }
            (Some(_), None) => return Err(ToolError::InvalidArgs("role needs maps_to (an empty string takes the entry away)".into())),
            (None, Some(_)) => return Err(ToolError::InvalidArgs("maps_to needs role".into())),
        };
        let map = self.doc(a)?.role_map();
        let entries: Vec<Value> =
            map.entries.iter().map(|e| json!({ "role": e.role, "maps_to": e.maps_to, "exact": e.exact, "loops": e.loops })).collect();
        out["role_map"] = json!({
            "has_structure_tree": map.has_structure_tree,
            "count": map.count,
            "truncated": map.truncated,
            "entries": entries,
            "unreadable": map.unreadable,
        });
        Ok(out)
    }

    /// The structure tree's class map (`/StructTreeRoot` `/ClassMap`): read, and take the entry
    /// named `remove` away when it is given.
    pub(crate) fn class_map(&mut self, a: &Args) -> Result<Value> {
        let mut out = match a.opt_str("remove")? {
            Some(class) if self.doc(a)?.class_map_removes(class) => {
                let mut out = self.apply(a, Edit::RemoveClassMapEntry(class.to_owned()))?;
                out["removed"] = json!(true);
                out
            }
            Some(_) => json!({ "removed": false }),
            None => json!({}),
        };
        let map = self.doc(a)?.class_map();
        let entries: Vec<Value> =
            map.entries.iter().map(|e| json!({ "class": e.class, "kind": e.kind, "items": e.items, "owners": e.owners, "exact": e.exact })).collect();
        out["class_map"] = json!({
            "has_structure_tree": map.has_structure_tree,
            "count": map.count,
            "truncated": map.truncated,
            "entries": entries,
            "unreadable": map.unreadable,
        });
        Ok(out)
    }

    /// The article threads (the catalog's `/Threads`): read, and take the one at position
    /// `remove` away when it is given.
    pub(crate) fn article_threads(&mut self, a: &Args) -> Result<Value> {
        let remove = match a.opt_int("remove")? {
            Some(i) => Some(usize::try_from(i).map_err(|_| ToolError::InvalidArgs("remove must be 0 or more".into()))?),
            None => None,
        };
        let mut out = match remove {
            Some(index) if self.doc(a)?.article_thread_removes(index) => {
                let mut out = self.apply(a, Edit::RemoveArticleThread(index))?;
                out["removed"] = json!(true);
                out
            }
            Some(_) => json!({ "removed": false }),
            None => json!({}),
        };
        let read = self.doc(a)?.article_threads();
        let threads: Vec<Value> = read
            .threads
            .iter()
            .map(|t| {
                json!({ "index": t.index, "title": t.title, "title_exact": t.title_exact, "beads": t.beads, "beads_capped": t.beads_capped, "closed": t.closed })
            })
            .collect();
        out["article_threads"] = json!({ "count": read.count, "truncated": read.truncated, "threads": threads, "unreadable": read.unreadable });
        Ok(out)
    }

    /// The named destinations: read, and take the one at position `remove` of the listing away
    /// when it is given.
    pub(crate) fn named_destinations(&mut self, a: &Args) -> Result<Value> {
        use pdfcraft_organize::destinations::{Removal, Source};
        let remove = match a.opt_int("remove")? {
            Some(i) if i < 0 => return Err(ToolError::InvalidArgs("remove must be 0 or more".into())),
            // A position this target's `usize` cannot hold is past every name.
            Some(i) => Some(usize::try_from(i).unwrap_or(usize::MAX)),
            None => None,
        };
        let removal = match remove {
            Some(index) => Some((index, self.doc(a)?.destination_removal(index))),
            None => None,
        };
        let mut out = match removal {
            Some((index, Removal::Yes)) => {
                let mut out = self.apply(a, Edit::RemoveNamedDestination(index))?;
                out["removed"] = json!(true);
                out
            }
            Some((_, Removal::Unsupported)) => {
                return Err(ToolError::InvalidArgs("that name is stored in an object this tool does not change (see removable)".into()));
            }
            Some((_, Removal::NoEntry)) => json!({ "removed": false }),
            None => json!({}),
        };
        let read = self.doc(a)?.named_destinations();
        let rows: Vec<Value> = read
            .destinations
            .iter()
            .map(|d| {
                let source = match d.source {
                    Source::Names => "names",
                    Source::Dests => "dests",
                };
                json!({ "index": d.index, "source": source, "name": d.name, "name_exact": d.name_exact, "page": d.page, "view": d.view, "removable": d.removable })
            })
            .collect();
        out["named_destinations"] =
            json!({ "truncated": read.truncated, "tree_capped": read.tree_capped, "destinations": rows, "unreadable": read.unreadable });
        Ok(out)
    }

    /// The portfolio collection (the catalog's `/Collection`): read, and take it away when
    /// `remove` is true.
    pub(crate) fn portfolio_collection(&mut self, a: &Args) -> Result<Value> {
        let mut out = match a.opt_bool("remove")? {
            Some(true) if self.doc(a)?.collection_removes() => {
                let mut out = self.apply(a, Edit::RemoveCollection)?;
                out["removed"] = json!(true);
                out
            }
            Some(true) => json!({ "removed": false }),
            _ => json!({}),
        };
        let read = self.doc(a)?.collection();
        let fields: Vec<Value> = read
            .fields
            .iter()
            .map(|f| {
                json!({ "key": f.key, "is_field": f.is_field, "name": f.name, "subtype": f.subtype, "order": f.order, "visible": f.visible, "editable": f.editable, "exact": f.exact })
            })
            .collect();
        out["collection"] = json!({
            "present": read.present,
            "has_entry": read.has_entry,
            "view": read.view,
            "initial_document": read.initial_document,
            "initial_document_cut": read.initial_document_cut,
            "count": read.count,
            "truncated": read.truncated,
            "capped": read.capped,
            "fields": fields,
        });
        Ok(out)
    }

    /// The reference XObjects (forms with a `/Ref`): read, and take the `/Ref` away from the one
    /// at position `remove` when it is given.
    pub(crate) fn reference_xobjects(&mut self, a: &Args) -> Result<Value> {
        let remove = match a.opt_int("remove")? {
            Some(i) => Some(usize::try_from(i).map_err(|_| ToolError::InvalidArgs("remove must be 0 or more".into()))?),
            None => None,
        };
        use pdfcraft_organize::reference_xobjects::Removal;
        let removal = match remove {
            Some(index) => Some((index, self.doc(a)?.reference_xobject_removal(index))),
            None => None,
        };
        let mut out = match removal {
            Some((index, Removal::Yes)) => {
                let mut out = self.apply(a, Edit::RemoveReferenceXObject(index))?;
                out["removed"] = json!(true);
                out
            }
            Some((_, Removal::Refused)) => {
                return Err(ToolError::InvalidArgs(
                    "that form is the catalog or a node of the page tree, which this tool does not change (see removable)".into(),
                ));
            }
            Some((_, Removal::NoEntry)) => json!({ "removed": false }),
            None => json!({}),
        };
        let read = self.doc(a)?.reference_xobjects();
        let items: Vec<Value> = read
            .items
            .iter()
            .map(|r| {
                json!({
                    "index": r.index,
                    "object": format!("{} {}", r.object.num, r.object.generation),
                    "file": r.file,
                    "page_index": r.page_index,
                    "page_label": r.page_label,
                    "has_id": r.has_id,
                    "removable": r.removable,
                    "exact": r.exact,
                })
            })
            .collect();
        out["reference_xobjects"] = json!({ "count": read.count, "truncated": read.truncated, "capped": read.capped, "items": items });
        Ok(out)
    }

    /// The screen annotations and what their `/A` holds: read, and take a sound, movie, rendition
    /// or rich-media action out of the one at position `remove` when it is given. There is no set.
    pub(crate) fn screen_actions(&mut self, a: &Args) -> Result<Value> {
        use pdfcraft_engine::ScreenActionChange as Change;
        let remove = match a.opt_int("remove")? {
            Some(i) => Some(usize::try_from(i).map_err(|_| ToolError::InvalidArgs("remove must be 0 or more".into()))?),
            None => None,
        };
        let mut out = json!({});
        if let Some(index) = remove {
            // What the removal takes out, as read before it.
            let before = self.doc(a)?.screen_actions().items.into_iter().find(|s| s.index == index);
            match self.doc(a)?.screen_action_change(index) {
                Change::Refused(why) => return Err(ToolError::Failed(why.into())),
                Change::Yes => {
                    out = self.apply(a, Edit::RemoveScreenAction(index))?;
                    out["changed"] = json!(true);
                    out["removed"] = json!(before.map(|s| json!({ "action": s.action, "kind": s.kind, "next": s.has_next })));
                }
                Change::NoChange | Change::NoEntry => out["changed"] = json!(false),
            }
        }
        let read = self.doc(a)?.screen_actions();
        let items: Vec<Value> = read
            .items
            .iter()
            .map(|s| {
                json!({
                    "index": s.index,
                    "object": format!("{} {}", s.object.num, s.object.generation),
                    "has_entry": s.has_entry,
                    "present": s.present,
                    "action": s.action,
                    "exact": s.exact,
                    "kind": s.kind,
                    "has_script": s.has_script,
                    "has_next": s.has_next,
                    "has_additional_actions": s.has_additional,
                    "removable": s.removable,
                })
            })
            .collect();
        out["screen_actions"] = json!({ "count": read.count, "truncated": read.truncated, "capped": read.capped, "items": items });
        Ok(out)
    }

    pub(crate) fn doc_optimize(&mut self, a: &Args) -> Result<Value> {
        use pdfcraft_engine::optimize::{Compatibility, Compression, ImageSettings, MonoCompression, Preset};
        const MONO_KEYS: [&str; 4] = ["downsample", "ppi", "above_ppi", "compression"];
        const IMAGE_KEYS: [&str; 5] = ["downsample", "ppi", "above_ppi", "compression", "quality"];
        let id = self.doc(a)?.id;
        let before = self.doc(a)?.bytes.len();
        let path = self.resolve(a.str("path")?, true)?;
        let mut settings = match a.opt_str("preset")? {
            None => Preset::Standard.settings(),
            Some(p) => Preset::from_name(p).ok_or_else(|| ToolError::InvalidArgs(format!("unknown preset {p:?} (standard, mobile)")))?.settings(),
        };
        if let Some(c) = a.opt_str("compatibility")? {
            settings.compatibility = Compatibility::from_name(c)
                .ok_or_else(|| ToolError::InvalidArgs(format!("unknown compatibility {c:?} (retain, 1.4, 1.5, 1.6, 1.7, 2.0)")))?;
        }
        let images = |key: &str, base: ImageSettings| -> Result<ImageSettings> {
            let Some(v) = a.get(key) else { return Ok(base) };
            let v = v.as_object().ok_or_else(|| {
                ToolError::InvalidArgs(format!("{key} must be an object: {{ downsample, ppi, above_ppi, compression: jpeg|zip|retain, quality }}"))
            })?;
            // A misspelled key would otherwise leave the preset's setting in place unnoticed.
            if let Some(k) = v.keys().find(|k| !IMAGE_KEYS.contains(&k.as_str())) {
                return Err(ToolError::InvalidArgs(format!("unknown {key} key {k:?} ({})", IMAGE_KEYS.join(", "))));
            }
            let mut s = base;
            // A null key counts as absent, as top-level arguments do (the `mono` block below does not).
            let given = |k: &str| v.get(k).filter(|x| !x.is_null());
            if let Some(d) = given("downsample") {
                s.downsample = d.as_bool().ok_or_else(|| ToolError::InvalidArgs(format!("{key} downsample must be true or false, not {d}")))?;
            }
            for (name, ppi) in [("ppi", &mut s.target_ppi), ("above_ppi", &mut s.above_ppi)] {
                if let Some(p) = given(name) {
                    *ppi = p
                        .as_f64()
                        .filter(|p| p.is_finite())
                        .ok_or_else(|| ToolError::InvalidArgs(format!("{key} {name} must be a number, not {p}")))?
                        .clamp(9.0, 2400.0);
                }
            }
            s.above_ppi = s.above_ppi.max(s.target_ppi);
            let quality = given("quality")
                .map(|q| {
                    q.as_u64().ok_or_else(|| ToolError::InvalidArgs(format!("{key} quality must be a whole number (clamped to 1 to 100), not {q}")))
                })
                .transpose()?;
            if let Some(c) = given("compression") {
                let q = quality.unwrap_or(60).clamp(1, 100) as u8;
                s.compression = match c.as_str() {
                    Some("jpeg") => Compression::Jpeg(q),
                    Some("zip" | "flate") => Compression::Flate,
                    Some("retain") => Compression::Retain,
                    _ => return Err(ToolError::InvalidArgs(format!("unknown {key} compression {c} (jpeg, zip, retain)"))),
                };
            }
            Ok(s)
        };
        settings.color = images("color", settings.color)?;
        settings.gray = images("gray", settings.gray)?;
        if let Some(mono) = a.get("mono") {
            let unknown = |c: &Value| ToolError::InvalidArgs(format!("unknown mono compression {c} (ccitt_g4, jbig2, retain)"));
            let mono = mono.as_object().ok_or_else(|| {
                ToolError::InvalidArgs("mono must be an object: { downsample, ppi, above_ppi, compression: ccitt_g4|jbig2|retain }".into())
            })?;
            // A misspelled key would otherwise leave the preset's setting in place unnoticed.
            if let Some(key) = mono.keys().find(|k| !MONO_KEYS.contains(&k.as_str())) {
                return Err(ToolError::InvalidArgs(format!("unknown mono key {key:?} ({})", MONO_KEYS.join(", "))));
            }
            let m = &mut settings.mono;
            if let Some(d) = mono.get("downsample") {
                m.downsample = d.as_bool().ok_or_else(|| ToolError::InvalidArgs(format!("mono downsample must be true or false, not {d}")))?;
            }
            for (key, ppi) in [("ppi", &mut m.target_ppi), ("above_ppi", &mut m.above_ppi)] {
                if let Some(p) = mono.get(key) {
                    *ppi = p
                        .as_f64()
                        .filter(|p| p.is_finite())
                        .ok_or_else(|| ToolError::InvalidArgs(format!("mono {key} must be a number, not {p}")))?
                        .clamp(9.0, 2400.0);
                }
            }
            m.above_ppi = m.above_ppi.max(m.target_ppi);
            if let Some(c) = mono.get("compression") {
                m.compression = match c.as_str() {
                    Some("ccitt_g4" | "ccitt" | "g4") => MonoCompression::CcittG4,
                    Some("jbig2") => MonoCompression::Jbig2,
                    Some("retain") => MonoCompression::Retain,
                    _ => return Err(unknown(c)),
                };
            }
        }
        for (key, flag) in [
            ("merge_fragments", &mut settings.merge_fragments),
            ("discard_thumbnails", &mut settings.discard_thumbnails),
            ("discard_alternate_images", &mut settings.discard_alternate_images),
            ("discard_tags", &mut settings.discard_tags),
            ("discard_search_index", &mut settings.discard_search_index),
            ("discard_print_settings", &mut settings.discard_print_settings),
            ("discard_form_actions", &mut settings.discard_form_actions),
            ("flate_unencoded", &mut settings.flate_unencoded),
            ("object_streams", &mut settings.object_streams),
            ("lzw_to_flate", &mut settings.lzw_to_flate),
            ("remove_invalid_links", &mut settings.remove_invalid_links),
            ("remove_unreferenced_dests", &mut settings.remove_unreferenced_dests),
            ("discard_external_refs", &mut settings.discard_external_refs),
        ] {
            if let Some(b) = a.opt_bool(key)? {
                *flag = b;
            }
        }
        let discard: Vec<pdfcraft_engine::Hidden> = match a.get("discard") {
            None => Vec::new(),
            Some(v) => v
                .as_array()
                .ok_or_else(|| ToolError::InvalidArgs("discard must be an array".into()))?
                .iter()
                .map(|x| {
                    x.as_str()
                        .and_then(pdfcraft_engine::Hidden::from_id)
                        .ok_or_else(|| ToolError::InvalidArgs(format!("unknown discard category {x}")))
                })
                .collect::<Result<_>>()?,
        };
        let target = match a.opt_int("target_bytes")? {
            None => None,
            Some(t) if t >= 1 => Some(usize::try_from(t).map_err(|_| ToolError::InvalidArgs(format!("target_bytes {t} is too large")))?),
            Some(t) => return Err(ToolError::InvalidArgs(format!("target_bytes must be at least 1, not {t}"))),
        };
        let (mut bytes, mut r) = self.session.optimized_bytes(id, &settings, &discard).map_err(failed)?;
        let mut attempts = 1;
        let mut step_used = 0;
        // Settings already optimized with; a rung that changes nothing is not run again.
        let mut tried = vec![settings.clone()];
        if let Some(t) = target {
            // Stronger image settings, one rung per attempt, until the copy fits. Every attempt
            // starts from the open document, so a failed one changes nothing; the number of
            // attempts is fixed. The smallest copy is kept if none fits.
            for (k, &(ppi, quality)) in TARGET_LADDER.iter().enumerate() {
                if bytes.len() <= t {
                    break;
                }
                let mut s = settings.clone();
                for img in [&mut s.color, &mut s.gray] {
                    img.downsample = true;
                    img.target_ppi = img.target_ppi.min(ppi);
                    img.above_ppi = img.above_ppi.min(ppi * 1.5).max(img.target_ppi);
                    // Only a caller who chose JPEG gets a lower quality; zip and retain stay as asked.
                    if let Compression::Jpeg(q) = img.compression {
                        img.compression = Compression::Jpeg(q.min(quality));
                    }
                }
                if tried.contains(&s) {
                    continue;
                }
                let (b2, r2) = self.session.optimized_bytes(id, &s, &discard).map_err(failed)?;
                tried.push(s);
                attempts += 1;
                if b2.len() < bytes.len() {
                    (bytes, r, step_used) = (b2, r2, k + 1);
                }
            }
        }
        write_atomic(&path, &bytes)?;
        fn valid_version(v: &[u8]) -> Option<&str> {
            if matches!(v, [b'1'..=b'9', b'.', b'0'..=b'9']) { std::str::from_utf8(v).ok() } else { None }
        }
        let catalog_version = (settings.compatibility == Compatibility::Retain)
            .then(|| {
                let pw = self.doc(a).ok().and_then(|d| d.password.as_deref());
                let doc = pdfcraft_cos::Document::open_with_password(bytes.clone(), pw).ok()?;
                let root = doc.root()?;
                // Through a reference too, as the optimizer reads it.
                let version = doc.get(root).as_dict()?.get(b"Version")?.clone();
                doc.resolve(&version).as_name().and_then(valid_version).map(str::to_owned)
            })
            .flatten();
        let header_version = bytes.strip_prefix(b"%PDF-").and_then(|b| b.get(..3)).and_then(valid_version);
        // Viewers read the later of the two (object streams raise the header past a lower catalog
        // entry). Both are "d.d", so text order is version order.
        let pdf_version = catalog_version.as_deref().into_iter().chain(header_version).max();
        let o = &r.optimize;
        let mut extra = json!({});
        if let Some(t) = target {
            let (ppi, quality) = step_used.checked_sub(1).and_then(|k| TARGET_LADDER.get(k)).copied().unzip();
            let lossy = matches!(settings.color.compression, Compression::Jpeg(_)) || matches!(settings.gray.compression, Compression::Jpeg(_));
            let quality = quality.filter(|_| lossy);
            extra = json!({
                "target_bytes": t,
                "target_reached": bytes.len() <= t,
                "attempts": attempts,
                "target_settings": { "step": step_used, "max_ppi": ppi, "max_jpeg_quality": quality },
            });
        }
        let mut out = json!({
            "path": path.to_string_lossy(),
            "pdf_version": pdf_version,
            "object_streams": settings.effective_object_streams(),
            "bytes_before": before,
            "bytes_after": bytes.len(),
            "merged_images": o.merged_images,
            "fragments_merged": o.fragments_merged,
            "images": o.images,
            "images_resampled": o.images_resampled,
            "images_recompressed": o.images_recompressed,
            "images_refused": o.images_refused,
            "image_bytes_before": o.image_bytes_before,
            "image_bytes_after": o.image_bytes_after,
            "thumbnails": o.thumbnails,
            "alternate_images": o.alternate_images,
            "search_indexes": o.search_indexes,
            "tags_removed": o.tags_removed,
            "print_settings": o.print_settings,
            "form_actions": o.form_actions,
            "streams_compressed": o.streams_compressed,
            "lzw_streams": o.lzw_streams,
            "invalid_links": o.invalid_links,
            "invalid_bookmarks": o.invalid_bookmarks,
            "unreferenced_dests": o.unreferenced_dests,
            "external_links": o.external_links,
            "external_bookmarks": o.external_bookmarks,
            "unused_xobjects": o.unused_xobjects,
            "merged_objects": r.merged,
            "discarded": r.discarded.iter().map(|(h, n)| json!({ "category": h.id(), "count": n })).collect::<Vec<_>>(),
        });
        if let (Some(o), Some(e)) = (out.as_object_mut(), extra.as_object()) {
            o.extend(e.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        Ok(out)
    }

    /// A watermark or background picture: `file` (an image or a PDF) and `file_page` (1-based).
    fn mark_file(&self, a: &Args) -> Result<Option<pdfcraft_engine::MarkFile>> {
        let Some(p) = a.opt_str("file")? else { return Ok(None) };
        let path = self.resolve(p, false)?;
        let bytes = read_input(&path)?;
        let page = a.opt_int("file_page")?.unwrap_or(1).max(1) as usize - 1;
        Ok(Some(pdfcraft_engine::MarkFile {
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            bytes: Arc::new(bytes),
            page,
        }))
    }

    pub(crate) fn apply(&mut self, a: &Args, edit: Edit) -> Result<Value> {
        let id = self.doc(a)?.id;
        self.session.apply(id, edit).map_err(failed)?;
        self.applied(a)
    }

    /// `text_prepare`: make a page's grouped text editable, with what that edit itself prepared,
    /// repaired and left grouped.
    pub(crate) fn prepare_text(&mut self, a: &Args, page: usize) -> Result<Value> {
        let id = self.doc(a)?.id;
        let prepared = self.session.prepare_text(id, page).map_err(failed)?;
        let mut out = self.applied(a)?;
        out["prepared_groups"] = json!(prepared.expanded);
        out["repaired_fonts"] = json!(prepared.repaired_fonts);
        out["refused_groups"] = json!(prepared.refused);
        Ok(out)
    }

    /// The document's summary after an edit, with what it stored as typed but drew with `?`.
    pub(crate) fn applied(&mut self, a: &Args) -> Result<Value> {
        let doc = self.doc(a)?;
        let mut out = summary(doc);
        // Text the edit stored as typed but drew with `?` (its font can't draw it): said, not silent.
        if !doc.edit_notices.is_empty() {
            out["warnings"] = json!(doc.edit_notices.iter().map(ToString::to_string).collect::<Vec<_>>());
        }
        // A paragraph whose font lacked letters of the new text was redrawn in a stand-in font:
        // which font, and which letters, as data (the warning says it in words).
        for notice in &doc.edit_notices {
            if let pdfcraft_engine::EditNotice::SubstituteFont { fonts, missing } = notice {
                out["substituted_fonts"] = json!(fonts);
                out["missing_letters"] = json!(missing.iter().collect::<String>());
            }
        }
        Ok(out)
    }

    /// Pages whose own backdrop hides a background (see `pdfcraft_edit::backdrop`).
    pub(crate) fn backdrops(&mut self, a: &Args) -> Result<Value> {
        let n = self.doc(a)?.info.pages.len();
        let pages = match a.opt_ints("pages")? {
            Some(_) => self.pages(a, "pages")?,
            None => (0..n).collect(),
        };
        let doc = self.doc(a)?;
        let found: Vec<Value> = pages
            .iter()
            .zip(doc.page_backdrops(&pages))
            .filter_map(|(p, found)| {
                let kind = match found? {
                    pdfcraft_engine::Backdrop::Fill => "fill",
                    pdfcraft_engine::Backdrop::Image => "image",
                };
                Some(json!({ "page": p + 1, "backdrop": kind }))
            })
            .collect();
        Ok(json!({ "pages": found }))
    }

    pub(crate) fn marks(&mut self, tool: &str, a: &Args) -> Result<Value> {
        use pdfcraft_engine::{Background, HeaderFooter, MarkKind, Watermark};
        let n = self.doc(a)?.info.pages.len();
        let pages = match a.opt_ints("pages")? {
            Some(_) => self.pages(a, "pages")?,
            None => (0..n).collect(),
        };
        let color =
            |key: &str, default: [f64; 3]| -> Result<[f64; 3]> { Ok(a.opt_str(key)?.map(comments::parse_color).transpose()?.unwrap_or(default)) };
        let replace = a.opt_bool("replace")?.unwrap_or(false);
        let edit = match tool {
            "doc_header_footer" => {
                let mut hf = HeaderFooter::default();
                for (k, key) in ["header_left", "header_center", "header_right", "footer_left", "footer_center", "footer_right"].iter().enumerate() {
                    hf.text[k] = a.opt_str(key)?.unwrap_or_default().to_string();
                }
                if let Some(s) = a.opt_num("font_size")? {
                    hf.font_size = s;
                }
                hf.font = match a.opt_str("font")? {
                    None | Some("Helvetica") => pdfcraft_engine::FontFamily::Helvetica,
                    Some("Times") => pdfcraft_engine::FontFamily::Times,
                    Some("Courier") => pdfcraft_engine::FontFamily::Courier,
                    Some(other) => {
                        return Err(ToolError::InvalidArgs(format!("font must be \"Helvetica\", \"Times\" or \"Courier\", not {other:?}")));
                    }
                };
                hf.color = color("color", hf.color)?;
                if let Some(m) = a.nums::<4>("margins")? {
                    hf.margins = m;
                }
                if let Some(s) = a.opt_int("start_number")? {
                    hf.start_number = s.clamp(1, u32::MAX as i64) as u32;
                }
                hf.shrink = a.opt_bool("shrink")?.unwrap_or(false);
                Edit::AddHeaderFooter { pages, settings: hf, replace }
            }
            "doc_watermark" => {
                let d = Watermark::default();
                let file = self.mark_file(a)?;
                let text = match &file {
                    Some(_) => a.opt_str("text")?.unwrap_or_default().to_string(),
                    None => a.str("text")?.to_string(),
                };
                let wm = Watermark {
                    text,
                    source: None,
                    scale: a.opt_num("scale")?.unwrap_or(d.scale).clamp(0.01, 1.0),
                    font_size: a.opt_num("font_size")?.unwrap_or(0.0),
                    color: color("color", d.color)?,
                    opacity: a.opt_num("opacity")?.unwrap_or(d.opacity),
                    rotation: a.opt_num("rotation")?.unwrap_or(d.rotation),
                    behind: a.opt_bool("behind")?.unwrap_or(false),
                    offset: [
                        a.opt_num("offset_x")?.unwrap_or(0.0).clamp(-WATERMARK_MAX_OFFSET, WATERMARK_MAX_OFFSET),
                        a.opt_num("offset_y")?.unwrap_or(0.0).clamp(-WATERMARK_MAX_OFFSET, WATERMARK_MAX_OFFSET),
                    ],
                };
                Edit::AddWatermark { pages, settings: wm, replace, file }
            }
            "doc_background" => Edit::AddBackground {
                pages,
                settings: Background {
                    color: color("color", [1.0; 3])?,
                    opacity: a.opt_num("opacity")?.unwrap_or(1.0),
                    scale: a.opt_num("scale")?.unwrap_or(1.0).clamp(0.01, 1.0),
                    replace_backdrop: a.opt_bool("replace_backdrop")?.unwrap_or(true),
                    ..Background::default()
                },
                replace,
                file: self.mark_file(a)?,
            },
            "doc_printer_marks" => {
                use pdfcraft_engine::PrinterMarks;
                let flag = |k: &str, default: bool| -> Result<bool> { Ok(a.opt_bool(k)?.unwrap_or(default)) };
                let (y, m, d) = self.session.today();
                let settings = PrinterMarks {
                    trim: flag("trim", true)?,
                    bleed: flag("bleed", true)?,
                    registration: flag("registration", true)?,
                    color_bars: flag("color_bars", true)?,
                    page_info: flag("page_info", true)?,
                    line_weight: a.opt_num("line_weight")?.unwrap_or(0.25),
                    make_room: match a.opt_num("make_room")? {
                        Some(r) if r < 0.0 => None,
                        Some(r) => Some(r),
                        None => Some(36.0),
                    },
                    label: a.opt_str("label")?.map(str::to_string).unwrap_or_else(|| self.doc(a).map(|d| d.name.clone()).unwrap_or_default()),
                    date: a.opt_str("date")?.map(str::to_string).unwrap_or_else(|| format!("{y:04}-{m:02}-{d:02}")),
                };
                Edit::AddPrinterMarks { pages, settings, replace: a.opt_bool("replace")?.unwrap_or(true) }
            }
            "doc_bates_remove" => {
                let chosen = a.opt_ints("pages")?.is_some().then_some(pages);
                Edit::RemoveMarks { kind: MarkKind::Bates, pages: chosen }
            }
            _ => {
                let kind = match a.str("kind")? {
                    "header_footer" => MarkKind::HeaderFooter,
                    "watermark" => MarkKind::Watermark,
                    "background" => MarkKind::Background,
                    "printer_marks" => MarkKind::PrinterMarks,
                    other => return Err(ToolError::InvalidArgs(format!("unknown kind {other:?}"))),
                };
                Edit::RemoveMarks { kind, pages: None }
            }
        };
        self.apply(a, edit)
    }

    pub(crate) fn export(&mut self, tool: &str, a: &Args) -> Result<Value> {
        let doc = self.doc_to_copy(a)?;
        let pages = match a.opt_ints("pages")? {
            Some(_) => self.pages(a, "pages")?,
            None => (0..doc.info.pages.len()).collect(),
        };
        let stem = doc.name.trim_end_matches(".pdf").trim_end_matches(".PDF").to_string();
        let mut ex = pdfcraft_engine::export::Exporter::new(doc);
        if tool == "doc_export_text" {
            let path = self.resolve(a.str("path")?, true)?;
            let text = ex.text_of(&pages).map_err(failed)?;
            write_atomic(&path, text.as_bytes())?;
            return Ok(json!({ "path": path.to_string_lossy(), "pages": pages.len(), "bytes": text.len() }));
        }
        let folder = self.resolve(a.str("folder")?, true)?;
        // min_size is read before the folder is made.
        if tool == "doc_export_all_images" {
            let min = u32::try_from(a.opt_int("min_size")?.unwrap_or(0).max(0)).unwrap_or(u32::MAX);
            let out = pdfcraft_engine::export::extract_images(&doc.export_source(), &pages, min).map_err(failed)?;
            return Written::within(&folder, |written| {
                let mut files = Vec::new();
                for (k, img) in out.images.iter().enumerate() {
                    let path = child(&folder, &pdfcraft_engine::export::image_file_name(&stem, img, k + 1, doc.info.pages.len()));
                    written.write(&path, &img.data)?;
                    files.push(json!({ "path": path.to_string_lossy(), "page": img.page + 1, "width": img.width, "height": img.height }));
                }
                let skipped: Vec<Value> = out.skipped.iter().map(|(p, r, why)| json!({ "page": p + 1, "object": r.num, "reason": why })).collect();
                Ok(json!({ "count": files.len(), "files": files, "skipped": skipped }))
            });
        }
        let dpi = a.opt_num("dpi")?.unwrap_or(150.0);
        let quality = a.opt_int("quality")?.unwrap_or(85).clamp(1, 100) as u8;
        let format = match a.opt_str("format")?.unwrap_or("png") {
            "png" => pdfcraft_engine::export::ImageFormat::Png,
            "jpeg" | "jpg" => pdfcraft_engine::export::ImageFormat::Jpeg { quality },
            "tiff" | "tif" => pdfcraft_engine::export::ImageFormat::Tiff,
            f => return Err(ToolError::InvalidArgs(format!("unknown format {f:?} (png, jpeg, tiff)"))),
        };
        let files = Written::within(&folder, |written| {
            let mut files = Vec::new();
            // A write error stops the export as it is, not as a page error.
            let mut write_error = None;
            let exported = ex.images(&pages, dpi, format, |p, img| {
                let path = child(&folder, &format!("{stem}_page_{}.{}", p + 1, format.extension()));
                written.write(&path, &img).map_err(|e| {
                    let message = e.to_string();
                    write_error = Some(e);
                    message
                })?;
                files.push(path.to_string_lossy().into_owned());
                Ok(())
            });
            if let Some(e) = write_error {
                return Err(e);
            }
            exported.map_err(failed)?;
            Ok(files)
        })?;
        let mut lowered = Vec::new();
        for &p in &pages {
            // A page too large for the renderer at `dpi` is drawn at the most it allows.
            let used = ex.dpi_used(p, dpi);
            if used < dpi.clamp(18.0, 1200.0) - 0.5 {
                lowered.push(json!({ "page": p + 1, "dpi": used.floor() }));
            }
        }
        if lowered.is_empty() {
            Ok(json!({ "count": files.len(), "files": files }))
        } else {
            Ok(json!({ "count": files.len(), "files": files, "lower_dpi": lowered }))
        }
    }

    pub(crate) fn doc_protect(&mut self, a: &Args) -> Result<Value> {
        use pdfcraft_engine::{Algorithm, Changes, Printing, Protection};
        let d = Protection::default();
        // Restrictions only exist behind a permissions password (ISO 32000-2 §7.6.4.4: /P is
        // enforced against the owner password; without one everything stays allowed). Refuse a
        // restriction that could not take effect instead of writing an unrestricted file (#134).
        let restriction = ["printing", "changes", "copy", "accessibility"].into_iter().find(|k| a.get(k).is_some());
        if let (None, Some(key)) = (a.opt_str("permissions_password")?, restriction) {
            return Err(ToolError::InvalidArgs(format!(
                "`{key}` needs `permissions_password`: with open_password alone the document is encrypted but nothing is restricted"
            )));
        }
        let p = Protection {
            open_password: a.opt_str("open_password")?.map(str::to_owned),
            permissions_password: a.opt_str("permissions_password")?.map(str::to_owned),
            printing: match a.opt_str("printing")? {
                None => d.printing,
                Some("none") => Printing::None,
                Some("low") => Printing::Low,
                Some("high") => Printing::High,
                Some(o) => return Err(ToolError::InvalidArgs(format!("unknown printing {o:?}"))),
            },
            changes: match a.opt_str("changes")? {
                None => d.changes,
                Some("none") => Changes::None,
                Some("pages") => Changes::Pages,
                Some("fill-sign") => Changes::FillSign,
                Some("comment-fill-sign") => Changes::CommentFillSign,
                Some("any-except-extract") => Changes::AnyExceptExtract,
                Some(o) => return Err(ToolError::InvalidArgs(format!("unknown changes {o:?}"))),
            },
            copy: a.opt_bool("copy")?.unwrap_or(d.copy),
            accessibility: a.opt_bool("accessibility")?.unwrap_or(d.accessibility),
            algorithm: match a.opt_str("compatibility")? {
                None | Some("aes-256") => Algorithm::Aes256,
                Some("aes-128") => Algorithm::Aes128,
                Some("rc4-128") => Algorithm::Rc4_128,
                Some("rc4-40") => Algorithm::Rc4_40,
                Some(o) => return Err(ToolError::InvalidArgs(format!("unknown compatibility {o:?}"))),
            },
            encrypt_metadata: a.opt_bool("encrypt_metadata")?.unwrap_or(d.encrypt_metadata),
        };
        let mut out = self.apply(a, Edit::Protect(p))?;
        out["security"] = security(self.doc(a)?);
        Ok(out)
    }
}

/// For each revision (`ends`, strictly increasing: `bytes[..end]` is that revision), the indexes
/// of the signatures that sign exactly it, in the order given. `signed` is each signed
/// signature's index and the length of the revision its byte range ends in; a signature signs the
/// revision `(start, end]` holds. One binary search per signature, so the work is
/// `signatures * log(revisions)` where a scan of every signature for every revision cost
/// `revisions * signatures` (a file's revisions and its signature fields are both file-controlled:
/// 100,000 revisions and 20,000 signature fields were 2 billion comparisons). A length past the
/// last revision, or 0, signs none. Each signature lands in at most one revision, so the names
/// listed are at most the names the document already holds.
fn signers_by_revision(ends: &[usize], signed: impl Iterator<Item = (usize, usize)>) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); ends.len()];
    for (index, len) in signed {
        if len == 0 {
            continue;
        }
        let revision = ends.partition_point(|&end| end < len);
        if let Some(slot) = out.get_mut(revision) {
            slot.push(index);
        }
    }
    out
}

#[cfg(test)]
mod revision_signer_tests {
    use super::signers_by_revision;

    #[test]
    fn a_signature_belongs_to_the_revision_its_length_falls_in() {
        let ends = [100, 250, 400];
        let signed = [(0, 100), (1, 101), (2, 250), (3, 400), (4, 401), (5, 0), (6, 1), (7, 100)];
        let got = signers_by_revision(&ends, signed.into_iter());
        assert_eq!(got, vec![vec![0, 6, 7], vec![1, 2], vec![3]]);
        assert!(signers_by_revision(&[], signed.into_iter()).is_empty());
    }

    #[test]
    fn many_revisions_and_signatures_are_matched_in_near_linear_time() {
        // 200,000 revisions and 200,000 signatures: matching every signature against every
        // revision would be 40 billion comparisons.
        let n = 200_000usize;
        let ends: Vec<usize> = (1..=n).map(|i| i * 10).collect();
        let started = std::time::Instant::now();
        let got = signers_by_revision(&ends, (0..n).map(|i| (i, (i + 1) * 10 - (i % 10))));
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "took {:?}", started.elapsed());
        assert_eq!(got.len(), n);
        assert!(got.iter().enumerate().all(|(r, v)| v == &[r]));
    }
}
