//! Bulleted and numbered lists in a text's typed lines: one item to a line, its marker written as
//! ordinary characters before it, so the list is plain text in the file and in any other program.
//! A blank line is not an item; numbering carries on past it.

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

#[cfg(test)]
mod tests {
    use super::{ListKind, list_kind, set_list};

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
