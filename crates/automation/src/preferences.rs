//! Preferences that need no document: Identity and Forms.

use pdfcraft_engine::js::MAX_IDENTITY_CHARS;
use serde_json::{Value, json};

use crate::comments::DEFAULT_AUTHOR;
use crate::{Args, Automation, Result, ToolError};

impl Automation {
    /// `identity`: set or read the name and email.
    pub(crate) fn identity(&mut self, a: &Args) -> Result<Value> {
        let name = a.opt_str("name")?;
        let email = a.opt_str("email")?;
        for (key, v) in [("name", name), ("email", email)] {
            if v.is_some_and(|v| v.trim().chars().count() > MAX_IDENTITY_CHARS) {
                return Err(ToolError::InvalidArgs(format!("{key} may be at most {MAX_IDENTITY_CHARS} characters")));
            }
        }
        if name.is_some() || email.is_some() {
            let now = self.session.identity().clone();
            self.session.set_identity(name.unwrap_or(&now.name), email.unwrap_or(&now.email));
        }
        let id = self.session.identity();
        let author = if id.name.is_empty() { DEFAULT_AUTHOR } else { id.name.as_str() };
        Ok(json!({ "name": id.name, "email": id.email, "author": author }))
    }

    /// `form_preferences`: Highlight fields and Auto-Complete.
    pub(crate) fn form_preferences(&mut self, a: &Args) -> Result<Value> {
        // Checked before anything changes, so a refused call changes nothing.
        let forget = a.opt_str("forget_entry")?.map(str::trim);
        if let Some(text) = forget
            && !self.session.form_entries().entries().iter().any(|e| e == text)
        {
            return Err(ToolError::InvalidArgs(format!("there is no remembered entry {text:?} to forget (see list_entries)")));
        }
        if let Some(on) = a.opt_bool("highlight_fields")? {
            self.session.set_field_highlight(on);
        }
        if let Some(mode) = a.opt_str("auto_complete")? {
            let on = match mode {
                "off" => false,
                "basic" => true,
                other => return Err(ToolError::InvalidArgs(format!("auto_complete must be \"off\" or \"basic\", not {other:?}"))),
            };
            self.session.set_form_autocomplete(on);
        }
        if let Some(text) = forget {
            self.session.form_entries_mut().remove(text);
        }
        if a.opt_bool("clear_entries")?.unwrap_or(false) {
            self.session.clear_form_entries();
        }
        let mut out = json!({
            "highlight_fields": self.session.field_highlight(),
            "auto_complete": if self.session.form_autocomplete() { "basic" } else { "off" },
            "entry_count": self.session.form_entries().len(),
        });
        // What is remembered is the person's own typing: listed only when asked for.
        if a.opt_bool("list_entries")?.unwrap_or(false) {
            out["entries"] = json!(self.session.form_entries().entries());
        }
        if let Some(typed) = a.opt_str("suggest")? {
            out["suggestions"] = json!(self.session.suggest_form_entries(typed));
        }
        Ok(out)
    }
}
