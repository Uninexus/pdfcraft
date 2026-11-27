//! Align and distribute objects selected together in Edit a PDF.
//!
//! The layout is worked out on displayed rectangles (points, origin top-left, y down), so "left"
//! is the side the reader sees on a rotated page. [`ArrangeOp`] names the command,
//! [`plan_offsets`] is the pure layout, and [`Document::arrange_objects`] turns a selection into
//! one [`Edit::ArrangeObjects`] that moves every object that has to move as one undo step.

use pdfcraft_cos::Document as Cos;
use pdfcraft_edit::{EditableObject, ObjectTarget};

use crate::{Document, Edit, EditError};

/// How close two rectangles must be for the same object to be recognised after others moved.
const SAME_RECT: f64 = 0.05;
/// A move smaller than this is no move.
const NO_MOVE: f64 = 1e-6;

/// What to do to the selected objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrangeOp {
    AlignLeft,
    AlignCenter,
    AlignRight,
    AlignTop,
    AlignMiddle,
    AlignBottom,
    /// Equal gaps between objects from left to right; the outer two stay.
    DistributeHorizontal,
    /// Equal gaps between objects from top to bottom; the outer two stay.
    DistributeVertical,
}

impl ArrangeOp {
    /// Every operation, in menu order.
    pub const ALL: [ArrangeOp; 8] = [
        ArrangeOp::AlignLeft,
        ArrangeOp::AlignCenter,
        ArrangeOp::AlignRight,
        ArrangeOp::AlignTop,
        ArrangeOp::AlignMiddle,
        ArrangeOp::AlignBottom,
        ArrangeOp::DistributeHorizontal,
        ArrangeOp::DistributeVertical,
    ];

    /// Whether this spreads objects out (three or more needed) rather than lines them up.
    pub fn distributes(self) -> bool {
        matches!(self, ArrangeOp::DistributeHorizontal | ArrangeOp::DistributeVertical)
    }

    /// How many objects the operation needs.
    pub fn needs(self) -> usize {
        if self.distributes() { 3 } else { 2 }
    }

    /// The name automation takes: `left`, `center`, `right`, `top`, `middle`, `bottom`,
    /// `distribute_horizontal` or `distribute_vertical`.
    pub fn name(self) -> &'static str {
        match self {
            ArrangeOp::AlignLeft => "left",
            ArrangeOp::AlignCenter => "center",
            ArrangeOp::AlignRight => "right",
            ArrangeOp::AlignTop => "top",
            ArrangeOp::AlignMiddle => "middle",
            ArrangeOp::AlignBottom => "bottom",
            ArrangeOp::DistributeHorizontal => "distribute_horizontal",
            ArrangeOp::DistributeVertical => "distribute_vertical",
        }
    }

    /// The registered command that runs this operation.
    pub fn command(self) -> &'static str {
        match self {
            ArrangeOp::AlignLeft => "arrange.align_left",
            ArrangeOp::AlignCenter => "arrange.align_center",
            ArrangeOp::AlignRight => "arrange.align_right",
            ArrangeOp::AlignTop => "arrange.align_top",
            ArrangeOp::AlignMiddle => "arrange.align_middle",
            ArrangeOp::AlignBottom => "arrange.align_bottom",
            ArrangeOp::DistributeHorizontal => "arrange.distribute_horizontal",
            ArrangeOp::DistributeVertical => "arrange.distribute_vertical",
        }
    }

    /// The operation the command `id` runs.
    pub fn from_command(id: &str) -> Option<ArrangeOp> {
        Self::ALL.into_iter().find(|op| op.command() == id)
    }

    /// The operation `name` ([`Self::name`]) stands for.
    pub fn from_name(name: &str) -> Option<ArrangeOp> {
        Self::ALL.into_iter().find(|op| op.name() == name)
    }
}

