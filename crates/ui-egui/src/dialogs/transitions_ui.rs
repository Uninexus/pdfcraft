//! Page Transitions (Acrobat: Organize pages ▸ Page transitions): how the chosen pages come on in full screen,
//! and whether the presentation moves on by itself.
//!
//! Each style offers only the directions ISO 32000-2 table 164 gives it. OK writes what the dialog shows to the
//! chosen pages as one undo step; "No transition" with the automatic turn off removes both.

use craft::pages::ui::{PageRangeChoices, PageRangeOptions};
use craft::pages::{PageRange, PageScope};
use craft::ui::contract::Bind;
use craft::ui::dialog::dialog_id;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
use pdfcraft_engine::{Edit, Transition, TransitionSpec};

use crate::PdfCraftApp;

/// The dialog's key.
const KEY: &str = "page.transitions";

/// The row value that means the page has no `/Trans`.
const NONE: &str = "none";

/// The longest time a transition or a page's display time may be set to here, in seconds.
const MAX_SECONDS: f64 = 3600.0;

/// The styles offered: the `/S` name (or [`NONE`]) and what the row calls it.
fn styles() -> Vec<(&'static str, &'static str)> {
    vec![
        (NONE, tl!("No transition")),
        ("Split", tl!("Split")),
        ("Blinds", tl!("Blinds")),
        ("Box", tl!("Box")),
        ("Wipe", tl!("Wipe")),
        ("Dissolve", tl!("Dissolve")),
        ("Glitter", tl!("Glitter")),
        ("R", tl!("Replace")),
        ("Fly", tl!("Fly")),
        ("Push", tl!("Push")),
        ("Cover", tl!("Cover")),
        ("Uncover", tl!("Uncover")),
        ("Fade", tl!("Fade")),
    ]
}

/// The `/Di` values `style` may take, as row ids; empty when it has no direction. [`NONE`] is Fly's `/None`.
fn directions(style: &str) -> &'static [&'static str] {
    match style {
        "Wipe" => &["0", "90", "180", "270"],
        "Glitter" => &["0", "270", "315"],
        "Fly" => &["0", "270", NONE],
        "Push" | "Cover" | "Uncover" => &["0", "270"],
        _ => &[],
    }
}

fn direction_label(id: &str) -> &'static str {
    match id {
        "90" => tl!("Bottom to top"),
        "180" => tl!("Right to left"),
        "270" => tl!("Top to bottom"),
        "315" => tl!("From the top left corner"),
        NONE => tl!("None"),
        _ => tl!("Left to right"),
    }
}

/// The rows for the style chosen, in a document of `count` pages with `current` on screen; `selected` is whether
/// pages are selected, so the range offers them.
fn params(style: &str, count: usize, current: usize, selected: bool) -> Vec<Param> {
    let choice = |options: Vec<(&str, &str)>, default: &str| {
        ParamKind::choice(options.into_iter().map(|(id, label)| (id.to_string(), label.to_string().into())).collect(), Some(default.to_string()))
    };
    let mut rows = vec![Param::new("style", tl!("Transition"), choice(styles(), NONE)).resets(["direction"])];
    let ids = directions(style);
    if let Some(first) = ids.first() {
        rows.push(Param::new("direction", tl!("Direction"), choice(ids.iter().map(|id| (*id, direction_label(id))).collect(), first)));
    }
    if matches!(style, "Split" | "Blinds") {
        rows.push(Param::new("dimension", tl!("Orientation"), choice(vec![("h", tl!("Horizontal")), ("v", tl!("Vertical"))], "h")));
    }
    if matches!(style, "Split" | "Box" | "Fly") {
        rows.push(Param::new("motion", tl!("Motion"), choice(vec![("in", tl!("Inward")), ("out", tl!("Outward"))], "in")));
    }
    if style != NONE && style != "R" {
        rows.push(Param::number("duration", tl!("Transition time (seconds)"), 1.0, 0.1, 2).bounds(Some(0.0), Some(MAX_SECONDS)));
    }
    rows.push(Param::new("auto", tl!("Turn the page automatically"), ParamKind::bool(false)));
    rows.push(
        Param::number("display", tl!("Seconds on each page"), 5.0, 1.0, 2).bounds(Some(0.0), Some(MAX_SECONDS)).enabled_when_in("auto", [true]),
    );
    let choices = if selected { PageRangeChoices::SelectedFromTo } else { PageRangeChoices::CurrentFromTo };
    rows.push(PageRangeOptions::param("range", tl!("Pages"), count, current, choices));
    rows
}

/// What the rows of `f` ask for. `kept` supplies what the dialog has no row for (the Fly scale and backdrop).
fn spec_of(f: &DialogFields, kept: Option<&Transition>) -> TransitionSpec {
    let style = f.text("style").unwrap_or(NONE);
    let transition = (style != NONE).then(|| {
        let mut t = Transition::new(style);
        if let Some(k) = kept.filter(|k| k.style == style) {
            (t.scale, t.opaque) = (k.scale, k.opaque);
        }
        if let Some(seconds) = f.number("duration") {
            t.duration = seconds.clamp(0.0, MAX_SECONDS) as f32;
        }
        t.horizontal = f.text("dimension") != Some("v");
        t.inward = f.text("motion") != Some("out");
        let ids = directions(style);
        let id = f.text("direction").filter(|id| ids.contains(id)).or(ids.first().copied());
        // A style without a direction leaves the default, which writes no entry that matters to it.
        t.direction = id.map_or(Some(0), |id| id.parse().ok());
        t
    });
    let display = (f.bool("auto") == Some(true)).then(|| f.number("display").unwrap_or(5.0).clamp(0.0, MAX_SECONDS) as f32);
    TransitionSpec { transition, display }
}

