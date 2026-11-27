//! Word's own lists in a .docx: a paragraph that opens with a bullet, or with a number or a small letter and its
//! mark, becomes a list item, and the marker leaves the text for Word to draw. A list goes on while each number is one
//! more than the last of its kind at its level, across the paragraphs and pages between; any other number starts a list
//! there. An item starting further right than the one before it is a level below it.
//! Dashes and asterisks stay text: they open as many sentences as lists.

use craft_wordmodel::numbering::{Level, ListKind, levels_for};
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

/// How much further right (points) an item must start to be a level below the one above it.
const INDENT: f64 = 8.0;
/// The levels a Word list has.
const LEVELS: usize = 9;

/// The lists of the document being built: the bulleted one, the last number and list of each numbered kind at each
/// level, and where the items around the one being placed start.
#[derive(Default)]
pub(crate) struct Lists {
    bullets: Option<u32>,
    numbered: Vec<(ListKind, u8, u32, u32)>,
    /// The left edge of each level open here, outermost first.
    edges: Vec<f64>,
}

impl Lists {
    /// A paragraph that is no list item: the next item starts at the outer level again.
    pub(crate) fn interrupt(&mut self) {
        self.edges.clear();
    }

    /// The level of an item that starts `left` points across the page, under the items before it.
    fn level(&mut self, left: f64) -> u8 {
        while self.edges.last().is_some_and(|&edge| left < edge - INDENT) {
            self.edges.pop();
        }
        if self.edges.last().is_none_or(|&edge| left > edge + INDENT) && self.edges.len() < LEVELS {
            self.edges.push(left);
        }
        u8::try_from(self.edges.len().saturating_sub(1)).unwrap_or(0)
    }

    /// The list item a paragraph opening with `marker`, `left` points across the page, is, adding its list to `doc`
    /// when it starts one. `None` when no list could be added: the paragraph then stays as it reads.
    pub(crate) fn item(&mut self, doc: &mut Document, marker: Marker, left: f64) -> Option<NumRef> {
        let level = self.level(left);
        let numbering = std::sync::Arc::make_mut(&mut doc.numbering);
        let num = match marker {
            Marker::Bullet => match self.bullets {
                Some(num) => num,
                None => *self.bullets.insert(numbering.try_add_list(ListKind::Bullet).ok()?),
            },
            Marker::Numbered(kind, number) => match self.numbered.iter_mut().find(|(k, at, ..)| *k == kind && *at == level) {
                Some((.., last, num)) if last.checked_add(1) == Some(number) => {
                    *last = number;
                    *num
                }
                found => {
                    let num = numbering.try_add_levels(None, levels(kind, level, number)?).ok()?;
                    match found {
                        Some(list) => *list = (kind, level, number, num),
                        None => self.numbered.push((kind, level, number, num)),
                    }
                    num
                }
            },
        };
        Some(NumRef { num, level })
    }
}

/// The levels of a numbered list whose items are at `level`, counting from `start` there in the marker's own style:
/// a letter under a number is a letter, whatever Word would put at that depth.
fn levels(kind: ListKind, level: u8, start: u32) -> Option<Vec<Level>> {
    let mut levels = levels_for(kind);
    let first = levels.first()?.clone();
    let here = levels.get_mut(usize::from(level))?;
    here.format = first.format;
    here.text = first.text.replace("%1", &format!("%{}", u32::from(level) + 1));
    here.start = start;
    Some(levels)
}

/// Takes the first `count` characters out of `paragraph`'s text; false when it could not.
pub(crate) fn strip(paragraph: &mut Paragraph, count: usize) -> bool {
    let end = paragraph.text.char_indices().nth(count).map_or(paragraph.text.len(), |(byte, _)| byte);
    paragraph.delete(0, end).is_ok()
}
