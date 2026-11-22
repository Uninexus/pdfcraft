//! A signature audit report: what the signatures in an open document say, as the existing
//! validator read them, in one structure and as plain text. Headless; it reads no file again and
//! makes no network request, and it signs nothing.
//!
//! The report is a summary written locally from the signatures in the file. It does not add to
//! what the validator decided, and it says what was and was not checked.

use std::sync::Arc;

use pdfcraft_sign::{DigestAlg, Modification, SignatureInfo, Status};

use crate::links::hides_text;
use crate::{DocId, Document, EditError, Session};

/// Most signature entries a report lists; the counts still cover every signature that was found.
pub const MAX_ENTRIES: usize = 50;
/// Longest text a report repeats from the file (a signer's name, a reason, a sentence), in
/// characters; a longer one is cut and the entry says so.
pub const MAX_TEXT_CHARS: usize = 240;
/// Most explanatory sentences of one signature a report repeats.
pub const MAX_DETAILS: usize = 12;

/// What the statuses mean, said in every report and in the tool's description.
pub const STATUS_MEANING: &str = "Valid means the signature verified, the signer is trusted, and no change the signature forbids follows it. Unknown means it was not shown to be invalid but not shown to be valid either: the signer is not on the trusted list, or something could not be checked (this includes signatures PdfCraft cannot check yet); the entry's own sentences say which. Invalid means the file was altered, or the signature does not verify or its data could not be read, or the signer's certificate is missing or was revoked according to revocation data in the file, or later changes are not permitted.";

/// What the validator did and did not do, for every report. Said in the report, never left to a
/// reader to assume.
pub const CHECKED: &[&str] = &[
    "For each signature PdfCraft could read, the signed bytes were hashed again and compared with the signature, and the signature value was checked against the signer's certificate. An entry's own sentences say when a signature could not be read or checked.",
    "For each signature checked that far, whether later revisions of the file changed anything and whether the signature permits those changes. An entry says \"not determined\" when this step was not reached or could not be finished (for example when the fields a signature locks could not all be compared).",
    "The certificate chain, as far as the file's own certificates and this program's list of trusted certificates allow. A signer counts as trusted if its certificate, or a certificate it chains to, is on that list (the number of certificates on it is given in the report).",
    "Revocation information for a signature's signer certificate, from the file's document security store (/DSS), when the file holds any and the signing time is known.",
    "A timestamp stored in the signature, or a document timestamp, when there is one. For a signature, its time is called validated when its authority chains to the trusted list; for a document timestamp, only when the entry's status is valid.",
];
/// What is not done, said in every report.
pub const NOT_CHECKED: &[&str] = &[
    "No online revocation check was made: no certificate authority, OCSP responder or time-stamp server was contacted.",
    "Revocation of intermediate certificates was not checked, and revocation data archived inside the signature itself (adbe-revocationInfoArchival) was not read.",
    "Revocation of the certificates of timestamp authorities (and of document timestamps) was not checked.",
    "Who the signer is in the real world was not checked, only what their certificate says.",
    "The report says nothing about whether a signature has legal effect, in any country.",
    "This report is a summary generated locally from the signatures in the file. It is not itself signed.",
];

