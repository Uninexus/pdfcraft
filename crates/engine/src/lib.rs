//! pdfcraft-engine — the façade every frontend talks to (architecture §4).
//!
//! Holds open documents, their edit history and the tool catalogue. Frontends never touch the
//! parsing, rendering or editing crates directly.
//!
//! **Editing model.** Each document keeps a `pdfcraft_cos::Document` (the object graph with a
//! copy-on-write overlay of edits). An edit runs on a clone, and on success the previous state is
//! pushed onto the undo stack (clones share all unchanged data, so this is cheap). After every
//! edit the *working file* is produced by an incremental write — original bytes plus one
//! appended revision — and the view is refreshed from it, so what you see is exactly what Save
//! will write. Saving rebases onto the written bytes, so the next save appends only new edits.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// Whether this build offers the AI assistant. A build with `PDFCRAFT_NO_AI` set at compile time
/// registers no `ai.` command or `ai_` tool and shows no AI window, tool group or preferences page.
pub const AI_ENABLED: bool = option_env!("PDFCRAFT_NO_AI").is_none();

pub mod actions;
pub mod ai_edit;
pub mod autotag;
pub mod bates_batch;
pub mod bookmark_json;
mod bookmarks;
pub mod build_info;
pub mod camera;
pub mod camera_clean;
pub mod camera_edges;
pub mod catalog;
pub mod commands;
mod comments;
pub mod compare;
mod copy_image;
pub mod redact_folder;
pub mod redact_keep;
pub use copy_image::{PagePicture, picture_at};
mod arrange;
pub mod dates;
mod document;
mod edit;
mod error;
pub mod export;
pub mod field_copies;
pub mod field_table;
mod fill_sign;
pub mod find_replace;
pub mod flatten_transparency;
pub mod folder_search;
pub mod form_entries;
pub mod guides;
mod history;
pub mod js;
mod layers;
pub mod linearization;
pub mod links;
mod live_scripts;
pub mod model3d;
mod object_clip;
pub mod ocr;
mod open;
pub mod open_parameters;
pub mod optimizer;
mod page_filter;
mod pictures;
pub mod plan_audit;
pub mod print_raster;
pub mod printcheck;
mod protection;
mod rename;
mod rich_copy;
pub mod scan_clean;
mod scan_colours;
mod scan_fields;
pub mod security_policy;
mod session;
pub mod signature_audit;
pub mod signature_image;
pub mod signature_scan;
mod sources;
pub mod split_name;
pub mod spooler;
mod stamp_preview;
#[cfg(test)]
mod test_hooks;
pub mod text_fonts;
pub mod text_links;
mod text_preview;
pub mod text_script;
pub mod text_style_bookmarks;
pub mod white_margins;
pub mod word_count;
pub mod xfa;
mod xfa_form;

use std::sync::Arc;

pub use links::{
    MAX_REMOTE_DOCUMENT_BYTES, RemoteDocumentPathError, read_remote_document, remote_document_path_is_absolute, resolve_remote_document_path,
};
pub use live_scripts::{MAX_TIMER_RUNS, ScriptRun};
pub use pdfcraft_edit::strip_subset_tag;
pub use rich_copy::RichCopy;
pub use signature_image::{ImageSignaturePreview, SignatureImage};
pub use signature_scan::{PhotoRect, ScanError, SignaturePhoto, Turn};
pub use stamp_preview::{CommentLayer, StampPreview};
pub use text_preview::{TextEditPreview, UnmarkedPages};

pub use pdfcraft_organize::{
    AF_RELATIONSHIPS, AssociatedFile, AssociatedList, BoxSpec, FieldConflicts, MAX_AF_BYTES, MAX_AF_ENTRIES, MAX_AF_ROWS, MAX_IMPORT_BOOKMARKS,
    OutlineNode, OutlineTree, PageBox, PageBoxReport, SplitBy, Transition, TransitionSpec, mime_for_name, split_ranges, unmapped as outline_unmapped,
    view_arity as outline_view_arity,
};

