use super::{BlockStyle, HashMap, Matrix, TextBlock, TextLine, paragraph_separator, same_base_font, same_font, strip_subset_tag, union};

fn same_plane(a: &TextLine, b: &TextLine) -> bool {
    let at = Matrix(a.origin.tm).then(&Matrix(a.origin.ctm));
    let bt = Matrix(b.origin.tm).then(&Matrix(b.origin.ctm));
    a.scope == b.scope
        && a.paint == b.paint
        && [0, 2].iter().all(|axis| {
            let ax = at.0.get(*axis).copied().unwrap_or(0.0);
            let ay = at.0.get(axis + 1).copied().unwrap_or(0.0);
            let bx = bt.0.get(*axis).copied().unwrap_or(0.0);
            let by = bt.0.get(axis + 1).copied().unwrap_or(0.0);
            let al = ax.hypot(ay);
            let bl = bx.hypot(by);
            al > 0.0 && bl > 0.0 && (ax / al - bx / bl).abs() < 0.001 && (ay / al - by / bl).abs() < 0.001
        })
}

pub(super) fn row_rect(row: &[usize], lines: &[TextLine]) -> Option<[f64; 4]> {
    row.iter().filter_map(|i| lines.get(*i)).map(|l| l.rect).reduce(union)
}

/// [`row_rect`] in its text's own frame ([`TextLine::frame_rect`]), so rows of turned text
/// compare along and across their baselines.
fn frame_row_rect(row: &[usize], lines: &[TextLine]) -> Option<[f64; 4]> {
    row.iter().filter_map(|i| lines.get(*i)).map(TextLine::frame_rect).reduce(union)
}

/// The left edge the rest of the page's text sits on, ignoring the rows of `block`. A single row
/// that does not share it is not merely an indented or full-width left-aligned line, so its own
/// centre is the only remaining explanation for where it sits.
fn neighbour_left(lines: &[TextLine], block: &TextBlock) -> Option<f64> {
    let mut edges: HashMap<i64, usize> = HashMap::new();
    for (i, line) in lines.iter().enumerate() {
        if block.lines.contains(&i) {
            continue;
        }
        // Quarter-point buckets: equal edges found through different runs still agree.
        *edges.entry((line.rect[0] * 4.0).round() as i64).or_insert(0) += 1;
    }
    let (best, _) = edges.into_iter().max_by_key(|(_, count)| *count)?;
    Some(best as f64 / 4.0)
}

/// Where an alignment picked for a one-row paragraph is measured, as `[left, right]`: the column of
/// text it sits in (an overlapping paragraph's left edge, to the furthest right its rows reach)
/// when that column holds it, otherwise its own box. Of the columns that hold it, the narrowest is
/// its own, the nearest on a tie: a heading across the whole page, even one closer than the body,
/// is wider than the column the title sits in, and gives it no frame while a narrower one does. A lone row's own box is no frame
/// to align in: left-aligning a title centred on the page left it where it was, and since a single
/// row grows to the page's margin, centring it again moved it right each time. `None` when no
/// alignment is asked for, when the paragraph is several rows, turned, or given a width or a move of
/// its own, or when its `draft` text changes and no column holds it.
pub(crate) fn alignment_frame(lines: &[TextLine], blocks: &[TextBlock], block: &TextBlock, style: &BlockStyle, draft: &str) -> Option<[f64; 2]> {
    // Every row of the draft is aligned, by its own alignment or the paragraph's: moving the
    // paragraph to its column's edge would move a row that has none. Entries past the draft's
    // rows name no row.
    let asked = (0..crate::paragraph_align::count(draft)).all(|row| style.aligns.get(row).copied().flatten().or(style.align).is_some());
    if !asked || style.width.is_some() || style.offset.is_some() || block.rows.len() != 1 || block.rotation.abs() > 0.01 {
        return None;
    }
    let own = [block.rect[0], block.rect[2]];
    let tolerance = block.size.max(1.0) * 0.5;
    let gap = |other: &TextBlock| (other.rect[1] - block.rect[3]).max(block.rect[1] - other.rect[3]).max(0.0);
    // The column an overlapping paragraph makes, when it holds the row.
    let column = |other: &TextBlock| -> Option<[f64; 2]> {
        let left = other.rect[0];
        // Whole rows, not their runs: a row of several styles reaches its last run's right edge.
        let right = other
            .rows
            .iter()
            .filter_map(|row| row_rect(row, lines))
            .filter(|row| (row[0] - left).abs() < tolerance)
            .map(|row| row[2])
            .fold(f64::NEG_INFINITY, f64::max);
        (right.is_finite() && right - left > own[1] - own[0] && own[0] >= left - tolerance && own[1] <= right + tolerance).then_some([left, right])
    };
    let frame = blocks
        .iter()
        .filter(|other| other.lines.iter().all(|i| !block.lines.contains(i)) && other.rect[2] > own[0] && other.rect[0] < own[1])
        .filter_map(|other| column(other).map(|frame| (frame, gap(other))))
        .min_by(|(a, near_a), (b, near_b)| (a[1] - a[0]).total_cmp(&(b[1] - b[0])).then(near_a.total_cmp(near_b)))
        .map(|(frame, _)| frame);
    // The row's own box, with no column to align in, is no width for new text to wrap at: edited,
    // the row keeps the width it would have had without the alignment.
    match frame {
        Some(frame) => Some(frame),
        None if draft != block.editor_text => None,
        None => Some(own),
    }
}

