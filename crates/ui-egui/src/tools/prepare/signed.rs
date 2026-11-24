//! The "When signed" part of a signature field's Field Properties: the fields a signature locks (`/Lock`) and the seed
//! values it suggests (`/SV`, here the reasons to pick from), and the commands behind it: `form.field.lock.set`,
//! `form.field.seed.set` and `form.field.seed.clear`.
//!
//! The commands make the edits the `signature_field_lock` and `signature_seed_values` tools make: the engine's
//! `signature_lock_change` and `signature_seed_change` decide, then `Edit::SetSignatureLock` or `Edit::SetSignatureSeed`
//! writes, one undo step each. They act only on a Field Properties dialog that is open on the active document. Setting
//! the reasons keeps the other seed entries the field holds (formats, digest methods, revocation); when the field's seed
//! values hold something this window cannot show whole, they are left alone.
//!
//! Both are entries in the file for the one who signs and for other programs. PdfCraft does not apply them: it does not
//! make the locked fields read-only after a signature and it does not read the seed values when it signs. The window says
//! so. Nothing is signed or sent.

use pdfcraft_engine::{DocId, Edit, LockAction, LockChange, LockSpec, SeedChange, SeedEntry, SeedSpec};

use super::FormFieldKind;
use super::dialog::FieldPropsDialog;
use super::signer::clean;
use crate::PdfCraftApp;

/// The longest piece of a refusal read from a file that is shown (characters).
const SHOWN_CHARS: usize = 300;

/// What a signature locks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LockChoice {
    /// Nothing: no `/Lock`.
    #[default]
    Nothing,
    /// Every field of the form.
    All,
    /// Only the fields listed.
    Include,
    /// Every field but the ones listed.
    Exclude,
}

/// The choices, in the order the window lists them.
pub const LOCK_CHOICES: [LockChoice; 4] = [LockChoice::Nothing, LockChoice::All, LockChoice::Include, LockChoice::Exclude];

impl LockChoice {
    pub fn label(self) -> &'static str {
        match self {
            LockChoice::Nothing => tl!("Lock nothing"),
            LockChoice::All => tl!("Lock all fields"),
            LockChoice::Include => tl!("Lock these fields"),
            LockChoice::Exclude => tl!("Lock all fields except these"),
        }
    }

    /// Whether a list of field names goes with the choice.
    pub fn lists(self) -> bool {
        matches!(self, LockChoice::Include | LockChoice::Exclude)
    }
}

/// What the file holds now, as the window says it (a live line), read again every pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignedView {
    pub lock: String,
    pub lock_has_entry: bool,
    pub seed: String,
    pub seed_has_entry: bool,
    /// The document's undo depth and whether a redo is waiting when the lines were made: they are made again when it
    /// moves, which is when an edit, an undo or a redo may have changed the file.
    pub stamp: (usize, bool),
}

/// The "When signed" entries as typed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignedDraft {
    pub lock: LockChoice,
    /// The fields of an include or exclude lock, one name per line.
    pub lock_fields: String,
    /// The reasons to pick from, one per line.
    pub reasons: String,
    /// The signer must pick one of the reasons.
    pub reason_required: bool,
    pub view: SignedView,
}

/// The names or reasons typed, one per line: trimmed, with the empty lines left out.
fn lines(text: &str) -> Vec<String> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect()
}

impl PdfCraftApp {
    /// The one field called `name` of document `id` as (object, kind), when the name picks out one field.
    fn one_field(&self, id: DocId, name: &str) -> Option<(pdfcraft_cos::ObjRef, FormFieldKind)> {
        let doc = self.session.get(id)?;
        let mut named = doc.form.iter().filter(|f| f.name == name);
        match (named.next(), named.next()) {
            (Some(f), None) => Some((f.obj, f.kind)),
            _ => None,
        }
    }

