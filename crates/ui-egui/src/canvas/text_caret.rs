//! Caret browsing (View ▸ Caret browsing, F7): a text caret on the pages, moved by keys, so part of a page's text can be
//! selected without a pointer. Shift extends the same selection a drag makes, so Copy and the markup tools act on it.
//!
//! The Select tool claims the keys while the caret is on ([`CARET_KEYS`] and the arrows) and queues them here; they are
//! applied after the page view's pass, from the pages' text layers.

use craft::keymap::{Chord, Key, Mods};
use craft::pages::ui::{TextSelection, TextSpan as FwSpan};
use craft::ui::contract::Politeness;
use craft::ui::runtime::access::announce;
use egui::Rect;
use pdfcraft_render::{DocInfo, PageText};

use super::DocView;
use super::selection::NO_TEXT;

const CMD_SHIFT: Mods = Mods { shift: true, ..Mods::CMD };

/// The chords the caret takes from the app's shortcuts while it is on. Plain arrows reach the tool anyway.
pub(crate) const CARET_KEYS: &[Chord] = &[
    Chord::plain(Key::Home),
    Chord::plain(Key::End),
    Chord::new(Mods::SHIFT, Key::Home),
    Chord::new(Mods::SHIFT, Key::End),
    Chord::new(Mods::SHIFT, Key::Left),
    Chord::new(Mods::SHIFT, Key::Right),
    Chord::new(Mods::SHIFT, Key::Up),
    Chord::new(Mods::SHIFT, Key::Down),
    Chord::new(Mods::CMD, Key::Left),
    Chord::new(Mods::CMD, Key::Right),
    Chord::new(CMD_SHIFT, Key::Left),
    Chord::new(CMD_SHIFT, Key::Right),
];

/// Where a key takes the caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
}

/// One queued key: where to, and whether it extends the selection.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Move {
    pub(crate) step: Step,
    pub(crate) extend: bool,
}

/// A place between glyphs: page `page`, before glyph `at` (`at` may be the page's glyph count).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Place {
    pub(crate) page: usize,
    pub(crate) at: usize,
}

/// The caret of one document tab while caret browsing is on.
#[derive(Debug)]
pub(crate) struct TextCaret {
    /// Where the caret is.
    pub(crate) head: Place,
    /// Where the selection started; the head's place when nothing is selected.
    anchor: Place,
    /// Shown after the glyph before it, at its line's end, rather than before the next line's first glyph.
    line_end: bool,
    /// The column Up and Down keep to.
    goal_x: Option<f32>,
    /// Keys taken in this pass, applied after it.
    pub(crate) moves: Vec<Move>,
    /// Passes left in which the page view takes the keyboard (the palette or menu gives its own focus back first).
    pub(crate) take_focus: u8,
}

/// The text layers the caret moves over: `None` for a page not read yet.
pub(crate) trait Layers {
    fn layer(&self, page: usize) -> Option<&PageText>;
    fn pages(&self) -> usize;
}

/// The nearest page from `page` (itself excluded) with text, forwards or backwards; `None` at the document's end or at a
/// page still to be read.
fn neighbour(layers: &dyn Layers, page: usize, forward: bool) -> Option<(usize, &PageText)> {
    let mut page = page;
    loop {
        page = if forward { page.checked_add(1).filter(|p| *p < layers.pages())? } else { page.checked_sub(1)? };
        let text = layers.layer(page)?;
        if !text.glyphs.is_empty() {
            return Some((page, text));
        }
    }
}

/// Whether a line ends before glyph `at`.
fn ends_line(text: &PageText, at: usize) -> bool {
    at > 0 && (at >= text.glyphs.len() || text.line_of.get(at - 1) != text.line_of.get(at))
}

/// Whether a word starts at glyph `at`.
fn starts_word(text: &PageText, at: usize) -> bool {
    let blank = |k: usize| text.glyphs.get(k).is_none_or(|g| g.text.trim().is_empty());
    !blank(at) && (at == 0 || blank(at - 1) || ends_line(text, at) || text.space_before.get(at).copied().unwrap_or(false))
}

/// The horizontal position of the place `at`, on the line ending there when `line_end`.
fn x_of(text: &PageText, at: usize, line_end: bool) -> Option<f32> {
    if line_end || at >= text.glyphs.len() { text.glyphs.get(at.checked_sub(1)?).map(|g| g.rect[2]) } else { text.glyphs.get(at).map(|g| g.rect[0]) }
}

/// The place on the line of glyph `glyph` closest to `x`.
fn closest_on_line(text: &PageText, glyph: usize, x: f32) -> Option<(usize, bool)> {
    let (first, last) = text.line_at(glyph)?;
    (first..=last + 1)
        .map(|at| (at, at == last + 1))
        .filter_map(|(at, end)| x_of(text, at, end).map(|found| ((found - x).abs(), at, end)))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, at, end)| (at, end))
}

