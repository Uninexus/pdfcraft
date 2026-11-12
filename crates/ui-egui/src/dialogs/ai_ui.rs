//! Ask AI Assistant: a question about the active document, answered by the model Preferences names.
//!
//! The text of the pages (all, or the current one) goes to the configured OpenAI-compatible server
//! when Ask is pressed, and to nowhere else. The request runs on a background thread
//! ([`crate::ai_settings`]'s `Task`), so the window never freezes.

use crate::persist::ai::page_texts;
use crate::{Dialog, PdfCraftApp, widgets};

/// The Ask dialog's own fields; the answer lives with the settings (`PdfCraftApp::ai`).
#[derive(Default)]
pub(crate) struct Draft {
    pub(crate) question: String,
    pub(crate) current_page: bool,
    /// What the shown answer was asked: the question and a description of the pages sent.
    asked: Option<(String, String)>,
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

/// The dialog's content. Returns `true` to close the dialog.
pub(crate) fn body(ui: &mut egui::Ui, app: &mut PdfCraftApp) -> bool {
    ui.heading(tl!("Ask AI Assistant"));
    let Some((view, id)) = app.active_ids() else {
        ui.label(tl!("Open a document to ask about it."));
        return false;
    };
    let Some(doc) = app.session.get(id) else { return true };
    if let Err(error) = doc.check_copying() {
        ui.label(error.to_string());
        return false;
    }
    app.ai.ask.poll();
    let running = app.ai.ask.is_running();
    let problem = app.ai.problem_with_config();
    let current = app.views[view].current;
    let url = app.ai.effective().base_url;

    if problem.is_some() {
        ui.add_space(4.0);
        ui.label(tl!("The AI assistant is not set up yet. Choose a server and a model in Preferences."));
        if widgets::pill_button(ui, tl!("Open Preferences"), true).clicked() {
            app.dialog = Some(Dialog::Preferences);
            return false;
        }
        ui.add_space(8.0);
    } else {
        ui.label(crate::i18n::fmt(tl!("When you press Ask, the text of the pages goes to {url}. Nothing is sent before."), &[("url", &url)]));
    }
    ui.add_space(6.0);
    ui.add(
        widgets::block(&mut app.ai_draft.question).desired_rows(3).desired_width(f32::INFINITY).hint_text(tl!("Ask a question about this document")),
    );
    ui.add(widgets::checkbox(&mut app.ai_draft.current_page, tl!("Current page only")));
    ui.add_space(6.0);
    let mut ask = false;
    ui.horizontal(|ui| {
        let ready = problem.is_none() && !running && !app.ai_draft.question.trim().is_empty();
        ask = ui.add_enabled_ui(ready, |ui| widgets::pill_button(ui, tl!("Ask"), true)).inner.clicked();
        if running {
            ui.spinner();
            ui.label(tl!("Waiting for the answer…"));
            if ui.button(tl!("Cancel")).clicked() {
                app.ai.ask.cancel();
            }
        }
    });
    if ask {
        let source = doc.export_source();
        let pages: Vec<usize> = if app.ai_draft.current_page { vec![current] } else { (0..source.pages).collect() };
        let question = app.ai_draft.question.trim().to_owned();
        app.ai_draft.asked = Some((question.clone(), pages_label(app.ai_draft.current_page.then_some(current))));
        let config = app.ai.effective();
        app.ai.ask.start(ui.ctx(), app.run_inline, move || {
            let texts = page_texts(source, &pages)?;
            pdfcraft_ai::chat(&config, &pdfcraft_ai::document_messages(&question, &texts)).map(|reply| reply.content).map_err(|e| e.to_string())
        });
    }
    match app.ai.ask.result() {
        Some(Ok(answer)) => {
            ui.separator();
            if let Some((question, pages)) = &app.ai_draft.asked {
                ui.label(egui::RichText::new(format!("{question} ({pages})")).weak());
            }
            egui::ScrollArea::vertical().id_salt("ai-answer").max_height(280.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.add(egui::Label::new(answer.as_str()).selectable(true));
            });
            if ui.button(tl!("Copy answer")).clicked() {
                ui.ctx().copy_text(answer.clone());
            }
        }
        Some(Err(error)) => {
            ui.separator();
            ui.colored_label(ui.visuals().error_fg_color, error.as_str());
        }
        None => {}
    }
    false
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
    fn the_answer_heading_names_the_pages_sent() {
        assert_eq!(pages_label(None), "All pages");
        assert_eq!(pages_label(Some(2)), "Page 3");
    }
}
