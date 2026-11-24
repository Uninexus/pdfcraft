//! Bates numbering over several files: the rules the batch tool (`doc_bates_multi`) and the Bates numbering on several
//! files dialog share. The numbers run on from one file to the next, in the order the files are given.

/// Longest prefix or suffix, in characters.
pub const MAX_AFFIX_CHARS: usize = 128;

/// Most digits a number is padded to.
pub const MAX_DIGITS: u8 = 12;

/// Why numbers can't be given out to a list of files. The index is the file's place in the list, from 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntervalError {
    /// The file has no pages to number.
    NoPages(usize),
    /// The numbers run past the largest the counter holds.
    Overflow(usize),
}

/// Whether a prefix and a suffix can be part of a Bates number: at most [`MAX_AFFIX_CHARS`] characters, and neither `#`
/// nor `>>`, which would end the number's field.
pub fn affixes_ok(prefix: &str, suffix: &str) -> bool {
    [prefix, suffix].iter().all(|text| text.chars().count() <= MAX_AFFIX_CHARS && !text.contains('#') && !text.contains(">>"))
}

/// The header or footer text that numbers a file's pages from `first`.
pub fn template(digits: u8, first: u64, prefix: &str, suffix: &str) -> String {
    format!("<<Bates Number#{digits}#{first}#{prefix}#{suffix}>>")
}

/// The first and last number of each file, from `start`, given each file's page count.
pub fn intervals(start: u64, pages: &[usize]) -> Result<Vec<(u64, u64)>, IntervalError> {
    let mut next = start;
    let mut out = Vec::with_capacity(pages.len());
    for (index, count) in pages.iter().enumerate() {
        let count = u64::try_from(*count).ok().filter(|c| *c > 0).ok_or(IntervalError::NoPages(index))?;
        let last = next.checked_add(count - 1).ok_or(IntervalError::Overflow(index))?;
        out.push((next, last));
        if index + 1 < pages.len() {
            next = last.checked_add(1).ok_or(IntervalError::Overflow(index))?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_continue_from_file_to_file() {
        assert_eq!(intervals(5, &[3, 1, 2]), Ok(vec![(5, 7), (8, 8), (9, 10)]));
    }

    #[test]
    fn a_file_without_pages_or_past_the_largest_number_is_named_by_its_place() {
        assert_eq!(intervals(1, &[2, 0]), Err(IntervalError::NoPages(1)));
        assert_eq!(intervals(u64::MAX - 1, &[1, 1, 1]), Err(IntervalError::Overflow(1)));
    }

    #[test]
    fn a_prefix_that_would_end_the_field_is_refused() {
        assert!(affixes_ok("ABC-", ""));
        for bad in ["a#b", "a>>b"] {
            assert!(!affixes_ok(bad, "") && !affixes_ok("", bad), "{bad}");
        }
        assert!(!affixes_ok(&"x".repeat(MAX_AFFIX_CHARS + 1), ""));
    }
}
