//! The AI Assistant window: a question about the active document (Ask, Summarize, Translate), or a request to change it
//! (Edit), answered by the model Preferences names.
//!
//! The text of the pages goes to the configured server when the button is pressed, and to nowhere else. Requests run on
//! a background thread ([`crate::ai_settings`]'s `Task`), so the window never freezes. Edit asks the model for a list
//! of changes, shows it, and changes nothing until Apply is pressed; the changes are ordinary undo steps and nothing is
//! saved. The window is beside the dialog host, like the JavaScript console, so it reads the app directly.

use craft::theme::variant::{Tone, Variant};
use craft::ui::buttons::{Button, Link};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::Dialog;
use craft::ui::display::{Shade, Spinner, Text};
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::runtime::env::Label;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::style::TypeRole;
use craft::ui::text::{SelectableText, TextField};
use pdfcraft_ai::agent::{PlanRequest, PlannedCall};
use pdfcraft_engine::ai_edit;
use serde_json::Value;

use crate::PdfCraftApp;
use crate::persist::ai::page_texts;

/// What the window asks the model to do with the pages.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Mode {
    #[default]
    Ask,
    Summarize,
    Translate,
    Edit,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Ask, Mode::Summarize, Mode::Translate, Mode::Edit];

    fn label(self) -> String {
        match self {
            Mode::Ask => tl!("Ask").to_string(),
            Mode::Summarize => tl!("Summarize").to_string(),
            Mode::Translate => tl!("Translate").to_string(),
            Mode::Edit => tl!("Edit").to_string(),
        }
    }
}

/// The instruction sent for `mode`: the typed question, the fixed summary request, or the translation request for the
/// typed language. `None` while the needed text is missing (and for Edit, which sends the typed request its own way).
pub(crate) fn instruction(mode: Mode, question: &str, language: &str) -> Option<String> {
    match mode {
        Mode::Ask => Some(question.trim().to_owned()).filter(|q| !q.is_empty()),
        Mode::Summarize => Some(pdfcraft_ai::SUMMARY_QUESTION.to_owned()),
        Mode::Translate => pdfcraft_ai::translate_question(language),
        Mode::Edit => None,
    }
}

/// One change the model proposes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Step {
    pub tool: String,
    pub args: Value,
    pub why: String,
    pub summary: String,
    pub removes: bool,
}

/// What the model proposed for an Edit request; nothing in the document has changed.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EditPlan {
    pub message: String,
    pub steps: Vec<Step>,
    /// The request this answers, for the heading.
    pub request: String,
}

/// The window's own fields; the answers live with the settings (`PdfCraftApp::ai`).
#[derive(Default)]
pub struct Draft {
    pub mode: Mode,
    pub question: String,
    /// The language Translate writes in, as typed.
    pub language: String,
    pub current_page: bool,
    /// What Edit is asked to do, as typed.
    pub request: String,
    /// Preferences ▸ AI Assistant: the port a local model server listens on, as typed.
    pub port: String,
    /// What the shown answer was asked: the question and a description of the pages sent.
    asked: Option<(String, String)>,
    /// The changes Apply would make, once the model has proposed them.
    pub(crate) plan: Option<EditPlan>,
    /// How many Undo steps the last Apply made, until they are undone or another request starts.
    pub(crate) applied: Option<usize>,
    /// Why the last Apply or Undo failed.
    pub(crate) apply_error: Option<String>,
}

/// The OpenAI-compatible address of a model server on this computer, for a port the user typed
/// (`11434` gives `http://localhost:11434/v1`). `None` when it is not a port number.
pub(crate) fn local_base_url(port: &str) -> Option<String> {
    let port: u16 = port.trim().parse().ok().filter(|p| *p != 0)?;
    Some(format!("http://localhost:{port}/v1"))
}

/// The port in a `http://localhost:11434/v1` style address, to show in the port box.
pub(crate) fn port_of(base_url: &str) -> Option<String> {
    let rest = base_url.trim().strip_prefix("http://")?;
    let authority = rest.split('/').next()?;
    let (_, port) = authority.rsplit_once(':')?;
    port.parse::<u16>().ok().map(|p| p.to_string())
}

