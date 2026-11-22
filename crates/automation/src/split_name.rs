//! `doc_split`'s `name_template`: parsing it, filling it in for a part, and making the result a safe file name.

use crate::error::{Result, ToolError};

/// Most bytes a part's name may have before ".pdf" and a "-N" suffix; a file name is at most 255.
pub(crate) const MAX_BASE_BYTES: usize = 200;

/// Longest `name_template` text, in bytes; a longer one is refused.
pub(crate) const MAX_TEMPLATE_BYTES: usize = 1000;

/// How much of an expanded template is kept before it is made a file name (the name is cut to 200 bytes anyway).
const MAX_EXPANDED_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Text(String),
    Name,
    Bookmark,
    Number,
    First,
    Last,
}

/// A parsed `name_template`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Template(Vec<Piece>);

/// What a part's placeholders stand for.
pub(crate) struct PartInfo<'a> {
    pub name: &'a str,
    /// The top-level bookmark whose page is the part's first page; empty when there is none.
    pub bookmark: &'a str,
    pub number: usize,
    pub first: usize,
    pub last: usize,
}

impl Template {
    /// Placeholders are `{name}`, `{bookmark}`, `{n}`, `{first}` and `{last}`; `{{` and `}}` are a literal brace.
    pub(crate) fn parse(text: &str) -> Result<Self> {
        if text.len() > MAX_TEMPLATE_BYTES {
            return Err(ToolError::InvalidArgs(format!("name_template is longer than {MAX_TEMPLATE_BYTES} bytes")));
        }
        let mut pieces = Vec::new();
        let mut literal = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '{' if chars.peek() == Some(&'{') => {
                    chars.next();
                    literal.push('{');
                }
                '}' if chars.peek() == Some(&'}') => {
                    chars.next();
                    literal.push('}');
                }
                '{' => {
                    let mut word = String::new();
                    loop {
                        match chars.next() {
                            Some('}') => break,
                            Some(c) if word.chars().count() < 16 => word.push(c),
                            _ => return Err(ToolError::InvalidArgs("name_template has a { without a closing }".into())),
                        }
                    }
                    let piece = match word.as_str() {
                        "name" => Piece::Name,
                        "bookmark" => Piece::Bookmark,
                        "n" => Piece::Number,
                        "first" => Piece::First,
                        "last" => Piece::Last,
                        _ => {
                            return Err(ToolError::InvalidArgs(format!(
                                "name_template has an unknown placeholder {{{word}}}; use {{name}}, {{bookmark}}, {{n}}, {{first}} or {{last}}"
                            )));
                        }
                    };
                    if !literal.is_empty() {
                        pieces.push(Piece::Text(std::mem::take(&mut literal)));
                    }
                    pieces.push(piece);
                }
                '}' => return Err(ToolError::InvalidArgs("name_template has a } without an opening {; write }} for a literal brace".into())),
                c => literal.push(c),
            }
        }
        if !literal.is_empty() {
            pieces.push(Piece::Text(literal));
        }
        Ok(Self(pieces))
    }

    /// Whether the template uses `{bookmark}`.
    pub(crate) fn uses_bookmark(&self) -> bool {
        self.0.contains(&Piece::Bookmark)
    }

    /// The file name for a part, without ".pdf", or `None` when nothing usable is left (the caller then uses the default name).
    pub(crate) fn apply(&self, part: &PartInfo) -> Option<String> {
        let mut out = String::new();
        let last_piece = self.0.len().saturating_sub(1);
        for (at, piece) in self.0.iter().enumerate() {
            let number;
            let text: &str = match piece {
                // A template that ends in ".pdf" (also "x.pdf." or "x.pdf .") does not get a second one: only the
                // template's own closing text is looked at, never a {bookmark} or {name} value.
                Piece::Text(t) if at == last_piece => without_pdf(t),
                Piece::Text(t) => t,
                Piece::Name => part.name,
                Piece::Bookmark => part.bookmark,
                Piece::Number => {
                    number = part.number.to_string();
                    &number
                }
                Piece::First => {
                    number = part.first.to_string();
                    &number
                }
                Piece::Last => {
                    number = part.last.to_string();
                    &number
                }
            };
            // A title can be any length: what is past the cap could not be part of the name anyway.
            let room = MAX_EXPANDED_BYTES.saturating_sub(out.len());
            match text.char_indices().find(|(i, c)| i + c.len_utf8() > room) {
                Some((end, _)) => {
                    out.push_str(&text[..end]);
                    break;
                }
                None => out.push_str(text),
            }
        }
        sanitize(&out)
    }
}

