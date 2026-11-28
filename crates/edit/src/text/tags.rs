//! Structure tags through a paragraph rewrite. A tagged page ties a structure element to the
//! content a marked-content sequence holds (ISO 32000-2 §14.7.5.1), by its `MCID`. A rewrite puts
//! the new text outside the paragraph's text object, so a sequence opened inside that text object
//! would be left empty and the new text untagged: the sequence goes round the new text instead.

use std::collections::HashSet;

use pdfcraft_content::Op;
use pdfcraft_cos::{Dict, Document, Object};

/// A tagged sequence the new text takes with it: the operators that open and close it now.
pub(crate) struct Tag {
    open: usize,
    close: usize,
    begin: Op,
}

/// The sequence opens with an `MCID`, or marks an artifact.
fn tagged(doc: &Document, res: &Dict, op: &Op) -> bool {
    if op.name(0) == Some(b"Artifact") {
        return true;
    }
    match op.operands.get(1) {
        Some(Object::Dict(properties)) => properties.get(b"MCID").is_some(),
        Some(Object::Name(_)) => {
            let named = op.name(1).and_then(|name| res.get(b"Properties").and_then(|o| doc.dict(o))?.get(name).cloned());
            named.and_then(|o| doc.dict(&o)).is_some_and(|properties| properties.get(b"MCID").is_some())
        }
        _ => false,
    }
}

/// Whether `op` puts something on the page.
fn draws(op: &Op) -> bool {
    [&b"Tj"[..], b"TJ", b"'", b"\"", b"Do", b"sh", b"BI", b"S", b"s", b"f", b"F", b"f*", b"B", b"B*", b"b", b"b*"].contains(&op.op.as_slice())
}

/// The tagged sequence that has to move with the paragraph whose first rewritten text-showing
/// operator is `shown`, in the text object opened at `bt`: the innermost one round it, when it
/// opens and closes inside that text object and holds nothing that stays. `None` when the tag can
/// stay where it is (it opens outside the text object, so the new text lands inside it), or can't
/// move (other text still needs it).
pub(crate) fn tag_to_move(doc: &Document, res: &Dict, ops: &[Op], bt: usize, shown: usize, dropped: &HashSet<usize>) -> Option<Tag> {
    let mut open = Vec::new();
    for (i, op) in ops.iter().enumerate().take(shown) {
        match op.op.as_slice() {
            b"BMC" | b"BDC" => open.push(i),
            b"EMC" => {
                open.pop();
            }
            _ => {}
        }
    }
    let open = open.into_iter().rev().find(|&i| ops.get(i).is_some_and(|op| tagged(doc, res, op)))?;
    if open <= bt {
        return None;
    }
    let close = super::rewrite::matching_emc(ops, open)?;
    let inside = ops.get(open..close)?;
    let leaves_text_object = inside.iter().any(|op| op.is("BT") || op.is("ET"));
    let stays = inside.iter().enumerate().any(|(i, op)| draws(op) && !dropped.contains(&(open + i)));
    if leaves_text_object || stays {
        return None;
    }
    Some(Tag { open, close, begin: ops.get(open)?.clone() })
}

impl Tag {
    /// Put the sequence round the new text in `block_ops` (its `q … Q`), and add the operators it
    /// opened and closed with to `dropped`: an `MCID` marks one sequence only.
    pub(crate) fn take(self, block_ops: &mut Vec<Op>, dropped: &mut HashSet<usize>) {
        let (Some(first), Some(last)) = (block_ops.iter().position(|op| op.is("q")), block_ops.iter().rposition(|op| op.is("Q"))) else {
            return;
        };
        block_ops.insert(last + 1, Op::new("EMC", vec![]));
        block_ops.insert(first, self.begin);
        dropped.insert(self.open);
        dropped.insert(self.close);
    }
}