pub use pdfcraft_organize::LabelStyle;
pub use pdfcraft_organize::Standard;
pub use pdfcraft_organize::articles::{MAX_BEADS_WALKED, MAX_LISTED_THREADS, MAX_TITLE_CHARS};
pub use pdfcraft_organize::base_url::{MAX_BASE_URL_BYTES, check_base_url};
pub use pdfcraft_organize::check_custom_key;
pub use pdfcraft_organize::class_map::{MAX_CLASS_NAME_BYTES, MAX_LISTED_CLASSES, MAX_LISTED_OWNERS};
pub use pdfcraft_organize::collection::{MAX_GETS as MAX_COLLECTION_GETS, MAX_LISTED_FIELDS, MAX_TEXT_CHARS as MAX_COLLECTION_TEXT_CHARS};
pub use pdfcraft_organize::destinations::{MAX_ITEMS_SCANNED, MAX_LISTED_DESTINATIONS, MAX_NAME_CHARS, MAX_TREE_DEPTH, MAX_TREE_NODES};
pub use pdfcraft_organize::embedded_action::{
    Change as EmbeddedActionChange, EmbeddedTarget, MAX_NAME_BYTES as MAX_EMBEDDED_NAME_BYTES, MAX_PAGE as MAX_EMBEDDED_PAGE,
};
pub use pdfcraft_organize::field_signer::{
    AddressError as SignerAddressError, Change as FieldSignerChange, FieldSigner, KEY as FIELD_SIGNER_KEY,
    MAX_READ_CHARS as FIELD_SIGNER_MAX_READ_CHARS, check_address as check_signer_address, is_hidden as is_hidden_char,
};
pub use pdfcraft_organize::launch_action::{Change as LaunchActionChange, LaunchTarget, MAX_FILE_BYTES as MAX_LAUNCH_FILE_BYTES};
pub use pdfcraft_organize::media_action::Change as MediaActionChange;
pub use pdfcraft_organize::named_action::{Change as NamedActionChange, NamedTarget, SAFE_NAMES as SAFE_MENU_NAMES};
pub use pdfcraft_organize::print_presets::{Duplex, MAX_PRINT_COPIES, MAX_PRINT_RANGES, MIN_PRINT_COPIES, PrintPresets, PrintScaling};
pub use pdfcraft_organize::role_map::{MAX_LISTED_ROLES, MAX_LOOP_STEPS, MAX_ROLE_BYTES};
pub use pdfcraft_organize::screen_action::Change as ScreenActionChange;
pub use pdfcraft_organize::signature_lock::{
    Change as LockChange, LockAction, LockSpec, MAX_GETS as MAX_LOCK_GETS, MAX_LOCK_NAMES, MAX_NAME_CHARS as MAX_LOCK_NAME_CHARS,
};
pub use pdfcraft_organize::signature_seed::{
    Change as SeedChange, DIGEST_METHODS as SEED_DIGEST_METHODS, ENTRIES as SEED_ENTRIES, Entry as SeedEntry, MAX_ENTRIES as MAX_SEED_ENTRIES,
    MAX_TEXT_CHARS as MAX_SEED_TEXT_CHARS, SUB_FILTERS as SEED_SUB_FILTERS, SeedList, SeedSpec,
};
pub use pdfcraft_organize::signing_handoff;
pub use pdfcraft_organize::signing_plan;
pub use pdfcraft_organize::thread_action::{
    Change as ThreadActionChange, MAX_BEADS as MAX_ACTION_BEADS, MAX_THREADS as MAX_ACTION_THREADS, ThreadTarget,
};
pub use pdfcraft_organize::view::{InitialView, Layout as InitialLayout, Magnification, Navigation};

