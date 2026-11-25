//! Sign ▸ Signing plan: the local record of who is asked to sign the open document, shown and edited through registered
//! commands.
//!
//! The commands are `sign.plan.show` (open the window), `sign.plan.recipient.add`, `sign.plan.recipient.remove` and
//! `sign.plan.message.set`. They apply the functions of `pdfcraft_engine::signing_plan` (the same rules the
//! `signing_plan_*` tools use) to the file beside the saved PDF and write it atomically. The window's buttons only ask for
//! a command; the command reads the file again, so what it changes is what is on disk, not what the window last showed.
//!
//! It is a record only: nothing is sent, nobody is notified and no signature is collected or checked. Once a recipient has
//! a status the plan can no longer be edited here; recording a turn (`sign.plan.record`, `sign.plan.decline`) and
//! signing as the next recipient (`sign.plan.sign_next`) are in the same window. A step is only recorded after the window
//! asked for it and named the recipient: the command refuses when the next recipient is no longer that person.

use std::path::{Path, PathBuf};

use craft::behavior::keyed;
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::contract::{
    Bind, CollectionEvent, Entry, Politeness, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing, clicked,
};
use craft::ui::dialog::Dialog;
use craft::ui::display::{Shade, Text};
use craft::ui::form::{Field, FieldGroup};
use craft::ui::list::{List, ListRow, RowEvent, TrailingFace, TrailingPart};
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::runtime::focus::FocusRequest;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::style::TypeRole;
use craft::ui::text::{SelectableText, TextField};
use pdfcraft_engine::DocId;
use pdfcraft_engine::signing_plan::{self as plan_rules, MAX_MESSAGE_CHARS, Plan, Role, Status};

use crate::PdfCraftApp;
use crate::prepare::signer::clean;

/// The longest piece of a refusal read from a file that the window shows (characters).
const SHOWN_ERROR_CHARS: usize = 300;
/// Lines of a plan check shown; Copy the check has them all.
const MAX_CHECK_LINES: usize = 200;
/// The Remove part of a recipient's row.
const REMOVE: TrailingPart = TrailingPart::new(0);
/// The roles, in the order the Role choice lists them.
const ROLES: [Role; 3] = [Role::Signer, Role::Approver, Role::Viewer];

/// What the window knows of the plan file, as of the last time it was read.
#[derive(Clone, Debug)]
pub enum View {
    /// The document has no file path (or not a .pdf one): there is nowhere to keep a plan.
    NoFile(String),
    /// There is no plan file beside the document yet.
    None,
    /// The plan.
    Plan(Box<Plan>),
    /// The file is there and cannot be used (a refusal, with hidden characters taken out).
    Unreadable(String),
}

/// The Signing plan window's draft: what is typed and what was last read. It lives only while the window is open and
/// belongs to the document it was opened on.
#[derive(Clone, Debug)]
pub struct Draft {
    /// The document the window was opened on: the commands refuse any other.
    pub doc: DocId,
    pub name: String,
    pub email: String,
    pub role: Role,
    pub message: String,
    /// The address of the recipient a Remove button asked to take out.
    pub remove: Option<String>,
    /// The registered command a button asked for, run by the window after this pass.
    pub command: Option<&'static str>,
    pub view: View,
    /// What the last command did, in words (a live region), and whether it worked.
    pub result: String,
    pub result_ok: bool,
    /// A step the person asked to record, waiting for them to confirm it: the plan has no undo.
    pub confirm: Option<Confirm>,
    /// The address (lower case) of the recipient whose step the person confirmed. A record or decline command is
    /// refused without it, and when the plan's next recipient is someone else by the time it runs.
    pub expect: Option<String>,
    /// Where the focus goes in the next pass, once the control it was on is gone.
    pub refocus: Option<Refocus>,
    /// The text of the last plan check (`sign.plan.audit`), until the plan changes.
    pub check: Option<String>,
    /// The Copy the check button was pressed: the window puts the text on the clipboard after this pass.
    pub copy_check: bool,
}

