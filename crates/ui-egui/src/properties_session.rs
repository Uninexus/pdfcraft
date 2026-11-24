//! Document Properties keeps its values in the dialog's fields and commits one revision-bound draft.

use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue};
use edit_session::EditSession;
use pdfcraft_engine::{
    DocId, Document, Duplex, Edit, InitialLayout, InitialView, MAX_PRINT_COPIES, MAX_PRINT_RANGES, MIN_PRINT_COPIES, Magnification, Navigation,
    PrintPresets, PrintScaling,
};
use serde_json::{Value, json};

use crate::PdfCraftApp;
use crate::dialogs::properties::PropsTab;

pub(crate) const INFO_KEYS: [&str; 4] = ["Title", "Author", "Subject", "Keywords"];

/// The Navigation tab choices: field id, value and English label.
pub(crate) const NAVIGATIONS: [(&str, Navigation, &str); 5] = [
    ("page-only", Navigation::PageOnly, "Page Only"),
    ("bookmarks", Navigation::Bookmarks, "Bookmarks Panel and Page"),
    ("pages", Navigation::Pages, "Pages Panel and Page"),
    ("attachments", Navigation::Attachments, "Attachments Panel and Page"),
    ("layers", Navigation::Layers, "Layers Panel and Page"),
];

/// The page layout choices: field id, value and English label.
pub(crate) const LAYOUTS: [(&str, InitialLayout, &str); 7] = [
    ("default", InitialLayout::Default, "Default"),
    ("single", InitialLayout::SinglePage, "Single Page"),
    ("single-continuous", InitialLayout::SinglePageContinuous, "Single Page Continuous"),
    ("two-up", InitialLayout::TwoUp, "Two-Up (Facing)"),
    ("two-up-continuous", InitialLayout::TwoUpContinuous, "Two-Up Continuous (Facing)"),
    ("two-up-cover", InitialLayout::TwoUpCoverPage, "Two-Up (Cover Page)"),
    ("two-up-continuous-cover", InitialLayout::TwoUpContinuousCoverPage, "Two-Up Continuous (Cover Page)"),
];

/// The named magnifications: field id, value and English label.
pub(crate) const MAGNIFICATIONS: [(&str, Magnification, &str); 6] = [
    ("default", Magnification::Default, "Default"),
    ("actual", Magnification::ActualSize, "Actual Size"),
    ("fit-page", Magnification::FitPage, "Fit Page"),
    ("fit-width", Magnification::FitWidth, "Fit Width"),
    ("fit-height", Magnification::FitHeight, "Fit Height"),
    ("fit-visible", Magnification::FitVisible, "Fit Visible"),
];

/// The field id of the choice that keeps a Print Dialog Presets entry the file holds but the dialog cannot read.
pub(crate) const KEPT: &str = "unreadable";

/// The page scaling choices: field id, value and English label.
pub(crate) const SCALINGS: [(&str, PrintScaling, &str); 2] = [("default", PrintScaling::AppDefault, "Default"), ("none", PrintScaling::None, "None")];

/// The duplex choices: field id, value (`None`: no entry) and English label.
pub(crate) const DUPLEXES: [(&str, Option<Duplex>, &str); 4] = [
    ("default", None, "Default"),
    ("simplex", Some(Duplex::Simplex), "Simplex (one side)"),
    ("flip_short_edge", Some(Duplex::FlipShortEdge), "Both sides, flip on short edge"),
    ("flip_long_edge", Some(Duplex::FlipLongEdge), "Both sides, flip on long edge"),
];

/// The paper source by PDF page size choices: field id, value (`None`: no entry) and English label.
pub(crate) const TRAYS: [(&str, Option<bool>, &str); 3] = [("default", None, "Default"), ("yes", Some(true), "Yes"), ("no", Some(false), "No")];

/// The page ranges as the dialog writes them: `1-3, 7`.
pub(crate) fn ranges_text(ranges: &[(usize, usize)]) -> String {
    ranges.iter().map(|(a, b)| if a == b { a.to_string() } else { format!("{a}-{b}") }).collect::<Vec<_>>().join(", ")
}

