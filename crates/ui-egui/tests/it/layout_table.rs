//! The Tags panel's table actions in the real shell (egui_kittest): turn a layout table into
//! blocks, and set a header cell's scope, each as one undoable step.

use crate::support::AppHarness as _;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pdfcraft_ui_egui::{PdfCraftApp, RightPanel};

/// A table with a header cell and a data cell in one row.
const TABLE: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R /MarkInfo << /Marked true >> /StructTreeRoot 5 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /StructParents 0 >> endobj
4 0 obj << /Length 70 >> stream
/Span << /MCID 0 >> BDC 10 10 20 20 re f EMC /Span << /MCID 1 >> BDC 50 10 20 20 re f EMC
endstream endobj
5 0 obj << /Type /StructTreeRoot /K 6 0 R /ParentTree 11 0 R >> endobj
6 0 obj << /S /Document /P 5 0 R /K [7 0 R] >> endobj
7 0 obj << /S /Table /P 6 0 R /K [8 0 R] >> endobj
8 0 obj << /S /TR /P 7 0 R /K [9 0 R 10 0 R] >> endobj
9 0 obj << /S /TH /P 8 0 R /Pg 3 0 R /K 0 >> endobj
10 0 obj << /S /TD /P 8 0 R /Pg 3 0 R /K 1 >> endobj
11 0 obj << /Nums [0 [9 0 R 10 0 R]] >> endobj
trailer << /Root 1 0 R >>
%%EOF";

fn tags_panel() -> Harness<'static, PdfCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 1400.0)).build_eframe(move |cc| {
        crate::support::install_runtime(&cc.egui_ctx);
        {
            let mut app = crate::control::new_app();
            app.set_option(&egui::Context::default(), "language", "en").unwrap();
            app.open_bytes(&egui::Context::default(), "table.pdf", None, TABLE.to_vec()).unwrap();
            app
        }
    });
    h.run_steps(4);
    assert!(h.execute("a11y.tags"));
    h.run_steps(2);
    assert_eq!(crate::support::right(h.state()), Some(RightPanel::Tags));
    h
}

fn kinds(h: &Harness<'static, PdfCraftApp>) -> Vec<String> {
    let s = h.state();
    s.session.get(s.views[0].id).unwrap().tags().into_iter().map(|tag| tag.kind).collect()
}

#[test]
fn a_layout_table_becomes_blocks_from_the_tags_panel_and_undo_restores_it() {
    let mut h = tags_panel();
    assert!(kinds(&h).iter().any(|k| k == "Table"));
    h.get_by_label("Table · 7").click();
    h.run_steps(2);
    h.get_by_label("Convert layout table to blocks").click();
    h.run_steps(3);
    let after = kinds(&h);
    assert!(!after.iter().any(|k| k == "Table"), "{after:?}");
    assert!(after.iter().any(|k| k == "Div"), "{after:?}");
    h.get_by_label("Div · 7");
    h.undo();
    h.run_steps(3);
    assert!(kinds(&h).iter().any(|k| k == "Table"));
}

#[test]
fn the_table_action_is_only_offered_on_table_tags() {
    let mut h = tags_panel();
    h.get_by_label("Document · 6").click();
    h.run_steps(2);
    assert!(h.query_by_label("Convert layout table to blocks").is_none());
}

#[test]
fn a_header_cell_gets_a_scope_and_a_data_cell_becomes_a_header() {
    let mut h = tags_panel();
    h.get_by_label("TH · 9").click();
    h.run_steps(2);
    h.get_by_label("Header scope: Column").click();
    h.run_steps(3);
    let s = h.state();
    let scope = s.session.get(s.views[0].id).unwrap().tags().into_iter().find(|tag| tag.object.num == 9).and_then(|tag| tag.scope);
    assert_eq!(scope.as_deref(), Some("Column"));
    h.get_by_label("TD · 10").click();
    h.run_steps(2);
    h.get_by_label("Make header cell").click();
    h.run_steps(3);
    assert_eq!(kinds(&h).iter().filter(|k| *k == "TH").count(), 2);
}