pub use pdfcraft_cos::{Algorithm, civil_from_days};
pub use pdfcraft_create::{
    CONVERTIBLE, CreateError, IMAGE_EXTENSIONS, ImageResolution, OFFICE_EXTENSIONS, SourceKind, TEXT_EXTENSIONS, decode_text, source_kind,
};
pub use pdfcraft_edit::added::{
    CHAR_SPACING_LIMIT as ADDED_CHAR_SPACING_LIMIT, paste_place as added_paste_place, paste_rect as added_paste_rect, text_rect as added_text_rect,
    text_width as added_text_width,
};
pub use pdfcraft_edit::lists;
pub use pdfcraft_edit::paragraph_align;
pub use pdfcraft_edit::{
    Added, AddedImage, AddedText, Align as TextAlign, Background, ColourTarget, ColoursConverted, Content as AddedContent, ConvertColours,
    Family as FontFamily, Hairlines, HairlinesFixed, HeaderFooter, Ink as TextInk, InkAliases, InksAliased, MarkEdge, MarkKind, PrinterMarks,
    SpotInk, Watermark,
};
pub use pdfcraft_forms::{
    BorderStyle, CalculatedField, CalculationIssue, CalculationOrder, CheckStyle, Field as FormField, FieldAction, FieldChange, FieldCopy, FieldFont,
    FieldKind as FormFieldKind, FieldProps, FieldValue, Look as FieldLook, LookPatch as FieldLookPatch, NewField, SubmitAction, SubmitFields,
    SubmitFormat, TabOrder, Trigger as FieldTrigger, Widget as FormWidget, af as form_scripts, appearance as form_appearance,
    appearance::auto_text_size_by as field_auto_text_size_by, appearance::comb_auto_text_size as field_comb_auto_text_size,
    calculations_changed as form_calculations_changed, flags as field_flags,
};

pub use pdfcraft_a11y as a11y;
pub use pdfcraft_edit::{Backdrop, BlockKey, BlockStyle, PageImage, RangeStyle, TextBlock, TextLine, block_overlapping, first_undrawable};
pub use pdfcraft_measure as measure;
pub use pdfcraft_xfa::Report as XfaLayout;

pub use arrange::{ArrangeOp, plan_offsets};
pub use object_clip::{ObjectClip, ObjectClipboard};
pub use pdfcraft_edit::{EditableObject, MAX_MOVE_OBJECTS, ObjectKind, ObjectTarget};

pub use pdfcraft_fonts::{MAX_SIGNATURE_CHARS, ScriptOutline, script_missing, script_outline, undrawable_message};

/// Comment geometry helpers (text-box line breaking) for frontends.
pub use pdfcraft_annot::appearance as annot_text;
pub use pdfcraft_annot::links::{Highlight as LinkHighlight, LinkAction, LinkItem, LinkStyle, LinkZoom};
pub use pdfcraft_annot::{
    Activation3d, AppearanceCache, AttachIcon, BorderShape, CALLOUT_KNEE_LINE, FillMark, LineEnding, Markup, NOTE_SIZE, NewAnnotation, NoteIcon,
    OverlayFont, OverlayLook, Props as CommentProps, ReviewState, Rgb, Shape, Side as CalloutSide, StampGroup, StampKind, Style, TextRun,
    callout_attach, callout_knee, callout_side, rect_quad, rich_line_ranges,
};
pub use pdfcraft_annot::{MAX_MODEL_BYTES as MAX_3D_MODEL_BYTES, ModelFormat as Model3dFormat};
pub use pdfcraft_forms::detect;
pub use pdfcraft_forms::{
    MAX_FIND_LEN as MAX_JAVASCRIPT_FIND_LEN, MAX_REPLACEMENT_LEN as MAX_JAVASCRIPT_REPLACEMENT_LEN, MAX_SCRIPT_LEN as MAX_JAVASCRIPT_LEN,
    MAX_SCRIPTS as MAX_JAVASCRIPTS, MAX_TOTAL_CODE as MAX_JAVASCRIPT_TOTAL, MAX_TOTAL_LABELS as MAX_JAVASCRIPT_LABELS,
    ReplaceReport as JavaScriptReplaceReport, ReplacedScript as JavaScriptReplaced, ScriptEntry as JavaScriptEntry,
    ScriptInventory as JavaScriptInventory, ScriptKind as JavaScriptKind,
};
pub use pdfcraft_optimize as optimize;
pub use pdfcraft_preflight as pdfa;
pub use pdfcraft_print as print;
pub use pdfcraft_redact::Elsewhere as RedactionElsewhere;
pub use pdfcraft_redact::codes::{CODE_SETS as REDACTION_CODE_SETS, CodeSet as RedactionCodeSet, CustomCodeSet as CustomRedactionCodeSet};
pub use pdfcraft_redact::patterns::{
    MAX_WORDS as REDACT_MAX_WORDS, PATTERNS as REDACT_PATTERNS, Pattern as RedactPattern, find as find_pattern, word_list as redact_word_list,
};
pub use pdfcraft_redact::redacted_file_name;
pub use pdfcraft_redact::sanitize::{HIDDEN, Hidden, SIGNER_LABEL_KEY as SANITIZED_SIGNER_LABEL_KEY};
pub use pdfcraft_redact::user_regex::{UserRegex as RedactRegex, VALIDATORS as REDACT_VALIDATORS, Validator as RedactValidator};
pub use pdfcraft_redact::{Claim as RedactionClaim, Identification as RedactionIdentification};
pub use pdfcraft_sign as sign;
pub use pdfcraft_sign::{SignOptions, SignatureInfo, Status as SignatureStatus, StatusCaptions, TrustStore};
pub use pdfcraft_xfdf::Format as DataFormat;
pub use pdfcraft_xfdf::{Outline as CommentOutline, comment_count, comment_outlines, comments_bounds, is_xfdf_comments};