/// The displayed offset `[dx, dy]` each of `rects` ([x0, y0, x1, y1], y down) moves by: aligned to
/// the selection's bounds, or spread with equal gaps. `None` when there are too few rectangles
/// ([`ArrangeOp::needs`]) or one isn't finite.
pub fn plan_offsets(rects: &[[f64; 4]], op: ArrangeOp) -> Option<Vec<[f64; 2]>> {
    if rects.len() < op.needs() || rects.iter().flatten().any(|v| !v.is_finite()) {
        return None;
    }
    let min = |i: usize| rects.iter().map(|r| r[i]).fold(f64::INFINITY, f64::min);
    let max = |i: usize| rects.iter().map(|r| r[i]).fold(f64::NEG_INFINITY, f64::max);
    let (bx, by) = ((min(0) + max(2)) / 2.0, (min(1) + max(3)) / 2.0);
    let each = |f: &dyn Fn(&[f64; 4]) -> [f64; 2]| rects.iter().map(f).collect::<Vec<_>>();
    Some(match op {
        ArrangeOp::AlignLeft => each(&|r| [min(0) - r[0], 0.0]),
        ArrangeOp::AlignRight => each(&|r| [max(2) - r[2], 0.0]),
        ArrangeOp::AlignCenter => each(&|r| [bx - (r[0] + r[2]) / 2.0, 0.0]),
        ArrangeOp::AlignTop => each(&|r| [0.0, min(1) - r[1]]),
        ArrangeOp::AlignBottom => each(&|r| [0.0, max(3) - r[3]]),
        ArrangeOp::AlignMiddle => each(&|r| [0.0, by - (r[1] + r[3]) / 2.0]),
        ArrangeOp::DistributeHorizontal => spread(rects, 0),
        ArrangeOp::DistributeVertical => spread(rects, 1),
    })
}

/// Offsets along `axis` (0: x, 1: y) that give equal gaps; the first and last by centre stay.
fn spread(rects: &[[f64; 4]], axis: usize) -> Vec<[f64; 2]> {
    let (lo, hi) = (axis, axis + 2);
    let mut order: Vec<usize> = (0..rects.len()).collect();
    order.sort_by(|&a, &b| (rects[a][lo] + rects[a][hi]).total_cmp(&(rects[b][lo] + rects[b][hi])));
    let mut out = vec![[0.0; 2]; rects.len()];
    let (Some(&first), Some(&last)) = (order.first(), order.last()) else { return out };
    let sizes: f64 = rects.iter().map(|r| r[hi] - r[lo]).sum();
    let gap = (rects[last][hi] - rects[first][lo] - sizes) / (rects.len() - 1) as f64;
    let mut cursor = rects[first][lo];
    for &i in &order {
        if i != first && i != last {
            out[i][axis] = cursor - rects[i][lo];
        }
        cursor += rects[i][hi] - rects[i][lo] + gap;
    }
    out
}

fn near(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= SAME_RECT)
}

/// Moves each object of `moves` by its own user-space offset, as one atomic change. Indexes can
/// shift as paragraphs regroup after a move, so each object is found again by its kind and
/// rectangle before it moves; a rectangle two objects share, or one that is gone, is refused.
pub(crate) fn move_each(doc: &mut Cos, page: usize, moves: &[(ObjectTarget, [f64; 2])]) -> Result<(), EditError> {
    let first = pdfcraft_edit::editable_objects(doc, page)?;
    let mut work = doc.clone();
    for (target, offset) in moves {
        let rect = first
            .iter()
            .find(|o| o.target == *target)
            .map(|o| o.rect)
            .ok_or_else(|| EditError::Invalid("a selected object no longer exists; select the objects again".into()))?;
        if offset.iter().all(|v| v.abs() < NO_MOVE) {
            continue;
        }
        let now = pdfcraft_edit::editable_objects(&work, page)?;
        let mut hits = now.iter().filter(|o| o.target.kind == target.kind && near(o.rect, rect));
        match (hits.next(), hits.next()) {
            (Some(found), None) => pdfcraft_edit::move_objects(&mut work, page, &[found.target], *offset)?,
            _ => return Err(EditError::Invalid("these objects overlap too closely to arrange; move them apart first".into())),
        }
    }
    *doc = work;
    Ok(())
}

