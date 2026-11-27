//! Typing into a text box on the page, with bold and italic: a new box, or one being edited. The framework's canvas
//! text editor runs over the box; PdfCraft lays the text out as the box's appearance will (the same faces, widths and
//! line breaks), so what shows while typing is what is saved. Each character carries its own bold and italic, which
//! therefore move with the text as it is edited.
//!
//! Sticky notes, callouts and the other comments keep the composer card, as do text boxes this editor can't show as
//! drawn: on a turned page or view, in a face other than plain Helvetica, with an oval or circle border, or holding
//! Thai.
//!
//! Tab leaves the text for the Bold button and Shift+Tab for Cancel; in the bar, Tab and Shift+Tab walk the buttons and
//! come back to the text past either end, Escape goes back to the text and Cmd+Enter commits.

use std::ops::Range;

use craft::behavior::kurbo::Point;
use craft::pages::ui::PageBody;
use craft::text::ui::editor::{
    CaretLine, CharToggle, EditKind, EditorEnd, EditorMode, HostLine, HostRun, StyleKey, TextEditor, TextEditorHost, TextSelection,
};
use craft::ui::buttons::{Button, ToggleButton};
use craft::ui::canvas_controls::TaskBar;
use craft::ui::contract::{Bind, Politeness, Props as _, Show as _};
use craft::ui::runtime::access::announce;
use craft::ui::runtime::focus::FocusRequest;
use egui::{Color32, FontFamily, FontId, Pos2, Rect, Stroke, vec2};
use pdfcraft_engine::{BorderShape, Edit, NewAnnotation, Shape, TextRun, rich_line_ranges};
use pdfcraft_fonts::Std14;
use pdfcraft_render::DocInfo;

use super::state::{Composer, ComposerKind};
use super::{CommentPrefs, CommentTool, color32, text_box_rect};
use crate::canvas::PageSession;

/// Line height over font size, as the text box appearance lays its lines out.
const LINE: f64 = 1.2;
/// The bar's buttons: Bold, Italic, Post or Save, Cancel.
const BAR: usize = 4;
/// The widest a new box grows before it wraps.
const MAX_WIDTH: f64 = 300.0;

/// (bold, italic) of one character.
type Style = (bool, bool);

/// Which box is being typed into.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    /// A new box with its top-left at `at` (user space), `size` across and down when it was dragged out.
    New { at: [f64; 2], size: Option<[f64; 2]> },
    /// The box at `index` in the page's annotations, over `rect` (user space).
    Existing { index: usize, rect: [f64; 4] },
}

/// The text box being typed into on one document tab.
#[derive(Debug)]
pub struct RichBox {
    pub(crate) page: usize,
    target: Target,
    text: String,
    /// One style a character of `text`.
    styles: Vec<Style>,
    /// What the next typed text takes, with nothing selected.
    typing: Style,
    selection: TextSelection,
    /// The text and runs it opened with.
    before: (String, Vec<TextRun>),
    size: f64,
    color: [f64; 3],
    border: f64,
    /// The editor takes the keyboard in its next pass.
    focus: bool,
    /// The bar button with the keyboard, by its place in the bar.
    in_bar: Option<usize>,
    /// The bar button to give the keyboard to in the next pass.
    wanted: Option<usize>,
    /// Cmd+Enter was pressed in the bar.
    commit: bool,
}

/// Whether the composer `c` is typed on the page: a text box, where the page and view are upright and the box can be
/// shown as it will be drawn.
pub(crate) fn on_page(c: &Composer, cx: &BoxCx<'_>) -> bool {
    let &BoxCx { info, doc, view_rotation, prefs } = cx;
    if !view_rotation.is_multiple_of(360) || info.pages.get(c.page).is_none_or(|p| p.rotation % 360 != 0) {
        return false;
    }
    if c.text.chars().any(|ch| ('\u{0E00}'..='\u{0E7F}').contains(&ch)) {
        return false;
    }
    match c.kind {
        ComposerKind::TextBox { .. } => prefs.style(CommentTool::TextBox).border_shape == BorderShape::Rectangle,
        ComposerKind::Edit(index) => {
            doc.comment_text_is_helvetica(c.page, index)
                && info
                    .annotations
                    .iter()
                    .any(|a| a.page == c.page && a.index == index && a.subtype == "FreeText" && a.intent.as_deref().is_none_or(|i| i == "FreeText"))
        }
        _ => false,
    }
}

