//! XMP metadata for PDF/A: the identification schema and the document information, kept
//! consistent (ISO 19005-2 §6.6).

use pdfcraft_cos::Document;

/// The largest XMP packet read; a larger one is treated as missing.
pub(crate) const MAX_XMP_BYTES: usize = 8 * 1024 * 1024;
/// Namespace URIs of the PDF/A and PDF/UA identification schemas.
pub(crate) const PDFA_ID: &str = "http://www.aiim.org/pdfa/ns/id/";
pub(crate) const PDFUA_ID: &str = "http://www.aiim.org/pdfua/ns/id/";
/// The PDF/X identification schema (`pdfxid:`).
pub(crate) const PDFX_ID: &str = "http://www.npes.org/pdfx/ns/id/";

/// Read an identification property by its expanded XML name, not its arbitrary prefix. A packet
/// that is not well-formed XML (an undeclared prefix, an illegal character) or is too deep or wide
/// to parse safely is read by the conventional prefix (`pdfaid:`, `pdfuaid:`) instead, as before.
pub(crate) fn identifier(xmp: &str, namespace: &str, local: &str) -> Option<String> {
    if xmp.len() > MAX_XMP_BYTES {
        return None;
    }
    match parsed_identifier(xmp, namespace, local) {
        Ok(found) => found,
        Err(()) => {
            let prefix = match namespace {
                PDFA_ID => "pdfaid",
                PDFUA_ID => "pdfuaid",
                PDFX_ID => "pdfxid",
                _ => return None,
            };
            value(xmp, &format!("{prefix}:{local}"))
        }
    }
}

/// Namespace scope tracking for `craft_xml::El` trees.
#[derive(Clone)]
pub(crate) struct NsScope<'a>(Vec<(&'a str, &'a str)>);

impl<'a> NsScope<'a> {
    pub(crate) fn root() -> Self {
        Self(vec![("xml", "http://www.w3.org/XML/1998/namespace")])
    }

    pub(crate) fn child(&self, el: &'a craft_xml::El) -> Self {
        let mut next = self.0.clone();
        next.extend(el.ns_decls());
        Self(next)
    }

    pub(crate) fn el_ns(&self, el: &craft_xml::El) -> Option<&'a str> {
        let prefix = el.prefix();
        self.0.iter().rfind(|(p, _)| *p == prefix).map(|(_, u)| *u).filter(|u| !u.is_empty())
    }

    pub(crate) fn attr_ns(&self, attr_name: &str) -> Option<&'a str> {
        let (prefix, _) = attr_name.split_once(':')?;
        if prefix == "xmlns" {
            return None;
        }
        self.0.iter().rfind(|(p, _)| *p == prefix).map(|(_, u)| *u).filter(|u| !u.is_empty())
    }

    pub(crate) fn attr(&self, el: &'a craft_xml::El, ns: &str, local: &str) -> Option<&'a str> {
        el.attrs.iter().find_map(|(k, v)| {
            let (_, l) = k.split_once(':')?;
            (l == local && self.attr_ns(k) == Some(ns)).then_some(v.as_str())
        })
    }

    pub(crate) fn is(&self, el: &craft_xml::El, ns: &str, local: &str) -> bool {
        el.local() == local && self.el_ns(el) == Some(ns)
    }
}

/// `xmp` parsed within the depth, attribute and node limits; `None` when it can't be.
pub(crate) fn parse(xmp: &str) -> Option<craft_xml::El> {
    if !xml_limits_ok(xmp) {
        return None;
    }
    options().parse(xmp.as_bytes()).ok()
}

/// The XML reader settings every XMP read uses: strict, no DOCTYPE, bounded depth and size.
fn options() -> craft_xml::Options<'static> {
    craft_xml::Options { doctype: craft_xml::Doctype::Reject, ..craft_xml::Options::default() }.strict().with_limits(craft_xml::Limits {
        max_depth: 1024,
        max_nodes: 100_000,
        max_attrs_total: 100_000,
        ..craft_xml::Limits::DEFAULT
    })
}

