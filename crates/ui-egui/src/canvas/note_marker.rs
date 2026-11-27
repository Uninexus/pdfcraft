//! Placing a sticky note or a text box by keys: with that tool active and the page view focused, a marker stands on the
//! page. The arrows move it (further with Shift), Enter opens the note or box there as a click would, and Escape leaves
//! the tool.
//!
//! The comment tool claims the keys while the marker shows ([`MARKER_KEYS`], the arrows and Enter) and queues them here;
//! they are applied after the page view's pass.

use craft::keymap::{Chord, Key, Mods};
use craft::ui::contract::Politeness;
use craft::ui::runtime::access::announce;
use egui::{Pos2, Rect, Vec2};
use pdfcraft_render::DocInfo;

use super::DocView;
use super::text_caret::Layers as _;
use crate::QuickTool;
use crate::comments::{CommentTool, Composer, ComposerKind};

/// The chords the marker takes from the app's shortcuts. Plain arrows and Enter reach the tool anyway.
pub(crate) const MARKER_KEYS: &[Chord] =
    &[Chord::new(Mods::SHIFT, Key::Left), Chord::new(Mods::SHIFT, Key::Right), Chord::new(Mods::SHIFT, Key::Up), Chord::new(Mods::SHIFT, Key::Down)];

/// How far an arrow moves the marker on screen, in points, without and with Shift.
const STEP: f32 = 8.0;
const FAR: f32 = 48.0;

/// What the marker places.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Placed {
    Note,
    TextBox,
}

impl Placed {
    /// What `tool` places by marker: `None` for a tool that has no marker.
    pub(crate) fn by(tool: QuickTool) -> Option<Self> {
        match tool {
            QuickTool::Comment(CommentTool::Note) => Some(Self::Note),
            QuickTool::Comment(CommentTool::TextBox) => Some(Self::TextBox),
            _ => None,
        }
    }

    fn composer(self) -> ComposerKind {
        match self {
            Self::Note => ComposerKind::Note,
            Self::TextBox => ComposerKind::TextBox { size: None },
        }
    }
}

/// One queued key.
#[derive(Clone, Copy, Debug)]
pub(crate) enum MarkerKey {
    /// An arrow: its direction on screen, and whether Shift asks for the longer step.
    Move {
        direction: Vec2,
        far: bool,
    },
    Place,
    Cancel,
}

/// The marker of one document tab.
#[derive(Debug)]
pub(crate) struct NoteMarker {
    placed: Placed,
    page: usize,
    /// Where the note's or box's corner would go, in page view space.
    at: [f32; 2],
    /// Keys taken in this pass, applied after it.
    pub(crate) keys: Vec<MarkerKey>,
}

impl DocView {
    /// Where the placing marker is drawn: `None` without one.
    pub fn note_marker_screen_pos(&self) -> Option<Pos2> {
        let marker = self.session.note_marker.as_ref()?;
        self.view_point_on_screen(marker.page, marker.at)
    }

    fn view_point_on_screen(&self, page: usize, at: [f32; 2]) -> Option<Pos2> {
        Some(self.page_xform(page)?.view_rect([at[0], at[1], at[0], at[1]]).min)
    }

    /// The page view point under `screen`, kept on the page.
    fn point_on_page(&self, page: usize, screen: Pos2) -> Option<[f32; 2]> {
        let (x, y) = self.page_xform(page)?.screen_to_view(screen);
        let (width, height) = *self.page_sizes.get(page)?;
        Some([x.clamp(0.0, width.max(0.0)), y.clamp(0.0, height.max(0.0))])
    }

    /// A new marker: at the text caret when there is one on screen, else in the middle of what shows of the current page.
    fn new_marker(&self, viewport: Rect, placed: Placed) -> Option<NoteMarker> {
        let marker = |page, at| NoteMarker { placed, page, at, keys: Vec::new() };
        if let Some(caret) = &self.session.caret
            && let Some(line) = self.layer(caret.head.page).and_then(|text| caret.line(text))
            && self.view_point_on_screen(caret.head.page, [line[0], line[1]]).is_some_and(|at| viewport.contains(at))
        {
            return Some(marker(caret.head.page, [line[0], line[1]]));
        }
        let shows = |page: usize| {
            let seen = self.page_xform(page)?.rect.intersect(viewport);
            seen.is_positive().then(|| (page, seen.center()))
        };
        let (page, centre) = shows(self.view.current).or_else(|| self.session.placed.iter().find_map(|placed| shows(placed.page)))?;
        Some(marker(page, self.point_on_page(page, centre)?))
    }
}