fn styles_of(text: &str, runs: &[TextRun]) -> Vec<Style> {
    let mut styles = vec![(false, false); text.chars().count()];
    for run in runs {
        for style in styles.iter_mut().take(run.end).skip(run.start) {
            *style = (style.0 || run.bold, style.1 || run.italic);
        }
    }
    styles
}

fn runs_of(styles: &[Style]) -> Vec<TextRun> {
    let mut runs: Vec<TextRun> = Vec::new();
    for (at, &(bold, italic)) in styles.iter().enumerate() {
        if !bold && !italic {
            continue;
        }
        match runs.last_mut() {
            Some(last) if last.end == at && (last.bold, last.italic) == (bold, italic) => last.end = at + 1,
            _ => runs.push(TextRun { start: at, end: at + 1, bold, italic }),
        }
    }
    runs
}

fn face((bold, italic): Style) -> Std14 {
    match (bold, italic) {
        (false, false) => Std14::Helvetica,
        (true, false) => Std14::HelveticaBold,
        (false, true) => Std14::HelveticaOblique,
        (true, true) => Std14::HelveticaBoldOblique,
    }
}

fn advance(ch: char, style: Style, size: f64) -> f64 {
    face(style).text_width(ch.encode_utf8(&mut [0; 4]), size)
}

impl RichBox {
    /// The text as typed so far.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The page it is on (0-based).
    pub fn page(&self) -> usize {
        self.page
    }

    /// Whether it is a box not yet on its page.
    pub(crate) fn is_new(&self) -> bool {
        matches!(self.target, Target::New { .. })
    }

    fn width_of_line(&self, chars: Range<usize>) -> f64 {
        self.text.chars().zip(&self.styles).skip(chars.start).take(chars.len()).map(|(ch, &s)| advance(ch, s, self.size)).sum()
    }

    /// The box in user space: an existing box's own width, else the one the new box will be given.
    fn rect(&self, info: &DocInfo) -> [f64; 4] {
        match self.target {
            Target::Existing { rect, .. } => rect,
            Target::New { at, size: Some(size) } => match info.pages.get(self.page) {
                Some(page) => super::drawn_text_box_rect(page, at, size, &self.text, self.size, self.border),
                None => text_box_rect(at, &self.text, self.size, 0),
            },
            Target::New { at, size: None } => {
                let mut rect = text_box_rect(at, &self.text, self.size, 0);
                // Bold lines are wider than the plain ones the box is measured by.
                let mut start = 0;
                let mut longest: f64 = 0.0;
                for line in self.text.split('\n') {
                    let end = start + line.chars().count();
                    longest = longest.max(self.width_of_line(start..end));
                    start = end + 1;
                }
                let wanted = (longest + 2.0 * (2.0 + self.border) + 4.0).min(MAX_WIDTH);
                rect[2] = rect[2].max(rect[0] + wanted);
                rect
            }
        }
    }

    fn all(&self, range: Range<usize>, of: impl Fn(Style) -> bool) -> bool {
        let mut any = false;
        let every = self.styles.iter().skip(range.start).take(range.len()).inspect(|_| any = true).all(|&s| of(s));
        any && every
    }

    /// Whether the selection (or what typing will take) is bold and italic.
    fn shown_style(&self) -> Style {
        let range = self.selection.range();
        if range.is_empty() { self.typing } else { (self.all(range.clone(), |s| s.0), self.all(range, |s| s.1)) }
    }