/// What Test connection says about the models the server listed.
pub(crate) fn describe_models(models: &[String], model: &str) -> String {
    const SHOWN: usize = 6;
    if models.is_empty() {
        return tl!("Connected. The server did not list any models.").to_owned();
    }
    let n = models.len().to_string();
    let mut shown = models.iter().take(SHOWN).cloned().collect::<Vec<_>>().join(", ");
    if models.len() > SHOWN {
        shown.push_str(", …");
    }
    let mut text = crate::i18n::fmt(tl!("Connected. Models: {models} ({n})"), &[("models", &shown), ("n", &n)]);
    let model = model.trim();
    if !model.is_empty() && !models.iter().any(|m| m == model) {
        text.push(' ');
        text.push_str(&crate::i18n::fmt(tl!("The server does not list \"{model}\"; check the model name."), &[("model", model)]));
    }
    text
}

/// A short description of the pages a question covers, for the answer's heading.
pub(crate) fn pages_label(current_page: Option<usize>) -> String {
    match current_page {
        Some(p) => crate::i18n::fmt(tl!("Page {n}"), &[("n", &p.saturating_add(1).to_string())]),
        None => tl!("All pages").to_owned(),
    }
}

/// What a footer or body button asks for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Run,
    Cancel,
    Copy,
    Apply,
    UndoApplied,
    Preferences,
}

impl PdfCraftApp {
    /// Opens the AI Assistant window in `mode`, or leaves the open one and switches it to `mode`.
    pub(crate) fn open_ai(&mut self, mode: Mode) {
        self.ai_draft.mode = mode;
        self.ai_open = true;
    }

    /// The command ids: `ai.ask`, `ai.summary`, `ai.translate`, `ai.edit`.
    pub(crate) fn open_ai_for(&mut self, command: &str) {
        let mode = match command {
            "ai.summary" => Mode::Summarize,
            "ai.translate" => Mode::Translate,
            "ai.edit" => Mode::Edit,
            _ => Mode::Ask,
        };
        self.open_ai(mode);
    }

    /// Test seam: shows `steps` as if the model had proposed them for `request`, with the check's own summaries.
    #[doc(hidden)]
    pub fn ai_propose_for_test(&mut self, request: &str, steps: &[(&str, Value)]) -> Result<(), String> {
        let (_, id) = self.active_ids().ok_or("no document")?;
        let shape = ai_edit::Shape::of(&self.session, id).ok_or("no document")?;
        let calls: Vec<(String, Value)> = steps.iter().map(|(t, a)| ((*t).to_owned(), a.clone())).collect();
        let summaries = ai_edit::check(&shape, &calls)?;
        let steps = calls
            .into_iter()
            .zip(summaries)
            .map(|((tool, args), summary)| Step { removes: ai_edit::removes_content(&tool), tool, args, why: String::new(), summary })
            .collect();
        self.ai_draft.plan = Some(EditPlan { message: String::new(), steps, request: request.to_owned() });
        Ok(())
    }

    /// Test seam: Apply as the window's button does. Returns the error shown, if any.
    #[doc(hidden)]
    pub fn ai_apply_for_test(&mut self, ctx: &egui::Context) -> Option<String> {
        self.apply_ai_plan(ctx);
        self.ai_draft.apply_error.clone()
    }

    /// Test seam: the window's "Undo these changes".
    #[doc(hidden)]
    pub fn ai_undo_for_test(&mut self, ctx: &egui::Context) {
        self.undo_ai_changes(ctx);
    }

