//! Pages ▸ Remove white margins: the dialog (pages and the padding to keep) and the background
//! run with its progress card. Finding the content of each page takes a raster of it, so the
//! measuring runs on a worker thread ([`pdfcraft_engine::white_margins`]), page by page, and
//! can be cancelled between pages. The new crop boxes are applied as one undoable step when it
//! is done, unless the document changed meanwhile.

use std::sync::{Arc, Mutex};

use egui::{Align, Layout};
use pdfcraft_engine::white_margins::{MAX_PADDING, WhiteMargins, WhiteMarginsReport};
use pdfcraft_engine::{DocId, EditError};

use crate::pageboxes::Range;
use crate::theme::{self, Tokens};
use crate::{PdfCraftApp, widgets};

/// The dialog's choices. Only the padding is kept between runs (in points, shown in the unit
/// chosen in Preferences); the pages go back to what was last chosen only while the app runs
/// and are re-clamped against each document.
#[derive(Clone, Debug, PartialEq)]
pub struct WhiteMarginsDraft {
    pub range: Range,
    pub padding: f64,
}

impl Default for WhiteMarginsDraft {
    fn default() -> Self {
        Self { range: Range::All, padding: 0.0 }
    }
}

impl WhiteMarginsDraft {
    /// The pages (0-based) the dialog applies to, in a document of `count` pages whose current
    /// page is `current`. Never empty for a document with pages; a range past the end is
    /// clamped to the last page, so a stale range can't mean "all pages" to the engine.
    pub fn pages(&self, current: usize, count: usize) -> Vec<usize> {
        let last = count.saturating_sub(1);
        match self.range {
            Range::All => (0..count).collect(),
            Range::Current => vec![current.min(last)],
            Range::Pages(a, b) => {
                let (a, b) = (a.max(1).min(count.max(1)), b.max(1).min(count.max(1)));
                (a.min(b) - 1..a.max(b).min(count)).collect()
            }
        }
    }

    /// Why the padding can't be used, if so.
    pub fn problem(&self, unit: crate::units::Unit) -> Option<String> {
        (!self.padding.is_finite() || !(0.0..=MAX_PADDING).contains(&self.padding)).then(|| {
            let max = format!("{} {}", (MAX_PADDING * unit.per_point() * 1000.0).round() / 1000.0, unit.label());
            crate::i18n::fmt(tl!("The padding must be between 0 and {max}"), &[("max", &max)])
        })
    }
}

/// Progress of a run: pages done, total, the result once finished, and a cancel request.
#[derive(Default)]
pub struct WhiteMarginsProgress {
    pub done: usize,
    pub total: usize,
    pub result: Option<Result<WhiteMargins, EditError>>,
    pub cancel: bool,
}

pub struct WhiteMarginsRun {
    pub doc: DocId,
    pub progress: Arc<Mutex<WhiteMarginsProgress>>,
}

