//! Pages ▸ Remove white margins (`page.remove_white_margins`): the dialog (the pages, and the padding to keep) and the
//! background run with its progress card. Finding the content of each page takes a raster of it, so the measuring runs
//! on the host's job queue ([`pdfcraft_engine::white_margins`]), page by page, and can be cancelled between pages. The
//! new crop boxes are applied as one undoable step once it is done, unless the document changed meanwhile.

use std::sync::Arc;

use craft::pages::PageRange;
use craft::pages::ui::{PageRangeChoices, PageRangeOptions};
use craft::ui::contract::Bind;
use craft::ui::dialog::dialog_id;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog};
use craft::ui::runtime::env::Label;
use pdfcraft_engine::white_margins::{MAX_PADDING, WhiteMargins, WhiteMarginsReport};
use pdfcraft_engine::{DocId, EditError};

use super::job_count::JobCount;
use crate::PdfCraftApp;
use crate::notices::{self, ProgressKey, ProgressNotice};
use crate::pageboxes::unit_kind;
use crate::units::Unit;

/// The dialog's key.
const KEY: &str = "page.remove_white_margins";

/// The dialog's choices, kept while the app runs; the pages are re-checked against each document.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct WhiteMarginsDraft {
    pub range: PageRange,
    /// Points to keep around the content; the dialog shows them in the Preferences unit.
    pub padding: f64,
}

/// A running measurement: its document, the count the job and the progress card share, and the result once measured
/// while its document is not in front.
pub struct WhiteMarginsRun {
    pub doc: DocId,
    pub progress: Arc<JobCount>,
    pub(crate) waiting: Option<WhiteMargins>,
}

/// The rows of the dialog for a document of `count` pages with `current` on screen, the padding in `unit`.
fn params(count: usize, current: usize, unit: Unit) -> Vec<Param> {
    let k = unit.per_point();
    let decimals = unit.decimals().max(3) as u32;
    vec![
        Param::read_only_text(
            "note",
            "",
            tl!("Crops each page to where its content is. The pages are not changed otherwise, and Undo puts the margins back."),
        )
        .full_width(),
        PageRangeOptions::param("range", tl!("Pages"), count, current, PageRangeChoices::CurrentFromTo),
        Param::number("padding", tl!("Keep around the content"), 0.0, 0.001 * k, decimals)
            .bounds(Some(0.0), Some(MAX_PADDING * k))
            .unit(unit_kind(unit)),
    ]
}

