//! The AI assistant's settings (Preferences ▸ AI Assistant) and the one-shot background request
//! that Test connection and the Ask AI Assistant dialog share.
//!
//! The settings live in `ai.json` in the app's settings folder, written with [`AiConfig::save`]
//! (owner-only on Unix, because the file can hold an API key). They are never stored in `app.ron`.
//! Without a settings folder (the web build, tests) they last for the session. Empty fields fall
//! back to the `PDFCRAFT_AI_*` environment variables; nothing is sent anywhere until a question is
//! asked or Test connection is pressed.

use std::path::{Path, PathBuf};
use std::sync::{OnceLock, mpsc};

use pdfcraft_ai::AiConfig;

static SETTINGS_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Tells the app which folder holds its settings, so the AI settings are kept in `ai.json` there.
/// The app calls this once at start; later calls change nothing.
pub fn set_settings_dir(dir: PathBuf) {
    let _ = SETTINGS_DIR.set(dir);
}

fn default_path() -> Option<PathBuf> {
    SETTINGS_DIR.get().map(|d| d.join("ai.json"))
}

/// A request run once on a background thread. Dropping or [`Task::cancel`]ling it forgets the
/// answer: a worker that finishes later finds nobody listening.
#[derive(Default)]
pub(crate) enum Task<T> {
    #[default]
    Idle,
    Running(mpsc::Receiver<Result<T, String>>),
    Done(Result<T, String>),
}

impl<T: Send + 'static> Task<T> {
    /// Starts `work`, forgetting any earlier request. On the web, and with `inline`, it runs here.
    pub(crate) fn start(&mut self, ctx: &egui::Context, inline: bool, work: impl FnOnce() -> Result<T, String> + Send + 'static) {
        let (sender, receiver) = mpsc::channel();
        let repaint = ctx.clone();
        let job = move || {
            // A panic in the worker must not leave the dialog waiting forever.
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).unwrap_or_else(|_| Err("the request stopped unexpectedly".to_owned()));
            let _ = sender.send(result);
            repaint.request_repaint();
        };
        #[cfg(not(target_arch = "wasm32"))]
        if !inline {
            return match std::thread::Builder::new().name("ai-request".into()).spawn(job) {
                Ok(_) => *self = Task::Running(receiver),
                Err(e) => *self = Task::Done(Err(e.to_string())),
            };
        }
        let _ = inline;
        job();
        *self = Task::Running(receiver);
        self.poll();
    }

    /// Takes the answer in when it has arrived. Returns whether the request is still running.
    pub(crate) fn poll(&mut self) -> bool {
        if let Task::Running(receiver) = self {
            match receiver.try_recv() {
                Ok(result) => *self = Task::Done(result),
                Err(mpsc::TryRecvError::Empty) => return true,
                Err(mpsc::TryRecvError::Disconnected) => *self = Task::Done(Err("the request stopped unexpectedly".to_owned())),
            }
        }
        false
    }

    pub(crate) fn cancel(&mut self) {
        *self = Task::Idle;
    }

    pub(crate) fn is_running(&self) -> bool {
        matches!(self, Task::Running(_))
    }

    pub(crate) fn result(&self) -> Option<&Result<T, String>> {
        match self {
            Task::Done(r) => Some(r),
            _ => None,
        }
    }
}

/// The edited settings, what was last saved, and the requests that use them.
pub struct AiSettings {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    timeout_secs: u64,
    path: Option<PathBuf>,
    saved: AiConfig,
    /// Why the settings could not be loaded or saved, shown in Preferences.
    pub(crate) problem: Option<String>,
    pub(crate) test: Task<Vec<String>>,
    pub(crate) ask: Task<String>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self::load(default_path())
    }
}

impl AiSettings {
    /// The settings stored at `path` (none when there is no file, or no place to keep one).
    pub fn load(path: Option<PathBuf>) -> Self {
        let (stored, problem) = match path.as_deref().map(AiConfig::load) {
            Some(Ok(c)) => (c, None),
            Some(Err(e)) => (AiConfig::default(), Some(e.to_string())),
            None => (AiConfig::default(), None),
        };
        AiSettings {
            base_url: stored.base_url.clone(),
            model: stored.model.clone(),
            api_key: stored.api_key.clone().unwrap_or_default(),
            timeout_secs: stored.timeout_secs,
            path,
            saved: stored,
            problem,
            test: Task::Idle,
            ask: Task::Idle,
        }
    }

