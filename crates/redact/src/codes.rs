//! Redaction code sets (Acrobat's Redaction Properties ▸ Redaction Code): the exemption codes
//! that justify a redaction, used as its overlay text. The built-in sets are the exemptions of
//! the U.S. Freedom of Information Act (5 U.S.C. 552(b)) and the U.S. Privacy Act
//! (5 U.S.C. 552a(d), (j) and (k)).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeSet {
    pub id: &'static str,
    pub name: &'static str,
    pub codes: &'static [&'static str],
}

pub const CODE_SETS: [CodeSet; 2] = [
    CodeSet {
        id: "foia",
        name: "U.S. FOIA",
        codes: &[
            "(b)(1)(A)",
            "(b)(1)(B)",
            "(b)(2)",
            "(b)(3)(A)",
            "(b)(3)(B)",
            "(b)(4)",
            "(b)(5)",
            "(b)(6)",
            "(b)(7)(A)",
            "(b)(7)(B)",
            "(b)(7)(C)",
            "(b)(7)(D)",
            "(b)(7)(E)",
            "(b)(7)(F)",
            "(b)(8)",
            "(b)(9)",
        ],
    },
    CodeSet {
        id: "privacy-act",
        name: "U.S. Privacy Act",
        codes: &["(d)(5)", "(j)(1)", "(j)(2)", "(k)(1)", "(k)(2)", "(k)(3)", "(k)(4)", "(k)(5)", "(k)(6)", "(k)(7)"],
    },
];

impl CodeSet {
    pub fn from_id(id: &str) -> Option<Self> {
        CODE_SETS.into_iter().find(|s| s.id == id)
    }

    /// The overlay text for the picked codes: in the set's order, once each, joined by ", ".
    /// Fails with the first picked code that isn't in the set.
    pub fn overlay<'a>(&self, picked: &[&'a str]) -> Result<String, &'a str> {
        overlay_of(self.codes, picked)
    }
}

fn overlay_of<'a, S: AsRef<str>>(codes: &[S], picked: &[&'a str]) -> Result<String, &'a str> {
    if let Some(bad) = picked.iter().find(|p| !codes.iter().any(|c| c.as_ref() == **p)) {
        return Err(bad);
    }
    Ok(codes.iter().map(AsRef::as_ref).filter(|c| picked.contains(c)).collect::<Vec<_>>().join(", "))
}

/// The longest id a custom code set may have, in characters.
pub const CUSTOM_ID_MAX: usize = 64;
/// The longest name a custom code set may have, in characters.
pub const CUSTOM_NAME_MAX: usize = 128;
/// The most codes a custom code set may hold.
pub const CUSTOM_CODES_MAX: usize = 256;
/// The longest code a custom code set may hold, in characters.
pub const CUSTOM_CODE_MAX: usize = 128;
/// The most bytes all the codes of a custom code set may take together.
pub const CUSTOM_TOTAL_MAX: usize = 16 * 1024;

fn undrawable(c: &char) -> String {
    pdfcraft_fonts::undrawable_message("a redaction code", *c)
}

/// Invisible formatting characters (Unicode category Cf, plus the line and paragraph separators)
/// that `char::is_control` lets through but that make two codes look alike or reorder their text.
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{AD}' | '\u{61C}' | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}' | '\u{FEFF}')
}

fn is_unseen(c: char) -> bool {
    c.is_control() || is_invisible(c)
}

/// Why a custom code set was refused. Each message names the offending field and says what is allowed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CodeSetError {
    #[error("id must be 1 to {CUSTOM_ID_MAX} characters of a-z, 0-9 and hyphen, with at least one letter or digit, got {0:?}")]
    BadId(String),
    #[error("id {0:?} is a built-in code set; choose another id")]
    BuiltInId(String),
    #[error(
        "name must be 1 to {CUSTOM_NAME_MAX} visible characters, with no control or invisible formatting characters (zero-width, bidirectional, soft hyphen, line separators)"
    )]
    BadName,
    #[error("codes must hold 1 to {CUSTOM_CODES_MAX} entries, got {0}")]
    CodeCount(usize),
    #[error(
        "codes[{0}] must be 1 to {CUSTOM_CODE_MAX} characters with no leading or trailing space, no line breaks, and no control or invisible formatting characters"
    )]
    BadCode(usize),
    #[error("codes[{index}]: {}", undrawable(.ch))]
    Undrawable { index: usize, ch: char },
    #[error("codes[{dup}] repeats codes[{first}]; each code may appear once")]
    DuplicateCode { first: usize, dup: usize },
    #[error("codes hold {0} bytes together; at most {CUSTOM_TOTAL_MAX} are allowed")]
    TooLarge(usize),
}

/// A code set a user defines (Acrobat's Redaction Properties ▸ Redaction Code ▸ Edit Codes). Built
/// only through [`CustomCodeSet::new`], which validates it, so a held value is always well formed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomCodeSet {
    id: String,
    name: String,
    codes: Vec<String>,
}

impl CustomCodeSet {
    pub fn new(id: &str, name: &str, codes: &[&str]) -> Result<Self, CodeSetError> {
        let id_ok = !id.is_empty()
            && id.len() <= CUSTOM_ID_MAX
            && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && id.bytes().any(|b| b != b'-');
        if !id_ok {
            return Err(CodeSetError::BadId(id.chars().take(CUSTOM_ID_MAX).collect()));
        }
        if CodeSet::from_id(id).is_some() {
            return Err(CodeSetError::BuiltInId(id.to_owned()));
        }
        let name_len = name.chars().count();
        if name_len == 0 || name_len > CUSTOM_NAME_MAX || name.chars().any(is_unseen) || name.trim().is_empty() {
            return Err(CodeSetError::BadName);
        }
        if codes.is_empty() || codes.len() > CUSTOM_CODES_MAX {
            return Err(CodeSetError::CodeCount(codes.len()));
        }
        let mut total = 0usize;
        for (i, c) in codes.iter().enumerate() {
            let len = c.chars().count();
            if len == 0 || len > CUSTOM_CODE_MAX || c.chars().any(is_unseen) || c.trim() != *c || c.is_empty() {
                return Err(CodeSetError::BadCode(i));
            }
            // The overlay is drawn in a standard font; a character it can't draw would become "?".
            if let Some(ch) = pdfcraft_fonts::first_undrawable(c) {
                return Err(CodeSetError::Undrawable { index: i, ch });
            }
            if let Some(first) = codes.iter().take(i).position(|p| p == c) {
                return Err(CodeSetError::DuplicateCode { first, dup: i });
            }
            total = total.saturating_add(c.len());
        }
        if total > CUSTOM_TOTAL_MAX {
            return Err(CodeSetError::TooLarge(total));
        }
        Ok(Self { id: id.to_owned(), name: name.to_owned(), codes: codes.iter().map(|c| (*c).to_owned()).collect() })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn codes(&self) -> &[String] {
        &self.codes
    }

    /// The overlay text for the picked codes, as [`CodeSet::overlay`] gives it: in the set's order,
    /// once each, joined by ", ". Fails with the first picked code that isn't in the set.
    pub fn overlay<'a>(&self, picked: &[&'a str]) -> Result<String, &'a str> {
        overlay_of(&self.codes, picked)
    }
}