/// A step waiting for the person's answer.
#[derive(Clone, Debug)]
pub struct Confirm {
    /// `sign.plan.record` or `sign.plan.decline`.
    pub command: &'static str,
    /// The address (lower case) of the recipient the question names.
    pub email: String,
    /// The question, naming the step and the person.
    pub question: String,
    /// The question was just asked: its Record it button takes the focus once.
    pub fresh: bool,
}

/// A control that takes the focus after a change, once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refocus {
    /// The button for the next recipient's turn.
    Turn,
    /// The Add recipient button.
    Add,
}

/// The question that asks whether to record a step: it names the step and the person.
fn question(decline: bool, role: Role, who: &str) -> String {
    let template = match (decline, role) {
        (true, _) => tl!(
            "Record {who} as declined? The plan can no longer be edited afterwards and a step cannot be taken back. Only the plan file changes: nothing is signed or sent."
        ),
        (false, Role::Signer) => tl!(
            "Record {who} as signed? The plan can no longer be edited afterwards and a step cannot be taken back. Only the plan file changes: nothing is signed or sent."
        ),
        (false, _) => tl!(
            "Record {who} as approved? The plan can no longer be edited afterwards and a step cannot be taken back. Only the plan file changes: nothing is signed or sent."
        ),
    };
    crate::i18n::fmt(template, &[("who", who)])
}

/// How a time (seconds since 1970, 0 when the host had no clock) is shown.
fn when(at: i64) -> String {
    match (at > 0).then(|| chrono::DateTime::from_timestamp(at, 0)).flatten() {
        Some(t) => t.format("%Y-%m-%d %H:%M UTC").to_string(),
        None => tl!("time not known").to_string(),
    }
}

pub(crate) fn status_label(s: Status) -> &'static str {
    match s {
        Status::Pending => tl!("Pending"),
        Status::Sent => tl!("Sent"),
        Status::Signed => tl!("Signed"),
        Status::Approved => tl!("Approved"),
        Status::Declined => tl!("Declined"),
    }
}

pub(crate) fn role_label(r: Role) -> &'static str {
    match r {
        Role::Signer => tl!("Signer"),
        Role::Approver => tl!("Approver"),
        Role::Viewer => tl!("Viewer"),
    }
}

pub(crate) fn state_label(plan: &Plan) -> &'static str {
    match plan.state() {
        "declined" => tl!("Declined: a recipient declined"),
        "completed" => tl!("Completed: everyone who signs or approves has done so"),
        _ => tl!("In progress"),
    }
}

/// The sidecar of the document at `path`, or why there is none.
pub(crate) fn sidecar(path: Option<&str>) -> Result<PathBuf, String> {
    let path = path
        .ok_or_else(|| tl!("The document has no saved file. Save it first: the signing plan is kept in a file beside the saved PDF.").to_string())?;
    if !Path::new(path).is_file() {
        return Err(
            tl!("The document's saved file is not there. Save it first: the signing plan is kept in a file beside the saved PDF.").to_string()
        );
    }
    plan_rules::sidecar_of(Path::new(path)).map_err(|e| e.0)
}

/// What the window shows for the document at `path`: the plan file read again, or why there is none.
fn view_of(path: Option<&str>) -> View {
    match sidecar(path) {
        Ok(sidecar) => read(&sidecar),
        Err(why) => View::NoFile(clean(&why, SHOWN_ERROR_CHARS)),
    }
}

fn read(sidecar: &Path) -> View {
    match plan_rules::read_sidecar(sidecar) {
        Ok(None) => View::None,
        Ok(Some(plan)) => View::Plan(Box::new(plan)),
        Err(e) => View::Unreadable(clean(&e.0, SHOWN_ERROR_CHARS)),
    }
}

/// What a recipient-or-message command changes in the plan.
enum Change {
    Add,
    Remove,
    Message,
    /// The next recipient's step is written down: their turn done (`false`) or declined (`true`).
    Record(bool),
}

impl PdfCraftApp {
    /// The document's saved path, or the refusal that says to save it.
    fn plan_sidecar(&self, id: DocId) -> Result<PathBuf, String> {
        sidecar(self.session.get(id).and_then(|d| d.path.as_deref()))
    }

