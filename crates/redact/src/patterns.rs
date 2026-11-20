//! Search & Redact patterns (Acrobat's Find Text ▸ Patterns): phone numbers, email addresses,
//! credit card numbers, US Social Security numbers, dates and IBANs, plus Canadian SINs and postal
//! codes, UK National Insurance numbers and Australian Tax File Numbers, and US Individual
//! Taxpayer Identification Numbers. Each matcher works on a page's
//! text as characters and returns character ranges; callers map them to glyphs and areas.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pattern {
    Phone,
    Email,
    CreditCard,
    Ssn,
    Date,
    Iban,
    /// Canadian Social Insurance Number (nine digits, Luhn checksum).
    Sin,
    /// Canadian postal code ("K1A 0B1").
    CaPostalCode,
    /// United Kingdom National Insurance number ("QQ 12 34 56 C"), by its format only.
    Nino,
    /// Australian Tax File Number ("123 456 782").
    Tfn,
    /// US Individual Taxpayer Identification Number ("912-70-1234").
    Itin,
}

pub const PATTERNS: [Pattern; 11] = [
    Pattern::Phone,
    Pattern::Email,
    Pattern::CreditCard,
    Pattern::Ssn,
    Pattern::Date,
    Pattern::Iban,
    Pattern::Sin,
    Pattern::CaPostalCode,
    Pattern::Nino,
    Pattern::Tfn,
    Pattern::Itin,
];

impl Pattern {
    pub fn label(self) -> &'static str {
        match self {
            Pattern::Phone => "Phone Numbers",
            Pattern::Email => "Email Addresses",
            Pattern::CreditCard => "Credit Cards",
            Pattern::Ssn => "Social Security Numbers",
            Pattern::Date => "Dates",
            Pattern::Iban => "IBAN Numbers",
            Pattern::Sin => "Canadian Social Insurance Numbers",
            Pattern::CaPostalCode => "Canadian Postal Codes",
            Pattern::Nino => "UK National Insurance Numbers",
            Pattern::Tfn => "Australian Tax File Numbers",
            Pattern::Itin => "US Individual Taxpayer Identification Numbers",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Pattern::Phone => "phone",
            Pattern::Email => "email",
            Pattern::CreditCard => "credit-card",
            Pattern::Ssn => "ssn",
            Pattern::Date => "date",
            Pattern::Iban => "iban",
            Pattern::Sin => "sin",
            Pattern::CaPostalCode => "ca-postal-code",
            Pattern::Nino => "nino",
            Pattern::Tfn => "tfn",
            Pattern::Itin => "itin",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        PATTERNS.into_iter().find(|p| p.id() == id)
    }
}

fn digit(c: Option<&char>) -> bool {
    c.is_some_and(char::is_ascii_digit)
}

fn word(c: Option<&char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric())
}

/// Digits and the separators allowed between them, from `start`: (end, digit count).
fn digit_run(s: &[char], start: usize, seps: &[char], max_len: usize) -> (usize, usize) {
    let (mut i, mut n) = (start, 0);
    while i < s.len() && i - start < max_len {
        let c = s[i];
        if c.is_ascii_digit() {
            n += 1;
        } else if !(seps.contains(&c)
            && (digit(s.get(i + 1)) || s.get(i + 1) == Some(&'('))
            // A slash only ever joins two digits ("0426/123456").
            && (c != '/' || digit(s.get(i + 1)) && digit(i.checked_sub(1).and_then(|p| s.get(p))))
            || (c == '(' || c == ')') && seps.contains(&c))
        {
            break;
        }
        i += 1;
    }
    // Never end on a separator.
    while i > start && !s[i - 1].is_ascii_digit() {
        i -= 1;
    }
    (i, n)
}

/// Whether the separated digit run goes on past `k` in the direction `step` (1 or −1): the next
/// digit after `k` is reached through separators only.
fn run_continues(s: &[char], k: usize, step: isize, seps: &[char]) -> bool {
    let mut j = k;
    loop {
        let Some(c) = j.checked_add_signed(step).and_then(|j| s.get(j)) else { return false };
        if c.is_ascii_digit() {
            return true;
        }
        if !seps.contains(c) {
            return false;
        }
        j = j.wrapping_add_signed(step);
    }
}

/// Words that label the number after them as a phone number, lowercased ("Tel.", "cell.", "Fax:").
const PHONE_LABELS: [&str; 24] = [
    "tel",
    "tél",
    "tele",
    "telefon",
    "telefono",
    "teléfono",
    "telephone",
    "téléphone",
    "phone",
    "ph",
    "cell",
    "cel",
    "cellulare",
    "cellphone",
    "mobile",
    "mob",
    "mobil",
    "móvil",
    "movil",
    "handy",
    "fax",
    "tlf",
    "telf",
    "gsm",
];

/// Whether the text before `i` is a phone label, with one to four spaces, colons or dots between.
fn labelled(s: &[char], i: usize) -> bool {
    let before = |k: usize| k.checked_sub(1).and_then(|p| s.get(p)).copied();
    let mut j = i;
    while i - j < 4 && before(j).is_some_and(|c| matches!(c, ' ' | ':' | '.' | '\u{a0}')) {
        j -= 1;
    }
    let end = j;
    while end - j < 12 && before(j).is_some_and(char::is_alphabetic) {
        j -= 1;
    }
    if end == i || before(j).is_some_and(char::is_alphanumeric) {
        return false;
    }
    let label: String = s.get(j..end).unwrap_or_default().iter().flat_map(|c| c.to_lowercase()).collect();
    PHONE_LABELS.contains(&label.as_str())
}

fn phone(s: &[char], i: usize) -> Option<usize> {
    const SEPS: [char; 5] = [' ', '-', '.', '(', ')'];
    // A slash joins the area code to the number in Italian and German landlines ("0426/123456").
    const RUN_SEPS: [char; 6] = [' ', '-', '.', '(', ')', '/'];
    let start_ok = s[i] == '+' && digit(s.get(i + 1)) || s[i] == '(' && digit(s.get(i + 1)) || s[i].is_ascii_digit();
    if !start_ok || word(i.checked_sub(1).and_then(|p| s.get(p))) || s.get(i.wrapping_sub(1)) == Some(&'+') {
        return None;
    }
    // A phone number is a whole run of digit groups. The tail of a longer one (an IBAN, a
    // company or social security number) is not a phone number, however it is grouped (#1884).
    if run_continues(s, i, -1, &SEPS) {
        return None;
    }
    let plus = s[i] == '+';
    let first = if plus { i + 1 } else { i };
    let (end, n) = digit_run(s, first, &RUN_SEPS, 28);
    let run = s.get(first..end)?;
    let seps = run.iter().filter(|c| !c.is_ascii_digit()).count();
    // Grouped by at least one separator, or labelled ("cell. 3451234567", #3205), so plain
    // numbers never match; nor does the head of a run too long to be a phone number.
    if seps == 0 && !labelled(s, i) || digit(s.get(end)) || run_continues(s, end.checked_sub(1)?, 1, &SEPS) {
        return None;
    }
    let slash = run.contains(&'/');
    // Italian geographic numbers retain their initial 0 after +39. For a labelled number,
    // apply the national slash rule after that country code and an optional separating space.
    // AGCOM's E.164 plan gives geographic numbers 6 to 11 national digits (#3205).
    let slash_run = if plus && slash {
        if !labelled(s, i) || !(8..=13).contains(&n) {
            return None;
        }
        let national = run.strip_prefix(&['3', '9'])?;
        national.strip_prefix(&[' ']).unwrap_or(national)
    } else {
        run
    };
    // Only an area code beginning with 0 takes a slash, so "2024/123456" is no phone number;
    // and only one, right after it (not "0426/12/3456" or "0426 12/3456").
    if run.iter().filter(|c| **c == '/').count() > 1
        || slash
            && (slash_run.first() != Some(&'0') || !digit(slash_run.get(1)) || !slash_run.iter().take_while(|c| **c != '/').all(char::is_ascii_digit))
    {
        return None;
    }
    let ok = if plus {
        // E.164: a `+` country code and up to 15 digits in all ("+49 2151 123456").
        (8..=15).contains(&n)
    } else {
        // North American: 10 digits, 11 with a country code ("(555) 123-4567").
        let nanp = (10..=11).contains(&n) && !slash;
        // National with a trunk 0 and 6–14 more digits ("0211 123456", "01 23 45 67 89").
        // The first group needs two digits so decimals ("0.1234567") don't match, and
        // day-first dates ("01.10.2026") are left to the date pattern.
        let mut digits = run.iter().skip_while(|c| !c.is_ascii_digit());
        let trunk_zero = digits.clone().next() == Some(&'0');
        let lead = digits.by_ref().take_while(|c| c.is_ascii_digit()).count();
        let trunk = trunk_zero && (7..=15).contains(&n) && lead >= 2 && date(s, i).is_none();
        nanp || trunk
    };
    ok.then_some(end - i)
}

