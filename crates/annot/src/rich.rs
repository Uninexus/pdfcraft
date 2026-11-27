//! Bold and italic runs in a text box's text: held in its rich text (`/RC`, XHTML, ISO 32000-2 §12.7.4.3) as spans,
//! read back from it (PdfCraft's own or another program's), and laid out for the drawn appearance.
//!
//! Positions count the characters of the text as `/Contents` holds it, with every line break (`\r\n`, `\r` or `\n`)
//! counted as one. A run adds to the box's own face: bold text in a bold box stays bold.

use pdfcraft_fonts::Std14;

use crate::text_style::{is_bold, is_italic, styled};

/// Characters `start..end` of a text box's text, drawn bold, italic or both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextRun {
    pub start: usize,
    pub end: usize,
    pub bold: bool,
    pub italic: bool,
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

/// (bold, italic) for each of `count` characters.
pub(crate) fn per_char(runs: &[TextRun], count: usize) -> Vec<(bool, bool)> {
    let mut flags = vec![(false, false); count];
    for run in runs {
        for flag in flags.iter_mut().take(run.end).skip(run.start) {
            flag.0 |= run.bold;
            flag.1 |= run.italic;
        }
    }
    flags
}

/// The fewest runs that say `flags`.
fn runs_of(flags: &[(bool, bool)]) -> Vec<TextRun> {
    let mut runs: Vec<TextRun> = Vec::new();
    for (at, &(bold, italic)) in flags.iter().enumerate() {
        if !bold && !italic {
            continue;
        }
        match runs.last_mut() {
            Some(last) if last.end == at && (last.bold, last.italic) == (bold, italic) => last.end = at + 1,
            _ => runs.push(TextRun { start: at, end: at + 1, bold, italic }),
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
        let mut current = (false, false);
        let close = |out: &mut String, piece: &mut String, (bold, italic): (bool, bool)| {
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

/// What a tag's `style` attribute adds: (bold, italic).
fn css_adds(tag: &str) -> (bool, bool) {
    let lower = tag.to_ascii_lowercase();
    let Some(at) = lower.find("style=") else { return (false, false) };
    let rest = lower.get(at + 6..).unwrap_or_default();
    let Some(quote) = rest.chars().next().filter(|c| matches!(c, '"' | '\'')) else { return (false, false) };
    let value = rest.get(1..).unwrap_or_default().split(quote).next().unwrap_or_default();
    let mut adds = (false, false);
    for declaration in value.split(';') {
        let Some((name, value)) = declaration.split_once(':') else { continue };
        let value = value.trim();
        match name.trim() {
            "font-weight" => adds.0 |= value.starts_with("bold") || value.parse::<u32>().is_ok_and(|w| w >= 600),
            "font-style" => adds.1 |= value.starts_with("italic") || value.starts_with("oblique"),
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
    let mut flags: Vec<(bool, bool)> = Vec::new();
    // The elements open inside a paragraph, with what each adds. The body's own style is the box's.
    let mut open: Vec<(bool, bool)> = Vec::new();
    let mut inside = false;
    let mut paragraphs = 0;
    let mut chars = rc.chars();
    while let Some(ch) = chars.next() {
        let mut push = |c: char, open: &[(bool, bool)]| {
            text.push(c);
            flags.push((open.iter().any(|f| f.0), open.iter().any(|f| f.1)));
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
                    "span" | "b" | "strong" | "i" | "em" if inside && !empty => {
                        if closing {
                            open.pop();
                        } else {
                            let own = css_adds(&tag);
                            open.push((own.0 || matches!(name.as_str(), "b" | "strong"), own.1 || matches!(name.as_str(), "i" | "em")));
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
fn face(base: Std14, (bold, italic): (bool, bool)) -> Std14 {
    styled(base, bold || is_bold(base), italic || is_italic(base))
}

/// A stretch of one line in one face.
pub(crate) type Segment = (Std14, String);

fn push(line: &mut Vec<Segment>, face: Std14, ch: char) {
    match line.last_mut() {
        Some(last) if last.0 == face => last.1.push(ch),
        _ => line.push((face, ch.to_string())),
    }
}

/// The width of a line of segments at `size`.
pub(crate) fn line_width(line: &[Segment], size: f64) -> f64 {
    line.iter().map(|(face, text)| face.text_width(text, size)).sum()
}

/// `text` wrapped `width` wide in lines of one-face segments: at spaces, and between characters for a word wider than
/// the line, as [`pdfcraft_fonts::wrap_with`] breaks text of one face.
pub(crate) fn wrap(text: &str, runs: &[TextRun], base: Std14, size: f64, width: f64) -> Vec<Vec<Segment>> {
    let text = lines_normalized(text);
    let flags = per_char(runs, text.chars().count());
    let styled: Vec<(char, Std14)> = text.chars().zip(&flags).map(|(c, &f)| (c, face(base, f))).collect();
    let advance = |&(ch, face): &(char, Std14)| face.text_width(ch.encode_utf8(&mut [0; 4]), size);
    let mut lines = Vec::new();
    for paragraph in styled.split(|(c, _)| *c == '\n') {
        let mut line: Vec<Segment> = Vec::new();
        let mut used = 0.0;
        // The space before the word being placed, in its own face.
        let mut gap: Option<(char, Std14)> = None;
        let mut at = 0;
        loop {
            let end = paragraph.iter().skip(at).position(|(c, _)| *c == ' ').map_or(paragraph.len(), |n| at + n);
            let word = paragraph.get(at..end).unwrap_or_default();
            let word_width: f64 = word.iter().map(advance).sum();
            let lead = gap.as_ref().map_or(0.0, advance);
            if used + lead + word_width <= width {
                for &(ch, face) in gap.iter().chain(word) {
                    push(&mut line, face, ch);
                }
                used += lead + word_width;
            } else {
                if line.iter().any(|(_, t)| !t.trim().is_empty()) {
                    lines.push(std::mem::take(&mut line));
                }
                line.clear();
                used = 0.0;
                for unit in word {
                    let unit_width = advance(unit);
                    if !line.is_empty() && used + unit_width > width {
                        lines.push(std::mem::take(&mut line));
                        used = 0.0;
                    }
                    push(&mut line, unit.1, unit.0);
                    used += unit_width;
                }
            }
            let Some(space) = paragraph.get(end) else { break };
            gap = Some(*space);
            at = end + 1;
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rich text as other programs write it: `<b>` and `<i>`, entities, weights as numbers, and rich text that no
    /// longer says what the box says.
    #[test]
    fn runs_are_read_from_other_programs_rich_text() {
        let rc = "<?xml version=\"1.0\"?><body style=\"font-weight:normal\">\n<p>A <b>b&amp;c <i>d</i></b></p>\n<p><span style='font-weight:700'>e</span>&#x66;</p></body>";
        let bold = |start, end| TextRun { start, end, bold: true, italic: false };
        assert_eq!(runs(rc, "A b&c d\r\nef"), [bold(2, 6), TextRun { start: 6, end: 7, bold: true, italic: true }, bold(8, 9)]);
        assert!(runs(rc, "something else").is_empty());
        for junk in ["<p><b>", "</p></p></span>", "<p>&#xFFFFFFFF;&;</p>", "<span style=\"", ""] {
            let _ = runs(junk, "x");
        }
    }
}