/// Whether a block reads as centred, and so must keep its centre when it grows or wraps.
///
/// Several ragged rows whose left edges disagree while their centres agree are centred. A one row
/// block cannot say that: all of its left edges are the same, so the ragged test never fires and a
/// centred title would be taken for left-aligned and grow to the right (#2581). Fall back to the
/// page instead — a lone row sitting on the page's own centre, and not on the edge the rest of the
/// text uses, is a centred title. A full-width left-aligned line still lines up with that edge, so
/// it is not mistaken for one.
pub(crate) fn is_centred(block: &TextBlock, lines: &[TextLine], page_centre: f64, size: f64) -> bool {
    let centre = (block.rect[0] + block.rect[2]) / 2.0;
    let lefts: Vec<_> = block.rows.iter().filter_map(|r| row_rect(r, lines).map(|r| r[0])).collect();
    if block.rows.len() > 1 {
        let left_span = lefts.iter().copied().fold(f64::NEG_INFINITY, f64::max) - lefts.iter().copied().fold(f64::INFINITY, f64::min);
        return left_span > size * 0.5
            && block.rows.iter().all(|row| row_rect(row, lines).is_some_and(|r| ((r[0] + r[2]) / 2.0 - centre).abs() < size));
    }
    lefts
        .first()
        .is_some_and(|left| (centre - page_centre).abs() < size && neighbour_left(lines, block).is_none_or(|edge| (*left - edge).abs() > size * 0.5))
}

pub(crate) fn group_blocks(lines: &[TextLine]) -> Vec<TextBlock> {
    group_layout(lines, true)
}

/// Exporters currently represent a block with one style. Keep their style boundaries until
/// they support rich runs, while edit selection may encompass the complete visual region.
/// Dotted-leader entries stay on one row and use the title run's style.
pub(crate) fn group_reading_blocks(lines: &[TextLine]) -> Vec<TextBlock> {
    let mut blocks = Vec::new();
    let mut start = 0;
    let mut offset = 0;
    for row in lines.chunk_by(|a, b| same_plane(a, b) && (a.origin.baseline - b.origin.baseline).abs() < a.size.min(b.size) * 0.2) {
        let mut text = String::new();
        for (i, line) in row.iter().enumerate() {
            if !text.ends_with(char::is_whitespace)
                && !line.text.starts_with(char::is_whitespace)
                && i.checked_sub(1).and_then(|i| row.get(i)).is_some_and(|prev| line.rect[0] - prev.rect[2] > line.size * 0.12)
            {
                text.push(' ');
            }
            text.push_str(&line.text);
        }
        if leader_entry(&text)
            && let Some(first) = row.first()
        {
            // Tab leaders are a single reading row, even when the title, dots and page
            // number are separate text objects far apart. Do not merge neighbouring entries.
            blocks.extend(group_layout(lines.get(start..offset).unwrap_or_default(), false));
            let mut line = first.clone();
            line.text = text;
            line.rect = row.iter().map(|line| line.rect).reduce(union).unwrap_or(first.rect);
            blocks.extend(group_layout(std::slice::from_ref(&line), false));
            start = offset + row.len();
        }
        offset += row.len();
    }
    blocks.extend(group_layout(lines.get(start..).unwrap_or_default(), false));
    blocks
}

