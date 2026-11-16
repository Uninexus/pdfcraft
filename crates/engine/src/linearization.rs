//! A read-only report on a file's linearization parameter dictionary (ISO 32000-2 Annex F, "Fast
//! Web View"): what the file declares and whether the declaration still holds for the bytes on
//! disk.
//!
//! The report reads only the first kilobytes of the file and never builds an object graph, so
//! a hostile file costs a bounded scan. It checks the parameter dictionary against the file's
//! length and offsets (the checks a reader makes before trusting it); it does not decode the hint
//! tables or check the order of the objects. An incremental update appends bytes after the
//! declared length `/L`, so a linearized file that has been saved incrementally no longer
//! validates: readers then fall back to reading it as an ordinary file. `/L` is compared with the
//! length of the whole file, as written; bytes before the `%PDF-` header are reported as a
//! problem, since readers differ on whether offsets count from the header or from the file start.

/// How far into the file the dictionary is looked for. This is a tolerance: Annex F.3.2 requires
/// the whole dictionary to lie within the first 1024 bytes, and a dictionary that ends beyond
/// them is found here and reported as a problem.
const WINDOW: usize = 4096;
/// The header must start in the first 1024 bytes (7.5.2), and the dictionary must end in them.
const HEADER_WINDOW: usize = 1024;
/// Integers above this are not exact as floats and are refused.
const MAX_EXACT: u64 = 1 << 53;
/// Entries read from the dictionary, and numbers from one array.
const MAX_KEYS: usize = 32;
const MAX_ARRAY: usize = 16;

/// One `/H` entry: where a hint stream starts and how long it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HintStream {
    pub offset: u64,
    pub length: u64,
}

/// What the file declares, and whether it holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Linearization {
    /// The first object after the header is a linearization parameter dictionary.
    pub linearized: bool,
    /// `linearized`, and every check on its entries passed (`problems` is empty).
    pub valid: bool,
    /// `/Linearized`, the version number of the linearization (1.0).
    pub version: Option<f64>,
    /// The file's length now.
    pub file_length: u64,
    /// `/L`: the file's length as written when it was linearized.
    pub declared_length: Option<u64>,
    /// Bytes after the declared length: incremental updates saved since (0 when none).
    pub appended_bytes: u64,
    /// `/O`: the object number of the first page's page object.
    pub first_page_object: Option<u64>,
    /// `/E`: the offset of the end of the first page's section.
    pub first_page_end: Option<u64>,
    /// `/N`: the number of pages.
    pub page_count: Option<u64>,
    /// `/T`: the offset of the white space before the first entry of the main cross-reference
    /// table (Annex F.3.2).
    pub main_xref_offset: Option<u64>,
    /// `/P`: the first page's page number (0 when absent).
    pub first_page_number: u64,
    /// `/H`: the primary hint stream, then the overflow hint stream when there is one.
    pub hint_streams: Vec<HintStream>,
    /// What does not hold, one line each.
    pub problems: Vec<String>,
}