impl Document {
    /// The edit that aligns or distributes `targets` (references from [`Self::editable_objects`]
    /// of `page`), or `None` when every object is already in place.
    pub fn arrange_objects(&self, page: usize, targets: &[ObjectTarget], op: ArrangeOp) -> Result<Option<Edit>, String> {
        if targets.len() < op.needs() {
            return Err(format!("select at least {} objects", op.needs()));
        }
        let info = self.info.pages.get(page).ok_or("the page no longer exists")?;
        let inventory = self.editable_objects(page)?;
        let mut found: Vec<&EditableObject> = Vec::with_capacity(targets.len());
        for target in targets {
            let object = inventory.iter().find(|o| o.target == *target).ok_or("a selected object no longer exists; select the objects again")?;
            if found.iter().any(|o| o.target == *target) {
                return Err("an object was selected more than once".into());
            }
            found.push(object);
        }
        let shown: Vec<[f64; 4]> = found
            .iter()
            .map(|o| {
                let (a, b) = (info.user_to_view_f64(o.rect[0], o.rect[1]), info.user_to_view_f64(o.rect[2], o.rect[3]));
                [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])]
            })
            .collect();
        let plan = plan_offsets(&shown, op).ok_or("these objects can't be arranged")?;
        let mut moves = Vec::new();
        for (object, delta) in found.iter().zip(plan) {
            if delta.iter().all(|v| v.abs() < NO_MOVE) {
                continue;
            }
            moves.push((object.target, self.object_move_offset(page, delta)?));
        }
        Ok((!moves.is_empty()).then_some(Edit::ArrangeObjects { page, moves, op }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(rects: &[[f64; 4]], op: ArrangeOp) -> Vec<[f64; 4]> {
        let plan = plan_offsets(rects, op).expect("enough rectangles");
        rects.iter().zip(plan).map(|(r, d)| [r[0] + d[0], r[1] + d[1], r[2] + d[0], r[3] + d[1]]).collect()
    }

    fn sample() -> Vec<[f64; 4]> {
        vec![[10.0, 20.0, 40.0, 50.0], [100.0, 5.0, 120.0, 95.0], [60.0, 70.0, 150.0, 80.0], [30.0, 200.0, 45.0, 260.0]]
    }

    #[test]
    fn aligning_makes_one_edge_equal_and_keeps_sizes() {
        let before = sample();
        let left = apply(&before, ArrangeOp::AlignLeft);
        let least = before.iter().map(|r| r[0]).fold(f64::INFINITY, f64::min);
        assert!(left.iter().all(|r| (r[0] - least).abs() < 0.01));
        let bottom = apply(&before, ArrangeOp::AlignBottom);
        let most = before.iter().map(|r| r[3]).fold(f64::NEG_INFINITY, f64::max);
        assert!(bottom.iter().all(|r| (r[3] - most).abs() < 0.01));
        let middle = apply(&before, ArrangeOp::AlignMiddle);
        assert!(middle.iter().all(|r| ((r[1] + r[3]) / 2.0 - (middle[0][1] + middle[0][3]) / 2.0).abs() < 0.01));
        for (a, b) in before.iter().zip(&left) {
            assert!(((a[2] - a[0]) - (b[2] - b[0])).abs() < 0.01);
        }
    }

    #[test]
    fn distributing_equalises_gaps_and_leaves_the_outer_objects() {
        let before = sample();
        for (op, lo, hi) in [(ArrangeOp::DistributeHorizontal, 0, 2), (ArrangeOp::DistributeVertical, 1, 3)] {
            let after = apply(&before, op);
            let mut order: Vec<usize> = (0..after.len()).collect();
            order.sort_by(|&a, &b| (after[a][lo] + after[a][hi]).total_cmp(&(after[b][lo] + after[b][hi])));
            let gaps: Vec<f64> = order.windows(2).map(|w| after[w[1]][lo] - after[w[0]][hi]).collect();
            assert!(gaps.iter().all(|g| (g - gaps[0]).abs() < 0.01), "{gaps:?}");
            let (first, last) = (order[0], order[order.len() - 1]);
            assert_eq!(after[first], before[first]);
            assert_eq!(after[last], before[last]);
        }
    }

    #[test]
    fn too_few_objects_are_not_arranged() {
        assert!(plan_offsets(&sample()[..1], ArrangeOp::AlignLeft).is_none());
        assert!(plan_offsets(&sample()[..2], ArrangeOp::DistributeVertical).is_none());
        assert!(plan_offsets(&sample()[..2], ArrangeOp::AlignTop).is_some());
    }
}