/// Draw the dialog body; returns (remove, cancel).
pub(crate) fn body(ui: &mut egui::Ui, app: &mut PdfCraftApp, _t: &Tokens) -> (bool, bool) {
    let Some((i, id)) = app.active_ids() else { return (false, true) };
    let current = app.views.get(i).map_or(0, |v| v.current);
    let count = app.session.get(id).map_or(1, |d| d.info.pages.len().max(1));
    let unit = app.units;
    let d = &mut app.white_margins_draft;
    ui.label(egui::RichText::new(tl!("Remove White Margins")).font(theme::semibold(18.0)));
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(tl!("Crops each page to where its content is. The pages are not changed otherwise, and Undo puts the margins back."))
            .small(),
    );
    ui.add_space(8.0);
    egui::Grid::new("white-margins-grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        ui.label(tl!("Pages"));
        ui.vertical(|ui| {
            widgets::radio_value(ui, &mut d.range, Range::All, tl!("All"));
            widgets::radio_value(ui, &mut d.range, Range::Current, crate::i18n::fmt(tl!("Current page ({n})"), &[("n", &(current + 1).to_string())]));
            let (mut a, mut b) = match d.range {
                Range::Pages(a, b) => (a, b),
                _ => (1, count),
            };
            ui.horizontal(|ui| {
                let on = matches!(d.range, Range::Pages(..));
                let from = widgets::radio(on, tl!("From"));
                if ui.add(from).clicked() {
                    d.range = Range::Pages(a, b);
                }
                let ra = ui.add(egui::DragValue::new(&mut a).range(1..=count));
                ui.label(tl!("to"));
                let rb = ui.add(egui::DragValue::new(&mut b).range(1..=count));
                if ra.changed() || rb.changed() {
                    d.range = Range::Pages(a.min(b), b.max(a));
                }
                ui.label(crate::i18n::fmt(tl!("of {name}"), &[("name", &count.to_string())]));
            });
        });
        ui.end_row();
        let label = ui.label(tl!("Keep around the content"));
        let k = unit.per_point();
        let mut v = d.padding * k;
        let changed = ui
            .add(egui::DragValue::new(&mut v).speed(unit.speed()).range(0.0..=MAX_PADDING * k).max_decimals(3).suffix(format!(" {}", unit.label())))
            .labelled_by(label.id)
            .changed();
        if changed {
            d.padding = (v / k).clamp(0.0, MAX_PADDING);
        }
        ui.end_row();
    });
    ui.add_space(12.0);
    let (mut go, mut cancel) = (false, false);
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if widgets::pill_button(ui, tl!("Remove"), true).clicked() {
            go = true;
        }
        if widgets::pill_button(ui, tl!("Cancel"), false).clicked() {
            cancel = true;
        }
    });
    (go, cancel)
}

/// What a finished run says: the pages cropped, left alone because they are blank, already
/// tight, or couldn't be measured.
pub(crate) fn summary(r: &WhiteMarginsReport) -> String {
    let lang = crate::i18n::current();
    let mut out = if r.cropped.is_empty() {
        tl!("No pages needed cropping.").to_string()
    } else {
        crate::i18n::trn(lang, r.cropped.len() as u64, "Cropped {n} page to its content.", "Cropped {n} pages to their content.")
    };
    if !r.blank.is_empty() {
        out.push(' ');
        out.push_str(&crate::i18n::trn(lang, r.blank.len() as u64, "{n} blank page was left alone.", "{n} blank pages were left alone."));
    }
    if !r.unchanged.is_empty() {
        out.push(' ');
        out.push_str(&crate::i18n::trn(lang, r.unchanged.len() as u64, "{n} page was already tight.", "{n} pages were already tight."));
    }
    if let Some((page, why)) = r.failed.first() {
        out.push(' ');
        // The reason is the engine's own sentence, in English.
        out.push_str(&crate::i18n::fmt(
            &crate::i18n::trn(
                lang,
                r.failed.len() as u64,
                "{n} page couldn't be measured and was left alone (page {p}: {why}).",
                "{n} pages couldn't be measured and were left alone (page {p}: {why}, …).",
            ),
            &[("p", &page.saturating_add(1).to_string()), ("why", why)],
        ));
    }
    out
}