    /// `sign.plan.show`: open the Signing plan window on the active document.
    pub fn show_signing_plan(&mut self) {
        let Some((_, id)) = self.active_ids() else { return };
        let view = view_of(self.session.get(id).and_then(|d| d.path.as_deref()));
        let message = if let View::Plan(p) = &view { p.message.clone() } else { String::new() };
        self.signing_plan = Some(Draft {
            doc: id,
            name: String::new(),
            email: String::new(),
            role: Role::Signer,
            message,
            remove: None,
            command: None,
            view,
            result: String::new(),
            result_ok: true,
            confirm: None,
            expect: None,
            refocus: None,
            check: None,
            copy_check: false,
        });
    }

    /// `sign.plan.recipient.add`, `sign.plan.recipient.remove` and `sign.plan.message.set`.
    ///
    /// They act only on a window that is open on the active document (Close discards the draft, and a draft of another
    /// document is refused), take what they need from it, read the plan file again, apply the change with the shared
    /// rules, and write the file atomically. A document with no saved path, a plan that cannot be read, a plan that has
    /// started and a change the rules refuse write nothing.
    pub fn change_signing_plan(&mut self, command: &str) {
        let Some((_, id)) = self.active_ids() else { return };
        let change = match command {
            "sign.plan.recipient.add" => Change::Add,
            "sign.plan.recipient.remove" => Change::Remove,
            "sign.plan.record" => Change::Record(false),
            "sign.plan.decline" => Change::Record(true),
            _ => Change::Message,
        };
        if self.signing_plan.as_ref().is_some_and(|d| d.doc != id) {
            self.signing_plan = None;
        }
        let Some(draft) = self.signing_plan.as_ref() else {
            return self.notify_tr("Open the signing plan first (Sign > Signing plan…)");
        };
        let outcome = self.plan_change(id, &change, draft);
        let path = self.session.get(id).and_then(|d| d.path.clone());
        let Some(draft) = self.signing_plan.as_mut() else { return };
        draft.remove = None;
        draft.confirm = None;
        // A confirmation is for one command: whatever came of it, the next step is asked for again.
        draft.expect = None;
        if outcome.is_ok() {
            draft.refocus = match change {
                Change::Record(_) => Some(Refocus::Turn),
                Change::Remove => Some(Refocus::Add),
                Change::Add | Change::Message => None,
            };
        }
        // Read it again either way, so the window shows the file as it is: what was written, or what made the command
        // refuse (a plan started elsewhere, a file gone).
        draft.view = view_of(path.as_deref());
        // A check says what the plan was when it was made.
        draft.check = None;
        draft.result_ok = outcome.is_ok();
        let said = match outcome {
            Ok(done) => {
                if matches!(change, Change::Add) {
                    draft.name.clear();
                    draft.email.clear();
                }
                done
            }
            Err(why) => why,
        };
        draft.result = said.clone();
        self.notify(said);
    }