/// Dotted leaders end in a page number, with a title before the dots. A dot in an article
/// number, or an ellipsis in ordinary prose, is not enough to make a contents entry.
fn leader_entry(text: &str) -> bool {
    let text = text.trim_end();
    let prefix = text.trim_end_matches(|c: char| {
        c.is_ascii_digit() || matches!(c, 'i' | 'v' | 'x' | 'l' | 'c' | 'd' | 'm' | 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M')
    });
    if prefix.len() == text.len() {
        return false;
    }
    let title = prefix.trim_end_matches(|c: char| c.is_whitespace() || matches!(c, '.' | '…'));
    let dots = prefix
        .get(title.len()..)
        .unwrap_or_default()
        .chars()
        .map(|c| match c {
            '.' => 1,
            '…' => 3,
            _ => 0,
        })
        .sum::<usize>();
    dots >= 3 && title.chars().any(char::is_alphabetic)
}

/// Text-space units → user space along the baseline. It is the vertical scale ([`Origin::k`])
/// unless the text matrix stretches the glyphs, as an OCR layer fitting each word to the scan does.
pub(super) fn across(line: &TextLine) -> f64 {
    let m = Matrix(line.origin.tm).then(&Matrix(line.origin.ctm)).0;
    let h = m[0].hypot(m[1]);
    if h.is_finite() && h > 1e-9 { h } else { line.origin.k }
}

/// Invisible text (rendering mode 3), as in an OCR layer over a scanned image.
fn invisible(line: &TextLine) -> bool {
    line.origin.state.render_mode == 3
}

fn same_style(a: &TextLine, b: &TextLine) -> bool {
    same_font(a, b.font.as_bytes(), &b.base_font, b.face.as_deref())
        // With no `/BaseFont` (or only a subset tag) there is no name to compare: the resource or
        // fallback face `same_font` matched is the whole identity, so keep the lines together.
        && (same_base_font(&a.base_font, &b.base_font)
            || (strip_subset_tag(&a.base_font).is_empty() && strip_subset_tag(&b.base_font).is_empty()))
        && a.bold == b.bold
        && a.italic == b.italic
        && a.color == b.color
        && (a.rotation() - b.rotation()).abs() < 0.01
        && (a.size - b.size).abs() < 0.01
}

/// Does a row open with a list marker? A bullet or a dash, or an ordered marker — digits (or a
/// single letter) followed by `.` or `)` and then a space or the row's end, so prose such as
/// `1.5` or `Dr. Smith` is left alone.
fn opens_with_marker(text: &str) -> bool {
    let Some(first) = text.chars().next() else { return false };
    if matches!(first, '•' | '◦' | '▪' | '‣' | '⁃' | '*' | '-' | '–' | '—') {
        return true;
    }
    if !first.is_ascii_digit() && !first.is_ascii_alphabetic() {
        return false;
    }
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    let letters = text.chars().take_while(char::is_ascii_alphabetic).count();
    // Both runs are ASCII characters, so their length in characters is their length in bytes:
    // the slice below starts on a character boundary.
    let rest = if digits > 0 {
        &text[digits..]
    } else if letters == 1 {
        &text[1..]
    } else {
        return false;
    };
    let mut after = rest.chars();
    matches!(after.next(), Some('.' | ')')) && after.next().is_none_or(char::is_whitespace)
}

/// A row that opens with a list marker opens its own item: joining it to the row above is what
/// let an edit of one paragraph delete every item after it (#2564). The other error, splitting
/// an ordinary paragraph, costs one more paragraph to click and never any text — so err this way.
fn starts_list_item(row: &[usize], lines: &[TextLine]) -> bool {
    row.iter().filter_map(|i| lines.get(*i)).map(|l| l.text.trim_start()).find(|t| !t.is_empty()).is_some_and(opens_with_marker)
}

fn group_layout(lines: &[TextLine], mixed: bool) -> Vec<TextBlock> {
    let mut rows: Vec<Vec<usize>> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let joins = rows.last().and_then(|r| r.last()).and_then(|i| lines.get(*i)).is_some_and(|prev| {
            let gap = line.frame_rect()[0] - prev.frame_rect()[2];
            // OCR words (invisible text over a scan) join only across a word space: a wider gap
            // is a table cell or a tab stop, which a rewrite would close up.
            let scanned = !mixed || (invisible(prev) == invisible(line) && (!invisible(line) || gap < prev.size.max(line.size) * 0.6));
            scanned
                && (mixed || same_style(prev, line))
                && same_plane(prev, line)
                // A superscript or subscript run (a footnote marker, say) sits on the same visual
                // row as the text around it, raised or lowered by `Ts` from the body baseline
                // (#3252). That shift is typically up to half the body size, not the marker's own
                // smaller size — but the wider tolerance only applies between a run and a
                // distinctly smaller or larger one, with no horizontal overlap (`gap >= 0.0`):
                // two equal-size runs can land within it too, at a baseline shift tight columns
                // or compressed text (`Tz`) can produce between genuinely separate rows, and the
                // original 20% tolerance is what keeps those apart.
                && ((prev.frame_origin().1 - line.frame_origin().1).abs() < prev.size.min(line.size) * 0.2
                    || (gap >= 0.0
                        && prev.size.min(line.size) < prev.size.max(line.size) * 0.9
                        && (prev.frame_origin().1 - line.frame_origin().1).abs() < prev.size.max(line.size) * 0.5))
                && gap >= -prev.size.min(line.size) * 0.2
                && gap < prev.size.max(line.size) * 1.5
                // Separate same-style text objects that merely touch or overlap, with no space
                // between them, are separate labels (for example a paragraph dragged against
                // another, #594). Style runs and words separated by a space or gap still join.
                && (prev.origin.bt_op == line.origin.bt_op
                    || gap > prev.size.min(line.size) * 0.01
                    || !same_style(prev, line)
                    || prev.text.ends_with(char::is_whitespace)
                    || line.text.starts_with(char::is_whitespace))
        });
        if joins && let Some(row) = rows.last_mut() {
            row.push(i);
        } else {
            rows.push(vec![i]);
        }
    }
    let mut blocks: Vec<TextBlock> = Vec::new();
    for row_at in 0..rows.len() {
        let Some(row) = rows.get(row_at) else { continue };
        let Some(first) = row.first().and_then(|i| lines.get(*i)) else { continue };
        let (Some(rect), Some(frame)) = (row_rect(row, lines), frame_row_rect(row, lines)) else { continue };
        let joins = blocks.last().is_some_and(|block| {
            // A bullet of its own opens a list item (#2564): such a row is a new paragraph, and
            // letting it join is what made an edit of the row above delete this item and the ones
            // after it.
            if starts_list_item(row, lines) {
                return false;
            }
            let Some(prev_row) = block.rows.last() else { return false };
            let Some(prev) = prev_row.first().and_then(|i| lines.get(*i)) else { return false };
            let Some(pr) = frame_row_rect(prev_row, lines) else { return false };
            let gap = prev.frame_origin().1 - first.frame_origin().1;
            let tol = prev.size.max(first.size).max(1.0);
            // The block opens with a list marker and this row sits further right: the item's text
            // is at a hanging indent, so the row it wrapped onto continues it (#2564). A row at
            // the block's own left edge is already covered by the match on `pr[0]`. Both boxes are
            // in the frame, as the rest of this comparison is, so turned text matches too.
            let hanging = frame[0] > pr[0]
                && frame[0] - pr[0] <= tol * 3.0
                && block.lines.first().and_then(|&i| lines.get(i)).is_some_and(|l| opens_with_marker(l.text.trim_start()));
            let aligned = (pr[0] - frame[0]).abs() < tol
                || ((pr[0] + pr[2] - frame[0] - frame[2]) / 2.0).abs() < tol
                || (pr[2] - frame[2]).abs() < tol
                || hanging;
            // Once a paragraph has two rows, its own leading is the measured baseline gap. A
            // blank line is about twice that, so a wider gap is the next paragraph (#2517). A
            // single row uses the text object's TL when it has one: a gap equal to that TL
            // joins, and a gap of twice the TL does not. `Td` does not set TL (`TD` does). A
            // step that repeats on the following row is that leading; one large gap, with no
            // second step to compare, stays a break. The old cap of 2.5 × the size joined the
            // blank line too.
            let own_leading = block.rows.len().checked_sub(2).and_then(|i| {
                let above = block.rows.get(i)?.first().and_then(|n| lines.get(*n));
                let below = prev_row.first().and_then(|n| lines.get(*n));
                match (above, below) {
                    (Some(above), Some(below)) => Some((above.frame_origin().1 - below.frame_origin().1).abs()),
                    _ => None,
                }
            });
            let stated = (prev.origin.state.leading > 0.0).then(|| prev.wrap_leading()).filter(|lead| lead.is_finite() && *lead > tol * 0.4);
            // The next row is not in this block yet. Its gap is only a leading when it matches
            // this one: a blank line is a single wider gap and has no matching step after it.
            let repeated = rows.get(row_at.saturating_add(1)).and_then(|next| {
                let nfirst = next.first().and_then(|i| lines.get(*i))?;
                let nframe = frame_row_rect(next, lines)?;
                let ngap = first.frame_origin().1 - nfirst.frame_origin().1;
                let same = same_plane(first, nfirst)
                    && (mixed || same_style(first, nfirst))
                    && first.size.min(nfirst.size) >= first.size.max(nfirst.size) * 0.4
                    && (nframe[0] - frame[0]).abs() < tol;
                let scale = gap.abs().max(ngap.abs());
                if same && ngap > tol * 0.8 && scale < tol * 4.0 && (ngap - gap).abs() <= scale * 0.08 + 0.5 { Some(gap) } else { None }
            });
            // A lone row with none of those: the step to the row after it is the leading of the
            // paragraph it opens, and the gap above it is a heading's or a blank line's when it
            // is much wider (#3577).
            let following = rows.get(row_at.saturating_add(1)).and_then(|next| {
                let nfirst = next.first().and_then(|i| lines.get(*i))?;
                let ngap = first.frame_origin().1 - nfirst.frame_origin().1;
                (same_plane(first, nfirst) && ngap > tol * 0.8 && ngap < tol * 2.05).then_some(ngap)
            });
            let max_gap = own_leading
                .filter(|lead| *lead > tol * 0.4)
                .or(stated)
                .or(repeated)
                .map_or_else(|| following.map_or(tol * 2.05, |lead| (lead * 1.35).min(tol * 2.05)), |lead| lead * 1.5);
            // OCR lines stay apart: the scan fixes where each one is, so they never rewrap together.
            (!mixed || !(invisible(prev) || invisible(first)))
                && (mixed || same_style(prev, first))
                && same_plane(prev, first)
                // The rewrite sets every row in the first one's horizontal scale.
                && (prev.origin.state.scale - first.origin.state.scale).abs() < 1e-6
                && aligned
                && first.size.min(prev.size) >= first.size.max(prev.size) * 0.4
                && gap > first.size.min(prev.size) * 0.8
                && gap < max_gap
        });
        if !joins {
            blocks.push(TextBlock {
                text: String::new(),
                editor_text: String::new(),
                reflow_text: String::new(),
                joins: Vec::new(),
                spans: Vec::new(),
                rect,
                base_font: first.base_font.clone(),
                size: first.size,
                bold: first.bold,
                italic: first.italic,
                color: first.color,
                rotation: first.rotation(),
                lines: Vec::new(),
                rows: Vec::new(),
                invisible: true,
            });
        }
        let Some(block) = blocks.last_mut() else { continue };
        let mut row_text = String::new();
        if !block.rows.is_empty() {
            block.editor_text.push('\n');
        }
        // The run placed before this one in the row, tracked rather than searched for: rows
        // don't share runs, so it is the last one this loop pushed.
        let mut prev_in_row: Option<&TextLine> = None;
        for &index in row {
            let Some(line) = lines.get(index) else { continue };
            // Some producers position separate words without encoding a space.
            if let Some(prev) = prev_in_row
                && !row_text.ends_with(char::is_whitespace)
                && !line.text.starts_with(char::is_whitespace)
                && line.frame_rect()[0] - prev.frame_rect()[2] > line.size * 0.12
            {
                row_text.push(' ');
                block.editor_text.push(' ');
            }
            let start = block.editor_text.len();
            block.editor_text.push_str(&line.text);
            block.spans.push((start..block.editor_text.len(), index));
            row_text.push_str(&line.text);
            block.invisible &= line.origin.state.render_mode == 3;
            block.lines.push(index);
            prev_in_row = Some(line);
        }
        if !block.text.is_empty() {
            if block.text.ends_with('-') {
                block.text.pop();
            } else {
                block.text.push(' ');
            }
        }
        block.text.push_str(row_text.trim_matches(paragraph_separator));
        block.rect = union(block.rect, rect);
        block.rows.push(row.clone());
    }
    for block in &mut blocks {
        block.reflow_text = reflow_text(block, lines);
        let entries = structured_entries(block, lines);
        let ragged = if entries { Vec::new() } else { ragged_breaks(block, lines) };
        block.joins = (1..block.rows.len()).map(|row| break_gap(block, lines, row, Some((&ragged, entries)))).collect();
    }
    for block in blocks.iter_mut().filter(|block| block.invisible && block.lines.len() > 1) {
        // Each word counts once: neighbouring words OCR sized alike share a run.
        let words = block
            .lines
            .iter()
            .filter_map(|i| lines.get(*i))
            .flat_map(|line| std::iter::repeat_n(line.size, line.text.split_whitespace().count().max(1)));
        if let Some(size) = median_size(words) {
            block.size = size;
        }
    }
    blocks
}

