//! Signature properties: one signature's verdict and details in a dialog, from `sign.properties` or the signature in
//! the Signatures panel. It reads what validation already found and changes nothing.

use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Entry, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::form::Field;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::pickers::Select;
use craft::ui::sections::{DialogRole, FooterButton};
use egui::Ui;
use pdfcraft_engine::{SignatureInfo, SignatureStatus, sign};

use crate::PdfCraftApp;

/// The value the form binds: which signature is shown, by its field name.
#[derive(Clone, Debug, Default, PartialEq)]
struct Chosen {
    field: String,
}

/// Signature properties, owned by the dialog host.
pub(crate) struct SignaturePropsDialog {
    id: DialogId,
    chosen: Chosen,
    /// The document's signatures as of this frame, oldest revision first.
    signatures: Vec<SignatureInfo>,
    /// Show certificate… was clicked: the chain to open the viewer on.
    certificate: Option<Vec<sign::Certificate>>,
}

fn signed(signatures: &[SignatureInfo]) -> Vec<SignatureInfo> {
    let mut signed: Vec<SignatureInfo> = signatures.iter().filter(|s| s.signed).cloned().collect();
    signed.sort_by_key(|s| s.revision);
    signed
}

/// The verdict, as the panel heads its findings.
fn verdict(status: SignatureStatus) -> &'static str {
    match status {
        SignatureStatus::Valid => tl!("Signature is valid:"),
        SignatureStatus::Unknown => tl!("Signature validity is UNKNOWN:"),
        SignatureStatus::Invalid => tl!("Signature is INVALID:"),
    }
}

/// What a certification signature (DocMDP 1 to 3) allows afterwards.
fn permits(level: u8) -> &'static str {
    match level {
        1 => tl!("No changes are allowed"),
        2 => tl!("Form filling and signing are allowed"),
        _ => tl!("Commenting, form filling and signing are allowed"),
    }
}

/// The named details of `s` that it has, in the order shown.
fn details(s: &SignatureInfo) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    let mut add = |label: &'static str, value: Option<String>| {
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            rows.push((label, value));
        }
    };
    let certificate = s.certificate.as_ref();
    add(tl!("Signer"), certificate.map(|c| c.subject.display()).or_else(|| s.signer.clone()));
    add(tl!("Issued by"), certificate.map(|c| c.issuer.display()));
    add(
        tl!("Certificate valid"),
        certificate.map(|c| crate::i18n::fmt(tl!("{from} to {to}"), &[("from", &c.not_before.to_string()), ("to", &c.not_after.to_string())])),
    );
    add(tl!("Signing time"), s.date.as_deref().map(sign::pdf::display_date));
    // A time is shown only for a timestamp that was checked and trusted.
    add(
        tl!("Timestamp"),
        s.timestamp_time.map(|t| t.to_string()).or_else(|| s.timestamp.then(|| tl!("Included, its time is not trusted").to_string())),
    );
    add(tl!("Reason"), s.reason.clone());
    add(tl!("Location"), s.location.clone());
    add(tl!("Contact"), s.contact.clone());
    add(tl!("Algorithm"), s.algorithm.clone());
    add(tl!("Format"), s.sub_filter.clone());
    add(tl!("Certification"), s.certify.map(|level| permits(level).to_string()));
    add(tl!("Revision"), Some(s.revision.to_string()));
    let field = match s.page {
        Some(page) => crate::i18n::fmt(tl!("{field} on page {p}"), &[("field", &s.field), ("p", &(page + 1).to_string())]),
        None => s.field.clone(),
    };
    add(tl!("Field"), Some(field));
    rows
}

impl PdfCraftApp {
    /// Opens Signature properties on the signature of the field `field`, else on the first one signed. A document
    /// without a signature says so.
    pub(crate) fn open_signature_properties(&mut self, field: Option<String>) {
        let Some((_, id)) = self.active_ids() else { return };
        let signatures = self.session.get(id).map(|d| signed(&d.signatures)).unwrap_or_default();
        let Some(first) = signatures.first() else {
            self.notify(tl!("This document has no signatures.").to_string());
            return;
        };
        let field = field.filter(|f| signatures.iter().any(|s| s.field == *f)).unwrap_or_else(|| first.field.clone());
        crate::dialogs::host::open(
            self,
            SignaturePropsDialog { id: dialog_id("signature-properties"), chosen: Chosen { field }, signatures, certificate: None },
        );
    }

    /// Keeps the open dialog on the active document's signatures as they are now; it closes when there are none.
    pub(crate) fn signature_properties_pass(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialogs.get_mut::<SignaturePropsDialog>() else { return };
        // The viewer takes this dialog's place: a window under a modal dialog could not be used.
        if let Some(chain) = dialog.certificate.take() {
            self.dialogs.cancel::<SignaturePropsDialog>(ctx);
            crate::sign_ui::cert_viewer::open(self, chain);
            return;
        }
        let signatures = self.active_ids().and_then(|(_, id)| self.session.get(id)).map(|d| signed(&d.signatures)).unwrap_or_default();
        if signatures.is_empty() {
            self.dialogs.cancel::<SignaturePropsDialog>(ctx);
        } else if let Some(dialog) = self.dialogs.get_mut::<SignaturePropsDialog>() {
            dialog.signatures = signatures;
        }
    }
}

impl StackDialog for SignaturePropsDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let SignaturePropsDialog { chosen, signatures, certificate, .. } = self;
        let close = FooterButton::new(tl!("Close").to_string(), DialogRole::Cancel);
        let out =
            FormDialog::new(tl!("Signature properties").to_string(), Bind::bound(chosen)).shaped(|d| d.buttons([close])).show(ui, |ui, _, c| {
                if signatures.len() > 1 {
                    let choices: Vec<Entry<String>> = signatures
                        .iter()
                        .map(|s| {
                            let name = s.signer.as_deref().unwrap_or(&s.field);
                            Entry::new(
                                s.field.clone(),
                                crate::i18n::fmt(tl!("Rev. {rev}: {name}"), &[("rev", &s.revision.to_string()), ("name", name)]),
                            )
                        })
                        .collect();
                    let caption = tl!("Signature:").to_string();
                    Field::new(caption.clone()).show(ui, |ui, _| {
                        if let Some(field) = Select::new(caption.clone(), Bind::given(c.field.clone()), &choices).show(ui).value().cloned() {
                            c.field = field;
                        }
                    });
                }
                let Some(s) = signatures.iter().find(|s| s.field == c.field).or_else(|| signatures.first()) else { return true };
                Text::new(verdict(s.status).to_string()).show(ui);
                for detail in &s.details {
                    Text::new(format!("• {}", detail.said(&crate::i18n::words))).show(ui);
                }
                for (label, value) in details(s) {
                    Field::new(label.to_string()).value(value.clone()).show(ui, |ui, _| {
                        Text::new(value.clone()).show(ui);
                    });
                }
                if let Some(signer) = &s.certificate
                    && Button::new(tl!("Show certificate…")).show(ui).clicked()
                {
                    *certificate = Some(crate::sign_ui::panel::certificate_chain(s, signer));
                    ui.ctx().request_repaint();
                }
                true
            });
        out.closed.map(|_| DialogResult::Cancel)
    }
}
