//! Compare files: choose the older version, then the Compare panel lists the differences
//! (replaced blue, inserted green, deleted red), shades them on the newer document's pages, and
//! offers a report or the differences as comments.

use egui::{Align, Color32, Layout};
use pdfcraft_engine::DocId;
use pdfcraft_engine::compare::{Comparison, Kind, PageRange};

use crate::theme::{self, Tokens};
use crate::{PdfCraftApp, RightPanel, widgets};

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
    /// Which half leads the other (the one the pointer was over last): 0 old, 1 new.
    pub leader: usize,
    /// Each half's scroll offset last frame, to tell which one moved.
    pub prev_scroll: [egui::Vec2; 2],
    /// Show: whether the text changes are in the list and the shading, and whether the visual
    /// differences (counted, never listed) are shaded (Acrobat's Filter).
    pub show_text: bool,
    pub show_visual: bool,
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
    Show { text: bool, visual: bool },
}

pub fn colour(kind: Kind) -> Color32 {
    let [r, g, b] = pdfcraft_engine::compare::colour(kind);
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// The Compare Files dialog: pick the older document. Returns (compare, cancel).
pub(crate) fn body(ui: &mut egui::Ui, app: &mut PdfCraftApp, t: &Tokens) -> (bool, bool) {
    ui.label(egui::RichText::new(tl!("Compare Files")).font(theme::semibold(18.0)));
    ui.add_space(8.0);
    let Some((active_i, new)) = app.active_ids() else { return (false, true) };
    let others: Vec<(DocId, String)> =
        app.views.iter().filter(|v| v.id != new).filter_map(|v| app.session.get(v.id).map(|d| (d.id, d.name.clone()))).collect();
    if app.compare_old.is_none_or(|o| !others.iter().any(|(id, _)| *id == o)) {
        app.compare_old = others.first().map(|(id, _)| *id);
    }
    // The Pages fields belong to this pair of files: another pair starts empty.
    if app.compare_fields_for != Some((new, app.compare_old)) {
        app.compare_fields_for = Some((new, app.compare_old));
        app.compare_old_pages.clear();
        app.compare_pages.clear();
    }
    let new_name = app.session.get(new).map(|d| d.name.clone()).unwrap_or_default();
    let page_count = app.views.get(active_i).map_or(0, |v| v.page_count());
    let old_count = app.compare_old.and_then(|o| app.session.get(o)).map_or(0, |d| d.info.pages.len());
    egui::Grid::new("compare-files").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
        ui.label(tl!("Old file:"));
        let shown =
            others.iter().find(|(id, _)| Some(*id) == app.compare_old).map_or(tl!("Open the older version first").to_string(), |(_, n)| n.clone());
        craft_ui::ComboBox::from_id_salt("compare-old").selected_text(shown).width(320.0).show_ui(ui, |ui| {
            for (id, n) in &others {
                ui.selectable_value(&mut app.compare_old, Some(*id), n);
            }
        });
        ui.end_row();
        ui.label(tl!("Old file pages:"));
        let field = ui.add(
            egui::TextEdit::singleline(&mut app.compare_old_pages)
                .desired_width(320.0)
                .hint_text(whole_document_hint(old_count, tl!("Choose the old file first"))),
        );
        field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, ui.is_enabled(), tl!("Old file pages")));
        ui.end_row();
        ui.label(tl!("New file:"));
        ui.label(egui::RichText::new(new_name).strong());
        ui.end_row();
        ui.label(tl!("New file pages:"));
        let field = ui.add(egui::TextEdit::singleline(&mut app.compare_pages).desired_width(320.0).hint_text(whole_document_hint(page_count, "")));
        field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, ui.is_enabled(), tl!("New file pages")));
        ui.end_row();
    });
    let (_, errors) = field_ranges(&app.compare_old_pages, old_count, &app.compare_pages, page_count);
    let mut pages_error = false;
    for (file, error) in [tl!("Old file"), tl!("New file")].into_iter().zip(errors) {
        if let Some((e, count)) = error {
            pages_error = true;
            ui.label(crate::i18n::fmt(&e.message(), &[("file", file), ("n", &count.to_string())]));
        }
    }
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(tl!(
            "Leave both fields empty to compare the whole files. Give one range for each file, such as 2-5 or 3; the pages of the two ranges are compared with each other. With a range for one file only, the same pages of the other file are used."
        ))
        .small()
        .color(t.text_muted),
    );
    ui.label(egui::RichText::new(tl!("Text is compared word by word; the differences are shown on the new file.")).small().color(t.text_muted));
    ui.add_space(10.0);
    let (mut go, mut cancel) = (false, false);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let ready = app.compare_old.is_some() && !pages_error;
            if ui.add_enabled_ui(ready, |ui| widgets::pill_button(ui, tl!("Compare"), true)).inner.clicked() {
                go = true;
            }
            if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
                cancel = true;
            }
        })
    });
    (go, cancel)
}

