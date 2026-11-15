//! Print production ▸ Print readiness: what a printer would look at, measured and reported, with
//! nothing changed (the same report as the `print_readiness` tool): image resolution where each
//! image is drawn, bleed, the output intent, fonts that aren't embedded, the PDF/A result, and
//! ink (total coverage, rich black text, overprint, spot colours).

use pdfcraft_engine::printcheck::PrintReadiness;

use crate::theme::{self, Tokens};
use crate::{PdfCraftApp, widgets};

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
    let Some(report) = app.readiness_report.clone() else {
        ui.label(tl!("There is no report to show."));
        return widgets::pill_button(ui, tl!("Close"), true).clicked();
    };
    findings(ui, &report, t);
    ui.add_space(12.0);
    let mut close = false;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        close = widgets::pill_button(ui, tl!("Close"), true).clicked();
        if widgets::pill_button(ui, tl!("Copy report"), false).clicked() {
            ui.ctx().copy_text(report.text());
        }
    });
    close
}

fn findings(ui: &mut egui::Ui, report: &PrintReadiness, t: &Tokens) {
    egui::ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
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
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(tl!("Not checked: PDF/X, ink inside images and shadings, overprint effects, transparency flattening."))
                .small()
                .color(t.text_muted),
        );
    });
}
