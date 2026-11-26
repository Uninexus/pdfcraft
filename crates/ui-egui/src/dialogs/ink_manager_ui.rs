//! Print production ▸ Ink Manager: the spot inks the document's Separation and DeviceN colour spaces name, listed as
//! the tool `doc_inks` lists them, and an alias that renames one ink to another existing name or to a typed one. An
//! alias is the engine's `AliasInks` edit, which `doc_ink_aliases` applies too: one undo step. Converting a spot ink
//! to a process colour and moving tint transforms are not done here.

use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::pickers::Select;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::style::TypeRole;
use craft::ui::text::TextField;
use egui::Ui;
use pdfcraft_engine::{DocId, Edit, InkAliases};

use crate::PdfCraftApp;

/// The dialog has no fields of its own; the form is only what the dialog shell binds.
#[derive(Clone, Debug, Default, PartialEq)]
struct NoFields;

/// One spot ink as the dialog shows it.
#[derive(Clone, Debug)]
struct InkRow {
    name: String,
    /// How many colour spaces name it.
    spaces: usize,
    /// Whether an alias can name it (a name that is not UTF-8 cannot).
    aliasable: bool,
}

/// What the active document holds, read when the document or its edit count changed.
#[derive(Clone, Debug, Default)]
struct Listing {
    inks: Vec<InkRow>,
    /// DeviceN spaces whose names were not read because the allowance was spent.
    unread_spaces: usize,
    /// The inks could not be read at all.
    unreadable: bool,
    /// The document's security allows changes.
    modifiable: bool,
}

/// Ink Manager, owned by the dialog host.
pub(crate) struct InkManagerDialog {
    id: DialogId,
    form: NoFields,
    shown: Listing,
    /// The document and edit count `shown` was read at.
    key: Option<(DocId, u64)>,
    /// The ink picked to be renamed.
    from: Option<String>,
    /// An existing ink whose name it takes; `None` when the typed name is used.
    existing: Option<String>,
    /// The name typed for the ink.
    typed: String,
    /// What the last Apply said, as (tone, text).
    notes: Vec<(Tone, String)>,
    /// Apply was pressed; `ink_manager_pass` carries it out.
    apply: bool,
}

impl InkManagerDialog {
    /// The name the picked ink is to be given.
    fn target(&self) -> String {
        self.existing.clone().unwrap_or_else(|| self.typed.trim().to_string())
    }
}

impl PdfCraftApp {
    /// Opens Ink Manager for the active document; the list is read on the first pass.
    pub(crate) fn open_ink_manager(&mut self) {
        if self.active_ids().is_none() {
            return;
        }
        let dialog = InkManagerDialog {
            id: dialog_id("ink-manager"),
            form: NoFields,
            shown: Listing::default(),
            key: None,
            from: None,
            existing: None,
            typed: String::new(),
            notes: Vec::new(),
            apply: false,
        };
        crate::dialogs::host::open(self, dialog);
    }

    /// The spot inks of document `id`, and whether it may be changed.
    fn ink_listing(&self, id: DocId) -> Listing {
        let Some(doc) = self.session.get(id) else { return Listing::default() };
        let modifiable = doc.read_only_reason.is_none() && doc.allows_modification();
        match self.session.list_inks(id) {
            Ok(found) => Listing {
                inks: found.inks.into_iter().map(|i| InkRow { name: i.name, spaces: i.spaces, aliasable: i.aliasable }).collect(),
                unread_spaces: found.unread_spaces,
                unreadable: false,
                modifiable,
            },
            Err(_) => Listing { unreadable: true, modifiable, ..Listing::default() },
        }
    }

