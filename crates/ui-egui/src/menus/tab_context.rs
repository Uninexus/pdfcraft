//! PDF-tab actions. Closed history contains paths only, never bytes or passwords.
use std::path::PathBuf;

#[cfg(not(target_arch = "wasm32"))]
use craft::host::queue::Priority;
use pdfcraft_engine::DocId;

pub(crate) enum Action {
    /// Close a tab, by its view key (`DocView::view_key`: split view can show one document in two
    /// tabs).
    Close(u64),
    Reopen,
    Reveal(DocId),
    Rename(DocId),
    /// Split right (or down) with a tab, by its view key.
    Split {
        tab: u64,
        down: bool,
    },
    /// Open a tab's document in a second PdfCraft window and close the tab here, by its view key.
    MoveToNewWindow(u64),
}

/// Why a tab can't move to a window of its own: the other window reads the file from disk, so the document needs a
/// saved path and nothing the file doesn't have, and no other view of it here, or it would be open in two processes
/// at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NewWindowRefusal {
    /// It has unsaved changes: the new window would show the older file, and closing the tab here would lose them.
    Unsaved,
    /// It has no file on disk (a new, combined or pasted document).
    NoPath,
    /// Split view shows the document in another tab too.
    OtherView,
}

/// The file a document with `path`, `unsaved` work and another view (`twin`) moves to a new window with, or why it
/// can't. Unsaved changes are checked first: a saved but edited document is not safe to hand over.
pub(crate) fn new_window_plan(path: Option<&str>, unsaved: bool, twin: bool) -> Result<&str, NewWindowRefusal> {
    if unsaved {
        return Err(NewWindowRefusal::Unsaved);
    }
    let path = path.filter(|p| !p.is_empty()).ok_or(NewWindowRefusal::NoPath)?;
    if twin {
        return Err(NewWindowRefusal::OtherView);
    }
    Ok(path)
}

/// The program and arguments that start a PdfCraft window of its own on `path`, at `page` (0-based). `--new-window`
/// keeps the launch out of the single-instance hand-off, which would pass the file to this very process, and out of
/// reopening last session's files. `path` is absolute, so a name that starts with `-` can't be read as an option.
#[cfg(not(target_arch = "wasm32"))]
fn new_window_command(exe: &std::path::Path, path: &std::path::Path, page: usize) -> (std::ffi::OsString, Vec<std::ffi::OsString>) {
    let page = page.saturating_add(1).to_string();
    (exe.as_os_str().to_owned(), vec!["--new-window".into(), "--page".into(), page.into(), path.as_os_str().to_owned()])
}

/// Start a new PdfCraft process on `path` at `page` (0-based), without waiting for it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn spawn_new_window(path: &str, page: usize) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
    let (program, args) = new_window_command(&exe, &path, page);
    let mut child = std::process::Command::new(program).args(args).spawn().map_err(|e| e.to_string())?;
    // Reap it when it ends, so it doesn't stay a zombie for as long as this window lives. If the thread can't start,
    // that is only logged.
    if let Err(e) = std::thread::Builder::new().name("pdfcraft-new-window".into()).spawn(move || drop(child.wait())) {
        log::warn!("new window: can't wait for the process: {e}");
    }
    Ok(())
}

/// The browser has no second window to start: the web says so instead.
#[cfg(target_arch = "wasm32")]
pub(crate) fn spawn_new_window(_path: &str, _page: usize) -> Result<(), String> {
    Err("Not available in the browser".into())
}

/// Reveal a local file without opening it or changing document/Recent state. The file manager call
/// blocks (a disconnected network mount, a slow `dbus-send` or `explorer.exe`), so it runs as a
/// job on the host's queue; a failure is shown by [`PdfCraftApp::drain_reveals`] on the pass that
/// receives the result.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn reveal_on_jobs(app: &mut crate::PdfCraftApp, path: PathBuf) {
    let target = path.clone();
    app.reveal_jobs.submit(path, Priority::VISIBLE, move |_| craft::platform::open::reveal(&target));
}

/// The browser has no file manager to show a file in: the web says so instead.
#[cfg(target_arch = "wasm32")]
pub(crate) fn reveal_on_jobs(app: &mut crate::PdfCraftApp, _path: PathBuf) {
    app.notify_error("The browser can't show files in a folder.".to_string());
}