impl PdfCraftApp {
    /// Remove the white margins of the pages chosen in the dialog, measuring in the background.
    pub fn start_white_margins(&mut self) {
        let Some((vi, id)) = self.active_ids() else { return };
        if self.white_margins_run.is_some() {
            self.notify_tr("White margins are already being removed");
            return;
        }
        // The jobs share the progress card.
        if self.optimize_run.is_some() {
            self.notify_tr("Optimization is already running");
            return;
        }
        if self.scan_clean_run.is_some() {
            self.notify_tr("Scanned pages are already being enhanced");
            return;
        }
        // What's typed in a form field is part of the document (#166).
        if !self.commit_form_typing() {
            return;
        }
        let d = self.white_margins_draft.clone();
        if let Some(problem) = d.problem(self.units) {
            self.notify(problem);
            return;
        }
        let current = self.views.get(vi).map_or(0, |v| v.current);
        let count = self.session.get(id).map_or(0, |doc| doc.info.pages.len());
        let pages = d.pages(current, count);
        if pages.is_empty() {
            self.notify_tr("The document has no pages");
            return;
        }
        let job = match self.session.white_margins_job(id, &pages, d.padding) {
            Ok(job) => job,
            Err(e) => return self.notify(e.to_string()),
        };
        let progress = Arc::new(Mutex::new(WhiteMarginsProgress { total: job.total(), ..Default::default() }));
        let p = progress.clone();
        let work = move || {
            let result = job.run(|done, total| {
                let Ok(mut s) = p.lock() else { return false };
                s.done = done;
                s.total = total;
                !s.cancel
            });
            if let Ok(mut s) = p.lock() {
                s.result = Some(result);
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        if self.run_inline {
            work();
        } else if let Err(e) = std::thread::Builder::new().name("pdfcraft-white-margins".into()).spawn(work) {
            // No worker: report the failure instead of waiting for a result that never comes.
            if let Ok(mut s) = progress.lock() {
                s.result = Some(Err(EditError::Invalid(e.to_string())));
            }
        }
        #[cfg(target_arch = "wasm32")]
        work();
        self.white_margins_run = Some(WhiteMarginsRun { doc: id, progress });
        self.poll_white_margins();
    }

    /// Stop a running measurement (nothing changes).
    pub fn cancel_white_margins(&mut self) {
        if let Some(r) = &self.white_margins_run
            && let Ok(mut s) = r.progress.lock()
        {
            s.cancel = true;
        }
    }

    /// Show progress; apply the result once the worker is done.
    pub(crate) fn poll_white_margins(&mut self) {
        let Some(run) = self.white_margins_run.as_ref() else { return };
        let (doc, progress) = (run.doc, run.progress.clone());
        if self.session.get(doc).is_none() {
            // The document was closed: stop measuring it and drop what it finds.
            if let Ok(mut s) = progress.lock() {
                s.cancel = true;
            }
            self.white_margins_run = None;
            self.progress_notice = None;
            return;
        }
        let Ok(mut s) = progress.lock() else {
            // The worker panicked while holding the lock: it will never report back.
            self.white_margins_run = None;
            self.progress_notice = None;
            self.notify_tr("Couldn't remove the white margins");
            return;
        };
        // A result waiting for its document to come forward can still be cancelled.
        if s.cancel && matches!(s.result, Some(Ok(_))) {
            drop(s);
            self.white_margins_run = None;
            self.progress_notice = None;
            self.notify_tr("Removing the white margins was cancelled");
            return;
        }
        let Some(result) = s.result.take() else {
            let (cancelling, done, total) = (s.cancel, s.done, s.total);
            drop(s);
            let label = if cancelling {
                tl!("Cancelling…").to_string()
            } else {
                crate::i18n::fmt(
                    tl!("Measuring margins… page {d} of {t}"),
                    &[("d", &(done + 1).min(total.max(1)).to_string()), ("t", &total.max(1).to_string())],
                )
            };
            let fraction = if total == 0 { 1.0 } else { done.min(total) as f32 / total as f32 };
            self.progress_notice = Some(crate::widgets::ProgressNotice { label, fraction, cancellable: !cancelling });
            if let Some(ctx) = &self.ctx {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
            return;
        };
        drop(s);
        self.white_margins_run = None;
        self.progress_notice = None;
        match result {
            Ok(measured) => {
                // Only for the document as it was measured. (The generation moves with every
                // edit, and also with toggles that don't touch the pages, such as a view change
                // or a save; those make the result stale too, which is the safe side.)
                let unchanged = self.session.get(doc).map(|d| d.edit_generation()) == Some(measured.generation());
                if !unchanged {
                    self.notify_tr("The document changed while its margins were being measured, so nothing was changed. Try again.");
                    return;
                }
                // The crop boxes go in through the view in front, so another tab's result
                // waits until its document is.
                if self.active_ids().map(|(_, active)| active) != Some(doc) {
                    if let Ok(mut s) = progress.lock() {
                        s.result = Some(Ok(measured));
                    }
                    self.white_margins_run = Some(WhiteMarginsRun { doc, progress });
                    self.progress_notice = Some(crate::widgets::ProgressNotice {
                        label: tl!("Margins measured. They are applied when their document is in front.").to_string(),
                        fraction: 1.0,
                        cancellable: true,
                    });
                    return;
                }
                if let Some(edit) = measured.edit()
                    && !self.apply_edit(edit)
                {
                    // The edit's own message is showing.
                    return;
                }
                let message = summary(&measured.report);
                self.notify(message);
            }
            Err(EditError::Cancelled) => self.notify_tr("Removing the white margins was cancelled"),
            Err(e) => self.notify(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::Unit;

    #[test]
    fn the_summary_says_what_was_cropped_and_what_was_left() {
        let r = WhiteMarginsReport {
            cropped: vec![(0, [0.0; 4]), (1, [0.0; 4])],
            unchanged: vec![2],
            blank: vec![3, 4],
            failed: vec![(5, "the page has no size".into())],
        };
        assert_eq!(
            summary(&r),
            "Cropped 2 pages to their content. 2 blank pages were left alone. 1 page was already tight. 1 page couldn't be measured and was left alone (page 6: the page has no size)."
        );
        assert_eq!(summary(&WhiteMarginsReport::default()), "No pages needed cropping.");
    }

    #[test]
    fn the_pages_follow_the_range_and_are_never_empty() {
        let mut d = WhiteMarginsDraft::default();
        assert_eq!(d.pages(1, 3), [0, 1, 2]);
        d.range = Range::Current;
        assert_eq!(d.pages(9, 3), [2], "a stale current page is clamped");
        d.range = Range::Pages(2, 3);
        assert_eq!(d.pages(0, 5), [1, 2]);
        d.range = Range::Pages(3, 2);
        assert_eq!(d.pages(0, 5), [1, 2], "a reversed range is put in order");
        d.range = Range::Pages(8, 9);
        assert_eq!(d.pages(0, 3), [2], "a range past the end is the last page, never all of them");
        d.range = Range::Pages(0, 0);
        assert_eq!(d.pages(0, 3), [0]);
    }

    #[test]
    fn the_padding_is_validated() {
        let mut d = WhiteMarginsDraft::default();
        assert!(d.problem(Unit::Points).is_none());
        for bad in [-1.0, f64::NAN, f64::INFINITY, MAX_PADDING + 1.0] {
            d.padding = bad;
            assert!(d.problem(Unit::Points).is_some(), "{bad}");
        }
        d.padding = MAX_PADDING;
        assert!(d.problem(Unit::Points).is_none());
        d.padding = -1.0;
        assert_eq!(d.problem(Unit::Inches).as_deref(), Some("The padding must be between 0 and 13.889 in"));
    }

    /// Three 300 × 400 pages: a 50 × 50 square on the first, nothing on the second, and a
    /// square covering the whole third.
    fn fixture() -> Vec<u8> {
        let streams = ["0 g 100 100 50 50 re f", "", "0 g 0 0 300 400 re f"];
        let mut pdf = String::from("%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n");
        pdf.push_str("2 0 obj << /Type /Pages /Kids [3 0 R 5 0 R 7 0 R] /Count 3 /MediaBox [0 0 300 400] >> endobj\n");
        for (n, content) in streams.iter().enumerate() {
            let (page, stream) = (3 + 2 * n, 4 + 2 * n);
            pdf.push_str(&format!("{page} 0 obj << /Type /Page /Parent 2 0 R /Contents {stream} 0 R >> endobj\n"));
            pdf.push_str(&format!("{stream} 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n", content.len() + 1));
        }
        pdf.push_str("trailer << /Root 1 0 R >>\n%%EOF");
        pdf.into_bytes()
    }

    fn app() -> PdfCraftApp {
        let mut app = PdfCraftApp::new();
        app.set_option("language", "en").unwrap();
        app.run_inline = true;
        app.open_bytes("margins.pdf", None, fixture()).unwrap();
        app
    }

    fn sizes(app: &PdfCraftApp) -> Vec<(f32, f32)> {
        let (_, id) = app.active_ids().unwrap();
        app.session.get(id).unwrap().info.pages.iter().map(|p| (p.width, p.height)).collect()
    }

    #[test]
    fn the_run_crops_the_pages_as_one_undoable_step_and_says_what_it_did() {
        let mut app = app();
        assert_eq!(sizes(&app), [(300.0, 400.0); 3]);
        app.start_white_margins();
        assert!(app.white_margins_run.is_none() && app.progress_notice.is_none(), "inline: done");
        let after = sizes(&app);
        assert!((after[0].0 - 50.0).abs() < 2.0 && (after[0].1 - 50.0).abs() < 2.0, "{after:?}");
        assert_eq!(&after[1..], [(300.0, 400.0); 2], "a blank page and a full page stay");
        assert_eq!(
            app.toast.as_ref().map(|t| t.0.as_str()),
            Some("Cropped 1 page to its content. 1 blank page was left alone. 1 page was already tight.")
        );
        let (_, id) = app.active_ids().unwrap();
        assert_eq!(app.session.get(id).unwrap().can_undo(), Some("Remove white margins"));
        app.undo();
        assert_eq!(sizes(&app), [(300.0, 400.0); 3], "one undo puts every page back");
    }

    #[test]
    fn the_chosen_pages_and_padding_are_used() {
        let mut app = app();
        app.white_margins_draft = WhiteMarginsDraft { range: Range::Pages(2, 3), padding: 10.0 };
        app.start_white_margins();
        assert_eq!(sizes(&app), [(300.0, 400.0); 3], "page 1 was not chosen");
        app.white_margins_draft = WhiteMarginsDraft { range: Range::Current, padding: 10.0 };
        app.start_white_margins();
        let first = sizes(&app)[0];
        assert!((first.0 - 70.0).abs() < 2.0 && (first.1 - 70.0).abs() < 2.0, "{first:?}");
        // A padding that is not allowed is refused with the reason, and nothing changes.
        app.undo();
        app.units = crate::units::Unit::Points;
        app.white_margins_draft.padding = -3.0;
        app.start_white_margins();
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("The padding must be between 0 and 1000 pt"));
        assert_eq!(sizes(&app), [(300.0, 400.0); 3]);
    }

    /// A finished measurement of the active document, waiting to be taken in.
    fn waiting(app: &mut PdfCraftApp) -> DocId {
        let (_, id) = app.active_ids().unwrap();
        let measured = app.session.white_margins_job(id, &[], 0.0).unwrap().run(|_, _| true).unwrap();
        let progress = Arc::new(Mutex::new(WhiteMarginsProgress { result: Some(Ok(measured)), ..Default::default() }));
        app.white_margins_run = Some(WhiteMarginsRun { doc: id, progress });
        id
    }

    #[test]
    fn a_result_is_refused_when_the_document_was_edited_while_it_ran() {
        let mut app = app();
        let id = waiting(&mut app);
        app.session.apply(id, pdfcraft_engine::Edit::RotatePages { pages: vec![0], degrees: 90 }).unwrap();
        let before = sizes(&app);
        app.poll_white_margins();
        assert!(app.white_margins_run.is_none() && app.progress_notice.is_none());
        assert_eq!(sizes(&app), before, "nothing was cropped");
        assert_eq!(
            app.toast.as_ref().map(|t| t.0.as_str()),
            Some("The document changed while its margins were being measured, so nothing was changed. Try again.")
        );
    }

    #[test]
    fn a_result_waits_for_its_document_to_come_forward_and_then_applies() {
        let mut app = app();
        let id = waiting(&mut app);
        app.open_bytes("other.pdf", None, fixture()).unwrap();
        assert_ne!(app.active_ids().map(|(_, a)| a), Some(id));
        app.poll_white_margins();
        assert!(app.white_margins_run.is_some(), "kept, not dropped");
        assert!(app.progress_notice.as_ref().is_some_and(|p| p.label.starts_with("Margins measured")));
        let widths = |app: &PdfCraftApp| app.session.get(id).unwrap().info.pages.iter().map(|p| p.width).collect::<Vec<_>>();
        assert_eq!(widths(&app), [300.0; 3], "nothing is applied to another tab");
        let index = app.views.iter().position(|v| v.id == id).unwrap();
        app.active = Some(index);
        app.poll_white_margins();
        assert!(app.white_margins_run.is_none() && app.progress_notice.is_none());
        assert!((widths(&app)[0] - 50.0).abs() < 2.0, "{:?}", widths(&app));
    }

    #[test]
    fn a_waiting_result_can_be_cancelled() {
        let mut app = app();
        let id = waiting(&mut app);
        app.open_bytes("other.pdf", None, fixture()).unwrap();
        app.poll_white_margins();
        app.cancel_white_margins();
        app.poll_white_margins();
        assert!(app.white_margins_run.is_none() && app.progress_notice.is_none());
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("Removing the white margins was cancelled"));
        assert_eq!(app.session.get(id).unwrap().info.pages[0].width, 300.0);
    }

    #[test]
    fn closing_the_document_stops_the_run_quietly() {
        let mut app = app();
        let (index, id) = app.active_ids().unwrap();
        let progress = Arc::new(Mutex::new(WhiteMarginsProgress { done: 1, total: 3, ..Default::default() }));
        app.white_margins_run = Some(WhiteMarginsRun { doc: id, progress: progress.clone() });
        app.poll_white_margins();
        assert!(app.progress_notice.is_some());
        app.toast = None;
        app.close_tab(index);
        app.poll_white_margins();
        assert!(app.white_margins_run.is_none() && app.progress_notice.is_none());
        assert!(progress.lock().unwrap().cancel, "the worker is told to stop");
        assert!(app.toast.is_none(), "nothing to say about a document that is gone");
    }

    /// Poll until the run is over, for at most ten seconds.
    fn poll_until_done(app: &mut PdfCraftApp) {
        let start = std::time::Instant::now();
        while app.white_margins_run.is_some() {
            assert!(start.elapsed() < std::time::Duration::from_secs(10), "the worker did not finish");
            std::thread::sleep(std::time::Duration::from_millis(5));
            app.poll_white_margins();
        }
    }

    #[test]
    fn the_real_worker_thread_measures_and_the_result_is_applied() {
        let mut app = app();
        app.run_inline = false;
        app.start_white_margins();
        poll_until_done(&mut app);
        let after = sizes(&app);
        assert!((after[0].0 - 50.0).abs() < 2.0, "{after:?}");
        assert!(app.toast.as_ref().is_some_and(|t| t.0.starts_with("Cropped 1 page")));
        assert!(app.progress_notice.is_none());
    }

    #[test]
    fn cancel_on_the_real_worker_thread_changes_nothing() {
        let mut app = app();
        let (_, id) = app.active_ids().unwrap();
        // Cancelled before the first page, so the outcome doesn't depend on timing.
        let job = app.session.white_margins_job(id, &[], 0.0).unwrap();
        let progress = Arc::new(Mutex::new(WhiteMarginsProgress { total: job.total(), cancel: true, ..Default::default() }));
        let p = progress.clone();
        let worker = std::thread::Builder::new()
            .name("pdfcraft-white-margins-test".into())
            .spawn(move || {
                let result = job.run(|done, total| {
                    let Ok(mut s) = p.lock() else { return false };
                    s.done = done;
                    s.total = total;
                    !s.cancel
                });
                if let Ok(mut s) = p.lock() {
                    s.result = Some(result);
                }
            })
            .unwrap();
        app.white_margins_run = Some(WhiteMarginsRun { doc: id, progress });
        poll_until_done(&mut app);
        worker.join().unwrap();
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("Removing the white margins was cancelled"));
        assert_eq!(sizes(&app), [(300.0, 400.0); 3]);
    }

    #[test]
    fn progress_shows_while_the_worker_runs_and_cancel_is_reported() {
        let mut app = app();
        let (_, id) = app.active_ids().unwrap();
        let progress = Arc::new(Mutex::new(WhiteMarginsProgress { done: 1, total: 3, ..Default::default() }));
        app.white_margins_run = Some(WhiteMarginsRun { doc: id, progress: progress.clone() });
        app.poll_white_margins();
        let shown = app.progress_notice.clone().unwrap();
        assert_eq!(shown.label, "Measuring margins… page 2 of 3");
        assert!((shown.fraction - 1.0 / 3.0).abs() < 1e-4 && shown.cancellable);
        // While it runs a second one is refused.
        app.start_white_margins();
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("White margins are already being removed"));
        app.cancel_white_margins();
        app.poll_white_margins();
        assert_eq!(app.progress_notice.as_ref().map(|p| p.label.as_str()), Some("Cancelling…"));
        progress.lock().unwrap().result = Some(Err(EditError::Cancelled));
        app.poll_white_margins();
        assert!(app.white_margins_run.is_none() && app.progress_notice.is_none());
        assert_eq!(app.toast.as_ref().map(|t| t.0.as_str()), Some("Removing the white margins was cancelled"));
        assert_eq!(sizes(&app), [(300.0, 400.0); 3]);
    }
}
