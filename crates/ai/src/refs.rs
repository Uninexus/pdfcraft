//! Page references in an answer, such as `(p. 20)`, `pp. 3-5` or `page 7`, so a front end can make
//! them clickable. The assistant is asked to cite pages like `(p. 3)`; models also write `page 3`,
//! `pages 3 and 5` and `pp. 3–5`, so those are found too.

/// A piece of an answer: plain text, or a page number the text points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    Text(String),
    /// A 1-based page, as written in the answer.
    Page(usize),
}

/// The longest page number looked at, in digits; longer runs are ordinary numbers.
const MAX_DIGITS: usize = 6;

/// Keywords that introduce page numbers, longest first. Matched without regard to case, at the
/// start of a word that follows a space, an opening bracket or a dash (not a `/` or `.` in an address).
const KEYWORDS: [&str; 5] = ["pages", "page", "pp.", "pp", "p."];

/// Splits `text` into text and page references. Only pages from 1 to `page_count` become
/// [`Segment::Page`]; any other number stays text. Joining the segments gives back `text`
/// except that each page is written as its number, so a front end shows the same words.
pub fn split_page_references(text: &str, page_count: usize) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    let mut plain = String::new();
    let mut i = 0usize;
    while i < text.len() {
        let rest = text.get(i..).unwrap_or_default();
        let at_word_start =
            text.get(..i).and_then(|b| b.chars().next_back()).is_none_or(|c| c.is_whitespace() || "([{\"'*_,;:>-\u{2013}\u{2014}".contains(c));
        if at_word_start && let Some(after_keyword) = keyword_end(rest) {
            let list = number_list(rest.get(after_keyword..).unwrap_or_default());
            if let Some(list) = list {
                let mut matched = false;
                let mut consumed = after_keyword;
                let mut local = vec![Segment::Text(rest.get(..after_keyword).unwrap_or_default().to_owned())];
                let tail = rest.get(after_keyword..).unwrap_or_default();
                for part in &list.parts {
                    match part {
                        Part::Gap(len) => {
                            let gap = tail.get(consumed - after_keyword..consumed - after_keyword + len).unwrap_or_default();
                            local.push(Segment::Text(gap.to_owned()));
                            consumed += len;
                        }
                        Part::Number { len, value } => {
                            let shown = tail.get(consumed - after_keyword..consumed - after_keyword + len).unwrap_or_default();
                            if (1..=page_count).contains(value) {
                                local.push(Segment::Page(*value));
                                matched = true;
                            } else {
                                local.push(Segment::Text(shown.to_owned()));
                            }
                            consumed += len;
                        }
                    }
                }
                if matched {
                    for seg in local {
                        match seg {
                            Segment::Text(t) => plain.push_str(&t),
                            page => {
                                if !plain.is_empty() {
                                    out.push(Segment::Text(std::mem::take(&mut plain)));
                                }
                                out.push(page);
                            }
                        }
                    }
                    i += consumed;
                    continue;
                }
            }
        }
        let ch = rest.chars().next().map_or(1, char::len_utf8);
        plain.push_str(rest.get(..ch).unwrap_or_default());
        i += ch;
    }
    if !plain.is_empty() {
        out.push(Segment::Text(plain));
    }
    out
}

enum Part {
    Gap(usize),
    Number { len: usize, value: usize },
}

struct NumberList {
    parts: Vec<Part>,
}

/// Where the keyword at the start of `s` ends, when `s` starts with one.
fn keyword_end(s: &str) -> Option<usize> {
    KEYWORDS.iter().find_map(|k| {
        let head = s.get(..k.len())?;
        head.eq_ignore_ascii_case(k).then_some(k.len())
    })
}

/// A number list at the start of `s` after optional spaces: `20`, `3-5`, `3, 5 and 7`.
/// Returns the pieces (the leading spaces first) or `None` when no number follows.
fn number_list(s: &str) -> Option<NumberList> {
    let mut parts = Vec::new();
    let mut pos = 0usize;
    let lead = s.len() - s.trim_start_matches([' ', '\u{a0}']).len();
    if lead > 0 {
        parts.push(Part::Gap(lead));
        pos = lead;
    }
    loop {
        let (len, value) = number_at(s.get(pos..)?)?;
        parts.push(Part::Number { len, value });
        pos += len;
        match separator_then_digit(s.get(pos..).unwrap_or_default()) {
            Some(sep) => {
                parts.push(Part::Gap(sep));
                pos += sep;
            }
            None => break,
        }
    }
    Some(NumberList { parts })
}

fn number_at(s: &str) -> Option<(usize, usize)> {
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > MAX_DIGITS {
        return None;
    }
    // A longer run of letters or digits glued on (`3rd`, `20x`) is not a page number.
    if s.get(digits..).and_then(|r| r.chars().next()).is_some_and(char::is_alphanumeric) {
        return None;
    }
    s.get(..digits)?.parse::<usize>().ok().map(|v| (digits, v))
}

/// The length of a separator (`, `, `-`, `–`, ` and `, ` & `) that is followed by a digit.
fn separator_then_digit(s: &str) -> Option<usize> {
    let mut len = 0usize;
    let trimmed = s.trim_start_matches([' ', '\u{a0}']);
    len += s.len() - trimmed.len();
    let after = if let Some(r) = trimmed.strip_prefix("and ") {
        len += 4;
        r
    } else {
        let r = trimmed.strip_prefix(['-', '\u{2013}', '\u{2014}', ',', ';', '&'])?;
        len += trimmed.len() - r.len();
        r
    };
    let rest = after.trim_start_matches([' ', '\u{a0}']);
    len += after.len() - rest.len();
    rest.chars().next().is_some_and(|c| c.is_ascii_digit()).then_some(len)
}
