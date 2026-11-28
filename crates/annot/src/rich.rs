//! Bold, italic and underlined runs in a text box's text: held in its rich text (`/RC`, XHTML, ISO 32000-2 §12.7.4.3) as spans,
//! read back from it (PdfCraft's own or another program's), and laid out for the drawn appearance.
//!
//! Positions count the characters of the text as `/Contents` holds it, with every line break (`\r\n`, `\r` or `\n`)
//! counted as one. A run adds to the box's own face: bold text in a bold box stays bold.

use std::ops::Range;

use pdfcraft_fonts::Std14;

use crate::text_style::{is_bold, is_italic, styled};

/// Characters `start..end` of a text box's text, drawn bold, italic, underlined or any of them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextRun {
    pub start: usize,
    pub end: usize,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

/// What a character is drawn with beyond the box's own face.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Flags {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl Flags {
    fn or(self, other: Flags) -> Flags {
        Flags { bold: self.bold || other.bold, italic: self.italic || other.italic, underline: self.underline || other.underline }
    }
}

/// The most rich text read: past this it is left alone and the box is drawn without runs.
const MAX_RC: usize = 1 << 20;

/// Whether `text` has letters that are shaped in one face of their own (Thai), which runs can't style.
pub(crate) fn shaped_only(text: &str) -> bool {
    text.chars().any(|c| ('\u{0E00}'..='\u{0E7F}').contains(&c))
}

