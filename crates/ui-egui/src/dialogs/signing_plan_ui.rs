//! Sign ▸ Signing plan: the local record of who is asked to sign the open document, shown and
//! edited through registered commands.
//!
//! The commands are `sign.plan.show` (open the dialog), `sign.plan.recipient.add`,
//! `sign.plan.recipient.remove` and `sign.plan.message.set`. They apply the functions of
//! `pdfcraft_engine::signing_plan` (the same rules the `signing_plan_*` tools use) to the file
//! beside the saved PDF and write it with the app's atomic write. The dialog's buttons only ask for
//! a command; the command reads the file again, so what it changes is what is on disk, not what the
//! dialog last showed.
//!
//! It is a record only: nothing is sent, nobody is notified and no signature is collected or
//! checked. Marking a recipient signed, approved or declined, and signing as the next recipient, are
//! not offered here (the `signing_plan_advance` and `signing_plan_sign` tools do those), and once a
//! recipient has a status the plan can no longer be edited here.

use std::path::{Path, PathBuf};

use pdfcraft_engine::signing_plan::{self as plan_rules, MAX_MESSAGE_CHARS, Plan, Role, Status};
use pdfcraft_engine::{DocId, is_hidden_char};

use crate::{Dialog, PdfCraftApp, widgets};

/// The longest piece of a refusal read from a file that the dialog shows (characters).
const SHOWN_ERROR_CHARS: usize = 300;

/// What the dialog knows of the plan file, as of the last time it was read.
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

/// The Signing plan dialog's draft: what is typed and what was last read. It lives only while the
/// dialog is open and belongs to the document it was opened on.
#[derive(Clone, Debug)]
pub struct Draft {
    /// The document the dialog was opened on: the commands refuse any other.
    pub doc: DocId,
    pub name: String,
    pub email: String,
    pub role: Role,
    pub message: String,
    /// The address of the recipient a Remove button asked to take out.
    pub remove: Option<String>,
    /// The registered command a button asked for, run by the dialog after this frame.
    pub command: Option<&'static str>,
    pub view: View,
    /// What the last command did, in words (a live region).
    pub result: String,
}

/// Text from a file or a refusal as it is shown: control, hidden and direction-changing characters
/// taken out, and cut at `max` characters.
fn clean(text: &str, max: usize) -> String {
    let clean: String = text.chars().filter(|c| !is_hidden_char(*c)).collect();
    if clean.chars().count() > max { format!("{}…", clean.chars().take(max).collect::<String>()) } else { clean }
}

/// How a time (seconds since 1970, 0 when the host had no clock) is shown.
fn when(at: i64) -> String {
    match (at > 0).then(|| chrono::DateTime::from_timestamp(at, 0)).flatten() {
        Some(t) => t.format("%Y-%m-%d %H:%M UTC").to_string(),
        None => tl!("time not known").to_string(),
    }
}

fn status_label(s: Status) -> &'static str {
    match s {
        Status::Pending => tl!("Pending"),
        Status::Sent => tl!("Sent"),
        Status::Signed => tl!("Signed"),
        Status::Approved => tl!("Approved"),
        Status::Declined => tl!("Declined"),
    }
}

fn role_label(r: Role) -> &'static str {
    match r {
        Role::Signer => tl!("Signer"),
        Role::Approver => tl!("Approver"),
        Role::Viewer => tl!("Viewer"),
    }
}

fn state_label(plan: &Plan) -> &'static str {
    match plan.state() {
        "declined" => tl!("Declined: a recipient declined"),
        "completed" => tl!("Completed: everyone who signs or approves has done so"),
        _ => tl!("In progress"),
    }
}

/// The sidecar of the document at `path`, or why there is none.
fn sidecar(path: Option<&str>) -> Result<PathBuf, String> {
    let path = path
        .ok_or_else(|| tl!("The document has no saved file. Save it first: the signing plan is kept in a file beside the saved PDF.").to_string())?;
    if !Path::new(path).is_file() {
        return Err(
            tl!("The document's saved file is not there. Save it first: the signing plan is kept in a file beside the saved PDF.").to_string()
        );
    }
    plan_rules::sidecar_of(Path::new(path)).map_err(|e| e.0)
}