use bookmarks::outline_rows;
pub use bookmarks::{BookmarkPage, BookmarkRow};
use comments::comment_list;
pub use comments::{SummarySort, comment_kind, comment_summary, comment_summary_pages, comment_type_name};
#[cfg(test)]
use document::page_runs;
pub use document::{COMMENT_TYPES, Document, EditNotice, MakeAccessible, TextNotice, display_bytes_for, render_password_for};
use document::{carry_verdict_marks, display_bytes, info_fields, recovered_render_password, same_pages, shown_under_redactions, undrawn_notices};
pub use edit::{BATCH_LABELS, EDIT_LABELS, Edit};
pub use error::{EditError, SourceProblem};
pub use fill_sign::{displayed_offset, text_box_at, typed_signature_graphic, typed_signature_shape};
pub use history::Label;
use layers::{apply_layer_state, print_layer_states, print_preview_layers, set_default_layer_state, use_layer_choices};
pub use open::{OpenContext, PreparedDocument};
pub use page_filter::{PageOrientation, PageParity, filter_pages};
pub use pdfcraft_export::language_tag;
pub use pictures::{MarkFile, image_dimensions};
use pictures::{embed_image, fit_picture, mark_source, model_poster};
use protection::unrestricted;
pub use protection::{Changes, Printing, Protection, SecuritySummary};
pub use rename::{RenameError, new_file_name, rename_target};
pub use session::{CommentSummary, INSERT_FROM_CLIPBOARD, InsertPlan, Session};
pub use sources::{CombineSource, Combined, OfficeBatch, OfficeLocator, OfficeRunner, combine_source_check, office_to_pdf, source_page_count};
use sources::{open_source, open_source_with};
#[cfg(test)]
use test_hooks::{
    check_refresh_panic, panic_on_refresh, record_full_refresh, record_link_scan, record_marker_scan, reset_secondary_scan_counts,
    secondary_scan_counts, take_full_refreshes,
};
use xfa_form::{
    XFA_REMOVED_PREFIX, XFA_SCRIPTS_OFF, decode_data_file, image_content_type, is_xfa_data_file, note_warnings, strip_xfa, sync_xfa_state,
    undrawn_field_warnings, xfa_after_data_change, xfa_layout, xfa_redraw, xfa_sync_datasets, xfa_values_from_datasets,
};

mod clock;
mod doc_state;

pub use pdfcraft_edit::ArrangeOrder;

use clock::{local_utc_offset, parse_ymd};
pub use doc_state::{DocId, ImageEdit, OptimizeReport, RecoverySnapshot, SplitPart, guard, merge_data_files, merge_data_files_counted};
use doc_state::{LOW_RESOLUTION_PRINT_DPI, render_threads, renderer_password, signatures_of, writable};

/// The configuration of the pool that draws the viewer: the document's own, plus installed CJK
/// fonts for fonts a PDF does not embed (`pdfcraft_render::RenderConfig::system_fonts`). Exports,
/// printing, OCR and automation use the document configuration itself, so nothing drawn with an
/// installed font is saved or printed (licences.md; docs/development.md "Fonts and attribution").
fn display_config(config: &pdfcraft_render::RenderConfig) -> pdfcraft_render::RenderConfig {
    config.clone().on_screen()
}

/// How long a custom stamp's text line may be, in characters (#1741).
pub const MAX_STAMP_LINE: usize = 200;

#[cfg(test)]
mod document_actions_tests;
#[cfg(test)]
mod edit_all_javascripts_tests;
#[cfg(test)]
mod link_javascript_tests;
#[cfg(test)]
mod tests;
