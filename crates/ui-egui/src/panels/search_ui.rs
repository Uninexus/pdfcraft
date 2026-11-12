//! Edit ▸ Advanced Search (⇧⌘F): the search in a side panel that lists every match with its
//! context, page by page; click a result to go to it. It shares the find bar's search.

use egui::text::LayoutJob;
use egui::{Color32, FontId, TextFormat};

use crate::canvas::DocView;
use crate::theme::{self, Tokens};
use crate::widgets;

/// What the panel asks the app to do beyond its own view.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use crate::search_folder::PanelAction;
#[cfg(target_arch = "wasm32")]
pub(crate) enum PanelAction {}

/// Glyphs of context shown on each side of a match.
const CONTEXT: usize = 36;

/// Draw the Search panel with its query, options, progress and clickable result excerpts.
pub(crate) fn panel(ui: &mut egui::Ui, t: &Tokens, view: &mut DocView, pages: usize) -> Option<PanelAction> {
    crate::widgets::begin_dialog(ui);
    let searched = view.texts.len() + view.text_failed.len();
    let find = view.find.as_mut()?;
    find.in_panel = true;
    let l = ui.label(egui::RichText::new(tl!("What word or phrase would you like to search for?")).color(t.text_muted));
    let r = ui
        .add(crate::widgets::line(&mut find.query).id(egui::Id::unique("search-panel-input")).hint_text(tl!("Search")).desired_width(f32::INFINITY))
        .accessible_name(tl!("Search"))
        .labelled_by(l.id);
    if find.focus {
        r.request_focus();
        find.focus = false;
    }
    ui.horizontal(|ui| {
        // Outlined boxes (the panel and the check-box fill are alike), as in dialogs.
        let w = &mut ui.visuals_mut().widgets;
        w.inactive.bg_stroke = egui::Stroke::new(1.0, t.border);
        w.inactive.bg_fill = t.hover;
        let a = ui.add(widgets::checkbox(&mut find.whole_words, tl!("Whole words only"))).changed();
        let b = ui.add(widgets::checkbox(&mut find.case_sensitive, tl!("Case-sensitive"))).changed();
        if a || b {
            find.case_query.clear();
        }
    });
    // Where to search: this document, or every PDF in a folder.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let folder = &mut view.folder_search;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Search in:")).color(t.text_muted));
            ui.selectable_value(&mut folder.on, false, tl!("This document"));
            ui.selectable_value(&mut folder.on, true, tl!("Folder"));
        });
        if folder.on {
            let (query, case_sensitive, whole_words) = (find.query.clone(), find.case_sensitive, find.whole_words);
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && folder.folder.is_some() && !query.trim().is_empty() {
                folder.start(ui.ctx(), &query, case_sensitive, whole_words);
            }
            return crate::search_folder::section(ui, t, folder, &query, case_sensitive, whole_words);
        }
    }
    if find.query != find.case_query {
        view.rerun_find();
    }
    let find = view.find.as_ref()?;
    ui.add_space(6.0);
    let status = if find.query.trim().is_empty() {
        String::new()
    } else if find.matches.is_empty() && searched < pages {
        crate::i18n::fmt(tl!("Searching… {s} of {p} pages"), &[("s", &searched.to_string()), ("p", &pages.to_string())])
    } else {
        let n = find.matches.len();
        let more = if searched < pages {
            crate::i18n::fmt(tl!(" (searching… {s} of {p} pages)"), &[("s", &searched.to_string()), ("p", &pages.to_string())])
        } else {
            String::new()
        };
        // English keeps its singular; Chinese has no plural.
        let count = if n == 1 { crate::i18n::fmt(tl!("1 instance"), &[]) } else { crate::i18n::fmt(tl!("{n} instances"), &[("n", &n.to_string())]) };
        format!("{count}{more}")
    };
    ui.label(egui::RichText::new(status).font(theme::semibold(12.5)));
    ui.add_space(4.0);
    let mut go = None;
    let mut last_page = None;
    for (i, (p, range)) in find.matches.iter().enumerate().take(2000) {
        if last_page != Some(*p) {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(crate::i18n::fmt(tl!("Page {label}"), &[("label", &(p + 1).to_string())])).small().color(t.text_faint));
            last_page = Some(*p);
        }
        let Some(text) = view.texts.get(p) else { continue };
        let n = text.glyphs.len();
        let before = text.text_of(range.start.saturating_sub(CONTEXT)..range.start.min(n));
        let hit = text.text_of(range.start.min(n)..range.end.min(n));
        let after = text.text_of(range.end.min(n)..(range.end + CONTEXT).min(n));
        let flat = |s: String| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut job = LayoutJob::default();
        let font = FontId::new(12.5, egui::FontFamily::Proportional);
        let plain = TextFormat { font_id: font.clone(), color: t.text_muted, ..Default::default() };
        job.append(&format!("…{} ", flat(before)), 0.0, plain.clone());
        job.append(
            &flat(hit),
            0.0,
            TextFormat {
                font_id: theme::semibold(12.5),
                color: t.text,
                background: Color32::from_rgba_unmultiplied(255, 214, 0, 90),
                ..Default::default()
            },
        );
        job.append(&format!(" {}…", flat(after)), 0.0, plain);
        job.wrap.max_width = ui.available_width();
        let selected = find.current == Some(i);
        let resp = ui.add(egui::Button::selectable(selected, job).wrap()).on_hover_text(tl!("Go to this result"));
        if resp.clicked() {
            go = Some(i);
        }
    }
    if let Some(i) = go {
        view.go_to_match(i);
    }
    None
}