/// What the dialog shows for the document at `path`: the plan file read again, or why there is none.
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
}

impl PdfCraftApp {
    /// The document's saved path, or the refusal that says to save it.
    fn plan_sidecar(&self, id: DocId) -> Result<PathBuf, String> {
        sidecar(self.session.get(id).and_then(|d| d.path.as_deref()))
    }

    /// `sign.plan.show`: open the Signing plan dialog on the active document.
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
        });
        self.dialog = Some(Dialog::SigningPlan);
    }

    /// `sign.plan.recipient.add`, `sign.plan.recipient.remove` and `sign.plan.message.set`.
    ///
    /// They act only on a dialog that is open on the active document (Escape or Close discards
    /// the draft, and a draft of another document is refused), take what they need from it, read the
    /// plan file again, apply the change with the shared rules, and write the file atomically. A
    /// document with no saved path, a plan that cannot be read, a plan that has started and a change
    /// the rules refuse write nothing.
    pub fn change_signing_plan(&mut self, command: &str) {
        let Some((_, id)) = self.active_ids() else { return };
        let change = match command {
            "sign.plan.recipient.add" => Change::Add,
            "sign.plan.recipient.remove" => Change::Remove,
            _ => Change::Message,
        };
        if self.signing_plan.as_ref().is_some_and(|d| self.dialog != Some(Dialog::SigningPlan) || d.doc != id) {
            self.signing_plan = None;
        }
        let Some(draft) = self.signing_plan.as_ref() else {
            return self.notify_tr("Open the signing plan first (Sign > Signing plan…)");
        };
        let outcome = self.plan_change(id, &change, draft);
        let path = self.session.get(id).and_then(|d| d.path.clone());
        let Some(draft) = self.signing_plan.as_mut() else { return };
        draft.remove = None;
        // Read it again either way, so the dialog shows the file as it is: what was written, or what
        // made the command refuse (a plan started elsewhere, a file gone).
        draft.view = view_of(path.as_deref());
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
        };
        let bytes = plan_rules::plan_bytes(&plan).map_err(shown)?;
        let target = path.to_string_lossy().into_owned();
        crate::editing::write_atomically(&target, &bytes)
            .map_err(|e| crate::i18n::fmt(tl!("Couldn't write the signing plan: {e}"), &[("e", &clean(&e.to_string(), SHOWN_ERROR_CHARS))]))?;
        Ok(said)
    }
}