/// Reads the linearization parameter dictionary of `bytes`, the whole file.
pub fn report(bytes: &[u8]) -> Linearization {
    let mut out = Linearization { file_length: bytes.len() as u64, ..Linearization::default() };
    let Some(dict) = parameter_dictionary(bytes) else {
        return out;
    };
    out.linearized = true;
    let get = |key: &str| dict.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_slice());
    let int = |key: &str| match get(key) {
        Some([n]) => n.int,
        _ => None,
    };
    out.version = match get("Linearized") {
        Some([n]) if n.value.is_finite() => Some(n.value),
        _ => None,
    };
    out.declared_length = int("L");
    out.first_page_object = int("O");
    out.first_page_end = int("E");
    out.page_count = int("N");
    out.main_xref_offset = int("T");
    out.first_page_number = int("P").unwrap_or(0);
    let mut problems = Vec::new();
    if dict.header > 0 {
        problems.push(format!(
            "{} bytes come before the %PDF- header: readers differ on whether offsets count from the header or from the start of the file",
            dict.header
        ));
    }
    if dict.end > HEADER_WINDOW {
        problems.push(format!(
            "the linearization dictionary ends at byte {}, past the first {HEADER_WINDOW} bytes where Annex F.3.2 requires it to lie",
            dict.end
        ));
    }
    if out.version.is_none() {
        problems.push("/Linearized is not a number".to_string());
    }
    for (key, value) in
        [("L", out.declared_length), ("O", out.first_page_object), ("E", out.first_page_end), ("N", out.page_count), ("T", out.main_xref_offset)]
    {
        if value.is_none() {
            problems.push(format!("/{key} is missing or is not a plain non-negative integer"));
        }
    }
    match get("H") {
        Some(h) if h.len() == 2 || h.len() == 4 => {
            for pair in h.chunks(2) {
                match pair {
                    [Num { int: Some(o), .. }, Num { int: Some(l), .. }] => out.hint_streams.push(HintStream { offset: *o, length: *l }),
                    _ => problems.push("/H holds a value that is not a non-negative integer".to_string()),
                }
            }
        }
        Some(_) => problems.push("/H must hold two numbers (hint stream) or four (with an overflow hint stream)".to_string()),
        None => problems.push("/H is missing".to_string()),
    }
    let len = out.file_length;
    if let Some(l) = out.declared_length {
        if len > l {
            out.appended_bytes = len - l;
            problems.push(format!(
                "the file is {len} bytes but /L says {l}: {} bytes were appended after it was linearized (for example an incremental update), so readers treat it as an ordinary file",
                len - l
            ));
        } else if len < l {
            problems.push(format!("the file is {len} bytes but /L says {l}: it is truncated or was changed after it was linearized"));
        }
    }
    for h in &out.hint_streams {
        if h.offset.checked_add(h.length).is_none_or(|end| end > len) {
            problems.push(format!("a hint stream at {} of {} bytes reaches past the end of the file", h.offset, h.length));
        }
    }
    for (key, value) in [("E", out.first_page_end), ("T", out.main_xref_offset)] {
        if value.is_some_and(|v| v > len) {
            problems.push(format!("/{key} points past the end of the file"));
        }
    }
    if out.page_count == Some(0) {
        problems.push("/N says the file has no pages".to_string());
    }
    if let Some(n) = out.page_count
        && out.first_page_number >= n.max(1)
    {
        problems.push("/P is not a page of the file".to_string());
    }
    out.valid = problems.is_empty();
    out.problems = problems;
    out
}

/// A number read from the dictionary.
#[derive(Clone, Copy, Debug)]
struct Num {
    value: f64,
    /// The value when the token is a plain integer: digits only, at most 2^53 (so exact as a
    /// float). A real (`4.0`), a signed value, an exponent (`1e5`) or a larger value is not one.
    int: Option<u64>,
}

/// The parameter dictionary as found.
struct Found {
    entries: Vec<(String, Vec<Num>)>,
    /// Offset of the `%PDF-` header.
    header: usize,
    /// Offset just past the dictionary's closing `>>`.
    end: usize,
}