    /// What the field `name` of document `id` holds as lock and seed values, said in words, and what to start the
    /// typed entries from.
    pub(crate) fn signed_state(&self, id: DocId, name: &str) -> Option<SignedDraft> {
        let (obj, _) = self.one_field(id, name)?;
        let doc = self.session.get(id)?;
        let stamp = (doc.undo_depth(), doc.can_redo().is_some());
        let lock = doc.signature_lock(obj);
        let seed = doc.signature_seed(obj);
        let list = |items: &[String]| items.iter().map(|s| clean(s, 200)).collect::<Vec<_>>().join(", ");
        let (choice, lock_line) = match (lock.has_entry, lock.action.as_deref()) {
            (false, _) => (LockChoice::Nothing, tl!("When signed: no fields are locked").to_string()),
            (true, Some("All")) if lock.present => (LockChoice::All, tl!("When signed: all fields are locked").to_string()),
            (true, Some("Include")) if lock.present => {
                (LockChoice::Include, crate::i18n::fmt(tl!("When signed: these fields are locked: {names}"), &[("names", &list(&lock.names))]))
            }
            (true, Some("Exclude")) if lock.present => (
                LockChoice::Exclude,
                crate::i18n::fmt(tl!("When signed: all fields except these are locked: {names}"), &[("names", &list(&lock.names))]),
            ),
            (true, _) => (LockChoice::Nothing, tl!("When signed: a lock this window does not describe").to_string()),
        };
        let required = seed.required.contains(&SeedEntry::Reasons);
        let seed_line = if !seed.has_entry {
            tl!("Seed values: none").to_string()
        } else if seed.reasons.items.is_empty() {
            tl!("Seed values: set, without reasons").to_string()
        } else if required {
            crate::i18n::fmt(tl!("Seed values: reasons {list}; the signer must pick one"), &[("list", &list(&seed.reasons.items))])
        } else {
            crate::i18n::fmt(tl!("Seed values: reasons {list}"), &[("list", &list(&seed.reasons.items))])
        };
        Some(SignedDraft {
            lock: choice,
            lock_fields: if choice.lists() { lock.names.join("\n") } else { String::new() },
            reasons: seed.reasons.items.join("\n"),
            reason_required: required,
            view: SignedView { lock: lock_line, lock_has_entry: lock.has_entry, seed: seed_line, seed_has_entry: seed.has_entry, stamp },
        })
    }

    /// Fills the "When signed" entries of the open Field Properties from the document. Asked when the dialog opens.
    pub(crate) fn load_signed_entry(&mut self, id: DocId) {
        let Some((field, signature)) = self.dialogs.get::<FieldPropsDialog>().map(|d| (d.signer_field().to_owned(), d.signer_field_is_signature()))
        else {
            return;
        };
        if !signature {
            return;
        }
        let Some(state) = self.signed_state(id, &field) else { return };
        if let Some(dialog) = self.dialogs.get_mut::<FieldPropsDialog>() {
            dialog.set_signed(state);
        }
    }

    /// Keeps the lines the open Field Properties says about the file current, so an undo or a redo is seen. What was typed
    /// is left as it is.
    pub(crate) fn refresh_signed_view(&mut self) {
        let Some((id, field, signature)) =
            self.dialogs.get::<FieldPropsDialog>().and_then(|d| Some((d.signer_doc()?, d.signer_field().to_owned(), d.signer_field_is_signature())))
        else {
            return;
        };
        if !signature {
            return;
        }
        let now = self.session.get(id).map(|d| (d.undo_depth(), d.can_redo().is_some()));
        if now == self.dialogs.get::<FieldPropsDialog>().map(|d| d.signed().view.stamp) {
            return;
        }
        let Some(state) = self.signed_state(id, &field) else { return };
        if let Some(dialog) = self.dialogs.get_mut::<FieldPropsDialog>() {
            dialog.set_signed_view(state.view);
        }
    }

    /// The signature field of the Field Properties open on the active document, with what is typed in it.
    fn open_signature_field(&mut self) -> Option<(DocId, String, pdfcraft_cos::ObjRef, SignedDraft)> {
        let (_, id) = self.active_ids()?;
        let open =
            self.dialogs.get::<FieldPropsDialog>().filter(|d| d.signer_doc() == Some(id)).map(|d| (d.signer_field().to_owned(), d.signed().clone()));
        let Some((name, typed)) = open else {
            self.notify_tr("Open the signature field's properties first");
            return None;
        };
        match self.one_field(id, &name) {
            Some((obj, FormFieldKind::Signature)) => Some((id, name, obj, typed)),
            Some(_) => {
                self.notify_tr("A lock and seed values can only be set on a signature field");
                None
            }
            None => {
                self.notify_tr("That name does not pick out one field");
                None
            }
        }
    }

    /// Shows the file's state in the open dialog after a command, and starts the typed entries from it.
    fn show_signed_state(&mut self, id: DocId, name: &str) {
        let Some(state) = self.signed_state(id, name) else { return };
        if let Some(dialog) = self.dialogs.get_mut::<FieldPropsDialog>().filter(|d| d.signer_doc() == Some(id) && d.signer_field() == name) {
            dialog.set_signed(state);
        }
    }