/// The dialog's content. Returns `true` when it should close.
pub(crate) fn body(ui: &mut egui::Ui, app: &mut PdfCraftApp) -> bool {
    ui.heading(tl!("Signing plan"));
    let t = crate::theme::Tokens::get(ui.ctx());
    let active = app.active_ids().map(|(_, id)| id);
    let Some(d) = app.signing_plan.as_mut().filter(|d| Some(d.doc) == active) else { return true };
    ui.label(
        egui::RichText::new(tl!(
            "A local record of who is asked to sign this document, in what order. It is kept in a file beside the PDF. It does not send the document, notify anyone or collect signatures, and it does not check any signature in the PDF. Marking a recipient as signed, approved or declined is not done here (use the signing_plan_advance tool), and once a recipient has a status the plan can no longer be edited."
        ))
        .small()
        .color(t.text_muted),
    );
    ui.add_space(6.0);
    let mut editable = false;
    match &d.view {
        View::NoFile(why) => {
            ui.label(why);
        }
        View::Unreadable(why) => {
            ui.label(tl!("The plan file beside this document cannot be used, and was left alone:"));
            ui.label(why);
        }
        View::None => {
            ui.label(tl!("There is no signing plan for this document yet. Add a recipient to make one."));
            editable = true;
        }
        View::Plan(plan) => {
            let r = ui.label(crate::i18n::fmt(tl!("State: {state}"), &[("state", state_label(plan))]));
            ui.ctx().accesskit_node_builder(r.id, |node| node.set_live(egui::accesskit::Live::Polite));
            if let Some(next) = plan.next().filter(|_| plan.state() == "in_progress") {
                ui.label(crate::i18n::fmt(tl!("Next: {who} ({email})"), &[("who", &clean(&next.name, 200)), ("email", &clean(&next.email, 254))]));
            }
            editable = !plan.started();
            if !editable {
                ui.label(
                    egui::RichText::new(tl!("A recipient already has a status, so recipients and the message can no longer be changed here."))
                        .small()
                        .color(t.text_muted),
                );
            }
            let mut remove = None;
            egui::Grid::new("signing-plan-recipients").num_columns(6).spacing([12.0, 6.0]).show(ui, |ui| {
                for head in [tl!("Order"), tl!("Name"), tl!("Email"), tl!("Role"), tl!("Status")] {
                    ui.label(egui::RichText::new(head).strong());
                }
                ui.label("");
                ui.end_row();
                for r in &plan.recipients {
                    let (name, email) = (clean(&r.name, 200), clean(&r.email, 254));
                    ui.label(r.order.to_string());
                    ui.label(&name);
                    ui.label(&email);
                    ui.label(role_label(r.role));
                    let history: Vec<String> = r.history.iter().map(|e| format!("{} ({})", status_label(e.status), when(e.at))).collect();
                    ui.label(status_label(r.status)).on_hover_text(history.join(", "));
                    let remove_button = ui.add_enabled(editable, egui::Button::new(tl!("Remove")));
                    remove_button
                        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, editable, format!("{}: {} ({})", tl!("Remove"), name, email)));
                    if remove_button.clicked() {
                        remove = Some(r.email.clone());
                    }
                    ui.end_row();
                }
            });
            if let Some(email) = remove {
                d.remove = Some(email);
                d.command = Some("sign.plan.recipient.remove");
            }
            ui.add_space(4.0);
            ui.label(egui::RichText::new(tl!("History")).strong());
            for r in &plan.recipients {
                let steps: Vec<String> = r.history.iter().map(|e| format!("{} {}", status_label(e.status), when(e.at))).collect();
                ui.label(format!("{}: {}", clean(&r.name, 200), steps.join(", ")));
            }
        }
    }
    if matches!(d.view, View::Plan(_) | View::None) {
        ui.add_space(8.0);
        widgets::section_title(ui, tl!("Add a recipient"));
        egui::Grid::new("signing-plan-add").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            let l = ui.label(tl!("Recipient name:"));
            ui.add_enabled(editable, widgets::line(&mut d.name).desired_width(300.0)).labelled_by(l.id);
            ui.end_row();
            let l = ui.label(tl!("Recipient email:"));
            ui.add_enabled(editable, widgets::line(&mut d.email).desired_width(300.0).hint_text(tl!("name@example.org"))).labelled_by(l.id);
            ui.end_row();
            ui.label(tl!("Role:"));
            ui.add_enabled_ui(editable, |ui| {
                ui.horizontal(|ui| {
                    for r in [Role::Signer, Role::Approver, Role::Viewer] {
                        widgets::radio_value(ui, &mut d.role, r, role_label(r));
                    }
                });
            });
            ui.end_row();
        });
        if ui.add_enabled(editable, egui::Button::new(tl!("Add recipient"))).clicked() {
            d.command = Some("sign.plan.recipient.add");
        }
        widgets::section_title(ui, tl!("Message"));
        let l = ui.label(tl!("Message to recipients:"));
        ui.add_enabled(
            editable,
            egui::TextEdit::multiline(&mut d.message).desired_rows(3).desired_width(f32::INFINITY).char_limit(MAX_MESSAGE_CHARS),
        )
        .labelled_by(l.id);
        if ui.add_enabled(editable, egui::Button::new(tl!("Set message"))).clicked() {
            d.command = Some("sign.plan.message.set");
        }
    }
    // A live region: what the last command did or why it did not.
    // It is there even when empty, so the first announcement is not missed.
    if !d.result.is_empty() {
        ui.add_space(6.0);
    }
    let r = ui.label(&d.result);
    ui.ctx().accesskit_node_builder(r.id, |node| node.set_live(egui::accesskit::Live::Polite));
    ui.add_space(12.0);
    let command = d.command.take();
    let close = widgets::pill_button(ui, tl!("Close"), true).clicked();
    if let Some(command) = command {
        app.execute(command);
    }
    close
}
