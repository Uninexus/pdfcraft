#[cfg(test)]
use super::check_refresh_panic;
use super::{
    Arc, DocId, Document, Edit, EditCtx, EditError, Label, MAX_UNDO, ObjectKind, OnceLock, RenderPool, SaveOptions, Scope, Session,
    XFA_REMOVED_PREFIX, XFA_SCRIPTS_OFF, carry_verdict_marks, check_permission, comment_list, display_bytes, display_config, guard, info_fields,
    inspect, js, keys_after, note_warnings, render_threads, run_edit, scan_colours, scope_of, shown_under_redactions, signatures_of, sync_xfa_state,
    undrawn_notices, uses_scripts, write_incremental, xfa, xfa_sync_datasets,
};
use crate::{EditNotice, MarkKind, print_preview_layers};

/// Add scan cover and ink to edits of invisible OCR text, returning no edit for an unchanged draft.
fn prepare_ocr_text_edit(editor: &mut crate::history::Editor, info: &pdfcraft_render::DocInfo, edit: Edit) -> Option<Edit> {
    // Rewriting an OCR word (invisible text over a scan) also hides the scanned word: cover it
    // in the paper's colour and draw the new text in the ink's.
    Some(match edit {
        Edit::EditTextBlock { page, block, text, mut style } => {
            // Whether refusing a stand-in font was asked for doesn't make it a formatting change. (An OCR
            // word edit that would need a stand-in falls back to the whole phrase, which refuses.)
            let plain = pdfcraft_edit::BlockStyle { refuse_substitute: false, ..style.clone() } == pdfcraft_edit::BlockStyle::default();
            // Only invisible text is sampled; the blocks are usually cached from opening it.
            if style.cover.is_none()
                && let Some(info) = info.pages.get(page)
                && let Some(b) = editor.text_blocks(page).ok().and_then(|b| b.get(block).filter(|b| b.invisible).cloned())
                && let Some((paper, ink)) = guard(|| scan_colours::sample(&editor.cos, info, page, &b)).ok().flatten()
            {
                style.cover = Some(paper);
                style.color = style.color.or(Some(ink));
                // A text edit alone leaves the words it doesn't change as the scan shows them.
                style.changed_words_only = plain;
                // Nothing to redraw: leave undo history and the dirty flag alone.
                if plain && b.same_words(&text) {
                    return None;
                }
            }
            Edit::EditTextBlock { page, block, text, style }
        }
        other => other,
    })
}

/// Whether the edit, or any edit inside a batch, deletes pages.
fn deletes_pages(edit: &Edit) -> bool {
    match edit {
        Edit::DeletePages { .. } => true,
        Edit::Batch { edits, .. } => edits.iter().any(deletes_pages),
        _ => false,
    }
}

/// Keep edits to the working copy until both the operation and its script callbacks succeed.
fn run_checked_edit(next: &mut pdfcraft_cos::Document, edit: &Edit, cx: &mut EditCtx) -> Result<(), EditError> {
    let result = guard(|| run_edit(next, edit, cx))
        .unwrap_or_else(|m| Err(EditError::Invalid(format!("{} failed unexpectedly ({m}); the document was not changed", edit.label()))));
    let refusal = cx.js.as_mut().and_then(|runner| runner.refused.take());
    if refusal.is_some() || result.is_err() {
        discard_edit_callbacks(cx);
    }
    refusal.map_or(result, |reason| Err(EditError::Invalid(reason)))
}

/// Discard callback effects if PDF commit fails after any code entered the retained realm.
fn discard_edit_callbacks(cx: &EditCtx) {
    if let Some(runner) = cx.js.as_ref()
        && runner.callbacks_touched_realm
        && let Ok(mut runtime) = runner.runtime.lock()
    {
        runtime.reset_after_refusal();
    }
}

/// Compare the icon and all of its indirect resources. Conservatively retire on budget exhaustion.
fn icon_changed(previous: &pdfcraft_cos::Document, restored: &pdfcraft_cos::Document, root: pdfcraft_cos::ObjRef, budget: &mut usize) -> bool {
    use pdfcraft_cos::Object;

    fn visit(object: &Object, refs: &mut Vec<pdfcraft_cos::ObjRef>, budget: &mut usize, depth: usize) -> Option<()> {
        *budget = budget.checked_sub(1)?;
        if depth == 128 {
            return None;
        }
        match object {
            Object::Ref(reference) => refs.push(*reference),
            Object::Array(values) => {
                for value in values {
                    visit(value, refs, budget, depth + 1)?;
                }
            }
            _ => {
                if let Some(dict) = object.as_dict() {
                    for (_, value) in dict.iter() {
                        visit(value, refs, budget, depth + 1)?;
                    }
                }
            }
        }
        Some(())
    }

    let mut pending = vec![root];
    let mut seen = std::collections::HashSet::new();
    while let Some(reference) = pending.pop() {
        if !seen.insert(reference) {
            continue;
        }
        if *budget == 0 {
            return true;
        }
        let before = previous.get(reference);
        let after = restored.get(reference);
        if (!Arc::ptr_eq(&before, &after) && before != after) || visit(&before, &mut pending, budget, 0).is_none() {
            return true;
        }
    }
    false
}

fn sync_script_history(doc: &Document, previous: &pdfcraft_cos::Document, restored: &pdfcraft_cos::Document) {
    if let Ok(mut runtime) = doc.js_runtime.lock() {
        let mut budget = 250_000;
        runtime.retire_changed_icons(|id| {
            let object = pdfcraft_cos::ObjRef::new((id >> 16) as u32, id as u16);
            icon_changed(previous, restored, object, &mut budget)
        });
        runtime.sync_field_ids(doc.form.iter().map(|f| crate::js::field_object_id(f.obj)));
    }
}

/// Whether `edit` is, or (in a batch) holds, a signer label edit.
fn contains_field_signer(edit: &Edit) -> bool {
    match edit {
        Edit::SetFieldSigner { .. } => true,
        Edit::Batch { edits, .. } => edits.iter().any(contains_field_signer),
        _ => false,
    }
}

/// What the change being judged is, for the words of a refusal.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Act {
    /// A signer label written on a form field.
    Label,
    /// A new signature made in another field.
    Signing,
}

/// The form of `cos`, tabbed as the reader chose for pages that set no order.
fn form_of(cos: &pdfcraft_cos::Document, structure: bool) -> Vec<pdfcraft_forms::Field> {
    let mut form = pdfcraft_forms::fields(cos);
    if structure {
        pdfcraft_forms::rank_tabs_with(cos, &mut form, true);
    }
    form
}

impl Act {
    fn outcome(self) -> &'static str {
        match self {
            Act::Label => "the signer label was not written",
            Act::Signing => "the document was not signed",
        }
    }

    fn with(self) -> &'static str {
        match self {
            Act::Label => "this label",
            Act::Signing => "this signature",
        }
    }
}

/// Why `after` (the signatures as validated once an edit is in) is worse than `before`, when it is:
/// a signature whose status fell, that now has changes it forbids, or whose changes can no longer
/// be checked (each with its own sentence), or that can no longer be matched. A signature that
/// already had changes it forbids passes when nothing about its verdict gets worse: the edit does
/// not make it worse. `None` when no signature is worse off.
fn signature_damage(before: &[pdfcraft_sign::pdf::SignatureInfo], after: &[pdfcraft_sign::pdf::SignatureInfo]) -> Option<String> {
    signature_damage_by(before, after, Act::Label)
}