/// [`identifier`] through an XML parser; `Err` when the packet can't be parsed within the limits.
fn parsed_identifier(xmp: &str, namespace: &str, local: &str) -> Result<Option<String>, ()> {
    fn walk<'a>(el: &'a craft_xml::El, parent: &NsScope<'a>, namespace: &str, local: &str) -> Result<Option<String>, ()> {
        let scope = parent.child(el);
        let el_prefix = el.prefix();
        if !el_prefix.is_empty() && el_prefix != "xml" && scope.el_ns(el).is_none() {
            return Err(());
        }
        for (k, _) in &el.attrs {
            if let Some((prefix, _)) = k.split_once(':')
                && !matches!(prefix, "xml" | "xmlns")
                && scope.attr_ns(k).is_none()
            {
                return Err(());
            }
        }
        if let Some(value) = scope.attr(el, namespace, local) {
            return Ok(Some(value.trim().to_owned()));
        }
        if scope.is(el, namespace, local) {
            return Ok(Some(el.text().trim().to_owned()));
        }
        for child in el.elements() {
            if let Some(found) = walk(child, &scope, namespace, local)? {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }
    let root = parse(xmp).ok_or(())?;
    walk(&root, &NsScope::root(), namespace, local)
}

/// Reject deep XML and start tags with too many attributes before parsing.
/// Each unquoted `=` in a valid start tag introduces an attribute, including namespace bindings.
/// Every `b[i]` and `hay[p..]` below is guarded by `i < b.len()` / `p < hay.len()`.
fn xml_limits_ok(text: &str) -> bool {
    fn find(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
        (from..hay.len()).find(|&p| hay[p..].starts_with(needle))
    }
    let b = text.as_bytes();
    let (mut i, mut depth, mut total_attributes) = (0usize, 0usize, 0usize);
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
        } else if b[i..].starts_with(b"<!--") {
            i = find(b, i + 4, b"-->").map_or(b.len(), |p| p + 3);
        } else if b[i..].starts_with(b"<![CDATA[") {
            i = find(b, i + 9, b"]]>").map_or(b.len(), |p| p + 3);
        } else if b[i..].starts_with(b"<?") {
            i = find(b, i + 2, b"?>").map_or(b.len(), |p| p + 2);
        } else if b[i..].starts_with(b"<!") {
            i = find(b, i + 2, b">").map_or(b.len(), |p| p + 1);
        } else if b[i..].starts_with(b"</") {
            depth = depth.saturating_sub(1);
            i = find(b, i + 2, b">").map_or(b.len(), |p| p + 1);
        } else {
            depth += 1;
            if depth > 1024 {
                return false;
            }
            // Skip the rest of the start tag, over any quoted attribute value, and remember its last
            // significant byte: a tag that ends in `/` closes itself, so it is one level deep while it
            // is read rather than a level left open.
            i += 1;
            let mut quote = 0u8;
            let mut last = 0u8;
            let mut attributes = 0usize;
            while i < b.len() {
                let c = b[i];
                if quote != 0 {
                    if c == quote {
                        quote = 0;
                    }
                } else if c == b'"' || c == b'\'' {
                    quote = c;
                } else if c == b'>' {
                    i += 1;
                    break;
                } else if c == b'=' {
                    attributes += 1;
                    total_attributes += 1;
                    if attributes > 128 || total_attributes > 4096 {
                        return false;
                    }
                    last = c;
                } else if !c.is_ascii_whitespace() {
                    last = c;
                }
                i += 1;
            }
            if last == b'/' {
                depth -= 1;
            }
        }
    }
    true
}

