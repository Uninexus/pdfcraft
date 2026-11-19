//! Print production ▸ Print readiness: what a printer would look at, measured and reported, with
//! nothing changed (the same report as the `print_readiness` tool): the findings, then what was
//! measured: the output intent, page boxes and bleed, image resolution where each image is drawn,
//! fonts that aren't embedded and the PDF/A-2b result. The measuring runs on the UI thread, as
//! the PDF/A check does.

use pdfcraft_engine::printcheck::{
    MIN_BLEED_MM, MIN_BLEED_RANGE, MIN_PPI_RANGE, NOT_CHECKED, PrintReadiness, TAC_LIMIT_PERCENT, TAC_LIMIT_RANGE, WARN_PPI,
};

use crate::theme::{self, Tokens};
use crate::{PdfCraftApp, widgets};

/// Detail lines shown: a document with thousands of PDF/A problems lists the first ones here and
/// the rest in Copy report.
const MAX_DETAIL_LINES: usize = 200;

/// Room the dialog needs besides the report's scroll area (title, the three threshold rows, the
/// Close row and the window's padding), and the least the report may be squeezed to. On a short
/// window the report scrolls and Close / Copy report stay in reach, as in the OCR dialog.
const READINESS_RESERVE: f32 = 400.0;
const READINESS_MIN_SCROLL: f32 = 40.0;
const READINESS_MAX_SCROLL: f32 = 420.0;

/// The three thresholds of the `print_readiness` tool (`ink_limit`, `min_bleed_mm`, `min_ppi`).
/// `None` is the engine's own default (300 %, 2 mm, 300 ppi, with the fixed wording of its
/// messages), so a user who changes nothing sees exactly what the tool gives without options.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReadinessOptions {
    pub ink_limit: Option<f64>,
    pub min_bleed_mm: Option<f64>,
    pub min_ppi: Option<f64>,
}

/// One of the three thresholds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadinessField {
    InkLimit,
    MinBleed,
    MinPpi,
}

impl ReadinessField {
    pub const ALL: [Self; 3] = [Self::InkLimit, Self::MinBleed, Self::MinPpi];

    /// The `ui.set` key (and `ui.state` name, with dashes as underscores).
    pub fn key(self) -> &'static str {
        match self {
            Self::InkLimit => "readiness-ink-limit",
            Self::MinBleed => "readiness-min-bleed-mm",
            Self::MinPpi => "readiness-min-ppi",
        }
    }

    /// The engine's range for it, so the dialog, the settings and the tool refuse the same values.
    fn range(self) -> &'static std::ops::RangeInclusive<f64> {
        match self {
            Self::InkLimit => &TAC_LIMIT_RANGE,
            Self::MinBleed => &MIN_BLEED_RANGE,
            Self::MinPpi => &MIN_PPI_RANGE,
        }
    }

    /// Whether `n` is a number the engine accepts (NaN and infinities are not).
    pub fn accepts(self, n: f64) -> bool {
        self.range().contains(&n)
    }

    /// Typed text to a setting: empty or `default` is the default (`None`), a number in range is
    /// that number (a comma may be the decimal mark and full-width digits are read as digits;
    /// `-0` is 0), anything else (text, NaN, infinity, out of range) is refused with the reason.
    pub fn parse(self, text: &str) -> Result<Option<f64>, String> {
        let text = normalise_number(text);
        if text.is_empty() || text.eq_ignore_ascii_case("default") {
            return Ok(None);
        }
        match text.parse::<f64>() {
            // `-0` is zero: it would otherwise be shown and saved as -0.
            Ok(n) if self.accepts(n) => Ok(Some(if n == 0.0 { 0.0 } else { n })),
            _ => Err(self.refusal()),
        }
    }

    /// The same refusal for the control channel: English, naming the key and the way back.
    pub fn key_error(self) -> String {
        format!("{} must be a number from {} to {}, or default", self.key(), self.range().start(), self.range().end())
    }

    /// The engine default, shown as the field's hint.
    fn default_value(self) -> f64 {
        match self {
            Self::InkLimit => TAC_LIMIT_PERCENT,
            Self::MinBleed => MIN_BLEED_MM,
            Self::MinPpi => WARN_PPI,
        }
    }

    fn refusal(self) -> String {
        let (lo, hi) = (self.range().start().to_string(), self.range().end().to_string());
        let template = match self {
            Self::InkLimit => tl!("Total ink limit must be a number from {min} to {max} percent (digits 0-9)."),
            Self::MinBleed => tl!("Least bleed must be a number from {min} to {max} millimetres (digits 0-9)."),
            Self::MinPpi => tl!("Least resolution must be a number from {min} to {max} pixels per inch (digits 0-9)."),
        };
        crate::i18n::fmt(template, &[("min", &lo), ("max", &hi)])
    }
}