/// The ranges `text` holds (empty: every page), or `None` when it is not a list of `first-last` or single pages.
fn parse_ranges(text: &str) -> Option<Vec<(usize, usize)>> {
    let mut out = Vec::new();
    for part in text.split(',').map(str::trim).filter(|part| !part.is_empty()) {
        let (first, last): (usize, usize) = match part.split_once('-') {
            Some((a, b)) => (a.trim().parse().ok()?, b.trim().parse().ok()?),
            None => {
                let page = part.parse().ok()?;
                (page, page)
            }
        };
        if first < 1 || first > last || out.len() >= MAX_PRINT_RANGES {
            return None;
        }
        out.push((first, last));
    }
    Some(out)
}

/// The percentages the magnification choice offers besides the named ones.
pub(crate) const PERCENTS: [f64; 7] = [25.0, 50.0, 75.0, 100.0, 125.0, 150.0, 200.0];

/// The field id of the percentage `p`.
pub(crate) fn percent_id(p: f64) -> String {
    format!("percent:{p}")
}

pub(crate) fn magnification_id(m: Magnification) -> String {
    match (MAGNIFICATIONS.iter().find(|(_, named, _)| *named == m), m) {
        (Some((id, ..)), _) => (*id).to_owned(),
        (None, Magnification::Percent(p)) => percent_id(p),
        (None, _) => "default".to_owned(),
    }
}

fn magnification_from(id: &str) -> Option<Magnification> {
    MAGNIFICATIONS.iter().find(|(named, ..)| *named == id).map(|(_, m, _)| *m).or_else(|| {
        let p: f64 = id.strip_prefix("percent:")?.parse().ok()?;
        p.is_finite().then_some(Magnification::Percent(p))
    })
}

/// The field key of a standard information entry.
pub(crate) fn description_key(key: &str) -> String {
    key.to_ascii_lowercase()
}

/// The custom properties the `custom` field holds.
pub(crate) fn custom_pairs(fields: &DialogFields) -> Option<Vec<(String, String)>> {
    let value = serde_json::to_value(fields.get("custom")?).ok()?;
    value.as_array()?.iter().map(|entry| Some((entry.get("name")?.as_str()?.to_owned(), entry.get("value")?.as_str()?.to_owned()))).collect()
}

/// The `custom` field for `props`.
pub(crate) fn custom_json(props: &[(String, String)]) -> FieldValue {
    FieldValue::Json(Value::Array(props.iter().map(|(name, value)| json!({ "name": name, "value": value })).collect()))
}

/// The small, accepted values shared by the Description, Initial View, Advanced and Custom tabs.
#[derive(Clone, Debug)]
pub(crate) struct Values {
    description: [String; 4],
    view: InitialView,
    custom: Vec<(String, String)>,
    /// The base address relative links resolve against (empty: none), written as the catalog's `/URI` `/Base`.
    base_url: String,
    /// What the Print dialog starts with, and the settings whose entry the file holds but cannot be read.
    print: PrintPresets,
    print_unreadable: Vec<&'static str>,
}

impl PartialEq for Values {
    /// Compare accepted values using the dialog's whitespace and unordered Custom-row rules.
    fn eq(&self, other: &Self) -> bool {
        // Preserve the dialog's existing whitespace/no-op rules, without stripping spaces
        // from the text widgets while the user types. Custom row order is not a PDF edit.
        self.description.iter().zip(&other.description).all(|(a, b)| a.trim() == b.trim())
            && self.view == other.view
            && self.base_url.trim() == other.base_url.trim()
            && self.print == other.print
            && self.custom.len() == other.custom.len()
            && self.custom.iter().all(|entry| other.custom.contains(entry))
    }
}

impl Values {
    /// Snapshot the editable values; PDF bytes and undo history remain owned by the engine.
    pub(crate) fn read(doc: &Document) -> Self {
        let (print, print_unreadable) = doc.print_presets();
        Self {
            description: INFO_KEYS.map(|key| doc.info_value(key).unwrap_or_default()),
            view: doc.initial_view(),
            custom: doc.custom_info(),
            base_url: doc.base_url().0.unwrap_or_default(),
            print,
            print_unreadable,
        }
    }

