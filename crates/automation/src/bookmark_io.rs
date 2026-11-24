//! Bookmark trees in and out of documents: `bookmark_export`, `bookmark_import`, `bookmark_copy`.
//!
//! The portable form is JSON: `{"bookmarks": [node, …]}`, where a node is `title`, `page`
//! (1-based, or `null` for one that goes nowhere), `view` (`{"type", "values"}`: a destination
//! type and its values in PDF user space, as the file stores them), `color`, `bold`, `italic`,
//! `open` and `children`. What export writes, import takes back unchanged. A file also carries
//! `"truncated": true` when the export left out bookmarks nested past the listing limit or a title that did not fit the listing's title allowance (that node has an empty title).
use crate::{Args, Automation, Result, ToolError, failed};
use pdfcraft_engine::bookmark_json::{bookmark_file_bytes, node_json, nodes_of as parse_nodes, parse_bookmark_file};
use pdfcraft_engine::{DocId, Edit, OutlineNode};
use serde_json::{Value, json};

impl Automation {
    pub(crate) fn bookmark_export(&self, a: &Args) -> Result<Value> {
        let id = self.doc(a)?.id;
        let path = a.opt_path("path")?.unwrap_or_default();
        let tree = self.outline_of(id, &path)?;
        let mut out = json!({ "bookmarks": tree.nodes.iter().map(node_json).collect::<Vec<_>>(), "truncated": tree.truncated });
        if let Some(file) = a.opt_str("out")? {
            let target = self.resolve(file, true)?;
            let text = bookmark_file_bytes(&tree.nodes, tree.truncated).map_err(failed)?;
            crate::write_atomic(&target, &text)?;
            out["path"] = json!(target);
        }
        Ok(out)
    }

    pub(crate) fn bookmark_import(&mut self, a: &Args) -> Result<Value> {
        let (nodes, truncated) = match (a.get("bookmarks"), a.opt_str("file")?) {
            (Some(list), None) => (nodes_of(list)?, false),
            (None, Some(file)) => {
                let path = self.resolve(file, false)?;
                let (list, truncated) = parse_bookmark_file(&crate::read_input(&path)?).map_err(failed)?;
                (nodes_of(&list)?, truncated)
            }
            _ => return Err(ToolError::InvalidArgs("give either bookmarks or file".into())),
        };
        let mut out = self.write_tree(a, nodes)?;
        out["truncated"] = json!(truncated);
        Ok(out)
    }

    pub(crate) fn bookmark_copy(&mut self, a: &Args) -> Result<Value> {
        let from = a.int("from")?;
        let from = DocId(u64::try_from(from).map_err(|_| ToolError::InvalidArgs("from must be positive".into()))?);
        let path = a.opt_path("path")?.unwrap_or_default();
        let tree = self.outline_of(from, &path)?;
        let mut out = self.write_tree(a, tree.nodes)?;
        out["truncated"] = json!(tree.truncated);
        Ok(out)
    }

    fn outline_of(&self, id: DocId, path: &[usize]) -> Result<pdfcraft_engine::OutlineTree> {
        match self.session.outline_tree(id, path) {
            // The engine names the path 0-based; the caller gave it 1-based.
            Some(tree) => tree.map_err(|e| match e {
                pdfcraft_organize::OutlineError::NoSuchBookmark(_) => failed(format!("there is no bookmark at {:?}", one_based(path))),
                e => failed(e),
            }),
            None => Err(failed(format!("document {} is not open, or its bookmarks can't be read for editing", id.0))),
        }
    }

    /// Write `nodes` into `doc` under `parent` at `position` (or in place of the outline with
    /// `replace`), and say which of them went nowhere because the document lacks their page.
    fn write_tree(&mut self, a: &Args, bookmarks: Vec<OutlineNode>) -> Result<Value> {
        let parent = a.opt_path("parent")?.unwrap_or_default();
        let index = a.opt_int("position")?.map_or(usize::MAX, |p| usize::try_from(p.max(1) - 1).unwrap_or(usize::MAX));
        let replace = a.opt_bool("replace")?.unwrap_or(false);
        let count = self.doc(a)?.info.pages.len();
        let unmapped = pdfcraft_engine::outline_unmapped(&bookmarks, count);
        let mut out = self.apply(a, Edit::ImportBookmarks { parent, index, bookmarks, replace })?;
        out["unmapped"] = json!(unmapped.iter().map(|p| one_based(p)).collect::<Vec<_>>());
        Ok(out)
    }
}

fn one_based(path: &[usize]) -> Vec<usize> {
    path.iter().map(|i| i.saturating_add(1)).collect()
}

fn nodes_of(list: &Value) -> Result<Vec<OutlineNode>> {
    parse_nodes(list).map_err(ToolError::InvalidArgs)
}
