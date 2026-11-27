//! The find bar's "Include bookmarks" option: a word only a bookmark's title holds is found when the option is on, and
//! moving to it goes where the bookmark goes. A bookmark that only opens a web address is not listed.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::PdfCraftApp;

use crate::support::{self, AppHarness as _};

/// Two pages without the word "zebra". A bookmark "Zebra terms" goes to page 2; "Zebra online" opens a web address.
const FIXTURE: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R /Outlines 5 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] >> endobj
4 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] >> endobj
5 0 obj << /Type /Outlines /First 6 0 R /Last 7 0 R /Count 2 >> endobj
6 0 obj << /Title (Zebra terms) /Parent 5 0 R /Next 7 0 R /Dest [4 0 R /Fit] >> endobj
7 0 obj << /Title (Zebra online) /Parent 5 0 R /Prev 6 0 R /A << /S /URI /URI (https://example.com/) >> >> endobj
trailer << /Root 1 0 R >>
%%EOF";

fn matches_found(h: &Harness<'static, PdfCraftApp>) -> usize {
    h.state().views[0].find.as_ref().map_or(0, |f| f.matches.len())
}

#[test]
fn the_find_bar_finds_a_bookmark_only_when_asked_to_and_goes_where_it_goes() {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1400.0)).build_eframe(|cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "terms.pdf", None, FIXTURE.to_vec()).expect("opens");
            app.set_option(&egui::Context::default(), "left", "closed").unwrap();
            app.set_option(&egui::Context::default(), "find", "zebra").unwrap();
            app
        }
    });
    h.run_steps(4);
    support::settle(&mut h);
    h.run_steps(3);
    assert_eq!(matches_found(&h), 0, "the page text has no such word");
    assert_eq!(h.state().views[0].current(), 0);

    h.get_by_label("Include bookmarks").click();
    h.run_steps(3);
    assert_eq!(matches_found(&h), 1, "only the bookmark that goes to a page");

    h.state_mut().views[0].find_step(true);
    h.run_steps(3);
    assert_eq!(h.state().views[0].current(), 1, "the bookmark's page");
    assert_eq!(h.query_all_by_label("Bookmark: Zebra terms").count(), 1, "the bar says which bookmark it is");

    h.get_by_label("Include bookmarks").click();
    h.run_steps(3);
    assert_eq!(matches_found(&h), 0);
}

/// Advanced search lists comment and bookmark hits under their own headings, and a click on one goes there.
#[test]
fn the_search_panel_lists_comment_and_bookmark_hits_when_asked_to() {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1400.0)).build_eframe(|cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "both.pdf", None, WITH_NOTE.to_vec()).expect("opens");
            app.set_option(&egui::Context::default(), "left", "closed").unwrap();
            app
        }
    });
    h.run_steps(4);
    assert!(h.execute("edit.advanced_search"));
    h.run_steps(2);
    h.state_mut().views[0].find.as_mut().expect("searching").query = "zebra".into();
    h.run_steps(3);
    support::settle(&mut h);
    assert_eq!(matches_found(&h), 0, "the page text has no such word");

    h.get_by_label("Include comments").click();
    h.run_steps(2);
    h.get_by_label("Include bookmarks").click();
    h.run_steps(4);
    assert_eq!(matches_found(&h), 2);
    h.get_by_label("2 instances");
    h.get_by_label_contains("Page 1: A zebra crossed here");

    h.get_by_label_contains("Zebra terms").click();
    h.run_steps(4);
    assert_eq!(h.state().views[0].current(), 1, "the bookmark's page");

    h.get_by_label_contains("A zebra crossed here").click();
    h.run_steps(4);
    assert_eq!(h.state().views[0].current(), 0, "the comment's page");
    assert!(!h.state().views[0].session.comments.selection().is_empty(), "the comment is selected");
}

/// As the fixture above, with a note on page 1.
const WITH_NOTE: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R /Outlines 5 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Annots [7 0 R] >> endobj
4 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] >> endobj
5 0 obj << /Type /Outlines /First 6 0 R /Last 6 0 R /Count 1 >> endobj
6 0 obj << /Title (Zebra terms) /Parent 5 0 R /Dest [4 0 R /Fit] >> endobj
7 0 obj << /Type /Annot /Subtype /Text /Rect [20 100 40 120] /NM (n1) /T (Alice) /Contents (A zebra crossed here) >> endobj
trailer << /Root 1 0 R >>
%%EOF";