/// What a finished run says: the pages cropped, left alone because they are blank, already tight, or couldn't be
/// measured.
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
    /// `page.remove_white_margins`: opens the dialog over the remembered choices.
    pub(crate) fn open_white_margins(&mut self, ctx: &egui::Context) {
        let Some((i, id)) = self.active_ids() else { return };
        let count = self.session.get(id).map_or(1, |d| d.info.pages.len().max(1));
        let current = self.views.get(i).map_or(0, |v| v.current());
        let unit = self.units;
        let d = &self.white_margins_draft;
        let mut fields = DialogFields::new();
        fields.set("range", PageRangeOptions::field_value(&d.range));
        fields.set("padding", d.padding * unit.per_point());
        let dialog = HostedParamDialog::new(ctx, KEY, fields, move |fields| {
            let pages = PageRangeOptions::value(fields, "range").unwrap_or_default().pages(count, current, &[]);
            ParamDialog::new(KEY, tl!("Remove white margins"), params(count, current, unit))
                .values(Bind::bound(fields))
                .ok_label(Label::from(tl!("Remove")))
                .ok_enabled(!pages.is_empty())
        });
        crate::dialogs::host::open(self, dialog);
    }

    /// Takes the dialog's answer, remembers it and starts the run.
    pub(crate) fn white_margins_pass(&mut self, ctx: &egui::Context) {
        let Some(answer) = self.dialogs.take_by_id::<HostedParamDialog>(dialog_id(KEY)) else { return };
        let Some(result) = answer.ok() else { return };
        let per_point = self.units.per_point();
        let d = &mut self.white_margins_draft;
        if let Some(range) = PageRangeOptions::value(&result.fields, "range") {
            d.range = range;
        }
        if let Some(v) = result.fields.number("padding") {
            d.padding = v / per_point;
        }
        self.start_white_margins(ctx);
    }

    /// Remove the white margins of the pages chosen in the dialog, measuring in the background.
    pub fn start_white_margins(&mut self, ctx: &egui::Context) {
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
        if !self.commit_form_typing(ctx) {
            return;
        }
        let d = self.white_margins_draft.clone();
        if !d.padding.is_finite() || !(0.0..=MAX_PADDING).contains(&d.padding) {
            let k = self.units.per_point();
            let max = format!("{} {}", (MAX_PADDING * k * 1000.0).round() / 1000.0, self.units.label());
            self.notify_fmt("The padding must be between 0 and {max}", &[("max", &max)]);
            return;
        }
        let current = self.views.get(vi).map_or(0, |v| v.current());
        let count = self.session.get(id).map_or(0, |doc| doc.info.pages.len());
        let pages = d.range.pages(count, current, &[]);
        // The engine reads an empty list as every page, so a range that picks none must not reach it.
        if pages.is_empty() {
            self.notify_tr("There are no pages in that range");
            return;
        }
        let job = match self.session.white_margins_job(id, &pages, d.padding) {
            Ok(job) => job,
            Err(e) => return self.notify(e.to_string()),
        };
        // A result nobody took (its document was closed) is not this run's.
        while self.white_margins_jobs.try_recv().is_some() {}
        let progress = Arc::new(JobCount::default());
        progress.set(0, job.total());
        let (count, wake) = (progress.clone(), self.wake.clone());
        self.white_margins_jobs.submit(id, Default::default(), move |_| {
            job.run(|done, total| {
                count.set(done, total);
                wake.wake();
                !count.cancelled()
            })
        });
        self.white_margins_run = Some(WhiteMarginsRun { doc: id, progress, waiting: None });
        self.post_white_margins_progress();
    }

    /// Stop a running measurement (nothing changes).
    pub fn cancel_white_margins(&mut self) {
        let Some(run) = &self.white_margins_run else { return };
        run.progress.cancel();
        self.post_white_margins_progress();
    }

    /// Shows the run's progress card.
    fn post_white_margins_progress(&mut self) {
        let Some(run) = &self.white_margins_run else { return };
        let (done, total) = run.progress.read();
        let cancelling = run.progress.cancelled();
        let (label, fraction) = if run.waiting.is_some() {
            (tl!("Margins measured. They are applied when their document is in front.").to_string(), 1.0)
        } else if cancelling {
            (tl!("Cancelling…").to_string(), 0.0)
        } else {
            let label = crate::i18n::fmt(
                tl!("Measuring margins… page {d} of {t}"),
                &[("d", &(done + 1).min(total.max(1)).to_string()), ("t", &total.max(1).to_string())],
            );
            (label, if total == 0 { 1.0 } else { done.min(total) as f32 / total as f32 })
        };
        let notice = ProgressNotice { label, fraction, cancellable: run.waiting.is_some() || !cancelling };
        notices::set_progress(self, ProgressKey::WhiteMargins, notice);
    }

    /// Ends the run and its card.
    fn end_white_margins(&mut self) {
        self.white_margins_run = None;
        notices::end_progress(self, ProgressKey::WhiteMargins);
    }

    /// Show progress; apply the result once the job is done and its document is in front.
    pub(crate) fn poll_white_margins(&mut self, ctx: &egui::Context) {
        let Some(run) = self.white_margins_run.as_ref() else { return };
        let (doc, progress) = (run.doc, run.progress.clone());
        if self.session.get(doc).is_none() {
            // The document was closed: stop measuring it and drop what it finds.
            progress.cancel();
            self.end_white_margins();
            return;
        }
        if run.waiting.is_none() {
            let Some(finished) = self.white_margins_jobs.try_recv() else {
                if progress.take_change().is_some() {
                    self.post_white_margins_progress();
                }
                return;
            };
            if finished.key != doc {
                return;
            }
            match finished.result {
                Ok(measured) => {
                    if let Some(run) = self.white_margins_run.as_mut() {
                        run.waiting = Some(measured);
                    }
                    // Says it is measured, whether it lands now or when its document is in front.
                    self.post_white_margins_progress();
                }
                Err(e) => {
                    self.end_white_margins();
                    match e {
                        EditError::Cancelled => self.notify_tr("Removing the white margins was cancelled"),
                        e => self.notify(e.to_string()),
                    }
                    return;
                }
            }
        }
        // A result waiting for its document to come forward can still be cancelled.
        if progress.cancelled() {
            self.end_white_margins();
            self.notify_tr("Removing the white margins was cancelled");
            return;
        }
        // Only for the document as it was measured. (The generation moves with every edit, and also with changes that
        // don't touch the pages, such as a save; those make the result stale too, which is the safe side.)
        let stale = self
            .white_margins_run
            .as_ref()
            .and_then(|r| r.waiting.as_ref())
            .zip(self.session.get(doc))
            .map(|(m, d)| d.edit_generation() != m.generation());
        if stale == Some(true) {
            self.end_white_margins();
            self.notify_tr("The document changed while its margins were being measured, so nothing was changed. Try again.");
            return;
        }
        // The crop boxes go in through the view in front, so another tab's result waits until its document is.
        if self.active_ids().map(|(_, active)| active) != Some(doc) {
            return;
        }
        let Some(measured) = self.white_margins_run.as_mut().and_then(|r| r.waiting.take()) else { return };
        self.end_white_margins();
        if let Some(edit) = measured.edit()
            && !self.apply_edit(ctx, edit)
        {
            // The edit's own message is showing.
            return;
        }
        self.notify(summary(&measured.report));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