/// The rows' values for what a page has now.
fn fields_of(transition: Option<&Transition>, display: Option<f32>) -> DialogFields {
    let mut f = DialogFields::new();
    f.set("style", transition.map_or(NONE, |t| t.style.as_str()).to_string());
    if let Some(t) = transition {
        let id = t.direction.map_or_else(|| NONE.to_string(), |d| d.to_string());
        if directions(&t.style).contains(&id.as_str()) {
            f.set("direction", id);
        }
        f.set("dimension", if t.horizontal { "h" } else { "v" });
        f.set("motion", if t.inward { "in" } else { "out" });
        f.set("duration", f64::from(t.duration));
    }
    f.set("auto", display.is_some());
    f.set("display", display.map_or(5.0, f64::from));
    f
}

impl PdfCraftApp {
    /// Opens Page Transitions over the active document, showing what the first target page has now.
    pub(crate) fn open_page_transitions(&mut self, ctx: &egui::Context) {
        let Some((i, id)) = self.active_ids() else { return };
        let view = &self.views[i];
        let current = view.current();
        let selected = !view.selected.items.is_empty();
        let first = view.target_pages().first().copied().unwrap_or(current);
        let Some(doc) = self.session.get(id) else { return };
        let count = doc.info.pages.len().max(1);
        let has = doc.info.transitions.iter().find(|t| t.page == first);
        let transition = has.and_then(|t| {
            let style = t.style?;
            Some(Transition {
                style: style.name().to_string(),
                duration: t.duration,
                horizontal: t.horizontal,
                inward: t.inward,
                direction: t.direction,
                scale: t.scale,
                opaque: t.opaque,
            })
        });
        let mut fields = fields_of(transition.as_ref(), has.and_then(|t| t.display));
        let scope = if selected { PageScope::Selection } else { PageScope::All };
        fields.set("range", PageRangeOptions::field_value(&PageRange { scope, ..PageRange::default() }));
        self.transition_seed = transition;
        let dialog = HostedParamDialog::new(ctx, KEY, fields, move |fields| {
            let rows = params(fields.text("style").unwrap_or(NONE), count, current, selected);
            ParamDialog::new(KEY, tl!("Page Transitions"), rows).values(Bind::bound(fields))
        });
        crate::dialogs::host::open(self, dialog);
    }

    /// Writes the transitions when the dialog was answered with OK.
    pub(crate) fn page_transitions_pass(&mut self, ctx: &egui::Context) {
        let Some(answer) = self.dialogs.take_by_id::<HostedParamDialog>(dialog_id(KEY)) else { return };
        let seed = self.transition_seed.take();
        let Some(result) = answer.ok() else { return };
        let Some((i, id)) = self.active_ids() else { return };
        let count = self.session.get(id).map_or(0, |d| d.info.pages.len());
        let view = &self.views[i];
        let range = PageRangeOptions::value(&result.fields, "range").unwrap_or_default();
        let pages = range.pages(count, view.current(), &view.selected.items);
        if pages.is_empty() {
            return;
        }
        let spec = spec_of(&result.fields, seed.as_ref());
        self.apply_edit(ctx, Edit::SetTransition { pages, spec });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_become_the_transition_and_display_time_they_show() {
        let nothing = spec_of(&DialogFields::new(), None);
        assert_eq!(nothing, TransitionSpec { transition: None, display: None });
        let mut f = DialogFields::new();
        f.set("style", "Wipe".to_string());
        f.set("direction", "270".to_string());
        f.set("duration", 2.5);
        f.set("auto", true);
        f.set("display", 7.0);
        let spec = spec_of(&f, None);
        let t = spec.transition.expect("a transition");
        assert_eq!((t.style.as_str(), t.direction, t.duration, spec.display), ("Wipe", Some(270), 2.5, Some(7.0)));
        // A direction the style doesn't have gives way to its first; Fly's None writes /None.
        f.set("style", "Push".to_string());
        f.set("direction", "90".to_string());
        assert_eq!(spec_of(&f, None).transition.unwrap().direction, Some(0));
        f.set("style", "Fly".to_string());
        f.set("direction", NONE.to_string());
        let kept = Transition { scale: 0.5, opaque: true, ..Transition::new("Fly") };
        let fly = spec_of(&f, Some(&kept)).transition.unwrap();
        assert_eq!((fly.direction, fly.scale, fly.opaque), (None, 0.5, true));
    }

    #[test]
    fn a_pages_transition_reads_back_into_the_rows_unchanged() {
        let t = Transition { horizontal: false, inward: false, duration: 0.5, ..Transition::new("Split") };
        let spec = spec_of(&fields_of(Some(&t), Some(3.0)), Some(&t));
        assert_eq!(spec.display, Some(3.0));
        let back = spec.transition.unwrap();
        assert_eq!((back.style.as_str(), back.horizontal, back.inward, back.duration), ("Split", false, false, 0.5));
    }
}
