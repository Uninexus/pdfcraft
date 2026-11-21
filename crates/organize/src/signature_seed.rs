//! The seed values of a signature field: its `/SV` dictionary (ISO 32000-2, signature fields), which tells
//! the one who signs what the field's author wants the signature to be: the reasons to pick from
//! (`/Reasons`, text strings), the signature formats allowed (`/SubFilter`, names), the digest
//! algorithms allowed (`/DigestMethod`, names) and whether revocation information must be
//! embedded (`/AddRevInfo`, a boolean). `/Ff` is a set of flags, one bit for each entry (bit 2
//! for `/SubFilter`, bit 4 for `/Reasons`, bit 6 for `/AddRevInfo`, bit 7 for `/DigestMethod`,
//! counted from 1): a set bit makes the entry required, a clear bit makes it a suggestion.
//!
//! This module reads the seed values of one signature field, sets four of them (the field's
//! dictionary gets a new direct `/SV`) or takes `/SV` out. **It does not apply them**: nothing
//! here reads a field's seed values when a signature is made (`crates/sign` does not look at
//! `/SV`), and nothing checks a signature against them. Not modelled, and never written:
//! `/Filter`, `/V`, `/Cert`, `/MDP`, `/TimeStamp`, `/LegalAttestation`, `/LockDocument`,
//! `/AppearanceFilter` and the flag bits of those. A read counts the keys of `/SV` that are not
//! `/Type`, `/Ff` or one of the four (`other_entries`), and a set is refused while there is one,
//! because a set replaces the dictionary and would lose it; a removal is not refused for it.
//!
//! The field is given as an object, which the caller found by name among the form's signature
//! fields; the object must not be the catalog, the `/Pages` node, a node under it or a page, and
//! must not hold a signature (`/V` is not null): seed values are read before a signature is made,
//! so ones edited afterwards would say something the signature does not.
//!
//! A read has one allowance of [`MAX_GETS`] reads, against which every read it makes is charged
//! before it is made: the field's own object, every hop of a reference chain (as
//! `Document::resolve` follows them, the first object and up to 32 more), and each entry of the
//! three lists looked at (one more, before the entry is looked at). When it is spent the read
//! stops and says so. Separate from that, [`structural`](crate::reference_xobjects) walks the
//! page tree with its own allowance of reads to find the nodes the field must not be, and its
//! first read, of the catalog, is not charged; a write walks it once and reads the field again.
//! Counting the keys of `/SV` makes no read.

use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString};

use crate::OrganizeError;
use crate::reference_xobjects::{Gets, Spent, structural};
use crate::role_map::shown;

/// Most entries of each list looked at and listed (all are counted); also the most a write
/// accepts for each.
pub const MAX_ENTRIES: usize = 16;
/// Most characters of a reason listed or written; a longer one in a file is cut and not `exact`.
pub const MAX_TEXT_CHARS: usize = 127;
/// Most reads one call of [`read_signature_seed`] makes: its allowance, shared by everything it
/// reads (the page-tree walk has its own).
pub const MAX_GETS: usize = 10_000;

/// The `/SubFilter` names a write accepts.
pub const SUB_FILTERS: [&str; 4] = ["adbe.pkcs7.detached", "adbe.pkcs7.sha1", "adbe.x509.rsa_sha1", "ETSI.CAdES.detached"];
/// The `/DigestMethod` names a write accepts.
pub const DIGEST_METHODS: [&str; 3] = ["SHA256", "SHA384", "SHA512"];

/// An entry a flag can make required.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    SubFilter,
    Reasons,
    AddRevInfo,
    DigestMethod,
}

/// The entries a flag can be set for, in the order a read lists them.
pub const ENTRIES: [Entry; 4] = [Entry::SubFilter, Entry::Reasons, Entry::AddRevInfo, Entry::DigestMethod];

impl Entry {
    /// The entry's value in `/Ff`.
    pub fn bit(self) -> i64 {
        match self {
            Entry::SubFilter => 1 << 1,
            Entry::Reasons => 1 << 3,
            Entry::AddRevInfo => 1 << 5,
            Entry::DigestMethod => 1 << 6,
        }
    }

    /// The name a read and a write use.
    pub fn name(self) -> &'static str {
        match self {
            Entry::SubFilter => "sub_filter",
            Entry::Reasons => "reasons",
            Entry::AddRevInfo => "add_rev_info",
            Entry::DigestMethod => "digest_method",
        }
    }

    pub fn parse(name: &str) -> Option<Entry> {
        ENTRIES.into_iter().find(|e| e.name() == name)
    }
}

