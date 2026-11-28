//! Bulleted and numbered lists in a text's typed lines: one item to a line, its marker written as
//! ordinary characters before it, so the list is plain text in the file and in any other program.
//! A blank line is not an item; numbering carries on past it.

use std::ops::{Range, RangeInclusive};

/// What marks the items of a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    Bullets,
    Numbers,
}

/// The bullet written before an item.
const BULLET: char = '•';
/// The marks read as a bullet.
const BULLETS: [char; 8] = [BULLET, '◦', '▪', '‣', '⁃', '*', '-', '–'];
/// The most digits an item's number has: a year opening a sentence ("2026. We began") is text.
const DIGITS: usize = 3;

/// A line's indent, marker with its number, and what follows the marker.
type Parts<'a> = (&'a str, Option<(ListKind, u32)>, &'a str);

/// The item `rest` is once a marker of `kind` has been taken off it. A marker ends at a space or at
/// the end of the line: "-5" and "1.5" are text.
fn item<'a>(indent: &'a str, rest: &'a str, kind: ListKind, number: u32) -> Option<Parts<'a>> {
    let ends = rest.chars().next().is_none_or(char::is_whitespace);
    ends.then(|| (indent, Some((kind, number)), rest.trim_start()))
}

/// A line taken apart: its indent, its marker if it has one (with the number a numbered one
/// says), and what follows the marker.
fn parts(line: &str) -> Parts<'_> {
    let body = line.trim_start();
    let indent = line.get(..line.len() - body.len()).unwrap_or_default();
    let bullet = body.strip_prefix(BULLETS).and_then(|rest| item(indent, rest, ListKind::Bullets, 0));
    let digits = body.len() - body.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let number = || {
        let said = body.get(..digits).filter(|_| (1..=DIGITS).contains(&digits))?.parse().ok()?;
        let rest = body.get(digits..)?.strip_prefix(['.', ')'])?;
        item(indent, rest, ListKind::Numbers, said)
    };
    bullet.or_else(number).unwrap_or((indent, None, body))
}

/// The kind of list `text` is: every line with something on it has a marker of that kind.
pub fn list_kind(text: &str) -> Option<ListKind> {
    let mut found = None;
    for line in text.split('\n').filter(|line| !line.trim().is_empty()) {
        let (kind, _) = parts(line).1?;
        if found.is_some_and(|other| other != kind) {
            return None;
        }
        found = Some(kind);
    }
    found
}

/// `text` with its lines made a list of `kind`, or with their markers taken off for `None`. A line
/// keeps its indent, and a marker it already has is replaced. Numbers count from the first line's
/// own number, or from 1. A text with nothing on it becomes one empty item.
pub fn set_list(text: &str, kind: Option<ListKind>) -> String {
    let empty = text.trim().is_empty();
    let mut number = match text.split('\n').find(|line| !line.trim().is_empty()).and_then(|line| parts(line).1) {
        Some((ListKind::Numbers, first)) => first,
        _ => 1,
    };
    let lines: Vec<String> = text
        .split('\n')
        .enumerate()
        .map(|(at, line)| {
            if line.trim().is_empty() && !(empty && at == 0) {
                return line.to_string();
            }
            let (indent, _, rest) = parts(line);
            match kind {
                None => format!("{indent}{rest}"),
                Some(ListKind::Bullets) => format!("{indent}{BULLET} {rest}"),
                Some(ListKind::Numbers) => {
                    let item = format!("{indent}{number}. {rest}");
                    number = number.saturating_add(1);
                    item
                }
            }
        })
        .collect();
    lines.join("\n")
}

/// The bytes of `text` that its lines `lines` take, without the last one's line break.
fn span(text: &str, lines: &RangeInclusive<usize>) -> Range<usize> {
    let mut at = 0;
    let (mut start, mut end) = (text.len(), text.len());
    for (number, line) in text.split('\n').enumerate() {
        if number == *lines.start() {
            start = at;
        }
        if number == *lines.end() {
            end = at + line.len();
        }
        at += line.len() + 1;
    }
    start.min(end)..end
}

/// A list button: lines `lines` of `text` (all of it for `None`) take `kind`, or lose their markers
/// when they all have it already. Returns the new text and whether the list is now on.
pub fn toggle(text: &str, lines: Option<RangeInclusive<usize>>, kind: ListKind) -> (String, bool) {
    let range = lines.map_or(0..text.len(), |lines| span(text, &lines));
    let part = text.get(range.clone()).unwrap_or_default();
    let on = list_kind(part) != Some(kind);
    let mut out = text.to_string();
    out.replace_range(range, &set_list(part, on.then_some(kind)));
    (out, on)
}

/// Where byte `at` of `before` is in `after`, which differs from it only in the lines' markers: as
/// far from the end of its line as it was.
pub fn moved(before: &str, after: &str, at: usize) -> usize {
    let head = before.get(..at).unwrap_or(before);
    let number = head.matches('\n').count();
    let from_end = before.get(head.len()..).and_then(|tail| tail.split('\n').next()).map_or(0, str::len);
    let mut start = 0;
    for (i, line) in after.split('\n').enumerate() {
        if i == number {
            let mut column = line.len().saturating_sub(from_end);
            while !line.is_char_boundary(column) {
                column -= 1;
            }
            return start + column;
        }
        start += line.len() + 1;
    }
    after.len()
}

/// What Enter does in a list: bytes `range` of the text become `text`, and the caret goes to byte
/// `caret` of the result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enter {
    pub range: Range<usize>,
    pub text: String,
    pub caret: usize,
}