/// The Compare panel.
pub(crate) fn panel(ui: &mut egui::Ui, t: &Tokens, state: &Option<CompareState>, id: DocId) -> Option<PanelAction> {
    let Some(s) = state.as_ref().filter(|s| s.new == id) else {
        ui.label(egui::RichText::new(tl!("Compare this file with an older version: Compare files… in All tools.")).color(t.text_muted));
        return None;
    };
    let mut action = None;
    let c = &s.result;
    if c.identical() {
        ui.label(tl!("No differences in the text."));
    }
    if s.old_range.is_some() || s.new_range.is_some() {
        let show = |r: Option<PageRange>| match r {
            None => tl!("all").to_string(),
            Some(r) if r.from == r.to => r.from.to_string(),
            Some(r) => format!("{}-{}", r.from, r.to),
        };
        ui.label(
            egui::RichText::new(crate::i18n::fmt(
                tl!("Compared pages: old file {old}, new file {new}"),
                &[("old", &show(s.old_range)), ("new", &show(s.new_range))],
            ))
            .small()
            .color(t.text_muted),
        );
    }
    ui.horizontal_wrapped(|ui| {
        for k in [Kind::Replaced, Kind::Inserted, Kind::Deleted] {
            let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().rect_filled(dot, egui::CornerRadius::same(2), colour(k));
            ui.label(crate::i18n::fmt(tl!("{n} {kind}"), &[("n", &c.count(k).to_string()), ("kind", tl!(k.label()))]));
        }
        let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
        ui.painter().rect_filled(dot, egui::CornerRadius::same(2), VISUAL);
        ui.label(crate::i18n::fmt(tl!("{n} {kind}"), &[("n", &s.visual.len().to_string()), ("kind", tl!("Visual"))]));
    });
    ui.horizontal(|ui| {
        ui.label(tl!("Show:"));
        let (mut text, mut visual) = (s.show_text, s.show_visual);
        let text_changed = ui.add(widgets::checkbox(&mut text, tl!("Text"))).changed();
        if ui.add(widgets::checkbox(&mut visual, tl!("Visual"))).changed() || text_changed {
            action = Some(PanelAction::Show { text, visual });
        }
    });
    ui.add_space(6.0);
    // Side by side, or the new file alone.
    ui.horizontal(|ui| {
        if widgets::pill_button(ui, tl!("Side by side"), s.side_by_side).clicked() {
            action = Some(PanelAction::SideBySide(true));
        }
        if widgets::pill_button(ui, tl!("New file only"), !s.side_by_side).clicked() {
            action = Some(PanelAction::SideBySide(false));
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button(tl!("Mark as comments")).clicked() {
            action = Some(PanelAction::Mark);
        }
        if ui.button(tl!("Report…")).clicked() {
            action = Some(PanelAction::Report);
        }
        if ui.button(tl!("Clear")).clicked() {
            action = Some(PanelAction::Clear);
        }
    });
    ui.separator();
    for (i, ch) in c.changes.iter().enumerate().filter(|_| s.show_text).take(2000) {
        let text = match ch.kind {
            Kind::Replaced => format!("\"{}\" → \"{}\"", ch.old.text, ch.new.text),
            Kind::Inserted => format!("+ \"{}\"", ch.new.text),
            Kind::Deleted => format!("− \"{}\"", ch.old.text),
        };
        let selected = s.selected == Some(i);
        let r = ui.push_id(i, |ui| {
            ui.horizontal(|ui| {
                let (bar, _) = ui.allocate_exact_size(egui::vec2(4.0, 34.0), egui::Sense::hover());
                ui.painter().rect_filled(bar, egui::CornerRadius::same(2), colour(ch.kind));
                ui.vertical(|ui| {
                    // With different ranges the two files' pages differ: name both.
                    let at = if (s.old_range.is_some() || s.new_range.is_some()) && ch.old.page != ch.new.page {
                        crate::i18n::fmt(
                            tl!("{kind} · page {p} (old file page {o})"),
                            &[("kind", tl!(ch.kind.label())), ("p", &(ch.new.page + 1).to_string()), ("o", &(ch.old.page + 1).to_string())],
                        )
                    } else {
                        crate::i18n::fmt(tl!("{kind} · page {p}"), &[("kind", tl!(ch.kind.label())), ("p", &(ch.new.page + 1).to_string())])
                    };
                    ui.label(egui::RichText::new(at).small().color(t.text_muted));
                    ui.selectable_label(selected, text)
                })
                .inner
            })
            .inner
        });
        if r.inner.clicked() {
            action = Some(PanelAction::Select(i));
        }
    }
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

/// Both Pages fields as ranges of the old and the new file, or the first message to show.
fn ranges(app: &PdfCraftApp, old: DocId, new: DocId) -> Result<(Option<PageRange>, Option<PageRange>), String> {
    let count = |id| app.session.get(id).map_or(0, |d| d.info.pages.len());
    let (old_n, new_n) = (count(old), count(new));
    let (ranges, errors) = field_ranges(&app.compare_old_pages, old_n, &app.compare_pages, new_n);
    for (file, error) in [tl!("Old file"), tl!("New file")].into_iter().zip(errors) {
        if let Some((e, n)) = error {
            return Err(crate::i18n::fmt(&e.message(), &[("file", file), ("n", &n.to_string())]));
        }
    }
    Ok(ranges)
}

impl PdfCraftApp {
    /// Set gaps for the active document and both visible comparison panes, leaving other tabs alone.
    pub(crate) fn set_active_page_gaps(&mut self, show: bool) {
        let Some((index, id)) = self.active_ids() else { return };
        let partner = side_by_side(self, index).and_then(|(old, _)| self.views.get(old).map(|v| v.id));
        for view in &mut self.views {
            if view.id == id || partner == Some(view.id) {
                view.set_page_gaps(show);
            }
        }
    }

    /// Compare the chosen older document with the active one and show the differences.
    pub fn run_compare(&mut self) {
        let Some((i, new)) = self.active_ids() else { return };
        let Some(old) = self.compare_old else { return };
        let (old_range, new_range) = match ranges(self, old, new) {
            Ok(r) => r,
            Err(e) => return self.notify_error(e),
        };
        match self.session.compare_in(old, new, old_range, new_range) {
            Ok(result) => {
                // The pages of the two ranges are paired by position, as `doc_compare` does.
                let visual = self.session.compare_visual_in(old, new, 72.0, old_range, new_range).unwrap_or_default();
                let n = result.changes.len();
                // Side by side when the old version is open in a tab and the window has the room.
                let side_by_side = self.views.iter().any(|v| v.id == old) && self.ctx.as_ref().is_none_or(|c| c.content_rect().width() >= 1000.0);
                self.compare = Some(crate::compare_ui::CompareState {
                    old,
                    new,
                    result,
                    old_range,
                    new_range,
                    visual,
                    selected: None,
                    side_by_side,
                    leader: 1,
                    prev_scroll: [egui::Vec2::ZERO; 2],
                    show_text: true,
                    show_visual: true,
                });
                self.shade_compare(i);
                self.right = Some(RightPanel::Compare);
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

    pub(crate) fn compare_action(&mut self, index: usize, a: PanelAction) {
        let Some(s) = self.compare.as_mut() else { return };
        let (old, new, old_range, new_range) = (s.old, s.new, s.old_range, s.new_range);
        match a {
            PanelAction::Select(i) => {
                s.selected = Some(i);
                let Some(ch) = s.result.changes.get(i) else { return };
                let rect = ch.new.rects.first().copied().or(ch.new.near).unwrap_or_default();
                let page = ch.new.page;
                let old_target = (ch.old.page, ch.old.rects.first().copied().or(ch.old.near).unwrap_or_default());
                let v = &mut self.views[index];
                v.go_to_page(page);
                v.flash = Some((page, rect.map(|x| x as f32), 0.0));
                // The old file's half jumps to its side of the change too.
                if let Some(oi) = self.views.iter().position(|v| v.id == old) {
                    let v = &mut self.views[oi];
                    v.go_to_page(old_target.0);
                    v.flash = Some((old_target.0, old_target.1.map(|x| x as f32), 0.0));
                }
            }
            PanelAction::SideBySide(on) => {
                s.side_by_side = on;
                s.leader = 1;
            }
            PanelAction::Show { text, visual } => {
                s.show_text = text;
                s.show_visual = visual;
                self.shade_compare(index);
            }
            PanelAction::Mark => match self.session.mark_differences_in(old, new, old_range, new_range) {
                Ok(n) => {
                    if let Some(info) = self.session.get(new).map(|d| d.info.clone()) {
                        self.views[index].document_changed(&info);
                    }
                    self.notify(if n == 1 {
                        tl!("Added 1 comment").to_string()
                    } else {
                        crate::i18n::fmt(tl!("Added {n} comments"), &[("n", &n.to_string())])
                    });
                }
                Err(e) => self.notify_error(e),
            },
            PanelAction::Report => match self.session.compare_report_in(old, new, old_range, new_range) {
                Ok(bytes) => {
                    if let Err(e) = self.open_bytes("Compare Report.pdf", None, bytes.to_vec()) {
                        self.notify_error(e);
                    }
                }
                Err(e) => self.notify_error(e),
            },
            PanelAction::Clear => {
                self.compare = None;
                self.views[index].compare_marks.clear();
                if let Some(oi) = self.views.iter().position(|v| v.id == old) {
                    self.views[oi].compare_marks.clear();
                }
                self.right = None;
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
            v.compare_marks = new_marks;
        }
        if let Some(v) = self.views.iter_mut().find(|v| v.id == old) {
            v.compare_marks = old_marks;
        }
    }
}

/// One half's shading: its side of the text changes (the old file has no side of an insertion)
/// and the visual differences no text change there covers, so both versions read with their own
/// shading. Hidden kinds are left out after the overlap test, so hiding the text does not uncover
/// the visual differences a text change explains.
fn shading(s: &CompareState, old: bool) -> Vec<(usize, [f32; 4], Color32)> {
    let overlaps = |p: usize, r: [f32; 4], marks: &[(usize, [f32; 4], Color32)]| {
        marks.iter().any(|(mp, m, _)| *mp == p && m[0] <= r[2] && r[0] <= m[2] && m[1] <= r[3] && r[1] <= m[3])
    };
    let mut text = Vec::new();
    for ch in s.result.changes.iter().filter(|ch| !(old && ch.kind == Kind::Inserted)) {
        let side = if old { &ch.old } else { &ch.new };
        text.extend(side.rects.iter().map(|r| (side.page, r.map(|v| v as f32), colour(ch.kind))));
    }
    let mut visual = Vec::new();
    for (old_page, new_page, r) in &s.visual {
        // Each half's regions are on its own pages.
        let p = if old { old_page } else { new_page };
        let r = r.map(|v| v as f32);
        // The old file's half also skips a region overlapping one it already shades.
        if !overlaps(*p, r, &text) && !(old && overlaps(*p, r, &visual)) {
            visual.push((*p, r, VISUAL));
        }
    }
    let mut out = if s.show_text { text } else { Vec::new() };
    if s.show_visual {
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
/// with its shading, scrolled and zoomed together. The half the pointer is over leads; the
/// other follows its zoom, layout and scroll offset.
pub(crate) fn compare_area(app: &mut PdfCraftApp, old: usize, new: usize, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let rect = ui.available_rect_before_wrap();
    let gap = 1.0;
    let half_w = ((rect.width() - gap) / 2.0).max(1.0);
    let caption_h = 22.0;
    let left = egui::Rect::from_min_size(rect.min, egui::vec2(half_w, rect.height()));
    let right = egui::Rect::from_min_size(egui::pos2(rect.min.x + half_w + gap, rect.min.y), egui::vec2(half_w, rect.height()));
    // Who leads: the half under the pointer (remembered), the new file at first.
    if let Some(p) = ui.input(|i| i.pointer.hover_pos())
        && let Some(c) = app.compare.as_mut()
    {
        if left.contains(p) {
            c.leader = 0;
        } else if right.contains(p) {
            c.leader = 1;
        }
    }
    let leader = app.compare.as_ref().map_or(1, |c| c.leader);
    let (lead, follow) = if leader == 0 { (old, new) } else { (new, old) };
    // The follower takes the leader's zoom and layout before either is drawn.
    let (zoom, fit, layout, continuous, page_gaps, rotation) = {
        let v = &app.views[lead];
        (v.zoom, v.fit, v.layout, v.continuous, v.show_page_gaps, v.rotation)
    };
    {
        let v = &mut app.views[follow];
        if v.zoom != zoom
            || v.fit != fit
            || v.layout != layout
            || v.continuous != continuous
            || v.show_page_gaps != page_gaps
            || v.rotation != rotation
        {
            v.zoom = zoom;
            v.fit = fit;
            v.layout = layout;
            v.continuous = continuous;
            v.show_page_gaps = page_gaps;
            v.rotation = rotation;
        }
    }
    let names = [old, new].map(|i| app.session.get(app.views[i].id).map(|d| d.name.clone()).unwrap_or_default());
    // Drawing a half can close its own tab (its snapshot bar's Close, #3915), which shifts or
    // removes the indexes below. Each half is found again by id, and the frame stops when one is gone.
    let ids = [app.views[old].id, app.views[new].id];
    let find = |app: &PdfCraftApp, id| app.views.iter().position(|v| v.id == id);
    ui.painter().vline(rect.min.x + half_w, rect.y_range(), egui::Stroke::new(gap, t.border));
    for (k, r) in [left, right].into_iter().enumerate() {
        // Both halves must still be open: once either has closed, the pair is over.
        let (Some(old_index), Some(new_index)) = (find(app, ids[0]), find(app, ids[1])) else { return };
        let index = if k == 0 { old_index } else { new_index };
        let caption = egui::Rect::from_min_size(r.min, egui::vec2(r.width(), caption_h));
        ui.painter().rect_filled(caption, 0.0, t.panel);
        let label = crate::i18n::fmt(if k == 0 { tl!("Old file: {name}") } else { tl!("New file: {name}") }, &[("name", &names[k])]);
        ui.painter().text(
            egui::pos2(caption.min.x + 10.0, caption.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(12.0),
            t.text_muted,
        );
        let body = egui::Rect::from_min_max(egui::pos2(r.min.x, r.min.y + caption_h), r.max);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).id_salt(("compare-half", k)));
        child.set_clip_rect(body);
        crate::canvas::document_area(app, index, &mut child);
    }
    // After both drew: the leader's scroll, when it moved, is the follower's next offset.
    let (Some(old), Some(new)) = (find(app, ids[0]), find(app, ids[1])) else { return };
    let follow = if leader == 0 { new } else { old };
    let offsets = [app.views[old].last_scroll, app.views[new].last_scroll];
    if let Some(c) = app.compare.as_mut() {
        let lead_k = if leader == 0 { 0 } else { 1 };
        if (offsets[lead_k] - c.prev_scroll[lead_k]).length() > 0.5 && (offsets[1 - lead_k] - offsets[lead_k]).length() > 0.5 {
            app.views[follow].sync_scroll = Some(offsets[lead_k]);
            ui.ctx().request_repaint();
        }
        c.prev_scroll = offsets;
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
        let f = |o, on, n, nn| field_ranges(o, on, n, nn);
        assert_eq!(f("", 4, "2", 4), ((r(2, 2), r(2, 2)), [None, None]));
        assert_eq!(f("2-3", 4, "", 4), ((r(2, 3), r(2, 3)), [None, None]));
        assert_eq!(f("", 4, "", 4), ((None, None), [None, None]));
        assert_eq!(f("1", 4, "3", 4), ((r(1, 1), r(3, 3)), [None, None]));
        // The other file lacks the pages: it is named, with its page count.
        let ((_, _), e) = f("", 6, "5", 6);
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