    /// The dialog's fields for these values, with the Custom tab's input boxes empty.
    pub(crate) fn to_fields(&self) -> DialogFields {
        let mut fields = DialogFields::new();
        for (key, text) in INFO_KEYS.iter().zip(&self.description) {
            fields.set(description_key(key), text.clone());
        }
        let v = &self.view;
        let id = |found: Option<&str>| found.unwrap_or("default").to_owned();
        fields.set("navigation", id(NAVIGATIONS.iter().find(|(_, n, _)| *n == v.navigation).map(|(id, ..)| *id)));
        fields.set("layout", id(LAYOUTS.iter().find(|(_, l, _)| *l == v.layout).map(|(id, ..)| *id)));
        fields.set("magnification", magnification_id(v.magnification));
        fields.set("page", i64::try_from(v.page.saturating_add(1)).unwrap_or(i64::MAX));
        fields.set("fit_window", v.fit_window);
        fields.set("center_window", v.center_window);
        fields.set("full_screen", v.full_screen);
        fields.set("display_title", if v.display_title { "title" } else { "file" });
        fields.set("hide_menubar", v.hide_menubar);
        fields.set("hide_toolbar", v.hide_toolbar);
        fields.set("hide_window_ui", v.hide_window_ui);
        fields.set("binding", if v.right_to_left { "right" } else { "left" });
        fields.set("language", v.language.clone().unwrap_or_default());
        fields.set("base_url", self.base_url.clone());
        self.print_to_fields(&mut fields);
        fields.set("custom", custom_json(&self.custom));
        fields.set("custom.name", "");
        fields.set("custom.value", "");
        fields.set("custom.unlisted", false);
        fields
    }

    /// The Print Dialog Presets rows: each shows the stored value, or the kept choice for an entry that cannot be read.
    fn print_to_fields(&self, fields: &mut DialogFields) {
        let p = &self.print;
        let shown = |setting: &str, id: &str| if self.print_unreadable.contains(&setting) { KEPT.to_owned() } else { id.to_owned() };
        let found = |id: Option<&&str>| id.copied().unwrap_or("default").to_owned();
        fields.set("print_scaling", shown("scaling", &found(SCALINGS.iter().find(|(_, s, _)| *s == p.scaling).map(|(id, ..)| id))));
        fields.set("print_duplex", shown("duplex", &found(DUPLEXES.iter().find(|(_, d, _)| *d == p.duplex).map(|(id, ..)| id))));
        fields
            .set("print_pick_tray", shown("pick_tray_by_size", &found(TRAYS.iter().find(|(_, t, _)| *t == p.pick_tray_by_size).map(|(id, ..)| id))));
        fields.set("print_copies", shown("copies", &p.copies.map_or_else(|| "default".to_owned(), |c| c.to_string())));
        fields.set("print_ranges", ranges_text(&p.page_ranges));
    }

    /// The presets `fields` hold over `base`: a kept choice, or no field, leaves that setting as it is stored.
    /// `None` when a setting is not one the dialog can write.
    fn print_from_fields(fields: &DialogFields, base: &Self) -> Option<PrintPresets> {
        let mut p = base.print.clone();
        let choice = |key: &str| fields.text(key).filter(|id| *id != KEPT);
        if let Some(found) = choice("print_scaling").and_then(|id| SCALINGS.iter().find(|(named, ..)| *named == id)) {
            p.scaling = found.1;
        }
        if let Some(found) = choice("print_duplex").and_then(|id| DUPLEXES.iter().find(|(named, ..)| *named == id)) {
            p.duplex = found.1;
        }
        if let Some(found) = choice("print_pick_tray").and_then(|id| TRAYS.iter().find(|(named, ..)| *named == id)) {
            p.pick_tray_by_size = found.1;
        }
        if let Some(id) = choice("print_copies") {
            p.copies =
                if id == "default" { None } else { Some(id.parse::<u8>().ok().filter(|c| (MIN_PRINT_COPIES..=MAX_PRINT_COPIES).contains(c))?) };
        }
        if let Some(text) = fields.text("print_ranges") {
            // Empty text over an unreadable entry, or the text the stored ranges give, is no change.
            let unchanged =
                text.trim() == ranges_text(&base.print.page_ranges) || (text.trim().is_empty() && base.print_unreadable.contains(&"page_ranges"));
            if !unchanged {
                p.page_ranges = parse_ranges(text)?;
            }
        }
        Some(p)
    }