/// The text a width change should reflow. A `/PCSoft` line is a wrap and rejoins. A `/PCHard`
/// line, or a short line that is not marked, keeps its newline. An unmarked line that reaches
/// the paragraph's right edge and is wide enough to be a measure rejoins too: that is how a
/// file written elsewhere wraps a body line.
fn reflow_text(block: &TextBlock, lines: &[TextLine]) -> String {
    let parts: Vec<&str> = block.editor_text.split('\n').collect();
    if parts.len() != block.rows.len() {
        return block.editor_text.clone();
    }
    let mut out = String::new();
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            match break_gap(block, lines, index, None) {
                Some(_) if out.ends_with('-') => {
                    out.pop();
                }
                // `gap` is the exact number of separator characters the wrap consumed (0 for a
                // Chinese/Japanese word split, which has no space of its own) when the marker or
                // the source recorded it; a line already ending in whitespace is left alone so a
                // row whose own text kept a trailing space isn't doubled.
                Some(gap) if !out.ends_with(char::is_whitespace) => out.push_str(&" ".repeat(gap)),
                Some(_) => {}
                None => out.push('\n'),
            }
        }
        out.push_str(part);
    }
    out
}

/// `None`: a hard break, kept as a newline. `Some(gap)`: a soft wrap, rejoined with exactly `gap`
/// spaces — the count a `/PCSoft` marker recorded ([`TextLine::soft_gap`]), or, for a line this
/// app did not mark (a file written elsewhere, or an older save), a single space when the
/// geometry looks like an ordinary wrapped body line (`#2529`).
///
/// `ragged` is `Some((verdicts, entries))` for a rewrite that rejoins rows around a replacement ([`TextBlock::joins`]),
/// where a wrong guess changes text the replacement does not touch; it holds the verdict of
/// [`ragged_breaks`] for each break of the block. A row that ends in a hyphen is never rejoined
/// then: it may be a real one ("well-" "known") that joining would delete. An unmarked row is a
/// wrap when either (a) at least two rows of the paragraph reach the measure (the widest row of an
/// address or a poem always "reaches" its own right edge, so one alone proves nothing) and this one
/// does too, as in justified text; or (b) the paragraph is ragged-right text that flows and
/// [`ragged_breaks`] says so of this break.
fn break_gap(block: &TextBlock, lines: &[TextLine], row_index: usize, ragged: Option<(&[bool], bool)>) -> Option<usize> {
    let line = block.rows.get(row_index).and_then(|row| row.first()).and_then(|i| lines.get(*i))?;
    if line.hard_break {
        return None;
    }
    if line.soft_break {
        return Some(line.soft_gap);
    }
    let prev = block.rows.get(row_index.wrapping_sub(1)).and_then(|row| row_rect(row, lines))?;
    let next = block.rows.get(row_index).and_then(|row| row_rect(row, lines))?;
    let size = line.size.max(1.0);
    let slack = block.rect[2] - prev[2];
    let width = prev[2] - prev[0];
    let reaches = |rect: [f64; 4]| block.rect[2] - rect[2] <= size * 0.75 && rect[2] - rect[0] >= size * 8.0;
    let mut justified = true;
    if let Some((_, entries)) = ragged {
        // Rows that are entries of a list (a table of contents, say) are never a paragraph's wraps.
        if entries {
            return None;
        }
        let full = block.rows.iter().filter_map(|row| row_rect(row, lines)).filter(|rect| reaches(*rect)).count();
        if ends_in_hyphen(&row_text(block, lines, row_index.saturating_sub(1))) {
            return None;
        }
        justified = full >= 2;
    }
    // Short rows define the right edge by their own letters. A body line that fills the
    // measure leaves almost no room for the next word.
    let fills = slack <= size * 0.75 && width >= size * 8.0 && (next[2] - next[0]) + size * 0.25 > slack;
    if !(justified && fills) && !ragged.is_some_and(|(r, _)| r.get(row_index).copied().unwrap_or(false)) {
        return None;
    }
    let before = row_text(block, lines, row_index.saturating_sub(1)).trim_end().chars().next_back();
    let after = row_text(block, lines, row_index).trim_start().chars().next();
    Some(wrap_gap(before, after))
}