/// Seed values to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeedSpec {
    reasons: Vec<String>,
    sub_filters: Vec<String>,
    digest_methods: Vec<String>,
    add_rev_info: Option<bool>,
    required: Vec<Entry>,
}

impl SeedSpec {
    /// Seed values, checked: at least one entry is given; at most [`MAX_ENTRIES`] of each list;
    /// a reason is 1 to [`MAX_TEXT_CHARS`] characters; a sub-filter is one of [`SUB_FILTERS`] and a
    /// digest method one of [`DIGEST_METHODS`] (compared byte for byte); none is listed twice; and
    /// every entry in `required` is given (and is listed once).
    pub fn new(
        reasons: Vec<String>,
        sub_filters: Vec<String>,
        digest_methods: Vec<String>,
        add_rev_info: Option<bool>,
        required: Vec<Entry>,
    ) -> Result<Self, OrganizeError> {
        if reasons.is_empty() && sub_filters.is_empty() && digest_methods.is_empty() && add_rev_info.is_none() {
            return Err(OrganizeError::Invalid("seed values with no entry; remove them instead".into()));
        }
        for (what, len) in [("reasons", reasons.len()), ("sub-filters", sub_filters.len()), ("digest methods", digest_methods.len())] {
            if len > MAX_ENTRIES {
                return Err(OrganizeError::Invalid(format!("at most {MAX_ENTRIES} {what}")));
            }
        }
        for (i, r) in reasons.iter().enumerate() {
            if r.is_empty() || r.chars().count() > MAX_TEXT_CHARS {
                return Err(OrganizeError::Invalid(format!("a reason is 1 to {MAX_TEXT_CHARS} characters")));
            }
            if reasons[..i].contains(r) {
                return Err(OrganizeError::Invalid(format!("the reason {r:?} is listed twice")));
            }
        }
        for (what, list, allowed) in [("sub-filter", &sub_filters, &SUB_FILTERS[..]), ("digest method", &digest_methods, &DIGEST_METHODS[..])] {
            for (i, n) in list.iter().enumerate() {
                if !allowed.iter().any(|a| a.as_bytes() == n.as_bytes()) {
                    return Err(OrganizeError::Invalid(format!("unknown {what} {n:?} (one of {})", allowed.join(", "))));
                }
                if list[..i].contains(n) {
                    return Err(OrganizeError::Invalid(format!("the {what} {n:?} is listed twice")));
                }
            }
        }
        // Revocation information can only be embedded by the PKCS#7 formats; a list of formats
        // that holds none of those two is refused with it (the CAdES name is not assumed to).
        if add_rev_info == Some(true) && !sub_filters.is_empty() && !sub_filters.iter().any(|n| n == "adbe.pkcs7.detached" || n == "adbe.pkcs7.sha1")
        {
            return Err(OrganizeError::Invalid(
                "embedding revocation information needs adbe.pkcs7.detached or adbe.pkcs7.sha1 among the sub-filters".into(),
            ));
        }
        for (i, e) in required.iter().enumerate() {
            if required[..i].contains(e) {
                return Err(OrganizeError::Invalid(format!("{} is required twice", e.name())));
            }
            let given = match e {
                Entry::SubFilter => !sub_filters.is_empty(),
                Entry::Reasons => !reasons.is_empty(),
                Entry::AddRevInfo => add_rev_info.is_some(),
                Entry::DigestMethod => !digest_methods.is_empty(),
            };
            if !given {
                return Err(OrganizeError::Invalid(format!("{} cannot be required without being given", e.name())));
            }
        }
        Ok(SeedSpec { reasons, sub_filters, digest_methods, add_rev_info, required })
    }

    fn to_dict(&self) -> Dict {
        let mut d = Dict::new();
        d.set(b"Type".to_vec(), Object::name("SV"));
        let flags = self.required.iter().fold(0, |f, e| f | e.bit());
        if flags != 0 {
            d.set(b"Ff".to_vec(), Object::Int(flags));
        }
        if !self.sub_filters.is_empty() {
            d.set(b"SubFilter".to_vec(), Object::Array(self.sub_filters.iter().map(|n| Object::name(n)).collect()));
        }
        if !self.reasons.is_empty() {
            d.set(b"Reasons".to_vec(), Object::Array(self.reasons.iter().map(|r| Object::String(PdfString::text(r))).collect()));
        }
        if !self.digest_methods.is_empty() {
            d.set(b"DigestMethod".to_vec(), Object::Array(self.digest_methods.iter().map(|n| Object::name(n)).collect()));
        }
        if let Some(b) = self.add_rev_info {
            d.set(b"AddRevInfo".to_vec(), Object::Bool(b));
        }
        d
    }
}