    /// The settings as typed, without the environment's.
    pub fn stored(&self) -> AiConfig {
        let key = self.api_key.trim();
        AiConfig {
            base_url: self.base_url.trim().to_owned(),
            api_key: (!key.is_empty()).then(|| key.to_owned()),
            model: self.model.trim().to_owned(),
            timeout_secs: self.timeout_secs,
        }
    }

    /// The settings a request uses: those typed, with empty fields filled from `PDFCRAFT_AI_*`.
    pub fn effective(&self) -> AiConfig {
        self.stored().or(AiConfig::from_env())
    }

    /// Whether a question can be sent: an address and a model are set.
    pub fn is_configured(&self) -> bool {
        self.effective().check().is_ok()
    }

    /// Why the settings are not usable yet, in the words [`AiConfig::check`] gives.
    pub fn problem_with_config(&self) -> Option<String> {
        self.effective().check().err().map(|e| e.to_string())
    }

    /// Writes the settings when they differ from what was last saved. A failure is kept in
    /// [`AiSettings::problem`] and not retried until the settings change again.
    pub fn save_if_changed(&mut self) {
        let now = self.stored();
        if now == self.saved {
            return;
        }
        self.saved = now.clone();
        let Some(path) = &self.path else { return };
        self.problem = now.save(path).err().map(|e| e.to_string());
        // The file can hold an API key: on Windows only the user and SYSTEM may read it (Unix gets
        // mode 0600 from the save itself).
        #[cfg(windows)]
        if self.problem.is_none()
            && let Err(e) =
                pdfcraft_platform::owner_only::current_user_sid().and_then(|me| pdfcraft_platform::owner_only::restrict_to_owner(path, &me))
        {
            self.problem = Some(format!("cannot restrict the AI settings file to your account: {e}"));
        }
    }