impl crate::PdfCraftApp {
    /// Shows the failures of finished reveal jobs; once a frame, whatever the window shows.
    pub(crate) fn drain_reveals(&mut self) {
        while let Some(done) = self.reveal_jobs.try_recv() {
            if let Err(e) = done.result {
                self.notify_error(e);
            }
        }
    }

    /// Open tab `index`'s document in a new window, at the page shown, and close the tab here. A document with unsaved
    /// changes, no file, a file that is gone or another tab of it is refused with a notice that says why; a launch that
    /// fails keeps the tab. The closed tab is not added to the reopen history: the document lives on in the other
    /// window, and reopening it here would show it twice.
    pub(crate) fn move_tab_to_new_window(&mut self, index: usize) {
        if cfg!(target_arch = "wasm32") {
            self.notify_tr("Moving a tab to a new window is not available in the browser");
            return;
        }
        let Some(view) = self.views.get(index) else { return };
        let (id, page) = (view.id, view.current());
        // A document under OCR review stays put until the review ends (and says so).
        if self.hold_close_for_ocr_review(index) {
            return;
        }
        let path = self.session.get(id).and_then(|d| d.path.clone());
        let path = match new_window_plan(path.as_deref(), self.has_unsaved_work(index), self.has_twin(index)) {
            Ok(path) => path.to_owned(),
            Err(NewWindowRefusal::Unsaved | NewWindowRefusal::NoPath) => {
                self.notify_tr("Save the document before moving it to a new window");
                return;
            }
            Err(NewWindowRefusal::OtherView) => {
                self.notify_tr("Close the other tab of this document before moving it to a new window");
                return;
            }
        };
        if !std::path::Path::new(&path).is_file() {
            self.notify_tr("The document's file no longer exists");
            return;
        }
        if let Err(e) = (self.window_spawner)(&path, page) {
            self.notify_fmt("Couldn't open a new window: {e}", &[("e", &e)]);
            return;
        }
        let history = self.closed_tabs.clone();
        self.close_tab(index);
        self.closed_tabs = history;
    }

    /// Drops the latest closed-tab entry of `path`, once Reopen Closed Tab has brought it back.
    pub(crate) fn forget_closed_tab(&mut self, path: &str) {
        if let Some(at) = self.closed_tabs.iter().rposition(|closed| closed == path) {
            self.closed_tabs.remove(at);
        }
    }

