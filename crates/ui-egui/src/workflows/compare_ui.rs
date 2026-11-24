//! Compare files: choose the older version, then the Compare panel lists the differences
//! (replaced blue, inserted green, deleted red), shades them on the newer document's pages, and
//! offers a report or the differences as comments.

use craft::behavior::{IconId, Selection, keyed};
use craft::pages::ui::{Lead, SyncedViews, SyncedViewsState};
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{Bind, Entry, Pad, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::{DialogResult, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{HueTarget, List, ListRow};
use craft::ui::notices::Notice;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::primitives::IconSpec;
use craft::ui::style::DataColor;
use egui::Color32;
use pdfcraft_engine::DocId;
use pdfcraft_engine::compare::{ChangeKinds, Comparison, Kind, PageRange, WordOptions};

use crate::{PdfCraftApp, RightPanel};

/// The key the Compare Files dialog files its answer under.
const DIALOG_KEY: &str = "compare";

/// The last comparison.
pub struct CompareState {
    pub old: DocId,
    pub new: DocId,
    pub result: Comparison,
    /// The pages of the old file this comparison covers (1-based, inclusive; `None`: every page).
    pub old_range: Option<PageRange>,
    /// The pages of the new file this comparison covers (`None`: every page).
    pub new_range: Option<PageRange>,
    /// Regions that look different (old page, new page, user-space box; pages 0-based and each
    /// document's own). Without ranges page n is against page n; with ranges the pages of the two
    /// ranges are paired by position.
    pub visual: Vec<(usize, usize, [f64; 4])>,
    pub selected: Option<usize>,
    /// Side by side: the old file on the left, the new on the right, scrolled and zoomed
    /// together.
    pub side_by_side: bool,
    /// The pair's divider and which half leads; the new file leads at first.
    pub views: SyncedViewsState,
    /// Show: the kinds of change in the list and the shading (replaced, inserted, deleted) and
    /// whether the visual differences (counted, never listed) are shaded, as Acrobat's Filter
    /// does. Mark as comments and the report follow it. Images, annotations, formatting and
    /// background changes are not kinds of their own: the comparison reads words and pixels, so
    /// they are all visual.
    pub kinds: ChangeKinds,
    /// The word differences the comparison left out (the dialog's Ignore checkboxes). Mark as
    /// comments and the report run with the same ones, so they match the list.
    pub options: WordOptions,
}

/// Visual differences are shaded orange.
pub const VISUAL: Color32 = Color32::from_rgb(0xF0, 0x8C, 0x1A);

/// What the panel asks for.
pub enum PanelAction {
    Select(usize),
    Mark,
    Report,
    Clear,
    SideBySide(bool),
    Show(ChangeKinds),
}

pub fn colour(kind: Kind) -> Color32 {
    let [r, g, b] = pdfcraft_engine::compare::colour(kind);
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// The key a document is offered under in the dialog's choice.
fn option_key(id: DocId) -> String {
    id.0.to_string()
}

/// Opens the Compare Files dialog: the older version among the other open documents, the active file
/// as the new one, and a range of pages for each file.
pub(crate) fn open_compare_dialog(app: &mut PdfCraftApp, ctx: &egui::Context) {
    let Some((_, new)) = app.active_ids() else { return };
    let others: Vec<(DocId, String, usize)> =
        app.views.iter().filter(|v| v.id != new).filter_map(|v| app.session.get(v.id).map(|d| (d.id, d.name.clone(), d.info.pages.len()))).collect();
    let old = app.compare_old.filter(|o| others.iter().any(|(id, _, _)| id == o)).or_else(|| others.first().map(|(id, _, _)| *id));
    let new_name = app.session.get(new).map(|d| d.name.clone()).unwrap_or_default();
    let new_count = app.session.get(new).map_or(0, |d| d.info.pages.len());
    // The Pages fields belong to this pair of files: another pair starts empty.
    let (old_pages, pages) = if app.compare_fields_for == Some((new, old)) {
        (app.compare_old_pages.clone(), app.compare_pages.clone())
    } else {
        (String::new(), String::new())
    };
    let others: Vec<(String, String, usize)> = others.into_iter().map(|(id, name, count)| (option_key(id), name, count)).collect();
    let mut fields = DialogFields::from_pairs([("old", old.map(option_key).unwrap_or_default()), ("old_pages", old_pages), ("pages", pages)]);
    for (key, on) in kind_values(app.compare_kinds) {
        fields.set(key, on);
    }
    fields.set(IGNORE_CASE, app.compare_ignore_case);
    fields.set(IGNORE_PUNCTUATION, app.compare_ignore_punctuation);
    // Passed inline: a closure made apart from the call gets one lifetime for its argument and result.
    let dialog = HostedParamDialog::new(ctx, DIALOG_KEY, fields, move |fields: &mut DialogFields| {
        let chosen = fields.text("old").filter(|o| !o.is_empty()).map(str::to_owned);
        let old_count = chosen.as_ref().and_then(|c| others.iter().find(|(key, _, _)| key == c)).map_or(0, |(_, _, count)| *count);
        let (_, errors) = field_ranges(fields.text("old_pages").unwrap_or_default(), old_count, fields.text("pages").unwrap_or_default(), new_count);
        let problems = range_messages(errors);
        let kinds = kinds_of(fields);
        let options = others.iter().map(|(key, name, _)| (key.clone(), name.clone().into())).collect();
        let params = vec![
            Param::new("old", tl!("Old file:").to_string(), ParamKind::choice(options, chosen.clone())),
            Param::new("old_pages", tl!("Old file pages:").to_string(), ParamKind::text(String::new()))
                .hint(whole_document_hint(old_count, tl!("Choose the old file first"))),
            Param::read_only_text("new", tl!("New file:").to_string(), new_name.clone()),
            Param::new("pages", tl!("New file pages:").to_string(), ParamKind::text(String::new())).hint(whole_document_hint(new_count, "")),
            Param::section(tl!("Kinds of change").to_string()),
            Param::new(KIND_KEYS[0], tl!("Replaced").to_string(), ParamKind::bool(true)),
            Param::new(KIND_KEYS[1], tl!("Inserted").to_string(), ParamKind::bool(true)),
            Param::new(KIND_KEYS[2], tl!("Deleted").to_string(), ParamKind::bool(true)),
            Param::new(KIND_KEYS[3], tl!("Visual").to_string(), ParamKind::bool(true)),
            Param::section(tl!("Ignore").to_string()),
            Param::new(IGNORE_CASE, tl!("Letter case").to_string(), ParamKind::bool(false)),
            Param::new(IGNORE_PUNCTUATION, tl!("Punctuation").to_string(), ParamKind::bool(false)),
        ];
        let reason = match problems.first() {
            Some(problem) => problem.clone(),
            None if chosen.is_none() => tl!("Open the older version first").to_string(),
            None => tl!("Choose at least one kind of change").to_string(),
        };
        ParamDialog::new(DIALOG_KEY, tl!("Compare Files").to_string(), params)
            .values(Bind::bound(fields))
            .ok_label(tl!("Compare").into())
            .ok_enabled(chosen.is_some() && problems.is_empty() && !kinds.is_empty())
            .ok_reason(reason.into())
            .footer_note(tl!("Text is compared word by word; the differences are shown on the new file.").to_string())
            .below(move |ui, _| {
                for problem in &problems {
                    Text::new(problem.clone()).tone(Tone::Bad).show(ui);
                }
                Text::new(tl!(
                    "Ignoring letter case or punctuation leaves out text changes that are only that; the words in the changes that remain keep their own text, and pixel differences are still found."
                ))
                .shade(Shade::Muted)
                .show(ui);
                Text::new(tl!(
                    "Images, annotations, formatting and background changes are not told apart: the comparison finds text changes and pixel differences, so they all count as visual."
                ))
                .shade(Shade::Muted)
                .show(ui);
                Text::new(tl!(
                    "Leave both fields empty to compare the whole files. Give one range for each file, such as 2-5 or 3; the pages of the two ranges are compared with each other. With a range for one file only, the same pages of the other file are used."
                ))
                .shade(Shade::Muted)
                .show(ui);
            })
    });
    crate::dialogs::host::open(app, dialog);
}

/// The dialog's keys of the four kinds of change, in the order of [`ChangeKinds::NAMES`].
const KIND_KEYS: [&str; 4] = ["kind_replaced", "kind_inserted", "kind_deleted", "kind_visual"];

/// The dialog's keys of the two Ignore checkboxes.
const IGNORE_CASE: &str = "ignore_case";
const IGNORE_PUNCTUATION: &str = "ignore_punctuation";

/// The kinds as the values of the dialog's checkboxes.
fn kind_values(kinds: ChangeKinds) -> [(&'static str, bool); 4] {
    [(KIND_KEYS[0], kinds.replaced), (KIND_KEYS[1], kinds.inserted), (KIND_KEYS[2], kinds.deleted), (KIND_KEYS[3], kinds.visual)]
}

/// The kinds the dialog's checkboxes hold (a missing value is on, the default).
fn kinds_of(fields: &DialogFields) -> ChangeKinds {
    let on = |key: &str| fields.bool(key).unwrap_or(true);
    ChangeKinds { replaced: on(KIND_KEYS[0]), inserted: on(KIND_KEYS[1]), deleted: on(KIND_KEYS[2]), visual: on(KIND_KEYS[3]) }
}

/// Once per frame: when the Compare Files dialog answered OK, runs the comparison it asked for.
pub(crate) fn pass(app: &mut PdfCraftApp, ctx: &egui::Context) {
    let Some(DialogResult::Ok(result)) = app.dialogs.take_by_id::<HostedParamDialog>(dialog_id(DIALOG_KEY)) else { return };
    let chosen = result.fields.text("old").unwrap_or_default();
    let Some(old) = app.views.iter().map(|v| v.id).find(|id| option_key(*id) == chosen) else { return };
    app.compare_old = Some(old);
    app.compare_kinds = kinds_of(&result.fields);
    app.compare_ignore_case = result.fields.bool(IGNORE_CASE).unwrap_or(false);
    app.compare_ignore_punctuation = result.fields.bool(IGNORE_PUNCTUATION).unwrap_or(false);
    app.compare_old_pages = result.fields.text("old_pages").unwrap_or_default().to_owned();
    app.compare_pages = result.fields.text("pages").unwrap_or_default().to_owned();
    app.compare_fields_for = app.active_ids().map(|(_, new)| (new, Some(old)));
    app.run_compare(ctx);
}

/// The text a change shows in the list.
fn change_text(ch: &pdfcraft_engine::compare::Change) -> String {
    match ch.kind {
        Kind::Replaced => format!("\"{}\" → \"{}\"", ch.old.text, ch.new.text),
        Kind::Inserted => format!("+ \"{}\"", ch.new.text),
        Kind::Deleted => format!("− \"{}\"", ch.old.text),
    }
}

/// The kind's name, written out so the catalogue extraction sees each word.
fn kind_label(kind: Kind) -> String {
    match kind {
        Kind::Replaced => tl!("Replaced").to_string(),
        Kind::Inserted => tl!("Inserted").to_string(),
        Kind::Deleted => tl!("Deleted").to_string(),
    }
}

/// A count of one kind of difference after its coloured badge.
fn count_row(ui: &mut egui::Ui, colour: Color32, n: usize, kind: &str) {
    let text = crate::i18n::fmt(tl!("{n} {kind}"), &[("n", &n.to_string()), ("kind", kind)]);
    ListRow::new(text).icon(IconSpec::svg(IconId::new("square"))).hue(DataColor::new(colour), HueTarget::Icon).show(ui);
}

/// The Compare panel, a dock body.
pub(crate) fn panel(ui: &mut egui::Ui, state: &Option<CompareState>, id: DocId) -> Option<PanelAction> {
    let Some(s) = state.as_ref().filter(|s| s.new == id) else {
        Notice::new(Tone::Info, tl!("Compare this file with an older version: Compare files… in All tools.").to_string()).show(ui);
        return None;
    };
    let mut action = None;
    let c = &s.result;
    Stack::new().pad(Pad::Control).show(ui, |ui, _| {
        if c.identical() {
            Notice::new(Tone::Info, tl!("No differences in the text.").to_string()).show(ui);
        }
        for k in [Kind::Replaced, Kind::Inserted, Kind::Deleted] {
            count_row(ui, colour(k), c.count(k), &kind_label(k));
        }
        count_row(ui, VISUAL, s.visual.len(), tl!("Visual"));
        let ranged = s.old_range.is_some() || s.new_range.is_some();
        if ranged {
            let show = |range: Option<PageRange>| match range {
                None => tl!("all").to_string(),
                Some(r) if r.from == r.to => r.from.to_string(),
                Some(r) => format!("{}-{}", r.from, r.to),
            };
            let text = crate::i18n::fmt(
                tl!("Compared pages: old file {old}, new file {new}"),
                &[("old", show(s.old_range).as_str()), ("new", show(s.new_range).as_str())],
            );
            Text::new(text).shade(Shade::Muted).show(ui);
        }
        Row::new().wrap(true).show(ui, |ui, _| {
            Text::new(tl!("Show:").to_string()).show(ui);
            let mut kinds = s.kinds;
            Checkbox::new(Bind::bound(&mut kinds.replaced)).label(tl!("Replaced").to_string()).show(ui);
            Checkbox::new(Bind::bound(&mut kinds.inserted)).label(tl!("Inserted").to_string()).show(ui);
            Checkbox::new(Bind::bound(&mut kinds.deleted)).label(tl!("Deleted").to_string()).show(ui);
            Checkbox::new(Bind::bound(&mut kinds.visual)).label(tl!("Visual").to_string()).show(ui);
            if kinds != s.kinds {
                action = Some(PanelAction::Show(kinds));
            }
        });
        let entries = [Entry::new(true, tl!("Side by side").to_string()), Entry::new(false, tl!("New file only").to_string())];
        let mut side = s.side_by_side;
        Segmented::new(tl!("Layout").to_string(), Bind::bound(&mut side), &entries).show(ui);
        if side != s.side_by_side {
            action = Some(PanelAction::SideBySide(side));
        }
        Row::new().show(ui, |ui, _| {
            // With no kind ticked there is nothing to mark or report.
            let any = !s.kinds.is_empty();
            if Button::new(tl!("Mark as comments").to_string()).enabled(any).show(ui).clicked() {
                action = Some(PanelAction::Mark);
            }
            if Button::new(tl!("Report…").to_string()).enabled(any).show(ui).clicked() {
                action = Some(PanelAction::Report);
            }
            if Button::new(tl!("Clear").to_string()).show(ui).clicked() {
                action = Some(PanelAction::Clear);
            }
        });
        let order: Vec<usize> = (0..c.changes.len()).filter(|i| c.changes.get(*i).is_some_and(|ch| s.kinds.wants(ch.kind))).take(2000).collect();
        let mut selection = Selection::new(s.selected.into_iter().collect(), s.selected, s.selected);
        let rows = keyed(&order, |i| *i);
        List::new(tl!("Differences").to_string()).select(SelectMode::Single).selection(Bind::bound(&mut selection)).show_items(
            ui,
            &rows,
            &mut |key, item| {
                let Some(ch) = c.changes.get(*key) else { return ListRow::new(String::new()) };
                let kind = kind_label(ch.kind);
                let page = (ch.new.page + 1).to_string();
                // With different ranges the two files' pages differ: name both.
                let detail = if ranged && ch.old.page != ch.new.page {
                    crate::i18n::fmt(
                        tl!("{kind} · page {p} (old file page {o})"),
                        &[("kind", kind.as_str()), ("p", page.as_str()), ("o", (ch.old.page + 1).to_string().as_str())],
                    )
                } else {
                    crate::i18n::fmt(tl!("{kind} · page {p}"), &[("kind", kind.as_str()), ("p", page.as_str())])
                };
                ListRow::new(change_text(ch))
                    .detail(detail)
                    .hue(DataColor::new(colour(ch.kind)), HueTarget::Icon)
                    .icon(IconSpec::svg(IconId::new("square")))
                    .state(item.row.clone())
            },
        );
        if let Some(picked) = selection.items.first().copied().filter(|p| Some(*p) != s.selected) {
            action = Some(PanelAction::Select(picked));
        }
    });
    action
}

/// Why a Pages field of the Compare Files dialog can't be used.
#[derive(Debug, PartialEq, Eq)]
pub enum RangeError {
    /// Not a page or a range of two pages.
    NotARange,
    /// A page below 1 or past the last one.
    Outside,
}

impl RangeError {
    /// The message, with `{file}` and `{n}` (the file's page count) still to fill in.
    fn message(&self) -> String {
        match self {
            RangeError::NotARange => tl!("{file}: enter one page or one range, such as 3 or 2-5").to_string(),
            RangeError::Outside => tl!("{file}: pages must be within 1-{n}").to_string(),
        }
    }
}

/// The hint of an empty Pages field: the whole file (`no_file` while there is no file yet).
fn whole_document_hint(count: usize, no_file: &str) -> String {
    if count == 0 { no_file.to_string() } else { format!("1-{count}") }
}

/// A Pages field of the Compare Files dialog: empty for the whole file (`None`), one page ("3")
/// or one range ("2-5", the ends in either order) of a file of `count` pages. Nothing is clamped.
pub fn parse_range(text: &str, count: usize) -> Result<Option<PageRange>, RangeError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let number = |t: &str| {
        let t = t.trim();
        if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
            return Err(RangeError::NotARange);
        }
        // Digits too long for a number are as far outside the file as any other.
        match t.parse::<usize>() {
            Ok(n) if (1..=count).contains(&n) => Ok(n),
            _ => Err(RangeError::Outside),
        }
    };
    let (a, b) = match text.split_once(['-', '\u{2013}']) {
        Some((a, b)) => (number(a)?, number(b)?),
        None => {
            let n = number(text)?;
            (n, n)
        }
    };
    Ok(Some(PageRange { from: a.min(b), to: a.max(b) }))
}

/// What is wrong with a Pages field, and the page count of its file.
type FieldError = Option<(RangeError, usize)>;

/// Both Pages fields of files of `old_n` and `new_n` pages as ranges of the old and the new file,
/// and what is wrong with each field. With a range for one file only, the other file uses the same
/// pages, and is at fault if it doesn't have them. While there is no old file (`old_n` is 0) its
/// field isn't checked.
fn field_ranges(old_text: &str, old_n: usize, new_text: &str, new_n: usize) -> ((Option<PageRange>, Option<PageRange>), [FieldError; 2]) {
    let old = if old_n == 0 { Ok(None) } else { parse_range(old_text, old_n) };
    let new = parse_range(new_text, new_n);
    let mut errors = [None, None];
    let (mut old_range, mut new_range) = (old.as_ref().ok().copied().flatten(), new.as_ref().ok().copied().flatten());
    if let Err(e) = old {
        errors[0] = Some((e, old_n));
    }
    if let Err(e) = new {
        errors[1] = Some((e, new_n));
    }
    if errors.iter().all(Option::is_none) && old_n > 0 {
        match (old_range, new_range) {
            (Some(r), None) if r.to > new_n => errors[1] = Some((RangeError::Outside, new_n)),
            (None, Some(r)) if r.to > old_n => errors[0] = Some((RangeError::Outside, old_n)),
            (Some(r), None) => new_range = Some(r),
            (None, Some(r)) => old_range = Some(r),
            _ => {}
        }
    }
    ((old_range, new_range), errors)
}

/// The messages for what is wrong with the two Pages fields, each naming its file and page count.
fn range_messages(errors: [FieldError; 2]) -> Vec<String> {
    [tl!("Old file"), tl!("New file")]
        .into_iter()
        .zip(errors)
        .filter_map(|(file, error)| {
            let (e, count) = error?;
            Some(crate::i18n::fmt(&e.message(), &[("file", file), ("n", count.to_string().as_str())]))
        })
        .collect()
}

/// Both Pages fields as ranges of the old and the new file, or the first message to show.
fn ranges(app: &PdfCraftApp, old: DocId, new: DocId) -> Result<(Option<PageRange>, Option<PageRange>), String> {
    let count = |id| app.session.get(id).map_or(0, |d| d.info.pages.len());
    let (ranges, errors) = field_ranges(&app.compare_old_pages, count(old), &app.compare_pages, count(new));
    match range_messages(errors).into_iter().next() {
        Some(message) => Err(message),
        None => Ok(ranges),
    }
}

impl PdfCraftApp {
    /// Set gaps for the active document and both visible comparison panes, leaving other tabs alone.
    pub(crate) fn set_active_page_gaps(&mut self, show: bool) {
        let Some((index, id)) = self.active_ids() else { return };
        let partner = side_by_side(self, index).and_then(|(old, _)| self.views.get(old).map(|v| v.id));
        for view in &mut self.views {
            if view.id == id || partner == Some(view.id) {
                view.view.page_gaps = show;
            }
        }
    }

    /// Compare the chosen older document with the active one and show the differences.
    pub fn run_compare(&mut self, ctx: &egui::Context) {
        let Some((i, new)) = self.active_ids() else { return };
        let Some(old) = self.compare_old else { return };
        let (old_range, new_range) = match ranges(self, old, new) {
            Ok(ranges) => ranges,
            Err(message) => return self.notify_error(message),
        };
        let options = WordOptions { ignore_case: self.compare_ignore_case, ignore_punctuation: self.compare_ignore_punctuation };
        match self.session.compare_in_options(old, new, old_range, new_range, options) {
            Ok(result) => {
                // The pages of the two ranges are paired by position, as `doc_compare` does.
                let visual = self.session.compare_visual_in(old, new, 72.0, old_range, new_range).unwrap_or_default();
                let n = result.changes.len();
                // Side by side when the old version is open in a tab and the window has the room.
                let side_by_side = self.views.iter().any(|v| v.id == old) && ctx.content_rect().width() >= 1000.0;
                self.compare = Some(crate::compare_ui::CompareState {
                    old,
                    new,
                    result,
                    old_range,
                    new_range,
                    visual,
                    selected: None,
                    side_by_side,
                    views: Default::default(),
                    kinds: self.compare_kinds,
                    options,
                });
                self.shade_compare(i);
                self.set_right(Some(RightPanel::Compare));
                self.notify(if n == 0 {
                    tl!("The text is the same").to_string()
                } else if n == 1 {
                    tl!("1 difference found").to_string()
                } else {
                    crate::i18n::fmt(tl!("{n} differences found"), &[("n", &n.to_string())])
                });
            }
            Err(e) => self.notify_error(e),
        }
    }

    pub(crate) fn compare_action(&mut self, ctx: &egui::Context, index: usize, a: PanelAction) {
        let Some(s) = self.compare.as_mut() else { return };
        let (old, new, old_range, new_range, kinds, options) = (s.old, s.new, s.old_range, s.new_range, s.kinds, s.options);
        match a {
            PanelAction::Select(i) => {
                s.selected = Some(i);
                let Some(ch) = s.result.changes.get(i) else { return };
                let rect = ch.new.rects.first().copied().or(ch.new.near).unwrap_or_default();
                let page = ch.new.page;
                let old_target = (ch.old.page, ch.old.rects.first().copied().or(ch.old.near).unwrap_or_default());
                let v = &mut self.views[index];
                v.go_to_page(page);
                v.session.flash = Some((page, rect.map(|x| x as f32), 0.0));
                // The old file's half jumps to its side of the change too.
                if let Some(oi) = self.views.iter().position(|v| v.id == old) {
                    let v = &mut self.views[oi];
                    v.go_to_page(old_target.0);
                    v.session.flash = Some((old_target.0, old_target.1.map(|x| x as f32), 0.0));
                }
            }
            PanelAction::SideBySide(on) => {
                s.side_by_side = on;
            }
            PanelAction::Show(shown) => {
                s.kinds = shown;
                self.shade_compare(index);
            }
            PanelAction::Mark => match self.session.mark_differences_in_options(old, new, old_range, new_range, kinds, options) {
                Ok(n) => {
                    if let Some(info) = self.session.get(new).map(|d| d.info.clone()) {
                        self.views[index].document_changed(ctx, &info);
                    }
                    self.notify(if n == 0 && !(kinds.replaced || kinds.inserted || kinds.deleted) {
                        tl!("No comments added: visual differences are shaded, not marked as comments").to_string()
                    } else if n == 1 {
                        tl!("Added 1 comment").to_string()
                    } else {
                        crate::i18n::fmt(tl!("Added {n} comments"), &[("n", &n.to_string())])
                    });
                }
                Err(e) => self.notify_error(e),
            },
            PanelAction::Report => match self.session.compare_report_in_options(old, new, old_range, new_range, kinds, options) {
                Ok(bytes) => {
                    if let Err(e) = self.open_bytes(ctx, "Compare Report.pdf", None, bytes.to_vec()) {
                        self.notify_error(e);
                    }
                }
                Err(e) => self.notify_error(e),
            },
            PanelAction::Clear => {
                self.compare = None;
                self.views[index].session.compare_marks.clear();
                if let Some(oi) = self.views.iter().position(|v| v.id == old) {
                    self.views[oi].session.compare_marks.clear();
                }
                self.set_right(None);
            }
        }
    }
}

impl PdfCraftApp {
    /// Shade the comparison on the new file's pages (view `index`) and, when it is open, the old
    /// file's, with the kinds of change the filter shows.
    fn shade_compare(&mut self, index: usize) {
        let Some(s) = self.compare.as_ref() else { return };
        let old = s.old;
        let (old_marks, new_marks) = (shading(s, true), shading(s, false));
        if let Some(v) = self.views.get_mut(index) {
            v.session.compare_marks = new_marks;
        }
        if let Some(v) = self.views.iter_mut().find(|v| v.id == old) {
            v.session.compare_marks = old_marks;
        }
    }
}

/// One half's shading: its side of the text changes (the old file has no side of an insertion)
/// and the visual differences no text change there covers, so both versions read with their own
/// shading. Hidden kinds are left out after the overlap test, so hiding a kind of text change does not
/// uncover the visual differences a text change explains.
fn shading(s: &CompareState, old: bool) -> Vec<(usize, [f32; 4], Color32)> {
    let overlaps = |p: usize, r: [f32; 4], marks: &[(usize, [f32; 4], Color32)]| {
        marks.iter().any(|(mp, m, _)| *mp == p && m[0] <= r[2] && r[0] <= m[2] && m[1] <= r[3] && r[1] <= m[3])
    };
    let (mut text, mut shown) = (Vec::new(), Vec::new());
    for ch in s.result.changes.iter().filter(|ch| !(old && ch.kind == Kind::Inserted)) {
        let side = if old { &ch.old } else { &ch.new };
        let marks: Vec<(usize, [f32; 4], Color32)> = side.rects.iter().map(|r| (side.page, r.map(|v| v as f32), colour(ch.kind))).collect();
        // Every text change counts for the overlap test; only the kinds shown are shaded.
        if s.kinds.wants(ch.kind) {
            shown.extend(marks.iter().copied());
        }
        text.extend(marks);
    }
    let mut visual = Vec::new();
    for (old_page, new_page, r) in &s.visual {
        // Each half's regions are on its own pages.
        let p = if old { *old_page } else { *new_page };
        let r = r.map(|v| v as f32);
        // The old file's half also skips a region overlapping one it already shades.
        if !overlaps(p, r, &text) && !(old && overlaps(p, r, &visual)) {
            visual.push((p, r, VISUAL));
        }
    }
    let mut out = shown;
    if s.kinds.visual {
        out.extend(visual);
    }
    out
}

/// The two halves of a side-by-side comparison, as view indexes (old, new), when the active
/// document is the new file of a comparison shown side by side and the old file is open too.
pub(crate) fn side_by_side(app: &PdfCraftApp, index: usize) -> Option<(usize, usize)> {
    let c = app.compare.as_ref().filter(|c| c.side_by_side)?;
    let new = app.views.get(index).filter(|v| v.id == c.new)?;
    let old = app.views.iter().position(|v| v.id == c.old)?;
    (old != index).then_some((old, app.views.iter().position(|v| v.id == new.id)?))
}

/// Side by side: the old file on the left, the new on the right, each a page view of its own
/// with its shading. The pair keeps them scrolled and zoomed together, the half last pressed leading.
pub(crate) fn compare_area(app: &mut PdfCraftApp, old: usize, new: usize, ui: &mut egui::Ui) {
    let (Some(old_view), Some(new_view)) = (app.views.get(old), app.views.get(new)) else { return };
    let ids = [old_view.id, new_view.id];
    let mut states = (old_view.view.clone(), new_view.view.clone());
    let names = ids.map(|id| app.session.get(id).map(|d| d.name.clone()).unwrap_or_default());
    let Some(mut pair) = app.compare.as_mut().map(|c| std::mem::take(&mut c.views)) else { return };
    let left = crate::i18n::fmt(tl!("Old file: {name}"), &[("name", &names[0])]);
    let right = crate::i18n::fmt(tl!("New file: {name}"), &[("name", &names[1])]);
    SyncedViews::new(tl!("Comparison").to_string(), &mut pair).captions(left, right).halves(
        ui,
        (&mut states.0, &mut states.1),
        |ui, rect, state, half| {
            let index = if half == Lead::Left { old } else { new };
            // The app draws through its own view state: the pair's state for this half stands in for it during the draw.
            if let Some(v) = app.views.get_mut(index) {
                std::mem::swap(&mut v.view, state);
            }
            // Each half's notices lie within its own half: draw in a child ui bounded by the half.
            let mut half_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(("compare-half", index)));
            half_ui.shrink_clip_rect(rect);
            let output = crate::canvas::document_area_at(app, index, rect, &mut half_ui);
            if let Some(v) = app.views.get_mut(index) {
                std::mem::swap(&mut v.view, state);
            }
            output
        },
    );
    // A half can close its own tab while it draws (#3915): each is found again by id.
    for (id, state) in ids.into_iter().zip([states.0, states.1]) {
        if let Some(v) = app.views.iter_mut().find(|v| v.id == id) {
            v.view = state;
        }
    }
    if let Some(c) = app.compare.as_mut() {
        c.views = pair;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(from: usize, to: usize) -> Option<PageRange> {
        Some(PageRange { from, to })
    }

    #[test]
    fn one_filled_field_applies_to_both_files() {
        let f = field_ranges;
        assert_eq!(f("", 4, "2", 4), ((r(2, 2), r(2, 2)), [None, None]));
        assert_eq!(f("2-3", 4, "", 4), ((r(2, 3), r(2, 3)), [None, None]));
        assert_eq!(f("", 4, "", 4), ((None, None), [None, None]));
        assert_eq!(f("1", 4, "3", 4), ((r(1, 1), r(3, 3)), [None, None]));
        // The other file lacks the pages: it is named, with its page count.
        let (_, e) = f("", 6, "5", 6);
        assert_eq!(e, [None, None]);
        let (_, e) = f("5", 6, "", 3);
        assert_eq!(e, [None, Some((RangeError::Outside, 3))]);
        let (_, e) = f("", 2, "3", 4);
        assert_eq!(e, [Some((RangeError::Outside, 2)), None]);
        // No old file yet: its field isn't checked.
        assert_eq!(f("9", 0, "2", 4), ((None, r(2, 2)), [None, None]));
    }

    #[test]
    fn a_pages_field_is_empty_one_page_or_one_range() {
        assert_eq!(parse_range("", 4), Ok(None));
        assert_eq!(parse_range("  ", 4), Ok(None));
        assert_eq!(parse_range("3", 4), Ok(r(3, 3)));
        assert_eq!(parse_range("2-4", 4), Ok(r(2, 4)));
        assert_eq!(parse_range(" 2 \u{2013} 3 ", 4), Ok(r(2, 3)));
        assert_eq!(parse_range("4-2", 4), Ok(r(2, 4)), "ends in either order");
    }

    #[test]
    fn a_pages_field_never_clamps_or_panics() {
        for bad in ["0", "5", "0-2", "1-5", "99999999999999999999999", "1-99999999999999999999999"] {
            assert_eq!(parse_range(bad, 4), Err(RangeError::Outside), "{bad}");
        }
        for bad in ["a", "-", "1-", "-3", "1,3", "1-2-3", "1;2", "+2", "\u{ff11}"] {
            assert_eq!(parse_range(bad, 4), Err(RangeError::NotARange), "{bad}");
        }
        assert_eq!(parse_range("1", 0), Err(RangeError::Outside), "a file with no pages has no page 1");
    }
}
