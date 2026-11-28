//! Print production ▸ Flatten transparency: the dialog (pages, resolution) and the background
//! run with its progress card. The pages are drawn on the host's job queue
//! ([`pdfcraft_engine::flatten_transparency`]); the result is applied as one undoable step when
//! it is done, unless the document changed meanwhile.

use std::sync::Arc;

use craft::pages::PageRange;
use craft::pages::ui::{PageRangeChoices, PageRangeOptions};
use craft::ui::dialog::{DialogResult, dialog_id};
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
use pdfcraft_engine::flatten_transparency::{DEFAULT_DPI, FlattenReport, Flattened};
use pdfcraft_engine::{DocId, EditError};

use super::job_count::JobCount;
use crate::PdfCraftApp;
use crate::notices::ProgressNotice;
use crate::notices::{self, ProgressKey};

/// The key the dialog and its result share.
const DIALOG_KEY: &str = "prepress.flatten_transparency";

/// The resolutions offered, in pixels per inch.
const RESOLUTIONS: [u32; 3] = [150, 300, 600];

/// The dialog's choices (kept between runs).
#[derive(Clone, Debug, PartialEq)]
pub struct FlattenDraft {
    pub range: PageRange,
    pub dpi: f64,
}

impl Default for FlattenDraft {
    fn default() -> Self {
        Self { range: PageRange::default(), dpi: DEFAULT_DPI }
    }
}

/// A running flatten: the count the job and the progress card share.
pub struct FlattenRun {
    pub progress: Arc<JobCount>,
}

fn params(count: usize, current: usize) -> Vec<Param> {
    let resolutions = RESOLUTIONS.iter().map(|r| (r.to_string(), crate::i18n::fmt(tl!("{n} ppi"), &[("n", &r.to_string())]).into())).collect();
    vec![
        Param::read_only_text(
            "note",
            String::new(),
            tl!("Each page that uses transparency becomes a picture: its text can no longer be selected or searched, hidden layers are dropped and its tags are lost. Comments and form fields stay."),
        )
        .full_width(),
        Param::new("dpi", tl!("Resolution"), ParamKind::radio(resolutions, Some("300".to_string()))),
        PageRangeOptions::param("pages", tl!("Pages"), count, current, PageRangeChoices::CurrentFromTo),
    ]
}

/// What a finished run says.
fn summary(r: &FlattenReport) -> String {
    let lang = crate::i18n::current();
    let mut out = if r.pages.is_empty() {
        tl!("No pages were flattened.").to_string()
    } else {
        crate::i18n::trn(lang, r.pages.len() as u64, "Flattened {n} page.", "Flattened {n} pages.")
    };
    if !r.opaque.is_empty() {
        out.push(' ');
        out.push_str(&crate::i18n::trn(lang, r.opaque.len() as u64, "{n} page uses no transparency.", "{n} pages use no transparency."));
    }
    if let Some(first) = r.failed.first() {
        out.push(' ');
        out.push_str(&crate::i18n::fmt(
            &crate::i18n::trn(
                lang,
                r.failed.len() as u64,
                "{n} page could not be drawn and was left as it was (page {p}).",
                "{n} pages could not be drawn and were left as they were (page {p}, …).",
            ),
            &[("p", &(first + 1).to_string())],
        ));
    }
    out
}

impl PdfCraftApp {
    /// Opens the Flatten transparency dialog over the remembered choices.
    pub(crate) fn open_flatten_transparency(&mut self, ctx: &egui::Context) {
        let Some((vi, id)) = self.active_ids() else { return };
        let count = self.session.get(id).map_or(1, |d| d.info.pages.len().max(1));
        let current = self.views.get(vi).map_or(0, |v| v.current());
        let draft = &self.flatten_draft;
        let fields = DialogFields::from_pairs([("dpi", format!("{:.0}", draft.dpi).into()), ("pages", PageRangeOptions::field_value(&draft.range))]);
        let dialog = HostedParamDialog::new(ctx, DIALOG_KEY, fields, move |fields| {
            let ready = !PageRangeOptions::value(fields, "pages").unwrap_or_default().pages(count, current, &[]).is_empty();
            ParamDialog::new(DIALOG_KEY, tl!("Flatten transparency"), params(count, current)).ok_label(tl!("Flatten").into()).ok_enabled(ready)
        });
        self.dialogs.open(dialog);
    }

