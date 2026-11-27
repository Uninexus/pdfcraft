//! View > Show Rulers: a ruler along the top and left of the document area, in the measuring units.
//!
//! The strips are drawn and read outside the page view (it is given the room inside them), so a press on a ruler never
//! reaches a page. The top ruler counts from the left edge of each page column and the left ruler from the top edge of each
//! page, along the screen's axes (a view turned a quarter shows the page's height across). A drag from a ruler onto a page
//! makes a guide there through the session's `add_guide`, the function `doc_guide_add` calls; a guide dragged by its line
//! (`guides`) and dropped on a ruler is removed. Guides are view state: no undo step, and the document is not dirty.

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2};

use craft::behavior::{Metrics, Sense as Behaviour};
use craft::ui::contract::{Common, Component, Cx, Focus as _, MeasureCx, NoValue, Outcome, Pointer as _, Props as _, Role, Show as _};
use craft::ui::runtime::env::{Env, Label};
use craft::ui::style::{ComponentStyle, StyleCx};

use crate::PdfCraftApp;
use crate::units::Unit;

use super::guides;

/// The least distance between two numbered ticks, in screen points.
const MIN_MAJOR: f32 = 48.0;

/// The most ticks one ruler draws per page.
const MAX_TICKS: usize = 400;

/// Where the rulers and the pages lie in the document area.
#[derive(Clone, Copy, Debug)]
pub(super) struct Strips {
    /// The top ruler.
    pub top: Rect,
    /// The left ruler, under the corner.
    pub left: Rect,
    /// Where the two meet.
    pub corner: Rect,
    /// The room left for the pages.
    pub pages: Rect,
}

/// What a ruler is painted with: theme tokens only.
struct Look {
    strip: Color32,
    page: Color32,
    tick: Color32,
    text: Color32,
    guide: Color32,
    font: FontId,
    line: f32,
}

fn colour(rgba: craft::theme::Rgba) -> Color32 {
    let [r, g, b, a] = rgba.0;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

fn look(ui: &Ui) -> Look {
    let env = Env::of(ui);
    let tokens = &env.tokens;
    let colors = &tokens.color;
    Look {
        strip: colour(colors.surface.chrome),
        page: colour(colors.surface.sunken),
        tick: colour(colors.text.muted),
        text: colour(colors.text.muted),
        guide: colour(colors.status.info.solid),
        font: FontId::proportional(tokens.typography.caption.size),
        line: tokens.stroke.thin,
    }
}

/// How thick a ruler is: a caption's line and the tight gap above and below it.
fn thickness(ui: &Ui) -> f32 {
    let env = Env::of(ui);
    env.tokens.typography.caption.line_height + 2.0 * env.tokens.space.gap.tight
}

/// Whether the tab at `index` shows rulers: they are on and it shows a page body.
fn shows(app: &PdfCraftApp, index: usize) -> bool {
    app.views.get(index).is_some_and(|view| view.show_rulers && !view.organize && !view.presenting())
}

/// The room the active tab's top ruler takes, none when it shows no rulers: floating panels stay below it.
pub(crate) fn top_room(app: &PdfCraftApp, ui: &Ui) -> f32 {
    if app.active.is_some_and(|index| shows(app, index)) { thickness(ui) } else { 0.0 }
}

/// The strips of the tab at `index` in `rect`, or `None` when its rulers are off or it shows no page body (the
/// Organize grid, full screen). The top strip lies under the notice bars (`notice_inset`, as the last pass measured).
pub(super) fn layout(app: &PdfCraftApp, index: usize, ui: &Ui, rect: Rect) -> Option<Strips> {
    if !shows(app, index) {
        return None;
    }
    let t = thickness(ui);
    let top_edge = rect.top() + app.notice_inset.max(0.0);
    let pages = Rect::from_min_max(pos2(rect.left() + t, top_edge + t), rect.max);
    if !(pages.width() > 0.0 && pages.height() > 0.0) {
        return None;
    }
    Some(Strips {
        top: Rect::from_min_max(pos2(pages.left(), top_edge), pos2(pages.right(), pages.top())),
        left: Rect::from_min_max(pos2(rect.left(), pages.top()), pos2(pages.left(), pages.bottom())),
        corner: Rect::from_min_max(pos2(rect.left(), top_edge), pos2(pages.left(), pages.top())),
        pages,
    })
}

/// One tick of a ruler.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Tick {
    /// Where it lies along the ruler, in screen points.
    pub at: f32,
    /// Its value in the unit.
    pub value: f64,
    /// Whether it is numbered (the others are halfway between).
    pub major: bool,
}