/// [`signature_damage`] for the change `act`, which only chooses the words.
fn signature_damage_by(before: &[pdfcraft_sign::pdf::SignatureInfo], after: &[pdfcraft_sign::pdf::SignatureInfo], act: Act) -> Option<String> {
    use pdfcraft_sign::pdf::{Modification, Status, kind};
    let rank = |s: Status| match s {
        Status::Valid => 2,
        Status::Unknown => 1,
        Status::Invalid => 0,
    };
    let word = |s: Status| match s {
        Status::Valid => "valid",
        Status::Unknown => "unknown",
        Status::Invalid => "invalid",
    };
    let (outcome, with) = (act.outcome(), act.with());
    let unmatched = || format!("the document's signatures could not be compared before and after, so {outcome}");
    for (i, old) in before.iter().enumerate().filter(|(_, s)| s.signed) {
        let Some(new) = after.get(i).filter(|n| n.signed && n.field == old.field) else { return Some(unmatched()) };
        // A signature that already had changes it forbids passes a label as long as its verdict is
        // no worse. For a new signature it must not gain a kind of forbidden change it did not have.
        let gained = match (&old.modification, &new.modification) {
            (Modification::Disallowed(_), Modification::Disallowed(_)) if act == Act::Label => false,
            (Modification::Disallowed(was), Modification::Disallowed(now)) => now.iter().any(|k| !was.contains(k)),
            (_, Modification::Disallowed(_)) => true,
            _ => false,
        };
        if let Modification::Disallowed(kinds) = &new.modification
            && gained
        {
            let list = kinds.join(", ");
            return Some(if new.certify == Some(1) {
                let what = if act == Act::Label { "any change to the form, a label on any field included," } else { "signing another field" };
                format!(
                    "the signature in field {:?} is a certification that allows no changes, so {what} is one it forbids ({list}); {outcome}",
                    old.field
                )
            } else if kinds.contains(&kind::LOCKED_FIELD) {
                let what = if act == Act::Label { "a label on it" } else { "signing it" };
                format!(
                    "this field is covered by the lock of the signature in field {:?}, so {what} is a change that signature forbids ({list}); {outcome}",
                    old.field
                )
            } else {
                format!("the signature in field {:?} forbids this change to the form ({list}), so {outcome}", old.field)
            });
        }
        if rank(new.status) < rank(old.status) {
            return Some(format!(
                "the signature in field {:?} would go from {} to {} with {with}, so {outcome}",
                old.field,
                word(old.status),
                word(new.status)
            ));
        }
        if old.changes_checked && !new.changes_checked {
            return Some(format!(
                "the changes made after the signature in field {:?} could no longer be checked with {with}, so {outcome}",
                old.field
            ));
        }
    }
    None
}

impl Session {
    /// Whether `signed`, the document `doc` signed in the field named `field` (what
    /// [`Session::sign`] returned, not yet saved or adopted), is a file whose signatures are as
    /// good as the open document's: the validator reads `signed` and every signature the open
    /// document holds must still be there and not worse (status, changes it forbids, changes
    /// that can still be checked), and `field` must hold a signature that is not invalid. The
    /// open document is not touched. The error says why not.
    pub fn check_signing(&self, doc: DocId, signed: &Arc<Vec<u8>>, field: &str) -> Result<(), String> {
        let d = self.get(doc).ok_or("there is no such document")?;
        let editor = d.editor.as_ref().ok_or("the document cannot be read")?;
        let cos = pdfcraft_cos::Document::open_with_password(signed.clone(), editor.keys.reopen.as_deref())
            .map_err(|e| format!("the signed file could not be read back, so the document was not signed ({e})"))?;
        let after = signatures_of(&cos, signed, &d.trust, &d.sig_cache);
        if let Some(why) = signature_damage_by(&d.signatures, &after, Act::Signing) {
            return Err(why);
        }
        match after.iter().find(|s| s.signed && s.field == field) {
            None => Err(format!("the signature in field {field:?} is not in the signed file, so the document was not signed")),
            Some(s) if s.status == pdfcraft_sign::pdf::Status::Invalid => {
                Err(format!("the new signature in field {field:?} reads as invalid, so the document was not signed"))
            }
            Some(_) => Ok(()),
        }
    }

    /// Undo what [`Session::apply`] just pushed (its edit could not be kept), and read the
    /// document again.
    fn roll_back_last(doc: &mut Document, dropped: Option<crate::history::Snapshot>) {
        if let Some(ed) = doc.editor.as_mut()
            && let Some((_, prev, keys, _)) = ed.undo.pop()
        {
            ed.cos = prev;
            ed.keys = keys;
            // The oldest step the push made room by dropping comes back.
            if let Some(oldest) = dropped {
                ed.undo.insert(0, oldest);
            }
        }
        Self::adopt_keys(doc);
        if !matches!(guard(|| Self::refresh(doc)), Ok(Ok(()))) {
            Self::show_released(doc);
        }
    }

    /// Whether [`Edit::DeletePages`] of `pages` (0-based) would be accepted by document `id`, for
    /// the refusals checked here: a read-only document, permissions that refuse page changes, a
    /// page that does not exist, and removing every page; each gives the error [`Session::apply`]
    /// gives. The edit is tried on a copy of the document: nothing changes, and no undo step or
    /// unsaved change is made. A refusal it cannot see is not reported: a signed file with no
    /// revisions to append to (`SignedRewrite`), a write error while an XFA form's data is synced
    /// or laid out again, and a failure of the refresh after the edit (which rolls the edit back).
    pub fn check_delete_pages(&self, id: DocId, pages: &[usize]) -> Result<(), EditError> {
        let doc = self.get(id).ok_or(EditError::NoDocument)?;
        let editor = doc.editor.as_ref().ok_or_else(|| EditError::ReadOnly(doc.read_only_reason.clone().unwrap_or_default()))?;
        let edit = Edit::DeletePages { pages: pages.to_vec() };
        if let Some(p) = editor.cos.permissions() {
            check_permission(&edit, &p)?;
        }
        let mut next = editor.cos.clone();
        let mut cx = EditCtx::new(self.now(), doc.generation ^ (id.0 << 48));
        cx.pages = doc.info.pages.len();
        run_checked_edit(&mut next, &edit, &mut cx)
    }

    /// Apply an edit. On success the previous state is undoable and the view data is refreshed.
    pub fn apply(&mut self, id: DocId, edit: Edit) -> Result<(), EditError> {
        self.apply_reporting(id, edit).map(|_| ())
    }

    /// Edit a PDF ▸ Edit text: make the grouped text on `page` (0-based) editable, as one undoable
    /// [`Edit::PrepareText`], and say what it prepared, which fonts' maps it repaired and which
    /// groups it left as they were (and why). The counts are this edit's own, not a preview's.
    pub fn prepare_text(&mut self, id: DocId, page: usize) -> Result<pdfcraft_edit::text::forms::Preparation, EditError> {
        self.apply_reporting(id, Edit::PrepareText { page })?
            .ok_or_else(|| EditError::Invalid("preparing grouped text gave no result; the document was not changed".into()))
    }