    /// The values `fields` hold, over `base` for what the dialog does not carry (the view's non-full-screen panel).
    /// `None` when the fields cannot be read as values.
    pub(crate) fn from_fields(fields: &DialogFields, base: &Self) -> Option<Self> {
        let description: Vec<String> = INFO_KEYS
            .iter()
            .zip(&base.description)
            .map(|(key, kept)| fields.text(&description_key(key)).map_or_else(|| kept.clone(), str::to_owned))
            .collect();
        let mut view = base.view.clone();
        let choice = |key: &str| fields.text(key);
        if let Some(n) = choice("navigation").and_then(|id| NAVIGATIONS.iter().find(|(named, ..)| *named == id)) {
            view.navigation = n.1;
        }
        if let Some(l) = choice("layout").and_then(|id| LAYOUTS.iter().find(|(named, ..)| *named == id)) {
            view.layout = l.1;
        }
        if let Some(m) = choice("magnification").and_then(magnification_from) {
            view.magnification = m;
        }
        if let Some(n) = fields.int("page") {
            view.page = usize::try_from(n.saturating_sub(1)).unwrap_or(0);
        }
        for (key, flag) in [
            ("fit_window", &mut view.fit_window),
            ("center_window", &mut view.center_window),
            ("full_screen", &mut view.full_screen),
            ("hide_menubar", &mut view.hide_menubar),
            ("hide_toolbar", &mut view.hide_toolbar),
            ("hide_window_ui", &mut view.hide_window_ui),
        ] {
            if let Some(on) = fields.bool(key) {
                *flag = on;
            }
        }
        if let Some(show) = choice("display_title") {
            view.display_title = show == "title";
        }
        if let Some(binding) = choice("binding") {
            view.right_to_left = binding == "right";
        }
        if let Some(language) = choice("language") {
            view.language = (!language.trim().is_empty()).then(|| language.trim().to_owned());
        }
        // A new base the engine would refuse is no value: OK is refused until it is corrected. One the file
        // already holds is kept as it is, whatever it is.
        let base_url = fields.text("base_url").map_or_else(|| base.base_url.clone(), |text| text.trim().to_owned());
        if base_url != base.base_url.trim() && !base_url.is_empty() && pdfcraft_engine::check_base_url(&base_url).is_err() {
            return None;
        }
        Some(Self {
            description: <[String; 4]>::try_from(description).ok()?,
            view,
            custom: custom_pairs(fields).unwrap_or_else(|| base.custom.clone()),
            base_url,
            print: Self::print_from_fields(fields, base)?,
            print_unreadable: base.print_unreadable.clone(),
        })
    }

    /// Translate an accepted draft into existing engine edits, preserving unrelated PDF metadata.
    fn edits_to(&self, after: &Self) -> Vec<Edit> {
        let mut edits: Vec<Edit> = INFO_KEYS
            .iter()
            .zip(self.description.iter().zip(&after.description))
            .filter(|(_, (before, after))| before.trim() != after.trim())
            .map(|(key, (_, value))| Edit::SetInfo { key: (*key).into(), value: value.clone() })
            .collect();
        if self.view != after.view {
            edits.push(Edit::SetInitialView(Box::new(after.view.clone())));
        }
        if self.base_url.trim() != after.base_url.trim() {
            let base = after.base_url.trim();
            edits.push(Edit::SetBaseUrl((!base.is_empty()).then(|| base.to_owned())));
        }
        if self.print != after.print {
            edits.push(Edit::SetPrintPresets(Box::new(after.print.clone())));
        }
        for (key, _) in self.custom.iter().filter(|(key, _)| !after.custom.iter().any(|(name, _)| name == key)) {
            edits.push(Edit::SetInfo { key: key.clone(), value: String::new() });
        }
        for (key, value) in after.custom.iter().filter(|entry| !self.custom.contains(entry)) {
            edits.push(Edit::SetInfo { key: key.clone(), value: value.clone() });
        }
        edits
    }
}

/// Changes to permissions, Undo/Redo and in-place reloads invalidate the original draft too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Context {
    document: DocId,
    generation: u64,
    editable: bool,
}

