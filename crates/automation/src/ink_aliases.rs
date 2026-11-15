//! Print production ▸ Ink Manager ▸ Ink aliases as tools: the spot colorants a document uses
//! (`doc_inks`), and renaming them so that two inks print as one (`doc_ink_aliases`).

use pdfcraft_engine::{Edit, InkAliases};
use serde_json::{Value, json};

use crate::{Args, Automation, Result, ToolError};

impl Automation {
    /// The spot colorants (Separation and DeviceN) the document's colour spaces name, each with
    /// how many colour spaces name it. Process colorants, All and None are not listed.
    pub(crate) fn doc_inks(&mut self, a: &Args) -> Result<Value> {
        let id = self.doc(a)?.id;
        let inks: Vec<Value> = self.session.list_inks(id).into_iter().map(|(name, spaces)| json!({ "name": name, "spaces": spaces })).collect();
        Ok(json!({ "inks": inks }))
    }

    /// `aliases`: `{ "from", "to" }` pairs. Each spot colorant named `from` is renamed `to` in
    /// every Separation and DeviceN colour space, as one undoable edit. A document that uses none
    /// of the `from` inks is not an error: `renamed` is 0 and no edit is made.
    pub(crate) fn doc_ink_aliases(&mut self, a: &Args) -> Result<Value> {
        let list = a
            .get("aliases")
            .and_then(Value::as_array)
            .ok_or_else(|| ToolError::InvalidArgs("aliases must be a list of { from, to } pairs".into()))?;
        let mut aliases = InkAliases::default();
        for pair in list {
            let name = |key: &str| pair.get(key).and_then(Value::as_str).map(str::to_owned);
            let (Some(from), Some(to)) = (name("from"), name("to")) else {
                return Err(ToolError::InvalidArgs("every alias needs a from and a to ink name".into()));
            };
            aliases.aliases.push((from, to));
        }
        if aliases.aliases.is_empty() {
            return Err(ToolError::InvalidArgs("give at least one { from, to } alias".into()));
        }
        aliases.check().map_err(ToolError::InvalidArgs)?;
        let id = self.doc(a)?.id;
        let found = self.session.count_ink_aliases(id, &aliases);
        if found.spaces == 0 {
            return Ok(json!({ "renamed": 0, "objects": 0 }));
        }
        let r = self.apply(a, Edit::AliasInks { aliases })?;
        Ok(json!({ "renamed": found.spaces, "objects": found.objects, "undo": r["undo"] }))
    }
}
