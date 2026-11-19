//! Auto-complete from previous entries (Preferences ▸ Forms ▸ Auto-Complete).
//!
//! A short list of the text people typed into fill-in form fields, offered back as a
//! suggestion when they start typing in another field. It lives below the viewer so the
//! desktop app and the headless tools share one set of rules. Nothing here touches the disk: the
//! app keeps the list in its own settings, and a headless session keeps it for the session only.
//!
//! What is never remembered: a password, signature, comb, file-select, multi-line, rich-text, read-only, calculated,
//! formatted (number, date, time, special) or validated field, or anything but a text field; and
//! text that is empty, longer than [`MAX_ENTRY_CHARS`] or has a control character in it (a
//! multi-line answer is not a value to suggest again). Every list is capped at [`MAX_ENTRIES`],
//! so a hostile settings file cannot make it grow.

use pdfcraft_forms::{Field, FieldKind, flags};

/// The most entries kept; the oldest go first.
pub const MAX_ENTRIES: usize = 200;
/// The longest entry kept, in characters.
pub const MAX_ENTRY_CHARS: usize = 200;
/// The most suggestions [`FormEntries::suggest`] returns.
pub const MAX_SUGGESTIONS: usize = 8;

/// The remembered entries, most recent first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormEntries {
    entries: Vec<String>,
}

/// Whether text typed into `field` may be remembered. See the module notes for what is not.
pub fn rememberable(field: &Field) -> bool {
    const SENSITIVE: u32 = flags::PASSWORD | flags::FILE_SELECT | flags::COMB | flags::READ_ONLY | flags::MULTILINE | flags::RICH_TEXT;
    field.kind == FieldKind::Text
        && field.flags & SENSITIVE == 0
        && field.widgets.iter().all(|w| !w.read_only)
        && field.actions.is_empty()
        && field.actions.unsupported.is_empty()
}

/// Whether `text` is a value worth remembering on its own.
fn acceptable(text: &str) -> bool {
    !text.trim().is_empty() && text.chars().count() <= MAX_ENTRY_CHARS && !text.chars().any(char::is_control)
}