    /// [`Session::apply`], giving back what an [`Edit::PrepareText`] did (`None` for other edits).
    fn apply_reporting(&mut self, id: DocId, edit: Edit) -> Result<Option<pdfcraft_edit::text::forms::Preparation>, EditError> {
        let now = self.now();
        let today = self.today();
        let js_off = self.js_off;
        let highlight = self.field_highlight;
        let xfa_time_scale = self.xfa_time_scale.unwrap_or(1);
        let doc = self.doc_mut(id)?;
        // An edit must never read globals from a completed-but-unapplied button invocation.
        // Fork its realm before any edit-triggered calculation, even if the edit later fails.
        #[cfg(not(target_arch = "wasm32"))]
        if doc.js_runtime.lock().map_err(|_| EditError::Invalid("script runtime unavailable".into()))?.is_pending() {
            js::replace_runtime(&mut doc.js_runtime);
        }
        doc.edit_notices.clear();
        let name = doc.name.clone();
        let is_xfa = doc.info.xfa.is_some();
        let mut scope = scope_of(&edit);
        let mut cx = EditCtx::new(now, doc.generation ^ (id.0 << 48));
        if !js_off {
            cx.xfa = doc.xfa_template.clone();
        }
        cx.xfa_time_scale = xfa_time_scale;
        cx.pages = doc.info.pages.len();
        cx.highlight = highlight;
        cx.today = today;
        let reason = doc.read_only_reason.clone().unwrap_or_default();
        let signed = doc.is_signed();
        let signatures_before = doc.signatures.clone();
        let editor = doc.editor.as_mut().ok_or(EditError::ReadOnly(reason))?;
        if let Some(p) = editor.cos.permissions() {
            check_permission(&edit, &p)?;
        }
        let Some(edit) = prepare_ocr_text_edit(editor, &doc.info, edit) else {
            return Ok(None);
        };
        // Nothing to bake: leave undo history and the dirty flag alone.
        if matches!(&edit, Edit::FlattenFillSign) && !pdfcraft_annot::has_visible_fill_sign(&editor.cos) {
            return Ok(None);
        }
        // A replace that finds nothing to change leaves no undo step and nothing unsaved. A
        // refusal is left for the edit itself to give.
        if let Edit::ReplaceInJavaScripts { find, replace, match_case } = &edit
            && pdfcraft_forms::preview_javascript_replace(&editor.cos, find, replace, *match_case).is_ok_and(|r| r.changed.is_empty())
        {
            return Ok(None);
        }
        let mut next = editor.cos.clone();
        if !js_off && uses_scripts(&edit) {
            let mut runner = js::JsRunner::new(&next, &name, highlight);
            runner.runtime = doc.js_runtime.clone();
            runner.doc.zoom = doc.js_zoom;
            runner.doc.page = doc.js_page;
            runner.doc.layers = crate::live_scripts::js_layers(&doc.info);
            cx.js = Some(runner);
        }
        cx.renamed_fields.clear();
        cx.redaction_overlays.clear();
        cx.redaction_marks.clear();
        // `next` is a copy: if the edit fails or crashes, the document is unchanged.
        run_checked_edit(&mut next, &edit, &mut cx)?;
        // The values of fields a signature locks stay as signed, however the edit would change them (typing, reset,
        // import, scripts), and the fields are not removed. A signature already invalid no longer holds anything back.
        if signed {
            let broken = pdfcraft_sign::pdf::signatures_locking_changed_values(&editor.cos, &next);
            if let Some(field) =
                broken.iter().find(|name| signatures_before.iter().any(|s| &s.field == *name && s.status != pdfcraft_sign::pdf::Status::Invalid))
            {
                discard_edit_callbacks(&cx);
                return Err(EditError::Locked(field.clone()));
            }
        }
        // A field's enter or exit scripts that changed nothing leave no edit behind (no undo step,
        // nothing unsaved); what they showed or asked for still goes out. Only an event on its own:
        // a batch keeps the other edits it holds.
        if cx.xfa_unchanged && matches!(edit, Edit::XfaEvent { .. }) {
            take_xfa_output(doc, cx.xfa_out, cx.xfa_ran_away);
            return Ok(None);
        }
        // Redaction removed what its own interpreter places under the marks. The marks were drawn
        // over the page as the renderer shows it, so the renderer has the last word: on every
        // apply the edit ran, a batch's included.
        if !cx.redaction_marks.is_empty() {
            // A batch that deletes a marked page and adds pages cannot be tied back to the marks:
            // a copy of a page may carry the text the redaction removed from the original only.
            // Fail closed.
            let before: std::collections::HashSet<_> = pdfcraft_model::pages(&editor.cos).iter().map(|p| p.obj).collect();
            let after: std::collections::HashSet<_> = pdfcraft_model::pages(&next).iter().map(|p| p.obj).collect();
            // Either refusal also discards what the batch's field scripts left in the realm.
            if cx.redaction_marks.iter().any(|(o, _)| !after.contains(o)) && after.iter().any(|o| !before.contains(o)) {
                discard_edit_callbacks(&cx);
                return Err(EditError::Invalid(
                    "a batch that deletes a page with a redaction mark and adds pages can't be verified; the document was not changed".into(),
                ));
            }
            guard(|| shown_under_redactions(&next, &cx.redaction_marks, &cx.redaction_overlays, doc.password.as_deref()))
                .unwrap_or_else(|m| Err(EditError::Invalid(format!("{} failed unexpectedly ({m}); the document was not changed", edit.label()))))
                .inspect_err(|_| discard_edit_callbacks(&cx))?;
        }
        // Renamed fields are said, not slipped in: a repair-log note (with the caveat that
        // scripts were left alone) and a count in the undo step's label.
        let renamed_note = |renamed: &[(String, String)]| {
            (!renamed.is_empty()).then(|| {
                let shown: Vec<String> = renamed.iter().take(5).map(|(old, new)| format!("{old} → {new}")).collect();
                let more = renamed.len().saturating_sub(shown.len());
                let more = if more > 0 { format!(", and {more} more") } else { String::new() };
                format!(
                    "inserted pages renamed {} form field{} to keep their names separate ({}{more}); scripts that refer to the old names were not rewritten",
                    renamed.len(),
                    if renamed.len() == 1 { "" } else { "s" },
                    shown.join(", ")
                )
            })
        };
        if let Some(note) = renamed_note(&cx.renamed_fields) {
            next.note_repair(note);
        }
        // Only Insert from File renames fields; its label says so in a wording the interface
        // translates (the count lives in the repair note).
        let label = match (&edit, cx.renamed_fields.len()) {
            (_, 0) => edit.label(),
            (Edit::InsertPagesFrom { name, .. }, n) => Label::with(
                if n == 1 { "Insert pages from {name} — 1 field renamed" } else { "Insert pages from {name} — fields renamed" },
                name.as_str(),
            ),
            _ => edit.label(),
        };
        // An XFA form's scripts answer the change: exit and validate scripts of the field, then
        // every calculation (and a reset recalculates). They read the data, so it is brought up
        // to date first. The fields whose exit scripts ran are noted once the edit is kept.
        let mut exited: Vec<String> = Vec::new();
        if let Some(tpl) = cx.xfa.clone() {
            if is_xfa && matches!(scope_of(&edit), Scope::Form | Scope::Full | Scope::Content) {
                let datasets = &mut cx.xfa_datasets;
                let notes = guard(|| xfa_sync_datasets(&mut next, Some(&tpl), datasets))
                    .unwrap_or_else(|m| Err(format!("writing the XFA data failed unexpectedly ({m})")))
                    .map_err(EditError::Write)?;
                cx.xfa_out.errors.extend(notes);
            }
            /// The fields edit `e` (and the edits it batches) gives new values, and whether it
            /// resets the form.
            fn collect(e: &Edit, changed: &mut Vec<String>, reset: &mut bool) {
                match e {
                    Edit::SetFieldValue { name, .. } => changed.push(name.clone()),
                    Edit::ResetForm { .. } => *reset = true,
                    Edit::Batch { edits, .. } => edits.iter().for_each(|e| collect(e, changed, reset)),
                    _ => {}
                }
            }
            let mut changed: Vec<String> = Vec::new();
            let mut reset = false;
            collect(&edit, &mut changed, &mut reset);
            if !changed.is_empty() || reset {
                let highlight = cx.highlight();
                let mut run = xfa::XfaRun {
                    page_count: cx.pages,
                    out: &mut cx.xfa_out,
                    datasets: cx.xfa_datasets,
                    ran_away: false,
                    highlight,
                    time_scale: cx.xfa_time_scale,
                };
                // Every changed field's scripts, then the calculations once (after a reset,
                // only those).
                let ran = guard(|| xfa::on_changes(&mut next, &tpl, &changed, &mut run, &mut exited))
                    .unwrap_or_else(|m| Err(format!("the form's scripts failed unexpectedly ({m})")));
                cx.xfa_datasets = run.datasets;
                cx.xfa_ran_away |= run.ran_away;
                // The next edit of a batch reads the highlight this one's scripts set.
                let on = run.highlight;
                cx.set_highlight(on);
                if let Err(e) = ran {
                    cx.xfa_out.errors.push(e);
                }
            }
        }
        // XFA forms keep their values in the datasets packet too, for Adobe's viewers.
        let mut xfa_notes = Vec::new();
        if is_xfa && matches!(scope_of(&edit), Scope::Form | Scope::Full | Scope::Content) {
            let (tpl, datasets) = (cx.xfa.as_deref(), &mut cx.xfa_datasets);
            xfa_notes = guard(|| xfa_sync_datasets(&mut next, tpl, datasets))
                .unwrap_or_else(|m| Err(format!("writing the XFA data failed unexpectedly ({m})")))
                .map_err(EditError::Write)
                .inspect_err(|_| discard_edit_callbacks(&cx))?;
            // Rebuilding from stale data would discard accepted widget values.
            if xfa_notes.is_empty() {
                let reflow = guard(|| xfa::relayout_changed_text(&mut next, tpl, &doc.form))
                    .unwrap_or_else(|m| Err(format!("laying out the XFA text failed unexpectedly ({m})")))
                    .map_err(EditError::Write)
                    .inspect_err(|_| discard_edit_callbacks(&cx))?;
                if let Some(warnings) = reflow {
                    xfa_notes.extend(warnings);
                    // Layout replaces pages and widgets, even if the page count is unchanged.
                    // Undo/redo must rebuild these too instead of refreshing only field values.
                    scope = Scope::Full;
                }
            }
        }
        // Signed documents are only ever saved incrementally: an edit that needs a full rewrite
        // (applying redactions, changing security, sanitizing) would invalidate the signatures.
        // So does any edit to a file rebuilt from object headers: it has no chain to append to.
        // Deleted pages remain in earlier incremental revisions (#3407), so rewrite
        // unsigned files in full. Signed files must retain their signed bytes.
        if !signed && deletes_pages(&edit) {
            next.require_full_save();
        }
        let rewrites = |c: &pdfcraft_cos::Document| c.full_save_required() || c.encryption_changed();
        if signed && ((rewrites(&next) && !rewrites(&editor.cos)) || (next.is_modified() && next.revisions().is_empty())) {
            discard_edit_callbacks(&cx);
            return Err(EditError::SignedRewrite(edit.label().to_string()));
        }
        let mut notices = edit_notices(&edit, &editor.cos, &next);
        // Places applied redactions left that still hold the removed text, and what became of
        // the PDF/A or PDF/UA identification in the XMP they removed: said, not silent.
        notices.extend(cx.redaction_elsewhere.filter(|e| e.total() > 0 || e.unsearched).map(EditNotice::RedactionElsewhere));
        if cx.redaction_identification != pdfcraft_redact::Identification::default() {
            notices.push(EditNotice::RedactionIdentification(cx.redaction_identification));
        }
        notices.extend(substitute_notice(&cx));
        let previous = std::mem::replace(&mut editor.cos, next);
        let keys = keys_after(&edit).unwrap_or_else(|| editor.keys.clone());
        let previous_keys = std::mem::replace(&mut editor.keys, keys);
        editor.undo.push((label, previous, previous_keys, scope));
        // Kept until the edit is accepted, so a refusal puts the oldest step back.
        let dropped = if editor.undo.len() > MAX_UNDO { Some(editor.undo.remove(0)) } else { None };
        // A refused signer label leaves the redo steps as they were.
        let redo_kept = (signed && contains_field_signer(&edit)).then(|| editor.redo.clone());
        editor.redo.clear();
        Self::adopt_keys(doc);
        let refreshed = guard(|| if scope == Scope::Content { Self::refresh_content(doc, Some(&edit)) } else { Self::refresh_scoped(doc, scope) })
            .unwrap_or_else(|m| Err(EditError::Reopen(m)));
        if let Err(e) = refreshed {
            discard_edit_callbacks(&cx);
            // Roll back: the edit produced something we cannot display.
            Self::roll_back_last(doc, dropped);
            return Err(e);
        }
        // A field's signer label is a change to the form that no signature covers. Whether a
        // signature already in the file tolerates it (a certification that allows no changes, a
        // lock that names the field) is the validator's verdict on the new bytes, not a copy of
        // its rules: a write that makes any signature worse is rolled back and refused.
        if signed
            && contains_field_signer(&edit)
            && let Some(why) = signature_damage(&signatures_before, &doc.signatures)
        {
            discard_edit_callbacks(&cx);
            Self::roll_back_last(doc, dropped);
            if let (Some(ed), Some(redo)) = (doc.editor.as_mut(), redo_kept) {
                ed.redo = redo;
            }
            return Err(EditError::Invalid(why));
        }
        doc.dirty = true;
        if let Ok(mut runtime) = doc.js_runtime.lock() {
            runtime.sync_field_ids(doc.form.iter().map(|f| crate::js::field_object_id(f.obj)));
        }
        doc.generation += 1;
        doc.edit_notices = notices;
        // Only this edit's exits are skipped on the next focus loss: one an earlier change ran is
        // no reason to skip a later one.
        doc.xfa_exited.clear();
        doc.xfa_exited.extend(exited);
        if !cx.xfa_removed.is_empty() {
            let removed: Vec<String> = cx.xfa_removed.iter().map(|n| format!("{XFA_REMOVED_PREFIX}{n}")).collect();
            doc.xfa_warnings.retain(|w| w.starts_with(XFA_REMOVED_PREFIX));
            note_warnings(&mut doc.xfa_warnings, &removed);
            note_warnings(&mut doc.info.warnings, &removed);
        }
        note_warnings(&mut doc.xfa_warnings, &xfa_notes);
        let notes = doc.xfa_warnings.clone();
        note_warnings(&mut doc.info.warnings, &notes);
        if let Some(js) = cx.js {
            doc.js_output.append(js.output);
        }
        take_xfa_output(doc, cx.xfa_out, cx.xfa_ran_away);
        Ok(cx.prepared)
    }