    /// Turns bold (or italic) on or off for the selection, or for what is typed next.
    fn toggle(&mut self, italic: bool) -> bool {
        let shown = self.shown_style();
        let on = !(if italic { shown.1 } else { shown.0 });
        let set = |style: &mut Style| if italic { style.1 = on } else { style.0 = on };
        let range = self.selection.range();
        if range.is_empty() {
            set(&mut self.typing);
        } else {
            self.styles.iter_mut().skip(range.start).take(range.len()).for_each(set);
        }
        on
    }

    /// What typing at the caret takes: the style of the character before it.
    fn follow_caret(&mut self) {
        let at = self.selection.range().start;
        self.typing = at.checked_sub(1).and_then(|i| self.styles.get(i)).or_else(|| self.styles.first()).copied().unwrap_or_default();
    }

    /// A key pressed with the keyboard on bar button `button`; false when the bar leaves it be.
    fn bar_key(&mut self, button: usize, key: egui::Key, modifiers: egui::Modifiers) -> bool {
        // Post (2) is passed over while there is nothing to post.
        let skip = |place: usize| place == 2 && !self.postable();
        match key {
            egui::Key::Escape if modifiers.is_none() => self.focus = true,
            egui::Key::Enter if modifiers.command => self.commit = true,
            egui::Key::Tab if modifiers.is_none() || modifiers.shift_only() => {
                let step = |place: usize| if modifiers.shift { place.checked_sub(1) } else { Some(place + 1).filter(|&p| p < BAR) };
                let mut next = step(button);
                if next.is_some_and(skip) {
                    next = next.and_then(step);
                }
                match next {
                    Some(place) => self.wanted = Some(place),
                    None => self.focus = true,
                }
            }
            _ => return false,
        }
        true
    }

    /// Whether there is something to save: a new box needs text, an existing one may be emptied.
    fn postable(&self) -> bool {
        !self.is_new() || !self.text.trim().is_empty()
    }

    /// The edit that saves what was typed, without trailing blank space, as the composer card saves it: `None` for an
    /// empty new box, or a box left as it was.
    fn edit(mut self, info: &DocInfo, prefs: &CommentPrefs) -> Option<Edit> {
        if !self.postable() {
            return None;
        }
        self.text.truncate(self.text.trim_end().len());
        self.styles.truncate(self.text.chars().count());
        let runs = runs_of(&self.styles);
        match self.target {
            Target::New { .. } => {
                let shape = Shape::TextBox { rect: self.rect(info), font_size: self.size };
                let new = NewAnnotation {
                    page: self.page,
                    shape,
                    style: prefs.style(CommentTool::TextBox),
                    contents: self.text,
                    author: prefs.author.clone(),
                };
                Some(if runs.is_empty() { Edit::AddAnnotation(new) } else { Edit::AddTextBox { new, runs } })
            }
            Target::Existing { index, .. } => {
                let (page, new_text) = (self.page, self.text != self.before.0);
                let mut edits = Vec::new();
                if new_text {
                    edits.push(Edit::SetAnnotationContents { page, index, text: self.text });
                }
                // New text drops the runs the box had: they are written again.
                if runs != self.before.1 || new_text && !runs.is_empty() {
                    edits.push(Edit::TextRunsAnnotation { page, index, runs });
                }
                match edits.len() {
                    0 | 1 => edits.pop(),
                    _ => Some(Edit::Batch { label: "Edit comment".into(), edits }),
                }
            }
        }
    }
}