    /// Asks the model for the changes `request` needs. Nothing is changed.
    fn start_edit_plan(&mut self, ctx: &egui::Context, request: String) {
        self.ai_draft.plan = None;
        self.ai_draft.applied = None;
        self.ai_draft.apply_error = None;
        let Some((_, id)) = self.active_ids() else { return };
        let Some(doc) = self.session.get(id) else { return };
        let Some(shape) = ai_edit::Shape::of(&self.session, id) else { return };
        let source = doc.export_source();
        let pages: Vec<usize> = (0..source.pages.min(200)).collect();
        let more = source.pages > pages.len();
        let config = self.ai.effective();
        let asked = request.clone();
        self.ai.plan.start(ctx, false, move || {
            let texts = page_texts(source, &pages)?;
            let context = pdfcraft_ai::document_context(&texts);
            let mut text = String::new();
            for (page, body) in &context.included {
                text.push_str(&format!("--- Page {page} ---\n{body}\n"));
            }
            if context.cut || more {
                text.push_str("[The document text was cut here to fit the size limit.]\n");
            }
            if text.trim().is_empty() {
                text.push_str("[No text could be read from the pages.]\n");
            }
            let description = format!("{} page(s)", shape.pages);
            let catalog = ai_edit::catalog();
            let req = PlanRequest { instruction: &request, document: &description, text_context: &text, catalog: &catalog };
            let check = |calls: &[PlannedCall]| ai_edit::check(&shape, &calls.iter().map(|c| (c.tool.clone(), c.args.clone())).collect::<Vec<_>>());
            let plan = pdfcraft_ai::agent::plan(&config, &req, &check).map_err(|e| e.to_string())?;
            let steps = plan
                .calls
                .into_iter()
                .zip(plan.summaries)
                .map(|(c, summary)| Step { removes: ai_edit::removes_content(&c.tool), tool: c.tool, args: c.args, why: c.why, summary })
                .collect();
            Ok(EditPlan { message: plan.message, steps, request: asked })
        });
    }

    /// Makes the changes the model proposed, one undo step each, all or none. Nothing is saved.
    pub(crate) fn apply_ai_plan(&mut self, ctx: &egui::Context) {
        let Some(plan) = self.ai_draft.plan.clone() else { return };
        self.ai_draft.apply_error = None;
        let Some((_, id)) = self.active_ids() else { return };
        let calls: Vec<(String, Value)> = plan.steps.iter().map(|s| (s.tool.clone(), s.args.clone())).collect();
        let depth = |app: &Self| app.session.get(id).map_or(0, |d| d.undo_depth());
        let start = depth(self);
        // The whole list is checked against the document as it is now, so a document that changed since the proposal
        // refuses it before anything runs.
        let checked = ai_edit::Shape::of(&self.session, id).ok_or_else(|| "the document is gone".to_owned()).and_then(|s| ai_edit::check(&s, &calls));
        if let Err(e) = checked {
            self.ai_draft.apply_error = Some(e);
            return;
        }
        for (at, (tool, args)) in calls.iter().enumerate() {
            let edits = match ai_edit::Shape::of(&self.session, id)
                .zip(self.session.get(id))
                .ok_or_else(|| "the document is gone".to_owned())
                .and_then(|(shape, doc)| ai_edit::plan_step(tool, args, &shape, Some(&doc.info)))
            {
                Ok((edits, _)) => edits,
                Err(e) => {
                    self.rollback_ai(ctx, id, start);
                    self.ai_draft.apply_error = Some(crate::i18n::fmt(
                        tl!("Step {n} could not be made: {why} The earlier steps were undone."),
                        &[("n", &(at + 1).to_string()), ("why", &e)],
                    ));
                    return;
                }
            };
            for edit in edits {
                if !self.apply_edit(ctx, edit) {
                    self.rollback_ai(ctx, id, start);
                    self.ai_draft.apply_error =
                        Some(crate::i18n::fmt(tl!("Step {n} could not be made. The earlier steps were undone."), &[("n", &(at + 1).to_string())]));
                    return;
                }
            }
        }
        let made = depth(self).saturating_sub(start);
        self.ai_draft.plan = None;
        self.ai_draft.applied = Some(made);
        self.notify_tr("The changes were made. Undo takes them back; nothing is saved yet.");
    }

    /// Undoes what Apply did.
    pub(crate) fn undo_ai_changes(&mut self, ctx: &egui::Context) {
        let Some(count) = self.ai_draft.applied.take() else { return };
        for _ in 0..count {
            self.undo(ctx);
        }
    }