/// The distance between numbered ticks, in the unit: 1, 2 or 5 times a power of ten, at least [`MIN_MAJOR`] on screen.
fn major_step(per_unit: f32) -> f64 {
    let least = f64::from(MIN_MAJOR) / f64::from(per_unit);
    let base = 10f64.powf(least.log10().floor());
    [1.0, 2.0, 5.0, 10.0].into_iter().map(|m| base * m).find(|step| *step >= least).unwrap_or(base * 10.0)
}

/// The ticks between screen positions `from` and `to` of a ruler whose zero is at `origin` and whose unit is `per_unit`
/// screen points long; none for a scale or range that is not usable.
pub(super) fn ticks(origin: f32, per_unit: f32, from: f32, to: f32) -> Vec<Tick> {
    let usable = [origin, per_unit, from, to].iter().all(|v| v.is_finite()) && per_unit > 0.0 && to > from;
    if !usable {
        return Vec::new();
    }
    let half = major_step(per_unit) / 2.0;
    let index = |at: f32| f64::from(at - origin) / f64::from(per_unit) / half;
    let (first, last) = (index(from).floor() as i64, index(to).ceil() as i64);
    (first..=last)
        .take(MAX_TICKS)
        .map(|k| {
            let value = k as f64 * half;
            Tick { at: origin + (value * f64::from(per_unit)) as f32, value, major: k % 2 == 0 }
        })
        .collect()
}

/// The number written at a numbered tick, with as many decimals as the step between them needs.
pub(super) fn label(value: f64, per_unit: f32) -> String {
    let step = major_step(per_unit);
    let decimals = (-step.log10().floor()).max(0.0) as usize;
    format!("{value:.decimals$}")
}

/// What the rulers' input left for the painting after the page view.
#[derive(Clone, Copy, Default)]
pub(super) struct RulerInput {
    /// The ruler that holds the keyboard focus.
    focused: Option<Rect>,
    /// A drag from a ruler under way: whether it is the top one, and where the pointer is.
    dragging: Option<(bool, Pos2)>,
}

/// The rulers as controls: a drag from one makes a guide, and so does Enter or Space on the one with the focus.
pub(super) fn input(app: &mut PdfCraftApp, index: usize, ui: &mut Ui, strips: &Strips) -> Option<RulerInput> {
    let key = app.views.get(index)?.view_key();
    let mut out = RulerInput::default();
    for from_top in [true, false] {
        let strip = if from_top { strips.top } else { strips.left };
        let name = if from_top { tl!("Horizontal ruler") } else { tl!("Vertical ruler") };
        let stop = RulerStop::new(name.to_string()).id(egui::Id::unique(("pdfcraft-ruler", key, from_top))).show_at(ui, strip);
        if stop.focused {
            out.focused = Some(strip);
        }
        if stop.activated {
            add_in_view(app, index, strips, from_top);
        }
        let resp = ui.interact(strip, egui::Id::unique(("pdfcraft-ruler-drag", key, from_top)), Sense::DRAG);
        let hint = tl!("Drag onto a page to add a guide, or press Enter to add one in the middle of the view");
        let cursor = if from_top { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::ResizeHorizontal };
        let resp = resp.on_hover_text(hint).on_hover_cursor(cursor);
        let pointer = ui.input(|i| i.pointer.latest_pos());
        if let Some(at) = pointer.filter(|_| resp.dragged()) {
            out.dragging = Some((from_top, at));
        }
        if resp.drag_stopped()
            && let Some(at) = pointer
        {
            drop_guide(app, index, strips, from_top, at);
        }
    }
    Some(out)
}

/// Paints the rulers of the tab at `index` after its pages, and applies what a guide's drag asked for.
pub(super) fn show(app: &mut PdfCraftApp, index: usize, ui: &mut Ui, strips: &Strips, input: Option<RulerInput>) {
    settle_guide_drag(app, index, ui, strips);
    let Some(view) = app.views.get(index) else { return };
    let pages = view.page_xforms();
    let current = view.view.current;
    let look = look(ui);
    paint(ui, strips, &pages, current, app.units, &look);
    app.left_rulers.push(strips.left);
    let input = input.unwrap_or_default();
    if let Some(strip) = input.focused {
        ui.painter().rect_stroke(strip, 0.0, Stroke::new(2.0 * look.line, look.guide), StrokeKind::Inside);
    }
    if let Some((from_top, at)) = input.dragging {
        preview(ui, strips, &pages, from_top, at, &look);
    }
}

/// Adds a guide through the middle of the part of the current page that is on screen (the first page on screen when the
/// current one is not), and says where it is.
fn add_in_view(app: &mut PdfCraftApp, index: usize, strips: &Strips, from_top: bool) {
    let Some(view) = app.views.get(index) else { return };
    let current = view.view.current;
    let pages = view.page_xforms();
    let on_screen = |xf: &super::xform::PageXform| xf.rect.intersect(strips.pages);
    let shown = pages.iter().filter(|(_, xf)| on_screen(xf).is_positive());
    let Some(at) = shown.clone().find(|(page, _)| *page == current).or(shown.clone().next()).map(|(_, xf)| on_screen(xf).center()) else {
        return;
    };
    if let Some(position) = drop_guide(app, index, strips, from_top, at) {
        let unit = app.units;
        let text = format!("{:.*} {}", unit.decimals(), position * unit.per_point(), unit.label());
        app.notify_fmt("Guide added at {position}", &[("position", &text)]);
    }
}