/// One list of `/SV` as read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeedList {
    /// The entries of the right kind among the first [`MAX_ENTRIES`] (after references), in file
    /// order: strings as text cut at [`MAX_TEXT_CHARS`] characters, names cut at 127 bytes.
    pub items: Vec<String>,
    /// How many entries the array has (after references), listed or not.
    pub count: usize,
    /// The array has more entries than [`MAX_ENTRIES`], so the rest were not looked at.
    pub truncated: bool,
    /// Entries among the first [`MAX_ENTRIES`] that are not of the right kind, and so not listed.
    pub skipped: usize,
}

/// What [`read_signature_seed`] read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureSeed {
    /// The field has an `/SV` entry that is not null: what a removal takes out.
    pub has_entry: bool,
    /// The entry is a dictionary after references.
    pub present: bool,
    /// `/Ff` when it is an integer (after references).
    pub flags: Option<i64>,
    /// The entries whose bit is set in `flags`, among the four this models.
    pub required: Vec<Entry>,
    /// `/Reasons`: strings.
    pub reasons: SeedList,
    /// `/SubFilter`: names (any name is listed, not only [`SUB_FILTERS`]).
    pub sub_filters: SeedList,
    /// `/DigestMethod`: names (any name is listed, not only [`DIGEST_METHODS`]).
    pub digest_methods: SeedList,
    /// `/AddRevInfo` when it is a boolean (after references).
    pub add_rev_info: Option<bool>,
    /// Keys of the `/SV` dictionary other than `/Type`, `/Ff` and the four entries above, whether
    /// or not their value is null; a set is refused while there is one.
    pub other_entries: usize,
    /// Every listed reason and name is whole: false when a reason was cut at
    /// [`MAX_TEXT_CHARS`] characters, or a name was cut at 127 bytes or is not valid UTF-8 (a
    /// reason that is not valid text is shown as it decodes and is not detected) (true when
    /// none is listed).
    pub exact: bool,
    /// The field holds a signature: its `/V` is there and not null (a reference counts, without
    /// being followed). A signed field's seed values are not edited.
    pub signed: bool,
    /// [`set_signature_seed`] and a removal can edit this field: it is a dictionary, holds no
    /// signature and is not the catalog, the `/Pages` node, a node under it or a page (found by
    /// number), and that page-tree walk was not cut short.
    pub settable: bool,
    /// The [`MAX_GETS`] allowance was spent, so what follows the stop was not read.
    pub capped: bool,
}

impl Default for SignatureSeed {
    /// No seed values, nothing cut (`exact` is true when nothing is listed), not settable.
    fn default() -> Self {
        SignatureSeed {
            has_entry: false,
            present: false,
            flags: None,
            required: Vec::new(),
            reasons: SeedList::default(),
            sub_filters: SeedList::default(),
            digest_methods: SeedList::default(),
            add_rev_info: None,
            other_entries: 0,
            exact: true,
            signed: false,
            settable: false,
            capped: false,
        }
    }
}

fn cut(s: &PdfString) -> (String, bool) {
    let mut chars = s.text_chars();
    let shown: String = chars.by_ref().take(MAX_TEXT_CHARS).collect();
    (shown, chars.next().is_none())
}

fn list(doc: &Document, gets: &mut Gets, entry: Option<&Object>, names: bool, exact: &mut bool, out: &mut SeedList) -> Result<(), Spent> {
    let Some(entry) = entry else { return Ok(()) };
    let held = gets.follow(doc, entry)?;
    let Object::Array(items) = &*held else { return Ok(()) };
    out.count = items.len();
    out.truncated = items.len() > MAX_ENTRIES;
    for item in items.iter().take(MAX_ENTRIES) {
        gets.charge()?;
        match (&*gets.follow(doc, item)?, names) {
            (Object::Name(n), true) => {
                let (text, whole) = shown(n);
                out.items.push(text);
                *exact &= whole;
            }
            (Object::String(s), false) => {
                let (text, whole) = cut(s);
                out.items.push(text);
                *exact &= whole;
            }
            _ => out.skipped += 1,
        }
    }
    Ok(())
}