/// Takes a text box the composer was opened for onto the page.
fn adopt(session: &mut PageSession, cx: &BoxCx<'_>) {
    if !session.comments.composer.as_ref().is_some_and(|c| on_page(c, cx)) {
        return;
    }
    let &BoxCx { info, doc, prefs, .. } = cx;
    let Some(c) = session.comments.composer.take() else { return };
    let text = c.text.replace("\r\n", "\n").replace('\r', "\n");
    let style = prefs.style(CommentTool::TextBox);
    let (target, runs, size, color, border) = match c.kind {
        ComposerKind::Edit(index) => {
            let Some(a) = info.annotations.iter().find(|a| a.page == c.page && a.index == index) else { return };
            let props = doc.comment_props(c.page, index);
            let rect = a.rect.map(f64::from);
            (
                Target::Existing { index, rect },
                doc.comment_text_runs(c.page, index),
                props.as_ref().and_then(|p| p.font_size).unwrap_or(12.0),
                props.as_ref().and_then(|p| p.color).unwrap_or([0.0; 3]),
                props.as_ref().and_then(|p| p.width).unwrap_or(1.0),
            )
        }
        ComposerKind::TextBox { size } => {
            let font_size = prefs.font_size(CommentTool::TextBox).unwrap_or(12.0);
            (Target::New { at: c.at, size }, Vec::new(), font_size, style.color, style.width)
        }
        _ => return,
    };
    let styles = styles_of(&text, &runs);
    let end = text.chars().count();
    let mut open = RichBox {
        page: c.page,
        target,
        before: (text.clone(), runs),
        text,
        styles,
        typing: (false, false),
        selection: TextSelection::caret_at(end),
        size,
        color,
        border,
        focus: true,
        in_bar: None,
        wanted: None,
        commit: false,
    };
    open.follow_caret();
    session.comments.rich = Some(open);
    // A selected box would keep its frame and resize handles over the text being typed.
    session.comments.clear_selection();
}

/// The box's text laid out in the screen points of the pass, the first line's top-left at `origin`.
struct Host<'a> {
    open: &'a mut RichBox,
    origin: Pos2,
    zoom: f64,
    /// The width lines wrap at, in points of the page.
    wrap: f64,
}

impl Host<'_> {
    fn line_ranges(&self) -> Vec<Range<usize>> {
        rich_line_ranges(&self.open.text, &runs_of(&self.open.styles), self.open.size, self.wrap)
    }

    fn line_height(&self) -> f32 {
        (self.open.size * LINE * self.zoom) as f32
    }

    /// Each character's start along its line on screen, and the line's end: one more entry than characters.
    fn columns(&self, chars: Range<usize>) -> Vec<f32> {
        let mut at = 0.0;
        let mut out = vec![0.0];
        for (ch, &style) in self.open.text.chars().zip(&self.open.styles).skip(chars.start).take(chars.len()) {
            at += advance(ch, style, self.open.size) * self.zoom;
            out.push(at as f32);
        }
        out
    }

    fn slice(&self, chars: Range<usize>) -> String {
        self.open.text.chars().skip(chars.start).take(chars.len()).collect()
    }

    fn line_of(&self, offset: usize) -> usize {
        let ranges = self.line_ranges();
        ranges.iter().position(|r| offset <= r.end).unwrap_or(ranges.len().saturating_sub(1))
    }

    fn range(&self, line: usize) -> Range<usize> {
        let ranges = self.line_ranges();
        ranges.get(line.min(ranges.len().saturating_sub(1))).cloned().unwrap_or(0..0)
    }

    /// The offset on line `line` nearest `x` (screen points from the origin).
    fn nearest(&self, line: usize, x: f32) -> usize {
        let range = self.range(line);
        let columns = self.columns(range.clone());
        let at = columns.iter().enumerate().min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs())).map_or(0, |(i, _)| i);
        range.start + at
    }

    fn x_of(&self, offset: usize, line: usize) -> f32 {
        let range = self.range(line);
        self.columns(range.clone()).get(offset.saturating_sub(range.start)).copied().unwrap_or(0.0)
    }

    fn point(&self, x: f32, line: usize) -> Point {
        Point::new(f64::from(self.origin.x + x), f64::from(self.origin.y + line as f32 * self.line_height()))
    }

    /// The pieces of `range`, one for each line it touches: (line, from x, to x).
    fn pieces(&self, range: Range<usize>) -> Vec<(usize, f32, f32)> {
        let mut out = Vec::new();
        for (line, r) in self.line_ranges().into_iter().enumerate() {
            let (from, to) = (range.start.max(r.start), range.end.min(r.end));
            if from <= to && (from < to || r.is_empty() && range.start <= r.start && range.end > r.start) {
                out.push((line, self.x_of(from, line), self.x_of(to, line)));
            }
        }
        out
    }

    /// Draws the text a word at a time, each word starting and ending where the saved box has it.
    fn paint(&self, painter: &egui::Painter) {
        let size = (self.open.size * self.zoom) as f32;
        let ink = color32(self.open.color);
        // The interface face is wider than Helvetica: one size for each of regular and bold makes the whole text fit,
        // so that words of a face don't differ in height.
        let fit = [false, true].map(|bold| {
            let (mut shown, mut saved) = (0.0, 0.0);
            for (ch, &style) in self.open.text.chars().zip(&self.open.styles).filter(|(ch, s)| s.0 == bold && !ch.is_control()) {
                let job = word_job(ch.encode_utf8(&mut [0; 4]), (bold, false), size, ink, 0.0);
                shown += painter.layout_job(job).size().x;
                saved += (advance(ch, style, self.open.size) * self.zoom) as f32;
            }
            if shown > saved && saved > 0.0 { size * (saved / shown) } else { size }
        });
        for (line, chars) in self.line_ranges().into_iter().enumerate() {
            let columns = self.columns(chars.clone());
            let text: Vec<char> = self.slice(chars.clone()).chars().collect();
            let styles = self.open.styles.get(chars).unwrap_or_default();
            for word in words(&text, styles) {
                let style = styles.get(word.start).copied().unwrap_or_default();
                let (Some(&from), Some(&to)) = (columns.get(word.start), columns.get(word.end)) else { continue };
                let text: String = text.get(word).unwrap_or_default().iter().collect();
                let size = if style.0 { fit[1] } else { fit[0] };
                let galley = fitted(painter, &text, style, size, ink, to - from);
                // On the line's baseline, whatever size the word was fitted at.
                let top = line as f32 * self.line_height() + ((self.open.size * self.zoom) as f32 - size) * 0.8;
                painter.galley(self.origin + vec2(from, top), galley, ink);
            }
        }
    }
}