/// The entries of the first object after the header when that is a dictionary with `/Linearized`
/// (the numbers of each value: one for a number, the elements for an array of numbers; any
/// other value is left out). The header must be in the first 1024 bytes (7.5.2); the object
/// must follow it directly (comments and white space between are skipped), so text elsewhere
/// that looks like an object is not mistaken for it.
fn parameter_dictionary(bytes: &[u8]) -> Option<Found> {
    let window = bytes.get(..bytes.len().min(WINDOW))?;
    let header = find(bytes.get(..bytes.len().min(HEADER_WINDOW))?, b"%PDF-", 0)?;
    let mut i = header;
    // Past the header line and any comment lines (the binary marker) and white space.
    loop {
        skip_space(window, &mut i);
        if window.get(i) == Some(&b'%') {
            skip_comment(window, &mut i);
        } else {
            break;
        }
    }
    // `N G obj`
    for part in 0..2 {
        let start = i;
        while window.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start || (part == 0 && i - start > 10) || (part == 1 && i - start > 5) {
            return None;
        }
        if !window.get(i).is_some_and(|c| is_space(*c)) {
            return None;
        }
        skip_space(window, &mut i);
    }
    if window.get(i..i + 3)? != b"obj" {
        return None;
    }
    i += 3;
    skip_space(window, &mut i);
    if window.get(i..i + 2)? != b"<<" {
        return None;
    }
    i += 2;
    let mut depth = 1usize;
    let mut entries: Vec<(String, Vec<Num>)> = Vec::new();
    while depth > 0 {
        skip_space(window, &mut i);
        let c = *window.get(i)?;
        match c {
            b'%' => skip_comment(window, &mut i),
            b'<' if window.get(i + 1) == Some(&b'<') => {
                depth += 1;
                i += 2;
            }
            b'>' if window.get(i + 1) == Some(&b'>') => {
                depth -= 1;
                i += 2;
            }
            b'(' => {
                // A string: skip to its matching parenthesis.
                let mut nest = 0usize;
                loop {
                    match *window.get(i)? {
                        b'\\' => i += 1,
                        b'(' => nest += 1,
                        b')' => {
                            nest -= 1;
                            if nest == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                i += 1;
            }
            b'/' => {
                let start = i + 1;
                i = start;
                while window.get(i).is_some_and(|b| !is_space(*b) && !is_delim(*b)) {
                    i += 1;
                }
                let name = decode_name(window.get(start..i)?);
                if depth != 1 || entries.len() >= MAX_KEYS {
                    continue;
                }
                loop {
                    skip_space(window, &mut i);
                    if window.get(i) == Some(&b'%') {
                        skip_comment(window, &mut i);
                    } else {
                        break;
                    }
                }
                let values = if window.get(i) == Some(&b'[') {
                    i += 1;
                    let mut nums = Vec::new();
                    loop {
                        skip_space(window, &mut i);
                        match window.get(i)? {
                            b']' => {
                                i += 1;
                                break;
                            }
                            b'%' => skip_comment(window, &mut i),
                            _ => {
                                let n = number(window, &mut i);
                                if nums.len() < MAX_ARRAY {
                                    nums.push(n.unwrap_or(Num { value: f64::NAN, int: None }));
                                }
                                if n.is_none() {
                                    // Not a number: step over a character so the scan always advances.
                                    i += 1;
                                }
                            }
                        }
                    }
                    nums
                } else {
                    number(window, &mut i).map(|n| vec![n]).unwrap_or_default()
                };
                entries.push((name, values));
            }
            _ => i += 1,
        }
    }
    entries.iter().any(|(k, _)| k == "Linearized").then_some(Found { entries, header, end: i })
}

/// A name with its `#xx` escapes decoded (7.3.5).
fn decode_name(raw: &[u8]) -> String {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while let Some(&c) = raw.get(i) {
        let hex = |k: usize| raw.get(i + k).and_then(|d| (*d as char).to_digit(16));
        match (c, hex(1), hex(2)) {
            (b'#', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A number token at `*i`, advancing past it; `None` (position unchanged) when it is not one.
fn number(b: &[u8], i: &mut usize) -> Option<Num> {
    let start = *i;
    let mut end = start;
    while b.get(end).is_some_and(|c| !is_space(*c) && !is_delim(*c)) {
        end += 1;
    }
    let token = b.get(start..end)?;
    // Only digits, signs and a point make a PDF number; `1e5` is not one.
    if token.is_empty() || !token.iter().all(|c| c.is_ascii_digit() || matches!(c, b'+' | b'-' | b'.')) {
        return None;
    }
    let value = std::str::from_utf8(token).ok()?.parse::<f64>().ok()?;
    let int =
        if token.iter().all(u8::is_ascii_digit) { std::str::from_utf8(token).ok()?.parse::<u64>().ok().filter(|v| *v <= MAX_EXACT) } else { None };
    *i = end;
    Some(Num { value, int })
}

/// Skips a `%` comment to the end of its line.
fn skip_comment(b: &[u8], i: &mut usize) {
    while b.get(*i).is_some_and(|c| !matches!(c, b'\n' | b'\r')) {
        *i += 1;
    }
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

fn skip_space(b: &[u8], i: &mut usize) {
    while b.get(*i).is_some_and(|c| is_space(*c)) {
        *i += 1;
    }
}

fn is_space(c: u8) -> bool {
    matches!(c, 0 | b'\t' | b'\n' | 0x0c | b'\r' | b' ')
}

fn is_delim(c: u8) -> bool {
    matches!(c, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file whose first object is a parameter dictionary, with `/L` the length it has.
    pub(super) fn linearized(extra: &str) -> Vec<u8> {
        let mk = |l: usize| {
            format!(
                "%PDF-1.7\n%\u{e2}\u{e3}\n1 0 obj\n<< /Linearized 1 /L {l:06} /H [ 300 40 ] /O 4 /E 900 /N 2 /T 700 {extra}>>\nendobj\n{}",
                "x".repeat(1000)
            )
            .into_bytes()
        };
        let len = mk(0).len();
        mk(len)
    }

    #[test]
    fn a_consistent_dictionary_is_valid_and_read_in_full() {
        let r = report(&linearized(""));
        assert!(r.linearized && r.valid, "{:?}", r.problems);
        assert_eq!(
            (r.version, r.first_page_object, r.first_page_end, r.page_count, r.main_xref_offset),
            (Some(1.0), Some(4), Some(900), Some(2), Some(700))
        );
        assert_eq!(r.hint_streams, vec![HintStream { offset: 300, length: 40 }]);
        assert_eq!((r.first_page_number, r.appended_bytes, r.declared_length), (0, 0, Some(r.file_length)));
    }

    #[test]
    fn bytes_appended_after_it_was_written_invalidate_it() {
        let mut b = linearized("/P 1 ");
        b.extend_from_slice(b"\n1 0 obj\n<< >>\nendobj\n");
        let r = report(&b);
        assert!(r.linearized && !r.valid);
        assert_eq!((r.appended_bytes, r.first_page_number), (22, 1));
        assert!(r.problems.iter().any(|p| p.contains("incremental update")), "{:?}", r.problems);
    }

    #[test]
    fn a_truncated_file_and_offsets_past_the_end_are_named() {
        let mut b = linearized("");
        b.truncate(b.len() - 600);
        let r = report(&b);
        assert!(!r.valid);
        assert!(r.problems.iter().any(|p| p.contains("truncated")), "{:?}", r.problems);
        assert!(r.problems.iter().any(|p| p.contains("/E points past")), "{:?}", r.problems);
    }

    #[test]
    fn an_ordinary_file_is_not_linearized() {
        let r = report(b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\n");
        assert!(!r.linearized && !r.valid && r.problems.is_empty());
        assert!(!report(b"").linearized);
        // The keyword later in the file does not count: only the first object after the header does.
        let mut late = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\n".to_vec();
        late.extend_from_slice(b"2 0 obj\n<< /Linearized 1 /L 10 /H [1 2] /O 1 /E 1 /N 1 /T 1 >>\nendobj\n");
        assert!(!report(&late).linearized);
    }

    #[test]
    fn hostile_dictionaries_are_reported_not_trusted() {
        let cases: [&[u8]; 9] = [
            b"%PDF-1.7\n1 0 obj\n<< /Linearized 1 /L 99999999999999999999999 /H [ 1 ] /O -4 /E 1e5 /N 0 /T 1 >>",
            b"%PDF-1.7\n1 0 obj\n<< /Linearized 1 /H [ 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 ] >>",
            b"%PDF-1.7\n1 0 obj\n<< /Linearized /H [ (a) /b << >> 1 2 ] /L (unterminated",
            b"%PDF-1.7\n1 0 obj\n<< /Linearized 1 /H [ 1 2",
            b"%PDF-1.7\n1 0 obj\n<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<<< /Linearized 1",
            b"%PDF-1.7\n1 0 obj\n<< /Linearized 1 /H [ 9007199254740992 9007199254740992 ] /L 5 >>",
            b"%PDF-1.7\n1 0 obj\n<< /Linearized 1 /L 0 /O 0 /E 0 /N 1 /T 0 /P 5 /H [0 0] >>",
            b"1 0 obj << /Linearized",
            b"%PDF-1.7\n1 0 obj << /Linearized [ [ [ ] ",
        ];
        for c in cases {
            let r = report(c);
            assert!(!r.valid, "{}", String::from_utf8_lossy(c));
        }
        let huge = report(cases[0]);
        assert!(huge.linearized && huge.declared_length.is_none() && huge.problems.iter().any(|p| p.contains("/L")));
        // A hint stream of 2^53 bytes at offset 2^53 is past the end, and a larger value is refused.
        assert!(report(b"%PDF-1.7\n1 0 obj\n<< /Linearized 1 /H [ 18446744073709551615 1 ] >>").problems.iter().any(|p| p.contains("/H holds")));
        assert!(report(cases[5]).problems.iter().any(|p| p.contains("reaches past")));
        assert_eq!(report(cases[1]).hint_streams.len(), 0);
    }

    #[test]
    fn a_dictionary_past_the_first_1024_bytes_is_named() {
        let mut b = b"%PDF-1.7\n".to_vec();
        b.extend_from_slice("% padding ".repeat(110).as_bytes());
        b.extend_from_slice(b"\n");
        let head = b.len();
        b.extend_from_slice(&linearized("")[9..]);
        let r = report(&b);
        assert!(r.linearized && head > 1024 && !r.valid);
        assert!(r.problems.iter().any(|p| p.contains("past the first 1024")), "{:?}", r.problems);
    }

    #[test]
    fn a_header_is_required_and_leading_bytes_are_reported() {
        // No header: not linearized.
        let b = linearized("");
        assert!(!report(&b[9..]).linearized);
        // Leading garbage that contains `obj` neither hides nor fakes the dictionary.
        let mut g = b"junk 9 9 obj << /Linearized 1 >> ".to_vec();
        g.extend_from_slice(&b);
        let r = report(&g);
        assert!(r.linearized && !r.valid);
        assert_eq!(r.version, Some(1.0));
        assert!(r.problems.iter().any(|p| p.contains("before the %PDF- header")), "{:?}", r.problems);
        // Garbage with an `obj` and no header at all.
        assert!(!report(b"zz 1 0 obj << /Linearized 1 /L 9 /H [1 2] /O 1 /E 1 /N 1 /T 1 >>").linearized);
        // The header, then something that is not an object.
        assert!(!report(b"%PDF-1.7\nstream 1 0 obj << /Linearized 1 >>").linearized);
    }

    #[test]
    fn only_plain_integers_count_as_integers() {
        for bad in ["4.0", "1e5", "+4", "-4", "9007199254740993", "99999999999999999999"] {
            let b = linearized("").to_vec();
            let text = String::from_utf8_lossy(&b).replace("/O 4", &format!("/O {bad}"));
            let r = report(text.as_bytes());
            assert_eq!(r.first_page_object, None, "{bad}");
            assert!(r.problems.iter().any(|p| p.contains("/O is missing or is not a plain")), "{bad}: {:?}", r.problems);
        }
        // The version is a real and stays one; /H entries are integers too.
        assert_eq!(report(&linearized("")).version, Some(1.0));
        let text = String::from_utf8_lossy(&linearized("")).replace("/H [ 300 40 ]", "/H [ 300.0 40 ]");
        assert!(report(text.as_bytes()).problems.iter().any(|p| p.contains("/H holds")));
    }

    #[test]
    fn comments_and_name_escapes_in_the_dictionary_are_read() {
        let text = String::from_utf8_lossy(&linearized(""))
            .replace("/N 2", "% a comment with /N 9 in it\n/N 2")
            .replace("/T 700", "/T % mid-entry comment\n 700")
            .replace("/E 900", "/#45 900");
        let r = report(text.as_bytes());
        assert_eq!((r.page_count, r.main_xref_offset, r.first_page_end), (Some(2), Some(700), Some(900)), "{:?}", r.problems);
        assert!(r.linearized);
    }
}
