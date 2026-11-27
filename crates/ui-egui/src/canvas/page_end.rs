//! Where the keyboard goes when Tab runs off a form: Tab on the last field and Shift+Tab on the first leave the fields,
//! so the page view never holds the keyboard (app-agents.md rule 21).
//!
//! The fields are stops inside the page view, which comes before them in Tab order. Backwards the focus therefore goes to
//! the page view itself, and forwards to a stop after it, "End of page content"; the next press moves on as usual.

use egui::{Rect, Stroke, StrokeKind, Ui};

use craft::behavior::{Metrics, Sense};
use craft::ui::contract::{Common, Component, Cx, Focus as _, MeasureCx, NoValue, Outcome, Props as _, Role, Show as _};
use craft::ui::runtime::env::{Env, Label};
use craft::ui::runtime::focus::{self, FocusTarget};
use craft::ui::style::{ComponentStyle, StyleCx};

use crate::PdfCraftApp;

/// Registers the stop after the page view of the tab at `index`, drawn over `rect` while it has the focus, and moves
/// the focus when a field's Tab ran off the form. A document without form fields gets no stop.
pub(super) fn show(app: &mut PdfCraftApp, index: usize, ui: &mut Ui, rect: Rect, page_view: egui::Id) {
    let Some(view) = app.views.get(index) else { return };
    if app.session.get(view.id).is_none_or(|doc| doc.form.is_empty()) {
        return;
    }
    let key = view.view_key();
    let id = egui::Id::unique(("pdfcraft-page-end", key));
    // A strip along the bottom edge: the stop takes no pointer, and its ring goes round the whole page view.
    let strip = Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 1.0), rect.max);
    let end = PageEnd::new(tl!("End of page content").to_string()).id(id).show_at(ui, strip);
    if end.focused {
        ring(ui, rect);
    }
    let Some(view) = app.views.get_mut(index) else { return };
    // The value typed in the field left behind is stored first: a refused one reopens its field, which keeps the keys.
    if view.session.forms.leave.is_some() && view.session.pending_edit.is_some() {
        ui.ctx().request_repaint();
        return;
    }
    if let Some(forward) = view.session.forms.leave.take()
        && view.session.forms.focus.is_none()
    {
        view.session.forms.refocus = false;
        let target = if forward { end.response.id } else { page_view };
        focus::request(ui.ctx(), FocusTarget::Id(target));
        ui.ctx().request_repaint();
    }
}

/// The focus ring round the page view.
pub(crate) fn ring(ui: &Ui, rect: Rect) {
    let env = Env::of(ui);
    let [r, g, b, a] = env.tokens.color.status.info.solid.0;
    let stroke = Stroke::new(2.0 * env.tokens.stroke.thin, egui::Color32::from_rgba_unmultiplied(r, g, b, a));
    ui.painter().rect_stroke(rect, 0.0, stroke, StrokeKind::Inside);
}

/// The style of the stop: it paints nothing of its own.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
pub(crate) struct PageEndStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What the stop reports.
pub(crate) struct PageEndOutput {
    pub(crate) response: egui::Response,
    pub(crate) focused: bool,
}

impl Outcome for PageEndOutput {
    type Value = NoValue;

    fn response(&self) -> &egui::Response {
        &self.response
    }
}

/// A Tab stop that does nothing but hold the focus: after the page view, and at the start of a side panel that has no
/// list to take it.
#[derive(Component)]
#[component(canvas_control, sample = PageEnd::sample)]
pub(crate) struct PageEnd {
    #[common]
    common: Common,
    focused: bool,
}

impl PageEnd {
    pub(crate) fn new(name: String) -> Self {
        PageEnd { common: Common::new(), focused: false }.label(Label::from(name))
    }

    /// For the component suite.
    fn sample() -> Self {
        PageEnd::new(tl!("End of page content").to_string())
    }
}

impl Component for PageEnd {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const OVERLAYS: bool = false;
    type Style = PageEndStyle;
    type Output = PageEndOutput;

    fn style(_: &StyleCx<'_>, _: PageEndStyleKey) -> PageEndStyle {
        PageEndStyle { metrics: Metrics::new() }
    }

    fn sense(&self) -> Sense {
        Sense::FOCUS
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> egui::Vec2 {
        egui::Vec2::ZERO
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.focused = cx.is_focused();
    }

    fn finish(self, cx: Cx<'_, Self>) -> PageEndOutput {
        PageEndOutput { response: cx.response.clone(), focused: self.focused }
    }
}