/// The stretches of a line drawn at once: runs of characters of one style, spaces left out, as ranges of its characters.
fn words(text: &[char], styles: &[Style]) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    for (at, (ch, style)) in text.iter().zip(styles).enumerate() {
        if ch.is_whitespace() {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.end == at && styles.get(last.start) == Some(style) => last.end = at + 1,
            _ => out.push(at..at + 1),
        }
    }
    out
}

fn word_job(text: &str, (bold, italic): Style, size: f32, ink: Color32, spacing: f32) -> egui::text::LayoutJob {
    let family = if bold { FontFamily::Name("semibold".into()) } else { FontFamily::Proportional };
    let format =
        egui::TextFormat { font_id: FontId::new(size, family), color: ink, italics: italic, extra_letter_spacing: spacing, ..Default::default() };
    let mut job = egui::text::LayoutJob::default();
    job.append(text, 0.0, format);
    job
}

/// `text` laid out `width` wide: its own letter shapes and kerning, the difference shared out between the letters.
fn fitted(painter: &egui::Painter, text: &str, style: Style, size: f32, ink: Color32, width: f32) -> std::sync::Arc<egui::Galley> {
    let natural = painter.layout_job(word_job(text, style, size, ink, 0.0));
    let gaps = text.chars().count().saturating_sub(1);
    if gaps == 0 {
        return natural;
    }
    painter.layout_job(word_job(text, style, size, ink, (width - natural.size().x) / gaps as f32))
}