    fn rollback_ai(&mut self, ctx: &egui::Context, id: pdfcraft_engine::DocId, start: usize) {
        while self.session.get(id).map_or(0, |d| d.undo_depth()) > start {
            let before = self.session.get(id).map_or(0, |d| d.undo_depth());
            self.undo(ctx);
            if self.session.get(id).map_or(0, |d| d.undo_depth()) >= before {
                break;
            }
        }
    }

    /// Sends the question to the model on a background thread.
    fn start_ai_question(&mut self, ctx: &egui::Context, question: String, heading: String, pages: Vec<usize>, label: String) {
        let Some((_, id)) = self.active_ids() else { return };
        let Some(doc) = self.session.get(id) else { return };
        let source = doc.export_source();
        self.ai_draft.asked = Some((heading, label));
        let config = self.ai.effective();
        self.ai.ask.start(ctx, false, move || {
            // The web build cannot call the endpoint, so don't read every page just to say so.
            if cfg!(target_arch = "wasm32") {
                return pdfcraft_ai::chat(&config, &[]).map(|reply| reply.content).map_err(|e| e.to_string());
            }
            let texts = page_texts(source, &pages)?;
            pdfcraft_ai::chat(&config, &pdfcraft_ai::document_messages(&question, &texts)).map(|reply| reply.content).map_err(|e| e.to_string())
        });
    }
}

/// The 1-based pages an answer cites (`(p. 20)`, `pages 3 and 5`), once each in order, for the links under it.
fn cited_pages(answer: &str, page_count: usize) -> Vec<usize> {
    use pdfcraft_ai::refs::{Segment, split_page_references};
    let mut out: Vec<usize> = Vec::new();
    for segment in split_page_references(answer, page_count) {
        if let Segment::Page(p) = segment
            && !out.contains(&p)
        {
            out.push(p);
        }
    }
    out
}

/// Shows the AI Assistant window while it is open, and carries out what its buttons asked for.
pub(crate) fn show(app: &mut PdfCraftApp, ui: &mut egui::Ui) {
    if !app.ai_open {
        return;
    }
    // Preferences takes over (opened from anywhere, not only this window's button); a question about unsaved work
    // is answered first, and the window comes back after it.
    if app.preferences.is_some() {
        app.ai_open = false;
        return;
    }
    if app.close_guard.is_some() {
        return;
    }
    let ctx = ui.ctx().clone();
    app.ai.ask.poll();
    app.ai.plan.poll();
    // A proposal the model has made is kept for review.
    if let Some(Ok(plan)) = app.ai.plan.result().cloned() {
        app.ai.plan.cancel();
        app.ai_draft.plan = Some(plan);
    }
    let mut open = true;
    let mut picked: Option<Action> = None;
    let mut mode = app.ai_draft.mode;
    let mut question = app.ai_draft.question.clone();
    let mut language = app.ai_draft.language.clone();
    let mut request = app.ai_draft.request.clone();
    let mut current_page_only = app.ai_draft.current_page;
    let active = app.active_ids();
    let page_count = active.map_or(0, |(view, _)| app.views[view].page_count());
    let mut jump: Option<usize> = None;
    {
        let app_ref: &PdfCraftApp = app;
        Dialog::new(tl!("AI Assistant")).opened(Bind::bound(&mut open)).buttons([FooterButton::new(tl!("Close"), DialogRole::Cancel)]).show(
            ui,
            |ui, _cx| {
                let entries: Vec<Entry<Mode>> = Mode::ALL.into_iter().map(|m| Entry::new(m, m.label())).collect();
                craft::ui::choice::Segmented::new(tl!("What to do"), Bind::bound(&mut mode), &entries).show(ui);
                let inputs = Inputs {
                    mode,
                    question: &mut question,
                    language: &mut language,
                    request: &mut request,
                    current_page_only: &mut current_page_only,
                    page_count,
                    picked: &mut picked,
                    jump: &mut jump,
                };
                body(ui, app_ref, inputs);
            },
        );
    }
    app.ai_draft.mode = mode;
    app.ai_draft.question = question;
    app.ai_draft.language = language;
    app.ai_draft.request = request;
    app.ai_draft.current_page = current_page_only;
    if let (Some(page), Some((view, _))) = (jump, active) {
        app.views[view].go_to_page(page.saturating_sub(1));
    }
    match picked {
        Some(Action::Run) => run(app, &ctx),
        Some(Action::Cancel) => {
            app.ai.ask.cancel();
            app.ai.plan.cancel();
        }
        Some(Action::Copy) => {
            if let Some(Ok(answer)) = app.ai.ask.result().cloned()
                && app.put_text(&answer)
            {
                app.notify_tr("Answer copied");
            }
        }
        Some(Action::Apply) => app.apply_ai_plan(&ctx),
        Some(Action::UndoApplied) => app.undo_ai_changes(&ctx),
        Some(Action::Preferences) => {
            app.ai_open = false;
            app.open_preferences_at("ai");
            return;
        }
        None => {}
    }
    if !open {
        app.ai_open = false;
        app.ai.ask.cancel();
        app.ai.plan.cancel();
    }
}

