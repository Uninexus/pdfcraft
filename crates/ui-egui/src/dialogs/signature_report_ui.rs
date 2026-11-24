//! Sign ▸ Signature audit report, and Sign ▸ Signer labels report: two read-only reports of the open document in one
//! window, each with Copy report and, for the audit, Save as PDF.
//!
//! The audit is `signature_audit_report`: what the signatures in the file say, as PdfCraft's validator read them, and what
//! was and was not checked. The signer labels report is `form_signer_report`: every form field with the signing-plan
//! recipient it is labelled for, the signature fields with none, the labels that name nobody in the plan beside the file
//! and the plan's signers who have no field. Both change nothing in the document, send nothing and sign nothing.

use craft::ui::contract::{Bind, Show as _, ShowContainer as _};
use craft::ui::dialog::Dialog;
use craft::ui::display::{Shade, Text};
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::style::TypeRole;
use craft::ui::text::SelectableText;
use pdfcraft_engine::DocId;
use pdfcraft_engine::FormFieldKind;
use pdfcraft_engine::signature_audit::AuditReport;

use crate::PdfCraftApp;
use crate::prepare::signer::{PlanNote, clean, plan_beside};

/// Lines of the report shown; Copy report has them all.
const MAX_SHOWN_LINES: usize = 400;
/// Rows listed in each part of the signer labels report.
const MAX_ROWS: usize = 200;

/// Which report the window holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportKind {
    Audit,
    Signers,
}

/// The open report: made when the window opened, so it says what the document held then.
#[derive(Clone, Debug)]
pub struct ReportWindow {
    pub kind: ReportKind,
    pub doc: DocId,
    /// The document's name, for the saved file's name.
    pub name: String,
    /// The report as plain text: what is shown and copied.
    pub text: String,
    /// The audit's data, for the PDF.
    pub audit: Option<AuditReport>,
}

#[derive(Clone, Copy)]
enum Action {
    Save,
    Copy,
}

impl ReportWindow {
    fn title(&self) -> &'static str {
        match self.kind {
            ReportKind::Audit => tl!("Signature audit report"),
            ReportKind::Signers => tl!("Signer labels report"),
        }
    }

    /// Whether Save as PDF is offered: only an audit of a document that holds a signature.
    pub fn can_save(&self) -> bool {
        self.audit.as_ref().is_some_and(|a| a.signed > 0)
    }
}

impl PdfCraftApp {
    /// `sign.audit_report`: audit the signatures of the active document and open the report.
    pub(crate) fn open_signature_audit(&mut self) {
        let Some((_, id)) = self.active_ids() else { return };
        match self.session.signature_audit(id) {
            Ok(report) => {
                let name = self.session.get(id).map(|d| d.name.clone()).unwrap_or_default();
                self.signature_report = Some(ReportWindow { kind: ReportKind::Audit, doc: id, name, text: report.text(), audit: Some(report) });
            }
            Err(e) => self.notify_error(e),
        }
    }

    /// `form.signer_report`: list the signer label of every form field of the active document and open the report.
    pub(crate) fn open_signer_report(&mut self) {
        let Some((_, id)) = self.active_ids() else { return };
        let Some(text) = self.signer_report_text(id) else { return };
        let name = self.session.get(id).map(|d| d.name.clone()).unwrap_or_default();
        self.signature_report = Some(ReportWindow { kind: ReportKind::Signers, doc: id, name, text, audit: None });
    }

