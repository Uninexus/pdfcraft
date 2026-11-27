//! Word's own lists in a .docx: a paragraph that opens with a bullet, or with a number or a small letter and its
//! mark, becomes a list item, and the marker leaves the text for Word to draw. A list goes on while each number is one
//! more than the last of its kind, across the paragraphs and pages between; any other number starts a list there.
//! Dashes and asterisks stay text: they open as many sentences as lists.

use craft_wordmodel::numbering::{ListKind, levels_for};
use craft_wordmodel::props::NumRef;
use craft_wordmodel::{Document, Paragraph};

/// The bullets taken for a list's: the filled and open marks, and the Symbol and Wingdings bullets Word writes.
const BULLETS: [char; 8] = ['\u{2022}', '\u{25E6}', '\u{25AA}', '\u{2023}', '\u{25CF}', '\u{25CB}', '\u{F0B7}', '\u{F0A7}'];

/// How a paragraph opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Marker {
    Bullet,
    /// `1.`, `1)` or `a)`, and the number it stands for.
    Numbered(ListKind, u32),
}

/// The marker `text` opens with and how many characters it and the space after it take, when text follows. A year or a
/// decimal ("2026. The", "1.5 kg") is not one.
pub(crate) fn marker(text: &str) -> Option<(Marker, usize)> {
    let mut chars = text.chars().peekable();
    let mut taken = 0;
    while chars.next_if(|c| c.is_whitespace()).is_some() {
        taken += 1;
    }
    let first = chars.next()?;
    taken += 1;
    let found = if BULLETS.contains(&first) {
        Marker::Bullet
    } else if first.is_ascii_digit() {
        let mut number = first.to_digit(10)?;
        let mut digits = 1;
        while let Some(d) = chars.next_if(char::is_ascii_digit) {
            number = number.saturating_mul(10).saturating_add(d.to_digit(10)?);
            digits += 1;
            taken += 1;
        }
        if digits > 3 || number == 0 {
            return None;
        }
        let kind = match chars.next()? {
            '.' => ListKind::Numbered,
            ')' => ListKind::NumberedParen,
            _ => return None,
        };
        taken += 1;
        Marker::Numbered(kind, number)
    } else if first.is_ascii_lowercase() && chars.next() == Some(')') {
        taken += 1;
        Marker::Numbered(ListKind::LowerLetter, u32::from(first) - u32::from('a') + 1)
    } else {
        return None;
    };
    // A bullet may touch its text; a number and its text are apart.
    let mut space = 0;
    while chars.next_if(|c| c.is_whitespace()).is_some() {
        space += 1;
    }
    if chars.next().is_none() || (space == 0 && found != Marker::Bullet) {
        return None;
    }
    Some((found, taken + space))
}

/// The lists of the document being built: the bulleted one, and the last number and list of each numbered kind.
#[derive(Default)]
pub(crate) struct Lists {
    bullets: Option<u32>,
    numbered: Vec<(ListKind, u32, u32)>,
}

impl Lists {
    /// The list item a paragraph opening with `marker` is, adding its list to `doc` when it starts one. `None` when no
    /// list could be added: the paragraph then stays as it reads.
    pub(crate) fn item(&mut self, doc: &mut Document, marker: Marker) -> Option<NumRef> {
        let numbering = std::sync::Arc::make_mut(&mut doc.numbering);
        let num = match marker {
            Marker::Bullet => match self.bullets {
                Some(num) => num,
                None => *self.bullets.insert(numbering.try_add_list(ListKind::Bullet).ok()?),
            },
            Marker::Numbered(kind, number) => match self.numbered.iter_mut().find(|(k, ..)| *k == kind) {
                Some((_, last, num)) if last.checked_add(1) == Some(number) => {
                    *last = number;
                    *num
                }
                found => {
                    let mut levels = levels_for(kind);
                    levels.first_mut()?.start = number;
                    let num = numbering.try_add_levels(None, levels).ok()?;
                    match found {
                        Some(list) => *list = (kind, number, num),
                        None => self.numbered.push((kind, number, num)),
                    }
                    num
                }
            },
        };
        Some(NumRef { num, level: 0 })
    }
}

/// Takes the first `count` characters out of `paragraph`'s text; false when it could not.
pub(crate) fn strip(paragraph: &mut Paragraph, count: usize) -> bool {
    let end = paragraph.text.char_indices().nth(count).map_or(paragraph.text.len(), |(byte, _)| byte);
    paragraph.delete(0, end).is_ok()
}