/// `text` without a closing `.pdf` (any case) and the dots and spaces around it.
fn without_pdf(text: &str) -> &str {
    let t = text.trim_end_matches(['.', ' ']);
    match t.len().checked_sub(4).filter(|at| t.is_char_boundary(*at)) {
        Some(at) if t[at..].eq_ignore_ascii_case(".pdf") => t[..at].trim_end_matches(['.', ' ']),
        _ => text,
    }
}

/// The key that decides whether two part names are the same file: the same on a system that ignores case.
pub(crate) fn fold_key(name: &str) -> String {
    // Upper then lower, so that σ and ς, which only share an upper case, fold together.
    name.to_uppercase().to_lowercase()
}

/// Whether a Windows device name (CON, PRN, AUX, NUL, COM0-9, LPT0-9 and COM and LPT with a superscript digit 1-3)
/// is the part of `name` before its first dot, ignoring case and trailing spaces.
fn is_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").trim_end().to_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let Some(digit) = stem.strip_prefix("COM").or_else(|| stem.strip_prefix("LPT")) else { return false };
    let mut chars = digit.chars();
    matches!((chars.next(), chars.next()), (Some('0'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'), None))
}

/// `name` with a `_` in front when it is a Windows device name, which no file can be called on Windows
/// even with an extension (`nul.x-part1` is one too).
pub(crate) fn guard_device_name(name: &str) -> String {
    if is_device_name(name) { format!("_{name}") } else { name.to_owned() }
}

/// A file name part (no extension) made safe on every system, or `None` when nothing is left.
///
/// Characters other than letters, digits, space and `-_.,()` become `_` (that takes every path
/// separator, `:` and control character); every `..` becomes `_`, repeated until none is left (so `...`
/// becomes `_.`); leading and trailing dots and spaces go (Windows drops trailing ones, and a leading
/// dot hides the file); a Windows device name (see [`guard_device_name`]) gets a `_` in front; and the
/// result is cut to [`MAX_BASE_BYTES`] bytes between characters, device-name check included.
pub(crate) fn sanitize(text: &str) -> Option<String> {
    // Dots and spaces at the ends go first, so a name of nothing but dots is empty, not `_`.
    let text = text.trim_matches(|c| c == '.' || c == ' ');
    let mut s: String = text.chars().map(|c| if c.is_alphanumeric() || " -_.,()".contains(c) { c } else { '_' }).collect();
    while s.contains("..") {
        s = s.replace("..", "_");
    }
    let mut s = guard_device_name(s.trim_matches(|c| c == '.' || c == ' '));
    if let Some((end, _)) = s.char_indices().find(|(at, c)| at + c.len_utf8() > MAX_BASE_BYTES) {
        s.truncate(end);
    }
    // The cut can leave dots or spaces at the end, and then a device name (`CON` plus spaces).
    let s = guard_device_name(s.trim_matches(|c| c == '.' || c == ' '));
    if s.is_empty() { None } else { Some(s) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part<'a>(bookmark: &'a str) -> PartInfo<'a> {
        PartInfo { name: "report", bookmark, number: 2, first: 5, last: 9 }
    }

    #[test]
    fn placeholders_are_filled_in() {
        let t = Template::parse("{name} {n} {bookmark} p{first}-{last} {{x}}").unwrap();
        assert_eq!(t.apply(&part("Intro")).as_deref(), Some("report 2 Intro p5-9 _x_"));
    }

    #[test]
    fn a_bad_template_is_refused() {
        for bad in ["{oops}", "{n", "}", "{", "{{}", "{nnnnnnnnnnnnnnnnnnnnnnnn}"] {
            assert!(matches!(Template::parse(bad), Err(ToolError::InvalidArgs(_))), "{bad}");
        }
    }

    #[test]
    fn unsafe_names_are_made_safe() {
        let cases: &[(&str, Option<&str>)] = &[
            ("../../etc/passwd", Some("___etc_passwd")),
            (r"a\b/c:d", Some("a_b_c_d")),
            ("a\u{0}b\u{7}c\n", Some("a_b_c_")),
            ("a...b", Some("a_.b")),
            ("a..b...c", Some("a_b_.c")),
            ("..", None),
            ("  ..  ", None),
            ("", None),
            ("name. . ", Some("name")),
            (".hidden", Some("hidden")),
            ("con", Some("_con")),
            ("CON.txt", Some("_CON.txt")),
            ("Aux", Some("_Aux")),
            ("nul ", Some("_nul")),
            ("COM1", Some("_COM1")),
            ("lpt9", Some("_lpt9")),
            ("COM0", Some("_COM0")),
            ("lpt0.txt", Some("_lpt0.txt")),
            ("COM\u{b2}", Some("_COM\u{b2}")),
            ("LPT\u{b3}.x", Some("_LPT\u{b3}.x")),
            ("COM10", Some("COM10")),
            ("COM\u{2074}", Some("COM\u{2074}")),
            ("console", Some("console")),
        ];
        for (input, want) in cases {
            assert_eq!(sanitize(input).as_deref(), *want, "{input:?}");
        }
    }

    #[test]
    fn a_long_name_is_cut_between_characters() {
        let s = sanitize(&"\u{e9}".repeat(300)).unwrap();
        assert!(s.len() <= MAX_BASE_BYTES && s.chars().all(|c| c == '\u{e9}'));
        // A cut that ends in a dot or space is trimmed.
        let s = sanitize(&format!("{} {}", "a".repeat(MAX_BASE_BYTES - 1), "b")).unwrap();
        assert_eq!(s, "a".repeat(MAX_BASE_BYTES - 1));
    }

    #[test]
    fn a_trailing_pdf_is_not_doubled() {
        for template in ["{n}.PDF", "{n}.pdf.", "{n}.pdf .", "{n} .Pdf. ."] {
            let t = Template::parse(template).unwrap();
            assert_eq!(t.apply(&part("")).as_deref(), Some("2"), "{template}");
        }
        // A value that ends in .pdf is the user's own text, and stays; so does a .pdf that a placeholder follows.
        let t = Template::parse("{bookmark}").unwrap();
        assert_eq!(t.apply(&part("a.pdf")).as_deref(), Some("a.pdf"));
        let t = Template::parse("x.pdf{n}").unwrap();
        assert_eq!(t.apply(&part("")).as_deref(), Some("x.pdf2"));
        // Nothing left after the .pdf: no name.
        assert_eq!(Template::parse(".pdf").unwrap().apply(&part("")), None);
        assert_eq!(Template::parse(" .PDF. ").unwrap().apply(&part("")), None);
    }

    #[test]
    fn a_device_name_is_guarded_after_a_cut_too() {
        // 197 spaces after CON: the cut ends in spaces, which go, and what is left is a device name.
        let s = sanitize(&format!("CON{}x", " ".repeat(197))).unwrap();
        assert_eq!(s, "_CON");
        let s = sanitize(&format!("NUL.{}", "a".repeat(300))).unwrap();
        assert!(s.starts_with("_NUL.") && s.len() <= MAX_BASE_BYTES);
        assert_eq!(guard_device_name("nul.x-part1"), "_nul.x-part1");
        assert_eq!(guard_device_name("nul-part1"), "nul-part1");
    }

    #[test]
    fn a_huge_value_or_template_is_bounded() {
        assert!(matches!(Template::parse(&"a".repeat(MAX_TEMPLATE_BYTES + 1)), Err(ToolError::InvalidArgs(_))));
        let t = Template::parse(&"{bookmark}".repeat(99)).unwrap();
        let name = t.apply(&part(&"b".repeat(5_000_000))).unwrap();
        assert!(name.len() <= MAX_BASE_BYTES);
    }

    #[test]
    fn names_that_differ_by_case_or_sigma_fold_together() {
        assert_eq!(fold_key("Intro"), fold_key("INTRO"));
        assert_eq!(fold_key("\u{3b1}\u{3c3}"), fold_key("\u{3b1}\u{3c2}"));
        assert_ne!(fold_key("a"), fold_key("b"));
    }

    #[test]
    fn an_empty_result_has_no_name() {
        let t = Template::parse("{bookmark}").unwrap();
        assert_eq!(t.apply(&part("")), None);
        assert_eq!(t.apply(&part("..")), None);
    }
}