/// How much of an em the slack may exceed the next word's room by and still be a wrap. Rows of
/// ragged text end a little short of the right edge of the widest one, which stands in for the
/// measure, so a word that only just fits is taken to have been pushed down.
const WRAP_TOLERANCE: f64 = 0.1;

/// Would the first word of the next row, with the space before it, not have fitted in the `slack`
/// that is left at the end of a row set at `size`? Then the row ended because it was full, which is
/// what a wrap is; with room to spare it ended on purpose.
fn word_would_not_fit(slack: f64, space: f64, word: f64, size: f64) -> bool {
    slack < space + word + size * WRAP_TOLERANCE
}

/// Does a row's text end in a hyphen: possibly a hyphenation, possibly a real one?
fn ends_in_hyphen(text: &str) -> bool {
    text.trim_end().ends_with(['-', '\u{2010}', '\u{00AD}'])
}

/// Does a row end a sentence or introduce what follows? Closing quotes and brackets after the
/// mark are looked through.
fn ends_sentence(text: &str) -> bool {
    text.trim_end()
        .trim_end_matches(['"', '\'', ')', ']', '\u{201D}', '\u{2019}', '\u{00BB}'])
        .ends_with(['.', '!', '?', ':', '\u{2026}', '\u{3002}', '\u{FF01}', '\u{FF1F}'])
}

