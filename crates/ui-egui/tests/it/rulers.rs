//! View > Show Rulers: guides are dragged out of the rulers onto a page, moved by their line, and dropped on a ruler.

use egui::{Event, Modifiers, PointerButton, Pos2, pos2};
use egui_kittest::Harness;
use pdfcraft_engine::guides::{Guide, GuideAxis};
use pdfcraft_ui_egui::PdfCraftApp;

use crate::support::AppHarness as _;

/// The height of the fixture's page, in points.
const PAGE_HEIGHT: f32 = 300.0;
const PAGE_WIDTH: f32 = 200.0;

/// The app with one 200 x 300 pt page open in a 1400 x 1400 window, rulers and guides shown.
fn harness() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1400.0)).build_eframe(|cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        let mut app = crate::control::new_app();
        app.set_option(&egui::Context::default(), "language", "en").unwrap();
        app.open_bytes(&egui::Context::default(), "doc.pdf", None, crate::control::fixture(1)).unwrap();
        app
    });
    h.set_option("zoom", "100").unwrap();
    h.run_steps(4);
    assert!(h.execute("view.show_rulers"));
    h.set_option("guides", "on").unwrap();
    h.run_steps(4);
    h
}

fn guides(h: &Harness<'static, PdfCraftApp>) -> Vec<Guide> {
    let id = h.state().views[0].id;
    h.state().session.guides(id, Some(0)).unwrap()
}

fn page(h: &Harness<'static, PdfCraftApp>) -> egui::Rect {
    h.state().views[0].page_screen_rect(0).expect("the page is on screen")
}

/// A point on the top ruler above `x`, and one on the left ruler beside `y`: just outside the room the pages have.
fn on_top_ruler(h: &Harness<'static, PdfCraftApp>, x: f32) -> Pos2 {
    pos2(x, h.state().views[0].viewport_rect().top() - 4.0)
}

fn on_left_ruler(h: &Harness<'static, PdfCraftApp>, y: f32) -> Pos2 {
    pos2(h.state().views[0].viewport_rect().left() - 4.0, y)
}

/// Press at `from`, move to `to` in steps and release there.
fn drag(h: &mut Harness<'static, PdfCraftApp>, from: Pos2, to: Pos2) {
    let press = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    h.hover_at(from);
    h.run_steps(1);
    h.event(press(from, true));
    h.run_steps(1);
    for k in 1..=4 {
        h.hover_at(from + (to - from) * (k as f32 / 4.0));
        h.run_steps(1);
    }
    h.event(press(to, false));
    h.run_steps(3);
}

/// The point `fraction` of the way down the page, and of the way across it, in points.
fn points(h: &Harness<'static, PdfCraftApp>, at: Pos2) -> (f32, f32) {
    let page = page(h);
    ((at.x - page.left()) / page.width() * PAGE_WIDTH, (at.y - page.top()) / page.height() * PAGE_HEIGHT)
}

#[test]
fn a_guide_is_dragged_out_of_the_top_ruler_moved_by_its_line_and_dropped_on_the_ruler() {
    let mut h = harness();
    assert!(guides(&h).is_empty());
    let shown = page(&h);

    // Out of the top ruler onto the page: a horizontal guide where the pointer let go.
    let drop = pos2(shown.center().x, shown.top() + shown.height() * 0.4);
    let from = on_top_ruler(&h, drop.x);
    drag(&mut h, from, drop);
    let made = guides(&h);
    assert_eq!(made.len(), 1, "{made:?}");
    assert_eq!(made[0].axis, GuideAxis::Horizontal);
    let wanted = points(&h, drop).1 as f64;
    assert!((made[0].position - wanted).abs() < 3.0, "guide at {} pt, dropped at {wanted} pt", made[0].position);

    // By its line to another place: the same guide, further down.
    let line = pos2(shown.center().x, shown.top() + made[0].position as f32 / PAGE_HEIGHT * shown.height());
    let lower = pos2(line.x, shown.top() + shown.height() * 0.7);
    drag(&mut h, line, lower);
    let moved = guides(&h);
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].id, made[0].id, "moved, not remade");
    assert!(moved[0].position > made[0].position + 30.0, "{} then {}", made[0].position, moved[0].position);
    assert!((moved[0].position - points(&h, lower).1 as f64).abs() < 3.0);

    // Onto the ruler: gone.
    let line = pos2(shown.center().x, shown.top() + moved[0].position as f32 / PAGE_HEIGHT * shown.height());
    let ruler = on_top_ruler(&h, line.x);
    drag(&mut h, line, ruler);
    assert!(guides(&h).is_empty());
}

#[test]
fn the_left_ruler_makes_a_vertical_guide() {
    let mut h = harness();
    let shown = page(&h);
    let drop = pos2(shown.left() + shown.width() * 0.25, shown.center().y);
    let from = on_left_ruler(&h, drop.y);
    drag(&mut h, from, drop);
    let made = guides(&h);
    assert_eq!(made.len(), 1, "{made:?}");
    assert_eq!(made[0].axis, GuideAxis::Vertical);
    assert!((made[0].position - points(&h, drop).0 as f64).abs() < 3.0, "{}", made[0].position);
}

#[test]
fn a_drag_from_a_ruler_that_ends_off_the_page_makes_no_guide_and_hidden_rulers_take_no_drag() {
    let mut h = harness();
    let shown = page(&h);
    let viewport = h.state().views[0].viewport_rect();
    // Ends on the pasteboard beside the page.
    let aside = pos2(viewport.right() - 10.0, shown.center().y);
    assert!(!shown.contains(aside));
    let from = on_top_ruler(&h, aside.x);
    drag(&mut h, from, aside);
    assert!(guides(&h).is_empty());

    // Where the ruler was, nothing takes a drag once the rulers are off.
    let from = on_top_ruler(&h, shown.center().x);
    h.set_option("rulers", "off").unwrap();
    h.run_steps(3);
    let drop = pos2(shown.center().x, shown.top() + shown.height() * 0.4);
    drag(&mut h, from, drop);
    assert!(guides(&h).is_empty(), "with the rulers off nothing makes a guide");
}

#[test]
fn a_ruler_reached_by_tab_adds_a_guide_in_the_middle_of_the_view_and_says_where() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable as _;
    for (ruler, axis) in [("Horizontal ruler", GuideAxis::Horizontal), ("Vertical ruler", GuideAxis::Vertical)] {
        for key in [egui::Key::Enter, egui::Key::Space] {
            let mut h = harness();
            let mut reached = false;
            for _ in 0..300 {
                h.key_press(egui::Key::Tab);
                h.run_steps(2);
                if h.query_by(|node| node.is_focused() && node.label().as_deref() == Some(ruler)).is_some() {
                    reached = true;
                    break;
                }
            }
            assert!(reached, "Tab never reached the {ruler}");
            assert!(guides(&h).is_empty());
            h.key_press(key);
            h.run_steps(3);
            let made = guides(&h);
            assert_eq!(made.len(), 1, "{ruler} {key:?}: {made:?}");
            assert_eq!(made[0].axis, axis);
            let page_size = if axis == GuideAxis::Horizontal { PAGE_HEIGHT } else { PAGE_WIDTH };
            assert!((made[0].position - f64::from(page_size) / 2.0).abs() < 3.0, "the whole page shows: {}", made[0].position);
            assert!(h.query_all_by_role(Role::Status).chain(h.query_all_by_role(Role::Alert)).count() > 0, "a live region");
            h.get_by_label_contains("Guide added at");
        }
    }
}