/// The value of a simple XMP property, written either as an attribute (`pdfaid:part="2"`) or
/// as an element (`<pdfaid:part>2</pdfaid:part>`, or inside an `rdf:Alt`/`rdf:Seq`).
pub fn value(xmp: &str, key: &str) -> Option<String> {
    let attr = format!("{key}=\"");
    if let Some(i) = xmp.find(&attr) {
        let rest = &xmp[i + attr.len()..];
        return rest.find('"').map(|e| unescape(&rest[..e]));
    }
    let attr1 = format!("{key}='");
    if let Some(i) = xmp.find(&attr1) {
        let rest = &xmp[i + attr1.len()..];
        return rest.find('\'').map(|e| unescape(&rest[..e]));
    }
    let open = format!("<{key}>");
    let close = format!("</{key}>");
    let start = xmp.find(&open)? + open.len();
    let end = start + xmp[start..].find(&close)?;
    let inner = &xmp[start..end];
    // rdf:Alt / rdf:Seq: the first rdf:li.
    if let Some(li) = inner.find("<rdf:li") {
        let s = li + inner[li..].find('>')? + 1;
        let e = s + inner[s..].find("</rdf:li>")?;
        return Some(unescape(inner[s..e].trim()));
    }
    Some(unescape(inner.trim()))
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A PDF date (`D:YYYYMMDDHHmmSSOHH'mm'`) as an XMP (ISO 8601) date.
pub fn iso_date(pdf: &str) -> Option<String> {
    let d = pdf.trim().trim_start_matches("D:");
    let digits: String = d.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() < 4 {
        return None;
    }
    let part = |a: usize, b: usize, def: &str| digits.get(a..b).unwrap_or(def).to_string();
    let (y, mo, da, h, mi, s) = (part(0, 4, "0000"), part(4, 6, "01"), part(6, 8, "01"), part(8, 10, "00"), part(10, 12, "00"), part(12, 14, "00"));
    let rest = &d[digits.len()..];
    let tz = match rest.chars().next() {
        Some('Z') | None => "Z".to_string(),
        Some(sign @ ('+' | '-')) => {
            let t: String = rest[1..].chars().filter(char::is_ascii_digit).collect();
            format!("{sign}{}:{}", t.get(0..2).unwrap_or("00"), t.get(2..4).unwrap_or("00"))
        }
        _ => "Z".to_string(),
    };
    Some(format!("{y}-{mo}-{da}T{h}:{mi}:{s}{tz}"))
}

/// The XMP namespace, where `ModifyDate` lives (the conventional prefixes are `xmp:` and `xap:`).
const XMP_NS: &str = "http://ns.adobe.com/xap/1.0/";
const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

/// The name an element is written with (prefix included), read from its start tag.
fn written_name(element: &str) -> Option<&str> {
    element.strip_prefix('<')?.split(|c: char| c.is_whitespace() || c == '>' || c == '/').next()
}

/// `xmp` with `ModifyDate` of the XMP namespace set to `iso`, an XMP date. Properties are found
/// by namespace, not prefix, so `xmp:` and `xap:` both count, and an element (empty, with
/// attributes, or with text) or an attribute is replaced in place; no second one is added. A
/// PDF/A packet without one gets it as a child of its first `rdf:Description`. Any other packet
/// without one, one that is not well-formed or within the parser's limits, and one that already
/// says `iso`, give `None`.
pub fn with_modify_date(xmp: &str, iso: &str) -> Option<String> {
    // A byte order mark is not part of the XML text: read without it and give it back.
    let (bom, xmp) = match xmp.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", xmp),
    };
    // Well-formed and within the parser's limits, or no edit at all; the event pass below then
    // only needs to find byte ranges.
    parse(xmp)?;
    let mut events = options().events(xmp.as_bytes());
    // (byte range replaced, its replacement)
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    // Namespace bindings in scope; each open element remembers how many it added and where it began.
    let mut scope: Vec<(String, String)> = vec![("xml".into(), "http://www.w3.org/XML/1998/namespace".into())];
    struct Open {
        added: usize,
        start: std::ops::Range<usize>,
        modify: bool,
        /// Value ranges of ModifyDate attributes of a ModifyDate element, rewritten with its start tag.
        own_attrs: Vec<std::ops::Range<usize>>,
        description: Option<Option<String>>,
    }
    let mut open: Vec<Open> = Vec::new();
    // The first rdf:Description: its whole range and the prefix bound to the XMP namespace there.
    let mut first_description: Option<(std::ops::Range<usize>, Option<String>)> = None;
    let mut seen_description = false;
    let namespace_of =
        |scope: &[(String, String)], prefix: &str| scope.iter().rfind(|(p, _)| p == prefix).map(|(_, u)| u.clone()).filter(|u| !u.is_empty());
    while let Some(event) = events.next().ok()? {
        match event {
            craft_xml::Event::Start { name, attrs } => {
                let span = events.span();
                let before = scope.len();
                for (k, v) in &attrs {
                    if k == "xmlns" {
                        scope.push((String::new(), v.clone()));
                    } else if let Some(p) = k.strip_prefix("xmlns:") {
                        scope.push((p.to_owned(), v.clone()));
                    }
                }
                let (prefix, local) = name.split_once(':').unwrap_or(("", name.as_str()));
                let ns = namespace_of(&scope, prefix);
                // A prefix nothing declares: the packet is not namespace-well-formed, leave it alone.
                if !prefix.is_empty() && prefix != "xml" && ns.is_none() {
                    return None;
                }
                let tag = xmp.get(span.clone())?;
                // What is inside a ModifyDate element is replaced with it; nothing there is edited on its own.
                let inside_modify = open.iter().any(|o| o.modify);
                let modify = !inside_modify && local == "ModifyDate" && ns.as_deref() == Some(XMP_NS);
                let mut own_attrs = Vec::new();
                // Attributes in the XMP namespace named ModifyDate: replace the value in place.
                for (aname, value_range) in tag_attributes(tag) {
                    let Some((ap, alocal)) = aname.split_once(':') else { continue };
                    if ap == "xmlns" || ap == "xml" {
                        continue;
                    }
                    let ans = namespace_of(&scope, ap);
                    ans.as_ref()?;
                    if alocal == "ModifyDate" && ans.as_deref() == Some(XMP_NS) && !inside_modify {
                        let range = span.start + value_range.start..span.start + value_range.end;
                        if modify {
                            own_attrs.push(range);
                        } else {
                            edits.push((range, iso.to_owned()));
                        }
                    }
                }
                let description = if !inside_modify && local == "Description" && ns.as_deref() == Some(RDF_NS) && !seen_description {
                    seen_description = true;
                    Some(namespace_prefix(&scope, XMP_NS))
                } else {
                    None
                };
                // An empty element's end event follows at once with the same span.
                open.push(Open { added: scope.len() - before, start: span, modify, own_attrs, description });
            }
            craft_xml::Event::End { .. } => {
                let end = events.span();
                let Some(el) = open.pop() else { continue };
                let whole = el.start.start..end.end;
                if el.modify {
                    let text = xmp.get(whole.clone())?;
                    let name = written_name(text)?;
                    // Keep the start tag's own attributes (such as namespace declarations) and write the value fresh.
                    let mut start = xmp.get(el.start.clone())?.to_owned();
                    let mut own = el.own_attrs.clone();
                    own.sort_by_key(|r| std::cmp::Reverse(r.start));
                    for r in own {
                        let rel = r.start.checked_sub(el.start.start)?..r.end.checked_sub(el.start.start)?;
                        start.get(rel.clone())?;
                        start.replace_range(rel, iso);
                    }
                    let open_tag = start.strip_suffix("/>").map_or_else(|| start.clone(), |t| format!("{t}>"));
                    edits.push((whole.clone(), format!("{open_tag}{iso}</{name}>")));
                }
                if let Some(prefix) = el.description {
                    first_description = Some((whole, prefix));
                }
                let keep = scope.len().saturating_sub(el.added);
                scope.truncate(keep);
            }
            _ => {}
        }
    }
    if edits.is_empty() {
        identifier(xmp, PDFA_ID, "part")?;
        let (range, prefix) = first_description?;
        let whole = xmp.get(range.clone())?;
        let written = written_name(whole)?;
        // A prefix already bound to the XMP namespace here; otherwise the element declares its own.
        let child = match prefix {
            Some(p) => format!("<{p}:ModifyDate>{iso}</{p}:ModifyDate>"),
            None => format!("<xmp:ModifyDate xmlns:xmp=\"{XMP_NS}\">{iso}</xmp:ModifyDate>"),
        };
        if let Some(head) = whole.strip_suffix("/>") {
            edits.push((range, format!("{head}>\n   {child}\n  </{written}>")));
        } else {
            let close = range.end.checked_sub(whole.len() - whole.rfind("</")?)?;
            edits.push((close..close, format!("   {child}\n  ")));
        }
    }
    edits.sort_by_key(|(r, _)| std::cmp::Reverse(r.start));
    // Applied last-start-first: each range must lie before the one applied before it, and on characters.
    let mut out = xmp.to_owned();
    let mut limit = out.len();
    for (range, with) in edits {
        if range.end > limit || !out.is_char_boundary(range.start) || !out.is_char_boundary(range.end) || range.start > range.end {
            return None;
        }
        out.replace_range(range.clone(), &with);
        limit = range.start;
    }
    (out != xmp).then(|| format!("{bom}{out}"))
}