/// Starts what the main button asks for in the current mode.
fn run(app: &mut PdfCraftApp, ctx: &egui::Context) {
    let Some((view, _)) = app.active_ids() else { return };
    let current = app.views[view].current();
    let mode = app.ai_draft.mode;
    if mode == Mode::Edit {
        let request = app.ai_draft.request.trim().to_owned();
        if !request.is_empty() {
            app.start_edit_plan(ctx, request);
        }
        return;
    }
    let Some(question) = instruction(mode, &app.ai_draft.question, &app.ai_draft.language) else { return };
    let source_pages = app.views[view].page_count();
    let pages: Vec<usize> = if app.ai_draft.current_page { vec![current] } else { (0..source_pages).collect() };
    let heading = match mode {
        Mode::Summarize => tl!("Summary").to_owned(),
        Mode::Translate => format!("{} {}", tl!("Translation into"), app.ai_draft.language.trim()),
        _ => question.clone(),
    };
    let label = pages_label(app.ai_draft.current_page.then_some(current));
    app.start_ai_question(ctx, question, heading, pages, label);
}

/// What one pass of the window reads and writes besides the app.
struct Inputs<'a> {
    mode: Mode,
    question: &'a mut String,
    language: &'a mut String,
    request: &'a mut String,
    current_page_only: &'a mut bool,
    page_count: usize,
    /// The button that was pressed.
    picked: &'a mut Option<Action>,
    /// The cited page that was clicked.
    jump: &'a mut Option<usize>,
}