impl TextCaret {
    /// The caret at the start of `page`'s text.
    pub(crate) fn at_page(page: usize) -> Self {
        let head = Place { page, at: 0 };
        Self { head, anchor: head, line_end: false, goal_x: None, moves: Vec::new(), take_focus: 3 }
    }

    /// The selection from the anchor to the caret, `None` when it covers no glyph.
    pub(crate) fn selection(&self, layers: &dyn Layers) -> Option<TextSelection> {
        let (low, high) = (self.anchor.min(self.head), self.anchor.max(self.head));
        if low == high {
            return None;
        }
        // The first glyph covered and the last, as (page, glyph).
        let start = match layers.layer(low.page) {
            Some(text) if low.at >= text.glyphs.len() => (neighbour(layers, low.page, true)?.0, 0),
            _ => (low.page, low.at),
        };
        let end = match high.at.checked_sub(1) {
            Some(glyph) => (high.page, glyph),
            None => neighbour(layers, high.page, false).map(|(page, text)| (page, text.glyphs.len() - 1))?,
        };
        if start > end {
            return None;
        }
        let (from, to) = if self.head < self.anchor { (end, start) } else { (start, end) };
        Some(TextSelection::new(from.0, FwSpan::run(from.1, to.0, to.1)))
    }

    /// Takes over a selection made some other way (a drag, Select All), or forgets the anchor of one that is gone.
    pub(crate) fn follow(&mut self, selection: Option<TextSelection>, layers: &dyn Layers) {
        if selection == self.selection(layers) {
            return;
        }
        match selection.map(|s| (s.page, s.span)) {
            Some((page, FwSpan::Run { anchor, head, head_page, .. })) => {
                let (a, h) = ((page, anchor), (head_page, head));
                let forward = a <= h;
                self.anchor = Place { page: a.0, at: a.1 + usize::from(!forward) };
                self.head = Place { page: h.0, at: h.1 + usize::from(forward) };
                self.line_end = forward && layers.layer(self.head.page).is_some_and(|text| ends_line(text, self.head.at));
            }
            _ => self.anchor = self.head,
        }
    }

    /// Puts the caret at the start of `page`, with nothing selected: the page it was on is no longer shown.
    pub(crate) fn restart(&mut self, page: usize) {
        self.head = Place { page, at: 0 };
        self.anchor = self.head;
        self.line_end = false;
        self.goal_x = None;
    }

    /// Moves the caret by one key. A page still to be read holds it where it is.
    pub(crate) fn step(&mut self, step: Step, extend: bool, layers: &dyn Layers) {
        let Some(text) = layers.layer(self.head.page) else { return };
        let len = text.glyphs.len();
        let at = self.head.at.min(len);
        // The glyph whose line the caret is on.
        let on = if self.line_end || at >= len { at.saturating_sub(1) } else { at };
        let page = self.head.page;
        // Left or Right without Shift goes to that end of a selection, as in any text.
        if !extend && matches!(step, Step::Left | Step::Right) && self.anchor != self.head {
            let to = if step == Step::Left { self.anchor.min(self.head) } else { self.anchor.max(self.head) };
            if to != self.head {
                self.line_end = step == Step::Right && layers.layer(to.page).is_some_and(|other| ends_line(other, to.at));
                self.head = to;
            } else if step == Step::Left {
                self.line_end = false;
            }
            self.anchor = self.head;
            self.goal_x = None;
            return;
        }
        let mut goal = None;
        let next: Option<(usize, usize, bool)> = match step {
            Step::Right if self.line_end && at < len => Some((page, at, false)),
            Step::Right if at < len => Some((page, at + 1, ends_line(text, at + 1))),
            Step::Right => neighbour(layers, page, true).map(|(p, _)| (p, 0, false)),
            Step::Left if !self.line_end && at < len && ends_line(text, at) => Some((page, at, true)),
            Step::Left if at > 0 => Some((page, at - 1, false)),
            Step::Left => neighbour(layers, page, false).map(|(p, t)| (p, t.glyphs.len(), true)),
            Step::WordRight => match (at + 1..len).find(|&k| starts_word(text, k)) {
                Some(k) => Some((page, k, false)),
                None if at < len => Some((page, len, true)),
                None => neighbour(layers, page, true).map(|(p, _)| (p, 0, false)),
            },
            Step::WordLeft => match (0..at).rev().find(|&k| starts_word(text, k)) {
                Some(k) => Some((page, k, false)),
                None => {
                    neighbour(layers, page, false).and_then(|(p, t)| (0..t.glyphs.len()).rev().find(|&k| starts_word(t, k)).map(|k| (p, k, false)))
                }
            },
            Step::LineStart => text.line_at(on).map(|(first, _)| (page, first, false)),
            Step::LineEnd => text.line_at(on).map(|(_, last)| (page, last + 1, true)),
            Step::Up | Step::Down => {
                let x = self.goal_x.or_else(|| x_of(text, at, self.line_end)).unwrap_or_default();
                goal = Some(x);
                let down = step == Step::Down;
                text.line_at(on).and_then(|(first, last)| {
                    let near = if down { Some(last + 1).filter(|k| *k < len) } else { first.checked_sub(1) };
                    match near {
                        Some(glyph) => closest_on_line(text, glyph, x).map(|(k, end)| (page, k, end)),
                        None => {
                            let (p, other) = neighbour(layers, page, down)?;
                            let glyph = if down { 0 } else { other.glyphs.len() - 1 };
                            closest_on_line(other, glyph, x).map(|(k, end)| (p, k, end))
                        }
                    }
                })
            }
        };
        self.goal_x = goal;
        if let Some((page, at, line_end)) = next {
            self.head = Place { page, at };
            self.line_end = line_end;
        }
        if !extend {
            self.anchor = self.head;
        }
    }