    /// Undo the last edit of document `id`, keeping it for [`Session::redo`]. Returns its label.
    pub fn undo(&mut self, id: DocId) -> Result<Label, EditError> {
        let doc = self.doc_mut(id)?;
        let editor = doc.editor.as_mut().ok_or(EditError::NothingToUndo)?;
        let (label, prev, keys, scope) = editor.undo.pop().ok_or(EditError::NothingToUndo)?;
        let current = std::mem::replace(&mut editor.cos, prev);
        let current_keys = std::mem::replace(&mut editor.keys, keys);
        editor.redo.push((label.clone(), current, current_keys, scope));
        Self::adopt_keys(doc);
        let refreshed = guard(|| Self::refresh_restored(doc, scope)).unwrap_or_else(|m| Err(EditError::Reopen(m)));
        if let Err(e) = refreshed {
            if let Some(ed) = doc.editor.as_mut()
                && let Some((label, current, current_keys, scope)) = ed.redo.pop()
            {
                let prev = std::mem::replace(&mut ed.cos, current);
                let keys = std::mem::replace(&mut ed.keys, current_keys);
                ed.undo.push((label, prev, keys, scope));
            }
            Self::adopt_keys(doc);
            if !matches!(guard(|| Self::refresh(doc)), Ok(Ok(()))) {
                Self::show_released(doc);
            }
            return Err(e);
        }
        doc.edit_notices.clear();
        Self::sync_dirty(doc);
        if let Some(editor) = doc.editor.as_ref()
            && let Some((_, previous, _, _)) = editor.redo.last()
        {
            sync_script_history(doc, previous, &editor.cos);
        }
        // The change whose exit ran is undone (or redone without its scripts): skip no exit.
        doc.xfa_exited.clear();
        doc.generation += 1;
        Ok(label)
    }

