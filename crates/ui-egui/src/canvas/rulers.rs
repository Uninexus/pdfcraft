//! View > Show Rulers: a ruler along the top and left of the document area, in the measuring units.
//!
//! The strips are drawn and read outside the page view (it is given the room inside them), so a press on a ruler never
//! reaches a page. The top ruler counts from the left edge of each page column and the left ruler from the top edge of each
//! page, along the screen's axes (a view turned a quarter shows the page's height across). A drag from a ruler onto a page
//! makes a guide there through the session's `add_guide`, the function `doc_guide_add` calls; a guide dragged by its line
//! (`guides`) and dropped on a ruler is removed. Guides are view state: no undo step, and the document is not dirty.

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, pos2};

use craft::ui::runtime::env::Env;

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

/// The strips of the tab at `index` in `rect`, or `None` when its rulers are off or it shows no page body (the
/// Organize grid, full screen). The top strip lies under the notice bars (`notice_inset`, as the last pass measured).
pub(super) fn layout(app: &PdfCraftApp, index: usize, ui: &Ui, rect: Rect) -> Option<Strips> {
    let view = app.views.get(index)?;
    if !view.show_rulers || view.organize || view.presenting() {
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

/// Paints the rulers of the tab at `index`, takes drags from them, and applies what a guide's drag asked for.
pub(super) fn show(app: &mut PdfCraftApp, index: usize, ui: &mut Ui, strips: &Strips) {
    settle_guide_drag(app, index, ui, strips);
    let Some(view) = app.views.get(index) else { return };
    let pages = view.page_xforms();
    let (key, current) = (view.view_key(), view.view.current);
    let look = look(ui);
    paint(ui, strips, &pages, current, app.units, &look);
    for from_top in [true, false] {
        let strip = if from_top { strips.top } else { strips.left };
        let name = if from_top { tl!("Horizontal ruler") } else { tl!("Vertical ruler") };
        let resp = ui.interact(strip, egui::Id::new(("pdfcraft-ruler", key, from_top)), Sense::drag());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, name));
        let hint = tl!("Drag onto a page to add a guide");
        let cursor = if from_top { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::ResizeHorizontal };
        let resp = resp.on_hover_text(hint).on_hover_cursor(cursor);
        let pointer = ui.input(|i| i.pointer.latest_pos());
        if let Some(at) = pointer.filter(|_| resp.dragged()) {
            preview(ui, strips, &pages, from_top, at, &look);
        }
        if resp.drag_stopped()
            && let Some(at) = pointer
        {
            drop_guide(app, index, strips, from_top, at);
        }
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

/// Makes the guide a drag from a ruler dropped at `at` stands for, when that is on a page.
fn drop_guide(app: &mut PdfCraftApp, index: usize, strips: &Strips, from_top: bool, at: Pos2) {
    let Some(view) = app.views.get_mut(index) else { return };
    let doc = view.id;
    let Some((page, xf)) = view.page_xforms().into_iter().find(|(_, xf)| xf.rect.contains(at)) else { return };
    if !strips.pages.contains(at) {
        return;
    }
    // A guide made while guides are hidden would be lost to sight at once.
    view.show_guides = true;
    let (axis, position) = guides::dropped(&xf, from_top, at);
    if let Err(error) = app.session.add_guide(doc, page, axis, position) {
        app.notify_error(error);
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
    fn zooming_out_numbers_fewer_ticks() {
        let near = ticks(0.0, 72.0, 0.0, 600.0).iter().filter(|t| t.major).count();
        let far = ticks(0.0, 7.2, 0.0, 600.0).iter().filter(|t| t.major).count();
        assert!(far < near, "{near} then {far}");
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