/// One signature field, document timestamp or empty signature field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditEntry {
    pub field: String,
    /// `signature`, `document_timestamp`, `unsigned_field`, or `unread_fields` (a stand-in row for
    /// signature fields the validator did not read because the form was too large).
    pub kind: &'static str,
    pub signer: Option<String>,
    pub certificate_subject: Option<String>,
    pub certificate_issuer: Option<String>,
    pub self_signed: Option<bool>,
    /// The signing time as the file writes it (`/M`), which the signer's own software set.
    pub claimed_time: Option<String>,
    /// The signing time the validator used for its checks (a signed attribute, else `/M`); both
    /// are claimed by the signer.
    pub signing_time: Option<String>,
    pub timestamp: bool,
    /// The generation time the timestamp token states. It is a validated time only when `status`
    /// is `valid`; otherwise it is only what the token claims.
    pub timestamp_time: Option<String>,
    pub digest: Option<&'static str>,
    pub algorithm: Option<String>,
    pub sub_filter: Option<String>,
    /// `valid`, `unknown`, `invalid` or `unsigned`.
    pub status: &'static str,
    /// The validator's explanatory sentences, in English, as it produced them.
    pub reasons: Vec<String>,
    /// 1-based revision of the file the signature covers; none when validation stopped before the
    /// signed byte range was read.
    pub revision: Option<usize>,
    /// `whole_file` (nothing follows it), `changes_allowed` or `changes_disallowed` (later
    /// revisions change the file; the second kind the signature does not permit),
    /// `not_determined` (validation stopped before the changes were classified) or
    /// `not_applicable` (an empty signature field).
    pub covers: &'static str,
    pub changes_after_signing: Vec<String>,
    /// DocMDP level 1 to 3 of a certification signature.
    pub certification_level: Option<u8>,
    /// 1-based page of the signature's widget.
    pub page: Option<usize>,
    pub reason: Option<String>,
    pub location: Option<String>,
    /// A text of this entry was cut at [`MAX_TEXT_CHARS`].
    pub text_cut: bool,
    /// The signature had more explanatory sentences than [`MAX_DETAILS`]; the rest are left out.
    pub reasons_cut: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditReport {
    pub document: String,
    pub pages: usize,
    pub bytes: usize,
    /// SHA-256 of the document as the program holds it now, in lower-case hex. That is the file
    /// as opened and any change not yet saved; it is not the file on disk when `unsaved_changes`
    /// or `changed_on_open` is set.
    pub sha256: String,
    pub unsaved_changes: bool,
    /// Opening the file made PdfCraft add changes of its own (a form's layout or scripts, a
    /// repaired page tree), so the bytes held may differ from the file on disk.
    pub changed_on_open: bool,
    /// The document's name was cut at [`MAX_TEXT_CHARS`].
    pub document_cut: bool,
    /// The count of signed and unknown signatures includes a stand-in row for signature fields
    /// the validator did not read (a form too large); those may or may not be signed.
    pub stand_in_counted: bool,
    /// Every signature field, document timestamp and empty signature field found.
    pub found: usize,
    pub signed: usize,
    pub valid: usize,
    pub unknown: usize,
    pub invalid: usize,
    /// Certificates on the trust list the signatures were checked against.
    pub trusted_certificates: usize,
    pub entries: Vec<AuditEntry>,
    /// More were found than [`MAX_ENTRIES`] listed.
    pub truncated: bool,
}

fn level_words(level: u8) -> &'static str {
    match level {
        1 => "no changes allowed",
        2 => "form filling and signing allowed",
        _ => "form filling, signing and comments allowed",
    }
}

/// `text` for a report: control characters, line breaks and other whitespace become spaces,
/// characters that hide or reorder text (bidirectional controls, zero-width characters, the byte
/// order mark) are dropped, and it is cut at [`MAX_TEXT_CHARS`]; the flag says whether it was cut.
fn clean(text: &str, cut: &mut bool) -> String {
    let one_line: String = text
        .chars()
        .filter_map(|c| {
            if c.is_control() || (c.is_whitespace() && c != ' ') {
                Some(' ')
            } else if hides_text(c) {
                None
            } else {
                Some(c)
            }
        })
        .collect();
    let one_line = one_line.trim();
    if one_line.chars().count() > MAX_TEXT_CHARS {
        *cut = true;
        one_line.chars().take(MAX_TEXT_CHARS).collect()
    } else {
        one_line.to_string()
    }
}

/// The stand-in row the listing adds for signature fields it did not read. A field that merely
/// has that name is not one: the row is also unknown and carries the listing's own sentence.
fn is_stand_in(s: &SignatureInfo) -> bool {
    s.signed
        && s.field == "UnreadFields"
        && s.signed_len == 0
        && s.status == Status::Unknown
        && s.details.len() == 1
        && s.details[0].message.starts_with("This form has too many fields")
}

