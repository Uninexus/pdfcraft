//! View ▸ Guides as tools: list, add, move and remove the ruler guides of a document (`doc_guides`,
//! `doc_guide_add`, `doc_guide_move`, `doc_guide_remove`) and snap a point to them (`doc_guide_snap`), and a point to a grid (`doc_grid_snap`). View state, not edits (see `pdfcraft_engine::guides`).

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

/// The optional `unit` argument (`pt`, `in`, `mm` or `cm`; default `pt`) as `(numerator,
/// denominator)`: a value in that unit is `value * numerator / denominator` points. The
/// division comes last so that a value exactly at a limit converts exactly (5080 mm is 14400 pt,
/// where a pre-divided factor of 72/25.4 gives 14400.000000000002).
fn grid_unit(a: &Args) -> Result<(f64, f64)> {
    let unit = a.opt_str("unit")?.unwrap_or("pt");
    [("pt", (1.0, 1.0)), ("in", (72.0, 1.0)), ("mm", (72.0, 25.4)), ("cm", (72.0, 2.54))]
        .into_iter()
        .find_map(|(name, ratio)| (name == unit).then_some(ratio))
        .ok_or_else(|| Args::wrong("unit", "pt, in, mm or cm"))
}

impl Automation {
    /// `grid_preferences`: set, clear or read the session's Units & Guides grid.
    pub(crate) fn grid_preferences(&mut self, a: &Args) -> Result<Value> {
        let (num, den) = grid_unit(a)?;
        let to_pt = |v: f64| v * num / den;
        let (spacing, offset_x, offset_y) = (a.opt_num("spacing")?, a.opt_num("offset_x")?, a.opt_num("offset_y")?);
        let subdivisions = match a.opt_int("subdivisions")? {
            None => None,
            Some(n) => Some(u32::try_from(n).map_err(|_| Args::wrong("subdivisions", "1 to 100"))?),
        };
        let clear = a.opt_bool("clear")?.unwrap_or(false);
        let given = spacing.is_some() || offset_x.is_some() || offset_y.is_some() || subdivisions.is_some();
        if clear && given {
            return Err(ToolError::InvalidArgs("give either clear: true, or the grid values to set, not both".into()));
        }
        // Everything is checked before the stored grid changes, so a refused call changes nothing.
        if clear {
            self.session.set_grid_preference(None).map_err(|e| ToolError::InvalidArgs(e.to_string()))?;
        } else if given {
            let now = self.session.grid_preference().unwrap_or(Grid::new(72.0));
            let grid = Grid {
                spacing: spacing.map_or(now.spacing, to_pt),
                subdivisions: subdivisions.unwrap_or(now.subdivisions),
                // `+ 0.0` turns a negative zero into 0, so it is not echoed as -0.0.
                offset_x: offset_x.map_or(now.offset_x, |v| to_pt(v) + 0.0),
                offset_y: offset_y.map_or(now.offset_y, |v| to_pt(v) + 0.0),
            };
            self.session.set_grid_preference(Some(grid)).map_err(|e| ToolError::InvalidArgs(e.to_string()))?;
        }
        Ok(match self.session.grid_preference() {
            None => json!({ "set": false }),
            Some(g) => json!({
                "set": true, "spacing_pt": g.spacing, "subdivisions": g.subdivisions, "pitch_pt": g.pitch(),
                "offset_x_pt": g.offset_x, "offset_y_pt": g.offset_y,
            }),
        })
    }

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

    pub(crate) fn doc_guide_move(&mut self, a: &Args) -> Result<Value> {
        let id = self.doc(a)?.id;
        let guide = a.opt_int("id")?.ok_or_else(|| Args::missing("id"))?;
        let guide = u64::try_from(guide).ok().filter(|g| *g >= 1).ok_or_else(|| Args::wrong("id", "a guide id from doc_guides (1 or more)"))?;
        let position = a.opt_num("position")?.ok_or_else(|| Args::missing("position"))?;
        let (guide, moved) = self.session.move_guide(id, guide, position).map_err(|e| ToolError::InvalidArgs(e.to_string()))?;
        let mut out = guide_json(&guide);
        out["moved"] = json!(moved);
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
        let (num, den) = grid_unit(a)?;
        // Whatever a call leaves out comes from the grid preference (`grid_preferences`), else
        // from the plain defaults (no subdivisions, lines through the top left corner); only the
        // spacing has no default.
        let stored = self.session.grid_preference();
        let mut from_preferences = Vec::new();
        let mut pick = |name: &'static str, given: Option<f64>, stored: Option<f64>, default: Option<f64>| -> Option<f64> {
            match (given, stored) {
                (Some(v), _) => Some(v * num / den),
                (None, Some(v)) => {
                    from_preferences.push(name);
                    Some(v)
                }
                (None, None) => default,
            }
        };
        let spacing = pick("spacing", a.opt_num("spacing")?, stored.map(|g| g.spacing), None)
            .ok_or_else(|| ToolError::InvalidArgs("missing argument spacing (or store a grid with grid_preferences)".into()))?;
        let offset_x = pick("offset_x", a.opt_num("offset_x")?, stored.map(|g| g.offset_x), Some(0.0)).unwrap_or(0.0);
        let offset_y = pick("offset_y", a.opt_num("offset_y")?, stored.map(|g| g.offset_y), Some(0.0)).unwrap_or(0.0);
        let subdivisions = match (a.opt_int("subdivisions")?, stored) {
            (Some(n), _) => u32::try_from(n).map_err(|_| Args::wrong("subdivisions", "1 to 100"))?,
            (None, Some(g)) => {
                from_preferences.push("subdivisions");
                g.subdivisions
            }
            (None, None) => 1,
        };
        let grid = Grid { spacing, subdivisions, offset_x, offset_y };
        let tolerance = a.opt_num("tolerance")?;
        let (sx, sy) = self.session.snap_to_grid(id, page, x, y, &grid, tolerance).map_err(|e| ToolError::InvalidArgs(e.to_string()))?;
        let one = |s: Option<pdfcraft_engine::guides::GridSnap>| s.map(|s| json!({ "position": s.position, "snapped": s.snapped }));
        let mut out = json!({ "spacing_pt": grid.spacing, "pitch_pt": grid.pitch(), "tolerance": tolerance, "from_preferences": from_preferences });
        if let Some(v) = one(sx) {
            out["x"] = v;
        }
        if let Some(v) = one(sy) {
            out["y"] = v;
        }
        Ok(out)
    }
}