    /// Take back the last edit of document `id` as [`Session::undo`] does, and forget it: it is not
    /// kept for [`Session::redo`]. For an edit made only to be saved, when the save then failed.
    pub fn discard_last_edit(&mut self, id: DocId) -> Result<Label, EditError> {
        let label = self.undo(id)?;
        if let Some(editor) = self.doc_mut(id)?.editor.as_mut() {
            editor.redo.pop();
        }
        Ok(label)
    }

    /// Redo the last edit of document `id` that [`Session::undo`] took back. Returns its label.
    pub fn redo(&mut self, id: DocId) -> Result<Label, EditError> {
        let doc = self.doc_mut(id)?;
        let editor = doc.editor.as_mut().ok_or(EditError::NothingToRedo)?;
        let (label, next, keys, scope) = editor.redo.pop().ok_or(EditError::NothingToRedo)?;
        let current = std::mem::replace(&mut editor.cos, next);
        let current_keys = std::mem::replace(&mut editor.keys, keys);
        editor.undo.push((label.clone(), current, current_keys, scope));
        Self::adopt_keys(doc);
        let refreshed = guard(|| Self::refresh_restored(doc, scope)).unwrap_or_else(|m| Err(EditError::Reopen(m)));
        if let Err(e) = refreshed {
            if let Some(ed) = doc.editor.as_mut()
                && let Some((label, current, current_keys, scope)) = ed.undo.pop()
            {
                let next = std::mem::replace(&mut ed.cos, current);
                let keys = std::mem::replace(&mut ed.keys, current_keys);
                ed.redo.push((label, next, keys, scope));
            }
            Self::adopt_keys(doc);
            if !matches!(guard(|| Self::refresh(doc)), Ok(Ok(()))) {
                Self::show_released(doc);
            }
            return Err(e);
        }
        doc.edit_notices.clear();
        Self::sync_dirty(doc);
        if let Some(editor) = doc.editor.as_ref()
            && let Some((_, previous, _, _)) = editor.undo.last()
        {
            sync_script_history(doc, previous, &editor.cos);
        }
        // The change whose exit ran is undone (or redone without its scripts): skip no exit.
        doc.xfa_exited.clear();
        doc.generation += 1;
        Ok(label)
    }

    /// Preferences ▸ Accessibility: whether pages that set no tab order are tabbed in the order of
    /// their document's structure tree, in the open documents and those opened later. Nothing in
    /// a document changes.
    pub fn set_tabs_follow_structure(&mut self, on: bool) {
        if self.tabs_follow_structure == on {
            return;
        }
        self.tabs_follow_structure = on;
        for doc in &mut self.docs {
            doc.tabs_follow_structure = on;
            Self::rank_form(doc);
        }
    }

    pub fn tabs_follow_structure(&self) -> bool {
        self.tabs_follow_structure
    }

    /// Rank `doc`'s form again for its tab setting. An XFA form's order is its template's.
    pub(crate) fn rank_form(doc: &mut Document) {
        let Some(editor) = doc.editor.as_ref().filter(|_| doc.info.xfa.is_none()) else { return };
        let mut form = (*doc.form).clone();
        pdfcraft_forms::rank_tabs_with(&editor.cos, &mut form, doc.tabs_follow_structure);
        doc.form = Arc::new(form);
    }

    /// Unsaved when the working graph is not the last opened or saved file. Undo back to that
    /// file is clean; undo past a later save is not, because the restored snapshot's own base
    /// is an earlier revision.
    pub(super) fn sync_dirty(doc: &mut Document) {
        doc.dirty = doc.editor.as_ref().is_some_and(|e| e.unsaved_file || e.cos.is_modified() || !Arc::ptr_eq(e.cos.bytes(), &e.saved));
    }

    /// Use the current state's password for viewing (protection edits change it).
    pub(super) fn adopt_keys(doc: &mut Document) {
        if let Some(e) = doc.editor.as_ref() {
            doc.password = e.keys.render.clone();
            doc.config.password = e.keys.render.as_deref().map(Arc::from);
        }
    }

    /// [`Self::refresh_scoped`] for a graph undo or redo just brought back. The history survives a
    /// full save, which renumbers the objects, so the graph may name the layers by other numbers
    /// than the open document's: the user's layer choices then follow their layers by name, as
    /// after the save itself, and the layer list is read again whatever `scope` was.
    fn refresh_restored(doc: &mut Document, scope: Scope) -> Result<(), EditError> {
        let renumbered =
            !doc.info.layers.is_empty() && doc.editor.as_ref().is_some_and(|e| layer_ids(&e.cos).iter().ne(doc.info.layers.iter().map(|l| &l.id)));
        if renumbered { Self::refresh_renumbered(doc) } else { Self::refresh_scoped(doc, scope) }
    }