/// Applies the move a guide's drag asked for in the page body, and removes the guide when the button comes up over a ruler.
fn settle_guide_drag(app: &mut PdfCraftApp, index: usize, ui: &Ui, strips: &Strips) {
    let Some(view) = app.views.get_mut(index) else { return };
    let doc = view.id;
    let moved = view.guide_move.take();
    let dragging = view.guide_drag;
    let (released, down, at) = ui.input(|i| (i.pointer.primary_released(), i.pointer.primary_down(), i.pointer.latest_pos()));
    let on_ruler = at.is_some_and(|p| [strips.top, strips.left, strips.corner].iter().any(|r| r.contains(p)));
    if released || !down {
        view.guide_drag = None;
    }
    if let Some(moved) = moved
        && !(released && on_ruler)
    {
        // A position another guide holds is refused: the guide stays where it was.
        let _ = app.session.move_guide(doc, moved.id, moved.position);
    }
    if released
        && on_ruler
        && let Some(id) = dragging
    {
        let _ = app.session.remove_guide(doc, id);
    }
}

/// Makes the guide a drag from a ruler dropped at `at` stands for, when that is on a page; its position in points.
fn drop_guide(app: &mut PdfCraftApp, index: usize, strips: &Strips, from_top: bool, at: Pos2) -> Option<f64> {
    let view = app.views.get_mut(index)?;
    let doc = view.id;
    let (page, xf) = view.page_xforms().into_iter().find(|(_, xf)| xf.rect.contains(at))?;
    if !strips.pages.contains(at) {
        return None;
    }
    // A guide made while guides are hidden would be lost to sight at once.
    view.show_guides = true;
    let (axis, position) = guides::dropped(&xf, from_top, at);
    match app.session.add_guide(doc, page, axis, position) {
        Ok(_) => Some(position),
        Err(error) => {
            app.notify_error(error);
            None
        }
    }
}

/// The line a drag from a ruler would make, drawn over the page under the pointer.
fn preview(ui: &Ui, strips: &Strips, pages: &[(usize, super::xform::PageXform)], from_top: bool, at: Pos2, look: &Look) {
    let Some((_, xf)) = pages.iter().find(|(_, xf)| xf.rect.contains(at)) else { return };
    let line =
        if from_top { [pos2(xf.rect.left(), at.y), pos2(xf.rect.right(), at.y)] } else { [pos2(at.x, xf.rect.top()), pos2(at.x, xf.rect.bottom())] };
    ui.painter().with_clip_rect(strips.pages).line_segment(line, Stroke::new(look.line, look.guide));
}

/// The strips, the page spans on them and the ticks and numbers.
fn paint(ui: &Ui, strips: &Strips, pages: &[(usize, super::xform::PageXform)], current: usize, unit: Unit, look: &Look) {
    let painter = ui.painter();
    for rect in [strips.top, strips.left, strips.corner] {
        painter.rect_filled(rect, 0.0, look.strip);
    }
    // The top ruler once for each column of pages, from the left edge of the page the view is on, else the first.
    let mut columns: Vec<f32> = Vec::new();
    let first = pages.iter().find(|(page, _)| *page == current).or(pages.first());
    for (_, xf) in first.into_iter().chain(pages.iter()) {
        let left = xf.rect.left();
        if columns.iter().any(|c| (c - left).abs() < 0.5) {
            continue;
        }
        columns.push(left);
        paint_axis(ui, strips.top, true, xf.rect.left()..=xf.rect.right(), xf.zoom(), unit, look);
    }
    for (_, xf) in pages {
        paint_axis(ui, strips.left, false, xf.rect.top()..=xf.rect.bottom(), xf.zoom(), unit, look);
    }
}

