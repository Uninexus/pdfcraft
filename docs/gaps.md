# Where PdfCraft falls short of Acrobat Pro

> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (never crash, page boxes: a box is read as the viewer reads it, from its first four entries, which must be numbers, so one huge array shared by many pages costs a constant per page; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (never crash, ink aliases: DeviceN colorant entries are read from one allowance of 2,000,000, so spaces sharing one huge names or attributes array cannot cost spaces times size steps; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (gap 22: `ns_list` says whether the namespace a role-map pair points at is one the structure tree root lists; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (gap 18: the Print readiness dialog has fields for the ink limit, least bleed and least resolution, remembered across restarts and settable through `ui.set`; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (gap 19: Compare dialog Ignore checkboxes are remembered across restarts and settable through `ui.set`; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (gap 21: the Compare Files dialog has Ignore checkboxes for letter case and punctuation, the `doc_compare` options; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (gap 21: `doc_compare` takes `ignore_punctuation`, headless text comparison that does not count punctuation-only differences; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (gap 18: `print_readiness` takes a `min_ppi` for the image-resolution finding, headless only; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (never crash, XMP preflight: the namespace scope used to read a metadata packet is a shared chain, so a packet with hundreds of declarations and tens of thousands of items no longer copies the declarations per item; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (never crash, name trees: attachments, page templates and carried attachments copy at most 32 MiB of key text per tree and refuse an edit, or a Combine of a source whose pages are all included, past it, so pairs sharing one huge key string cannot build gigabytes of entries; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (never crash, named destinations on import: Combine, Insert, Extract and Split read the source's destination names once into an index instead of walking the tree per link, so a file of N links to M names no longer costs N x M steps; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (never crash, form values: a form's `/V` and `/DV` string, name and array text is spent from one 64 MiB allowance, and a field past it is flagged `value_omitted`, refused writes, kept by reset, left undrawn (including by property edits and XFA layout), kept by AF calculations and the XFA data layer, and left out of every export with the names reported, so fields sharing one huge array cannot each hold a copy; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (never crash, form choice fields: a form's fields share an allowance of 1,000,000 `/Opt` entries and 16 MiB of option text, and a combo or list box past it is flagged, refused writes and left undrawn, skipped by reset and recalculation, reported in the repair log and `form_fields`, so fields naming one shared array cannot cost fields times options; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (never crash, form actions: a `ResetForm` or `Hide` field list over 65,536 entries or 4 MiB of names is not read, so entries sharing a deep chain of long partial names cannot build gigabytes of names; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (never crash, named destinations: a `/Names` or `/Kids` array of a destination name tree that several nodes share is read once, so a small file cannot cost nodes times entries steps or use up the key allowance before its later names; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (never crash, bookmarks: a listing spends titles from a 16 MiB allowance and copies only the first six items of a destination array, so bookmarks sharing one huge title or array cannot make a listing gigabytes long; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (never crash, attachments: a `/Names` array of the attachment tree that several nodes share is read once, so a small file cannot list or rewrite hundreds of thousands of attachments; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (gap 15: `form_merge_data` refuses a table of more than 2,000,000 cells and counts a comma-holding field name as one column; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (gap 17: `redact_mark` pattern `nino` finds UK National Insurance numbers by format only, headless and in the UI pattern list; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (gap 18: `print_readiness` takes a `min_bleed_mm` for the page-bleed finding, headless only; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (gap 16: `doc_guide_snap` works out, headless, where a point lands when it snaps to a page's guides; the app does not snap yet; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (gap 21: `doc_compare` takes `ignore_case`, headless text comparison that does not count case-only differences; estimates unchanged)
> **Last reviewed:** 2026-11-19 | **Last updated:** 2026-11-19 | **Change:** minor (gap 22: `ns_list` reports `/NS` values that more than one structure namespace carries; estimates unchanged)
> **Last reviewed:** 2026-11-18 | **Last updated:** 2026-11-18 | **Change:** minor (gap 23: `ai_redaction_suggest` suggests passages to redact through the user's own AI endpoint, headless and suggest-only; estimates unchanged)
> **Last reviewed:** 2026-11-18 | **Last updated:** 2026-11-18 | **Change:** minor (gap 22: `ns_list` lists the PDF 2.0 structure namespaces read-only; estimates unchanged)
> **Last reviewed:** 2026-11-16 | **Last updated:** 2026-11-16 | **Change:** minor (gap 22: `dpart_list` lists the PDF 2.0 document part tree read-only; estimates unchanged)
> **Last reviewed:** 2026-11-16 | **Last updated:** 2026-11-16 | **Change:** minor (gap 17: `redact_mark` can leave the first or last letters or digits of a pattern or regex match readable, headless; estimates unchanged)
> **Last reviewed:** 2026-11-16 | **Last updated:** 2026-11-16 | **Change:** minor (gap 18: `print_readiness` takes an `ink_limit` for the total-ink finding, headless only; estimates unchanged)
> **Last reviewed:** 2026-11-16 | **Last updated:** 2026-11-16 | **Change:** minor (gap 21: `doc_compare` tells visual regions that overlap a comment annotation only one file has from other visual changes; estimates unchanged)
> **Last reviewed:** 2026-11-16 | **Last updated:** 2026-11-16 | **Change:** minor (gap 17: redact_mark patterns sin and ca-postal-code, Canadian Social Insurance Numbers and postal codes; estimates unchanged)
> **Last reviewed:** 2026-11-16 | **Last updated:** 2026-11-16 | **Change:** minor (gap 19: Preferences ▸ Forms gains a Required field border colour; estimates unchanged)
> **Last reviewed:** 2026-11-16 | **Last updated:** 2026-11-16 | **Change:** minor (gap 13: listing signatures no longer reads every page when a document has no signature field, and measured that each save keeps one more copy of the file in undo history; estimates unchanged)
> **Last reviewed:** 2026-11-16 | **Last updated:** 2026-11-16 | **Change:** minor (gap 8: `doc_create` from `office` makes a PDF from one Office document, headless; estimates unchanged)
> **Last reviewed:** 2026-11-14 | **Last updated:** 2026-11-14 | **Change:** minor (gap 21: Compare files can be limited to replaced, inserted, deleted and visual changes in the dialog, panel, report, Mark as comments and tools; gap 19: Preferences gains Identity (name, email) and Forms (highlight fields and colour, field tooltips); estimates unchanged)
> **Last reviewed:** 2026-11-13 | **Last updated:** 2026-11-13 | **Change:** minor (gap 21: the Compare Files dialog takes a page range for each file, and the report and Mark as comments follow them; estimates unchanged)
> **Last reviewed:** 2026-11-13 | **Last updated:** 2026-11-13 | **Change:** minor (gap 8: Insert pages, Combine files and Create ▸ Multiple files take Office documents through the installed LibreOffice; gap 8's first-beta sub-estimate re-estimated from 5–10 h to 3–6 h)
> **Last reviewed:** 2026-11-11 | **Last updated:** 2026-11-11 | **Change:** minor (gap 8: known residual risk for the LibreOffice sidecar, sandbox tracked in #6953; estimates unchanged)
> **Last reviewed:** 2026-10-28 | **Last updated:** 2026-10-28 | **Change:** minor (M10, #4119 partial: Enhance scanned file cleans scanned pages in place: background removal, edge shadow removal, despeckle; estimates unchanged)
> **Last reviewed:** 2026-10-25 · **Last updated:** 2026-10-25 · **Change:** minor (gaps 5 and 14 current state: native release Hans face via LXGW WenKai Lite; Arabic logical catalog, explicit RTL direction and mirrored stock chrome, with native/browser/IME workflows unqualified; no readiness re-estimate)
> **Last reviewed:** 2026-10-15 · **Last updated:** 2026-10-15 · **Change:** minor (gap 3: #2517 a blank line stays a paragraph break; estimates unchanged) · **Target:** Adobe Acrobat Pro (Acrobat DC, continuous track 26.002.21931, macOS)

> **Last reviewed:** 2026-10-15 · **Last updated:** 2026-10-19 · **Change:** minor (gap 2: composite fonts with `/ToUnicode /Identity-H` read as Unicode, hayro-interpret patch 44; estimates unchanged) · **Target:** Adobe Acrobat Pro (Acrobat DC, continuous track 26.002.21931, macOS)
> **Last reviewed:** 2026-10-17 · **Last updated:** 2026-10-17 · **Change:** minor (gap 6: #2662 redaction audit counts a cover over one word of a long line; estimates unchanged) · **Target:** Adobe Acrobat Pro (Acrobat DC, continuous track 26.002.21931, macOS)
> **Last reviewed:** 2026-10-22 · **Last updated:** 2026-10-22 · **Change:** minor (gap 3: #2517 a partial rewrite inside one text object keeps the next paragraph; estimates unchanged) · **Target:** Adobe Acrobat Pro (Acrobat DC, continuous track 26.002.21931, macOS)
> **Last reviewed:** 2026-10-18 · **Last updated:** 2026-10-18 · **Change:** minor (gap 3: #2517 a partial rewrite keeps a kept row inside the paragraph and leaves the next marked region closed; estimates unchanged) · **Target:** Adobe Acrobat Pro (Acrobat DC, continuous track 26.002.21931, macOS)
> **Last reviewed:** 2026-10-15 · **Last updated:** 2026-10-15 · **Change:** minor (gap 12: reading order is scored, not just word F1) · **Target:** Adobe Acrobat Pro (Acrobat DC, continuous track 26.002.21931, macOS)
> **Last reviewed:** 2026-10-14 · **Last updated:** 2026-10-14 · **Change:** minor (M1: raw-deflate fallback preserves decoding-limit errors; estimates unchanged) · **Target:** Adobe Acrobat Pro (Acrobat DC, continuous track 26.002.21931, macOS)
> **Last reviewed:** 2026-10-13 · **Last updated:** 2026-10-15 · **Change:** minor (gap 3: find and replace in Edit text mode, partial; estimates unchanged) · **Target:** Adobe Acrobat Pro (Acrobat DC, continuous track 26.002.21931, macOS)
> **Last reviewed:** 2026-10-12 · **Last updated:** 2026-10-13 · **Change:** minor (gap 14: Vietnamese catalog shipped; Hindi/Indonesian/Korean and Arabic RTL remain) · **Target:** Adobe Acrobat Pro (Acrobat DC, continuous track 26.002.21931, macOS)
> **Last reviewed:** 2026-10-13 · **Last updated:** 2026-10-13 · **Change:** minor (gap 6: #1910 Organize Pages opens on the current page; estimates unchanged) · **Target:** Adobe Acrobat Pro (Acrobat DC, continuous track 26.002.21931, macOS)
> **Last reviewed:** 2026-10-12 · **Last updated:** 2026-10-12 · **Change:** minor (gap 6: #1398 stamps read upright on rotated pages; estimates unchanged) · **Target:** Adobe Acrobat Pro (Acrobat DC, continuous track 26.002.21931, macOS)
> **Last reviewed:** 2026-10-12 · **Last updated:** 2026-10-12 · **Change:** minor (gap 6: #1317, #1327, #1302, #1315 and #1340; estimates unchanged) · **Target:** Adobe Acrobat Pro (Acrobat DC, continuous track 26.002.21931, macOS)
> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-12 · **Change:** minor (#1316: replacing a line keeps a neighbouring line whose glyph box overlaps; gap 3 remains) · **Target:** Adobe Acrobat Pro (Acrobat DC, continuous track 26.002.21931, macOS)
> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-12 · **Change:** gap 4 closed: the web build prints through the browser's print dialog; exclusive image-crop resize follow-up; minor (Word invoice regression coverage, #773; Office fidelity still partial; gap 2: hayro patch counts after the native-gradient backport) · **Target:** Adobe Acrobat Pro (Acrobat DC, continuous track 26.002.21931, macOS)
> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-12 · **Change:** minor (gap 13: large-file memory measured, edits keep two copies instead of three, file-backed input blocked on gap 2) · **Target:** Adobe Acrobat Pro (Acrobat DC, continuous track 26.002.21931, macOS)
> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-11 · **Change:** minor (gap 1 closed by #836; fixed issues removed from gaps 3, 6 and 16) · **Target:** Adobe Acrobat Pro (Acrobat DC, continuous track 26.002.21931, macOS)
> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-11 · **Change:** minor (document-font subset completion and curved/shared image clips in the contributor fork; gap 2: hayro patch counts after the native-gradient backport) · **Target:** Adobe Acrobat Pro (Acrobat DC, continuous track 26.002.21931, macOS)

Every known shortfall, one entry each, ranked by what it costs users. This is the work list: agents
choose from the top unless the owner says otherwise, and prefer these over new P2/P3 checklist
features. The numbers behind it are in [target-app-parity.md](target-app-parity.md); the per-feature
detail is in `parity/acrobat-features.toml` (`cargo xtask parity --partial` lists what each partial
feature lacks).

No gap blocks the [alpha gate](roadmap.md#alpha-gate); gap 8 holds the convert-and-share
workflow's partial sub-cases and is the first beta item. Gap numbers are kept when a gap closes, so
references elsewhere stay valid (gap 1, encrypted output Acrobat rejected, closed with #836; gap 4,
no printing on the web, closed by printing through the browser's print dialog).

Hours are Opus 5.5 agent wall-clock hours. "Doc" names the parity document the gap belongs to.
When a gap closes, delete its entry, note it in the [ROADMAP.md](../ROADMAP.md) progress log and
update the numbers.

## Summary

| # | Gap | Kind | Hours | Doc |
|---|---|---|---|---|
| 2 | Rendering is borrowed and its fidelity unmeasured | feature, spec | 110–190 | [pdf-spec-parity.md](pdf-spec-parity.md) |
| 3 | Editing existing text is fragile on real files | feature | 100–180 | [target-app-parity.md](target-app-parity.md) |
| 5 | Chinese UI lacks a web glyph face | localization | 6–12 | [localization-parity.md](localization-parity.md) |
| 6 | Open-issue backlog of wrong results in shipped features | stability | 45–80 | this file |
| 7 | OCR reads only unaccented Latin | feature | 40–75 | [file-format-parity.md](file-format-parity.md) |
| 8 | Office export weak, Office import basic | file format | 55–100 | [file-format-parity.md](file-format-parity.md) |
| 9 | No Preflight; PDF/A partial; PDF/X and PDF/UA absent | standards | 65–115 | [pdf-spec-parity.md](pdf-spec-parity.md) |
| 10 | Signatures: no PKCS #11, timestamp servers or online revocation | feature, hardware | 40–70 | [pdf-spec-parity.md](pdf-spec-parity.md) |
| 11 | Accessibility: checker only, no tagging tools | feature | 40–70 | [target-app-parity.md](target-app-parity.md) |
| 12 | No measured fidelity harness against Acrobat | quality | (in 2) | [target-app-parity.md](target-app-parity.md) |
| 13 | Performance budgets and lazy loading of very large files | performance | 25–45 | this file |
| 14 | Arabic RTL unqualified end to end; no Hindi, Indonesian, Korean | localization | 25–45 | [localization-parity.md](localization-parity.md) |
| 15 | Forms: JavaScript object model, actions, submit, XFA depth | feature | 40–75 | [pdf-spec-parity.md](pdf-spec-parity.md) |
| 16 | UI precision gaps (multi-select move, rulers, accelerators, windows) | UI/UX | 30–60 | [ui-parity.md](ui-parity.md) |
| 17 | Certificate security and other protect gaps | feature, spec | 15–30 | [pdf-spec-parity.md](pdf-spec-parity.md) |
| 18 | Print production tools absent | feature | 25–45 | [target-app-parity.md](target-app-parity.md) |
| 19 | Preferences mostly missing | UI/UX | 15–25 | [ui-parity.md](ui-parity.md) |
| 20 | Scanners, GPU page rendering, pen pressure, Read Out Loud | hardware | 25–50 | [hardware-parity.md](hardware-parity.md) |
| 21 | Compare: filters by Acrobat's finer kinds of change, page ranges, scanned documents | feature | 8–16 | [target-app-parity.md](target-app-parity.md) |
| 22 | PDF 2.0 extras, linearization, Arlington validation | spec | 20–35 | [pdf-spec-parity.md](pdf-spec-parity.md) |
| 23 | Local AI provider (summarize, ask, alt text, redaction suggestions) | AI | 20–40 | [target-app-parity.md](target-app-parity.md) |
| 24 | Checklist not diffed against Acrobat's own menus | measurement | 4–8 | [target-app-parity.md](target-app-parity.md) |

## The gaps

### 2. Rendering is borrowed and its fidelity unmeasured

- **Missing:** our own renderer (M2: `model` crate, font engine, DisplayList devices, ADR-0004), or
  an owner decision to adopt another engine; and any measurement of rendering against Acrobat.
- **Evidence:** pages are drawn by vendored `hayro` 0.7 with 26 local patches to `hayro-interpret`,
  15 to `hayro`, 3 to `hayro-jbig2` and 17 to `hayro-syntax` (`vendor/README.md`); `lopdf` still
  backs document inspection. All 7 rendering P0 features are `partial`. User reports: TikZ pattern
  fills drawn wrong ([#794](https://github.com/storytold/pdfcraft/issues/794)), blurry text at
  1080p/1440p ([#730](https://github.com/storytold/pdfcraft/issues/730)), fixed zoom sizes differ
  from Acrobat ([#739](https://github.com/storytold/pdfcraft/issues/739)). An outside author
  proposes his MIT/Apache renderer as the bridge
  ([#841](https://github.com/storytold/pdfcraft/issues/841)).
- **Impact:** every page a user sees. Wrong glyphs, shadings or blend modes erode trust faster than
  any missing feature.
- **Fix:** first a fidelity harness (gap 12), then the renderer: own devices (150–250 h in the old
  plan) or an adopted engine plus our own font and text layers.
- **Estimate:** 110–190 h (includes the harness). **Owner decision needed** on #841 vs M2.
  **Doc:** [pdf-spec-parity.md](pdf-spec-parity.md).

### 3. Editing existing text is fragile on real files

- **Fork progress (2026-10-11):** compatible words and subset-font spaces form lines;
  source fonts lacking a space glyph remain reusable; inline editing previews remove source
  glyphs on a clone. Aligned visual rows now form mixed-style regions (including matrix-scaled headings),
  with source-run styles preserved during editing and delete/save/reopen regressions. A private
  real-file acceptance check and opt-in UI regression supplement synthetic coverage.
  Embedded sfnt faces now render in the editor, including Mac format-6 subsets adapted in memory
  through the PDF Unicode map. Source baselines remain stable; draft frames expand/shrink with
  visible text, and `text_edit_layout` measures saved draft bounds without applying them.
  Page/path-clipped artwork bounds and text-first hit testing prevent large cropped images from
  blocking text. A separate private four-page regression opens all 73 detected text regions;
  curve/hole hit testing now evaluates clipping paths and fill rules. A clip the image owns
  moves with it across nested states and content streams; shared path clips stay where they are.
  Full matching document fonts complete absent subset glyphs (also beside a single-run
  paragraph's subset); a paragraph redrawn in a stand-in font for a letter its subset lacks is
  named in a notice with the letters, and `text_edit` can refuse that instead; explicit installed/file font
  selection, TrueType/OpenType-CFF embedding, disjoint style ranges, marked paragraph boundaries
  and single-face OpenType shaping/bidi have regression coverage. Actual Chinese keyboard
  insertion/deletion and save/reopen renders are checked on all four private pages.
  Legacy non-SFNT previews, mixed-font/style contextual runs, structure-tree fidelity and reliable
  semantic grouping on every untagged PDF remain open. Replacing one line or paragraph no longer
  removes a distinct neighbour whose glyph box overlaps (#1316); a second drawing on the same
  baseline is still replaced. A blank line stays a paragraph break. An explicit `TL` is the
  measured baseline gap, and growing a paragraph so its new row is one leading above the next
  one does not absorb that paragraph (#2517). A partial rewrite of the last paragraph keeps a
  source row inside that paragraph, and does not close a following marked region early. Two
  paragraphs that share one text object keep that boundary when the first is rewritten. A heading
  and smaller body lines (#1756) and cells in different columns (#1680, #2478) are still grouped
  by the same geometry. A row that a list item wraps onto stays with that item. Estimates are unchanged.

- **Missing:** robust in-place editing of text PdfCraft didn't write: subset fonts without the
  needed glyphs, CJK and right-to-left text, rotated text, lists, superscript/subscript, keeping
  structure tags, find and replace beyond plain paragraphs (form fields, comments, matches across
  paragraphs); vector and object editing; arrange and align.
- **Evidence:** `edit.text-edit` is partial (font reused or Helvetica substituted);
  `edit.rtl-cjk-editing`, `edit.rotated-text-editing`, `edit.keep-tags-on-edit`,
  `edit.vector-edit`, `edit.edit-object-tool` are planned. Users:
  [#766](https://github.com/storytold/pdfcraft/issues/766) (editing Arabic text),
  [#844](https://github.com/storytold/pdfcraft/issues/844) (several elements now move together on
  one page, not across pages),
  [#787](https://github.com/storytold/pdfcraft/issues/787) (move images between pages),
  [#791](https://github.com/storytold/pdfcraft/issues/791).
- **Impact:** "Edit PDF" is one of Acrobat Pro's top reasons to buy. Substituting Helvetica changes
  how a document looks.
- **Estimate:** 100–180 h (the longest feature pole, M7). **Doc:** [target-app-parity.md](target-app-parity.md).

### 5. Chinese UI lacks a web glyph face

- **Missing:** a Simplified Chinese (`Hans`) UI face in the web build and a working fallback. Native release builds embed LXGW WenKai Lite from the pinned craft-fonts native Simplified Chinese manifest.
- **Evidence:** [#826](https://github.com/storytold/pdfcraft/issues/826),
  [#688](https://github.com/storytold/pdfcraft/issues/688),
  [#728](https://github.com/storytold/pdfcraft/issues/728),
  [#758](https://github.com/storytold/pdfcraft/issues/758),
  [#689](https://github.com/storytold/pdfcraft/issues/689) (Flatpak) predate the native face; the
  web build still embeds only BIZ UDPGothic Regular plus the small Arabic, Hebrew, Thai and Devanagari faces.
  craft-fonts has Noto Sans CJK SC, but at 16.4 MB it is too large to embed whole; a web face needs
  a cut of it (storytold/craft-libs#213).
- **Impact:** the second-largest language group sees a broken interface in the web build although
  the catalog is 97% translated.
- **Estimate:** 6–12 h (cut CJK faces through craft-fonts-build plus the fallback chain). **Doc:**
  [localization-parity.md](localization-parity.md).

### 6. Open-issue backlog of wrong results in shipped features

- **Missing:** fixes for the ≈ 13 of ≈ 25 filed reports of features marked `shipped` that give wrong results still open,
  plus triage of ≈ 220 other open issues.
- **Evidence:** 245 open issues on 2026-10-10. The first batch of `fix(...)` reports was fixed by
  2026-10-10 (#799, #800, #807, #809, #810, #811, #815, #816, #817, #818, #819, #821). A later
  batch keeps a cloudy comment flatten cannot draw (#1317), leaves a refused text-box restyle or
  resize unchanged (#1327), keeps arrow defaults off plain lines (#1302), stops a custom text
  match on the separator instead of the next glyph (#1315), and states the vertical-text
  redaction refusal without claiming an overlap (#1340). Organize Pages opens on the page being
  read, selected and scrolled into view (#1910). Stamps placed on a rotated page read upright
  (#1398). Another batch keeps a spaced XFA radio value in the datasets (#1319), replaces only the
  selected pages when they are not adjacent (#1305), and leaves unrelated page streams in place
  when a figure is marked decorative (#1324). A black box over one word of a long line, and an
  unapplied redaction mark over it, audit as a cover instead of a clean bill (#2662). The rest of
  the backlog is untriaged.
- **Fix verified (2026-10-14):** raw-deflate fallback returns a decoding-limit error instead of
  accepting an empty partial stream when a misleading zlib header triggers a retry. Synthetic
  filter and COS stream regressions cover refusal at the limit and decoding within it.
- **Fix verified (2026-10-20, #3673):** Character encoding accepts Identity-H/V fonts
  whose Adobe CJK collection supplies the Unicode mapping, independently of predefined
  non-Identity CMaps. Unknown CMaps and foreign registries remain refused. Synthetic checker
  and read-only automation regressions cover the change; per-glyph validation and the
  separate embedded-font extraction gap remain open. No readiness or backlog re-count.
- **Impact:** every one is a user who lost trust. Crash reports outrank feature work
  (`AGENTS.md` §4).
- **Estimate:** 45–80 h (≈ 0.2–0.4 h per issue). **Doc:** this file.

### 7. OCR reads only unaccented Latin

- **Missing:** every other script and accented Latin; editable-text output; deskew, rotation,
  despeckle and background clean-up as options of Recognize text (Enhance scanned file removes
  the background and edge shadows on grey and RGB scans and despeckles those and 1-bit ones; it
  reports CMYK, indexed, JPEG 2000, JBIG2 and CCITT scans and scans drawn inside Form XObjects
  and leaves them as they are); MRC compression;
  pluggable engines; OCR in the
  browser; scanner input.
- **Evidence:** `ocr.ocr-languages` partial ("the models read only the Latin alphabet without
  accents"); 19 of 26 OCR features planned; [#744](https://github.com/storytold/pdfcraft/issues/744)
  (Dutch). Acrobat ships on-device recognition for dozens of languages.
- **Impact:** scanned documents are a core Pro workflow outside English.
- **Estimate:** 40–75 h, partly blocked on openly licensed models. **Doc:**
  [file-format-parity.md](file-format-parity.md).

### 8. Office export weak, Office import basic

- **Missing:** Word export that keeps layout; Excel and PowerPoint export; creating PDFs from HTML,
  web pages and PostScript; SVG/XML/EPS/PS/JPEG 2000 export. Office documents open from File ▸ Open,
  and Insert pages, Combine files and Create ▸ Multiple files take them too (desktop, through an
  installed LibreOffice, converted on a worker thread); headlessly `doc_create` from `office` makes a
  PDF from one document. Create ▸ Single file already converts one Office file in the app; there is no Create ▸ From Office menu entry, conversion settings or progress yet.
- **Evidence:** `create.export-docx` partial; the synthetic invoice regression for
  [#773](https://github.com/storytold/pdfcraft/issues/773) now covers absent inferred borders,
  floating watermark text and per-page section sizes. Exact font matching, vector rules and
  broader layout fidelity remain incomplete;
  `create.from-office` partial (LibreOffice sidecar; Writer and Calc never update linked content,
  but Impress has no such setting and the process's network access isn't sandboxed);
  `create.export-xlsx`, `create.export-pptx`,
  `create.from-html` planned.
- **Known residual risk:** conversion runs the user's installed LibreOffice. PdfCraft
  opens no connection itself. Macros are disabled, untrusted referer links are blocked,
  and Writer and Calc never update links. Impress has no link-update setting, and the
  `soffice` process isn't network-sandboxed, so linked content in presentations could be
  fetched. Sandboxing the converter is [#6953](https://github.com/storytold/pdfcraft/issues/6953).
- **Impact:** "Export PDF" and "Create PDF" are two of Acrobat's most used tools.
- **Gate:** lossy or missing sub-cases of the convert-and-share workflow (partial, not blocking).
  First beta items: the rest of Create PDF from Office (a Create ▸ From Office menu command, conversion
  settings, progress and cancel for large batches; 3–6 h) and a Word export that keeps layout (20–35 h).
- **Estimate:** 55–100 h. **Doc:** [file-format-parity.md](file-format-parity.md).

### 9. No Preflight; PDF/A partial; PDF/X and PDF/UA absent

- **Missing:** Preflight (profiles, single checks and fixups, results tree, reports, droplets);
  PDF/A-1, PDF/A-4 and conformance levels a/u; PDF/X verify and convert; PDF/UA-1/2 verify; PDF/E,
  PDF/VT; output-intent choice; transparency flattening.
- **Evidence:** 12 Preflight features and 9 standards features planned; `optimize.pdfa-validate`,
  `optimize.pdfa-convert` partial (2b/3b structural rules; CMYK documents get no output intent).
  Acrobat ships hundreds of built-in Preflight profiles.
- **Shipped slice:** `pdfx_verify` (headless, read-only) reports what a file declares as PDF/X
  (`GTS_PDFXVersion`, `GTS_PDFXConformance`, `pdfxid:`) and the structural conditions of PDF/X-1a,
  X-3 and X-4 it can decide from the object graph (identification, `GTS_PDFX` output intent,
  Trapped (True or False), TrimBox or ArtBox, file ID, encryption, embedded fonts, transparency for X-1a and X-3).
  It never claims conformance. `optimize.pdfx-validate` is partial: no dialog, no profile or colour
  checks, no X-5/X-6, and no conversion.
- **Impact:** blocks print shops, archives and accessibility compliance work.
- **Estimate:** 65–115 h. **Doc:** [pdf-spec-parity.md](pdf-spec-parity.md).

### 10. Signatures: no PKCS #11, timestamp servers or online revocation

- **Missing:** PKCS #11 tokens and smart cards on macOS and Linux; timestamp-server preferences;
  fetching OCSP/CRL online for LTV; FieldMDP and locking fields on signing; the OS trust store; EU
  Trusted Lists; signature graphics and saved appearances; compare signed version; signing in the
  browser.
- **Evidence:** `sign.pkcs11`, `sign.timestamp-servers`, `sign.field-mdp`, `sign.os-trust`,
  `sign.eutl` planned; `sign.pades-bt`, `sign.ltv`, `sign.ocsp-crl` partial (caller-supplied
  evidence). Users ask for Acrobat-like validation
  ([#824](https://github.com/storytold/pdfcraft/issues/824),
  [#775](https://github.com/storytold/pdfcraft/issues/775),
  [#718](https://github.com/storytold/pdfcraft/issues/718)).
- **Impact:** government and legal users sign with tokens and need B-LT/B-LTA.
- **Estimate:** 40–70 h; tokens need hardware to test. **Doc:** [pdf-spec-parity.md](pdf-spec-parity.md).

### 11. Accessibility: only text-only automatic structure creation

- **Missing:** apply auto-tag proposals to marked content and a valid parent tree, tag fields
  into structure trees with unusual layouts (only a direct Document root, whose ParentTree is a
  Nums array or a bounded Kids number tree, is extended),
  visual Tags, Order and Content panels, Reading Order tool, table editor, the Make Accessible
  guided window (only the headless tool exists), Read Out Loud; full keyboard operation of PdfCraft itself and a screen-reader audit.
- **Evidence:** all 32 checker rules, figure alt text, tag property and sibling order edits, and
  a bounded auto-tag preview exist. Apply writes valid structure and parent-tree links for
  text-only files, tagging headings by size and weight; it refuses mixed content and existing tags;
  `accessibility_make_accessible` applies the safe fixes (title, language, tab order) in one undoable
  step and reports what remains; `accessibility_reading_order` (headless, read-only) lists the tagged content in logical order, inline elements included, with type, page, MCID, text and inherited alternate text and language, bounded in items, nodes, depth, string and total output size; `a11y.keyboard-only` (P0) planned, `a11y.app-screen-reader` partial.
- **Impact:** most untagged files still need another editor to build their structure; PdfCraft itself is
  not yet usable without a mouse.
- **Estimate:** 40–70 h. **Doc:** [target-app-parity.md](target-app-parity.md).

### 12. No measured fidelity harness against Acrobat

- **Missing:** side-by-side comparison of rendering, text extraction, form calculation and
  signature validation against Acrobat on synthetic fixtures; an interop suite that opens every
  kind of output we write (encrypted ones above all, after #774) in pdf.js, MuPDF and Poppler as
  oracle processes.
- **Progress (2026-10-14):** rendering now has an oracle too. `cargo xtask render-oracle` compares
  page 1 of every corpus file against Poppler's `pdftoppm` and reports a disagreement rate: the
  share of pixels with no near match in the other renderer's 3×3 neighbourhood, which forgives
  antialiasing and subpixel placement but not missing or wrong content (`crates/oracle`, module
  `pixels`). **First reading, 250 files at 72 dpi, tolerance 8: 229 compared, median 0.89%, mean
  2.16%, p90 5.35%, 25 within 0.1%.** It reports rather than gates — the corpus is not fetched on
  CI, and a first reading of a borrowed renderer is a baseline to improve on, not a bar to clear.
  The worst results cluster on JBIG2 `bitmap-*` files (`hayro-jbig2` carries 3 local patches) and
  on ZapfDingbats, which points the renderer work at something specific for the first time.
- **Read the score as a disagreement, not a verdict (2026-10-14).** The worst file of the 250,
  `bug1552113.pdf` at 84%, turned out to be Poppler's defect: it paints the link's `/Border [0 0
  112]` as a 112-unit band over the two lines of text, and the fixture is named "this text should
  be visible". Ours is the readable rendering. Anything at the top of the list needs a person to
  look at both images before it is treated as a bug of ours.
- **The JBIG2 cluster was Poppler too, and the suite checks itself (2026-10-14).** The next four
  worst files were JBIG2, which looked like a defect in `hayro-jbig2` and its local patches. It is
  the reverse. The 96 `bitmap-*.pdf` fixtures are a conformance suite: one picture encoded through
  every corner of JBIG2, so a correct decoder renders all 96 identically — a real test needing no
  reference image and no oracle. **We render 1 distinct image across all 96; Poppler renders 8**,
  drawing a blank page for the seven halftone-skip and symbol fixtures and mis-decoding TPGRON
  refinement (its output changes when TPGRON is switched on, which a correct decoder's cannot).
  Pinned as `crates/render/tests/it/jbig2_invariants.rs`. **Poppler is not a usable oracle for JBIG2**,
  and roughly a fifth of its fixtures should be read as its bugs rather than ours.
- **Annotations are shown, and that was measured rather than argued.** Poppler synthesises an
  appearance for an annotation that has none, where we draw only what the document carries (link
  borders without `/AP` are recorded as not covered under `view.render-annotation-appearances`), so
  showing them scores us for a known gap. Hiding them is worse, because it then scores Poppler for
  every appearance we *do* draw: over the same 250 files, shown gives mean 2.16% / p90 5.35% / 25
  files within 0.1%, hidden gives 2.18% / 5.55% / 22, with `annotation-tx3.pdf` jumping to 45.64%.
  The biases nearly cancel; `--hide-annotations` tells the two apart for one file at a time.
- **The text score was blind to reading order (2026-10-15).** Word F1 is a bag of words, so a page
  emitted back to front scores a perfect 1.000 — and reading order is most of what text extraction
  has left to get wrong (#2387, #2586). `cargo xtask text-oracle` now reports a second score: the
  longest common subsequence of the two word sequences over the longer one, both cut to 3,000
  words. The hole was not hypothetical. Measuring #2431, a change that moves text and alters
  nothing else, **word F1 called 26 of the 29 files whose output changed identical**, where the
  order score separated 18 improvements from 6 regressions and named their shape: forms, where a
  checkbox's number was parted from its label (`prefilled_f1040.pdf` 0.672 → 0.481). Read it as a
  disagreement, like the pixel score: the same split scored as a win on one file and a loss on
  another purely because `pdftotext` makes it in one place and not the other. The corpus-wide
  first reading has not been taken yet.
- **Evidence:** text extraction (word-F1 0.98 against `pdftotext`) and now rendering (above); qpdf
  checks written output on every CI run since `PDFCRAFT_ORACLES_REQUIRED`. Acrobat captures must
  stay local in `plan/acrobat/` (clean-room, `AGENTS.md` §1.1).
- **Still missing:** Acrobat itself as the reference rather than other open renderers; form
  calculation and signature validation; the interop suite that opens every kind of output we write
  (encrypted ones above all, after #774) in pdf.js and MuPDF; and 15 of those 250 files could not
  be compared at all because the two renderers round the page box to different pixel sizes.
- **Impact:** until it is complete, quality numbers outside rendering and text are judgements.
- **Estimate:** counted in gap 2 (≈ 20–30 h of it; roughly a third done).
  **Doc:** [target-app-parity.md](target-app-parity.md).

### 13. Performance budgets and lazy loading of very large files

- **Missing:** `misc.performance-budgets` (P0) and `core.lazy-loading` (P0): open GB files within a
  memory budget, with budgets checked on Tier-1 platforms.
- **Evidence:** #307 brought a 9,156-page file from > 10 GiB to ≈ 500 MiB inspection, but
  file-backed input and global memory budgets remain open; blurry fonts at common resolutions
  (#730). Measured 2026-10-12 on synthetic files (release build, Linux): a 1 GiB file opens at
  1.0 GiB peak in 0.7 s, so viewing holds the whole file once; after edits it held two copies and
  peaked at three (3.1 GiB after five edits), because the previous working file stayed alive while
  the next was written. Edits now let go of it first: five edits peak at 2.1 GiB (8.5 s, was 16 s).
  Saving still peaks at three copies (the saved file is assembled in memory beside the working
  one), and display appearances for comments without one add a copy. A 100,000-page file (28 MB)
  opens at 403 MiB in 5.5 s and each edit re-inspects it for ≈ 5 s.
  Measured 2026-11-16 (release build, Linux, synthetic files, `pdfcraft-cli run --script`, peak
  resident set from `getrusage`): a 400 MiB file with one 400 MiB stream opens at 416 MiB, peaks at
  816 MiB after a page rotation and at 1,218 MiB when that edit is saved (three copies), whether
  the save is incremental (+354 bytes) or a full rewrite to a new path. The writer already
  allocates the saved file once at its exact size, so that third copy is the saved file itself.
  What remains after a save is worse: every save leaves one more full copy alive, because the
  undo history keeps the cos state of each earlier file and each state holds its own bytes. Five
  rotate-and-save rounds on the same file peaked at 2,817 MiB, seven copies. Rebasing the history
  onto the newest saved bytes needs a rebase call in `craft-pdf` (a saved incremental file starts
  with the old one, so offsets stay valid); until then a long session on a large file pays one
  file per save.
  Re-inspection after an edit, measured on a 100,000-page file (21 MB, one rotation per edit):
  ≈ 1.75 s inside the refresh (writing the revision 4 ms, inspecting 0.87 s, form fields 0.37 s,
  signature listing 0.25 s, display copy 0.26 s) plus the edit itself, ≈ 3.2 s per edit in all.
  The signature listing walked every page to map annotations to pages even when the document has
  no signature field; it now reads the pages only for a signature field (same rows), which takes
  that step from ≈ 0.25 s to under a millisecond (refresh 1.78 s to 1.62 s, mean of three runs,
  ±0.1 s noise). Inspection (hayro and the lazy structure walk), form fields and the display
  copy each still read every page on every full refresh.
- **Blocked part:** reading a GB file from disk on demand (`ByteSource`) needs every reader to
  take ranges, but the `hayro` renderer parses one contiguous buffer (`Pdf::new(PdfData)`), so it
  waits on gap 2's renderer decision. Until then the budget is one copy of the file to view and two
  to edit.
- **Estimate:** 25–45 h. **Doc:** this file.

### 14. Arabic RTL unqualified end to end; no Hindi, Indonesian, Korean

- **Missing:** end-to-end qualification of right-to-left interface workflows on native platforms
  and in the browser (typed RTL text, IME composition, clipboard/selection and complex multi-line
  bidirectional text); catalogs for Hindi, Indonesian and Korean (Acrobat ships Korean; it ships
  Arabic in its Middle East edition; it ships none of Hindi or Indonesian). The logical catalog,
  explicit right-to-left label direction and mirrored stock chrome are implemented and
  unit-tested; complex RTL workflows are not yet accepted.
  Vietnamese ships as a catalog (`vi`, 100% of the current localizable union) but stays `partial`
  like other non-English languages (no translated help; engine/OS details stay English).
- **Evidence:** [localization.md](localization.md) §Arabic; `misc.rtl-ui` planned (end-to-end
  qualification pending).
- **Estimate:** 25–45 h (Arabic RTL 20–35, three catalogs 8–12 plus review). **Doc:**
  [localization-parity.md](localization-parity.md).

### 15. Forms: JavaScript object model, actions, submit, XFA depth

- **Missing:** the wider Acrobat JavaScript object model (annotations, layers, dialogs), document
  actions, the Actions tab triggers that run JavaScript, form submission, the debugger, barcode
  fields, rich-text values; XFA flattening and complete dynamic-form fidelity. Auto-complete
  from previous entries is basic only: no Advanced mode (Acrobat's second level), and nothing is
  remembered from date, number or formatted fields. Edit all JavaScripts: Edit ▸ JavaScripts…
  lists every script of a document (named, open action, document, field, link, annotation,
  page and bookmark actions) with filters, and edits or deletes one by id, undoably and without
  running it (`doc_javascripts` and `doc_javascript_edit` do the same headlessly); code over
  64 KiB is shown cut and can only be deleted. XFA scripts are not listed, and there is no
  syntax colouring, search and replace across scripts or single combined editor.
- **Evidence:** `form.edit-all-javascripts` partial, `form.js-host` partial, `form.document-actions`, `form.submit-form`,
  `edit.action-javascript`, `form.js-debugger`, `form.xfa-flatten` planned; `form.auto-complete` partial; XFA layout, FormCalc
  and data partial.
- **Estimate:** 40–75 h. **Doc:** [pdf-spec-parity.md](pdf-spec-parity.md).

### 16. UI precision gaps

- **Missing:** moving several selected objects across pages (same-page moves shipped, #844), single-key accelerators for the remaining tools (V, H, and the opt-in Z and G shipped),
  rulers and grids (ruler guides are in as a headless slice: `doc_guides`, `doc_guide_add`, `doc_guide_remove`, per page and
  session-only, `doc_guide_snap` works out headlessly where a point lands on the nearest guide within a tolerance, and View ▸ Guides ▸ Show Guides draws them over the pages and Clear Guides removes them; the app has no rulers to drag them from and does not snap to them), loupe and dynamic zoom, multiple windows and spreadsheet split (split
  view shipped), comment
  pop-ups that can be moved, connector lines, customisable quick tools; shortcut display and tool-mode bugs (#746, #721), macOS window controls
  (#789).
- **Estimate:** 30–60 h. **Doc:** [ui-parity.md](ui-parity.md).

### 17. Certificate security and other protect gaps

- **Missing:** encrypting for certificate recipients (and opening such files), security policies,
  attachment-only encryption, redaction pattern locales (Canadian SIN and postal codes and UK National Insurance numbers are in), partial-word redaction in the app
  (`redact_mark` has `keep_first` and `keep_last` headlessly), folder redaction, protected view,
  URL and attachment trust.
- **Estimate:** 15–30 h. **Doc:** [pdf-spec-parity.md](pdf-spec-parity.md).

### 18. Print production tools absent

- **Missing:** Output Preview, separations, overprint preview, the ink manager window,
  colour conversion, fix hairlines, flattener preview, colour management settings;
  print as image; PostScript output. Printer marks are in (Print production ▸ Add printer marks…).
  colour conversion, printer marks, flattener preview, colour management settings;
  print as image; PostScript output. Fix hairlines is in (Print production ▸ Fix hairlines…).
  Total area coverage and the overprint, rich-black text and spot-colour notes are in as a first slice
  (`print_readiness`, the ink findings): painted vector CMYK colours only, not those inside images,
  shadings, patterns or annotation appearances, with settable thresholds (`print_readiness` `ink_limit`, 100 to 400 percent, default 300; likewise `min_bleed_mm`, 0 to 50, default 2, for the bleed warning, and `min_ppi`, 36 to 1200, default 300, for the image-resolution warning; the Print readiness dialog has a field for each, remembered, and `ui.set readiness-ink-limit`, `readiness-min-bleed-mm` and `readiness-min-ppi` set them).
  Ink aliases are in as a headless slice (`doc_inks`, `doc_ink_aliases`: spot colorants listed and renamed
  so two inks print as one), with no Ink Manager window yet.
  Page boxes can be read back headlessly (`page_boxes`: all five boxes, which the file sets, the rotation and the
  bleed past the trim), which print checks need; there is still no UI for it beyond Set Page Boxes.
  Print with a comments summary is in as a first slice (Print ▸ Include a summary of the comments;
  `doc_print` `comment_summary`): comments-only summary pages for the printed pages follow the sheets, not yet
  in the preview, with no sort choice in the dialog, and not with Booklet, Poster, Label, a selected area, Print as
  image, a copy-restricted or low-resolution-only document, or in a browser.
- **Evidence:** 17 print-production features planned (L area).
- **Estimate:** 25–45 h (part of the L area's 30–55). **Doc:** [target-app-parity.md](target-app-parity.md).

### 19. Preferences mostly missing

- **Missing:** most of Acrobat's twenty or so preference pages (General, Documents, Commenting, Forms, Identity,
  Signatures, Security, Measuring, Units and guides, Convert, …). Today: Interface language,
  Documents and view, Units and paper, Identity (name, email), Comment, Forms (highlight fields,
  highlight colour, required-field border colour, field tooltips, Auto-Complete Off or Basic), Fill & Sign, JavaScript. Identity has no
  login name, title or organization: nothing in PdfCraft reads them yet. Forms has no Advanced
  auto-complete, spelling or calculation-order options.
- **Estimate:** 15–25 h. **Doc:** [ui-parity.md](ui-parity.md).

### 20. Scanners, GPU page rendering, pen pressure, Read Out Loud

- **Missing:** TWAIN/WIA/ICA/SANE scanning; GPU-accelerated page rendering (pages are rasterized on
  the CPU and uploaded); pressure-sensitive ink; platform text-to-speech.
- **Estimate:** 25–50 h; scanners need hardware. **Doc:** [hardware-parity.md](hardware-parity.md).

### 21. Compare: filters by Acrobat's finer kinds of change, page ranges in the dialog, scanned documents

- **Done:** the synchronised side-by-side view; the Compare panel's Show: Text / Visual filter
  (Text hides the text changes from the list and the shading, Visual hides the visual differences
  from the shading; they are counted, not listed); a page range for each document in the engine
  and `doc_compare` (`doc_from`/`doc_to`, `other_from`/`other_to`); the Compare Files dialog's Old
  file pages and New file pages fields (one page or one range each, empty for the whole file),
  which the comparison, the panel, the report and Mark as comments all use, as do
  `doc_compare_report` and `doc_compare_mark`. Results and the report name each file's own pages.
  A filter by kind of change: the Compare Files dialog's Kinds of change checkboxes and the panel's
  Show checkboxes (Replaced, Inserted, Deleted, Visual), which the list, the shading, Mark as
  comments and the report follow, and `kinds` on `doc_compare`, `doc_compare_report` and
  `doc_compare_mark` (bad values are refused, listing the allowed ones). A report of a filter says
  what it hid and that the finer kinds are not told apart. `doc_compare` with `visual` also tells
  each visual region's cause: `annotation` when it overlaps (by `/Rect`) a comment (markup)
  annotation that only one of the two files has on that page (added, removed, moved or edited),
  otherwise `other`; form fields, links, popups and media are not looked at, and at most 2,000
  comments per page are matched. `doc_compare` takes `ignore_case` (words that differ only in
  letter case are not text changes; text comparison only, headless only).
- **Missing:** telling images, formatting and background/header-footer changes apart, and using
  the annotation cause as a filter, in the dialog, the report or Mark as comments (the comparison
  reads words and pixels, so all of them are Visual, which is shaded and counted
  but never listed or marked; the dialog and the report say so; Acrobat's defaults with
  annotations, formatting and background off cannot be matched; an annotation present in both
  files but drawn differently, or a change under an unchanged annotation's box, is not seen as an
  annotation change; form-field and link changes are not told apart either), scrolling the two halves by matching content rather than by
  offset, scanned documents. A range is one contiguous run of pages per file (the dialog's old
  Pages field also took a list such as `1,3`).
- **Estimate:** 8–16 h. **Doc:** [target-app-parity.md](target-app-parity.md).

### 22. PDF 2.0 extras, linearization, Arlington validation

- **Missing:** UTF-8 text strings, associated files (`/AF`), document part editing, structure namespaces;
  linearized save (Fast Web View); deterministic save; structural validation against the Arlington
  model.
- **Evidence:** `core.linearization` is partial: `doc_linearization` (read-only) reports a file's
  linearization parameter dictionary and whether it still holds (the file's length against `/L`,
  hint streams and offsets inside the file); a linearized file saved incrementally reads as no
  longer valid. Hint tables, object order and writing a linearized file are not done.
  `core.pdf20-associated-files` is partial: `af_list` reads the `/AF` arrays of the catalog and of
  every page; `af_add` and `af_remove` add and remove a document-level associated file (an embedded
  Filespec with `/AFRelationship`, in the catalog `/AF` and the attachments; one undo step, refusing
  unknown relationships, EncryptedPayload, a non-array `/AF`, over 256 entries or 32 MiB; listing capped at 2,000 rows; the attachment of a removed file stays while another `/AF` lists it). Page,
  annotation and other-object writing, EncryptedPayload and any UI are not done.
  `core.pdf20-dpart` is partial: `dpart_list` (read-only) lists the document part tree under
  `/DPartRoot` (node, parent, depth, `/Start` and `/End` as pages, child count, `/DPM` entries as short
  text; at most 5,000 nodes and 64 levels, each node read once). The tree is not validated, and
  creating or editing parts and any UI are not done.
  `core.pdf20-namespaces` is partial: `ns_list` (read-only) lists the namespace dictionaries in the
  structure tree root's `/Namespaces` (object, `/NS`, whether `/Schema` is present, `/RoleMapNS`
  count and its first 64 entries; at most 1,000 namespaces; the same reference listed twice is listed once). Repeated `/NS` values are reported (`duplicate_ns`, compared as listed text), and each `[name namespace]` role pair says whether its namespace dictionary is one `/Namespaces` lists (`role_namespace_listed`, `unlisted_role_namespaces`). Nothing else is validated (use by
  structure elements, other role map consistency); adding or editing namespaces and any UI are not done.
- **Estimate:** 20–35 h. **Doc:** [pdf-spec-parity.md](pdf-spec-parity.md).

### 23. Local AI provider

- **Missing:** the opt-in provider interface (local model or the user's own endpoint) is in, with
  summarize, ask with page citations and translate. Redaction suggestions are in headlessly only
  (`ai_redaction_suggest`, suggest-only, no UI); alternate-text suggestions are missing.
  Acrobat's AI Assistant is a cloud service and out of scope.
- **Estimate:** 20–40 h. **Doc:** [target-app-parity.md](target-app-parity.md).

### 24. Checklist not diffed against Acrobat's own menus

- **Missing:** a menu and tool listing captured from Acrobat (names only, `plan/acrobat/`, never
  committed) diffed against the checklist, so that features the checklist never listed show up.
  `misc.localization` and `misc.installers` are now `partial` (Chinese, Telugu and Arabic are not
  fully usable yet; Linux packages are unsigned).
- **Estimate:** 4–8 h. **Doc:** [target-app-parity.md](target-app-parity.md).

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-19 | minor | never crash, page boxes: a page box (media, crop, bleed, trim, art; direct, indirect or inherited) is read as the viewer reads a rectangle: its first four entries, each resolved, which must all be numbers; a non-number among them, or fewer than four entries, is no box, and no later entry is read. So one array shared by every page (for example 1,000,000 entries over 100,000 pages) costs at most four entries per box instead of its length per page and box. Applies to `page_boxes`, `page_set_box` and the Set Page Boxes dialog (`rect_of` in organize), the printer-marks box reader (`pdfcraft_organize_boxes`, edit) and the preflight page-size check (`page_box_out_of_range`, PDF/A 6.1.13). What is charged: nothing is spent from an allowance; the bound is four entries read per box. Behaviour: an array of exactly four numbers reads as before; an array of more than four entries whose first four are numbers, which the old readers refused (organize reported it `ignored`, edit defaulted it, preflight skipped its size check), now reads as its first four, and preflight now flags such a box that is too small or large; an array whose first four are not all numbers, but which holds four numbers among more entries, used to read and is now no box, as in the viewer. Not covered: the page-tree walk still clones each page's dictionary and copies a /Pages node's inherited values into each page, so a huge direct inherited /MediaBox, /CropBox or /Resources over many pages is still copied per page; other array readers (annotation /Rect in annot, redaction verify extract) are unchanged. Tested: `a_box_is_its_first_four_entries_all_numbers_as_the_viewer_reads_it` (organize), `printer_marks_read_a_long_box_array_from_its_first_four_entries` (edit) and the new lines of `implementation_limits_are_reported` (preflight). Ran: cargo fmt --check; cargo clippy -p pdfcraft-organize -p pdfcraft-edit -p pdfcraft-preflight -p pdfcraft-automation --all-targets -- -D warnings -A clippy::collapsible_match; cargo test --no-fail-fast -p pdfcraft-organize -p pdfcraft-edit -p pdfcraft-preflight -p pdfcraft-automation; cargo xtask layers; cargo xtask assets; cargo xtask parity; cargo xtask wasm. Failed, not related: the known root-only batch_failures::read_only_output_folder_publishes_nothing (automation 554 of 555 passed; organize 268, edit 670 lib tests and preflight 91 passed). Each new test was also run against the previous reader and failed there. No hand-run test. Estimates unchanged. |
| 2026-11-19 | minor | never crash, ink aliases: `doc_inks` and `doc_ink_aliases` read DeviceN colorant entries from one allowance of 2,000,000 per reading or pass, a names array or dictionary shared by many spaces counting once for each space that names it, so a small file of thousands of spaces over one array of a million names, or one padded attributes dictionary, can no longer cost billions of steps. `doc_inks` charges only each DeviceN space's names array; once the allowance is gone it leaves the remaining DeviceN spaces unread and says so in `unread_spaces` (their inks are missing and counts may be low). `doc_ink_aliases` charges the names, the colorants dictionary keys, the solidities and dot gain keys, the printing order, and the attributes and mixing hints dictionaries before copying them, and refuses the whole call with nothing written when a pass runs out. Engine: `pdfcraft_edit::list_inks` returns `InkList { inks, unread_spaces }`. Tested: `spaces_sharing_one_big_names_array_spend_the_allowance_once_each`, `aliasing_refuses_when_the_spaces_would_cost_more_than_the_allowance_and_writes_nothing` (drives `alias_inks` and compares the document before and after) and `spaces_sharing_one_padded_attributes_or_hints_dictionary_are_charged_for_each_copy` (edit; each shown failing with its check disabled), and `unread_spaces` 0 asserted in the automation ink test. Ran: cargo fmt --check; cargo clippy -p pdfcraft-edit -p pdfcraft-automation -p pdfcraft-engine --all-targets -- -D warnings -A clippy::collapsible_match; cargo test --no-fail-fast -p pdfcraft-edit -p pdfcraft-automation -p pdfcraft-engine; cargo xtask layers; cargo xtask assets; cargo xtask parity; cargo xtask wasm. Failed, not related: the known root-only batch_failures::read_only_output_folder_publishes_nothing (automation 554 of 555 passed; edit 669 lib tests and engine 876 passed, 2 ignored). Estimates unchanged. |
| 2026-11-19 | minor | Gap 22: `ns_list` says, for each `[name namespace]` role pair, whether the dictionary it points at is one the structure tree root's `/Namespaces` lists (`role_namespace_object`, `role_namespace_listed`; `unlisted_role_namespaces` counts the `false` ones among the at most 64 roles listed per namespace, over the namespaces read (not document-wide); unknown when the list was cut at 1,000 and the target was not read). Read only. Tested: `role_pairs_say_whether_their_namespace_is_listed` and `structure_namespaces::role_pairs_naming_an_unlisted_namespace_are_reported_through_the_tool`. Ran: cargo fmt --check; cargo clippy -p pdfcraft-organize -p pdfcraft-automation -p pdfcraft-engine --all-targets -- -D warnings -A clippy::collapsible_match; cargo test --no-fail-fast -p pdfcraft-organize -p pdfcraft-automation -p pdfcraft-engine; cargo xtask layers; cargo xtask assets; cargo xtask parity; cargo xtask wasm. Failed, not related: the known root-only `batch_failures::read_only_output_folder_publishes_nothing` (automation 554 of 555 passed; organize 267 and engine 876 passed, 2 ignored). Estimates unchanged. |
| 2026-11-19 | minor | Gap 18: the Print readiness dialog has three fields, Total ink limit (%), Least bleed (mm) and Least resolution (ppi), for the `ink_limit`, `min_bleed_mm` and `min_ppi` of `print_readiness_with_options`. An empty field is the default (the engine's own, so the report and its wording are exactly what they were). A typed value is applied with Enter or when the field loses the keyboard, not on every keystroke, because applying measures the document again on the UI thread (the PDF/A check and every page and image); the typed text stays in the field and the line under it says `Press Enter to apply` while it waits. Each field has one line under it that is always there, so the dialog does not move: the allowed range and default, in red the reason when the text was refused (a value that is not a number, NaN, infinite or outside the engine's range: ink 100 to 400, bleed 0 to 50 mm, ppi 36 to 1200, from its own `TAC_LIMIT_RANGE`, `MIN_BLEED_RANGE` and `MIN_PPI_RANGE`; a refusal changes nothing and says to use digits 0-9). A comma is accepted as the decimal mark, full-width digits are read as digits, and -0 is 0. Reset to defaults (disabled at the defaults) clears all three. The findings, the Measured lines and Copy report (which gets a line naming each changed threshold) all come from the one measuring that follows the options; the engine's default constants (`TAC_LIMIT_PERCENT`, `MIN_BLEED_MM`, `WARN_PPI`) are now public and give the field hints. The report's scroll area is capped by the window height, so Close and Copy report stay in reach on a short window. The values are remembered in the settings JSON (`print_readiness` with `ink_limit`, `min_bleed_mm` and `min_ppi`; a default one is not written, each key is read alone, and a missing, mistyped or out-of-range saved value keeps the current choice). Headless: `ui.set readiness-ink-limit`, `readiness-min-bleed-mm` and `readiness-min-ppi` (a number or `default`; anything else is refused with `<key> must be a number from X to Y, or default` and changes nothing; an open report is measured again once; setting the value a field already has still puts its text right) and `ui.state.print_readiness` (each a number, or null for the default); the control docs say so. If the engine ever refused a threshold the error is shown, not replaced by a default report. The new labels and messages use `tl!` and are English only: translations are rows in craft-i18n and show once `craft-i18n.rev` moves, and a missing row falls back to English. The tools are unchanged (`print_readiness` already took all three). Tested: `print_readiness::every_threshold_changes_the_findings_the_measured_lines_and_copy_report`, `the_dialog_measures_with_the_default_thresholds_until_one_is_set`, `refused_thresholds_change_nothing_and_say_what_is_allowed`, `a_typed_field_applies_when_valid_and_is_refused_visibly_when_not`, `typing_into_the_dialog_fields_applies_on_enter_or_leaving_and_a_bad_value_is_refused_on_screen`, `on_a_short_window_the_report_scrolls_and_close_stays_in_reach`, `typed_numbers_accept_a_comma_and_full_width_digits_and_read_minus_zero_as_zero`, `control_refusals_name_the_key_and_an_unchanged_set_clears_a_refused_text`, `the_thresholds_are_remembered_each_alone_and_bad_saved_values_keep_the_current_choice` and `control::print_readiness_thresholds_are_set_and_reported_through_the_control_channel`; the report-follows-options, typed-field, Enter/leave typing, short-window, number-forms, control-refusal and remembered tests were shown failing with the report measured without the options, a keystroke applying, the short-window cap, the number normalising, the key error and the restore removed. The existing ink-findings and fonts tests now wait for Close to stop moving (a `settle` helper) instead of a fixed frame count. Known limits: no Preferences page control; the dialog drawn by a GPU was not run (the dialog tests use the kittest harness without a GPU); a threshold change still measures the whole document again (the ink limit is applied inside the engine's measuring, so only the apply moment was changed, not a separate judging step). Ran: cargo fmt --check; cargo clippy -p pdfcraft-ui-egui -p pdfcraft-engine --all-targets -- -D warnings -A clippy::collapsible_match; cargo test -p pdfcraft-ui-egui --test it print_readiness (14 passed); cargo test -p pdfcraft-engine printcheck; cargo test --no-fail-fast -p pdfcraft-ui-egui (everything else passed, failures below); cargo xtask layers; cargo xtask assets; cargo xtask parity; cargo xtask wasm. Failed, not related: 46 tests in the sandbox, none of them touching the Print readiness dialog, its thresholds or settings: 45 panic with `Failed to create render state: CustomNativeAdapterSelectionError("No adapter found")` (no GPU: 1 in the lib binary, `canvas::tests::a_moved_picture_is_drawn_from_the_fresh_raster_while_a_tile_is_still_stale`; 42 in the `it` binary, among them `control::form_tab_numbers_can_be_shown_and_captured_through_the_control_channel`, `control::screenshots_of_window_and_region`, the `comments::`, `content::`, `editing::`, `fill_sign::`, `status_layers::` and `view::` raster tests and `ui::the_drop_line_shows_in_front_of_the_first_tab`; `rendered_page_content_is_identical_with_and_without_decoration` in the page-shadows binary; `play_sessions_fetch_paragraph_bugs` in the play-sessions binary) and `find_focus::find_again_while_composing_keeps_the_query` (in `it`) panics in the epaint font code (no CJK font). Passed: lib 1106 of 1107, `it` 1998 of 2041. Estimates unchanged. |
| 2026-11-19 | minor | Gap 19: the Compare Files dialog's Ignore checkboxes (Letter case, Punctuation) are remembered with the other Preferences settings (`compare_ignore` with `case` and `punctuation` in the settings JSON): each is read alone, and a missing, mistyped or corrupt value keeps the current choice. Headless: `ui.set compare-ignore-case` and `compare-ignore-punctuation` (`true` or `false`; anything else is refused and changes nothing) and `ui.state.compare_ignore`. Tested: `preferences_pages::the_compare_ignore_options_are_set_refused_and_remembered`, `preferences_pages::corrupt_saved_compare_ignore_options_keep_the_current_choice` (both shown failing with the restore reverted) and `control::compare_ignore_options_are_set_and_reported_through_the_control_channel`. Ran: cargo fmt --check; cargo clippy -p pdfcraft-ui-egui --all-targets -- -D warnings -A clippy::collapsible_match; cargo test -p pdfcraft-ui-egui --test it compare_ignore (3 passed); cargo test --no-fail-fast -p pdfcraft-ui-egui (everything else passed, failures below); cargo xtask layers; cargo xtask assets; cargo xtask parity; cargo xtask wasm. Failed, not related: 46 tests in the sandbox, none of them touching compare options or settings: 45 panic with `Failed to create render state: CustomNativeAdapterSelectionError("No adapter found")` (no GPU: 1 in the lib binary, `canvas::tests::a_moved_picture_is_drawn_from_the_fresh_raster_while_a_tile_is_still_stale`; 42 in the `it` binary; `rendered_page_content_is_identical_with_and_without_decoration` in the page-shadows binary; `play_sessions_fetch_paragraph_bugs` in the play-sessions binary) and `find_focus::find_again_while_composing_keeps_the_query` (in `it`) panics in the epaint font code (no CJK font). Not run: tests of other crates; the Compare dialog drawn by a GPU. No Preferences page control; the dialog's own checkboxes are the place. |
| 2026-11-19 | minor | Gap 21: the Compare Files dialog has an Ignore row with Letter case and Punctuation checkboxes (off by default) that run the comparison with the `ignore_case` and `ignore_punctuation` options of `doc_compare`; the panel, shading, Mark as comments and Report… all use the same options (the report says what was ignored) and pixel differences are still found. Engine: `mark_differences_in_options` and `compare_report_in_options`. English-only strings (craft-i18n rows to follow). Tested: `compare::compare_dialog_ignore_checkboxes_leave_out_case_and_punctuation_changes` (ui-egui; fails with Mark and Report reverted to no options) and `tests::compare_marks_and_report_follow_the_ignored_word_differences` (engine). Ran: `cargo fmt --check`; `cargo clippy -p pdfcraft-engine -p pdfcraft-automation -p pdfcraft-ui-egui --all-targets -- -D warnings -A clippy::collapsible_match`; `cargo test -p pdfcraft-engine -p pdfcraft-automation -p pdfcraft-ui-egui --no-fail-fast`; `cargo xtask layers`; `cargo xtask assets`; `cargo xtask parity`; `cargo xtask wasm`. Failed, not related: the known root-only `batch_failures::read_only_output_folder_publishes_nothing` (automation) and 44 ui-egui tests that need a GPU (`egui_kittest` wgpu) or a Japanese font (`find_focus::find_again_while_composing_keeps_the_query`, no CJK face without `CRAFT_FONTS_DIR`); everything else passed (engine 876, automation 553 of 554, ui-egui lib 1106 and `it` 1985). Not run: a screenshot, the workspace-wide sweep. Estimates unchanged. |
| 2026-11-19 | minor | Gap 21: `doc_compare` takes `ignore_punctuation` (boolean, default false): words that differ only in sentence punctuation (. , ; : ! ? quotes and apostrophes, brackets, hyphen, underscore, Unicode dashes, quotes, bullets and ellipsis, guillemets, inverted marks, pilcrow, middle dot, CJK stops) are not counted as text changes and keep their own text. Kept, because they change a figure or name: symbols (`$ + = < > ^ | ~`), `# % & @ * / \`, the section sign, per mille and basis point signs, primes, the fraction slash, a `.` or `,` between two digits (10.5 is not 105, 1.000 is not 1,000) and a dash before a digit unless a letter precedes it (-5 is not 5). Words made only of such marks equal one another (a hyphen and an em dash), never a missing word or a word with another character; the section sign stays, so `§5` and `5` differ. It combines with `ignore_case`, affects the text comparison only, and the report, Mark as comments and the dialog do not take it. Headless only; English-only tool text. Tested: `compare::tests::ignoring_punctuation_treats_words_that_differ_only_in_marks_as_equal` and `compare_ignore_punctuation::ignore_punctuation_hides_mark_only_changes_and_keeps_real_ones`. Ran: `cargo fmt --check`; `cargo clippy -p pdfcraft-compare -p pdfcraft-engine -p pdfcraft-automation --all-targets -- -D warnings -A clippy::collapsible_match`; `cargo test -p pdfcraft-compare -p pdfcraft-engine -p pdfcraft-automation --no-fail-fast` (one known failure: `batch_failures::read_only_output_folder_publishes_nothing`, which fails when run as root; the engine suite passed, 875 tests); `cargo check -p pdfcraft-ui-egui`; `cargo xtask layers`; `cargo xtask assets`; `cargo xtask parity`; `cargo xtask wasm`. Not run: the workspace-wide clippy and test sweep, the ui-egui tests (it only consumes `compare_in`, which is unchanged), GPU tests, and any UI. Estimates unchanged. |
| 2026-11-19 | minor | Gap 18: `print_readiness` takes an optional `min_ppi` (36 to 1200 pixels per inch, default 300, other values refused): the least effective resolution an image may be drawn at before `image-ppi` warns (the message says `N ppi was asked for` and shows the drawn resolution to a tenth); an image is a fail below the lesser of 150 and the value. Headless only: the Print readiness dialog and Preferences have no setting. Tested: `printcheck::the_least_resolution_is_settable_and_bad_values_are_refused` (engine) and `print_readiness_min_ppi_through_tools` (end to end through the tool, refused values and the document unchanged), both shown failing with the fixed 150 and 300 put back. Ran: `cargo fmt --check`; `cargo clippy -p pdfcraft-engine -p pdfcraft-automation --all-targets -- -D warnings -A clippy::collapsible_match`; `cargo test --no-fail-fast` on both (all pass except the root-only `batch_failures::read_only_output_folder_publishes_nothing` and the load-sensitive timing test `printcheck::many_form_drawings_with_a_big_resource_dictionary_stay_fast`, which passed alone); `cargo xtask layers`, `assets`, `parity`, `wasm`. Not run: ui-egui tests, the workspace sweep. Estimates unchanged. |
| 2026-11-19 | minor | Never crash: PDF/A, PDF/UA and PDF/X preflight read a metadata packet's namespaces through a shared chain of the declarations each element adds, instead of copying every declaration in scope for every element (items x declarations on a wide array under many declarations). Results are unchanged. Estimates unchanged. |
| 2026-11-19 | minor | Never crash: the name-tree reader behind attachments, page templates and carried attachments charges every copied key to an allowance of 32 MiB per tree. Past it, adding, deleting or editing an attachment, moving a template, and Combine of a source whose pages are all included fail with an error and change nothing (nothing is dropped); listings show what was read. Ordinary files are unchanged. Estimates unchanged. |
| 2026-11-19 | minor | Never crash: importing pages (Combine, Insert, Extract, Split) resolves named destinations of links, bookmarks and GoTo actions from one index of the source's `/Dests` dictionary and name tree, read the first time one is needed (once per page copy; an Insert's bookmark mapper builds its own), in one walk that copies only new names, instead of one full tree lookup per link (links x names steps). Results are unchanged. Estimates unchanged. |
| 2026-11-19 | minor | Never crash: a form's `/V` and `/DV` string, name and array text is spent from one 64 MiB allowance (decoded bytes plus 24 per entry, entries that are not text included), so fields naming one shared array no longer each hold a copy (fields x entries). A field whose value does not fit is flagged `value_omitted` (repair log, `form_fields`): it is not written, Clear Form and ResetForm keep it, its appearance is not redrawn (property edits and XFA layout included), an AF calculation reading it keeps its total (a JavaScript one still reads it as empty), the XFA datasets are not rewritten empty for it, and text, XML, XFDF and FDF exports leave it out and name it (`doc_export_data` `left_out`). Its `/V` and `/DV` stay in the file. Estimates unchanged. |
| 2026-11-19 | minor | Never crash: a form's choice fields read at most 1,000,000 `/Opt` entries and 16 MiB of option text in all, and a combo or list box whose options no longer fit is flagged `options_omitted` (repair log, `form_fields`): its value and options are not written, Clear Form, ResetForm and the field's own calculation skip it (a script writing into it still gets the error), its appearance is not redrawn (check boxes and radio groups read only a list of one entry per widget, text fields none), and its `/Opt`, `/V` and `/I` stay in the file, so many fields naming one shared option array no longer copy it each. Estimates unchanged. |
| 2026-11-19 | minor | Never crash: a `ResetForm` `/Fields` or `Hide` `/T` array of more than 65,536 entries, or whose field names total more than 4 MiB, is not read (the Actions tab shows an action of another type, kept as it is, and a push button runs nothing), so entries naming widgets under one chain of long partial names no longer build a copy of the chain each. Estimates unchanged. |
| 2026-11-19 | minor | Never crash: the named-destination name tree (bookmark and link destinations in `doc_info`, links copied by extract, split and Combine) is read with each shared `/Names` or `/Kids` array once, so a small file whose nodes all name one array no longer costs nodes times entries steps or hides its later names. Estimates unchanged. |
| 2026-11-19 | minor | Never crash: listing an editable document's bookmarks no longer copies a shared huge `/Title` or `/Dest` array into every bookmark. Titles are spent from a 16 MiB allowance per listing call (a title that does not fit what is left lists as empty; a copy or export then reports `truncated` and a Cut keeps its source) and a destination keeps six items. Estimates unchanged. |
| 2026-11-19 | minor | Never crash: the attachment name tree is read with each shared `/Names` array once (`doc_info`, `attachment_*`, `af_*`, page templates, Combine), so a small file whose nodes all name one array no longer lists or rewrites nodes times entries attachments. Estimates unchanged. |
| 2026-11-19 | minor | Gap 15: Merge data files into a spreadsheet (`form_merge_data`, Forms menu) refuses a table of more than 2,000,000 cells, which files that each bring their own field names reach quickly, and the tool's column count no longer splits a field name that holds commas. Estimates unchanged. |
| 2026-11-19 | minor | Gap 17: `redact_mark` finds UK National Insurance numbers (`nino`), by format only (two letters, six digits, a final A to D; unallocated prefixes and unused letters excluded; no checksum, so a match is a candidate). Other countries' identifiers are still missing. Estimates unchanged. |
| 2026-11-19 | minor | Gap 18: `print_readiness` takes an optional `min_bleed_mm` (0 to 50 millimetres, default 2, other values refused): the least bleed past the trim, on a page's tightest side, before `page-bleed` warns on a page with a TrimBox; 0 turns it off. Headless only: the Print readiness dialog and Preferences have no setting. Tested: `printcheck::the_least_bleed_is_settable_and_bad_values_are_refused` (engine) and `print_readiness_min_bleed_through_tools` (end to end through the tool, refused values and the document unchanged); the engine test was shown failing with the `min_bleed > 0.0` guard removed (a BleedBox inside the TrimBox flagged at 0), and the tool test with the engine comparison put back to the fixed 2 mm; 0 stays silent for a BleedBox smaller than the TrimBox, `min_bleed_mm: null` is treated as not given (as `ink_limit: null` is), and a margin within 1e-6 mm of the figure is not flagged. Ran on the final tree: `cargo fmt --check`; `cargo clippy -p pdfcraft-engine -p pdfcraft-automation --all-targets -- -D warnings -A clippy::collapsible_match` (the allow is for a pre-existing hit at `crates/xfdf/src/xfdf.rs:326` on main with this toolchain, unrelated); `cargo test -p pdfcraft-engine -p pdfcraft-automation --no-fail-fast` (all pass except `batch_failures::read_only_output_folder_publishes_nothing`, which fails when run as root); `cargo xtask layers`, `assets`, `parity`, `wasm`. Not run: ui-egui tests (the UI is unchanged), the workspace-wide sweep. Estimates unchanged. |
| 2026-11-19 | minor | Gap 16: `doc_guide_snap` (read-only) gives where an `x` and/or `y` on a page lands on that page's nearest vertical and horizontal guide within `tolerance` points (default 4, 0 to 72, edge included; the lower position wins a tie; a page with no guides snaps to nothing). Headless arithmetic on the listed guides only: the app has no rulers and does not snap while dragging, and guides stay session-only view state. Estimates unchanged. |
| 2026-11-19 | minor | Gap 21: `doc_compare` takes `ignore_case`: words that differ only in letter case are not text changes (headless; the visual regions still see the different pixels, and `doc_compare_report`, `doc_compare_mark` and the Compare dialog do not take it). Estimates unchanged. |
| 2026-11-19 | minor | Gap 22: `ns_list` lists each `/NS` value that more than one structure namespace dictionary carries (`duplicate_ns`; text compared as listed, cut at 200 characters); other validation, editing and any UI are still not done. Estimates unchanged. |
| 2026-11-18 | minor | Gap 23: `ai_redaction_suggest` asks the user's own AI endpoint for passages to redact and returns only those that occur in the page text, with pages and counts (`misc.ai-redaction-suggest` is partial); it marks nothing, has no UI and was tested only against a stand-in server. Estimates unchanged. |
| 2026-11-18 | minor | Gap 22: `ns_list` lists the PDF 2.0 structure namespaces (`/StructTreeRoot` `/Namespaces`, with `/RoleMapNS` entries) read-only (`core.pdf20-namespaces` is partial); no validation, writing or UI. Estimates unchanged. |
| 2026-11-16 | minor | Gap 22: `dpart_list` lists the PDF 2.0 document part tree (`/DPartRoot`) read-only (`core.pdf20-dpart` is partial); no validation, writing or UI. Estimates unchanged. |
| 2026-11-16 | minor | Gap 17: `redact_mark` with `pattern` or `regex` takes `keep_first` and `keep_last`, leaving that many letters or digits at the start or end of each match readable (headless; no UI control, not for find or words). Estimates unchanged. |
| 2026-11-16 | minor | Gap 18: `print_readiness` takes an optional `ink_limit` (100 to 400 percent, default 300, other values refused) for the `ink-total` finding, and the finding names the limit used; no setting in the Print readiness dialog or Preferences, and still only painted vector CMYK colours (`print.total-area-coverage` stays partial). Estimates unchanged. |
| 2026-11-16 | minor | Gap 21: `doc_compare` with `visual` gives each visual region a `cause`, `annotation` when it overlaps a comment (markup) annotation that only one of the two files has on that page and `other` otherwise; images, formatting and background changes are still not told apart, and the kinds filter, the dialog and the report do not use it yet (`misc.compare-filters` stays partial). Estimates unchanged. |
| 2026-11-16 | minor | Gap 16: View ▸ Guides ▸ Show Guides draws a document's ruler guides over the pages (per tab, off by default, `ui.set guides`) and Clear Guides removes them; no rulers to drag from and no snapping yet (`view.guides` stays partial). Estimates unchanged. |
| 2026-11-16 | minor | Gap 22: `af_list`, `af_add` and `af_remove` list and edit PDF 2.0 associated files (`/AF`) at document level (`core.pdf20-associated-files` is partial). Estimates unchanged. |
| 2026-11-16 | minor | Gap 22: `doc_linearization` reports a file's linearization parameter dictionary and whether it still holds (read-only; `core.linearization` is partial). Estimates unchanged. |
| 2026-11-16 | minor | Gap 11: `accessibility_reading_order` lists the tagged content in logical reading order, inline elements included, with type, page, MCID, text and inherited alt text and language (headless, read-only). Estimates unchanged. |
| 2026-11-16 | minor | Gap 17: `redact_mark` finds Canadian Social Insurance Numbers (`sin`) and postal codes (`ca-postal-code`); other locales still missing. Estimates unchanged. |
| 2026-11-16 | minor | Gap 18: `page_boxes` lists every page's media, crop, bleed, trim and art boxes, which the file sets, the rotation and the bleed past the trim (headless, read-only). Estimates unchanged. |
| 2026-11-16 | minor | Gap 13: the signature listing reads the page tree only for a document that has a signature field (≈ 0.25 s less per edit of a 100,000-page file); measured that each save keeps one more copy of the file in undo history. Estimates unchanged. |
| 2026-11-16 | minor | Gap 8: `doc_create` from `office` converts one Office document into a new PDF through an installed LibreOffice (headless; no menu command yet). Estimates unchanged. |
| 2026-11-14 | minor | Gap 21: Compare files filters by kind of change (replaced, inserted, deleted, visual) in the dialog, the panel, Mark as comments, the report and `doc_compare`, `doc_compare_report`, `doc_compare_mark` (`kinds`); images, annotations, formatting and background changes are still not told apart. Estimates unchanged. |
| 2026-11-13 | minor | Gap 21: the Compare Files dialog has Old file pages and New file pages fields; the comparison, panel, report and Mark as comments use them (`doc_compare_report` and `doc_compare_mark` take the same range arguments as `doc_compare`). Estimates unchanged. |
| 2026-11-13 | minor | Gap 8: Insert pages, Combine files and Create ▸ Multiple files take Office documents through the installed LibreOffice, converted on a worker thread (all or nothing when LibreOffice is missing). Gap 8's remaining first-beta Office item re-estimated from 5–10 h to 3–6 h. |
| 2026-11-11 | minor | Gap 8: known residual risk for the LibreOffice sidecar (Impress has no link-update setting; the soffice process isn't network-sandboxed). Sandboxing tracked in #6953. Estimates unchanged. |
| 2026-10-26 | minor | Gap 8: the LibreOffice profile never updates Writer or Calc linked content on load; Impress has no such setting and the process's network access isn't sandboxed. Estimates unchanged. |
| 2026-10-25 | minor | Gaps 5 and 14 current state: native release builds embed LXGW WenKai Lite for Simplified Chinese, while the web build has no Hans face; Arabic catalogs stay logical with explicit RTL label direction and mirrored stock chrome, while native/browser RTL input, IME and complex workflows remain unqualified. Estimates unchanged. |
| 2026-10-18 | minor | Gap 3: #2517 a partial rewrite keeps a kept source row in the paragraph and leaves the next marked region closed around its text. Estimates unchanged. |
| 2026-10-15 | minor | Gap 3: #2517 a blank line at ordinary leading stays a paragraph break. Estimates unchanged. |
| 2026-10-19 | minor | Gap 2: a Type 0 font whose `/ToUnicode` is the name `/Identity-H` or `/Identity-V` (WSCAD) reads its codes as Unicode, so such pages have text to select, find and edit (hayro-interpret patch 44, `view.render-cid-fonts`). Estimates unchanged. |
| 2026-10-15 | minor | Gap 21: the side-by-side view had already shipped; the Compare panel filters text changes and visual differences. Estimate 10–20 h → 8–16 h |
| 2026-10-20 | minor | Gap 6 #3673: Character encoding accepts Identity-H/V with Adobe CJK collections, or a predefined non-Identity CMap; negative and indirect-reference regressions and a read-only automation test passed after failing before the fix. Per-glyph validation and embedded-font extraction remain separate. Scores and estimates unchanged. |
| 2026-10-17 | minor | Gap 6: the redaction audit counts a cover over one word of a long line instead of a clean bill (#2662, fixed by #2698). Estimates unchanged. |
| 2026-10-15 | minor | Gap 12: the text oracle scores reading order as well as word F1, which is a bag of words and cannot see order; the first measurement separated 18 improvements from 6 regressions in a change F1 called unchanged on 26 of 29 files. Estimates unchanged. |
| 2026-10-14 | minor | M1: raw-deflate fallback preserves decoding-limit errors in strict and tolerant decoding; synthetic filter and COS stream regressions. Estimates unchanged. |
| 2026-10-15 | minor | Gap 3: find and replace in Edit text mode (the Find box's Replace row, `text_replace`); each match is rewritten inside its own paragraph, and matches in form fields, comments or across paragraphs are left and counted. Estimates unchanged. |
| 2026-10-15 | patch | Gap 24: record localization and installers as partial (unusable Chinese, Telugu and Arabic; unsigned Linux packages) |
| 2026-10-13 | minor | Gap 6: Organize Pages selects and reveals the current page on open, including after an earlier grid selection elsewhere (#1910). Estimates unchanged. |
| 2026-10-12 | minor | Gap 6: rubber stamps, explicit image rectangles and PDF-page stamps read upright on rotated pages (#1398). Estimates unchanged. |
| 2026-10-12 | minor | #1316: replacing a line or paragraph keeps a neighbouring line whose glyph box overlaps. Gap 3 remains. Estimates unchanged. |
| 2026-10-12 | minor | Exclusive image crops resize with their pixels across nested states, intervening operators and content streams; shared clips remain fixed. Synthetic engine/tool regressions and private image save/reopen checks; estimates unchanged. |
| 2026-10-12 | minor | Gap 13: measured large-file memory; edits no longer keep the previous working file while writing the next (peak 3 → 2 copies); file-backed input recorded as blocked on gap 2 |
| 2026-10-11 | minor | Closed gap 4: the web build prints through the browser's print dialog (sheets drawn by PdfCraft at 150–300 dpi, one per printed page); number kept |
| 2026-10-11 | minor | Word invoice failure #773 has synthetic regression coverage; broader Office conversion remains gap 8. |
| 2026-10-11 | minor | Contributor fork: mixed-style visual regions, source-font space reuse and a read-only textless background. Embedded sfnt source faces and Mac subset Unicode mappings keep edit baselines stable; live frames follow Apply/Save geometry as text grows or shrinks. Engine/tool/UI regressions and private all-page checks cover the changes; full matching font import, CFF, range styles, shaping and private Chinese typing checks added; legacy programs and general fidelity remain open; estimates unchanged |
| 2026-10-11 | patch | Gap 2: 23 hayro-interpret and 10 hayro patches after axial and radial shadings became native gradients (backport of hayro 0.8) |
| 2026-10-11 | minor | Closed gap 1 (#774, fixed by #836; numbers kept, its interop suite moved to gap 12); gap 6 lists the fixed #799–#821 batch; gap 3 and 16 updated for #844 (same-page move), #740 (column select) and #759 (1:1 touchpad scrolling) |
| 2026-10-10 | minor | Removed the alpha markers: with the cross-app gate rule, gaps 1 and 8 are partial sub-cases, not blockers; stage alpha |
| 2026-10-10 | minor | Marked the alpha blockers (gap 1 and the Office part of gap 8) after the core-workflow gate put the stage at pre-alpha |
| 2026-10-10 | major | Created. Ranked 24 gaps from the 2026-10-10 re-measure, the open GitHub issues and the former ROADMAP.md §Where we're lacking and where we're going (renderer, hardening, fidelity, editing, Pro workflows, 1.0 polish), which this file replaces |