    /// Renames ink `from` to `to` as the tool `doc_ink_aliases` does, and says what happened. Nothing is written unless
    /// a colour space names the ink; a refusal is told as one.
    fn run_ink_alias(&mut self, ctx: &egui::Context, id: DocId, from: Option<String>, to: String) -> Vec<(Tone, String)> {
        let Some(from) = from else { return vec![(Tone::Warn, tl!("Choose the ink to rename first.").to_string())] };
        if to.is_empty() {
            return vec![(Tone::Warn, tl!("Type a new name or choose another ink.").to_string())];
        }
        if from == to {
            return vec![(Tone::Warn, tl!("That is the ink's own name. Give it a different name.").to_string())];
        }
        let aliases = InkAliases { aliases: vec![(from.clone(), to.clone())] };
        let refused = |why: String| vec![(Tone::Bad, crate::i18n::fmt(tl!("Not applied: {reason}"), &[("reason", &why)]))];
        if let Err(why) = aliases.check() {
            return refused(why);
        }
        let found = match self.session.preview_ink_aliases(id, &aliases) {
            Ok(found) => found,
            Err(e) => return refused(e.to_string()),
        };
        if found.spaces == 0 {
            return vec![(Tone::Warn, tl!("No colour space names that ink, so nothing changed.").to_string())];
        }
        if !self.apply_edit(ctx, Edit::AliasInks { aliases }) {
            return vec![(Tone::Bad, tl!("The document did not accept the change, so nothing was renamed.").to_string())];
        }
        let mut notes = vec![(
            Tone::Good,
            crate::i18n::fmt(
                tl!("Renamed {from} to {to} in {spaces} colour spaces."),
                &[("from", &from), ("to", &to), ("spaces", &found.spaces.to_string())],
            ),
        )];
        if !found.different_alternates.is_empty() {
            let names = found.different_alternates.join(", ");
            notes.push((
                Tone::Warn,
                crate::i18n::fmt(
                    tl!("Different definitions under one name: {names}. The page looks the same on screen, but PDF/X wants one definition for each name."),
                    &[("names", &names)],
                ),
            ));
        }
        if found.skipped_nested > 0 {
            notes.push((
                Tone::Info,
                crate::i18n::fmt(
                    tl!("{n} nested structures were too deep to read and were left alone."),
                    &[("n", &found.skipped_nested.to_string())],
                ),
            ));
        }
        notes
    }

    /// Carries out the Apply the open dialog was asked for and reads the list again when the document changed.
    pub(crate) fn ink_manager_pass(&mut self, ctx: &egui::Context) {
        let Some(asked) = self.dialogs.get_mut::<InkManagerDialog>().map(|d| std::mem::take(&mut d.apply)) else { return };
        let Some((_, id)) = self.active_ids() else {
            self.dialogs.cancel::<InkManagerDialog>(ctx);
            return;
        };
        if asked {
            let request = self.dialogs.get::<InkManagerDialog>().map(|d| (d.from.clone(), d.target()));
            let modifiable = self.ink_listing(id).modifiable;
            let notes = match request {
                Some((from, to)) if modifiable => Some(self.run_ink_alias(ctx, id, from, to)),
                Some(_) => Some(vec![(Tone::Bad, tl!("This document does not allow changes, so inks cannot be renamed.").to_string())]),
                None => None,
            };
            if let (Some(notes), Some(d)) = (notes, self.dialogs.get_mut::<InkManagerDialog>()) {
                d.notes = notes;
            }
        }
        let generation = self.session.get(id).map(|d| d.edit_generation());
        let now = generation.map(|g| (id, g));
        if self.dialogs.get::<InkManagerDialog>().is_some_and(|d| d.key != now) {
            let listing = self.ink_listing(id);
            if let Some(d) = self.dialogs.get_mut::<InkManagerDialog>() {
                // A pick that the document no longer has is dropped; a renamed ink is gone from the list.
                if d.from.as_ref().is_some_and(|f| !listing.inks.iter().any(|i| &i.name == f)) {
                    d.from = None;
                }
                if d.existing.as_ref().is_some_and(|e| !listing.inks.iter().any(|i| &i.name == e)) {
                    d.existing = None;
                }
                if d.from.is_none() {
                    d.typed.clear();
                }
                d.shown = listing;
                d.key = now;
            }
        }
    }
}

/// The line under an ink's name: how many colour spaces name it.
fn detail(row: &InkRow) -> String {
    if row.spaces == 1 {
        tl!("named by 1 colour space").to_string()
    } else {
        crate::i18n::fmt(tl!("named by {n} colour spaces"), &[("n", &row.spaces.to_string())])
    }
}

