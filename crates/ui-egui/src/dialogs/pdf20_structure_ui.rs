//! Standards: PDF 2.0 structure. The tools `ns_list` and `dpart_list` in one dialog: the structure namespaces of the
//! document and its document part tree, read only. It lists what the files hold and what the reads left out; it
//! never says a namespace or a part tree is valid.

use craft::theme::variant::Tone;
use craft::ui::contract::{Bind, Show as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::style::TypeRole;
use egui::Ui;
use pdfcraft_engine::DocId;

use crate::PdfCraftApp;

/// The dialog has no fields of its own; the form is only what the dialog shell binds.
#[derive(Clone, Debug, Default, PartialEq)]
struct NoFields;

/// One line of a listing.
#[derive(Clone, Debug)]
struct Line {
    /// Nesting level: a part's depth, or 1 for a role under its namespace.
    indent: usize,
    text: String,
    /// A detail under a line, not a heading of its own.
    muted: bool,
}

/// What the active document holds, read when the document or its edit count changed.
#[derive(Clone, Debug, Default)]
struct Listing {
    has_structure_tree: bool,
    namespace_count: usize,
    namespaces: Vec<Line>,
    namespace_notes: Vec<String>,
    has_root: bool,
    part_count: usize,
    parts: Vec<Line>,
    part_notes: Vec<String>,
}

/// PDF 2.0 structure, owned by the dialog host.
pub(crate) struct Pdf20StructureDialog {
    id: DialogId,
    form: NoFields,
    shown: Listing,
    /// The document and edit count `shown` was read at.
    key: Option<(DocId, u64)>,
}

fn count_text(n: usize) -> String {
    n.to_string()
}

/// The pages a part covers, 1-based.
fn pages_text(start: Option<usize>, end: Option<usize>) -> String {
    match (start, end) {
        (Some(a), Some(b)) if a == b => crate::i18n::fmt(tl!("page {n}"), &[("n", &count_text(a))]),
        (Some(a), Some(b)) => crate::i18n::fmt(tl!("pages {from} to {to}"), &[("from", &count_text(a)), ("to", &count_text(b))]),
        (Some(a), None) => crate::i18n::fmt(tl!("page {n}"), &[("n", &count_text(a))]),
        (None, Some(b)) => crate::i18n::fmt(tl!("ends on page {n}"), &[("n", &count_text(b))]),
        (None, None) => tl!("no pages named").to_string(),
    }
}

/// A line with a count in it.
fn counted(template: &str, n: usize) -> String {
    crate::i18n::fmt(template, &[("count", &count_text(n))])
}

impl PdfCraftApp {
    /// Opens PDF 2.0 structure for the active document; the lists are read on the first pass.
    pub(crate) fn open_pdf20_structure(&mut self) {
        if self.active_ids().is_none() {
            return;
        }
        let dialog = Pdf20StructureDialog { id: dialog_id("pdf20-structure"), form: NoFields, shown: Listing::default(), key: None };
        crate::dialogs::host::open(self, dialog);
    }

    /// The namespaces and document parts of document `id`.
    fn pdf20_structure_listing(&self, id: DocId) -> Listing {
        let Some(doc) = self.session.get(id) else { return Listing::default() };
        let ns = doc.structure_namespaces();
        let mut out = Listing { has_structure_tree: ns.has_structure_tree, namespace_count: ns.namespaces.len(), ..Listing::default() };
        for n in &ns.namespaces {
            let name = n.ns.clone().unwrap_or_else(|| tl!("(no /NS identifier)").to_string());
            let object = n.object.map(|o| o.to_string()).unwrap_or_else(|| tl!("direct").to_string());
            let schema = if n.has_schema { tl!("has a /Schema") } else { tl!("no /Schema") };
            out.namespaces.push(Line { indent: 0, text: name, muted: false });
            out.namespaces.push(Line {
                indent: 1,
                text: crate::i18n::fmt(
                    tl!("object {object}, {schema}, {count} role map entries"),
                    &[("object", &object), ("schema", schema), ("count", &count_text(n.role_count))],
                ),
                muted: true,
            });
            for r in &n.roles {
                let role = r.role.clone().unwrap_or_else(|| tl!("(no role name)").to_string());
                let mut text = crate::i18n::fmt(tl!("{type} maps to {role}"), &[("type", &r.structure_type), ("role", &role)]);
                match (&r.role_namespace, r.role_namespace_listed) {
                    (Some(space), Some(false)) => {
                        text = crate::i18n::fmt(tl!("{text} in {space} (not listed in /Namespaces)"), &[("text", &text), ("space", space)]);
                    }
                    (Some(space), _) => text = crate::i18n::fmt(tl!("{text} in {space}"), &[("text", &text), ("space", space)]),
                    (None, Some(false)) => text = crate::i18n::fmt(tl!("{text} in a namespace not listed in /Namespaces"), &[("text", &text)]),
                    (None, _) => {}
                }
                out.namespaces.push(Line { indent: 2, text, muted: false });
            }
            if n.role_count > n.roles.len() {
                out.namespaces.push(Line {
                    indent: 2,
                    text: counted(tl!("{count} more role map entries are not listed."), n.role_count.saturating_sub(n.roles.len())),
                    muted: true,
                });
            }
        }
        if ns.truncated {
            out.namespace_notes.push(tl!("The list was cut short: more namespaces exist than are listed.").to_string());
        }
        if ns.skipped > 0 {
            out.namespace_notes.push(counted(tl!("{count} items of /Namespaces are not dictionaries and are not listed."), ns.skipped));
        }
        if ns.without_ns > 0 {
            out.namespace_notes.push(counted(tl!("{count} namespaces have no /NS identifier."), ns.without_ns));
        }
        for d in &ns.duplicates {
            out.namespace_notes.push(crate::i18n::fmt(
                tl!("The identifier {ns} is carried by {count} namespaces."),
                &[("ns", &d.ns), ("count", &count_text(d.count))],
            ));
        }
        if ns.unlisted_role_namespaces > 0 {
            out.namespace_notes
                .push(counted(tl!("{count} role entries name a namespace that /Namespaces does not list."), ns.unlisted_role_namespaces));
        }

        let parts = doc.document_parts();
        out.has_root = parts.has_root;
        out.part_count = parts.parts.len();
        if let Some(level) = parts.record_level {
            out.parts.push(Line { indent: 0, text: crate::i18n::fmt(tl!("Record level {level}"), &[("level", &level.to_string())]), muted: true });
        }
        let mut metadata_cut = false;
        for p in &parts.parts {
            let pages = pages_text(p.start_page, p.end_page);
            let object = p.object.to_string();
            out.parts.push(Line {
                indent: p.depth,
                text: crate::i18n::fmt(
                    tl!("Part {object}: {pages}, {count} child parts"),
                    &[("object", &object), ("pages", &pages), ("count", &count_text(p.children))],
                ),
                muted: false,
            });
            let mut flags = Vec::new();
            if p.range_inverted {
                flags.push(tl!("its pages run backwards").to_string());
            }
            if p.range_unresolved {
                flags.push(tl!("its /Start or /End names no page of the document").to_string());
            }
            if p.parent_mismatch {
                flags.push(tl!("its /Parent is not the part that lists it").to_string());
            }
            if !flags.is_empty() {
                out.parts.push(Line { indent: p.depth.saturating_add(1), text: flags.join("; "), muted: true });
            }
            if !p.metadata.is_empty() {
                let entries: Vec<String> = p.metadata.iter().map(|(k, v)| format!("{k} = {v}")).collect();
                out.parts.push(Line {
                    indent: p.depth.saturating_add(1),
                    text: crate::i18n::fmt(tl!("/DPM: {entries}"), &[("entries", &entries.join("; "))]),
                    muted: true,
                });
            }
            metadata_cut |= p.metadata_truncated;
        }
        if parts.truncated {
            out.part_notes.push(tl!("The tree was cut short: more parts, levels, children or pages exist than were read.").to_string());
        }
        if parts.skipped > 0 {
            out.part_notes.push(counted(tl!("{count} references were not read as parts (not dictionaries, or already read)."), parts.skipped));
        }
        if metadata_cut {
            out.part_notes.push(tl!("Some /DPM entries are not listed.").to_string());
        }
        if parts.inverted_ranges > 0 {
            out.part_notes.push(counted(tl!("{count} parts have an end page before their start page."), parts.inverted_ranges));
        }
        if parts.unresolved_ranges > 0 {
            out.part_notes.push(counted(tl!("{count} parts have a /Start or /End that names no page of the document."), parts.unresolved_ranges));
        }
        if parts.parent_mismatches > 0 {
            out.part_notes.push(counted(tl!("{count} parts have a /Parent that is not the part that lists them."), parts.parent_mismatches));
        }
        out
    }

    /// Reads the lists again when the document or its edit count changed.
    pub(crate) fn pdf20_structure_pass(&mut self) {
        if self.dialogs.get::<Pdf20StructureDialog>().is_none() {
            return;
        }
        let Some((_, id)) = self.active_ids() else { return };
        let now = self.session.get(id).map(|d| (id, d.edit_generation()));
        if self.dialogs.get::<Pdf20StructureDialog>().is_some_and(|d| d.key != now) {
            let listing = self.pdf20_structure_listing(id);
            if let Some(d) = self.dialogs.get_mut::<Pdf20StructureDialog>() {
                d.shown = listing;
                d.key = now;
            }
        }
    }
}

/// Shows lines, each indented by its level.
fn show_lines(ui: &mut Ui, lines: &[Line]) {
    for line in lines {
        ui.horizontal(|ui| {
            ui.add_space(16.0 * line.indent.min(16) as f32);
            let text = Text::new(line.text.clone());
            if line.muted {
                text.role(TypeRole::Caption).shade(Shade::Muted).show(ui);
            } else {
                text.show(ui);
            }
        });
    }
}

fn show_notes(ui: &mut Ui, notes: &[String]) {
    for note in notes {
        Notice::new(Tone::Warn, note.clone()).layout(NoticeLayout::Inline).show(ui);
    }
}

impl StackDialog for Pdf20StructureDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let Pdf20StructureDialog { form, shown, .. } = self;
        let close = FooterButton::new(tl!("Close").to_string(), DialogRole::Cancel);
        let out = FormDialog::new(tl!("PDF 2.0 structure").to_string(), Bind::bound(form)).shaped(|d| d.buttons([close])).show(ui, |ui, _, _| {
            Text::new(
                tl!("Read only: the structure namespaces and the document part tree the file holds, as the tools ns_list and dpart_list list them. Nothing is changed, and nothing here says they are valid.")
                    .to_string(),
            )
            .role(TypeRole::Caption)
            .shade(Shade::Muted)
            .show(ui);

            Text::new(tl!("Namespaces").to_string()).role(TypeRole::Heading).show(ui);
            if shown.namespaces.is_empty() {
                Text::new(tl!("This document has no namespaces.").to_string()).show(ui);
                if !shown.has_structure_tree {
                    Text::new(tl!("It has no structure tree.").to_string()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                }
            } else {
                Text::new(counted(tl!("{count} namespaces"), shown.namespace_count)).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                show_lines(ui, &shown.namespaces);
            }
            show_notes(ui, &shown.namespace_notes);

            Text::new(tl!("Document parts").to_string()).role(TypeRole::Heading).show(ui);
            if shown.part_count == 0 {
                Text::new(tl!("This document has no document parts.").to_string()).show(ui);
                if shown.has_root {
                    Text::new(tl!("It has a /DPartRoot, but no part nodes were read from it.").to_string())
                        .role(TypeRole::Caption)
                        .shade(Shade::Muted)
                        .show(ui);
                }
            } else {
                Text::new(counted(tl!("{count} parts"), shown.part_count)).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
            }
            show_lines(ui, &shown.parts);
            show_notes(ui, &shown.part_notes);
            true
        });
        out.closed.map(|_| DialogResult::Cancel)
    }
}