    /// Once per frame: takes the dialog's answer, remembers it and starts the run.
    pub(crate) fn flatten_transparency_pass(&mut self, ctx: &egui::Context) {
        let Some(DialogResult::Ok(answer)) = self.dialogs.take_by_id::<HostedParamDialog>(dialog_id(DIALOG_KEY)) else { return };
        let d = &mut self.flatten_draft;
        if let Some(range) = PageRangeOptions::value(&answer.fields, "pages") {
            d.range = range;
        }
        if let Some(dpi) = answer.fields.text("dpi").and_then(|t| t.parse().ok()) {
            d.dpi = dpi;
        }
        self.start_flatten_transparency(ctx);
    }

    /// Flatten the pages chosen in the dialog, in the background.
    pub(crate) fn start_flatten_transparency(&mut self, ctx: &egui::Context) {
        let Some((vi, id)) = self.active_ids() else { return };
        if self.flatten_run.is_some() {
            self.notify_tr("Pages are already being flattened");
            return;
        }
        // What's typed in a form field is part of the document (#166).
        if !self.commit_form_typing(ctx) {
            return;
        }
        let count = self.session.get(id).map_or(0, |d| d.info.pages.len());
        let current = self.views.get(vi).map_or(0, |v| v.current());
        let pages = self.flatten_draft.range.pages(count, current, &[]);
        // The engine reads an empty list as every page, so a range that picks none must not reach it.
        if pages.is_empty() {
            self.notify_tr("There are no pages in that range");
            return;
        }
        let job = match self.session.flatten_transparency_job(id, &pages, self.flatten_draft.dpi) {
            Ok(job) => job,
            Err(e) => return self.notify(e.to_string()),
        };
        let progress = Arc::new(JobCount::default());
        progress.set(0, job.total());
        let (count, wake) = (progress.clone(), self.wake.clone());
        self.flatten_jobs.submit(id, Default::default(), move |_| {
            pdfcraft_engine::guard(|| {
                job.run(|done, total| {
                    count.set(done, total);
                    wake.wake();
                    !count.cancelled()
                })
            })
            .unwrap_or_else(|message| Err(EditError::Invalid(message)))
        });
        self.flatten_run = Some(FlattenRun { progress });
        self.post_flatten_progress();
    }

    /// Stop a running flatten (nothing changes).
    pub(crate) fn cancel_flatten_transparency(&mut self) {
        let Some(run) = &self.flatten_run else { return };
        run.progress.cancel();
        self.post_flatten_progress();
    }

    /// Shows the run's progress card.
    fn post_flatten_progress(&mut self) {
        let Some(run) = &self.flatten_run else { return };
        let (done, total) = run.progress.read();
        let cancelling = run.progress.cancelled();
        let label = if cancelling {
            tl!("Cancelling…").to_string()
        } else {
            crate::i18n::fmt(
                tl!("Flattening… page {d} of {t}"),
                &[("d", &(done + 1).min(total.max(1)).to_string()), ("t", &total.max(1).to_string())],
            )
        };
        let fraction = if total == 0 { 1.0 } else { done.min(total) as f32 / total as f32 };
        notices::set_progress(self, ProgressKey::Flatten, ProgressNotice { label, fraction, cancellable: !cancelling });
    }

    /// Show progress; apply the result once the job is done.
    pub(crate) fn poll_flatten_transparency(&mut self) {
        let Some(run) = self.flatten_run.as_ref() else { return };
        let progress = run.progress.clone();
        let Some(finished) = self.flatten_jobs.try_recv() else {
            if progress.take_change().is_some() {
                self.post_flatten_progress();
            }
            return;
        };
        notices::end_progress(self, ProgressKey::Flatten);
        self.flatten_run = None;
        self.land_flatten(finished.key, finished.result);
    }

    /// Applies a finished run to `doc` as one undoable step and says what it did.
    fn land_flatten(&mut self, doc: DocId, result: Result<Flattened, EditError>) {
        match result.and_then(|flat| self.session.apply_flatten_transparency(doc, flat)) {
            Ok(report) => {
                // The page rasters, tiles and thumbnails show the old content at the same scale.
                for v in self.views.iter_mut().filter(|v| v.id == doc) {
                    v.invalidate_content();
                }
                let msg = summary(&report);
                self.notify(msg);
            }
            Err(EditError::Cancelled) => self.notify_tr("Flattening was cancelled"),
            Err(e) => self.notify(e.to_string()),
        }
    }
}