impl StackDialog for InkManagerDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let InkManagerDialog { form, shown, from, existing, typed, notes, apply, .. } = self;
        let close = FooterButton::new(tl!("Close").to_string(), DialogRole::Cancel);
        let out = FormDialog::new(tl!("Ink Manager").to_string(), Bind::bound(form)).shaped(|d| d.buttons([close])).show(ui, |ui, _, _| {
            Text::new(
                tl!("The spot inks that the document's Separation and DeviceN colour spaces name. Giving an ink another ink's name makes them print as one. Each colour space keeps its own alternate and tint transform.")
                    .to_string(),
            )
            .role(TypeRole::Caption)
            .shade(Shade::Muted)
            .show(ui);
            if !shown.modifiable {
                Notice::new(Tone::Warn, tl!("This document does not allow changes, so inks cannot be renamed.").to_string())
                    .layout(NoticeLayout::Inline)
                    .show(ui);
            }
            if shown.unreadable {
                Notice::new(Tone::Bad, tl!("The inks of this document could not be read.").to_string()).layout(NoticeLayout::Inline).show(ui);
            }
            if shown.unread_spaces > 0 {
                let line = crate::i18n::fmt(
                    tl!("{n} DeviceN colour spaces were not read because the document has too many colorant names, so some inks are missing from the list and the counts may be low."),
                    &[("n", &shown.unread_spaces.to_string())],
                );
                Notice::new(Tone::Warn, line).layout(NoticeLayout::Inline).show(ui);
            }
            if shown.inks.is_empty() && !shown.unreadable {
                Text::new(tl!("This document uses no spot inks.").to_string()).show(ui);
            }
            for (at, ink) in shown.inks.iter().enumerate() {
                let line = detail(ink);
                Field::new(ink.name.clone()).value(line.clone()).id_salt(at).show(ui, |ui, _| {
                    Text::new(line.clone()).show(ui);
                    if !ink.aliasable {
                        Text::new(tl!("This name is not UTF-8, so an alias cannot name it.").to_string()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                    }
                });
                // Outside the field, so the button keeps its own name: inside, the field names its control after the row.
                let label = crate::i18n::fmt(tl!("Alias {name}"), &[("name", &ink.name)]);
                if Button::new(label).enabled(shown.modifiable && ink.aliasable).id_salt(at).show(ui).clicked() {
                    *from = Some(ink.name.clone());
                    *existing = None;
                }
            }
            if let Some(picked) = from.clone() {
                Text::new(crate::i18n::fmt(tl!("Renaming {name}"), &[("name", &picked)])).role(TypeRole::Heading).show(ui);
                let mut entries = vec![Entry::new(0_usize, tl!("Type a name below").to_string())];
                let mut names: Vec<&str> = Vec::new();
                for ink in shown.inks.iter().filter(|i| i.aliasable && i.name != picked) {
                    names.push(&ink.name);
                    entries.push(Entry::new(names.len(), ink.name.clone()));
                }
                let at = existing.as_ref().and_then(|e| names.iter().position(|n| *n == e.as_str())).map_or(0, |p| p + 1);
                let mut chosen = None;
                let caption = tl!("Use the name of this ink:").to_string();
                Field::new(caption.clone()).id_salt("ink-existing").show(ui, |ui, _| {
                    chosen = Select::new(caption.clone(), Bind::given(at), &entries).enabled(shown.modifiable).show(ui).value().copied();
                });
                if let Some(index) = chosen {
                    *existing = index.checked_sub(1).and_then(|p| names.get(p)).map(|n| (*n).to_string());
                }
                let caption = tl!("New ink name").to_string();
                Field::new(caption.clone()).id_salt("ink-typed").show(ui, |ui, _| {
                    TextField::new(caption.clone(), Bind::bound(&mut *typed)).enabled(shown.modifiable && existing.is_none()).show(ui);
                });
                if Button::new(tl!("Apply").to_string()).enabled(shown.modifiable).show(ui).clicked() {
                    *apply = true;
                }
            }
            for (tone, text) in &*notes {
                Notice::new(*tone, text.clone()).layout(NoticeLayout::Inline).show(ui);
            }
            true
        });
        out.closed.map(|_| DialogResult::Cancel)
    }
}