impl Context {
    /// Capture the identity, monotonic generation and permission that bind the draft.
    pub(crate) fn read(doc: &Document) -> Self {
        Self { document: doc.id, generation: doc.edit_generation(), editable: doc.allows_modification() }
    }
}

pub(crate) type Draft = EditSession<Values, Context>;

impl PdfCraftApp {
    /// Discard the session, without issuing an engine command. The values are the dialog's.
    pub(crate) fn discard_properties(&mut self) {
        self.props_session = None;
    }

    /// Bind edits to one document revision. Clean inspection tabs may follow same-document
    /// edits, preserving the existing live Fonts/Space Audit behavior without rebasing input.
    pub(crate) fn prepare_properties(&mut self, ctx: &egui::Context) -> bool {
        let Some((fields, shown)) = self.properties_fields() else {
            self.discard_properties();
            return false;
        };
        self.stage_properties(&fields);
        let live = self.active_ids().and_then(|(_, id)| self.session.get(id)).map(|doc| (Values::read(doc), Context::read(doc)));
        let stale = match (&self.props_session, &live) {
            (Some(draft), Some((value, context))) => draft.check_live(value, context).is_err(),
            (Some(_), None) => true,
            (None, _) => false,
        };
        let clean_inspector = matches!(shown, PropsTab::Fonts | PropsTab::Advanced | PropsTab::SpaceAudit | PropsTab::Security)
            && self.props_session.as_ref().is_some_and(|draft| {
                draft.is_valid()
                    && draft.original() == draft.preview()
                    && live.as_ref().is_some_and(|(_, context)| context.document == draft.context().document)
            })
            && fields.text("custom.name").is_none_or(str::is_empty)
            && fields.text("custom.value").is_none_or(str::is_empty);
        if stale && clean_inspector {
            // The live document is the same file: show it as it is now, on the same tab.
            self.rebuild_properties(ctx, shown);
            return true;
        }
        if stale || live.is_none() {
            self.close_properties(ctx);
            self.discard_properties();
            if stale {
                self.notify(tl!("The document changed. Reopen Document Properties to edit the current version.").to_string());
            }
            return false;
        }
        true
    }

    /// Stage the dialog's values as the session's proposal. The Custom tab's Name/Value input boxes
    /// are not accepted properties until Add or Update is pressed.
    pub(crate) fn stage_properties(&mut self, fields: &DialogFields) {
        if let Some(draft) = &mut self.props_session {
            let proposal = Values::from_fields(fields, draft.original()).ok_or(());
            let _ = draft.propose(proposal);
        }
    }