fn entry(s: &SignatureInfo) -> AuditEntry {
    let mut cut = false;
    let mut text = |t: &str| clean(t, &mut cut);
    let unread = is_stand_in(s);
    let (signer, subject, issuer) = (
        s.signer.as_deref().map(&mut text),
        s.certificate.as_ref().map(|c| text(&c.subject.display())),
        s.certificate.as_ref().map(|c| text(&c.issuer.display())),
    );
    let (claimed_time, reason, location) =
        (s.date.as_deref().map(&mut text), s.reason.as_deref().map(&mut text), s.location.as_deref().map(&mut text));
    let (sub_filter, algorithm) = (s.sub_filter.as_deref().map(&mut text), s.algorithm.as_deref().map(&mut text));
    let reasons: Vec<String> = s.details.iter().take(MAX_DETAILS).map(|d| text(&d.message)).collect();
    let reasons_cut = s.details.len() > MAX_DETAILS;
    // The listing starts every row as "nothing follows it"; that is a finding only when the
    // change check ran.
    let (covers, changes) = if !s.signed {
        ("not_applicable", Vec::new())
    } else if !s.changes_checked {
        ("not_determined", Vec::new())
    } else {
        match &s.modification {
            Modification::None => ("whole_file", Vec::new()),
            Modification::Allowed(c) => ("changes_allowed", c.iter().map(|k| k.to_string()).collect()),
            Modification::Disallowed(c) => ("changes_disallowed", c.iter().map(|k| k.to_string()).collect()),
        }
    };
    AuditEntry {
        field: clean(&s.field, &mut cut),
        kind: if !s.signed {
            "unsigned_field"
        } else if unread {
            "unread_fields"
        } else if s.doc_timestamp {
            "document_timestamp"
        } else {
            "signature"
        },
        signer,
        certificate_subject: subject,
        certificate_issuer: issuer,
        self_signed: s.certificate.as_ref().map(|c| c.is_self_signed()),
        claimed_time,
        signing_time: s.signing_time.map(|t| t.to_string()),
        timestamp: s.timestamp,
        timestamp_time: s.timestamp_time.map(|t| t.to_string()),
        digest: s.digest.map(DigestAlg::name),
        algorithm,
        sub_filter,
        status: if !s.signed {
            "unsigned"
        } else {
            match s.status {
                Status::Valid => "valid",
                Status::Unknown => "unknown",
                Status::Invalid => "invalid",
            }
        },
        reasons,
        revision: (s.signed && s.signed_len > 0).then_some(s.revision),
        covers,
        changes_after_signing: changes,
        certification_level: s.certify,
        page: s.page.map(|p| p + 1),
        reason,
        location,
        text_cut: cut,
        reasons_cut,
    }
}

/// The report of `doc`'s signatures as they were validated.
pub fn audit(doc: &Document) -> AuditReport {
    let mut name_cut = false;
    let signed: Vec<&SignatureInfo> = doc.signatures.iter().filter(|s| s.signed).collect();
    let count = |st: Status| signed.iter().filter(|s| s.status == st).count();
    AuditReport {
        document: clean(&doc.name, &mut name_cut),
        pages: doc.info.pages.len(),
        bytes: doc.bytes.len(),
        sha256: DigestAlg::Sha256.digest(&[doc.bytes.as_slice()]).iter().map(|b| format!("{b:02x}")).collect(),
        unsaved_changes: doc.dirty,
        changed_on_open: doc.changed_on_open,
        document_cut: name_cut,
        stand_in_counted: doc.signatures.iter().any(is_stand_in),
        found: doc.signatures.len(),
        signed: signed.len(),
        valid: count(Status::Valid),
        unknown: count(Status::Unknown),
        invalid: count(Status::Invalid),
        trusted_certificates: doc.trust.certs.len(),
        entries: doc.signatures.iter().take(MAX_ENTRIES).map(entry).collect(),
        truncated: doc.signatures.len() > MAX_ENTRIES,
    }
}