    /// The change, written; the words that say what happened, or why it did not.
    fn plan_change(&self, id: DocId, change: &Change, draft: &Draft) -> Result<String, String> {
        let path = self.plan_sidecar(id)?;
        let now = self.session.now_secs();
        let current = match plan_rules::read_sidecar(&path) {
            Ok(plan) => plan,
            Err(e) => return Err(clean(&e.0, SHOWN_ERROR_CHARS)),
        };
        let shown = |e: plan_rules::PlanError| clean(&e.0, SHOWN_ERROR_CHARS);
        let (plan, said) = match change {
            Change::Add => {
                let mut plan = current.unwrap_or_else(|| Plan::empty(now));
                plan.add_recipient(&draft.name, &draft.email, draft.role, now).map_err(shown)?;
                let who = plan.recipients.last().map(|r| r.name.clone()).unwrap_or_default();
                (
                    plan,
                    crate::i18n::fmt(tl!("{who} was added to the signing plan. Nothing was sent and nobody was told"), &[("who", &clean(&who, 200))]),
                )
            }
            Change::Remove => {
                let Some(email) = draft.remove.as_deref() else {
                    return Err(tl!("Choose a recipient to remove first").to_string());
                };
                let mut plan = current.ok_or_else(|| tl!("There is no signing plan to take a recipient out of").to_string())?;
                plan.remove_recipient(email).map_err(shown)?;
                (plan, crate::i18n::fmt(tl!("{who} was removed from the signing plan"), &[("who", &clean(email, 254))]))
            }
            Change::Message => {
                let mut plan = current.ok_or_else(|| tl!("There is no signing plan yet: add a recipient first").to_string())?;
                if plan.message == draft.message {
                    return Err(tl!("The plan already has that message").to_string());
                }
                plan.set_message(&draft.message).map_err(shown)?;
                (plan, tl!("The message was set. It is kept in the plan file; nothing was sent").to_string())
            }
            Change::Record(decline) => {
                let expected = draft.expect.as_deref().ok_or_else(|| {
                    tl!("Confirm the step in the Signing plan window first: choose Record for the next recipient and answer the question").to_string()
                })?;
                let mut plan = current.ok_or_else(|| tl!("There is no signing plan to record a step in").to_string())?;
                let next = plan.next().cloned().ok_or_else(|| tl!("The plan is finished: everyone who signs or approves has done so").to_string())?;
                if next.email.to_lowercase() != expected {
                    return Err(crate::i18n::fmt(
                        tl!("The plan changed after you were asked: the next recipient is now {who}, not {expected}. Nothing was recorded"),
                        &[("who", &clean(&next.name, 200)), ("expected", &clean(expected, 254))],
                    ));
                }
                let index = plan.turn_of(&next.email.to_lowercase()).map_err(shown)?;
                let status = if *decline {
                    Status::Declined
                } else {
                    Status::done_for(next.role).ok_or_else(|| tl!("That recipient is a viewer, who has no turn").to_string())?
                };
                plan.record(index, status, now).map_err(shown)?;
                let template = match status {
                    Status::Declined => {
                        tl!("{who} is recorded as declined. Only the plan file was changed: nothing was signed, sent or checked")
                    }
                    Status::Approved => {
                        tl!("{who} is recorded as approved. Only the plan file was changed: nothing was signed, sent or checked")
                    }
                    _ => tl!("{who} is recorded as signed. Only the plan file was changed: nothing was signed, sent or checked"),
                };
                let said = crate::i18n::fmt(template, &[("who", &clean(&next.name, 200))]);
                (plan, said)
            }
        };
        let bytes = plan_rules::plan_bytes(&plan).map_err(shown)?;
        craft::atomic::atomic_write(&path, &bytes)
            .map_err(|e| crate::i18n::fmt(tl!("Couldn't write the signing plan: {e}"), &[("e", &clean(&e.to_string(), SHOWN_ERROR_CHARS))]))?;
        Ok(said)
    }
}

impl PdfCraftApp {
    /// Says what a command did in a notice and, while the window is open, in its live line.
    pub(crate) fn say_plan(&mut self, text: String, ok: bool) {
        if let Some(d) = self.signing_plan.as_mut() {
            d.result = text.clone();
            d.result_ok = ok;
        }
        self.notify(text);
    }

    /// `sign.plan.audit`: sets the plan beside the signatures the saved file holds and shows the result in the Signing plan
    /// window (opened first when it is not). It only reads: nothing is written, signed or sent.
    pub fn check_signing_plan(&mut self) {
        let Some((_, id)) = self.active_ids() else { return };
        if self.signing_plan.as_ref().is_none_or(|d| d.doc != id) {
            self.show_signing_plan();
        }
        match self.plan_check(id) {
            Ok((text, line)) => {
                if let Some(d) = self.signing_plan.as_mut() {
                    d.check = Some(text);
                }
                self.say_plan(line, true);
            }
            Err(why) => {
                if let Some(d) = self.signing_plan.as_mut() {
                    d.check = None;
                }
                self.say_plan(why, false);
            }
        }
    }