impl TextEditorHost for Host<'_> {
    fn text(&self) -> &str {
        &self.open.text
    }

    fn replace(&mut self, range: Range<usize>, text: &str, _kind: EditKind) -> Option<usize> {
        let count = self.open.styles.len();
        let start = range.start.min(count);
        let end = range.end.clamp(start, count);
        // Text typed over a selection takes the style the selection began with.
        let style = if start < end { self.open.styles.get(start).copied().unwrap_or(self.open.typing) } else { self.open.typing };
        let byte = |at: usize| self.open.text.char_indices().nth(at).map_or(self.open.text.len(), |(b, _)| b);
        let (a, b) = (byte(start), byte(end));
        let added = text.chars().count();
        self.open.text.replace_range(a..b, text);
        self.open.styles.splice(start..end, std::iter::repeat_n(style, added));
        self.open.typing = style;
        Some(added)
    }

    fn hit(&mut self, at: Point) -> (usize, usize) {
        let last = self.line_ranges().len().saturating_sub(1);
        let row = ((at.y as f32 - self.origin.y) / self.line_height()).floor();
        let line = if row.is_finite() && row > 0.0 { (row as usize).min(last) } else { 0 };
        (self.nearest(line, at.x as f32 - self.origin.x), line)
    }

    fn caret(&mut self, offset: usize, line: Option<usize>) -> Option<CaretLine> {
        let line = line.unwrap_or_else(|| self.line_of(offset));
        let top = self.point(self.x_of(offset, line), line);
        Some(CaretLine::new(top, Point::new(top.x, top.y + f64::from(self.line_height()))))
    }

    fn selection_quads(&mut self, range: Range<usize>) -> Vec<[Point; 4]> {
        let h = f64::from(self.line_height());
        self.pieces(range)
            .into_iter()
            .map(|(line, a, b)| {
                let (tl, tr) = (self.point(a, line), self.point(b, line));
                [tl, tr, Point::new(tr.x, tr.y + h), Point::new(tl.x, tl.y + h)]
            })
            .collect()
    }

    fn underline_segments(&mut self, range: Range<usize>) -> Vec<[Point; 2]> {
        let h = f64::from(self.line_height());
        self.pieces(range)
            .into_iter()
            .map(|(line, a, b)| {
                let (l, r) = (self.point(a, line), self.point(b, line));
                [Point::new(l.x, l.y + h), Point::new(r.x, r.y + h)]
            })
            .collect()
    }

    fn lines(&mut self) -> Vec<HostLine> {
        let (h, size) = (self.line_height(), (self.open.size * self.zoom) as f32);
        self.line_ranges()
            .into_iter()
            .enumerate()
            .map(|(i, chars)| {
                let columns = self.columns(chars.clone());
                let widths: Vec<f32> = columns.windows(2).map(|w| w[1] - w[0]).collect();
                let positions: Vec<f32> = columns.iter().take(widths.len()).copied().collect();
                let text = self.slice(chars.clone());
                let width = columns.last().copied().unwrap_or(0.0);
                let top = self.origin.y + i as f32 * h;
                let rect = Rect::from_min_size(Pos2::new(self.origin.x, top), vec2(width, h));
                let runs = vec![HostRun::new(0..text.len(), false, positions, widths)];
                HostLine::new(text, chars, rect, top + size, runs)
            })
            .collect()
    }

    fn line_range(&mut self, line: usize) -> Range<usize> {
        self.range(line)
    }

    fn line_step(&mut self, offset: usize, lines: i32) -> usize {
        let from = self.line_of(offset);
        let last = self.line_ranges().len().saturating_sub(1) as i64;
        let to = (from as i64 + i64::from(lines)).clamp(0, last) as usize;
        let x = self.x_of(offset, from);
        self.nearest(to, x)
    }

    fn line_edge(&mut self, offset: usize, end: bool) -> usize {
        let range = self.range(self.line_of(offset));
        if end { range.end } else { range.start }
    }

    fn style_key(&mut self, key: StyleKey, selection: TextSelection) -> bool {
        let italic = match key {
            StyleKey::Toggle(CharToggle::FauxBold) => false,
            StyleKey::Toggle(CharToggle::FauxItalic) => true,
            _ => return false,
        };
        self.open.selection = selection;
        self.open.toggle(italic);
        true
    }
}

fn say(ctx: &egui::Context, italic: bool, on: bool) {
    let words = match (italic, on) {
        (false, true) => tl!("Bold on"),
        (false, false) => tl!("Bold off"),
        (true, true) => tl!("Italic on"),
        (true, false) => tl!("Italic off"),
    };
    announce(ctx, words.to_string(), Politeness::Polite);
}