    /// Where the settings are kept, when they are kept.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

/// The pages of `source`, as `(1-based page, text)`, for a question. Reading stops once more text
/// than a question can carry has been gathered, so a long file is not read to its end.
pub(crate) fn page_texts(source: pdfcraft_engine::export::ExportSource, pages: &[usize]) -> Result<Vec<(usize, String)>, String> {
    let mut renderer = pdfcraft_render::PageRenderer::new(source.bytes, source.config);
    let mut out = Vec::new();
    let mut chars = 0usize;
    for &page in pages {
        if page >= source.pages {
            return Err(format!("page {} is out of range", page.saturating_add(1)));
        }
        let text = renderer
            .filtered_text(page, pdfcraft_render::text::TextFilter::default())
            .map_err(|e| format!("page {}: {e}", page.saturating_add(1)))?
            .plain_text();
        chars = chars.saturating_add(text.chars().count());
        out.push((page.saturating_add(1), text));
        if chars > pdfcraft_ai::MAX_CONTEXT_CHARS {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("pdfcraft-ai-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn wait<T: Send + 'static>(task: &mut Task<T>) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while task.poll() {
            assert!(std::time::Instant::now() < deadline, "worker timed out");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn settings_are_saved_once_per_change_and_read_back() {
        let dir = temp("save");
        let path = dir.join("ai.json");
        let mut s = AiSettings::load(Some(path.clone()));
        assert!(s.problem.is_none() && !path.exists());
        s.save_if_changed();
        assert!(!path.exists(), "nothing typed, nothing written");
        s.base_url = "  http://localhost:11434/v1 ".into();
        s.model = "llama3.1".into();
        s.api_key = " sk-secret ".into();
        s.save_if_changed();
        assert!(s.problem.is_none());
        let back = AiSettings::load(Some(path.clone()));
        assert_eq!((back.base_url.as_str(), back.model.as_str(), back.api_key.as_str()), ("http://localhost:11434/v1", "llama3.1", "sk-secret"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "the key must not be readable by others");
        }
        // Clearing the key removes it from the file.
        s.api_key.clear();
        s.save_if_changed();
        assert_eq!(AiConfig::load(&path).unwrap().api_key, None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_corrupt_file_is_reported_and_replaced_by_the_next_change() {
        let dir = temp("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ai.json");
        std::fs::write(&path, "{ not json").unwrap();
        let mut s = AiSettings::load(Some(path.clone()));
        assert!(s.problem.as_deref().is_some_and(|p| p.contains("ai.json")), "{:?}", s.problem);
        assert!(s.base_url.is_empty());
        s.model = "m".into();
        s.save_if_changed();
        assert!(s.problem.is_none());
        assert_eq!(AiConfig::load(&path).unwrap().model, "m");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_save_that_fails_is_reported_once_and_does_not_panic() {
        let dir = temp("fail");
        std::fs::create_dir_all(&dir).unwrap();
        // A folder where the file should be: the rename onto it fails.
        let path = dir.join("ai.json");
        std::fs::create_dir_all(path.join("inner")).unwrap();
        let mut s = AiSettings { path: Some(path), ..AiSettings::load(None) };
        s.model = "m".into();
        s.save_if_changed();
        assert!(s.problem.is_some());
        s.problem = None;
        s.save_if_changed();
        assert!(s.problem.is_none(), "an unchanged setting is not retried every frame");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn without_a_folder_the_settings_last_for_the_session() {
        let mut s = AiSettings::load(None);
        s.base_url = "http://localhost:1234/v1".into();
        s.model = "m".into();
        s.save_if_changed();
        assert!(s.problem.is_none() && s.path().is_none());
        assert_eq!(s.stored().base_url, "http://localhost:1234/v1");
    }

    #[test]
    fn typed_fields_win_and_empty_ones_take_the_environment() {
        let mut s = AiSettings::load(None);
        assert_eq!(s.stored(), AiConfig::default());
        s.base_url = "ftp://nope".into();
        s.model = "m".into();
        assert!(!s.is_configured());
        assert!(s.problem_with_config().is_some_and(|p| p.contains("http")));
        s.base_url = "http://localhost:11434/v1".into();
        assert!(s.is_configured());
        assert_eq!(s.effective().base_url, "http://localhost:11434/v1");
        assert_eq!(s.stored().api_key, None);
    }

    #[test]
    fn a_task_goes_from_idle_to_running_to_done_and_can_be_cancelled() {
        let (release, gate) = mpsc::channel::<()>();
        let mut task: Task<String> = Task::Idle;
        assert!(!task.poll() && task.result().is_none());
        task.start(&egui::Context::default(), false, move || {
            let _ = gate.recv();
            Ok("answer".to_owned())
        });
        assert!(task.is_running() && task.poll() && task.result().is_none(), "waits for the worker");
        let _ = release.send(());
        wait(&mut task);
        assert_eq!(task.result(), Some(&Ok("answer".to_owned())));
        // A new request replaces the answer; a cancelled one is never shown.
        let (release, gate) = mpsc::channel::<()>();
        task.start(&egui::Context::default(), false, move || {
            let _ = gate.recv();
            Ok("stale".to_owned())
        });
        assert!(task.result().is_none());
        task.cancel();
        let _ = release.send(());
        assert!(!task.poll() && task.result().is_none());
    }

    #[test]
    fn a_failure_or_a_panic_in_the_worker_becomes_an_error() {
        let mut task: Task<()> = Task::Idle;
        task.start(&egui::Context::default(), true, || Err("no server".to_owned()));
        assert_eq!(task.result(), Some(&Err("no server".to_owned())));
        for inline in [true, false] {
            task.start(&egui::Context::default(), inline, || -> Result<(), String> { panic!("deliberate panic in the worker") });
            wait(&mut task);
            assert!(matches!(task.result(), Some(Err(e)) if e.contains("unexpectedly")), "inline {inline}");
        }
    }

    #[test]
    fn page_texts_reads_the_requested_pages_and_refuses_bad_ones() {
        let bytes = pdfcraft_engine::Session::new().create_from_text("Doc", "Hello world\u{c}Second page").unwrap();
        let mut session = pdfcraft_engine::Session::new();
        let id = session.open_new("doc.pdf", bytes).unwrap();
        let source = || session.get(id).unwrap().export_source();
        let all = page_texts(source(), &[0, 1]).unwrap();
        assert_eq!(all.iter().map(|(p, _)| *p).collect::<Vec<_>>(), [1, 2]);
        assert!(all[0].1.contains("Hello world") && all[1].1.contains("Second page"));
        let second = page_texts(source(), &[1]).unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].0, 2);
        assert!(page_texts(source(), &[5]).unwrap_err().contains("out of range"));
    }
}
