//! Print production ▸ Print readiness: what a printer would look at, measured and reported, with nothing changed (the
//! same report as the `print_readiness` tool): the findings, then what was measured (the output intent, page boxes and
//! bleed, image resolution where each image is drawn, fonts that aren't embedded and the PDF/A-2b result). The
//! measuring runs on the UI thread, as the PDF/A check does: when the dialog opens and when a threshold is applied.

use craft::theme::variant::Tone;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::Dialog;
use craft::ui::display::{Shade, Text};
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::runtime::focus;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::style::TypeRole;
use craft::ui::text::{SelectableText, TextField};
use pdfcraft_engine::printcheck::{
    MIN_BLEED_MM, MIN_BLEED_RANGE, MIN_PPI_RANGE, NOT_CHECKED, PrintReadiness, TAC_LIMIT_PERCENT, TAC_LIMIT_RANGE, WARN_PPI,
};

use crate::PdfCraftApp;

/// Detail lines shown: a document with thousands of PDF/A problems lists the first ones here and the rest in Copy
/// report.
const MAX_DETAIL_LINES: usize = 200;

/// What a footer button asks for.
#[derive(Clone, Copy)]
enum Action {
    Copy,
    Reset,
    Close,
}

/// The three thresholds of the `print_readiness` tool (`ink_limit`, `min_bleed_mm`, `min_ppi`). `None` is the engine's
/// own default (300 %, 2 mm, 300 ppi, with the fixed wording of its messages), so a user who changes nothing sees
/// exactly what the tool gives without options.
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

    /// The field's label in the dialog.
    fn label(self) -> String {
        match self {
            Self::InkLimit => tl!("Total ink limit (%)").to_string(),
            Self::MinBleed => tl!("Least bleed (mm)").to_string(),
            Self::MinPpi => tl!("Least resolution (ppi)").to_string(),
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

    /// Typed text to a setting: empty or `default` is the default (`None`), a number in range is that number (a comma
    /// may be the decimal mark and full-width digits are read as digits; `-0` is 0), anything else (text, NaN,
    /// infinity, out of range) is refused with the reason.
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

/// Trim, take full-width digits and separators as their ASCII forms and a comma as the decimal mark.
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

/// What the three fields currently say, as typed (a field the user is half way through typing is not a setting yet).
#[derive(Clone, Debug, Default)]
pub struct ReadinessEdit {
    pub text: [String; 3],
    /// Why a field's text was refused when it was applied (Enter or leaving the field); cleared as soon as the text
    /// changes. Shown on the field's own line, which is always there.
    pub refused: [Option<String>; 3],
}

impl ReadinessEdit {
    fn from_options(options: &ReadinessOptions) -> Self {
        let show = |v: Option<f64>| v.map(|n| n.to_string()).unwrap_or_default();
        Self { text: [show(options.ink_limit), show(options.min_bleed_mm), show(options.min_ppi)], refused: Default::default() }
    }
}

impl PdfCraftApp {
    /// Measures the active document with the remembered thresholds and opens the report. Nothing is edited.
    pub(crate) fn open_print_readiness(&mut self) {
        self.readiness_edit = ReadinessEdit::from_options(&self.readiness_options);
        match self.measure_print_readiness() {
            Ok(true) => self.readiness_open = true,
            Ok(false) => {}
            Err(e) => self.notify_error(e),
        }
    }

    /// Measures the active document with `readiness_options` and keeps the report; the one place the dialog's report is
    /// made, so the findings, the measured lines and Copy report all follow them. `Ok(false)`: there is no document.
    /// The measuring runs on the UI thread, so it is done when a threshold is applied (Enter, leaving the field,
    /// `ui.set`, Reset), not on every keystroke.
    fn measure_print_readiness(&mut self) -> Result<bool, String> {
        let Some((_, id)) = self.active_ids() else { return Ok(false) };
        let o = self.readiness_options;
        let Some(doc) = self.session.get(id) else { return Ok(false) };
        self.readiness_report = Some(doc.print_readiness_with_options(o.ink_limit, o.min_bleed_mm, o.min_ppi)?);
        Ok(true)
    }

    /// Uses `value` for `field` and measures an open report again. Leaves the typed text alone. If the engine refuses it
    /// (not expected after the range check) the old value is kept and the error returned.
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
        if self.readiness_open
            && let Err(e) = self.measure_print_readiness()
        {
            self.readiness_options = before;
            return Err(e);
        }
        Ok(())
    }

    /// Sets one threshold (`ui.set`, Reset and the settings end here). A refused value changes nothing. An open report
    /// is measured again, and the field's text shows the setting again.
    pub fn set_readiness_option(&mut self, field: ReadinessField, value: Option<f64>) -> Result<(), String> {
        self.apply_readiness_option(field, value)?;
        self.readiness_edit = ReadinessEdit::from_options(&self.readiness_options);
        Ok(())
    }

    /// The field's text changed (a keystroke): remember it and forget an old refusal. Nothing is measured until it is
    /// applied.
    pub fn type_readiness_field(&mut self, field: ReadinessField, text: &str) {
        if let Some(slot) = self.readiness_edit.text.get_mut(field as usize) {
            *slot = text.to_string();
        }
        if let Some(slot) = self.readiness_edit.refused.get_mut(field as usize) {
            *slot = None;
        }
    }

    /// Applies the field's typed text (Enter, or the field lost the keyboard): a valid value is used and the report
    /// measured again; an invalid one stays in the field with its reason and changes nothing.
    pub fn apply_readiness_field(&mut self, field: ReadinessField) -> Result<(), String> {
        let text = self.readiness_edit.text.get(field as usize).cloned().unwrap_or_default();
        let result = field.parse(&text).and_then(|value| self.apply_readiness_option(field, value));
        if let Some(slot) = self.readiness_edit.refused.get_mut(field as usize) {
            *slot = result.as_ref().err().cloned();
        }
        result
    }

    /// Types `text` into a field and applies it.
    pub fn edit_readiness_field(&mut self, field: ReadinessField, text: &str) -> Result<(), String> {
        self.type_readiness_field(field, text);
        self.apply_readiness_field(field)
    }

    /// The report as plain text, for Copy report: the engine's text, with a line saying which thresholds were changed
    /// when any were (a report read later says what it was measured against).
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

    /// Carries out what the fields did in one pass: a keystroke is remembered, Enter or leaving the field applies the text.
    /// The reason a text is refused is kept in `readiness_edit.refused` and shown under the field.
    fn apply_field_events(&mut self, events: &FieldEvents, text: &[String; 3]) {
        for (at, field) in ReadinessField::ALL.into_iter().enumerate() {
            if events.typed[at] {
                self.type_readiness_field(field, &text[at]);
            }
            if events.apply[at] {
                let _ = self.apply_readiness_field(field);
            }
        }
    }

    /// Carries out the footer button the dialog was answered with.
    fn run_footer_action(&mut self, picked: Option<Action>) {
        match picked {
            Some(Action::Copy) => {
                if let Some(report) = self.readiness_report_text()
                    && self.put_text(&report)
                {
                    self.notify_tr("Report copied");
                }
            }
            Some(Action::Reset) => {
                for field in ReadinessField::ALL {
                    // Setting the default is always accepted.
                    let _ = self.set_readiness_option(field, None);
                }
            }
            Some(Action::Close) | None => {}
        }
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

/// What the three fields did in one pass: the text a keystroke changed, and the text Enter or leaving the field applies.
#[derive(Default)]
struct FieldEvents {
    typed: [bool; 3],
    apply: [bool; 3],
}

/// Shows the Print readiness dialog while it is open, and carries out the button it was answered with. It closes with
/// the document it measured.
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
    let current = app.readiness_options;
    let mut text = app.readiness_edit.text.clone();
    let refused = app.readiness_edit.refused.clone();
    let actions = [
        (Action::Copy, FooterButton::new(tl!("Copy report"), DialogRole::Other).stays_open(true)),
        // Always there (disabled at the defaults), so the dialog keeps its buttons.
        (Action::Reset, FooterButton::new(tl!("Reset to defaults"), DialogRole::Other).stays_open(true).enabled(!current.is_default())),
        (Action::Close, FooterButton::new(tl!("Close"), DialogRole::Cancel)),
    ];
    let buttons: Vec<FooterButton> = actions.iter().map(|(_, b)| b.clone()).collect();
    let mut open = true;
    let mut events = FieldEvents::default();
    let shown = Dialog::new(tl!("Print readiness")).opened(Bind::bound(&mut open)).buttons(buttons).show(ui, |ui, _cx| {
        Text::new(tl!("Measures what a printer looks at. The document is not changed.")).shade(Shade::Muted).show(ui);
        options_body(ui, &mut text, &refused, &current, &mut events);
        report_body(ui, &report);
    });
    let picked = shown.output.picked.and_then(|at| actions.get(at)).map(|(a, _)| *a);
    // The report is the app's again before anything below measures a new one.
    app.readiness_report = Some(report);
    app.apply_field_events(&events, &text);
    app.run_footer_action(picked);
    if !open {
        app.close_print_readiness();
    }
}

/// The three fields. A value is applied with Enter or when the field loses the keyboard, not on every keystroke
/// (applying measures the document again). Each field has one line under it that is always there, so the dialog does
/// not move: the allowed range, which says to press Enter while typed text is waiting, and the reason when the text was
/// refused.
fn options_body(ui: &mut egui::Ui, text: &mut [String; 3], refused: &[Option<String>; 3], current: &ReadinessOptions, events: &mut FieldEvents) {
    for (at, field) in ReadinessField::ALL.into_iter().enumerate() {
        let before = text[at].clone();
        let default = field.default_value().to_string();
        let edit = TextField::new(field.label(), Bind::bound(&mut text[at])).placeholder(default.clone()).submits().show(ui);
        events.typed[at] = text[at] != before;
        // The focus is on the field's text part, which `focus::is_focused` counts as the field; the response alone
        // does not report the field losing it.
        let had_focus_key = egui::Id::new(("readiness-field-focus", at));
        let has_focus = focus::is_focused(ui.ctx(), edit.response.id);
        let had_focus = ui.ctx().data_mut(|d| d.get_temp::<bool>(had_focus_key).unwrap_or(false));
        ui.ctx().data_mut(|d| d.insert_temp(had_focus_key, has_focus));
        events.apply[at] = edit.submitted || edit.response.lost_focus() || (had_focus && !has_focus);
        match &refused[at] {
            Some(why) => Text::new(why.clone()).role(TypeRole::Caption).tone(Tone::Bad).show(ui),
            None => {
                let pending = matches!(field.parse(&text[at]), Ok(v) if v != current.get(field));
                let template = if pending {
                    tl!("From {min} to {max}, default {default}. Press Enter to apply.")
                } else {
                    tl!("From {min} to {max}, default {default}.")
                };
                let (lo, hi) = (field.range().start().to_string(), field.range().end().to_string());
                let line = crate::i18n::fmt(template, &[("min", &lo), ("max", &hi), ("default", &default)]);
                Text::new(line).role(TypeRole::Caption).shade(Shade::Muted).show(ui)
            }
        };
    }
    Text::new(tl!("Empty keeps the default. The settings are remembered.")).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
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