/// Enter with the caret at byte `at`: in an item, the next item starts (numbered ones after it
/// count on), and on an empty item the list ends. `None` where Enter is only a line break: outside a
/// list, or with the caret before the item's text.
pub fn enter(text: &str, at: usize) -> Option<Enter> {
    let head = text.get(..at)?;
    let start = head.rfind('\n').map_or(0, |i| i + 1);
    let end = text.get(at..)?.find('\n').map_or(text.len(), |i| at + i);
    let line = text.get(start..end)?;
    let (indent, marker, rest) = parts(line);
    let (kind, number) = marker?;
    if rest.trim().is_empty() {
        return Some(Enter { range: start..end, text: String::new(), caret: start });
    }
    if at < start + (line.len() - rest.len()) {
        return None;
    }
    let mut put = match kind {
        ListKind::Bullets => format!("\n{indent}{} ", line.trim_start().chars().next()?),
        ListKind::Numbers => format!("\n{indent}{}. ", number.saturating_add(1)),
    };
    let caret = at + put.len();
    put.push_str(text.get(at..end)?.trim_start());
    // The items that counted on from this one move up by one.
    let mut to = end;
    if kind == ListKind::Numbers {
        let mut expected = number.saturating_add(1);
        let mut next = end;
        let mut between = String::new();
        for following in text.get(end..)?.split('\n').skip(1) {
            next += 1 + following.len();
            between.push('\n');
            if following.trim().is_empty() {
                between.push_str(following);
                continue;
            }
            let (its_indent, its_marker, its_rest) = parts(following);
            if its_indent != indent || its_marker != Some((ListKind::Numbers, expected)) {
                break;
            }
            expected = expected.saturating_add(1);
            put.push_str(&between);
            put.push_str(&format!("{indent}{expected}. {its_rest}"));
            between.clear();
            to = next;
        }
    }
    Some(Enter { range: at..to, text: put, caret })
}

#[cfg(test)]
mod tests {
    use super::{ListKind, enter, list_kind, moved, set_list, toggle};

    /// `text` after Enter where `|` is, with `|` where the caret goes; `None` for a plain break.
    fn pressed(text: &str) -> Option<String> {
        let at = text.find('|')?;
        let mut out = text.replacen('|', "", 1);
        let done = enter(&out, at)?;
        out.replace_range(done.range, &done.text);
        out.insert(done.caret, '|');
        Some(out)
    }

    #[test]
    fn enter_starts_the_next_item_or_ends_the_list() {
        assert_eq!(pressed("• Milk|").as_deref(), Some("• Milk\n• |"));
        assert_eq!(pressed("  - Mi|lk").as_deref(), Some("  - Mi\n  - |lk"));
        assert_eq!(pressed("1. a|\n2. b\n\n3. c\n9. d").as_deref(), Some("1. a\n2. |\n3. b\n\n4. c\n9. d"));
        assert_eq!(pressed("1. a\n2. |").as_deref(), Some("1. a\n|"));
        assert_eq!(pressed("• a\n• |\ntail").as_deref(), Some("• a\n|\ntail"));
        assert_eq!(pressed("plain|"), None);
        assert_eq!(pressed("|• Milk"), None);
    }

    #[test]
    fn a_button_toggles_the_chosen_lines_and_the_caret_stays_with_its_text() {
        let text = "Buy\nMilk\nEggs";
        let (listed, on) = toggle(text, Some(1..=2), ListKind::Numbers);
        assert_eq!((listed.as_str(), on), ("Buy\n1. Milk\n2. Eggs", true));
        assert_eq!(moved(text, &listed, text.len()), listed.len());
        assert_eq!(listed.get(moved(text, &listed, 4)..).and_then(|s| s.get(..4)), Some("Milk"));
        assert_eq!(toggle(&listed, Some(1..=2), ListKind::Numbers), (text.to_string(), false));
        // A caret inside a marker that changes lands on a character.
        let after = set_list("1. a", Some(ListKind::Bullets));
        assert!(after.is_char_boundary(moved("1. a", &after, 1)));
    }

    #[test]
    fn lines_become_a_list_change_kind_and_go_back() {
        let plain = "Milk\n\n  Eggs\nBread";
        let bullets = set_list(plain, Some(ListKind::Bullets));
        assert_eq!(bullets, "• Milk\n\n  • Eggs\n• Bread");
        assert_eq!(list_kind(&bullets), Some(ListKind::Bullets));
        let numbers = set_list(&bullets, Some(ListKind::Numbers));
        assert_eq!(numbers, "1. Milk\n\n  2. Eggs\n3. Bread");
        assert_eq!(list_kind(&numbers), Some(ListKind::Numbers));
        assert_eq!(set_list(&numbers, None), plain);
        assert_eq!(list_kind(plain), None);
        assert_eq!(set_list("", Some(ListKind::Bullets)), "• ");
    }

    #[test]
    fn numbers_count_from_the_first_and_text_that_only_looks_marked_stays() {
        assert_eq!(set_list("4) four\nfive\n9. six", Some(ListKind::Numbers)), "4. four\n5. five\n6. six");
        for text in ["-5 degrees", "1.5 litres", "2026. We began", "J. Smith", "*note*"] {
            assert_eq!(list_kind(text), None, "{text}");
            assert_eq!(set_list(text, None), text);
        }
        // Mixed markers are no one list.
        assert_eq!(list_kind("- a\n1. b"), None);
        assert_eq!(set_list("- a\n1. b", None), "a\nb");
    }
}