/// The keys a box being typed into takes from the frame's input before the app's shortcuts read it: Ctrl+B and Ctrl+I
/// (Command on macOS) to bold and italicise, and, in the bar, Tab and Shift+Tab to walk its buttons, Escape to go back
/// to the text and Cmd+Enter to commit.
pub(crate) fn take_keys(app: &mut crate::PdfCraftApp, ctx: &egui::Context, raw: &mut egui::RawInput) {
    let Some(open) = app.active.and_then(|i| app.views.get_mut(i)).and_then(|view| view.session.comments.rich.as_mut()) else { return };
    raw.events.retain(|event| {
        let egui::Event::Key { key, pressed: true, modifiers, .. } = event else { return true };
        if let Some(button) = open.in_bar
            && open.bar_key(button, *key, *modifiers)
        {
            return false;
        }
        if !modifiers.command || modifiers.shift || modifiers.alt {
            return true;
        }
        let italic = match key {
            egui::Key::B => false,
            egui::Key::I => true,
            _ => return true,
        };
        let on = open.toggle(italic);
        say(ctx, italic, on);
        false
    });
}

/// What the editor reads of the document and the view in a pass.
pub(crate) struct BoxCx<'a> {
    pub info: &'a DocInfo,
    pub doc: &'a pdfcraft_engine::Document,
    pub view_rotation: u16,
    pub prefs: &'a CommentPrefs,
}