/// Scripts written without spaces between words (Chinese, Japanese, Korean, Thai, …): text in
/// them often runs straight into an address, so their letters never count as part of one.
fn spaceless(c: char) -> bool {
    matches!(
        u32::from(c),
        0x0E00..=0x0EFF | 0x1000..=0x109F | 0x1100..=0x11FF | 0x1780..=0x17FF | 0x2E80..=0x9FFF | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF
    )
}

/// A letter or digit of an address: ASCII, or of any script written with spaces, so
/// internationalized addresses (RFC 6531, IDN) such as "ayşe@örnek.com.tr" match.
fn address_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c.is_alphanumeric() && !spaceless(c)
}

fn email(s: &[char], i: usize) -> Option<usize> {
    let local = |c: &char| address_char(*c) || "._%+-".contains(*c);
    // A match starts at the first character of a run of local characters, which may be
    // punctuation ("_admin@host.com"), but the run needs a letter or digit.
    // An apostrophe ("o'brien@example.com") belongs to the local part but never starts it, so a
    // quote around an address ("'john@example.com'") stays outside the match.
    let inner = |c: &char| local(c) || *c == '\'';
    let before = |n: usize| i.checked_sub(n).and_then(|p| s.get(p));
    if !local(&s[i]) || before(1).is_some_and(local) || (before(1) == Some(&'\'') && before(2).is_some_and(inner)) {
        return None;
    }
    let mut j = i;
    while j < s.len() && inner(&s[j]) {
        j += 1;
    }
    if s.get(j) != Some(&'@') || !s[i..j].iter().any(|c| address_char(*c)) {
        return None;
    }
    let d0 = j + 1;
    let mut k = d0;
    while k < s.len() && (address_char(s[k]) || s[k] == '-' || s[k] == '.' && s.get(k + 1).is_some_and(|c| address_char(*c))) {
        k += 1;
    }
    let domain: String = s[d0..k].iter().collect();
    let tld = domain.rsplit('.').next().unwrap_or("");
    (domain.contains('.') && tld.chars().count() >= 2 && tld.chars().all(char::is_alphabetic)).then_some(k - i)
}

