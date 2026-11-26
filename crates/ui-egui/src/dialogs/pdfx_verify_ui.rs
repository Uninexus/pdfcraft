//! Standards ▸ Verify PDF/X: the tool `pdfx_verify` in a dialog. The person picks the PDF/X family (the one the
//! document declares by default) and presses Verify; the findings are listed as the tool lists them. It reports
//! only: nothing is changed, and no findings is never worded as conformance.

use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Entry, Show as _, ShowCollection as _, ShowContainer as _, WordSetCx as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::list::{Finding, Findings, FindingsList, FindingsSummary, FindingsWords, Severity};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::pickers::Select;
use craft::ui::sections::{DialogRole, FooterButton};
use egui::Ui;
use pdfcraft_engine::pdfa::pdfx::Family;

use crate::PdfCraftApp;

/// Which family the findings are measured against.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Choice {
    /// The family the document declares.
    #[default]
    Declared,
    X1a,
    X3,
    X4,
}

impl Choice {
    fn family(self) -> Option<Family> {
        match self {
            Choice::Declared => None,
            Choice::X1a => Some(Family::X1a),
            Choice::X3 => Some(Family::X3),
            Choice::X4 => Some(Family::X4),
        }
    }
}

/// The value the form binds.
#[derive(Clone, Debug, Default, PartialEq)]
struct ChoiceForm {
    choice: Choice,
}

/// The last verification.
#[derive(Clone, Debug, Default)]
struct Shown {
    /// The family the findings were measured against.
    checked: Option<&'static str>,
    /// `GTS_PDFXVersion` as the document states it.
    version: Option<String>,
    /// The wording of each finding with the id of its rule.
    findings: Vec<(String, &'static str)>,
}

/// Standards ▸ Verify PDF/X, owned by the dialog host.
pub(crate) struct PdfxVerifyDialog {
    id: DialogId,
    form: ChoiceForm,
    /// `None` until Verify runs and after the family changes.
    shown: Option<Shown>,
    /// Verify was pressed; `pdfx_verify_pass` runs it.
    verify: bool,
}

impl PdfCraftApp {
    /// Opens Verify PDF/X for the active document, with nothing verified yet.
    pub(crate) fn open_pdfx_verify(&mut self) {
        if self.active_ids().is_none() {
            return;
        }
        let dialog = PdfxVerifyDialog { id: dialog_id("pdfx-verify"), form: ChoiceForm::default(), shown: None, verify: false };
        crate::dialogs::host::open(self, dialog);
    }

    /// Runs the verification the open dialog asked for.
    pub(crate) fn pdfx_verify_pass(&mut self) {
        let Some((choice, asked)) = self.dialogs.get_mut::<PdfxVerifyDialog>().map(|d| (d.form.choice, std::mem::take(&mut d.verify))) else {
            return;
        };
        if !asked {
            return;
        }
        let report = self.active_ids().and_then(|(_, id)| self.session.get(id)).map(|d| d.pdfx_verify(choice.family()));
        let shown = report.map(|r| Shown {
            checked: r.checked.map(|f| f.label()),
            version: r.version,
            findings: r.findings.into_iter().map(|f| (f.message, f.rule)).collect(),
        });
        if let Some(d) = self.dialogs.get_mut::<PdfxVerifyDialog>() {
            d.shown = shown;
        }
    }
}

impl StackDialog for PdfxVerifyDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let PdfxVerifyDialog { form, shown, verify, .. } = self;
        let before = form.choice;
        let close = FooterButton::new(tl!("Close").to_string(), DialogRole::Cancel);
        let out = FormDialog::new(tl!("Verify PDF/X").to_string(), Bind::bound(&mut *form)).shaped(|d| d.buttons([close])).show(ui, |ui, _, f| {
            Text::new(
                tl!("Report only: PdfCraft reads what the document declares and its structure, and lists what stands between it and the chosen PDF/X family. Nothing is changed.")
                    .to_string(),
            )
            .shade(Shade::Muted)
            .show(ui);
            let choices = [
                Entry::new(Choice::Declared, tl!("The family the document declares").to_string()),
                Entry::new(Choice::X1a, Family::X1a.label().to_string()),
                Entry::new(Choice::X3, Family::X3.label().to_string()),
                Entry::new(Choice::X4, Family::X4.label().to_string()),
            ];
            let caption = tl!("PDF/X family:").to_string();
            Field::new(caption.clone()).show(ui, |ui, _| {
                if let Some(c) = Select::new(caption.clone(), Bind::given(f.choice), &choices).show(ui).value().copied() {
                    f.choice = c;
                }
            });
            // Outside the field, so the button keeps its own name: inside, the field names its control after its caption.
            if Button::new(tl!("Verify").to_string()).show(ui).clicks > 0 {
                *verify = true;
            }
            match &*shown {
                None => {
                    Text::new(tl!("Verify to see what the document needs.").to_string()).shade(Shade::Muted).show(ui);
                }
                Some(s) => {
                    let version = s.version.clone().unwrap_or_else(|| tl!("none").to_string());
                    Text::new(crate::i18n::fmt(tl!("Declared version: {version}"), &[("version", &version)])).show(ui);
                    if let Some(family) = s.checked {
                        Text::new(crate::i18n::fmt(tl!("Checked against {family}"), &[("family", family)])).shade(Shade::Muted).show(ui);
                    }
                    if s.findings.is_empty() {
                        Text::new(tl!("No problems found by these checks. This is not a validator result and does not make the file PDF/X.").to_string())
                            .show(ui);
                    } else {
                        let findings: Vec<Finding> = s
                            .findings
                            .iter()
                            .enumerate()
                            .map(|(n, (message, rule))| Finding::new(n as u64, Severity::Error, crate::i18n::words(message)).detail(rule.to_string()))
                            .collect();
                        FindingsSummary::new(&findings).show(ui);
                        // The dialog's body scrolls: the list takes its full height and owns no scroll of its own.
                        FindingsList::new(tl!("PDF/X problems").to_string()).natural_height(true).show_items(
                            ui,
                            &Findings::new(&findings),
                            &mut |key, icx| findings[*key as usize].row(&icx.words::<FindingsWords>()),
                        );
                    }
                    Text::new(
                        tl!("Not checked: colour spaces, output profile contents, registered printing conditions, page box geometry, image and font internals, and the other conditions of ISO 15930. PDF/X-5 and PDF/X-6 are not checked.")
                            .to_string(),
                    )
                    .shade(Shade::Muted)
                    .show(ui);
                }
            }
            true
        });
        if form.choice != before {
            *shown = None;
        }
        out.closed.map(|_| DialogResult::Cancel)
    }
}