/// The page body's part: the box being typed into, its editor and its Bold, Italic, Post and Cancel bar. Returns the
/// edit once it is committed (Post or Save, Cmd+Enter, or a press elsewhere on the page); Escape in the text or Cancel
/// drops it.
pub(crate) fn show(ui: &mut egui::Ui, body: &PageBody<'_>, session: &mut PageSession, cx: &BoxCx<'_>) -> Option<Edit> {
    let &BoxCx { info, prefs, .. } = cx;
    adopt(session, cx);
    let open = session.comments.rich.as_mut()?;
    // Known again once the bar is drawn: a box scrolled out of view takes no keys.
    open.in_bar = None;
    let placed = body.pages.iter().find(|p| p.page == open.page)?;
    let page = info.pages.get(open.page)?;
    let zoom = placed.scale;
    let rect_user = open.rect(info);
    let corner = {
        let v = page.user_to_view(rect_user[0] as f32, rect_user[3] as f32);
        placed.rect.lerp_inside(vec2(v[0] / page.width.max(f32::EPSILON), v[1] / page.height.max(f32::EPSILON)))
    };
    let pad = 2.0 + open.border;
    let width = (rect_user[2] - rect_user[0]).max(1.0);
    let (border, ink) = (open.border, color32(open.color));
    let fresh = std::mem::take(&mut open.focus);
    let ctx = ui.ctx().clone();

    let origin = corner + vec2((pad * zoom) as f32, (pad * zoom) as f32);
    let mut host = Host { open, origin, zoom, wrap: (width - 2.0 * pad).max(1.0) };
    let lines = host.line_ranges().len();
    let height = (((lines as f64) * host.open.size * LINE + 2.0 * pad + 2.0) * zoom) as f32;
    let rect = Rect::from_min_size(corner, vec2((width * zoom) as f32, height));
    let clip = ui.clip_rect();
    ui.set_clip_rect(clip.intersect(body.rect));
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, Color32::WHITE);
    if border > 0.0 {
        painter.rect_stroke(rect, 0.0, Stroke::new((border * zoom) as f32, ink), egui::StrokeKind::Inside);
    }
    host.paint(painter);
    // Where the typing happens, for a box with no border of its own.
    painter.rect_stroke(rect.expand(2.0), 0.0, Stroke::new(1.0, ui.visuals().selection.bg_fill), egui::StrokeKind::Outside);
    if host.open.text.is_empty() {
        painter.text(origin, egui::Align2::LEFT_TOP, tl!("Type text"), FontId::proportional((host.open.size * zoom) as f32), Color32::from_gray(150));
    }
    let mut selection = host.open.selection;
    let editor = TextEditor::new(tl!("Type text"), &mut host, Bind::bound(&mut selection)).esc_commits(false).mode(EditorMode::FieldLines);
    let out = if fresh { editor.request_focus(FocusRequest::Take) } else { editor }.show_at(ui, rect);
    let (changed, moved) = (out.changed, out.selection_changed);
    let ended = out.ended.filter(|end| !(fresh && *end == EditorEnd::Commit));
    ui.set_clip_rect(clip);
    let open = session.comments.rich.as_mut()?;
    open.selection = selection;
    if moved && !changed {
        open.follow_caret();
    }

    // The bar under the box.
    let anchor: Vec<Point> = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()].map(|p| body.view.to_doc(p)).to_vec();
    let (mut bold, mut italic) = open.shown_style();
    let (was_bold, was_italic) = (bold, italic);
    let (mut post, mut cancel) = (std::mem::take(&mut open.commit), false);
    // Tab out of the text goes to the bar's first button, Shift+Tab to its last.
    let wanted = match ended {
        Some(EditorEnd::Next) => Some(0),
        Some(EditorEnd::Previous) => Some(BAR - 1),
        _ => open.wanted.take(),
    };
    let mut in_bar = None;
    // The composer card's words: a new box is posted, an edit saved.
    let post_word = if open.is_new() { tl!("Post") } else { tl!("Save") };
    let postable = open.postable();
    let bar = TaskBar::fixed(tl!("Text box"), &anchor).show(ui, body.rect, |ui, _| {
        let mut held = |place: usize, focused: bool| {
            if focused {
                in_bar = Some(place);
            }
        };
        let mut bold = ToggleButton::new(tl!("Bold"), Bind::bound(&mut bold));
        let mut italic = ToggleButton::new(tl!("Italic"), Bind::bound(&mut italic));
        let mut answer = Button::new(post_word).enabled(postable);
        let mut leave = Button::new(tl!("Cancel"));
        match wanted {
            Some(0) => bold = bold.request_focus(FocusRequest::Take),
            Some(1) => italic = italic.request_focus(FocusRequest::Take),
            Some(2) => answer = answer.request_focus(FocusRequest::Take),
            Some(_) => leave = leave.request_focus(FocusRequest::Take),
            None => {}
        }
        held(0, bold.show(ui).response.has_focus());
        held(1, italic.show(ui).response.has_focus());
        let answer = answer.show(ui);
        post |= answer.clicked();
        held(2, answer.response.has_focus());
        let leave = leave.show(ui);
        cancel = leave.clicked();
        held(3, leave.response.has_focus());
    });
    open.in_bar = in_bar;
    // A press on the bar takes the focus from the text, which ends the editor's session but not the box.
    let on_bar = ui.input(|i| i.pointer.primary_pressed() && i.pointer.interact_pos().is_some_and(|at| bar.response.rect.contains(at)));
    for (slot, changed_to, was) in [(false, bold, was_bold), (true, italic, was_italic)] {
        if changed_to != was {
            let on = open.toggle(slot);
            say(&ctx, slot, on);
            // Back to typing.
            open.focus = true;
        }
    }

    if cancel || ended == Some(EditorEnd::Cancel) {
        session.comments.rich = None;
        return None;
    }
    if post || ended == Some(EditorEnd::Commit) && !on_bar {
        let open = session.comments.rich.take()?;
        session.comments.tool_done = true;
        return open.edit(info, prefs);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A word of the preview takes the width the saved box gives it, wider or narrower than the interface face's own.
    #[test]
    fn a_preview_word_is_fitted_to_its_saved_width() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for factor in [0.9, 1.1] {
                let natural = fitted(ui.painter(), "R", (false, false), 20.0, Color32::BLACK, 0.0).size().x;
                let wanted = fitted(ui.painter(), "Really.", (false, false), 20.0, Color32::BLACK, natural * 5.0 * factor);
                assert!((wanted.size().x - natural * 5.0 * factor).abs() < 1.5, "{} {}", wanted.size().x, natural * 5.0 * factor);
            }
        });
        output.textures_delta.clear();
    }
}