fn read(doc: &Document, gets: &mut Gets, field: ObjRef, out: &mut SignatureSeed) -> Result<(), Spent> {
    let held = gets.get(doc, field)?;
    let Some(d) = held.as_dict() else { return Ok(()) };
    out.settable = true;
    out.signed = d.get(b"V").is_some_and(|v| !matches!(v, Object::Null));
    let Some(entry) = d.get(b"SV") else { return Ok(()) };
    out.has_entry = !matches!(entry, Object::Null);
    let sv = gets.follow(doc, entry)?;
    let Object::Dict(sv) = &*sv else { return Ok(()) };
    out.present = true;
    out.other_entries =
        sv.iter().filter(|(k, _)| !matches!(k.as_slice(), b"Type" | b"Ff" | b"Reasons" | b"SubFilter" | b"DigestMethod" | b"AddRevInfo")).count();
    if let Some(v) = sv.get(b"Ff") {
        out.flags = gets.follow(doc, v)?.as_int();
        let flags = out.flags.unwrap_or(0);
        out.required = ENTRIES.into_iter().filter(|e| flags & e.bit() != 0).collect();
    }
    list(doc, gets, sv.get(b"SubFilter"), true, &mut out.exact, &mut out.sub_filters)?;
    list(doc, gets, sv.get(b"Reasons"), false, &mut out.exact, &mut out.reasons)?;
    list(doc, gets, sv.get(b"DigestMethod"), true, &mut out.exact, &mut out.digest_methods)?;
    if let Some(v) = sv.get(b"AddRevInfo")
        && let Object::Bool(b) = &*gets.follow(doc, v)?
    {
        out.add_rev_info = Some(*b);
    }
    Ok(())
}

/// The seed values of `field` as stored, with whether they can be edited. At most [`MAX_GETS`]
/// reads of its own, plus one walk of the page tree when the object is a dictionary that holds no
/// signature (see the module notes). An object that is not a dictionary, or is missing, reads as
/// no seed values and is not settable.
pub fn read_signature_seed(doc: &Document, field: ObjRef) -> SignatureSeed {
    read_with(doc, field, MAX_GETS)
}

pub(crate) fn read_with(doc: &Document, field: ObjRef, gets: usize) -> SignatureSeed {
    let mut out = SignatureSeed::default();
    let mut gets = Gets(gets);
    if read(doc, &mut gets, field, &mut out).is_err() {
        out.capped = true;
    }
    // `settable` was set by the read when the object is a dictionary; the walk narrows it.
    if out.settable && !out.signed && !out.capped {
        let (nodes, cut_short) = structural(doc);
        out.settable = !cut_short && !nodes.contains(&field.num);
    } else {
        out.settable = false;
    }
    out
}

/// What [`set_signature_seed`] would do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// It would write the field's dictionary.
    Yes,
    /// It would write nothing: a removal of seed values the field does not have.
    NoChange,
    /// It is refused, for this reason.
    Refused(&'static str),
}

/// What setting `seed` (a removal when `None`) on `field` would do.
pub fn signature_seed_change(doc: &Document, field: ObjRef, seed: Option<&SeedSpec>) -> Change {
    change_with(doc, field, seed, MAX_GETS)
}

pub(crate) fn change_with(doc: &Document, field: ObjRef, seed: Option<&SeedSpec>, gets: usize) -> Change {
    let read = read_with(doc, field, gets);
    if read.capped {
        return Change::Refused("the field could not be read within the allowance of reads");
    }
    if read.signed {
        return Change::Refused("the field holds a signature, and a signed field's seed values are not edited");
    }
    if !read.settable {
        return Change::Refused("the object is not a field dictionary this can edit (the catalog, the page tree and pages are not)");
    }
    if seed.is_none() && !read.has_entry {
        return Change::NoChange;
    }
    if seed.is_some() && read.other_entries > 0 {
        return Change::Refused("the field's seed values have entries this does not write (remove them first, which drops those too)");
    }
    Change::Yes
}

/// Set the `/SV` of `field` to a new direct dictionary, or take `/SV` out when `seed` is `None`.
/// A removal of seed values the field does not have is no change. An indirect `/SV` dictionary
/// that is replaced or removed is left in the file unreferenced.
pub fn set_signature_seed(doc: &mut Document, field: ObjRef, seed: Option<&SeedSpec>) -> Result<(), OrganizeError> {
    match signature_seed_change(doc, field, seed) {
        Change::NoChange => return Ok(()),
        Change::Refused(why) => return Err(OrganizeError::Invalid(why.into())),
        Change::Yes => {}
    }
    doc.update_dict(field, |d| match seed {
        Some(spec) => d.set(b"SV".to_vec(), Object::Dict(spec.to_dict())),
        None => {
            d.remove(b"SV");
        }
    })?;
    Ok(())
}