impl FormEntries {
    /// The entries, most recent first.
    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// How many entries are kept.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Forget everything.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Forget one entry (exactly as written). Returns whether it was there.
    pub fn remove(&mut self, text: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e != text);
        self.entries.len() != before
    }

    /// Remember `text` typed into `field`, if the field and the text allow it. It goes first;
    /// an earlier copy (the same text, case included) is dropped. Returns whether it was kept.
    pub fn remember(&mut self, field: &Field, text: &str) -> bool {
        if !rememberable(field) {
            return false;
        }
        self.push(text)
    }

    fn push(&mut self, text: &str) -> bool {
        let text = text.trim();
        if !acceptable(text) {
            return false;
        }
        self.entries.retain(|e| e != text);
        self.entries.insert(0, text.to_owned());
        self.entries.truncate(MAX_ENTRIES);
        true
    }

    /// A list restored from stored text (most recent first). Anything that is not an acceptable
    /// entry is skipped, repeats are dropped and the list is cut to [`MAX_ENTRIES`], so a
    /// corrupt or hostile file gives a smaller list, never an error.
    pub fn restored<I, S>(stored: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut out = Self::default();
        // Bounded by the cap on what is read, not on what is kept: a file of a million repeats
        // stops here.
        for s in stored.into_iter().take(MAX_ENTRIES.saturating_mul(4)) {
            let s = s.as_ref().trim();
            if acceptable(s) && !out.entries.iter().any(|e| e == s) && out.entries.len() < MAX_ENTRIES {
                out.entries.push(s.to_owned());
            }
        }
        out
    }

    /// The remembered entries that begin with `typed` (ignoring case), most recent first, at
    /// most [`MAX_SUGGESTIONS`]. Nothing for empty text, and an entry equal to what is typed
    /// is not offered back.
    pub fn suggest(&self, typed: &str) -> Vec<&str> {
        // Longer than any entry can be: nothing starts with it.
        if typed.is_empty() || typed.chars().nth(MAX_ENTRY_CHARS).is_some() {
            return Vec::new();
        }
        let want: Vec<char> = typed.chars().flat_map(char::to_lowercase).collect();
        self.entries
            .iter()
            .map(String::as_str)
            .filter(|e| *e != typed)
            .filter(|e| e.chars().flat_map(char::to_lowercase).take(want.len()).eq(want.iter().copied()))
            .take(MAX_SUGGESTIONS)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdfcraft_cos::ObjRef;

    fn field(kind: FieldKind, flags: u32) -> Field {
        Field {
            name: "f".into(),
            obj: ObjRef::new(1, 0),
            kind,
            value: Vec::new(),
            default: Vec::new(),
            flags,
            max_len: None,
            options: Vec::new(),
            selected: Vec::new(),
            single_value_is_array: false,
            options_omitted: false,
            value_omitted: false,
            da: String::new(),
            quadding: 0,
            tooltip: None,
            widgets: Vec::new(),
            actions: Default::default(),
            button: None,
        }
    }

    #[test]
    fn newest_first_deduplicated_and_capped() {
        let f = field(FieldKind::Text, 0);
        let mut e = FormEntries::default();
        assert!(e.remember(&f, "Ada"));
        assert!(e.remember(&f, "Grace"));
        assert!(e.remember(&f, "Ada"));
        assert_eq!(e.entries(), ["Ada", "Grace"]);
        for i in 0..MAX_ENTRIES + 20 {
            e.remember(&f, &format!("entry {i}"));
        }
        assert_eq!(e.len(), MAX_ENTRIES);
        assert_eq!(e.entries()[0], format!("entry {}", MAX_ENTRIES + 19));
        assert!(!e.entries().iter().any(|s| s == "Ada"), "the oldest went first");
    }

    #[test]
    fn sensitive_and_computed_fields_are_never_remembered() {
        let mut e = FormEntries::default();
        for (kind, fl) in [
            (FieldKind::Text, flags::PASSWORD),
            (FieldKind::Text, flags::FILE_SELECT),
            (FieldKind::Text, flags::COMB),
            (FieldKind::Text, flags::MULTILINE),
            (FieldKind::Text, flags::RICH_TEXT),
            (FieldKind::Text, flags::READ_ONLY),
            (FieldKind::Signature, 0),
            (FieldKind::Combo, 0),
            (FieldKind::CheckBox, 0),
        ] {
            assert!(!e.remember(&field(kind, fl), "secret"), "{kind:?} {fl}");
        }
        let mut formatted = field(FieldKind::Text, 0);
        formatted.actions.format = pdfcraft_forms::af::Format::Date("mm/dd/yyyy".into());
        assert!(!e.remember(&formatted, "01/02/2003"));
        assert!(e.is_empty());
    }

    #[test]
    fn unsuitable_text_is_refused() {
        let f = field(FieldKind::Text, 0);
        let mut e = FormEntries::default();
        for text in ["", "   ", "two\nlines", "tab\there", &"x".repeat(MAX_ENTRY_CHARS + 1)] {
            assert!(!e.remember(&f, text), "{text:?}");
        }
        assert!(e.remember(&f, &"x".repeat(MAX_ENTRY_CHARS)));
    }

    #[test]
    fn restoring_survives_hostile_input() {
        let many: Vec<String> = (0..10_000).map(|i| format!("e{i}")).collect();
        assert_eq!(FormEntries::restored(&many).len(), MAX_ENTRIES);
        let junk = ["ok", "ok", "", "a\u{0}b", &"y".repeat(5000), "  padded  "];
        assert_eq!(FormEntries::restored(junk).entries(), ["ok", "padded"]);
        assert!(FormEntries::restored(std::iter::repeat_n("same", 1_000_000)).len() == 1);
    }

    #[test]
    fn suggestions_match_a_prefix_ignoring_case() {
        let f = field(FieldKind::Text, 0);
        let mut e = FormEntries::default();
        for t in ["Maple Street", "main road", "Oak Avenue", "MAPLE"] {
            e.remember(&f, t);
        }
        assert_eq!(e.suggest("ma"), ["MAPLE", "main road", "Maple Street"]);
        assert_eq!(e.suggest("MAPLE"), ["Maple Street"], "an exact copy is not offered back");
        assert!(e.suggest("").is_empty());
        assert!(e.suggest("zzz").is_empty());
        assert!(e.suggest("treet").is_empty(), "a prefix, not a substring");
        assert!(e.suggest(&"m".repeat(500)).is_empty());
    }

    #[test]
    fn remove_forgets_one_entry() {
        let f = field(FieldKind::Text, 0);
        let mut e = FormEntries::default();
        e.remember(&f, "a");
        e.remember(&f, "b");
        assert!(e.remove("a") && !e.remove("a"));
        assert_eq!(e.entries(), ["b"]);
    }
}