/// The prefix bound to `uri` in `scope` (innermost wins), if it is a non-empty one.
fn namespace_prefix(scope: &[(String, String)], uri: &str) -> Option<String> {
    let (p, u) = scope.iter().rfind(|(_, u)| u == uri)?;
    // A later binding of the same prefix to another URI hides this one.
    let current = scope.iter().rfind(|(q, _)| q == p)?;
    (current.1 == *u && !p.is_empty()).then(|| p.clone())
}

/// The attributes of a start tag as written: each name with the byte range of its value (inside
/// the quotes) in `tag`. Stops at the first thing that is not an attribute.
fn tag_attributes(tag: &str) -> Vec<(&str, std::ops::Range<usize>)> {
    let bytes = tag.as_bytes();
    let mut out = Vec::new();
    let mut i = 1;
    let skip_ws = |mut i: usize| {
        while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        i
    };
    // The element name.
    while bytes.get(i).is_some_and(|b| !b.is_ascii_whitespace() && *b != b'>' && *b != b'/') {
        i += 1;
    }
    loop {
        i = skip_ws(i);
        let name_start = i;
        while bytes.get(i).is_some_and(|b| !b.is_ascii_whitespace() && !matches!(*b, b'=' | b'>' | b'/')) {
            i += 1;
        }
        if i == name_start {
            break;
        }
        let name_end = i;
        i = skip_ws(i);
        if bytes.get(i) != Some(&b'=') {
            break;
        }
        i = skip_ws(i + 1);
        let Some(&quote) = bytes.get(i).filter(|q| matches!(**q, b'"' | b'\'')) else { break };
        let value_start = i + 1;
        let Some(len) = bytes.get(value_start..).and_then(|rest| rest.iter().position(|b| *b == quote)) else { break };
        let value_end = value_start + len;
        if let Some(name) = tag.get(name_start..name_end) {
            out.push((name, value_start..value_end));
        }
        i = value_end + 1;
    }
    out
}