impl AuditReport {
    /// The report in plain readable text, for a PDF or a screen.
    pub fn text(&self) -> String {
        let mut t = String::new();
        let mut line = |s: String| {
            t.push_str(&s);
            t.push('\n');
        };
        line("Signature audit report".into());
        line(String::new());
        line("This is a summary generated locally from the signatures in the file. It is not itself signed, and it is not a legal document.".into());
        line(String::new());
        line(format!("Document: {}", self.document));
        if self.document_cut {
            line(format!("The document's name was cut at {MAX_TEXT_CHARS} characters."));
        }
        line(format!("Pages: {}", self.pages));
        line(format!("Size: {} bytes", self.bytes));
        line(format!("SHA-256 of the document as PdfCraft holds it: {}", self.sha256));
        if self.unsaved_changes {
            line("The document has changes that are not saved; the fingerprint and the signatures are those of the document as it is now, not of the file on disk.".into());
        }
        if self.changed_on_open {
            line("Opening the file made PdfCraft add changes of its own (for example a form's layout or data sync). The signatures were checked against those bytes, so changes reported after signing may include PdfCraft's own, and the fingerprint may differ from the file on disk even with nothing edited. Saving does not clear this note.".into());
        }
        line(String::new());
        line(format!(
            "Signatures: {} signed ({} valid, {} unknown, {} invalid); {} found in all, counting empty signature fields.",
            self.signed, self.valid, self.unknown, self.invalid, self.found
        ));
        if self.stand_in_counted {
            line("The counts include one stand-in row for signature fields PdfCraft did not read; they may or may not be signed.".into());
        }
        line(STATUS_MEANING.into());
        line(format!("Certificates on the trusted list: {}.", self.trusted_certificates));
        if self.truncated {
            line(format!("Only the first {MAX_ENTRIES} entries are listed below; the counts above cover all of them."));
        }
        for (i, e) in self.entries.iter().enumerate() {
            line(String::new());
            let what = match e.kind {
                "document_timestamp" => "document timestamp",
                "unsigned_field" => "empty signature field",
                "unread_fields" => "signature fields not read",
                _ => "signature",
            };
            line(format!("{}. Field {} ({what})", i + 1, e.field));
            line(format!("   Status: {}", e.status));
            if e.kind == "unsigned_field" {
                continue;
            }
            if e.kind == "unread_fields" {
                for r in &e.reasons {
                    line(format!("   - {r}"));
                }
                continue;
            }
            let or = |o: &Option<String>| o.clone().unwrap_or_else(|| "not stated".into());
            line(format!("   Signer: {}", or(&e.signer)));
            line(format!("   Certificate subject: {}", or(&e.certificate_subject)));
            line(format!("   Certificate issuer: {}", or(&e.certificate_issuer)));
            if e.self_signed == Some(true) {
                line("   The certificate is self-signed.".into());
            }
            line(format!("   Signing time written in the file (/M), claimed by the signer: {}", or(&e.claimed_time)));
            if let Some(time) = &e.signing_time {
                line(format!("   Signing time used for checks (signed attribute, else /M; both claimed by the signer): {time}"));
            }
            line(match (&e.timestamp_time, e.timestamp) {
                (Some(time), _) if e.kind == "signature" || e.status == "valid" => format!("   Timestamp: validated, {time}"),
                (Some(time), _) => format!("   Timestamp: time stated by the token, not validated ({time})"),
                (None, true) => "   Timestamp: present, but not validated".into(),
                (None, false) => "   Timestamp: none".into(),
            });
            line(format!("   Hash algorithm: {}", e.digest.unwrap_or("not stated")));
            line(format!("   Format: {}", or(&e.sub_filter)));
            line(match e.revision {
                Some(r) => format!("   Revision covered: {r}"),
                None => "   Revision covered: not determined".into(),
            });
            line(match e.covers {
                "not_determined" => {
                    "   Coverage: not determined; validation stopped before it could tell whether changes follow the signature.".into()
                }
                "whole_file" => "   Coverage: the signature covers the whole file; nothing follows it.".into(),
                "changes_allowed" => {
                    format!("   Coverage: changes follow the signature, and it permits them ({}).", e.changes_after_signing.join(", "))
                }
                _ => format!("   Coverage: changes follow the signature, and it does not permit them ({}).", e.changes_after_signing.join(", ")),
            });
            match (e.certification_level, e.kind) {
                (Some(l), _) => line(format!("   Certification level: {l} ({})", level_words(l))),
                (None, "signature") => line("   Certification level: none (an approval signature)".into()),
                _ => {}
            }
            if let Some(p) = e.page {
                line(format!("   Page: {p}"));
            }
            if let Some(r) = &e.reason {
                line(format!("   Reason given: {r}"));
            }
            if let Some(l) = &e.location {
                line(format!("   Location given: {l}"));
            }
            for r in &e.reasons {
                line(format!("   - {r}"));
            }
            if e.text_cut {
                line(format!("   Some text above was cut at {MAX_TEXT_CHARS} characters."));
            }
            if e.reasons_cut {
                line(format!("   Only the first {MAX_DETAILS} explanatory sentences are listed; the rest are left out."));
            }
        }
        line(String::new());
        line("What was checked".into());
        for c in CHECKED {
            line(format!("- {c}"));
        }
        line(String::new());
        line("What was not checked, and what this report does not say".into());
        for c in NOT_CHECKED {
            line(format!("- {c}"));
        }
        t
    }
}

impl Session {
    /// The signature audit report of the open document `id`.
    pub fn signature_audit(&self, id: DocId) -> Result<AuditReport, EditError> {
        let doc = self.get(id).ok_or(EditError::NoDocument)?;
        // Without the structure no signature was looked for, and "none found" would be a guess.
        if doc.editor.is_none() {
            return Err(EditError::ReadOnly(doc.read_only_reason.clone().unwrap_or_default()));
        }
        Ok(audit(doc))
    }

