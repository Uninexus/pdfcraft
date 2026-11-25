//! View ▸ 3D model: the 3D model of the current page (ISO 32000-2 §13.6) drawn in a window the reader can turn,
//! zoom and pan.
//!
//! It reads the page's first 3D annotation through the engine (`Document::model_3d`) and draws it with the same
//! `prc-render` path as the `model_render` tool, with the same GUI-free navigation state (`prc_render::nav::Orbit`).
//! Only PRC models are drawn: a U3D model, a page with no 3D annotation or a model that cannot be read gets a plain
//! message in place of the picture. Nothing is written to the document.

use craft::theme::variant::Tone;
use craft::ui::contract::{Bind, Show as _, ShowContainer as _};
use craft::ui::dialog::Dialog;
use craft::ui::display::{Shade, Text};
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::style::TypeRole;
use craft_prc::PrcFile;
use egui::accesskit::Role;
use pdfcraft_engine::DocId;
use pdfcraft_engine::model3d::{Model3d, Model3dKind};
use pdfcraft_prc_render::nav::{NamedView, Orbit, Upright, reach};
use pdfcraft_prc_render::{Bounds, RenderOptions, WorldMesh, assemble, render};
use std::hash::{Hash as _, Hasher as _};

use crate::PdfCraftApp;

/// The picture's size in points, which is the size it is drawn at, so it stays quick to redraw.
const PICTURE: egui::Vec2 = egui::vec2(560.0, 420.0);
/// Radians of turn for one point of a drag.
const TURN_PER_POINT: f64 = 0.01;
/// Radians of turn for one arrow-key press.
const TURN_PER_KEY: f64 = 0.1;
/// The zoom of one key press, and of one point of scrolling (an exponent).
const ZOOM_PER_KEY: f64 = 1.25;
const ZOOM_PER_SCROLL_POINT: f64 = 0.0025;
/// What a picture's height spans, in model radii, at zoom 1; a drag pans by this much per picture height.
const SPAN: f64 = 2.1;

/// What a footer button does.
#[derive(Clone, Copy)]
enum Action {
    View(NamedView),
    Reset,
    Close,
}

/// The buttons, in footer order.
const ACTIONS: [Action; 7] = [
    Action::View(NamedView::Front),
    Action::View(NamedView::Back),
    Action::View(NamedView::Right),
    Action::View(NamedView::Top),
    Action::View(NamedView::Isometric),
    Action::Reset,
    Action::Close,
];

fn view_label(view: NamedView) -> &'static str {
    match view {
        NamedView::Isometric => tl!("Isometric"),
        NamedView::Front => tl!("Front"),
        NamedView::Right => tl!("Right"),
        NamedView::Top => tl!("Top"),
        NamedView::Back => tl!("Back"),
    }
}

fn button(action: Action, ready: bool) -> FooterButton {
    match action {
        Action::View(view) => FooterButton::new(view_label(view), DialogRole::Other).stays_open(true).enabled(ready),
        Action::Reset => FooterButton::new(tl!("Reset view"), DialogRole::Other).stays_open(true).enabled(ready),
        Action::Close => FooterButton::new(tl!("Close"), DialogRole::Cancel),
    }
}

/// A model that is loaded and drawn.
struct Scene {
    meshes: Vec<WorldMesh>,
    bounds: Bounds,
    reach: f64,
    orbit: Orbit,
    /// The named view the picture is in, until the reader turns, zooms or pans it.
    view: Option<NamedView>,
    /// The picture changed since it was drawn.
    dirty: bool,
    /// The last picture drawn: its texture, size and a hash of its pixels.
    drawn: Option<Drawn>,
    /// Why the last draw failed, if it did.
    error: Option<String>,
}

struct Drawn {
    texture: egui::TextureHandle,
    size: [usize; 2],
    hash: u64,
}

/// The open 3D model window: the page's model, or the reason there is none to show.
pub struct ModelWindow {
    doc: DocId,
    /// 0-based page the window was opened for.
    page: usize,
    scene: Result<Scene, String>,
}

impl ModelWindow {
    /// Whether a model is loaded, so there is a picture (or an error saying why it could not be drawn).
    #[must_use]
    pub fn has_model(&self) -> bool {
        self.scene.is_ok()
    }