    /// The text of the check of document `id` and the one line that says how it came out, or why there is none.
    fn plan_check(&self, id: DocId) -> Result<(String, String), String> {
        let doc = self.session.get(id).ok_or_else(|| tl!("Open a document first").to_string())?;
        let sidecar = sidecar(doc.path.as_deref())?;
        if doc.dirty {
            return Err(tl!("The document has changes that are not saved. Save it first: the check is about the saved file.").to_string());
        }
        if doc.changed_on_open {
            return Err(tl!(
                "Opening this file changed it (a repair or a form layout), so it cannot be compared with the file on disk. Save it, reopen it and try again."
            )
            .to_string());
        }
        let plan = match plan_rules::read_sidecar(&sidecar) {
            Ok(Some(plan)) => plan,
            Ok(None) => return Err(tl!("There is no signing plan beside this document yet: add a recipient first.").to_string()),
            Err(e) => return Err(clean(&e.0, SHOWN_ERROR_CHARS)),
        };
        if let Some(path) = doc.path.as_deref() {
            let on_disk = self.host.files().read(Path::new(path)).map_err(|e| clean(&e.to_string(), SHOWN_ERROR_CHARS))?;
            if on_disk != *doc.saved_bytes() {
                return Err(tl!("The file changed on disk since it was opened. Reopen it and check again.").to_string());
            }
        }
        let report = self.session.signature_audit(id).map_err(|e| clean(&e.to_string(), SHOWN_ERROR_CHARS))?;
        let audit = pdfcraft_engine::plan_audit::cross_check(doc, &report, &plan);
        let line = match audit.consistent {
            Some(true) => tl!("The plan and the file agree, and nothing was left undetermined."),
            Some(false) => tl!("The plan and the file do not agree: see the problems listed."),
            None => tl!("No problem was found, but something could not be determined: see the open questions."),
        };
        Ok((audit.text(&plan), line.to_string()))
    }

    /// `sign.plan.sign_next`: start signing as the plan's next recipient. It finds the one unsigned signature field
    /// labelled for them, turns to its page and opens Sign with a Digital ID on it. The plan is only a record: once the
    /// signed file is saved, the step is written down with Record (a signature cannot be told from outside who made it).
    pub fn sign_as_next_recipient(&mut self, ctx: &egui::Context) {
        let Some((index, id)) = self.active_ids() else { return };
        let sidecar = match self.plan_sidecar(id) {
            Ok(p) => p,
            Err(why) => return self.notify(clean(&why, SHOWN_ERROR_CHARS)),
        };
        let plan = match plan_rules::read_sidecar(&sidecar) {
            Ok(Some(plan)) => plan,
            Ok(None) => return self.notify_tr("There is no signing plan beside this document: add a recipient in Sign > Signing plan… first"),
            Err(e) => return self.notify(clean(&e.0, SHOWN_ERROR_CHARS)),
        };
        let Some(next) = plan.next().filter(|_| plan.state() == "in_progress").cloned() else {
            return self.notify_tr("The signing plan has no one left to sign");
        };
        let who = clean(&next.name, 200);
        if next.role != Role::Signer {
            return self.notify_fmt("{who} approves rather than signs: record the step in the Signing plan window", &[("who", &who)]);
        }
        let key = next.email.to_lowercase();
        let Some(doc) = self.session.get(id) else { return };
        let fields: Vec<(String, Option<usize>)> = doc
            .form
            .iter()
            .filter(|f| f.kind == pdfcraft_engine::FormFieldKind::Signature)
            .filter(|f| {
                let read = doc.field_signer(f.obj);
                !read.signed && read.email.as_deref().is_some_and(|e| e.to_lowercase() == key)
            })
            .map(|f| (f.name.clone(), f.widgets.first().and_then(|w| w.page)))
            .collect();
        let (name, page) = match fields.as_slice() {
            [(name, Some(page))] => (name.clone(), *page),
            [(name, None)] => {
                return self.notify_fmt("The signature field {name} is on no page, so it cannot be signed from here", &[("name", &clean(name, 200))]);
            }
            [] => {
                return self.notify_fmt(
                    "No unsigned signature field is labelled for {who}: set the signer in the field's properties first",
                    &[("who", &who)],
                );
            }
            _ => {
                return self
                    .notify_fmt("More than one unsigned signature field is labelled for {who}: only one can be signed here", &[("who", &who)]);
            }
        };
        if let Some(view) = self.views.get_mut(index) {
            view.go_to_page(page);
        }
        // Sign opens as a dialog of its own: the plan window closes first, so two modals never stack. Reopen it with
        // Sign > Signing plan… to record the step.
        self.signing_plan = None;
        self.start_signing(ctx, page, None, Some(name), None);
        self.notify_fmt(
            "Signing as {who}. Save the signed file over this document so the plan stays beside it, then record the step in the Signing plan window",
            &[("who", &who)],
        );
    }
}