/// Where the marker stands, as it is read out: the page, and which third of it down and across.
fn place_words(view: &DocView, marker: &NoteMarker) -> Option<String> {
    let (width, height) = *view.page_sizes.get(marker.page)?;
    let third = |value: f32, size: f32| usize::from(value > size / 3.0) + usize::from(value > size * 2.0 / 3.0);
    let rough = match (third(marker.at[1], height), third(marker.at[0], width)) {
        (0, 0) => crate::i18n::t("top left"),
        (0, 1) => crate::i18n::t("top"),
        (0, _) => crate::i18n::t("top right"),
        (1, 0) => crate::i18n::t("left"),
        (1, 1) => crate::i18n::t("middle"),
        (1, _) => crate::i18n::t("right"),
        (_, 0) => crate::i18n::t("bottom left"),
        (_, 1) => crate::i18n::t("bottom"),
        (_, _) => crate::i18n::t("bottom right"),
    };
    let page = (marker.page + 1).to_string();
    let words = match marker.placed {
        Placed::Note => crate::i18n::t("Note marker: page {page}, {place}"),
        Placed::TextBox => crate::i18n::t("Text box marker: page {page}, {place}"),
    };
    Some(crate::i18n::fmt(words, &[("page", &page), ("place", rough)]))
}

/// Keeps the marker while `active` says what to place (its tool, the page view focused, nothing being written): puts one
/// on the page, applies the keys the pass queued and reads out where it is. True when it was cancelled: the tool is left.
pub(super) fn update(view: &mut DocView, info: &DocInfo, viewport: Rect, ctx: &egui::Context, active: Option<Placed>) -> bool {
    let held = view.session.note_marker.take();
    let Some(placed) = active else { return false };
    // The reader has gone to other pages: the marker comes to where they are looking.
    let Some(mut marker) = held.filter(|m| m.placed == placed && view.session.placed.iter().any(|shown| shown.page == m.page)) else {
        view.session.note_marker = view.new_marker(viewport, placed);
        // The arrows and Escape are the marker's now: a comment left selected (one just placed) would take them.
        if view.session.note_marker.is_some() {
            view.session.comments.select(None);
        }
        if let Some(said) = view.session.note_marker.as_ref().and_then(|m| place_words(view, m)) {
            announce(ctx, said, Politeness::Polite);
            ctx.request_repaint();
        }
        return false;
    };
    let keys = std::mem::take(&mut marker.keys);
    let mut moved = false;
    for key in keys {
        match key {
            MarkerKey::Cancel => {
                announce(ctx, crate::i18n::t("Marker cancelled"), Politeness::Polite);
                ctx.request_repaint();
                return true;
            }
            MarkerKey::Move { direction, far } => {
                let step = if far { FAR } else { STEP };
                if let Some(at) = view
                    .view_point_on_screen(marker.page, marker.at)
                    .and_then(|screen| view.point_on_page(marker.page, screen + direction.normalized() * step))
                {
                    marker.at = at;
                    moved = true;
                }
            }
            MarkerKey::Place => {
                if let Some(page) = info.pages.get(marker.page) {
                    let [x, y] = page.view_to_user(marker.at[0], marker.at[1]);
                    view.session.comments.composer = Some(Composer {
                        page: marker.page,
                        at: [f64::from(x), f64::from(y)],
                        kind: marker.placed.composer(),
                        text: String::new(),
                        focus: true,
                    });
                    ctx.request_repaint();
                    return false;
                }
            }
        }
    }
    if moved {
        if let Some(said) = place_words(view, &marker) {
            announce(ctx, said, Politeness::Polite);
        }
        if view.view_point_on_screen(marker.page, marker.at).is_some_and(|at| !viewport.contains(at)) {
            view.reveal_rect(marker.page, [marker.at[0], marker.at[1], marker.at[0], marker.at[1]], info);
        }
        ctx.request_repaint();
    }
    view.session.note_marker = Some(marker);
    false
}