/// One page's span on a ruler: shaded, with ticks from its start and a number at each major one.
fn paint_axis(ui: &Ui, strip: Rect, horizontal: bool, span: std::ops::RangeInclusive<f32>, zoom: f32, unit: Unit, look: &Look) {
    let along = |r: Rect| if horizontal { r.x_range() } else { r.y_range() };
    let visible = along(strip);
    let (from, to) = (span.start().max(visible.min), span.end().min(visible.max));
    let empty = to <= from;
    if empty {
        return;
    }
    let painter = ui.painter().with_clip_rect(strip);
    let shade = if horizontal {
        Rect::from_min_max(pos2(from, strip.top()), pos2(to, strip.bottom()))
    } else {
        Rect::from_min_max(pos2(strip.left(), from), pos2(strip.right(), to))
    };
    painter.rect_filled(shade, 0.0, look.page);
    let per_unit = zoom / unit.per_point() as f32;
    let stroke = Stroke::new(look.line, look.tick);
    let across = if horizontal { strip.height() } else { strip.width() };
    for tick in ticks(*span.start(), per_unit, from, to) {
        let reach = if tick.major { across * 0.6 } else { across * 0.3 };
        let (a, b) = if horizontal {
            (pos2(tick.at, strip.bottom()), pos2(tick.at, strip.bottom() - reach))
        } else {
            (pos2(strip.right(), tick.at), pos2(strip.right() - reach, tick.at))
        };
        painter.line_segment([a, b], stroke);
        if tick.major {
            let at = if horizontal { pos2(tick.at + 2.0, strip.top()) } else { pos2(strip.left() + 1.0, tick.at + 1.0) };
            painter.text(at, Align2::LEFT_TOP, label(tick.value, per_unit), look.font.clone(), look.text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_are_numbered_every_few_dozen_points_and_halved_between() {
        // 72 screen points per unit: a numbered tick every unit (72 >= 48), a plain one halfway.
        let all = ticks(100.0, 72.0, 100.0, 400.0);
        let majors: Vec<f64> = all.iter().filter(|t| t.major).map(|t| t.value).collect();
        assert!(majors.starts_with(&[0.0, 1.0, 2.0]), "{majors:?}");
        assert!(all.iter().any(|t| !t.major && (t.value - 0.5).abs() < 1e-9));
        assert!(all.iter().all(|t| t.at >= 100.0 - 72.0 && t.at <= 400.0 + 72.0));
    }

    #[test]
    fn zooming_out_numbers_larger_steps_and_keeps_them_apart() {
        let majors = |per_unit: f32| ticks(0.0, per_unit, 0.0, 600.0).into_iter().filter(|t| t.major).collect::<Vec<_>>();
        let (near, far) = (majors(72.0), majors(7.2));
        let step = |m: &[Tick]| m[1].value - m[0].value;
        assert!(step(&far) > step(&near), "{} then {}", step(&near), step(&far));
        for m in [near, far] {
            assert!(m[1].at - m[0].at >= MIN_MAJOR, "{m:?}");
        }
    }

    #[test]
    fn a_ruler_without_a_usable_scale_has_no_ticks() {
        assert!(ticks(0.0, 0.0, 0.0, 100.0).is_empty());
        assert!(ticks(f32::NAN, 10.0, 0.0, 100.0).is_empty());
        assert!(ticks(0.0, 10.0, 100.0, 0.0).is_empty());
    }

    #[test]
    fn a_number_has_the_decimals_its_step_needs() {
        assert_eq!(label(2.0, 72.0), "2");
        assert_eq!(label(0.5, 200.0), "0.5");
    }
}

/// The style of a ruler's control: the strip paints itself.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
pub(crate) struct RulerStopStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What a ruler's control reports.
pub(crate) struct RulerStopOutput {
    response: egui::Response,
    activated: bool,
    focused: bool,
}

impl Outcome for RulerStopOutput {
    type Value = NoValue;

    fn response(&self) -> &egui::Response {
        &self.response
    }
}

/// A ruler as the framework's keyboard and assistive technology see it: a Tab stop with a button's role and keys. It
/// paints nothing, and the pointer's drag is read from the strip itself.
#[derive(Component)]
#[component(canvas_control, sample = RulerStop::sample)]
pub(crate) struct RulerStop {
    #[common]
    common: Common,
    activated: bool,
    focused: bool,
}

impl RulerStop {
    fn new(name: String) -> Self {
        RulerStop { common: Common::new(), activated: false, focused: false }.label(Label::from(name))
    }

    /// For the component suite.
    fn sample() -> Self {
        RulerStop::new(tl!("Horizontal ruler").to_string())
    }
}

impl Component for RulerStop {
    const ROLE: Role = Role::BUTTON;
    const PARTS: bool = false;
    const OVERLAYS: bool = false;
    type Style = RulerStopStyle;
    type Output = RulerStopOutput;

    fn style(_: &StyleCx<'_>, _: RulerStopStyleKey) -> RulerStopStyle {
        RulerStopStyle { metrics: Metrics::new() }
    }

    fn sense(&self) -> Behaviour {
        Behaviour::FOCUS
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> egui::Vec2 {
        egui::Vec2::ZERO
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.activated = cx.activation().is_some();
        self.focused = cx.is_focused();
    }

    fn finish(self, cx: Cx<'_, Self>) -> RulerStopOutput {
        RulerStopOutput { response: cx.response.clone(), activated: self.activated, focused: self.focused }
    }
}