/// The text with one kind of line break.
pub(crate) fn lines_normalized(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// What each of `count` characters is drawn with.
pub(crate) fn per_char(runs: &[TextRun], count: usize) -> Vec<Flags> {
    let mut flags = vec![Flags::default(); count];
    for run in runs {
        for flag in flags.iter_mut().take(run.end).skip(run.start) {
            *flag = flag.or(Flags { bold: run.bold, italic: run.italic, underline: run.underline });
        }
    }
    flags
}

/// The fewest runs that say `flags`.
fn runs_of(flags: &[Flags]) -> Vec<TextRun> {
    let mut runs: Vec<TextRun> = Vec::new();
    for (at, &flag) in flags.iter().enumerate() {
        if flag == Flags::default() {
            continue;
        }
        let Flags { bold, italic, underline } = flag;
        match runs.last_mut() {
            Some(last) if last.end == at && (last.bold, last.italic, last.underline) == (bold, italic, underline) => last.end = at + 1,
            _ => runs.push(TextRun { start: at, end: at + 1, bold, italic, underline }),
        }
    }
    runs
}

/// `runs` in their shortest form, refusing one that is empty or ends past the `text`.
pub(crate) fn checked(runs: &[TextRun], text: &str) -> Result<Vec<TextRun>, String> {
    let count = lines_normalized(text).chars().count();
    if let Some(bad) = runs.iter().find(|r| r.start >= r.end || r.end > count) {
        return Err(format!("a run from {} to {} does not fit text of {count} characters", bad.start, bad.end));
    }
    Ok(runs_of(&per_char(runs, count)))
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The paragraphs of the rich text's body: one `<p>` a line, a `<span>` around each styled stretch.
pub(crate) fn body(text: &str, runs: &[TextRun]) -> String {
    let text = lines_normalized(text);
    let flags = per_char(runs, text.chars().count());
    let mut out = String::new();
    let mut at = 0;
    for line in text.split('\n') {
        out.push_str("<p dir=\"ltr\">");
        let mut piece = String::new();
        let mut current = Flags::default();
        let close = |out: &mut String, piece: &mut String, Flags { bold, italic, underline }: Flags| {
            if piece.is_empty() {
                return;
            }
            let mut css = Vec::new();
            if bold {
                css.push("font-weight:bold");
            }
            if italic {
                css.push("font-style:italic");
            }
            if underline {
                css.push("text-decoration:underline");
            }
            if css.is_empty() {
                out.push_str(&escape(piece));
            } else {
                out.push_str(&format!("<span style=\"{}\">{}</span>", css.join(";"), escape(piece)));
            }
            piece.clear();
        };
        for ch in line.chars() {
            let flag = flags.get(at).copied().unwrap_or_default();
            if flag != current {
                close(&mut out, &mut piece, current);
                current = flag;
            }
            piece.push(ch);
            at += 1;
        }
        close(&mut out, &mut piece, current);
        out.push_str("</p>");
        // The line break.
        at += 1;
    }
    out
}

/// One `&…;` reference.
fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let number = name.strip_prefix('#')?;
            let code = match number.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => number.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

/// What a tag's `style` attribute adds.
fn css_adds(tag: &str) -> Flags {
    let lower = tag.to_ascii_lowercase();
    let Some(at) = lower.find("style=") else { return Flags::default() };
    let rest = lower.get(at + 6..).unwrap_or_default();
    let Some(quote) = rest.chars().next().filter(|c| matches!(c, '"' | '\'')) else { return Flags::default() };
    let value = rest.get(1..).unwrap_or_default().split(quote).next().unwrap_or_default();
    let mut adds = Flags::default();
    for declaration in value.split(';') {
        let Some((name, value)) = declaration.split_once(':') else { continue };
        let value = value.trim();
        match name.trim() {
            "font-weight" => adds.bold |= value.starts_with("bold") || value.parse::<u32>().is_ok_and(|w| w >= 600),
            "font-style" => adds.italic |= value.starts_with("italic") || value.starts_with("oblique"),
            "text-decoration" => adds.underline |= value.contains("underline"),
            _ => {}
        }
    }
    adds
}

/// The runs of the rich text `rc`, when the text of its paragraphs is `contents`: rich text that says something else
/// is out of date and has none.
pub(crate) fn runs(rc: &str, contents: &str) -> Vec<TextRun> {
    if rc.len() > MAX_RC {
        return Vec::new();
    }
    let wanted = lines_normalized(contents);
    let mut text = String::new();
    let mut flags: Vec<Flags> = Vec::new();
    // The elements open inside a paragraph, with what each adds. The body's own style is the box's.
    let mut open: Vec<Flags> = Vec::new();
    let mut inside = false;
    let mut paragraphs = 0;
    let mut chars = rc.chars();
    while let Some(ch) = chars.next() {
        let mut push = |c: char, open: &[Flags]| {
            text.push(c);
            flags.push(open.iter().fold(Flags::default(), |all, f| all.or(*f)));
        };
        match ch {
            '<' => {
                let tag: String = chars.by_ref().take_while(|c| *c != '>').collect();
                let closing = tag.starts_with('/');
                let empty = tag.ends_with('/');
                let name = tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or_default().to_ascii_lowercase();
                match name.as_str() {
                    "p" if closing => {
                        inside = false;
                        open.clear();
                    }
                    "p" => {
                        if paragraphs > 0 {
                            push('\n', &[]);
                        }
                        paragraphs += 1;
                        inside = !empty;
                        open = vec![css_adds(&tag)];
                    }
                    "br" if inside && !closing => push('\n', &[]),
                    "span" | "b" | "strong" | "i" | "em" | "u" if inside && !empty => {
                        if closing {
                            open.pop();
                        } else {
                            let named = Flags {
                                bold: matches!(name.as_str(), "b" | "strong"),
                                italic: matches!(name.as_str(), "i" | "em"),
                                underline: name == "u",
                            };
                            open.push(css_adds(&tag).or(named));
                        }
                    }
                    _ => {}
                }
            }
            '&' if inside => {
                let name: String = chars.by_ref().take_while(|c| *c != ';').take(12).collect();
                if let Some(c) = entity(&name) {
                    push(c, &open);
                }
            }
            c if inside => push(c, &open),
            _ => {}
        }
    }
    if lines_normalized(&text) != wanted {
        return Vec::new();
    }
    runs_of(&flags)
}

/// The face of text with `flags` in a box whose own face is `base`.
fn face(base: Std14, flags: Flags) -> Std14 {
    styled(base, flags.bold || is_bold(base), flags.italic || is_italic(base))
}

/// A stretch of one line in one face, underlined or not.
pub(crate) type Segment = (Std14, String, bool);

fn push(line: &mut Vec<Segment>, face: Std14, underline: bool, ch: char) {
    match line.last_mut() {
        Some(last) if last.0 == face && last.2 == underline => last.1.push(ch),
        _ => line.push((face, ch.to_string(), underline)),
    }
}

/// The width of a line of segments at `size`.
pub(crate) fn line_width(line: &[Segment], size: f64) -> f64 {
    line.iter().map(|(face, text, _)| face.text_width(text, size)).sum()
}

/// The lines `text` with `runs` is broken into, `width` wide in Helvetica at `size`, as ranges of characters (a line
/// break counting as one): what a text box's appearance draws on each line, for an editor to lay out alike.
pub fn line_ranges(text: &str, runs: &[TextRun], size: f64, width: f64) -> Vec<Range<usize>> {
    ranges(&faces(text, runs, Std14::Helvetica), size, width)
}

/// Each character of `text` with the face it is drawn in.
fn faces(text: &str, runs: &[TextRun], base: Std14) -> Vec<(char, Std14)> {
    let text = lines_normalized(text);
    let flags = per_char(runs, text.chars().count());
    text.chars().zip(&flags).map(|(c, &f)| (c, face(base, f))).collect()
}

fn advance(&(ch, face): &(char, Std14), size: f64) -> f64 {
    face.text_width(ch.encode_utf8(&mut [0; 4]), size)
}

/// `styled` wrapped `width` wide: at spaces, and between characters for a word wider than the line, as
/// [`pdfcraft_fonts::wrap_with`] breaks text of one face. A space a line breaks at is on neither line.
fn ranges(styled: &[(char, Std14)], size: f64, width: f64) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut base = 0;
    for paragraph in styled.split(|(c, _)| *c == '\n') {
        // The line being filled, within the paragraph.
        let (mut start, mut end) = (0, 0);
        let mut used = 0.0;
        let mut at = 0;
        loop {
            let word_end = paragraph.iter().skip(at).position(|(c, _)| *c == ' ').map_or(paragraph.len(), |n| at + n);
            let word = paragraph.get(at..word_end).unwrap_or_default();
            let word_width: f64 = word.iter().map(|c| advance(c, size)).sum();
            // The space before the word, which comes with it.
            let lead = at.checked_sub(1).and_then(|i| paragraph.get(i)).map_or(0.0, |c| advance(c, size));
            if used + lead + word_width <= width {
                end = word_end;
                used += lead + word_width;
            } else {
                // Indentation alone is not a line: it is dropped before an over-long word.
                if paragraph.get(start..end).is_some_and(|line| line.iter().any(|(c, _)| !c.is_whitespace())) {
                    lines.push(base + start..base + end);
                }
                (start, end, used) = (at, at, 0.0);
                for (offset, unit) in word.iter().enumerate() {
                    let unit_width = advance(unit, size);
                    if end > start && used + unit_width > width {
                        lines.push(base + start..base + end);
                        start = at + offset;
                        used = 0.0;
                    }
                    end = at + offset + 1;
                    used += unit_width;
                }
            }
            if word_end >= paragraph.len() {
                break;
            }
            at = word_end + 1;
        }
        lines.push(base + start..base + end);
        base += paragraph.len() + 1;
    }
    lines
}

/// `text` wrapped `width` wide in lines of one-face segments.
pub(crate) fn wrap(text: &str, runs: &[TextRun], base: Std14, size: f64, width: f64) -> Vec<Vec<Segment>> {
    let styled = faces(text, runs, base);
    let flags = per_char(runs, styled.len());
    ranges(&styled, size, width)
        .into_iter()
        .map(|range| {
            let mut line: Vec<Segment> = Vec::new();
            for (&(ch, face), flag) in styled.get(range.clone()).unwrap_or_default().iter().zip(flags.get(range).unwrap_or_default()) {
                push(&mut line, face, flag.underline, ch);
            }
            line
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rich text as other programs write it: `<b>` and `<i>`, entities, weights as numbers, and rich text that no
    /// longer says what the box says.
    #[test]
    fn runs_are_read_from_other_programs_rich_text() {
        let rc = "<?xml version=\"1.0\"?><body style=\"font-weight:normal\">\n<p>A <b>b&amp;c <i>d</i></b></p>\n<p><span style='font-weight:700'>e</span>&#x66;</p></body>";
        let bold = |start, end| TextRun { start, end, bold: true, ..TextRun::default() };
        assert_eq!(runs(rc, "A b&c d\r\nef"), [bold(2, 6), TextRun { start: 6, end: 7, bold: true, italic: true, underline: false }, bold(8, 9)]);
        assert!(runs(rc, "something else").is_empty());
        for junk in ["<p><b>", "</p></p></span>", "<p>&#xFFFFFFFF;&;</p>", "<span style=\"", ""] {
            let _ = runs(junk, "x");
        }
    }

    #[test]
    fn underlined_runs_are_written_and_read_back_and_split_the_drawn_line() {
        let runs = [TextRun { start: 4, end: 8, underline: true, ..TextRun::default() }];
        let rc = format!("<body>{}</body>", body("Due now, please", &runs));
        assert!(rc.contains("text-decoration:underline"), "{rc}");
        assert_eq!(super::runs(&rc, "Due now, please"), runs);
        // Other programs' <u> too.
        assert_eq!(super::runs("<body><p>a <u>b</u></p></body>", "a b"), [TextRun { start: 2, end: 3, underline: true, ..TextRun::default() }]);
        let line = &wrap("Due now, please", &runs, Std14::Helvetica, 12.0, 500.0)[0];
        let said: Vec<_> = line.iter().map(|(_, words, under)| (words.as_str(), *under)).collect();
        assert_eq!(said, [("Due ", false), ("now,", true), (" please", false)]);
    }
}
