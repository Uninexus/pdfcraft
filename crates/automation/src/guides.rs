//! View ▸ Guides as tools: list, add and remove the ruler guides of a document (`doc_guides`,
//! `doc_guide_add`, `doc_guide_remove`) and snap a point to them (`doc_guide_snap`), and a point to a grid (`doc_grid_snap`). View state, not edits (see `pdfcraft_engine::guides`).

use pdfcraft_engine::guides::{Grid, Guide, GuideAxis};
use serde_json::{Value, json};

use crate::{Args, Automation, Result, ToolError};

fn guide_json(g: &Guide) -> Value {
    json!({ "id": g.id, "page": g.page + 1, "axis": g.axis.name(), "position": g.position })
}

/// The optional 1-based `page` as 0-based. Not checked against the document: guides may name a
/// page that no longer exists, and must still be found and removed.
fn stale_page(a: &Args) -> Result<Option<usize>> {
    match a.opt_int("page")? {
        None => Ok(None),
        Some(p) if p >= 1 => Ok(Some(usize::try_from(p - 1).unwrap_or(usize::MAX))),
        Some(_) => Err(Args::wrong("page", "1 or more")),
    }
}

impl Automation {
    pub(crate) fn doc_guides(&mut self, a: &Args) -> Result<Value> {
        let id = self.doc(a)?.id;
        let page = stale_page(a)?;
        let guides = self.session.guides(id, page).map_err(ToolError::from)?;
        Ok(json!({ "guides": guides.iter().map(guide_json).collect::<Vec<_>>() }))
    }

    pub(crate) fn doc_guide_add(&mut self, a: &Args) -> Result<Value> {
        let id = self.doc(a)?.id;
        let page = self.page(a)?;
        let axis = GuideAxis::parse(a.str("axis")?).ok_or_else(|| Args::wrong("axis", "vertical or horizontal"))?;
        let position = a.opt_num("position")?.ok_or_else(|| Args::missing("position"))?;
        let (guide, created) = self.session.add_guide(id, page, axis, position).map_err(|e| ToolError::InvalidArgs(e.to_string()))?;
        let mut out = guide_json(&guide);
        out["created"] = json!(created);
        Ok(out)
    }

    pub(crate) fn doc_guide_remove(&mut self, a: &Args) -> Result<Value> {
        let id = self.doc(a)?.id;
        let guide = a.opt_int("id")?;
        let all = a.opt_bool("all")?.unwrap_or(false);
        let removed = match (guide, all) {
            (Some(_), true) | (None, false) => return Err(ToolError::InvalidArgs("give either id, or all: true".into())),
            (Some(g), false) => {
                let g = u64::try_from(g).ok().filter(|g| *g >= 1).ok_or_else(|| Args::wrong("id", "a guide id from doc_guides (1 or more)"))?;
                usize::from(self.session.remove_guide(id, g).map_err(ToolError::from)?)
            }
            (None, true) => {
                let page = stale_page(a)?;
                self.session.clear_guides(id, page).map_err(ToolError::from)?
            }
        };
        Ok(json!({ "removed": removed }))
    }

    pub(crate) fn doc_guide_snap(&mut self, a: &Args) -> Result<Value> {
        let id = self.doc(a)?.id;
        let page = stale_page(a)?.ok_or_else(|| Args::missing("page"))?;
        let x = a.opt_num("x")?;
        let y = a.opt_num("y")?;
        if x.is_none() && y.is_none() {
            return Err(ToolError::InvalidArgs("give x, y, or both".into()));
        }
        let tolerance = a.opt_num("tolerance")?.unwrap_or(4.0);
        let (sx, sy) = self.session.snap_to_guides(id, page, x, y, tolerance).map_err(|e| ToolError::InvalidArgs(e.to_string()))?;
        let one = |s: Option<pdfcraft_engine::guides::AxisSnap>| {
            s.map(|s| json!({ "position": s.position, "guide": s.guide, "snapped": s.guide.is_some() }))
        };
        let mut out = json!({ "tolerance": tolerance });
        if let Some(v) = one(sx) {
            out["x"] = v;
        }
        if let Some(v) = one(sy) {
            out["y"] = v;
        }
        Ok(out)
    }

    pub(crate) fn doc_grid_snap(&mut self, a: &Args) -> Result<Value> {
        let id = self.doc(a)?.id;
        let page = self.page(a)?;
        let x = a.opt_num("x")?;
        let y = a.opt_num("y")?;
        if x.is_none() && y.is_none() {
            return Err(ToolError::InvalidArgs("give x, y, or both".into()));
        }
        let unit = a.opt_str("unit")?.unwrap_or("pt");
        let per_unit = [("pt", 1.0), ("in", 72.0), ("mm", 72.0 / 25.4), ("cm", 72.0 / 2.54)]
            .into_iter()
            .find_map(|(name, points)| (name == unit).then_some(points))
            .ok_or_else(|| Args::wrong("unit", "pt, in, mm or cm"))?;
        let spacing = a.opt_num("spacing")?.ok_or_else(|| Args::missing("spacing"))? * per_unit;
        let subdivisions = match a.opt_int("subdivisions")? {
            None => 1,
            Some(n) => u32::try_from(n).map_err(|_| Args::wrong("subdivisions", "1 to 100"))?,
        };
        let grid = Grid {
            spacing,
            subdivisions,
            offset_x: a.opt_num("offset_x")?.unwrap_or(0.0) * per_unit,
            offset_y: a.opt_num("offset_y")?.unwrap_or(0.0) * per_unit,
        };
        let tolerance = a.opt_num("tolerance")?;
        let (sx, sy) = self.session.snap_to_grid(id, page, x, y, &grid, tolerance).map_err(|e| ToolError::InvalidArgs(e.to_string()))?;
        let one = |s: Option<pdfcraft_engine::guides::GridSnap>| s.map(|s| json!({ "position": s.position, "snapped": s.snapped }));
        let mut out = json!({ "spacing_pt": grid.spacing, "pitch_pt": grid.pitch(), "tolerance": tolerance });
        if let Some(v) = one(sx) {
            out["x"] = v;
        }
        if let Some(v) = one(sy) {
            out["y"] = v;
        }
        Ok(out)
    }
}