    /// Check and consume once on the serialized UI command path. An engine failure retains
    /// the dialog's values and restores the uncommitted session, so the user can correct them.
    pub(crate) fn finish_properties(&mut self, ctx: &egui::Context) -> bool {
        if !self.prepare_properties(ctx) {
            return true;
        }
        if let Some((fields, _)) = self.properties_fields() {
            self.stage_properties(&fields);
        }
        let Some((live, context)) = self.active_ids().and_then(|(_, id)| self.session.get(id)).map(|doc| (Values::read(doc), Context::read(doc)))
        else {
            return false;
        };
        if self.props_session.as_ref().is_none_or(|draft| draft.check(&live, &context).is_err()) {
            return false;
        }
        let Some(draft) = self.props_session.take() else { return false };
        match draft.finish(&live, &context) {
            Ok(Some(change)) => {
                let edits = change.before.edits_to(&change.after);
                if self.apply_edit(ctx, Edit::Batch { label: "Change document properties".into(), edits }) {
                    true
                } else {
                    // Session::apply is atomic on error; do not rebase to any newer document.
                    let mut retry = EditSession::new(change.before, context);
                    let _ = retry.propose::<()>(Ok(change.after));
                    self.props_session = Some(retry);
                    false
                }
            }
            Ok(None) => true,
            Err(_) => {
                self.close_properties(ctx);
                self.discard_properties();
                self.notify(tl!("The document changed. Reopen Document Properties to edit the current version.").to_string());
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialogs::properties::properties_id;
    use craft::ui::patterns::form_dialogs::HostedParamDialog;

    /// Open a one-page document with its Properties dialog and a pending title edit.
    fn app() -> (PdfCraftApp, egui::Context) {
        let mut app = PdfCraftApp::for_unit_tests();
        let ctx = egui::Context::default();
        let bytes = app.session.create_blank(200.0, 300.0, 1).unwrap();
        app.open_bytes(&egui::Context::default(), "properties.pdf", None, bytes.as_ref().clone()).unwrap();
        app.open_properties(&ctx, PropsTab::Description);
        assert!(app.prepare_properties(&ctx));
        set_field(&mut app, &ctx, "title", json!("Draft title"));
        (app, ctx)
    }

    /// Set one row of the open dialog as automation does.
    fn set_field(app: &mut PdfCraftApp, ctx: &egui::Context, field: &str, value: Value) {
        app.dialogs.set_json_field(ctx, properties_id(), field, value).unwrap();
    }

    fn dialog_open(app: &PdfCraftApp) -> bool {
        app.dialogs.get_by_id::<HostedParamDialog>(properties_id()).is_some()
    }

    #[test]
    /// A concurrent edit must be rejected at commit even before another UI frame checks freshness.
    fn commit_rechecks_revision_without_an_intervening_frame() {
        let (mut app, ctx) = app();
        let (_, id) = app.active_ids().unwrap();
        app.session.apply(id, Edit::SetInfo { key: "Title".into(), value: "Newer title".into() }).unwrap();
        let generation = app.session.get(id).unwrap().edit_generation();
        assert!(app.finish_properties(&ctx));
        let doc = app.session.get(id).unwrap();
        assert_eq!(doc.info_value("Title").as_deref(), Some("Newer title"));
        assert_eq!(doc.edit_generation(), generation);
        assert!(app.props_session.is_none());
        assert!(!dialog_open(&app));
    }

    #[test]
    /// An invalid view must roll back the title too, retain input, and permit one corrected undo step.
    fn engine_refusal_retains_draft_and_retry_is_one_atomic_edit() {
        let (mut app, ctx) = app();
        let (_, id) = app.active_ids().unwrap();
        let original_bytes = app.session.get(id).unwrap().bytes.clone();
        let generation = app.session.get(id).unwrap().edit_generation();
        // The page row has no upper bound of its own; the engine rejects the entire title + view batch atomically.
        set_field(&mut app, &ctx, "page", json!(9_000_000_000_000_i64));
        assert!(!app.finish_properties(&ctx));
        let doc = app.session.get(id).unwrap();
        assert_eq!(doc.bytes, original_bytes);
        assert_eq!(doc.info_value("Title"), None);
        assert_eq!(doc.edit_generation(), generation);
        assert!(!doc.dirty && doc.can_undo().is_none());
        let (fields, _) = app.properties_fields().unwrap();
        assert_eq!(fields.text("title"), Some("Draft title"));
        assert_eq!(fields.int("page"), Some(9_000_000_000_000));
        assert!(app.props_session.is_some());

        set_field(&mut app, &ctx, "page", json!(1));
        assert!(app.finish_properties(&ctx));
        assert_eq!(app.session.get(id).unwrap().info_value("Title").as_deref(), Some("Draft title"));
        assert_eq!(app.session.get(id).unwrap().can_undo(), Some("Change document properties"));
        app.session.undo(id).unwrap();
        assert_eq!(app.session.get(id).unwrap().info_value("Title"), None);
        assert!(app.session.get(id).unwrap().can_undo().is_none());
    }

    #[test]
    /// A session opened on another document cannot commit while that document is the active one.
    fn buffers_from_another_document_cannot_commit() {
        let (mut app, ctx) = app();
        let (_, id) = app.active_ids().unwrap();
        let bytes = app.session.create_blank(200.0, 300.0, 1).unwrap();
        app.open_bytes(&egui::Context::default(), "other.pdf", None, bytes.as_ref().clone()).unwrap();
        let (_, other) = app.active_ids().unwrap();
        assert_ne!(id, other);
        assert!(app.finish_properties(&ctx));
        for doc_id in [id, other] {
            let doc = app.session.get(doc_id).unwrap();
            assert_eq!(doc.info_value("Title"), None);
            assert!(!doc.dirty && doc.can_undo().is_none());
        }
    }
}
