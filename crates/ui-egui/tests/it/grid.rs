//! View > Show Grid: the page grid drawn over each page, on screen only.

use egui_kittest::Harness;
use pdfcraft_engine::guides::Grid;
use pdfcraft_ui_egui::PdfCraftApp;

use crate::support::AppHarness as _;

/// The app with one 200 x 300 pt page open in a 1400 x 1400 window.
fn harness() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1400.0)).build_eframe(|cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        let mut app = crate::control::new_app();
        app.set_option(&egui::Context::default(), "language", "en").unwrap();
        app.open_bytes(&egui::Context::default(), "doc.pdf", None, crate::control::fixture(1)).unwrap();
        app
    });
    h.run_steps(4);
    h
}

/// The two-point lines in the last frame, as screen end points.
fn lines(h: &Harness<'static, PdfCraftApp>) -> Vec<[egui::Pos2; 2]> {
    fn walk(shape: &egui::Shape, out: &mut Vec<[egui::Pos2; 2]>) {
        match shape {
            egui::Shape::LineSegment { points, .. } => out.push(*points),
            egui::Shape::Path(p) if !p.closed && p.points.len() == 2 => out.push([p.points[0], p.points[1]]),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    h.output().shapes.iter().for_each(|clipped| walk(&clipped.shape, &mut out));
    out
}

/// How many straight vertical and horizontal lines run inside the first page, and how many lines there are elsewhere.
fn counts(h: &Harness<'static, PdfCraftApp>) -> (usize, usize, usize) {
    let page = h.state().views[0].page_screen_rect(0).expect("the page is on screen").expand(1.0);
    let all = lines(h);
    let inside = |l: &&[egui::Pos2; 2]| page.contains(l[0]) && page.contains(l[1]);
    let vertical = all.iter().filter(inside).filter(|l| (l[0].x - l[1].x).abs() < 0.5).count();
    let horizontal = all.iter().filter(inside).filter(|l| (l[0].y - l[1].y).abs() < 0.5).count();
    (vertical, horizontal, all.iter().filter(|l| !inside(l)).count())
}

fn show(h: &mut Harness<'static, PdfCraftApp>, grid: Grid) {
    h.state_mut().session.set_grid_preference(Some(grid)).unwrap();
    h.run_steps(3);
}

#[test]
fn the_shown_grid_is_drawn_over_the_page_and_a_finer_spacing_draws_more_lines() {
    let mut h = harness();
    let hidden = counts(&h);

    // Off by default; the command and the option both turn it on and off.
    assert!(h.execute("view.show_grid"));
    h.run_steps(3);
    let shown = counts(&h);
    assert!(shown.0 > hidden.0 && shown.1 > hidden.1, "grid lines run both ways over the page: {hidden:?} then {shown:?}");
    assert_eq!(shown.2, hidden.2, "nothing is drawn outside the page");

    let coarse = Grid { spacing: 72.0, subdivisions: 1, offset_x: 0.0, offset_y: 0.0 };
    show(&mut h, coarse);
    let one_inch = counts(&h);
    show(&mut h, Grid { spacing: 36.0, ..coarse });
    let half_inch = counts(&h);
    assert!(half_inch.0 > one_inch.0 && half_inch.1 > one_inch.1, "halving the spacing draws more lines: {one_inch:?} then {half_inch:?}");
    show(&mut h, Grid { subdivisions: 4, ..coarse });
    let divided = counts(&h);
    assert!(divided.0 > one_inch.0 && divided.1 > one_inch.1, "subdivisions add lines between the main ones: {one_inch:?} then {divided:?}");

    // A refused grid is not stored, so what is drawn stays.
    assert!(h.state_mut().session.set_grid_preference(Some(Grid::new(0.0))).is_err());
    h.run_steps(3);
    assert_eq!(counts(&h), divided);

    h.set_option("grid", "off").unwrap();
    h.run_steps(3);
    assert_eq!(counts(&h), hidden, "hidden again");
    assert!(h.set_option("grid", "maybe").is_err());
    h.set_option("grid", "on").unwrap();
    h.run_steps(3);
    assert_eq!(counts(&h), divided, "shown again");
}