/// Shows the window while it is open, and runs the command a button asked for. It closes with the document it was opened
/// on (a plan of another document is not shown), and Close discards the draft.
pub(crate) fn show(app: &mut PdfCraftApp, ui: &mut egui::Ui) {
    let Some(mut d) = app.signing_plan.take() else { return };
    if app.active_ids().map(|(_, id)| id) != Some(d.doc) {
        return;
    }
    let mut open = true;
    let buttons = vec![FooterButton::new(tl!("Close"), DialogRole::Cancel)];
    let _ = Dialog::new(tl!("Signing plan")).opened(Bind::bound(&mut open)).buttons(buttons).show(ui, |ui, _cx| body(ui, &mut d));
    if !open {
        return;
    }
    let command = d.command.take();
    if std::mem::take(&mut d.copy_check)
        && let Some(text) = d.check.clone()
        && app.put_text(&text)
    {
        app.notify_tr("Check copied");
    }
    app.signing_plan = Some(d);
    if let Some(command) = command {
        let ctx = ui.ctx().clone();
        app.execute(&ctx, command);
    }
}

/// The next recipient's turn: record their step (asked first) or start signing as them.
fn turn(ui: &mut egui::Ui, plan: &Plan, refocus: &mut Option<Refocus>, confirm: &mut Option<Confirm>, command: &mut Option<&'static str>) {
    if let Some(next) = plan.next().filter(|_| plan.state() == "in_progress") {
        let who = clean(&next.name, 200);
        Text::new(tl!("Turn")).role(TypeRole::Heading).show(ui);
        let done = if next.role == Role::Signer { tl!("Record {who} as signed") } else { tl!("Record {who} as approved") };
        let email = next.email.to_lowercase();
        let mut record = Button::new(crate::i18n::fmt(done, &[("who", &who)]));
        if refocus.take_if(|r| *r == Refocus::Turn).is_some() {
            record = record.request_focus(FocusRequest::Take);
        }
        if clicked(&record.show(ui)) {
            *confirm = Some(Confirm { command: "sign.plan.record", email: email.clone(), question: question(false, next.role, &who), fresh: true });
        }
        if clicked(&Button::new(crate::i18n::fmt(tl!("Record {who} as declined"), &[("who", &who)])).show(ui)) {
            *confirm = Some(Confirm { command: "sign.plan.decline", email, question: question(true, next.role, &who), fresh: true });
        }
        if next.role == Role::Signer && clicked(&Button::new(crate::i18n::fmt(tl!("Sign as {who}…"), &[("who", &who)])).show(ui)) {
            *command = Some("sign.plan.sign_next");
        }
    }
}