fn luhn(digits: &[u32]) -> bool {
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(k, d)| {
            if k % 2 == 1 {
                let x = d * 2;
                if x > 9 { x - 9 } else { x }
            } else {
                *d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

fn credit_card(s: &[char], i: usize) -> Option<usize> {
    if !s[i].is_ascii_digit() || digit(i.checked_sub(1).and_then(|p| s.get(p))) {
        return None;
    }
    // Room for the longest card (19 digits in groups, 23 characters) and a tail of up to four digits.
    let (end, n) = digit_run(s, i, &[' ', '-'], 29);
    // A run longer than that is some other, longer number.
    if end == i || run_continues(s, end - 1, 1, &[' ', '-']) {
        return None;
    }
    // The longest run may carry a trailing number (an expiry, a year, a CVV), so try it and then
    // each shorter run ending at a group boundary, longest first. The dropped tail is at most four
    // digits, so a longer number (an IBAN's digits) doesn't match on a prefix.
    let mut count = n;
    for e in (i + 1..=end).rev() {
        if n - count > 4 {
            break;
        }
        if !s.get(e - 1).is_some_and(char::is_ascii_digit) {
            continue;
        }
        let boundary = if e == end { !digit(s.get(end)) } else { matches!(s.get(e), Some(' ' | '-')) };
        if boundary && (13..=19).contains(&count) {
            let digits: Vec<u32> = s[i..e].iter().filter_map(|c| c.to_digit(10)).collect();
            if luhn(&digits) {
                return Some(e - i);
            }
        }
        count = count.saturating_sub(1);
    }
    None
}

fn ssn(s: &[char], i: usize) -> Option<usize> {
    if digit(i.checked_sub(1).and_then(|p| s.get(p))) {
        return None;
    }
    let g = |from: usize, n: usize| (from..from + n).all(|k| digit(s.get(k)));
    let sep = |k: usize| matches!(s.get(k), Some('-' | ' '));
    if !(g(i, 3) && sep(i + 3) && g(i + 4, 2) && sep(i + 6) && g(i + 7, 4)) || digit(s.get(i + 11)) {
        return None;
    }
    let area: String = s[i..i + 3].iter().collect();
    (area != "000" && area != "666" && !area.starts_with('9')).then_some(11)
}

/// Characters that may join the groups of a SIN: hyphen, space, no-break space, non-breaking
/// hyphen, en dash and full stop ("130-692-544", "130.692.544").
fn sin_separator(c: char) -> bool {
    matches!(c, '-' | ' ' | '\u{a0}' | '\u{2011}' | '\u{2013}' | '.')
}

/// A Canadian Social Insurance Number: three groups of three digits joined by one consistent
/// separator ([`sin_separator`]), or nine plain digits after a label ([`sin_labelled`]); Luhn
/// checksum valid and a first digit that Canada issues (not 0 or 8). The number may not touch
/// other digits. A SIN left unredacted is a privacy leak, so the labels are generous.
fn sin(s: &[char], i: usize) -> Option<usize> {
    if digit(i.checked_sub(1).and_then(|p| s.get(p))) {
        return None;
    }
    let g = |from: usize, n: usize| (from..from + n).all(|k| digit(s.get(k)));
    let (len, ok) = match s.get(i + 3) {
        Some(&c) if sin_separator(c) => (11, g(i, 3) && s.get(i + 7) == Some(&c) && g(i + 4, 3) && g(i + 8, 3)),
        _ => (9, g(i, 9) && sin_labelled(s, i)),
    };
    // A longer digit run is another number.
    if !ok || digit(s.get(i + len)) {
        return None;
    }
    let digits: Vec<u32> = s.get(i..i + len)?.iter().filter_map(|c| c.to_digit(10)).collect();
    (digits.first().is_some_and(|d| !matches!(d, 0 | 8)) && luhn(&digits)).then_some(len)
}

/// Labels (lowercase) that name the number after them a SIN; French "NAS" included.
const SIN_LABELS: [&str; 14] = [
    "sin",
    "s.i.n",
    "sin no",
    "sin num",
    "sin number",
    "social insurance number",
    "social insurance no",
    "social insurance num",
    "nas",
    "n.a.s",
    "numéro d'assurance sociale",
    "numero d'assurance sociale",
    "numéro d\u{2019}assurance sociale",
    "numero d\u{2019}assurance sociale",
];

/// Whether a SIN label, in any case, stands directly before `i` with up to six spaces, no-break
/// spaces, colons, dots, hyphens or hashes between, so nine unseparated digits are read as a SIN.
fn sin_labelled(s: &[char], i: usize) -> bool {
    labelled_by(s, i, &SIN_LABELS)
}

/// [`sin_labelled`] for any list of lowercase labels.
fn labelled_by(s: &[char], i: usize, labels: &[&str]) -> bool {
    let mut k = i;
    let mut gap = 0;
    while let Some(c) = k.checked_sub(1).and_then(|p| s.get(p)) {
        if !matches!(c, ' ' | '\u{a0}' | ':' | '#' | '.' | '-') || gap == 6 {
            break;
        }
        gap += 1;
        k -= 1;
    }
    let head = s.get(..k).unwrap_or_default();
    let tail: Vec<char> = head.iter().rev().take(30).rev().flat_map(|c| c.to_lowercase()).collect();
    let tail: String = tail.into_iter().map(|c| if c == '\u{a0}' { ' ' } else { c }).collect();
    labels.iter().any(|l| tail.strip_suffix(l).is_some_and(|rest| !rest.chars().last().is_some_and(char::is_alphanumeric)))
}

/// Labels (lowercase) that name the number after them an Australian Tax File Number.
const TFN_LABELS: [&str; 5] = ["tfn", "t.f.n", "tfn no", "tfn number", "tax file number"];

/// An Australian Tax File Number of nine digits: three groups of three joined by one consistent
/// hyphen, space or no-break space, or nine plain digits after a label ([`TFN_LABELS`], read like
/// the SIN labels). The weighted checksum of the Australian Taxation Office's published
/// check-digit rule must hold: the digits times 1, 4, 3, 7, 5, 8, 6, 9 and 10 sum to a multiple
/// of 11. The number may not touch other digits. The older eight-digit form is not matched, and
/// about one nine-digit number in eleven passes the checksum, so a match is a candidate.
fn tfn(s: &[char], i: usize) -> Option<usize> {
    if digit(i.checked_sub(1).and_then(|p| s.get(p))) {
        return None;
    }
    let g = |from: usize, n: usize| (from..from + n).all(|k| digit(s.get(k)));
    let (len, ok) = match s.get(i + 3) {
        Some(&c) if matches!(c, '-' | ' ' | '\u{a0}') => (11, g(i, 3) && s.get(i + 7) == Some(&c) && g(i + 4, 3) && g(i + 8, 3)),
        _ => (9, g(i, 9) && labelled_by(s, i, &TFN_LABELS)),
    };
    if !ok || digit(s.get(i + len)) {
        return None;
    }
    const WEIGHTS: [u32; 9] = [1, 4, 3, 7, 5, 8, 6, 9, 10];
    let sum: u32 = s.get(i..i + len)?.iter().filter_map(|c| c.to_digit(10)).zip(WEIGHTS).map(|(d, w)| d * w).sum();
    sum.is_multiple_of(11).then_some(len)
}

/// Labels (lowercase) that name the number after them a US Individual Taxpayer Identification
/// Number.
const ITIN_LABELS: [&str; 4] = ["itin", "i.t.i.n", "itin no", "itin number"];

/// A US Individual Taxpayer Identification Number: nine digits as 3-2-4 joined by one consistent
/// hyphen, space or no-break space, or nine plain digits after a label ([`ITIN_LABELS`], read like
/// the SIN labels). It begins with 9 and its fourth and fifth digits are 50 to 65, 70 to 88, 90
/// to 92 or 94 to 99, the ranges the Internal Revenue Service publishes. There is no checksum, so
/// a match is a candidate. Social Security numbers never begin with 9, so this does not overlap
/// [`ssn`]. The number may not touch other digits.
fn itin(s: &[char], i: usize) -> Option<usize> {
    if digit(i.checked_sub(1).and_then(|p| s.get(p))) {
        return None;
    }
    let g = |from: usize, n: usize| (from..from + n).all(|k| digit(s.get(k)));
    let (len, ok) = match s.get(i + 3) {
        Some(&c) if matches!(c, '-' | ' ' | '\u{a0}') => (11, g(i, 3) && s.get(i + 6) == Some(&c) && g(i + 4, 2) && g(i + 7, 4)),
        _ => (9, g(i, 9) && labelled_by(s, i, &ITIN_LABELS)),
    };
    if !ok || digit(s.get(i + len)) {
        return None;
    }
    let digits: Vec<u32> = s.get(i..i + len)?.iter().filter_map(|c| c.to_digit(10)).collect();
    let group = digits.get(3).copied()? * 10 + digits.get(4).copied()?;
    (digits.first() == Some(&9) && matches!(group, 50..=65 | 70..=88 | 90..=92 | 94..=99)).then_some(len)
}

/// A Canadian postal code: letter, digit, letter, an optional space or hyphen, digit, letter,
/// digit ("K1A 0B1"), in either case. The first letter is one of Canada's postal districts and D,
/// F, I, O, Q and U are never used; neither end may touch a letter or digit.
fn ca_postal_code(s: &[char], i: usize) -> Option<usize> {
    if word(i.checked_sub(1).and_then(|p| s.get(p))) {
        return None;
    }
    let at = |k: usize| s.get(k).map(char::to_ascii_uppercase);
    let letter = |k: usize| at(k).is_some_and(|c| c.is_ascii_uppercase() && !"DFIOQU".contains(c));
    let first = at(i).is_some_and(|c| "ABCEGHJKLMNPRSTVXY".contains(c));
    let gap = usize::from(matches!(s.get(i + 3), Some(' ' | '-' | '\u{a0}')));
    let n = i + 3 + gap;
    let ok = first && digit(s.get(i + 1)) && letter(i + 2) && digit(s.get(n)) && letter(n + 1) && digit(s.get(n + 2));
    (ok && !word(s.get(n + 3))).then_some(n + 3 - i)
}

/// A United Kingdom National Insurance number by its format alone (there is no checksum): two
/// letters, six digits and a final letter A to D, in either case. A single space or no-break space
/// may stand after the letters, between the three pairs of digits and before the final letter, in
/// any of those places or none ("QQ123456C", "QQ 12 34 56 C", "AB 123456 C"), but every joiner in
/// one number is the same character. The first letter is never D, F, I, Q, U or V, the second
/// never D, F, I, O, Q, U or V, and the prefixes BG, GB, KN, NK, NT, TN and ZZ are not allocated.
/// Neither end may touch a letter or digit. The format also fits other codes, so a match is a
/// candidate, not proof of a number.
fn nino(s: &[char], i: usize) -> Option<usize> {
    if word(i.checked_sub(1).and_then(|p| s.get(p))) {
        return None;
    }
    let at = |k: usize| s.get(k).map(char::to_ascii_uppercase);
    let (a, b) = (at(i)?, at(i + 1)?);
    if !a.is_ascii_uppercase()
        || !b.is_ascii_uppercase()
        || "DFIQUV".contains(a)
        || "DFIOQUV".contains(b)
        || matches!((a, b), ('B', 'G') | ('G', 'B') | ('K', 'N') | ('N', 'K') | ('N', 'T') | ('T', 'N') | ('Z', 'Z'))
    {
        return None;
    }
    let mut joiner: Option<char> = None;
    let mut k = i + 2;
    // Skips one joiner when there is one; false when it differs from the joiner used before.
    let mut skip = |k: &mut usize| match s.get(*k) {
        Some(&c) if matches!(c, ' ' | '\u{a0}') => {
            *k += 1;
            joiner.is_none_or(|j| j == c) && {
                joiner = Some(c);
                true
            }
        }
        _ => true,
    };
    if !skip(&mut k) {
        return None;
    }
    for n in 0..3 {
        if !(digit(s.get(k)) && digit(s.get(k + 1))) {
            return None;
        }
        k += 2;
        if n < 2 && !skip(&mut k) {
            return None;
        }
    }
    if !skip(&mut k) || !at(k).is_some_and(|c| matches!(c, 'A'..='D')) || word(s.get(k + 1)) {
        return None;
    }
    Some(k + 1 - i)
}

/// Month names the date pattern knows: English, then the languages whose dates are written
/// "5 Ekim 2024" / "5. Oktober 2024" (day, month name, year).
const MONTHS: [[&str; 12]; 5] = [
    ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"],
    ["ocak", "şubat", "mart", "nisan", "mayıs", "haziran", "temmuz", "ağustos", "eylül", "ekim", "kasım", "aralık"],
    ["januar", "februar", "märz", "april", "mai", "juni", "juli", "august", "september", "oktober", "november", "dezember"],
    ["janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre"],
    ["gennaio", "febbraio", "marzo", "aprile", "maggio", "giugno", "luglio", "agosto", "settembre", "ottobre", "novembre", "dicembre"],
];

/// Abbreviations that may end in a full stop: English and Turkish ("Oca.", "Şub", "Ağu").
const MONTH_ABBREVIATIONS: [[&str; 12]; 2] = [
    ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"],
    ["oca", "şub", "mar", "nis", "may", "haz", "tem", "ağu", "eyl", "eki", "kas", "ara"],
];

/// One lowercase character for `c`, with Turkish dotted and dotless i the same as i, so
/// "MAYIS", "Mayıs" and "EKİM" match and every character stays one character.
fn fold(c: char) -> char {
    match c {
        'I' | '\u{130}' | '\u{131}' => 'i',
        _ => c.to_lowercase().next().unwrap_or(c),
    }
}

/// Characters `name` covers at `i`, when it stands there as a whole word (any case).
fn word_at(s: &[char], i: usize, name: &str) -> Option<usize> {
    let mut n = 0;
    for c in name.chars() {
        if s.get(i + n).copied().map(fold) != Some(fold(c)) {
            return None;
        }
        n += 1;
    }
    (!s.get(i + n).is_some_and(|c| c.is_alphabetic())).then_some(n)
}

/// A month name or its abbreviation (optionally with a full stop) at `i`.
fn month(s: &[char], i: usize) -> Option<usize> {
    if word(i.checked_sub(1).and_then(|p| s.get(p))) {
        return None;
    }
    if let Some(n) = MONTHS.iter().flatten().filter_map(|m| word_at(s, i, m)).max() {
        return Some(n);
    }
    let n = MONTH_ABBREVIATIONS.iter().flatten().find_map(|m| word_at(s, i, m))?;
    Some(if s.get(i + n) == Some(&'.') { n + 1 } else { n })
}

fn number(s: &[char], i: usize, min: usize, max: usize) -> Option<usize> {
    let n = s[i.min(s.len())..].iter().take(max + 1).take_while(|c| c.is_ascii_digit()).count();
    (n >= min && n <= max).then_some(n)
}

fn date(s: &[char], i: usize) -> Option<usize> {
    if word(i.checked_sub(1).and_then(|p| s.get(p))) {
        return None;
    }
    let spaces = |k: usize| s[k.min(s.len())..].iter().take_while(|c| **c == ' ').count();
    // Numeric: m/d/yy(yy), m-d-yyyy, d.m.yyyy, yyyy-mm-dd.
    if let Some(a) = number(s, i, 1, 4) {
        let sep = s.get(i + a).copied();
        if matches!(sep, Some('-'..='/'))
            && let Some(b) = number(s, i + a + 1, 1, 2)
            && s.get(i + a + 1 + b).copied() == sep
            && let Some(c) = number(s, i + a + b + 2, if a == 4 { 1 } else { 2 }, if a == 4 { 2 } else { 4 })
            && (a <= 2 || a == 4)
            && !digit(s.get(i + a + b + 2 + c))
        {
            return Some(a + b + c + 2);
        }
        // "5 January 2024", "5 Ekim 2024", "5. Oktober 2024".
        let day = i + a + usize::from(s.get(i + a) == Some(&'.'));
        if a <= 2
            && let k = day + spaces(day)
            && k > day
            && let Some(m) = month(s, k)
        {
            let k2 = k + m + spaces(k + m);
            if let Some(y) = number(s, k2, 4, 4) {
                return Some(k2 + y - i);
            }
        }
        return None;
    }
    // "January 5, 2024" / "Jan. 5 2024".
    let m = month(s, i)?;
    let k = i + m + spaces(i + m);
    let d = number(s, k, 1, 2)?;
    let mut k2 = k + d;
    if s.get(k2) == Some(&',') {
        k2 += 1;
    }
    k2 += spaces(k2);
    let y = number(s, k2, 4, 4)?;
    Some(k2 + y - i)
}

/// An IBAN (ISO 13616): a two-letter country code, two check digits and the rest
/// of the country's exact length in letters and digits, either compact or in the
/// printed groups of four. A space is accepted only after a full group, so an
/// ordinary word after a code ("BA37 leaves at noon") ends the run. Valid iff the
/// number is one modulo 97 after moving the first four characters to the end. The
/// country code must be in the registry, and an alphanumeric character glued to
/// the end rejects the match.
fn iban(s: &[char], i: usize) -> Option<usize> {
    if !(s.get(i).copied().is_some_and(|c| c.is_ascii_alphabetic())
        && s.get(i + 1).copied().is_some_and(|c| c.is_ascii_alphabetic())
        && s.get(i + 2).copied().is_some_and(|c| c.is_ascii_digit())
        && s.get(i + 3).copied().is_some_and(|c| c.is_ascii_digit()))
    {
        return None;
    }
    // Not the tail of a longer alphanumeric run.
    if s.get(i.wrapping_sub(1)).is_some_and(|c| c.is_alphanumeric()) {
        return None;
    }
    let country: String = s.iter().skip(i).take(2).map(char::to_ascii_uppercase).collect();
    let len = iban_length(&country)?;
    // The IBAN's characters uppercased, read up to the country's length.
    let mut digits = Vec::with_capacity(len);
    let mut j = i;
    let mut after_space = false;
    while digits.len() < len {
        match s.get(j) {
            Some(c) if c.is_ascii_alphanumeric() => {
                digits.push(c.to_ascii_uppercase());
                after_space = false;
            }
            // A single space between two groups of four.
            Some(&' ') if digits.len() % 4 == 0 && !after_space => after_space = true,
            _ => return None,
        }
        j += 1;
    }
    // Reject an IBAN glued directly to trailing alphanumerics ("...00ABC").
    // A space-separated suffix ("...00 123") is not part of the IBAN.
    if s.get(j).is_some_and(|c| c.is_alphanumeric()) {
        return None;
    }
    iban_check(&digits).then_some(j - i)
}

/// Whether `digits` is a valid IBAN: move the first four characters to the end,
/// replace letters by two digits (A = 10, ..., Z = 35), and the number must be
/// one modulo 97.
fn iban_check(digits: &[char]) -> bool {
    let mut rem = 0u32;
    for c in digits.iter().skip(4).chain(digits.iter().take(4)).copied() {
        let value = iban_value(c);
        // A letter stands for two decimal digits (A = 10, ..., Z = 35),
        // a digit for one.
        rem = if c.is_ascii_digit() { (rem * 10 + value) % 97 } else { (rem * 100 + value) % 97 };
    }
    rem == 1
}

/// The value of an IBAN character: a digit is itself, a letter is 10 (A) to 35 (Z).
fn iban_value(c: char) -> u32 {
    match c {
        '0'..='9' => c as u32 - '0' as u32,
        _ => c as u32 - 'A' as u32 + 10,
    }
}

/// The exact length of an IBAN for each country code of the SWIFT IBAN registry
/// (release 99, December 2024), counting letters and digits only. A country code
/// that is not listed has no IBANs, so nothing starting with it is matched.
const IBAN_LENGTHS: [(&str, usize); 89] = [
    ("AD", 24),
    ("AE", 23),
    ("AL", 28),
    ("AT", 20),
    ("AZ", 28),
    ("BA", 20),
    ("BE", 16),
    ("BG", 22),
    ("BH", 22),
    ("BI", 27),
    ("BR", 29),
    ("BY", 28),
    ("CH", 21),
    ("CR", 22),
    ("CY", 28),
    ("CZ", 24),
    ("DE", 22),
    ("DJ", 27),
    ("DK", 18),
    ("DO", 28),
    ("EE", 20),
    ("EG", 29),
    ("ES", 24),
    ("FI", 18),
    ("FK", 18),
    ("FO", 18),
    ("FR", 27),
    ("GB", 22),
    ("GE", 22),
    ("GI", 23),
    ("GL", 18),
    ("GR", 27),
    ("GT", 28),
    ("HN", 28),
    ("HR", 21),
    ("HU", 28),
    ("IE", 22),
    ("IL", 23),
    ("IQ", 23),
    ("IS", 26),
    ("IT", 27),
    ("JO", 30),
    ("KW", 30),
    ("KZ", 20),
    ("LB", 28),
    ("LC", 32),
    ("LI", 21),
    ("LT", 20),
    ("LU", 20),
    ("LV", 21),
    ("LY", 25),
    ("MC", 27),
    ("MD", 24),
    ("ME", 22),
    ("MK", 19),
    ("MN", 20),
    ("MR", 27),
    ("MT", 31),
    ("MU", 30),
    ("NI", 28),
    ("NL", 18),
    ("NO", 15),
    ("OM", 23),
    ("PK", 24),
    ("PL", 28),
    ("PS", 29),
    ("PT", 25),
    ("QA", 29),
    ("RO", 24),
    ("RS", 22),
    ("RU", 33),
    ("SA", 24),
    ("SC", 31),
    ("SD", 18),
    ("SE", 24),
    ("SI", 19),
    ("SK", 24),
    ("SM", 27),
    ("SO", 23),
    ("ST", 25),
    ("SV", 28),
    ("TL", 23),
    ("TN", 24),
    ("TR", 26),
    ("UA", 29),
    ("VA", 22),
    ("VG", 24),
    ("XK", 20),
    ("YE", 30),
];

/// The length of an IBAN of `country`, if the registry lists it.
fn iban_length(country: &str) -> Option<usize> {
    IBAN_LENGTHS.iter().find(|(code, _)| *code == country).map(|&(_, len)| len)
}

/// Character ranges of `text` matching `pattern`.
pub fn find(pattern: Pattern, text: &[char]) -> Vec<Range<usize>> {
    let f: fn(&[char], usize) -> Option<usize> = match pattern {
        Pattern::Phone => phone,
        Pattern::Email => email,
        Pattern::CreditCard => credit_card,
        Pattern::Ssn => ssn,
        Pattern::Date => date,
        Pattern::Iban => iban,
        Pattern::Sin => sin,
        Pattern::CaPostalCode => ca_postal_code,
        Pattern::Nino => nino,
        Pattern::Tfn => tfn,
        Pattern::Itin => itin,
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        match f(text, i) {
            Some(n) if n > 0 => {
                out.push(i..i + n);
                i += n;
            }
            _ => i += 1,
        }
    }
    out
}

/// The most entries a word list keeps (Find Text ▸ Multiple words or phrases).
pub const MAX_WORDS: usize = 1000;
/// The most characters an entry keeps.
pub const MAX_WORD_CHARS: usize = 256;

/// A word list from text with one word or phrase per line: trimmed, without empty lines or
/// repeats, in order, at most [`MAX_WORDS`] entries of at most [`MAX_WORD_CHARS`] characters.
pub fn word_list(text: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    text.lines()
        .map(|l| l.trim().chars().take(MAX_WORD_CHARS).collect::<String>())
        .filter(|w| !w.is_empty() && seen.insert(w.clone()))
        .take(MAX_WORDS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_lists_are_trimmed_unique_and_bounded() {
        assert_eq!(word_list("  John Smith \n\n\tACME\r\nJohn Smith\n  \nő ű"), ["John Smith", "ACME", "ő ű"]);
        let long = "é".repeat(MAX_WORD_CHARS + 10);
        assert_eq!(word_list(&long)[0].chars().count(), MAX_WORD_CHARS);
        let many: String = (0..MAX_WORDS + 50).map(|i| format!("w{i}\n")).collect();
        let list = word_list(&many);
        assert_eq!((list.len(), list.last().map(String::as_str)), (MAX_WORDS, Some("w999")));
        assert!(word_list("").is_empty());
    }

    fn found(p: Pattern, s: &str) -> Vec<String> {
        let c: Vec<char> = s.chars().collect();
        find(p, &c).into_iter().map(|r| c[r].iter().collect()).collect()
    }

    #[test]
    fn email_local_part_may_contain_an_apostrophe() {
        assert_eq!(found(Pattern::Email, "Mail o'brien@example.com today"), ["o'brien@example.com"]);
        assert_eq!(found(Pattern::Email, "Mail 'john@example.com' today"), ["john@example.com"]);
        // Only the leading quote stays outside; the whole local part is matched.
        assert_eq!(found(Pattern::Email, "Mail 'john'@example.com today"), ["john'@example.com"]);
    }

    #[test]
    fn phone_numbers() {
        assert_eq!(
            found(Pattern::Phone, "Call (555) 123-4567 or 555.987.6543, +1 555 222 3333; not 12345 or 5551234567890"),
            ["(555) 123-4567", "555.987.6543", "+1 555 222 3333"]
        );
        // #525: international numbers match whole; "+49" used to stay visible.
        assert_eq!(found(Pattern::Phone, "Tel. +49 2151 123456."), ["+49 2151 123456"]);
        assert_eq!(
            found(Pattern::Phone, "+44 20 7946 0018, +33 1 23 45 67 89, +49 (0) 2151 123456, (0211) 123456, 030 12345678, 01 23 45 67 89"),
            ["+44 20 7946 0018", "+33 1 23 45 67 89", "+49 (0) 2151 123456", "(0211) 123456", "030 12345678", "01 23 45 67 89"]
        );
        // Not phone numbers: day-first dates, decimals, unseparated, too short or too long.
        assert_eq!(
            found(
                Pattern::Phone,
                "on 01.10.2026 or 09-10-2026, ratio 0.1234567, id 0211123456, +4921511234567, +1 234 567, 0211 12, +49 1234 5678 9012 3456"
            ),
            Vec::<String>::new()
        );
    }

    /// #3205: Italian numbers written with a slash, and, after a phone label, without separators.
    #[test]
    fn slashed_and_labelled_unseparated_phone_numbers() {
        let lines = [
            "A tel. 0426 123456",
            "B tel. +39 0426 123456",
            "C cell. 345 1234567",
            "D cell. 3451234567",
            "E tel. 0426/123456",
            "F cell. 345.123.4567",
            "G tel. 0426.123456",
            "H cell. +393451234567",
            "Fax: 0211123456",
            "Telefono 0426/123456",
        ];
        assert_eq!(
            found(Pattern::Phone, &lines.join("\n")),
            [
                "0426 123456",
                "+39 0426 123456",
                "345 1234567",
                "3451234567",
                "0426/123456",
                "345.123.4567",
                "0426.123456",
                "+393451234567",
                "0211123456",
                "0426/123456"
            ]
        );
        // Without a label, or after a word that only ends like one, unseparated numbers stay
        // unmarked; a slash needs a trunk 0 and digits on both sides, and dates stay dates.
        assert_eq!(
            found(
                Pattern::Phone,
                "id 0211123456, +4921511234567, hotel 3451234567, prot. 2024/123456, on 01/10/2026, \
                 tel. 12345, tel 0426/ 123456, +39 0426/123456, tel 0426/12/3456, tel 0426 12/3456"
            ),
            Vec::<String>::new()
        );
    }

    #[test]
    /// Labelled Italian geographic numbers match completely while dates and other digit runs stay unmarked.
    fn labelled_italian_international_slash_numbers_match_whole() {
        for (text, number) in [
            ("tel. +39 0426/123456", "+39 0426/123456"),
            ("Fax: +390426/123456", "+390426/123456"),
            ("Telefono +39 02/12345678", "+39 02/12345678"),
            ("tel. +39 02/1234", "+39 02/1234"),
            ("tel. +39 0426/1234567", "+39 0426/1234567"),
        ] {
            assert_eq!(found(Pattern::Phone, text), [number], "{text}");
        }
        for text in [
            "+39 0426/123456",
            "tel. +39 2024/123456",
            "tel. +39 0/123456789",
            "tel. +39 0426/12/3456",
            "tel. +39 0426 12/3456",
            "tel. +39 0426/ 123456",
            "tel. +39 02/123",
            "tel. +39 0426/12345678",
            "tel. +39 0426/1234567890",
            "tel. +39  0426/123456",
            "tel. +39-0426/123456",
            "tel. +39 -0426/123456",
            "tel. +40 0426/123456",
            "protocol 2024/123456",
            "on 01/10/2026",
        ] {
            assert!(found(Pattern::Phone, text).is_empty(), "{text}");
        }
    }

    #[test]
    fn the_tail_of_a_longer_number_is_not_a_phone_number() {
        // #1884: the last groups of a French social security number, an IBAN and a SIRET used
        // to be marked as phone numbers (and the start left visible). Example numbers.
        for s in ["NIR 1 85 05 78 006 084 36 end", "IBAN FR76 3000 6000 0112 3456 7890 189", "SIRET 123 456 782 00010"] {
            assert_eq!(found(Pattern::Phone, s), Vec::<String>::new(), "{s}");
        }
        assert_eq!(found(Pattern::Phone, "tel 01 23 45 67 89 / +33 1 23 45 67 89"), ["01 23 45 67 89", "+33 1 23 45 67 89"]);
        // Nor the head of a run too long to be one: 28 characters in, this one still goes on.
        assert_eq!(found(Pattern::Phone, "01 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7"), Vec::<String>::new());
    }

    #[test]
    fn emails() {
        assert_eq!(
            found(Pattern::Email, "Write to ada.lovelace+pdf@example.co.uk, or bob@host (no TLD) or x@y.z."),
            ["ada.lovelace+pdf@example.co.uk"]
        );
    }

    #[test]
    fn emails_with_a_leading_punctuation_character() {
        for lead in ["_", "-", ".", "%", "+"] {
            let address = format!("{lead}john@example.com");
            assert_eq!(found(Pattern::Email, &format!("Write to {address} now")), [address.as_str()], "lead {lead:?}");
        }
        // A run of local characters is still matched whole, never from its middle.
        assert_eq!(found(Pattern::Email, "a.b@x.com, foo_john@example.com"), ["a.b@x.com", "foo_john@example.com"]);
        // Punctuation alone is no local part.
        assert_eq!(found(Pattern::Email, "Write to ..@example.com or -@example.com"), Vec::<String>::new());
    }

    #[test]
    fn internationalized_emails() {
        assert_eq!(
            found(Pattern::Email, "Yazın: ayşe.yılmaz@örnek.com.tr, ÇAĞLAR@ŞİRKET.COM.TR; müller@bücher.de, иван@пример.рф, josé@correo.es."),
            ["ayşe.yılmaz@örnek.com.tr", "ÇAĞLAR@ŞİRKET.COM.TR", "müller@bücher.de", "иван@пример.рф", "josé@correo.es"]
        );
        // Text in scripts without spaces stays out of the address around it.
        assert_eq!(found(Pattern::Email, "メールはtaro@example.jpまで、联系support@example.cn谢谢"), ["taro@example.jp", "support@example.cn"]);
        // Still no address without a dot in the domain or with a one-letter or numeric TLD.
        assert_eq!(found(Pattern::Email, "ayşe@örnek, ş@ğ.ü, kişi@alan.123, a@b.c"), Vec::<String>::new());
    }

    #[test]
    fn credit_cards_need_a_valid_checksum() {
        assert_eq!(
            found(Pattern::CreditCard, "Visa 4111 1111 1111 1111, bad 4111 1111 1111 1112, amex 3782-822463-10005"),
            ["4111 1111 1111 1111", "3782-822463-10005"]
        );
    }

    #[test]
    fn credit_card_followed_by_another_number() {
        for tail in ["12/26", "2026", "123", "12 26"] {
            assert_eq!(found(Pattern::CreditCard, &format!("Card 4111 1111 1111 1111 {tail}")), ["4111 1111 1111 1111"], "tail {tail}");
        }
        assert_eq!(found(Pattern::CreditCard, "Card 3782-822463-10005 123"), ["3782-822463-10005"]);
        // A run that is not a card at any group boundary still matches nothing.
        assert_eq!(found(Pattern::CreditCard, "Card 4111 1111 1111 1112 12/26"), Vec::<String>::new());
        // A longer grouped number doesn't match on a card-shaped prefix.
        assert_eq!(found(Pattern::CreditCard, "IBAN 4111 1111 1111 1111 0000 0000"), Vec::<String>::new());
    }

    #[test]
    fn social_security_numbers() {
        assert_eq!(
            found(Pattern::Ssn, "SSN 123-45-6789 and 123 45 6789; invalid 000-12-3456, 666-12-3456, 912-12-3456, 1234-56-7890"),
            ["123-45-6789", "123 45 6789"]
        );
    }

    #[test]
    fn canadian_social_insurance_numbers() {
        // 130 692 544 is Luhn-valid.
        assert_eq!(
            found(Pattern::Sin, "SIN 130-692-544 and 130 692 544 and SIN: 130692544; bad 130-692-545, 130-692 544"),
            ["130-692-544", "130 692 544", "130692544"].map(String::from)
        );
        // Other joiners: no-break space, non-breaking hyphen, en dash, full stop.
        assert_eq!(
            found(Pattern::Sin, "130\u{a0}692\u{a0}544 130\u{2011}692\u{2011}544 130\u{2013}692\u{2013}544 130.692.544"),
            ["130\u{a0}692\u{a0}544", "130\u{2011}692\u{2011}544", "130\u{2013}692\u{2013}544", "130.692.544"].map(String::from)
        );
        // Labels in other forms and cases.
        for label in [
            "SIN.",
            "SIN No.",
            "sin number:",
            "S.I.N.",
            "Social Insurance Number:",
            "social insurance no.",
            "NAS",
            "nas :",
            "Numéro d'assurance sociale",
            "NUMÉRO D\u{2019}ASSURANCE SOCIALE :",
            "SIN#",
            "SIN\u{a0}",
        ] {
            let text = format!("{label} 130692544");
            assert_eq!(found(Pattern::Sin, &text), ["130692544"], "{text}");
        }
        // Unlabelled nine digits, a label glued to a word, a longer run (labelled or not), a first
        // digit of 8, mixed joiners and a leading 0 (046 454 286 is Luhn-valid but never issued).
        for text in [
            "order 130692544",
            "basin 130692544",
            "SIN 1306925441",
            "1306925441",
            "1130-692-544",
            "130-692-5441",
            "800-000-002",
            "SIN 046 454 286",
            "SIN 046454286",
            "130-692 544",
            "130.692-544",
        ] {
            assert!(found(Pattern::Sin, text).is_empty(), "{text}");
        }
    }

    #[test]
    fn uk_national_insurance_numbers() {
        assert_eq!(
            found(Pattern::Nino, "NI AB123456C, AB 12 34 56 D, ab\u{a0}12\u{a0}34\u{a0}56\u{a0}a, AB 123456 C, AB123456 C and jc654321b."),
            ["AB123456C", "AB 12 34 56 D", "ab\u{a0}12\u{a0}34\u{a0}56\u{a0}a", "AB 123456 C", "AB123456 C", "jc654321b"].map(String::from)
        );
        // Every unallocated prefix, and every excluded first or second letter with a valid partner.
        for prefix in ["BG", "GB", "KN", "NK", "NT", "TN", "ZZ"] {
            assert!(found(Pattern::Nino, &format!("{prefix}123456A")).is_empty(), "{prefix}");
        }
        for first in "DFIQUV".chars() {
            assert!(found(Pattern::Nino, &format!("{first}A123456A")).is_empty(), "first {first}");
        }
        for second in "DFIOQUV".chars() {
            assert!(found(Pattern::Nino, &format!("A{second}123456A")).is_empty(), "second {second}");
        }
        assert_eq!(found(Pattern::Nino, "AB123456A"), ["AB123456A"]);
        // A suffix past D or missing, mixed joiners, doubled joiners, wrong digit counts and glued text.
        for text in [
            "AB123456E",
            "AB123456",
            "AB 12 34 56",
            "AB 12  34 56 A",
            "AB 12\u{a0}34 56 A",
            "AB 12 34 56\u{a0}A",
            "AB 1 23456 A",
            "AB1234567A",
            "AB12345A",
            "XAB123456A",
            "AB123456AX",
            "AB123456A1",
            "1AB123456A",
        ] {
            assert!(found(Pattern::Nino, text).is_empty(), "{text}");
        }
        // The end of the text mid-number.
        assert!(found(Pattern::Nino, "AB 12 34 5").is_empty());
        assert!(found(Pattern::Nino, "A").is_empty());
    }

    #[test]
    fn australian_tax_file_numbers() {
        // 123 456 782 and 865 432 102 satisfy the weighted checksum.
        assert_eq!(
            found(Pattern::Tfn, "TFN 123 456 782, 123-456-782, 123\u{a0}456\u{a0}782, TFN: 123456782 and tax file number 865432102."),
            ["123 456 782", "123-456-782", "123\u{a0}456\u{a0}782", "123456782", "865432102"].map(String::from)
        );
        // Every label form, glued to the digits after punctuation.
        for label in ["T.F.N.", "TFN No.", "TFN number", "tfn:", "Tax File Number"] {
            let text = format!("{label} 123456782");
            assert_eq!(found(Pattern::Tfn, &text), ["123456782"], "{text}");
        }
        for text in [
            "order 123456782",
            "notfn 123456782",
            "TFN 1234567821",
            "1234567821",
            "1123 456 782",
            "123 456 7821",
            "123 456 783",
            "123-456 782",
            "123.456.782",
            "123 456 78",
            "123456782",
        ] {
            assert!(found(Pattern::Tfn, text).is_empty(), "{text}");
        }
        assert!(found(Pattern::Tfn, "123 456 7").is_empty());
    }

    #[test]
    fn us_individual_taxpayer_identification_numbers() {
        assert_eq!(
            found(Pattern::Itin, "ITIN 912-70-1234, 900 50 0001, 999\u{a0}99\u{a0}9999, ITIN: 912701234 and itin no. 977651234."),
            ["912-70-1234", "900 50 0001", "999\u{a0}99\u{a0}9999", "912701234", "977651234"].map(String::from)
        );
        for label in ["I.T.I.N.", "ITIN No.", "ITIN number", "itin:", "ITIN no"] {
            let text = format!("{label} 912701234");
            assert_eq!(found(Pattern::Itin, &text), ["912701234"], "{text}");
        }
        // The fourth and fifth digits at both ends of every allowed range.
        for group in ["50", "65", "70", "88", "90", "92", "94", "99"] {
            let text = format!("900-{group}-1234");
            assert_eq!(found(Pattern::Itin, &text), std::slice::from_ref(&text), "{text}");
        }
        for text in [
            // Outside the ranges: 49, 66, 69, 89, 93 and 00.
            "912-49-1234",
            "912-66-1234",
            "912-69-1234",
            "912-89-1234",
            "912-93-1234",
            "912-00-1234",
            // Not a 9 first: a Social Security number's shape.
            "812-70-1234",
            "123-45-6789",
            // Digits touching, wrong grouping, mixed or other separators.
            "9912-70-1234",
            "912-70-12345",
            "912-70 1234",
            "912.70.1234",
            "912-7-01234",
            "912-70-123",
            // Plain digits without a label, or after other labels.
            "912701234",
            "order 912701234",
            "SSN 912701234",
            "noitin 912701234",
            "ITIN 9127012345",
        ] {
            assert!(found(Pattern::Itin, text).is_empty(), "{text}");
        }
        assert!(found(Pattern::Itin, "912-70").is_empty());
        assert!(found(Pattern::Itin, "9").is_empty());
    }

    #[test]
    fn canadian_postal_codes() {
        assert_eq!(
            found(
                Pattern::CaPostalCode,
                "K1A 0B1, K1A\u{a0}0B1, m5v-3l9 and V6B2W9. Bad: D1A 0B1, K1A 0U1, K1A0B, AK1A 0B1, K1A 0B1X, 12345, W1A 0B1, Z1A 0B1"
            ),
            ["K1A 0B1", "K1A\u{a0}0B1", "m5v-3l9", "V6B2W9"].map(String::from)
        );
    }

    #[test]
    fn dates() {
        assert_eq!(
            found(Pattern::Date, "Due 10/01/2026, 2026-10-01, 1.10.26, March 5, 2024, Jan. 7 2025 and 5 June 2023; not 10/2026 or Mayday 12"),
            ["10/01/2026", "2026-10-01", "1.10.26", "March 5, 2024", "Jan. 7 2025", "5 June 2023"]
        );
    }

    #[test]
    fn dates_with_turkish_month_names_in_any_case() {
        assert_eq!(
            found(Pattern::Date, "Tarih: 10 Ekim 2026, 3 Mayıs 2025, 5 Şubat 2024, 5 ŞUBAT 2024, 3 MAYIS 2025, 1 EKİM 2026 ve 9 kasım 2023."),
            ["10 Ekim 2026", "3 Mayıs 2025", "5 Şubat 2024", "5 ŞUBAT 2024", "3 MAYIS 2025", "1 EKİM 2026", "9 kasım 2023"]
        );
        assert_eq!(
            found(Pattern::Date, "12 Oca 2024, 7 Şub. 2025, 30 Ağu 2023, 1 ARA. 2022, 2 Haz 2021"),
            ["12 Oca 2024", "7 Şub. 2025", "30 Ağu 2023", "1 ARA. 2022", "2 Haz 2021"]
        );
        // Words that only start like a month, or a month without a day and year, stay.
        assert_eq!(
            found(Pattern::Date, "5 Martı 2024, Ekimde 2026, 3 Mayısta 2025, ara 2024, Kasım ayında, 12 Aralık, Aralık 2024"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn dates_with_german_french_and_italian_month_names() {
        assert_eq!(
            found(Pattern::Date, "am 10. Oktober 2026, 3 März 2025, le 14 juillet 2024, 1 août 2023, il 25 dicembre 2022"),
            ["10. Oktober 2026", "3 März 2025", "14 juillet 2024", "1 août 2023", "25 dicembre 2022"]
        );
        // "Mai" alone, a dotted day without a month and an unknown word are not dates.
        assert_eq!(found(Pattern::Date, "Mai 2024, Kapitel 5. Absatz 2024, 5 Oktoberfest 2024"), Vec::<String>::new());
    }

    #[test]
    fn case_folding_keeps_one_character_per_character() {
        let s: Vec<char> = "\u{130}\u{131}I\u{15E}".chars().collect();
        assert_eq!(s.iter().copied().map(fold).collect::<String>(), "iii\u{15F}");
    }

    #[test]
    fn iban_matches_compact_spaced_and_rejects_near_matches() {
        assert_eq!(Pattern::from_id("iban").map(Pattern::id), Some("iban"));

        // Spaced and compact IBANs match, a wrong check digit is rejected ("DE89
        // 3704 0044 0532 0130 00" and "DE89370400440532013000" match, "...01"
        // does not).
        if let Some(iban_pattern) = Pattern::from_id("iban") {
            // Spaced IBAN (valid German IBAN: DE89 3704 0044 0532 0130 00)
            assert_eq!(found(iban_pattern, "DE89 3704 0044 0532 0130 00"), vec!["DE89 3704 0044 0532 0130 00"]);
            // Compact IBAN (same number, no spaces)
            assert_eq!(found(iban_pattern, "DE89370400440532013000"), vec!["DE89370400440532013000"]);
            // Wrong check digit should not match (DE89...01 instead of DE89...00)
            assert_eq!(found(iban_pattern, "DE89 3704 0044 0532 0130 01"), Vec::<String>::new());
            // Near match: IBAN followed by extra digits should match only the IBAN
            assert_eq!(found(iban_pattern, "DE89 3704 0044 0532 0130 00 123"), vec!["DE89 3704 0044 0532 0130 00"]);
        }
    }

    #[test]
    fn iban_rejects_unknown_country_code_wrong_length_and_glued_tail() {
        if let Some(iban_pattern) = Pattern::from_id("iban") {
            // Unknown country code: the check digits pass mod-97, but ISO 13616 defines no length for XX.
            assert_eq!(found(iban_pattern, "XX65000000000000000000"), Vec::<String>::new());
            // DE is 22 characters long. This 15-character number passes mod-97 but has
            // the wrong national length, so it must be rejected.
            assert_eq!(found(iban_pattern, "DE8300000000000"), Vec::<String>::new());
            // A valid IBAN glued directly to trailing alphanumerics is not an IBAN.
            assert_eq!(found(iban_pattern, "DE89370400440532013000ABC"), Vec::<String>::new());
            assert_eq!(found(iban_pattern, "ABCDE89370400440532013000"), Vec::<String>::new());
            // Letters in place of the two check digits.
            assert_eq!(found(iban_pattern, "DEAB 3704 0044 0532 0130 00"), Vec::<String>::new());
            // A space-separated suffix does not glue to the IBAN.
            assert_eq!(found(iban_pattern, "DE89 3704 0044 0532 0130 00 123"), vec!["DE89 3704 0044 0532 0130 00"]);
        }
    }

    #[test]
    fn iban_accepts_spaces_only_between_groups_of_four() {
        // A code followed by ordinary words: the words end the run, so nothing is marked.
        for s in [
            "Flight BA37 leaves at noon from gate four",
            "Flight BA30 leaves at noon",
            "Gate BA45 leaves at noon",
            "BA72 have water are word",
            "GB00 their an her make that",
            "SI30 write can all more",
        ] {
            assert_eq!(found(Pattern::Iban, s), Vec::<String>::new(), "{s}");
        }
        // A valid IBAN with a space in the wrong place, or two spaces, is not marked.
        for s in ["DE89 3704 0044 0532 013 000", "DE89 3704 0044 0532 0 13000", "DE8 9370 4004 4053 2013 000", "DE89  3704 0044 0532 0130 00"] {
            assert_eq!(found(Pattern::Iban, s), Vec::<String>::new(), "{s}");
        }
        // Any group boundary may be a space or not, and the IBAN inside a sentence is marked alone.
        assert_eq!(found(Pattern::Iban, "DE893704 0044 0532013000"), vec!["DE893704 0044 0532013000"]);
        assert_eq!(found(Pattern::Iban, "Pay to DE89 3704 0044 0532 0130 00 today"), vec!["DE89 3704 0044 0532 0130 00"]);
    }

    /// One example IBAN per country of the SWIFT IBAN registry (release 99).
    const IBAN_REGISTRY_EXAMPLES: [&str; IBAN_LENGTHS.len()] = [
        "AD1200012030200359100100",
        "AE070331234567890123456",
        "AL47212110090000000235698741",
        "AT611904300234573201",
        "AZ21NABZ00000000137010001944",
        "BA391290079401028494",
        "BE68539007547034",
        "BG80BNBG96611020345678",
        "BH67BMAG00001299123456",
        "BI4210000100010000332045181",
        "BR1800360305000010009795493C1",
        "BY13NBRB3600900000002Z00AB00",
        "CH9300762011623852957",
        "CR05015202001026284066",
        "CY17002001280000001200527600",
        "CZ6508000000192000145399",
        "DE89370400440532013000",
        "DJ2100010000000154000100186",
        "DK5000400440116243",
        "DO28BAGR00000001212453611324",
        "EE382200221020145685",
        "EG380019000500000000263180002",
        "ES9121000418450200051332",
        "FI2112345600000785",
        "FK88SC123456789012",
        "FO6264600001631634",
        "FR1420041010050500013M02606",
        "GB29NWBK60161331926819",
        "GE29NB0000000101904917",
        "GI75NWBK000000007099453",
        "GL8964710001000206",
        "GR1601101250000000012300695",
        "GT82TRAJ01020000001210029690",
        "HN88CABF00000000000250005469",
        "HR1210010051863000160",
        "HU42117730161111101800000000",
        "IE29AIBK93115212345678",
        "IL620108000000099999999",
        "IQ98NBIQ850123456789012",
        "IS140159260076545510730339",
        "IT60X0542811101000000123456",
        "JO94CBJO0010000000000131000302",
        "KW81CBKU0000000000001234560101",
        "KZ86125KZT5004100100",
        "LB62099900000001001901229114",
        "LC55HEMM000100010012001200023015",
        "LI21088100002324013AA",
        "LT121000011101001000",
        "LU280019400644750000",
        "LV80BANK0000435195001",
        "LY83002048000020100120361",
        "MC5811222000010123456789030",
        "MD24AG000225100013104168",
        "ME25505000012345678951",
        "MK07250120000058984",
        "MN121234123456789123",
        "MR1300020001010000123456753",
        "MT84MALT011000012345MTLCAST001S",
        "MU17BOMM0101101030300200000MUR",
        "NI45BAPR00000013000003558124",
        "NL91ABNA0417164300",
        "NO9386011117947",
        "OM810180000001299123456",
        "PK36SCBL0000001123456702",
        "PL61109010140000071219812874",
        "PS92PALS000000000400123456702",
        "PT50000201231234567890154",
        "QA58DOHB00001234567890ABCDEFG",
        "RO49AAAA1B31007593840000",
        "RS35260005601001611379",
        "RU0304452522540817810538091310419",
        "SA0380000000608010167519",
        "SC18SSCB11010000000000001497USD",
        "SD2129010501234001",
        "SE4550000000058398257466",
        "SI56263300012039086",
        "SK3112000000198742637541",
        "SM86U0322509800000000270100",
        "SO211000001001000100141",
        "ST68000100010051845310112",
        "SV62CENR00000000000000700025",
        "TL380080012345678910157",
        "TN5910006035183598478831",
        "TR330006100519786457841326",
        "UA213223130000026007233566001",
        "VA59001123000012345678",
        "VG96VPVG0000012345678901",
        "XK051212012345678906",
        "YE15CBYE0001018861234567891234",
    ];

    /// `iban` in the printed form: groups of four separated by single spaces.
    fn printed(iban: &str) -> String {
        iban.chars().collect::<Vec<_>>().chunks(4).map(|group| group.iter().collect::<String>()).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn iban_registry_examples_cover_every_country_and_length() {
        let from_examples: Vec<(&str, usize)> = IBAN_REGISTRY_EXAMPLES.iter().map(|e| (&e[..2], e.len())).collect();
        assert_eq!(from_examples.as_slice(), IBAN_LENGTHS.as_slice());
    }

    #[test]
    fn iban_matches_the_registry_example_of_every_country() {
        for example in IBAN_REGISTRY_EXAMPLES {
            assert_eq!(found(Pattern::Iban, example), vec![example], "compact {example}");
            let spaced = printed(example);
            assert_eq!(found(Pattern::Iban, &spaced), vec![spaced.clone()], "printed {spaced}");
        }
    }

    #[test]
    fn iban_rejects_a_registry_example_with_one_digit_changed() {
        for example in IBAN_REGISTRY_EXAMPLES {
            // The last digit becomes the next digit, so the country, the length and the layout stay the same.
            let Some(at) = example.rfind(|c: char| c.is_ascii_digit()) else { continue };
            let digit = example[at..=at].parse::<u32>().unwrap_or(0);
            let changed = format!("{}{}{}", &example[..at], (digit + 1) % 10, &example[at + 1..]);
            assert_eq!(found(Pattern::Iban, &changed), Vec::<String>::new(), "{changed}");
        }
    }
}