/// Trim, take full-width digits and separators as their ASCII forms and a comma as the decimal
/// mark (as the print and measure dialogs do).
fn normalise_number(text: &str) -> String {
    text.trim()
        .chars()
        .map(|c| match c {
            '\u{FF10}'..='\u{FF19}' => char::from(b'0' + (c as u32 - 0xFF10) as u8),
            ',' | '\u{FF0C}' | '\u{FF0E}' => '.',
            '\u{FF0D}' | '\u{2212}' => '-',
            c => c,
        })
        .collect()
}

impl ReadinessOptions {
    pub fn get(&self, field: ReadinessField) -> Option<f64> {
        match field {
            ReadinessField::InkLimit => self.ink_limit,
            ReadinessField::MinBleed => self.min_bleed_mm,
            ReadinessField::MinPpi => self.min_ppi,
        }
    }

    pub fn set(&mut self, field: ReadinessField, value: Option<f64>) {
        match field {
            ReadinessField::InkLimit => self.ink_limit = value,
            ReadinessField::MinBleed => self.min_bleed_mm = value,
            ReadinessField::MinPpi => self.min_ppi = value,
        }
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// What the three fields currently say, as typed (a field the user is half way through typing is
/// not a setting yet).
#[derive(Clone, Debug, Default)]
pub struct ReadinessEdit {
    pub text: [String; 3],
    /// Why a field's text was refused when it was applied (Enter or leaving the field); cleared
    /// as soon as the text changes. Shown on the field's own line, which is always there.
    pub refused: [Option<String>; 3],
}

impl ReadinessEdit {
    fn from_options(options: &ReadinessOptions) -> Self {
        let show = |v: Option<f64>| v.map(|n| n.to_string()).unwrap_or_default();
        Self { text: [show(options.ink_limit), show(options.min_bleed_mm), show(options.min_ppi)], refused: Default::default() }
    }
}

impl PdfCraftApp {
    /// Measure the active document with the remembered thresholds and open the report. Nothing is edited.
    pub(crate) fn open_print_readiness(&mut self) {
        self.readiness_edit = ReadinessEdit::from_options(&self.readiness_options);
        match self.measure_print_readiness() {
            Ok(true) => self.dialog = Some(crate::Dialog::PrintReadiness),
            Ok(false) => {}
            Err(e) => self.notify_error(e),
        }
    }

    /// Measure the active document with `readiness_options` and keep the report; the one place the
    /// dialog's report is made, so the findings, the measured lines and Copy report all follow them.
    /// `Ok(false)`: there is no document. The measuring runs on the UI thread, so it is done when
    /// a threshold is applied (Enter, leaving the field, `ui.set`, Reset), not on every keystroke.
    fn measure_print_readiness(&mut self) -> Result<bool, String> {
        let Some((_, id)) = self.active_ids() else { return Ok(false) };
        let o = self.readiness_options;
        let Some(doc) = self.session.get(id) else { return Ok(false) };
        self.readiness_report = Some(doc.print_readiness_with_options(o.ink_limit, o.min_bleed_mm, o.min_ppi)?);
        Ok(true)
    }

    /// Use `value` for `field` and measure an open report again. Leaves the typed text alone. If the
    /// engine refuses it (not expected after the range check) the old value is kept and the error returned.
    fn apply_readiness_option(&mut self, field: ReadinessField, value: Option<f64>) -> Result<(), String> {
        if let Some(n) = value
            && !field.accepts(n)
        {
            return Err(field.refusal());
        }
        let before = self.readiness_options;
        if before.get(field) == value {
            return Ok(());
        }
        self.readiness_options.set(field, value);
        if self.dialog == Some(crate::Dialog::PrintReadiness)
            && let Err(e) = self.measure_print_readiness()
        {
            self.readiness_options = before;
            return Err(e);
        }
        Ok(())
    }

    /// Set one threshold (`ui.set`, Reset and the settings end here). A refused value changes
    /// nothing. An open report is measured again, and the field's text shows the setting again.
    pub fn set_readiness_option(&mut self, field: ReadinessField, value: Option<f64>) -> Result<(), String> {
        self.apply_readiness_option(field, value)?;
        self.readiness_edit = ReadinessEdit::from_options(&self.readiness_options);
        Ok(())
    }

    /// The field's text changed (a keystroke): remember it and forget an old refusal. Nothing is
    /// measured until it is applied.
    pub fn type_readiness_field(&mut self, field: ReadinessField, text: &str) {
        if let Some(slot) = self.readiness_edit.text.get_mut(field as usize) {
            *slot = text.to_string();
        }
        if let Some(slot) = self.readiness_edit.refused.get_mut(field as usize) {
            *slot = None;
        }
    }

    /// Apply the field's typed text (Enter, or the field lost the keyboard): a valid value is used
    /// and the report measured again; an invalid one stays in the field with its reason and changes nothing.
    pub fn apply_readiness_field(&mut self, field: ReadinessField) -> Result<(), String> {
        let text = self.readiness_edit.text.get(field as usize).cloned().unwrap_or_default();
        let result = field.parse(&text).and_then(|value| self.apply_readiness_option(field, value));
        if let Some(slot) = self.readiness_edit.refused.get_mut(field as usize) {
            *slot = result.as_ref().err().cloned();
        }
        result
    }

    /// Type `text` into a field and apply it.
    pub fn edit_readiness_field(&mut self, field: ReadinessField, text: &str) -> Result<(), String> {
        self.type_readiness_field(field, text);
        self.apply_readiness_field(field)
    }

    /// The report as plain text, for Copy report: the engine's text, with a line saying which
    /// thresholds were changed when any were (a report read later says what it was measured against).
    pub fn readiness_report_text(&self) -> Option<String> {
        let report = self.readiness_report.as_ref()?;
        let text = report.text();
        let o = &self.readiness_options;
        if o.is_default() {
            return Some(text);
        }
        let mut parts = Vec::new();
        if let Some(n) = o.ink_limit {
            parts.push(format!("total ink limit {n}%"));
        }
        if let Some(n) = o.min_bleed_mm {
            parts.push(format!("least bleed {n} mm"));
        }
        if let Some(n) = o.min_ppi {
            parts.push(format!("least image resolution {n} ppi"));
        }
        let line = format!("Thresholds changed from the defaults: {}\n", parts.join(", "));
        Some(match text.split_once('\n') {
            Some((title, rest)) => format!("{title}\n{line}{rest}"),
            None => format!("{text}\n{line}"),
        })
    }
}

/// The three fields. A value is applied with Enter or when the field loses the keyboard, not on
/// every keystroke (applying measures the document again). Each field has one line under it that is
/// always there, so the dialog does not move: the allowed range, which says to press Enter while
/// typed text is waiting, and the reason in red when the text was refused.
fn options_ui(ui: &mut egui::Ui, app: &mut PdfCraftApp, t: &Tokens) {
    let rows = [
        (ReadinessField::InkLimit, tl!("Total ink limit (%)")),
        (ReadinessField::MinBleed, tl!("Least bleed (mm)")),
        (ReadinessField::MinPpi, tl!("Least resolution (ppi)")),
    ];
    egui::Grid::new("readiness_options").num_columns(2).spacing([12.0, 2.0]).show(ui, |ui| {
        for (field, label) in rows {
            let name = ui.label(label);
            let mut text = app.readiness_edit.text.get(field as usize).cloned().unwrap_or_default();
            let default = field.default_value().to_string();
            let edit = ui.add(widgets::line(&mut text).hint_text(default.as_str()).desired_width(80.0).char_limit(16)).labelled_by(name.id);
            if edit.changed() {
                app.type_readiness_field(field, &text);
            }
            if edit.lost_focus() {
                // The reason is kept in `readiness_edit.refused` and shown below.
                let _ = app.apply_readiness_field(field);
            }
            ui.end_row();
            ui.label("");
            let refused = app.readiness_edit.refused.get(field as usize).cloned().flatten();
            let pending = matches!(field.parse(&text), Ok(v) if v != app.readiness_options.get(field));
            let (line, colour) = match refused {
                Some(why) => (why, ui.visuals().error_fg_color),
                None => {
                    let template = if pending {
                        tl!("From {min} to {max}, default {default}. Press Enter to apply.")
                    } else {
                        tl!("From {min} to {max}, default {default}.")
                    };
                    let (lo, hi) = (field.range().start().to_string(), field.range().end().to_string());
                    (crate::i18n::fmt(template, &[("min", &lo), ("max", &hi), ("default", &default)]), t.text_muted)
                }
            };
            ui.label(egui::RichText::new(line).small().color(colour));
            ui.end_row();
        }
    });
    ui.label(egui::RichText::new(tl!("Empty keeps the default. The settings are remembered.")).small().color(t.text_muted));
    // Always there (disabled at the defaults), so the dialog keeps its height.
    ui.add_enabled_ui(!app.readiness_options.is_default(), |ui| {
        if widgets::pill_button(ui, tl!("Reset to defaults"), false).clicked() {
            for field in ReadinessField::ALL {
                // Setting the default is always accepted.
                let _ = app.set_readiness_option(field, None);
            }
        }
    });
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
    options_ui(ui, app, t);
    ui.add_space(8.0);
    let mut copied = false;
    let mut close = false;
    let text = app.readiness_report_text();
    let max_report = (ui.ctx().content_rect().height() - READINESS_RESERVE).clamp(READINESS_MIN_SCROLL, READINESS_MAX_SCROLL);
    match &app.readiness_report {
        None => {
            ui.label(tl!("There is no report to show."));
            close = widgets::pill_button(ui, tl!("Close"), true).clicked();
        }
        Some(report) => {
            report_body(ui, report, t, max_report);
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = widgets::pill_button(ui, tl!("Close"), true).clicked();
                if widgets::pill_button(ui, tl!("Copy report"), false).clicked()
                    && let Some(text) = text
                {
                    ui.ctx().copy_text(text);
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

fn report_body(ui: &mut egui::Ui, report: &PrintReadiness, t: &Tokens, max_height: f32) {
    egui::ScrollArea::vertical().max_height(max_height).auto_shrink([false, true]).show(ui, |ui| {
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
