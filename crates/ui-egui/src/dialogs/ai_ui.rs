//! Ask AI Assistant: a question about the active document, answered by the model Preferences names.
//!
//! The text of the pages (all, or the current one) goes to the configured OpenAI-compatible server
//! when Ask is pressed, and to nowhere else. The request runs on a background thread
//! ([`crate::ai_settings`]'s `Task`), so the window never freezes.

use crate::persist::ai::page_texts;
use crate::{Dialog, PdfCraftApp, widgets};

/// What the dialog asks the model to do with the pages.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) enum Mode {
    #[default]
    Ask,
    Summarize,
    Translate,
}

/// The instruction sent for `mode`: the typed question, the fixed summary request, or the
/// translation request for the typed language. `None` while the needed text is missing.
pub(crate) fn instruction(mode: Mode, question: &str, language: &str) -> Option<String> {
    match mode {
        Mode::Ask => Some(question.trim().to_owned()).filter(|q| !q.is_empty()),
        Mode::Summarize => Some(pdfcraft_ai::SUMMARY_QUESTION.to_owned()),
        Mode::Translate => pdfcraft_ai::translate_question(language),
    }
}

/// The Ask dialog's own fields; the answer lives with the settings (`PdfCraftApp::ai`).
#[derive(Default)]
pub(crate) struct Draft {
    pub(crate) mode: Mode,
    pub(crate) question: String,
    /// The language Translate writes in, as typed.
    pub(crate) language: String,
    pub(crate) current_page: bool,
    /// Preferences ▸ AI Assistant: the port a local model server listens on, as typed.
    pub(crate) port: String,
    /// What the shown answer was asked: the question and a description of the pages sent.
    asked: Option<(String, String)>,
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

/// The dialog's content. Returns `true` to close the dialog.
pub(crate) fn body(ui: &mut egui::Ui, app: &mut PdfCraftApp) -> bool {
    ui.heading(tl!("AI Assistant"));
    ui.horizontal(|ui| {
        ui.selectable_value(&mut app.ai_draft.mode, Mode::Ask, tl!("Ask"));
        ui.selectable_value(&mut app.ai_draft.mode, Mode::Summarize, tl!("Summarize"));
        ui.selectable_value(&mut app.ai_draft.mode, Mode::Translate, tl!("Translate"));
    });
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
        ui.label(tl!("The AI assistant is not set up yet. Choose a server and a model, or sign in with ChatGPT, in Preferences."));
        if widgets::pill_button(ui, tl!("Open Preferences"), true).clicked() {
            app.dialog = Some(Dialog::Preferences);
            return false;
        }
        ui.add_space(8.0);
    } else {
        let target = if app.ai.provider == pdfcraft_ai::Provider::ChatGpt { tl!("OpenAI (your ChatGPT plan)").to_owned() } else { url };
        ui.label(crate::i18n::fmt(
            tl!("When you press the button, the text of the pages goes to {url}. Nothing is sent before."),
            &[("url", &target)],
        ));
    }
    ui.add_space(6.0);
    match app.ai_draft.mode {
        Mode::Ask => {
            ui.add(
                widgets::block(&mut app.ai_draft.question)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text(tl!("Ask a question about this document")),
            );
        }
        Mode::Summarize => {
            ui.label(tl!("Writes a short summary with the key points and page numbers."));
        }
        Mode::Translate => {
            ui.add(widgets::line(&mut app.ai_draft.language).desired_width(240.0).hint_text(tl!("Language, for example Spanish")).char_limit(60));
        }
    }
    ui.add(widgets::checkbox(&mut app.ai_draft.current_page, tl!("Current page only")));
    ui.add_space(6.0);
    let mut ask = false;
    ui.horizontal(|ui| {
        let ready = problem.is_none() && !running && instruction(app.ai_draft.mode, &app.ai_draft.question, &app.ai_draft.language).is_some();
        let label = match app.ai_draft.mode {
            Mode::Ask => tl!("Ask"),
            Mode::Summarize => tl!("Summarize"),
            Mode::Translate => tl!("Translate"),
        };
        ask = ui.add_enabled_ui(ready, |ui| widgets::pill_button(ui, label, true)).inner.clicked();
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
        let question = instruction(app.ai_draft.mode, &app.ai_draft.question, &app.ai_draft.language).unwrap_or_default();
        let heading = match app.ai_draft.mode {
            Mode::Ask => question.clone(),
            Mode::Summarize => tl!("Summary").to_owned(),
            Mode::Translate => format!("{} {}", tl!("Translation into"), app.ai_draft.language.trim()),
        };
        app.ai_draft.asked = Some((heading, pages_label(app.ai_draft.current_page.then_some(current))));
        let config = app.ai.effective();
        app.ai.ask.start(ui.ctx(), app.run_inline, move || {
            // The web build cannot call the endpoint, so don't read every page just to say so.
            if cfg!(target_arch = "wasm32") {
                return pdfcraft_ai::chat(&config, &[]).map(|reply| reply.content).map_err(|e| e.to_string());
            }
            let texts = page_texts(source, &pages)?;
            pdfcraft_ai::chat(&config, &pdfcraft_ai::document_messages(&question, &texts)).map(|reply| reply.content).map_err(|e| e.to_string())
        });
    }
    let page_count = app.views[view].page_count();
    let mut jump = None;
    match app.ai.ask.result() {
        Some(Ok(answer)) => {
            ui.separator();
            if let Some((question, pages)) = &app.ai_draft.asked {
                ui.label(egui::RichText::new(format!("{question} ({pages})")).weak());
            }
            egui::ScrollArea::vertical().id_salt("ai-answer").max_height(280.0).auto_shrink([false, true]).show(ui, |ui| {
                jump = answer_with_links(ui, answer, page_count);
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
    if let Some(page) = jump {
        app.views[view].go_to_page(page.saturating_sub(1));
    }
    false
}

/// The answer's text, with each page it cites ("(p. 20)", "pages 3 and 5") as a link. Returns the
/// 1-based page that was clicked. Without citations it is one selectable label.
fn answer_with_links(ui: &mut egui::Ui, answer: &str, page_count: usize) -> Option<usize> {
    use pdfcraft_ai::refs::{Segment, split_page_references};
    let segments = split_page_references(answer, page_count);
    if !segments.iter().any(|s| matches!(s, Segment::Page(_))) {
        ui.add(egui::Label::new(answer).selectable(true));
        return None;
    }
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for segment in &segments {
            match segment {
                Segment::Text(text) => {
                    ui.add(egui::Label::new(text.as_str()).selectable(true));
                }
                Segment::Page(page) => {
                    let hint = crate::i18n::fmt(tl!("Go to page {n}"), &[("n", &page.to_string())]);
                    if ui.link(page.to_string()).on_hover_text(hint).clicked() {
                        clicked = Some(*page);
                    }
                }
            }
        }
    });
    clicked
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
    }

    #[test]
    fn the_answer_heading_names_the_pages_sent() {
        assert_eq!(pages_label(None), "All pages");
        assert_eq!(pages_label(Some(2)), "Page 3");
    }
}