    /// Rebuild the working bytes and the view data that `scope` may have changed. Metadata edits
    /// keep the renderer; other scopes also replace it.
    /// Comment and form edits skip the full re-inspection (seconds on very large files): the
    /// comment list and field values are re-read from the object graph instead.
    fn refresh_scoped(doc: &mut Document, scope: Scope) -> Result<(), EditError> {
        if scope == Scope::Full || doc.info.encrypted != doc.editor.as_ref().is_some_and(|e| e.cos.output_handler().is_some()) {
            return Self::refresh(doc);
        }
        if scope == Scope::Content {
            return Self::refresh_content(doc, None);
        }
        if scope == Scope::Links {
            return Self::refresh_links(doc);
        }
        // Metadata edits keep the renderer, which holds the working file anyway.
        if scope != Scope::Metadata {
            Self::release_working(doc);
        }
        let Some(editor) = doc.editor.as_mut() else { return Ok(()) };
        // Comment edits, and form edits (a script may change annotation properties), may change
        // annotation dictionaries, invalidating cached appearances.
        if matches!(scope, Scope::Comments | Scope::Form) {
            editor.appearances.clear();
        }
        let bytes = if editor.cos.is_modified() {
            Arc::new(write_incremental(&editor.cos, &SaveOptions::default()).map_err(|e| EditError::Write(e.to_string()))?)
        } else {
            editor.cos.bytes().clone()
        };
        if scope == Scope::Metadata {
            for (field, value) in info_fields(&mut doc.info, &editor.cos) {
                *field = value;
            }
            doc.info.custom = pdfcraft_organize::custom_info(&editor.cos);
            doc.info.file_size = bytes.len();
            #[cfg(test)]
            check_refresh_panic();
            if !doc.signatures.is_empty() {
                let mut signatures = signatures_of(&editor.cos, &bytes, &doc.trust, &doc.sig_cache);
                // The display copy is kept for a metadata edit, and so are its marks, unless the
                // edit changed a verdict (a certification can forbid it): the page then draws the
                // new one.
                if !carry_verdict_marks(&doc.signatures, &mut signatures) {
                    doc.display = display_bytes(Some(editor), &bytes, &mut signatures, &doc.captions);
                    doc.renderer = RenderPool::new(doc.display.clone(), render_threads(), display_config(&doc.config));
                }
                doc.signatures = Arc::new(signatures);
            }
            doc.bytes = bytes;
            return Ok(());
        }
        let mut form = form_of(&editor.cos, doc.tabs_follow_structure && doc.info.xfa.is_none());
        xfa::mark_script_buttons(&editor.cos, &mut form);
        let form = Arc::new(form);
        match scope {
            Scope::Metadata => return Ok(()),
            Scope::Comments => {
                doc.info.annotations = comment_list(&editor.cos);
                // Link indexes are positions in /Annots, so comment insertion/deletion can move them.
                doc.links = OnceLock::new();
            }
            Scope::Form => {
                // PERF: keystroke path, index once. A linear find per field is O(F^2);
                // forms reach 10k fields.
                let mut by_name = std::collections::HashMap::with_capacity(form.len());
                for x in form.iter() {
                    // First match wins (do not simplify to collect: it keeps the last).
                    by_name.entry(x.name.as_str()).or_insert(x);
                }
                for f in &mut doc.info.fields {
                    if let Some(ff) = by_name.get(f.name.as_str()) {
                        f.value = match ff.kind {
                            pdfcraft_forms::FieldKind::CheckBox | pdfcraft_forms::FieldKind::Radio => {
                                Some(ff.value.first().cloned().unwrap_or_else(|| "Off".into()))
                            }
                            _ if ff.value.is_empty() => None,
                            _ => Some(ff.value.join(", ")),
                        };
                    }
                }
            }
            Scope::Links => return Self::refresh_links(doc),
            // Handled above; kept as a full refresh so the match stays total.
            Scope::Full => return Self::refresh(doc),
            // Handled above (content-only refresh); the arm keeps the match total.
            Scope::Content => return Self::refresh_content(doc, None),
        }
        doc.info.file_size = bytes.len();
        doc.form = form;
        let mut signatures = if doc.signatures.is_empty() { Vec::new() } else { signatures_of(&editor.cos, &bytes, &doc.trust, &doc.sig_cache) };
        doc.display = display_bytes(Some(editor), &bytes, &mut signatures, &doc.captions);
        doc.signatures = Arc::new(signatures);
        doc.renderer = RenderPool::new(doc.display.clone(), render_threads(), display_config(&doc.config));
        doc.bytes = bytes;
        Ok(())
    }

    /// Which derived lists a content-only edit can change: added content (`None`, from
    /// undo/redo, re-reads both: always correct, rarely paid).
    pub(crate) fn content_refresh_lists(edit: Option<&Edit>) -> (bool, bool) {
        if let Some(Edit::Batch { edits, .. }) = edit {
            // A batch of content edits (from a script, say) changes what its parts change.
            return edits.iter().fold((false, false), |(added, marks), part| {
                let (a, m) = Self::content_refresh_lists(Some(part));
                (added || a, marks || m)
            });
        }
        let touches_added = edit.is_none_or(|e| {
            matches!(
                e,
                Edit::AddText { .. }
                    | Edit::AddImage { .. }
                    | Edit::UpdateContent { .. }
                    | Edit::DeleteContent { .. }
                    | Edit::ReplaceImage { .. }
                    | Edit::PasteContent { .. }
                    | Edit::EditPageImage { .. }
            ) || matches!(e, Edit::MoveObjects { objects, .. } | Edit::DeleteObjects { objects, .. } if objects.iter().any(|o| o.kind == ObjectKind::Added))
                || matches!(e, Edit::ArrangeObjects { moves, .. } if moves.iter().any(|(o, _)| o.kind == ObjectKind::Added))
        });
        let touches_marks = edit.is_none_or(|e| {
            matches!(
                e,
                Edit::AddHeaderFooter { .. }
                    | Edit::AddWatermark { .. }
                    | Edit::AddBackground { .. }
                    | Edit::AddPrinterMarks { .. }
                    | Edit::RemoveMarks { .. }
            )
        });
        (touches_added, touches_marks)
    }

    /// Rebuild working bytes, display bytes and renderer after a content-only edit: page
    /// content streams, resources and added content/marks. The page tree, annotations, form
    /// fields, links, outline and fonts are unchanged, so the full re-inspection (seconds on
    /// very large files) is skipped. `edit` selects which derived lists are re-read (`None`,
    /// from undo/redo, re-reads both: always correct, rarely paid). The cached display
    /// appearances are kept: content edits leave every annotation dictionary alone.
    fn refresh_content(doc: &mut Document, edit: Option<&Edit>) -> Result<(), EditError> {
        Self::release_working(doc);
        sync_xfa_state(doc);
        let Some(editor) = doc.editor.as_mut() else { return Ok(()) };
        let bytes = if editor.cos.is_modified() {
            Arc::new(write_incremental(&editor.cos, &SaveOptions::default()).map_err(|e| EditError::Write(e.to_string()))?)
        } else {
            editor.cos.bytes().clone()
        };
        // The edit must still leave a file the viewer can open; anything else rolls back above.
        pdfcraft_render::check_pages(bytes.clone(), doc.password.as_deref(), doc.info.pages.len()).map_err(|e| EditError::Reopen(e.to_string()))?;
        let (touches_added, touches_marks) = Self::content_refresh_lists(edit);
        if touches_added || touches_marks {
            doc.marker_scan = OnceLock::new();
        }
        doc.info.file_size = bytes.len();
        let mut signatures = if doc.signatures.is_empty() { Vec::new() } else { signatures_of(&editor.cos, &bytes, &doc.trust, &doc.sig_cache) };
        doc.display = display_bytes(Some(editor), &bytes, &mut signatures, &doc.captions);
        doc.signatures = Arc::new(signatures);
        doc.renderer = RenderPool::new(doc.display.clone(), render_threads(), display_config(&doc.config));
        doc.bytes = bytes;
        doc.fonts_stale = true;
        Ok(())
    }