/// The window's content.
fn body(ui: &mut egui::Ui, d: &mut Draft) {
    Text::new(tl!(
        "A local record of who is asked to sign this document, in what order. It is kept in a file beside the PDF. It does not send the document, notify anyone or collect signatures, and it does not check any signature in the PDF. Once a recipient has a status the plan can no longer be edited here."
    ))
    .shade(Shade::Muted)
    .show(ui);
    let mut editable = false;
    let mut remove = None;
    match &d.view {
        View::NoFile(why) => {
            Notice::new(Tone::Warn, why.clone()).layout(NoticeLayout::Inline).id_salt(10usize).show(ui);
        }
        View::Unreadable(why) => {
            let line =
                crate::i18n::fmt(tl!("The plan file beside this document cannot be used, and was left alone: {why}"), &[("why", why.as_str())]);
            Notice::new(Tone::Bad, line).layout(NoticeLayout::Inline).id_salt(11usize).show(ui);
        }
        View::None => {
            Text::new(tl!("There is no signing plan for this document yet. Add a recipient to make one.")).show(ui);
            editable = true;
        }
        View::Plan(plan) => {
            let state = crate::i18n::fmt(tl!("State: {state}"), &[("state", state_label(plan))]);
            Notice::new(Tone::Info, state).layout(NoticeLayout::Inline).announce(Politeness::Polite).id_salt(12usize).show(ui);
            if let Some(next) = plan.next().filter(|_| plan.state() == "in_progress") {
                let line = crate::i18n::fmt(tl!("Next: {who} ({email})"), &[("who", &clean(&next.name, 200)), ("email", &clean(&next.email, 254))]);
                Text::new(line).show(ui);
            }
            editable = !plan.started();
            if !editable {
                Text::new(tl!("A recipient already has a status, so recipients and the message can no longer be changed here."))
                    .role(TypeRole::Caption)
                    .shade(Shade::Muted)
                    .show(ui);
            }
            Text::new(tl!("Recipients")).role(TypeRole::Heading).show(ui);
            let rows: Vec<usize> = (0..plan.recipients.len()).collect();
            let source = keyed(&rows, |row: &usize| *row);
            let list = List::<usize>::new(tl!("Recipients")).select(SelectMode::None).sizing(Sizing::Content).natural_height(true).show_items(
                ui,
                &source,
                &mut |row, _| {
                    let Some(r) = plan.recipients.get(*row) else { return ListRow::new(String::new()) };
                    let (name, email) = (clean(&r.name, 200), clean(&r.email, 254));
                    let text = format!("{}. {name} ({email}), {}, {}", r.order, role_label(r.role), status_label(r.status));
                    let mut item = ListRow::new(text);
                    if editable {
                        let who = crate::i18n::fmt(tl!("Remove {who}"), &[("who", &format!("{name} ({email})"))]);
                        item = item.trailing_part(REMOVE, TrailingFace::Strong(tl!("Remove").into()), Label::from(who));
                    }
                    item
                },
            );
            for event in list.events {
                if let CollectionEvent::Item(row, RowEvent::Trailing { part, .. }) = event
                    && part == REMOVE
                    && let Some(r) = plan.recipients.get(row)
                {
                    remove = Some(r.email.clone());
                }
            }
            turn(ui, plan, &mut d.refocus, &mut d.confirm, &mut d.command);
            Text::new(tl!("History")).role(TypeRole::Heading).show(ui);
            for r in &plan.recipients {
                let steps: Vec<String> = r.history.iter().map(|e| format!("{} {}", status_label(e.status), when(e.at))).collect();
                Text::new(format!("{}: {}", clean(&r.name, 200), steps.join(", "))).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
            }
        }
    }
    if let Some(email) = remove {
        d.remove = Some(email);
        d.command = Some("sign.plan.recipient.remove");
    }
    if matches!(d.view, View::Plan(_) | View::None) {
        FieldGroup::new(tl!("Add a recipient")).show(ui, |ui, _| {
            Field::new(tl!("Recipient name:")).id_salt("plan-name").show(ui, |ui, _| {
                TextField::new(tl!("Recipient name:"), Bind::bound(&mut d.name)).enabled(editable).show(ui);
            });
            Field::new(tl!("Recipient email:")).id_salt("plan-email").show(ui, |ui, _| {
                TextField::new(tl!("Recipient email:"), Bind::bound(&mut d.email)).placeholder(tl!("name@example.org")).enabled(editable).show(ui);
            });
            let roles: Vec<Entry<usize>> = ROLES.iter().enumerate().map(|(i, r)| Entry::new(i, role_label(*r))).collect();
            let at = ROLES.iter().position(|r| *r == d.role).unwrap_or(0);
            let mut picked = None;
            Field::new(tl!("Role:")).id_salt("plan-role").show(ui, |ui, _| {
                picked = Select::new(tl!("Role:"), Bind::given(at), &roles).enabled(editable).show(ui).value().copied();
            });
            if let Some(role) = picked.and_then(|i| ROLES.get(i)) {
                d.role = *role;
            }
        });
        let mut add = Button::new(tl!("Add recipient")).enabled(editable);
        if editable && d.refocus.take_if(|r| *r == Refocus::Add).is_some() {
            add = add.request_focus(FocusRequest::Take);
        }
        if clicked(&add.show(ui)) {
            d.command = Some("sign.plan.recipient.add");
        }
        Text::new(tl!("Message")).role(TypeRole::Heading).show(ui);
        TextField::new(tl!("Message to recipients"), Bind::bound(&mut d.message)).lines(3).limit(MAX_MESSAGE_CHARS).enabled(editable).show(ui);
        if clicked(&Button::new(tl!("Set message")).enabled(editable).show(ui)) {
            d.command = Some("sign.plan.message.set");
        }
    }
    Text::new(tl!("Hand-off package")).role(TypeRole::Heading).show(ui);
    Text::new(tl!(
        "A zip of the saved PDF, this plan and a manifest, to send by mail or a shared folder. Nothing is sent by PdfCraft. The package is not signed or encrypted: its hashes only catch a file damaged or changed on the way. A package is only ever written to a new file."
    ))
    .role(TypeRole::Caption)
    .shade(Shade::Muted)
    .show(ui);
    let has_plan = matches!(d.view, View::Plan(_));
    if clicked(&Button::new(tl!("Save the hand-off package…")).enabled(has_plan).show(ui)) {
        d.command = Some("sign.plan.handoff_export");
    }
    if clicked(&Button::new(tl!("Open a hand-off package…")).show(ui)) {
        d.command = Some("sign.plan.handoff_import");
    }
    Text::new(tl!("Check")).role(TypeRole::Heading).show(ui);
    Text::new(tl!(
        "Sets the plan beside the signatures the saved file holds. It only reads: nothing is changed, signed or sent. A field's signer label is text anyone can write, so this does not show who signed."
    ))
    .role(TypeRole::Caption)
    .shade(Shade::Muted)
    .show(ui);
    if clicked(&Button::new(tl!("Check the plan")).enabled(has_plan).show(ui)) {
        d.command = Some("sign.plan.audit");
    }
    if let Some(text) = d.check.clone() {
        let lines: Vec<&str> = text.lines().collect();
        let shown = lines.iter().take(MAX_CHECK_LINES).copied().collect::<Vec<_>>().join("\n");
        SelectableText::new(tl!("Plan check"), &shown).multiline().show(ui);
        if clicked(&Button::new(tl!("Copy the check")).show(ui)) {
            d.copy_check = true;
        }
    }
    if let Some(c) = d.confirm.as_mut() {
        Notice::new(Tone::Warn, c.question.clone()).layout(NoticeLayout::Inline).announce(Politeness::Polite).id_salt(14usize).show(ui);
        let mut record = Button::new(tl!("Record it"));
        if std::mem::take(&mut c.fresh) {
            record = record.request_focus(FocusRequest::Take);
        }
        if clicked(&record.show(ui)) {
            d.expect = Some(c.email.clone());
            d.command = Some(c.command);
        }
        if clicked(&Button::new(tl!("Keep the plan as it is")).show(ui)) {
            d.confirm = None;
        }
    }
    // A live region: what the last command did or why it did not. It stays until the next command.
    if !d.result.is_empty() {
        let tone = if d.result_ok { Tone::Info } else { Tone::Bad };
        Notice::new(tone, d.result.clone()).layout(NoticeLayout::Inline).announce(Politeness::Polite).id_salt(13usize).show(ui);
    }
}

/// What `ui.state` reports of the window: whether it is open and, when it is, what it shows.
pub(crate) fn state(draft: Option<&Draft>) -> serde_json::Value {
    use serde_json::json;
    let Some(d) = draft else { return json!({ "open": false }) };
    let (kind, plan) = match &d.view {
        View::NoFile(why) => ("no_file", json!({ "why": why })),
        View::None => ("none", serde_json::Value::Null),
        View::Unreadable(why) => ("unreadable", json!({ "why": why })),
        View::Plan(p) => (
            "plan",
            json!({
                "state": p.state(),
                "started": p.started(),
                "message": p.message,
                "recipients": p.recipients.iter().map(|r| json!({
                    "order": r.order, "name": clean(&r.name, 200), "email": clean(&r.email, 254),
                    "role": format!("{:?}", r.role).to_lowercase(), "status": format!("{:?}", r.status).to_lowercase(),
                })).collect::<Vec<_>>(),
            }),
        ),
    };
    json!({ "open": true, "plan_file": kind, "plan": plan, "result": d.result, "result_ok": d.result_ok })
}
