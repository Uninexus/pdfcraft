//! View ▸ Guides as tools: list, add and remove the ruler guides of a document (`doc_guides`,
//! `doc_guide_add`, `doc_guide_remove`) and snap a point to them (`doc_guide_snap`). View state, not edits (see `pdfcraft_engine::guides`).

use pdfcraft_engine::guides::{Guide, GuideAxis};
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
}
