//! Print production ▸ Print readiness: what a printer would look at, measured and reported, with nothing changed (the
//! same report as the `print_readiness` tool): the findings, then what was measured (the output intent, page boxes and
//! bleed, image resolution where each image is drawn, fonts that aren't embedded and the PDF/A-2b result). The
//! measuring runs on the UI thread, as the PDF/A check does, once when the dialog opens.

use craft::theme::variant::Tone;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::Dialog;
use craft::ui::display::{Shade, Text};
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::style::TypeRole;
use craft::ui::text::SelectableText;
use pdfcraft_engine::printcheck::{NOT_CHECKED, PrintReadiness};

use crate::PdfCraftApp;

/// Detail lines shown: a document with thousands of PDF/A problems lists the first ones here and the rest in Copy
/// report.
const MAX_DETAIL_LINES: usize = 200;

/// What a footer button asks for.
#[derive(Clone, Copy)]
enum Action {
    Copy,
    Close,
}

impl PdfCraftApp {
    /// Measures the active document and opens the report. Nothing is edited.
    pub(crate) fn open_print_readiness(&mut self) {
        match self.measure_print_readiness() {
            Ok(true) => self.readiness_open = true,
            Ok(false) => {}
            Err(e) => self.notify_error(e),
        }
    }

    /// Measures the active document and keeps the report; `Ok(false)` when there is no document.
    fn measure_print_readiness(&mut self) -> Result<bool, String> {
        let Some((_, id)) = self.active_ids() else { return Ok(false) };
        let Some(doc) = self.session.get(id) else { return Ok(false) };
        self.readiness_report = Some(doc.print_readiness());
        Ok(true)
    }

    /// The report as plain text, for Copy report.
    pub fn readiness_report_text(&self) -> Option<String> {
        self.readiness_report.as_ref().map(PrintReadiness::text)
    }

    /// Closes the report and drops it.
    fn close_print_readiness(&mut self) {
        self.readiness_open = false;
        self.readiness_report = None;
    }
}

fn severity_label(severity: &str) -> String {
    match severity {
        "fail" => tl!("Stop").to_string(),
        "warn" => tl!("Warning").to_string(),
        _ => tl!("Note").to_string(),
    }
}

/// Shows the Print readiness dialog while it is open, and carries out the button it was answered with. It closes
/// with the document it measured.
pub(crate) fn show(app: &mut PdfCraftApp, ui: &mut egui::Ui) {
    if !app.readiness_open {
        return;
    }
    if app.active_ids().is_none() {
        app.close_print_readiness();
        return;
    }
    let Some(report) = app.readiness_report.take() else {
        app.readiness_open = false;
        return;
    };
    let actions = [
        (Action::Copy, FooterButton::new(tl!("Copy report"), DialogRole::Other).stays_open(true)),
        (Action::Close, FooterButton::new(tl!("Close"), DialogRole::Cancel)),
    ];
    let buttons: Vec<FooterButton> = actions.iter().map(|(_, b)| b.clone()).collect();
    let mut open = true;
    let shown = Dialog::new(tl!("Print readiness")).opened(Bind::bound(&mut open)).buttons(buttons).show(ui, |ui, _cx| {
        report_body(ui, &report);
    });
    let picked = shown.output.picked.and_then(|at| actions.get(at)).map(|(a, _)| *a);
    if matches!(picked, Some(Action::Copy)) && app.put_text(&report.text()) {
        app.notify_tr("Report copied");
    }
    if open {
        app.readiness_report = Some(report);
    } else {
        app.close_print_readiness();
    }
}

fn report_body(ui: &mut egui::Ui, report: &PrintReadiness) {
    Text::new(tl!("Measures what a printer looks at. The document is not changed.")).shade(Shade::Muted).show(ui);
    Text::new(tl!("Findings")).role(TypeRole::Heading).show(ui);
    if report.findings.is_empty() {
        Text::new(tl!("No stop for image resolution, bleed or ink.")).show(ui);
    }
    for (at, finding) in report.findings.iter().enumerate() {
        let tone = match finding.severity {
            "fail" => Tone::Bad,
            "warn" => Tone::Warn,
            _ => Tone::Info,
        };
        // The severity is a word as well as a colour and an icon.
        let text = format!("{}: {}", severity_label(finding.severity), finding.message);
        Notice::new(tone, text).layout(NoticeLayout::Inline).id_salt(at).show(ui);
    }
    Text::new(tl!("Measured")).role(TypeRole::Heading).show(ui);
    let details = report.details();
    let lines: Vec<&str> = details.lines().collect();
    let shown = lines.iter().take(MAX_DETAIL_LINES).copied().collect::<Vec<_>>().join("\n");
    SelectableText::new(tl!("Measured"), &shown).multiline().show(ui);
    if lines.len() > MAX_DETAIL_LINES {
        let more = crate::i18n::fmt(tl!("… and {n} more lines (Copy report has them all)"), &[("n", &(lines.len() - MAX_DETAIL_LINES).to_string())]);
        Text::new(more).shade(Shade::Muted).show(ui);
    }
    // The engine's own sentence, the one the text report ends with.
    Text::new(NOT_CHECKED).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
}