    /// `form.field.lock.set`: write the lock typed in the open Field Properties (or take the lock out, for Lock nothing)
    /// on its signature field. One undo step; refused where `signature_field_lock` is.
    pub fn set_signature_lock(&mut self, ctx: &egui::Context) {
        let Some((id, name, obj, typed)) = self.open_signature_field() else { return };
        let action = match typed.lock {
            LockChoice::Nothing => None,
            LockChoice::All => Some(LockAction::All),
            LockChoice::Include => Some(LockAction::Include),
            LockChoice::Exclude => Some(LockAction::Exclude),
        };
        let spec = match action {
            None => None,
            Some(action) => {
                let names = if typed.lock.lists() { lines(&typed.lock_fields) } else { Vec::new() };
                match LockSpec::new(action, names) {
                    Ok(spec) => Some(spec),
                    Err(e) => return self.notify(clean(&e.to_string(), SHOWN_CHARS)),
                }
            }
        };
        let Some(doc) = self.session.get(id) else { return };
        if let Some(spec) = &spec {
            // A name is a field's fully qualified name, or one above it (it covers the fields below).
            for n in spec.names() {
                let below = format!("{n}.");
                if !doc.form.iter().any(|f| f.name == *n || f.name.starts_with(&below)) {
                    return self.notify_fmt("There is no field named {name} to lock", &[("name", &clean(n, 200))]);
                }
            }
        }
        match doc.signature_lock_change(obj, spec.as_ref()) {
            LockChange::Refused(why) => return self.notify(why),
            LockChange::NoChange => return self.notify_tr("This field has no lock to take out"),
            LockChange::Yes => {}
        }
        let removal = spec.is_none();
        if !self.apply_edit(ctx, Edit::SetSignatureLock { field: obj, lock: spec }) {
            return;
        }
        self.show_signed_state(id, &name);
        self.notify(if removal {
            tl!("The lock was taken out").to_string()
        } else {
            tl!("The lock was set. It is a note in the file: PdfCraft does not make the fields read-only after a signature, and nothing was signed or sent")
                .to_string()
        });
    }

    /// `form.field.seed.set` and `form.field.seed.clear`: write the reasons typed in the open Field Properties as the
    /// signature field's seed values (the formats, digest methods and revocation entries it holds are kept), or take the
    /// seed values out. One undo step; refused where `signature_seed_values` is.
    pub fn set_signature_seed(&mut self, ctx: &egui::Context, clear: bool) {
        let Some((id, name, obj, typed)) = self.open_signature_field() else { return };
        let spec = if clear {
            None
        } else {
            let Some(doc) = self.session.get(id) else { return };
            let read = doc.signature_seed(obj);
            let whole = read.exact
                && !read.reasons.truncated
                && !read.sub_filters.truncated
                && !read.digest_methods.truncated
                && read.reasons.skipped + read.sub_filters.skipped + read.digest_methods.skipped == 0;
            if read.has_entry && !whole {
                return self.notify_tr("The stored seed values hold entries this window cannot show whole, so they were left alone");
            }
            let reasons = lines(&typed.reasons);
            let mut required: Vec<SeedEntry> = read.required.iter().copied().filter(|e| *e != SeedEntry::Reasons).collect();
            if typed.reason_required && !reasons.is_empty() {
                required.push(SeedEntry::Reasons);
            }
            match SeedSpec::new(reasons, read.sub_filters.items.clone(), read.digest_methods.items.clone(), read.add_rev_info, required) {
                Ok(spec) => Some(spec),
                Err(_) if typed.reasons.trim().is_empty() => {
                    return self.notify_tr("Type at least one reason, or use No seed values to take them out");
                }
                Err(e) => return self.notify(clean(&e.to_string(), SHOWN_CHARS)),
            }
        };
        let Some(doc) = self.session.get(id) else { return };
        match doc.signature_seed_change(obj, spec.as_ref()) {
            SeedChange::Refused(why) => return self.notify(why),
            SeedChange::NoChange => return self.notify_tr("This field has no seed values to take out"),
            SeedChange::Yes => {}
        }
        if !self.apply_edit(ctx, Edit::SetSignatureSeed { field: obj, seed: spec }) {
            return;
        }
        self.show_signed_state(id, &name);
        self.notify(if clear {
            tl!("The seed values were taken out").to_string()
        } else {
            tl!("The seed values were set. They are a note in the file: PdfCraft does not read them when it signs, and nothing was signed or sent")
                .to_string()
        });
    }
}
