# Requesting signatures: provider integration, server plan and local design

Status: proposal (2026-11-22). Nothing here is built. It follows the rules in `AGENTS.md`: Rust only,
every feature a headless tool first, secrets never in code or logs, our own wording for feature names
(the feature is **Request signatures**; providers are named only as integration targets).

## What exists today

- `crates/sign`: PAdES B-B (`ETSI.CAdES.detached`) signing, validation, RFC 3161 timestamps, DSS and
  revocation data, PKCS#12 and OS key stores (Windows, macOS keychain).
- Fill & Sign marks (typed, drawn and image signatures) and flatten on save.
- `signature_seed_values` (#166): a signature field can say which reasons, formats and digests are allowed.
- Document permissions and security handlers in `crates/crypt`.

What is missing: asking someone else to sign, tracking who has, and collecting the result.

## Goals

1. A user prepares a document (fields already exist in Prepare Form) and sends it to one or more signers.
2. Three ways to do that, behind one interface, so none is required:
   - **Local**: no server. Sign a file yourself, or pass a file around by hand (mail, share).
   - **PdfCraft server**: a self-hostable signing server (section "Server design").
   - **Third-party e-signature service**: a provider plug-in (established e-signature services; named here only as
     integration targets, never as our feature names). PdfCraft uploads, the provider runs the signing, PdfCraft downloads the
     signed file and its audit trail.
3. Everything works headless (CLI and MCP), the UI is a thin layer (AGENTS rule 14).
4. The signed result is a normal PDF that validates with `crates/sign`, whichever route made it.

Non-goals for the first release: qualified signatures with remote trust services, in-app video ID,
payment collection, bulk send.

## Provider integration option

One trait in a new crate `crates/esign` (below the UI layer, no egui):

```rust
pub trait SigningProvider {
    fn id(&self) -> &'static str;                      // "local", "pdfcraft-server", "<provider>"
    fn capabilities(&self) -> Capabilities;            // ordering, in-person, reminders, templates, audit trail
    fn create(&self, req: &Request) -> Result<Envelope, EsignError>;   // upload + recipients + fields
    fn status(&self, id: &EnvelopeId) -> Result<Status, EsignError>;
    fn download(&self, id: &EnvelopeId) -> Result<Completed, EsignError>; // signed PDF + certificate/audit PDF
    fn cancel(&self, id: &EnvelopeId) -> Result<(), EsignError>;
    fn remind(&self, id: &EnvelopeId, who: &RecipientId) -> Result<(), EsignError>;
}
```

- `Request` carries the document bytes, ordered recipients (name, email, role, order, auth level), the field
  placements read from the PDF's own form fields (name, page, rect, kind), a message, an expiry.
- Fields map by name: a signature field `Signature1` becomes the provider's signature tab. Fields the provider
  cannot express are listed back to the user before sending (AGENTS rule 15: no silent drops).
- Providers are separate modules or crates behind cargo features (`esign-<provider>`), so a build without them
  has no extra dependency and the licence rule (rule 26) is checked per provider SDK. Prefer plain HTTPS+JSON
  through `ureq`, which `crates/ai` already uses; no vendor SDK.
- **Authentication**: OAuth 2 authorization-code with PKCE on loopback for each provider, or an API key where
  the provider only offers one. Tokens live in the OS credential store (`crates/sign/src/keychain.rs` and `windows.rs` show the platform
  stores for signing IDs; a token store is new work); never
  in settings files, logs or the control channel. Refresh is automatic; a revoked token asks the user again.
- **Configuration** (`Preferences > Signing services`): add or remove a provider, choose a default, see
  connection state. Each provider has a data-region choice where it offers one.
- **Privacy**: the document and recipient addresses leave the machine only after the user presses Send in a
  dialog that names the provider, lists recipients and says what is uploaded. The headless tool needs an
  explicit `confirm_upload: true`. No provider is contacted at startup or by a background task.
- **Result handling**: the downloaded file is checked with `pdfcraft_sign::validate` and shown as
  *signed by provider*, with the provider's certificate and audit PDF kept as attachments or beside the file.
  A signature that fails validation is reported as such, not as success.
- **Webhooks**: a desktop app cannot receive them; it polls `status` while the dialog is open and on demand.
  The PdfCraft server (below) can receive provider webhooks and the app asks the server instead of the provider.

Headless tools (names provisional): `esign_providers`, `esign_connect`, `esign_send`, `esign_status`,
`esign_fetch`, `esign_cancel`, `esign_remind`. Each follows the existing conventions: typed arguments refused
whole before any change, one undo step where the document changes, results with a stable `status` enum
(`draft`, `sent`, `viewed`, `signed`, `completed`, `declined`, `expired`, `cancelled`, `failed`).

## Local design (built first)

This part needs no server and no network, and it is the foundation the others reuse.

1. **Signing plan document**: `Request` and `Envelope` as plain data (serde), stored in the document's
   attachments or in a sidecar `.pdfcraft-sign.json` (user choice, default sidecar so the PDF bytes stay
   untouched). Records recipients, order, field assignments, status history.
2. **Field assignment**: in Prepare Form, a field gets an *assigned to* recipient (new field property, stored
   in a private key of the widget so other readers ignore it). Field Properties gets a "Signer" combo.
3. **Sequential local signing**: "Sign as next recipient" opens the file, shows only the fields assigned to
   that recipient, signs with a chosen digital ID (existing PAdES path) or Fill & Sign mark, saves an
   incremental update, and advances the plan. Each step is a normal incremental save, so earlier signatures
   stay valid (`DocMDP`/field lock rules from `signature_field_lock` are honoured).
4. **Hand-off packages**: export a plan plus PDF as one zip for sending by mail or a shared folder; import
   brings the returned file back and checks that earlier signatures are still valid and that nothing outside
   the recipient's fields changed.
5. **Audit page**: optional appended page and a machine-readable audit record (who, when, digest of the file
   before and after each step, signing certificate fingerprint). Built from data we hold, not from claims.
6. **UI**: *Tools > Request signatures* wizard (document, recipients, order, message, route), a status panel
   per document, and the existing Fill & Sign for the "sign it myself" path.
7. **Tests first**: plan round-trip, order enforcement, a recipient cannot change other fields, earlier
   signatures survive the next step (the sign crate's validator is the oracle), refused calls write nothing.

## Server design (planned, after the local part)

A self-hostable server that makes "send for signature" work for people without the app, and receives provider
webhooks. Rust, a new workspace member `apps/pdfcraft-sign-server` (and `pdfcraft-cli sign-server` to run it
for a test). Not required: the app works without it.

**Roles**: sender (app or API client), signer (a browser, no install), administrator.

**Components**
- *HTTP API* (JSON over HTTPS): create envelope, upload document, add recipients, send, status, download,
  cancel, remind. Versioned (`/v1`), OpenAPI generated from the Rust types. Auth for senders by API key or
  OAuth client credentials; per-key scopes and rate limits.
- *Signing page*: a small web front end served by the same binary. Rust only for the app code (rule 11), so
  the page is server-rendered HTML with a small amount of static script for the signature pad; no JS framework,
  no build chain. PDF page images are rendered server-side with `crates/render`; signers never receive a
  working copy of the file until they finish. Mobile-first, keyboard accessible, works without cookies via a
  one-time signed token in the link.
- *Signer authentication levels*: link only; link plus emailed one-time code; link plus SMS code (via a
  pluggable gateway); link plus access code the sender passes on another channel. The level is recorded.
- *Signature production*: the server holds no signer private keys. For a "simple" electronic signature it
  applies the signer's drawn or typed mark and then **seals** the result with the organisation's PAdES
  certificate (`crates/sign`) and an RFC 3161 timestamp, so the document is tamper-evident and verifies in any
  reader. Where the signer brings their own digital ID, the browser signs the digest locally (WebCrypto or a
  small helper) and the server embeds the CMS blob; this is optional and later.
- *Audit trail*: append-only event log per envelope (created, sent, opened, identity step, viewed page, signed,
  declined), with IP, user agent, time source, document digests before and after. Exported as a certificate
  PDF appended or attached. Hash-chained so truncation shows.
- *Notifications*: email through SMTP (configurable), reminders, expiry, optional webhook to the sender.
- *Storage*: SQLite by default (single binary, backups by copy), PostgreSQL option. Documents are stored
  encrypted at rest with a per-envelope key wrapped by a server master key (file, OS store or KMS adapter);
  deleted after a retention period the administrator sets, with the audit record kept.
- *Provider bridge*: the server can itself be a `SigningProvider` client for a third-party service and relay
  webhooks to the app, so a desktop user gets one status feed regardless of provider.

**Deployment**: one static binary and one config file; container image and a systemd unit as examples. TLS by
reverse proxy or built in. No telemetry, no phoning home.

**Threat model (outline, to be expanded in an ADR)**
- Link theft: single-use, short-lived tokens, optional second factor, rate limits, lockout, token bound to
  recipient and envelope.
- Document tampering before signing: digest recorded at send, re-checked at each step.
- Replay or signing a different document than shown: the signed digest is the displayed one; the page shows
  the digest summary.
- Insider and operator access: documents encrypted at rest; audit log hash-chained and optionally shipped to
  an external sink.
- SSRF and file attacks: the server never fetches URLs from documents; uploads pass through the same hostile
  input limits as the rest of PdfCraft (parse in a bounded worker, catch_unwind guards).
- Data location and legal weight: the server states plainly that it produces **advanced-style sealed
  electronic signatures**, not qualified signatures, and what the evidence includes. Wording needs legal
  review before anything is called valid under a particular law (eIDAS, ESIGN, UETA).

**Phasing**
1. Local design (above) plus the `SigningProvider` trait with the `local` provider.
2. One third-party provider behind a feature flag, to prove the trait, OAuth and mapping.
3. Server: API, signing page, simple signatures, seal, audit, SQLite, email.
4. Server extras: SMS and access-code auth, PostgreSQL, KMS adapter, webhook bridge.
5. Own-certificate signing in the browser; templates; bulk send.

## Open questions for the owner

1. Which provider first? The choice drives the OAuth work, the field mapping and the licence check of any SDK.
2. Should the server be offered only self-hosted, or also run as a hosted service? Hosted adds legal, privacy
   and operations scope well beyond the code.
3. Is the sealed simple signature enough for the first server release, or must it support the signer's own
   certificate from day one?
4. Where should the plan live by default: a sidecar file, or inside the PDF as an attachment?
5. Region and language targets (the audit wording and the legal text need translation and review per region).

## Checks this design will need

`cargo xtask layers` (the new crate sits above `sign` and `forms`, below `ui-egui`), `cargo xtask assets`
and `cargo xtask parity` (new `sign.request-*` entries, starting as `planned`), `cargo xtask wasm`
(`crates/esign` provider code gated for wasm: no loopback listener there), and a hostile-input sweep entry
for every new tool.