    /// A new PDF holding `report` as plain text.
    pub fn signature_audit_pdf(&self, report: &AuditReport) -> Result<Arc<Vec<u8>>, EditError> {
        self.create_report("Signature audit report", &report.text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdfcraft_sign::{Detail, Fill};

    fn info(field: &str, status: Status) -> SignatureInfo {
        SignatureInfo {
            field: field.into(),
            signed: true,
            page: None,
            rect: None,
            visible: false,
            signer: None,
            certificate: None,
            chain: Vec::new(),
            date: None,
            signing_time: None,
            reason: None,
            location: None,
            contact: None,
            sub_filter: None,
            doc_timestamp: false,
            certify: None,
            revision: 1,
            signed_len: 100,
            digest: None,
            algorithm: None,
            timestamp: false,
            timestamp_time: None,
            status,
            modification: Modification::None,
            changes_checked: true,
            verdict_shown: false,
            details: Vec::new(),
        }
    }

    fn report_of(entry: AuditEntry) -> AuditReport {
        AuditReport {
            document: "d.pdf".into(),
            pages: 1,
            bytes: 1,
            sha256: String::new(),
            unsaved_changes: false,
            changed_on_open: false,
            document_cut: false,
            stand_in_counted: false,
            found: 1,
            signed: 1,
            valid: 0,
            unknown: 1,
            invalid: 0,
            trusted_certificates: 0,
            entries: vec![entry],
            truncated: false,
        }
    }

    #[test]
    fn a_document_timestamp_time_is_validated_only_when_the_status_is_valid_but_a_signatures_when_its_authority_is_trusted() {
        let mut e = entry(&info("T", Status::Unknown));
        e.kind = "document_timestamp";
        e.timestamp = true;
        e.timestamp_time = Some("2023-11-14 22:13:20 UTC".into());
        assert!(!report_of(e.clone()).text().contains("Timestamp: validated"));
        e.status = "valid";
        assert!(report_of(e.clone()).text().contains("Timestamp: validated"));
        // A signature's timestamp time is set only after the token's authority was trusted.
        e.kind = "signature";
        e.status = "unknown";
        assert!(report_of(e).text().contains("Timestamp: validated"));
    }

    #[test]
    fn a_report_of_a_document_without_its_structure_is_refused() {
        let mut s = Session::new();
        let bytes = s.create_blank(300.0, 400.0, 1).unwrap();
        let id = s.open("a.pdf", None, bytes, None).unwrap();
        assert!(s.signature_audit(id).is_ok());
        s.doc_mut(id).unwrap().editor = None;
        assert!(matches!(s.signature_audit(id), Err(EditError::ReadOnly(_))));
    }

    #[test]
    fn changes_the_signature_cannot_be_compared_with_are_not_reported_as_permitted() {
        let mut s = info("S", Status::Unknown);
        s.modification = Modification::Allowed(vec!["form fill"]);
        assert_eq!(entry(&s).covers, "changes_allowed");
        s.changes_checked = false;
        assert_eq!(entry(&s).covers, "not_determined");
    }

    #[test]
    fn only_the_listings_own_stand_in_row_is_called_unread_fields() {
        // A real signature field that happens to be called UnreadFields stays a signature.
        let mut real = info("UnreadFields", Status::Unknown);
        real.signed_len = 0;
        real.details =
            vec![Detail { message: "The signature could not be read.".into(), pattern: "The signature could not be read.", fill: Fill::Nothing }];
        assert_eq!(entry(&real).kind, "signature");
        let mut stand_in = real.clone();
        let sentence = "This form has too many fields, or a field name too long, for PdfCraft to read them all.";
        stand_in.details = vec![Detail { message: sentence.into(), pattern: sentence, fill: Fill::Nothing }];
        assert_eq!(entry(&stand_in).kind, "unread_fields");
        assert!(is_stand_in(&stand_in) && !is_stand_in(&real));
    }

    #[test]
    fn a_document_name_cut_at_the_limit_says_so() {
        let mut cut = false;
        let long = "n".repeat(MAX_TEXT_CHARS + 5);
        assert_eq!(clean(&long, &mut cut).chars().count(), MAX_TEXT_CHARS);
        assert!(cut);
        let mut r = report_of(entry(&info("S", Status::Unknown)));
        assert!(!r.text().contains("name was cut"));
        r.document_cut = true;
        assert!(r.text().contains("name was cut"));
    }
}
