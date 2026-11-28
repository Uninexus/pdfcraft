//! Where elements added to a structure tree go, so that the tree reads in page order: before the
//! first element that begins on a later page, and inside a grouping element whose own content
//! lies on both sides. What the tree already held is not reordered.

use super::{Document, HashMap, ObjRef, Object};

/// How deep elements are followed.
const MAX_DEPTH: usize = 256;
/// The types that only group others, so another element can go among their kids.
const GROUPS: [&[u8]; 6] = [b"Document", b"DocumentFragment", b"Part", b"Sect", b"Art", b"Div"];

/// First and last page an element's content is on, as page numbers.
type Span = Option<(usize, usize)>;

pub(super) struct Order {
    /// Page → its number.
    pages: HashMap<ObjRef, usize>,
    /// Element → its span, read once.
    spans: HashMap<ObjRef, Span>,
}

fn join(a: Span, b: Span) -> Span {
    match (a, b) {
        (Some(a), Some(b)) => Some((a.0.min(b.0), a.1.max(b.1))),
        (a, b) => a.or(b),
    }
}

impl Order {
    pub(super) fn new(pages: impl Iterator<Item = ObjRef>) -> Self {
        Order { pages: pages.enumerate().map(|(number, page)| (page, number)).collect(), spans: HashMap::new() }
    }

    fn span(&mut self, out: &Document, element: ObjRef, depth: usize) -> Span {
        if let Some(span) = self.spans.get(&element) {
            return *span;
        }
        // Entered before its kids, so a loop ends here.
        self.spans.insert(element, None);
        let held = out.get(element);
        let dict = held.as_dict()?;
        let on = |page: Option<ObjRef>| page.and_then(|p| self.pages.get(&p)).map(|n| (*n, *n));
        let own = on(dict.reference(b"Pg"));
        let kids = match dict.get(b"K").map(|k| (k, out.resolve(k))) {
            Some((k, list)) => list.as_array().cloned().unwrap_or_else(|| vec![k.clone()]),
            None => Vec::new(),
        };
        let mut span = None;
        let mut below = Vec::new();
        for kid in &kids {
            match out.resolve(kid).as_dict() {
                Some(d) if d.contains(b"S") => below.extend(kid.as_ref()),
                // A marked content or object reference, on its own page or the element's.
                Some(d) => span = join(span, d.reference(b"Pg").map_or(own, |p| on(Some(p)))),
                None => span = join(span, own),
            }
        }
        if depth < MAX_DEPTH {
            for kid in below {
                span = join(span, self.span(out, kid, depth + 1));
            }
        }
        self.spans.insert(element, span);
        span
    }

    /// Put `element` among `list`, the kids of `parent`, or deeper where the page order wants it.
    pub(super) fn place(&mut self, out: &mut Document, list: &mut Vec<Object>, parent: ObjRef, element: ObjRef, depth: usize) {
        let own = self.span(out, element, 0);
        let at = own
            .and_then(|(first, _)| list.iter().position(|o| o.as_ref().and_then(|r| self.span(out, r, 0)).is_some_and(|(begins, _)| begins > first)));
        let at = at.unwrap_or(list.len());
        // The element before may reach past the new one: then the new one belongs inside it.
        let around = at.checked_sub(1).and_then(|i| list.get(i)).and_then(Object::as_ref);
        if depth < MAX_DEPTH
            && let (Some((_, last)), Some(around)) = (own, around)
            && self.span(out, around, 0).is_some_and(|(_, ends)| ends > last)
            && let Some(mut dict) = out.get(around).as_dict().cloned()
            && dict.name(b"S").is_some_and(|s| GROUPS.contains(&s))
            // One kid needs no array.
            && let Some(mut kids) = dict.get(b"K").map(|k| out.resolve(k).as_array().cloned().unwrap_or_else(|| vec![k.clone()]))
            // Only among elements: marked content of the group's own has no place to compare.
            && kids.iter().all(|k| out.resolve(k).as_dict().is_some_and(|d| d.contains(b"S")))
        {
            self.place(out, &mut kids, around, element, depth + 1);
            dict.set(b"K".to_vec(), Object::Array(kids));
            out.set(around, Object::Dict(dict));
            self.spans.remove(&around);
            return;
        }
        if let Some(mut dict) = out.get(element).as_dict().cloned() {
            dict.set(b"P".to_vec(), Object::Ref(parent));
            out.set(element, Object::Dict(dict));
        }
        list.insert(at, Object::Ref(element));
    }
}