/// The width of the first word of row `row` and of the space that would precede it. The word is
/// measured from the ink boxes of the row (those of runs that touch are one word); when the row
/// has none it is the row's average advance per character times the word's length, an
/// approximation good enough to tell a word wider than the slack from one that is not. The space
/// is the gap after that word when the row has a second word, else a quarter em (0.6 em in a
/// monospaced face). A script written without spaces has no word to move: its "word" is the one
/// character that would have to fit, with no space before it.
fn next_word_width(block: &TextBlock, lines: &[TextLine], row: usize, text: &str, before: Option<char>) -> Option<(f64, f64)> {
    let rect = row_rect(block.rows.get(row)?, lines)?;
    let first = lines.get(*block.rows.get(row)?.first()?)?;
    let size = first.size.max(1.0);
    let text = text.trim_start();
    let after = text.chars().next()?;
    if before.is_some_and(no_space_script) && no_space_script(after) {
        let per_char = (rect[2] - rect[0]) / text.trim_end().chars().count().max(1) as f64;
        return (per_char.is_finite() && per_char > 0.0).then_some((per_char, 0.0));
    }
    let mono = first.base_font.to_ascii_lowercase().contains("courier") || first.base_font.to_ascii_lowercase().contains("mono");
    let default_space = size * if mono { 0.6 } else { 0.25 };
    let mut boxes = block.rows.get(row)?.iter().filter_map(|i| lines.get(*i)).flat_map(|l| l.ink.iter().copied());
    let (mut word, mut space) = (None::<f64>, default_space);
    if let Some(open) = boxes.next() {
        let (left, mut right) = (open[0], open[2]);
        for b in boxes {
            let gap = b[0] - right;
            if gap < size * 0.12 {
                right = right.max(b[2]);
            } else {
                if gap < size * 0.8 {
                    space = gap;
                }
                break;
            }
        }
        word = Some(right - left);
    }
    let word = word.filter(|w| w.is_finite() && *w > 0.0).or_else(|| {
        let per_char = (rect[2] - rect[0]) / text.trim_end().chars().count().max(1) as f64;
        let chars = text.split_whitespace().next().map_or(0, |w| w.chars().count());
        Some(per_char * chars as f64).filter(|w| w.is_finite() && *w > 0.0)
    })?;
    Some((word, space))
}