    /// The caret as a line in its page's view space, `[x, top, x, bottom]`.
    pub(crate) fn line(&self, text: &PageText) -> Option<[f32; 4]> {
        let at = self.head.at.min(text.glyphs.len());
        let beside = if self.line_end || at >= text.glyphs.len() { at.checked_sub(1)? } else { at };
        let rect = text.glyphs.get(beside)?.rect;
        let x = x_of(text, at, self.line_end)?;
        Some([x, rect[1], x, rect[3]])
    }
}

/// The most of a selection that is read out.
const ANNOUNCED: usize = 200;

impl Layers for DocView {
    fn layer(&self, page: usize) -> Option<&PageText> {
        // A page that could not be read has nothing to stand on: the caret passes over it.
        self.session.texts.get(&page).map(|t| &**t).or_else(|| self.session.text_failed.contains(&page).then_some(&NO_TEXT))
    }

    fn pages(&self) -> usize {
        self.page_count
    }
}

impl DocView {
    /// Whether the tab has its text caret on (View > Caret browsing).
    pub(crate) fn caret_browsing(&self) -> bool {
        self.session.caret.is_some()
    }

    /// Puts a caret at the start of the current page's text, or takes it away; the selection stays.
    pub(crate) fn set_caret_browsing(&mut self, on: bool) {
        self.session.caret = on.then(|| TextCaret::at_page(self.view.current));
    }

    /// Where the text caret is drawn: `None` while caret browsing is off, its page is off screen or has no text.
    pub fn caret_screen_rect(&self) -> Option<Rect> {
        let caret = self.session.caret.as_ref()?;
        let page = caret.head.page;
        let rect = self.page_xform(page)?.view_rect(caret.line(self.layer(page)?)?);
        // The line has no width of its own, or no height in a view turned a quarter.
        let thin = |side: f32| if side < 1.0 { 0.75 } else { 0.0 };
        Some(rect.expand2(egui::vec2(thin(rect.width()), thin(rect.height()))))
    }
}

/// Applies the keys the pass queued for the caret: the selection follows it, the view scrolls to it and a selection is
/// read out.
pub(super) fn move_caret(view: &mut DocView, info: &DocInfo, viewport: Rect, ctx: &egui::Context) {
    let Some(mut caret) = view.session.caret.take() else { return };
    let moves = std::mem::take(&mut caret.moves);
    if moves.is_empty() {
        view.session.caret = Some(caret);
        return;
    }
    let before = view.session.text.selection;
    caret.follow(before, view);
    // The reader has gone to other pages: the keys move the caret where they are looking.
    if !view.session.placed.iter().any(|p| p.page == caret.head.page) {
        caret.restart(view.view.current);
    }
    for one in moves {
        caret.step(one.step, one.extend, view);
    }
    let selection = caret.selection(view);
    let line = view.layer(caret.head.page).and_then(|text| caret.line(text));
    let page = caret.head.page;
    view.session.text.selection = selection;
    view.session.all_text = false;
    view.session.caret = Some(caret);
    if selection != before {
        let words = selection.and_then(|s| s.text(&view.doc_texts()));
        let said = match words {
            Some(words) => {
                let mut short: String = words.chars().take(ANNOUNCED).collect();
                if short.len() < words.len() {
                    short.push('\u{2026}');
                }
                Some(crate::i18n::fmt(crate::i18n::t("Selected: {text}"), &[("text", &short)]))
            }
            None => before.map(|_| crate::i18n::t("Selection cleared").to_owned()),
        };
        if let Some(said) = said {
            announce(ctx, said, Politeness::Polite);
        }
    }
    if let Some(line) = line
        && !view.caret_screen_rect().is_some_and(|shown| viewport.contains_rect(shown))
    {
        view.reveal_rect(page, line, info);
    }
    ctx.request_repaint();
}