    /// The text of the signer labels report for document `id`.
    fn signer_report_text(&self, id: DocId) -> Option<String> {
        let doc = self.session.get(id)?;
        let plan = plan_beside(doc.path.as_deref());
        let people: &[(String, String)] = match &plan {
            PlanNote::Found(p) => p,
            _ => &[],
        };
        let in_plan = |email: &str| people.iter().any(|(_, e)| e.to_lowercase() == email.to_lowercase());
        let (mut assigned, mut missing, mut outside) = (Vec::new(), Vec::new(), Vec::new());
        let mut named: Vec<String> = Vec::new();
        for f in doc.form.iter() {
            let kind = f.kind == FormFieldKind::Signature;
            let read = doc.field_signer(f.obj);
            match read.email.as_deref().map(|e| clean(e, 254)).filter(|e| !e.trim().is_empty()) {
                Some(email) => {
                    named.push(email.to_lowercase());
                    if matches!(plan, PlanNote::Found(_)) && !in_plan(&email) {
                        outside.push(format!("{}: {}", clean(&f.name, 200), email));
                    }
                    assigned.push(format!("{}: {}", clean(&f.name, 200), email));
                }
                None if kind => missing.push(clean(&f.name, 200)),
                None => {}
            }
        }
        let without_field: Vec<String> = people
            .iter()
            .filter(|(_, e)| !named.contains(&e.to_lowercase()))
            .map(|(n, e)| format!("{} ({})", clean(n, 200), clean(e, 254)))
            .collect();
        let mut t = String::new();
        let mut line = |s: String| {
            t.push_str(&s);
            t.push('\n');
        };
        line(crate::i18n::fmt(tl!("Signer labels of {name}"), &[("name", &clean(&doc.name, 200))]));
        line(crate::i18n::fmt(
            tl!("{fields} form fields, {assigned} with a signer, {missing} signature fields with none"),
            &[("fields", &doc.form.len().to_string()), ("assigned", &assigned.len().to_string()), ("missing", &missing.len().to_string())],
        ));
        line(match &plan {
            PlanNote::Found(p) => {
                crate::i18n::fmt(tl!("The signing plan beside the file has {n} signers or approvers."), &[("n", &p.len().to_string())])
            }
            PlanNote::None => tl!("There is no signing plan beside the file, so the labels were not checked against one.").to_string(),
            PlanNote::NotChecked => tl!("The document has no saved .pdf file, so no signing plan was looked for.").to_string(),
            PlanNote::Unreadable(why) => crate::i18n::fmt(tl!("The signing plan beside the file cannot be used: {why}"), &[("why", why.as_str())]),
        });
        let mut part = |title: &str, rows: &[String]| {
            if rows.is_empty() {
                return;
            }
            line(String::new());
            line(format!("{title} ({})", rows.len()));
            for r in rows.iter().take(MAX_ROWS) {
                line(format!("- {r}"));
            }
            if rows.len() > MAX_ROWS {
                line(crate::i18n::fmt(tl!("… and {n} more"), &[("n", &(rows.len() - MAX_ROWS).to_string())]));
            }
        };
        part(tl!("Fields with a signer"), &assigned);
        part(tl!("Signature fields with no signer"), &missing);
        part(tl!("Labels that name someone who is not a signer or approver in the plan"), &outside);
        part(tl!("Plan recipients with no field"), &without_field);
        line(String::new());
        line(tl!("A signer is a label stored in the file. It does not restrict who can fill or sign a field, nothing is sent and nobody is notified, and other PDF readers ignore it.").to_string());
        Some(t)
    }
}

/// Shows the report while it is open and carries out the footer button it was answered with. It closes with the document.
pub(crate) fn show(app: &mut PdfCraftApp, ui: &mut egui::Ui) {
    let Some(w) = app.signature_report.take() else { return };
    if app.active_ids().map(|(_, id)| id) != Some(w.doc) {
        return;
    }
    let mut actions: Vec<(Option<Action>, FooterButton)> = Vec::new();
    if w.kind == ReportKind::Audit {
        actions.push((Some(Action::Save), FooterButton::new(tl!("Save as PDF…"), DialogRole::Other).stays_open(true).enabled(w.can_save())));
    }
    actions.push((Some(Action::Copy), FooterButton::new(tl!("Copy report"), DialogRole::Other).stays_open(true)));
    actions.push((None, FooterButton::new(tl!("Close"), DialogRole::Cancel)));
    let buttons: Vec<FooterButton> = actions.iter().map(|(_, b)| b.clone()).collect();
    let mut open = true;
    let lines: Vec<&str> = w.text.lines().collect();
    let shown = lines.iter().take(MAX_SHOWN_LINES).copied().collect::<Vec<_>>().join("\n");
    let more = lines.len().saturating_sub(MAX_SHOWN_LINES);
    let answered = Dialog::new(w.title()).opened(Bind::bound(&mut open)).buttons(buttons).show(ui, |ui, _cx| {
        Text::new(tl!("A summary made on this computer. The document is not changed.")).shade(Shade::Muted).show(ui);
        SelectableText::new(w.title(), &shown).multiline().show(ui);
        if more > 0 {
            let line = crate::i18n::fmt(tl!("… and {n} more lines (Copy report has them all)"), &[("n", &more.to_string())]);
            Text::new(line).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
        }
    });
    let picked = answered.output.picked.and_then(|at| actions.get(at)).and_then(|(a, _)| *a);
    match picked {
        Some(Action::Copy) => {
            if app.put_text(&w.text) {
                app.notify_tr("Report copied");
            }
        }
        Some(Action::Save) => save_pdf(app, &w),
        None => {}
    }
    if open {
        app.signature_report = Some(w);
    }
}

/// Makes the audit's PDF and asks where to save it.
fn save_pdf(app: &mut PdfCraftApp, w: &ReportWindow) {
    let Some(report) = w.audit.as_ref().filter(|a| a.signed > 0) else {
        return app.notify_tr("The document has no signatures, so there is nothing to put in a report file");
    };
    match app.session.signature_audit_pdf(report) {
        Ok(bytes) => {
            let stem = w.name.trim_end_matches(".pdf").trim_end_matches(".PDF");
            let stem: String = stem.chars().filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':')).collect();
            let title = tl!("Save the signature audit report").to_string();
            app.write_files(&[(format!("{stem} Signature Audit Report.pdf"), bytes)], &title);
        }
        Err(e) => app.notify_error(e),
    }
}