    pub(crate) fn tab_action(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Close(tab) => {
                if let Some(i) = self.tab_with_key(tab) {
                    self.request_close_tab(ctx, i);
                }
            }
            Action::Split { tab, down } => {
                if let Some(i) = self.tab_with_key(tab) {
                    self.split_tab(ctx, i, down);
                }
            }
            Action::Reopen => {
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(path) = self.closed_tabs.last().cloned() {
                    // A tab that shows the file already (opened again through File > Open) comes to the front.
                    // Otherwise the open is queued, and its entry goes only once the file has opened.
                    match self.open_path(&path) {
                        crate::OpenOutcome::Opened | crate::OpenOutcome::Focused => self.forget_closed_tab(&path),
                        crate::OpenOutcome::Pending => {
                            if let Some(pending) = self.pending_opens.iter_mut().find(|p| p.path.as_deref() == Some(path.as_str())) {
                                pending.reopen = true;
                            }
                        }
                        crate::OpenOutcome::Failed(_) => {}
                    }
                }
            }
            Action::Reveal(id) => {
                if let Some(path) = self.session.get(id).and_then(|d| d.path.clone()) {
                    reveal_on_jobs(self, PathBuf::from(path));
                }
            }
            Action::Rename(id) => self.start_rename(ctx, id),
            Action::MoveToNewWindow(tab) => {
                if let Some(i) = self.tab_with_key(tab) {
                    self.move_tab_to_new_window(i);
                }
            }
        }
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::PdfCraftApp;
    use egui::accesskit::Role;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };
    use std::sync::Arc;

    const PDF: &[u8] = include_bytes!("../../tests/data/form.pdf");

    /// An English app on a host whose jobs run when the test says.
    fn app_on(jobs: Arc<craft::host::ManualJobs>) -> PdfCraftApp {
        let info = craft::host::AppInfo { id: "pdfcraft", name: "PdfCraft", version: "0" };
        let host = craft::host::Host::builder(info).jobs(jobs).build();
        let wake = craft::host::Wake::none();
        let (intake, _) = craft::platform::open::FileIntake::new(wake.clone());
        let clipboard = craft::platform::clipboard::Clipboard::new(craft::platform::clipboard::EventClipboard::new(|| 0, |_| Ok(0)));
        let watch = craft::platform::watch::FileWatch::new(wake.clone());
        let services = crate::AppServices::new(wake, clipboard, None, None, intake, watch);
        let mut app = PdfCraftApp::with_host(Arc::new(host), services);
        app.set_option(&egui::Context::default(), "language", "en").unwrap();
        app
    }

    /// Runs the queued opens and takes their results, as the next frame would.
    fn settle(app: &mut PdfCraftApp, jobs: &craft::host::ManualJobs) {
        jobs.run_all();
        app.poll_pending_opens(&egui::Context::default());
    }

    /// An English app whose jobs never run.
    fn app() -> PdfCraftApp {
        app_on(Arc::new(craft::host::ManualJobs::new()))
    }

    /// The unsaved-changes dialog's "don't keep the changes" and Cancel answers as the framework words
    /// them on this OS.
    fn answers() -> (String, String) {
        // PLAN: assumed paths `craft::ui::dialog::DialogWords`, `craft::ui::runtime::words::CommonWords`,
        // `craft::i18n::WordSet`; their fields display as their English text.
        use craft::i18n::WordSet as _;
        let dialog = craft::ui::dialog::DialogWords::english();
        let common = craft::ui::runtime::words::CommonWords::english();
        // The app words the question as production does on every platform: "Don't save", in lower case.
        let _ = dialog;
        ("Don't save".to_string(), common.cancel.as_str().to_string())
    }

    #[test]
    fn confirmed_close_uses_id_and_cancel_keeps_history_empty() {
        let ctx = &egui::Context::default();
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|cc| {
            crate::PdfCraftApp::install_runtime(&cc.egui_ctx);
            {
                let mut app = app();
                app.open_bytes(ctx, "first.pdf", Some("/tmp/first.pdf".into()), PDF.to_vec()).unwrap();
                app.open_bytes(ctx, "second.pdf", None, PDF.to_vec()).unwrap();
                app
            }
        });
        h.run_steps(4);
        let (first, first_tab) = (h.state().views[0].id, h.state().views[0].view_key());
        h.state_mut().session.apply(first, pdfcraft_engine::Edit::InsertBlankPage { at: 0, width: 300.0, height: 400.0 }).unwrap();
        h.state_mut().tab_action(&egui::Context::default(), Action::Close(first_tab));
        h.run_steps(3);
        let (dont_save, cancel) = answers();
        assert_eq!(h.state().views.len(), 2);
        assert!(h.state().closed_tabs.is_empty());
        h.get_by_role_and_label(Role::Button, &cancel).click();
        h.run_steps(3);
        assert_eq!(h.state().views.len(), 2);
        assert!(h.state().closed_tabs.is_empty());
        h.state_mut().tab_action(&egui::Context::default(), Action::Close(first_tab));
        h.run_steps(3);
        h.get_by_role_and_label(Role::Button, &dont_save).click();
        h.run_steps(3);
        assert_eq!(h.state().views.len(), 1);
        assert_eq!(h.state().closed_tabs, ["/tmp/first.pdf"]);
        assert_eq!(h.state().active, Some(0));
        h.state_mut().close_tab(0);
        assert_eq!(h.state().closed_tabs.len(), 1, "pathless tabs are not recorded");
    }

    #[test]
    fn reopen_reads_saved_file_and_history_is_bounded() {
        use std::io::Write as _;
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("pdfcraft-tab-{}-{nonce}-年度 report.pdf", std::process::id()));
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
        file.write_all(PDF).unwrap();
        drop(file);
        let jobs = Arc::new(craft::host::ManualJobs::new());
        let mut app = app_on(jobs.clone());
        let path_text = path.to_str().unwrap();
        for _ in 0..12 {
            app.open_path(path_text);
            settle(&mut app, &jobs);
            app.close_tab(0);
        }
        assert_eq!(app.closed_tabs.len(), 10);
        app.tab_action(&egui::Context::default(), Action::Reopen);
        assert_eq!(app.closed_tabs.len(), 10, "the entry stays until the file has opened");
        settle(&mut app, &jobs);
        assert_eq!(app.views.len(), 1);
        let doc = app.session.get(app.views[0].id).unwrap();
        assert_eq!(doc.path.as_deref(), Some(path_text));
        assert!(!doc.dirty);
        assert_eq!(app.closed_tabs.len(), 9);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn failed_reopen_keeps_history_and_retry_does_not_duplicate_tabs() {
        use std::io::Write as _;
        let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("pdfcraft-retry-{}-{nonce}.pdf", std::process::id()));
        let text = path.to_str().unwrap();
        let mut file = std::fs::OpenOptions::new().create_new(true).write(true).open(&path).unwrap();
        file.write_all(PDF).unwrap();
        drop(file);
        let jobs = Arc::new(craft::host::ManualJobs::new());
        let mut app = app_on(jobs.clone());
        app.open_path(text);
        settle(&mut app, &jobs);
        app.close_tab(0);
        std::fs::remove_file(&path).unwrap();
        app.tab_action(&egui::Context::default(), Action::Reopen);
        settle(&mut app, &jobs);
        assert!(app.views.is_empty());
        assert_eq!(app.closed_tabs, [text]);
        std::fs::write(&path, PDF).unwrap();
        app.tab_action(&egui::Context::default(), Action::Reopen);
        settle(&mut app, &jobs);
        assert_eq!(app.views.len(), 1);
        assert!(app.closed_tabs.is_empty());
        app.closed_tabs.push(text.into());
        app.tab_action(&egui::Context::default(), Action::Reopen);
        assert_eq!(app.views.len(), 1, "an already-open file is selected, not duplicated");
        assert!(app.closed_tabs.is_empty());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_failed_reveal_is_reported_back_to_the_ui_thread() {
        let jobs = Arc::new(craft::host::ManualJobs::new());
        let held = jobs.clone();
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(move |cc| {
            crate::PdfCraftApp::install_runtime(&cc.egui_ctx);
            app_on(held)
        });
        h.run_steps(2);
        reveal_on_jobs(h.state_mut(), PathBuf::from("/pdfcraft-no-such-dir/missing.pdf"));
        assert_eq!(jobs.waiting(), 1, "the reveal is a job on the host");
        h.run_steps(2);
        let posted = |h: &Harness<'_, PdfCraftApp>| crate::notices::posted(&h.ctx).len();
        assert_eq!(posted(&h), 0, "nothing is shown until the job ends");
        jobs.run_all();
        h.run_steps(2);
        assert_eq!(posted(&h), 1, "the failure is shown as a notice");
    }

    #[test]
    fn inactive_tab_menu_closes_target_and_disables_unavailable_actions() {
        let ctx = &egui::Context::default();
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_eframe(|cc| {
            crate::PdfCraftApp::install_runtime(&cc.egui_ctx);
            {
                let mut app = app();
                for name in ["first.pdf", "second.pdf"] {
                    app.open_bytes(ctx, name, None, PDF.to_vec()).unwrap();
                }
                app
            }
        });
        h.run_steps(8);
        h.get_by_role_and_label(egui::accesskit::Role::Tab, "first.pdf").click_secondary();
        h.run_steps(2);
        assert!(h.get_by_label("Reopen Closed Tab").accesskit_node().is_disabled());
        let reveal_label = if cfg!(target_os = "macos") { "Reveal in Finder" } else { "Reveal in File Explorer" };
        assert!(h.get_by_label(reveal_label).accesskit_node().is_disabled());
        assert_eq!(h.state().active, Some(1));
        h.get_by_label("Close Tab").click();
        h.run_steps(3);
        assert_eq!(h.state().views.len(), 1);
        assert_eq!(h.state().session.get(h.state().views[0].id).unwrap().name, "second.pdf");
    }
}