fn info(doc: &Document, key: &str) -> Option<String> {
    let i = doc.trailer().get(b"Info").map(|i| doc.resolve(i))?;
    i.as_dict()?.get(key.as_bytes()).and_then(|v| doc.resolve(v).as_string().map(|s| s.to_text())).filter(|s| !s.is_empty())
}

/// The XMP packet for `doc` declaring `level`, from its document information.
pub fn packet(doc: &Document, level: crate::Level) -> String {
    let mut props = String::new();
    if let Some(t) = info(doc, "Title") {
        props.push_str(&format!("   <dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>\n", esc(&t)));
    }
    if let Some(a) = info(doc, "Author") {
        props.push_str(&format!("   <dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>\n", esc(&a)));
    }
    if let Some(s) = info(doc, "Subject") {
        props.push_str(&format!("   <dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:description>\n", esc(&s)));
    }
    if let Some(k) = info(doc, "Keywords") {
        props.push_str(&format!("   <pdf:Keywords>{}</pdf:Keywords>\n", esc(&k)));
    }
    if let Some(p) = info(doc, "Producer") {
        props.push_str(&format!("   <pdf:Producer>{}</pdf:Producer>\n", esc(&p)));
    }
    if let Some(c) = info(doc, "Creator") {
        props.push_str(&format!("   <xmp:CreatorTool>{}</xmp:CreatorTool>\n", esc(&c)));
    }
    if let Some(d) = info(doc, "CreationDate").and_then(|d| iso_date(&d)) {
        props.push_str(&format!("   <xmp:CreateDate>{d}</xmp:CreateDate>\n"));
    }
    if let Some(d) = info(doc, "ModDate").and_then(|d| iso_date(&d)) {
        props.push_str(&format!("   <xmp:ModifyDate>{d}</xmp:ModifyDate>\n"));
    }
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
 <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
  <rdf:Description rdf:about=\"\"\n\
    xmlns:dc=\"http://purl.org/dc/elements/1.1/\"\n\
    xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\"\n\
    xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"\n\
    xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\">\n\
   <pdfaid:part>{}</pdfaid:part>\n\
   <pdfaid:conformance>B</pdfaid:conformance>\n\
{props}  </rdf:Description>\n\
 </rdf:RDF>\n\
</x:xmpmeta>\n\
<?xpacket end=\"w\"?>",
        level.part()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_attribute_and_element_forms() {
        let a = "<rdf:Description pdfaid:part=\"2\" pdfaid:conformance='B'/>";
        assert_eq!(value(a, "pdfaid:part").as_deref(), Some("2"));
        assert_eq!(value(a, "pdfaid:conformance").as_deref(), Some("B"));
        let e = "<dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">A &amp; B</rdf:li></rdf:Alt></dc:title><pdfaid:part>3</pdfaid:part>";
        assert_eq!(value(e, "dc:title").as_deref(), Some("A & B"));
        assert_eq!(value(e, "pdfaid:part").as_deref(), Some("3"));
        assert_eq!(value(e, "pdfuaid:part"), None);
    }

    const HEAD: &str = "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">";
    const TAIL: &str = "</rdf:RDF></x:xmpmeta>";
    const ISO: &str = "2023-11-14T22:13:20Z";

    fn doc(body: &str) -> String {
        format!("{HEAD}{body}{TAIL}")
    }

    fn modified(xmp: &str) -> Vec<String> {
        fn walk(el: &craft_xml::El, parent: &NsScope<'_>, found: &mut Vec<String>) {
            let scope = parent.child(el);
            if scope.is(el, XMP_NS, "ModifyDate") {
                found.push(el.text().into_owned());
            }
            if let Some(v) = scope.attr(el, XMP_NS, "ModifyDate") {
                found.push(v.to_owned());
            }
            for child in el.elements() {
                walk(child, &scope, found);
            }
        }
        let d = parse(xmp).expect("well-formed");
        let mut found = Vec::new();
        walk(&d, &NsScope::root(), &mut found);
        found
    }

    #[test]
    fn modify_date_is_replaced_in_every_form_by_namespace() {
        let old = "2020-01-01T00:00:00Z";
        for body in [
            format!("<rdf:Description xmlns:xmp=\"{XMP_NS}\"><xmp:ModifyDate>{old}</xmp:ModifyDate></rdf:Description>"),
            format!("<rdf:Description xmlns:xap=\"{XMP_NS}\"><xap:ModifyDate>{old}</xap:ModifyDate></rdf:Description>"),
            format!("<rdf:Description xmlns:xmp=\"{XMP_NS}\"><xmp:ModifyDate/></rdf:Description>"),
            format!("<rdf:Description xmlns:xmp=\"{XMP_NS}\"><xmp:ModifyDate rdf:x=\"1\"></xmp:ModifyDate></rdf:Description>"),
            format!("<rdf:Description xmlns:xmp=\"{XMP_NS}\" xmp:ModifyDate=\"{old}\"/>"),
            format!("<rdf:Description xmlns:xmp=\"{XMP_NS}\" xmp:ModifyDate = '{old}'/>"),
        ] {
            let out = with_modify_date(&doc(&body), ISO).unwrap_or_else(|| panic!("changed: {body}"));
            assert_eq!(modified(&out), [ISO], "{out}");
        }
        let current = doc(&format!("<rdf:Description xmlns:xmp=\"{XMP_NS}\"><xmp:ModifyDate>{ISO}</xmp:ModifyDate></rdf:Description>"));
        assert_eq!(with_modify_date(&current, ISO), None, "already current");
    }

    /// A packet with `body` as the one description, its namespaces declared on the root.
    fn desc(body: &str) -> String {
        doc(&format!("<rdf:Description xmlns:xmp=\"{XMP_NS}\">{body}</rdf:Description>"))
    }

    #[test]
    fn overlapping_modify_date_forms_do_not_panic() {
        // An element that also carries the attribute, nested elements, and an attribute inside an
        // element each used to give overlapping edits, applied out of range.
        let long = "2020-01-01T00:00:00Z and a longer value than the date";
        for body in [
            format!("<xmp:ModifyDate xmp:ModifyDate=\"{long}\"/>"),
            format!("<xmp:ModifyDate xmp:ModifyDate=\"{long}\">{long}</xmp:ModifyDate>"),
            format!("<xmp:ModifyDate><xmp:ModifyDate>{long}</xmp:ModifyDate></xmp:ModifyDate>"),
            format!("<xmp:ModifyDate><x xmp:ModifyDate=\"{long}\"/>text</xmp:ModifyDate>"),
        ] {
            let packet = desc(&body);
            let out = with_modify_date(&packet, ISO).unwrap_or_else(|| panic!("changed: {body}"));
            assert!(out.contains(ISO), "{out}");
            assert!(!out.contains(long), "{out}");
            assert!(parse(&out).is_some(), "still well-formed: {out}");
        }
        // The bare form from the review: a long attribute on a self-closed element.
        let packet = format!("<xmp:ModifyDate xmlns:xmp=\"{XMP_NS}\" xmp:ModifyDate=\"{}\"/>", "x".repeat(40));
        let out = with_modify_date(&packet, ISO).expect("changed");
        assert!(parse(&out).is_some() && !out.contains("xxxx"), "{out}");
    }

    #[test]
    fn start_tag_values_with_markup_characters_are_kept() {
        for tricky in ["a>b", "a=b", "it's", "say \"hi\" >"] {
            let quote = if tricky.contains('"') { '\'' } else { '"' };
            let body = format!("<xmp:ModifyDate note={quote}{tricky}{quote}>2020</xmp:ModifyDate>");
            let out = with_modify_date(&desc(&body), ISO).expect("changed");
            assert!(out.contains(&format!("note={quote}{tricky}{quote}>{ISO}</xmp:ModifyDate>")), "{out}");
            // The same text in the attribute being replaced and in a neighbour.
            let body = format!("<xmp:ModifyDate note={quote}{tricky}{quote}/>");
            let out = with_modify_date(&desc(&body), ISO).expect("changed");
            assert!(out.contains(&format!("note={quote}{tricky}{quote}>{ISO}</xmp:ModifyDate>")), "{out}");
            let body = format!("<rdf:Description xmp:ModifyDate=\"2020\" note={quote}{tricky}{quote}/>");
            let out = with_modify_date(&desc(&body), ISO).expect("changed");
            assert_eq!(modified(&out), [ISO], "{out}");
            assert!(out.contains(&format!("note={quote}{tricky}{quote}")), "{out}");
        }
    }

    #[test]
    fn default_namespace_and_rebound_prefixes_are_read_by_namespace() {
        let packet = format!("{HEAD}<rdf:Description><ModifyDate xmlns=\"{XMP_NS}\">2020</ModifyDate></rdf:Description>{TAIL}");
        let out = with_modify_date(&packet, ISO).expect("changed");
        assert!(out.contains(&format!("<ModifyDate xmlns=\"{XMP_NS}\">{ISO}</ModifyDate>")), "{out}");
        // A child that rebinds the prefix to another namespace: its ModifyDate is not the XMP one.
        let body = "<xmp:ModifyDate>2020</xmp:ModifyDate><x xmlns:xmp=\"urn:other\"><xmp:ModifyDate>keep</xmp:ModifyDate></x>";
        let out = with_modify_date(&desc(body), ISO).expect("changed");
        assert!(out.contains("<xmp:ModifyDate>keep</xmp:ModifyDate>"), "{out}");
        assert_eq!(modified(&out), [ISO], "{out}");
    }

    #[test]
    fn multibyte_cdata_and_entity_content_is_replaced_cleanly() {
        for inner in ["2020-01-01", "\u{65e5}\u{672c}\u{8a9e} \u{1f600}", "<![CDATA[a > b]]>", "&amp;&#x41;&lt;"] {
            let body = format!("\u{3042}\u{3044}<!-- \u{1f600} --><xmp:ModifyDate note=\"\u{3046}\">{inner}</xmp:ModifyDate>\u{3048}");
            let out = with_modify_date(&desc(&body), ISO).expect("changed");
            assert_eq!(modified(&out), [ISO], "{out}");
            assert!(out.contains("\u{3042}\u{3044}<!--") && out.contains("\u{3048}</rdf:Description>"), "{out}");
        }
        let body = "<xmp:ModifyDate>\u{1f600}</xmp:ModifyDate>";
        let out = with_modify_date(&format!("\u{feff}{}", desc(body)), ISO).expect("bom packet");
        assert!(out.starts_with('\u{feff}') && modified(out.trim_start_matches('\u{feff}')) == [ISO], "{out}");
    }

    #[test]
    fn an_undeclared_prefix_is_left_untouched() {
        let packet = doc("<rdf:Description><ns:ModifyDate>2020</ns:ModifyDate></rdf:Description>");
        assert_eq!(with_modify_date(&packet, ISO), None);
        let packet = doc(&format!("<rdf:Description xmlns:xmp=\"{XMP_NS}\" nope:ModifyDate=\"1\" xmp:ModifyDate=\"2\"/>"));
        assert_eq!(with_modify_date(&packet, ISO), None);
    }

    #[test]
    fn modify_date_is_added_to_pdfa_once_and_otherwise_left() {
        let aid = "xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\"";
        // xmp declared on an earlier Description; the last is pdfaid only.
        let two = doc(&format!(
            "<rdf:Description rdf:about=\"\" xmlns:xmp=\"{XMP_NS}\"><xmp:CreatorTool>t</xmp:CreatorTool></rdf:Description>\
<rdf:Description rdf:about=\"\" {aid}><pdfaid:part>2</pdfaid:part></rdf:Description>"
        ));
        let out = with_modify_date(&two, ISO).unwrap();
        assert_eq!(modified(&out), [ISO], "{out}");
        assert_eq!(identifier(&out, PDFA_ID, "part").as_deref(), Some("2"));
        // No xmp namespace anywhere, and a self-closing Description.
        let bare = doc(&format!("<rdf:Description rdf:about=\"\" {aid} pdfaid:part=\"2\"/>"));
        let out = with_modify_date(&bare, ISO).unwrap();
        assert_eq!(modified(&out), [ISO], "{out}");
        // Not PDF/A: untouched. Not well-formed: untouched.
        assert_eq!(with_modify_date(&doc("<rdf:Description rdf:about=\"\"></rdf:Description>"), ISO), None);
        assert_eq!(with_modify_date("<xmp:ModifyDate>unterminated", ISO), None);
    }

    #[test]
    fn pdf_dates_become_iso_dates() {
        assert_eq!(iso_date("D:20240105093000+01'00'").as_deref(), Some("2024-01-05T09:30:00+01:00"));
        assert_eq!(iso_date("D:20240105").as_deref(), Some("2024-01-05T00:00:00Z"));
        assert_eq!(iso_date("D:20240105120000Z").as_deref(), Some("2024-01-05T12:00:00Z"));
        assert_eq!(iso_date("junk"), None);
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;

    #[test]
    fn attribute_and_namespace_budgets_bound_parser_work() {
        let prefix = r#"<root xmlns:a="http://www.aiim.org/pdfa/ns/id/" a:part="2" "#;
        let attrs = |count| (0..count).map(|i| format!("x{i}=\"v=1\" ")).collect::<String>();
        let at_limit = format!("{prefix}{}/>", attrs(126));
        assert_eq!(identifier(&at_limit, PDFA_ID, "part").as_deref(), Some("2"));
        let over_limit = format!("{prefix}{}/>", attrs(127));
        assert_eq!(identifier(&over_limit, PDFA_ID, "part"), None);
        let namespaces = (0..129).map(|i| format!("xmlns:n{i}=\"urn:{i}\" ")).collect::<String>();
        assert_eq!(identifier(&format!("<root {namespaces}/>"), PDFA_ID, "part"), None);
        let many_elements = format!("<root>{}</root>", r#"<n a="v"/>"#.repeat(4097));
        assert!(!xml_limits_ok(&many_elements));
        // Not well-formed (the prefix is never declared): read by the conventional prefix, as before.
        let undeclared = r#"<rdf:Description pdfaid:part="3" pdfaid:conformance="u"/>"#;
        assert_eq!(identifier(undeclared, PDFA_ID, "part").as_deref(), Some("3"));
        assert_eq!(identifier(undeclared, PDFA_ID, "conformance").as_deref(), Some("u"));
        let over_bytes = format!("<root>{}</root>", " ".repeat(8 * 1024 * 1024));
        assert_eq!(identifier(&over_bytes, PDFA_ID, "part"), None);
    }
}