    /// The plain message shown in place of the picture: no 3D annotation, a U3D model or an unreadable one.
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        match &self.scene {
            Err(message) => Some(message),
            Ok(scene) => scene.error.as_deref(),
        }
    }

    /// The size in pixels of the picture last drawn.
    #[must_use]
    pub fn picture_size(&self) -> Option<[usize; 2]> {
        self.scene.as_ref().ok()?.drawn.as_ref().map(|d| d.size)
    }

    /// A hash of the pixels of the picture last drawn: it changes when the view does.
    #[must_use]
    pub fn picture_hash(&self) -> Option<u64> {
        self.scene.as_ref().ok()?.drawn.as_ref().map(|d| d.hash)
    }

    /// The named view the picture is in; `None` once it is turned, zoomed or panned.
    #[must_use]
    pub fn named_view(&self) -> Option<&'static str> {
        self.scene.as_ref().ok()?.view.map(NamedView::token)
    }

    fn ready(&self) -> bool {
        self.scene.is_ok()
    }

    fn run(&mut self, action: Action) {
        let Ok(scene) = &mut self.scene else { return };
        match action {
            Action::View(view) => scene.set_view(view),
            Action::Reset => scene.set_view(NamedView::Isometric),
            Action::Close => {}
        }
    }

    /// The window's content: the picture and its hint, or the message.
    fn body(&mut self, ui: &mut egui::Ui) {
        let label = crate::i18n::fmt(tl!("3D model, page {n}"), &[("n", &self.page.saturating_add(1).to_string())]);
        match &mut self.scene {
            Err(message) => {
                Text::new(message.clone()).show(ui);
            }
            Ok(scene) => {
                scene.picture(ui, &label);
                Text::new(tl!(
                    "Drag to turn, Shift-drag or middle-drag to pan, scroll to zoom. With the picture focused: arrow keys turn, plus and minus zoom."
                ))
                .role(TypeRole::Caption)
                .shade(Shade::Muted)
                .show(ui);
                if let Some(error) = &scene.error {
                    Notice::new(Tone::Bad, error.clone()).layout(NoticeLayout::Inline).show(ui);
                }
            }
        }
    }
}

impl Scene {
    fn new(model: &Model3d) -> Result<Scene, String> {
        let prc =
            PrcFile::parse(&model.data).map_err(|e| crate::i18n::fmt(tl!("The 3D model could not be read: {why}"), &[("why", &e.to_string())]))?;
        let meshes = assemble(&prc).map_err(|e| crate::i18n::fmt(tl!("The 3D model could not be read: {why}"), &[("why", &e.to_string())]))?;
        let bounds = Bounds::of(&meshes).ok_or_else(|| tl!("The 3D model has nothing to draw.").to_string())?;
        let reach = reach(&meshes, &bounds);
        let orbit = Orbit::named(NamedView::Isometric, true, Upright::default());
        Ok(Scene { meshes, bounds, reach, orbit, view: Some(NamedView::Isometric), dirty: true, drawn: None, error: None })
    }

    fn aspect() -> f64 {
        f64::from(PICTURE.x) / f64::from(PICTURE.y)
    }

    /// Looks from `view`, fitted again.
    fn set_view(&mut self, view: NamedView) {
        self.orbit = Orbit::named(view, self.orbit.perspective(), Upright::default());
        self.view = Some(view);
        self.dirty = true;
    }

    fn turn(&mut self, yaw: f64, pitch: f64) {
        self.orbit.turn(yaw, pitch);
        self.changed();
    }

    fn zoom(&mut self, factor: f64, offset: [f64; 2]) {
        self.orbit.zoom_at(&self.bounds, self.reach, Self::aspect(), factor, offset);
        self.changed();
    }

    fn pan(&mut self, right: f64, up: f64) {
        self.orbit.pan_by([right, up]);
        self.changed();
    }

    fn changed(&mut self) {
        self.view = None;
        self.dirty = true;
    }

