//! The Organize panel's Pages selected menu picks pages by kind: odd, even, landscape, portrait, all or none.

use crate::support::AppHarness as _;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

/// Whether page `i` of the fixture is landscape: a mix that does not line up with odd and even.
fn is_landscape_page(i: usize) -> bool {
    matches!(i, 1 | 2 | 5)
}

/// A six-page document whose pages are portrait or landscape by [`is_landscape_page`].
fn mixed_fixture() -> Vec<u8> {
    let n = 6;
    let mut objs: Vec<String> = vec!["<< /Type /Catalog /Pages 2 0 R >>".into()];
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + i)).collect();
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")));
    for i in 0..n {
        let media = if is_landscape_page(i) { "[0 0 300 200]" } else { "[0 0 200 300]" };
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox {media} >>"));
    }
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn organize() -> Harness<'static, PdfCraftApp> {
    let bytes = mixed_fixture();
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1400.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "mixed.pdf", None, bytes.clone()).unwrap();
            app.set_option(&egui::Context::default(), "organize", "on").unwrap();
            app
        }
    });
    h.run_steps(4);
    h
}

/// Opens the field's menu and chooses `entry`.
fn choose(h: &mut Harness<'static, PdfCraftApp>, entry: &str) {
    h.get_by_label("Select pages").click();
    h.run_steps(3);
    h.get_by_label(entry).click();
    h.run_steps(3);
}

/// The pages the grid has selected, sorted.
fn selected(h: &Harness<'static, PdfCraftApp>) -> Vec<usize> {
    let mut pages = h.state().views[0].selected.items.clone();
    pages.sort_unstable();
    pages
}

#[test]
fn the_pages_selected_menu_picks_odd_and_landscape_pages() {
    let mut h = organize();
    let (count, landscape) = {
        let app = h.state();
        let doc = app.session.get(app.views[0].id).unwrap();
        let landscape: Vec<usize> = doc.info.pages.iter().enumerate().filter(|(_, p)| p.width > p.height).map(|(i, _)| i).collect();
        (doc.info.pages.len(), landscape)
    };
    assert!(!landscape.is_empty() && landscape.len() < count, "the fixture mixes portrait and landscape pages");

    choose(&mut h, "Odd pages");
    let odd = selected(&h);
    assert!(!odd.is_empty() && odd.len() < count);
    assert!(odd.iter().all(|p| p % 2 == 0), "Odd pages are the 1st, 3rd, 5th: indices 0, 2, 4");
    assert!((0..count).filter(|p| p % 2 == 0).all(|p| odd.contains(&p)), "every odd page is picked");

    choose(&mut h, "Landscape pages");
    let picked = selected(&h);
    assert_eq!(picked, landscape, "Landscape pages replaces the selection with exactly the landscape pages");
    assert_ne!(picked, odd, "a menu choice is one criterion, not combined with the earlier one");

    choose(&mut h, "Portrait pages");
    let portrait = selected(&h);
    assert!(portrait.iter().all(|p| !landscape.contains(p)));
    assert_eq!(portrait.len() + landscape.len(), count, "portrait and landscape together are every page");

    choose(&mut h, "Even pages");
    assert!(selected(&h).iter().all(|p| p % 2 == 1));

    choose(&mut h, "None");
    assert!(selected(&h).is_empty());
}