    /// Let go of the working file before the next one is written (gap 13). The working file is
    /// the original plus one appended revision, so on a 1 GiB file keeping the old one (in the
    /// working bytes, the display bytes and the renderer) while writing the new one held three
    /// copies at once; this keeps two. Until the refresh finishes, the working and display bytes
    /// are the original's (already held by the editor, so free) and the renderer draws nothing.
    /// A failed edit rolls back and refreshes again, which replaces all three; where a refresh
    /// still fails, [`Self::show_released`] draws the original rather than nothing. Save writes
    /// from the editor, never from these.
    pub(crate) fn release_working(doc: &mut Document) {
        let Some(original) = doc.editor.as_ref().map(|e| e.cos.bytes().clone()) else { return };
        // Dropping the pool stops its workers; each lets go of the file when it exits.
        doc.renderer = RenderPool::new_inline(Arc::new(Vec::new()), doc.config.clone());
        doc.display = original.clone();
        doc.bytes = original;
        // The page shows the original's own artwork again, so nothing counts as drawn, even if
        // the refresh that follows fails.
        if doc.signatures.iter().any(|s| s.verdict_shown) {
            for sig in Arc::make_mut(&mut doc.signatures) {
                sig.verdict_shown = false;
            }
        }
        #[cfg(test)]
        check_refresh_panic();
    }

    /// After a refresh failed: draw the display bytes it left (the original, once released),
    /// so the page still shows a document.
    pub(super) fn show_released(doc: &mut Document) {
        doc.renderer = RenderPool::new(doc.display.clone(), render_threads(), display_config(&doc.config));
    }

    /// Rebuild working bytes, inspection and renderer from the current edit state.
    pub(crate) fn refresh(doc: &mut Document) -> Result<(), EditError> {
        Self::refresh_with_indexes(doc, true, true, false)
    }

    /// [`Self::refresh`] after a full rewrite renumbered the objects: the user's layer choices
    /// follow their layers to the new numbers instead of being looked up by the old ones.
    pub(super) fn refresh_renumbered(doc: &mut Document) -> Result<(), EditError> {
        Self::refresh_with_indexes(doc, true, true, true)
    }

    /// Reinspect a link edit while preserving the page-content marker scan cache.
    fn refresh_links(doc: &mut Document) -> Result<(), EditError> {
        Self::refresh_with_indexes(doc, false, true, false)
    }

    fn refresh_with_indexes(doc: &mut Document, invalidate_markers: bool, invalidate_links: bool, renumbered: bool) -> Result<(), EditError> {
        #[cfg(test)]
        crate::record_full_refresh();
        // A full refresh may follow structural or annotation changes: cached appearances
        // may be stale, so they are rebuilt below.
        if let Some(editor) = doc.editor.as_mut() {
            editor.appearances.clear();
        }
        Self::release_working(doc);
        sync_xfa_state(doc);
        let Some(editor) = doc.editor.as_mut() else { return Ok(()) };
        let bytes = if editor.cos.is_modified() {
            Arc::new(write_incremental(&editor.cos, &SaveOptions::default()).map_err(|e| EditError::Write(e.to_string()))?)
        } else {
            editor.cos.bytes().clone()
        };
        let info = inspect(bytes.clone(), doc.password.as_deref()).map_err(|e| EditError::Reopen(e.to_string()))?;
        // Keep the user's layer choices where the layers still exist.
        let mut info = info;
        if renumbered {
            carry_renumbered_layer_choices(&doc.info.layers, &mut info.layers);
            // The renderer's overrides name the layers by their numbers too.
            if !doc.config.layers.is_empty() || info.layers.iter().any(|l| l.visible != l.initial) {
                doc.config.layers = Arc::new(info.layers.iter().map(|l| (l.id.0 as i32, l.id.1 as i32, l.visible)).collect());
            }
        } else {
            for l in &mut info.layers {
                if let Some(old) = doc.info.layers.iter().find(|o| o.id == l.id) {
                    l.visible = old.visible;
                }
            }
        }
        note_warnings(&mut info.warnings, &doc.xfa_warnings);
        // As in `build_document`: the inspection reads only `/Info`, so fields stated only in XMP
        // are filled in again after every full refresh.
        for (field, value) in info_fields(&mut info, &editor.cos) {
            if field.is_none() {
                *field = value;
            }
        }
        doc.info = info;
        doc.config.print_layers = print_preview_layers(Some(&editor.cos), &doc.info.layers);
        doc.fonts_stale = false;
        let mut form = form_of(&editor.cos, doc.tabs_follow_structure && doc.info.xfa.is_none());
        xfa::mark_script_buttons(&editor.cos, &mut form);
        doc.form = Arc::new(form);
        if invalidate_markers {
            doc.marker_scan = OnceLock::new();
        }
        if invalidate_links {
            doc.links = OnceLock::new();
        }
        let mut signatures = signatures_of(&editor.cos, &bytes, &doc.trust, &doc.sig_cache);
        doc.display = display_bytes(Some(editor), &bytes, &mut signatures, &doc.captions);
        doc.signatures = Arc::new(signatures);
        doc.renderer = RenderPool::new(doc.display.clone(), render_threads(), display_config(&doc.config));
        doc.bytes = bytes;
        Ok(())
    }
}