fn body(ui: &mut egui::Ui, app: &PdfCraftApp, inputs: Inputs<'_>) {
    let Inputs { mode, question, language, request, current_page_only, page_count, picked, jump } = inputs;
    let Some((_, id)) = app.active_ids() else {
        Text::new(tl!("Open a document to use the AI assistant.")).shade(Shade::Muted).show(ui);
        return;
    };
    let Some(doc) = app.session.get(id) else { return };
    if let Err(error) = doc.check_copying() {
        Text::new(error.to_string()).tone(Tone::Bad).show(ui);
        return;
    }
    let running = app.ai.ask.is_running() || app.ai.plan.is_running();
    let problem = app.ai.problem_with_config();
    if problem.is_some() {
        Notice::new(
            Tone::Warn,
            Label::from(tl!("The AI assistant is not set up yet. Choose a server and a model, or sign in with ChatGPT, in Preferences.").to_string()),
        )
        .layout(NoticeLayout::Inline)
        .show(ui);
        if Button::new(tl!("Open Preferences")).show(ui).clicked() {
            *picked = Some(Action::Preferences);
        }
    } else {
        let target = if app.ai.provider == pdfcraft_ai::Provider::ChatGpt {
            tl!("OpenAI (your ChatGPT plan)").to_owned()
        } else {
            app.ai.effective().base_url
        };
        let line =
            crate::i18n::fmt(tl!("When you press the button, the text of the pages goes to {url}. Nothing is sent before."), &[("url", &target)]);
        Text::new(line).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    }
    match mode {
        Mode::Ask => {
            TextField::new(tl!("Question"), Bind::bound(question)).placeholder(tl!("Ask a question about this document")).lines(3).show(ui);
        }
        Mode::Summarize => {
            Text::new(tl!("Writes a short summary with the key points and page numbers.")).show(ui);
        }
        Mode::Translate => {
            TextField::new(tl!("Language"), Bind::bound(language)).placeholder(tl!("Language, for example Spanish")).limit(60).show(ui);
        }
        Mode::Edit => {
            Text::new(tl!(
                "Say what to change, for example: rotate pages 3 to 5, or delete the blank pages. You see the changes before they are made."
            ))
            .show(ui);
            TextField::new(tl!("What should change?"), Bind::bound(request))
                .placeholder(tl!("Rotate pages 3 to 5 by 90 degrees"))
                .lines(3)
                .limit(2_000)
                .show(ui);
            if let Some(why) = &doc.read_only_reason {
                Notice::new(Tone::Warn, Label::from(why.to_string())).layout(NoticeLayout::Inline).show(ui);
            }
        }
    }
    if mode != Mode::Edit {
        Checkbox::new(Bind::bound(current_page_only)).label(tl!("Current page only").to_owned()).show(ui);
    }
    let ready = problem.is_none()
        && !running
        && match mode {
            Mode::Edit => !request.trim().is_empty() && doc.read_only_reason.is_none(),
            _ => instruction(mode, question, language).is_some(),
        };
    let run_label = match mode {
        Mode::Ask => tl!("Ask"),
        Mode::Summarize => tl!("Summarize"),
        Mode::Translate => tl!("Translate"),
        Mode::Edit => tl!("Suggest changes"),
    };
    ui.horizontal(|ui| {
        if Button::new(run_label).variant(Variant::PRIMARY).enabled(ready).show(ui).clicked() {
            *picked = Some(Action::Run);
        }
        if running {
            Spinner::new().show(ui);
            Text::new(tl!("Waiting for the answer…")).show(ui);
            if Button::new(tl!("Cancel")).show(ui).clicked() {
                *picked = Some(Action::Cancel);
            }
        }
    });
    if mode == Mode::Edit {
        edit_result(ui, app, picked);
        return;
    }
    match app.ai.ask.result() {
        Some(Ok(answer)) => {
            if let Some((heading, pages)) = &app.ai_draft.asked {
                Text::new(format!("{heading} ({pages})")).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
            }
            SelectableText::new(tl!("Answer"), answer).multiline().show(ui);
            let cited = cited_pages(answer, page_count);
            if !cited.is_empty() {
                Text::new(tl!("Pages cited")).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                ui.horizontal_wrapped(|ui| {
                    for page in cited {
                        let hint = crate::i18n::fmt(tl!("Go to page {n}"), &[("n", &page.to_string())]);
                        if Link::new(hint).show(ui).clicked() {
                            *jump = Some(page);
                        }
                    }
                });
            }
            if Button::new(tl!("Copy answer")).show(ui).clicked() {
                *picked = Some(Action::Copy);
            }
        }
        Some(Err(error)) => {
            Notice::new(Tone::Bad, Label::from(error.clone())).layout(NoticeLayout::Inline).show(ui);
        }
        None => {}
    }
}