    /// Draws the model again when the view changed.
    fn redraw(&mut self, ctx: &egui::Context) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let width = PICTURE.x as u32;
        let height = PICTURE.y as u32;
        let drawn = self
            .orbit
            .camera(&self.bounds, self.reach, Self::aspect())
            .and_then(|camera| render(&self.meshes, &camera, &RenderOptions { width, height, ..RenderOptions::default() }));
        match drawn {
            Ok(image) => {
                let size = [image.width as usize, image.height as usize];
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                image.rgba.hash(&mut hasher);
                let hash = hasher.finish();
                let colours = egui::ColorImage::from_rgba_unmultiplied(size, &image.rgba);
                match &mut self.drawn {
                    Some(old) => {
                        old.texture.set(colours, egui::TextureOptions::LINEAR);
                        (old.size, old.hash) = (size, hash);
                    }
                    None => {
                        let texture = ctx.load_texture("model3d", colours, egui::TextureOptions::LINEAR);
                        self.drawn = Some(Drawn { texture, size, hash });
                    }
                }
                self.error = None;
            }
            Err(e) => self.error = Some(crate::i18n::fmt(tl!("The 3D model could not be drawn: {why}"), &[("why", &e.to_string())])),
        }
    }

    /// The picture, the surface the reader turns, zooms and pans.
    fn picture(&mut self, ui: &mut egui::Ui, label: &str) {
        self.redraw(ui.ctx());
        let (rect, response) = ui.allocate_exact_size(PICTURE, egui::Sense::click_and_drag());
        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        let value = match self.view {
            Some(view) => view_label(view).to_string(),
            None => tl!("Custom view").to_string(),
        };
        response.widget_info(|| egui::WidgetInfo {
            label: Some(label.to_string()),
            current_text_value: Some(value.clone()),
            ..egui::WidgetInfo::new(Role::Image)
        });
        if let Some(drawn) = &self.drawn {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(drawn.texture.id(), rect, uv, egui::Color32::WHITE);
        }
        if response.has_focus() {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(response.id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() });
            });
            self.keys(ui);
        }
        self.pointer(ui, &response, rect);
    }

    fn keys(&mut self, ui: &egui::Ui) {
        let pressed = |key| ui.input(|i| i.key_pressed(key));
        let step = |negative: egui::Key, positive: egui::Key| f64::from(i8::from(pressed(positive)) - i8::from(pressed(negative)));
        let (yaw, pitch) = (step(egui::Key::ArrowRight, egui::Key::ArrowLeft), step(egui::Key::ArrowUp, egui::Key::ArrowDown));
        if yaw != 0.0 || pitch != 0.0 {
            self.turn(yaw * TURN_PER_KEY, pitch * TURN_PER_KEY);
        }
        if pressed(egui::Key::Plus) || pressed(egui::Key::Equals) {
            self.zoom(ZOOM_PER_KEY, [0.0, 0.0]);
        }
        if pressed(egui::Key::Minus) {
            self.zoom(1.0 / ZOOM_PER_KEY, [0.0, 0.0]);
        }
    }

    fn pointer(&mut self, ui: &egui::Ui, response: &egui::Response, rect: egui::Rect) {
        let delta = response.drag_delta();
        if delta != egui::Vec2::ZERO {
            let panning = response.dragged_by(egui::PointerButton::Middle) || ui.input(|i| i.modifiers.shift);
            if panning {
                let span = SPAN / self.orbit.zoom() / f64::from(rect.height().max(1.0));
                self.pan(-f64::from(delta.x) * span, f64::from(delta.y) * span);
            } else {
                self.turn(-f64::from(delta.x) * TURN_PER_POINT, f64::from(delta.y) * TURN_PER_POINT);
            }
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta().y);
            if scroll != 0.0 {
                let height = f64::from(rect.height().max(1.0));
                let offset = ui.input(|i| i.pointer.hover_pos()).map_or([0.0, 0.0], |p| {
                    let from_centre = p - rect.center();
                    [f64::from(from_centre.x) / height, -f64::from(from_centre.y) / height]
                });
                self.zoom((f64::from(scroll) * ZOOM_PER_SCROLL_POINT).exp(), offset);
            }
        }
    }
}

impl PdfCraftApp {
    /// `view.model3d`: opens the 3D model window for the current page of the active document. A page with no 3D
    /// annotation, or one in U3D, opens the window with a message saying so.
    pub(crate) fn open_model_3d(&mut self) {
        let Some((view, id)) = self.active_ids() else { return };
        let page = self.views[view].current();
        let Some(doc) = self.session.get(id) else { return };
        let scene = match doc.model_3d_annotations(page) {
            Err(e) => Err(e),
            Ok(found) => match found.first() {
                None => Err(tl!("This page has no 3D model.").to_string()),
                Some(&index) => doc.model_3d(page, index).and_then(|model| {
                    if model.kind == Model3dKind::U3d {
                        Err(tl!("This 3D model is in U3D, which is not supported yet. Only PRC models can be shown.").to_string())
                    } else {
                        Scene::new(&model)
                    }
                }),
            },
        };
        self.model3d = Some(ModelWindow { doc: id, page, scene });
    }
}

/// Shows the window while it is open and carries out the button it was answered with. It closes with its document.
pub(crate) fn show(app: &mut PdfCraftApp, ui: &mut egui::Ui) {
    let Some(mut w) = app.model3d.take() else { return };
    if app.active_ids().map(|(_, id)| id) != Some(w.doc) {
        return;
    }
    let ready = w.ready();
    let buttons: Vec<FooterButton> = ACTIONS.iter().map(|a| button(*a, ready)).collect();
    let mut open = true;
    let answered = Dialog::new(tl!("3D model")).opened(Bind::bound(&mut open)).buttons(buttons).show(ui, |ui, _cx| w.body(ui));
    if let Some(action) = answered.output.picked.and_then(|at| ACTIONS.get(at)) {
        w.run(*action);
    }
    if open {
        app.model3d = Some(w);
    }
}
