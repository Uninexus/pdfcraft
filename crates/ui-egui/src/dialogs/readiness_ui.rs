//! Print production ▸ Print readiness: what a printer would look at, measured and reported, with
//! nothing changed (the same report as the `print_readiness` tool): the findings, then what was
//! measured: the output intent, page boxes and bleed, image resolution where each image is drawn,
//! fonts that aren't embedded and the PDF/A-2b result. The measuring runs on the UI thread, as
//! the PDF/A check does.

use pdfcraft_engine::printcheck::{NOT_CHECKED, PrintReadiness};

use crate::theme::{self, Tokens};
use crate::{PdfCraftApp, widgets};

/// Detail lines shown: a document with thousands of PDF/A problems lists the first ones here and
/// the rest in Copy report.
const MAX_DETAIL_LINES: usize = 200;

impl PdfCraftApp {
    /// Measure the active document and open the report. Nothing is edited.
    pub(crate) fn open_print_readiness(&mut self) {
        let Some((_, id)) = self.active_ids() else { return };
        let Some(report) = self.session.get(id).map(|d| d.print_readiness()) else { return };
        self.readiness_report = Some(report);
        self.dialog = Some(crate::Dialog::PrintReadiness);
    }
}

fn severity_label(severity: &str) -> String {
    match severity {
        "fail" => tl!("Stop").to_string(),
        "warn" => tl!("Warning").to_string(),
        _ => tl!("Note").to_string(),
    }
}

/// The dialog. Returns `true` to close it.
pub(crate) fn body(ui: &mut egui::Ui, app: &mut PdfCraftApp, t: &Tokens) -> bool {
    ui.label(egui::RichText::new(tl!("Print readiness")).font(theme::semibold(18.0)));
    ui.label(egui::RichText::new(tl!("Measures what a printer looks at. The document is not changed.")).small().color(t.text_muted));
    ui.add_space(8.0);
    let mut copied = false;
    let mut close = false;
    match &app.readiness_report {
        None => {
            ui.label(tl!("There is no report to show."));
            close = widgets::pill_button(ui, tl!("Close"), true).clicked();
        }
        Some(report) => {
            report_body(ui, report, t);
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = widgets::pill_button(ui, tl!("Close"), true).clicked();
                if widgets::pill_button(ui, tl!("Copy report"), false).clicked() {
                    ui.ctx().copy_text(report.text());
                    copied = true;
                }
            });
        }
    }
    if copied {
        app.notify_tr("Report copied");
    }
    close
}

fn report_body(ui: &mut egui::Ui, report: &PrintReadiness, t: &Tokens) {
    egui::ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
        widgets::section_title(ui, tl!("Findings"));
        if report.findings.is_empty() {
            ui.label(tl!("No stop for image resolution, bleed or ink."));
        }
        for finding in &report.findings {
            ui.horizontal_wrapped(|ui| {
                let colour = match finding.severity {
                    "fail" => ui.visuals().error_fg_color,
                    "warn" => ui.visuals().warn_fg_color,
                    _ => t.text_muted,
                };
                ui.label(egui::RichText::new(severity_label(finding.severity)).strong().color(colour));
                ui.label(&finding.message);
            });
        }
        widgets::section_title(ui, tl!("Measured"));
        let details = report.details();
        let lines: Vec<&str> = details.lines().collect();
        for line in lines.iter().take(MAX_DETAIL_LINES) {
            ui.label(egui::RichText::new(*line).small());
        }
        if lines.len() > MAX_DETAIL_LINES {
            let more =
                crate::i18n::fmt(tl!("… and {n} more lines (Copy report has them all)"), &[("n", &(lines.len() - MAX_DETAIL_LINES).to_string())]);
            ui.label(egui::RichText::new(more).small().color(t.text_muted));
        }
        ui.add_space(8.0);
        // The engine's own sentence, the one the text report ends with.
        ui.label(egui::RichText::new(NOT_CHECKED).small().color(t.text_muted));
    });
}