/// The Edit tab's result: the model's message, the list of changes, and the buttons that make or undo them.
fn edit_result(ui: &mut egui::Ui, app: &PdfCraftApp, picked: &mut Option<Action>) {
    if let Some(Err(error)) = app.ai.plan.result() {
        Notice::new(Tone::Bad, Label::from(error.clone())).layout(NoticeLayout::Inline).show(ui);
    }
    if let Some(error) = &app.ai_draft.apply_error {
        Notice::new(Tone::Bad, Label::from(error.clone())).layout(NoticeLayout::Inline).show(ui);
    }
    if let Some(count) = app.ai_draft.applied {
        let line = crate::i18n::fmt(tl!("Done: {n} change(s) made. Nothing is saved yet."), &[("n", &count.to_string())]);
        Notice::new(Tone::Good, Label::from(line)).layout(NoticeLayout::Inline).show(ui);
        if Button::new(tl!("Undo these changes")).show(ui).clicked() {
            *picked = Some(Action::UndoApplied);
        }
    }
    let Some(plan) = &app.ai_draft.plan else { return };
    if !plan.message.is_empty() {
        Text::new(plan.message.clone()).show(ui);
    }
    if plan.steps.is_empty() {
        Text::new(tl!("No change is proposed.")).shade(Shade::Muted).show(ui);
        return;
    }
    Text::new(tl!("Proposed changes")).role(TypeRole::Heading).show(ui);
    for (i, step) in plan.steps.iter().enumerate() {
        let mut line = format!("{}. {}", i + 1, step.summary);
        if !step.why.is_empty() {
            line.push_str(&format!(" ({})", step.why));
        }
        Text::new(line).show(ui);
    }
    if plan.steps.iter().any(|s| s.removes) {
        Notice::new(Tone::Warn, Label::from(tl!("Some of these changes remove content. Undo takes them back until you save.").to_string()))
            .layout(NoticeLayout::Inline)
            .show(ui);
    }
    if Button::new(tl!("Make these changes")).variant(Variant::PRIMARY).show(ui).clicked() {
        *picked = Some(Action::Apply);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn test_connection_text_lists_models_and_flags_an_unknown_one() {
        let ok = describe_models(&names(&["llama3.1", "qwen2.5"]), "llama3.1");
        assert!(ok.contains("llama3.1, qwen2.5") && !ok.contains("check the model"), "{ok}");
        let wrong = describe_models(&names(&["llama3.1"]), "gpt-4o");
        assert!(wrong.contains("\"gpt-4o\"") && wrong.contains("check the model name"), "{wrong}");
        assert!(describe_models(&[], "m").contains("did not list"));
        let many: Vec<String> = (0..9).map(|i| format!("m{i}")).collect();
        let text = describe_models(&many, "");
        assert!(text.contains("m5, …") && !text.contains("m6") && text.contains("(9)"), "{text}");
    }

    #[test]
    fn a_typed_port_gives_a_local_address_and_back() {
        assert_eq!(local_base_url(" 8080 ").as_deref(), Some("http://localhost:8080/v1"));
        for bad in ["", "0", "65536", "-1", "80a", "localhost:1"] {
            assert_eq!(local_base_url(bad), None, "{bad:?}");
        }
        assert_eq!(port_of("http://localhost:11434/v1").as_deref(), Some("11434"));
        assert_eq!(port_of("http://192.168.1.5:8000"), Some("8000".into()));
        assert_eq!(port_of("https://api.openai.com/v1"), None);
        assert_eq!(port_of("http://localhost/v1"), None);
    }

    #[test]
    fn each_mode_builds_its_instruction_or_waits_for_its_text() {
        assert_eq!(instruction(Mode::Ask, "  why? ", ""), Some("why?".into()));
        assert_eq!(instruction(Mode::Ask, "  ", ""), None);
        assert!(instruction(Mode::Summarize, "", "").expect("summary").contains("Summarize"));
        assert_eq!(instruction(Mode::Translate, "", " "), None);
        assert!(instruction(Mode::Translate, "", "French").expect("translate").contains("into French"));
        assert_eq!(instruction(Mode::Edit, "x", "y"), None, "Edit sends its own request");
    }

    #[test]
    fn the_answer_heading_names_the_pages_sent() {
        assert_eq!(pages_label(None), "All pages");
        assert_eq!(pages_label(Some(2)), "Page 3");
    }

    #[test]
    fn cited_pages_are_listed_once_each_in_order_and_only_when_they_exist() {
        assert_eq!(cited_pages("See (p. 3), then pages 5 and 3 (p. 99).", 10), vec![3, 5]);
        assert!(cited_pages("No citations here.", 10).is_empty());
    }
}
