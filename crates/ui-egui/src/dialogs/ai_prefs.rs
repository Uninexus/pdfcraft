//! Preferences ▸ AI Assistant: which server answers questions about a document, and which model.
//!
//! The settings live in `ai.json` ([`crate::ai_settings`]), not in the Preferences draft, so each field is applied as it
//! is typed and saved when the keyboard leaves it. Test connection, Find models and the ChatGPT sign-in run on a
//! background thread; nothing is sent anywhere until one of them is pressed or a question is asked.

use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::choice::Segmented;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _};
use craft::ui::display::{Shade, Spinner, Text};
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::pickers::ComboBox;
use craft::ui::runtime::env::Label;
use craft::ui::style::TypeRole;
use craft::ui::text::TextField;
use pdfcraft_ai::Provider;

use super::ai_ui::{describe_models, local_base_url, port_of};
use crate::PdfCraftApp;

/// The servers the preset buttons fill in.
const PRESETS: [(&str, &str); 3] =
    [("Ollama", "http://localhost:11434/v1"), ("LM Studio", "http://localhost:1234/v1"), ("OpenAI", "https://api.openai.com/v1")];

/// The longest address, model name or key accepted in the AI settings.
const MAX_FIELD_CHARS: usize = 512;

fn caption(ui: &mut egui::Ui, text: impl Into<String>) {
    Text::new(text.into()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
}

/// The whole pane.
pub(crate) fn panel(ui: &mut egui::Ui, app: &mut PdfCraftApp) {
    caption(
        ui,
        tl!(
            "Ask questions about a document, or have it changed, with a language model you choose (File menu, AI Assistant). Nothing is sent until you press a button there. A local server such as Ollama or LM Studio keeps the text on this computer; a hosted one receives it."
        ),
    );
    app.ai.test.poll();
    let entries = [
        Entry::new(Provider::Endpoint, tl!("An API endpoint (local model or API key)").to_string()),
        #[cfg(not(target_arch = "wasm32"))]
        Entry::new(Provider::ChatGpt, tl!("My ChatGPT plan").to_string()),
    ];
    let mut provider = app.ai.provider;
    Segmented::new(tl!("Use"), Bind::bound(&mut provider), &entries).show(ui);
    if provider != app.ai.provider {
        app.ai.provider = provider;
        app.ai.test.cancel();
    }
    if app.ai.provider == Provider::ChatGpt {
        #[cfg(not(target_arch = "wasm32"))]
        chatgpt(ui, app);
    } else {
        endpoint(ui, app);
    }
    model(ui, app);
    if app.ai.provider == Provider::Endpoint {
        TextField::new(tl!("API key"), Bind::bound(&mut app.ai.api_key)).masked().limit(MAX_FIELD_CHARS).show(ui);
        caption(
            ui,
            tl!(
                "A local server usually needs no key. Empty fields use the PDFCRAFT_AI_BASE_URL, PDFCRAFT_AI_MODEL and PDFCRAFT_AI_API_KEY environment variables."
            ),
        );
    }
    ui.horizontal(|ui| {
        let running = app.ai.test.is_running();
        if Button::new(tl!("Test connection")).enabled(!running).show(ui).clicked() {
            start_models(ui, app);
        }
        if running {
            Spinner::new().show(ui);
        }
    });
    match app.ai.test.result() {
        Some(Ok(models)) => {
            Text::new(describe_models(models, &app.ai.effective().model)).show(ui);
        }
        Some(Err(error)) => {
            Notice::new(Tone::Bad, Label::from(error.clone())).layout(NoticeLayout::Inline).show(ui);
        }
        None => {}
    }
    // Not on every keystroke: a disk write per key would stall typing on a slow drive.
    if !ui.ctx().memory(|m| m.focused().is_some()) {
        app.ai.save_if_changed();
    }
    if let Some(problem) = &app.ai.problem {
        Notice::new(Tone::Bad, Label::from(problem.clone())).layout(NoticeLayout::Inline).show(ui);
    }
}

fn start_models(ui: &egui::Ui, app: &mut PdfCraftApp) {
    let config = app.ai.effective();
    app.ai.test.start(ui.ctx(), false, move || pdfcraft_ai::list_models(&config).map_err(|e| e.to_string()));
}

/// The server's address and, for one on this computer, its port.
fn endpoint(ui: &mut egui::Ui, app: &mut PdfCraftApp) {
    ui.horizontal_wrapped(|ui| {
        for (name, url) in PRESETS {
            if Button::new(name).show(ui).clicked() {
                app.ai.base_url = url.to_owned();
            }
        }
    });
    TextField::new(tl!("Base URL"), Bind::bound(&mut app.ai.base_url)).placeholder("http://localhost:11434/v1").limit(MAX_FIELD_CHARS).show(ui);
    // A model server on this computer: the port, then the models it has loaded to pick from.
    if app.ai_draft.port.is_empty()
        && let Some(port) = port_of(&app.ai.base_url)
    {
        app.ai_draft.port = port;
    }
    TextField::new(tl!("Local server port"), Bind::bound(&mut app.ai_draft.port)).placeholder("11434").limit(5).show(ui);
    let url = local_base_url(&app.ai_draft.port);
    let running = app.ai.test.is_running();
    ui.horizontal(|ui| {
        if Button::new(tl!("Find models")).enabled(url.is_some() && !running).show(ui).clicked()
            && let Some(url) = url
        {
            app.ai.base_url = url;
            start_models(ui, app);
        }
        if running {
            Spinner::new().show(ui);
        }
    });
}

/// The model: a pick from what the server listed, else a typed name.
fn model(ui: &mut egui::Ui, app: &mut PdfCraftApp) {
    let found: Vec<String> = match app.ai.test.result() {
        Some(Ok(models)) => models.clone(),
        _ => Vec::new(),
    };
    if found.is_empty() {
        TextField::new(tl!("Model"), Bind::bound(&mut app.ai.model)).limit(MAX_FIELD_CHARS).show(ui);
        if app.ai.provider == Provider::Endpoint {
            caption(ui, tl!("Type a port and press Find models to pick from what the server has loaded, or type a model name."));
        }
        return;
    }
    let options: Vec<Entry<String>> = found.iter().map(|m| Entry::new(m.clone(), m.clone())).collect();
    let mut chosen = app.ai.model.clone();
    ComboBox::new(tl!("Model"), Bind::bound(&mut chosen), &options).placeholder(tl!("Choose a model")).show(ui);
    if chosen != app.ai.model {
        app.ai.model = chosen;
    }
}

/// The "My ChatGPT plan" settings: sign in with the system browser, no API key and no API billing.
#[cfg(not(target_arch = "wasm32"))]
fn chatgpt(ui: &mut egui::Ui, app: &mut PdfCraftApp) {
    caption(
        ui,
        tl!(
            "Uses the ChatGPT Plus or Pro plan you already pay for, through OpenAI's Sign in with ChatGPT. No API key and no API billing. PdfCraft opens your browser to sign in and keeps the sign-in on this computer."
        ),
    );
    app.ai.sign_in.poll();
    if let Some(done) = app.ai.sign_in.result().cloned() {
        app.ai.sign_in.cancel();
        app.ai.sign_in_error = done.err();
        app.ai.reload_chatgpt();
        // The model list belongs to the account that just signed in (or out).
        app.ai.test.cancel();
    }
    let signing_in = app.ai.sign_in.is_running();
    let status = app.ai.chatgpt.clone();
    ui.horizontal_wrapped(|ui| {
        if signing_in {
            Spinner::new().show(ui);
            Text::new(tl!("Finish signing in in your browser…")).show(ui);
            if Button::new(tl!("Cancel")).show(ui).clicked() {
                app.ai.cancel_chatgpt_sign_in();
            }
        } else if status.signed_in && status.plan_usage {
            let who = if status.email.is_empty() {
                tl!("Signed in").to_owned()
            } else {
                crate::i18n::fmt(tl!("Signed in as {email}"), &[("email", &status.email)])
            };
            Text::new(format!("{who}. {}", tl!("Using ChatGPT plan"))).show(ui);
            if Button::new(tl!("Sign out")).show(ui).clicked() {
                app.ai.chatgpt_sign_out();
            }
            if Button::new(tl!("Manage usage")).show(ui).clicked() {
                app.open_url(pdfcraft_ai::chatgpt::USAGE_URL);
            }
        } else {
            if Button::new(tl!("Sign in with ChatGPT")).show(ui).clicked() {
                app.ai.start_chatgpt_sign_in(ui.ctx(), false);
            }
            if status.signed_in {
                Text::new(tl!("Signed in, but plan usage was not allowed. Sign in again and allow it.")).show(ui);
            }
        }
    });
    if let Some(error) = &app.ai.sign_in_error {
        Notice::new(Tone::Bad, Label::from(error.clone())).layout(NoticeLayout::Inline).show(ui);
    }
    caption(ui, tl!("Your plan's usage limits apply. If a limit is reached, PdfCraft says so and does not switch to API billing."));
    // The model list comes from the account, once signed in.
    if status.signed_in && !signing_in && app.ai.test.result().is_none() && !app.ai.test.is_running() {
        start_models(ui, app);
    }
}