/// The optional content groups `cos` lists as layers, in order (what the inspection reads: a group
/// listed twice is one layer, and one that is not a dictionary is none).
fn layer_ids(cos: &pdfcraft_cos::Document) -> Vec<(u32, u16)> {
    let props =
        cos.trailer().get(b"Root").map(|r| cos.resolve(r)).and_then(|r| r.as_dict().and_then(|r| r.get(b"OCProperties").map(|p| cos.resolve(p))));
    let groups = props.as_deref().and_then(|p| p.as_dict()).and_then(|p| p.get(b"OCGs").map(|g| cos.resolve(g)));
    let mut ids: Vec<(u32, u16)> = Vec::new();
    for group in groups.as_deref().and_then(|g| g.as_array()).into_iter().flatten() {
        let Some(id) = group.as_ref().map(|id| (id.num, id.generation)) else { continue };
        if cos.resolve(group).as_dict().is_some() && !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

/// Give `new`, the layers of a file whose objects were renumbered, the visibility the user chose on
/// `old`. The numbers say nothing across a rewrite (one may now belong to another layer), so each
/// layer is matched by name: the nth layer called so in `new` is the nth one called so in `old`
/// (the writer keeps the `/OCGs` order). A layer with no counterpart keeps the file's own default.
fn carry_renumbered_layer_choices(old: &[pdfcraft_render::Layer], new: &mut [pdfcraft_render::Layer]) {
    let mut seen: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for layer in new {
        let nth = seen.entry(layer.name.as_str()).or_insert(0);
        let counterpart = old.iter().filter(|o| o.name == layer.name).nth(*nth);
        *nth += 1;
        layer.visible = counterpart.map_or(layer.initial, |o| o.visible);
    }
}

/// Hand on what an edit's XFA scripts showed or asked for (`out`), and when one of them ran away,
/// turn the form's scripts off and say so.
fn take_xfa_output(doc: &mut Document, out: js::JsOutput, ran_away: bool) {
    doc.js_output.append(out);
    if ran_away {
        doc.xfa_template = None;
        doc.xfa_scripts_off = true;
        doc.js_output.errors.push(XFA_SCRIPTS_OFF.into());
    }
}

/// The notice for text edits that drew a paragraph in a stand-in font (one for the whole edit,
/// batches included): the stand-ins once each, and the letters their paragraphs' fonts lacked.
fn substitute_notice(cx: &EditCtx) -> Option<EditNotice> {
    if cx.substituted.is_empty() {
        return None;
    }
    let mut fonts: Vec<String> = Vec::new();
    let mut missing: Vec<char> = Vec::new();
    for (font, letters) in &cx.substituted {
        if !fonts.contains(font) {
            fonts.push(font.clone());
        }
        for letter in letters {
            if !missing.contains(letter) {
                missing.push(*letter);
            }
        }
    }
    missing.truncate(pdfcraft_edit::text::MAX_MISSING_REPORTED);
    Some(EditNotice::SubstituteFont { fonts, missing })
}

/// What `edit`, taking `prev` to `next`, wants said: never silent (app-agents.md rule 13).
fn edit_notices(edit: &Edit, prev: &pdfcraft_cos::Document, next: &pdfcraft_cos::Document) -> Vec<EditNotice> {
    // What it drew with `?` (a value or comment text its font can't draw, stored as typed).
    let mut notices: Vec<EditNotice> = if matches!(scope_of(edit), Scope::Form | Scope::Comments | Scope::Full) {
        guard(|| undrawn_notices(prev, next)).unwrap_or_default().into_iter().map(EditNotice::Undrawn).collect()
    } else {
        Vec::new()
    };
    // Pages whose own white backdrop Remove background couldn't turn back on.
    if matches!(edit, Edit::RemoveMarks { kind: MarkKind::Background, .. }) {
        let pages = guard(|| pdfcraft_edit::backdrops_left_off(next)).unwrap_or_default();
        if !pages.is_empty() {
            notices.push(EditNotice::BackdropsLeftOff { pages });
        }
    }
    notices
}

#[cfg(test)]
mod signature_damage_tests {
    use super::{Act, contains_field_signer, signature_damage, signature_damage_by};
    use crate::Edit;
    use pdfcraft_sign::pdf::{Modification, SignatureInfo, Status, kind};

    fn sig(status: Status, modification: Modification, checked: bool, certify: Option<u8>) -> SignatureInfo {
        SignatureInfo {
            field: "Sig".into(),
            signed: true,
            page: None,
            rect: None,
            visible: false,
            signer: None,
            certificate: None,
            chain: Vec::new(),
            date: None,
            signing_time: None,
            reason: None,
            location: None,
            contact: None,
            sub_filter: None,
            doc_timestamp: false,
            certify,
            revision: 1,
            signed_len: 1,
            digest: None,
            algorithm: None,
            timestamp: false,
            timestamp_time: None,
            status,
            modification,
            changes_checked: checked,
            verdict_shown: false,
            details: Vec::new(),
        }
    }

    fn damage(old: SignatureInfo, new: SignatureInfo) -> Option<String> {
        signature_damage(&[old], &[new])
    }

    #[test]
    fn a_signing_may_not_add_a_kind_of_forbidden_change_to_a_signature_that_already_had_one() {
        let old = sig(Status::Invalid, Modification::Disallowed(vec![kind::OTHER]), true, None);
        let same = sig(Status::Invalid, Modification::Disallowed(vec![kind::OTHER]), true, None);
        let more = sig(Status::Invalid, Modification::Disallowed(vec![kind::OTHER, kind::LOCKED_FIELD]), true, None);
        assert!(signature_damage_by(std::slice::from_ref(&old), &[same], Act::Signing).is_none());
        let why = signature_damage_by(std::slice::from_ref(&old), std::slice::from_ref(&more), Act::Signing).unwrap();
        assert!(why.contains("covered by the lock") && why.contains("the document was not signed"), "{why}");
        // A label keeps its older, looser rule: the same signature passes it.
        assert!(signature_damage_by(&[old], &[more], Act::Label).is_none());
    }

    #[test]
    fn forbidden_changes_are_refused_even_when_the_status_was_already_invalid() {
        let old = sig(Status::Invalid, Modification::Allowed(vec![]), true, None);
        let new = sig(Status::Invalid, Modification::Disallowed(vec![kind::OTHER]), true, None);
        assert!(damage(old, new).unwrap().contains("forbids this change"));
    }

    #[test]
    fn a_status_that_falls_is_refused_with_its_own_sentence() {
        let old = sig(Status::Valid, Modification::None, true, None);
        let new = sig(Status::Unknown, Modification::None, true, None);
        let why = damage(old, new).unwrap();
        assert!(why.contains("from valid to unknown"), "{why}");
    }

    #[test]
    fn changes_that_can_no_longer_be_checked_are_refused_with_their_own_sentence() {
        let old = sig(Status::Unknown, Modification::None, true, None);
        let new = sig(Status::Unknown, Modification::None, false, None);
        let why = damage(old, new).unwrap();
        assert!(why.contains("could no longer be checked") && !why.contains("lock"), "{why}");
    }

    #[test]
    fn the_sentence_follows_the_cause_of_a_forbidden_change() {
        let old = sig(Status::Valid, Modification::None, true, None);
        let cert = sig(Status::Invalid, Modification::Disallowed(vec![kind::OTHER]), true, Some(1));
        let why = damage(old.clone(), cert).unwrap();
        assert!(why.contains("certification that allows no changes") && why.contains("any field"), "{why}");
        let lock = sig(Status::Invalid, Modification::Disallowed(vec![kind::LOCKED_FIELD]), true, None);
        let why = damage(old.clone(), lock).unwrap();
        assert!(why.contains("covered by the lock") && !why.contains("any field"), "{why}");
        let other = sig(Status::Invalid, Modification::Disallowed(vec![kind::PAGES]), true, Some(2));
        let why = damage(old, other).unwrap();
        assert!(why.contains("forbids this change") && !why.contains("any field") && !why.contains("lock"), "{why}");
    }

    #[test]
    fn a_signature_that_already_forbade_the_changes_is_not_made_worse_by_the_label() {
        let broken = sig(Status::Invalid, Modification::Disallowed(vec![kind::OTHER]), true, None);
        assert_eq!(damage(broken.clone(), broken), None);
        let fine = sig(Status::Valid, Modification::Allowed(vec![kind::FORM_FILL]), true, Some(2));
        assert_eq!(damage(fine.clone(), fine), None);
    }

    #[test]
    fn a_signature_that_cannot_be_matched_is_refused() {
        let one = sig(Status::Valid, Modification::None, true, None);
        assert!(signature_damage(std::slice::from_ref(&one), &[]).unwrap().contains("could not be compared"));
        // An unsigned field in front changes nothing.
        let mut unsigned = one.clone();
        unsigned.signed = false;
        assert!(signature_damage(&[unsigned, one], &[sig(Status::Valid, Modification::None, true, None)]).is_some());
    }

    #[test]
    fn a_batch_holding_a_label_edit_counts_as_one() {
        let label = Edit::SetFieldSigner { field: pdfcraft_cos::ObjRef::new(1, 0), email: None };
        assert!(contains_field_signer(&label));
        assert!(contains_field_signer(&Edit::Batch { label: "x".into(), edits: vec![Edit::Batch { label: "y".into(), edits: vec![label] }] }));
        assert!(!contains_field_signer(&Edit::RemoveCollection));
    }
}