/// A run of leader characters: a table of contents' dots, an underscored fill, a rule of dashes.
fn has_leader(text: &str) -> bool {
    let mut run = 0;
    for c in text.chars() {
        if matches!(c, '.' | '\u{2026}' | '_' | '-' | '\u{00B7}' | '\u{2022}') {
            run += if c == '\u{2026}' { 3 } else { 1 };
            if run >= 3 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

/// Does the row open with a section number or a list marker ("8", "3.2", "1.1.", a bullet)?
fn opens_with_number_or_marker(text: &str) -> bool {
    let text = text.trim_start();
    let token = text.split_whitespace().next().unwrap_or("");
    let numbered = token.chars().any(|c| c.is_ascii_digit()) && token.chars().all(|c| c.is_ascii_digit() || c == '.') && text.len() > token.len();
    numbered || opens_with_marker(text)
}

/// Are the rows of `block` entries of a list or table rather than the wraps of a paragraph: a
/// leader run or a gap of more than three ems between two words (the dots or the tab before a
/// page number, a table's columns) in any row, or section numbers or markers opening more than one
/// row? A rewrite keeps every row of such a block, however long the rows are and however the next
/// entry's first word would not have fitted on the one above.
fn structured_entries(block: &TextBlock, lines: &[TextLine]) -> bool {
    let size = block.size.max(1.0);
    let mut numbered = 0;
    for (index, row) in block.rows.iter().enumerate() {
        let text = row_text(block, lines, index);
        if has_leader(&text) {
            return true;
        }
        let mut right = None::<f64>;
        for b in row.iter().filter_map(|i| lines.get(*i)).flat_map(|l| l.ink.iter()) {
            if right.is_some_and(|r| b[0] - r > size * 3.0) {
                return true;
            }
            right = Some(right.map_or(b[2], |r| r.max(b[2])));
        }
        numbered += usize::from(opens_with_number_or_marker(&text));
    }
    numbered > 1
}

/// What one break of an unmarked paragraph says about ragged-right flow.
#[derive(Clone, Copy, PartialEq)]
enum Break {
    /// Not a break to judge: it has its own rule (a hyphen, a mark, a new paragraph's indent), or
    /// ends a sentence with room to spare, or follows a row too short to be a measure.
    Apart,
    /// The row before it is the widest, so it has no room left whatever follows: no evidence.
    Widest,
    /// The next word would not have fitted in the room left on a row that is not the widest.
    Proof,
    /// The next word would have fitted, or could not be measured: these are lines of their own.
    Lines,
}

/// For each break of `block` (index 0, the first row, has none), whether it is a wrap of ragged-right
/// text, judged once for the whole paragraph. The test is the classic one, "would the next word have
/// fitted?", against the right edge of the widest row. That row's own break proves nothing (it has
/// no room left whatever follows), so a paragraph needs two breaks after rows that are not the
/// widest where the word would not have fitted, and no break where it would have: one row that
/// ended early shows the lines are the author's own. Rows must share a left edge, but for a first
/// line's indent or a hanging indent after a list marker, which also rules out centred and
/// right-aligned text, whose rows look ragged by half of the slack; a later row that starts most of an em
/// or more to the right opens a new paragraph.
fn ragged_breaks(block: &TextBlock, lines: &[TextLine]) -> Vec<bool> {
    let rows = block.rows.len();
    let none = vec![false; rows];
    if rows < 3 {
        return none;
    }
    let rects: Vec<[f64; 4]> = block.rows.iter().filter_map(|row| row_rect(row, lines)).collect();
    let texts: Vec<String> = (0..rows).map(|row| row_text(block, lines, row)).collect();
    if rects.len() != rows {
        return none;
    }
    let size = block.size.max(1.0);
    let Some(left) = rects.iter().skip(1).map(|r| r[0]).reduce(f64::min) else { return none };
    let marker = texts.first().is_some_and(|t| opens_with_marker(t.trim_start()));
    let first_ok = rects.first().is_some_and(|r| {
        (r[0] - left).abs() < size * 0.25 || (r[0] > left && r[0] - left <= size * 4.0) || (marker && r[0] < left && left - r[0] <= size * 4.0)
    });
    let mut indented = vec![false; rows];
    for (i, r) in rects.iter().enumerate().skip(1) {
        if r[0] - left >= size * 0.75 && r[0] - left <= size * 4.0 {
            if let Some(slot) = indented.get_mut(i) {
                *slot = true;
            }
        } else if r[0] - left >= size * 0.25 {
            return none;
        }
    }
    if !first_ok {
        return none;
    }
    let widest = rects.iter().map(|r| r[2]).fold(f64::MIN, f64::max);
    let mut kinds = vec![Break::Apart; rows];
    let mut proofs = 0;
    for i in 1..rows {
        let above = i.saturating_sub(1);
        let (Some(prev), Some(prev_text), Some(text), Some(lead)) =
            (rects.get(above), texts.get(above), texts.get(i), block.rows.get(i).and_then(|r| r.first()).and_then(|n| lines.get(*n)))
        else {
            return none;
        };
        let slack = widest - prev[2];
        let kind = if lead.hard_break || lead.soft_break || indented.get(i).copied().unwrap_or(false) || ends_in_hyphen(prev_text) {
            Break::Apart
        } else {
            let before = prev_text.trim_end().chars().next_back();
            match next_word_width(block, lines, i, text, before) {
                None => Break::Lines,
                Some((word, space)) => {
                    if !word_would_not_fit(slack, space, word, size) {
                        Break::Lines
                    } else if prev[2] - prev[0] < size * 8.0 || (ends_sentence(prev_text) && slack > size) {
                        Break::Apart
                    } else if slack <= size * 0.25 {
                        Break::Widest
                    } else {
                        Break::Proof
                    }
                }
            }
        };
        proofs += usize::from(kind == Break::Proof);
        if let Some(slot) = kinds.get_mut(i) {
            *slot = kind;
        }
    }
    if proofs < 2 || kinds.contains(&Break::Lines) {
        return none;
    }
    kinds.into_iter().map(|k| matches!(k, Break::Proof | Break::Widest)).collect()
}

/// The separators a wrap stood for between the characters `before` and `after`: none between two
/// of a script written without spaces, one otherwise.
fn wrap_gap(before: Option<char>, after: Option<char>) -> usize {
    usize::from(!(before.is_some_and(no_space_script) && after.is_some_and(no_space_script)))
}

/// The text of row `row` of `block`, its runs joined.
fn row_text(block: &TextBlock, lines: &[TextLine], row: usize) -> String {
    block.rows.get(row).into_iter().flatten().filter_map(|i| lines.get(*i)).map(|l| l.text.as_str()).collect()
}

/// Scripts written without a space between words: Han, Hiragana, Katakana, their punctuation and
/// the full-width forms, and Thai. A row that wraps between two of them gets no space back.
pub(crate) fn no_space_script(c: char) -> bool {
    matches!(c, '\u{0E00}'..='\u{0E7F}' | '\u{3000}'..='\u{30FF}' | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '\u{FF00}'..='\u{FFEF}')
}

/// The size an OCR phrase is set in. Recognize text sizes each word from its own box and a guess
/// at how much of the em its letters fill, so one phrase's words come out several points apart
/// (#3313); a rewrite draws the phrase at this one size. `None` when no size is usable.
fn median_size(sizes: impl Iterator<Item = f64>) -> Option<f64> {
    let mut sizes: Vec<f64> = sizes.filter(|size| size.is_finite() && *size > 0.0).collect();
    sizes.sort_by(f64::total_cmp);
    let middle = sizes.len() / 2;
    let upper = sizes.get(middle).copied()?;
    if sizes.len().is_multiple_of(2) { sizes.get(middle.saturating_sub(1)).map(|lower| (lower + upper) / 2.0) } else { Some(upper) }
}

#[cfg(test)]
mod wrap_tests {
    use std::sync::Arc;

    use super::{ends_sentence, group_layout, no_space_script, word_would_not_fit, wrap_gap};

    fn page(content: &str) -> pdfcraft_cos::Document {
        let objs: Vec<String> = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".into(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into(),
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offs = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offs.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let x = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for o in offs {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n", objs.len() + 1).as_bytes());
        pdfcraft_cos::Document::open(Arc::new(out)).unwrap()
    }

    /// Without ink boxes the first word of a row is measured by the row's average advance, which
    /// still tells ragged-right flow from lines (the rows here are cut so each next word is wider
    /// than the room left).
    #[test]
    fn a_row_without_ink_boxes_is_measured_by_its_average_advance() {
        let rows = [
            "The committee reviewed the proposal and agreed that the",
            "schedule for the new building would depend on the weather",
            "during the autumn months so the contractor was asked to",
            "prepare a second plan that avoids the lowest field and the",
            "river bank",
        ];
        let content: String = rows.iter().enumerate().map(|(i, r)| format!("BT /F1 10 Tf 72 {} Td ({r}) Tj ET\n", 700.0 - 12.0 * i as f64)).collect();
        let mut lines = crate::text_lines(&page(&content), 0).unwrap();
        let with_ink = group_layout(&lines, false);
        lines.iter_mut().for_each(|l| l.ink.clear());
        let without = group_layout(&lines, false);
        assert_eq!(with_ink.len(), 1);
        assert_eq!(with_ink[0].joins, without[0].joins);
        assert!(without[0].joins.iter().all(|j| *j == Some(1)), "{:?}", without[0].joins);
    }

    #[test]
    fn a_sentence_end_looks_through_closing_marks() {
        assert!(ends_sentence("it ended.\u{201D} "));
        assert!(ends_sentence("(see below.)"));
        assert!(ends_sentence("as follows:"));
        assert!(!ends_sentence("and then"));
        assert!(!ends_sentence("well-"));
    }

    /// A 10 pt row with the space and a 30 pt word to place: 2.5 + 30 = 32.5 pt of room needed.
    #[test]
    fn the_next_word_decides_whether_a_row_was_full() {
        // Fits with room to spare: a deliberate break.
        assert!(!word_would_not_fit(40.0, 2.5, 30.0, 10.0));
        // Does not fit: a wrap.
        assert!(word_would_not_fit(20.0, 2.5, 30.0, 10.0));
        assert!(word_would_not_fit(0.0, 2.5, 30.0, 10.0));
        // The tolerance (a tenth of an em) forgives a word that only just fits, not more.
        assert!(word_would_not_fit(33.0, 2.5, 30.0, 10.0));
        assert!(!word_would_not_fit(33.6, 2.5, 30.0, 10.0));
        // A script without spaces needs no space: one character of 10 pt against 5 pt of slack.
        assert!(word_would_not_fit(5.0, 0.0, 10.0, 10.0));
        assert!(!word_would_not_fit(15.0, 0.0, 10.0, 10.0));
    }

    #[test]
    fn rows_of_unspaced_scripts_wrap_without_a_space() {
        assert!(['日', '本', 'あ', 'カ', '。', 'ก', 'ａ'].into_iter().all(no_space_script));
        assert!(!['a', 'é', '한', '1', ' '].into_iter().any(no_space_script));
        assert_eq!(wrap_gap(Some('語'), Some('文')), 0);
        assert_eq!(wrap_gap(Some('ก'), Some('ข')), 0);
        assert_eq!(wrap_gap(Some('d'), Some('e')), 1);
        assert_eq!(wrap_gap(Some('語'), Some('e')), 1, "a space belongs between a word of each");
        assert_eq!(wrap_gap(None, Some('e')), 1);
    }
}
